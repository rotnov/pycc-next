//! `None` into an `object` return slot (#1387, `crate::object_none`): both
//! walkers admit a bare `return` and `return None` in a function declared
//! `-> Any`/`-> object`. Since #1475 every neighbouring `None`-into-object
//! seam boxes too (`crate::object_box`); falling off the end of such a
//! function and an `Optional` value keep their diagnostics.
//!
//! A fully annotated module is decided by the check phase alone; a module
//! holding one unannotated private helper (`_id`) also runs the constraint
//! solver, whose per-function verdict wins (D-220), so each admitted shape
//! is asserted under both.

use super::*;

/// The module's diagnostics, lowered as an `--ext` module (D-258) or a
/// `native` one.
fn diagnostics(source: &str, ext: bool) -> Vec<pycc_diag::Diagnostic> {
    let parsed = pycc_parser::parse(source).expect("test fixture must parse");
    let mut resolved = pycc_hir::ResolvedImports::default();
    resolved.set_ext_module(ext);
    let hir = pycc_hir::lower_module(&parsed, &resolved, None)
        .map(|lowered| lowered.hir)
        .unwrap_or_else(|diagnostics| panic!("{source}\nmust lower: {diagnostics:#?}"));
    check_all(&hir).err().unwrap_or_default()
}

/// One unannotated private helper and a call that resolves it: present, the
/// module runs the constraint solver as well as the check phase.
const SOLVER: &str = "def _id(x):\n    return x\n\n\ndef use() -> int:\n    return _id(1)\n\n\n";

fn checks(source: &str) {
    for prefix in ["", SOLVER] {
        let source = format!("from typing import Any\n\n\n{prefix}{source}");
        let diagnostics = diagnostics(&source, true);
        assert!(diagnostics.is_empty(), "{source}\n{diagnostics:#?}");
    }
}

fn refused(source: &str, ext: bool, code: &str, message: &str) {
    for prefix in ["", SOLVER] {
        let source = format!("from typing import Any\n\n\n{prefix}{source}");
        let diagnostics = diagnostics(&source, ext);
        assert!(
            diagnostics
                .iter()
                .any(|d| d.code == code && d.message.contains(message)),
            "{source}\nexpected {code} {message:?}, got {diagnostics:#?}"
        );
    }
}

#[test]
fn a_bare_return_and_return_none_reach_an_object_return() {
    for ret in ["Any", "object"] {
        checks(&format!("def f(x: int) -> {ret}:\n    return None\n"));
        checks(&format!(
            "def f(x: int) -> {ret}:\n    if x:\n        return\n    return None\n"
        ));
        // The lark `ParserState.feed_token` shape: an infinite loop whose
        // only exits are returns, one of them bare, beside a returned object.
        checks(&format!(
            "def f(xs: list[Any], n: int) -> {ret}:\n    i = 0\n    while True:\n        if i >= n:\n            return\n        if i == 5:\n            return xs[i]\n        i = i + 1\n"
        ));
    }
}

#[test]
fn a_method_returning_an_object_admits_a_bare_return() {
    checks(
        "class P:\n    def __init__(self, n: int) -> None:\n        self.n = n\n\n    def feed(self, o: object) -> Any:\n        if self.n:\n            return\n        return o\n",
    );
}

#[test]
fn a_none_typed_expression_is_boxed_at_an_object_return() {
    // #1475: a call whose type is `None` is evaluated and answers CPython's
    // `None` (`crate::object_box`).
    checks("def g() -> None:\n    return\n\n\ndef f() -> Any:\n    return g()\n");
}

#[test]
fn falling_off_the_end_of_an_object_function_is_still_refused() {
    refused(
        "def f(x: int) -> Any:\n    if x:\n        return None\n",
        true,
        "T0022",
        "can exit without returning `object`",
    );
}

#[test]
fn every_other_none_into_object_seam_is_boxed() {
    // #1475: an annotated binding and a call argument box `None` too.
    checks("def f() -> None:\n    y: Any = None\n    print(y)\n");
    checks("def g(o: object) -> None:\n    return\n\n\ndef f() -> None:\n    g(None)\n");
}

#[test]
fn an_optional_into_an_object_seam_keeps_its_diagnostic() {
    // An `Optional` value has no single packer, at every seam.
    refused(
        "def f(n: int | None) -> Any:\n    return n\n",
        true,
        "T0022",
        "",
    );
    refused(
        "def f(n: int | None) -> None:\n    y: Any = n\n    print(y)\n",
        true,
        "T0025",
        "",
    );
    refused(
        "def g(o: object) -> None:\n    return\n\n\ndef f(n: int | None) -> None:\n    g(n)\n",
        true,
        "T0021",
        "",
    );
}

#[test]
fn a_native_return_type_still_refuses_a_bare_return() {
    // The admission is keyed on the declared `object`, nothing else: a
    // bare `return` in an `int` function is unchanged in either mode.
    for ext in [true, false] {
        refused(
            "def f(x: int) -> int:\n    if x:\n        return\n    return x\n",
            ext,
            "T0022",
            "",
        );
    }
}
