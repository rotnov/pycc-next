//! Unit tests for whole-program linking (#898, Part 1 of #881, D-222):
//! the equivalence with the single-file `lower_all` path, the flat
//! namespace's collision rule, the cross-module seeding reconciliation,
//! and the program-wide exception-tag assignment.
//!
//! Kept here rather than in `crates/pycc_hir/src/tests.rs`, which #663
//! already tracks as oversized.

use super::*;
use crate::pycc_parser_test_helper::parse;
use crate::{HirItem, ResolvedImports, lower_all, lower_module};

/// Lowers `source` as one standalone module named `display_path`.
fn input(display_path: &str, source: &str) -> LinkInput {
    LinkInput {
        display_path: display_path.to_string(),
        module: lower_module(&parse(source), &ResolvedImports::default(), None)
            .expect("a fixture module must lower"),
    }
}

/// The full program pipeline over `inputs`.
fn link_and_finalize(inputs: Vec<LinkInput>) -> Result<HirModule, Vec<(usize, Diagnostic)>> {
    let linked = link(inputs)?;
    finalize(linked).map_err(|diagnostics| {
        diagnostics
            .into_iter()
            .map(|diagnostic| (0, diagnostic))
            .collect()
    })
}

fn first_error(inputs: Vec<LinkInput>) -> (usize, Diagnostic) {
    let mut errors = link_and_finalize(inputs).expect_err("these inputs must be rejected");
    assert_eq!(errors.len(), 1, "link reports exactly the first problem");
    errors.remove(0)
}

/// The exception type tag of every class in `hir`, in program order,
/// restricted to the user classes (the synthetic builtins carry none).
fn user_tags(hir: &HirModule) -> Vec<(String, Option<u8>)> {
    hir.class_defs
        .iter()
        .filter(|(name, _)| !is_builtin_exception_class(name))
        .map(|(name, def)| (name.clone(), def.exception_type_tag))
        .collect()
}

const POINT: &str = "class Point:\n    def __init__(self, x: int) -> None:\n        self.x = x\n";

const SEEDING: &str =
    "class MyError(ValueError):\n    pass\n\n\ndef main() -> None:\n    raise MyError(\"x\")\n";

#[test]
fn linking_one_unseeded_module_equals_lower_all() {
    let source = "def helper(x: int) -> int:\n    return x + 1\n\n\nvalue = 3\n";
    let linked = link_and_finalize(vec![input("a.py", source)]).expect("must link");
    let single = lower_all(&parse(source)).expect("must lower");
    assert_eq!(linked, single);
    assert!(!linked.seeded_builtin_exception_classes);
}

#[test]
fn linking_one_seeded_module_equals_lower_all() {
    // The seeded synthetic classes are stripped and re-appended by `link`;
    // the result must still be byte-identical to the single-file path,
    // including the trailing synthetic block and the tag numbering.
    let linked = link_and_finalize(vec![input("a.py", SEEDING)]).expect("must link");
    let single = lower_all(&parse(SEEDING)).expect("must lower");
    assert_eq!(linked, single);
    assert!(linked.seeded_builtin_exception_classes);
}

#[test]
fn items_are_concatenated_in_input_order() {
    let linked = link_and_finalize(vec![
        input("dep.py", "def first() -> int:\n    return 1\n"),
        input("main.py", "def second() -> int:\n    return 2\n"),
    ])
    .expect("must link");
    let names: Vec<&str> = linked
        .items
        .iter()
        .map(|item| match item {
            HirItem::Function { name, .. } => name.as_str(),
            HirItem::TopLevelStmt(_) => "<stmt>",
        })
        .collect();
    assert_eq!(names, vec!["first", "second"]);
}

#[test]
fn type_aliases_and_imports_are_unioned() {
    let linked = link_and_finalize(vec![
        input("dep.py", "type Meters = int\nimport math\n"),
        input("main.py", "type Seconds = float\n"),
    ])
    .expect("must link");
    let aliases: Vec<&str> = linked
        .type_aliases
        .iter()
        .map(|(name, _)| name.as_str())
        .collect();
    assert_eq!(aliases, vec!["Meters", "Seconds"]);
    assert_eq!(linked.imports.len(), 1);
}

