//! `MirExpr::ObjectBox` insertion (Part 2 of #1387).
//!
//! `pycc_types::object_box::admits` lets a native `int`, `float`, `bool`,
//! `str`, compiled-instance or `None` value into an `object` slot. At a
//! statement seam whose slot type this pass knows -- an `AnnAssign` under
//! an `object` annotation, a plain rebinding of an `object`-typed name and
//! an attribute store into an `object`-typed slot -- [`box_into`] wraps the
//! value so its `.ty()` is the slot's and codegen packs it.

use crate::{MirExpr, Ty};

/// `value` wrapped in `MirExpr::ObjectBox` when it moves into a slot of
/// type `slot_ty` that is `object` while the value itself is not; `value`
/// unchanged otherwise.
pub(crate) fn box_into(value: MirExpr, slot_ty: &Ty) -> MirExpr {
    if *slot_ty == Ty::Object && value.ty() != Ty::Object {
        MirExpr::ObjectBox(Box::new(value))
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_native_value_into_an_object_slot_is_boxed() {
        assert_eq!(
            box_into(MirExpr::IntLiteral(3), &Ty::Object),
            MirExpr::ObjectBox(Box::new(MirExpr::IntLiteral(3)))
        );
        assert_eq!(box_into(MirExpr::NoneLiteral, &Ty::Object).ty(), Ty::Object);
    }

    #[test]
    fn an_object_value_or_a_native_slot_is_left_alone() {
        assert_eq!(
            box_into(MirExpr::NotImplemented, &Ty::Object),
            MirExpr::NotImplemented
        );
        assert_eq!(
            box_into(MirExpr::IntLiteral(3), &Ty::Int),
            MirExpr::IntLiteral(3)
        );
    }
}
