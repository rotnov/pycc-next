//! Typing of typed numpy array operations (`HirExpr::Nd`, built by
//! `pycc_hir::ndarray`).
//!
//! The array itself is an `object` by the time this crate sees it; each
//! `Nd` node carries the dtype and dimension count it was lowered for, so
//! typing one is local: every index must be an `int` (not a `bool`, which
//! numpy reads as a mask), a
//! stored value must fit the element type, and the result is the element
//! type, a tuple of ints (`.shape`), an int or `None`.

use crate::{Environment, infer_expr_in};
use pycc_diag::{Diagnostic, Span};
use pycc_hir::Ty;
use pycc_hir::ndarray::{NdDtype, NdExpr, NdOp};

/// Whether a value of type `value` can be stored into an element of
/// `dtype`: what numpy stores without a cast it would warn about. An `int`
/// or `bool` widens into `float64`; a `bool` into `int64`.
fn storable(value: &Ty, dtype: NdDtype) -> bool {
    match dtype {
        NdDtype::F64 => matches!(value, Ty::Float | Ty::Int | Ty::Bool),
        NdDtype::I64 | NdDtype::Byte => matches!(value, Ty::Int | Ty::Bool),
        NdDtype::Bool => matches!(value, Ty::Bool),
    }
}

pub(crate) fn infer_nd(
    env: &Environment,
    local_names: &[&str],
    nd: &NdExpr,
) -> Result<Ty, Diagnostic> {
    let base = infer_expr_in(env, local_names, &nd.base)?;
    if base != Ty::Object {
        return Err(Diagnostic::error(
            "T0021",
            format!(
                "`{}` operations need a numpy array, found `{}`",
                nd.nd.name(),
                base.name()
            ),
            Span::new(0, 0),
        ));
    }
    let (indices, value) = match nd.op {
        NdOp::Get { .. } => (&nd.args[..], None),
        NdOp::Set { .. } => {
            let (value, indices) = nd.args.split_last().expect("a store has a value");
            (indices, Some(value))
        }
        _ => (&[][..], None),
    };
    for index in indices {
        let index_ty = infer_expr_in(env, local_names, index)?;
        // `bool` is refused although it is an `int`: numpy reads `a[True]`
        // as a boolean mask, not as `a[1]`.
        if index_ty != Ty::Int {
            let diagnostic = Diagnostic::error(
                "T0021",
                format!(
                    "`{}` index must be `int`, found `{}`",
                    nd.nd.name(),
                    index_ty.name()
                ),
                Span::new(0, 0),
            )
            .with_help("use an `int` value (slices and other index forms run through CPython only on an untyped array)");
            return Err(crate::locate::place_expr(diagnostic, index));
        }
    }
    if let Some(value) = value {
        let value_ty = infer_expr_in(env, local_names, value)?;
        if !storable(&value_ty, nd.nd.dtype) {
            let diagnostic = Diagnostic::error(
                "T0021",
                format!(
                    "cannot store `{}` into an element of `{}`",
                    value_ty.name(),
                    nd.nd.name()
                ),
                Span::new(0, 0),
            )
            .with_help(format!("store a `{}` value", nd.nd.elem_ty().name()));
            return Err(crate::locate::place_expr(diagnostic, value));
        }
    }
    Ok(nd.ty())
}