#[test]
fn two_modules_defining_one_function_name_collide() {
    let (index, diagnostic) = first_error(vec![
        input("dep.py", "def helper() -> int:\n    return 1\n"),
        input("main.py", "def helper() -> int:\n    return 2\n"),
    ]);
    assert_eq!(
        index, 1,
        "the collision is reported at the later definition"
    );
    assert_eq!(diagnostic.code, "C0001");
    assert!(
        diagnostic
            .message
            .contains("top-level name `helper` is already defined by `dep.py`"),
        "unexpected message: {}",
        diagnostic.message
    );
    assert!(
        diagnostic
            .message
            .contains("a separate namespace per module is not supported yet"),
        "unexpected message: {}",
        diagnostic.message
    );
}

#[test]
fn a_top_level_loop_variable_collides_across_modules() {
    // A name bound by a top-level statement is a definition just like a
    // `def`: the loop target `total` is reported at the statement span.
    let source = "total = 0\nfor total in range(2):\n    print(total)\n";
    let (index, diagnostic) = first_error(vec![input("dep.py", source), input("main.py", source)]);
    assert_eq!(index, 1);
    assert_eq!(diagnostic.code, "C0001");
    assert!(
        diagnostic
            .message
            .contains("top-level name `total` is already defined by `dep.py`"),
        "unexpected message: {}",
        diagnostic.message
    );
}

#[test]
fn a_module_rebinding_its_own_name_does_not_collide_with_itself() {
    // `own` dedupes within one input, so `x = 1; x = 2` in a single module
    // is still legal -- only a name owned by an *earlier* input collides.
    let linked = link_and_finalize(vec![
        input("dep.py", "x = 1\nx = 2\n"),
        input("main.py", "y = 3\n"),
    ])
    .expect("must link");
    assert_eq!(linked.items.len(), 3);
}

#[test]
fn a_class_defined_in_two_modules_collides() {
    let (index, diagnostic) = first_error(vec![input("dep.py", POINT), input("main.py", POINT)]);
    assert_eq!(index, 1);
    assert!(
        diagnostic
            .message
            .contains("top-level name `Point` is already defined by `dep.py`"),
        "unexpected message: {}",
        diagnostic.message
    );
}

#[test]
fn two_seeded_modules_keep_exactly_one_synthetic_class_set() {
    let other = "class OtherError(ValueError):\n    pass\n\n\ndef other() -> None:\n    raise OtherError(\"y\")\n";
    let linked = link_and_finalize(vec![input("dep.py", other), input("main.py", SEEDING)])
        .expect("must link");
    assert!(linked.seeded_builtin_exception_classes);
    let synthetic = linked
        .class_defs
        .iter()
        .filter(|(name, _)| is_builtin_exception_class(name))
        .count();
    assert_eq!(synthetic, builtin_exception_class_defs().len());
}

#[test]
fn exception_tags_are_assigned_across_seeded_modules_in_program_order() {
    let other = "class OtherError(ValueError):\n    pass\n\n\ndef other() -> None:\n    raise OtherError(\"y\")\n";
    let linked = link_and_finalize(vec![input("dep.py", other), input("main.py", SEEDING)])
        .expect("must link");
    assert_eq!(
        user_tags(&linked),
        vec![
            (
                "OtherError".to_string(),
                Some(FIRST_USER_EXCEPTION_TYPE_TAG)
            ),
            (
                "MyError".to_string(),
                Some(FIRST_USER_EXCEPTION_TYPE_TAG + 1)
            ),
        ]
    );
}

#[test]
fn an_unseeded_module_links_with_a_seeded_one() {
    let plain = "def helper() -> int:\n    return 1\n";
    let linked = link_and_finalize(vec![input("dep.py", plain), input("main.py", SEEDING)])
        .expect("must link");
    assert!(linked.seeded_builtin_exception_classes);
    assert_eq!(
        user_tags(&linked),
        vec![("MyError".to_string(), Some(FIRST_USER_EXCEPTION_TYPE_TAG))]
    );
}

#[test]
fn two_modules_that_never_name_a_builtin_exception_seed_nothing() {
    let linked = link_and_finalize(vec![
        input("dep.py", "def helper() -> int:\n    return 1\n"),
        input("main.py", "def main() -> None:\n    print(helper())\n"),
    ])
    .expect("must link");
    assert!(!linked.seeded_builtin_exception_classes);
    assert!(linked.class_defs.is_empty());
}

