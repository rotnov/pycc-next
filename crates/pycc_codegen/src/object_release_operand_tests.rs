//! Part 3 of #1092: the release of unbound produced operands of `print`,
//! f-strings, `raise`, conditional expressions and boolean operators.
//!
//! Each test pairs a produced operand (`copy.a`, a new reference the
//! consumer must release) with its borrowed control (`copy`, a module
//! global the consumer must never release), so a test that passes because
//! nothing is ever released, or because everything is, fails one side.

use super::*;
use pycc_mir::{BoolOpKind, MirFStringPart};

fn print(args: Vec<MirExpr>) -> MirStmt {
    MirStmt::ExprStmt(MirExpr::Call {
        callee: "print".to_string(),
        args,
        ty: Ty::None,
    })
}

fn fstring(value: MirExpr) -> MirExpr {
    MirExpr::FString(vec![
        MirFStringPart::Literal("v=".to_string()),
        MirFStringPart::Interpolation(boxed(value)),
    ])
}

fn if_exp(test: MirExpr, body: MirExpr, orelse: MirExpr, ty: Ty) -> MirExpr {
    MirExpr::IfExp {
        test: boxed(test),
        body: boxed(body),
        orelse: boxed(orelse),
        ty,
    }
}

fn bool_op(op: BoolOpKind, left: MirExpr, right: MirExpr, ty: Ty, truth_only: bool) -> MirExpr {
    MirExpr::BoolOp {
        op,
        left: boxed(left),
        right: boxed(right),
        ty,
        truth_only,
    }
}

/// `print(copy.a, 1, copy.b)`: each produced argument is held until its
/// own write, so the second argument's failure releases the first, and each
/// is released once its text is written. `print(copy, 1, copy)` releases
/// nothing.
#[test]
fn print_releases_each_produced_argument_after_writing_it() {
    let ir = f_ir(
        "release_print",
        vec![print(vec![attr("a"), MirExpr::IntLiteral(1), attr("b")])],
    );
    let fails = blocks(&ir, "foreign_attr_fail");
    assert_eq!(fails.len(), 2, "{ir}");
    assert_eq!(releases(fails[0]), 0, "{ir}");
    assert_eq!(releases(fails[1]), 1, "the first argument: {ir}");
    // The first argument's conversion fails holding both; the second's
    // holds only itself, the first being written and released already.
    let to_str = blocks(&ir, "foreign_to_str_fail");
    assert_eq!(to_str.len(), 2, "{ir}");
    assert_eq!(releases(to_str[0]), 2, "{ir}");
    assert_eq!(releases(to_str[1]), 1, "{ir}");
    // Plus the second argument's `NameError` exit and effect check (one
    // each) and each argument's release after its write.
    assert_eq!(releases(&ir), 8, "{ir}");
    let borrowed = f_ir(
        "release_print_borrowed",
        vec![print(vec![
            copy_name(),
            MirExpr::IntLiteral(1),
            copy_name(),
        ])],
    );
    assert_eq!(releases(&borrowed), 0, "{borrowed}");
}

/// `f"v={copy.a}"` releases the interpolated object once it is formatted;
/// `f"v={copy}"` does not.
#[test]
fn an_fstring_releases_a_produced_interpolation() {
    let ir = discard_ir("release_fstring", fstring(attr("a")));
    assert_eq!(releases(&ir), 2, "{ir}");
    let borrowed = discard_ir("release_fstring_borrowed", fstring(copy_name()));
    assert_eq!(releases(&borrowed), 0, "{borrowed}");
}

/// `raise copy.a` releases its produced operand after the raise has taken
/// its own reference; `raise copy` does not.
#[test]
fn a_raise_releases_its_produced_operand() {
    let raise = |value| MirStmt::ObjRaise { value };
    let ir = f_ir("release_raise", vec![raise(attr("a"))]);
    assert_eq!(releases(&ir), 1, "{ir}");
    let raised = ir
        .find("@pycc_ext_obj_raise(")
        .unwrap_or_else(|| panic!("{ir}"));
    let release = ir.rfind(RELEASE).unwrap_or_else(|| panic!("{ir}"));
    assert!(raised < release, "released after the raise: {ir}");
    let borrowed = f_ir("release_raise_borrowed", vec![raise(copy_name())]);
    assert_eq!(releases(&borrowed), 0, "{borrowed}");
}

