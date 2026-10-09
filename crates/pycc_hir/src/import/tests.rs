//! Unit tests for the project-import half of `import.rs` (#898, Part 1 of
//! #881, D-222): what a resolved dependency binds into the importing
//! module, and every shape the lookup chain rejects.
//!
//! Kept here rather than in `crates/pycc_hir/src/tests.rs`, which #663
//! already tracks as oversized.

use super::*;
use crate::pycc_parser_test_helper::parse;
use crate::{LoweredModule, lower_module};

mod annotations_ext;
mod annotations_foreign;
mod block;
mod block_from;
mod carrier;
mod dotted;
mod fallback;
mod from_foreign;
mod multi;
mod object_receivers;

const DEP: &str = "dep.py";

/// Lowers `source` as a standalone module (no project imports), the way
/// the driver lowers a dependency before its importer.
fn lower_dependency(source: &str) -> HirModule {
    lower_dependency_module(source).hir
}

/// [`lower_dependency`], keeping the whole [`LoweredModule`] (its
/// `__slots__` side table in particular, #1368).
fn lower_dependency_module(source: &str) -> LoweredModule {
    lower_module(&parse(source), &ResolvedImports::default(), None)
        .expect("a dependency fixture must lower")
}

/// One already-loaded dependency, registered under [`DEP`].
struct Fixture {
    origin: HirModule,
    /// The dependency's `__slots__` side table (#1368).
    class_slots: Vec<ClassSlotsRow>,
}

impl Fixture {
    fn new(source: &str) -> Self {
        let lowered = lower_dependency_module(source);
        Self {
            origin: lowered.hir,
            class_slots: lowered.class_slots,
        }
    }

    /// Answers every project import request in `source` with this
    /// dependency, offering `submodules` as its submodule names.
    fn lower(&self, source: &str, submodules: &[&str]) -> Result<LoweredModule, Vec<Diagnostic>> {
        let parsed = parse(source);
        let mut resolved = ResolvedImports::default();
        resolved.add_module(DEP.to_string(), &self.origin, &self.class_slots);
        for request in project_import_requests(&parsed) {
            resolved.insert(
                request.span,
                ResolvedImport::Module(ResolvedModule {
                    display_path: DEP.to_string(),
                    hir: &self.origin,
                    submodule_names: submodules.iter().map(|name| (*name).to_string()).collect(),
                }),
            );
        }
        lower_module(&parsed, &resolved, None)
    }

    fn lower_ok(&self, source: &str) -> LoweredModule {
        self.lower(source, &[])
            .unwrap_or_else(|diagnostics| panic!("must lower: {:?}", diagnostics[0].message))
    }

    fn first_error(&self, source: &str, submodules: &[&str]) -> Diagnostic {
        let mut diagnostics = self
            .lower(source, submodules)
            .expect_err("fixture must fail to lower");
        diagnostics.remove(0)
    }
}

/// Lowers `source` with every project import answered by `answer`.
fn lower_with_answer(source: &str, answer: &ResolvedImport<'_>) -> Vec<Diagnostic> {
    let parsed = parse(source);
    let mut resolved = ResolvedImports::default();
    for request in project_import_requests(&parsed) {
        resolved.insert(request.span, answer.clone());
    }
    lower_module(&parsed, &resolved, None).expect_err("fixture must fail to lower")
}

const DEFINITIONS: &str = "class Point:\n    def __init__(self, x: int) -> None:\n        \
                           self.x = x\n\n\ntype Alias = int\n\n\ndef helper(n: int) -> int:\n    \
                           return n\n\n\nvalue = 7\n";

fn binding_kinds(module: &LoweredModule) -> Vec<(&str, ProjectBindingKind)> {
    module
        .hir
        .imports
        .iter()
        .filter_map(|binding| match binding {
            ImportBinding::Project {
                local_name, kind, ..
            } => Some((local_name.as_str(), *kind)),
            _ => None,
        })
        .collect()
}

