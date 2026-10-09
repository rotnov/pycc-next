//! A list display bound to a CPython object slot, `x: object = [a, b]`
//! (Part 2d of #1371, D-258 rule 4).
//!
//! The empty-container pre-pass (`crate::empty_container`) rewrites a list
//! display it resolves to an object slot into [`pycc_hir::HirExpr::ObjectList`];
//! this module types that node. Codegen builds a fresh CPython `list` and
//! boxes each element through a `pycc_ext_obj_pack_*` helper, so every
//! element must be one the shim can box ([`super::is_object_operand`]:
//! `int`, `float`, `bool`, `str`, another object or, since #1470, an
//! instance of a regular user class, which crosses as its carrier). Any
//! other element type -- a pycc container, an enum member, an exception
//! instance or `None` -- has no boundary representation yet and is refused
//! with
//! [`super::object_operation_unsupported`], naming the first offending
//! element in source order. The elements need not share a type: a CPython
//! list is heterogeneous, so D-105's homogeneity rule (`T0032`) does not
//! apply to this node.

use super::{is_object_operand, object_operation_unsupported, refuse_classmethod_cls};
use crate::Environment;
use crate::infer_expr_in;
use pycc_diag::Diagnostic;
use pycc_hir::{HirExpr, Ty};

/// Types `[e1, e2, ...]` built as a CPython `list`, inferring each element
/// left to right. The result is [`Ty::Object`].
pub(crate) fn object_list_ty(
    env: &Environment,
    local_names: &[&str],
    elements: &[HirExpr],
) -> Result<Ty, Diagnostic> {
    for element in elements {
        let element_ty = infer_expr_in(env, local_names, element)?;
        refuse_classmethod_cls(
            env,
            element,
            "a class method's `cls` as an element of a list display bound to a CPython object",
        )?;
        if !is_object_operand(env, &element_ty) {
            return Err(object_operation_unsupported(&format!(
                "a `{}` element in a list display bound to a CPython object",
                element_ty.name()
            )));
        }
    }
    Ok(Ty::Object)
}

#[cfg(test)]
mod tests;
