//! Module-scope statement checking (`check_stmt`) and branch/loop joins.

use super::*;

/// Issue #118 Part 1: joins two branch environments into `env` after an `if`
/// statement. The join lattice is: `Definitely` > `Maybe` > unbound.
///
/// - A name bound `Definitely` in **both** branches (with compatible types)
///   joins to `Definitely`.
/// - A name bound in only one branch (or `Maybe` in either) joins to `Maybe`.
/// - A name bound `Definitely` in one branch and `Maybe` in the other joins to
///   `Maybe`.
/// - A name unbound in one branch and bound in the other joins to `Maybe`.
/// - A name unbound in both branches stays unbound.
///
/// Types from both branches must be compatible (via `is_assignable`); a
/// mismatch produces `T0023`. The first-established representation (type)
/// wins, matching `check_assignment`'s first-assignment-wins rule.
pub(crate) fn join_if_branches(
    env: &mut Environment,
    body_env: &Environment,
    orelse_env: &Environment,
) -> Result<(), Diagnostic> {
    let mut joined: HashMap<String, BindingState> = HashMap::new();
    // Pass 1: process every name bound in the body branch. For each, look
    // up the orelse branch's state (if any) and join. This pass covers all
    // names in `body_env`; the `(None, None)` case never arises because we
    // iterate `body_env.bindings` directly (each entry is `Some` on the body
    // side by construction).
    for (name, body_state) in &body_env.bindings {
        let orelse_state = orelse_env.bindings.get(name);
        match (body_state, orelse_state) {
            // Both branches bind the name.
            (BindingState::Definitely(bt), Some(BindingState::Definitely(ot))) => {
                if bt != ot && !is_assignable(bt.clone(), ot.clone()) {
                    non_strict::request_widen(name, bt, ot, |t| {
                        matches!(t, Ty::Object | Ty::None) || foreign::is_object_operand(env, t)
                    });
                    return Err(Diagnostic::error(
                        "T0023",
                        format!(
                            "cannot assign `{}` to `{name}`, previously inferred as `{}`",
                            ot.name(),
                            bt.name()
                        ),
                        Span::new(0, 0),
                    ).with_help(format!("change the value to `{}` (the expected/declared type), or the declaration/annotation to `{}` (the actual type)", bt.name(), ot.name())));
                }
                // First-assignment-wins: keep the body's type (matching
                // check_assignment's representation stickiness).
                joined.insert(name.clone(), BindingState::Definitely(bt.clone()));
            }
            // One or both branches have Maybe, or only one branch binds it.
            (BindingState::Definitely(ty), Some(BindingState::Maybe(_)))
            | (BindingState::Maybe(_), Some(BindingState::Definitely(ty)))
            | (BindingState::Maybe(ty), Some(BindingState::Maybe(_))) => {
                // Join of Definitely and Maybe = Maybe; Maybe and Maybe = Maybe.
                // Keep the type from whichever is available (first wins).
                joined.insert(name.clone(), BindingState::Maybe(ty.clone()));
            }
            // Body binds it, orelse does not -> Maybe.
            (BindingState::Definitely(ty), None) | (BindingState::Maybe(ty), None) => {
                joined.insert(name.clone(), BindingState::Maybe(ty.clone()));
            }
        }
    }
    // Pass 2: process names bound only in the orelse branch (not in body).
    // Each such name is Maybe (the body branch might have been the taken one).
    for (name, orelse_state) in &orelse_env.bindings {
        if body_env.bindings.contains_key(name) {
            continue; // already handled in pass 1
        }
        match orelse_state {
            BindingState::Definitely(ty) | BindingState::Maybe(ty) => {
                joined.insert(name.clone(), BindingState::Maybe(ty.clone()));
            }
        }
    }
    env.bindings = joined;
    // Part 2a of #1142 (#1165): provenance joins as a union. A name owned on
    // either path is owned for every reader after the join -- the alternative
    // (intersection) would report the *parameter* refusal for a name no
    // parameter ever bound. Codegen's epilogue is safe on the untaken path
    // because `storage_slot_at_entry` null-initializes the slot and
    // `pycc_rt_buffer_f64_free` is a documented no-op on null.
    env.owned_buffers
        .extend(body_env.owned_buffers.iter().cloned());
    env.owned_buffers
        .extend(orelse_env.owned_buffers.iter().cloned());
    // Blocker fix (D-068 review of #780): reconcile the `narrowed` overlay
    // the same way `bindings` is reconciled just above, instead of leaving
    // it as whatever it was before this `if` ran. See
    // `narrow::join_narrowed`'s doc comment for the full soundness
    // rationale -- a name stays narrowed only if both branches still narrow
    // it to the exact same type; a kill (or a narrowing established in only
    // one branch) drops out.
    env.narrowed = narrow::join_narrowed(&body_env.narrowed, &[&orelse_env.narrowed]);
    Ok(())
}

