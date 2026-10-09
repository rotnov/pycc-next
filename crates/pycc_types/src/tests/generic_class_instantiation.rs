//! PEP 695 generic-class instantiation unit tests for the type-checking
//! crate root.
//!
//! Extracted verbatim from `tests.rs` under AGENTS.md's decomposability rule
//! (part of #695, which tracks decomposing that oversized file). These are the
//! #387 tests that drive the full `check_and_resolve` -> `monomorphize` ->
//! `instantiate_generic_class_methods` pipeline for a generic class, plus two
//! `check`-only coverage tests for the `GenericClassInstantiate` arm — its
//! undefined-class rejection and the `reject_generic_calls_in_expr` traversal
//! arm — which stop at `check` and never reach that pipeline. Their
//! fixture helper `generic_class_module_with_call` stays in the parent,
//! because `tests/generic_method_instantiation.rs` uses it too and sibling
//! child modules cannot see each other's private items. As a child module this
//! still sees the parent's private items directly through `use super::*`, so
//! nothing needed widened visibility; only the tests' location changed.

use super::*;

// -- PEP 695 generic class instantiation (#387) coverage --------------

#[test]
fn check_and_resolve_monomorphizes_a_generic_class_instantiation() {
    let hir = generic_class_module_with_call();
    // `check` must accept the module.
    assert!(check(&hir).is_ok());
    // `check_and_resolve` must monomorphize it.
    let resolved = check_and_resolve(&hir).unwrap();
    // The original `C.__init__` should be dropped (it's generic), and
    // the monomorphized `0gen_C__T_int.__init__` should appear.
    assert!(find_function(&resolved, "C.__init__").is_none());
    assert!(
        find_function(&resolved, "0gen_C__T_int.__init__").is_some(),
        "monomorphized __init__ should exist"
    );
    // The GenericClassInstantiate should be rewritten to an ordinary
    // Call. The rewritten top-level statement keeps its original
    // position; monomorphized functions are appended after.
    assert!(resolved.items.iter().any(|item| matches!(
        item,
        HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call { callee, .. }))
        if callee == "0gen_C__T_int"
    )));
    // The monomorphized class def should be in class_defs.
    assert!(
        resolved
            .class_defs
            .iter()
            .any(|(name, _)| name == "0gen_C__T_int"),
        "monomorphized class def should exist"
    );
}

// Bug 2 (#387): a generic class whose methods have no `Ty::Param` in
// their signatures (e.g. `class Marker[T]: def __init__(self) -> None:
// self.x = 0`) must still be monomorphized. Before the fix,
// `monomorphize`'s early return (`generics.is_empty()`) skipped
// `instantiate_generic_class_methods` entirely, leaving the
// `GenericClassInstantiate` expression unrewritten and panicking at MIR
// lowering.
#[test]
fn check_and_resolve_monomorphizes_a_generic_class_with_no_param_in_methods() {
    let class_def = HirClassDef {
        class_attrs: Vec::new(),
        exception_type_tag: None,
        name: "Marker".to_string(),
        bases: Vec::new(),
        mro: vec!["Marker".to_string()],
        attrs: vec![("x".to_string(), Ty::Int)],
        methods: vec![("__init__".to_string(), "Marker.__init__".to_string())],
        type_param: Some("T".to_string()),
        properties: Vec::new(),
        static_methods: Vec::new(),
        class_methods: Vec::new(),
        is_enum: false,
        implicit_object_init: false,
        method_defaults: Vec::new(),
        enum_members: Vec::new(),
        is_dataclass: false,
        dataclass_fields: Vec::new(),
        is_protocol: false,
        runtime_checkable: false,
        protocol_members: Vec::new(),
        abstract_methods: Vec::new(),
        is_abstract: false,
    };
    let init = HirItem::Function {
        name: "Marker.__init__".to_string(),
        params: vec![(
            "self".to_string(),
            Ty::Instance(Box::new("Marker".to_string())),
        )],
        return_ty: Ty::None,
        body: vec![HirStmt::AttrSet {
            base: HirExpr::Name("self".to_string()),
            attr: "x".to_string(),
            value: HirExpr::IntLiteral(0),
        }],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            init,
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::GenericClassInstantiate {
                class: "Marker".to_string(),
                type_arg: Ty::Int,
                args: vec![],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: vec![("Marker".to_string(), class_def)],
    };
    assert!(check(&hir).is_ok());
    let resolved = check_and_resolve(&hir).unwrap();
    // The monomorphized __init__ should exist.
    assert!(
        find_function(&resolved, "0gen_Marker__T_int.__init__").is_some(),
        "monomorphized __init__ should exist even with no Ty::Param in methods"
    );
    // The GenericClassInstantiate should be rewritten to an ordinary Call.
    assert!(resolved.items.iter().any(|item| matches!(
        item,
        HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call { callee, .. }))
        if callee == "0gen_Marker__T_int"
    )));
}

