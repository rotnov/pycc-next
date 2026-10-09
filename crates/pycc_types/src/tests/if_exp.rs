//! Conditional-expression typing (#1395): the branch join in both the
//! validation pass and the solver, the condition admission, and every
//! walker arm that must reach all three parts of the node.

use super::*;

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

/// The type of `a if c else b` with `c`, `a`, `b` bound to the given types.
fn if_exp_ty(test: Ty, body: Ty, orelse: Ty) -> Result<Ty, pycc_diag::Diagnostic> {
    let mut env = Environment::new();
    env.bind("c".to_string(), test);
    env.bind("a".to_string(), body);
    env.bind("b".to_string(), orelse);
    infer_expr(
        &env,
        &HirExpr::IfExp {
            test: Box::new(HirExpr::Name("c".to_string())),
            body: Box::new(HirExpr::Name("a".to_string())),
            orelse: Box::new(HirExpr::Name("b".to_string())),
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
        ("a: list[int], b: list[int]", "list[int]"),
        ("a: C, b: C", "C"),
        ("a: int, b: int | None", "int | None"),
        ("a: int | None, b: int", "int | None"),
        ("a: int | None, b: int | None", "int | None"),
        ("a: float | None, b: float", "float | None"),
    ] {
        checks(&format!(
            "class C:\n    def __init__(self) -> None:\n        self.n = 1\n\ndef f(t: bool, {params}) -> {ret}:\n    return a if t else b\n"
        ));
    }
    checks("def f(t: bool, a: int) -> int | None:\n    return a if t else None\n");
    checks("def f(t: bool, a: bool) -> bool | None:\n    return None if t else a\n");
}

#[test]
fn branches_with_no_common_type_are_refused_naming_both() {
    for (params, body, orelse, names) in [
        ("a: int, b: str", "a", "b", "int and str"),
        ("a: bool, b: int", "a", "b", "bool and int"),
        ("a: int, b: float", "a", "b", "int and float"),
        ("a: str", "a", "None", "str and None"),
        ("a: int", "None", "None", "None and None"),
    ] {
        refused(
            &format!("def f(t: bool, {params}) -> None:\n    print({body} if t else {orelse})\n"),
            "T0021",
            &format!(
                "conditional expression branches have no common type: {names} (pycc has no union types)"
            ),
        );
    }
}

#[test]
fn an_object_branch_does_not_join_a_native_one() {
    let err = if_exp_ty(Ty::Bool, Ty::Object, Ty::Int).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("object and int"), "{}", err.message);
    assert_eq!(
        if_exp_ty(Ty::Bool, Ty::Object, Ty::Object).unwrap(),
        Ty::Object
    );
}

#[test]
fn every_truth_testable_condition_is_admitted() {
    for test in [
        Ty::Bool,
        Ty::Int,
        Ty::Float,
        Ty::Str,
        Ty::None,
        Ty::Optional(Box::new(Ty::Int)),
        Ty::Object,
        Ty::Set(Box::new(Ty::Int)),
        Ty::FrozenSet(Box::new(Ty::Int)),
    ] {
        assert_eq!(
            if_exp_ty(test.clone(), Ty::Int, Ty::Int).unwrap(),
            Ty::Int,
            "{test:?}"
        );
    }
}

#[test]
fn a_condition_with_no_testable_truth_is_refused() {
    for (param, name) in [
        ("xs: list[int]", "list[int]"),
        ("d: dict[str, int]", "dict[str, int]"),
        ("p: tuple[int, int]", "tuple[int, int]"),
    ] {
        let arg = param.split(':').next().unwrap();
        refused(
            &format!("def f({param}) -> int:\n    return 1 if {arg} else 2\n"),
            "T0021",
            &format!(
                "conditional expression condition of type `{name}` has no truth value pycc can test"
            ),
        );
    }
}

#[test]
fn an_instance_condition_whose_class_defines_a_truth_dunder_is_refused() {
    refused(
        "class C:\n    def __init__(self) -> None:\n        self.n = 0\n    def __bool__(self) -> bool:\n        return False\n\ndef f(c: C) -> int:\n    return 1 if c else 2\n",
        "T0021",
        "conditional expression condition of type `C` is not supported: class `C` defines `__bool__`, and pycc does not call it for a truth test yet",
    );
    checks(
        "class C:\n    def __init__(self) -> None:\n        self.n = 0\n\ndef f(c: C) -> int:\n    return 1 if c else 2\n",
    );
}

