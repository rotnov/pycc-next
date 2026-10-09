//! MIR lowering of the conditional expression `body if test else orelse`
//! (#1395).
//!
//! The node's type is decided here from its already-lowered branches with
//! `pycc_hir::if_exp_result_ty`, the one join rule `pycc_types` checked the
//! program with, the same way `crate::boolop` recomputes an `and`/`or`.

use crate::MirExpr;
use pycc_hir::if_exp_result_ty;

pub(crate) fn lower_if_exp(test: MirExpr, body: MirExpr, orelse: MirExpr) -> MirExpr {
    let ty = if_exp_result_ty(&body.ty(), &orelse.ty())
        .expect("pycc_types admitted this conditional expression with the same join rule");
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
    #[should_panic(expected = "same join rule")]
    fn branches_the_checker_would_refuse_are_an_internal_error() {
        lower_if_exp(
            MirExpr::BoolLiteral(true),
            MirExpr::IntLiteral(1),
            MirExpr::StringLiteral("s".to_string()),
        );
    }
}
