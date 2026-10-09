use super::continuation_narrows;
use crate::{HirExpr, HirStmt};

fn ret() -> HirStmt {
    HirStmt::Return(Some(HirExpr::IntLiteral(0)))
}

fn assign(name: &str) -> HirStmt {
    HirStmt::Assign {
        target: name.to_string(),
        value: HirExpr::IntLiteral(1),
    }
}

#[test]
fn a_terminating_body_with_an_untouched_orelse_narrows() {
    assert!(continuation_narrows(&[ret()], &[], "o"));
    assert!(continuation_narrows(&[ret()], &[assign("other")], "o"));
}

#[test]
fn a_body_that_falls_through_does_not_narrow() {
    assert!(!continuation_narrows(&[assign("other")], &[], "o"));
}

/// The #1476 review finding: the surviving `else` rebinds the name.
#[test]
fn an_orelse_that_rebinds_the_name_does_not_narrow() {
    assert!(!continuation_narrows(&[ret()], &[assign("o")], "o"));
}

/// An `elif` arm is a nested `if` in `orelse`; a rebinding in any of its
/// arms counts.
#[test]
fn a_rebinding_in_an_elif_arm_does_not_narrow() {
    let elif = HirStmt::If {
        test: HirExpr::Name("flag".to_string()),
        body: vec![assign("o")],
        orelse: vec![],
    };
    assert!(!continuation_narrows(&[ret()], &[elif], "o"));
}
