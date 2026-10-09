//! A plain dotted foreign import (#1381, Part 3 of #1138): `import a.b`
//! binds the root `a`, `import a.b as c` binds the leaf `a.b` to `c`, and
//! the shadow rules compare the module each binding binds.

use super::*;
use crate::{FromImport, HirItem, HirStmt, foreign_binds_root, foreign_bound_module};

/// Lowers `source` the way the driver answers it for a program with no
/// project modules: every absolute plain `import` and every absolute
/// `from ... import` that `pycc_std` does not resolve is foreign.
fn lower_with_driver(source: &str) -> Result<LoweredModule, Vec<Diagnostic>> {
    let parsed = parse(source);
    let mut resolved = ResolvedImports::default();
    for request in project_import_requests(&parsed) {
        if request.level == 0 {
            resolved.insert(request.span, ResolvedImport::Foreign);
        }
    }
    lower_module(&parsed, &resolved, None)
}

fn lower_ok(source: &str) -> LoweredModule {
    lower_with_driver(source)
        .unwrap_or_else(|diagnostics| panic!("{source:?} must lower: {diagnostics:#?}"))
}

fn only_message(source: &str) -> String {
    let diagnostics = lower_with_driver(source).expect_err("fixture must fail to lower");
    assert_eq!(diagnostics.len(), 1, "{source:?}: {diagnostics:#?}");
    diagnostics[0].message.clone()
}

/// `(local_name, module_path, site)` of every foreign binding, in order.
fn foreign(module: &LoweredModule) -> Vec<(&str, &str, ForeignImportSite)> {
    module
        .hir
        .imports
        .iter()
        .filter_map(|binding| match binding {
            ImportBinding::Foreign {
                local_name,
                module_path,
                site,
                ..
            } => Some((local_name.as_str(), module_path.as_str(), *site)),
            _ => None,
        })
        .collect()
}

#[test]
fn an_unaliased_dotted_import_binds_the_root_and_an_aliased_one_the_leaf() {
    let lowered = lower_ok("import xml.dom.minidom\nimport re._parser as sre_parse\n");
    assert_eq!(
        foreign(&lowered),
        vec![
            ("xml", "xml.dom.minidom", ForeignImportSite::Item(0)),
            ("sre_parse", "re._parser", ForeignImportSite::Item(0)),
        ]
    );
}

#[test]
fn a_nested_dotted_import_lowers_to_a_foreign_import_node_and_keeps_optional() {
    let source =
        "try:\n    import xml.dom\n    import email.utils as eu\nexcept ImportError:\n    pass\n";
    let lowered = lower_ok(source);
    assert_eq!(
        foreign(&lowered),
        vec![
            (
                "xml",
                "xml.dom",
                ForeignImportSite::Block { optional: true }
            ),
            (
                "eu",
                "email.utils",
                ForeignImportSite::Block { optional: true }
            ),
        ]
    );
    let HirItem::TopLevelStmt(HirStmt::Try { body, .. }) = &lowered.hir.items[0] else {
        panic!("a try statement: {:#?}", lowered.hir.items);
    };
    assert!(matches!(
        &body[0],
        HirStmt::ForeignImport { bindings, .. }
            if bindings == &vec![("xml".to_string(), "xml.dom".to_string(), None)]
    ));
}

#[test]
fn a_dotted_import_under_type_checking_binds_nothing() {
    let lowered =
        lower_ok("from typing import TYPE_CHECKING\nif TYPE_CHECKING:\n    import xml.dom\n");
    assert!(foreign(&lowered).is_empty(), "{:#?}", lowered.hir.imports);
}

/// `import a.b as a` would bind the leaf under the root's own name, which
/// [`foreign_binds_root`] reads as the root binding of `import a.b`.
#[test]
fn binding_a_dotted_import_to_its_own_root_name_is_refused() {
    for source in [
        "import xml.dom as xml\n",
        "if c:\n    import xml.dom as xml\n",
    ] {
        assert_eq!(
            only_message(source),
            "binding the CPython module `xml.dom` to `xml`, the name of its own top-level \
             package, is not supported yet",
            "{source:?}"
        );
    }
}

