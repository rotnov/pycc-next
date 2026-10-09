//! The constraint solver's `try`/`except` and `try`/`except*` walk.
//!
//! Extracted from the two near-identical `HirStmt::Try`/`HirStmt::TryStar`
//! arms of [`collect_block_constraints`] per AGENTS.md's file-decomposition
//! rule (#1289). The arms differed only in what an `as` name binds to, so
//! one walk now serves both, parameterized on [`TryShape::star`].

use super::*;

/// The parts of a `try` statement the solver walks, and which of the two
/// statement forms it is.
pub(super) struct TryShape<'a> {
    pub(super) body: &'a [HirStmt],
    pub(super) handlers: &'a [pycc_hir::HirExceptHandler],
    pub(super) orelse: &'a [HirStmt],
    pub(super) finalbody: &'a [HirStmt],
    /// `true` for `except*` (PEP 654, Part 3 of #382, #542): an `as` binding
    /// always resolves to `ExceptionGroup`, never the named handler type --
    /// see `check_try_star_stmt` in `pycc_types::exception` for the same rule
    /// at type-checking time. `false` for a plain `except`, whose `as` name
    /// binds the handler's own exception type.
    pub(super) star: bool,
}

/// #382 (PR-22 Part 1): collect constraints from the try body, each handler,
/// the else body, and the finally body. The try body's bindings are joined
/// back as `Maybe` (the body may raise before reaching an assignment).
///
/// #1289: that loop-style join alone left a name `Maybe` even when every
/// path that completes the statement binds it, so an unannotated helper
/// such as `try: r = 10 // d / except ZeroDivisionError: r = -1 / return r`
/// failed with `T0021`. The paths that fall through the statement -- the
/// `else` path after a completed body, and every handler whose body does not
/// always terminate -- are collected and handed to
/// [`solver::promote_try_fallthrough`], with every handler's `as` name
/// excluded from promotion, mirroring the check phase's
/// `exception::try_join::join_try_outcome`.
pub(super) fn collect_try_constraints(
    signatures: &HashMap<String, SignatureTerms>,
    parents: &mut Vec<usize>,
    concrete: &mut Vec<Option<Ty>>,
    constraints: &mut SolverConstraints,
    env: &mut ConstraintEnvironment<'_, '_>,
    shape: TryShape<'_>,
    return_term: Option<TypeTerm>,
) -> Result<(), Diagnostic> {
    // Issue #771 join-site follow-up: include `opaque_bindings` alongside
    // `bindings` here. A name definitely-but-opaquely bound before this
    // construct must count as pre-existing too -- otherwise a branch that
    // reassigns it to a real, solver-representable term looks "newly
    // introduced" to the join helper below, and when only one branch
    // performs that reassignment the name is misclassified as bound in both
    // branches (the other, untouched branch still carries the opaque
    // marker), unmasking a term that only reflects one path as if it were
    // unconditionally correct. Confirmed as a real gap by the pinned local
    // reviewer's second pass.
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
        shape.body,
        return_term.clone(),
    )?;
    solver::join_loop_body_solver(env, &body_env, &pre_existing);
    let mut fallthrough = Vec::new();
    // #1476: a handler can run after any rebinding in the body, so the
    // body's rebindings are not narrowed in it (the check phase's
    // `apply_kill_prescan` on each handler environment).
    let body_kills = pycc_hir::killed_names(shape.body);
    for handler in shape.handlers {
        let mut henv = env.clone();
        object_narrow::kill_names(&mut henv, &body_kills);
        // Bind the `as` name in the handler environment.
        // Inside the handler body, the binding is definite.
        let binding_type = if shape.star {
            Some("ExceptionGroup".to_string())
        } else {
            handler
                .exc_type
                .as_deref()
                .map(pycc_hir::except_handler_binding_type_name)
        };
        if let (Some(name), Some(binding_type)) = (&handler.name, binding_type) {
            // Round-11 review finding 2: an `as` name rebinds, so it drops
            // any artifact-owned buffer provenance it carried; the join
            // helper then carries that invalidation back out of the handler
            // environment.
            henv.rebind_over_owned_buffer(name);
            henv.bindings
                .insert(name.clone(), Ok(Ty::Instance(Box::new(binding_type))));
        }
        // #1476: the `as` name holds the exception, never a narrowed value.
        if let Some(name) = &handler.name {
            henv.narrowed.remove(name);
        }
        collect_block_constraints(
            signatures,
            parents,
            concrete,
            constraints,
            &mut henv,
            &handler.body,
            return_term.clone(),
        )?;
        solver::join_loop_body_solver(env, &henv, &pre_existing);
        if !crate::block_always_returns(&handler.body) {
            fallthrough.push(henv);
        }
    }
    // `else` runs only after the body completed, so it starts from the
    // body's own state, exactly as the check phase's `else_env` does.
    let mut else_env = body_env.clone();
    collect_block_constraints(
        signatures,
        parents,
        concrete,
        constraints,
        &mut else_env,
        shape.orelse,
        return_term.clone(),
    )?;
    solver::join_loop_body_solver(env, &else_env, &pre_existing);
    if !crate::block_always_returns(shape.body) && !crate::block_always_returns(shape.orelse) {
        fallthrough.push(else_env);
    }
    // A name any handler binds with `as` is never promoted: its slot holds
    // an exception instance on that handler's path, and codegen cannot
    // represent a later read of it (see `promote_try_fallthrough`).
    let mut excluded = pre_existing;
    excluded.extend(shape.handlers.iter().filter_map(|h| h.name.clone()));
    solver::promote_try_fallthrough(env, &fallthrough, &excluded);
    // #1476: after the `try`, a name stays narrowed only if every path that
    // falls through still narrows it to the same type, as the check phase's
    // `join_try_outcome` joins its exits.
    let joined_narrowed = fallthrough.split_first().map(|(first, rest)| {
        let rest: Vec<&HashMap<String, Ty>> = rest.iter().map(|exit| &exit.narrowed).collect();
        crate::narrow::join_narrowed(&first.narrowed, &rest)
    });
    // The finally body runs after any path, so a name any path rebinds is
    // not narrowed in it.
    let mut path_kills = body_kills;
    path_kills.extend(pycc_hir::killed_names(shape.orelse));
    for handler in shape.handlers {
        path_kills.extend(handler.name.iter().cloned());
        path_kills.extend(pycc_hir::killed_names(&handler.body));
    }
    object_narrow::kill_names(env, &path_kills);
    // The finally body always runs — collect in-place.
    collect_block_constraints(
        signatures,
        parents,
        concrete,
        constraints,
        env,
        shape.finalbody,
        return_term,
    )?;
    // With no path falling through, nothing after the `try` runs. Otherwise
    // the fall-through join continues, less what the finally rebinds.
    if let Some(mut narrowed) = joined_narrowed {
        object_narrow::kill_names_in(&mut narrowed, &pycc_hir::killed_names(shape.finalbody));
        env.narrowed = narrowed;
    }
    Ok(())
}
