//! #1289: the join shared by `check_try_stmt` and `check_try_star_stmt` --
//! the state after a `try`/`try*` statement and the check of its `finally`.
//!
//! Split out of `exception.rs` under AGENTS.md's file-decomposition rule when
//! this join pushed that file past the ~1,000-line threshold; the `finally`
//! delete prescan moved with it because the join is its only caller.

use super::*;
use std::collections::HashSet;

/// A checked `try`/`try*` statement's parts together with the environment
/// each of its paths ended in, which is everything [`join_try_outcome`]
/// needs to compute the state after the statement.
pub(super) struct TryPaths<'a> {
    pub(super) body: &'a [HirStmt],
    pub(super) handlers: &'a [HirExceptHandler],
    pub(super) orelse: &'a [HirStmt],
    pub(super) finalbody: &'a [HirStmt],
    pub(super) body_env: &'a Environment,
    pub(super) handler_envs: &'a [Environment],
    pub(super) else_env: &'a Environment,
}

/// #1289: joins a checked `try`/`try*` statement's paths into `env` and
/// checks its `finally` block. Shared by [`check_try_stmt`] and
/// [`check_try_star_stmt`], whose handler loops differ but whose joins do
/// not.
///
/// Two separate questions are answered over two separate sets of
/// environments.
///
/// **Type consistency, over every environment.** The body, then each
/// handler in source order, then `else` are walked, and every name that is
/// not bound before the `try` must keep one representation across all of
/// them: a later type must be assignable to the first-established one, in
/// `check_assignment`'s direction. `Maybe` bindings and terminating handlers
/// count, because every path stores into the same slot. The first-established
/// type becomes the name's type after the statement.
///
/// A name that any handler of the statement binds with `as` is left out of
/// this walk entirely, on every path, and out of the definiteness join below:
/// its type and its definiteness after the statement are exactly the
/// pre-#1289 conservative join's. On that join `try: e = 10 // d / except
/// ZeroDivisionError as e: ...` is accepted, a later `e = 5` is `T0023`
/// against the exception type, and a later read is `T0041`.
///
/// `join_if_branches` is deliberately not used to fold the paths: it checks
/// `is_assignable(first, later)`, the reverse direction, and keeps the first
/// path's type. For that reason `if d == 0: x = True / else: x = 1 /
/// print(x)` prints `True True` where CPython prints `True 1` (an `if`/`else`
/// defect outside #1289's scope); a `try` fold through it would also refuse
/// a valid `try: x = 10 // d / except ZeroDivisionError: x = False`.
///
/// **Definiteness, over the paths that fall through the statement only.**
/// Those are the `else` path (when neither the body nor `else` always
/// terminates) and every handler whose body does not always terminate. A
/// name is `Definitely` bound after the statement when it is `Definitely`
/// bound on every such path. A handler's `as` name is never promoted this
/// way and keeps its conservative state: CPython unbinds it on that
/// handler's exit with an implicit `del`, and even when that handler always
/// terminates, codegen gives the name one slot typed for the exception
/// instance, which a later read of the body's `int` cannot share. So `try: e = 10 // d / except ZeroDivisionError as e: raise /
/// return e` is `T0041` although CPython runs it: a documented limitation
/// that refuses rather than miscompiles.
///
/// The pre-#1289 conservative join (the body joined like a loop body, then every
/// handler and `else` like `if` branches) is still computed, with the
/// first-established types written over its own for every name that is not
/// an `as` name. It supplies the name set and buffer provenance, and, less
/// every name any path rebinds ([`apply_finally_kill_prescan`]), is the
/// entry state of `finally`, which can be entered after any partial run.
/// When no path falls through, it is also the state after the statement.
/// Otherwise `finally` is checked a second time
/// against the fall-through join to compute that state; checking twice has
/// no side effect beyond the environment it mutates.
///
/// The `else` fold now propagates `join_if_branches`'s error instead of
/// discarding it, like the handler folds always did. No test reaches that
/// error: it fires only for a name `Definitely` bound on both sides with
/// incompatible types, and after the loop-style body join every name new to
/// the statement is `Maybe` while a pre-existing name keeps its pinned type
/// on every path. The type walk above is what refuses a handler/`else`
/// mismatch (`tests/diagnostics/t0023_try_handler_else_mismatch.py`).
pub(super) fn join_try_outcome(
    env: &mut Environment,
    local_names: &[&str],
    return_ty: Option<&Ty>,
    paths: TryPaths<'_>,
) -> Result<(), Diagnostic> {
    let mut conservative = env.clone();
    join_loop_body(&mut conservative, paths.body_env);
    for handler_env in paths.handler_envs {
        let previous = conservative.clone();
        join_if_branches(&mut conservative, &previous, handler_env)?;
    }
    let previous = conservative.clone();
    join_if_branches(&mut conservative, &previous, paths.else_env)?;
    let as_names: HashSet<&str> = paths
        .handlers
        .iter()
        .filter_map(|handler| handler.name.as_deref())
        .collect();
    for (name, ty) in first_established_types(env, &paths, &as_names)? {
        let (BindingState::Definitely(slot) | BindingState::Maybe(slot)) = conservative
            .bindings
            .get_mut(&name)
            .expect("a name bound on a path is in the conservative join");
        *slot = ty;
    }

    let mut exits: Vec<Environment> = Vec::new();
    if !block_always_returns(paths.body) && !block_always_returns(paths.orelse) {
        exits.push(paths.else_env.clone());
    }
    for (handler, handler_env) in paths.handlers.iter().zip(paths.handler_envs) {
        if block_always_returns(&handler.body) {
            continue;
        }
        let mut exit = handler_env.clone();
        if let Some(name) = &handler.name {
            exit.narrowed.remove(name);
        }
        exits.push(exit);
    }
    let fallthrough = exits.split_first().map(|(first, rest)| {
        let mut joined = env.clone();
        joined.bindings = conservative
            .bindings
            .iter()
            .map(|(name, state)| {
                if as_names.contains(name.as_str()) {
                    return (name.clone(), state.clone());
                }
                let ty = state.ty().clone();
                let definite = exits.iter().all(|exit| {
                    matches!(exit.bindings.get(name), Some(BindingState::Definitely(_)))
                });
                let state = if definite {
                    BindingState::Definitely(ty)
                } else {
                    BindingState::Maybe(ty)
                };
                (name.clone(), state)
            })
            .collect();
        let rest: Vec<&HashMap<String, Ty>> = rest.iter().map(|exit| &exit.narrowed).collect();
        joined.narrowed = crate::narrow::join_narrowed(&first.narrowed, &rest);
        joined.owned_buffers = conservative.owned_buffers.clone();
        joined
    });

    let TryPaths {
        body,
        handlers,
        orelse,
        finalbody,
        ..
    } = paths;
    apply_finally_delete_prescan(&mut conservative, body, handlers, orelse, finalbody);
    apply_finally_kill_prescan(&mut conservative, body, handlers, orelse);
    check_stmt_sequence_shared(&mut conservative, local_names, finalbody, return_ty)?;
    let Some(mut joined) = fallthrough else {
        *env = conservative;
        return Ok(());
    };
    apply_finally_delete_prescan(&mut joined, body, handlers, orelse, finalbody);
    check_stmt_sequence_shared(&mut joined, local_names, finalbody, return_ty)?;
    *env = joined;
    Ok(())
}

