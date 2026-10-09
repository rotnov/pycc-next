//! Unit tests for string-literal type annotations (Part 1 of #889): a
//! quoted annotation lowers exactly as its unquoted spelling does, in every
//! annotation position, and an unparsable one is `C0001` on the literal.
//!
//! Most cases compare the quoted module's HIR with the unquoted module's
//! HIR. The top-level strings are unquoted by `pycc_parser::parse_all`
//! before lowering, and the nested ones (`list["C"]`) by
//! `string_annotation_to_ty`, so between them the cases cover both paths.

use crate::{HirModule, Ty, lower_checked};
use pycc_diag::{Diagnostic, Span};

fn lower_ok(source: &str) -> HirModule {
    let module = crate::pycc_parser_test_helper::parse(source);
    lower_checked(&module).expect("test fixture should lower")
}

fn lower_err(source: &str) -> Diagnostic {
    let module = crate::pycc_parser_test_helper::parse(source);
    lower_checked(&module).expect_err("test fixture should be rejected")
}

/// `quoted` and `unquoted` lower to the same items and classes.
fn assert_same_hir(quoted: &str, unquoted: &str) {
    let quoted_hir = lower_ok(quoted);
    let unquoted_hir = lower_ok(unquoted);
    assert_eq!(quoted_hir.items, unquoted_hir.items, "source: {quoted:?}");
    assert_eq!(
        quoted_hir.class_defs, unquoted_hir.class_defs,
        "source: {quoted:?}"
    );
}

/// The span of `needle`'s first occurrence in `source`.
fn span_of(source: &str, needle: &str) -> Span {
    let start = u32::try_from(source.find(needle).expect("the fixture holds the text"))
        .expect("the fixture is short");
    Span::new(
        start,
        start + u32::try_from(needle.len()).expect("the needle is short"),
    )
}

#[test]
fn a_quoted_parameter_and_return_annotation_lower_like_the_unquoted_ones() {
    assert_same_hir(
        "def f(a: \"int\", b: 'str') -> \"float\":\n    return 1.5\n",
        "def f(a: int, b: str) -> float:\n    return 1.5\n",
    );
}

#[test]
fn a_quoted_enclosing_class_names_the_class() {
    // #889's own example: a method returning its own class.
    assert_same_hir(
        "class Node:\n    def __init__(self) -> None:\n        self.n = 1\n    def me(self) -> \"Node\":\n        return self\n    def other(self, o: 'Node') -> int:\n        return o.n\n",
        "class Node:\n    def __init__(self) -> None:\n        self.n = 1\n    def me(self) -> Node:\n        return self\n    def other(self, o: Node) -> int:\n        return o.n\n",
    );
}

#[test]
fn a_quoted_subscripted_generic_of_the_enclosing_class_resolves() {
    // The lark subject's `-> 'ParserState[StateT]'` shape.
    assert_same_hir(
        "class Box[T]:\n    def __init__(self, v: T) -> None:\n        self.v = v\n    def same(self) -> 'Box[T]':\n        return self\n",
        "class Box[T]:\n    def __init__(self, v: T) -> None:\n        self.v = v\n    def same(self) -> Box[T]:\n        return self\n",
    );
}

#[test]
fn quoted_variable_annotations_lower_like_the_unquoted_ones() {
    // Module-level and local `AnnAssign`, and a class-body attribute.
    assert_same_hir(
        "x: \"int\" = 1\nclass C:\n    K: \"int\" = 3\n    def __init__(self) -> None:\n        self.n = 0\ndef f() -> int:\n    y: 'list[int]' = [1]\n    return y[0]\n",
        "x: int = 1\nclass C:\n    K: int = 3\n    def __init__(self) -> None:\n        self.n = 0\ndef f() -> int:\n    y: list[int] = [1]\n    return y[0]\n",
    );
}

#[test]
fn a_nested_quoted_class_resolves_through_the_resolver_arm() {
    // A string inside an annotation keeps its quotes through `parse_all`,
    // so these reach `string_annotation_to_ty`: one names a builtin inside
    // a container, the other a user class inside `Final`.
    assert_same_hir(
        "class C:\n    def __init__(self) -> None:\n        self.n = 1\nc: Final[\"C\"] = C()\ndef f(xs: list[\"int\"]) -> int:\n    return xs[0]\n",
        "class C:\n    def __init__(self) -> None:\n        self.n = 1\nc: Final[C] = C()\ndef f(xs: list[int]) -> int:\n    return xs[0]\n",
    );
}

#[test]
fn a_quoted_final_is_still_final() {
    // `is_final` is what the type checker's reassignment check (`T0045`)
    // reads; the end-to-end refusal is pinned in
    // `tests/issue_889_string_annotation.rs`.
    assert_same_hir("x: \"Final[int]\" = 1\n", "x: Final[int] = 1\n");
    let hir = lower_ok("x: \"Final[int]\" = 1\n");
    assert!(matches!(
        hir.items.as_slice(),
        [crate::HirItem::TopLevelStmt(crate::HirStmt::AnnAssign {
            is_final: true,
            ..
        })]
    ));
}

#[test]
fn a_quoted_class_var_is_a_class_variable() {
    assert_same_hir(
        "from typing import ClassVar\nclass C:\n    X: \"ClassVar[int]\" = 7\n    def __init__(self) -> None:\n        self.n = 0\n",
        "from typing import ClassVar\nclass C:\n    X: ClassVar[int] = 7\n    def __init__(self) -> None:\n        self.n = 0\n",
    );
}

