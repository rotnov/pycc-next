//! MIR lowering of the conditional expression `body if test else orelse`
//! (#1395).
//!
//! The node's type is decided here from its already-lowered branches with
//! `pycc_hir::if_exp_result_ty`, the one join rule `pycc_types` checked the
//! program with, the same way `crate::boolop` recomputes an `and`/`or`.

use crate::MirExpr;
use pycc_hir::if_exp_result_ty;

pub(crate) fn lower_if_exp(test: MirExpr, body: MirExpr, orelse: MirExpr) -> MirExpr {
    // Non-strict mode (#3): `pycc_types` admits branches with no common
    // native type only there, and only when both box into an object.
    let Some(ty) = if_exp_result_ty(&body.ty(), &orelse.ty()) else {
        return MirExpr::IfExp {
            test: Box::new(test),
            body: Box::new(crate::object_box::box_into(body, &crate::Ty::Object)),
            orelse: Box::new(crate::object_box::box_into(orelse, &crate::Ty::Object)),
            ty: crate::Ty::Object,
        };
    };
    MirExpr::IfExp {
        test: Box::new(test),
        body: Box::new(body),
        orelse: Box::new(orelse),
        ty,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Ty;

    #[test]
    fn the_node_is_typed_by_the_shared_join() {
        let lowered = lower_if_exp(
            MirExpr::BoolLiteral(true),
            MirExpr::IntLiteral(1),
            MirExpr::NoneLiteral,
        );
        assert_eq!(lowered.ty(), Ty::Optional(Box::new(Ty::Int)));
    }

    #[test]
    fn branches_with_no_common_native_type_are_boxed_into_an_object() {
        let lowered = lower_if_exp(
            MirExpr::BoolLiteral(true),
            MirExpr::IntLiteral(1),
            MirExpr::StringLiteral("s".to_string()),
        );
        assert_eq!(lowered.ty(), Ty::Object);
        let MirExpr::IfExp { body, .. } = lowered else {
            panic!("an IfExp")
        };
        assert_eq!(*body, MirExpr::ObjectBox(Box::new(MirExpr::IntLiteral(1))));
    }
}
