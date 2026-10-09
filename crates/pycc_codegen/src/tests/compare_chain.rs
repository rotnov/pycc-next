//! #1212: a chained comparison whose `int` middle operand is a fresh
//! temporary. Its release and link guard append blocks of their own, so the
//! phi join's incoming blocks must be read after them: `compile_to_object`
//! runs `module.verify()`, which rejects a phi naming a block that is not a
//! predecessor of the join.

use super::*;
use pycc_mir::{MirCompareKind, MirCompareLink};

fn sum(a: i64, b: i64) -> MirExpr {
    MirExpr::BinOp {
        op: BinOpKind::Add,
        left: Box::new(MirExpr::IntLiteral(a)),
        right: Box::new(MirExpr::IntLiteral(b)),
        ty: Ty::Int,
    }
}

fn print_chain(first: i64, middle: MirExpr, last: i64) -> MirItem {
    MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
        callee: "print".to_string(),
        args: vec![MirExpr::CompareChain {
            first: Box::new(MirExpr::IntLiteral(first)),
            links: vec![
                MirCompareLink {
                    kind: MirCompareKind::Plain(CmpOpKind::Lt),
                    right: middle,
                },
                MirCompareLink {
                    kind: MirCompareKind::Plain(CmpOpKind::Lt),
                    right: MirExpr::IntLiteral(last),
                },
            ],
        }],
        ty: Ty::None,
    }))
}

#[test]
fn a_chain_with_an_int_temporary_middle_operand_verifies_and_runs() {
    let mir = MirModule {
        items: vec![
            // Both links hold.
            print_chain(0, sum(1, 1), 3),
            // Link 1 fails.
            print_chain(0, sum(1, 1), 1),
            // Link 0 fails: the middle operand is released in its false
            // exit.
            print_chain(5, sum(1, 1), 9),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("compare_chain_int_middle")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("compare_chain_int_middle.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("compare_chain_int_middle");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n"),
        "True\nFalse\nFalse\n"
    );
}
