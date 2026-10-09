//! #1325: binding a CPython object value to a module-level name, and a
//! bare-name `for` over such a name.
//!
//! Kept apart from `foreign/tests.rs` (see #1314); the fixture helpers are
//! copied from `call_tests.rs` because a sibling module cannot reach them.

/// Lowers `source` with every `import` request answered `Foreign`, the way
/// the driver answers an undotted non-`pycc_std` root.
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

const FROM_FORM: &str = "from itertools import product\n";

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

/// Every producer shape binds at module scope, and the bound name is
/// itself an `object` other names can alias.
#[test]
fn a_module_level_binding_is_admitted_for_every_producer() {
    admitted(&format!("{FROM_FORM}x = product(\"ab\")\ny = x\n"));
    admitted("import m\nx = m.f()\n");
    admitted("import m\nx = m.pi\n");
    admitted("import m\nx = m.pi[0]\n");
    admitted("import m\nx = m\n");
    // Rebinding to another `object` is the same type, so no `T0023`.
    admitted(&format!(
        "{FROM_FORM}x = product(\"ab\")\nx = product(\"c\")\n"
    ));
}

/// A bare-name `for` over an `object` name: nested (the inner loop's
/// iterable is the outer loop's `object` target), repeated over the same
/// name, and over a from-imported foreign name directly.
#[test]
fn a_bare_name_for_over_an_object_is_admitted() {
    admitted(&format!(
        "{FROM_FORM}x = product(\"ab\")\nfor t in x:\n    for u in t:\n        pass\nfor t in x:\n    pass\n"
    ));
    admitted("from sys import path\nfor p in path:\n    pass\n");
}

/// The binding keeps the name's type fixed, so an `object` value into a
/// native name is the ordinary redefinition refusal, also against either
/// annotation form; the other order boxes the native value (#1475).
#[test]
fn a_type_change_is_refused_like_any_other() {
    refused(
        &format!("{FROM_FORM}x = 1\nx = product()\n"),
        "T0023",
        "cannot assign `object` to `x`, previously inferred as `int`",
    );
    // #1475: a native value is boxed into the `object` name, as CPython
    // rebinds it.
    admitted(&format!("{FROM_FORM}x = product()\nx = 1\n"));
    refused(
        &format!("{FROM_FORM}x: int\nx = product()\n"),
        "T0026",
        "cannot assign `object` to `x`, previously declared as `x: int`",
    );
    refused(
        &format!("{FROM_FORM}x: int = product()\n"),
        "T0025",
        "cannot assign `object` to `x: int`",
    );
}

/// A binding on only one path leaves the name maybe-bound, which both a
/// read and a loop over it refuse.
#[test]
fn a_maybe_bound_object_name_is_refused_by_a_read_and_a_loop() {
    let prefix = format!("{FROM_FORM}import sys\nif len(sys.argv) > 5:\n    x = product()\n");
    for tail in ["y = x\n", "for t in x:\n    pass\n"] {
        refused(
            &format!("{prefix}{tail}"),
            "T0041",
            "may not be bound on every path reaching this use",
        );
    }
}

/// Since Part 1 of #1333 the binding is admitted in every function-body
/// shape too -- an annotated `def`, a method, an unannotated private helper
/// and a generic function -- and so is a function-body read of a
/// module-level `object` name bound by assignment.
#[test]
fn a_function_body_binding_is_admitted() {
    for source in [
        format!("{FROM_FORM}def g() -> None:\n    x = product()\n"),
        format!("{FROM_FORM}class C:\n    def m(self) -> None:\n        x = product()\n"),
        format!("{FROM_FORM}def _h():\n    x = product()\n    return 1\n"),
        format!(
            "{FROM_FORM}def g[T](a: T) -> T:\n    x = product()\n    return a\n\n\nprint(g(1))\n"
        ),
        format!("{FROM_FORM}x = product()\ndef g() -> None:\n    y = x\n"),
    ] {
        admitted(&source);
    }
}

/// A function-body `for` over a module-level `object` name the module bound
/// by assignment is still refused (Part 2 of #1333, #1363), and a
/// comprehension may not iterate it anywhere.
#[test]
fn iterating_a_bound_object_name_outside_the_admitted_positions_is_refused() {
    let phrase = "using `x`, which is bound to a CPython object";
    for tail in [
        "def g() -> None:\n    for t in x:\n        pass\n",
        "ys = [t for t in x]\n",
    ] {
        refused(
            &format!("{FROM_FORM}x = product()\n{tail}"),
            "I0404",
            phrase,
        );
    }
}

/// `del` keeps its own pre-existing refusal: releasing the object could run
/// a foreign finalizer.
#[test]
fn deleting_a_bound_object_name_keeps_its_own_refusal() {
    refused(
        &format!("{FROM_FORM}x = product()\ndel x\n"),
        "C0001",
        "it holds a CPython object",
    );
}

