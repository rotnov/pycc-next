//! A foreign `from X import a, b` (#1278): each name binds the CPython
//! object `X.<name>`, one `ImportBinding::Foreign` per name, and every
//! shape the channel does not carry keeps its `C0001`.

use super::*;
use crate::{FromImport, foreign_bound_object, foreign_import_statement, opens_foreign_statement};

/// Lowers `source`, answering every request for a module in `foreign` with
/// `ResolvedImport::Foreign` and leaving every other request unanswered.
pub(super) fn lower_foreign(
    source: &str,
    foreign: &[&str],
) -> Result<LoweredModule, Vec<Diagnostic>> {
    let parsed = parse(source);
    let mut resolved = ResolvedImports::default();
    for request in project_import_requests(&parsed) {
        if request
            .module
            .as_deref()
            .is_some_and(|module| foreign.contains(&module))
        {
            resolved.insert(request.span, ResolvedImport::Foreign);
        }
    }
    lower_module(&parsed, &resolved, None)
}

pub(super) fn only_error(result: Result<LoweredModule, Vec<Diagnostic>>) -> Diagnostic {
    let diagnostics = result.expect_err("fixture must fail to lower");
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    diagnostics.into_iter().next().expect("one diagnostic")
}

fn from_import(name: &str, fromlist: &[&str], index: usize) -> FromImport {
    FromImport {
        name: name.to_string(),
        fromlist: fromlist.iter().map(ToString::to_string).collect(),
        index,
        level: 0,
    }
}

#[test]
fn every_name_of_a_foreign_from_import_binds_its_own_object_in_order() {
    let source = "from itertools import product, chain\n";
    let lowered = lower_foreign(source, &["itertools"]).expect("must lower");
    let statement = Span::new(0, source.trim_end().len() as u32);
    let expected = vec![
        ImportBinding::Foreign {
            local_name: "product".to_string(),
            module_path: "itertools".to_string(),
            from: Some(from_import("product", &["product", "chain"], 0)),
            site: crate::ForeignImportSite::Item(0),
            span: statement,
        },
        ImportBinding::Foreign {
            local_name: "chain".to_string(),
            module_path: "itertools".to_string(),
            from: Some(from_import("chain", &["product", "chain"], 1)),
            site: crate::ForeignImportSite::Item(0),
            span: statement,
        },
    ];
    assert_eq!(lowered.hir.imports, expected);
}

#[test]
fn a_name_pycc_resolves_by_its_spelling_is_refused() {
    let source = "from builtins import range\n";
    let diagnostic = only_error(lower_foreign(source, &["builtins"]));
    assert_eq!(diagnostic.code, "C0001");
    assert_eq!(
        diagnostic.message,
        "binding the CPython object `builtins.range` to `range`, a name pycc resolves by its \
         spelling (a Python builtin, a stdlib module, or a typing, decorator or base-class \
         marker), is not supported yet"
    );
    assert_eq!(
        diagnostic.span,
        Some(Span::new(0, source.trim_end().len() as u32))
    );
}

/// #1378: a legacy `typing` container alias is resolved by its spelling in
/// annotations, so an unaliased foreign from-import of that name is refused
/// too -- otherwise `List[int]` would silently mean the builtin `list`
/// rather than the CPython object. This import was admitted before #1378.
#[test]
fn a_legacy_typing_container_alias_from_a_foreign_module_is_refused() {
    let source = "from os import List\n";
    let diagnostic = only_error(lower_foreign(source, &["os"]));
    assert_eq!(diagnostic.code, "C0001");
    assert_eq!(
        diagnostic.message,
        "binding the CPython object `os.List` to `List`, a name pycc resolves by its \
         spelling (a Python builtin, a stdlib module, or a typing, decorator or base-class \
         marker), is not supported yet"
    );
}

#[test]
fn a_spelling_later_in_the_list_refuses_the_whole_statement() {
    let diagnostic = only_error(lower_foreign(
        // `NDArray`, not `ndarray`: `from numpy import ndarray` is a
        // buffer-carrier pair since #1380, while `numpy.NDArray` is not.
        "from numpy import array, NDArray\n",
        &["numpy"],
    ));
    assert!(
        diagnostic.message.contains("`numpy.NDArray` to `NDArray`"),
        "{}",
        diagnostic.message
    );
}

#[test]
fn an_aliased_foreign_from_import_keeps_its_c0001() {
    let diagnostic = only_error(lower_foreign(
        "from itertools import product as p\n",
        &["itertools"],
    ));
    assert_eq!(
        diagnostic.message,
        "`from ... import x as y` aliasing is not supported yet"
    );
}

