//! A `return` statement's constraints: the body of
//! [`super::block_stmt::collect_stmt_constraints`]'s `HirStmt::Return` arm,
//! its admissions (a buffer egress, `None` or a boxed value into an
//! `object` slot, a tuple reboxed into object slots) checked before the
//! returned value is unified with the return term.

use super::*;

pub(super) fn collect_return_constraints(
    signatures: &HashMap<String, SignatureTerms>,
    parents: &mut Vec<usize>,
    concrete: &mut Vec<Option<Ty>>,
    constraints: &mut SolverConstraints,
    env: &mut ConstraintEnvironment<'_, '_>,
    value: Option<&HirExpr>,
    return_term: Option<TypeTerm>,
) -> Result<(), Diagnostic> {
    let Some(return_term) = return_term else {
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
        && crate::object_none::admits_none_return(declared, value)
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
        // A tuple whose elements box into the declared tuple's
        // `object` slots: the check phase's `tuple_reboxes`.
        || matches!(
            (&return_term, &actual),
            (Ok(Ty::Tuple(slots)), Some(Ok(Ty::Tuple(items))))
                if items.len() == slots.len()
                    && items.iter().zip(slots.iter()).any(|(i, s)| i != s && *s == Ty::Object)
                    && items.iter().zip(slots.iter()).all(|(i, s)| i == s || *s == Ty::Object)
        )
    {
        return Ok(());
    }
    if actual.is_none()
        && crate::non_strict::is_non_strict()
        && let Err(var) = &return_term
    {
        constraints.opaque_returns.push(*var);
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
    Ok(())
}
