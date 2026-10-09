//! `lower_module` behaviour for a base class that names a CPython builtin
//! type (Part 1 of #1283, #1318): the builtin-base `C0001` joins D-219's
//! cascade suppression in both directions, a user class of the same name
//! shadows the builtin, and a name the module rebinds earlier keeps the
//! unknown-class message. The message builder's own round trip is in
//! `cascade_classifier`.

use super::*;

/// The single diagnostic `source` reports.
fn only_diagnostic(source: &str) -> Diagnostic {
    let mut diagnostics = lower_all_err(source);
    assert_eq!(diagnostics.len(), 1, "{source}: {diagnostics:#?}");
    diagnostics.remove(0)
}

#[test]
fn a_builtin_type_base_reports_the_builtin_base_message() {
    let source = "class fzset(frozenset):\n    pass\n";
    assert_c0001(
        &only_diagnostic(source),
        &builtin_base_message("fzset", "frozenset"),
        span_of(source, "class fzset(frozenset):\n    pass", 0),
    );
}

#[test]
fn a_failed_user_class_named_like_a_builtin_silences_its_subclass() {
    // The failed `class frozenset:` poisons `frozenset`, so the subclass's
    // builtin-base `C0001` is a cascade and is suppressed: only the root
    // cause is reported.
    let source = "class frozenset:\n    async def m(self) -> None:\n        pass\n\n\n\
                  class F(frozenset):\n    pass\n";
    let diagnostic = only_diagnostic(source);
    assert_eq!(diagnostic.code, "C0001");
    assert_eq!(diagnostic.message, "an async method is not supported yet");
}

#[test]
fn a_builtin_base_failure_silences_a_later_annotation_naming_the_class() {
    // The lark shape: `class fzset(frozenset)` fails and poisons `fzset`,
    // so `def g(x: fzset)` is a cascade and reports nothing of its own.
    let source = "class fzset(frozenset):\n    def __repr__(self) -> str:\n        return \"x\"\n\n\n\
                  def g(x: fzset) -> int:\n    return 1\n";
    assert_eq!(
        only_diagnostic(source).message,
        builtin_base_message("fzset", "frozenset")
    );
}

#[test]
fn a_user_class_named_like_a_builtin_shadows_it_as_a_base() {
    let hir = lower_all_ok("class frozenset:\n    x: int = 1\n\n\nclass F(frozenset):\n    pass\n");
    let (_, derived) = hir
        .class_defs
        .iter()
        .find(|(name, _)| name == "F")
        .expect("`F` lowers");
    assert_eq!(derived.bases, vec!["frozenset".to_string()]);
}

#[test]
fn an_unrelated_stdlib_import_does_not_count_as_a_rebinding() {
    // `import math` puts a binding in the import table, so the rebinding
    // check walks it and finds no match.
    let source = "import math\n\n\nclass L(list):\n    pass\n";
    assert_eq!(
        only_diagnostic(source).message,
        builtin_base_message("L", "list")
    );
}

// -- A name the module rebinds earlier keeps the unknown-class message ------
//
// One test per source `class::mro::module_rebinds` consults.

#[test]
fn a_type_alias_named_like_a_builtin_keeps_the_unknown_class_message() {
    assert_eq!(
        only_diagnostic("type list = int\n\n\nclass A(list):\n    pass\n").message,
        unknown_base_message("A", "list")
    );
}

#[test]
fn a_foreign_import_named_like_a_builtin_keeps_the_unknown_class_message() {
    // The driver answers `import list` as a foreign module, so the name is
    // bound to a CPython module object, not to the builtin type.
    let source = "import list\n\n\nclass G(list):\n    pass\n";
    let module = parse(source);
    let mut resolved = ResolvedImports::default();
    resolved.insert(span_of(source, "list", 0), crate::ResolvedImport::Foreign);
    let diagnostics =
        lower_module(&module, &resolved, None).expect_err("the class must fail to lower");
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    assert_eq!(diagnostics[0].message, unknown_base_message("G", "list"));
}

#[test]
fn a_function_named_like_a_builtin_keeps_the_unknown_class_message() {
    assert_eq!(
        only_diagnostic(
            "def frozenset() -> int:\n    return 1\n\n\nclass F(frozenset):\n    pass\n"
        )
        .message,
        unknown_base_message("F", "frozenset")
    );
}

#[test]
fn a_top_level_binding_named_like_a_builtin_keeps_the_unknown_class_message() {
    assert_eq!(
        only_diagnostic("frozenset = 1\n\n\nclass C(frozenset):\n    pass\n").message,
        unknown_base_message("C", "frozenset")
    );
}

// -- A failed import of a builtin-shaped name silences the subclass ----------

/// Every diagnostic `source` reports when the driver answers each of its
/// imports as a foreign (CPython-backed) module.
fn all_foreign_diagnostics(source: &str) -> Vec<Diagnostic> {
    let module = parse(source);
    let mut resolved = ResolvedImports::default();
    for request in crate::project_import_requests(&module) {
        resolved.insert(request.span, crate::ResolvedImport::Foreign);
    }
    lower_module(&module, &resolved, None).expect_err("the module must fail to lower")
}

#[test]
fn a_failed_aliased_import_of_a_builtin_name_silences_its_subclass() {
    // `import json as list` fails (binding a foreign module to a name pycc
    // resolves by spelling) and poisons `list`, so the class's builtin-base
    // `C0001` is a cascade: only the import's diagnostic is reported.
    let diagnostics =
        all_foreign_diagnostics("import json as list\n\n\nclass A(list):\n    pass\n");
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    assert!(
        diagnostics[0]
            .message
            .starts_with("binding the CPython module `json` to `list`"),
        "{diagnostics:#?}"
    );
}

#[test]
fn a_failed_from_import_of_a_builtin_name_silences_its_subclass() {
    let diagnostics =
        all_foreign_diagnostics("from json import frozenset\n\n\nclass E(frozenset):\n    pass\n");
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    assert!(
        diagnostics[0]
            .message
            .starts_with("binding the CPython object `json.frozenset` to `frozenset`"),
        "{diagnostics:#?}"
    );
}

#[test]
fn a_failed_def_named_like_a_builtin_leaves_the_builtin_base_message() {
    // An unannotated `def frozenset()` fails (`T0001`) and, being a `def`,
    // poisons nothing and is not in the item list, so the rebinding check
    // cannot see it: the class reports the builtin-type text beside it.
    let diagnostics =
        lower_all_err("def frozenset():\n    return 1\n\n\nclass F(frozenset):\n    pass\n");
    assert_eq!(diagnostics.len(), 2, "{diagnostics:#?}");
    assert_eq!(diagnostics[0].code, "T0001");
    assert_eq!(
        diagnostics[1].message,
        builtin_base_message("F", "frozenset")
    );
}
