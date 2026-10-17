//! The numeric builtins over native values: `abs(x)`, `divmod(a, b)` and
//! `int(x)`.
//!
//! - `abs` of an `int`/`bool` is an `int`, of a `float` a `float`.
//! - `divmod` of two `int`/`bool` operands is `tuple[int, int]`.
//! - `int` of an `int`, `bool` or `float` is an `int` (a CPython object
//!   argument keeps its own arm in `crate::expr`/`crate::constraints`).
//!
//! Every other argument shape returns `None` so the caller's existing
//! handling (`C0001` for an unsupported builtin) still applies. Both type
//! paths -- the public-body check and the private-helper solver -- call
//! [`result_ty`] with the argument types they have resolved, so they cannot
//! disagree.

use pycc_diag::{Diagnostic, Span};
use pycc_hir::Ty;

/// The builtins this module types.
pub(crate) const NAMES: [&str; 3] = ["abs", "divmod", "int"];

fn int_like(ty: &Ty) -> bool {
    matches!(ty, Ty::Int | Ty::Bool)
}

/// The result type of `callee(*arg_tys)`, `Some(Err(T0021))` for a wrong
/// argument count, or `None` when the call is not one this module admits.
pub(crate) fn result_ty(callee: &str, arg_tys: &[Ty]) -> Option<Result<Ty, Diagnostic>> {
    let arity = if callee == "divmod" { 2 } else { 1 };
    if !NAMES.contains(&callee) {
        return None;
    }
    // `int(x, base)` is valid Python this compiler does not implement, so it
    // stays the caller's `C0001` rather than an arity error.
    if callee == "int" && arg_tys.len() == 2 {
        return None;
    }
    if arg_tys.len() != arity {
        return Some(Err(Diagnostic::error(
            "T0021",
            format!(
                "`{callee}` expects exactly {arity} argument{}, got {}",
                if arity == 1 { "" } else { "s" },
                arg_tys.len()
            ),
            Span::new(0, 0),
        )
        .with_help(format!(
            "pass exactly {arity} argument{}",
            if arity == 1 { "" } else { "s" }
        ))));
    }
    // An `IntEnum` member is its `int` value (`pycc_hir::int_enum`).
    let arg_tys: Vec<Ty> = arg_tys.iter().map(pycc_hir::int_enum::as_int).collect();
    let ty = match (callee, arg_tys.as_slice()) {
        ("abs", [x]) if int_like(x) => Ty::Int,
        ("abs", [Ty::Float]) => Ty::Float,
        ("abs", [Ty::Num]) => Ty::Num,
        ("divmod", [a, b]) if int_like(a) && int_like(b) => {
            Ty::Tuple(Box::new(vec![Ty::Int, Ty::Int]))
        }
        ("int", [x]) if int_like(x) || matches!(x, Ty::Float | Ty::Num) => Ty::Int,
        _ => return None,
    };
    Some(Ok(ty))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admits_numeric_arguments_and_types_the_result() {
        assert_eq!(result_ty("abs", &[Ty::Bool]).unwrap().unwrap(), Ty::Int);
        assert_eq!(result_ty("abs", &[Ty::Float]).unwrap().unwrap(), Ty::Float);
        assert_eq!(
            result_ty("divmod", &[Ty::Int, Ty::Bool]).unwrap().unwrap(),
            Ty::Tuple(Box::new(vec![Ty::Int, Ty::Int]))
        );
        assert_eq!(result_ty("int", &[Ty::Float]).unwrap().unwrap(), Ty::Int);
    }

    #[test]
    fn leaves_other_shapes_to_the_caller() {
        assert!(result_ty("abs", &[Ty::Str]).is_none());
        assert!(result_ty("divmod", &[Ty::Float, Ty::Int]).is_none());
        assert!(result_ty("int", &[Ty::Object]).is_none());
        assert!(result_ty("len", &[Ty::Int]).is_none());
        assert!(result_ty("int", &[Ty::Float, Ty::Int]).is_none());
    }

    #[test]
    fn a_wrong_argument_count_is_t0021() {
        let err = result_ty("divmod", &[Ty::Int]).unwrap().unwrap_err();
        assert_eq!(err.code, "T0021");
        assert!(
            err.message.contains("exactly 2 arguments, got 1"),
            "{}",
            err.message
        );
        let err = result_ty("abs", &[]).unwrap().unwrap_err();
        assert!(
            err.message.contains("exactly 1 argument, got 0"),
            "{}",
            err.message
        );
    }
}
