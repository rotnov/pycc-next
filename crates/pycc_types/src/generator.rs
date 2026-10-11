//! Generator functions: checking a `yield` against the generator's element
//! type.
//!
//! `pycc_hir` lowers a `yield value` statement to a call of
//! [`pycc_hir::YIELD_MARKER`] and gives the function the return type
//! `Generator[T]` (annotated, or `Generator[<inferred>]` for the solver to
//! fill from the yielded values).

use crate::{Environment, infer_expr_in};
use pycc_diag::{Diagnostic, Span};
use pycc_hir::{HirExpr, Ty};

/// Checks `yield args[0]` in a function returning `return_ty`.
pub(crate) fn check_yield(
    env: &Environment,
    local_names: &[&str],
    args: &[HirExpr],
    return_ty: &Ty,
) -> Result<(), Diagnostic> {
    let Ty::Generator(element) = return_ty else {
        return Err(Diagnostic::error(
            "C0001",
            "`yield` is supported only in a function whose return type is a generator",
            Span::new(0, 0),
        ));
    };
    let actual = infer_expr_in(env, local_names, &args[0])?;
    if crate::class::is_assignable_env(env, &actual, element) {
        return Ok(());
    }
    Err(Diagnostic::error(
        "T0022",
        format!(
            "expected a yielded value of type `{}`, got `{}`",
            element.name(),
            actual.name()
        ),
        Span::new(0, 0),
    )
    .with_help(format!("yield a `{}` value", element.name())))
}

/// The builtins that consume a generator argument entirely (or, `any` and
/// `all`, until the first deciding value) and accept a list of its values
/// in its place.
pub fn is_generator_consumer(callee: &str) -> bool {
    matches!(
        callee,
        "list" | "sum" | "min" | "max" | "sorted" | "set" | "frozenset" | "any" | "all" | "tuple"
    )
}

/// `arg_tys` with a generator argument of a consumer seen as the list of
/// its values, or `None` when nothing changes.
pub(crate) fn consumer_arg_tys(callee: &str, arg_tys: &[Ty]) -> Option<Vec<Ty>> {
    if !is_generator_consumer(callee) || !arg_tys.iter().any(|t| matches!(t, Ty::Generator(_))) {
        return None;
    }
    Some(
        arg_tys
            .iter()
            .map(|t| match t {
                Ty::Generator(element) => Ty::List(element.clone()),
                other => other.clone(),
            })
            .collect(),
    )
}

/// `next(g)` / `next(g, default)` of a generator: its element type.
pub(crate) fn next_call(
    callee: &str,
    arg_tys: &[Ty],
    env: &Environment,
) -> Result<Option<Ty>, Diagnostic> {
    if callee != "next" {
        return Ok(None);
    }
    let Some(Ty::Generator(element)) = arg_tys.first() else {
        return Ok(None);
    };
    match arg_tys {
        [_] => Ok(Some((**element).clone())),
        [_, default] if crate::class::is_assignable_env(env, default, element) => {
            Ok(Some((**element).clone()))
        }
        [_, default] => Err(Diagnostic::error(
            "T0021",
            format!(
                "the default of `next()` on a `{}` must be a `{}`, got `{}`",
                arg_tys[0].name(),
                element.name(),
                default.name()
            ),
            Span::new(0, 0),
        )),
        _ => Err(Diagnostic::error(
            "T0033",
            format!("`next` expects 1 or 2 arguments, got {}", arg_tys.len()),
            Span::new(0, 0),
        )),
    }
}
