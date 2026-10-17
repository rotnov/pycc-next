//! `for x in name:` (`HirStmt::ForList`): the loop node is picked by the
//! iterable's type -- a list, `str`, homogeneous tuple, dict, set,
//! generator or CPython object.

use super::{MirExpr, MirStmt, bind, bind_variable, lookup, lower_loop_body};
use crate::HirClassDef;
use pycc_hir::{HirStmt, Ty};
use std::collections::HashMap;

/// Lowers `for var in list: body`.
pub(super) fn lower_for_list(
    var: &String,
    list: &String,
    body: &[HirStmt],
    scopes: &mut Vec<HashMap<String, Ty>>,
    classes: &HashMap<String, HirClassDef>,
    current_class: Option<&str>,
) -> MirStmt {
    // The loop variable's type is `list`'s element (or, for a
    // dict-typed binding, key) type, derived via the same `lookup`
    // mechanism every other name reference in this crate uses --
    // mirroring `pycc_types::check_stmt`'s own `ForList` arm
    // (`check_assignment(env, var, *elem_ty)` / `check_assignment(env,
    // var, kv.0)` / `check_assignment(env, var, *elem_ty)` for the
    // set case), not hardcoded to `Ty::Int`. Empirically only a
    // `Ty::List(Box::new(Ty::Int))`, `Ty::Dict(Box::new((Ty::Str,
    // Ty::Int)))`, `Ty::Set(Box::new(Ty::Int))`, or
    // `Ty::FrozenSet(Box::new(Ty::Int))` binding, or (#1343) a set or
    // frozenset of a user-class instance, ever reaches this arm today (`pycc_types`' T0034/T0036/T0037/T0038
    // gates reject every other element/key-value combination before
    // HIR ever constructs one -- see those gates' own comments and
    // this crate's own genericity tests), but deriving here keeps
    // this lowering correct on its own terms rather than baking in
    // an assumption this crate has no way to verify independently.
    // `HirStmt::ForList` is reused unconditionally by `pycc_hir`'s
    // own lowering for any bare-name iterable, dict, list, or set
    // alike (it has no type information to pick a different node)
    // -- this is the point where the real type is resolved and
    // where a dict- or set-typed binding is routed into
    // `MirStmt::ForDict`/`MirStmt::ForSet`, and an object-typed one
    // into `MirStmt::ForObject` (#1325), instead of
    // `MirStmt::ForList`, mirroring `lower_expr`'s own
    // `HirExpr::Subscript` arm doing the same list/dict routing for
    // reads (subscripting a set is rejected earlier, by
    // `pycc_types`' own T0033, so there is no set counterpart there).
    // A narrowed pointer-represented `T | None` iterates as its `T` (the
    // slot already holds the `T` pointer).
    let list_ty = match lookup(scopes, list) {
        Ty::Optional(inner)
            if pycc_hir::is_pointer_optional_inner(&inner)
                && crate::narrowed_ty(scopes, list).is_some() =>
        {
            *inner
        }
        other => other,
    };
    match list_ty {
        Ty::List(elem_ty) | Ty::VarTuple(elem_ty) => {
            bind_variable(scopes, var.clone(), (*elem_ty).clone());
            // D-068 re-review of #780 (sixth round): see the
            // `ForRange` arm's identical comment above -- the loop
            // variable's own rebinding must kill a stale narrowing
            // sentinel too.
            crate::kill_narrowing(scopes, var);
            let body = lower_loop_body(body, scopes, classes, current_class);
            MirStmt::ForList {
                var: var.clone(),
                var_ty: *elem_ty,
                list: list.clone(),
                body,
            }
        }
        Ty::Str => {
            // `for c in s`: an index loop over the code points of the
            // string the loop started with (held in a synthetic name, a
            // digit-led name no source can spell, as CPython's iterator
            // holds it), reading one character per trip: no list of every
            // character is built first, which an early `break` would waste.
            let text = format!("0chars_{var}_{list}");
            let index = format!("0index_{var}_{list}");
            bind_variable(scopes, text.clone(), Ty::Str);
            bind_variable(scopes, index.clone(), Ty::Int);
            bind_variable(scopes, var.clone(), Ty::Str);
            crate::kill_narrowing(scopes, var);
            let lowered = lower_loop_body(body, scopes, classes, current_class);
            let name = |name: &str, ty: Ty| MirExpr::Name {
                name: name.to_string(),
                ty,
            };
            let len = crate::RtCallExpr {
                sig: pycc_hir::builtins::resolve_call("len", &[Ty::Str])
                    .and_then(Result::ok)
                    .expect("`len(str)` is a table builtin"),
                args: vec![name(&text, Ty::Str)],
            };
            let mut body = vec![MirStmt::Assign {
                target: var.clone(),
                value: crate::expr::builtin_call::lower_str_getitem(name(&text, Ty::Str), name(&index, Ty::Int)),
            }];
            body.extend(lowered);
            MirStmt::Seq(vec![
                MirStmt::Assign {
                    target: text.clone(),
                    value: name(list, Ty::Str),
                },
                MirStmt::ForRange {
                    var: index,
                    start: MirExpr::IntLiteral(0),
                    stop: MirExpr::RtCall(Box::new(len)),
                    step: MirExpr::IntLiteral(1),
                    body,
                },
            ])
        }
        // `for x in t` over a tuple whose elements share one type: the
        // loop runs over the list of its elements, bound to a synthetic
        // name as for a `str`.
        Ty::Tuple(items) if pycc_hir::homogeneous_tuple_element(&items).is_some() => {
            let elem_ty = items[0].clone();
            let elements = format!("0items_{var}_{list}");
            bind_variable(
                scopes,
                elements.clone(),
                Ty::List(Box::new(elem_ty.clone())),
            );
            bind_variable(scopes, var.clone(), elem_ty.clone());
            crate::kill_narrowing(scopes, var);
            let body = lower_loop_body(body, scopes, classes, current_class);
            let source = MirExpr::Name {
                name: list.clone(),
                ty: Ty::Tuple(items),
            };
            MirStmt::Seq(vec![
                MirStmt::Assign {
                    target: elements.clone(),
                    value: crate::tuple_to_list(source),
                },
                MirStmt::ForList {
                    var: var.clone(),
                    var_ty: elem_ty,
                    list: elements,
                    body,
                },
            ])
        }
        Ty::Dict(kv) => {
            bind_variable(scopes, var.clone(), kv.0.clone());
            // D-068 re-review of #780 (sixth round): see the
            // `ForRange` arm's identical comment above.
            crate::kill_narrowing(scopes, var);
            let body = lower_loop_body(body, scopes, classes, current_class);
            MirStmt::ForDict {
                var: var.clone(),
                var_ty: kv.0,
                dict: list.clone(),
                body,
            }
        }
        // `for x in s:` (PR-11 Task 8, D-123): iterates a set's own
        // elements, binding the loop variable as the set's element
        // type -- mirrors the `Ty::Dict` arm immediately above,
        // which mirrors `pycc_types::check_stmt`'s own identical
        // `Ty::Set(elem_ty) => *elem_ty` arm (added in that crate's
        // Task 7 fix round).
        Ty::Set(elem_ty) | Ty::FrozenSet(elem_ty) => {
            let var_ty = *elem_ty;
            bind_variable(scopes, var.clone(), var_ty.clone());
            // D-068 re-review of #780 (sixth round): see the
            // `ForRange` arm's identical comment above.
            crate::kill_narrowing(scopes, var);
            let body = lower_loop_body(body, scopes, classes, current_class);
            MirStmt::ForSet {
                var: var.clone(),
                var_ty,
                set: list.clone(),
                body,
            }
        }
        // `for x in gen:` over a generator: the loop holds its own
        // reference in a hidden local (a digit-led name no source
        // can spell), taken over from a desugaring temporary bound
        // only for this loop (`_pycc_N`, `pycc_ast::desugar`).
        Ty::Generator(sig) => {
            let elem_ty = sig.0.clone();
            let hold = format!("0genhold_{var}_{list}");
            bind_variable(scopes, hold.clone(), Ty::Generator(sig.clone()));
            bind_variable(scopes, var.clone(), elem_ty.clone());
            crate::kill_narrowing(scopes, var);
            let body = lower_loop_body(body, scopes, classes, current_class);
            MirStmt::ForGen {
                var: var.clone(),
                var_ty: elem_ty,
                generator: list.clone(),
                hold,
                consume: list.starts_with("_pycc_"),
                body,
            }
        }
        // #1325: a bare name bound to a CPython object is the
        // bare-name form of `HirStmt::ForObject` below, so it lowers
        // to the same node with a `Name` iterable. `bind`, for the
        // reason that arm's comment gives.
        Ty::Object => {
            let iter = MirExpr::Name {
                name: list.clone(),
                ty: Ty::Object,
            };
            bind(scopes, var.clone(), Ty::Object);
            crate::kill_narrowing(scopes, var);
            let body = lower_loop_body(body, scopes, classes, current_class);
            MirStmt::ForObject {
                var: var.clone(),
                iter,
                body,
            }
        }
        other => panic!(
            "pycc_mir: internal error: `{list}` is neither a list, dict, set, nor frozenset (found `{}`) -- pycc_types::check should have rejected this HIR before it reached pycc_mir",
            other.name()
        ),
    }
}
