//! Unit tests for `and`/`or` lowering (#1211): the right fold, the value
//! join [`bool_op_result_ty`], the truth-context marker's placement, and the
//! walrus refusal.

use super::*;
use crate::expr::rename_name_in_expr;
use crate::{HirItem, HirStmt, lower_checked, pycc_parser_test_helper};

fn name(n: &str) -> HirExpr {
    HirExpr::Name(n.to_string())
}

fn bool_op(op: BoolOpKind, left: HirExpr, right: HirExpr, truth_only: bool) -> HirExpr {
    HirExpr::BoolOp {
        op,
        left: Box::new(left),
        right: Box::new(right),
        truth_only,
    }
}

/// The body of the first function `source` defines.
fn function_body(source: &str) -> Vec<HirStmt> {
    let module = lower_checked(&pycc_parser_test_helper::parse(source)).unwrap();
    module
        .items
        .into_iter()
        .find_map(|item| match item {
            HirItem::Function { body, .. } => Some(body),
            HirItem::TopLevelStmt(_) => None,
        })
        .expect("the source defines a function")
}

fn lowering_error(source: &str) -> pycc_diag::Diagnostic {
    lower_checked(&pycc_parser_test_helper::parse(source)).unwrap_err()
}

#[test]
fn bool_op_kind_spells_its_python_operator() {
    assert_eq!(BoolOpKind::And.as_str(), "and");
    assert_eq!(BoolOpKind::Or.as_str(), "or");
}

#[test]
fn fold_right_nests_a_two_and_a_three_operand_chain() {
    assert_eq!(
        fold_bool_op(BoolOpKind::Or, vec![name("a"), name("b")]),
        bool_op(BoolOpKind::Or, name("a"), name("b"), false)
    );
    assert_eq!(
        fold_bool_op(BoolOpKind::And, vec![name("a"), name("b"), name("c")]),
        bool_op(
            BoolOpKind::And,
            name("a"),
            bool_op(BoolOpKind::And, name("b"), name("c"), false),
            false
        )
    );
}

#[test]
fn equal_operand_types_join_to_that_type() {
    let optional_int = Ty::Optional(Box::new(Ty::Int));
    let instance = Ty::Instance(Box::new("C".to_string()));
    for ty in [
        Ty::Bool,
        Ty::Int,
        Ty::Float,
        Ty::Str,
        optional_int.clone(),
        instance.clone(),
    ] {
        assert_eq!(
            bool_op_result_ty(BoolOpKind::And, &ty, &ty),
            Some(ty.clone()),
            "{ty:?}"
        );
    }
    // `or` strips a left `Optional[T]` to `T` first, so two equal
    // `Optional` operands join as `Optional[T]` through the `T` with
    // `Optional[T]` row.
    assert_eq!(
        bool_op_result_ty(BoolOpKind::Or, &optional_int, &optional_int),
        Some(optional_int)
    );
}

#[test]
fn bool_and_int_join_to_int_in_either_order() {
    for op in [BoolOpKind::And, BoolOpKind::Or] {
        assert_eq!(bool_op_result_ty(op, &Ty::Bool, &Ty::Int), Some(Ty::Int));
        assert_eq!(bool_op_result_ty(op, &Ty::Int, &Ty::Bool), Some(Ty::Int));
    }
}

#[test]
fn a_bare_type_and_its_optional_join_to_the_optional() {
    let optional_int = Ty::Optional(Box::new(Ty::Int));
    assert_eq!(
        bool_op_result_ty(BoolOpKind::And, &Ty::Int, &optional_int),
        Some(optional_int.clone())
    );
    assert_eq!(
        bool_op_result_ty(BoolOpKind::And, &optional_int, &Ty::Int),
        Some(optional_int.clone())
    );
    assert_eq!(
        bool_op_result_ty(BoolOpKind::Or, &Ty::Int, &optional_int),
        Some(optional_int)
    );
}

#[test]
fn or_joins_a_left_optional_as_its_payload() {
    let optional_float = Ty::Optional(Box::new(Ty::Float));
    assert_eq!(
        bool_op_result_ty(BoolOpKind::Or, &optional_float, &Ty::Float),
        Some(Ty::Float)
    );
}