#[test]
fn every_project_binding_kind_binds_from_a_resolved_dependency() {
    let fixture = Fixture::new(DEFINITIONS);
    let lowered = fixture.lower_ok(
        "from dep import Point, Alias, helper, value\n\np = Point(1)\nn: Alias = helper(value)\n",
    );
    assert_eq!(
        binding_kinds(&lowered),
        vec![
            ("Point", ProjectBindingKind::Class),
            ("Alias", ProjectBindingKind::TypeAlias),
            ("helper", ProjectBindingKind::Function),
            ("value", ProjectBindingKind::Variable),
        ]
    );
    // The copies the importer needed in scope are stripped again, so
    // `program::link` sees each definition exactly once.
    assert!(
        !lowered
            .hir
            .class_defs
            .iter()
            .any(|(name, _)| name == "Point")
    );
    assert!(
        !lowered
            .hir
            .type_aliases
            .iter()
            .any(|(name, _)| name == "Alias")
    );
    assert!(
        lowered
            .definition_spans
            .iter()
            .all(|(name, _)| name != "Point")
    );
}

#[test]
fn a_name_the_dependency_does_not_define_is_a_t0021_import_error() {
    let fixture = Fixture::new(DEFINITIONS);
    let diagnostic = fixture.first_error("from dep import Pont\n", &[]);
    assert_eq!(diagnostic.code, "T0021");
    assert_eq!(
        diagnostic.message,
        "module `dep` (`dep.py`) has no top-level name `Pont`"
    );
}

#[test]
fn a_relative_import_with_no_module_names_the_package_in_its_missing_name_error() {
    let fixture = Fixture::new(DEFINITIONS);
    let diagnostic = fixture.first_error("from . import Pont\n", &[]);
    assert_eq!(diagnostic.code, "T0021");
    assert_eq!(
        diagnostic.message,
        "package `dep.py` has no top-level name `Pont`"
    );
}

#[test]
fn importing_a_submodule_name_is_a_capability_gap_not_a_missing_name() {
    let fixture = Fixture::new(DEFINITIONS);
    let diagnostic = fixture.first_error("from dep import sub\n", &["sub"]);
    assert_eq!(diagnostic.code, "C0001");
    assert_eq!(
        diagnostic.message,
        "module namespace bindings (`from pkg import submodule`) are not supported yet"
    );
}

#[test]
fn a_bare_import_of_a_resolved_project_module_names_the_module_namespace_gap() {
    let diagnostics = lower_with_answer("import geometry\n", &ResolvedImport::Found);
    assert_eq!(diagnostics[0].code, "C0001");
    assert_eq!(
        diagnostics[0].message,
        "module namespace bindings (`import geometry`) are not supported yet"
    );
}

#[test]
fn a_from_import_answered_with_found_falls_back_to_the_single_file_path() {
    // `Found` is only ever the answer to a bare `import m`; a `from`
    // import carrying it must lower exactly as it would unanswered.
    let diagnostics = lower_with_answer("from nowhere import x\n", &ResolvedImport::Found);
    assert_eq!(diagnostics[0].code, "C0001");
    assert_eq!(
        diagnostics[0].message,
        "import of module `nowhere` is not supported yet"
    );
}

#[test]
fn a_not_found_answer_is_reported_verbatim_at_the_statement_span() {
    let source = "from a.b import x\n";
    let diagnostics = lower_with_answer(
        source,
        &ResolvedImport::NotFound {
            code: "E0108",
            message: "import cycle: `a.py` -> `b.py` -> `a.py`".to_string(),
        },
    );
    assert_eq!(diagnostics[0].code, "E0108");
    assert_eq!(
        diagnostics[0].message,
        "import cycle: `a.py` -> `b.py` -> `a.py`"
    );
    assert_eq!(
        diagnostics[0].span,
        Some(Span::new(0, source.trim_end().len() as u32))
    );
}

#[test]
fn a_synthetic_builtin_exception_class_is_not_importable() {
    // The dependency raises, so HIR lowering seeded the builtin
    // exception classes into its class table; they are not definitions the
    // module can re-export.
    let fixture = Fixture::new("def f() -> int:\n    raise ValueError\n");
    let diagnostic = fixture.first_error("from dep import ValueError\n", &[]);
    assert_eq!(diagnostic.code, "T0021");
    assert_eq!(
        diagnostic.message,
        "module `dep` (`dep.py`) has no top-level name `ValueError`"
    );
}