#[test]
fn the_solver_joins_concrete_branches_of_an_unannotated_helper() {
    checks("def _g(t: bool):\n    return 1 if t else 2\n\ndef f() -> int:\n    return _g(True)\n");
    checks(
        "def _g(t: bool):\n    return 1 if t else None\n\ndef f() -> int | None:\n    return _g(True)\n",
    );
    refused(
        "def _g(t: bool):\n    return 1 if t else \"s\"\n\nprint(_g(True))\n",
        "T0021",
        "cannot infer return type of private helper `_g`",
    );
}

#[test]
fn the_solver_unifies_an_inference_variable_branch() {
    checks(
        "def _g(a, t: bool):\n    return a if t else 1\n\ndef f() -> int:\n    return _g(2, True)\n",
    );
    checks("def _g(a, b):\n    return a if True else b\n\ndef f() -> int:\n    return _g(1, 2)\n");
    refused(
        "def _g(a):\n    return a if True else \"s\"\n\nprint(_g(1))\n",
        "T0021",
        "",
    );
}

#[test]
fn the_solver_yields_no_term_for_a_variable_against_none() {
    refused(
        "def _g(a):\n    return a if True else None\n\nprint(_g(1))\n",
        "T0021",
        "cannot infer return type of private helper `_g`",
    );
    refused(
        "def _g(a):\n    return None if True else a\n\nprint(_g(1))\n",
        "T0021",
        "cannot infer return type of private helper `_g`",
    );
}

#[test]
fn the_solver_defers_a_branch_with_no_type_term() {
    refused(
        "def _g(c: bool):\n    if c:\n        v = 1\n    return v if c else 2\n\nprint(_g(True))\n",
        "T0021",
        "cannot infer return type of private helper `_g`",
    );
}

#[test]
fn a_walrus_in_the_condition_binds_in_the_solver() {
    checks(
        "def _g(a: int):\n    if (1 if (n := a) else 0):\n        return n\n    return 0\n\ndef f() -> int:\n    return _g(1)\n",
    );
}

#[test]
fn a_walrus_in_the_condition_of_an_if_test_type_checks() {
    checks(
        "def f(a: int) -> int:\n    if (1 if (n := a) else 0):\n        return n\n    return 0\n",
    );
}

#[test]
fn a_protocol_call_inside_a_conditional_expression_is_specialized() {
    let source = "from typing import Protocol\nclass P(Protocol):\n    def m(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.n = 0\n    def m(self) -> int:\n        return 0\n\ndef use(p: P) -> int:\n    return p.m()\n\nprint(use(C()) if use(C()) else use(C()))\n";
    let resolved = check_source(source).expect("must type-check");
    let debug = format!("{resolved:?}");
    assert!(
        !debug.contains("callee: \"use\""),
        "every call inside the conditional expression must be rewritten to its specialization: {debug}"
    );
}

#[test]
fn a_generic_call_inside_a_conditional_expression_is_specialized() {
    checks(
        "def ident[T](x: T) -> T:\n    return x\n\ndef f(t: bool) -> int:\n    return ident(1) if ident(t) else ident(2)\n",
    );
}

#[test]
fn a_generic_class_instantiation_inside_a_conditional_expression_is_collected() {
    checks(
        "class Box[T]:\n    def __init__(self, v: T) -> None:\n        self.v = v\n\ndef f(t: bool) -> int:\n    b = Box[int](1) if t else Box[int](2)\n    return b.v\n",
    );
}

/// A generic function's body is walked for calls to generic functions, and
/// the walk reaches every part of a conditional expression.
#[test]
fn a_generic_call_inside_a_conditional_expression_in_a_generic_function_is_refused() {
    for expr in ["g(x) if t else x", "x if g(t) else x", "x if t else g(x)"] {
        refused(
            &format!(
                "def g[T](x: T) -> T:\n    return x\n\ndef f[T](x: T, t: bool) -> T:\n    return {expr}\n"
            ),
            "T0042",
            "generic function `f` calls generic function `g`",
        );
    }
}

/// The same walk passes a conditional expression with no generic call.
#[test]
fn a_conditional_expression_in_a_generic_function_without_a_generic_call_checks() {
    checks(
        "def f[T](x: T, y: T, t: bool) -> T:\n    return x if t else y\n\n\
         def main() -> int:\n    return f(1, 2, True)\n",
    );
}

#[test]
fn an_empty_list_branch_is_refused_with_t0003() {
    refused(
        "def f(t: bool) -> int:\n    xs: list[int] = [] if t else [1]\n    return len(xs)\n",
        "T0003",
        "an empty list literal has no inferable element type",
    );
}