#[test]
fn a_module_shadowing_a_builtin_exception_cannot_link_with_a_seeded_one() {
    let shadowing = "helper = 1\n\n\ndef ValueError() -> int:\n    return 1\n";
    let (index, diagnostic) =
        first_error(vec![input("main.py", SEEDING), input("dep.py", shadowing)]);
    assert_eq!(index, 1, "reported against the shadowing module");
    assert_eq!(diagnostic.code, "C0001");
    assert!(
        diagnostic.message.contains(
            "module `dep.py` defines `ValueError`, which `main.py` uses as the builtin exception"
        ),
        "unexpected message: {}",
        diagnostic.message
    );
    assert!(
        diagnostic.span.expect("a span is attached").start > 0,
        "reported at the definition"
    );
}

#[test]
fn a_shadow_that_binds_nothing_is_reported_at_the_module_start() {
    // `ValueError: int` is an annotation with no value: the AST shadow scan
    // sees it, but it binds nothing at runtime, so `lower_module` records no
    // definition span for it and `definition_span` falls back to the module
    // start.
    let (index, diagnostic) = first_error(vec![
        input("main.py", SEEDING),
        input("dep.py", "ValueError: int\n"),
    ]);
    assert_eq!(index, 1);
    let span = diagnostic.span.expect("a span is attached");
    assert_eq!(span.start, 0);
    assert_eq!(span.end, 0);
}

#[test]
fn two_shadowing_modules_link_without_seeding() {
    // No input seeded, so the cross-module shadow gate never runs and the
    // program is simply un-seeded, exactly as each module was alone.
    let linked = link_and_finalize(vec![
        input("dep.py", "ValueError = 1\n"),
        input("main.py", "TypeError = 2\n"),
    ])
    .expect("must link");
    assert!(!linked.seeded_builtin_exception_classes);
}

#[test]
fn the_exception_class_budget_is_program_wide() {
    // Neither module alone exceeds the limit; together they do.
    let half = MAX_USER_EXCEPTION_CLASSES / 2 + 1;
    let dep = many_exception_classes("D", half);
    let main = many_exception_classes("M", half);
    let (_, diagnostic) = first_error(vec![input("dep.py", &dep), input("main.py", &main)]);
    assert_eq!(diagnostic.code, "C0001");
    assert!(
        diagnostic.message.contains(&format!(
            "program declares more than {MAX_USER_EXCEPTION_CLASSES} exception classes"
        )),
        "unexpected message: {}",
        diagnostic.message
    );
}

fn many_exception_classes(prefix: &str, count: usize) -> String {
    let mut source = String::new();
    for index in 0..count {
        source.push_str(&format!(
            "class {prefix}{index}(Exception):\n    pass\n\n\n"
        ));
    }
    source.push_str(&format!(
        "def {}() -> None:\n    raise {prefix}0(\"x\")\n",
        prefix.to_lowercase()
    ));
    source
}

/// Lowers `source` with the driver answering `ResolvedImport::Foreign` for
/// the statement spelled `import_stmt`, so the module carries an
/// `ImportBinding::Foreign` (Part 1 of #1026). `input` cannot do this: it
/// lowers against an empty answer table, which that branch never fires on.
fn foreign_input(display_path: &str, source: &str, import_stmt: &str) -> LinkInput {
    let parsed = parse(source);
    let resolved = foreign_answer(&parsed, import_stmt);
    LinkInput {
        display_path: display_path.to_string(),
        module: lower_module(&parsed, &resolved, None).expect("a fixture module must lower"),
    }
}

/// The driver's `ResolvedImport::Foreign` answer for the plain
/// `import_stmt` (`import <name>`), keyed by the span `pycc_hir` itself
/// requests it under -- the alias's own span since #1280 -- rather than a
/// re-derived one.
fn foreign_answer(parsed: &pycc_ast::ModModule, import_stmt: &str) -> ResolvedImports<'static> {
    let name = import_stmt
        .strip_prefix("import ")
        .expect("a foreign fixture imports with a plain `import <name>`");
    let request = crate::project_import_requests(parsed)
        .into_iter()
        .find(|request| request.module.as_deref() == Some(name) && request.names.is_empty())
        .expect("the fixture must contain its import statement");
    let mut resolved = ResolvedImports::default();
    resolved.insert(request.span, crate::ResolvedImport::Foreign);
    resolved
}

