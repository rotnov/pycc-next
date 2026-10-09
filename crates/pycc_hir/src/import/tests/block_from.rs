//! A CPython-backed `from X import a, b` nested in a module-level
//! `if`/`try` block (#1383): it lowers to `HirStmt::ForeignImport` only
//! when the driver answered it foreign, binds as the top-level form does
//! (#1278), and every other answer keeps the block-body diagnostic.

use super::*;
use crate::{FromImport, HirItem, HirStmt};

/// Lowers `source`, answering every request whose module is in `foreign`
/// with `ResolvedImport::Foreign` and leaving every other request
/// unanswered, as `src/modules.rs` answers a nested from-import.
fn lower_foreign(source: &str, foreign: &[&str]) -> Result<LoweredModule, Vec<Diagnostic>> {
    lower_answering(source, |request| {
        request
            .module
            .as_deref()
            .is_some_and(|module| foreign.contains(&module))
            .then_some(ResolvedImport::Foreign)
    })
}

/// Lowers `source` with `answer`'s reply to every request.
fn lower_answering(
    source: &str,
    answer: impl Fn(&ProjectImportRequest) -> Option<ResolvedImport<'static>>,
) -> Result<LoweredModule, Vec<Diagnostic>> {
    let parsed = parse(source);
    let mut resolved = ResolvedImports::default();
    for request in project_import_requests(&parsed) {
        if let Some(answer) = answer(&request) {
            resolved.insert(request.span, answer);
        }
    }
    lower_module(&parsed, &resolved, None)
}

fn lower_ok(source: &str, foreign: &[&str]) -> LoweredModule {
    lower_foreign(source, foreign)
        .unwrap_or_else(|diagnostics| panic!("{source:?} must lower: {diagnostics:#?}"))
}

fn only_error(result: Result<LoweredModule, Vec<Diagnostic>>) -> Diagnostic {
    let diagnostics = result.expect_err("fixture must fail to lower");
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    diagnostics.into_iter().next().expect("one diagnostic")
}

fn from_import(name: &str, fromlist: &[&str], index: usize, level: u32) -> FromImport {
    FromImport {
        name: name.to_string(),
        fromlist: fromlist.iter().map(ToString::to_string).collect(),
        index,
        level,
    }
}

/// The span of the statement starting at `needle`'s first occurrence and
/// ending at the end of its line.
fn statement_at(source: &str, needle: &str) -> Span {
    let start = source.find(needle).expect("needle");
    let end = start + source[start..].find('\n').expect("a line end");
    Span::new(start as u32, end as u32)
}