/// `1 if copy.a else 2` releases its produced test after the truth test;
/// `1 if copy else 2` does not.
#[test]
fn a_conditional_expression_releases_its_produced_test() {
    let int = MirExpr::IntLiteral;
    let ir = discard_ir(
        "release_if_exp_test",
        if_exp(attr("a"), int(1), int(2), Ty::Int),
    );
    assert_eq!(releases(&ir), 2, "{ir}");
    let borrowed = discard_ir(
        "release_if_exp_test_borrowed",
        if_exp(copy_name(), int(1), int(2), Ty::Int),
    );
    assert_eq!(releases(&borrowed), 0, "{borrowed}");
}

/// A conditional expression whose arms are both produced objects is itself
/// produced, so its discarded result is released; with one borrowed arm it
/// is not, and nothing else is incremented to make it so (#1499).
#[test]
fn a_conditional_object_result_is_produced_only_when_every_arm_is() {
    let ir = discard_ir(
        "release_if_exp_result",
        if_exp(copy_name(), attr("a"), attr("b"), Ty::Object),
    );
    assert_eq!(releases(&ir), 1, "{ir}");
    let mixed = discard_ir(
        "release_if_exp_mixed",
        if_exp(copy_name(), attr("a"), copy_name(), Ty::Object),
    );
    assert_eq!(releases(&mixed), 0, "{mixed}");
}

/// `if copy.a and copy.b:` tests each operand's truth and releases each
/// produced one after its test; borrowed operands are never released.
#[test]
fn a_truth_only_boolean_operator_releases_each_produced_operand() {
    let stmt = |left, right| MirStmt::If {
        test: bool_op(BoolOpKind::And, left, right, Ty::Bool, true),
        body: vec![MirStmt::Return(None)],
        orelse: Vec::new(),
    };
    let ir = f_ir("release_boolop_truth", vec![stmt(attr("a"), attr("b"))]);
    assert_eq!(releases(&ir), 4, "{ir}");
    let borrowed = f_ir(
        "release_boolop_truth_borrowed",
        vec![stmt(copy_name(), copy_name())],
    );
    assert_eq!(releases(&borrowed), 0, "{borrowed}");
}

/// `copy.a or copy.b` (an object result): the left operand is held across
/// its truth test, released when the right is evaluated instead, and
/// otherwise becomes the result, which the discarding statement releases.
#[test]
fn a_value_boolean_operator_releases_an_unselected_left_operand() {
    let ir = discard_ir(
        "release_boolop_value",
        bool_op(BoolOpKind::Or, attr("a"), attr("b"), Ty::Object, false),
    );
    assert_eq!(releases(&ir), 3, "{ir}");
    let mixed = discard_ir(
        "release_boolop_value_mixed",
        bool_op(BoolOpKind::Or, attr("a"), copy_name(), Ty::Object, false),
    );
    assert_eq!(releases(&mixed), 2, "{mixed}");
    let borrowed = discard_ir(
        "release_boolop_value_borrowed",
        bool_op(BoolOpKind::Or, copy_name(), copy_name(), Ty::Object, false),
    );
    assert_eq!(releases(&borrowed), 0, "{borrowed}");
}

/// The classifier's Part 3 arms: a comprehension over an object, and an
/// object-typed `BoolOp`/`IfExp` whose every arm is owned -- a produced
/// object or a converted scalar.
#[test]
fn the_part_3_producers_are_classified() {
    let comprehension = |source| {
        MirExpr::Comprehension(Box::new(pycc_mir::MirComprehension {
            var: "x".to_string(),
            var_ty: Ty::Object,
            source,
            cond: None,
            elt: pycc_mir::MirCompElt::List(MirExpr::IntLiteral(1)),
        }))
    };
    assert!(is_produced(&comprehension(pycc_mir::CompSource::Object(
        copy_name()
    ))));
    let value = |left, right| bool_op(BoolOpKind::Or, left, right, Ty::Object, false);
    assert!(is_produced(&value(attr("a"), attr("b"))));
    assert!(is_produced(&value(attr("a"), MirExpr::IntLiteral(1))));
    assert!(!is_produced(&value(attr("a"), copy_name())));
    assert!(!is_produced(&bool_op(
        BoolOpKind::Or,
        attr("a"),
        attr("b"),
        Ty::Bool,
        true
    )));
    let choose = |body, orelse| if_exp(copy_name(), body, orelse, Ty::Object);
    assert!(is_produced(&choose(attr("a"), attr("b"))));
    assert!(!is_produced(&choose(copy_name(), attr("b"))));
}