/// Issue #118 Part 1: joins a loop body environment back into `env` after a
/// `while` or `for` loop. The loop body may execute zero times, so every
/// body-only binding joins back as `Maybe`. A name that was `Definitely`
/// bound before the loop stays `Definitely` (it was bound regardless of
/// whether the loop ran), unless the body leaves it `Maybe` -- a `del`
/// (#1244). A name that was `Maybe` before the loop and is also bound in the
/// body stays `Maybe`.
pub(crate) fn join_loop_body(env: &mut Environment, body_env: &Environment) {
    // For each name bound in the body but not already Definitely bound in env,
    // downgrade to Maybe. Names already Definitely bound in env are unchanged.
    for (name, state) in &body_env.bindings {
        match env.bindings.get(name) {
            Some(BindingState::Definitely(_)) => {
                // Already definite before the loop -- stays definite. But if
                // the body assigned a different (incompatible) type,
                // check_assignment already caught that inside the body check.
                // Keep the existing definite binding -- unless the body left
                // it `Maybe`, which only a `del` can do (#1244): the loop
                // (or the `try` body this also joins) may have deleted it.
                if let BindingState::Maybe(ty) = state {
                    env.bindings
                        .insert(name.clone(), BindingState::Maybe(ty.clone()));
                }
            }
            _ => {
                // Not bound in env, or maybe-bound: the body may or may not
                // have run, so this is Maybe.
                let ty = match state {
                    BindingState::Definitely(ty) | BindingState::Maybe(ty) => ty.clone(),
                };
                env.bindings.insert(name.clone(), BindingState::Maybe(ty));
            }
        }
    }
    // Part 2a of #1142 (#1165): see `join_if_branches` -- a buffer allocated
    // in a loop body is owned after the loop, whether or not the body ran.
    env.owned_buffers
        .extend(body_env.owned_buffers.iter().cloned());
    // Blocker fix (D-068 review of #780): a loop may run zero or more
    // times, so a name stays narrowed after the loop only if the body
    // still narrows it to the same type it had going in -- the loop
    // running zero times is exactly "env's own narrowed map", and the loop
    // body having run is "body_env's narrowed map after the body executed
    // once", so intersecting the two covers both cases. A kill inside the
    // body (e.g. `while flag: x = None`) drops `x` out, matching
    // `join_if_branches`'s identical fix. See `narrow::join_narrowed`.
    env.narrowed = narrow::join_narrowed(&env.narrowed, &[&body_env.narrowed]);
}

/// Issue #118 Part 1: fast-path helper for module-scope `if` statements
/// where neither branch introduces new bindings. Checks both branches
/// in-place without cloning env, matching the pre-#118 behavior.
pub(crate) fn check_if_branches_in_place(
    env: &mut Environment,
    body: &[HirStmt],
    orelse: &[HirStmt],
) -> Result<(), Diagnostic> {
    // Warning fix (D-068 review of #780): route through the
    // narrowing-aware sequence checker, not a raw per-statement loop.
    // `introduces_bindings` gates this fast path on "no new bindings", but
    // says nothing about whether a *nested* statement recognizes an
    // early-return narrowing guard (`narrow::apply_post_if_narrowing`) that
    // needs to propagate to later statements in this same body/orelse --
    // skipping that propagation here silently rejected an otherwise valid
    // nested guard shape. See `crates/pycc_types/src/narrow.rs`.
    narrow::check_stmt_sequence(env, body)?;
    narrow::check_stmt_sequence(env, orelse)
}

/// Issue #118 Part 1: fast-path helper for module-scope `while` loops
/// where the body introduces no new bindings. Checks the body in-place
/// without cloning env.
pub(crate) fn check_while_body_in_place(
    env: &mut Environment,
    body: &[HirStmt],
) -> Result<(), Diagnostic> {
    // Warning fix (D-068 review of #780): see `check_if_branches_in_place`'s
    // identical comment.
    narrow::check_stmt_sequence(env, body)
}

