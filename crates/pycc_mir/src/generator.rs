//! Generators: the builtins that consume a generator argument.
//!
//! `pycc_types` types `next(g)` as the generator's element type and the
//! consumers `pycc_types::generator::is_generator_consumer` names as if
//! given the list of the generator's values. `next`, `list`, `any` and `all`
//! get their own nodes; every other consumer receives that list
//! (`MirExpr::GenToList`), which it consumes entirely anyway.

use crate::{HirClassDef, MirExpr, MirStmt, Ty, lower_expr};
use pycc_hir::{HirExpr, HirStmt};
use std::collections::HashMap;

/// The lowering of builtin call `callee(args)` when its first argument is a
/// generator, or the arguments back (a generator argument of a consumer
/// wrapped in `GenToList`) for the ordinary lowering to continue with.
pub(crate) fn lower_generator_consumer(
    callee: &str,
    mut args: Vec<MirExpr>,
) -> Result<MirExpr, Vec<MirExpr>> {
    if !args.iter().any(|arg| matches!(arg.ty(), Ty::Generator(_))) {
        return Err(args);
    }
    let first = args.first().map(MirExpr::ty);
    match (callee, first) {
        ("next", Some(Ty::Generator(element))) => {
            let mut args = args.into_iter();
            let generator = Box::new(args.next().expect("checked above"));
            Ok(MirExpr::GenNext {
                generator,
                default: args.next().map(Box::new),
                ty: element.0,
            })
        }
        ("list", Some(Ty::Generator(element))) if args.len() == 1 => Ok(MirExpr::GenToList {
            generator: Box::new(args.remove(0)),
            ty: Ty::List(Box::new(element.0)),
        }),
        ("any" | "all", Some(Ty::Generator(_))) if args.len() == 1 => Ok(MirExpr::GenAnyAll {
            generator: Box::new(args.remove(0)),
            all: callee == "all",
        }),
        _ if is_consumer(callee) => Err(args
            .into_iter()
            .map(|arg| match arg.ty() {
                // `min`/`max` of a generator of CPython objects take it as
                // a CPython iterator (`pycc_hir::builtins::resolve_call`).
                Ty::Generator(element)
                    if matches!(callee, "min" | "max") && element.0 == Ty::Object =>
                {
                    arg
                }
                Ty::Generator(element) => MirExpr::GenToList {
                    generator: Box::new(arg),
                    ty: Ty::List(Box::new(element.0)),
                },
                _ => arg,
            })
            .collect()),
        _ => Err(args),
    }
}

/// `pycc_types::generator::is_generator_consumer`'s list (this crate does
/// not depend on `pycc_types`).
fn is_consumer(callee: &str) -> bool {
    matches!(
        callee,
        "list"
            | "sum"
            | "min"
            | "max"
            | "sorted"
            | "set"
            | "frozenset"
            | "any"
            | "all"
            | "tuple"
            | "__pycc_counter_of"
    )
}

/// The `scopes` key (in a generator function's own frame) holding its
/// `Ty::Generator` type; `$` keeps it apart from every source name.
pub(crate) const GENERATOR_KEY: &str = "$generator";

/// The enclosing generator function's `(yield, send, return)` types.
fn enclosing_generator(scopes: &[HashMap<String, Ty>]) -> (Ty, Ty, Ty) {
    match scopes
        .iter()
        .rev()
        .find_map(|scope| scope.get(GENERATOR_KEY))
    {
        Some(Ty::Generator(sig)) => (**sig).clone(),
        _ => panic!(
            "pycc_mir: internal error: a `yield` outside a generator function -- \
             pycc_types should have rejected it"
        ),
    }
}

/// A `yield value` used as a value (`MirExpr::GenYield`) or a `yield from
/// sub` (`MirExpr::GenYieldFrom`).
pub(crate) fn lower_yield_expr(
    callee: &str,
    args: &[HirExpr],
    scopes: &[HashMap<String, Ty>],
    classes: &HashMap<String, HirClassDef>,
    current_class: Option<&str>,
) -> MirExpr {
    let operand = Box::new(lower_expr(&args[0], scopes, classes, current_class));
    if callee == pycc_hir::YIELD_MARKER {
        let (_, sent, _) = enclosing_generator(scopes);
        return MirExpr::GenYield {
            value: operand,
            ty: sent,
        };
    }
    let Ty::Generator(sig) = operand.ty() else {
        panic!(
            "pycc_mir: internal error: `yield from` of a non-generator value -- \
             pycc_types should have rejected it"
        )
    };
    MirExpr::GenYieldFrom {
        sub: operand,
        ty: sig.2,
    }
}

