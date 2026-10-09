//! Chained-comparison typing (#1212, Part 4 of #1018): per-link admission in
//! the validation pass, the solver, and every walker's chain arm.

use super::*;
use pycc_hir::{CmpOpKind, CompareLink};

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

/// The type of `a op1 b op2 c` with the three names bound to `tys`.
fn chain_ty(tys: [Ty; 3], ops: [CmpOpKind; 2]) -> Result<Ty, pycc_diag::Diagnostic> {
    let mut env = Environment::new();
    for (name, ty) in ["a", "b", "c"].into_iter().zip(tys) {
        env.bind(name.to_string(), ty);
    }
    infer_expr(
        &env,
        &HirExpr::CompareChain {
            first: Box::new(HirExpr::Name("a".to_string())),
            links: vec![
                CompareLink {
                    op: ops[0],
                    right: HirExpr::Name("b".to_string()),
                },
                CompareLink {
                    op: ops[1],
                    right: HirExpr::Name("c".to_string()),
                },
            ],
        },
    )
}

#[test]
fn a_chain_of_admissible_links_is_bool() {
    assert_eq!(
        chain_ty(
            [Ty::Int, Ty::Float, Ty::Bool],
            [CmpOpKind::Lt, CmpOpKind::GtE]
        ),
        Ok(Ty::Bool)
    );
    assert_eq!(
        chain_ty([Ty::Str, Ty::Str, Ty::Str], [CmpOpKind::Lt, CmpOpKind::Eq]),
        Ok(Ty::Bool)
    );
}

#[test]
fn each_link_is_checked_on_its_own_pair() {
    // Link 0 is bad.
    let err = chain_ty([Ty::Int, Ty::Str, Ty::Str], [CmpOpKind::Lt, CmpOpKind::Lt]).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(
        err.message.contains("cannot compare `int` and `str`"),
        "{}",
        err.message
    );
    // Link 1 is bad, although `a` and `c` alone would compare.
    let err = chain_ty(
        [Ty::Int, Ty::Str, Ty::Int],
        [CmpOpKind::NotEq, CmpOpKind::Lt],
    )
    .unwrap_err();
    assert_eq!(err.code, "T0021");
    // Only link 1 is bad.
    let err = chain_ty([Ty::Int, Ty::Int, Ty::Str], [CmpOpKind::Lt, CmpOpKind::Lt]).unwrap_err();
    assert!(
        err.message.contains("cannot compare `int` and `str`"),
        "{}",
        err.message
    );
}

#[test]
fn source_chains_type_check_in_every_position() {
    checks(
        "def f(a: int, b: float, s: str, o: int | None) -> bool:\n\
         \x20   ok = 0 <= a < b\n\
         \x20   if \"a\" < s <= \"z\" and o is None is None:\n        return ok\n\
         \x20   return 1 == 1.0 != 2\n",
    );
    checks(
        "from dataclasses import dataclass\n\n@dataclass\nclass P:\n    x: int\n\n\
         def f(p: P, q: P) -> bool:\n    return p == q != P(1)\n",
    );
    refused(
        "class C:\n    def __init__(self) -> None:\n        self.n = 0\n\n\
         def f(p: C, q: C) -> bool:\n    return 0 < 1 == p == q\n",
        "T0021",
        "cannot compare",
    );
}

#[test]
fn the_solver_types_a_chain_bool() {
    checks("def _g(a, b):\n    return 0 < a < b\n\ndef f() -> bool:\n    return _g(1, 2)\n");
    checks(
        "def _g(a: int, s: str):\n    return 0 < a < 3\n\ndef f() -> bool:\n    return _g(1, \"x\")\n",
    );
}

#[test]
fn a_walrus_in_operand_one_binds_in_the_solver() {
    checks(
        "def _g(a: int, b: int):\n    if 0 < (n := a) < b:\n        return n\n    return 0\n\ndef f() -> int:\n    return _g(1, 2)\n",
    );
}

#[test]
fn a_protocol_call_inside_a_chain_is_specialized() {
    let source = "from typing import Protocol\nclass P(Protocol):\n    def m(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.n = 0\n    def m(self) -> int:\n        return 0\n\ndef use(p: P) -> int:\n    return p.m()\n\nprint(0 <= use(C()) < 7)\nprint(use(C()) <= 0 < 7)\n";
    let resolved = check_source(source).expect("must type-check");
    let debug = format!("{resolved:?}");
    assert!(
        !debug.contains("callee: \"use\""),
        "a call inside a chain must be rewritten to its specialization: {debug}"
    );
}

#[test]
fn a_generic_call_inside_a_chain_is_specialized() {
    checks("def g[T](x: T) -> T:\n    return x\n\nprint(g(0) < g(1) < 2)\n");
}

#[test]
fn a_generic_class_instantiated_inside_a_chain_is_collected() {
    checks(
        "class Box[T]:\n    def __init__(self, x: T) -> None:\n        self.x = x\n\n\
         def get(b: Box[int]) -> int:\n    return b.x\n\n\
         print(0 < get(Box(1)) < 2)\n",
    );
}

/// A generic function's body is walked for calls to generic functions, and
/// the walk reaches every chain operand.
#[test]
fn a_generic_call_inside_a_chain_in_a_generic_function_is_refused() {
    refused(
        "def g[T](x: T) -> T:\n    return x\n\ndef f[T](x: T) -> T:\n    b = 0 < 1 < g(1)\n    return x\n",
        "T0042",
        "generic function `f` calls generic function `g`",
    );
}

/// The same walk admits a chain with no generic call.
#[test]
fn a_chain_without_a_generic_call_in_a_generic_function_is_admitted() {
    checks("def f[T](x: T) -> T:\n    b = 0 < 1 < 2\n    return x\n\nprint(f(1))\n");
}

/// An operand error inside a chain still surfaces through the generic-call
/// rewrite.
#[test]
fn an_undefined_call_inside_a_chain_is_refused() {
    let err = check_source("def g[T](x: T) -> T:\n    return x\n\nprint(g(0) < 1 < nope())\n")
        .expect_err("an undefined call must be refused");
    assert_eq!(err.code, "T0021", "{}", err.message);
}
