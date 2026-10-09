//! Unit tests for `func::params` — the parameter-shape rules a `def` and a
//! method share, and the default-value rules Part 2 of #884 (#1189) adds.
//!
//! Split into its own file so `func.rs` stays under the ~1,000-line
//! decomposition threshold.

use pycc_diag::{Diagnostic, Span};

/// The single diagnostic `source` must be rejected with.
fn lower_err(source: &str) -> Diagnostic {
    let module = crate::pycc_parser_test_helper::parse(source);
    crate::lower_checked(&module).expect_err("test fixture should be rejected")
}

/// Every diagnostic `source` is rejected with, in report order.
fn lower_all_err(source: &str) -> Vec<Diagnostic> {
    let module = crate::pycc_parser_test_helper::parse(source);
    crate::lower_all(&module).expect_err("test fixture should be rejected")
}

/// The lowered parameter list of the module's single function.
fn params_of(source: &str) -> Vec<(String, crate::Ty)> {
    let module = crate::pycc_parser_test_helper::parse(source);
    let hir = crate::lower_checked(&module).expect("test fixture should lower");
    hir.items
        .iter()
        .find_map(|item| match item {
            crate::HirItem::Function { params, .. } => Some(params.clone()),
            crate::HirItem::TopLevelStmt(_) => None,
        })
        .expect("the fixture must hold a function")
}

/// The span of `needle`'s first occurrence in `source`.
fn span_of(source: &str, needle: &str) -> Span {
    let start = u32::try_from(source.find(needle).expect("the fixture must hold the text"))
        .expect("the fixture is short");
    Span::new(
        start,
        start + u32::try_from(needle.len()).expect("the needle is short"),
    )
}

// --- parameter shapes -------------------------------------------------

#[test]
fn each_unsupported_parameter_kind_keeps_its_own_message() {
    for (source, message) in [
        (
            "def f(*args) -> None:\n    return\n",
            "`*args` is not supported yet",
        ),
        (
            "def f(*, a: int) -> None:\n    print(a)\n",
            "keyword-only parameters are not supported yet",
        ),
        (
            "def f(**kwargs) -> None:\n    return\n",
            "`**kwargs` is not supported yet",
        ),
    ] {
        let diagnostic = lower_err(source);
        assert_eq!(diagnostic.code, "C0001", "source: {source}");
        assert_eq!(diagnostic.message, message, "source: {source}");
    }
}

#[test]
fn a_method_reports_the_same_shape_messages_as_a_module_level_def() {
    // The two textually duplicated copies of these checks are now one
    // function; this is the guard that the method caller still reaches it.
    for (parameters, message) in [
        ("self, *args", "`*args` is not supported yet"),
        (
            "self, *, a: int",
            "keyword-only parameters are not supported yet",
        ),
        ("self, **kwargs", "`**kwargs` is not supported yet"),
    ] {
        let source = format!("class C:\n    def m({parameters}) -> None:\n        return\n");
        let diagnostic = lower_err(&source);
        assert_eq!(diagnostic.code, "C0001", "parameters: {parameters}");
        assert_eq!(diagnostic.message, message, "parameters: {parameters}");
    }
}

#[test]
fn an_unsupported_parameter_kind_is_reported_before_a_default_on_it() {
    // Shape rejection runs first, so a keyword-only parameter that also
    // carries a default keeps the keyword-only message.
    let diagnostic = lower_err("def f(*, a: int = 1) -> None:\n    print(a)\n");
    assert_eq!(diagnostic.code, "C0001");
    assert_eq!(
        diagnostic.message,
        "keyword-only parameters are not supported yet"
    );
}

// --- defaults a module-level `def` admits -----------------------------

#[test]
fn an_admitted_default_lowers_the_parameter_with_its_annotated_type() {
    assert_eq!(
        params_of("def f(a: int, b: str = \"s\") -> None:\n    return\n"),
        vec![
            ("a".to_string(), crate::Ty::Int),
            ("b".to_string(), crate::Ty::Str)
        ]
    );
}