#[test]
fn a_wildcard_foreign_from_import_keeps_its_c0001() {
    let diagnostic = only_error(lower_foreign("from itertools import *\n", &["itertools"]));
    assert_eq!(
        diagnostic.message,
        "`from ... import *` (wildcard import) is not supported yet"
    );
}

/// `import copy` binds the module and `from copy import copy` binds the
/// function: the same local name, two different objects.
#[test]
fn a_from_import_shadowing_the_module_import_of_its_own_name_is_refused() {
    let diagnostic = only_error(lower_foreign(
        "import copy\nfrom copy import copy\n",
        &["copy"],
    ));
    assert_eq!(diagnostic.code, "C0001");
    assert!(
        diagnostic.message.contains("shadowing a foreign import"),
        "{}",
        diagnostic.message
    );
}

#[test]
fn two_from_imports_of_one_name_from_different_modules_are_refused() {
    let diagnostic = only_error(lower_foreign(
        "from json import dumps\nfrom pickle import dumps\n",
        &["json", "pickle"],
    ));
    assert!(
        diagnostic.message.contains("shadowing a foreign import"),
        "{}",
        diagnostic.message
    );
}

/// The same object bound twice is harmless, as `import numpy` twice is.
#[test]
fn the_same_name_imported_twice_from_one_module_lowers() {
    let lowered = lower_foreign(
        "from itertools import product, product\nfrom itertools import product\n",
        &["itertools"],
    )
    .expect("must lower");
    assert_eq!(lowered.hir.imports.len(), 3);
}

#[test]
fn re_exporting_a_dependency_s_foreign_from_import_is_refused() {
    let parsed = parse("from itertools import product\n");
    let mut resolved = ResolvedImports::default();
    for request in project_import_requests(&parsed) {
        resolved.insert(request.span, ResolvedImport::Foreign);
    }
    let fixture = Fixture {
        origin: lower_module(&parsed, &resolved, None)
            .expect("a dependency fixture must lower")
            .hir,
        class_slots: Vec::new(),
    };
    let diagnostic = fixture.first_error("from dep import product\n", &[]);
    assert_eq!(diagnostic.code, "C0001");
    assert_eq!(
        diagnostic.message,
        "`dep.py` binds `product` to the CPython object `itertools.product`; re-exporting a \
         foreign import across project modules is not supported yet"
    );
}

#[test]
fn the_statement_and_object_renderings_cover_both_forms() {
    let from = from_import("chain", &["product", "chain"], 1);
    assert_eq!(
        foreign_import_statement("itertools", "itertools", None),
        "import itertools"
    );
    assert_eq!(
        foreign_import_statement("it", "itertools", None),
        "import itertools as it"
    );
    assert_eq!(
        foreign_import_statement("xml", "xml.dom", None),
        "import xml.dom"
    );
    assert_eq!(
        foreign_import_statement("d", "xml.dom", None),
        "import xml.dom as d"
    );
    assert_eq!(
        foreign_import_statement("chain", "itertools", Some(&from)),
        "from itertools import product, chain"
    );
    assert_eq!(
        foreign_bound_object("itertools", None),
        "the CPython module `itertools`"
    );
    assert_eq!(
        foreign_bound_object("itertools", Some(&from)),
        "the CPython object `itertools.chain`"
    );
    assert!(opens_foreign_statement(None));
    assert!(!opens_foreign_statement(Some(&from)));
    assert!(opens_foreign_statement(Some(&from_import(
        "product",
        &["product", "chain"],
        0
    ))));
}

/// Lowers `source`, answering every relative request `Foreign`, as the
/// driver does for the entry module under `pycc build --ext
/// --foreign-relative-imports` (#1366), and every absolute one in `foreign`
/// too.
pub(super) fn lower_relative(
    source: &str,
    foreign: &[&str],
) -> Result<LoweredModule, Vec<Diagnostic>> {
    let parsed = parse(source);
    let mut resolved = ResolvedImports::default();
    for request in project_import_requests(&parsed) {
        if request.level > 0
            || request
                .module
                .as_deref()
                .is_some_and(|module| foreign.contains(&module))
        {
            resolved.insert(request.span, ResolvedImport::Foreign);
        }
    }
    lower_module(&parsed, &resolved, None)
}

fn relative_from_import(name: &str, fromlist: &[&str], index: usize, level: u32) -> FromImport {
    FromImport {
        level,
        ..from_import(name, fromlist, index)
    }
}

