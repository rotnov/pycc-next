//! Comprehension and loop iteration sources: [`resolve_comp_source`] and
//! the conversions that turn a `str`, a generator or a homogeneous tuple
//! into the list a loop walks.

use super::{
    CompSource, HirClassDef, IndexUse, MirExpr, RtCallExpr, bind_variable, index_operand,
    kill_narrowing, lookup, lower_expr,
};
use pycc_hir::{CompIter, Ty};
use std::collections::HashMap;

/// Resolves a `pycc_hir::CompIter` into a fully-typed `CompSource`,
/// lowering any range sub-expressions and binding `var`'s type into
/// `scopes` -- mirrors `HirStmt::ForList`'s own resolution exactly
/// (`lower_stmt`'s existing `ForList` arm), reused via this shared helper
/// rather than duplicated three times (once per comprehension kind, PR-12,
/// D-117). Takes `scopes` as `&mut [HashMap<String, Ty>]` (a slice), not
/// `&mut Vec<..>` like `lower_stmt`/`lower_item` -- unlike those two, this
/// helper never pushes/pops a scope itself, so `clippy::ptr_arg` genuinely
/// flags an owned `Vec` parameter here as unnecessary. `lower_item`'s
/// `&mut Vec` is a real need: its `Function` arm calls `scopes.push(..)`
/// before lowering the function body and `scopes.pop()` after, and
/// `&mut [_]` has no such methods. `lower_stmt` itself never calls a
/// `Vec`-only method on `scopes` either (every other callee it passes
/// `scopes` to -- `lower_expr`, `bind_variable`, `lookup`,
/// `resolve_comp_source` -- already takes a slice); it is self-recursive
/// (it calls itself for nested statement bodies, e.g. `If`/`While`), not
/// mutually recursive with `lower_item` (`lower_item` calls `lower_stmt`,
/// but `lower_stmt` never calls back into `lower_item`). `clippy::ptr_arg`
/// does not fire on a self-recursive function, so nothing forces or even
/// flags `lower_stmt`'s own `&mut Vec` as unnecessary; it keeps the owned
/// type by convention, matching its only caller, `lower_item`, rather than
/// out of its own requirement.
pub(crate) fn resolve_comp_source(
    iter: &CompIter,
    var: &str,
    scopes: &mut [HashMap<String, Ty>],
    classes: &HashMap<String, HirClassDef>,
    current_class: Option<&str>,
) -> (CompSource, Ty) {
    match iter {
        CompIter::Range { start, stop, step } => {
            let range_operand = |e| index_operand(e, IndexUse::Range);
            let start = range_operand(lower_expr(start, scopes, classes, current_class));
            let stop = range_operand(lower_expr(stop, scopes, classes, current_class));
            let step = range_operand(lower_expr(step, scopes, classes, current_class));
            bind_variable(scopes, var.to_string(), Ty::Int);
            // D-068 re-review of #780 (sixth round): a comprehension's own
            // loop variable is a rebinding exactly like `ForRange`'s
            // induction variable (see that arm's identical comment in
            // `stmt.rs`) -- must kill a stale narrowing sentinel too,
            // mirroring the checker's `check_assignment(env, var, Ty::Int)`
            // for the equivalent `ListCompAssign`/`SetCompAssign`/
            // `DictCompAssign` arms.
            kill_narrowing(scopes, var);
            (CompSource::Range { start, stop, step }, Ty::Int)
        }
        CompIter::Name(name) => match lookup(scopes, name) {
            Ty::List(elem_ty) => {
                bind_variable(scopes, var.to_string(), (*elem_ty).clone());
                kill_narrowing(scopes, var);
                (CompSource::List(name.clone()), *elem_ty)
            }
            Ty::Dict(kv) => {
                bind_variable(scopes, var.to_string(), kv.0.clone());
                kill_narrowing(scopes, var);
                (CompSource::Dict(name.clone()), kv.0)
            }
            Ty::Set(elem_ty) | Ty::FrozenSet(elem_ty) => {
                bind_variable(scopes, var.to_string(), (*elem_ty).clone());
                kill_narrowing(scopes, var);
                (CompSource::Set(name.clone()), *elem_ty)
            }
            Ty::Str => {
                bind_variable(scopes, var.to_string(), Ty::Str);
                kill_narrowing(scopes, var);
                let name = MirExpr::Name {
                    name: name.clone(),
                    ty: Ty::Str,
                };
                (CompSource::ListExpr(str_to_list(name)), Ty::Str)
            }
            // A tuple whose elements share one type iterates the list of
            // its elements.
            Ty::Tuple(items) if pycc_hir::homogeneous_tuple_element(&items).is_some() => {
                let elem = items[0].clone();
                bind_variable(scopes, var.to_string(), elem.clone());
                kill_narrowing(scopes, var);
                let name = MirExpr::Name {
                    name: name.clone(),
                    ty: Ty::Tuple(items),
                };
                (CompSource::ListExpr(tuple_to_list(name)), elem)
            }
            // A generator is consumed into the list of its values first.
            Ty::Generator(sig) => {
                let elem = sig.0.clone();
                bind_variable(scopes, var.to_string(), elem.clone());
                kill_narrowing(scopes, var);
                let generator = MirExpr::Name {
                    name: name.clone(),
                    ty: Ty::Generator(sig),
                };
                (CompSource::ListExpr(gen_to_list(generator)), elem)
            }
            // Non-strict mode: a name bound to a CPython object.
            Ty::Object => {
                bind_variable(scopes, var.to_string(), Ty::Object);
                kill_narrowing(scopes, var);
                let name = MirExpr::Name {
                    name: name.clone(),
                    ty: Ty::Object,
                };
                (CompSource::Object(name), Ty::Object)
            }
            other => panic!(
                "pycc_mir: internal error: `{name}` is neither a list, dict, set, nor frozenset (found `{}`) -- pycc_types::check should have rejected this HIR before it reached pycc_mir",
                other.name()
            ),
        },
        // Part 1 of #1255: `pycc_types` proved the iterable a CPython
        // object, so the loop variable is one too. The iterable is lowered
        // before `var` is bound, so it reads only the enclosing bindings.
        CompIter::Iterable(iterable) => {
            let iterable = lower_expr(iterable, scopes, classes, current_class);
            let (source, elem) = match iterable.ty() {
                Ty::List(elem) => (CompSource::ListExpr(iterable), *elem),
                Ty::Str => (CompSource::ListExpr(str_to_list(iterable)), Ty::Str),
                // `tuple_to_list` reads the tuple once per element: a
                // subscript or attribute read has no side effect.
                Ty::Tuple(items) if pycc_hir::homogeneous_tuple_element(&items).is_some() => {
                    let elem = items[0].clone();
                    (CompSource::ListExpr(tuple_to_list(iterable)), elem)
                }
                Ty::Generator(sig) => {
                    let elem = sig.0.clone();
                    (CompSource::ListExpr(gen_to_list(iterable)), elem)
                }
                _ => (CompSource::Object(iterable), Ty::Object),
            };
            bind_variable(scopes, var.to_string(), elem.clone());
            kill_narrowing(scopes, var);
            (source, elem)
        }
    }
}