/// The constraint solver sees the module-level binding and loop alongside
/// an unannotated helper it has to infer, and neither disturbs the other.
#[test]
fn an_unannotated_helper_beside_the_binding_still_solves() {
    admitted(&format!(
        "{FROM_FORM}def _h(a):\n    return a\n\n\nx = product()\nfor t in x:\n    pass\nprint(_h(1))\n"
    ));
}

/// The new bare-name route is module-body only: inside a function body a
/// loop over a module-level foreign import's name -- which #1316 made
/// readable there -- keeps the read refusal in an annotated `def`, a
/// method and an unannotated helper alike, and a bare module is no
/// exception (Part 2 of #1333, #1363).
#[test]
fn a_function_body_bare_name_loop_over_a_foreign_import_is_refused() {
    for (source, name) in [
        (
            "from sys import path\n\n\ndef f() -> None:\n    for p in path:\n        pass\n",
            "path",
        ),
        (
            "import sys\n\n\ndef f() -> None:\n    for q in sys:\n        pass\n",
            "sys",
        ),
        (
            "from sys import path\n\n\nclass C:\n    def m(self) -> None:\n        for p in path:\n            pass\n",
            "path",
        ),
        (
            "from sys import path\n\n\ndef _h():\n    for p in path:\n        pass\n    return 1\n\n\nprint(_h())\n",
            "path",
        ),
    ] {
        refused(
            source,
            "I0404",
            &format!("using `{name}`, which is bound to a CPython object"),
        );
    }
}

/// `check_and_resolve_all_keyed`'s single diagnostic for `source`, after
/// asserting the check phase alone admits it.
fn monomorphization_refusal(source: &str) -> pycc_diag::Diagnostic {
    let hir = lower_all_foreign(source);
    assert!(
        crate::check_all(&hir).is_ok(),
        "{source:?}: the check phase admits it"
    );
    let Err(diagnostics) = crate::check_and_resolve_all_keyed(&hir) else {
        panic!("{source:?}: monomorphization refuses it");
    };
    let [(_, diagnostic)] = diagnostics.as_slice() else {
        panic!("{source:?}: exactly one diagnostic: {diagnostics:?}");
    };
    diagnostic.clone()
}

/// The monomorphization pass re-walks the module body without the foreign
/// names (the gap `foreign/tests.rs`'s
/// `the_monomorphization_pass_walks_a_for_loop_iterable` pins for an
/// attribute iterable), so in a module that also defines a generic function
/// a module-level statement the check phase admits is refused there with
/// `T0021` whenever it *reads* a foreign name: a direct call (pre-existing
/// since #1313), the binding #1325 admits, and an alias of a foreign import.
/// A bare-name loop over a foreign import is not refused by the walk
/// itself: its `ForList` arm's lookup of the iterable misses and types the
/// loop target `Ty::Infer`, so an empty body passes, but a body that applies
/// one of #1026's conversions to the target (`str(t)`) is refused with
/// `C0001`, because the conversion gates admit exactly `Ty::Object`. #1325
/// widens the pre-existing gap rather than opening it; #1101 tracks it.
#[test]
fn a_module_with_a_generic_function_refuses_foreign_reads_in_monomorphization() {
    const GENERIC: &str = "def g[T](a: T) -> T:\n    return a\n\n\n";
    for (body, message) in [
        ("product(\"ab\")\n", "call to undefined function `product`"),
        (
            "x = product(\"ab\")\n",
            "call to undefined function `product`",
        ),
    ] {
        let diagnostic =
            monomorphization_refusal(&format!("{FROM_FORM}{GENERIC}{body}print(g(1))\n"));
        assert_eq!(diagnostic.code, "T0021", "{diagnostic:?}");
        assert!(diagnostic.message.contains(message), "{diagnostic:?}");
    }
    let diagnostic = monomorphization_refusal(&format!(
        "from sys import path\n{GENERIC}y = path\nprint(g(1))\n"
    ));
    assert_eq!(diagnostic.code, "T0021", "{diagnostic:?}");
    assert!(
        diagnostic.message.contains("name `path` is not defined"),
        "{diagnostic:?}"
    );
    let hir = lower_all_foreign(&format!(
        "from sys import path\n{GENERIC}for t in path:\n    pass\nprint(g(1))\n"
    ));
    crate::check_and_resolve_all_keyed(&hir).expect("a bare-name loop passes monomorphization");
    let diagnostic = monomorphization_refusal(&format!(
        "from sys import path\n{GENERIC}for t in path:\n    print(str(t))\nprint(g(1))\n"
    ));
    assert_eq!(diagnostic.code, "C0001", "{diagnostic:?}");
    assert!(
        diagnostic.message.contains("call to builtin `str`"),
        "{diagnostic:?}"
    );
}