#[test]
fn a_default_never_infers_a_type_for_an_unannotated_parameter() {
    // An unannotated parameter of a private `def` keeps `Ty::Infer`
    // whatever its default is, so a default is not a back door into
    // inference this part does not implement.
    assert_eq!(
        params_of("def _f(a = \"s\"):\n    return\n"),
        vec![("a".to_string(), crate::Ty::Infer)]
    );
    assert_eq!(
        params_of("def _f(a = 1):\n    return\n"),
        vec![("a".to_string(), crate::Ty::Infer)]
    );
}

#[test]
fn each_assignable_literal_default_is_accepted() {
    for (annotation, literal) in [
        ("int", "1"),
        ("float", "1.5"),
        ("bool", "True"),
        ("str", "\"s\""),
        // `bool` is an `int` subtype at a checked boundary (D-086).
        ("int", "True"),
        // A bare value and a bare `None` both widen into `T | None`.
        ("int | None", "None"),
        ("int | None", "1"),
    ] {
        let source = format!("def f(a: {annotation} = {literal}) -> None:\n    return\n");
        let module = crate::pycc_parser_test_helper::parse(&source);
        assert!(
            crate::lower_checked(&module).is_ok(),
            "annotation `{annotation}`, default `{literal}`"
        );
    }
}

// --- defaults the recognizer refuses ----------------------------------

const UNADMITTED_MESSAGE: &str = "only a literal `int`, `float`, `bool`, `str`, or `None` \
     default parameter value is supported yet (a unary `-`/`+` may be applied to a numeric \
     literal)";

#[test]
fn every_default_outside_the_admitted_subset_is_a_capability_error() {
    for default in [
        // A name.
        "OTHER",
        // A call.
        "len(\"x\")",
        // Containers.
        "[]",
        "(1, 2)",
        "{}",
        // An f-string.
        "f\"{1}\"",
        // A complex literal: `lower_expr` rejects it as an argument too.
        "1j",
        // An `int` outside `i64`.
        "99999999999999999999",
        // A unary operator over something that is not a numeric literal.
        "-OTHER",
        // A non-folding unary operator over a numeric literal.
        "~1",
        // A binary fold the recognizer deliberately does not perform.
        "1 + 1",
    ] {
        let source = format!("def f(a: int = {default}) -> None:\n    return\n");
        let diagnostic = lower_err(&source);
        assert_eq!(diagnostic.code, "C0001", "default: {default}");
        assert_eq!(diagnostic.message, UNADMITTED_MESSAGE, "default: {default}");
        assert_eq!(
            diagnostic.span,
            Some(span_of(&source, default)),
            "default: {default}"
        );
    }
}

#[test]
fn a_negated_int_outside_i64_is_still_refused() {
    // `fold_int_literal_sign` applies the sign before the range check, so
    // `-9223372036854775808` is admitted; one step further is not.
    let diagnostic = lower_err("def f(a: int = -9223372036854775809) -> None:\n    return\n");
    assert_eq!(diagnostic.code, "C0001");
    assert_eq!(diagnostic.message, UNADMITTED_MESSAGE);
}

#[test]
fn a_signed_complex_literal_is_refused() {
    let diagnostic = lower_err("def f(a: int = -1j) -> None:\n    return\n");
    assert_eq!(diagnostic.code, "C0001");
    assert_eq!(diagnostic.message, UNADMITTED_MESSAGE);
}

#[test]
fn a_default_on_a_type_parameter_annotated_parameter_is_a_capability_error() {
    let source = "def f[T](a: T = 1) -> None:\n    return\n";
    let diagnostic = lower_err(source);
    assert_eq!(diagnostic.code, "C0001");
    assert_eq!(
        diagnostic.message,
        "a default value on parameter `a` of `f`, whose annotation is a type parameter, is not \
         supported yet"
    );
    assert_eq!(diagnostic.span, Some(span_of(source, "1")));
}

// --- defaults the type rule refuses -----------------------------------

