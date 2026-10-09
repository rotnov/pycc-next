//! Unit tests of the shared MRO helpers and `binds_name` (#1345). The
//! end-to-end behaviour is pinned through the CLI in
//! `tests/issue_1284_foreign_static_class_attr.rs`.

use super::*;
use crate::{
    HirModule, ResolvedImport, ResolvedImports, lower_module, program, project_import_requests,
};
use pycc_diag::Diagnostic;
use std::collections::HashMap;

/// Lowers `source` with every import request answered as a foreign module,
/// returning the first diagnostic on failure.
pub(crate) fn lower_foreign(source: &str) -> Result<HirModule, Diagnostic> {
    let parsed = crate::pycc_parser_test_helper::parse(source);
    let mut resolved = ResolvedImports::default();
    for request in project_import_requests(&parsed) {
        resolved.insert(request.span, ResolvedImport::Foreign);
    }
    let first = |diagnostics: Vec<Diagnostic>| {
        diagnostics
            .into_iter()
            .next()
            .expect("an Err is never empty")
    };
    let lowered = lower_module(&parsed, &resolved, None).map_err(first)?;
    program::finalize(lowered.hir).map_err(first)
}

/// The class table of `source`, keyed by name.
fn classes(source: &str) -> HashMap<String, HirClassDef> {
    lower_foreign(source)
        .expect("fixture must lower")
        .class_defs
        .into_iter()
        .collect()
}

const FS: &str = "import os\n\n\nclass FS:\n    exists = staticmethod(os.path.exists)\n\n\n";

fn fixture(rest: &str) -> HashMap<String, HirClassDef> {
    classes(&format!("{FS}{rest}"))
}

fn mro<'a>(table: &'a HashMap<String, HirClassDef>, class: &str) -> &'a [String] {
    &table[class].mro
}

#[test]
fn read_and_call_expressions_rebuild_the_reference() {
    let dotted = ForeignCallableRef {
        root: "os".to_string(),
        path: vec!["path".to_string(), "exists".to_string()],
    };
    let os_path = HirExpr::AttrGet {
        base: Box::new(HirExpr::Name("os".to_string())),
        attr: "path".to_string(),
    };
    assert_eq!(
        dotted.read_expr(),
        HirExpr::AttrGet {
            base: Box::new(os_path.clone()),
            attr: "exists".to_string(),
        }
    );
    let arg = HirExpr::Name("p".to_string());
    assert_eq!(
        dotted.call_expr(vec![arg.clone()]),
        HirExpr::MethodCall {
            base: Box::new(os_path),
            method: "exists".to_string(),
            args: vec![arg.clone()],
        }
    );
    let bare = ForeignCallableRef {
        root: "add".to_string(),
        path: Vec::new(),
    };
    assert_eq!(bare.read_expr(), HirExpr::Name("add".to_string()));
    assert_eq!(
        bare.call_expr(vec![arg.clone()]),
        HirExpr::Call {
            callee: "add".to_string(),
            args: vec![arg],
        }
    );
}

#[test]
fn the_class_namespace_winner_is_positional() {
    let table = fixture(
        "class A:\n    exists = 1\n\n\nclass M:\n    def exists(self) -> int:\n        return 1\n\n\n\
         class D1(FS):\n    pass\n\n\nclass D2(A, FS):\n    pass\n\n\nclass D3(M, FS):\n    pass\n",
    );
    let lookup = |name: &str| table.get(name);
    assert!(matches!(
        class_namespace_winner(mro(&table, "D1"), lookup, "exists"),
        Some(ClassNamespaceWinner::ForeignStatic { owner: "FS", .. })
    ));
    assert_eq!(
        class_namespace_winner(mro(&table, "D2"), lookup, "exists"),
        Some(ClassNamespaceWinner::Other { owner: "A" })
    );
    assert_eq!(
        class_namespace_winner(mro(&table, "D3"), lookup, "exists"),
        Some(ClassNamespaceWinner::Other { owner: "M" })
    );
    assert_eq!(
        class_namespace_winner(mro(&table, "D1"), lookup, "nope"),
        None
    );
    // A class the lookup does not know is skipped.
    let only_fs = |name: &str| (name == "FS").then(|| &table["FS"]);
    assert!(class_name_foreign_static(mro(&table, "D3"), only_fs, "exists").is_some());
    assert!(class_name_foreign_static(mro(&table, "D3"), lookup, "exists").is_none());
}

#[test]
fn an_instance_slot_anywhere_in_the_mro_declines_the_instance_path() {
    let table = fixture(
        "class S:\n    def __init__(self) -> None:\n        self.exists = 1\n\n\n\
         class D(S, FS):\n    pass\n",
    );
    let lookup = |name: &str| table.get(name);
    assert!(mro_has_instance_slot(mro(&table, "D"), lookup, "exists"));
    assert!(instance_foreign_static(mro(&table, "D"), lookup, "exists").is_none());
    assert!(class_name_foreign_static(mro(&table, "D"), lookup, "exists").is_some());
    assert!(instance_foreign_static(mro(&table, "FS"), lookup, "exists").is_some());
    assert!(!method_shadows_foreign_static(
        mro(&table, "D"),
        lookup,
        "exists"
    ));
}

