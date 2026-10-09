//! Comparisons, identity tests, membership tests and `isinstance` with a
//! CPython object operand (Part 1 and Part 2b of #1371).
//!
//! The rules, each matching CPython 3.14 for what it admits:
//!
//! * **Identity** (`is`/`is not`) is admitted between two objects and
//!   between an object and the `None` literal; it is pointer identity and
//!   cannot raise, so the result is `bool`. An object against any other
//!   native value is refused (`I0404`): pycc would have to box the native
//!   value into a fresh object, whose identity CPython's program never
//!   compared against. Two native non-`None` operands keep the `C0001` the
//!   HIR gate gives a literal operand (`pycc_hir::compare_chain`). A chain
//!   link reaches here the same way since Part 11 of #1371, though a chain
//!   with an object operand is refused before its links are typed.
//! * **Rich comparison** (`==`, `!=`, `<`, `<=`, `>`, `>=`) is admitted
//!   when one operand is an object and the other is an object, one of the
//!   four packable scalars (`int`, `float`, `bool`, `str`) or, since #1470,
//!   an instance of a regular user class, which crosses as its carrier and
//!   compares through the class's own dunders (`super::is_object_operand`;
//!   a class method's own `cls` stays refused). Its type is the
//!   **object** CPython's `PyObject_RichCompare` returns, not `bool`: the
//!   comparison's own result is an arbitrary object (an array, say, or one
//!   whose `__bool__` raises), and a truth context truth-tests it exactly as
//!   CPython does. A `-> bool` return or a `bool` annotation therefore
//!   refuses it with the ordinary assignability diagnostic; `bool(...)`
//!   converts it explicitly.
//! * **Membership** (`in`/`not in`, Part 2b) is admitted when the container
//!   is an object and the item is an object or one of the four packable
//!   scalars. CPython's `PySequence_Contains` truth-tests `__contains__`'s
//!   answer itself, so the result is `bool`. An object item in a native
//!   container, or an unpackable item in an object, is refused (`I0404`);
//!   two native operands keep the HIR's `C0001`, because no native
//!   membership test is lowered.
//! * **A chained comparison** with an object operand is refused: its
//!   short-circuit over raising, object-valued links is a later part of the
//!   #1371 series.
//! * **`isinstance(o, C)`** with an object `o` is a run-time
//!   `PyObject_IsInstance` (`pycc_mir`'s `MirExpr::ObjIsInstance`), never
//!   the compile-time fold every other first argument gets. `C` must be an
//!   object-typed expression (a foreign class such as `mod.Cls`), one of
//!   the builtin class names `int`, `float`, `bool`, `str`, `list`, `dict`,
//!   `tuple` ([`is_object_isinstance_builtin`]), or -- Part 7 of #1371 -- a
//!   plain class compiled in this module, answered against the host type
//!   objects of the published classes whose MRO contains it
//!   ([`compiled_class_refusal`] names the kinds that stay refused). A
//!   compiled class is consulted before the builtin names, so a
//!   module-level `class int: ...` is the class tested, and refused when
//!   its kind is (`pycc_mir`'s `obj_compare::lower_object_isinstance` and
//!   `pycc_hir::isinstance_narrow_target` take the same order). A
//!   module function spelled like a class shadows it and is refused, as
//!   is a tuple of classes, which has no lowering yet.

use super::object_operation_unsupported;
use crate::Environment;
use crate::infer_expr_in;
use pycc_diag::{Diagnostic, Span};
use pycc_hir::{CmpOpKind, HirClassDef, HirExpr, Ty, is_builtin_type_name};

/// Types an `is`/`is not` between two operands neither of which is the
/// `None` literal (see the module doc).
pub(crate) fn general_identity_ty(
    op: CmpOpKind,
    left_ty: &Ty,
    right_ty: &Ty,
) -> Result<Ty, Diagnostic> {
    match (left_ty, right_ty) {
        (Ty::Object, Ty::Object) => Ok(Ty::Bool),
        (Ty::Object, other) | (other, Ty::Object) => Err(object_operation_unsupported(&format!(
            "testing a CPython object's identity against a `{}` value",
            other.name()
        ))),
        // `HirExpr::Compare` carries no source range, so no real span exists at this layer (Part 5 of #1371).
        _ => Err(Diagnostic::error(
            "C0001",
            format!("comparison operator not supported yet: {op:?}"),
            Span::new(0, 0),
        )),
    }
}

/// Types a rich comparison when either operand is a CPython object, and
/// answers `None` when neither is, so the caller's native rules apply.
///
/// The non-object operand must be one the shim can box
/// ([`super::is_object_operand`]); `left` and `right` are the operand
/// expressions, consulted only to refuse a class method's own `cls`.
pub(crate) fn rich_compare_ty(
    env: &Environment,
    (left, left_ty): (&HirExpr, &Ty),
    (right, right_ty): (&HirExpr, &Ty),
) -> Option<Result<Ty, Diagnostic>> {
    let (other_expr, other) = match (left_ty, right_ty) {
        (Ty::Object, _) => (right, right_ty),
        (_, Ty::Object) => (left, left_ty),
        _ => return None,
    };
    if let Err(diagnostic) = super::refuse_classmethod_cls(
        env,
        other_expr,
        "comparing a CPython object with a class method's `cls`",
    ) {
        return Some(Err(diagnostic));
    }
    Some(if super::is_object_operand(env, other) {
        Ok(Ty::Object)
    } else {
        Err(object_operation_unsupported(&format!(
            "comparing a CPython object with a `{}` value",
            other.name()
        )))
    })
}