#[test]
fn operands_with_no_common_type_are_refused() {
    let pairs = [
        (Ty::Int, Ty::Str),
        (Ty::Int, Ty::Float),
        (Ty::Float, Ty::Bool),
        (
            Ty::Instance(Box::new("A".to_string())),
            Ty::Instance(Box::new("B".to_string())),
        ),
        (Ty::None, Ty::None),
        (
            Ty::Optional(Box::new(Ty::Int)),
            Ty::Optional(Box::new(Ty::Float)),
        ),
    ];
    for (left, right) in pairs {
        assert_eq!(
            bool_op_result_ty(BoolOpKind::And, &left, &right),
            None,
            "{left:?} and {right:?}"
        );
    }
    // `and` does not strip a left `Optional`: `Optional[float]` with
    // `float` joins as `Optional[float]`, and `Optional[int]` with `float`
    // has no common type.
    assert_eq!(
        bool_op_result_ty(
            BoolOpKind::And,
            &Ty::Optional(Box::new(Ty::Int)),
            &Ty::Float
        ),
        None
    );
}

#[test]
fn if_elif_and_while_tests_are_truth_context() {
    let body = function_body(
        "def f(a: int, b: str) -> None:\n    if a and b:\n        pass\n    elif b or a:\n        pass\n    while a and b:\n        pass\n",
    );
    let HirStmt::If { test, orelse, .. } = &body[0] else {
        panic!("expected an if, got {:?}", body[0]);
    };
    assert_eq!(*test, bool_op(BoolOpKind::And, name("a"), name("b"), true));
    let HirStmt::If { test, .. } = &orelse[0] else {
        panic!("expected the elif as a nested if, got {:?}", orelse[0]);
    };
    assert_eq!(*test, bool_op(BoolOpKind::Or, name("b"), name("a"), true));
    let HirStmt::While { test, .. } = &body[1] else {
        panic!("expected a while, got {:?}", body[1]);
    };
    assert_eq!(*test, bool_op(BoolOpKind::And, name("a"), name("b"), true));
}

#[test]
fn a_not_operand_and_a_nested_chain_are_truth_context() {
    let body = function_body(
        "def f(a: int, b: str, c: bool) -> None:\n    x = not (a or b and c)\n    if not (a and b):\n        pass\n",
    );
    let HirStmt::Assign { value, .. } = &body[0] else {
        panic!("expected an assignment, got {:?}", body[0]);
    };
    assert_eq!(
        *value,
        HirExpr::UnaryOp {
            op: UnaryOpKind::Not,
            operand: Box::new(bool_op(
                BoolOpKind::Or,
                name("a"),
                bool_op(BoolOpKind::And, name("b"), name("c"), true),
                true
            )),
        }
    );
    let HirStmt::If { test, .. } = &body[1] else {
        panic!("expected an if, got {:?}", body[1]);
    };
    assert_eq!(
        *test,
        HirExpr::UnaryOp {
            op: UnaryOpKind::Not,
            operand: Box::new(bool_op(BoolOpKind::And, name("a"), name("b"), true)),
        }
    );
}

#[test]
fn a_walrus_or_a_comparison_under_a_condition_keeps_value_context() {
    let body = function_body(
        "def f(a: int, b: int, c: int) -> None:\n    if (x := a or b):\n        pass\n    if (a or b) == c:\n        pass\n",
    );
    let HirStmt::If { test, .. } = &body[0] else {
        panic!("expected an if, got {:?}", body[0]);
    };
    let HirExpr::NamedExpr { value, .. } = test else {
        panic!("expected a walrus test, got {test:?}");
    };
    assert_eq!(
        **value,
        bool_op(BoolOpKind::Or, name("a"), name("b"), false)
    );
    let HirStmt::If { test, .. } = &body[1] else {
        panic!("expected an if, got {:?}", body[1]);
    };
    let HirExpr::Compare { left, .. } = test else {
        panic!("expected a comparison test, got {test:?}");
    };
    assert_eq!(**left, bool_op(BoolOpKind::Or, name("a"), name("b"), false));
}