#[test]
fn linking_rebases_a_foreign_import_item_index_onto_the_program() {
    // The first module contributes three items, so the second module's
    // items start at program index 3; its own import sits at local index
    // 1, which makes the expected rebased index 4. Both numbers are
    // greater than one and different from each other, so an
    // implementation that dropped either the offset or the local index
    // lands somewhere else.
    let first = input("first.py", "a = 1\nb = 2\nc = 3\n");
    assert_eq!(first.module.hir.items.len(), 3);
    let second = foreign_input("second.py", "d = 4\nimport numpy\n", "import numpy");
    assert_eq!(
        second.module.hir.imports,
        vec![ImportBinding::Foreign {
            local_name: "numpy".to_string(),
            module_path: "numpy".to_string(),
            from: None,
            site: crate::ForeignImportSite::Item(1),
            // `import numpy` follows `d = 4\n`, so the recorded span is
            // the import statement's own range, not the module's start.
            span: Span::new(6, 18),
        }],
        "the fixture's own index must be local, or the rebase below proves nothing"
    );

    let linked = link(vec![first, second]).expect("the fixture program must link");

    assert_eq!(
        linked.imports,
        vec![ImportBinding::Foreign {
            local_name: "numpy".to_string(),
            module_path: "numpy".to_string(),
            from: None,
            site: crate::ForeignImportSite::Item(4),
            span: Span::new(6, 18),
        }]
    );
}

const FOREIGN_DEP: &str = "import json\n\n\ndef f() -> int:\n    return json()\n";
const SHADOWING_ENTRY: &str = "def json() -> int:\n    return 1\n";
const SHADOW_MESSAGE: &str = "module `main.py` defines `json`, which `dep.py` binds to a \
                              CPython module object; shadowing a foreign import across \
                              modules is not supported yet";

#[test]
fn a_definition_shadowing_another_module_s_foreign_import_is_rejected() {
    // The entry module is linked last, after the dependency that binds
    // `json` to the CPython module object.
    let (index, diagnostic) = first_error(vec![
        foreign_input("dep.py", FOREIGN_DEP, "import json"),
        input("main.py", SHADOWING_ENTRY),
    ]);
    assert_eq!(index, 1, "the diagnostic belongs to the shadowing module");
    assert_eq!(diagnostic.code, "C0001");
    assert_eq!(diagnostic.message, SHADOW_MESSAGE);
    assert_eq!(
        diagnostic.span,
        Some(Span::new(0, SHADOWING_ENTRY.trim_end().len() as u32)),
        "the span is the shadowing definition's own statement"
    );
}

#[test]
fn a_definition_before_the_foreign_module_in_link_order_is_rejected_too() {
    // The reverse dependency order: the shadowing definition is linked
    // first, so a check that only consulted the incrementally built
    // `owners` map would miss it.
    let (index, diagnostic) = first_error(vec![
        input("main.py", SHADOWING_ENTRY),
        foreign_input("dep.py", FOREIGN_DEP, "import json"),
    ]);
    assert_eq!(index, 0, "the diagnostic still belongs to the definition");
    assert_eq!(diagnostic.code, "C0001");
    assert_eq!(diagnostic.message, SHADOW_MESSAGE);
}

#[test]
fn a_foreign_import_no_other_module_shadows_still_links() {
    let linked = link(vec![
        input("a.py", "def first() -> int:\n    return 1\n"),
        foreign_input("dep.py", FOREIGN_DEP, "import json"),
    ])
    .expect("a foreign import no module shadows must link");
    assert_eq!(
        linked.imports,
        vec![ImportBinding::Foreign {
            local_name: "json".to_string(),
            module_path: "json".to_string(),
            from: None,
            site: crate::ForeignImportSite::Item(1),
            span: Span::new(0, "import json".len() as u32),
        }],
        "the dependency-local index 0 is rebased past `a.py`'s one item"
    );
}

#[test]
fn a_module_shadowing_its_own_foreign_import_never_reaches_this_gate() {
    // `import json` then `def json()` in one file is refused by
    // `import::reject_shadowed_foreign_imports` while the module is still
    // being lowered, so no `LinkInput` for it can exist and this gate --
    // which exists for the case a *different* module's definition silently
    // changes what the foreign module's own call resolves to -- stays
    // cross-module. Asserted here rather than only at the lowering site so
    // the two rules cannot drift into either a gap or a double report.
    let source = "import json\n\n\ndef json() -> int:\n    return 1\n";
    let parsed = parse(source);
    let resolved = foreign_answer(&parsed, "import json");
    let diagnostics =
        lower_module(&parsed, &resolved, None).expect_err("the shadowing module must be refused");
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert_eq!(diagnostics[0].code, "C0001", "{diagnostics:?}");
    assert!(
        diagnostics[0]
            .message
            .contains("shadowing a foreign import is not supported yet"),
        "{diagnostics:?}"
    );
}

