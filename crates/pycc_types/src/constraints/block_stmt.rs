//! One statement's constraint collection, the body of
//! [`super::collect_block_constraints`]'s loop: split out of `block.rs`
//! so each statement's diagnostic can be placed at that statement (#10).

use super::*;

#[allow(clippy::too_many_lines)]
pub(super) fn collect_stmt_constraints(
    signatures: &HashMap<String, SignatureTerms>,
    parents: &mut Vec<usize>,
    concrete: &mut Vec<Option<Ty>>,
    constraints: &mut SolverConstraints,
    env: &mut ConstraintEnvironment<'_, '_>,
    stmt: &HirStmt,
    return_term: Option<TypeTerm>,
) -> Result<(), Diagnostic> {
    match stmt {
        HirStmt::Assign { target, value } => {
            // A value assignment re-shadows any earlier same-named `def`
            // (D-110), independent of the first-term-wins rule below.
            env.defs_rebound.remove(target.as_str());
            // Issue #359 (Part 2 of #118): an unconditional assignment
            // upgrades a maybe-bound name back to definitely bound
            // (mirrors the validation pass's contract: `if c: x = 1`
            // followed by `x = 2` makes `x` readable).
            env.maybe_bindings.remove(target.as_str());
            // Issue #771: this is a fresh unconditional (re)assignment,
            // so any previous opaque marker for `target` is stale
            // regardless of what the new initializer's term turns out
            // to be — it is either replaced by a real term below, or
            // reinstated as opaque by the `else` arm. It is dropped only
            // once the initializer is collected, which may still read
            // the old binding (`a = a | b`).
            // Part 2a of #1142 (#1165): the solver's one admitting seam
            // for the buffer producer, mirroring `crate::check_stmt`'s.
            // The length argument is still collected so its own
            // constraints (and its own diagnostics) are not skipped.
            if let Some((callee, len_arg)) = resolved_producer_call(signatures, env, value) {
                // Guard first, matching `check_assignment`'s order: a
                // parameter rebinding is refused before the length
                // argument's own constraints are collected.
                reject_buffer_parameter_rebinding(env, target)?;
                reject_final_rebinding(env, target)?;
                // The collected term is *used*, not discarded: it is the
                // length type the check phase's own seam refuses when it
                // is not an `int`. See `reject_non_int_producer_length`,
                // and `resolved_producer_call` for why an admitted set
                // wider than the check phase's shows up as a wrong
                // message rather than a wrong program.
                let len_term = collect_expr_constraints(
                    signatures,
                    parents,
                    concrete,
                    &mut constraints.deferred,
                    env,
                    len_arg,
                )?;
                reject_non_int_producer_length(callee, len_term, parents, concrete)?;
                env.opaque_bindings.remove(target.as_str());
                env.bindings
                    .entry(target.clone())
                    .or_insert(Ok(Ty::MemoryView));
                env.owned_buffers.insert(target.clone());
                return Ok(());
            }
            let term = collect_expr_constraints(
                signatures,
                parents,
                concrete,
                &mut constraints.deferred,
                env,
                value,
            )?;
            // Round-11 review finding 2: the initializer is collected
            // first and the stale provenance dropped only afterwards,
            // because the initializer may still read the buffer the
            // name is about to stop denoting (`a = a[0]`).
            env.rebind_over_owned_buffer(target);
            env.opaque_bindings.remove(target.as_str());
            // #1476: a first binding from a bare narrowed read keeps the
            // `object` term (`object_narrow::is_bare_narrowed_read`).
            let term = if object_narrow::is_bare_narrowed_read(env, value) {
                Some(Ok(Ty::Object))
            } else {
                term
            };
            if let Some(term) = term {
                env.bindings.entry(target.clone()).or_insert(term);
            } else {
                // The target is unconditionally assigned, but the
                // solver produced no term for the initializer (e.g. a
                // class-target `cast`, `.pop()`, an attribute read, a
                // method call, or a heterogeneous container literal).
                // Track it as opaquely-but-definitely bound so a later
                // read reports "no term" instead of the misleading
                // "not bound before this use".
                env.opaque_bindings.insert(target.clone());
            }
        }
        HirStmt::AnnAssign {
            target,
            value: Some(value),
            annotation,
            is_final,
        } => {
            // Part 4 of #1026 (PR 4c of #1083) deliberately adds **no
            // branch here**, unlike PRs 4a and 4b, whose `float`/`bool`/
            // `int`/`str` arms this solver mirrors above. The relaxation
            // is in `check_stmt`'s own `AnnAssign` arm only, and two
            // independent facts keep this arm correct without it --
            // both re-verified against this tree:
            //
            // 1. A foreign name's term is `Ok(Ty::Object)`, never
            //    `Err(var)`, so the `AnnotationDefaultConstraint` pushed
            //    below is skipped by `apply_annotation_defaults`' very
            //    first statement (`let Err(var) = ... else { continue }`).
            // 2. Even for an `Err(var)` initializer, the annotation a
            //    4c assignment carries is a `Ty::Tuple`, and
            //    `is_private_solver_scalar` is `Int | Float | Bool |
            //    Str | None` -- so the same loop's second guard skips it
            //    anyway. That guard is not incidental: private-helper
            //    inference is scalar-only on purpose, so an annotated
            //    local cannot leak a container type into an otherwise
            //    unresolved signature.
            //
            // A third fact makes the question moot in practice: this
            // solver runs only over unannotated private helpers (#142),
            // i.e. inside a function body, where reading a foreign name
            // is `I0404` before any of this is reached.
            //
            // Issue #359 (Part 2 of #118): an unconditional annotated
            // assignment upgrades a maybe-bound name back to definitely
            // bound, same as a plain `Assign`.
            env.maybe_bindings.remove(target.as_str());
            // Issue #771: this arm always binds `target` into
            // `env.bindings` below regardless of the initializer term
            // (see the comment further down), so `target` is never
            // opaque by the time this arm finishes. Clear any stale
            // opaque marker for hygiene, mirroring the `maybe_bindings`
            // removal above; `HirExpr::Name`'s lookup already prefers
            // `bindings` over `opaque_bindings` either way.
            env.opaque_bindings.remove(target.as_str());
            // Part 2a of #1142 (#1165): see the plain `Assign` arm. No
            // `AnnotationDefaultConstraint` is pushed for a producer --
            // the term is already the concrete `Ty::MemoryView`, never
            // an `Err(var)`, so `apply_annotation_defaults` would skip
            // it anyway.
            if let Some((callee, len_arg)) = resolved_producer_call(signatures, env, value) {
                // See the plain `Assign` arm: guard before collecting.
                reject_buffer_parameter_rebinding(env, target)?;
                reject_final_rebinding(env, target)?;
                let len_term = collect_expr_constraints(
                    signatures,
                    parents,
                    concrete,
                    &mut constraints.deferred,
                    env,
                    len_arg,
                )?;
                reject_non_int_producer_length(callee, len_term, parents, concrete)?;
                // The one condition this arm adds over the plain
                // `Assign` arm, mirroring the check phase's own extra
                // step: the declared annotation must admit a buffer.
                reject_producer_annotation_mismatch(target, annotation)?;
                env.bindings
                    .entry(target.clone())
                    .or_insert(Ok(Ty::MemoryView));
                env.owned_buffers.insert(target.clone());
                // Recorded *after* the binding, exactly where
                // `check_stmt_in_function` records it, and that order is
                // the rule rather than a detail: a declaration's own
                // first assignment is not a reassignment, so recording
                // before `reject_final_rebinding` above would refuse the
                // declaration itself.
                //
                // Recorded here only, and not on this arm's ordinary
                // (non-producer) path, because this is the only binding
                // whose later reassignment the solver can *mask*: a
                // `Final` name the solver does not bind to
                // `Ty::MemoryView` records no artifact-owned provenance,
                // so no later use of it produces a solver diagnostic to
                // displace the check phase's `T0045` with. A recording
                // that changes no outcome is a guard no test can kill.
                if *is_final {
                    env.finals.insert(target.clone());
                }
                return Ok(());
            }
            if let Some(term) = collect_expr_constraints(
                signatures,
                parents,
                concrete,
                &mut constraints.deferred,
                env,
                value,
            )? {
                // The annotation is a directional bound on the initializer,
                // not a symmetric equality. Defer it until every hard
                // call/operator constraint has been collected so later
                // `bool` evidence is not widened to `int` merely because
                // this body was visited first.
                constraints
                    .annotation_defaults
                    .push(AnnotationDefaultConstraint {
                        initializer: term,
                        annotation: annotation.clone(),
                    });
            }
            // Round-11 review finding 2: see the plain `Assign` arm --
            // collected first, invalidated after, bound below.
            env.rebind_over_owned_buffer(target);
            // A scalar target has the declared type even when the
            // collector cannot produce an initializer term. A non-scalar
            // target is still bound, but deliberately remains unresolved:
            // otherwise a hand-built HIR module could use that annotation
            // to materialize an inferred container signature through a
            // later `return target`. Keep the existing first-binding-wins
            // representation rule; final validation checks initializer
            // compatibility and re-declarations whenever signature
            // inference can otherwise complete.
            if !env.bindings.contains_key(target) {
                let target_term = if is_private_solver_scalar(annotation) {
                    Ok(annotation.clone())
                } else {
                    let var = fresh_variable(parents, concrete);
                    constraints.non_scalar_local_terms.push(var);
                    Err(var)
                };
                env.bindings.insert(target.clone(), target_term);
            }
        }
        // Deliberately out of scope for issue #245: `ConstraintEnvironment`
        // (unlike the checker's `Environment`) has no declared-but-unbound
        // side-table, and every entry in its `env.bindings` is treated as
        // resolved and readable. Registering the annotation here the same
        // way the `Some(value)` arm above does would make the name
        // readable in this solver, silently accepting `def _f():\n    x:
        // int\n    return x` (should still raise T0021, unbound) instead
        // of only `def _f():\n    x: int\n    x = 1\n    return x`. A
        // parallel declared-side-table for `ConstraintEnvironment` is a
        // separate, independently-testable follow-up if solver-scope
        // coverage of this gap is wanted later.
        // #1244: a `del` binds nothing to infer; the checker owns its
        // binding-state rule. A nested foreign import (#1291) needs no constraint: its names
        // are seeded as `Ty::Object` globals position-blind, like a
        // top-level foreign import's (`constraints/signatures.rs`).
        HirStmt::AnnAssign { value: None, .. }
        | HirStmt::Delete { .. }
        | HirStmt::ForeignImport { .. } => {}
        // Part 2c of #1371: a slice `del` binds nothing, but its base and
        // bounds are ordinary reads -- a call to an unannotated helper
        // in one still needs its call-site constraints. `pycc_hir`
        // refuses a walrus in them, so there is nothing to pre-bind.
        HirStmt::DeleteSlice {
            base,
            start,
            stop,
            step,
            ..
        } => {
            for operand in std::iter::once(&**base)
                .chain([start, stop, step].into_iter().flatten().map(|b| &**b))
            {
                collect_expr_constraints(
                    signatures,
                    parents,
                    concrete,
                    &mut constraints.deferred,
                    env,
                    operand,
                )?;
            }
        }
        // #1457: an attribute `del` reads its base, for the same reason.
        HirStmt::DeleteAttr { base, .. } => {
            collect_expr_constraints(
                signatures,
                parents,
                concrete,
                &mut constraints.deferred,
                env,
                base,
            )?;
        }
        HirStmt::DeleteItem { base, key, .. } => {
            for expr in [base, key] {
                collect_expr_constraints(
                    signatures,
                    parents,
                    concrete,
                    &mut constraints.deferred,
                    env,
                    expr,
                )?;
            }
        }
        // `yield value`: the value unifies with the generator's element
        // type, which the body walk carries as its return term.
        HirStmt::ExprStmt(HirExpr::Call { callee, args })
            if callee == pycc_hir::YIELD_MARKER && env.in_generator =>
        {
            let actual = collect_expr_constraints(
                signatures,
                parents,
                concrete,
                &mut constraints.deferred,
                env,
                &args[0],
            )?;
            if let (Some(element), Some(actual)) = (return_term.clone(), actual) {
                unify_terms(element, actual, parents, concrete, "T0022", "generator element type")?;
            }
        }
        HirStmt::ExprStmt(expr) => {
            // PEP 572 (#774), deep-review follow-up (round 4): bind
            // before unifying -- see `bind_named_expr_targets`'s own
            // doc comment for why this pre-pass is required.
            bind_named_expr_targets(
                signatures,
                parents,
                concrete,
                &mut constraints.deferred,
                env,
                expr,
            )?;
            collect_expr_constraints(
                signatures,
                parents,
                concrete,
                &mut constraints.deferred,
                env,
                expr,
            )?;
        }
        HirStmt::If { test, body, orelse } => {
            // PEP 572 (#774), deep-review follow-up (round 4): bind
            // before unifying -- the test always executes, so this
            // binding is unconditional relative to the branch join
            // below (mirroring `crate::collect_named_expr_bindings`'s
            // own `If` arm ordering rationale in the full checker).
            bind_named_expr_targets(
                signatures,
                parents,
                concrete,
                &mut constraints.deferred,
                env,
                test,
            )?;
            collect_expr_constraints(
                signatures,
                parents,
                concrete,
                &mut constraints.deferred,
                env,
                test,
            )?;
            // Issue #359 (Part 2 of #118): clone the environment for
            // each branch so bindings from one branch do not leak into
            // the other, then join them back — mirroring the validation
            // pass's `join_if_branches` (D-147). Names introduced by
            // only one branch are tracked as maybe-bound in
            // `maybe_bindings`; names introduced by both branches are
            // definitely bound.
            // Issue #771 join-site follow-up: include `opaque_bindings`
            // alongside `bindings` here. A name definitely-but-opaquely
            // bound before this construct must count as pre-existing
            // too -- otherwise a branch that reassigns it to a real,
            // solver-representable term looks "newly introduced" to the
            // join helper below, and when only one branch performs that
            // reassignment the name is misclassified as bound in both
            // branches (the other, untouched branch still carries the
            // opaque marker), unmasking a term that only reflects one
            // path as if it were unconditionally correct. Confirmed as a
            // real gap by the pinned local reviewer's second pass.
            let pre_existing: HashSet<String> = env
                .bindings
                .keys()
                .chain(env.opaque_bindings.iter())
                .cloned()
                .collect();
            // #1476: an `isinstance` guard narrows the branch it admits.
            let narrowing = object_narrow::branch_target(signatures, env, test);
            let mut body_env = env.clone();
            let mut orelse_env = env.clone();
            if let Some((name, inner, narrows_body)) = narrowing {
                let branch_env = if narrows_body {
                    &mut body_env
                } else {
                    &mut orelse_env
                };
                branch_env.narrowed.insert(name, inner);
            }
            collect_block_constraints(
                signatures,
                parents,
                concrete,
                constraints,
                &mut body_env,
                body,
                return_term.clone(),
            )?;
            collect_block_constraints(
                signatures,
                parents,
                concrete,
                constraints,
                &mut orelse_env,
                orelse,
                return_term.clone(),
            )?;
            solver::join_if_branches_solver(env, &body_env, &orelse_env, &pre_existing);
        }
        HirStmt::Break | HirStmt::Continue => {}
        // The loop, then its `else:` clause in the shape the check
        // phase gives it (`pycc_hir::loop_else_as_if` when a `break`
        // can skip it, the plain statements otherwise).
        HirStmt::LoopElse { loop_stmt, orelse } => {
            let mut sequence = vec![loop_stmt.as_ref().clone()];
            if pycc_hir::loop_body(loop_stmt).is_some_and(pycc_hir::body_breaks_loop) {
                sequence.push(pycc_hir::loop_else_as_if(orelse));
            } else {
                sequence.extend(orelse.iter().cloned());
            }
            collect_block_constraints(
                signatures,
                parents,
                concrete,
                constraints,
                env,
                &sequence,
                return_term.clone(),
            )?;
        }
        HirStmt::While { test, body } => {
            // PEP 572 (#774), deep-review follow-up (round 4): bind
            // before unifying -- see the `If` arm's own doc comment
            // just above for why this order is required.
            bind_named_expr_targets(
                signatures,
                parents,
                concrete,
                &mut constraints.deferred,
                env,
                test,
            )?;
            collect_expr_constraints(
                signatures,
                parents,
                concrete,
                &mut constraints.deferred,
                env,
                test,
            )?;
            // Issue #359 (Part 2 of #118): clone the environment for
            // the loop body so body-only bindings do not leak into the
            // post-loop environment as definitely bound. A `while` body
            // may execute zero times, so every body-only binding is
            // maybe-bound — mirroring the validation pass's
            // `join_loop_body` (D-147).
            // Issue #771 join-site follow-up: include `opaque_bindings`
            // alongside `bindings` here. A name definitely-but-opaquely
            // bound before this construct must count as pre-existing
            // too -- otherwise a branch that reassigns it to a real,
            // solver-representable term looks "newly introduced" to the
            // join helper below, and when only one branch performs that
            // reassignment the name is misclassified as bound in both
            // branches (the other, untouched branch still carries the
            // opaque marker), unmasking a term that only reflects one
            // path as if it were unconditionally correct. Confirmed as a
            // real gap by the pinned local reviewer's second pass.
            let pre_existing: HashSet<String> = env
                .bindings
                .keys()
                .chain(env.opaque_bindings.iter())
                .cloned()
                .collect();
            let mut body_env = env.clone();
            collect_block_constraints(
                signatures,
                parents,
                concrete,
                constraints,
                &mut body_env,
                body,
                return_term.clone(),
            )?;
            solver::join_loop_body_solver(env, &body_env, &pre_existing);
        }
        HirStmt::ForRange {
            var,
            start,
            stop,
            step,
            body,
        } => {
            for (position, expr) in [("start", start), ("stop", stop), ("step", step)] {
                if let Some(term @ Err(_)) = collect_expr_constraints(
                    signatures,
                    parents,
                    concrete,
                    &mut constraints.deferred,
                    env,
                    expr,
                )? {
                    unify_terms(
                        term,
                        Ok(Ty::Int),
                        parents,
                        concrete,
                        "T0021",
                        &format!("range {position}"),
                    )?;
                }
            }
            // Issue #359 (Part 2 of #118): snapshot the pre-loop
            // binding names so the loop variable and body-only
            // bindings can be tracked as maybe-bound after the loop
            // (a `for` loop may execute zero times).
            // Issue #771 join-site follow-up: include `opaque_bindings`
            // alongside `bindings` here. A name definitely-but-opaquely
            // bound before this construct must count as pre-existing
            // too -- otherwise a branch that reassigns it to a real,
            // solver-representable term looks "newly introduced" to the
            // join helper below, and when only one branch performs that
            // reassignment the name is misclassified as bound in both
            // branches (the other, untouched branch still carries the
            // opaque marker), unmasking a term that only reflects one
            // path as if it were unconditionally correct. Confirmed as a
            // real gap by the pinned local reviewer's second pass.
            let pre_existing: HashSet<String> = env
                .bindings
                .keys()
                .chain(env.opaque_bindings.iter())
                .cloned()
                .collect();
            if let Some(existing) = env.bindings.get(var).cloned() {
                unify_terms(
                    existing,
                    Ok(Ty::Int),
                    parents,
                    concrete,
                    "T0023",
                    &format!("assignment to for-loop target `{var}`"),
                )?;
            } else {
                env.bindings.insert(var.clone(), Ok(Ty::Int));
            }
            let mut body_env = env.clone();
            collect_block_constraints(
                signatures,
                parents,
                concrete,
                constraints,
                &mut body_env,
                body,
                return_term.clone(),
            )?;
            solver::join_loop_body_solver(env, &body_env, &pre_existing);
            // The loop variable itself is maybe-bound after the loop
            // if it was newly introduced (the loop may not execute).
            if !pre_existing.contains(var) {
                env.maybe_bindings.insert(var.clone());
            }
        }
        HirStmt::ForList { var, list, body } => {
            // Unlike `ForRange`, we have no concrete `Ty::Int` fact to
            // unify the loop variable against -- this solver doesn't
            // track a `list`-typed name's element type at all (see the
            // `HirExpr::ListLiteral`/`Subscript`/`ListAppend` arms
            // above). Give `var` a fresh, unconstrained term so a body
            // reference to it doesn't spuriously fail as "not bound"
            // (it *is* locally bound, just not solver-typed); real
            // element-type checking happens in the second, real check
            // pass (`check_with_signatures_all`).
            // Issue #359 (Part 2 of #118): snapshot the pre-loop binding
            // names so the loop variable and body-only bindings can be
            // tracked as maybe-bound after the loop (a `for` loop may
            // execute zero times).
            // Issue #771 join-site follow-up: include `opaque_bindings`
            // alongside `bindings` here. A name definitely-but-opaquely
            // bound before this construct must count as pre-existing
            // too -- otherwise a branch that reassigns it to a real,
            // solver-representable term looks "newly introduced" to the
            // join helper below, and when only one branch performs that
            // reassignment the name is misclassified as bound in both
            // branches (the other, untouched branch still carries the
            // opaque marker), unmasking a term that only reflects one
            // path as if it were unconditionally correct. Confirmed as a
            // real gap by the pinned local reviewer's second pass.
            let pre_existing: HashSet<String> = env
                .bindings
                .keys()
                .chain(env.opaque_bindings.iter())
                .cloned()
                .collect();
            // Round-11 review finding 2: the loop variable rebinds the
            // name on every iteration, so any artifact-owned buffer
            // provenance it carried is stale from here on.
            env.rebind_over_owned_buffer(var);
            if !env.bindings.contains_key(var) {
                let known = env
                    .bindings
                    .get(list)
                    .cloned()
                    .and_then(|t| resolved_term(t, parents, concrete))
                    .and_then(|ty| match ty {
                        Ty::List(elem) | Ty::Set(elem) | Ty::FrozenSet(elem) => Some(*elem),
                        Ty::Dict(kv) => Some(kv.0),
                        Ty::Str => Some(Ty::Str),
                        _ => None,
                    });
                // An iterable whose element this pass cannot type (a list
                // of function values): the target is opaque as well, so a
                // call of it is left to the check phase.
                if known.is_none() {
                    env.opaque_bindings.insert(var.clone());
                }
                let term = match known {
                    Some(elem) => Ok(elem),
                    None => fresh_term(parents, concrete),
                };
                env.bindings.insert(var.clone(), term);
            }
            let mut body_env = env.clone();
            collect_block_constraints(
                signatures,
                parents,
                concrete,
                constraints,
                &mut body_env,
                body,
                return_term.clone(),
            )?;
            solver::join_loop_body_solver(env, &body_env, &pre_existing);
            // The loop variable itself is maybe-bound after the loop
            // if it was newly introduced (the loop may not execute).
            if !pre_existing.contains(var) {
                env.maybe_bindings.insert(var.clone());
            }
        }
        // PR 3c of #1082: `for x in o.attr:` / `for x in o.method(...):`.
        // The iterable is a real expression, unlike `ForList`'s bare
        // name, so it is walked; the loop variable gets a fresh
        // unconstrained term for exactly the reason `ForList`'s does
        // (this solver has no `Ty::Object` fact to unify against).
        HirStmt::ForObject { var, iter, body } => {
            collect_expr_constraints(
                signatures,
                parents,
                concrete,
                &mut constraints.deferred,
                env,
                iter,
            )?;
            // Round-11 review finding 2: see the `ForList` arm.
            env.rebind_over_owned_buffer(var);
            if !env.bindings.contains_key(var) {
                let term = fresh_term(parents, concrete);
                env.bindings.insert(var.clone(), term);
            }
            collect_block_constraints(
                signatures,
                parents,
                concrete,
                constraints,
                env,
                body,
                return_term.clone(),
            )?;
        }
        // A generator's `return` carries no value (`pycc_hir` refuses one).
        HirStmt::Return(_) if env.in_generator => {}
        HirStmt::Return(value) => {
            let Some(return_term) = return_term.clone() else {
                return Ok(());
            };
            // Part 2b of #1142 (#1164), the solver half of
            // `crate::check_stmt_in_function`'s own egress interception.
            // It must run here, before `collect_expr_constraints` reaches
            // the `Name` seam above, for the ordering reason the `len`
            // and `Subscript` interceptions already record: that seam
            // calls `reject_memoryview_read` and would report the owned
            // refusal for the very expression this admits. The declared
            // return type is `return_term`'s `Ok` arm -- an *inferred*
            // return (an `Err(var)` inference variable standing in for an
            // unannotated helper) declines, so only a written
            // `-> memoryview` annotation admits an egress.
            //
            // The third conjunct is this walker's half of the
            // definite-assignment check `crate::check_stmt_in_function`'s
            // own comment enumerates (review round 3 of #1164). The two
            // walkers must agree on the admission predicate, and this is
            // the only spelling available here: `ConstraintEnvironment`
            // carries no three-way lattice, only the `maybe_bindings`
            // side-table, so the test is `!maybe` rather than
            // `== Definitely`. Like the check phase's conjunct it reads
            // the *result* of a join and so closes every join form at
            // once -- `join_if_branches_solver`, `join_loop_body_solver`
            // (the `while`, `for`-`range`, `for`-list, `match`-case and
            // `try` arms all route through it) and the `for` loop
            // variable's own insertion.
            //
            // It is deliberately **not independently observable**, and
            // stands for consistency rather than for a diagnostic of its
            // own: `collect_expr_constraints`'s `Name` arm tests
            // `maybe_bindings` before it reaches `bindings`, so a
            // maybe-bound operand answers `Ok(None)` and the fall-through
            // unifies nothing either way. The check phase's `T0041` is
            // the user-facing gate, exactly as D-147 places it. What the
            // conjunct buys is that the solver can never *widen* the
            // admission past the check phase's -- the subset property
            // `docs/TYPE_SYSTEM.md`'s `memoryview` row requires of this
            // mirror -- if either arm's `Name` ordering later changes.
            //
            // Review round 5's `returns_inside_finally` test is a refusal
            // rather than a decline, exactly as in the check phase: see
            // `crate::buffer::buffer_return_inside_finally`. Part 1 of
            // #1175 runs it once, after the provenance verdict, for both
            // provenances -- the same restructure the check phase made,
            // for the same reason.
            //
            // Part 1 of #1175's caller-owned provenance is spelled here
            // as `Some(Ok(Ty::MemoryView))`, which is
            // `reject_buffer_parameter_rebinding`'s own spelling and
            // exact for the same reason its doc comment gives: an
            // `Err(var)` term is only ever resolved by
            // `apply_annotation_defaults`, whose `is_private_solver_
            // scalar` guard admits `Int | Float | Bool | Str | None`
            // only, so no inferred term can become `MemoryView`. A
            // `memoryview` parameter's term is therefore always this
            // concrete one, and the solver's admission stays a subset of
            // the check phase's -- the property `docs/TYPE_SYSTEM.md`'s
            // `memoryview` row requires of this mirror. The shared
            // `admits_buffer_egress` is what makes that subset hold by
            // construction rather than by two parallel edits.
            if let Some(expr) = value
                && let Some((name, shape)) =
                    crate::buffer::admitted_buffer_return(expr, return_term.as_ref().ok())
                && crate::buffer::admits_buffer_egress(
                    env.owned_buffers.contains(name),
                    !env.maybe_bindings.contains(name),
                    matches!(env.bindings.get(name), Some(Ok(Ty::MemoryView))),
                    shape,
                )
            {
                // The post-admission block, kept line-for-line parallel
                // with the check phase's own (`crate::check_function`'s
                // `HirStmt::Return` arm) so a future divergence is
                // visible in review. See that block for why the `!owned`
                // gate is deliberately absent from both (#1179).
                if let crate::buffer::AdmittedBufferReturn::Slice {
                    start,
                    stop,
                    has_step,
                } = shape
                {
                    if has_step {
                        return Err(crate::buffer::buffer_slice_step_unsupported(name));
                    }
                    // D1: this branch `continue`s before
                    // `collect_expr_constraints` ever sees the operand,
                    // so the bounds are walked here -- both to keep
                    // propagating a genuine error such as an unbound
                    // local, exactly as this file's own `HirExpr::Slice`
                    // arm does, and to apply the `T0021` gate the
                    // admitted branch owes. The gate fires only on a
                    // term this solver actually pinned down, so the
                    // solver's admission stays a subset of the check
                    // phase's rather than refusing what the check phase
                    // would still infer.
                    for (label, bound) in [("start", start), ("stop", stop)] {
                        if let Some(bound) = bound {
                            let bound_term = collect_expr_constraints(
                                signatures,
                                parents,
                                concrete,
                                &mut constraints.deferred,
                                env,
                                bound,
                            )?;
                            if let Some(Ok(bound_ty)) = bound_term
                                && !crate::is_assignable(bound_ty.clone(), Ty::Int)
                            {
                                return Err(crate::buffer::buffer_slice_bound_not_an_int(
                                    label, &bound_ty,
                                ));
                            }
                        }
                    }
                }
                if env.returns_inside_finally {
                    return Err(crate::buffer::buffer_return_inside_finally(name));
                }
                return Ok(());
            }
            // #1387: a bare `return` / `return None` into a declared
            // `object` slot, the check phase's own admission through the
            // one shared predicate (`crate::object_none`). An inferred
            // return (`Err(var)`) declines, as the buffer egress does.
            if let Ok(declared) = &return_term
                && crate::object_none::admits_none_return(declared, value.as_ref())
            {
                return Ok(());
            }
            let actual = match value {
                Some(expr) => collect_expr_constraints(
                    signatures,
                    parents,
                    concrete,
                    &mut constraints.deferred,
                    env,
                    expr,
                )?,
                None => Some(Ok(Ty::None)),
            };
            // #1475: a concrete value returned into a declared `object`
            // slot is a boxing seam, not a unification. The solver has no
            // class environment to tell a boxable value from a refused
            // one (a container, an `Optional`), so it defers to the check
            // phase's `crate::object_box::admits_value`, which decides
            // every such return.
            if let (Ok(Ty::Object), Some(Ok(actual_ty))) = (&return_term, &actual)
                && *actual_ty != Ty::Object
            {
                return Ok(());
            }
            // Non-strict mode (#3): a still-unresolved value returned into
            // a declared `object` slot is the same boxing seam (unifying
            // would pin `return a + 1`'s `int` to `object`), and an object
            // returned into a scalar slot is the run-time conversion.
            if matches!((&return_term, &actual), (Ok(Ty::Object), Some(Err(_))))
                || matches!(
                    (&return_term, &actual),
                    (Ok(declared), Some(Ok(actual_ty)))
                        if crate::non_strict::admits_coercion(actual_ty, declared)
                )
            {
                return Ok(());
            }
            if let Some(actual) = actual {
                // #949: this solver runs over *every* module-level
                // function, not only D-038 private helpers, so the context
                // string cannot be "private helper ..." unconditionally --
                // that misdescribed every annotated (and every public)
                // function whose body returned the wrong type. The real
                // discriminator is the return term itself: `Ok(ty)` is a
                // written annotation, `Err(var)` an inference variable
                // standing in for an unannotated helper's return type.
                // Only the latter is genuinely an "inferred" return.
                //
                // The declared case therefore gets both a different
                // `context` -- which `unify_terms` interpolates into its
                // `T0042` "cannot be inferred through a PEP 695 generic
                // function's own type parameter" message, the same misnomer
                // one message over -- and, via `declared`, a
                // declared-vs-actual `T0022` message with a `help`
                // suggestion. The inferred case keeps the original wording
                // verbatim so `_helper` diagnostics are unchanged.
                let declared = return_term.as_ref().ok().cloned();
                let context = if declared.is_some() {
                    "declared return type"
                } else {
                    "private helper return type"
                };
                unify_terms_with_declared(
                    return_term,
                    actual,
                    parents,
                    concrete,
                    "T0022",
                    context,
                    declared.as_ref(),
                )?;
            }
        }
        // PR-11 Task 3 (D-123): `dict`'s own type isn't tracked by this
        // solver either (same reasoning as `ForList`'s `list` field
        // above) -- recurse into `key`/`value` only to keep propagating
        // genuine errors; real item-assignment type-checking is
        // `check_stmt`/`check_stmt_in_function`'s job.
        HirStmt::DictSet {
            dict: _,
            key,
            value,
        } => {
            collect_expr_constraints(
                signatures,
                parents,
                concrete,
                &mut constraints.deferred,
                env,
                key,
            )?;
            collect_expr_constraints(
                signatures,
                parents,
                concrete,
                &mut constraints.deferred,
                env,
                value,
            )?;
        }
        // D-154 (Part 1 of #375): `base`'s own type isn't tracked by
        // this solver either (same reasoning as `DictSet` above) --
        // recurse into `base`/`value` only to keep propagating genuine
        // errors; real attribute-slot type-checking is
        // `check_stmt`/`check_stmt_in_function`'s job.
        HirStmt::AttrSet { base, value, .. } => {
            collect_expr_constraints(
                signatures,
                parents,
                concrete,
                &mut constraints.deferred,
                env,
                base,
            )?;
            collect_expr_constraints(
                signatures,
                parents,
                concrete,
                &mut constraints.deferred,
                env,
                value,
            )?;
        }
        // PR-12 Task 3 (D-117): no unification term is registered for
        // `target` -- per D-116's own correction note ("a
        // container-literal assignment's target never receives a solver
        // binding at all... confirmed empirically for all four container
        // types"), a comprehension's own `target` gets that same
        // treatment, matching `ListLiteral`/`DictLiteral`/`SetLiteral`
        // above. Unlike a bare no-op, though, this arm still binds the
        // loop variable (mirroring `ForRange`/`ForList` above, via the
        // shared `bind_comp_loop_var` helper) and recurses into
        // `cond`/`elt` (or `key`/`value`) to keep propagating genuine
        // errors and, critically, to let a call inside a comprehension's
        // own sub-expressions still participate in this solver's
        // argument<->parameter unification (self-review finding: an
        // earlier no-op-only version of this arm made an unannotated
        // private-helper parameter used only inside a comprehension's
        // `elt` spuriously fail to infer with "cannot infer type of
        // parameter ...; add an annotation", since the call inside `elt`
        // was never visited at all -- see
        // `private_helper_parameter_is_inferred_through_a_comprehension_s_elt`).
        HirStmt::ListCompAssign {
            target,
            var,
            iter,
            cond,
            elt,
        }
        | HirStmt::SetCompAssign {
            target,
            var,
            iter,
            cond,
            elt,
        } => {
            bind_comp_loop_var(
                signatures,
                parents,
                concrete,
                &mut constraints.deferred,
                env,
                var,
                iter,
            )?;
            if let Some(cond) = cond {
                collect_expr_constraints(
                    signatures,
                    parents,
                    concrete,
                    &mut constraints.deferred,
                    env,
                    cond,
                )?;
            }
            let elt_term = collect_expr_constraints(
                signatures,
                parents,
                concrete,
                &mut constraints.deferred,
                env,
                elt,
            )?;
            // Round-11 review finding 2: this solver binds no term for a
            // comprehension `target` at all, so before this call the
            // name kept whatever it was bound to before -- including an
            // artifact-owned `Ty::MemoryView`, which a later read then
            // refused with the owned-buffer `C0001` instead of letting
            // the check phase report its `T0023` for the reassignment.
            // The comprehension's own sub-expressions are collected
            // first, since they may still read that buffer.
            //
            // `var` needs no such call: it is the D-117 synthesized
            // internal loop name (see `HirStmt::ListCompAssign`'s own
            // doc comment), which cannot collide with a source name.
            //
            // #1343: a set comprehension's container is the same
            // defaulted term as the expression form's.
            let container = match stmt {
                HirStmt::SetCompAssign { .. } => set_comp::set_comp_container(
                    elt_term,
                    parents,
                    concrete,
                    &mut constraints.deferred,
                ),
                _ => set_comp::list_comp_container(
                    elt_term,
                    parents,
                    concrete,
                    &mut constraints.deferred,
                ),
            };
            bind_comp_target(env, target, container);
        }
        HirStmt::DictCompAssign {
            target,
            var,
            iter,
            cond,
            key,
            value,
        } => {
            bind_comp_loop_var(
                signatures,
                parents,
                concrete,
                &mut constraints.deferred,
                env,
                var,
                iter,
            )?;
            if let Some(cond) = cond {
                collect_expr_constraints(
                    signatures,
                    parents,
                    concrete,
                    &mut constraints.deferred,
                    env,
                    cond,
                )?;
            }
            let key_term = collect_expr_constraints(
                signatures,
                parents,
                concrete,
                &mut constraints.deferred,
                env,
                key,
            )?;
            let value_term = collect_expr_constraints(
                signatures,
                parents,
                concrete,
                &mut constraints.deferred,
                env,
                value,
            )?;
            let container = set_comp::dict_comp_container(
                key_term,
                value_term,
                parents,
                concrete,
                &mut constraints.deferred,
            );
            // Round-11 review finding 2: see the list/set arm above.
            bind_comp_target(env, target, container);
        }
        HirStmt::Match { subject, cases } => {
            collect_expr_constraints(
                signatures,
                parents,
                concrete,
                &mut constraints.deferred,
                env,
                subject,
            )?;
            for case in cases {
                // Issue #771 join-site follow-up: include `opaque_bindings`
                // alongside `bindings` here. A name definitely-but-opaquely
                // bound before this construct must count as pre-existing
                // too -- otherwise a branch that reassigns it to a real,
                // solver-representable term looks "newly introduced" to the
                // join helper below, and when only one branch performs that
                // reassignment the name is misclassified as bound in both
                // branches (the other, untouched branch still carries the
                // opaque marker), unmasking a term that only reflects one
                // path as if it were unconditionally correct. Confirmed as a
                // real gap by the pinned local reviewer's second pass.
                let pre_existing: HashSet<String> = env
                    .bindings
                    .keys()
                    .chain(env.opaque_bindings.iter())
                    .cloned()
                    .collect();
                let mut case_env = env.clone();
                // Round-11 review finding 2: a `case` pattern's capture
                // names rebind inside the case body. This solver binds
                // no term for them (a capture's type is the subject's,
                // which the check phase resolves), so without this the
                // name kept its artifact-owned `Ty::MemoryView` binding
                // and a read of it -- in the body or, through the join
                // below, after the `match` -- raised the owned-buffer
                // `C0001` over the check phase's `T0023`. Dropped in the
                // case environment rather than in `env` so the join
                // helper carries it out only for the paths that rebind.
                let mut captures = Vec::new();
                crate::collect_pattern_capture_names(&case.pattern, &mut captures);
                for capture in captures {
                    case_env.rebind_over_owned_buffer(capture);
                }
                collect_block_constraints(
                    signatures,
                    parents,
                    concrete,
                    constraints,
                    &mut case_env,
                    &case.body,
                    return_term.clone(),
                )?;
                solver::join_loop_body_solver(env, &case_env, &pre_existing);
            }
        }
        HirStmt::Try {
            body,
            handlers,
            orelse,
            finalbody,
        }
        | HirStmt::TryStar {
            body,
            handlers,
            orelse,
            finalbody,
        } => try_stmt::collect_try_constraints(
            signatures,
            parents,
            concrete,
            constraints,
            env,
            try_stmt::TryShape {
                body,
                handlers,
                orelse,
                finalbody,
                star: matches!(stmt, HirStmt::TryStar { .. }),
            },
            return_term.clone(),
        )?,
        HirStmt::Raise { exc, cause } => {
            // #382 (PR-22 Part 1): A raise expression that is a direct
            // call to a builtin exception class (e.g.
            // `ValueError("msg")`) is classified as C0001 by
            // `collect_expr_constraints`: that collector resolves a
            // callee against the `signatures` map it was handed, which
            // holds only `HirItem::Function` entries, so a class name
            // misses and falls through to the known-callable-builtin
            // arm. Part 1 of #541 seeded the seven builtin exception
            // names into `Environment::classes`, but that table is not
            // what this collector consults, so the classification is
            // unchanged. The actual validation is done by
            // `check_raise_stmt` in the check pass, so errors from
            // constraint collection for raise operands are deliberately
            // ignored here — they would otherwise prevent the solver
            // path from reaching `check_with_signatures_all`, where the
            // real check succeeds.
            if let Some(exc_expr) = exc {
                let _ = collect_expr_constraints(
                    signatures,
                    parents,
                    concrete,
                    &mut constraints.deferred,
                    env,
                    exc_expr,
                );
            }
            if let Some(cause_expr) = cause {
                let _ = collect_expr_constraints(
                    signatures,
                    parents,
                    concrete,
                    &mut constraints.deferred,
                    env,
                    cause_expr,
                );
            }
        }
    }
    Ok(())
}
