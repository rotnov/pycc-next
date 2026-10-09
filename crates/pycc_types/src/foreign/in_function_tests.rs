//! #1316: a module-level foreign name read inside a function body.
//!
//! Kept apart from `foreign/tests.rs`, which is already past the ~1,000
//! line decomposition threshold; the fixture helpers are copied from
//! `foreign/call_tests.rs` because a sibling module cannot reach them.

/// Lowers `source` with every `import` request answered `Foreign`.
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

const IMPORT: &str = "import copy\n";

fn admitted(source: &str) {
    check(source).unwrap_or_else(|diagnostics| panic!("{source:?}: {diagnostics:#?}"));
}

/// The single diagnostic `source` is refused with, asserted by code and a
/// message phrase.
fn refused(source: &str, code: &str, phrase: &str) {
    let diagnostics = check(source).expect_err(source);
    assert_eq!(diagnostics.len(), 1, "{source:?}: {diagnostics:#?}");
    assert_eq!(diagnostics[0].code, code, "{source:?}: {diagnostics:#?}");
    assert!(
        diagnostics[0].message.contains(phrase),
        "{source:?}: {diagnostics:#?}"
    );
}

/// Every operation a module body admits on a CPython object is admitted
/// in an annotated function body too.
#[test]
fn every_admitted_operation_is_admitted_in_a_function_body() {
    for body in [
        // Attribute load, method call, direct call.
        "    copy.__name__\n",
        "    copy.copy(3)\n",
        "    copy()\n",
        // `len`, subscript, truth test in condition and value position.
        "    n = len(copy.__name__)\n",
        "    copy.__name__[0]\n",
        "    if copy.__name__:\n        pass\n",
        "    b = bool(copy)\n",
        // The four conversions.
        "    s = str(copy.__name__)\n",
        "    i = int(copy.x)\n",
        "    f = float(copy.x)\n",
        "    t = bool(copy.x)\n",
    ] {
        admitted(&format!("{IMPORT}def f() -> None:\n{body}"));
    }
}

/// The issue's own example: the result of a conversion leaves the function.
#[test]
fn a_converted_result_may_be_returned() {
    admitted(&format!(
        "{IMPORT}def name() -> str:\n    return str(copy.__name__)\n\n\nprint(name())\n"
    ));
}

#[test]
fn a_method_body_may_read_a_foreign_global() {
    admitted(&format!(
        "{IMPORT}class A:\n    def m(self) -> str:\n        return str(copy.__name__)\n\n\nprint(A().m())\n"
    ));
}

/// The solver path: an unannotated private helper's return is inferred
/// through the foreign read.
#[test]
fn an_unannotated_helper_may_read_a_foreign_global() {
    admitted(&format!(
        "{IMPORT}def _ident():\n    return len(copy.__name__)\n\n\nprint(_ident())\n"
    ));
}

/// A function-local binding of the same name is an ordinary local.
#[test]
fn a_local_rebinding_shadows_the_foreign_global() {
    admitted(&format!(
        "{IMPORT}def f() -> int:\n    copy = 1\n    return copy\n\n\nprint(f())\n"
    ));
}

/// Part 1 of #1333: a CPython object may become a function-local value, a
/// return value and an argument to an ordinary pycc function.
#[test]
fn a_foreign_object_may_become_a_function_local_value() {
    for body in [
        "def f() -> None:\n    x = copy\n",
        "def _f():\n    return copy\n",
        "def _g(x):\n    pass\n\n\ndef f() -> None:\n    _g(copy)\n",
    ] {
        admitted(&format!("{IMPORT}{body}"));
    }
}

/// What stays refused: a function-body `for` over an object (Part 2 of
/// #1333, #1363) and passing an object to a generic function.
#[test]
fn a_function_body_loop_or_generic_argument_stays_refused() {
    for (phrase, body) in [
        (
            "is not supported inside a function body",
            "def f() -> None:\n    for x in copy.xs:\n        pass\n",
        ),
        (
            "passing a CPython object to a generic function",
            "def _gen[T](x: T) -> T:\n    return x\n\n\ndef f() -> None:\n    _gen(copy)\n",
        ),
    ] {
        refused(&format!("{IMPORT}{body}"), "I0404", phrase);
    }
}