#[test]
fn an_imported_class_arrives_with_its_whole_ancestor_chain() {
    let fixture = Fixture::new(
        "class Base:\n    def __init__(self, x: int) -> None:\n        self.x = x\n\n\nclass Sub\
         (Base):\n    def tag(self) -> int:\n        return self.x\n",
    );
    // `Base` is never imported by name, but `Sub`'s MRO needs it, so the
    // copy walks the whole chain.
    let lowered = fixture.lower_ok(
        "from dep import Sub\n\n\nclass Deeper(Sub):\n    def more(self) -> int:\n        \
         return self.tag()\n",
    );
    let deeper = lowered
        .hir
        .class_defs
        .iter()
        .find(|(name, _)| name == "Deeper")
        .expect("the local subclass is defined here");
    assert_eq!(deeper.1.mro, vec!["Deeper", "Sub", "Base"]);
}

#[test]
fn an_imported_exception_subclass_can_be_subclassed_again() {
    let fixture = Fixture::new("class MyError(ValueError):\n    pass\n");
    let lowered =
        fixture.lower_ok("from dep import MyError\n\n\nclass Worse(MyError):\n    pass\n");
    let worse = lowered
        .hir
        .class_defs
        .iter()
        .find(|(name, _)| name == "Worse")
        .expect("the local subclass is defined here");
    assert_eq!(worse.1.mro[..2], ["Worse", "MyError"]);
}

#[test]
fn defining_a_class_the_module_already_imported_collides_with_the_import() {
    let fixture = Fixture::new(DEFINITIONS);
    let diagnostic = fixture.first_error(
        "from dep import Point\n\n\nclass Point:\n    def __init__(self) -> None:\n        \
         self.y = 1\n",
        &[],
    );
    assert_eq!(diagnostic.code, "C0001");
    assert!(
        diagnostic.message.contains("import of the same name"),
        "{}",
        diagnostic.message
    );
}

#[test]
fn a_re_export_is_followed_one_hop_to_the_module_that_defines_the_name() {
    let base_module = lower_dependency_module(DEFINITIONS);
    let base = base_module.hir;
    // `pkg/__init__.py` re-exports every kind of binding, including a
    // stdlib symbol it imported itself.
    let package_source = "from base import Point, Alias, helper, value\nfrom math import sqrt\n";
    let parsed = parse(package_source);
    let mut package_resolved = ResolvedImports::default();
    package_resolved.add_module("base.py".to_string(), &base, &base_module.class_slots);
    for request in project_import_requests(&parsed) {
        package_resolved.insert(
            request.span,
            ResolvedImport::Module(ResolvedModule {
                display_path: "base.py".to_string(),
                hir: &base,
                submodule_names: Vec::new(),
            }),
        );
    }
    let package_module =
        lower_module(&parsed, &package_resolved, None).expect("the package fixture must lower");
    let package = package_module.hir;

    let importer_source = "from pkg import Point, Alias, helper, value, sqrt\n\np = Point(1)\nn: Alias = \
         helper(value)\nr: float = sqrt(4.0)\n";
    let importer = parse(importer_source);
    let mut resolved = ResolvedImports::default();
    resolved.add_module("base.py".to_string(), &base, &base_module.class_slots);
    resolved.add_module(
        "pkg/__init__.py".to_string(),
        &package,
        &package_module.class_slots,
    );
    for request in project_import_requests(&importer) {
        resolved.insert(
            request.span,
            ResolvedImport::Module(ResolvedModule {
                display_path: "pkg/__init__.py".to_string(),
                hir: &package,
                submodule_names: Vec::new(),
            }),
        );
    }
    let lowered = lower_module(&importer, &resolved, None).expect("the re-export chain must lower");
    // The class and alias were copied from `base.py`, the module that
    // actually defines them, and every binding still points there.
    assert_eq!(
        binding_kinds(&lowered),
        vec![
            ("Point", ProjectBindingKind::Class),
            ("Alias", ProjectBindingKind::TypeAlias),
            ("helper", ProjectBindingKind::Function),
            ("value", ProjectBindingKind::Variable),
        ]
    );
    assert!(
        lowered
            .hir
            .imports
            .iter()
            .any(|binding| matches!(binding, ImportBinding::Symbol { local_name, .. } if local_name == "sqrt"))
    );
}

