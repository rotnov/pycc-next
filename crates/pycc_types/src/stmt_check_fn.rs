//! Function-scope statement checking (`check_function`).

use super::*;

/// Checks `function` in complete isolation from any module -- no sibling
/// function signatures, no module-level globals, and (D-154, Part 1 of
/// #375) no class table either, since this entry point takes a bare
/// `&HirItem` with no enclosing `&HirModule` to source `class_defs` from.
/// A body that calls a sibling function or reads a module global already
/// couldn't check here before D-154 existed; a body that instantiates a
/// class, or reads/writes/calls a method on an instance, is the same kind
/// of gap, not a new one -- `Environment::classes` is simply empty.
/// Production compilation never reaches this function: `check`/
/// `check_and_resolve` always build their `Environment` from a real
/// `HirModule` (via `class::bind_classes`), so this isolation only affects
/// a caller that deliberately checks one function outside any module
/// context -- this crate's own unit tests, and the workspace's own
/// direct-API integration tests (e.g. `tests/slice0.rs`), today.
pub fn check_function(function: &HirItem) -> Result<(), Diagnostic> {
    let local_names = match function {
        HirItem::Function { params, body, .. } => function_local_names(params, body),
        HirItem::TopLevelStmt(_) => Vec::new(),
    };
    check_function_in(&Environment::new(), function, &local_names)
}

/// Checks one function's body, resolving sibling calls and module-level
/// global reads against a clone of `module_env` (see D-040/D-041/D-055) instead
/// of an isolated, self-only scope. Lexically local binding targets are removed
/// from that clone before the body is checked. The clone owns independent value
/// bindings while sharing the immutable function registry through copy-on-write
/// storage, so a function's parameters and local assignments never leak back
/// into the module scope or into any other function's check.
pub(crate) fn check_function_in(
    module_env: &Environment,
    function: &HirItem,
    local_names: &[&str],
) -> Result<(), Diagnostic> {
    let HirItem::Function {
        name,
        params,
        return_ty,
        body,
    } = function
    else {
        panic!("check_function called with a non-Function HirItem");
    };
    let standalone_params;
    let (resolved_params, resolved_return, signature_was_registered) =
        if let Some((param_tys, return_ty)) = module_env.lookup_function(name) {
            (param_tys.as_slice(), return_ty.clone(), true)
        } else {
            standalone_params = params.iter().map(|(_, ty)| ty.clone()).collect::<Vec<_>>();
            (standalone_params.as_slice(), return_ty.clone(), false)
        };
    if resolved_params.contains(&Ty::Infer) || resolved_return == Ty::Infer {
        return Err(Diagnostic::error(
            "T0021",
            format!("cannot check private helper `{name}` before its signature is inferred"),
            Span::new(0, 0),
        ));
    }
    let mut env = module_env.child_for_function(local_names);
    // PEP 695 (#387): record the function's own type-parameter name (if any)
    // so `check_stmt_in_function`'s `Return` arm can reject a generic
    // function returning its own `Ty::Param` as a concrete scalar. Errors
    // from `generic_type_param_name` (multiple params, container-position
    // occurrence) are suppressed here with `.ok()`: a generic function
    // reaching `check_function_in` through `check_generic_function_in` has
    // already been validated by that function's own
    // `generic_type_param_name` call, and a non-generic function returns
    // `Ok(None)` unconditionally.
    // Part 2b of #1142 (#1164), review round 5: the single pending-return
    // record's precondition, computed once for the whole body. Read only by
    // the buffer-egress admission in `check_stmt_in_function`'s
    // `HirStmt::Return` arm; see `crate::buffer::buffer_return_inside_finally`.
    env.returns_inside_finally = pycc_hir::body_returns_inside_finally(body);
    env.return_inferred = *return_ty == Ty::Infer;
    env.in_classmethod = name.ends_with(".classmethod");
    env.own_type_param = generic_type_param_name(params, return_ty).ok().flatten();
    // #433: extract the class name from a mangled `<ClassName>.<method>`
    // name so `infer_expr_in`'s `HirExpr::Super` arm can resolve the next
    // class in the MRO. A top-level function name contains no `.`, so
    // `current_class` stays `None` for those (and for the module-level
    // environment, which never goes through `check_function_in`).
    //
    // #1337 (D-254): a receiver-exact copy of an inherited body is spelled
    // with its *receiver* prefix, but `super()` anchors at the class that
    // defines the body -- the copy's origin class. The receiver the
    // `super()` search follows is `self`'s type (`class::super_call`).
    env.current_class = match pycc_hir::inherited_copy_origin(name, &|n| module_env.classes.get(n))
    {
        Some(copy) => Some(copy.origin_class),
        None => name
            .split('.')
            .next()
            .filter(|prefix| *prefix != name)
            .map(String::from),
    };
    if !signature_was_registered {
        env.bind_function(
            name.clone(),
            resolved_params.to_vec(),
            resolved_return.clone(),
        );
    }
    for ((param_name, _), param_ty) in params.iter().zip(resolved_params.iter().cloned()) {
        env.bind(param_name.clone(), param_ty);
    }
    // #380 (PR-20): determine whether this function is an abstract method.
    // An abstract method has a declaration-style body (`...` or `pass`)
    // that is not lowered — its HIR body is just `Return(None)`. The
    // function name is mangled as `<ClassName>.<method>`; we check whether
    // the class lists it in `abstract_methods`. Skip body checking and
    // the return-contract check for abstract methods — the body is never
    // executed (a concrete subclass overrides it).
    let is_abstract_method = name
        .split('.')
        .next()
        .filter(|class_name| *class_name != name)
        .and_then(|class_name| module_env.classes.get(class_name))
        .is_some_and(|class_def| {
            let method_name = name.split('.').nth(1).unwrap_or("");
            class_def.abstract_methods.iter().any(|m| m == method_name)
        });
    if !is_abstract_method {
        narrow::check_stmt_sequence_in_function(
            &mut env,
            local_names,
            body,
            resolved_return.clone(),
        )?;
    }
    if !is_abstract_method
        && resolved_return != Ty::None
        && !matches!(resolved_return, Ty::Generator(_))
        && !block_always_returns(body)
    {
        let diagnostic = Diagnostic::error(
            "T0022",
            format!(
                "function `{name}` can exit without returning `{}`",
                resolved_return.name()
            ),
            Span::new(0, 0),
        )
        .with_help(format!("return a `{}` value", resolved_return.name()));
        return Err(crate::locate::place_labeled(
            diagnostic,
            crate::locate::returns_span(),
            format!("expected `{}` because of this return type", resolved_return.name()),
        ));
    }
    Ok(())
}

