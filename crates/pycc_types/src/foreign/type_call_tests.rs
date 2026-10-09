//! Part 11 of #1371: `type(o)` on a CPython object, and `is` chains over
//! one.
//!
//! `type(o)` with one object argument is typed as an `object` by both the
//! checker (`crate::expr`) and the solver (`crate::constraints`, reached by
//! an unannotated return); every other `type(...)` shape keeps its
//! `C0001`. An `is`/`is not` chain now reaches the checker, which refuses
//! one with an object operand with the `I0404` every other object chain
//! gets. The fixture helpers are copied from `foreign/raise_tests.rs`
//! because a sibling module cannot reach them.

fn lower_all_foreign(source: &str) -> pycc_hir::HirModule {
    let module = pycc_parser::parse(source).expect("test source must parse");
    let mut resolved = pycc_hir::ResolvedImports::default();
    resolved.set_ext_module(true);
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

const IMPORT: &str = "from h import k, j\n";

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

/// `type(o)` is an `object` wherever one is: printed, bound, compared by
/// identity, nested, read for an attribute, in a module body and a
/// function body.
#[test]
fn type_of_an_object_is_admitted_as_an_object() {
    admitted(&format!("{IMPORT}print(type(k))\n"));
    admitted(&format!("{IMPORT}t = type(k)\nprint(t.__name__)\n"));
    admitted(&format!(
        "{IMPORT}def f() -> None:\n    t = type(k)\n    print(t is type(j))\n    print(type(type(k)))\nf()\n"
    ));
    admitted(&format!(
        "{IMPORT}def f() -> bool:\n    return type(k) is not type(j)\nprint(f())\n"
    ));
}

/// An unannotated return is typed by the solver, whose own `type(o)` arm
/// answers `object` too; a caller then consumes it as one.
#[test]
fn the_solver_types_type_of_an_object_as_an_object() {
    admitted(&format!(
        "{IMPORT}def _g():\n    return type(k)\n\n\nprint(_g().__name__)\n"
    ));
}

/// An unannotated parameter is still unresolved when the solver walks the
/// body: its `type(x)` is read as an object, and the final pass re-types
/// the call with the parameter's resolved type -- an object is admitted, a
/// native value keeps the known-builtin `C0001`.
#[test]
fn type_of_an_unannotated_parameter_follows_its_resolved_type() {
    admitted(&format!(
        "{IMPORT}def _g(x):\n    return type(x)\n\n\nprint(_g(k).__name__)\n"
    ));
    refused(
        "def _g(x):\n    return type(x)\n\n\nprint(_g(5))\n",
        "C0001",
        "call to builtin `type` is valid Python but not implemented yet",
    );
}

/// Every other `type(...)` shape keeps its known-builtin `C0001`: a native
/// argument, two arguments, none, and a stdlib module alias spelled
/// `type`.
#[test]
fn the_other_type_shapes_keep_their_c0001() {
    let phrase = "call to builtin `type` is valid Python but not implemented yet";
    refused("print(type(5))\n", "C0001", phrase);
    refused(&format!("{IMPORT}print(type(k, j))\n"), "C0001", phrase);
    refused("print(type())\n", "C0001", phrase);
    refused(
        &format!("{IMPORT}def _g():\n    return type(5)\n\n\nprint(_g())\n"),
        "C0001",
        phrase,
    );
}

/// A program's own `def type` keeps its meaning: the call resolves to it,
/// and a CPython object argument meets its `int` parameter.
#[test]
fn a_user_type_function_is_not_the_builtin() {
    let diagnostics = check(&format!(
        "{IMPORT}def type(x: int) -> int:\n    return x\n\n\nprint(type(k))\n"
    ))
    .expect_err("an object argument to an `int` parameter");
    assert!(
        diagnostics
            .iter()
            .all(|diagnostic| !diagnostic.message.contains("builtin `type`")),
        "{diagnostics:#?}"
    );
}

/// An `is`/`is not` chain with an object operand reaches the checker and
/// is refused with the object chain's own `I0404`, whichever link holds
/// the object.
#[test]
fn an_identity_chain_over_an_object_is_refused_with_i0404() {
    let phrase = "a chained comparison with a CPython object operand";
    refused(&format!("{IMPORT}print(k is k is k)\n"), "I0404", phrase);
    refused(
        &format!("{IMPORT}print(k is not j is k)\n"),
        "I0404",
        phrase,
    );
    refused(
        &format!("{IMPORT}def f(a: int) -> bool:\n    return a is k is a\n"),
        "I0404",
        phrase,
    );
}

/// An `isinstance` guard narrows an `object` name to a native type, but
/// `type(o)` still asks CPython for the object's own class -- a subclass of
/// `int` stays that subclass -- so a bare narrowed read is the object here,
/// as an identity operand or a nested guard's is (#1476). Both the checker
/// and the solver (an unannotated return) read it so; a derived native
/// value keeps the `C0001`.
#[test]
fn type_of_a_narrowed_object_is_the_objects_own() {
    admitted("def f(o: object) -> None:\n    if isinstance(o, int):\n        print(type(o))\n");
    admitted(
        "def _g(o: object):\n    if isinstance(o, int):\n        return type(o)\n    return o\n\n\nprint(_g(3))\n",
    );
    refused(
        "def f(o: object) -> None:\n    if isinstance(o, int):\n        print(type(o + 1))\n",
        "C0001",
        "call to builtin `type` is valid Python but not implemented yet",
    );
}

/// A native identity chain keeps the `C0001` a single native identity
/// comparison gets.
#[test]
fn a_native_identity_chain_keeps_its_c0001() {
    refused(
        "def f(a: int, b: int) -> bool:\n    return a is b is a\n",
        "C0001",
        "comparison operator not supported yet: Is",
    );
}