/// The unaliased form binds its root, so a root pycc resolves by its
/// spelling is refused exactly as an alias spelled that way is.
#[test]
fn an_unaliased_dotted_import_whose_root_pycc_resolves_by_spelling_is_refused() {
    for root in ["typing", "math", "range"] {
        assert_eq!(
            only_message(&format!("import {root}.sub\n")),
            format!(
                "binding the CPython module `{root}.sub` to `{root}`, a name pycc resolves by \
                 its spelling (a Python builtin, a stdlib module, or a typing, decorator or \
                 base-class marker), is not supported yet"
            )
        );
    }
    // The aliased form binds the leaf to an ordinary name.
    lower_ok("import typing.sub as ts\n");
}

/// Imports that bind one module to one name are identical pairs even when
/// they import different submodules; each still keeps its own binding, so
/// each runs its own import.
#[test]
fn imports_binding_the_same_root_are_not_a_shadow() {
    for source in [
        "import os\nimport os.path\n",
        "import os.path\nimport os\n",
        "import xml.dom\nimport xml.sax\n",
        "import xml.dom, xml.sax\n",
        "if c:\n    import xml.dom\nelse:\n    import xml.sax\n",
    ] {
        let lowered = lower_ok(source);
        assert_eq!(foreign(&lowered).len(), 2, "{source:?}");
    }
}

#[test]
fn imports_binding_different_modules_to_one_name_are_a_shadow() {
    for source in [
        "import xml.dom as x\nimport xml.sax as x\n",
        "import os.path as p\nimport os as p\n",
        "import os.path\nfrom json import os\n",
    ] {
        let message = only_message(source);
        assert!(
            message.contains("shadowing a foreign import"),
            "{source:?}: {message}"
        );
    }
}

/// #1485: the optional-dependency fallback groups on the bound root.
#[test]
fn the_optional_dependency_fallback_groups_on_the_root() {
    let lowered = lower_ok("try:\n    import xml.dom\nexcept ImportError:\n    xml = None\n");
    assert_eq!(
        foreign(&lowered),
        vec![(
            "xml",
            "xml.dom",
            ForeignImportSite::Block { optional: true }
        )]
    );
}

/// `del xml` after `import xml.dom` deletes the same binding `del xml`
/// after `import xml` does.
#[test]
fn deleting_the_root_of_a_dotted_import_matches_the_undotted_form() {
    let dotted = lower_with_driver("import xml.dom\ndel xml\n");
    let undotted = lower_with_driver("import xml\ndel xml\n");
    match (dotted, undotted) {
        (Ok(dotted), Ok(undotted)) => assert_eq!(
            dotted.hir.items, undotted.hir.items,
            "both forms delete the same binding"
        ),
        (Err(dotted), Err(undotted)) => assert_eq!(
            dotted.iter().map(|d| &d.message).collect::<Vec<_>>(),
            undotted.iter().map(|d| &d.message).collect::<Vec<_>>()
        ),
        (dotted, undotted) => panic!("the forms diverge: {dotted:#?} vs {undotted:#?}"),
    }
}

#[test]
fn the_predicates_distinguish_root_leaf_and_from_bindings() {
    let from = FromImport {
        name: "b".to_string(),
        fromlist: vec!["b".to_string()],
        index: 0,
        level: 0,
    };
    assert!(foreign_binds_root("a", "a.b", None));
    assert!(!foreign_binds_root("c", "a.b", None));
    assert!(!foreign_binds_root("a", "a", None));
    assert!(!foreign_binds_root("a", "a.b", Some(&from)));
    assert_eq!(foreign_bound_module("a", "a.b.c", None), "a");
    assert_eq!(foreign_bound_module("c", "a.b.c", None), "a.b.c");
    assert_eq!(foreign_bound_module("a", "a", None), "a");
}