#[test]
fn rule_seven_fires_only_for_a_method_kind_winner_before_a_foreign_entry() {
    let table = fixture(
        "from typing import Protocol\n\n\n\
         class M:\n    def exists(self) -> int:\n        return 1\n\n\n\
         class St:\n    @staticmethod\n    def exists() -> int:\n        return 1\n\n\n\
         class Cm:\n    @classmethod\n    def exists(cls) -> int:\n        return 1\n\n\n\
         class P(Protocol):\n    def exists(self) -> int: ...\n\n\n\
         class L:\n    exists = 1\n\n\n\
         class DM(M, FS):\n    pass\n\n\nclass DS(St, FS):\n    pass\n\n\n\
         class DC(Cm, FS):\n    pass\n\n\nclass DP(P, FS):\n    pass\n\n\n\
         class DL(L, FS):\n    pass\n\n\nclass MF(FS, M):\n    pass\n",
    );
    let lookup = |name: &str| table.get(name);
    for class in ["DM", "DS", "DC", "DP"] {
        assert!(
            method_shadows_foreign_static(mro(&table, class), lookup, "exists"),
            "{class}"
        );
    }
    // A literal winner, a foreign winner, a plain method with no foreign
    // entry after it, and a missing name all decline.
    assert!(!method_shadows_foreign_static(
        mro(&table, "DL"),
        lookup,
        "exists"
    ));
    assert!(!method_shadows_foreign_static(
        mro(&table, "MF"),
        lookup,
        "exists"
    ));
    assert!(!method_shadows_foreign_static(
        mro(&table, "M"),
        lookup,
        "exists"
    ));
    assert!(!method_shadows_foreign_static(
        mro(&table, "DM"),
        lookup,
        "nope"
    ));
}

#[test]
fn subclass_divergence_names_a_subclass_with_a_different_winner() {
    let table = fixture(
        "class D1(FS):\n    pass\n\n\n\
         class D2(FS):\n    def exists(self) -> int:\n        return 1\n\n\n\
         class Plain:\n    def run(self) -> int:\n        return 1\n\n\n\
         class Sub(Plain):\n    def run(self) -> int:\n        return 2\n",
    );
    let lookup = |name: &str| table.get(name);
    let mut all: Vec<&HirClassDef> = table.values().collect();
    all.sort_by(|a, b| a.name.cmp(&b.name));
    assert_eq!(
        subclass_divergence("FS", all.iter().copied(), lookup, "exists"),
        Some("D2")
    );
    // D1 inherits the same winner; an override with no foreign entry on
    // either side is not this refusal's business.
    assert_eq!(
        subclass_divergence("D1", all.iter().copied(), lookup, "exists"),
        None
    );
    assert_eq!(
        subclass_divergence("Plain", all.iter().copied(), lookup, "run"),
        None
    );
    assert_eq!(
        subclass_divergence("Nope", all.iter().copied(), lookup, "exists"),
        None
    );
}

#[test]
fn subclass_divergence_ignores_a_name_the_receiver_class_never_binds() {
    // `Base` binds no `exists`; only its subclass carries the foreign
    // attribute. That is an ordinary unknown attribute of `Base`, so the
    // subclass-override refusal stays out of it.
    let table = fixture("class Base:\n    pass\n\n\nclass Sub(Base, FS):\n    pass\n");
    let lookup = |name: &str| table.get(name);
    let all: Vec<&HirClassDef> = table.values().collect();
    assert_eq!(
        instance_winner(mro(&table, "Sub"), lookup, "exists"),
        InstanceWinner::Foreign("FS")
    );
    assert_eq!(
        subclass_divergence("Base", all.iter().copied(), lookup, "exists"),
        None
    );
}

#[test]
fn an_instance_winner_classifies_slot_foreign_other_and_missing() {
    let table = fixture(
        "class S:\n    def __init__(self) -> None:\n        self.exists = 1\n\n\n\
         class D5(S, FS):\n    pass\n\n\n\
         class Base:\n    pass\n\n\nclass Sub(Base):\n    exists = 1\n",
    );
    let lookup = |name: &str| table.get(name);
    let all: Vec<&HirClassDef> = table.values().collect();
    // FS's foreign winner against D5's slot.
    assert_eq!(
        subclass_divergence("FS", all.iter().copied(), lookup, "exists"),
        Some("D5")
    );
    // Missing against a literal: no foreign entry, no refusal.
    assert_eq!(
        subclass_divergence("Base", all.iter().copied(), lookup, "exists"),
        None
    );
    assert_eq!(
        instance_winner(mro(&table, "Base"), lookup, "exists"),
        InstanceWinner::Missing
    );
    assert_eq!(
        instance_winner(mro(&table, "Sub"), lookup, "exists"),
        InstanceWinner::Other("Sub")
    );
    assert_eq!(
        instance_winner(mro(&table, "D5"), lookup, "exists"),
        InstanceWinner::Slot
    );
    assert_eq!(
        instance_winner(mro(&table, "FS"), lookup, "exists"),
        InstanceWinner::Foreign("FS")
    );
}

fn binds(source: &str, name: &str) -> bool {
    let module = crate::pycc_parser_test_helper::parse(source);
    binds_name(&module.body, &[], name)
}

#[test]
fn binds_name_sees_every_binding_statement_but_not_nested_scopes() {
    for source in [
        "def x() -> None:\n    pass\n",
        "class x:\n    pass\n",
        "import x\n",
        "import x.y\n",
        "import a as x\n",
        "from a import x\n",
        "from a import b as x\n",
        "x = 1\n",
        "if True:\n    def x() -> None:\n        pass\n",
        "for x in range(2):\n    pass\n",
    ] {
        assert!(binds(source, "x"), "{source:?}");
    }
    for source in [
        "def f() -> None:\n    x = 1\n",
        "class C:\n    def x(self) -> None:\n        pass\n",
        "import y as z\n",
        "import a.x\n",
        "from a import x as y\n",
        "print(x)\n",
    ] {
        assert!(!binds(source, "x"), "{source:?}");
    }
}
