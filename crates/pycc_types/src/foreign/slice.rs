//! Slicing a CPython object: the load `o[a:b:c]` (Part 2b of #1371) and the
//! deletion `del o[a:b:c]` (Part 2c).
//!
//! The store `o[a:b] = v` is a separate HIR shape refused before this
//! crate, exactly as for `o[k] = v`. Each present bound must be one of the
//! packable operands ([`super::is_packable_operand`]: `int`, `float`,
//! `bool`, `str` or another object), because codegen boxes it through a
//! `pycc_ext_obj_pack_*` helper into the `slice` object CPython's
//! `PySlice_New` builds; an absent bound is CPython's `None`, as in `o[:b]`.
//! Any other bound type -- a container, an instance or `None` itself -- is
//! refused with [`super::object_operation_unsupported`].
//!
//! A load's result is [`Ty::Object`]: whatever the object's own
//! `__getitem__` answers for a `slice` key. A deletion has no result; it is
//! `PyObject_DelItem` with the same `slice` key. A bound CPython's
//! `__getitem__`/`__delitem__` would refuse (a `float` bound on a list, say)
//! is admitted here and raises CPython's own `TypeError` at run time, the
//! same split `o[k]` makes for its key.
//!
//! `pycc_hir` lowers every `del x[a:b]` to `HirStmt::DeleteSlice` without
//! knowing `x`'s type, so [`check_delete_slice`] owns the refusal of every
//! base that is not an object. It keeps the HIR's former `C0001` text and,
//! unlike most type-stage diagnostics, its source location: the node
//! carries the slice target's span.

use super::{is_packable_operand, object_operation_unsupported};
use crate::Environment;
use crate::infer_expr_in;
use pycc_diag::{Diagnostic, Span};
use pycc_hir::{HirExpr, Ty};

/// Types `base[start:stop:step]` once `base` is known to be a CPython
/// object, inferring each present bound left to right.
pub(crate) fn object_slice_ty(
    env: &Environment,
    local_names: &[&str],
    bounds: [Option<&HirExpr>; 3],
) -> Result<Ty, Diagnostic> {
    for bound in bounds.into_iter().flatten() {
        let bound_ty = infer_expr_in(env, local_names, bound)?;
        if !is_packable_operand(&bound_ty) {
            return Err(object_operation_unsupported(&format!(
                "slicing a CPython object with a `{}` bound",
                bound_ty.name()
            )));
        }
    }
    Ok(Ty::Object)
}

/// Checks `del base[start:stop:step]` (Part 2c of #1371): the base is
/// inferred first, then -- only for a CPython object base -- every present
/// bound, under exactly [`object_slice_ty`]'s rule. Any other base is the
/// located `C0001` the HIR reported before this part.
pub(crate) fn check_delete_slice(
    env: &Environment,
    local_names: &[&str],
    base: &HirExpr,
    bounds: [Option<&HirExpr>; 3],
    span: Span,
) -> Result<(), Diagnostic> {
    if infer_expr_in(env, local_names, base)? != Ty::Object {
        return Err(Diagnostic::error(
            "C0001",
            "a `del` of a slice (`del xs[a:b]`) is not supported yet",
            span,
        ));
    }
    object_slice_ty(env, local_names, bounds).map(|_| ())
}

#[cfg(test)]
mod tests;
