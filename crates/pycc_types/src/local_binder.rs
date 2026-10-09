//! The flat whole-function local binder (#380, PR-20), shared by protocol
//! monomorphization (`monomorphize.rs`) and the D-245 empty-container
//! pre-pass (`empty_container.rs`). It walks a body in source order and
//! records each assignment's inferred type into an `Environment`, never
//! validating: a statement it cannot type leaves the environment unchanged.
//! Extracted from `lib.rs` by #1445, which added its `try` arm.

use crate::{BindingState, Environment, collect_named_expr_bindings, infer_expr_in};
use pycc_hir::{HirExpr, HirStmt, Ty};

/// #380 (PR-20): Pre-binds function-local variable types into `env` by
/// walking the body in source order and inferring each assignment's
/// value type. This lets the protocol monomorphization pass resolve
/// local variables (not just module-level globals) when inferring the
/// concrete type of a call-site argument.
pub(crate) fn bind_local_types_in_body(
    env: &mut Environment,
    local_names: &[&str],
    body: &[HirStmt],
) {
    for stmt in body {
        bind_local_types_in_stmt(env, local_names, stmt);
    }
}

pub(crate) fn bind_local_types_in_stmt(
    env: &mut Environment,
    local_names: &[&str],
    stmt: &HirStmt,
) {
    match stmt {
        HirStmt::Assign { target, value } => {
            bind_named_expr_types_in_expr(env, local_names, value);
            if let Ok(ty) = infer_expr_in(env, local_names, value) {
                // #1021 review round 5: first assignment wins, mirroring
                // D-040's sticky-representation rule in `check_assignment` --
                // a compatible reassignment there returns without rebinding,
                // so the *first* inferred type stays the name's recorded
                // representation. Overwriting here made this binder disagree
                // with the checker for the one compatible-but-narrower
                // reassignment this type system has (`v = 5` then `v = True`),
                // which is not a missed resolution but a wrong one: the
                // empty-container pre-pass resolved `xs.append(v)` to
                // `list[bool]` and D-228 then reported a `T0034` naming a type
                // the source never mentions, for a program whose `xs = [v]`
                // spelling compiles. An incompatible reassignment is rejected
                // by the checker regardless, so keeping the first type can
                // never admit a program the checker rejects.
                if env.lookup_any(target).is_none() {
                    env.bind(target.clone(), ty);
                }
            }
        }
        HirStmt::AnnAssign {
            target,
            annotation,
            value,
            ..
        } => {
            if let Some(val) = value {
                bind_named_expr_types_in_expr(env, local_names, val);
                // #1021 review round 6: mirror `check_stmt_in_function`'s own
                // `AnnAssign` arm exactly -- it binds the *annotation* through
                // `check_assignment`, except for #380's protocol special case,
                // where the concrete inferred type is bound instead. Binding
                // the inferred type unconditionally made this binder disagree
                // with the checker for a widening initializer (`v: int =
                // True`): the pre-pass recorded `bool`, resolved
                // `xs.append(v)` to `list[bool]`, and D-228 then reported a
                // `T0034` naming a type the source never mentions, for a
                // program whose `xs = [v]` spelling compiles. It is the same
                // defect round 5 fixed in the `Assign` arm above, reached
                // through the annotation instead of a reassignment.
                //
                // Binding the annotation also subsumes round 1's fallback:
                // `xs: list[int] = []`, whose raw empty literal cannot infer,
                // still seeds `list[int]`, so a later resolution derived from
                // `xs` (`for x in xs: ys.append(x)`) keeps resolving instead
                // of falling through to `T0003`, and round 2's refutation
                // still holds -- `xs: list[int] = undefined_name` reports the
                // undefined name rather than a spurious `T0003`, pinned by
                // `tests/diagnostics/t0021_annotated_broken_value_still_reports_the_real_defect`.
                // Only the protocol arm still needs that fallback, since an
                // uninferable value leaves it nothing concrete to bind.
                if matches!(annotation, Ty::Protocol(_)) {
                    // #380/#953: a protocol annotation is a compile-time-only
                    // interface, so the checker and `pycc_mir` both need the
                    // concrete type for static dispatch. This arm also runs
                    // over environments that already carry `Protocol(P)` for
                    // the target -- `specialize_protocol_functions` clones one
                    // -- so it overwrites rather than deferring to that
                    // earlier binding: leaving `Protocol(P)` in place drops
                    // the specialization and `pycc_mir` panics on the
                    // unrecorded `$fn:C.same`
                    // (`tests/issue_953_protocol_argument.rs`).
                    let ty =
                        infer_expr_in(env, local_names, val).unwrap_or_else(|_| annotation.clone());
                    env.bind(target.clone(), ty);
                } else if env.lookup_any(target).is_none() {
                    // D-040 stickiness, for the same reason the `Assign` arm
                    // above applies it: `check_assignment` keeps a name's
                    // first recorded representation on a compatible rebind, so
                    // `v = 5` then `v: bool = True` stays an `int` for the
                    // checker and the producer must resolve `list[int]`.
                    env.bind(target.clone(), annotation.clone());
                }
            } else if env.lookup_any(target).is_none() {
                // #1021 review round 17: the same D-040 stickiness the valued
                // arm above applies, for the same reason and with the same
                // failure when it is omitted. A value-less annotation reaches
                // the checker as `Environment::declare`, which keeps an
                // existing runtime binding rather than replacing it, so for
                // `v = 1; v: bool; xs = []; xs.append(v)` the checker still
                // sees `v` as `int` -- and the `xs = [v]` spelling of that
                // program checks clean. Binding `bool` here resolved the
                // producer to `list[bool]` and D-228 reported a `T0034`
                // naming a type the program never produces: wrong, not
                // missed, which is exactly what D-245's invariant forbids.
                env.bind(target.clone(), annotation.clone());
            }
        }
        // PEP 572 (#774): `test` can itself contain a walrus target
        // (`if (n := f()):`), and this pass -- unlike `bind_local_types_in_body`'s
        // own recursion into `body`/`orelse` -- has no other point where
        // `test` is visited at all, so a walrus bound only in `test` was
        // never pre-bound here without this call.
        HirStmt::If { test, body, orelse } => {
            bind_named_expr_types_in_expr(env, local_names, test);
            bind_local_types_in_body(env, local_names, body);
            bind_local_types_in_body(env, local_names, orelse);
        }
        HirStmt::While { test, body } => {
            bind_named_expr_types_in_expr(env, local_names, test);
            bind_local_types_in_body(env, local_names, body);
        }
        HirStmt::ForRange { var, body, .. } => {
            env.bind(var.clone(), Ty::Int);
            bind_local_types_in_body(env, local_names, body);
        }
        HirStmt::ForList { var, list, body } => {
            if let Some(BindingState::Definitely(Ty::List(elt_ty))) = env.binding_state(list) {
                env.bind(var.clone(), (**elt_ty).clone());
            }
            bind_local_types_in_body(env, local_names, body);
        }
        // PEP 572 (#774): a bare expression statement is the other
        // placement `violates_walrus_placement` permits a walrus in
        // (`n := f()` on its own line) -- without a dedicated arm this fell
        // into the catch-all below and its walrus target was never
        // pre-bound.
        HirStmt::ExprStmt(expr) => bind_named_expr_types_in_expr(env, local_names, expr),
        // #1445: every suite of a `try`/`try`-`except*`, in source order,
        // flat like an `if`'s two arms -- the body, each handler's body, the
        // `else` and the `finally`. A handler's `as` name is not bound: its
        // type is the checker's to decide, and leaving it unbound costs a
        // missed resolution, never a wrong one. Without this arm a name bound
        // in any suite was invisible to the empty-container pre-pass, so
        // lark's `size = len(rule.expansion)` after `action, arg = ...` inside
        // a `try` never typed, and neither did `s = value_stack[-size:]`.
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
        } => {
            bind_local_types_in_body(env, local_names, body);
            for handler in handlers {
                bind_local_types_in_body(env, local_names, &handler.body);
            }
            bind_local_types_in_body(env, local_names, orelse);
            bind_local_types_in_body(env, local_names, finalbody);
        }
        _ => {}
    }
}

/// Best-effort counterpart to a dedicated walk: reuses the already-exhaustive
/// `collect_named_expr_bindings` (`lib.rs`) to find and bind every
/// `HirExpr::NamedExpr` reachable from `expr` at any depth, exactly as
/// `bind_local_types_in_stmt`'s `Assign`/`AnnAssign` arms already bind their
/// own targets. Its `Result` is discarded rather than propagated, matching
/// every other binding attempt in this pass -- this walk only grows `env`
/// for later resolution, it never validates, and `collect_named_expr_bindings`
/// itself still binds every target it reaches before returning any error
/// for a *later* sibling, so a discarded `Err` does not lose an earlier
/// successful binding.
fn bind_named_expr_types_in_expr(env: &mut Environment, local_names: &[&str], expr: &HirExpr) {
    let _ = collect_named_expr_bindings(env, local_names, expr);
}