#[test]
fn a_default_of_the_wrong_type_is_a_call_argument_mismatch() {
    for (annotation, literal, from) in [
        ("int", "None", "None"),
        ("str", "None", "None"),
        ("float", "1", "int"),
        ("int", "\"s\"", "str"),
        ("str", "1", "int"),
        ("bool", "1", "int"),
    ] {
        let source = format!("def f(a: {annotation} = {literal}) -> None:\n    return\n");
        let diagnostic = lower_err(&source);
        assert_eq!(diagnostic.code, "T0021", "annotation: {annotation}");
        assert_eq!(
            diagnostic.message,
            format!("default value of parameter `a` of `f` expects `{annotation}`, got `{from}`"),
            "annotation: {annotation}"
        );
        assert_eq!(
            diagnostic.help.as_deref(),
            Some(
                format!(
                    "a default value is passed as an argument, so it must already be \
                     `{annotation}`"
                )
                .as_str()
            ),
            "annotation: {annotation}"
        );
        assert_eq!(
            diagnostic.span,
            Some(span_of(&source, literal)),
            "annotation: {annotation}"
        );
    }
}

#[test]
fn a_wider_optional_annotation_keeps_its_own_unsupported_annotation_error() {
    // `str | None` is not an annotation this compiler supports; the
    // annotation is resolved first, so its own diagnostic wins over
    // anything the default could say. This is a deliberate change: before
    // this part the same source reported `C0001 default parameter values
    // are not supported yet` at the parameter, because the default check
    // ran before `annotation_to_ty`.
    let source = "def f(a: str | None = None) -> None:\n    return\n";
    let diagnostic = lower_err(source);
    assert_eq!(diagnostic.code, "T0049", "{diagnostic:?}");
    assert_eq!(
        diagnostic.message,
        "`Optional[str]` is not supported yet -- only `Optional[int]`, \
         `Optional[float]`, and `Optional[bool]` (`int | None`, `float | None`, \
         `bool | None`) are"
    );
    assert_eq!(diagnostic.span, Some(span_of(source, "str | None")));
}

#[test]
fn an_optional_annotation_still_rejects_an_unassignable_default() {
    let diagnostic = lower_err("def f(a: int | None = \"s\") -> None:\n    return\n");
    assert_eq!(diagnostic.code, "T0021");
    assert_eq!(
        diagnostic.message,
        "default value of parameter `a` of `f` expects `int | None`, got `str`"
    );
}

#[test]
fn an_unsupported_annotation_is_reported_before_its_default() {
    // Under `DefaultPolicy::Admit` the annotation is resolved first, so an
    // annotation pycc does not support keeps reporting its own diagnostic,
    // at the annotation's own span rather than the default's.
    let source = "def f(a: list[int] | None = None) -> None:\n    return\n";
    let diagnostic = lower_err(source);
    assert_eq!(diagnostic.code, "T0049", "{diagnostic:?}");
    assert_eq!(
        diagnostic.message,
        "`Optional[list[int]]` is not supported yet -- only `Optional[int]`, \
         `Optional[float]`, and `Optional[bool]` (`int | None`, `float | None`, \
         `bool | None`) are"
    );
    assert_eq!(diagnostic.span, Some(span_of(source, "list[int] | None")));
}

// --- `DefaultPolicy::Reject` and method defaults ---------------------

#[test]
fn a_default_on_a_protocol_member_keeps_the_unchanged_capability_error() {
    // Only a protocol member still passes `DefaultPolicy::Reject`; every
    // other method admits a default since the method part of #1140
    // (`class::method_tests`).
    let diagnostic = lower_err(
        "from typing import Protocol\n\nclass P(Protocol):\n    def m(self, a: int = 1) -> None:\n        ...\n",
    );
    assert_eq!(diagnostic.code, "C0001");
    assert_eq!(
        diagnostic.message,
        "default parameter values are not supported yet"
    );
}