/// The bindings of every top-level-statement `ForeignImport` node reached
/// through `if`/`try` nesting, in source order.
fn nested_bindings(module: &LoweredModule) -> Vec<Vec<(String, String, Option<FromImport>)>> {
    fn walk(stmts: &[HirStmt], out: &mut Vec<Vec<(String, String, Option<FromImport>)>>) {
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

fn sites(module: &LoweredModule) -> Vec<(&str, ForeignImportSite)> {
    module
        .hir
        .imports
        .iter()
        .filter_map(|binding| match binding {
            ImportBinding::Foreign {
                local_name, site, ..
            } => Some((local_name.as_str(), *site)),
            _ => None,
        })
        .collect()
}

const BLOCK_TEXT: &str = "an `import` inside a block body";

#[test]
fn a_nested_from_import_is_requested_flagged_nested() {
    let source = "from a import x\nif c:\n    from b import y\n";
    let requests = project_import_requests(&parse(source));
    let flags: Vec<(Option<&str>, bool)> = requests
        .iter()
        .map(|request| (request.module.as_deref(), request.nested))
        .collect();
    assert_eq!(flags, vec![(Some("a"), false), (Some("b"), true)]);
    assert_eq!(requests[1].span, statement_at(source, "from b"));
    assert_eq!(requests[1].names, vec!["y".to_string()]);

    // A `pycc_std` module and `__future__` stay unrequested when nested.
    let requests = project_import_requests(&parse(
        "if c:\n    from math import sqrt\n    from __future__ import annotations\n",
    ));
    assert!(requests.is_empty(), "{requests:#?}");
}

#[test]
fn a_foreign_from_import_in_an_if_body_lowers_to_a_foreign_import_node() {
    let source = "if c:\n    from colorsys import hls_to_rgb\n";
    let lowered = lower_ok(source, &["colorsys"]);
    let statement = statement_at(source, "from colorsys");
    let from = from_import("hls_to_rgb", &["hls_to_rgb"], 0, 0);
    assert_eq!(
        lowered.hir.items,
        vec![HirItem::TopLevelStmt(HirStmt::If {
            test: crate::HirExpr::Name("c".to_string()),
            body: vec![HirStmt::ForeignImport {
                bindings: vec![(
                    "hls_to_rgb".to_string(),
                    "colorsys".to_string(),
                    Some(from.clone())
                )],
                span: statement,
            }],
            orelse: vec![],
        })]
    );
    assert_eq!(
        lowered.hir.imports,
        vec![ImportBinding::Foreign {
            local_name: "hls_to_rgb".to_string(),
            module_path: "colorsys".to_string(),
            from: Some(from),
            site: ForeignImportSite::Block { optional: false },
            span: statement,
        }]
    );
}

#[test]
fn a_multi_name_from_import_is_one_node_with_its_names_in_order() {
    let lowered = lower_ok(
        "if c:\n    from itertools import product, chain\n",
        &["itertools"],
    );
    let fromlist = ["product", "chain"];
    assert_eq!(
        nested_bindings(&lowered),
        vec![vec![
            (
                "product".to_string(),
                "itertools".to_string(),
                Some(from_import("product", &fromlist, 0, 0))
            ),
            (
                "chain".to_string(),
                "itertools".to_string(),
                Some(from_import("chain", &fromlist, 1, 0))
            ),
        ]]
    );
}

/// A dotted module (Part 1 of #1138) and a relative one answered foreign
/// (the entry module under `--foreign-relative-imports`, #1366) lower
/// nested as they do at top level.
#[test]
fn a_dotted_or_relative_nested_from_import_answered_foreign_lowers() {
    let lowered = lower_ok("if c:\n    from os.path import join\n", &["os.path"]);
    assert_eq!(
        nested_bindings(&lowered),
        vec![vec![(
            "join".to_string(),
            "os.path".to_string(),
            Some(from_import("join", &["join"], 0, 0))
        )]]
    );

    let lowered = lower_answering("if c:\n    from .x import a\n", |request| {
        (request.level == 1).then_some(ResolvedImport::Foreign)
    })
    .expect("must lower");
    assert_eq!(
        nested_bindings(&lowered),
        vec![vec![(
            "a".to_string(),
            "x".to_string(),
            Some(from_import("a", &["a"], 0, 1))
        )]]
    );
}

#[test]
fn a_try_body_whose_handler_catches_import_error_makes_it_optional() {
    for (source, optional) in [
        (
            "try:\n    from itertools import product\nexcept ImportError:\n    pass\n",
            true,
        ),
        (
            "try:\n    from itertools import product\nexcept ValueError:\n    pass\n",
            false,
        ),
    ] {
        let lowered = lower_ok(source, &["itertools"]);
        assert_eq!(
            sites(&lowered),
            vec![("product", ForeignImportSite::Block { optional })],
            "{source:?}"
        );
    }
}

/// The same `from X import a` twice is exempt from shadowing (#1291),
/// whether in both arms or nested and at top level.
#[test]
fn an_identical_from_import_twice_is_accepted() {
    let lowered = lower_ok(
        "if c:\n    from itertools import product\nelse:\n    from itertools import product\n",
        &["itertools"],
    );
    assert_eq!(nested_bindings(&lowered).len(), 2);

    let lowered = lower_ok(
        "from itertools import product\nif c:\n    from itertools import product\n",
        &["itertools"],
    );
    assert_eq!(
        sites(&lowered),
        vec![
            ("product", ForeignImportSite::Item(0)),
            ("product", ForeignImportSite::Block { optional: false }),
        ]
    );
}

#[test]
fn shadowing_a_nested_from_import_follows_the_top_level_rule() {
    // The `except ImportError: product = None` fallback is admitted since
    // #1485 (`tests/fallback.rs`).
    for source in [
        "if c:\n    from itertools import product\nproduct = 1\n",
        "if c:\n    from itertools import product\nelse:\n    from functools import product\n",
        "import product\nif c:\n    from product import product\n",
    ] {
        let error = only_error(lower_foreign(
            source,
            &["itertools", "functools", "product"],
        ));
        assert!(
            error.message.contains("shadowing a foreign import"),
            "{source:?}: {}",
            error.message
        );
    }
}

/// The top-level form's own refusals, reported at the nested statement
/// rather than the enclosing block.
#[test]
fn a_nested_from_import_reports_its_top_level_refusal_at_its_own_span() {
    for (source, needle, message) in [
        (
            "if c:\n    from itertools import *\n",
            "from itertools",
            "`from ... import *` (wildcard import) is not supported yet".to_string(),
        ),
        (
            "if c:\n    from itertools import product as p\n",
            "from itertools",
            "`from ... import x as y` aliasing is not supported yet".to_string(),
        ),
        (
            "if c:\n    from itertools import range\n",
            "from itertools",
            "binding the CPython object `itertools.range` to `range`, a name pycc resolves by \
             its spelling (a Python builtin, a stdlib module, or a typing, decorator or \
             base-class marker), is not supported yet"
                .to_string(),
        ),
    ] {
        let error = only_error(lower_foreign(source, &["itertools"]));
        assert_eq!(error.message, message, "{source:?}");
        assert_eq!(error.span, Some(statement_at(source, needle)), "{source:?}");
    }
}

/// Only a foreign answer lowers a nested from-import: none at all, a
/// project module, a `NotFound` and a `Found` all keep the block-body text
/// at the nested statement.
#[test]
fn a_nested_from_import_not_answered_foreign_keeps_the_block_diagnostic() {
    let source = "if c:\n    from helper import g\n";
    let statement = statement_at(source, "from helper");
    let fixture = Fixture::new("def g() -> None:\n    pass\n");
    for result in [
        lower_answering(source, |_| None),
        fixture.lower(source, &[]),
        lower_answering(source, |_| {
            Some(ResolvedImport::NotFound {
                code: "T0021",
                message: "no module named `helper`".to_string(),
            })
        }),
        lower_answering(source, |_| Some(ResolvedImport::Found)),
    ] {
        let error = only_error(result);
        assert!(error.message.contains(BLOCK_TEXT), "{}", error.message);
        assert_eq!(error.span, Some(statement));
    }
}

#[test]
fn a_type_checking_body_from_import_is_folded_away() {
    let lowered = lower_ok(
        "from typing import TYPE_CHECKING\nif TYPE_CHECKING:\n    from itertools import product\n",
        &["itertools"],
    );
    assert!(sites(&lowered).is_empty());
    assert!(nested_bindings(&lowered).is_empty());
}

#[test]
fn a_nested_from_import_colliding_with_a_class_is_refused_at_its_own_statement() {
    let source = "class product:\n    pass\nif c:\n    from itertools import product\n";
    let error = only_error(lower_foreign(source, &["itertools"]));
    assert_eq!(
        error.message,
        "import `product` collides with a class of the same name already defined in this module"
    );
    assert_eq!(error.span, Some(statement_at(source, "from itertools")));
}