#[test]
fn project_import_requests_skips_everything_the_stdlib_registry_answers() {
    let module = parse(
        "import math\nfrom math import sqrt\nimport geometry\nimport a, b\nimport geometry as g\n\
         from .rel import x\nfrom pkg.sub import y\nvalue = 1\n",
    );
    let requests = project_import_requests(&module);
    let shapes: Vec<(u32, Option<&str>, usize)> = requests
        .iter()
        .map(|request| {
            (
                request.level,
                request.module.as_deref(),
                request.names.len(),
            )
        })
        .collect();
    assert_eq!(
        shapes,
        vec![
            (0, Some("geometry"), 0),
            // #1280: a multi-name `import a, b` asks one request per
            // qualifying alias, each keyed by that alias's own span.
            (0, Some("a"), 0),
            (0, Some("b"), 0),
            // #1291: an aliased `import geometry as g` is requested too, so
            // the driver can answer it (a project module is still refused by
            // lowering until #964; a CPython-backed one binds `g`).
            (0, Some("geometry"), 0),
            (1, Some("rel"), 1),
            (0, Some("pkg.sub"), 1),
        ]
    );
    let statement = "import math\nfrom math import sqrt\nimport geometry\n".len() as u32;
    let alias = |offset: u32| Span::new(statement + offset, statement + offset + 1);
    assert_eq!(requests[1].span, alias("import ".len() as u32));
    assert_eq!(requests[2].span, alias("import a, ".len() as u32));
    // #1291: the aliased request is keyed by its own alias span.
    let aliased = "import math\nfrom math import sqrt\nimport geometry\nimport a, b\n".len() as u32
        + "import ".len() as u32;
    assert_eq!(requests[3].span.start, aliased);
}

#[test]
fn project_import_requests_skips_a_future_import() {
    // `__future__` is not a registry answer, so without D-229's early
    // return the directive would be forwarded to the driver, which would
    // load a sibling `__future__.py` and bind its names. A bare
    // `import __future__` is not the directive and is still requested.
    let module = parse("from __future__ import annotations\nimport __future__\n");
    let requests = project_import_requests(&module);
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].level, 0);
    assert_eq!(requests[0].module.as_deref(), Some("__future__"));
    assert!(requests[0].names.is_empty());
    // A relative `from .__future__ import x` is an ordinary project import.
    let module = parse("from .__future__ import annotations\n");
    assert_eq!(project_import_requests(&module).len(), 1);
}

#[test]
fn a_wildcard_import_of_a_resolved_dependency_is_rejected() {
    let fixture = Fixture::new(DEFINITIONS);
    let diagnostic = fixture.first_error("from dep import *\n", &[]);
    assert_eq!(diagnostic.code, "C0001");
    assert_eq!(
        diagnostic.message,
        "`from ... import *` (wildcard import) is not supported yet"
    );
}

#[test]
fn aliasing_a_name_imported_from_a_resolved_dependency_is_rejected() {
    let fixture = Fixture::new(DEFINITIONS);
    let diagnostic = fixture.first_error("from dep import helper as h\n", &[]);
    assert_eq!(diagnostic.code, "C0001");
    assert_eq!(
        diagnostic.message,
        "`from ... import x as y` aliasing is not supported yet"
    );
}

#[test]
fn two_imported_classes_sharing_an_ancestor_both_bind() {
    // Each `from` statement copies its class *with its ancestors*, so the
    // second statement meets `ValueError` already in this module's table.
    // Both classes must still bind.
    // The dedup guard in `module.rs` that skips the second copy has no
    // observable effect in Part 1 (`strip_imported` removes every
    // imported entry again), so this covers the reachability of the
    // path, not the guard itself.
    let fixture =
        Fixture::new("class Left(ValueError):\n    pass\n\n\nclass Right(ValueError):\n    pass\n");
    let lowered = fixture.lower_ok("from dep import Left\nfrom dep import Right\n");
    assert_eq!(
        binding_kinds(&lowered),
        vec![
            ("Left", ProjectBindingKind::Class),
            ("Right", ProjectBindingKind::Class),
        ]
    );
}

