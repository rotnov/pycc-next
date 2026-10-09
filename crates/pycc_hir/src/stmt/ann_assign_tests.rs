//! Unit tests for `ann_assign`'s attribute-target arm (#1264, Part 3 of
//! #1218): the one admitted shape and every refusal, each on the target's
//! own span. Kept in a file of their own beside the code they pin, per
//! AGENTS.md's decomposability rule (`crate::tests` is already several
//! thousand lines).

use crate::{HirExpr, HirItem, HirStmt, Ty, lower_checked};
use pycc_diag::{Diagnostic, Span};

const CLASS: &str = "class C:\n    def __init__(self) -> None:\n        self.n = 0\n";

fn lower_err(source: &str) -> Diagnostic {
    let module = crate::pycc_parser_test_helper::parse(source);
    lower_checked(&module).unwrap_err()
}

/// The span of the first occurrence of `needle` in `source`.
fn span_of(source: &str, needle: &str) -> Span {
    let start = source.find(needle).expect("source carries the needle");
    let start = u32::try_from(start).expect("test source fits a span");
    Span::new(start, start + needle.len() as u32)
}

/// Asserts a `C0001` whose message starts with `prefix`, ends with the
/// shared admitted-form sentence, and sits on `target`'s span.
fn assert_refused(source: &str, prefix: &str, target: &str) {
    let diagnostic = lower_err(source);
    assert_eq!(diagnostic.code, "C0001", "source: {source:?}");
    assert_eq!(
        diagnostic.message,
        format!("{prefix} -- {}", super::ADMITTED_FORM),
        "source: {source:?}"
    );
    assert_eq!(
        diagnostic.span,
        Some(span_of(source, target)),
        "source: {source:?}"
    );
}

/// The body of the module-level function named `name`.
fn function_body(source: &str, name: &str) -> Vec<HirStmt> {
    let module = crate::pycc_parser_test_helper::parse(source);
    let hir = lower_checked(&module).expect("test fixture should lower successfully");
    hir.items
        .into_iter()
        .find_map(|item| match item {
            HirItem::Function { name: n, body, .. } if n == name => Some(body),
            _ => None,
        })
        .expect("the function is lowered")
}

#[test]
fn an_annotated_empty_list_attribute_target_lowers_to_a_typed_attr_set() {
    let body = function_body(
        &format!("{CLASS}def f(c: C) -> None:\n    c.xs: list[int] = []\n"),
        "f",
    );
    assert!(
        matches!(
            &body[..],
            [HirStmt::AttrSet { attr, value: HirExpr::EmptyList(Ty::Int), .. }] if attr == "xs"
        ),
        "{body:?}"
    );
}

#[test]
fn an_annotated_empty_dict_attribute_target_lowers_to_a_typed_attr_set() {
    let body = function_body(
        &format!("{CLASS}def f(c: C) -> None:\n    c.d: dict[str, int] = {{}}\n"),
        "f",
    );
    assert!(
        matches!(
            &body[..],
            [HirStmt::AttrSet { attr, value: HirExpr::EmptyDict(pair), .. }]
                if attr == "d" && **pair == (Ty::Str, Ty::Int)
        ),
        "{body:?}"
    );
}

#[test]
fn a_module_level_annotated_attribute_target_is_refused() {
    let source = format!("{CLASS}c = C()\nc.xs: list[int] = []\n");
    assert_refused(
        &source,
        "an annotated attribute target at module level is not supported yet",
        "c.xs",
    );
}

#[test]
fn a_super_base_is_refused_before_any_annotation_or_value_shape() {
    // Even a shape the attribute arm would refuse anyway (a scalar
    // annotation) gets #448's dedicated message, on `super()`'s span.
    for line in ["super().x: int = 1", "super().xs: list[int] = []"] {
        let source = format!("{CLASS}    def m(self) -> None:\n        {line}\n");
        let diagnostic = lower_err(&source);
        assert_eq!(diagnostic.code, "C0001");
        assert_eq!(
            diagnostic.message,
            "super().attr = value is not supported yet — super() attribute assignment is not \
             implemented in this version"
        );
        assert_eq!(diagnostic.span, Some(span_of(&source, "super()")));
    }
}

#[test]
fn a_final_annotated_attribute_target_is_refused() {
    let source =
        format!("{CLASS}    def m(self) -> None:\n        self.xs: Final[list[int]] = []\n");
    assert_refused(
        &source,
        "a `Final` annotation on an attribute target is not supported yet",
        "self.xs",
    );
}

#[test]
fn an_annotated_attribute_target_without_a_value_is_refused() {
    let source = format!("{CLASS}    def m(self) -> None:\n        self.xs: list[int]\n");
    assert_refused(
        &source,
        "an annotated attribute target without a value is not supported yet",
        "self.xs",
    );
}

#[test]
fn every_other_annotation_and_value_shape_is_refused_naming_the_annotation() {
    for (annotation, value) in [
        ("int", "1"),
        ("list[int]", "[1]"),
        ("list[int]", "{}"),
        ("dict[str, int]", "[]"),
        ("dict[str, int]", "{\"k\": 1}"),
        ("set[int]", "set()"),
    ] {
        let source =
            format!("{CLASS}    def m(self) -> None:\n        self.v: {annotation} = {value}\n");
        assert_refused(
            &source,
            &format!(
                "an annotated attribute target annotated `{annotation}` with this value is not \
                 supported yet"
            ),
            "self.v",
        );
    }
}

#[test]
fn a_bare_list_or_dict_attribute_annotation_gets_the_parameterized_advice() {
    // Also through a transparent `Annotated[...]` wrapper.
    for (bare, example) in [
        ("list", "list[int]"),
        ("dict", "dict[str, int]"),
        ("Annotated[list, 0]", "list[int]"),
    ] {
        let source = format!("{CLASS}    def m(self) -> None:\n        self.v: {bare} = []\n");
        let diagnostic = lower_err(&source);
        assert_eq!(diagnostic.code, "C0001", "{diagnostic:?}");
        assert!(diagnostic.message.contains(example), "{diagnostic:?}");
    }
}

#[test]
fn a_bare_set_or_tuple_attribute_annotation_gets_no_parameterized_advice() {
    // The advice would name `set[int]`/`tuple[int, int]`, which this
    // position refuses too, so the attribute target keeps the generic
    // message.
    for (bare, example) in [("set", "set[int]"), ("tuple", "tuple[int, int]")] {
        let source = format!("{CLASS}    def m(self) -> None:\n        self.v: {bare} = []\n");
        let diagnostic = lower_err(&source);
        assert_eq!(diagnostic.code, "C0001", "{diagnostic:?}");
        assert!(!diagnostic.message.contains(example), "{diagnostic:?}");
    }
}

#[test]
fn a_mistyped_element_annotation_on_an_attribute_target_is_the_local_error() {
    // `annotation_to_ty` validates it exactly as it would a local's.
    let source = format!("{CLASS}    def m(self) -> None:\n        self.xs: list[str] = []\n");
    assert_eq!(lower_err(&source).code, "T0034");
}

#[test]
fn a_non_attribute_non_name_annotated_target_keeps_the_bare_name_message() {
    let diagnostic = lower_err("def f(xs: list[int]) -> None:\n    xs[0]: int = 1\n");
    assert_eq!(diagnostic.code, "C0001");
    assert!(
        diagnostic
            .message
            .starts_with("only assigning to a bare name is supported so far, got"),
        "{diagnostic:?}"
    );
}