/// Every plain `import` request in `source` answered `Foreign`, including
/// an aliased or a nested one (#1291).
fn all_foreign_input(display_path: &str, source: &str) -> LinkInput {
    let parsed = parse(source);
    let mut resolved = ResolvedImports::default();
    for request in crate::project_import_requests(&parsed) {
        resolved.insert(request.span, crate::ResolvedImport::Foreign);
    }
    LinkInput {
        display_path: display_path.to_string(),
        module: lower_module(&parsed, &resolved, None).expect("a fixture module must lower"),
    }
}

#[test]
fn one_name_bound_to_two_cpython_modules_across_modules_is_rejected() {
    let entry = "import colorsys as json\n";
    let (index, diagnostic) = first_error(vec![
        all_foreign_input("dep.py", "import json\n"),
        all_foreign_input("main.py", entry),
    ]);
    assert_eq!(index, 1, "the diagnostic belongs to the later module");
    assert_eq!(diagnostic.code, "C0001");
    assert_eq!(
        diagnostic.message,
        "module `main.py` binds `json` to the CPython module `colorsys`, which `dep.py` binds \
         to `json`; shadowing a foreign import across modules is not supported yet"
    );
    assert_eq!(
        diagnostic.span,
        Some(Span::new(0, entry.trim_end().len() as u32))
    );
}

#[test]
fn an_identical_foreign_pair_across_modules_links() {
    let linked = link_and_finalize(vec![
        all_foreign_input("dep.py", "import json\n"),
        all_foreign_input(
            "main.py",
            "try:\n    import json\nexcept ImportError:\n    pass\n",
        ),
    ])
    .expect("the same module bound to the same name in two modules must link");
    let sites: Vec<crate::ForeignImportSite> = linked
        .imports
        .iter()
        .filter_map(|binding| match binding {
            ImportBinding::Foreign { site, .. } => Some(*site),
            _ => None,
        })
        .collect();
    // The nested import keeps its `Block` site, and its #1290 `optional`
    // flag, through the rebase.
    assert_eq!(
        sites,
        vec![
            crate::ForeignImportSite::Item(0),
            crate::ForeignImportSite::Block { optional: true }
        ]
    );
}

/// #1381: `import os` and `import os.path` bind the one module `os`, so the
/// pair links across modules; a different module bound to the root's name
/// is refused, and both directions name the bound root.
#[test]
fn dotted_imports_compare_the_bound_root_across_modules() {
    link_and_finalize(vec![
        all_foreign_input("dep.py", "import os.path\n"),
        all_foreign_input("main.py", "import os\n"),
    ])
    .expect("`import os` and `import os.path` bind the same module");
    let (_, diagnostic) = first_error(vec![
        all_foreign_input("dep.py", "import xml.dom\n"),
        all_foreign_input("main.py", "import colorsys as xml\n"),
    ]);
    assert_eq!(
        diagnostic.message,
        "module `main.py` binds `xml` to the CPython module `colorsys`, which `dep.py` binds \
         to `xml`; shadowing a foreign import across modules is not supported yet"
    );
    let (_, reversed) = first_error(vec![
        all_foreign_input("dep.py", "import colorsys as xml\n"),
        all_foreign_input("main.py", "import xml.dom\n"),
    ]);
    assert_eq!(
        reversed.message,
        "module `main.py` binds `xml` to the CPython module `xml`, which `dep.py` binds \
         to `colorsys`; shadowing a foreign import across modules is not supported yet"
    );
}

/// #1278: a foreign from-import's identity is the module *and* the name, so
/// the same local name taken from two modules is refused across modules.
#[test]
fn one_name_from_two_cpython_modules_across_modules_is_rejected() {
    let (index, diagnostic) = first_error(vec![
        all_foreign_input("dep.py", "from json import dumps\n"),
        all_foreign_input("main.py", "from pickle import dumps\n"),
    ]);
    assert_eq!(index, 1);
    assert_eq!(diagnostic.code, "C0001");
    assert_eq!(
        diagnostic.message,
        "module `main.py` binds `dumps` to the CPython object `pickle.dumps`, which `dep.py` \
         binds to `json.dumps`; shadowing a foreign import across modules is not supported yet"
    );
}

