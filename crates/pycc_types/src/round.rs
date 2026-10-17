//! The `round()` builtin (CPython's `float.__round__` / `int.__round__`).
//!
//! Admitted shapes: `round(x)` for an `int`, `bool`, `float` or
//! `int | float` `x`, which is an `int`, and `round(x, ndigits)` for an
//! `int`/`bool`/`float`/`int | float` `x` and an `int`/`bool` `ndigits`,
//! which has `x`'s type (a `bool` `x` gives an `int`). Every other argument
//! shape stays `C0001`.

use pycc_diag::{Diagnostic, Span};
use pycc_hir::Ty;

/// The result type of `round(*args)` for argument types `arg_tys`.
pub(crate) fn round_call_ty(arg_tys: &[Ty]) -> Result<Ty, Diagnostic> {
    let is_int = |t: &Ty| matches!(t, Ty::Int | Ty::Bool);
    match arg_tys {
        [x] if is_int(x) || matches!(x, Ty::Float | Ty::Num) => Ok(Ty::Int),
        [Ty::Float, ndigits] if is_int(ndigits) => Ok(Ty::Float),
        // `int.__round__(ndigits)` is an `int` (rounded half to even to a
        // multiple of `10 ** -ndigits`); an `int | float` keeps its kind.
        [x, ndigits] if is_int(x) && is_int(ndigits) => Ok(Ty::Int),
        [Ty::Num, ndigits] if is_int(ndigits) => Ok(Ty::Num),
        _ => Err(Diagnostic::error(
            "C0001",
            format!(
                "`round()` of ({}) is not supported yet -- only `round(x)` and \
                 `round(x, int)` of an `int`, `float` or `int | float` `x` are",
                arg_tys.iter().map(Ty::name).collect::<Vec<_>>().join(", ")
            ),
            Span::new(0, 0),
        )),
    }
}
