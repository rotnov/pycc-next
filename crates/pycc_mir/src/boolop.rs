//! MIR lowering of `and`/`or` (#1211, Part 3 of #1018).
//!
//! The node's type is decided here from its already-lowered operands, the
//! same way [`binop_result_ty`](crate::binop) recomputes a binary
//! expression's: a truth-only node is `bool`, and a value-context node takes
//! `pycc_hir::bool_op_result_ty`, the one join rule `pycc_types` checked the
//! program with.

use crate::{BoolOpKind, MirExpr, Ty};
use pycc_hir::bool_op_result_ty;

pub(crate) fn lower_bool_op(
    op: BoolOpKind,
    left: MirExpr,
    right: MirExpr,
    truth_only: bool,
) -> MirExpr {
    let ty = if truth_only {
        Ty::Bool
    } else {
        bool_op_result_ty(op, &left.ty(), &right.ty())
            .expect("pycc_types admitted this `and`/`or` with the same join rule")
    };
    MirExpr::BoolOp {
        op,
        left: Box::new(left),
        right: Box::new(right),
        ty,
        truth_only,
    }
}
