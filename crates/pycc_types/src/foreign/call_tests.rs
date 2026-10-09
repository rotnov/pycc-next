//! #1313: a direct call of a CPython object (`product("ab")` after
//! `from itertools import product`) in the module body, below its import.
//!
//! Kept apart from `foreign/tests.rs` (see #1314); the few fixture helpers
//! below are copied from there because a sibling module cannot reach them.

/// Lowers `source` with every `import` request answered `Foreign`, the way
/// the driver answers an undotted non-`pycc_std` root. Both source shapes
/// the issue names go through the same path: `import itertools` binds a
/// module-form foreign object, `from itertools import product` a from-form
/// one.
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

const FROM_FORM: &str = "from itertools import product\n";
const MODULE_FORM: &str = "import itertools\n";

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

/// Every packable scalar argument type is admitted in both import shapes,
/// and the call's `object` result feeds a consumer that accepts one.
#[test]
fn a_module_body_call_with_scalar_arguments_is_admitted_in_both_shapes() {
    for (import, callee) in [(FROM_FORM, "product"), (MODULE_FORM, "itertools")] {
        admitted(&format!("{import}{callee}(1, 2.5, True, \"ab\")\n"));
        admitted(&format!("{import}s = str({callee}(\"ab\"))\n"));
    }
}

#[test]
fn a_zero_argument_call_is_admitted() {
    admitted(&format!("{FROM_FORM}product()\n"));
}

#[test]
fn a_non_packable_argument_is_refused() {
    refused(
        &format!("{FROM_FORM}product([1])\n"),
        "I0404",
        "passing a `list[int]` argument to a CPython object's call",
    );
    // Part 8 of #1371 admits a `None` argument (`keyword_call_tests.rs`).
}

/// Part 2a of #1371: a second CPython object is packable
/// (`pycc_ext_obj_pack_object`), so passing one as an argument is admitted.
#[test]
fn an_object_argument_is_admitted() {
    admitted(&format!("{FROM_FORM}product(product)\n"));
    admitted(&format!("{FROM_FORM}product(1, product, \"ab\")\n"));
}

/// #1316: a direct call of a module-level foreign name is admitted in an
/// annotated `def`, an unannotated private helper and a generic function
/// body alike -- the type layer lifts the function-body read refusal for
/// a name the module binds to a CPython object.
#[test]
fn a_call_in_any_function_body_is_admitted() {
    for body in [
        "def f() -> None:\n    product(\"ab\")\n",
        "def _helper():\n    product(\"ab\")\n",
        "def g[T](x: T) -> T:\n    product(\"ab\")\n    return x\n",
    ] {
        admitted(&format!("{FROM_FORM}{body}"));
    }
}

/// A function-local rebinding shadows the module-level foreign name, so
/// the lift does not reach it: the local is an ordinary `int`.
#[test]
fn a_local_rebinding_shadows_the_foreign_callee() {
    refused(
        &format!("{FROM_FORM}def f() -> None:\n    product = 1\n    product(\"ab\")\n"),
        "T0021",
        "product",
    );
}

/// Since Part 1 of #1333 a CPython object may be passed into a
/// pycc-compiled function, at module scope or inside a function body. A
/// generic function is the one callee that still refuses it, because a type
/// variable bound to `object` would need an instantiation that carries the
/// reference.
#[test]
fn passing_a_foreign_object_to_a_pycc_function_is_admitted() {
    let helper = "def _g(x) -> None:\n    pass\n";
    for tail in ["_g(product)\n", "def f() -> None:\n    _g(product)\n"] {
        admitted(&format!("{FROM_FORM}{helper}{tail}"));
    }
    refused(
        &format!("{FROM_FORM}def ident[T](x: T) -> T:\n    return x\n\n\nident(product)\n"),
        "I0404",
        "passing a CPython object to a generic function",
    );
}

/// Above its import the name is not bound yet, so the call takes the
/// undefined-callee path.
#[test]
fn a_call_above_the_import_is_an_undefined_function() {
    refused(
        &format!("product(\"ab\")\n{FROM_FORM}"),
        "T0021",
        "call to undefined function `product`",
    );
}

#[test]
fn a_call_after_a_block_that_may_skip_the_import_is_t0041() {
    refused(
        "c = True\nif c:\n    import itertools\nitertools(\"a\")\n",
        "T0041",
        "may not be bound",
    );
    admitted(
        "c = True\nif c:\n    import itertools\nelse:\n    import itertools\nitertools(\"a\")\n",
    );
}

/// Part 9 of #1371: a call's `object` result is raised the way CPython
/// raises it; `foreign/raise_tests.rs` owns the rest of `raise o`.
#[test]
fn raising_a_call_result_is_admitted() {
    admitted(&format!("{FROM_FORM}raise product(\"a\")\n"));
}

/// The result has no consumer beyond the ones every object producer has:
/// binding it to a module-level name is admitted (#1325), and so are
/// printing it and interpolating it into an f-string (#1340).
#[test]
fn binding_and_printing_a_call_result_are_admitted() {
    admitted(&format!("{FROM_FORM}x = product(\"ab\")\n"));
    admitted(&format!("{FROM_FORM}print(product(\"ab\"))\n"));
    admitted(&format!("{FROM_FORM}print(f\"<{{product(\"ab\")}}>\")\n"));
}

/// A `for` loop target bound to a CPython object is callable in the loop
/// body: the admission is by type, not by how the name was bound.
#[test]
fn a_call_of_a_foreign_loop_target_is_admitted() {
    admitted("import sys\nfor f in sys.meta_path:\n    f()\n");
}