/// Issue #118 Part 1: fast-path helper for function-scope `if` statements
/// where neither branch introduces new bindings.
pub(crate) fn check_if_branches_in_place_in_function(
    env: &mut Environment,
    local_names: &[&str],
    body: &[HirStmt],
    orelse: &[HirStmt],
    return_ty: Ty,
) -> Result<(), Diagnostic> {
    // Warning fix (D-068 review of #780): see the module-scope
    // `check_if_branches_in_place`'s identical comment -- route through the
    // narrowing-aware sequence checker so a nested early-return guard's
    // narrowing propagates to later statements even on this fast path.
    narrow::check_stmt_sequence_in_function(env, local_names, body, return_ty.clone())?;
    narrow::check_stmt_sequence_in_function(env, local_names, orelse, return_ty)
}

/// Issue #118 Part 1: fast-path helper for function-scope `while` loops
/// where the body introduces no new bindings.
pub(crate) fn check_while_body_in_place_in_function(
    env: &mut Environment,
    local_names: &[&str],
    body: &[HirStmt],
    return_ty: Ty,
) -> Result<(), Diagnostic> {
    // Warning fix (D-068 review of #780): see
    // `check_if_branches_in_place_in_function`'s identical comment.
    narrow::check_stmt_sequence_in_function(env, local_names, body, return_ty)
}

pub(crate) fn check_stmt_in_function(
    env: &mut Environment,
    local_names: &[&str],
    stmt: &HirStmt,
    return_ty: Ty,
) -> Result<(), Diagnostic> {
    check_stmt_in_function_here(env, local_names, stmt, return_ty)
        .map_err(|diagnostic| crate::locate::place_stmt(diagnostic, stmt))
}

/// [`check_stmt_in_function`] before its diagnostic is placed at `stmt`.
fn check_stmt_in_function_here(
    env: &mut Environment,
    local_names: &[&str],
    stmt: &HirStmt,
    return_ty: Ty,
) -> Result<(), Diagnostic> {
    // See the module-scope `check_stmt`.
    match stmt {
        HirStmt::LoopElse { loop_stmt, orelse } => loop_flow::check_loop(
            env,
            loop_stmt,
            orelse,
            |env| check_stmt_arm_in_function(env, local_names, loop_stmt, return_ty.clone()),
            |env, suite| {
                narrow::check_stmt_sequence_in_function(env, local_names, suite, return_ty.clone())
            },
        ),
        _ if loop_body(stmt).is_some() => loop_flow::check_loop(
            env,
            stmt,
            &[],
            |env| check_stmt_arm_in_function(env, local_names, stmt, return_ty.clone()),
            |_, _| Ok(()),
        ),
        _ => check_stmt_arm_in_function(env, local_names, stmt, return_ty),
    }
}

