//! Walrus and comprehension loop-variable bindings in the constraint environment.

use super::*;

/// PEP 572 (#774), deep-review follow-up (round 4): binds each walrus
/// target's own inferred type into the solver's environment before the
/// containing `If`/`While` test or bare `ExprStmt` is walked for
/// unification by `collect_expr_constraints` above -- mirroring `crate::
/// collect_named_expr_bindings`'s own pre-pass pattern in the full
/// flow-sensitive checker exactly, including its documented tradeoff of
/// recomputing `value`'s constraints rather than threading the term through
/// from this pre-pass (see that function's own doc comment). Without this,
/// a walrus target reaches `local_names` (via `collect_named_expr_names_in_
/// expr`) but never gets a `bindings` entry, so a later read of that name
/// wrongly fails `collect_expr_constraints`'s own `Name` arm's `unbound_
/// local` check even though the walrus unconditionally executes whenever
/// the statement containing it is reached.
///
/// Walked in the same left-to-right evaluation order as `collect_expr_
/// constraints`'s own traversal, so a later sibling sub-expression that
/// reads an earlier walrus-bound name (`(a := 1) + (b := a + 1)`) resolves
/// correctly. Each target is bound as a *definite* binding -- not a
/// maybe-binding -- mirroring the `Assign` arm above exactly (`defs_
/// rebound.remove`, `maybe_bindings.remove`, `bindings.entry(name).or_
/// insert(term)`): a walrus nested anywhere in an `if`/`while` test
/// executes whenever the statement itself is reached, unlike a name
/// assigned in only one branch of the `if`, which is bound separately by
/// `collect_block_constraints`'s own branch-join logic, not by this walk.
pub(crate) fn bind_named_expr_targets(
    signatures: &HashMap<String, SignatureTerms>,
    parents: &mut Vec<usize>,
    concrete: &mut Vec<Option<Ty>>,
    deferred: &mut DeferredConstraints,
    env: &mut ConstraintEnvironment<'_, '_>,
    expr: &HirExpr,
) -> Result<(), Diagnostic> {
    match expr {
        HirExpr::NamedExpr { name, value } => {
            bind_named_expr_targets(signatures, parents, concrete, deferred, env, value)?;
            // Issue #771 (D-199), mirrored here for the walrus operator: a
            // `:=` target is unconditionally (re)bound wherever it is
            // evaluated, so any earlier `def` shadow, maybe-bound marker, or
            // stale opaque marker for `name` is cleared regardless of what
            // the new value's term turns out to be below.
            env.defs_rebound.remove(name.as_str());
            env.maybe_bindings.remove(name.as_str());
            env.opaque_bindings.remove(name.as_str());
            let term =
                collect_expr_constraints(signatures, parents, concrete, deferred, env, value)?;
            // Round-11 review finding 2: a walrus rebinds its target, so it
            // drops any artifact-owned buffer provenance the name carried,
            // exactly as the `Assign` arm does and after the value is
            // collected for the same reason.
            env.rebind_over_owned_buffer(name);
            if let Some(term) = term {
                env.bindings.entry(name.clone()).or_insert(term);
            } else {
                // The solver produced no term for the walrus value (e.g. a
                // class-target `cast`, `.pop()`, an attribute read, a method
                // call, or a heterogeneous container literal). Track `name`
                // as opaquely-but-definitely bound so a later read reports
                // "no term" instead of misreporting it as unbound.
                env.opaque_bindings.insert(name.clone());
            }
            Ok(())
        }
        HirExpr::IntLiteral(_)
        | HirExpr::FloatLiteral(_)
        | HirExpr::BoolLiteral(_)
        | HirExpr::OpaqueBool
        | HirExpr::StringLiteral(_)
        | HirExpr::EmptyList(_)
        | HirExpr::EmptySet(_)
        | HirExpr::EmptyDict(_)
        | HirExpr::NoneLiteral
        | HirExpr::NotImplemented
        | HirExpr::Name(_)
        | HirExpr::Super => Ok(()),
        HirExpr::ListPop { list } => {
            if let Some(receiver) = list.attr_expr() {
                bind_named_expr_targets(signatures, parents, concrete, deferred, env, receiver)?;
            }
            Ok(())
        }
        // #1254 (D-250): lowering refuses a walrus inside a comprehension.
        HirExpr::Comprehension(_) => Ok(()),
        HirExpr::Nd(_) => {
            for sub in pycc_hir::ndarray::expr_children(expr) {
                bind_named_expr_targets(signatures, parents, concrete, deferred, env, sub)?;
            }
            Ok(())
        }
        HirExpr::Call { args, .. } => {
            for arg in args {
                bind_named_expr_targets(signatures, parents, concrete, deferred, env, arg)?;
            }
            Ok(())
        }
        HirExpr::BinOp { left, right, .. }
        | HirExpr::AugBinOp { left, right, .. }
        | HirExpr::Compare { left, right, .. }
        | HirExpr::BoolOp { left, right, .. } => {
            bind_named_expr_targets(signatures, parents, concrete, deferred, env, left)?;
            bind_named_expr_targets(signatures, parents, concrete, deferred, env, right)
        }
        HirExpr::IfExp { test, body, orelse } => {
            for part in [test, body, orelse] {
                bind_named_expr_targets(signatures, parents, concrete, deferred, env, part)?;
            }
            Ok(())
        }
        HirExpr::CompareChain { first, links } => {
            for operand in pycc_hir::compare_chain_operands(first, links) {
                bind_named_expr_targets(signatures, parents, concrete, deferred, env, operand)?;
            }
            Ok(())
        }
        HirExpr::UnaryOp { operand, .. } | HirExpr::Unpack { value: operand, .. } => {
            bind_named_expr_targets(signatures, parents, concrete, deferred, env, operand)
        }
        HirExpr::FString(parts) => {
            for part in parts {
                if let FStringPart::Interpolation(inner) = part {
                    bind_named_expr_targets(signatures, parents, concrete, deferred, env, inner)?;
                }
            }
            Ok(())
        }
        HirExpr::ListLiteral(es)
        | HirExpr::ObjectList(es)
        | HirExpr::SetLiteral(es)
        | HirExpr::TupleLiteral(es) => {
            for e in es {
                bind_named_expr_targets(signatures, parents, concrete, deferred, env, e)?;
            }
            Ok(())
        }
        HirExpr::Subscript { base, index } => {
            bind_named_expr_targets(signatures, parents, concrete, deferred, env, base)?;
            bind_named_expr_targets(signatures, parents, concrete, deferred, env, index)
        }
        HirExpr::Slice {
            base,
            start,
            stop,
            step,
        } => {
            bind_named_expr_targets(signatures, parents, concrete, deferred, env, base)?;
            for bound in [start, stop, step].into_iter().flatten() {
                bind_named_expr_targets(signatures, parents, concrete, deferred, env, bound)?;
            }
            Ok(())
        }
        HirExpr::ListAppend { list, value } => {
            for sub in list.attr_expr().into_iter().chain([value.as_ref()]) {
                bind_named_expr_targets(signatures, parents, concrete, deferred, env, sub)?;
            }
            Ok(())
        }
        HirExpr::SetAdd { value, .. } => {
            bind_named_expr_targets(signatures, parents, concrete, deferred, env, value)
        }
        HirExpr::DictLiteral(pairs) => {
            for (k, v) in pairs {
                bind_named_expr_targets(signatures, parents, concrete, deferred, env, k)?;
                bind_named_expr_targets(signatures, parents, concrete, deferred, env, v)?;
            }
            Ok(())
        }
        HirExpr::DictGetOrDefault { dict, key, default } => {
            for sub in dict
                .attr_expr()
                .into_iter()
                .chain([key.as_ref(), default.as_ref()])
            {
                bind_named_expr_targets(signatures, parents, concrete, deferred, env, sub)?;
            }
            Ok(())
        }
        HirExpr::AttrGet { base, .. } => {
            bind_named_expr_targets(signatures, parents, concrete, deferred, env, base)
        }
        HirExpr::MethodCall { base, args, .. } => {
            bind_named_expr_targets(signatures, parents, concrete, deferred, env, base)?;
            for arg in args {
                bind_named_expr_targets(signatures, parents, concrete, deferred, env, arg)?;
            }
            Ok(())
        }
        HirExpr::ReceiverDispatchedCall { call, .. } => {
            bind_named_expr_targets(signatures, parents, concrete, deferred, env, call)
        }
        HirExpr::KeywordCall { call, keywords, .. } => {
            bind_named_expr_targets(signatures, parents, concrete, deferred, env, call)?;
            for (_, value) in keywords {
                bind_named_expr_targets(signatures, parents, concrete, deferred, env, value)?;
            }
            Ok(())
        }
        // #1411: `type(self)(args)` walks its arguments the same way.
        HirExpr::GenericClassInstantiate { args, .. } | HirExpr::ReceiverClassCall { args } => {
            for arg in args {
                bind_named_expr_targets(signatures, parents, concrete, deferred, env, arg)?;
            }
            Ok(())
        }
        HirExpr::ExprCall { callee, args } => {
            bind_named_expr_targets(signatures, parents, concrete, deferred, env, callee)?;
            for arg in args {
                bind_named_expr_targets(signatures, parents, concrete, deferred, env, arg)?;
            }
            Ok(())
        }
    }
}

