//! Unit tests for the return-position check in `lower_return_annotation`
//! (#934): a protocol class written as a function's or method's return type
//! annotation is `C0001` at HIR lowering, on the annotation's own span.
//!
//! Kept beside the code they pin, in a file of their own, per AGENTS.md's
//! decomposability rule: `func.rs` carries no inline test module and
//! `crate::tests` is already several thousand lines. The `assert_*` helpers
//! are local copies of `crate::tests`'s private ones (three lines each), so
//! nothing there needs its visibility widened.

use crate::{Ty, lower_checked};
use pycc_diag::Span;

/// The protocol and a conforming concrete class every case below shares.
const PRELUDE: &str = "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return self.x\n";

const MESSAGE_P: &str = "a protocol class (`P`) as a return type annotation is not supported yet -- a protocol type is currently supported in parameter and variable positions only";

/// The span of the bare name after the *last* `-> ` in `source`: the
/// return annotation of the definition under test.
fn last_return_annotation_span(source: &str, name: &str) -> Span {
    let arrow = format!("-> {name}:");
    let start = source.rfind(&arrow).expect("source carries the annotation") + 3;
    let start = u32::try_from(start).expect("test source fits a span");
    Span::new(start, start + name.len() as u32)
}

fn assert_capability_error(source: &str, expected_message: &str, expected_span: Span) {
    let module = crate::pycc_parser_test_helper::parse(source);
    let diagnostic = lower_checked(&module).unwrap_err();

    assert_eq!(diagnostic.code, "C0001", "source: {source:?}");
    assert_eq!(diagnostic.message, expected_message, "source: {source:?}");
    assert_eq!(diagnostic.span, Some(expected_span), "source: {source:?}");
}

fn lower_ok(source: &str) -> crate::HirModule {
    let module = crate::pycc_parser_test_helper::parse(source);
    lower_checked(&module).expect("test fixture should lower successfully")
}

fn lower_err_code(source: &str) -> &'static str {
    let module = crate::pycc_parser_test_helper::parse(source);
    lower_checked(&module).unwrap_err().code
}

// -- the gate fires --------------------------------------------------------

#[test]
fn a_protocol_return_annotation_on_a_module_level_function_is_c0001() {
    // The issue's own shape. Before #934 this lowered and type-checked, and
    // `pycc_mir` panicked on `p.foo()` because `P` has no method table.
    let source = format!("{PRELUDE}def make() -> P:\n    return C()\n");
    assert_capability_error(
        &source,
        MESSAGE_P,
        last_return_annotation_span(&source, "P"),
    );
}

#[test]
fn a_sub_protocol_return_annotation_is_c0001_and_names_the_sub_protocol() {
    // `annotation_to_ty` resolves `Q` through `class_defs` to
    // `Ty::Protocol("Q")` (a protocol inheriting a protocol is itself a
    // protocol), so the same gate fires and the message names `Q`, not `P`.
    let source = format!(
        "{PRELUDE}class Q(P):\n    def bar(self) -> int: ...\ndef make() -> Q:\n    return C()\n"
    );
    assert_capability_error(
        &source,
        "a protocol class (`Q`) as a return type annotation is not supported yet -- a protocol type is currently supported in parameter and variable positions only",
        last_return_annotation_span(&source, "Q"),
    );
}

#[test]
fn a_protocol_return_annotation_on_a_private_helper_is_c0001() {
    // `is_public == false` only changes what a *missing* annotation means
    // (D-038); a written `-> P` is rejected regardless.
    let source = format!("{PRELUDE}def _make() -> P:\n    return C()\n");
    assert_capability_error(
        &source,
        MESSAGE_P,
        last_return_annotation_span(&source, "P"),
    );
}

#[test]
fn a_protocol_return_annotation_on_a_concrete_method_is_c0001() {
    // Reached through `class::lower_method`'s call into
    // `lower_return_annotation`, with `class_name == Some("D")`.
    let source = format!(
        "{PRELUDE}class D:\n    def __init__(self) -> None:\n        self.y = 0\n    def clone(self) -> P:\n        return C()\n"
    );
    assert_capability_error(
        &source,
        MESSAGE_P,
        last_return_annotation_span(&source, "P"),
    );
}

// -- ordering against the element-type gates -------------------------------

#[test]
fn a_container_of_protocol_return_annotation_still_reports_the_element_gate_first() {
    // The protocol check runs *after* `annotation_to_ty`, so a nested
    // protocol reports the element-type gate the container would have
    // reported anyway (D-228's ordering for containers): `list[P]` is
    // D-105's `T0034`, and `P | None` is `T0049`. Neither reaches the new
    // check, which is why it does not recurse.
    assert_eq!(
        lower_err_code(&format!(
            "{PRELUDE}def make() -> list[P]:\n    return [C()]\n"
        )),
        "T0034"
    );
    assert_eq!(
        lower_err_code(&format!(
            "{PRELUDE}def make() -> P | None:\n    return None\n"
        )),
        "T0049"
    );
}

// -- the gate does not fire ------------------------------------------------

#[test]
fn a_protocol_in_parameter_and_variable_positions_still_lowers() {
    // The positions the message names as supported: a protocol-typed
    // parameter (monomorphized per D-166 item 7), a protocol-typed local,
    // and a protocol-typed module-level variable (both bind the value's
    // concrete type in `pycc_mir`, D-166 item 6).
    let hir = lower_ok(&format!(
        "{PRELUDE}def use(p: P) -> int:\n    return p.foo()\ndef local() -> int:\n    x: P = C()\n    return x.foo()\np: P = C()\nprint(use(p) + local())\n"
    ));
    let use_return_ty = hir.items.iter().find_map(|item| match item {
        crate::HirItem::Function {
            name, return_ty, ..
        } if name == "use" => Some(return_ty.clone()),
        _ => None,
    });
    assert_eq!(use_return_ty, Some(Ty::Int));
}

#[test]
fn a_pep_695_type_parameter_shadowing_a_protocol_name_still_lowers() {
    // `annotation_to_ty` checks the function's own type parameter before
    // `class_defs`, so `-> P` here is `Ty::Param("P")`, not
    // `Ty::Protocol("P")`, and the gate correctly stays quiet -- the
    // protocol class is shadowed inside `ident`.
    let hir = lower_ok(&format!(
        "{PRELUDE}def ident[P](x: P) -> P:\n    return x\nprint(ident(1))\n"
    ));
    let ident_return_ty = hir.items.iter().find_map(|item| match item {
        crate::HirItem::Function {
            name, return_ty, ..
        } if name == "ident" => Some(return_ty.clone()),
        _ => None,
    });
    assert_eq!(ident_return_ty, Some(Ty::Param(Box::new("P".to_string()))));
}
