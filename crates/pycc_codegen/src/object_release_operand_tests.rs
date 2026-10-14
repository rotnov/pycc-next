//! Part 3 of #1092: the release of unbound produced operands of `print`,
//! f-strings, `raise`, conditional expressions and boolean operators.
//!
//! Each test pairs a produced operand (`copy.a`, a new reference the
//! consumer must release) with its borrowed control (`copy`, a module
//! global the consumer must never release), so a test that passes because
//! nothing is ever released, or because everything is, fails one side.
//! Each `copy.a` load also pins `copy` ([`PINNED_LOAD`]), and so does a
//! consumer that runs code on a bare `copy` (`print`, an f-string), which
//! then releases it like a produced operand (`operand_pin`).

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
/// is released once its text is written. In `print(copy, 1, copy)` each
/// module global is pinned (a `__repr__` could rebind it) and released the
/// same way.
#[test]
fn print_releases_each_produced_argument_after_writing_it() {
    let ir = f_ir(
        "release_print",
        vec![print(vec![attr("a"), MirExpr::IntLiteral(1), attr("b")])],
    );
    let fails = blocks(&ir, "foreign_attr_fail");
    assert_eq!(fails.len(), 2, "{ir}");
    assert_eq!(releases(fails[0]), 1, "the pinned `copy`: {ir}");
    assert_eq!(releases(fails[1]), 2, "the first argument and `copy`: {ir}");
    // The first argument's conversion fails holding both; the second's
    // holds only itself, the first being written and released already.
    let to_str = blocks(&ir, "foreign_to_str_fail");
    assert_eq!(to_str.len(), 2, "{ir}");
    assert_eq!(releases(to_str[0]), 2, "{ir}");
    assert_eq!(releases(to_str[1]), 1, "{ir}");
    // Plus the second argument's `NameError` exit and effect check (one
    // each) and each argument's release after its write.
    assert_eq!(releases(&ir), 8 + 2 * PINNED_LOAD, "{ir}");
    let borrowed = f_ir(
        "release_print_borrowed",
        vec![print(vec![
            copy_name(),
            MirExpr::IntLiteral(1),
            copy_name(),
        ])],
    );
    // Each `copy` is pinned across `print` and held like a produced
    // argument: the conversions' failure edges (two, one), the second
    // read's `NameError` exit, and each write.
    assert_eq!(releases(&borrowed), 6, "{borrowed}");
}

/// `f"v={copy.a}"` releases the interpolated object once it is formatted;
/// `f"v={copy}"` releases only `copy`, pinned across the formatting.
#[test]
fn an_fstring_releases_a_produced_interpolation() {
    let ir = discard_ir("release_fstring", fstring(attr("a")));
    assert_eq!(releases(&ir), 2 + PINNED_LOAD, "{ir}");
    let borrowed = discard_ir("release_fstring_borrowed", fstring(copy_name()));
    assert_eq!(
        releases(&borrowed),
        PINNED_LOAD,
        "only the pinned `copy`: {borrowed}"
    );
}

/// `raise copy.a` releases its produced operand after the raise has taken
/// its own reference; `raise copy` does not.
#[test]
fn a_raise_releases_its_produced_operand() {
    let raise = |value| MirStmt::ObjRaise { value };
    let ir = f_ir("release_raise", vec![raise(attr("a"))]);
    assert_eq!(releases(&ir), 1 + PINNED_LOAD, "{ir}");
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
    assert_eq!(releases(&ir), 2 + PINNED_LOAD, "{ir}");
    let borrowed = discard_ir(
        "release_if_exp_test_borrowed",
        if_exp(copy_name(), int(1), int(2), Ty::Int),
    );
    assert_eq!(releases(&borrowed), 0, "{borrowed}");
}