/// Types `item in container` / `item not in container` (Part 2b of #1371).
///
/// Admitted only with an object container and a packable item
/// (`super::is_packable_operand`); the result is `bool`, because CPython's
/// `in` always answers one (`PySequence_Contains`). An object on the wrong
/// side of a native value is refused with `I0404`, and two native operands
/// keep the `C0001` HIR gives a literal or display container
/// (`pycc_hir::compare_chain`), since pycc lowers no native membership test.
pub(crate) fn membership_ty(
    op: CmpOpKind,
    item_ty: &Ty,
    container_ty: &Ty,
) -> Result<Ty, Diagnostic> {
    match (item_ty, container_ty) {
        (item, Ty::Object) if super::is_packable_operand(item) => Ok(Ty::Bool),
        (item, Ty::Object) => Err(object_operation_unsupported(&format!(
            "testing membership of a `{}` value in a CPython object",
            item.name()
        ))),
        (Ty::Object, container) => Err(object_operation_unsupported(&format!(
            "testing membership of a CPython object in a `{}` value",
            container.name()
        ))),
        // `HirExpr::Compare` carries no source range (Part 5 of #1371).
        _ => Err(Diagnostic::error(
            "C0001",
            format!("comparison operator not supported yet: {op:?}"),
            Span::new(0, 0),
        )),
    }
}

/// `Err(I0404)` when any operand of a chained comparison is an object.
pub(crate) fn reject_object_in_chain(operand_tys: &[Ty]) -> Result<(), Diagnostic> {
    if operand_tys.iter().any(|ty| matches!(ty, Ty::Object)) {
        return Err(object_operation_unsupported(
            "a chained comparison with a CPython object operand",
        ));
    }
    Ok(())
}

/// Whether `name` is a builtin class an object `isinstance` names by
/// CPython's own type object (`pycc_mir::ObjBuiltinClass`): the four scalar
/// type names and, since Part 7 of #1371, `list`, `dict` and `tuple`, which
/// otherwise have no binding at all (`T0021` "name `list` is not defined").
pub(crate) fn is_object_isinstance_builtin(name: &str) -> bool {
    is_builtin_type_name(name) || matches!(name, "list" | "dict" | "tuple")
}

/// Why `isinstance(o, C)` with an object `o` refuses the compiled class
/// `class_def`, or `None` for a class it admits (Part 7 of #1371).
///
/// Admitted is a plain class: its answer is `PyObject_IsInstance` against
/// the host type object of every *published* class whose MRO contains it,
/// since a published type is the only carrier a compiled instance has on
/// the CPython side and none of them can be subclassed there. The refused
/// kinds have another CPython identity that answer would miss:
///
/// * an **exception class** is registered as a real CPython exception
///   class, so a host-raised `m.MyError` *is* an instance of it;
/// * a **protocol** is checked structurally, not by type object;
/// * an **enum**'s members are native values with no host carrier;
/// * a **generic class** is compiled per specialization, so its bare name
///   names no single class.
pub(crate) fn compiled_class_refusal(class_def: &HirClassDef) -> Option<&'static str> {
    if class_def.exception_type_tag.is_some() {
        Some("exception class")
    } else if class_def.is_protocol {
        Some("protocol")
    } else if class_def.is_enum {
        Some("enum")
    } else if class_def.type_param.is_some() {
        Some("generic class")
    } else {
        None
    }
}

/// Types `isinstance(o, class_arg)` once `o` is known to be an object (see
/// the module doc for what `class_arg` may be).
pub(crate) fn check_object_isinstance(
    env: &Environment,
    local_names: &[&str],
    class_arg: &HirExpr,
) -> Result<Ty, Diagnostic> {
    if let HirExpr::TupleLiteral(_) = class_arg {
        return Err(object_operation_unsupported(
            "testing a CPython object with `isinstance` against a tuple of classes",
        ));
    }
    // A local or parameter spelled like a builtin or a compiled class
    // shadows it, as in CPython: it is then the evaluated operand below.
    if let HirExpr::Name(name) = class_arg
        && !local_names.contains(&name.as_str())
    {
        // A module function spelled like a class shadows it, as in
        // CPython, where the guard then raises `TypeError` (argument 2 is
        // not a class); pycc refuses it here.
        if env.lookup_function(name).is_some() {
            return Err(object_operation_unsupported(&format!(
                "testing a CPython object with `isinstance` against the function `{name}`"
            )));
        }
        // A compiled class spelled like a builtin shadows the builtin, as
        // the guard's lowering does (see the module doc).
        if let Some(class_def) = env.lookup_class(name) {
            return match compiled_class_refusal(class_def) {
                None => Ok(Ty::Bool),
                Some(kind) => Err(object_operation_unsupported(&format!(
                    "testing a CPython object with `isinstance` against the pycc {kind} `{name}`"
                ))),
            };
        }
        if is_object_isinstance_builtin(name) {
            return Ok(Ty::Bool);
        }
    }
    match infer_expr_in(env, local_names, class_arg)? {
        Ty::Object => Ok(Ty::Bool),
        other => Err(object_operation_unsupported(&format!(
            "testing a CPython object with `isinstance` against a `{}` value",
            other.name()
        ))),
    }
}

#[cfg(test)]
mod tests;