/// `import copy` in one module and `from copy import copy` in another bind
/// `copy` to two different objects of one module.
#[test]
fn a_module_import_and_a_from_import_of_one_name_across_modules_are_rejected() {
    let (_, diagnostic) = first_error(vec![
        all_foreign_input("dep.py", "import copy\n"),
        all_foreign_input("main.py", "from copy import copy\n"),
    ]);
    assert_eq!(
        diagnostic.message,
        "module `main.py` binds `copy` to the CPython object `copy.copy`, which `dep.py` binds \
         to `copy`; shadowing a foreign import across modules is not supported yet"
    );
    let (_, reversed) = first_error(vec![
        all_foreign_input("dep.py", "from copy import copy\n"),
        all_foreign_input("main.py", "import copy\n"),
    ]);
    assert_eq!(
        reversed.message,
        "module `main.py` binds `copy` to the CPython module `copy`, which `dep.py` binds to \
         `copy.copy`; shadowing a foreign import across modules is not supported yet"
    );
}

#[test]
fn a_definition_shadowing_another_module_s_foreign_from_import_is_rejected() {
    let (index, diagnostic) = first_error(vec![
        all_foreign_input("dep.py", "from itertools import product\n"),
        input("main.py", "product = 1\n"),
    ]);
    assert_eq!(index, 1);
    assert_eq!(
        diagnostic.message,
        "module `main.py` defines `product`, which `dep.py` binds to the CPython object \
         `itertools.product`; shadowing a foreign import across modules is not supported yet"
    );
}

/// The same object in two modules links, and the rebase keeps each
/// binding's from-import unchanged.
#[test]
fn an_identical_foreign_from_pair_across_modules_links_and_keeps_its_from() {
    let linked = link_and_finalize(vec![
        all_foreign_input("dep.py", "from itertools import product\n"),
        all_foreign_input("main.py", "x = 1\nfrom itertools import chain, product\n"),
    ])
    .expect("the same object bound to the same name in two modules must link");
    let froms: Vec<(String, Option<usize>, crate::ForeignImportSite)> = linked
        .imports
        .iter()
        .filter_map(|binding| match binding {
            ImportBinding::Foreign { from, site, .. } => Some((
                from.as_ref()
                    .map(|from| from.name.clone())
                    .unwrap_or_default(),
                from.as_ref().map(|from| from.index),
                *site,
            )),
            _ => None,
        })
        .collect();
    assert_eq!(
        froms,
        vec![
            (
                "product".to_string(),
                Some(0),
                crate::ForeignImportSite::Item(0)
            ),
            // `dep.py` contributes no item, so `main.py`'s local index 1
            // (after `x = 1`) is also its program index.
            (
                "chain".to_string(),
                Some(0),
                crate::ForeignImportSite::Item(1)
            ),
            (
                "product".to_string(),
                Some(1),
                crate::ForeignImportSite::Item(1)
            ),
        ]
    );
}

/// #1366: a relative foreign from-import's identity includes its level, so
/// `from .x import a` and `from x import a` in two modules are two objects.
/// The relative binding sits in the *earlier* module, the owner, so the
/// message renders it through `dotted()`'s relative arm; a real build links
/// the entry last, so only this arrangement reaches it.
#[test]
fn a_relative_and_an_absolute_from_import_across_modules_are_two_objects() {
    let (index, diagnostic) = first_error(vec![
        all_foreign_input("dep.py", "from . import a\n"),
        all_foreign_input("main.py", "from x import a\n"),
    ]);
    assert_eq!(index, 1);
    assert_eq!(
        diagnostic.message,
        "module `main.py` binds `a` to the CPython object `x.a`, which `dep.py` binds to `.a`; \
         shadowing a foreign import across modules is not supported yet"
    );
    let (_, diagnostic) = first_error(vec![
        all_foreign_input("dep.py", "from ..x import a\n"),
        all_foreign_input("main.py", "from .x import a\n"),
    ]);
    assert_eq!(
        diagnostic.message,
        "module `main.py` binds `a` to the CPython object `.x.a`, which `dep.py` binds to \
         `..x.a`; shadowing a foreign import across modules is not supported yet"
    );
    link_and_finalize(vec![
        all_foreign_input("dep.py", "from .x import a\n"),
        all_foreign_input("main.py", "from .x import a\n"),
    ])
    .expect("the same relative object in two modules must link");
}
