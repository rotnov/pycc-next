//! Unit tests for `receiver_class_call.rs` (#1411): which `type(...)(...)`
//! shapes lower to `HirExpr::ReceiverClassCall`, and the refusals that name
//! the construct for every other shape.

use crate::{HirExpr, HirItem, HirStmt, lower_checked};

/// The body of the function item `name` that `source` lowers to.
fn body_of(source: &str, name: &str) -> Vec<HirStmt> {
    let module = pycc_parser::parse(source).expect("test fixture must parse");
    let hir = lower_checked(&module).expect("test fixture must lower");
    hir.items
        .into_iter()
        .find_map(|item| match item {
            HirItem::Function { name: n, body, .. } if n == name => Some(body),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no item `{name}`"))
}

/// The rendered message and the span start of `source`'s lowering error.
fn refusal(source: &str) -> (String, usize) {
    let module = pycc_parser::parse(source).expect("test fixture must parse");
    let error = lower_checked(&module).expect_err("fixture must be rejected");
    let start = error.span.expect("a located refusal").start as usize;
    (error.message, start)
}

const CLASS_HEAD: &str = "class C:\n    def __init__(self, n: int) -> None:\n        self.n = n\n";

#[test]
fn type_self_call_in_a_method_lowers_to_a_receiver_class_call() {
    let source =
        format!("{CLASS_HEAD}    def m(self) -> C:\n        return type(self)(self.n + 1)\n");
    let body = body_of(&source, "C.m");
    let [HirStmt::Return(Some(HirExpr::ReceiverClassCall { args }))] = body.as_slice() else {
        panic!("expected `return <receiver class call>`, got {body:?}");
    };
    assert!(
        matches!(args.as_slice(), [HirExpr::BinOp { .. }]),
        "{args:?}"
    );
}

#[test]
fn a_walrus_or_a_comprehension_in_the_arguments_is_lowered_through() {
    // A named expression in the arguments is seen by the walrus walkers,
    // and a comprehension whose element constructs is renamed through.
    let source = format!(
        "{CLASS_HEAD}    def m(self) -> list[int]:\n\
         \x20       if type(self)((k := 2)).n > 1:\n\
         \x20           return [type(self)(i).n for i in range(k)]\n\
         \x20       return []\n"
    );
    let body = body_of(&source, "C.m");
    assert!(
        format!("{body:?}").matches("ReceiverClassCall").count() == 2,
        "{body:?}"
    );
}

#[test]
fn type_of_anything_but_the_receiver_is_refused_at_the_inner_call() {
    for (source, inner) in [
        (
            format!("{CLASS_HEAD}    def m(self, o: C) -> C:\n        return type(o)(1)\n"),
            "type(o)",
        ),
        (
            format!("{CLASS_HEAD}    def m(self) -> C:\n        return type(self, 1)(1)\n"),
            "type(self, 1)",
        ),
        (
            format!("{CLASS_HEAD}    def m(self) -> C:\n        return type()(1)\n"),
            "type()",
        ),
        (
            "class D:\n    def __init__(this, n: int) -> None:\n        this.n = n\n\
             \x20   def m(this) -> D:\n        return type(this)(1)\n"
                .to_string(),
            "type(this)",
        ),
    ] {
        let (message, start) = refusal(&source);
        assert!(
            message.contains("calling `type(...)` is supported only as `type(self)(...)`"),
            "{source}: {message}"
        );
        assert_eq!(&source[start..start + inner.len()], inner, "{source}");
    }
}

#[test]
fn type_self_outside_a_method_is_refused() {
    for source in [
        "type(self)(1)\n".to_string(),
        "def f(self: int) -> int:\n    return type(self)(1)\n".to_string(),
    ] {
        let (message, start) = refusal(&source);
        assert!(
            message.contains("`type(self)(...)` is supported only inside a method of a class"),
            "{source}: {message}"
        );
        assert!(source[start..].starts_with("type(self)"), "{source}");
    }
}

#[test]
fn an_argument_lowering_error_propagates() {
    let source =
        format!("{CLASS_HEAD}    def m(self) -> C:\n        return type(self)(lambda: 1)\n");
    let (message, _) = refusal(&source);
    assert!(!message.contains("type("), "{message}");
}
