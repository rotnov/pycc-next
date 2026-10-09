//! A CPython-backed `import` nested in a module-level `if`/`try` block
//! (#1291): which nested imports lower to `HirStmt::ForeignImport`, which
//! keep a diagnostic, and what the driver is asked to answer.

use super::*;
use crate::{HirItem, HirStmt};

/// Lowers `source` the way the driver answers it: a request for a name in
/// `found` is a project module, any other undotted plain `import` is
/// foreign, and everything else is left unanswered.
fn lower_with_driver(source: &str, found: &[&str]) -> Result<LoweredModule, Vec<Diagnostic>> {
    let parsed = parse(source);
    let mut resolved = ResolvedImports::default();
    for request in project_import_requests(&parsed) {
        let Some(module) = request.module.as_deref() else {
            continue;
        };
        if found.contains(&module) {
            resolved.insert(request.span, ResolvedImport::Found);
        } else if request.names.is_empty() && request.level == 0 && !module.contains('.') {
            resolved.insert(request.span, ResolvedImport::Foreign);
        }
    }
    lower_module(&parsed, &resolved, None)
}

fn lower_ok(source: &str) -> LoweredModule {
    lower_with_driver(source, &[])
        .unwrap_or_else(|diagnostics| panic!("{source:?} must lower: {diagnostics:#?}"))
}

fn errors(source: &str, found: &[&str]) -> Vec<Diagnostic> {
    lower_with_driver(source, found).expect_err("fixture must fail to lower")
}

fn only_message(source: &str) -> String {
    let diagnostics = errors(source, &[]);
    assert_eq!(diagnostics.len(), 1, "{source:?}: {diagnostics:#?}");
    diagnostics[0].message.clone()
}

