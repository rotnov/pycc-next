//! Part 9 of #1371: `raise o` with a CPython object `o`.
//!
//! Whether `o` is an exception at all is CPython's to decide at run time,
//! exactly as for CPython's own `raise`, so the checker admits every
//! `Ty::Object` operand. Chaining a cause onto, or from, an object is not
//! built yet and is refused with `C0001`. The fixture helpers are copied
//! from `foreign/call_tests.rs` because a sibling module cannot reach them.

fn lower_all_foreign(source: &str) -> pycc_hir::HirModule {
    let module = pycc_parser::parse(source).expect("test source must parse");
    let mut resolved = pycc_hir::ResolvedImports::default();
    for request in pycc_hir::project_import_requests(&module) {
        resolved.insert(request.span, pycc_hir::ResolvedImport::Foreign);
    }
    pycc_hir::lower_module(&module, &resolved, None)
        .unwrap_or_else(|diagnostics| panic!("{source:?} must lower: {diagnostics:#?}"))
        .hir
}

fn check(source: &str) -> Result<(), Vec<pycc_diag::Diagnostic>> {
    crate::check_all(&lower_all_foreign(source)).map(|_| ())
}

const IMPORT: &str = "from errs import Boom\n";

fn admitted(source: &str) {
    check(source).unwrap_or_else(|diagnostics| panic!("{source:?}: {diagnostics:#?}"));
}

fn refused(source: &str, code: &str, phrase: &str) {
    let diagnostics = check(source).expect_err(source);
    assert_eq!(diagnostics.len(), 1, "{source:?}: {diagnostics:#?}");
    assert_eq!(diagnostics[0].code, code, "{source:?}: {diagnostics:#?}");
    assert!(
        diagnostics[0].message.contains(phrase),
        "{source:?}: {diagnostics:#?}"
    );
}

/// Every operand shape that evaluates to an object is admitted in the
/// module body: a constructor call (lark's `raise UnexpectedToken(...)`),
/// a bare class or instance name, and an attribute of a module object.
#[test]
fn every_object_operand_shape_is_admitted_in_the_module_body() {
    admitted(&format!("{IMPORT}raise Boom(1, \"a\")\n"));
    admitted(&format!("{IMPORT}raise Boom\n"));
    admitted(&format!("{IMPORT}b = Boom()\nraise b\n"));
    admitted("import errs\nraise errs.Boom(2)\n");
}

/// A function body, a method body, and a `raise` nested in control flow
/// and in an `except` handler are all admitted.
#[test]
fn an_object_raise_is_admitted_in_functions_methods_and_nested_blocks() {
    admitted(&format!(
        "{IMPORT}def f() -> None:\n    raise Boom(1)\nf()\n"
    ));
    admitted(&format!(
        "{IMPORT}def f(n: int) -> int:\n    if n > 0:\n        return n\n    raise Boom(n)\nprint(f(1))\n"
    ));
    admitted(&format!(
        "{IMPORT}class C:\n    def m(self) -> None:\n        raise Boom(\"m\")\nC().m()\n"
    ));
    admitted(&format!(
        "{IMPORT}def f(n: int) -> None:\n    while n > 0:\n        try:\n            n -= 1\n        except ValueError:\n            raise Boom(n)\nf(2)\n"
    ));
}

/// A cause on either side of an object is the deferred `raise ... from`.
/// `from None` is not a cause: `pycc_hir` collapses `raise X from None`
/// into `raise X` for every operand (PEP 409, #540), so it is admitted.
#[test]
fn chaining_a_cause_onto_or_from_an_object_is_refused() {
    for source in [
        format!("{IMPORT}raise Boom(1) from ValueError(\"c\")\n"),
        format!("{IMPORT}raise ValueError(\"e\") from Boom(1)\n"),
        format!("{IMPORT}raise Boom(1) from Boom(2)\n"),
    ] {
        refused(
            &source,
            "C0001",
            "`raise ... from ...` with a CPython object",
        );
    }
    admitted(&format!("{IMPORT}raise Boom(1) from None\n"));
}

/// A native non-exception operand is still refused: only `Ty::Object` is
/// deferred to run time.
#[test]
fn a_native_non_exception_operand_is_still_refused() {
    refused(
        &format!("{IMPORT}raise 1\n"),
        "T0021",
        "can only raise exception instances",
    );
    // The native `from` path is unchanged.
    admitted("raise ValueError(\"e\") from KeyError(\"c\")\n");
}

/// Matching a foreign exception class in an `except` clause is out of
/// scope for Part 9 and stays refused; `except Exception` and the builtin
/// classes catch an object raise through the bridge's tag map instead.
#[test]
fn an_except_clause_naming_a_foreign_class_is_still_refused() {
    refused(
        &format!("{IMPORT}try:\n    raise Boom(1)\nexcept Boom:\n    pass\n"),
        "T0021",
        "`Boom` is not a recognized exception class",
    );
    admitted(&format!(
        "{IMPORT}try:\n    raise Boom(1)\nexcept Exception:\n    pass\n"
    ));
}