#[test]
fn a_type_alias_reached_through_two_modules_lowers_and_binds() {
    // `Alias` arrives both from the module that defines it and from the
    // module that re-exports it, so the importer meets the same alias
    // twice and must still lower and bind it.
    // The dedup guard in `module.rs` that skips the second copy has no
    // observable effect in Part 1 (`strip_imported` removes every
    // imported entry again), so this covers the reachability of the
    // path, not the guard itself.
    let defining =
        lower_dependency("type Alias = int\n\n\ndef helper(n: Alias) -> Alias:\n    return n\n");
    let reexport = {
        let parsed = parse("from defs import Alias, helper\n");
        let mut resolved = ResolvedImports::default();
        resolved.add_module("defs.py".to_string(), &defining, &[]);
        for request in project_import_requests(&parsed) {
            resolved.insert(
                request.span,
                ResolvedImport::Module(ResolvedModule {
                    display_path: "defs.py".to_string(),
                    hir: &defining,
                    submodule_names: Vec::new(),
                }),
            );
        }
        lower_module(&parsed, &resolved, None)
            .expect("the re-exporting fixture must lower")
            .hir
    };
    let parsed =
        parse("from defs import Alias\nfrom mid import Alias, helper\n\nn: Alias = helper(1)\n");
    let mut resolved = ResolvedImports::default();
    resolved.add_module("defs.py".to_string(), &defining, &[]);
    resolved.add_module("mid.py".to_string(), &reexport, &[]);
    for request in project_import_requests(&parsed) {
        let (display_path, hir) = match request.module.as_deref() {
            Some("defs") => ("defs.py", &defining),
            _ => ("mid.py", &reexport),
        };
        resolved.insert(
            request.span,
            ResolvedImport::Module(ResolvedModule {
                display_path: display_path.to_string(),
                hir,
                submodule_names: Vec::new(),
            }),
        );
    }
    let lowered = lower_module(&parsed, &resolved, None).expect("the importer must lower");
    assert_eq!(
        binding_kinds(&lowered),
        vec![
            ("Alias", ProjectBindingKind::TypeAlias),
            ("Alias", ProjectBindingKind::TypeAlias),
            ("helper", ProjectBindingKind::Function),
        ]
    );
}

#[test]
fn an_imported_alias_to_a_class_the_importer_never_copied_still_accepts_a_subscript() {
    // #931 step 2b: `bind_project_name`'s TypeAlias branch pushes the alias
    // (`A` -> `Ty::Instance("G")`) but only the Class branch runs
    // `copy_class_with_ancestors`, so `G` is absent from the importer's
    // `class_defs`. The subscript arm resolves `A` through the alias table
    // to a class-shaped `Ty` it cannot look up, and fails open exactly as it
    // did before #931 rather than rejecting `A[int]` as a non-class alias.
    // The missing copy itself is a pre-existing #881-area gap.
    let fixture = Fixture::new(
        "class G[T]:\n    def __init__(self) -> None:\n        self.v = 1\n\ntype A = G\n",
    );
    let lowered = fixture.lower_ok("from dep import A\n\ndef f(x: A[int]) -> int:\n    return 1\n");
    let HirItem::Function { params, .. } = &lowered.hir.items[0] else {
        panic!("expected a function item: {:?}", lowered.hir.items[0]);
    };
    assert_eq!(params[0].1, Ty::Instance(Box::new("G".to_string())));
    // Proof that `G` really is absent here: the bare name is unknown.
    let diagnostic = fixture.first_error(
        "from dep import A\n\ndef f(x: G) -> int:\n    return 1\n",
        &[],
    );
    assert_eq!(diagnostic.code, "C0001", "{}", diagnostic.message);
    assert!(
        diagnostic
            .message
            .contains("type annotation `G` is not supported yet"),
        "{}",
        diagnostic.message
    );
}

