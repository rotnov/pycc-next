//! Storing and deleting an attribute of a CPython object: `o.x = v` and
//! `del o.x` (Part 2 of #1443, #1457).
//!
//! The load `o.x` is typed in `expr.rs` (it answers [`Ty::Object`]). A store
//! has no result: codegen packs the value into a new reference and calls
//! CPython's `PyObject_SetAttr`, so the value is admitted under exactly the
//! rule a method-call argument is ([`super::check_object_call_args`]): a
//! packable scalar, another object, `None` or a carriable class instance.
//! Any other value type is refused with
//! [`super::object_operation_unsupported`]. Whether the object accepts the
//! attribute at all -- a read-only property, a `__slots__` class, a
//! descriptor whose `__set__` raises -- is the object's own answer at run
//! time, raised as CPython's exception, the same split a method call makes.
//!
//! `pycc_hir` lowers every `del x.a` to `HirStmt::DeleteAttr` without
//! knowing `x`'s type, so [`check_delete_attr`] owns the refusal of every
//! base that is not an object, located at the attribute target like the
//! slice deletion's (`super::slice::check_delete_slice`).

use super::check_object_call_args;
use crate::Environment;
use crate::infer_expr_in;
use pycc_diag::{Diagnostic, Span};
use pycc_hir::{HirExpr, Ty};

/// Checks the value of `base.attr = value` once `base` is known to be a
/// CPython object (`class::check_attr_set` routes the object case here).
pub(crate) fn check_object_attr_set(
    env: &Environment,
    local_names: &[&str],
    value: &HirExpr,
) -> Result<(), Diagnostic> {
    let value_ty = infer_expr_in(env, local_names, value)?;
    check_object_call_args(env, [value], &[value_ty], "attribute store")
}

/// Checks `del base.attr` (#1457): only a CPython object base is admitted.
/// Any other base -- a class instance, whose attribute slots are fixed by
/// its declaration, or a builtin value -- is a located `C0001`.
pub(crate) fn check_delete_attr(
    env: &Environment,
    local_names: &[&str],
    base: &HirExpr,
    span: Span,
) -> Result<(), Diagnostic> {
    let base_ty = infer_expr_in(env, local_names, base)?;
    if base_ty != Ty::Object {
        return Err(Diagnostic::error(
            "C0001",
            format!(
                "a `del` of an attribute (`del obj.attr`) is supported only on a CPython \
                 object, not on `{}`",
                base_ty.name()
            ),
            span,
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