/// A conditional expression's object result is always owned: a produced arm
/// is moved into it and a borrowed arm retained, so its discarded result is
/// released either way.
#[test]
fn a_conditional_object_result_is_always_owned() {
    let ir = discard_ir(
        "release_if_exp_result",
        if_exp(copy_name(), attr("a"), attr("b"), Ty::Object),
    );
    assert_eq!(releases(&ir), 1 + 2 * PINNED_LOAD, "{ir}");
    let mixed = discard_ir(
        "release_if_exp_mixed",
        if_exp(copy_name(), attr("a"), copy_name(), Ty::Object),
    );
    assert_eq!(releases(&mixed), 1 + PINNED_LOAD, "{mixed}");
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
    assert_eq!(releases(&ir), 4 + 2 * PINNED_LOAD, "{ir}");
    let borrowed = f_ir(
        "release_boolop_truth_borrowed",
        vec![stmt(copy_name(), copy_name())],
    );
    assert_eq!(releases(&borrowed), 0, "{borrowed}");
}

/// `copy.a or copy.b` (an object result): the left operand is held across
/// its truth test, released when the right is evaluated instead, and
/// otherwise becomes the result, which the discarding statement releases. A
/// borrowed arm is retained into the result, which is always owned.
#[test]
fn a_value_boolean_operator_releases_an_unselected_left_operand() {
    let ir = discard_ir(
        "release_boolop_value",
        bool_op(BoolOpKind::Or, attr("a"), attr("b"), Ty::Object, false),
    );
    assert_eq!(releases(&ir), 3 + 2 * PINNED_LOAD, "{ir}");
    let mixed = discard_ir(
        "release_boolop_value_mixed",
        bool_op(BoolOpKind::Or, attr("a"), copy_name(), Ty::Object, false),
    );
    assert_eq!(releases(&mixed), 3 + PINNED_LOAD, "{mixed}");
    let borrowed = discard_ir(
        "release_boolop_value_borrowed",
        bool_op(BoolOpKind::Or, copy_name(), copy_name(), Ty::Object, false),
    );
    assert_eq!(releases(&borrowed), 1, "{borrowed}");
}

/// The classifier: a comprehension over an object, and every object-typed
/// `BoolOp`/`IfExp` (whose arms are moved or retained into the result) are
/// produced; a truth-only `BoolOp` is a `bool`.
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
    assert!(is_produced(&value(attr("a"), copy_name())));
    assert!(!is_produced(&bool_op(
        BoolOpKind::Or,
        attr("a"),
        attr("b"),
        Ty::Bool,
        true
    )));
    let choose = |body, orelse| if_exp(copy_name(), body, orelse, Ty::Object);
    assert!(is_produced(&choose(attr("a"), attr("b"))));
    assert!(is_produced(&choose(copy_name(), attr("b"))));
}

/// A fresh native `str` argument of a call on a CPython object
/// (`copy.m("s")`) is copied by its packer, so it is still the compiled
/// code's: the call releases it once the call is done, and the call's own
/// failure edge releases it too (`native_operand`). A borrowed `str` read
/// (the parameter `s`) is never released there -- its slot owns it.
#[test]
fn a_fresh_native_str_argument_is_released_after_the_call() {
    const STR_DECREF: &str = "call void @pycc_rt_str_decref(";
    let call = |arg: MirExpr| MirExpr::ObjMethodCall {
        base: boxed(copy_name()),
        method: "m".to_string(),
        args: vec![arg],
    };
    let ir = discard_ir(
        "release_native_str_arg",
        call(MirExpr::StringLiteral("s".to_string())),
    );
    for label in ["foreign_call_cont", "foreign_call_fail"] {
        let found = blocks(&ir, label);
        assert_eq!(found.len(), 1, "{label}\n{ir}");
        assert_eq!(found[0].matches(STR_DECREF).count(), 1, "{label}\n{ir}");
    }
    let ir = functions_ir(
        "borrowed_native_str_arg",
        vec![MirItem::Function {
            name: "f".to_string(),
            params: vec![("s".to_string(), Ty::Str)],
            return_ty: Ty::None,
            body: vec![
                MirStmt::ExprStmt(call(MirExpr::Name {
                    name: "s".to_string(),
                    ty: Ty::Str,
                })),
                MirStmt::Return(None),
            ],
        }],
        &["pyfn_f"],
    )
    .remove(0);
    for label in ["foreign_call_cont", "foreign_call_fail"] {
        let found = blocks(&ir, label);
        assert_eq!(found.len(), 1, "{label}\n{ir}");
        assert!(!found[0].contains(STR_DECREF), "{label}\n{ir}");
    }
}