/// A dependency that binds `name` to a CPython module object: `source`'s
/// `import_stmt` is answered `ResolvedImport::Foreign`, the way the driver
/// answers an import that resolves to neither a project module nor a
/// `pycc_std` one (Part 1 of #1026).
fn foreign_dependency(source: &str, import_stmt: &str) -> HirModule {
    let parsed = parse(source);
    let name = import_stmt
        .strip_prefix("import ")
        .expect("a foreign fixture imports with a plain `import <name>`");
    // Keyed by the span `pycc_hir` requests the answer under -- the alias's
    // own span since #1280 -- rather than a re-derived one.
    let request = project_import_requests(&parsed)
        .into_iter()
        .find(|request| request.module.as_deref() == Some(name) && request.names.is_empty())
        .expect("the fixture must contain its import statement");
    let mut resolved = ResolvedImports::default();
    resolved.insert(request.span, ResolvedImport::Foreign);
    lower_module(&parsed, &resolved, None)
        .expect("a dependency fixture must lower")
        .hir
}

#[test]
fn re_exporting_a_dependency_s_foreign_import_is_refused() {
    // Finding A of the #1087 review: cloning the `Foreign` binding would
    // carry the dependency's own item index into the importer, where
    // `program::link` rebases it as though it were the importer's -- an
    // out-of-range splice in `pycc_mir` -- and would also run a second
    // CPython import for the one `import json` statement in `dep.py`.
    let fixture = Fixture {
        origin: foreign_dependency(
            "def a() -> int:\n    return 1\n\n\nimport json\n",
            "import json",
        ),
        class_slots: Vec::new(),
    };
    let source = "from dep import json\n";
    let diagnostic = fixture.first_error(source, &[]);
    assert_eq!(diagnostic.code, "C0001");
    assert_eq!(
        diagnostic.message,
        "`dep.py` binds `json` to the CPython module object `json`; re-exporting a \
         foreign import across project modules is not supported yet"
    );
    assert_eq!(
        diagnostic.span,
        Some(Span::new(0, source.trim_end().len() as u32)),
        "the diagnostic points at the importing statement"
    );
}

/// #1381: a re-exported `import xml.dom` names the module it binds, `xml`.
#[test]
fn re_exporting_a_dependency_s_dotted_import_names_its_root() {
    let fixture = Fixture {
        origin: foreign_dependency("import xml.dom\n", "import xml.dom"),
        class_slots: Vec::new(),
    };
    let diagnostic = fixture.first_error("from dep import xml\n", &[]);
    assert_eq!(
        diagnostic.message,
        "`dep.py` binds `xml` to the CPython module object `xml`; re-exporting a \
         foreign import across project modules is not supported yet"
    );
}

#[test]
fn a_dependency_with_no_preceding_items_is_refused_the_same_way() {
    // The smallest shape of the same defect, where the dependency
    // produces no item before its import. Whether the stale index landed
    // inside the importer's item vector (a silent duplicate
    // `pycc_ext_obj_import`) or past its end (a panic in `pycc_mir`)
    // depended only on how many items each side happened to have, so the
    // refusal has to cover this shape too.
    let fixture = Fixture {
        origin: foreign_dependency("import json\n", "import json"),
        class_slots: Vec::new(),
    };
    let diagnostic = fixture.first_error("from dep import json\n", &[]);
    assert_eq!(diagnostic.code, "C0001");
    assert!(
        diagnostic.message.contains("re-exporting a foreign import"),
        "{}",
        diagnostic.message
    );
}

#[test]
fn a_dependency_s_other_names_are_still_importable_alongside_a_foreign_import() {
    // The refusal is scoped to the foreign name itself: a module that also
    // does `import json` still re-exports its own definitions.
    let fixture = Fixture {
        origin: foreign_dependency(
            "import json\n\n\ndef helper(n: int) -> int:\n    return n\n",
            "import json",
        ),
        class_slots: Vec::new(),
    };
    let lowered = fixture.lower_ok("from dep import helper\n\nn: int = helper(1)\n");
    assert_eq!(
        binding_kinds(&lowered),
        vec![("helper", ProjectBindingKind::Function)]
    );
}
