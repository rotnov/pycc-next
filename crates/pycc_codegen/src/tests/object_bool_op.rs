//! Part 6 of #1371: `and`/`or` with a CPython object operand
//! (`boolop.rs`), pinned on the LLVM IR of an `ext` object.

use super::*;
use pycc_mir::BoolOpKind;

fn bool_op(op: BoolOpKind, left: MirExpr, right: MirExpr, truth_only: bool) -> MirExpr {
    let ty = if truth_only { Ty::Bool } else { Ty::Object };
    MirExpr::BoolOp {
        op,
        left: Box::new(left),
        right: Box::new(right),
        ty,
        truth_only,
    }
}

/// Compiles `exprs` as discarded module-body statements and hands the IR
/// to `check`.
fn module_ir(label: &str, exprs: Vec<MirExpr>, check: impl Fn(&str)) {
    compile_ext_items_checking_ir(
        label,
        with_foreign_numpy(
            exprs
                .into_iter()
                .map(|expr| MirItem::TopLevelStmt(MirStmt::ExprStmt(expr)))
                .collect(),
        ),
        check,
    );
}

fn count(ir: &str, needle: &str) -> usize {
    ir.lines().filter(|line| line.contains(needle)).count()
}

/// Every boxable native operand is boxed by its own packer, on either side
/// and under either operator, each box's `NULL` routed to the failure edge;
/// an object operand is never re-boxed, and the node never touches a
/// reference count.
#[test]
fn a_native_operand_is_boxed_by_its_packer_and_an_object_passes_through() {
    module_ir(
        "obj_bool_op_box",
        vec![
            bool_op(BoolOpKind::Or, numpy_pi(), MirExpr::IntLiteral(1), false),
            bool_op(
                BoolOpKind::And,
                MirExpr::FloatLiteral(1.5),
                numpy_pi(),
                false,
            ),
            bool_op(
                BoolOpKind::Or,
                MirExpr::BoolLiteral(false),
                numpy_pi(),
                false,
            ),
            bool_op(
                BoolOpKind::And,
                numpy_pi(),
                MirExpr::StringLiteral("s".to_string()),
                false,
            ),
            bool_op(BoolOpKind::Or, numpy_pi(), numpy_pi(), false),
        ],
        |ir| {
            for packer in [
                "call ptr @pycc_ext_obj_pack_int(",
                "call ptr @pycc_ext_obj_pack_float(",
                "call ptr @pycc_ext_obj_pack_bool(",
                "call ptr @pycc_ext_obj_pack_str(",
            ] {
                assert_eq!(count(ir, packer), 1, "{packer}: {ir}");
            }
            assert_eq!(count(ir, "call ptr @pycc_ext_obj_pack_object("), 0, "{ir}");
            let routed = ir
                .lines()
                .filter(|line| {
                    line.contains("%boolop_box_failed") && line.contains("= icmp eq ptr")
                })
                .count();
            assert_eq!(routed, 4, "{ir}");
            // The left operand's truth test, on the three nodes whose left
            // operand is an object.
            assert_eq!(count(ir, "call i32 @pycc_ext_obj_truthy("), 3, "{ir}");
            assert!(!ir.contains("DecRef"), "{ir}");
        },
    );
}

/// In truth context nothing is boxed: each object operand is truth-tested
/// where it is evaluated.
#[test]
fn a_truth_context_node_tests_each_object_operand_and_boxes_nothing() {
    module_ir(
        "obj_bool_op_truth",
        vec![
            bool_op(BoolOpKind::And, numpy_pi(), MirExpr::IntLiteral(1), true),
            bool_op(BoolOpKind::Or, MirExpr::IntLiteral(0), numpy_pi(), true),
        ],
        |ir| {
            assert_eq!(count(ir, "call i32 @pycc_ext_obj_truthy("), 2, "{ir}");
            assert!(!ir.contains("@pycc_ext_obj_pack_"), "{ir}");
            assert!(!ir.contains("boolop_box"), "{ir}");
        },
    );
}