/// #1289: the type-consistency half of [`join_try_outcome`]. Returns the
/// first-established type of every name a path binds that `env` (the state
/// before the `try`) does not and no handler binds with `as`, or `T0023` for
/// the first later binding that cannot be assigned to it. Names are visited in sorted order within each
/// environment so the reported name does not depend on hash order.
fn first_established_types(
    env: &Environment,
    paths: &TryPaths<'_>,
    as_names: &HashSet<&str>,
) -> Result<HashMap<String, Ty>, Diagnostic> {
    let walk = std::iter::once(paths.body_env)
        .chain(paths.handler_envs)
        .chain(std::iter::once(paths.else_env));
    let mut first: HashMap<String, Ty> = HashMap::new();
    for path_env in walk {
        let mut names: Vec<&String> = path_env
            .bindings
            .keys()
            .filter(|name| !env.bindings.contains_key(*name) && !as_names.contains(name.as_str()))
            .collect();
        names.sort();
        for name in names {
            let ty = path_env.bindings[name].ty();
            let Some(previous) = first.get(name) else {
                first.insert(name.clone(), ty.clone());
                continue;
            };
            if !crate::class::is_assignable_env(env, ty, previous) {
                return Err(Diagnostic::error(
                    "T0023",
                    format!(
                        "cannot assign `{}` to `{name}`, previously inferred as `{}`",
                        ty.name(),
                        previous.name()
                    ),
                    Span::new(0, 0),
                )
                .with_help(format!(
                    "change the value to `{}` (the expected/declared type), or the declaration/annotation to `{}` (the actual type)",
                    previous.name(),
                    ty.name()
                )));
            }
        }
    }
    Ok(first)
}

/// #1244: a `finally` block can be entered after only a partial run of the
/// try body, of any handler, or of the `else` block (an exception escaping
/// any of them), so every name any of them deletes may be unbound there.
/// Applied to both environments [`join_try_outcome`] checks `finally`
/// against -- the conservative entry state and the fall-through join that
/// also flows out of the `try` -- so the post-`try` state is conservative
/// too (`docs/TYPE_SYSTEM.md`'s "`del` statement" section lists the
/// limitation). A `try` without `finally` needs no prescan: the joins
/// already account for every path.
fn apply_finally_delete_prescan(
    env: &mut Environment,
    body: &[HirStmt],
    handlers: &[HirExceptHandler],
    orelse: &[HirStmt],
    finalbody: &[HirStmt],
) {
    if finalbody.is_empty() {
        return;
    }
    crate::narrow::apply_delete_prescan(env, body, None);
    for handler in handlers {
        crate::narrow::apply_delete_prescan(env, &handler.body, None);
    }
    crate::narrow::apply_delete_prescan(env, orelse, None);
}

/// #1476: a `finally` block can be entered after any path's rebinding --
/// the try body's, a handler's `as` name or body, or `else`'s -- so none of
/// those names is narrowed in it. The conservative join alone does not
/// drop such a name when the rebinding path narrows it again to the same
/// type before it ends (`o = p` then `if not isinstance(o, int): return`),
/// although the `finally` runs between the two when the guard returns.
/// Applied only to the conservative entry state, the one every `finally`
/// read must type under: the fall-through join is the state after the
/// `try`, where a rebinding that returns before reaching it does not count,
/// as the constraint solver's `collect_try_constraints` and `pycc_mir`'s
/// `stmt::try_stmt` also compute it.
fn apply_finally_kill_prescan(
    env: &mut Environment,
    body: &[HirStmt],
    handlers: &[HirExceptHandler],
    orelse: &[HirStmt],
) {
    crate::narrow::apply_kill_prescan(env, body);
    crate::narrow::apply_kill_prescan(env, orelse);
    for handler in handlers {
        if let Some(name) = &handler.name {
            env.narrowed.remove(name);
        }
        crate::narrow::apply_kill_prescan(env, &handler.body);
    }
}
