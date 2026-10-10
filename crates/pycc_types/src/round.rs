//! The `round()` builtin (CPython's `float.__round__` / `int.__round__`).
//!
//! Admitted shapes: `round(x)` for an `int`, `bool` or `float` `x`, which is
//! an `int`, and `round(x, ndigits)` for a `float` `x` and an `int`/`bool`
//! `ndigits`, which is a `float`. `round(int, ndigits)` and every other
//! argument shape stay `C0001`.

use pycc_diag::{Diagnostic, Span};
use pycc_hir::Ty;

/// The result type of `round(*args)` for argument types `arg_tys`.
pub(crate) fn round_call_ty(arg_tys: &[Ty]) -> Result<Ty, Diagnostic> {
    let is_int = |t: &Ty| matches!(t, Ty::Int | Ty::Bool);
    match arg_tys {
        [x] if is_int(x) || *x == Ty::Float => Ok(Ty::Int),
        [Ty::Float, ndigits] if is_int(ndigits) => Ok(Ty::Float),
        _ => Err(Diagnostic::error(
            "C0001",
            format!(
                "`round()` of ({}) is not supported yet -- only `round(int)`, \
                 `round(float)` and `round(float, int)` are",
                arg_tys.iter().map(Ty::name).collect::<Vec<_>>().join(", ")
            ),
            Span::new(0, 0),
        )),
    }
}
