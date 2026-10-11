//! Function values (`Ty::Callable`): a module-level function used as a
//! value, a nested function closed over its enclosing variables
//! (`pycc_hir::CLOSURE_MARKER`), and a call of a variable holding one.

use crate::{Environment, infer_expr_in};
use pycc_diag::{Diagnostic, Span};
use pycc_hir::{HirExpr, Ty};

fn unsupported(message: impl Into<String>) -> Diagnostic {
    Diagnostic::error("C0001", message, Span::new(0, 0))
}

/// Whether a value of type `ty` can be captured by a function value: one
/// word (not a struct-by-value tuple or optional, not a buffer view).
fn capturable(ty: &Ty) -> bool {
    match ty {
        Ty::Tuple(elements) => pycc_hir::tuple_is_boxed(elements),
        Ty::Optional(_) | Ty::MemoryView | Ty::Infer | Ty::Param(_) => false,
        _ => true,
    }
}

/// The function value of module-level function `name`, if `name` is one.
pub(crate) fn function_value(env: &Environment, name: &str) -> Option<Result<Ty, Diagnostic>> {
    let (params, ret) = env.lookup_function(name)?;
    let ty = Ty::Callable(Box::new((params.clone(), ret.clone())));
    if crate::empty_container::contains_infer(&ty) {
        return Some(Err(unsupported(format!(
            "the function `{name}` is used as a value, but its signature is inferred -- \
             annotate its parameters and return type"
        ))));
    }
    Some(Ok(ty))
}

/// `__pycc_closure__(f, captures...)`: the function value of `f` with its
/// first parameters bound to `captures`.
pub(crate) fn closure_value(
    env: &Environment,
    local_names: &[&str],
    args: &[HirExpr],
) -> Result<Ty, Diagnostic> {
    let [HirExpr::Name(function), captures @ ..] = args else {
        return Err(unsupported("a closure marker names its function first"));
    };
    let Some((params, ret)) = env.lookup_function(function) else {
        return Err(unsupported(format!("`{function}` is not a function")));
    };
    for (capture, param) in captures.iter().zip(params) {
        let actual = infer_expr_in(env, local_names, capture)?;
        if !capturable(&actual) {
            return Err(unsupported(format!(
                "a nested function used as a value capturing a `{}` is not supported yet",
                actual.name()
            )));
        }
        if !crate::class::is_assignable_env(env, &actual, param) {
            return Err(Diagnostic::error(
                "T0021",
                format!(
                    "a nested function used as a value captures a `{}` where `{}` is expected",
                    actual.name(),
                    param.name()
                ),
                Span::new(0, 0),
            ));
        }
    }
    let ty = Ty::Callable(Box::new((params[captures.len()..].to_vec(), ret.clone())));
    if crate::empty_container::contains_infer(&ty) {
        return Err(unsupported(
            "a nested function used as a value needs annotated parameters (and a return \
             type its body determines)",
        ));
    }
    Ok(ty)
}

/// `callee(args)` of a variable holding a function value of type `sig`.
pub(crate) fn call_value(
    env: &Environment,
    local_names: &[&str],
    callee: &str,
    sig: &(Vec<Ty>, Ty),
    args: &[HirExpr],
) -> Result<Ty, Diagnostic> {
    if args.len() != sig.0.len() {
        return Err(Diagnostic::error(
            "T0033",
            format!(
                "`{callee}` takes {} argument{} but {} were given",
                sig.0.len(),
                if sig.0.len() == 1 { "" } else { "s" },
                args.len()
            ),
            Span::new(0, 0),
        ));
    }
    for (index, (arg, param)) in args.iter().zip(&sig.0).enumerate() {
        let actual = infer_expr_in(env, local_names, arg)?;
        if !crate::class::is_assignable_env(env, &actual, param) {
            return Err(Diagnostic::error(
                "T0021",
                format!(
                    "argument {} of `{callee}` expects `{}`, got `{}`",
                    index + 1,
                    param.name(),
                    actual.name()
                ),
                Span::new(0, 0),
            ));
        }
    }
    Ok(sig.1.clone())
}