// Bug 3 (#387): when a generic class has a redefined method (two
// `HirItem::Function` items with the same mangled name, per #386 rebind
// semantics), `instantiate_generic_class_methods` must specialize the
// *last* matching item (the one whose body actually runs), not the first.
// Before the fix, `find` returned the first (stale, shadowed) definition.
#[test]
fn check_and_resolve_monomorphizes_the_last_redefined_method_of_a_generic_class() {
    let param = Ty::Param(Box::new("T".to_string()));
    let class_def = HirClassDef {
        class_attrs: Vec::new(),
        exception_type_tag: None,
        name: "C".to_string(),
        bases: Vec::new(),
        mro: vec!["C".to_string()],
        attrs: vec![("v".to_string(), Ty::Param(Box::new("T".to_string())))],
        methods: vec![
            ("__init__".to_string(), "C.__init__".to_string()),
            ("fetch".to_string(), "C.fetch".to_string()),
        ],
        type_param: Some("T".to_string()),
        properties: Vec::new(),
        static_methods: Vec::new(),
        class_methods: Vec::new(),
        is_enum: false,
        implicit_object_init: false,
        method_defaults: Vec::new(),
        enum_members: Vec::new(),
        is_dataclass: false,
        dataclass_fields: Vec::new(),
        is_protocol: false,
        runtime_checkable: false,
        protocol_members: Vec::new(),
        abstract_methods: Vec::new(),
        is_abstract: false,
    };
    let init = HirItem::Function {
        name: "C.__init__".to_string(),
        params: vec![
            ("self".to_string(), Ty::Instance(Box::new("C".to_string()))),
            ("v".to_string(), param.clone()),
        ],
        return_ty: Ty::None,
        body: vec![HirStmt::AttrSet {
            base: HirExpr::Name("self".to_string()),
            attr: "v".to_string(),
            value: HirExpr::Name("v".to_string()),
        }],
    };
    // First `fetch` definition — returns 1.
    let fetch_first = HirItem::Function {
        name: "C.fetch".to_string(),
        params: vec![("self".to_string(), Ty::Instance(Box::new("C".to_string())))],
        return_ty: Ty::Int,
        body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(1)))],
    };
    // Second `fetch` definition — returns 2 (the rebind, should win).
    let fetch_second = HirItem::Function {
        name: "C.fetch".to_string(),
        params: vec![("self".to_string(), Ty::Instance(Box::new("C".to_string())))],
        return_ty: Ty::Int,
        body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(2)))],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            init,
            fetch_first,
            fetch_second,
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::GenericClassInstantiate {
                class: "C".to_string(),
                type_arg: Ty::Int,
                args: vec![HirExpr::IntLiteral(42)],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: vec![("C".to_string(), class_def)],
    };
    assert!(check(&hir).is_ok());
    let resolved = check_and_resolve(&hir).unwrap();
    // The monomorphized fetch should return 2 (the last definition), not 1.
    let mono_fetch =
        find_function(&resolved, "0gen_C__T_int.fetch").expect("monomorphized fetch should exist");
    // The inner `matches!` uses a guard (`if n == 2`) rather than a bare
    // pattern so that llvm-cov would not flag the implicit `_ => false`
    // arm as an uncovered region under D-014 while these tests lived in
    // `lib.rs` — the guard's own true/false branch is the tracked region,
    // and it is exercised here.
    assert!(
        matches!(
            mono_fetch,
            HirItem::Function { body, .. }
            if matches!(&body[0], HirStmt::Return(Some(HirExpr::IntLiteral(n))) if *n == 2)
        ),
        "monomorphized fetch should use the last (rebind) definition's body"
    );
}