/// A comprehension binds its target locally, so a comprehension over a
/// CPython object stays refused inside a function body.
#[test]
fn a_comprehension_over_a_foreign_object_stays_refused() {
    let source = format!("{IMPORT}def f() -> None:\n    xs = [y for y in copy]\n");
    let diagnostics = check(&source).expect_err("a comprehension over an object is refused");
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    assert_eq!(diagnostics[0].code, "I0404", "{diagnostics:#?}");
}

/// An unannotated parameter is inferred as `object` from a call that
/// passes a CPython object (Part 1 of #1333), so `len(x)` on it is the
/// dynamic CPython call.
#[test]
fn an_inferred_parameter_may_be_an_object() {
    admitted(&format!(
        "{IMPORT}def _g(x) -> int:\n    return len(x)\n\n\n_g(copy)\n"
    ));
}

#[test]
fn a_maybe_bound_foreign_global_is_t0041() {
    refused(
        "flag = True\nif flag:\n    import copy\ndef f() -> None:\n    copy.__name__\n",
        "T0041",
        "may not be bound",
    );
}

#[test]
fn a_global_statement_is_unchanged_c0001() {
    let source = format!("{IMPORT}def f() -> None:\n    global copy\n    copy.__name__\n");
    let module = pycc_parser::parse(&source).expect("test source must parse");
    let mut resolved = pycc_hir::ResolvedImports::default();
    for request in pycc_hir::project_import_requests(&module) {
        resolved.insert(request.span, pycc_hir::ResolvedImport::Foreign);
    }
    let diagnostics = pycc_hir::lower_module(&module, &resolved, None)
        .expect_err("`global` is refused while lowering");
    assert_eq!(diagnostics[0].code, "C0001", "{diagnostics:#?}");
}

/// The known #1101 gap: a PEP 695 generic reading a foreign global passes
/// the check phase, but monomorphization does not seed foreign names and
/// refuses the read as an undefined name.
#[test]
fn a_generic_reading_a_foreign_global_hits_the_monomorphization_gap() {
    let source = format!(
        "{IMPORT}def _gen[T](x: T) -> T:\n    copy.__name__\n    return x\n\n\nprint(_gen(1))\n"
    );
    let hir = lower_all_foreign(&source);
    assert!(crate::check_all(&hir).is_ok(), "the check phase admits it");
    let Err(diagnostics) = crate::check_and_resolve_all_keyed(&hir) else {
        panic!("monomorphization refuses the read");
    };
    assert_eq!(diagnostics[0].1.code, "T0021", "{diagnostics:#?}");
    assert!(
        diagnostics[0].1.message.contains("`copy` is not defined"),
        "{diagnostics:#?}"
    );
}

/// A declared parameter keeps the ordinary argument mismatch: the
/// CPython-object refusal is reached only by a parameter an `object` is
/// assignable to. A method or constructor never has one -- its parameters
/// are declared, or refused as uninferable -- so it needs no refusal of
/// its own.
#[test]
fn a_declared_parameter_keeps_its_argument_mismatch() {
    refused(
        &format!(
            "{IMPORT}class A:\n    def _m(self, x):\n        pass\n\n\ndef f() -> None:\n    A()._m(copy)\n"
        ),
        "T0021",
        "cannot infer type of parameter `x` in private helper `A._m`",
    );
    refused(
        &format!(
            "{IMPORT}class Box[T]:\n    def __init__(self, v: T) -> None:\n        self.v = v\n\n\ndef f() -> None:\n    Box(copy)\n"
        ),
        "T0021",
        "argument 1 of `Box` expects `T`, got `object`",
    );
    refused(
        &format!("{IMPORT}def _g(x: int) -> None:\n    pass\n\n\ndef f() -> None:\n    _g(copy)\n"),
        "T0021",
        "argument 1 of `_g` expects `int`, got `object`",
    );
}
