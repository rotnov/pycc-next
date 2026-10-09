//! `and`/`or` typing (#1211, Part 3 of #1018): the value join in both the
//! validation pass and the solver, the truth context, and every operand
//! refusal `boolop.rs` owns.

use super::*;
use crate::unop::is_truth_testable;
use pycc_hir::BoolOpKind;

fn checks(source: &str) {
    if let Err(err) = check_source(source) {
        panic!(
            "{source}\nexpected to type-check, got {}: {}",
            err.code, err.message
        );
    }
}

fn refused(source: &str, code: &str, message: &str) {
    let err = check_source(source).expect_err(source);
    assert_eq!(err.code, code, "{source}: {}", err.message);
    assert!(
        err.message.contains(message),
        "{source}\nexpected a message containing {message:?}, got {:?}",
        err.message
    );
}

/// The type of `expr` with `a`, `b` bound to `left`, `right`.
fn value_ty(op: BoolOpKind, left: Ty, right: Ty) -> Result<Ty, pycc_diag::Diagnostic> {
    let mut env = Environment::new();
    env.bind("a".to_string(), left);
    env.bind("b".to_string(), right);
    infer_expr(
        &env,
        &HirExpr::BoolOp {
            op,
            left: Box::new(HirExpr::Name("a".to_string())),
            right: Box::new(HirExpr::Name("b".to_string())),
            truth_only: false,
        },
    )
}

#[test]
fn every_join_row_types_an_annotated_function() {
    for (params, ret) in [
        ("a: int, b: int", "int"),
        ("a: bool, b: bool", "bool"),
        ("a: float, b: float", "float"),
        ("a: str, b: str", "str"),
        ("a: bool, b: int", "int"),
        ("a: int, b: bool", "int"),
        ("a: int, b: int | None", "int | None"),
        ("a: int | None, b: int", "int"),
        ("a: int | None, b: int | None", "int | None"),
        ("a: float | None, b: float", "float"),
        ("a: C, b: C", "C"),
    ] {
        checks(&format!(
            "class C:\n    def __init__(self) -> None:\n        self.n = 1\n\ndef f({params}) -> {ret}:\n    return a or b\n"
        ));
    }
    // `and` keeps a left `Optional` whole.
    checks("def f(a: int | None, b: int) -> int | None:\n    return a and b\n");
}

#[test]
fn a_value_join_with_no_common_type_is_refused() {
    refused(
        "def f(a: int, b: str) -> None:\n    print(a or b)\n",
        "T0021",
        "`or` operands have no common type: int and str (pycc has no union types)",
    );
    refused(
        "def f(a: int, b: float) -> None:\n    print(a and b)\n",
        "T0021",
        "`and` operands have no common type: int and float",
    );
}

#[test]
fn a_container_operand_is_refused_in_both_contexts() {
    refused(
        "def f(xs: list[int], b: int) -> None:\n    if xs and b:\n        print(1)\n",
        "T0021",
        "`and` operand of type `list[int]` has no truth value pycc can test",
    );
    refused(
        "def f(a: int, xs: dict[str, int]) -> None:\n    print(a or xs)\n",
        "T0021",
        "`or` operand of type `dict[str, int]` has no truth value pycc can test",
    );
}

#[test]
fn a_none_operand_is_refused_only_in_value_context() {
    refused(
        "def f(a: int) -> None:\n    print(a or None)\n",
        "T0021",
        "`or` operand `None` has no value pycc can join (pycc has no union types)",
    );
    checks("def f(a: int) -> None:\n    if a or None:\n        print(1)\n");
}

