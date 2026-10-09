//! Part 1 of #1333: a CPython object bound, returned and passed inside a
//! function body, and the constraint solver's `object` terms that make an
//! unannotated private helper's signature resolve.
//!
//! Kept apart from `foreign/tests.rs` (see #1314); the fixture helpers are
//! copied from `call_tests.rs` because a sibling module cannot reach them.

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

const IMPORT: &str = "import json\n";

fn admitted(source: &str) {
    crate::check_all(&lower_all_foreign(source))
        .unwrap_or_else(|diagnostics| panic!("{source:?}: {diagnostics:#?}"));
}

/// The single diagnostic `source` is refused with, asserted by code and a
/// message phrase.
fn refused(source: &str, code: &str, phrase: &str) {
    let diagnostics = crate::check_all(&lower_all_foreign(source)).expect_err(source);
    assert_eq!(diagnostics.len(), 1, "{source:?}: {diagnostics:#?}");
    assert_eq!(diagnostics[0].code, code, "{source:?}: {diagnostics:#?}");
    assert!(
        diagnostics[0].message.contains(phrase),
        "{source:?}: {diagnostics:#?}"
    );
}

/// `object + int` is refused naming `object`, so a helper whose result is
/// used this way proves the solver resolved its return to `object` rather
/// than admitting it under some other type.
const RESOLVED_TO_OBJECT: &str = "operator Add is not defined for `object` and `int`";

/// A private helper returning a method call on a foreign import resolves
/// to `object`.
#[test]
fn a_helper_returning_a_method_call_on_an_import_resolves_to_object() {
    let helper = "def _a(s):\n    return json.loads(s)\n\n\n";
    admitted(&format!("{IMPORT}{helper}print(len(_a(\"[1]\")))\n"));
    refused(
        &format!("{IMPORT}{helper}print(_a(\"[1]\") + 1)\n"),
        "T0021",
        RESOLVED_TO_OBJECT,
    );
}

/// A method call on a function-local object binding resolves to `object`:
/// the solver's `MethodCall` arm lifts a concrete `object` receiver term.
#[test]
fn a_method_call_on_a_local_object_resolves_to_object() {
    let helper = "def _k(s):\n    o = json.loads(s)\n    return o.keys()\n\n\n";
    admitted(&format!("{IMPORT}{helper}print(len(_k(\"{{}}\")))\n"));
    refused(
        &format!("{IMPORT}{helper}print(_k(\"{{}}\") + 1)\n"),
        "T0021",
        RESOLVED_TO_OBJECT,
    );
}

/// A call of a function-local binding of a foreign callable resolves to
/// `object`: the solver's bound-callee gate lifts a concrete `object` term.
#[test]
fn a_call_of_a_local_object_binding_resolves_to_object() {
    let helper = "def _g(s):\n    g = json.loads\n    return g(s)\n\n\n";
    admitted(&format!("{IMPORT}{helper}print(len(_g(\"[1]\")))\n"));
    refused(
        &format!("{IMPORT}{helper}print(_g(\"[1]\") + 1)\n"),
        "T0021",
        RESOLVED_TO_OBJECT,
    );
}

/// The three guards Part 1 removes, each in the shape that used to reach
/// it: a local binding and its alias, a `return`, an argument, a
/// function-body read of a module-level object global, and a call of one.
#[test]
fn binding_returning_and_passing_an_object_in_a_function_are_admitted() {
    for body in [
        "def f() -> None:\n    y = json.loads(\"[1]\")\n    z = y\n    print(len(z))\n",
        "def _r():\n    return json.loads(\"[1]\")\n\n\nprint(len(_r()))\n",
        "def _n(o):\n    return len(o)\n\n\ndef f() -> None:\n    _n(json.loads(\"[1]\"))\n",
        "P = json.loads(\"[1]\")\n\n\ndef f() -> None:\n    print(len(P))\n",
        "G = json.loads\n\n\ndef f() -> None:\n    print(len(G(\"[1]\")))\n",
    ] {
        admitted(&format!("{IMPORT}{body}"));
    }
}

/// An object bound through a parameter is still unresolved in the solver
/// (Part 3 of #1333, #1364): the helper's return and callee cannot be
/// inferred, whatever order the definitions appear in.
#[test]
fn an_object_reached_through_a_parameter_is_part_3() {
    refused(
        &format!("{IMPORT}def _k(o):\n    return o.keys()\n\n\nprint(len(_k(json)))\n"),
        "T0021",
        "cannot infer return type of private helper `_k`",
    );
    refused(
        &format!("{IMPORT}def _f(g):\n    return g(\"[1]\")\n\n\nprint(len(_f(json.loads)))\n"),
        "T0021",
        "name `g` is bound to a non-callable value",
    );
}

/// The bound-callee lift still walks every argument, so an argument's own
/// solver diagnostic propagates rather than being masked by the `object`
/// result: an unbound-local read in the call's arguments is reported.
#[test]
fn an_argument_error_in_a_call_of_a_local_object_binding_propagates() {
    let helper = "def _u(s):\n    g = json.loads\n    r = g(t)\n    t = s\n    return r\n\n\n";
    let source = format!("{IMPORT}{helper}print(len(_u(\"[1]\")))\n");
    let diagnostics = crate::check_all(&lower_all_foreign(&source)).expect_err(&source);
    assert_eq!(diagnostics.len(), 1, "{source:?}: {diagnostics:#?}");
    assert_eq!(diagnostics[0].code, "T0021", "{source:?}: {diagnostics:#?}");
    assert!(
        diagnostics[0].message.contains("`t`") && diagnostics[0].message.contains("before"),
        "{source:?}: {diagnostics:#?}"
    );
}

/// A function-local binding keeps the name's type fixed exactly like a
/// module-level one (`binding_tests.rs`): an `object` value into a native
/// name is the ordinary redefinition refusal, also against an annotation,
/// and the other order boxes the native value (#1475).
#[test]
fn a_function_local_type_change_is_refused_like_any_other() {
    let def = "def g() -> None:\n";
    refused(
        &format!("{IMPORT}{def}    x = 1\n    x = json.loads(\"1\")\n"),
        "T0023",
        "cannot assign `object` to `x`, previously inferred as `int`",
    );
    // #1475: a native value is boxed into the `object` name.
    admitted(&format!(
        "{IMPORT}{def}    x = json.loads(\"1\")\n    x = 1\n"
    ));
    refused(
        &format!("{IMPORT}{def}    x: int = json.loads(\"1\")\n"),
        "T0025",
        "cannot assign `object` to `x: int`",
    );
}

/// A function-local binding on only one path leaves the name maybe-bound,
/// which a later read refuses at compile time.
#[test]
fn a_maybe_bound_function_local_object_name_is_refused() {
    refused(
        &format!(
            "{IMPORT}import sys\ndef g() -> None:\n    if len(sys.argv) > 5:\n        x = json.loads(\"1\")\n    y = x\n"
        ),
        "T0041",
        "may not be bound on every path reaching this use",
    );
}

/// An unannotated private method may return the object itself, like a
/// private helper: the solver resolves its return to `object`.
#[test]
fn a_private_method_returning_an_object_resolves_to_object() {
    let class = "class C:\n    def _o(self):\n        return json.loads(\"[1]\")\n\n    \
                 def m(self) -> int:\n        return len(self._o())\n\n\n";
    admitted(&format!("{IMPORT}{class}print(C().m())\n"));
    refused(
        &format!("{IMPORT}{class}print(C()._o() + 1)\n"),
        "T0021",
        RESOLVED_TO_OBJECT,
    );
}