/// #1366: a relative foreign from-import records its dots as `level`, and
/// the module as written without them -- `""` for `from . import x`.
#[test]
fn a_relative_foreign_from_import_records_its_level() {
    let source = "from . import sib\nfrom ..a.b import c, d\n";
    let lowered = lower_relative(source, &[]).expect("must lower");
    let first = Span::new(0, 17);
    let second = Span::new(18, source.trim_end().len() as u32);
    let expected = vec![
        ImportBinding::Foreign {
            local_name: "sib".to_string(),
            module_path: String::new(),
            from: Some(relative_from_import("sib", &["sib"], 0, 1)),
            site: crate::ForeignImportSite::Item(0),
            span: first,
        },
        ImportBinding::Foreign {
            local_name: "c".to_string(),
            module_path: "a.b".to_string(),
            from: Some(relative_from_import("c", &["c", "d"], 0, 2)),
            site: crate::ForeignImportSite::Item(0),
            span: second,
        },
        ImportBinding::Foreign {
            local_name: "d".to_string(),
            module_path: "a.b".to_string(),
            from: Some(relative_from_import("d", &["c", "d"], 1, 2)),
            site: crate::ForeignImportSite::Item(0),
            span: second,
        },
    ];
    assert_eq!(lowered.hir.imports, expected);
}

/// #1366: the spelling refusal renders the relative object with its dots,
/// and no doubled separator for `from . import len`.
#[test]
fn a_relative_name_pycc_resolves_by_its_spelling_is_refused_with_its_dots() {
    let diagnostic = only_error(lower_relative("from . import len\n", &[]));
    assert_eq!(diagnostic.code, "C0001");
    assert!(
        diagnostic
            .message
            .starts_with("binding the CPython object `.len` to `len`,"),
        "{}",
        diagnostic.message
    );
    let diagnostic = only_error(lower_relative("from ..a.b import x, range\n", &[]));
    assert!(
        diagnostic
            .message
            .starts_with("binding the CPython object `..a.b.range` to `range`,"),
        "{}",
        diagnostic.message
    );
}

#[test]
fn a_relative_aliased_or_wildcard_foreign_from_import_keeps_its_c0001() {
    let diagnostic = only_error(lower_relative("from .sib import x as y\n", &[]));
    assert_eq!(
        diagnostic.message,
        "`from ... import x as y` aliasing is not supported yet"
    );
    let diagnostic = only_error(lower_relative("from .sib import *\n", &[]));
    assert_eq!(
        diagnostic.message,
        "`from ... import *` (wildcard import) is not supported yet"
    );
}

/// #1366: `from .x import a` and `from x import a` are two objects, so
/// binding both to `a` is refused; the same relative statement twice is the
/// identical pair.
#[test]
fn a_relative_and_an_absolute_import_of_one_name_are_two_objects() {
    let diagnostic = only_error(lower_relative(
        "from .x import a\nfrom x import a\n",
        &["x"],
    ));
    assert_eq!(diagnostic.code, "C0001");
    assert!(
        diagnostic.message.contains("shadowing a foreign import"),
        "{}",
        diagnostic.message
    );
    let diagnostic = only_error(lower_relative("from .x import a\nfrom ..x import a\n", &[]));
    assert!(
        diagnostic.message.contains("shadowing a foreign import"),
        "{}",
        diagnostic.message
    );
    let lowered = lower_relative("from .x import a\nfrom .x import a\n", &[]).expect("must lower");
    assert_eq!(lowered.hir.imports.len(), 2);
}

/// Without a `Foreign` answer a relative import keeps its `C0001`.
#[test]
fn an_unanswered_relative_import_keeps_its_c0001() {
    let diagnostic = only_error(lower_foreign("from .sib import x\n", &[]));
    assert_eq!(
        diagnostic.message,
        "a relative import (`from . import ...`) is not supported yet"
    );
}

/// #1366: every rendering spells the relative module with its dots, and
/// writes the separating dot only when there is a module name.
#[test]
fn the_relative_renderings_spell_the_dots() {
    let absolute = from_import("x", &["x"], 0);
    let bare = relative_from_import("sib", &["sib", "other"], 0, 1);
    let dotted = relative_from_import("c", &["c"], 0, 2);
    assert_eq!(absolute.spelled_module("m"), "m");
    assert_eq!(absolute.spelled_object("m"), "m.x");
    assert_eq!(bare.spelled_module(""), ".");
    assert_eq!(bare.spelled_object(""), ".sib");
    assert_eq!(dotted.spelled_module("a.b"), "..a.b");
    assert_eq!(dotted.spelled_object("a.b"), "..a.b.c");
    assert_eq!(
        foreign_import_statement("sib", "", Some(&bare)),
        "from . import sib, other"
    );
    assert_eq!(
        foreign_import_statement("c", "a.b", Some(&dotted)),
        "from ..a.b import c"
    );
    assert_eq!(
        foreign_bound_object("", Some(&bare)),
        "the CPython object `.sib`"
    );
    assert_eq!(
        foreign_bound_object("a.b", Some(&dotted)),
        "the CPython object `..a.b.c`"
    );
}