#[test]
fn an_instance_whose_class_defines_a_truth_dunder_is_refused() {
    refused(
        "class C:\n    def __init__(self) -> None:\n        self.n = 0\n    def __bool__(self) -> bool:\n        return False\n\ndef f(c: C) -> None:\n    if c and True:\n        print(1)\n",
        "T0021",
        "`and` operand of type `C` is not supported: class `C` defines `__bool__`, and pycc does not call it for a truth test yet",
    );
    // Found through a base class, in value context.
    refused(
        "class B:\n    def __init__(self) -> None:\n        self.n = 0\n    def __len__(self) -> int:\n        return 0\n\nclass D(B):\n    def __init__(self) -> None:\n        self.n = 1\n\ndef f(d: D, e: D) -> D:\n    return d or e\n",
        "T0021",
        "`or` operand of type `D` is not supported: class `B` defines `__len__`",
    );
}

#[test]
fn an_instance_of_a_class_without_a_truth_dunder_is_admitted() {
    checks(
        "class C:\n    def __init__(self) -> None:\n        self.n = 0\n    def size(self) -> int:\n        return 0\n\ndef f(c: C) -> None:\n    if c and c.size():\n        print(1)\n",
    );
}

/// Part 6 of #1371: an object joins an object or a boxable scalar to
/// `object`, in either order and under either operator.
#[test]
fn a_cpython_object_operand_joins_to_object() {
    for op in [BoolOpKind::And, BoolOpKind::Or] {
        for other in [Ty::Object, Ty::Bool, Ty::Int, Ty::Float, Ty::Str] {
            assert_eq!(value_ty(op, Ty::Object, other.clone()).unwrap(), Ty::Object);
            assert_eq!(value_ty(op, other, Ty::Object).unwrap(), Ty::Object);
        }
    }
}

/// An object paired with a value pycc cannot box is `I0404` naming the
/// other operand's type, whichever side the object is on.
#[test]
fn a_cpython_object_with_an_unboxable_operand_is_refused_with_i0404() {
    let instance = Ty::Instance(Box::new("C".to_string()));
    let err = value_ty(BoolOpKind::Or, Ty::Object, instance).unwrap_err();
    assert_eq!(err.code, "I0404");
    assert!(
        err.message
            .contains("joining a CPython object with a `C` value in an `or`"),
        "{}",
        err.message
    );
    let optional = Ty::Optional(Box::new(Ty::Int));
    let expected = format!(
        "joining a CPython object with a `{}` value in an `and`",
        optional.name()
    );
    let err = value_ty(BoolOpKind::And, optional, Ty::Object).unwrap_err();
    assert_eq!(err.code, "I0404");
    assert!(err.message.contains(&expected), "{}", err.message);
}

/// In truth context an object operand is admitted next to any
/// truth-testable operand, the node typed `bool`.
#[test]
fn a_cpython_object_operand_is_admitted_in_truth_context() {
    let mut env = Environment::new();
    env.bind("a".to_string(), Ty::Bool);
    env.bind("o".to_string(), Ty::Object);
    for (left, right) in [("a", "o"), ("o", "a"), ("o", "o")] {
        let truth = HirExpr::BoolOp {
            op: BoolOpKind::And,
            left: Box::new(HirExpr::Name(left.to_string())),
            right: Box::new(HirExpr::Name(right.to_string())),
            truth_only: true,
        };
        assert_eq!(infer_expr(&env, &truth).unwrap(), Ty::Bool);
    }
}

#[test]
fn value_context_types_the_join_and_truth_context_types_bool() {
    assert_eq!(
        value_ty(BoolOpKind::Or, Ty::Bool, Ty::Int).unwrap(),
        Ty::Int
    );
    let mut env = Environment::new();
    env.bind("a".to_string(), Ty::Int);
    env.bind("b".to_string(), Ty::Str);
    let truth = HirExpr::BoolOp {
        op: BoolOpKind::And,
        left: Box::new(HirExpr::Name("a".to_string())),
        right: Box::new(HirExpr::Name("b".to_string())),
        truth_only: true,
    };
    assert_eq!(infer_expr(&env, &truth).unwrap(), Ty::Bool);
}

