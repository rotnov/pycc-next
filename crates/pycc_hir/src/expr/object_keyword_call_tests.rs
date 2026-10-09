//! Unit tests for Part 8 of #1371's keyword-call deferral
//! (`expr::object_keyword_call`).

use crate::{HirExpr, HirItem, HirStmt, KEYWORD_CALL_UNSUPPORTED};
use pycc_diag::Diagnostic;

fn lower(source: &str) -> Result<crate::HirModule, Diagnostic> {
    crate::lower_checked(&crate::pycc_parser_test_helper::parse(source))
}

/// The last top-level expression statement of `source`, which must lower.
fn last_expr(source: &str) -> HirExpr {
    let hir = lower(source).unwrap_or_else(|d| panic!("{source:?} must lower: {d:?}"));
    match hir.items.last() {
        Some(HirItem::TopLevelStmt(HirStmt::ExprStmt(expr))) => expr.clone(),
        other => panic!("{source:?}: expected a trailing expression, got {other:?}"),
    }
}

/// `source` is refused with the keyword `C0001` spanning `call_text`.
fn assert_refused_at(source: &str, call_text: &str) {
    let diagnostic = lower(source).expect_err(source);
    assert_eq!(diagnostic.code, "C0001", "{source:?}: {diagnostic:?}");
    assert_eq!(diagnostic.message, KEYWORD_CALL_UNSUPPORTED, "{source:?}");
    let start = source.rfind(call_text).expect("call text occurs") as u32;
    let span = diagnostic.span.expect("located");
    assert_eq!(
        (span.start, span.end),
        (start, start + call_text.len() as u32),
        "{source:?}"
    );
}

/// The three shapes whose callee can be a CPython object are deferred:
/// the positional half is lowered exactly as the keyword-free call, the
/// keywords keep their source order, and the node carries the call's span.
#[test]
fn the_object_call_shapes_are_deferred_with_their_keywords_in_order() {
    let source = "f(1, b=2, a=\"x\")\n";
    let HirExpr::KeywordCall {
        call,
        keywords,
        span,
    } = last_expr(source)
    else {
        panic!("{source:?} must defer");
    };
    assert_eq!(
        *call,
        HirExpr::Call {
            callee: "f".to_string(),
            args: vec![HirExpr::IntLiteral(1)],
        }
    );
    assert_eq!(
        keywords,
        vec![
            ("b".to_string(), HirExpr::IntLiteral(2)),
            ("a".to_string(), HirExpr::StringLiteral("x".to_string())),
        ]
    );
    assert_eq!((span.start, span.end), (0, 16));

    for (source, is_shape) in [
        (
            "o = 1\no.m(1, k=2)\n",
            (|e: &HirExpr| matches!(e, HirExpr::MethodCall { .. })) as fn(&HirExpr) -> bool,
        ),
        ("t = {}\nt[\"a\"](k=2)\n", |e| {
            matches!(e, HirExpr::ExprCall { .. })
        }),
    ] {
        let HirExpr::KeywordCall { call, .. } = last_expr(source) else {
            panic!("{source:?} must defer");
        };
        assert!(is_shape(&call), "{source:?}: {call:?}");
    }
}

/// `**` unpacking (on any callee, a `def` the binder holds included), a
/// positional half that does not lower, a stdlib intrinsic,
/// `super().m(k=1)` and a generic class instantiation keep the keyword
/// `C0001` at the call. A held `def` the binder refuses for its evaluation
/// order is `keyword_bind::eval_order`'s test.
#[test]
fn every_other_shape_keeps_the_keyword_refusal_at_the_call() {
    assert_refused_at("d = {}\nf(1, **d)\n", "f(1, **d)");
    assert_refused_at(
        "def f(a: int, b: int) -> None:\n    print(a)\n\n\nf(b=1, a=2)\nf(a=1, **{})\n",
        "f(a=1, **{})",
    );
    assert_refused_at("super(x=1)\n", "super(x=1)");
    assert_refused_at("import math\nmath.sqrt(x=1.0)\n", "math.sqrt(x=1.0)");
    assert_refused_at(
        "class B:\n    def m(self, a: int) -> None:\n        print(a)\n\n\n\
         class C(B):\n    def m(self, a: int) -> None:\n        super().m(a=a)\n",
        "super().m(a=a)",
    );
    assert_refused_at(
        "class C[T]:\n    def __init__(self, a: T) -> None:\n        self.a = a\n\n\nC[int](a=1)\n",
        "C[int](a=1)",
    );
}

/// A keyword value that fails to lower reports its own diagnostic, not the
/// keyword refusal.
#[test]
fn a_keyword_value_that_fails_to_lower_reports_its_own_error() {
    let diagnostic = lower("f(k=super())\n").expect_err("a bare super() value");
    assert_ne!(
        diagnostic.message, KEYWORD_CALL_UNSUPPORTED,
        "{diagnostic:?}"
    );
    assert_eq!(diagnostic.code, "C0001", "{diagnostic:?}");
}

/// A keyword call inside a comprehension has the loop variable renamed in
/// its positional half and in its keyword values alike.
#[test]
fn a_keyword_call_in_a_comprehension_renames_the_loop_variable() {
    let hir = lower("ys = [f(x, k=x) for x in range(3)]\n").expect("lowers");
    let debug = format!("{hir:?}");
    assert!(debug.contains("KeywordCall"), "{debug}");
    assert!(!debug.contains("Name(\"x\")"), "{debug}");
}
