//! Unit tests for `set_element.rs` (#1343, Part 1 of #1336): one test per
//! arm, driven through the real parser, HIR lowering and checker.

use super::{HashMethodRefusal, check_set_element};
use crate::env::Environment;
use pycc_diag::Diagnostic;
use pycc_hir::Ty;

fn check(source: &str) -> Result<(), Diagnostic> {
    let module = pycc_parser::parse(source).expect("test fixture must parse");
    let hir = pycc_hir::lower_checked(&module).expect("test fixture must lower");
    crate::check_and_resolve(&hir).map(|_| ())
}

/// The one diagnostic `class_body` gets when its class `R` is the element of
/// a set literal and of `.add` on a `set[R]` parameter.
fn refusals(class_body: &str) -> [Diagnostic; 2] {
    let literal =
        format!("{class_body}\ndef f() -> int:\n    s = {{R(1), R(2)}}\n    return len(s)\n");
    let add = format!("{class_body}\ndef g(s: set[R]) -> None:\n    s.add(R(3))\n");
    [literal, add].map(|source| check(&source).expect_err("fixture must be refused"))
}

const INIT: &str = "    def __init__(self, v: int) -> None:\n        self.v = v\n";
const HASH: &str = "    def __hash__(self) -> int:\n        return self.v\n";
const EQ: &str = "    def __eq__(self, other: R) -> bool:\n        return self.v == other.v\n";

#[test]
fn an_int_and_an_admitted_instance_element_pass() {
    check("def f() -> int:\n    s = {1, 2}\n    s.add(3)\n    return len(s)\n").unwrap();
    for body in [
        format!("class R:\n{INIT}"),
        format!("class R:\n{INIT}{HASH}{EQ}"),
        format!("class B:\n{INIT}{HASH}{EQ}\nclass R(B):\n    pass\n")
            .replace("other: R", "other: B"),
    ] {
        let source = format!(
            "{body}\ndef f(s: set[R]) -> int:\n    t = {{R(1), R(2)}}\n    s.add(R(3))\n    return len(t)\n"
        );
        check(&source).unwrap_or_else(|d| panic!("{source}: {}", d.message));
    }
}

#[test]
fn a_dataclass_with_its_own_hash_compares_through_the_synthesized_eq() {
    let source = "from dataclasses import dataclass\n\n@dataclass\nclass R:\n    v: int\n\n    def __hash__(self) -> int:\n        return self.v\n\ndef f() -> int:\n    return len({R(1), R(1)})\n";
    check(source).unwrap_or_else(|d| panic!("{}", d.message));
}

#[test]
fn an_eq_without_hash_is_t0054_with_help() {
    for error in refusals(&format!("class R:\n{INIT}{EQ}")) {
        assert_eq!(error.code, "T0054");
        assert_eq!(
            error.message,
            "cannot use 'R' as a set element (unhashable type: 'R')"
        );
        assert!(
            error
                .help
                .as_deref()
                .unwrap()
                .contains("define `__hash__` on `R`"),
            "{error:?}"
        );
    }
}

#[test]
fn a_hash_side_refusal_is_c0001_with_its_help() {
    let body = "from dataclasses import dataclass\n\n@dataclass\nclass R:\n    v: int\n";
    for error in refusals(body) {
        assert_eq!(error.code, "C0001");
        assert_eq!(
            error.message,
            "a set element of class `R` is valid Python but not implemented yet"
        );
        assert!(
            error
                .help
                .as_deref()
                .unwrap()
                .contains("define `__hash__` explicitly")
        );
    }
}

#[test]
fn an_eq_side_refusal_is_c0001_with_its_help() {
    let property = "    @property\n    def __eq__(self) -> int:\n        return 1\n";
    let subclassed = format!("class R:\n{INIT}{HASH}{EQ}\nclass S(R):\n    pass\n");
    for (body, needle) in [
        (
            format!("class R:\n{INIT}{HASH}{property}"),
            "binds `__eq__`",
        ),
        (subclassed, "subclass `S`"),
    ] {
        for error in refusals(&body) {
            assert_eq!(error.code, "C0001", "{body}");
            assert!(error.help.as_deref().unwrap().contains(needle), "{error:?}");
        }
    }
}

#[test]
fn a_hash_method_signature_pycc_does_not_compile_is_c0001() {
    for hash in [
        "    def __hash__(self, k: int) -> int:\n        return k\n",
        "    def __hash__(self) -> int | None:\n        v: int | None = None\n        return v\n",
    ] {
        for error in refusals(&format!("class R:\n{INIT}{hash}")) {
            assert_eq!(error.code, "C0001", "{hash}: {}", error.message);
            assert!(error.help.as_deref().unwrap().contains("`R.__hash__`"));
        }
    }
}

#[test]
fn a_hash_method_returning_a_non_integer_is_t0021() {
    let hash = "    def __hash__(self) -> str:\n        return \"a\"\n";
    for error in refusals(&format!("class R:\n{INIT}{hash}")) {
        assert_eq!(error.code, "T0021");
        assert_eq!(error.message, "`__hash__` method should return an integer");
    }
}

#[test]
fn an_eq_method_signature_pycc_does_not_compile_is_c0001() {
    for eq in [
        "    def __eq__(self, other: R, extra: int) -> bool:\n        return True\n",
        "    def __eq__(self, other: R) -> int:\n        return 1\n",
        "    def __eq__(self, other: U) -> bool:\n        return True\n",
    ] {
        let body = format!("class U:\n    pass\n\nclass R:\n{INIT}{HASH}{eq}");
        for error in refusals(&body) {
            assert_eq!(error.code, "C0001", "{eq}");
            assert!(
                error
                    .help
                    .as_deref()
                    .unwrap()
                    .contains("`def __eq__(self, other: K) -> bool`"),
                "{error:?}"
            );
        }
    }
}

#[test]
fn a_non_int_non_instance_element_is_t0038() {
    let error = check_set_element(&Ty::Str, &Environment::default()).unwrap_err();
    assert_eq!(error.code, "T0038");
    assert_eq!(
        error.message,
        "set[str] is not compiled yet (D-122) -- only set[int] and a set of a user-class \
         instance are"
    );
}

#[test]
fn a_hash_method_refusal_wraps_in_the_callers_headline() {
    let not_compiled = HashMethodRefusal::NotCompiled("h".to_string()).into_diagnostic(|help| {
        Diagnostic::error("C0001", "x", pycc_diag::Span::new(0, 0)).with_help(help)
    });
    assert_eq!(
        (not_compiled.code, not_compiled.help.as_deref()),
        ("C0001", Some("h"))
    );
}