// #377: A generic class with a @property getter should monomorphize
// the property's getter function and copy the PropertyDef into the
// monomorphized class's property table with the mangled getter name.
#[test]
fn check_and_resolve_monomorphizes_a_generic_class_property_getter() {
    let param = Ty::Param(Box::new("T".to_string()));
    let class_def = HirClassDef {
        class_attrs: Vec::new(),
        exception_type_tag: None,
        name: "Box".to_string(),
        bases: Vec::new(),
        mro: vec!["Box".to_string()],
        attrs: vec![("_v".to_string(), param.clone())],
        methods: vec![("__init__".to_string(), "Box.__init__".to_string())],
        type_param: Some("T".to_string()),
        properties: vec![PropertyDef {
            name: "val".to_string(),
            getter: "Box.val".to_string(),
            setter: None,
        }],
        static_methods: Vec::new(),
        class_methods: Vec::new(),
        is_enum: false,
        implicit_object_init: false,
        method_defaults: Vec::new(),
        enum_members: Vec::new(),
        is_dataclass: false,
        dataclass_fields: Vec::new(),
        is_protocol: false,
        runtime_checkable: false,
        protocol_members: Vec::new(),
        abstract_methods: Vec::new(),
        is_abstract: false,
    };
    let init = HirItem::Function {
        name: "Box.__init__".to_string(),
        params: vec![
            (
                "self".to_string(),
                Ty::Instance(Box::new("Box".to_string())),
            ),
            ("v".to_string(), param.clone()),
        ],
        return_ty: Ty::None,
        body: vec![HirStmt::AttrSet {
            base: HirExpr::Name("self".to_string()),
            attr: "_v".to_string(),
            value: HirExpr::Name("v".to_string()),
        }],
    };
    let getter = HirItem::Function {
        name: "Box.val".to_string(),
        params: vec![(
            "self".to_string(),
            Ty::Instance(Box::new("Box".to_string())),
        )],
        return_ty: param.clone(),
        body: vec![HirStmt::Return(Some(HirExpr::AttrGet {
            base: Box::new(HirExpr::Name("self".to_string())),
            attr: "_v".to_string(),
        }))],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            init,
            getter,
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::GenericClassInstantiate {
                class: "Box".to_string(),
                type_arg: Ty::Int,
                args: vec![HirExpr::IntLiteral(42)],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: vec![("Box".to_string(), class_def)],
    };
    assert!(check(&hir).is_ok());
    let resolved = check_and_resolve(&hir).unwrap();
    // The monomorphized getter should exist.
    let mono_getter = find_function(&resolved, "0gen_Box__T_int.val")
        .expect("monomorphized property getter should exist");
    // Use `matches!` with a guard rather than `if let`/`let else` so that
    // llvm-cov would not flag the implicit else branch as uncovered under
    // D-014 while these tests lived in `lib.rs` — the guard's own true
    // branch is the tracked region.
    assert!(
        matches!(mono_getter, HirItem::Function { return_ty, .. } if *return_ty == Ty::Int),
        "monomorphized getter should be a Function returning Int"
    );
    // The monomorphized class should have the property in its table.
    let mono_class = resolved
        .class_defs
        .iter()
        .find(|(n, _)| n == "0gen_Box__T_int")
        .expect("monomorphized class should exist");
    assert_eq!(mono_class.1.properties.len(), 1);
    assert_eq!(mono_class.1.properties[0].name, "val");
    assert_eq!(mono_class.1.properties[0].getter, "0gen_Box__T_int.val");
    assert!(mono_class.1.properties[0].setter.is_none());
}

// #377: A generic class with a @property getter AND setter should
// monomorphize both functions and copy the PropertyDef with both
// mangled names.
#[test]
fn check_and_resolve_monomorphizes_a_generic_class_property_getter_and_setter() {
    let param = Ty::Param(Box::new("T".to_string()));
    let class_def = HirClassDef {
        class_attrs: Vec::new(),
        exception_type_tag: None,
        name: "Box".to_string(),
        bases: Vec::new(),
        mro: vec!["Box".to_string()],
        attrs: vec![("_v".to_string(), param.clone())],
        methods: vec![("__init__".to_string(), "Box.__init__".to_string())],
        type_param: Some("T".to_string()),
        properties: vec![PropertyDef {
            name: "val".to_string(),
            getter: "Box.val".to_string(),
            setter: Some("Box.val.setter".to_string()),
        }],
        static_methods: Vec::new(),
        class_methods: Vec::new(),
        is_enum: false,
        implicit_object_init: false,
        method_defaults: Vec::new(),
        enum_members: Vec::new(),
        is_dataclass: false,
        dataclass_fields: Vec::new(),
        is_protocol: false,
        runtime_checkable: false,
        protocol_members: Vec::new(),
        abstract_methods: Vec::new(),
        is_abstract: false,
    };
    let init = HirItem::Function {
        name: "Box.__init__".to_string(),
        params: vec![
            (
                "self".to_string(),
                Ty::Instance(Box::new("Box".to_string())),
            ),
            ("v".to_string(), param.clone()),
        ],
        return_ty: Ty::None,
        body: vec![HirStmt::AttrSet {
            base: HirExpr::Name("self".to_string()),
            attr: "_v".to_string(),
            value: HirExpr::Name("v".to_string()),
        }],
    };
    let getter = HirItem::Function {
        name: "Box.val".to_string(),
        params: vec![(
            "self".to_string(),
            Ty::Instance(Box::new("Box".to_string())),
        )],
        return_ty: param.clone(),
        body: vec![HirStmt::Return(Some(HirExpr::AttrGet {
            base: Box::new(HirExpr::Name("self".to_string())),
            attr: "_v".to_string(),
        }))],
    };
    let setter = HirItem::Function {
        name: "Box.val.setter".to_string(),
        params: vec![
            (
                "self".to_string(),
                Ty::Instance(Box::new("Box".to_string())),
            ),
            ("new_v".to_string(), param.clone()),
        ],
        return_ty: Ty::None,
        body: vec![HirStmt::AttrSet {
            base: HirExpr::Name("self".to_string()),
            attr: "_v".to_string(),
            value: HirExpr::Name("new_v".to_string()),
        }],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            init,
            getter,
            setter,
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::GenericClassInstantiate {
                class: "Box".to_string(),
                type_arg: Ty::Int,
                args: vec![HirExpr::IntLiteral(42)],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: vec![("Box".to_string(), class_def)],
    };
    assert!(check(&hir).is_ok());
    let resolved = check_and_resolve(&hir).unwrap();
    // Both monomorphized functions should exist.
    assert!(
        find_function(&resolved, "0gen_Box__T_int.val").is_some(),
        "monomorphized property getter should exist"
    );
    assert!(
        find_function(&resolved, "0gen_Box__T_int.val.setter").is_some(),
        "monomorphized property setter should exist"
    );
    // The monomorphized class should have the property with both names.
    let mono_class = resolved
        .class_defs
        .iter()
        .find(|(n, _)| n == "0gen_Box__T_int")
        .expect("monomorphized class should exist");
    assert_eq!(mono_class.1.properties.len(), 1);
    assert_eq!(mono_class.1.properties[0].getter, "0gen_Box__T_int.val");
    assert_eq!(
        mono_class.1.properties[0].setter.as_ref().unwrap(),
        "0gen_Box__T_int.val.setter"
    );
}

// #377: If a property's getter or setter function is in the class's
// property table but NOT in hir.items (an internal inconsistency), the
// monomorphization silently skips the missing function and still creates
// the PropertyDef with the mangled name. This covers the `if let Some`
// else paths for both getter and setter.
#[test]
fn check_and_resolve_monomorphizes_properties_with_missing_functions() {
    let class_def = HirClassDef {
        class_attrs: Vec::new(),
        exception_type_tag: None,
        name: "Box".to_string(),
        bases: Vec::new(),
        mro: vec!["Box".to_string()],
        attrs: vec![("_v".to_string(), Ty::Int)],
        methods: vec![("__init__".to_string(), "Box.__init__".to_string())],
        type_param: Some("T".to_string()),
        properties: vec![PropertyDef {
            name: "val".to_string(),
            getter: "Box.val".to_string(),
            setter: Some("Box.val.setter".to_string()),
        }],
        static_methods: Vec::new(),
        class_methods: Vec::new(),
        is_enum: false,
        implicit_object_init: false,
        method_defaults: Vec::new(),
        enum_members: Vec::new(),
        is_dataclass: false,
        dataclass_fields: Vec::new(),
        is_protocol: false,
        runtime_checkable: false,
        protocol_members: Vec::new(),
        abstract_methods: Vec::new(),
        is_abstract: false,
    };
    let init = HirItem::Function {
        name: "Box.__init__".to_string(),
        params: vec![(
            "self".to_string(),
            Ty::Instance(Box::new("Box".to_string())),
        )],
        return_ty: Ty::None,
        body: vec![HirStmt::AttrSet {
            base: HirExpr::Name("self".to_string()),
            attr: "_v".to_string(),
            value: HirExpr::IntLiteral(0),
        }],
    };
    // Note: no getter or setter HirItem::Function in hir.items —
    // the property table references them but they don't exist.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            init,
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::GenericClassInstantiate {
                class: "Box".to_string(),
                type_arg: Ty::Int,
                args: vec![],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: vec![("Box".to_string(), class_def)],
    };
    assert!(check(&hir).is_ok());
    let resolved = check_and_resolve(&hir).unwrap();
    // The PropertyDef should still be created with mangled names, even
    // though the functions themselves were not monomorphized.
    let mono_class = resolved
        .class_defs
        .iter()
        .find(|(n, _)| n == "0gen_Box__T_int")
        .expect("monomorphized class should exist");
    assert_eq!(mono_class.1.properties.len(), 1);
    assert_eq!(mono_class.1.properties[0].getter, "0gen_Box__T_int.val");
    assert_eq!(
        mono_class.1.properties[0].setter.as_ref().unwrap(),
        "0gen_Box__T_int.val.setter"
    );
    // The monomorphized getter/setter functions should NOT exist.
    assert!(find_function(&resolved, "0gen_Box__T_int.val").is_none());
    assert!(find_function(&resolved, "0gen_Box__T_int.val.setter").is_none());
}

// #377: A class_def with a duplicate property entry causes the second
// iteration to produce the same mangled getter/setter name, so
// `seen.insert` returns false and the `instantiations.push` is skipped
// for both getter and setter.
#[test]
fn check_and_resolve_dedups_property_getter_and_setter_monomorphization() {
    let param = Ty::Param(Box::new("T".to_string()));
    let class_def = HirClassDef {
        class_attrs: Vec::new(),
        exception_type_tag: None,
        name: "Box".to_string(),
        bases: Vec::new(),
        mro: vec!["Box".to_string()],
        attrs: vec![("_v".to_string(), param.clone())],
        methods: vec![("__init__".to_string(), "Box.__init__".to_string())],
        type_param: Some("T".to_string()),
        properties: vec![
            PropertyDef {
                name: "val".to_string(),
                getter: "Box.val".to_string(),
                setter: Some("Box.val.setter".to_string()),
            },
            // Duplicate entry — same property name and mangled names.
            PropertyDef {
                name: "val".to_string(),
                getter: "Box.val".to_string(),
                setter: Some("Box.val.setter".to_string()),
            },
        ],
        static_methods: Vec::new(),
        class_methods: Vec::new(),
        is_enum: false,
        implicit_object_init: false,
        method_defaults: Vec::new(),
        enum_members: Vec::new(),
        is_dataclass: false,
        dataclass_fields: Vec::new(),
        is_protocol: false,
        runtime_checkable: false,
        protocol_members: Vec::new(),
        abstract_methods: Vec::new(),
        is_abstract: false,
    };
    let init = HirItem::Function {
        name: "Box.__init__".to_string(),
        params: vec![
            (
                "self".to_string(),
                Ty::Instance(Box::new("Box".to_string())),
            ),
            ("v".to_string(), param.clone()),
        ],
        return_ty: Ty::None,
        body: vec![HirStmt::AttrSet {
            base: HirExpr::Name("self".to_string()),
            attr: "_v".to_string(),
            value: HirExpr::Name("v".to_string()),
        }],
    };
    let getter = HirItem::Function {
        name: "Box.val".to_string(),
        params: vec![(
            "self".to_string(),
            Ty::Instance(Box::new("Box".to_string())),
        )],
        return_ty: param.clone(),
        body: vec![HirStmt::Return(Some(HirExpr::AttrGet {
            base: Box::new(HirExpr::Name("self".to_string())),
            attr: "_v".to_string(),
        }))],
    };
    let setter = HirItem::Function {
        name: "Box.val.setter".to_string(),
        params: vec![
            (
                "self".to_string(),
                Ty::Instance(Box::new("Box".to_string())),
            ),
            ("new_v".to_string(), param.clone()),
        ],
        return_ty: Ty::None,
        body: vec![HirStmt::AttrSet {
            base: HirExpr::Name("self".to_string()),
            attr: "_v".to_string(),
            value: HirExpr::Name("new_v".to_string()),
        }],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            init,
            getter,
            setter,
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::GenericClassInstantiate {
                class: "Box".to_string(),
                type_arg: Ty::Int,
                args: vec![HirExpr::IntLiteral(42)],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: vec![("Box".to_string(), class_def)],
    };
    assert!(check(&hir).is_ok());
    let resolved = check_and_resolve(&hir).unwrap();
    // Exactly one monomorphized getter and one setter (deduped).
    let getter_count = count_function(&resolved, "0gen_Box__T_int.val");
    let setter_count = count_function(&resolved, "0gen_Box__T_int.val.setter");
    assert_eq!(
        getter_count, 1,
        "getter should be monomorphized exactly once"
    );
    assert_eq!(
        setter_count, 1,
        "setter should be monomorphized exactly once"
    );
}

#[test]
fn type_check_expr_rejects_generic_class_instantiate_for_undefined_class() {
    // Exercises `type_check_expr`'s `GenericClassInstantiate` arm's
    // error path (class not in `env.classes`).
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::TopLevelStmt(HirStmt::ExprStmt(
            HirExpr::GenericClassInstantiate {
                class: "NoSuchClass".to_string(),
                type_arg: Ty::Int,
                args: vec![HirExpr::IntLiteral(1)],
            },
        ))],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0001");
    assert!(err.message.contains("class `NoSuchClass` is not defined"));
}

#[test]
fn reject_generic_calls_in_expr_handles_generic_class_instantiate() {
    // Exercises `reject_generic_calls_in_expr`'s
    // `GenericClassInstantiate` arm: a generic function body that
    // contains a `GenericClassInstantiate` expression. The arm recurses
    // into args (here, a `Name` leaf), proving the arm is traversed.
    let param = Ty::Param(Box::new("T".to_string()));
    let class_def = HirClassDef {
        class_attrs: Vec::new(),
        exception_type_tag: None,
        name: "C".to_string(),
        bases: Vec::new(),
        mro: vec!["C".to_string()],
        attrs: vec![("x".to_string(), Ty::Param(Box::new("T".to_string())))],
        methods: vec![("__init__".to_string(), "C.__init__".to_string())],
        type_param: Some("T".to_string()),
        properties: Vec::new(),
        static_methods: Vec::new(),
        class_methods: Vec::new(),
        is_enum: false,
        implicit_object_init: false,
        method_defaults: Vec::new(),
        enum_members: Vec::new(),
        is_dataclass: false,
        dataclass_fields: Vec::new(),
        is_protocol: false,
        runtime_checkable: false,
        protocol_members: Vec::new(),
        abstract_methods: Vec::new(),
        is_abstract: false,
    };
    let init = HirItem::Function {
        name: "C.__init__".to_string(),
        params: vec![
            ("self".to_string(), Ty::Instance(Box::new("C".to_string()))),
            ("x".to_string(), param.clone()),
        ],
        return_ty: Ty::None,
        body: vec![HirStmt::AttrSet {
            base: HirExpr::Name("self".to_string()),
            attr: "x".to_string(),
            value: HirExpr::Name("x".to_string()),
        }],
    };
    let generic_fn = HirItem::Function {
        name: "f".to_string(),
        params: vec![("x".to_string(), param)],
        return_ty: Ty::None,
        body: vec![
            HirStmt::ExprStmt(HirExpr::GenericClassInstantiate {
                class: "C".to_string(),
                type_arg: Ty::Int,
                args: vec![HirExpr::Name("x".to_string())],
            }),
            HirStmt::Return(None),
        ],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            init,
            generic_fn,
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
                callee: "f".to_string(),
                args: vec![HirExpr::IntLiteral(1)],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: vec![("C".to_string(), class_def)],
    };
    // `check` must accept this — the GenericClassInstantiate in the
    // generic function body is not a generic *function* call, so
    // `reject_generic_calls_in_expr` should recurse and return Ok.
    assert!(check(&hir).is_ok());
}