/// Every `ForeignImport` node reached through `if`/`try` nesting, in
/// source order.
fn nested_nodes(module: &LoweredModule) -> Vec<Vec<Binding>> {
    fn walk(stmts: &[HirStmt], out: &mut Vec<Vec<Binding>>) {
        for stmt in stmts {
            match stmt {
                HirStmt::ForeignImport { bindings, .. } => out.push(bindings.clone()),
                HirStmt::If { body, orelse, .. } => {
                    walk(body, out);
                    walk(orelse, out);
                }
                HirStmt::Try {
                    body,
                    handlers,
                    orelse,
                    finalbody,
                }
                | HirStmt::TryStar {
                    body,
                    handlers,
                    orelse,
                    finalbody,
                } => {
                    walk(body, out);
                    for handler in handlers {
                        walk(&handler.body, out);
                    }
                    walk(orelse, out);
                    walk(finalbody, out);
                }
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    for item in &module.hir.items {
        if let HirItem::TopLevelStmt(stmt) = item {
            walk(std::slice::from_ref(stmt), &mut out);
        }
    }
    out
}

fn foreign_sites(module: &LoweredModule) -> Vec<(&str, &str, ForeignImportSite)> {
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

/// One `HirStmt::ForeignImport` binding: local name, module path, and the
/// from-import it binds a name of, if any.
type Binding = (String, String, Option<crate::FromImport>);

fn pair(local: &str, module: &str) -> Vec<Binding> {
    vec![(local.to_string(), module.to_string(), None)]
}

const BLOCK_TEXT: &str = "an `import` inside a block body";

#[test]
fn an_import_in_an_if_body_lowers_to_a_foreign_import_node() {
    let source = "if c:\n    import colorsys\n";
    let lowered = lower_ok(source);
    let statement = Span::new(10, source.trim_end().len() as u32);
    assert_eq!(
        lowered.hir.items,
        vec![HirItem::TopLevelStmt(HirStmt::If {
            test: crate::HirExpr::Name("c".to_string()),
            body: vec![HirStmt::ForeignImport {
                bindings: pair("colorsys", "colorsys"),
                span: statement,
            }],
            orelse: vec![],
        })]
    );
    assert_eq!(
        lowered.hir.imports,
        vec![ImportBinding::Foreign {
            local_name: "colorsys".to_string(),
            module_path: "colorsys".to_string(),
            from: None,
            site: ForeignImportSite::Block { optional: false },
            span: statement,
        }]
    );
}

#[test]
fn every_try_and_elif_position_lowers_a_nested_import() {
    // The flag is #1290's `optional`: only a `try` body whose handler
    // catches a failed import guards it.
    for (source, optional) in [
        (
            "try:\n    import colorsys\nexcept Exception:\n    pass\n",
            true,
        ),
        (
            "try:\n    pass\nexcept Exception:\n    import colorsys\n",
            false,
        ),
        (
            "try:\n    pass\nexcept Exception:\n    pass\nelse:\n    import colorsys\n",
            false,
        ),
        ("try:\n    pass\nfinally:\n    import colorsys\n", false),
        (
            "try:\n    pass\nexcept* ValueError:\n    import colorsys\n",
            false,
        ),
        (
            "try:\n    if c:\n        import colorsys\nexcept Exception:\n    pass\n",
            true,
        ),
        ("if c:\n    pass\nelif d:\n    import colorsys\n", false),
    ] {
        let lowered = lower_ok(source);
        assert_eq!(
            nested_nodes(&lowered),
            vec![pair("colorsys", "colorsys")],
            "{source:?}"
        );
        assert_eq!(
            foreign_sites(&lowered),
            vec![(
                "colorsys",
                "colorsys",
                ForeignImportSite::Block { optional }
            )],
            "{source:?}"
        );
    }
}

#[test]
fn a_type_checking_body_binds_nothing_and_its_else_is_admitted() {
    let lowered = lower_ok(
        "from typing import TYPE_CHECKING\nif TYPE_CHECKING:\n    import colorsys\nelse:\n    \
         import json\n",
    );
    assert_eq!(nested_nodes(&lowered), vec![pair("json", "json")]);
    assert_eq!(
        foreign_sites(&lowered),
        vec![("json", "json", ForeignImportSite::Block { optional: false })]
    );

    // The same guard as an `elif` test.
    let lowered = lower_ok(
        "from typing import TYPE_CHECKING\nif c:\n    pass\nelif TYPE_CHECKING:\n    \
         import colorsys\n",
    );
    assert!(foreign_sites(&lowered).is_empty());
    assert!(nested_nodes(&lowered).is_empty());
}

#[test]
fn an_aliased_foreign_import_binds_the_alias_at_top_level_and_nested() {
    let lowered = lower_ok("import colorsys as c\n");
    assert_eq!(
        foreign_sites(&lowered),
        vec![("c", "colorsys", ForeignImportSite::Item(0))]
    );
    let lowered = lower_ok("if x:\n    import colorsys as c\n");
    assert_eq!(
        foreign_sites(&lowered),
        vec![(
            "c",
            "colorsys",
            ForeignImportSite::Block { optional: false }
        )]
    );
    assert_eq!(nested_nodes(&lowered), vec![pair("c", "colorsys")]);
}

#[test]
fn nested_imports_that_are_not_all_foreign_keep_a_diagnostic() {
    for source in [
        "if c:\n    import math\n",
        "if c:\n    import colorsys, math\n",
        "if c:\n    from colorsys import hls_to_rgb\n",
    ] {
        let message = only_message(source);
        assert!(message.contains(BLOCK_TEXT), "{source:?}: {message}");
    }
    assert_eq!(
        only_message("if c:\n    import os.path\n"),
        "import of module `os.path` is not supported yet"
    );
    let diagnostics = errors("if c:\n    import pkg\n", &["pkg"]);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    assert!(
        diagnostics[0].message.contains("module namespace"),
        "{}",
        diagnostics[0].message
    );
}

#[test]
fn a_loop_body_import_keeps_the_block_diagnostic() {
    // A `with` statement is refused as a whole before its body is lowered.
    for source in [
        "for i in range(3):\n    import colorsys\n",
        "while c:\n    import colorsys\n",
    ] {
        let message = only_message(source);
        assert!(message.contains(BLOCK_TEXT), "{source:?}: {message}");
    }
}

#[test]
fn a_failed_block_reports_in_source_order_and_binds_nothing() {
    // The block fails after its import lowered: the unsupported statement
    // is the only diagnostic, and the truncated bindings mean the later
    // top-level `colorsys = 1` is not reported as shadowing anything.
    let diagnostics = errors(
        "if c:\n    import colorsys\n    lambda: 1\ncolorsys = 1\n",
        &[],
    );
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    assert!(
        !diagnostics[0].message.contains("import"),
        "{}",
        diagnostics[0].message
    );

    // An earlier unsupported statement is reported, not the later import's
    // deferred diagnostic.
    let message = only_message("if c:\n    lambda: 1\n    import os.path\n");
    assert!(!message.contains("os.path"), "{message}");

    // The import comes first: its own deferred diagnostic replaces the
    // catch-all block text.
    assert_eq!(
        only_message("if c:\n    import os.path\n    lambda: 1\n"),
        "import of module `os.path` is not supported yet"
    );
}

#[test]
fn an_alias_shadowing_a_resolved_spelling_is_refused() {
    // One name per category and per marker family (#1291 review): a stdlib
    // module, a builtin function, class, exception and scalar, and each
    // marker family -- the `TYPE_CHECKING` fold, the base-class, enum,
    // decorator and dataclass markers, and the import-free annotation
    // names.
    for alias in [
        "typing",
        "math",
        "range",
        "super",
        "property",
        "staticmethod",
        "classmethod",
        "ValueError",
        "int",
        "TYPE_CHECKING",
        "Enum",
        "StrEnum",
        "Protocol",
        "ABC",
        "auto",
        "override",
        "abstractmethod",
        "dataclass",
        "runtime_checkable",
        "dataclass_transform",
        "field",
        "ClassVar",
        "Final",
        "Self",
        "Annotated",
        "Any",
        "TypeAlias",
        "NDArray",
        "ndarray",
        // #1378: the legacy `typing` container aliases.
        "Dict",
        "FrozenSet",
        "List",
        "Set",
        "Tuple",
    ] {
        for source in [
            format!("import colorsys as {alias}\n"),
            format!("if c:\n    import colorsys as {alias}\n"),
        ] {
            let message = only_message(&source);
            assert_eq!(
                message,
                format!(
                    "binding the CPython module `colorsys` to `{alias}`, a name pycc resolves \
                     by its spelling (a Python builtin, a stdlib module, or a typing, decorator \
                     or base-class marker), is not supported yet"
                ),
                "{source:?}"
            );
        }
    }
}

/// The optional-dependency idiom is admitted since #1485: the `except` arm's
/// `colorsys = None` is the import's fallback, not a second definition.
/// The other fallback shapes are in `tests/fallback.rs`.
#[test]
fn the_optional_dependency_idiom_is_admitted() {
    let lowered = lower_ok("try:\n    import colorsys\nexcept Exception:\n    colorsys = None\n");
    assert_eq!(nested_nodes(&lowered), vec![pair("colorsys", "colorsys")]);
    assert!(
        !lowered
            .definition_spans
            .iter()
            .any(|(name, _)| name == "colorsys")
    );
}

#[test]
fn shadowing_a_nested_import_follows_the_top_level_rule() {
    let message = only_message("if c:\n    import colorsys\ncolorsys = 1\n");
    assert!(message.contains("shadowing a foreign import"), "{message}");

    let lowered = lower_ok("if c:\n    import colorsys\nelse:\n    import colorsys\n");
    assert_eq!(
        nested_nodes(&lowered),
        vec![pair("colorsys", "colorsys"), pair("colorsys", "colorsys")]
    );

    let message = only_message("import colorsys as x\nimport json as x\n");
    assert!(message.contains("shadowing a foreign import"), "{message}");

    lower_ok("import numpy\nimport numpy\n");
}

/// A nested foreign import that names a class the module already defines
/// is refused exactly like the top-level form (#1291 review), and the
/// diagnostic points at the nested `import` statement, not at the block.
/// The seeded builtin exception classes count as defined classes, which
/// only the unaliased form can reach: an alias spelled `ValueError` is a
/// builtin, refused first by the alias guard.
#[test]
fn a_nested_import_colliding_with_a_class_is_refused_at_its_own_statement() {
    let source = "class Point:\n    pass\nif c:\n    x = 1\n    import colorsys as Point\n";
    let diagnostics = errors(source, &[]);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    assert_eq!(
        diagnostics[0].message,
        "import `Point` collides with a class of the same name already defined in this module"
    );
    let start = u32::try_from(source.find("import colorsys").unwrap()).unwrap();
    let end = u32::try_from(source.len() - 1).unwrap();
    assert_eq!(diagnostics[0].span, Some(Span::new(start, end)));

    // The exception classes are seeded only in a module that names one.
    let handler = "try:\n    pass\nexcept ValueError:\n    pass\n";
    for import in ["import ValueError\n", "if c:\n    import ValueError\n"] {
        let source = format!("{import}{handler}");
        let source = source.as_str();
        assert_eq!(
            only_message(source),
            "import `ValueError` collides with a class of the same name already defined in \
             this module",
            "{source:?}"
        );
    }
}

#[test]
fn nested_and_aliased_imports_are_requested_under_their_alias_spans() {
    let source = "if c:\n    import a as x\ntry:\n    import b\nexcept Exception:\n    \
                  from d import e\n";
    let requests = project_import_requests(&parse(source));
    let modules: Vec<&str> = requests
        .iter()
        .filter_map(|request| request.module.as_deref())
        .collect();
    // The handler's `from d import e` is requested too since #1383, under
    // its statement's span.
    assert_eq!(modules, vec!["a", "b", "d"]);
    let alias_start = |needle: &str| source.find(needle).expect("needle") as u32;
    assert_eq!(requests[0].span.start, alias_start("a as x"));
    assert_eq!(requests[1].span.start, alias_start("b\n"));
    assert_eq!(requests[2].span.start, alias_start("from d"));
    assert_eq!(requests[2].names, vec!["e".to_string()]);
    assert!(
        requests.iter().all(|request| request.nested),
        "{requests:#?}"
    );
}

#[test]
fn function_and_loop_body_imports_are_not_requested_but_type_checking_ones_are() {
    let requests = project_import_requests(&parse(
        "def f() -> None:\n    import a\nfor i in range(3):\n    import b\nwhile c:\n    \
         import g\n",
    ));
    assert!(requests.is_empty(), "{requests:#?}");

    let requests = project_import_requests(&parse(
        "from typing import TYPE_CHECKING\nif TYPE_CHECKING:\n    import a\nelif c:\n    \
         import b\nelse:\n    import d\n",
    ));
    let modules: Vec<&str> = requests
        .iter()
        .filter_map(|request| request.module.as_deref())
        .collect();
    assert_eq!(modules, vec!["a", "b", "d"]);
}

/// #1290: which `try` shapes make a nested import optional.
#[test]
fn a_try_whose_handler_catches_a_failed_import_makes_its_body_import_optional() {
    for (source, optional) in [
        // Each qualifying handler shape.
        (
            "try:\n    import colorsys\nexcept ImportError:\n    pass\n",
            true,
        ),
        (
            "try:\n    import colorsys\nexcept ModuleNotFoundError:\n    pass\n",
            true,
        ),
        ("try:\n    import colorsys\nexcept:\n    pass\n", true),
        (
            "try:\n    import colorsys\nexcept (ValueError, ImportError):\n    pass\n",
            true,
        ),
        (
            "try:\n    import colorsys\nexcept* ImportError:\n    pass\n",
            true,
        ),
        // A later handler qualifies as well as the first.
        (
            "try:\n    import colorsys\nexcept ValueError:\n    pass\nexcept ImportError:\n    \
             pass\n",
            true,
        ),
        // A non-matching handler, and a `try` with only `finally`.
        (
            "try:\n    import colorsys\nexcept ValueError:\n    pass\n",
            false,
        ),
        (
            "try:\n    import colorsys\nexcept (ValueError, KeyError):\n    pass\n",
            false,
        ),
        ("try:\n    import colorsys\nfinally:\n    pass\n", false),
        // The guard reaches through a nested `if`/`try`, and an inner
        // non-matching `try` does not remove an outer guard.
        (
            "try:\n    try:\n        import colorsys\n    except ValueError:\n        pass\nexcept \
             ImportError:\n    pass\n",
            true,
        ),
        (
            "if c:\n    try:\n        import colorsys\n    except ImportError:\n        pass\n",
            true,
        ),
        // A handler body of a guarding `try` nested inside an outer guard
        // inherits the outer guard.
        (
            "try:\n    try:\n        pass\n    except ValueError:\n        import colorsys\nexcept \
             ImportError:\n    pass\n",
            true,
        ),
        // The fallback import in the handler is what runs: required.
        (
            "try:\n    pass\nexcept ImportError:\n    import colorsys\n",
            false,
        ),
    ] {
        let lowered = lower_ok(source);
        assert_eq!(
            foreign_sites(&lowered),
            vec![(
                "colorsys",
                "colorsys",
                ForeignImportSite::Block { optional }
            )],
            "{source:?}"
        );
        // The statement still lowers to its node whatever the flag.
        assert_eq!(
            nested_nodes(&lowered),
            vec![pair("colorsys", "colorsys")],
            "{source:?}"
        );
    }
}

/// #1290: a handler type that is not a bare name or a tuple of names never
/// guards, even when it spells an import-error class.
#[test]
fn a_non_name_handler_type_does_not_catch_a_failed_import() {
    let parsed =
        parse("try:\n    pass\nexcept builtins.ImportError:\n    pass\nexcept f():\n    pass\n");
    let pycc_ast::Stmt::Try(try_stmt) = &parsed.body[0] else {
        panic!("not a try");
    };
    assert!(!crate::import::block::handlers_catch_import_error(
        &try_stmt.handlers
    ));
    assert!(!crate::import::block::handlers_catch_import_error(&[]));
}
