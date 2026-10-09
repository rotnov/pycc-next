//! Unit tests for `check_method_call_args` (Part 1 of #1191, issue #1438).

use crate::check_and_resolve;
use pycc_diag::Diagnostic;

fn check_source(source: &str) -> Result<pycc_hir::HirModule, Diagnostic> {
    let module = pycc_parser::parse(source).expect("test fixture must parse");
    let hir = pycc_hir::lower_checked(&module).expect("test fixture must lower");
    check_and_resolve(&hir)
}

/// Lowers `source` as a module compiled into an `--ext` artifact, where
/// `Any` is the opaque CPython object (`Ty::Object`), then type-checks it.
fn check_ext_source(source: &str) -> Result<pycc_hir::HirModule, Diagnostic> {
    let module = pycc_parser::parse(source).expect("test fixture must parse");
    let mut resolved = pycc_hir::ResolvedImports::default();
    resolved.set_ext_module(true);
    let lowered = pycc_hir::lower_module(&module, &resolved, None)
        .unwrap_or_else(|diagnostics| panic!("test fixture must lower: {diagnostics:#?}"));
    check_and_resolve(&lowered.hir)
}

const CLASS: &str = "class C:\n\
    \x20   def m(self, a: int, b: int = 2, c: str = 'x') -> int:\n        return a + b\n\
    \x20   def plain(self, a: int) -> int:\n        return a\n";

#[test]
fn a_call_omitting_only_defaulted_parameters_is_accepted() {
    for call in ["C().m(1)\n", "C().m(1, 5)\n", "C().m(1, 5, 'y')\n"] {
        check_source(&format!("{CLASS}{call}")).unwrap_or_else(|err| panic!("{call:?}: {err:?}"));
    }
}

#[test]
fn omitting_a_required_parameter_reports_the_accepted_range() {
    let err = check_source(&format!("{CLASS}C().m()\n")).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(err.message, "`m` expects from 1 to 3 argument(s), got 0");
    assert_eq!(
        err.help.as_deref(),
        Some("pass at least 1 and at most 3 argument(s)")
    );
}

#[test]
fn passing_too_many_arguments_reports_the_accepted_range() {
    let err = check_source(&format!("{CLASS}C().m(1, 2, 'y', 4)\n")).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(err.message, "`m` expects from 1 to 3 argument(s), got 4");
}

#[test]
fn a_method_without_defaults_keeps_the_exact_arity_message() {
    let err = check_source(&format!("{CLASS}C().plain()\n")).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(err.message, "`plain` expects 1 argument(s), got 0");
}

#[test]
fn a_wrong_supplied_argument_is_still_t0021() {
    let err = check_source(&format!("{CLASS}C().m('no')\n")).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(err.message, "argument 1 of `m` expects `int`, got `str`");
}

#[test]
fn filling_a_none_default_at_an_object_parameter_is_refused() {
    let err = check_ext_source(
        "from typing import Any\nclass C:\n    def m(self, a: int, o: Any = None) -> int:\n        return a\n\
         C().m(1)\n",
    )
    .unwrap_err();
    assert_eq!(err.code, "C0001");
    assert_eq!(
        err.message,
        "filling the `None` default of argument 2 of `m`, an `object` parameter, \
         at an in-module call is not supported yet"
    );
    // Passing an `object` value explicitly is the documented workaround.
    check_ext_source(
        "from typing import Any\nclass C:\n    def m(self, a: int, o: Any = None) -> int:\n        return a\n\
         \x20   def run(self, o: Any) -> int:\n        return self.m(1, o)\n",
    )
    .expect("an explicit object argument is accepted");
}

/// A PEP 695 generic class's specializations carry the origin's method
/// defaults, re-keyed to the specialized names, so a call on an explicit
/// specialization (`Box[int](1).get()`) is filled exactly like one on the
/// origin (`Box(1).get()`) instead of reporting the arity `T0021` from the
/// build entry point.
#[test]
fn a_generic_class_specialization_fills_the_same_defaults() {
    let source = "class Box[T]:\n\
        \x20   def __init__(self, v: T) -> None:\n        self.v = v\n\
        \x20   def get(self, k: int = 1) -> int:\n        return k\n\
        print(Box(1).get())\nprint(Box[int](1).get())\n";
    let module = pycc_parser::parse(source).expect("test fixture must parse");
    let hir = pycc_hir::lower_checked(&module).expect("test fixture must lower");
    crate::check_and_resolve_all_keyed(&hir).unwrap_or_else(|err| panic!("{err:?}"));
}