#[test]
fn mixed_types_are_admitted_in_truth_context() {
    checks(
        "def f(n: int, s: str, x: float, o: int | None) -> None:\n    if n and s or x and o:\n        print(1)\n    while n and s:\n        n = 0\n    print(not (n or s))\n",
    );
}

#[test]
fn the_solver_joins_concrete_operands_of_an_unannotated_helper() {
    checks("def _g():\n    return 1 or 2\n\ndef f() -> int:\n    return _g()\n");
    checks("def _g():\n    return True or 2\n\ndef f() -> int:\n    return _g()\n");
    // A concrete pair with no join yields no term, so the helper's return
    // type cannot be inferred.
    refused(
        "def _g():\n    return 1 or \"s\"\n\nprint(_g())\n",
        "T0021",
        "cannot infer return type of private helper `_g`",
    );
}

#[test]
fn the_solver_unifies_inference_variable_operands() {
    checks("def _g(a, b):\n    return a or b\n\ndef f() -> int:\n    return _g(1, 2)\n");
    checks("def _g(a):\n    return a and 1\n\ndef f() -> int:\n    return _g(3)\n");
    refused(
        "def _g(a):\n    return a or \"s\"\n\nprint(_g(1))\n",
        "T0021",
        "",
    );
}

#[test]
fn the_solver_defers_an_operand_with_no_type_term() {
    // `v` is maybe-bound, so the solver has no term for it and the node
    // yields none either: the helper's return type cannot be inferred.
    refused(
        "def _g(c: bool):\n    if c:\n        v = 1\n    return v or 2\n\nprint(_g(True))\n",
        "T0021",
        "cannot infer return type of private helper `_g`",
    );
}

#[test]
fn the_solver_types_a_truth_context_node_bool() {
    checks(
        "def _g(a: int, s: str):\n    if a and s:\n        return 1\n    return 0\n\ndef f() -> int:\n    return _g(1, \"x\")\n",
    );
}

#[test]
fn a_walrus_in_the_first_operand_binds_in_the_solver() {
    checks(
        "def _g(a: int, b: int):\n    if (n := a) and b:\n        return n\n    return 0\n\ndef f() -> int:\n    return _g(1, 2)\n",
    );
}

#[test]
fn a_type_checking_marker_operand_keeps_its_refusal() {
    refused(
        "import typing\nif typing.TYPE_CHECKING and True:\n    print(1)\n",
        "T0021",
        "compile-time marker",
    );
}

#[test]
fn a_protocol_call_inside_or_is_specialized() {
    let source = "from typing import Protocol\nclass P(Protocol):\n    def m(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.n = 0\n    def m(self) -> int:\n        return 0\n\ndef use(p: P) -> int:\n    return p.m()\n\nprint(use(C()) or 7)\n";
    let resolved = check_source(source).expect("must type-check");
    let debug = format!("{resolved:?}");
    assert!(
        !debug.contains("callee: \"use\""),
        "the call inside `or` must be rewritten to its specialization: {debug}"
    );
}

#[test]
fn truth_testable_admits_scalars_optionals_and_instances_only() {
    for ty in [
        Ty::Bool,
        Ty::Int,
        Ty::Float,
        Ty::Str,
        Ty::None,
        Ty::Optional(Box::new(Ty::Int)),
        Ty::Instance(Box::new("C".to_string())),
    ] {
        assert!(is_truth_testable(&ty), "{ty:?}");
    }
    for ty in [Ty::List(Box::new(Ty::Int)), Ty::Object] {
        assert!(!is_truth_testable(&ty), "{ty:?}");
    }
}

/// A generic function's body is walked for calls to generic functions, and
/// the walk reaches both operands of `and`/`or`.
#[test]
fn a_generic_call_inside_or_in_a_generic_function_is_refused() {
    refused(
        "def g[T](x: T) -> T:\n    return x\n\ndef f[T](x: T) -> T:\n    return x or g(x)\n",
        "T0042",
        "generic function `f` calls generic function `g`",
    );
}
