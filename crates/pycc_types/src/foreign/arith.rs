//! Arithmetic and item stores on a CPython object (non-strict mode, #3).
//!
//! A binary operator with at least one `object` operand, a unary `-`, `+` or
//! `~` on an `object`, and `o[k] = v` / `del o[k]` on an `object` base run
//! CPython's own protocol through the C API (`PyNumber_*`,
//! `PyObject_SetItem`, `PyObject_DelItem`), so every result -- including the
//! exception a mismatched pair raises -- is exactly CPython's. The native
//! operand of a mixed pair is boxed the way a rich-comparison operand is
//! ([`super::is_object_operand`]); the result is always `object`.

use super::{
    check_object_call_args, is_object_operand, is_packable_operand, object_operation_unsupported,
};
use crate::{Environment, infer_expr_in};
use pycc_diag::{Diagnostic, Span};
use pycc_hir::{HirExpr, Ty};

/// `Some(result)` when a binary operator has an `object` operand, `None`
/// when neither operand is one (the native rules apply).
///
/// `env` widens the operand rule to class instances (their carrier); the
/// private-helper solver has no environment and passes `None`, which keeps
/// the narrower scalar-or-object rule.
pub(crate) fn object_binop_ty(
    env: Option<&Environment>,
    left: &Ty,
    right: &Ty,
) -> Option<Result<Ty, Diagnostic>> {
    if !crate::non_strict::is_non_strict() || (*left != Ty::Object && *right != Ty::Object) {
        return None;
    }
    let admitted = |ty: &Ty| match env {
        Some(env) => is_object_operand(env, ty),
        None => is_packable_operand(ty),
    };
    Some(if admitted(left) && admitted(right) {
        Ok(Ty::Object)
    } else {
        Err(object_operation_unsupported(&format!(
            "a binary operator between `{}` and `{}`",
            left.name(),
            right.name()
        )))
    })
}

/// `o[k] = v` on an `object` base: the key is a boxable operand and the
/// value is admitted under the attribute-store rule (`None` included).
pub(crate) fn check_object_set_item(
    env: &Environment,
    value: &HirExpr,
    key_ty: &Ty,
    value_ty: &Ty,
) -> Result<(), Diagnostic> {
    check_key(env, key_ty, "storing into")?;
    check_object_call_args(env, [value], std::slice::from_ref(value_ty), "item store")
}

/// Checks `del base[key]`: a `dict` with a key of its key type, a `list`
/// with an int index (`pycc_hir::builtins`' `__pycc_delitem__`), or a
/// CPython object; any other base keeps the located `C0001` a subscript
/// deletion always had.
pub(crate) fn check_delete_item(
    env: &Environment,
    local_names: &[&str],
    base: &HirExpr,
    key: &HirExpr,
    span: Span,
) -> Result<(), Diagnostic> {
    let base_ty = infer_expr_in(env, local_names, base)?;
    if matches!(base_ty, Ty::Dict(_) | Ty::List(_)) {
        let key_ty = infer_expr_in(env, local_names, key)?;
        if let Some(Ok(_)) =
            pycc_hir::builtins::resolve_method(&base_ty, "__pycc_delitem__", &[key_ty.clone()])
        {
            return Ok(());
        }
        return Err(Diagnostic::error(
            "T0001",
            format!(
                "`del` of a `{}` item with a `{}` key",
                base_ty.name(),
                key_ty.name()
            ),
            span,
        ));
    }
    // Strict mode admits no CPython-object item deletion (#3 is the
    // non-strict fallback's).
    if base_ty == Ty::Object && !crate::non_strict::is_non_strict() {
        return Err(Diagnostic::error(
            "C0001",
            "a `del` of a subscript of a CPython object is not supported yet",
            span,
        ));
    }
    if base_ty != Ty::Object {
        return Err(Diagnostic::error(
            "C0001",
            format!(
                "a `del` of a subscript (`del d[k]`, `del xs[i]`) is supported only on a \
                 dict, a list or a CPython object, not on `{}`",
                base_ty.name()
            ),
            span,
        ));
    }
    let key_ty = infer_expr_in(env, local_names, key)?;
    check_key(env, &key_ty, "deleting an item of")
}

fn check_key(env: &Environment, key_ty: &Ty, what: &str) -> Result<(), Diagnostic> {
    if is_object_operand(env, key_ty) {
        Ok(())
    } else {
        Err(object_operation_unsupported(&format!(
            "{what} a CPython object with a `{}` key",
            key_ty.name()
        )))
    }
}

/// Non-strict mode (#3): a list, tuple, set or dict display that
/// [`pycc_hir::display_is_object`] builds as the CPython container -- an
/// `object` element, or elements with no native representation -- is that
/// container (`Ty::Object`), every element boxed. `None` when the mode is
/// strict, an element does not infer (the ordinary arm reports it) or the
/// display stays native.
pub(crate) fn object_display_ty<'a>(
    env: &Environment,
    local_names: &[&str],
    kind: pycc_hir::DisplayKind,
    elements: impl IntoIterator<Item = &'a HirExpr>,
) -> Option<Result<Ty, Diagnostic>> {
    if !crate::non_strict::is_non_strict() {
        return None;
    }
    let tys = elements
        .into_iter()
        .map(|element| infer_expr_in(env, local_names, element).ok())
        .collect::<Option<Vec<Ty>>>()?;
    if tys.is_empty() && crate::non_strict::empty_display_untyped() {
        return None;
    }
    if !pycc_hir::display_is_object(kind, &tys) {
        return None;
    }
    // `None` is CPython's own `Py_None` there.
    Some(match tys.iter().find(|ty| **ty != Ty::None && !is_object_operand(env, ty)) {
        None => Ok(Ty::Object),
        Some(ty) => Err(object_operation_unsupported(&format!(
            "a `{}` element in a display with a CPython object element",
            ty.name()
        ))),
    })
}