/// [`check_stmt_in_function`] for one statement, with a loop checked
/// without its `loop_flow` frame.
pub(crate) fn check_stmt_arm_in_function(
    env: &mut Environment,
    local_names: &[&str],
    stmt: &HirStmt,
    return_ty: Ty,
) -> Result<(), Diagnostic> {
    match stmt {
        // `yield value` in a generator function: the value must suit the
        // generator's element type.
        HirStmt::ExprStmt(HirExpr::Call { callee, args }) if callee == pycc_hir::YIELD_MARKER => {
            crate::generator::check_yield(env, local_names, args, &return_ty)
        }
        HirStmt::Return(None) => {
            if return_ty != Ty::None
                && !matches!(return_ty, Ty::Generator(_))
                && !object_none::admits_none_return(&return_ty, None)
            {
                return Err(Diagnostic::error(
                    "T0022",
                    format!(
                        "expected a return value of type `{}`, got none",
                        return_ty.name()
                    ),
                    Span::new(0, 0),
                )
                .with_help(format!("return a `{}` value", return_ty.name())));
            }
            Ok(())
        }
        HirStmt::Return(Some(expr)) if object_none::admits_none_return(&return_ty, Some(expr)) => {
            // #1387: `return None` into an `object` slot; see `object_none`.
            Ok(())
        }
        HirStmt::Return(Some(expr)) => {
            // Part 2b of #1142 (#1164): the one position an artifact-owned
            // buffer name is admitted as a whole value. Intercepted here,
            // before `infer_expr_in` reaches its `Name` arm's
            // `reject_memoryview_read`, on the model that arm's own doc
            // comment records for `b[i]` and `len(b)` -- the refusal is not
            // weakened, the set of expressions that reach it narrows by one.
            //
            // Part 1 of #1175 adds the **second** provenance: the name a
            // `memoryview` parameter binds. The provenance split itself now
            // lives in `crate::buffer::admits_buffer_egress`, which both
            // walkers call with their own spelling of the same three facts,
            // so the solver's admission cannot drift wider than this one.
            // `owned_buffers` membership is this environment's answer to
            // "artifact-owned"; a `Ty::MemoryView` binding outside that set
            // is a parameter and nothing else.
            //
            // An early-return admission owes an account of *every* check the
            // ordinary path would have run, not only the one it was designed
            // to bypass. Two run below, and the two answers differ.
            //
            // The **assignability** check is bypassed deliberately: the
            // operand's type is `Ty::MemoryView` by construction of
            // `owned_buffers` on the owned arm and by the `buffer_bound`
            // conjunct itself on the caller-owned one, and the declared type
            // is `Ty::MemoryView` by `admitted_buffer_return`'s own test, so
            // the two agree on both arms.
            //
            // The **definite-assignment** check is *not* bypassed, and the
            // third conjunct below is what keeps it. `owned_buffers` joins as
            // a union across control flow while the binding joins on the
            // `Definitely`/`Maybe`/unbound lattice, so the two sets disagree
            // for exactly the name that is owned on some path and unbound on
            // another (review round 3 of #1164). Consulting `owned_buffers`
            // alone admitted `if c: a = ndarray(4)` / `return a`, whose false
            // path loads the null-initialized slot and hands the host an
            // internal `SystemError` instead of the `T0041` this contract
            // owes -- see `docs/TYPE_SYSTEM.md`'s definite-assignment clause.
            //
            // The conjunct tests the *result* of a join rather than any
            // particular join, so it closes every join form at once -- `if`
            // without `else`, an `if`/`else` binding on one side only, a
            // `while`, `for`-`range` or `for`-list body, a non-exhaustive
            // `match`, and a `try` body -- rather than the one
            // counter-example that prompted it. It also fails closed on
            // `None`: a name that is not bound at all falls through to the
            // ordinary `T0021`.
            //
            // The `returns_inside_finally` test is review round 5's, and
            // unlike the conjuncts above it is a *refusal* rather than a
            // decline: see `crate::buffer::buffer_return_inside_finally` for
            // the host crash it closes and why the mechanism's cardinality
            // assumption, not the set of paths reaching it, is what had to
            // change. It is a whole-function property, so it is computed
            // once in `check_function_in` rather than re-walked here.
            //
            // Part 1 of #1175 keeps it for the caller-owned provenance too,
            // and runs it **once**, after the provenance verdict rather than
            // inside either arm -- a duplicated check is a check that drifts.
            // A caller-owned return transfers no ownership and so needs no
            // pending-return record of its own, but a function that also
            // allocates owned storage still has one, and Part 1 does not
            // carry the argument about that record's state on the parameter
            // path. The narrowing is deliberate and conservative, not a
            // necessity; the D-244 amendment records it as revisitable with
            // #1173.
            if let Some((name, shape)) =
                crate::buffer::admitted_buffer_return(expr, Some(&return_ty))
            {
                let state = env.binding_state(name);
                if crate::buffer::admits_buffer_egress(
                    env.owned_buffers.contains(name),
                    matches!(state, Some(BindingState::Definitely(_))),
                    state.is_some_and(|state| matches!(state.ty(), Ty::MemoryView)),
                    shape,
                ) {
                    // The post-admission block, kept line-for-line parallel
                    // with the solver's own (`crate::constraints`) so a
                    // future divergence is visible in review. Every refusal
                    // here exists *because* a buffer boundary was admitted;
                    // the `!owned` gate is not one of them and lives in
                    // `admits_buffer_egress`'s formula alone (#1179).
                    if let crate::buffer::AdmittedBufferReturn::Slice {
                        start,
                        stop,
                        has_step,
                    } = shape
                    {
                        if has_step {
                            return Err(crate::buffer::buffer_slice_step_unsupported(name));
                        }
                        // D1: this branch exits before `infer_expr_in` ever
                        // sees the operand, so the bound type check the
                        // ordinary `HirExpr::Slice` arm performs has to be
                        // repeated here. Without it `b[1.5:3]` would reach
                        // an `i64` out-slot write undiagnosed and `b[x:3]`
                        // with `x` unbound would reach `pycc_mir::lookup`'s
                        // "check should have rejected this HIR" panic.
                        for (label, bound) in [("start", start), ("stop", stop)] {
                            if let Some(bound) = bound {
                                let bound_ty = infer_expr_in(env, local_names, bound)?;
                                if !is_assignable(bound_ty.clone(), Ty::Int) {
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
            }
            // Part 1 of #1333: returning a CPython object is admitted; the
            // ordinary assignability check below governs it, so `-> str`
            // returning an object stays the `T0022` mismatch.
            let actual = infer_expr_in(env, local_names, expr)?;
            if !class::is_assignable_env(env, &actual, &return_ty)
                && !object_box::admits_value(env, expr, &actual, &return_ty)?
                && !non_strict::admits_coercion(&actual, &return_ty)
            {
                // #1344: an inferred `set[int]` return the solver could not
                // widen to the `set[C]` the body builds is a compiler limit,
                // not a user error.
                if env.return_inferred
                    && let Some(diag) =
                        crate::comprehension::inferred_set_return_limit(&return_ty, &actual)
                {
                    return Err(diag);
                }
                // #380 (PR-20): if the mismatch involves a protocol,
                // produce a detailed T0046 conformance error.
                let diag = if matches!(return_ty, Ty::Protocol(_))
                    || matches!(actual, Ty::Protocol(_))
                {
                    class::assignable_error(env, &actual, &return_ty)
                } else {
                    Diagnostic::error(
                        "T0022",
                        format!(
                            "expected return type `{}`, got `{}`",
                            return_ty.name(),
                            actual.name()
                        ),
                        Span::new(0, 0),
                    )
                    .with_help(
                        foreign::object_into_scalar_help(&actual, &return_ty)
                            .unwrap_or_else(|| format!("return a `{}` value", return_ty.name())),
                    )
                };
                return Err(diag);
            }
            // PEP 695 (#387): `is_assignable`'s `from == Ty::Param` clause
            // lets a generic function's own `Ty::Param` pass as any concrete
            // scalar (the clause is needed for non-generic functions reading
            // generic class instance attributes). Narrow that here: if the
            // returned value is the *current function's own* `Ty::Param` and
            // the declared return type is a concrete scalar, reject -- after
            // monomorphization `T` is substituted with the call-site scalar,
            // which may not match the declared return type (e.g.
            // `def bad[T](x: T) -> int: return x` called as `bad("s")`).
            // A class-owned `Ty::Param` (the function has no own type param)
            // is not rejected here -- monomorphization of the class
            // substitutes it before codegen.
            if let (Some(own), Ty::Param(actual_name)) = (&env.own_type_param, &actual)
                && own.as_str() == actual_name.as_ref()
                && matches!(return_ty, Ty::Int | Ty::Float | Ty::Bool | Ty::Str)
            {
                return Err(Diagnostic::error(
                    "T0022",
                    format!(
                        "expected return type `{}`, got `{}`",
                        return_ty.name(),
                        actual.name()
                    ),
                    Span::new(0, 0),
                ).with_help(format!(
                    "a generic function's type parameter `{own}` is not guaranteed to be `{}` at every call site",
                    return_ty.name()
                )));
            }
            Ok(())
        }
        HirStmt::If { test, body, orelse } => {
            // PEP 572 (#774): bind before validating -- see the module-scope
            // `check_stmt`'s `ExprStmt` arm's doc comment for why a walrus
            // value that references an earlier walrus in the same
            // expression (`(a := 1) + (b := a + 1)`) requires the binding
            // pass to run before the full-expression `infer_expr_in` check.
            // `collect_local_names` (Step 1) already added the name to
            // `local_names`. The test always executes, so this binding is
            // unconditional relative to the branch join below.
            collect_named_expr_bindings(env, local_names, test)?;
            infer_expr_in(env, local_names, test)?;
            // Issue #118 Part 1: check each branch in an independent clone of
            // env, then join the results. A no-else `if` makes all body-only
            // bindings `Maybe`.
            // Issue #769 (Part 2 of #747): see the module-scope `If` arm's
            // identical comment -- a narrowing-eligible test forces the
            // slow/cloning path.
            let narrowing = narrow::narrowing_target(env, test);
            // Fast path: if neither branch introduces any new bindings and
            // no narrowing applies, skip the clone+join and check both
            // branches in-place.
            if narrowing.is_none() && !introduces_bindings(body) && !introduces_bindings(orelse) {
                check_if_branches_in_place_in_function(
                    env,
                    local_names,
                    body,
                    orelse,
                    return_ty.clone(),
                )
            } else {
                let mut body_env = env.clone();
                let mut orelse_env = env.clone();
                if let Some(target) = &narrowing {
                    narrow::apply_branch_narrowing(&mut body_env, &mut orelse_env, target);
                }
                narrow::check_stmt_sequence_in_function(
                    &mut body_env,
                    local_names,
                    body,
                    return_ty.clone(),
                )?;
                narrow::check_stmt_sequence_in_function(
                    &mut orelse_env,
                    local_names,
                    orelse,
                    return_ty.clone(),
                )?;
                if let Some(target) = &narrowing {
                    narrow::end_branch_narrowing(
                        env,
                        &mut body_env,
                        &mut orelse_env,
                        target,
                        body,
                        orelse,
                    );
                }
                join_if_branches(env, &body_env, &orelse_env)?;
                loop_flow::refine_if_join(env, body, orelse, &body_env, &orelse_env);
                Ok(())
            }
        }
        // See the module-scope `Break` arm.
        HirStmt::Break => {
            env.loop_exits.record_break(&env.bindings);
            Ok(())
        }
        HirStmt::Continue => Ok(()),
        HirStmt::LoopElse { .. } => {
            unreachable!("`check_stmt_in_function` checks a loop `else:` clause")
        }
        HirStmt::While { test, body } => {
            // Issue #769 follow-up (D-068 re-review round 3): see the
            // module-scope `While` arm's identical comment and
            // `narrow::apply_kill_prescan`'s doc comment.
            narrow::apply_kill_prescan(env, body);
            narrow::apply_delete_prescan(env, body, None);
            // PEP 572 (#774): bind before validating, mirroring the `If`
            // arm just above.
            collect_named_expr_bindings(env, local_names, test)?;
            infer_expr_in(env, local_names, test)?;
            // Issue #118 Part 1: the loop body may execute zero times, so
            // every body-only binding joins back as `Maybe`.
            // Fast path: if the body introduces no bindings, check in-place.
            if !introduces_bindings(body) {
                check_while_body_in_place_in_function(env, local_names, body, return_ty.clone())
            } else {
                let mut body_env = env.clone();
                narrow::check_stmt_sequence_in_function(
                    &mut body_env,
                    local_names,
                    body,
                    return_ty.clone(),
                )?;
                join_loop_body(env, &body_env);
                Ok(())
            }
        }
        HirStmt::ForRange {
            var,
            start,
            stop,
            step,
            body,
        } => {
            check_range_operand_in(env, local_names, "start", start)?;
            check_range_operand_in(env, local_names, "stop", stop)?;
            check_range_operand_in(env, local_names, "step", step)?;
            // Issue #118 Part 1: track whether the loop variable was already
            // definitely bound before the loop (see check_stmt's ForRange).
            let was_definite = matches!(env.binding_state(var), Some(BindingState::Definitely(_)));
            check_assignment(env, var, Ty::Int)?;
            let mut body_env = env.clone();
            // Issue #769 follow-up (D-068 re-review round 3): see
            // `narrow::apply_kill_prescan`.
            narrow::apply_kill_prescan(&mut body_env, body);
            narrow::apply_delete_prescan(&mut body_env, body, Some(var));
            narrow::check_stmt_sequence_in_function(
                &mut body_env,
                local_names,
                body,
                return_ty.clone(),
            )?;
            join_loop_body(env, &body_env);
            // Issue #118 Part 1: if the loop variable was not definitely bound
            // before the loop, downgrade it to Maybe (the loop may execute
            // zero times). A pre-bound variable stays Definitely bound.
            if !was_definite && let Some(ty) = env.lookup_any(var) {
                env.bind_maybe(var.to_string(), ty);
            }
            Ok(())
        }
        HirStmt::ForList { var, list, body } => {
            // #379 (PR-19): `for c in Color:` inside a function body —
            // same enum iteration intercept as the module-scope `check_stmt`
            // arm above. A class name is not a value binding, so
            // `lookup_bound_name` fails; check `env.lookup_class` first.
            if let Some(class_def) = env.lookup_class(list)
                && !class_def.enum_members.is_empty()
            {
                return check_enum_loop_body_function(
                    env,
                    var,
                    list,
                    body,
                    local_names,
                    return_ty.clone(),
                );
            }
            // Non-strict mode (#3): a local or parameter bound to a CPython
            // object iterates through CPython, as in a module body.
            if crate::non_strict::is_non_strict()
                && let Some(BindingState::Definitely(Ty::Object)) = env.binding_state(list)
            {
                return foreign::for_loop::check_function_object_loop(
                    env,
                    local_names,
                    var,
                    body,
                    return_ty.clone(),
                );
            }
            let list_ty = lookup_bound_name(env, local_names, list)?;
            // See the module-scope `check_stmt` arm's own comment (PR-11
            // Task 3, D-123): `for k in d:` iterates a dict's keys. PR-11
            // Task 7 (D-123): `for x in s:` iterates a set's elements.
            let var_ty = match list_ty {
                Ty::List(elem_ty) => *elem_ty,
                Ty::Str => Ty::Str,
                Ty::Dict(kv) => kv.0,
                Ty::Set(elem_ty) | Ty::FrozenSet(elem_ty) | Ty::Generator(elem_ty) => *elem_ty,
                other => {
                    return Err(Diagnostic::error(
                        "T0033",
                        format!(
                            "`{}` cannot be iterated with `for ... in ...` (only list[T]/str/dict[K, V]/set[T]/frozenset[T] supports this)",
                            other.name()
                        ),
                        Span::new(0, 0),
                    ));
                }
            };
            // Issue #118 Part 1: track whether the loop variable was already
            // definitely bound before the loop (see check_stmt's ForList).
            let was_definite = matches!(env.binding_state(var), Some(BindingState::Definitely(_)));
            check_assignment(env, var, var_ty)?;
            let mut body_env = env.clone();
            // Issue #769 follow-up (D-068 re-review round 3): see
            // `narrow::apply_kill_prescan`.
            narrow::apply_kill_prescan(&mut body_env, body);
            narrow::apply_delete_prescan(&mut body_env, body, Some(var));
            narrow::check_stmt_sequence_in_function(
                &mut body_env,
                local_names,
                body,
                return_ty.clone(),
            )?;
            join_loop_body(env, &body_env);
            // Issue #118 Part 1: if the loop variable was not definitely bound
            // before the loop, downgrade it to Maybe (the loop may execute
            // zero times). A pre-bound variable stays Definitely bound.
            if !was_definite && let Some(ty) = env.lookup_any(var) {
                env.bind_maybe(var.to_string(), ty);
            }
            Ok(())
        }
        // PR 3c of #1082: refused unconditionally inside a function body.
        // A function-body `for` over a CPython object needs its loop and
        // iterator emission moved onto the function failure edge, which is
        // Part 2 of #1333 (#1363); the module body's own loop runs on the
        // module-exec edge instead. The iterable is therefore never inferred
        // here -- there is no shape of it this arm could accept yet.
        //
        // The message states that bound and stops there. `pycc_hir` routes
        // *every* attribute and attribute-callee-call iterable to
        // `ForObject`, so this arm also fires for `d.keys()` and
        // `xs.copy()`, which a module body refuses too (with `T0036` and
        // `T0043`). A message promising that a module body implements the
        // construct would send those callers to a scope where their program
        // fails differently.
        // Non-strict mode (#3) admits it when the iterable is an object.
        HirStmt::ForObject { var, iter, body } if non_strict::is_non_strict() => {
            if infer_expr_in(env, local_names, iter)? != Ty::Object {
                return Err(Diagnostic::error(
                    "I0404",
                    "`for ... in <attribute or method call>` is only supported when the \
                     iterable is a CPython object"
                        .to_string(),
                    Span::new(0, 0),
                ));
            }
            foreign::for_loop::check_function_object_loop(
                env,
                local_names,
                var,
                body,
                return_ty.clone(),
            )
        }
        HirStmt::ForObject { .. } => Err(Diagnostic::error(
            "I0404",
            "`for ... in <attribute or method call>` is not supported inside a function body \
             yet, so the statement is refused here whatever the iterable turns out to be"
                .to_string(),
            Span::new(0, 0),
        )),
        // PR-12 Task 3 (D-117): `target = <comp>` in a function body,
        // checked by the shared `comprehension::check_comp_assign` helper.
        HirStmt::ListCompAssign {
            target,
            var,
            iter,
            cond,
            elt,
        } => check_comp_assign(
            env,
            local_names,
            target,
            CompView {
                var,
                iter,
                cond: cond.as_deref(),
                elts: CompElts::List(elt),
            },
        ),
        HirStmt::SetCompAssign {
            target,
            var,
            iter,
            cond,
            elt,
        } => check_comp_assign(
            env,
            local_names,
            target,
            CompView {
                var,
                iter,
                cond: cond.as_deref(),
                elts: CompElts::Set(elt),
            },
        ),
        HirStmt::DictCompAssign {
            target,
            var,
            iter,
            cond,
            key,
            value,
        } => check_comp_assign(
            env,
            local_names,
            target,
            CompView {
                var,
                iter,
                cond: cond.as_deref(),
                elts: CompElts::Dict(key, value),
            },
        ),
        HirStmt::Assign { target, value } => {
            // #1021: `name_binding` is a no-op for every code but `T0003`,
            // and for a `T0003` it substitutes the binding's name only when
            // `value` is itself the empty literal that failed -- the only
            // locator a `T0003` gets, since `HirStmt::Assign` carries no span
            // and every container diagnostic in this crate renders at `1:1`.
            // A `T0003` from a nested element position (`[[]]`) keeps the
            // generic wording; see `name_binding`'s own documentation.
            // Part 2a of #1142 (#1165): the buffer producer is admitted at
            // exactly this position -- an assignment's whole right-hand side
            // -- and `buffer::producer_assignment_ty` is the one seam that
            // admits it. `None` means the value is not a producer here
            // (including when the program's own `class`/`def`/binding of the
            // spelling wins, D-244 #1129 statement (h)) and the ordinary
            // inference below runs unchanged.
            if let Some(produced) = buffer::producer_assignment_ty(env, local_names, value) {
                let ty = produced?;
                check_assignment(env, target, ty)?;
                env.owned_buffers.insert(target.clone());
                return Ok(());
            }
            let ty = infer_expr_in(env, local_names, value)
                .map_err(|d| empty_container::name_binding(d, target, value))?;
            check_assignment_boxing(env, target, ty, value)
        }
        HirStmt::AnnAssign {
            target,
            annotation,
            value,
            is_final,
        } => {
            // Part 1 of #1027: a `memoryview` annotation is admitted only in
            // a signature, never on a declaration -- see
            // `expr::reject_memoryview_declaration`. Checked ahead of the
            // value/no-value split so both shapes route through the one
            // contract.
            //
            // Part 2a of #1142 (#1165): see the module-scope arm's comment --
            // the refusal is gated, not widened, so `a: NDArray = ndarray(n)`
            // is admitted while a value-less `a: NDArray` is refused exactly
            // as before.
            let produced = value
                .as_ref()
                .and_then(|value| buffer::producer_assignment_ty(env, local_names, value));
            if produced.is_none() {
                reject_memoryview_declaration(target, annotation)?;
            }
            if let Some(value) = value {
                let inferred = match produced {
                    Some(produced) => produced?,
                    None => infer_expr_in(env, local_names, value)
                        .map_err(|d| empty_container::name_binding(d, target, value))?,
                };
                if !class::is_assignable_env(env, &inferred, annotation)
                    && !object_box::admits_value(env, value, &inferred, annotation)?
                {
                    // #380 (PR-20): if the mismatch involves a protocol,
                    // produce a detailed T0046 conformance error.
                    let diag = if matches!(annotation, Ty::Protocol(_))
                        || matches!(inferred, Ty::Protocol(_))
                    {
                        class::assignable_error(env, &inferred, annotation)
                    } else {
                        annotation_initializer_mismatch(target, &inferred, annotation)
                    };
                    return Err(crate::locate::place_labeled(
                        diag,
                        crate::locate::expr_span(value),
                        format!("expected `{}`, found `{}`", annotation.name(), inferred.name()),
                    ));
                }
                // See the module-scope `check_stmt` arm's comment: route
                // through `check_assignment` so a name's first-established
                // representation stays sticky, matching `pycc_mir`'s own
                // `bind_variable` invariant.
                // #380 (PR-20): when the annotation is a protocol type,
                // bind with the concrete (inferred) type instead — see
                // the module-scope arm's comment for the rationale.
                let bind_ty = if matches!(annotation, Ty::Protocol(_)) {
                    inferred.clone()
                } else {
                    annotation.clone()
                };
                check_assignment(env, target, bind_ty)?;
                // Part 2a of #1142 (#1165): record the provenance the read
                // seams dispatch on. A `Ty::MemoryView` can only reach this
                // point from the producer -- the gated declaration refusal
                // above rejects every other initializer under a buffer
                // annotation -- so this needs no second look at `value`.
                if matches!(inferred, Ty::MemoryView) {
                    env.owned_buffers.insert(target.clone());
                }
            } else {
                // See the module-scope `check_stmt` arm's comment (issue
                // #245): retain the declared type via `env.declare` without
                // binding it, so a premature read still raises T0021 and a
                // later assignment is checked against the declaration.
                env.declare(target.clone(), annotation.clone())?;
            }
            // PEP 591 (#383): see the module-scope `check_stmt` arm's
            // comment — record the name as `Final` *after*
            // `check_assignment` returns so the initial assignment is not
            // rejected, only a subsequent reassignment.
            if *is_final {
                env.finals.insert(target.clone());
            }
            Ok(())
        }
        HirStmt::ExprStmt(expr) => {
            // PEP 572 (#774): bind before validating, mirroring the
            // module-scope `check_stmt`'s `ExprStmt` arm's doc comment.
            collect_named_expr_bindings(env, local_names, expr)?;
            infer_expr_in(env, local_names, expr).map(|_| ())
        }
        HirStmt::DictSet { dict, key, value } => check_dict_set(env, local_names, dict, key, value),
        HirStmt::AttrSet { base, attr, value } => {
            class::check_attr_set(env, local_names, base, attr, value)
        }
        HirStmt::Match { subject, cases } => {
            check_match(env, local_names, subject, cases, Some(&return_ty))
        }
        HirStmt::Try {
            body,
            handlers,
            orelse,
            finalbody,
        } => check_try_stmt(
            env,
            local_names,
            body,
            handlers,
            orelse,
            finalbody,
            Some(&return_ty),
        ),
        HirStmt::TryStar {
            body,
            handlers,
            orelse,
            finalbody,
        } => check_try_star_stmt(
            env,
            local_names,
            body,
            handlers,
            orelse,
            finalbody,
            Some(&return_ty),
        ),
        HirStmt::Raise { exc, cause } => check_raise_stmt(env, local_names, exc, cause),
        HirStmt::Delete { name } => del_stmt::check_delete(env, name),
        HirStmt::DeleteSlice {
            base,
            start,
            stop,
            step,
            span,
        } => foreign::slice::check_delete_slice(
            env,
            local_names,
            base,
            [start.as_deref(), stop.as_deref(), step.as_deref()],
            *span,
        ),
        HirStmt::DeleteAttr { base, span, .. } => {
            foreign::attr_store::check_delete_attr(env, local_names, base, *span)
        }
        HirStmt::DeleteItem { base, key, span } => {
            foreign::arith::check_delete_item(env, local_names, base, key, *span)
        }
        // `pycc_hir` never produces this node in a function body; binding
        // it here keeps the two statement checkers in step (#1291).
        HirStmt::ForeignImport { bindings, .. } => {
            foreign::bind_block_import(env, bindings);
            Ok(())
        }
    }
}