#[test]
fn a_quoted_later_defined_class_stays_rejected() {
    // Annotation resolution sees only the classes defined so far, quoted
    // or not. Resolving a later class is deferred on #889; until then the
    // quoted spelling must fail closed exactly like the unquoted one.
    let quoted = "def f(a: \"Later\") -> int:\n    return 1\nclass Later:\n    def __init__(self) -> None:\n        self.n = 1\n";
    let diagnostic = lower_err(quoted);
    assert_eq!(diagnostic.code, "C0001");
    assert_eq!(diagnostic.span, Some(span_of(quoted, "Later")));
    assert_eq!(
        diagnostic.message,
        lower_err(&quoted.replace("\"Later\"", "Later")).message
    );
}

#[test]
fn an_unparsable_top_level_string_annotation_is_c0001_on_the_literal() {
    let source = "def f(x: \"not a type\") -> int:\n    return 1\n";
    let diagnostic = lower_err(source);
    assert_eq!(diagnostic.code, "C0001");
    assert_eq!(diagnostic.span, Some(span_of(source, "\"not a type\"")));
    assert!(
        diagnostic
            .message
            .starts_with("string type annotation `not a type` is not a valid Python expression: "),
        "{}",
        diagnostic.message
    );
    assert!(!diagnostic.message.contains("byte range"));
}

#[test]
fn an_unparsable_nested_string_annotation_is_c0001_on_the_literal() {
    let source = "def f(x: list[\"a b\"]) -> int:\n    return 1\n";
    let diagnostic = lower_err(source);
    assert_eq!(diagnostic.code, "C0001");
    assert_eq!(diagnostic.span, Some(span_of(source, "\"a b\"")));
    assert!(diagnostic.message.contains("string type annotation `a b`"));
}

#[test]
fn a_quoted_unsupported_shape_reports_its_own_diagnostic_on_the_literal() {
    // A nested string that parses but names an unsupported shape gets that
    // shape's own message, placed on the literal it came from.
    let source = "def f(x: list[\"a.b\"]) -> int:\n    return 1\n";
    let diagnostic = lower_err(source);
    assert_eq!(diagnostic.code, "C0001");
    assert_eq!(diagnostic.span, Some(span_of(source, "\"a.b\"")));
    assert!(
        diagnostic.message.contains("got an attribute expression"),
        "{}",
        diagnostic.message
    );
}

#[test]
fn a_quoted_optional_lowers_to_the_optional_type() {
    let hir = lower_ok("def f(a: \"int | None\") -> int:\n    return 1\n");
    let params = hir.items.iter().find_map(|item| match item {
        crate::HirItem::Function { params, .. } => Some(params.clone()),
        crate::HirItem::TopLevelStmt(_) => None,
    });
    assert_eq!(
        params,
        Some(vec![("a".to_string(), Ty::Optional(Box::new(Ty::Int)))])
    );
}

#[test]
fn a_quoted_type_alias_value_keeps_its_refusal() {
    // A type alias's value is an expression, not an annotation, so a quoted
    // one is refused exactly as before Part 1 of #889, in both spellings.
    // A quoted bare container (`"list"`) gets no bare-container advice: the
    // advice would see through the quotes.
    for (source, literal) in [
        ("type X = \"int\"\n", "\"int\""),
        ("X: TypeAlias = \"int\"\n", "\"int\""),
        ("type X = \"list\"\n", "\"list\""),
        ("X: TypeAlias = \"dict\"\n", "\"dict\""),
    ] {
        let diagnostic = lower_err(source);
        assert_eq!(diagnostic.code, "C0001", "{source:?}");
        assert_eq!(
            diagnostic.span,
            Some(span_of(source, literal)),
            "{source:?}"
        );
        assert!(
            diagnostic.message.contains("got a string literal"),
            "{source:?}: {}",
            diagnostic.message
        );
    }
}

#[test]
fn a_string_nested_in_a_type_alias_value_resolves() {
    for (quoted, unquoted) in [
        ("type X = list[\"int\"]\n", "type X = list[int]\n"),
        (
            "X: TypeAlias = list[\"int\"]\n",
            "X: TypeAlias = list[int]\n",
        ),
    ] {
        let aliases = lower_ok(quoted).type_aliases;
        assert_eq!(aliases, lower_ok(unquoted).type_aliases, "{quoted:?}");
        assert_eq!(aliases.len(), 1, "{quoted:?}");
    }
}

#[test]
fn a_quoted_pep_591_nesting_fails_like_the_unquoted_one() {
    // `lower_class_attr`'s two nesting checks match by shape; a string
    // nested in the wrapper must fail exactly as the unquoted spelling does.
    let prelude = "from typing import ClassVar, Final\nclass C:\n    ";
    let tail = "\n    def __init__(self) -> None:\n        self.n = 0\n";
    for (quoted, unquoted) in [
        (
            "X: ClassVar[\"Final[int]\"] = 1",
            "X: ClassVar[Final[int]] = 1",
        ),
        (
            "X: Final[\"ClassVar[int]\"] = 1",
            "X: Final[ClassVar[int]] = 1",
        ),
    ] {
        let quoted_error = lower_err(&format!("{prelude}{quoted}{tail}"));
        let unquoted_error = lower_err(&format!("{prelude}{unquoted}{tail}"));
        assert_eq!(quoted_error.code, "C0001", "{quoted:?}");
        assert!(
            quoted_error.message.contains("PEP 591 forbids nesting"),
            "{quoted:?}: {}",
            quoted_error.message
        );
        assert_eq!(quoted_error.message, unquoted_error.message, "{quoted:?}");
    }
}