/// Binds a comprehension's synthesized loop variable (PR-12 Task 3, D-117)
/// in the constraint solver's environment, mirroring `collect_block_constraints`'s
/// own `ForRange`/`ForList` arms exactly rather than duplicating their logic
/// a third time: a `CompIter::Range` iterable gives the loop variable the
/// concrete `Ty::Int` fact (unifying against any existing term, exactly like
/// `ForRange`'s own loop variable), while a `CompIter::Name` iterable gives
/// it a fresh, unconstrained term (exactly like `ForList`'s own loop
/// variable) since this solver doesn't track a list/dict/set-typed name's
/// element type at all. Shared by all three comprehension statement kinds.
pub(crate) fn bind_comp_loop_var(
    signatures: &HashMap<String, SignatureTerms>,
    parents: &mut Vec<usize>,
    concrete: &mut Vec<Option<Ty>>,
    deferred: &mut DeferredConstraints,
    env: &mut ConstraintEnvironment<'_, '_>,
    var: &str,
    iter: &CompIter,
) -> Result<(), Diagnostic> {
    match iter {
        CompIter::Range { start, stop, step } => {
            for (position, expr) in [("start", start), ("stop", stop), ("step", step)] {
                if let Some(term @ Err(_)) =
                    collect_expr_constraints(signatures, parents, concrete, deferred, env, expr)?
                {
                    unify_terms(
                        term,
                        Ok(Ty::Int),
                        parents,
                        concrete,
                        "T0021",
                        &format!("comprehension range {position}"),
                    )?;
                }
            }
            if let Some(existing) = env.bindings.get(var).cloned() {
                unify_terms(
                    existing,
                    Ok(Ty::Int),
                    parents,
                    concrete,
                    "T0023",
                    &format!("assignment to comprehension loop variable `{var}`"),
                )?;
            } else {
                env.bindings.insert(var.to_string(), Ok(Ty::Int));
            }
        }
        CompIter::Name(name) => {
            let known = env
                .bindings
                .get(name)
                .cloned()
                .and_then(|t| resolved_term(t, parents, concrete))
                .and_then(|ty| match ty {
                    Ty::List(elem) | Ty::Set(elem) | Ty::FrozenSet(elem) => Some(*elem),
                    Ty::Dict(kv) => Some(kv.0),
                    Ty::Str => Some(Ty::Str),
                    // Non-strict mode: iterated through CPython.
                    Ty::Object if crate::non_strict::is_non_strict() => Some(Ty::Object),
                    _ => None,
                });
            match known {
                Some(elem) if !env.bindings.contains_key(var) => {
                    env.bindings.insert(var.to_string(), Ok(elem));
                }
                _ => {
                    if !env.bindings.contains_key(var) {
                        let term = fresh_term(parents, concrete);
                        env.bindings.insert(var.to_string(), term);
                    }
                }
            }
        }
        // Part 1 of #1255: the iterable's own constraints are collected
        // against the enclosing bindings; its loop variable is a CPython
        // object, which the check phase verifies.
        CompIter::Iterable(iterable) => {
            let term =
                collect_expr_constraints(signatures, parents, concrete, deferred, env, iterable)?;
            let elem = match term.and_then(|t| resolved_term(t, parents, concrete)) {
                Some(Ty::List(elem)) => Ok(*elem),
                Some(Ty::Str) => Ok(Ty::Str),
                Some(Ty::Object) => Ok(Ty::Object),
                None => fresh_term(parents, concrete),
                Some(_) => fresh_term(parents, concrete),
            };
            env.bindings.insert(var.to_string(), elem);
        }
    }
    Ok(())
}

/// Binds a comprehension statement's `target` (`name = <comp>`) exactly as
/// the `Assign` arm binds a plain assignment's target, with the produced
/// container type as its term (#1254). Before this, the solver bound no term
/// for `target` at all, so any later read of it in a module that also holds
/// an unannotated private helper (the only case the solver runs for) failed
/// with a spurious `T0021` "not bound before this use". The container term
/// is the one the comprehension's own arm produced: see
/// [`crate::comprehension::comp_container_ty`] for the element gate the
/// check phase applies and [`set_comp::set_comp_container`] for a set
/// comprehension's term.
pub(crate) fn bind_comp_target(
    env: &mut ConstraintEnvironment<'_, '_>,
    target: &str,
    container: TypeTerm,
) {
    env.defs_rebound.remove(target);
    env.maybe_bindings.remove(target);
    env.opaque_bindings.remove(target);
    env.rebind_over_owned_buffer(target);
    env.bindings.entry(target.to_string()).or_insert(container);
}
