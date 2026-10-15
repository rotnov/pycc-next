//! Lowering of arithmetic with a CPython object operand (non-strict mode,
//! #3) to [`MirExpr::ObjBinOp`] and [`MirExpr::ObjUnaryOp`].
//!
//! `pycc_types::foreign::arith` owns which shapes are admitted; this module
//! only routes them. The native lowerings would be wrong for an object, not
//! merely unimplemented: CPython's operators dispatch on the operands' own
//! types (`__add__`, `__radd__`, ...), which only the C API reproduces.

use super::MirExpr;
use super::object_narrow::object_operand;
use pycc_hir::{BinOpKind, Ty, UnaryOpKind};

/// `Ok(ObjBinOp)` when either lowered operand is a CPython object; `Err`
/// hands the operands back for the native lowering.
pub(super) fn lower_object_binop(
    op: BinOpKind,
    in_place: bool,
    left: MirExpr,
    right: MirExpr,
) -> Result<MirExpr, Box<(MirExpr, MirExpr)>> {
    // `str % args` reaches here only as `pycc_types` admitted it in
    // non-strict mode: CPython's printf-style formatting.
    let formats = op == BinOpKind::Mod && left.ty() == Ty::Str;
    if left.ty() == Ty::Object || right.ty() == Ty::Object || formats {
        Ok(MirExpr::ObjBinOp {
            op,
            in_place,
            left: Box::new(object_operand(left)),
            right: Box::new(object_arg(object_operand(right))),
        })
    } else {
        Err(Box::new((left, right)))
    }
}

/// Whether a lowered display is built as the CPython container
/// ([`pycc_hir::display_is_object`], the rule `pycc_types` checked).
pub(super) fn is_object_display(kind: pycc_hir::DisplayKind, elements: &[MirExpr]) -> bool {
    let tys: Vec<Ty> = elements.iter().map(MirExpr::ty).collect();
    pycc_hir::display_is_object(kind, &tys)
}

/// A tuple, set or dict display with an object element, built as the
/// CPython container; a narrowed element is read as the object it still is.
pub(crate) fn object_display(kind: crate::ObjDisplayKind, elements: Vec<MirExpr>) -> MirExpr {
    MirExpr::ObjDisplay {
        kind,
        elements: elements.into_iter().map(display_element).collect(),
    }
}

/// An argument (or stored value) of an operation on a CPython object: a
/// native dict, set or tuple display -- which `pycc_types` admits there in
/// non-strict mode -- crosses boxed, so it is built as the CPython
/// container (`pycc_codegen::object_box`).
pub(crate) fn object_arg(arg: MirExpr) -> MirExpr {
    match arg {
        MirExpr::DictLiteral(_) | MirExpr::SetLiteral { .. } => MirExpr::ObjectBox(Box::new(arg)),
        // A generator crosses as the CPython iterator over it.
        _ if matches!(arg.ty(), Ty::Generator(_)) => MirExpr::ObjectBox(Box::new(arg)),
        MirExpr::TupleLiteral(ref elements)
            if elements
                .iter()
                .any(|e| !matches!(e.ty(), Ty::Int | Ty::Float | Ty::Bool)) =>
        {
            MirExpr::ObjectBox(Box::new(arg))
        }
        other => other,
    }
}

/// An element of a display built as the CPython container: a narrowed read
/// is the object it still is, and `None` is boxed as `Py_None`.
pub(crate) fn display_element(element: MirExpr) -> MirExpr {
    if element.ty() == Ty::None {
        return MirExpr::ObjectBox(Box::new(element));
    }
    object_operand(element)
}

/// `Some(ObjUnaryOp)` for `-o`, `+o` or `~o` on a CPython object.
pub(super) fn lower_object_unary(op: UnaryOpKind, operand: &MirExpr) -> Option<MirExpr> {
    (operand.ty() == Ty::Object && op != UnaryOpKind::Not).then(|| MirExpr::ObjUnaryOp {
        op,
        operand: Box::new(operand.clone()),
    })
}