/// `list(g)` of a generator: the iteration source of a comprehension over
/// it, consumed first as `sum`/`sorted` consume one.
pub(crate) fn gen_to_list(generator: MirExpr) -> MirExpr {
    let Ty::Generator(sig) = generator.ty() else {
        panic!("pycc_mir: internal error: gen_to_list of a non-generator");
    };
    MirExpr::GenToList {
        generator: Box::new(generator),
        ty: Ty::List(Box::new(sig.0)),
    }
}

/// `list(s)` of a `str`: the iteration source for `for c in s`.
pub(crate) fn str_to_list(s: MirExpr) -> MirExpr {
    MirExpr::RtCall(Box::new(RtCallExpr {
        sig: pycc_hir::builtins::resolve_call("list", &[Ty::Str])
            .and_then(Result::ok)
            .expect("`list(str)` is a table builtin"),
        args: vec![s],
    }))
}

/// The list of a tuple's elements, `[t[0], t[1], ...]`: the iteration
/// source for `for x in t` over a tuple whose elements share one type
/// (`pycc_hir::homogeneous_tuple_element`). `t` is read once per element,
/// so it is a name.
pub(crate) fn tuple_to_list(tuple: MirExpr) -> MirExpr {
    let Ty::Tuple(items) = tuple.ty() else {
        panic!("pycc_mir: internal error: tuple_to_list of a non-tuple");
    };
    MirExpr::ListLiteral(
        (0..items.len())
            .map(|position| MirExpr::Subscript {
                base: Box::new(tuple.clone()),
                index: Box::new(MirExpr::IntLiteral(position as i64)),
            })
            .collect(),
    )
}
