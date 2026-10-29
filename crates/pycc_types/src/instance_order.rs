//! `sorted()`, `min()`, `max()` and `list.sort()` over class instances
//! (ROADMAP v0.4 accept 5): the elements compare through the class's
//! `__lt__` (a sort, `min`) or `__gt__` (`max`), each falling back to the
//! other one reflected as CPython's rich comparison does. `pycc_mir`
//! resolves the same method (`pycc_mir::expr::instance_order`) and
//! `pycc_codegen::instance_order` drives the runtime's ordering with it.

use crate::Environment;
use pycc_diag::{Diagnostic, Span};
use pycc_hir::{HirExpr, Ty};

/// Whether instances of `class` order: a method of its MRO named `__lt__`
/// or `__gt__` takes another instance of `class` and returns `bool`.
fn orders(env: &Environment, class: &str) -> Result<(), Diagnostic> {
    let receiver = Ty::Instance(Box::new(class.to_string()));
    let declared = |method: &str| {
        env.lookup_class(class).is_some_and(|def| {
            def.mro.iter().any(|mro_class| {
                env.lookup_class(mro_class)
                    .is_some_and(|mro_def| mro_def.methods.iter().any(|(name, _)| name == method))
            })
        })
    };
    let Some(method) = ["__lt__", "__gt__"].into_iter().find(|m| declared(m)) else {
        return Err(Diagnostic::error(
            "T0021",
            format!(
                "instances of `{class}` cannot be ordered: the class defines neither `__lt__` nor \
                 `__gt__`"
            ),
            Span::new(0, 0),
        )
        .with_help("define `__lt__` (or use `@dataclass(order=True)`), or pass `key=`"));
    };
    let other = HirExpr::Name("self".to_string());
    match crate::class::resolve_method_call(
        env,
        &receiver,
        method,
        std::slice::from_ref(&other),
        std::slice::from_ref(&receiver),
    ) {
        Ok(Ty::Bool) => Ok(()),
        Ok(other) => Err(Diagnostic::error(
            "T0021",
            format!(
                "`{class}.{method}` returns `{}`, so instances of `{class}` cannot be ordered \
                 natively",
                other.name()
            ),
            Span::new(0, 0),
        )),
        Err(diagnostic) => Err(diagnostic),
    }
}

/// The element class of `ty` when it is a list of instances of a user
/// class.
fn instance_list(env: &Environment, ty: &Ty) -> Option<String> {
    match ty {
        Ty::List(elem) => match elem.as_ref() {
            Ty::Instance(class)
                if env.lookup_class(class).is_some()
                    && !pycc_hir::is_builtin_exception_class(class) =>
            {
                Some(class.to_string())
            }
            _ => None,
        },
        _ => None,
    }
}

/// Whether `pair` is a `(key, element)` pair whose key is an immutable
/// builtin value (numbers, `bool`, `str`, tuples of them), compared without
/// running any code.
pub(crate) fn inert_pair(pair: &Ty) -> bool {
    matches!(pair, Ty::Tuple(items) if items.len() == 2 && pycc_hir::builtins::inert(&items[0]))
}

/// The `(key class, element)` of a list of `(key, element)` pairs (or, for
/// `min`/`max`, a generator of them) whose key is an instance of a user
/// class (`pycc_ast::desugar::keys`).
fn instance_pairs(env: &Environment, ty: &Ty) -> Option<(String, Ty)> {
    let pair = match ty {
        Ty::List(pair) => pair.as_ref(),
        Ty::Generator(sig) => &sig.0,
        _ => return None,
    };
    match pair {
        Ty::Tuple(items) if items.len() == 2 => {
            let class = instance_list(env, &Ty::List(Box::new(items[0].clone())))?;
            Some((class, items[1].clone()))
        }
        _ => None,
    }
}

/// The type of `sorted(xs)`, `min(xs)`, `max(xs)` of a list of instances,
/// or `min(a, b, ...)`/`max(a, b, ...)` of instances of one class, or of
/// the `key=`/`reverse=` forms (`__pycc_*_keyed`) over instance keys;
/// `None` for any other call.
pub(crate) fn call_ty(env: &Environment, callee: &str, arg_tys: &[Ty]) -> Option<Result<Ty, Diagnostic>> {
    match (callee, arg_tys) {
        ("__pycc_sorted_keyed", [pairs, Ty::Bool]) => {
            let (class, elem) = instance_pairs(env, pairs)?;
            Some(orders(env, &class).map(|()| Ty::List(Box::new(elem))))
        }
        ("__pycc_sort_keyed", [Ty::List(_), pairs, Ty::Bool]) => {
            let (class, _) = instance_pairs(env, pairs)?;
            Some(orders(env, &class).map(|()| Ty::None))
        }
        ("__pycc_min_keyed" | "__pycc_max_keyed", [pairs]) => {
            let (class, elem) = instance_pairs(env, pairs)?;
            Some(orders(env, &class).map(|()| elem))
        }
        ("sorted" | "min" | "max", [list]) => {
            let class = instance_list(env, list)?;
            Some(orders(env, &class).map(|()| {
                if callee == "sorted" {
                    list.clone()
                } else {
                    Ty::Instance(Box::new(class))
                }
            }))
        }
        ("min" | "max", [Ty::Instance(first), rest @ ..])
            if !rest.is_empty()
                && rest.iter().all(|t| matches!(t, Ty::Instance(c) if c == first))
                && env.lookup_class(first).is_some()
                && !pycc_hir::is_builtin_exception_class(first) =>
        {
            Some(orders(env, first).map(|()| arg_tys[0].clone()))
        }
        _ => None,
    }
}

/// The type of `xs.sort()` on a list of instances; `None` for any other
/// method call.
pub(crate) fn method_ty(
    env: &Environment,
    base_ty: &Ty,
    method: &str,
    arg_tys: &[Ty],
) -> Option<Result<Ty, Diagnostic>> {
    if method != "sort" || !arg_tys.is_empty() {
        return None;
    }
    let class = instance_list(env, base_ty)?;
    Some(orders(env, &class).map(|()| Ty::None))
}