pub fn check_stmt(env: &mut Environment, stmt: &HirStmt) -> Result<(), Diagnostic> {
    check_stmt_here(env, stmt).map_err(|diagnostic| crate::locate::place_stmt(diagnostic, stmt))
}

/// [`check_stmt`] before its diagnostic is placed at `stmt`.
fn check_stmt_here(env: &mut Environment, stmt: &HirStmt) -> Result<(), Diagnostic> {
    // A loop is checked inside a `loop_flow` frame, which joins its normal
    // exit (through its `else:` clause) with the state at each `break`.
    match stmt {
        HirStmt::LoopElse { loop_stmt, orelse } => loop_flow::check_loop(
            env,
            loop_stmt,
            orelse,
            |env| check_stmt_arm(env, loop_stmt),
            narrow::check_stmt_sequence,
        ),
        _ if loop_body(stmt).is_some() => loop_flow::check_loop(
            env,
            stmt,
            &[],
            |env| check_stmt_arm(env, stmt),
            |_, _| Ok(()),
        ),
        _ => check_stmt_arm(env, stmt),
    }
}

/// [`check_stmt`] for one statement, with a loop checked without its
/// `loop_flow` frame.
pub(crate) fn check_stmt_arm(env: &mut Environment, stmt: &HirStmt) -> Result<(), Diagnostic> {
    match stmt {
        HirStmt::Assign { target, value } => {
            // Part 2a of #1142 (#1165): this is `check_stmt`, the
            // *module-scope* statement walk, so it deliberately carries no
            // admitting seam for the buffer producer -- the function-scope
            // walk (`check_stmt_in_function`) is the only one that admits it.
            // A module-level `a = ndarray(n)` reaches `infer_expr` below and
            // is refused by `expr.rs`'s `Call` arm with
            // `buffer::producer_at_module_scope`, on its own ground: a
            // module-level frame gets no owned-slot epilogue, so the
            // free-at-exit lifetime has no exit to run at.
            let ty = infer_expr(env, value)?;
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
            // Part 2a of #1142 (#1165) admits one shape this refusal used to
            // cover: `a: NDArray = ndarray(n)`. The call is *gated* rather
            // than the contract widened, so the value-less shape the refusal
            // exists for is unchanged. Its message is not byte-identical: the
            // tail claiming a buffer is admitted only as a parameter was
            // corrected once Part 2a gave the type a second provenance.
            let produced = value
                .as_ref()
                .and_then(|value| buffer::producer_assignment_ty(env, &[], value));
            if produced.is_none() {
                reject_memoryview_declaration(target, annotation)?;
            }
            if let Some(value) = value {
                let inferred = match produced {
                    Some(produced) => produced?,
                    None => infer_expr(env, value)?,
                };
                // Part 4 of #1026 (PR 4c of #1083): a foreign CPython
                // object under a fixed-arity all-`float` tuple annotation
                // is admitted here, ahead of the assignability test, rather
                // than by widening `is_assignable`. The distinction is the
                // whole design: `is_assignable` is consulted wherever a
                // value flows into a declared type -- a parameter, a
                // `return`, a class attribute -- and widening it would
                // admit the pair at every one of them, while what Part 4
                // supports is exactly this statement, in exactly a module
                // body (an in-function occurrence is already `I0404` at the
                // read of the foreign name, so that arm needs no branch).
                // `foreign::is_object_float_tuple_annotation` owns the rule
                // and records what stays refused.
                //
                // Nothing below needs a second branch: `bind_ty` already
                // resolves to the annotation for a `Ty::Tuple`, so the name
                // binds to the tuple type the unpack really produces.
                let unpacks_into_float_tuple = matches!(inferred, Ty::Object)
                    && foreign::is_object_float_tuple_annotation(annotation);
                if !unpacks_into_float_tuple
                    && !class::is_assignable_env(env, &crate::int_enum_slot(&inferred, annotation), annotation)
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
                        format!(
                            "expected `{}`, found `{}`",
                            annotation.name(),
                            inferred.name()
                        ),
                    ));
                }
                // Route through `check_assignment` (not a raw `env.bind`) so a
                // name's first-established representation stays sticky across
                // an annotated re-declaration, exactly as it already does for
                // plain `Assign` -- `pycc_mir`'s own `bind_variable` (D-040's
                // "first assignment fixes a binding's representation"
                // invariant) keeps the *first* recorded MIR type regardless of
                // a later compatible reassignment, so the checker's `env` must
                // agree or a later annotated reassignment (e.g. `x = 1` then
                // `x: str = "s"`, where `is_assignable(Str, Str)` alone would
                // wrongly accept it) could diverge from what codegen actually
                // stores.
                // #380 (PR-20): when the annotation is a protocol type, bind
                // with the concrete (inferred) type instead — the protocol
                // type is a compile-time-only interface, and the MIR needs
                // the concrete type for method/attribute resolution (static
                // dispatch). The conformance check above already validated
                // that the inferred type conforms to the protocol.
                let bind_ty = if matches!(annotation, Ty::Protocol(_)) {
                    inferred.clone()
                } else {
                    annotation.clone()
                };
                check_assignment(env, target, bind_ty)?;
                // Part 2a of #1142 (#1165): no owned-buffer provenance is
                // recorded here, for the same reason the plain `Assign` arm
                // above admits no producer -- at module scope `inferred` is
                // never `Ty::MemoryView`, because the only expression that
                // could produce one is refused by `buffer::
                // producer_at_module_scope` before this point.
            } else {
                // No initializer: register no *binding* (a premature read
                // still raises the existing T0021 -- collect_local_names
                // (Step 1) already marked `target` local, and `declare`
                // never touches `bindings`), but do retain the declared
                // type (issue #245) so a later plain or annotated
                // assignment is checked against it instead of silently
                // treating the first later assignment as the initial,
                // unconstrained binding.
                env.declare(target.clone(), annotation.clone())?;
            }
            // PEP 591 (#383): record the name as `Final` *after*
            // `check_assignment` returns, so the initial assignment's own
            // `check_assignment` call does not yet see the name in `finals`
            // and is not rejected. A subsequent plain `Assign` or valued
            // `AnnAssign` to the same name will see it in both `finals` and
            // `bindings`, and is rejected with `T0045`. A value-less `Final`
            // declaration (`x: Final[int]` with no `= ...`) also inserts into
            // `finals` — the first real assignment to it is allowed (the name
            // is in `declared`, not `bindings`, so `check_assignment`'s
            // `finals` check does not fire), but a second assignment is
            // rejected.
            if *is_final {
                env.finals.insert(target.clone());
            }
            Ok(())
        }
        HirStmt::ExprStmt(expr) => {
            // PEP 572 (#774): bind any walrus target the expression
            // introduces (`x := value`) *before* the full `infer_expr`
            // validation pass below -- a walrus value can itself reference
            // an earlier walrus bound within the very same expression
            // (`(a := 1) + (b := a + 1)`), and `infer_expr`'s own read-only
            // walk never binds anything, so `b`'s value would otherwise see
            // `a` as never bound. `collect_named_expr_bindings` performs its
            // own `infer_expr_in` call on each walrus's `value` as it binds
            // it (in the expression's true left-to-right evaluation order),
            // so by the time the full-expression `infer_expr` call below
            // runs, every walrus name it may reference is already bound;
            // `infer_expr` still does the real work of enforcing T0050 on
            // each `NamedExpr` node and type-checking the expression as a
            // whole (a nested walrus's own `check_assignment` inside this
            // first pass can also fail, e.g. T0045/T0023, which aborts
            // before `infer_expr` ever runs -- fine, since either error
            // aborts the same statement).
            collect_named_expr_bindings(env, &[], expr)?;
            infer_expr(env, expr).map(|_| ())
        }
        HirStmt::If { test, body, orelse } => {
            // PEP 572 (#774): bind before validating, mirroring the
            // `ExprStmt` arm's own ordering rationale above -- the test
            // always executes, so this binding is unconditional relative to
            // the branch join below.
            collect_named_expr_bindings(env, &[], test)?;
            infer_expr(env, test)?;
            // Issue #118 Part 1: check each branch in an independent clone of
            // env, then join the results. A no-else `if` makes all body-only
            // bindings `Maybe` (the orelse clone is empty, so every body
            // binding is "one branch only" -> Maybe).
            // Issue #769 (Part 2 of #747): a narrowing-eligible test needs
            // per-branch overlay state (`narrow::apply_branch_narrowing`),
            // which only exists on a branch-local `env` clone -- force the
            // slow/cloning path even when neither branch introduces a
            // binding, so the fast path below never silently skips
            // narrowing.
            let narrowing = narrow::narrowing_target(env, test);
            // Fast path: if neither branch introduces any new bindings and
            // no narrowing applies, skip the clone+join and check both
            // branches in-place (matching the pre-#118 behavior for
            // guard-only ifs).
            if narrowing.is_none() && !introduces_bindings(body) && !introduces_bindings(orelse) {
                check_if_branches_in_place(env, body, orelse)
            } else {
                let mut body_env = env.clone();
                let mut orelse_env = env.clone();
                if let Some(target) = &narrowing {
                    narrow::apply_branch_narrowing(&mut body_env, &mut orelse_env, target);
                }
                narrow::check_stmt_sequence(&mut body_env, body)?;
                narrow::check_stmt_sequence(&mut orelse_env, orelse)?;
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
        // A `break` binds nothing and checks nothing (lowering already
        // proved it has an enclosing loop); its loop's exit join needs the
        // bindings in effect here. A `continue` returns to the loop head,
        // which needs no record (see `loop_flow`).
        HirStmt::Break => {
            env.loop_exits.record_break(&env.bindings);
            Ok(())
        }
        HirStmt::Continue => Ok(()),
        HirStmt::LoopElse { .. } => unreachable!("`check_stmt` checks a loop `else:` clause"),
        HirStmt::While { test, body } => {
            // Issue #769 follow-up (D-068 re-review round 3): a `while`
            // body can be re-entered, and `test` itself re-executes on
            // every iteration too -- prescan and drop any name `body`
            // kills *before* checking `test`, so both the test and the
            // body (fast in-place path or slow clone+join path, which
            // clones `env` after this line and so inherits the pruning)
            // see it. See `narrow::apply_kill_prescan`'s doc comment.
            narrow::apply_kill_prescan(env, body);
            narrow::apply_delete_prescan(env, body, None);
            // PEP 572 (#774): bind before validating -- see the `ExprStmt`
            // arm's doc comment above for why this order is required.
            collect_named_expr_bindings(env, &[], test)?;
            infer_expr(env, test)?;
            // Issue #118 Part 1: the loop body may execute zero times, so
            // every body-only binding joins back as `Maybe`.
            // Fast path: if the body introduces no bindings, check in-place.
            if !introduces_bindings(body) {
                check_while_body_in_place(env, body)
            } else {
                let mut body_env = env.clone();
                narrow::check_stmt_sequence(&mut body_env, body)?;
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
            check_range_operand(env, "start", start)?;
            check_range_operand(env, "stop", stop)?;
            check_range_operand(env, "step", step)?;
            // Issue #118 Part 1: track whether the loop variable was already
            // definitely bound before the loop. If so, it stays definite
            // after the loop (the variable was bound regardless of whether
            // the loop ran). If not, it is `Maybe` after the loop (the loop
            // may execute zero times).
            let was_definite = matches!(env.binding_state(var), Some(BindingState::Definitely(_)));
            check_assignment(env, var, Ty::Int)?;
            let mut body_env = env.clone();
            // Issue #769 follow-up (D-068 re-review round 3): the loop
            // body can re-run, so prescan-drop any name it kills before
            // checking it. See `narrow::apply_kill_prescan`.
            narrow::apply_kill_prescan(&mut body_env, body);
            narrow::apply_delete_prescan(&mut body_env, body, Some(var));
            narrow::check_stmt_sequence(&mut body_env, body)?;
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
            // #379 (PR-19): `for c in Color:` — iterating an enum class's
            // members. A class name is not a value binding (classes live in
            // `env.classes`, not `env.bindings`), so `lookup_bound_name`
            // fails. Check `env.lookup_class` first: if `list` is an enum
            // class (has non-empty `enum_members`), bind `var` to
            // `Ty::Instance(list)` and check the body. The actual unrolling
            // (expanding the loop into N sequential copies) is done by a
            // separate HIR→HIR rewrite pass that runs after
            // `check_and_resolve` (see `unroll_enum_loops`), so MIR never
            // sees an enum iterable.
            if let Some(class_def) = env.lookup_class(list)
                && !class_def.enum_members.is_empty()
            {
                return check_enum_loop_body_module(env, var, list, body);
            }
            // #1325: a bare name bound to a CPython object iterates exactly
            // like `for x in o.attr:` does -- `pycc_hir` has no type
            // information, so the bare name arrives here rather than as a
            // `HirStmt::ForObject`, and `pycc_mir` routes it to
            // `MirStmt::ForObject`. Keyed on the type alone: a bare imported
            // module iterates too, and fails at run time with CPython's own
            // `TypeError`, which import provenance could not rule out anyway
            // (`from os import path` binds a module, `from sys import path` a
            // list). A `Maybe` binding falls through to `lookup_bound_name`
            // and keeps its possibly-unbound diagnostic.
            if let Some(BindingState::Definitely(Ty::Object)) = env.binding_state(list) {
                return foreign::for_loop::check_module_object_loop(env, var, body);
            }
            // Module (top-level) scope has no "local before assignment"
            // concept the way a function body does -- every other arm here
            // (e.g. `ExprStmt` via `infer_expr`) resolves names with an
            // empty `local_names` slice too, so an unresolved `list` is
            // simply "not defined," never `unbound_local`.
            let list_ty = lookup_bound_name(env, &[], list)?;
            // PR-11 Task 3 (D-123): `for k in d:` iterates a dict's own keys
            // in insertion order, mirroring `len()`'s own relaxation to
            // accept `Ty::Dict` alongside `Ty::List` at this crate's other
            // hand-recognized dispatch points. `HirStmt::ForList` itself is
            // reused unconditionally for any bare-name iterable, dict or
            // list alike (`pycc_hir`'s own lowering has no type information
            // to pick a different node) -- this is the point where the real
            // type is resolved. PR-11 Task 7 (D-123): `for x in s:` iterates
            // a set's own elements (order is this implementation's own
            // insertion order, not a CPython-matching guarantee -- see
            // D-123's own iteration-order caveat), so `Ty::Set` is accepted
            // here too, binding the loop variable as the set's element type.
            let var_ty = match list_ty {
                Ty::List(elem_ty) => *elem_ty,
                Ty::Str => Ty::Str,
                Ty::Dict(kv) => kv.0,
                Ty::Set(elem_ty) | Ty::FrozenSet(elem_ty) => *elem_ty,
                Ty::Generator(sig) => sig.0,
                // A tuple whose elements share one type.
                Ty::Tuple(items) if pycc_hir::homogeneous_tuple_element(&items).is_some() => {
                    items[0].clone()
                }
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
            // definitely bound before the loop (see ForRange above).
            let was_definite = matches!(env.binding_state(var), Some(BindingState::Definitely(_)));
            check_assignment(env, var, var_ty)?;
            let mut body_env = env.clone();
            // Issue #769 follow-up (D-068 re-review round 3): see
            // `narrow::apply_kill_prescan`.
            narrow::apply_kill_prescan(&mut body_env, body);
            narrow::apply_delete_prescan(&mut body_env, body, Some(var));
            narrow::check_stmt_sequence(&mut body_env, body)?;
            join_loop_body(env, &body_env);
            // Issue #118 Part 1: if the loop variable was not definitely bound
            // before the loop, downgrade it to Maybe (the loop may execute
            // zero times). A pre-bound variable stays Definitely bound.
            if !was_definite && let Some(ty) = env.lookup_any(var) {
                env.bind_maybe(var.to_string(), ty);
            }
            Ok(())
        }
        // PR 3c of #1082 (Part 3 of #1026): `for x in o.attr:` and
        // `for x in o.method(...):`, the two iterable shapes `pycc_hir`
        // admits without type information. This is the point where the
        // iterable's real type decides, and the dispatch is on that
        // resolved type and nothing else.
        HirStmt::ForObject { var, iter, body } => {
            let iter_ty = infer_expr(env, iter)?;
            if !matches!(iter_ty, Ty::Object) {
                // Reachable: an attribute load whose base is not a foreign
                // object lowers to this node too, so `for x in C.value:`
                // over an `int` class attribute lands here and reports
                // `got `int``. (`for x in xs.copy():` over a `list` also
                // lowers to this node, but its receiver is rejected with
                // `T0043` while `iter` is inferred above, so it never
                // reaches this arm.) Iterating any of those is not
                // supported by another arm either, so the refusal is
                // correct -- only its wording is specific to this shape.
                return Err(Diagnostic::error(
                    "I0404",
                    format!(
                        "`for ... in <attribute or method call>` is only supported when the \
                         iterable is a CPython object (Part 3 of #1026), got `{}`",
                        iter_ty.name()
                    ),
                    Span::new(0, 0),
                ));
            }
            foreign::for_loop::check_module_object_loop(env, var, body)
        }
        // PR-12 Task 3 (D-117): `target = <comp>` at module scope, checked
        // by the shared `comprehension::check_comp_assign` helper.
        HirStmt::ListCompAssign {
            target,
            var,
            iter,
            cond,
            elt,
        } => check_comp_assign(
            env,
            &[],
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
            &[],
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
            &[],
            target,
            CompView {
                var,
                iter,
                cond: cond.as_deref(),
                elts: CompElts::Dict(key, value),
            },
        ),
        HirStmt::Return(_) => Err(Diagnostic::error(
            "T0024",
            "'return' outside a function is not allowed".to_string(),
            Span::new(0, 0),
        )),
        HirStmt::DictSet { dict, key, value } => check_dict_set(env, &[], dict, key, value),
        HirStmt::AttrSet { base, attr, value } => {
            class::check_attr_set(env, &[], base, attr, value)
        }
        HirStmt::Match { subject, cases } => check_match(env, &[], subject, cases, None),
        HirStmt::Try {
            body,
            handlers,
            orelse,
            finalbody,
        } => check_try_stmt(env, &[], body, handlers, orelse, finalbody, None),
        HirStmt::TryStar {
            body,
            handlers,
            orelse,
            finalbody,
        } => check_try_star_stmt(env, &[], body, handlers, orelse, finalbody, None),
        HirStmt::Raise { exc, cause } => check_raise_stmt(env, &[], exc, cause),
        HirStmt::Delete { name } => del_stmt::check_delete(env, name),
        HirStmt::DeleteSlice {
            base,
            start,
            stop,
            step,
            span,
        } => foreign::slice::check_delete_slice(
            env,
            &[],
            base,
            [start.as_deref(), stop.as_deref(), step.as_deref()],
            *span,
        ),
        HirStmt::SetSlice {
            base,
            start,
            stop,
            step,
            value,
            span,
        } => foreign::slice::check_set_slice(
            env,
            &[],
            base,
            [start.as_deref(), stop.as_deref(), step.as_deref()],
            value,
            *span,
        ),
        HirStmt::DeleteAttr { base, span, .. } => {
            foreign::attr_store::check_delete_attr(env, &[], base, *span)
        }
        HirStmt::DeleteItem { base, key, span } => {
            foreign::arith::check_delete_item(env, &[], base, key, *span)
        }
        HirStmt::ForeignImport { bindings, .. } => {
            foreign::bind_block_import(env, bindings);
            Ok(())
        }
    }
}

/// `d[k] = v` (PR-11 Task 3, D-123): insert-or-update. Shared between
/// module (`check_stmt`, `local_names = &[]`) and function-body
/// (`check_stmt_in_function`) scope, mirroring how `check_range_operand`/
/// `check_range_operand_in` are already split for `ForRange`. Reuses
/// `T0033` for a non-`dict` base (matching `ListAppend`'s own "does not
/// support X" shape) and `T0021` for a key or value type mismatch
/// (matching `Subscript`'s and `ListAppend`'s own reuse of `T0021` for an
/// operand/assignment constraint mismatch).
pub(crate) fn check_dict_set(
    env: &Environment,
    local_names: &[&str],
    dict: &str,
    key: &HirExpr,
    value: &HirExpr,
) -> Result<(), Diagnostic> {
    // The buffer element store is admitted *before* the `Ty::Dict`
    // destructure below, because that destructure's `else` is the `T0033`
    // refusal -- and before it, through `lookup_bound_name_for_store`,
    // because `lookup_bound_name`'s own `reject_memoryview_read` would
    // otherwise report `C0001` one line earlier still.
    //
    // Non-strict mode (#3): `o[k] = v` on a CPython object is
    // `PyObject_SetItem`. The base is read through the ordinary `Name`
    // arm, which admits an object read; any other answer falls through.
    if non_strict::is_non_strict()
        && let Ok(Ty::Object) = infer_expr_in(env, local_names, &HirExpr::Name(dict.to_string()))
    {
        let key_ty = infer_expr_in(env, local_names, key)?;
        let value_ty = infer_expr_in(env, local_names, value)?;
        return crate::foreign::arith::check_object_set_item(env, value, &key_ty, &value_ty);
    }
    let dict_ty = lookup_bound_name_for_store(env, local_names, dict)?;
    if dict_ty == Ty::MemoryView {
        return check_buffer_set(env, local_names, key, value);
    }
    if let Ty::List(elem) = &dict_ty {
        let index_ty = infer_expr_in(env, local_names, key)?;
        let value_ty = infer_expr_in(env, local_names, value)?;
        return list_ops::check_list_set(elem, &index_ty, &value_ty).inspect_err(|_| {
            // Non-strict mode (#3): a list holding values of two types is
            // CPython's own list, as every Python list is.
            non_strict::request_widen(dict, &dict_ty, &Ty::Object, |_| true);
        });
    }
    let Ty::Dict(kv) = &dict_ty else {
        return Err(Diagnostic::error(
            "T0033",
            format!("`{}` does not support item assignment", dict_ty.name()),
            Span::new(0, 0),
        ));
    };
    let (key_ty, val_ty) = kv.as_ref();
    // Exact `Ty` equality on the key, not `is_assignable` -- same reasoning
    // as `Subscript`'s own `Ty::Dict` read arm.
    let key_expr_ty = infer_expr_in(env, local_names, key)?;
    if key_expr_ty != *key_ty {
        // Non-strict mode: a key of another type (a CPython object, say)
        // makes the dict CPython's own, which hashes it as CPython does.
        non_strict::request_widen(dict, &dict_ty, &Ty::Object, |_| true);
        return Err(Diagnostic::error(
            "T0021",
            format!(
                "dict key type mismatch: expected `{}`, found `{}`",
                key_ty.name(),
                key_expr_ty.name()
            ),
            Span::new(0, 0),
        )
        .with_help(format!("use a `{}` value here", key_ty.name())));
    }
    let value_ty = infer_expr_in(env, local_names, value)?;
    if !is_assignable(value_ty.clone(), val_ty.clone()) {
        non_strict::request_widen(dict, &dict_ty, &Ty::Object, |_| true);
        return Err(Diagnostic::error(
            "T0021",
            format!(
                "cannot assign `{}` to a dict value of `{}`",
                value_ty.name(),
                val_ty.name()
            ),
            Span::new(0, 0),
        ).with_help(format!("change the value to `{}` (the expected/declared type), or the declaration/annotation to `{}` (the actual type)", val_ty.name(), value_ty.name())));
    }
    Ok(())
}

/// `b[i] = v` on a name bound to a `memoryview` (Part 1 of #1142): the
/// store counterpart of the `Subscript` load `expr.rs` intercepts, and the
/// second operation on such a name this compiler admits after `b[i]` and
/// `len(b)`.
///
/// The index rule is the load's rule verbatim -- `T0021` and `is_assignable`
/// against `Ty::Int`, so `bool` is accepted under D-086 -- because a load
/// and a store that disagreed about `b[True]` would be a defect on its own.
/// The value rule is `check_dict_set`'s own: `T0021` and `is_assignable`
/// against the element type, which for the one admitted buffer format
/// (`'d'`) is `Ty::Float`. `is_assignable` is not a widening here -- D-086
/// admits no implicit `int` -> `float` -- so `b[0] = 1` is refused and the
/// generated code has exactly one scalar shape to store.
///
/// This admits the store's *target name* and nothing else: every other read
/// of a buffer-bound name stays `reject_memoryview_read`'s `C0001`, which is
/// what keeps D-244's wholly-wrapper-owned lifetime true by construction.
pub(crate) fn check_buffer_set(
    env: &Environment,
    local_names: &[&str],
    key: &HirExpr,
    value: &HirExpr,
) -> Result<(), Diagnostic> {
    let index_ty = infer_expr_in(env, local_names, key)?;
    if !is_assignable(index_ty.clone(), Ty::Int) {
        return Err(Diagnostic::error(
            "T0021",
            format!(
                "`memoryview` index must be `int`, found `{}`",
                index_ty.name()
            ),
            Span::new(0, 0),
        )
        .with_help("use an `int` value"));
    }
    let value_ty = infer_expr_in(env, local_names, value)?;
    if !is_assignable(value_ty.clone(), Ty::Float) {
        return Err(Diagnostic::error(
            "T0021",
            format!(
                "cannot assign `{}` to a `memoryview` element of `float`",
                value_ty.name()
            ),
            Span::new(0, 0),
        )
        .with_help("use a `float` value here"));
    }
    Ok(())
}