/// `yield from sub` as a statement: delegation to a generator, or -- for a
/// list -- the loop `for v in sub: yield v`, which is what delegating to a
/// list iterator does.
pub(crate) fn lower_yield_from_stmt(
    operand: &HirExpr,
    scopes: &mut Vec<HashMap<String, Ty>>,
    classes: &HashMap<String, HirClassDef>,
    current_class: Option<&str>,
) -> MirStmt {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let value = lower_expr(operand, scopes, classes, current_class);
    match value.ty() {
        Ty::Generator(sig) => MirStmt::ExprStmt(MirExpr::GenYieldFrom {
            sub: Box::new(value),
            ty: sig.2,
        }),
        list_ty @ (Ty::List(_) | Ty::VarTuple(_)) => {
            let element = Box::new(pycc_hir::sequence_element(&list_ty).cloned().expect("a sequence"));
            let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let list = format!("0yieldfrom_list{n}");
            let var = format!("0yieldfrom_item{n}");
            crate::bind_variable(scopes, list.clone(), list_ty.clone());
            crate::bind_variable(scopes, var.clone(), (*element).clone());
            MirStmt::Seq(vec![
                MirStmt::Assign {
                    target: list.clone(),
                    value,
                },
                MirStmt::ForList {
                    var: var.clone(),
                    var_ty: (*element).clone(),
                    list,
                    body: vec![MirStmt::Yield(MirExpr::Name {
                        name: var,
                        ty: *element,
                    })],
                },
            ])
        }
        other => panic!(
            "pycc_mir: internal error: `yield from` of a `{}` -- pycc_types should have \
             rejected it",
            other.name()
        ),
    }
}

/// `gen.send(v)`, `gen.throw(e)` or `gen.close()` on the generator `base`
/// of type `Generator[sig]`.
pub(crate) fn lower_generator_method(
    base: MirExpr,
    sig: &(Ty, Ty, Ty),
    method: &str,
    args: &[HirExpr],
    scopes: &[HashMap<String, Ty>],
    classes: &HashMap<String, HirClassDef>,
    current_class: Option<&str>,
) -> MirExpr {
    let generator = Box::new(base);
    match method {
        "send" => MirExpr::GenSend {
            generator,
            value: Box::new(lower_expr(&args[0], scopes, classes, current_class)),
            ty: sig.0.clone(),
        },
        "throw" => {
            let mut scratch = scopes.to_vec();
            MirExpr::GenThrow {
                generator,
                exception: Box::new(crate::exception::lower_exception_value(
                    &args[0],
                    &mut scratch,
                    classes,
                    current_class,
                )),
                ty: sig.0.clone(),
            }
        }
        "close" => MirExpr::GenClose { generator },
        other => panic!(
            "pycc_mir: internal error: generator method `{other}` -- pycc_types should \
             have rejected it"
        ),
    }
}

/// The `scopes` key recording the `e.value` type of a `StopIteration`
/// handler binding `e`.
fn stop_value_key(name: &str) -> String {
    format!("$stopvalue:{name}")
}

/// The type of `base.value` when `base` names a caught `StopIteration`
/// whose `try` body resumes generators (`pycc_hir::stop_iteration_sources`).
pub(crate) fn stop_value_ty(base: &HirExpr, scopes: &[HashMap<String, Ty>]) -> Option<Ty> {
    let HirExpr::Name(name) = base else {
        return None;
    };
    let key = stop_value_key(name);
    scopes
        .iter()
        .rev()
        .find_map(|scope| scope.get(&key))
        .cloned()
}

/// The `e.value` type of a `StopIteration` caught around the (already
/// lowered) `try` body `body`, when its generators agree on one.
pub(crate) fn try_stop_value_ty(
    body: &[HirStmt],
    scopes: &[HashMap<String, Ty>],
    classes: &HashMap<String, HirClassDef>,
    current_class: Option<&str>,
) -> Option<Ty> {
    let sources = pycc_hir::stop_iteration_sources(body);
    if sources.is_empty() {
        return None;
    }
    let tys: Vec<Ty> = sources
        .into_iter()
        .map(|source| lower_expr(source, scopes, classes, current_class).ty())
        .collect();
    pycc_hir::stop_iteration_value_ty(&tys).ok()
}

/// Records `e.value`'s type for the handler binding `name` of type `ty`
/// when it catches `StopIteration`, else forgets any earlier one.
pub(crate) fn bind_stop_value(
    scopes: &mut [HashMap<String, Ty>],
    name: &str,
    ty: &Ty,
    stop_value: Option<&Ty>,
) {
    let key = stop_value_key(name);
    let top = scopes.last_mut().expect("lowering always has a scope");
    match (ty, stop_value) {
        (Ty::Instance(class), Some(value)) if class.as_str() == "StopIteration" => {
            top.insert(key, value.clone());
        }
        _ => {
            top.remove(&key);
        }
    }
}