#[test]
fn a_protocol_member_default_is_reported_before_its_annotation_is_resolved() {
    // Under `DefaultPolicy::Reject` the default check runs *before*
    // annotation resolution, so an unsupported annotation on a defaulted
    // protocol-member parameter keeps reporting the default's own capability
    // error rather than the annotation's.
    let diagnostic = lower_err(
        "from typing import Protocol\n\nclass P(Protocol):\n    def m(self, a: list[int] | None = None) -> None:\n        ...\n",
    );
    assert_eq!(diagnostic.code, "C0001");
    assert_eq!(
        diagnostic.message,
        "default parameter values are not supported yet"
    );
}

#[test]
fn a_method_default_is_reported_after_its_annotation_is_resolved() {
    // A method admits defaults, so it follows the module-level order: the
    // unsupported annotation reports first.
    let diagnostic = lower_err(
        "class C:\n    def m(self, a: list[int] | None = None) -> None:\n        return\n",
    );
    assert_eq!(diagnostic.code, "T0049", "{diagnostic:?}");
}

#[test]
fn a_receiver_parameter_keeps_its_own_default_message() {
    let diagnostic =
        lower_err("class C:\n    def m(self = 1, a: int = 2) -> None:\n        return\n");
    assert_eq!(diagnostic.code, "C0001");
    assert_eq!(
        diagnostic.message,
        "a method's receiver parameter cannot have a default value"
    );
}

#[test]
fn a_classmethod_cls_parameter_keeps_its_own_default_message() {
    let diagnostic =
        lower_err("class C:\n    @classmethod\n    def m(cls = 1) -> None:\n        return\n");
    assert_eq!(diagnostic.code, "C0001");
    assert!(
        diagnostic
            .message
            .contains("`cls` cannot have a default value"),
        "{diagnostic:?}"
    );
}

// --- multi-diagnostic collection --------------------------------------

#[test]
fn two_bad_defaults_in_one_module_are_both_reported() {
    // `signature_of` stays infallible, so collecting the signature table
    // cannot short-circuit the per-item diagnostic collection #878 relies
    // on: each bad `def` reports once, at its own default.
    let diagnostics = lower_all_err(
        "def f(a: int = OTHER) -> None:\n    return\n\ndef g(b: int = OTHER) -> None:\n    return\n",
    );
    let unadmitted: Vec<_> = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.message == UNADMITTED_MESSAGE)
        .collect();
    assert_eq!(unadmitted.len(), 2, "{diagnostics:?}");
}

#[test]
fn one_bad_default_with_several_call_sites_is_reported_once() {
    // Rejection lives at the `def`, never at a call, so the user sees one
    // diagnostic however many times the function is called.
    let diagnostics =
        lower_all_err("def f(a: int = OTHER) -> None:\n    return\n\nf(1)\nf(2)\nf(3)\n");
    let unadmitted: Vec<_> = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.message == UNADMITTED_MESSAGE)
        .collect();
    assert_eq!(unadmitted.len(), 1, "{diagnostics:?}");
}

// --- the non-default arms of the rewritten parameter loop ---------------

#[test]
fn a_public_functions_unannotated_parameter_still_needs_an_annotation() {
    for source in [
        "def f(a) -> None:\n    return\n",
        // A default never supplies the annotation a public function's
        // parameter needs: `T0001` wins over both inference and the
        // default's own rules.
        "def f(a = 1) -> None:\n    return\n",
    ] {
        let diagnostic = lower_err(source);
        assert_eq!(diagnostic.code, "T0001", "{source}");
        assert_eq!(
            diagnostic.message, "parameter `a` of public function `f` needs a type annotation",
            "{source}"
        );
    }
}

#[test]
fn a_bare_container_annotation_still_carries_its_advice() {
    let diagnostic = lower_err("def f(a: list) -> None:\n    return\n");
    assert_eq!(diagnostic.code, "C0001");
    assert!(
        diagnostic.message.contains("list["),
        "the bare-container advice must survive the parameter loop's rewrite: {diagnostic:?}"
    );
}