#[test]
fn an_assignment_and_a_match_guard_are_value_context() {
    let body = function_body(
        "def f(n: int, s: int) -> None:\n    y = n or s\n    match n:\n        case _ if n and s:\n            pass\n",
    );
    let HirStmt::Assign { value, .. } = &body[0] else {
        panic!("expected an assignment, got {:?}", body[0]);
    };
    assert_eq!(*value, bool_op(BoolOpKind::Or, name("n"), name("s"), false));
    let HirStmt::Match { cases, .. } = &body[1] else {
        panic!("expected a match, got {:?}", body[1]);
    };
    assert_eq!(
        cases[0].guard,
        Some(bool_op(BoolOpKind::And, name("n"), name("s"), false))
    );
}

#[test]
fn a_comprehension_filter_is_truth_context() {
    let body = function_body("def f(a: int) -> None:\n    xs = {i for i in range(5) if i and a}\n");
    let HirStmt::SetCompAssign { cond, var, .. } = &body[0] else {
        panic!("expected a set comprehension, got {:?}", body[0]);
    };
    let cond = cond.as_deref().expect("the comprehension has a filter");
    assert_eq!(*cond, bool_op(BoolOpKind::And, name(var), name("a"), true));
}

#[test]
fn renaming_inside_a_boolean_operator_keeps_its_context_flag() {
    let renamed = rename_name_in_expr(
        bool_op(BoolOpKind::Or, name("i"), name("j"), true),
        "i",
        "k",
    );
    assert_eq!(renamed, bool_op(BoolOpKind::Or, name("k"), name("j"), true));
}

#[test]
fn a_walrus_in_a_short_circuited_operand_is_refused() {
    let err =
        lowering_error("def f(a: int, b: int) -> None:\n    if a or (n := b):\n        print(n)\n");
    assert_eq!(err.code, "C0001");
    assert_eq!(
        err.message,
        "a walrus assignment (`:=`) in a short-circuited `and`/`or` operand is not supported"
    );
    // A third operand is short-circuited too.
    let err = lowering_error(
        "def f(a: int, b: int) -> None:\n    if a and b and (n := b):\n        print(n)\n",
    );
    assert_eq!(err.code, "C0001");
}

#[test]
fn a_walrus_in_the_first_operand_is_admitted() {
    let body =
        function_body("def f(a: int, b: int) -> None:\n    if (n := a) and b:\n        print(n)\n");
    let HirStmt::If { test, .. } = &body[0] else {
        panic!("expected an if, got {:?}", body[0]);
    };
    let HirExpr::BoolOp { left, .. } = test else {
        panic!("expected a boolean operator, got {test:?}");
    };
    assert!(matches!(**left, HirExpr::NamedExpr { .. }), "{left:?}");
}

#[test]
fn an_unsupported_operand_propagates_its_own_error() {
    let err = lowering_error("def f(a: int) -> None:\n    x = a or (lambda: 1)\n");
    assert_eq!(err.code, "C0001");
    assert!(!err.message.contains("walrus"), "{}", err.message);
}

/// Part 6 of #1371: an `object` joins an `object` or a boxable scalar, in
/// either order, to `object`. Every other pairing with an `object` is
/// refused, including a left `Optional` that `or` would otherwise strip.
#[test]
fn an_object_joins_an_object_or_a_boxable_scalar_to_object() {
    for op in [BoolOpKind::And, BoolOpKind::Or] {
        for other in [Ty::Object, Ty::Bool, Ty::Int, Ty::Float, Ty::Str] {
            assert_eq!(
                bool_op_result_ty(op, &Ty::Object, &other),
                Some(Ty::Object),
                "{op:?} object, {other:?}"
            );
            assert_eq!(
                bool_op_result_ty(op, &other, &Ty::Object),
                Some(Ty::Object),
                "{op:?} {other:?}, object"
            );
        }
        for other in [
            Ty::None,
            Ty::Optional(Box::new(Ty::Int)),
            Ty::List(Box::new(Ty::Int)),
            Ty::Instance(Box::new("C".to_string())),
        ] {
            assert_eq!(bool_op_result_ty(op, &Ty::Object, &other), None);
            assert_eq!(bool_op_result_ty(op, &other, &Ty::Object), None);
        }
    }
}
