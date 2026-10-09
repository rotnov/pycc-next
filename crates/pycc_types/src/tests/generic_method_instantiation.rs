//! Generic-method instantiation unit tests for the type-checking crate root.
//!
//! Extracted verbatim from `tests.rs` under AGENTS.md's decomposability rule
//! (part of #695, which tracks decomposing that oversized file). These are the
//! `instantiate_generic_class_methods` edge cases and the
//! `rewrite_generic_calls_in_expr` error paths.
//! The `generic_class_module_with_call` fixture helper they share with
//! `tests/generic_class_instantiation.rs` stays in the parent, because sibling
//! child modules cannot see each other's private items. As a child module this
//! still sees the parent's private items directly through `use super::*`, so
//! nothing needed widened visibility; only the tests' location changed.

use super::*;

// -- instantiate_generic_class_methods edge cases ----------------------

#[test]
fn instantiate_generic_class_methods_skips_a_nonexistent_class() {
    // A `GenericClassInstantiate` for a class not in `class_defs`:
    // `instantiate_generic_class_methods` should `continue` past it
    // (the `let Some(class_def) = ... else { continue }` arm).
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::TopLevelStmt(HirStmt::ExprStmt(
            HirExpr::GenericClassInstantiate {
                class: "Ghost".to_string(),
                type_arg: Ty::Int,
                args: vec![HirExpr::IntLiteral(1)],
            },
        ))],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    // `check_and_resolve` should still succeed — the Ghost class is
    // not in class_defs, so `instantiate_generic_class_methods` skips
    // it. (Note: `check` itself would reject this with T0001, but
    // `monomorphize` is called after `check` passes. To exercise the
    // `continue` arm directly, we call `check_and_resolve` which runs
    // `check` first — so this test actually exercises the `check`
    // rejection path, not the `continue` in monomorphize. The
    // `continue` arm is covered by the fact that
    // `collect_generic_class_instantiations_from_stmt` collects pairs
    // from the HIR, and `instantiate_generic_class_methods` looks them
    // up — if a class is not found, it continues. But since `check`
    // already rejected undefined classes, this path is only reachable
    // if the HIR is hand-built. We test it via `check_and_resolve`
    // on a valid module where the class exists but has no type_param.)
    assert!(check(&hir).is_err());
}

#[test]
fn check_and_resolve_monomorphizes_a_generic_class_with_self_typed_method() {
    // Exercises `substitute_ty_with_class`'s `Ty::Instance` arm: a
    // method that takes `Self` (lowered to `Ty::Instance("C")`) as a
    // parameter type. At monomorphization time, `Instance("C")` must
    // be rewritten to `Instance("0gen_C__T_int")`.
    let param = Ty::Param(Box::new("T".to_string()));
    let self_ty = Ty::Instance(Box::new("C".to_string()));
    let init = HirItem::Function {
        name: "C.__init__".to_string(),
        params: vec![
            ("self".to_string(), self_ty.clone()),
            ("x".to_string(), param),
        ],
        return_ty: Ty::None,
        body: vec![HirStmt::AttrSet {
            base: HirExpr::Name("self".to_string()),
            attr: "x".to_string(),
            value: HirExpr::Name("x".to_string()),
        }],
    };
    // A method that takes `Self` as a parameter (PEP 673).
    let merge = HirItem::Function {
        name: "C.merge".to_string(),
        params: vec![
            ("self".to_string(), self_ty.clone()),
            ("other".to_string(), self_ty),
        ],
        return_ty: Ty::None,
        body: vec![HirStmt::Return(None)],
    };
    let class_def = HirClassDef {
        class_attrs: Vec::new(),
        exception_type_tag: None,
        name: "C".to_string(),
        bases: Vec::new(),
        mro: vec!["C".to_string()],
        attrs: vec![("x".to_string(), Ty::Param(Box::new("T".to_string())))],
        methods: vec![
            ("__init__".to_string(), "C.__init__".to_string()),
            ("merge".to_string(), "C.merge".to_string()),
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
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            init,
            merge,
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::GenericClassInstantiate {
                class: "C".to_string(),
                type_arg: Ty::Int,
                args: vec![HirExpr::IntLiteral(1)],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: vec![("C".to_string(), class_def)],
    };
    let resolved = check_and_resolve(&hir).unwrap();
    // The monomorphized merge method should have Instance("0gen_C__T_int")
    // for both self and other parameters.
    let mono_merge =
        find_function(&resolved, "0gen_C__T_int.merge").expect("monomorphized merge should exist");
    let expected = Ty::Instance(Box::new("0gen_C__T_int".to_string()));
    assert!(matches!(
        mono_merge,
        HirItem::Function { params, .. }
        if params[0].1 == expected && params[1].1 == expected
    ));
}

#[test]
fn check_and_resolve_monomorphizes_a_generic_class_with_control_flow_methods() {
    // Exercises `substitute_stmt_with_class`'s `If`, `While`, and
    // `ForRange` arms, plus the `other` fallback (via `ExprStmt`/
    // `Return`): a generic class with a method whose body contains
    // those statement shapes, each wrapping a nested `AnnAssign` with
    // a `Ty::Param("T")` annotation. The `ForList` arm is covered
    // by the direct `substitute_stmt_with_class_recurses_into_for_list_body`
    // unit test in `tests/generic_class_substitution.rs`.
    let param = Ty::Param(Box::new("T".to_string()));
    let self_ty = Ty::Instance(Box::new("C".to_string()));
    let init = HirItem::Function {
        name: "C.__init__".to_string(),
        params: vec![
            ("self".to_string(), self_ty.clone()),
            ("x".to_string(), param.clone()),
        ],
        return_ty: Ty::None,
        body: vec![HirStmt::AttrSet {
            base: HirExpr::Name("self".to_string()),
            attr: "x".to_string(),
            value: HirExpr::Name("x".to_string()),
        }],
    };
    let nested_ann = |marker: &str| HirStmt::AnnAssign {
        is_final: false,
        target: marker.to_string(),
        annotation: Ty::Param(Box::new("T".to_string())),
        value: None,
    };
    let process = HirItem::Function {
        name: "C.process".to_string(),
        params: vec![("self".to_string(), self_ty), ("n".to_string(), Ty::Int)],
        return_ty: Ty::None,
        body: vec![
            // If → AnnAssign in body and orelse
            HirStmt::If {
                test: HirExpr::BoolLiteral(true),
                body: vec![nested_ann("if_body")],
                orelse: vec![nested_ann("if_orelse")],
            },
            // While → AnnAssign in body
            HirStmt::While {
                test: HirExpr::BoolLiteral(true),
                body: vec![nested_ann("while_body")],
            },
            // ForRange → AnnAssign in body
            HirStmt::ForRange {
                var: "i".to_string(),
                start: HirExpr::IntLiteral(0),
                stop: HirExpr::Name("n".to_string()),
                step: HirExpr::IntLiteral(1),
                body: vec![nested_ann("for_range_body")],
            },
            // ExprStmt (other fallback)
            HirStmt::ExprStmt(HirExpr::Call {
                callee: "print".to_string(),
                args: vec![HirExpr::IntLiteral(1)],
            }),
            // Return(None) (other fallback)
            HirStmt::Return(None),
        ],
    };
    let class_def = HirClassDef {
        class_attrs: Vec::new(),
        exception_type_tag: None,
        name: "C".to_string(),
        bases: Vec::new(),
        mro: vec!["C".to_string()],
        attrs: vec![("x".to_string(), Ty::Param(Box::new("T".to_string())))],
        methods: vec![
            ("__init__".to_string(), "C.__init__".to_string()),
            ("process".to_string(), "C.process".to_string()),
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
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            init,
            process,
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::GenericClassInstantiate {
                class: "C".to_string(),
                type_arg: Ty::Int,
                args: vec![HirExpr::IntLiteral(1)],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: vec![("C".to_string(), class_def)],
    };
    let resolved = check_and_resolve(&hir).unwrap();
    // The monomorphized process method should exist with all AnnAssign
    // annotations substituted from Ty::Param("T") to Ty::Int.
    let mono_process = find_function(&resolved, "0gen_C__T_int.process")
        .expect("monomorphized process should exist");
    // Verify the function body's AnnAssign annotations were substituted.
    assert!(matches!(
        mono_process,
        HirItem::Function { body, .. }
        if matches!(&body[0], HirStmt::If { body, orelse, .. }
            if matches!(&body[0], HirStmt::AnnAssign { target, annotation, .. }
                if target == "if_body" && *annotation == Ty::Int)
            && matches!(&orelse[0], HirStmt::AnnAssign { target, annotation, .. }
                if target == "if_orelse" && *annotation == Ty::Int))
        && matches!(&body[1], HirStmt::While { body, .. }
            if matches!(&body[0], HirStmt::AnnAssign { target, annotation, .. }
                if target == "while_body" && *annotation == Ty::Int))
        && matches!(&body[2], HirStmt::ForRange { body, .. }
            if matches!(&body[0], HirStmt::AnnAssign { target, annotation, .. }
                if target == "for_range_body" && *annotation == Ty::Int))
    ));
}

#[test]
fn check_and_resolve_monomorphizes_generic_class_instantiation_inside_a_function_body() {
    // Exercises `collect_generic_class_instantiations_from_stmt`'s
    // `Function` body iteration path in `instantiate_generic_class_methods`
    // (the `HirItem::Function { body, .. }` arm at line 5472).
    let param = Ty::Param(Box::new("T".to_string()));
    let self_ty = Ty::Instance(Box::new("C".to_string()));
    let init = HirItem::Function {
        name: "C.__init__".to_string(),
        params: vec![
            ("self".to_string(), self_ty.clone()),
            ("x".to_string(), param),
        ],
        return_ty: Ty::None,
        body: vec![HirStmt::AttrSet {
            base: HirExpr::Name("self".to_string()),
            attr: "x".to_string(),
            value: HirExpr::Name("x".to_string()),
        }],
    };
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
    let caller = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![
            HirStmt::ExprStmt(HirExpr::GenericClassInstantiate {
                class: "C".to_string(),
                type_arg: Ty::Int,
                args: vec![HirExpr::IntLiteral(1)],
            }),
            HirStmt::Return(None),
        ],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![init, caller],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: vec![("C".to_string(), class_def)],
    };
    let resolved = check_and_resolve(&hir).unwrap();
    assert!(
        find_function(&resolved, "0gen_C__T_int.__init__").is_some(),
        "monomorphized __init__ should exist"
    );
    // The GenericClassInstantiate inside f's body should be rewritten.
    let f_resolved = find_function(&resolved, "f").expect("f should exist");
    assert!(matches!(
        f_resolved,
        HirItem::Function { body, .. }
        if matches!(&body[0], HirStmt::ExprStmt(HirExpr::Call { callee, .. })
            if callee == "0gen_C__T_int")
    ));
}

#[test]
fn check_and_resolve_dedupes_identical_generic_class_instantiations() {
    // Two call sites with the same (C, Int) pair should produce exactly
    // one monomorphized class, not two.
    let hir = generic_class_module_with_call();
    // Add a second call site.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: hir
            .items
            .into_iter()
            .chain(std::iter::once(HirItem::TopLevelStmt(HirStmt::ExprStmt(
                HirExpr::GenericClassInstantiate {
                    class: "C".to_string(),
                    type_arg: Ty::Int,
                    args: vec![HirExpr::IntLiteral(2)],
                },
            ))))
            .collect(),
        type_aliases: hir.type_aliases,
        imports: hir.imports,
        class_defs: hir.class_defs,
    };
    let resolved = check_and_resolve(&hir).unwrap();
    assert_eq!(
        count_function(&resolved, "0gen_C__T_int.__init__"),
        1,
        "should dedupe to one monomorphized __init__"
    );
}

#[test]
fn check_and_resolve_monomorphizes_two_different_type_args_into_distinct_classes() {
    // C[int](1) and C[str](1) should produce two monomorphized classes.
    let hir = generic_class_module_with_call();
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: hir
            .items
            .into_iter()
            .chain(std::iter::once(HirItem::TopLevelStmt(HirStmt::ExprStmt(
                HirExpr::GenericClassInstantiate {
                    class: "C".to_string(),
                    type_arg: Ty::Str,
                    args: vec![HirExpr::StringLiteral("hi".to_string())],
                },
            ))))
            .collect(),
        type_aliases: hir.type_aliases,
        imports: hir.imports,
        class_defs: hir.class_defs,
    };
    let resolved = check_and_resolve(&hir).unwrap();
    assert!(
        find_function(&resolved, "0gen_C__T_int.__init__").is_some(),
        "int specialization should exist"
    );
    assert!(
        find_function(&resolved, "0gen_C__T_str.__init__").is_some(),
        "str specialization should exist"
    );
}

// -- rewrite_generic_calls_in_expr error paths (lines 5036-5049) ------

#[test]
fn check_and_resolve_rejects_generic_class_instantiate_for_undefined_class() {
    // Exercises `rewrite_generic_calls_in_expr`'s GCI arm's class-not-found
    // error path (lines 5038-5044). A GCI for a class not in the
    // environment. `check` would reject this first, but `check_and_resolve`
    // runs `check` and then `monomorphize`. To exercise the `rewrite`
    // error path directly, we need a module where `check` passes but
    // `monomorphize`'s `rewrite_generic_calls_in_expr` finds the class
    // missing. This can happen if the class is defined but not registered
    // in the `env` used by `monomorphize`. However, `check` and
    // `monomorphize` use the same env, so this path is only reachable
    // if the HIR is hand-built without a class_def but with a GCI.
    // Since `check` rejects undefined classes first, this test verifies
    // that `check` catches it (T0001), which is the user-facing behavior.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::TopLevelStmt(HirStmt::ExprStmt(
            HirExpr::GenericClassInstantiate {
                class: "Ghost".to_string(),
                type_arg: Ty::Int,
                args: vec![HirExpr::IntLiteral(1)],
            },
        ))],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert!(check_and_resolve(&hir).is_err());
}

#[test]
fn check_and_resolve_rejects_generic_class_instantiate_for_non_generic_class() {
    // Exercises `rewrite_generic_calls_in_expr`'s GCI arm's
    // class-not-generic error path (lines 5045-5049) and
    // `instantiate_generic_class_methods`' `type_param is None` continue
    // (line 5486). A GCI for a class that exists but has no `type_param`.
    // `check` passes (the class exists), and `monomorphize` doesn't
    // return early because a generic function is present. Then:
    // - `instantiate_generic_class_methods` finds class D, sees
    //   `type_param: None`, and continues (line 5486).
    // - `rewrite_generic_calls_in_stmt` processes the top-level GCI,
    //   calls `rewrite_generic_calls_in_expr`, which finds class D in
    //   env, sees `type_param: None`, and returns T0042.
    let self_ty = Ty::Instance(Box::new("D".to_string()));
    let d_init = HirItem::Function {
        name: "D.__init__".to_string(),
        params: vec![
            ("self".to_string(), self_ty.clone()),
            ("x".to_string(), Ty::Int),
        ],
        return_ty: Ty::None,
        body: vec![HirStmt::AttrSet {
            base: HirExpr::Name("self".to_string()),
            attr: "x".to_string(),
            value: HirExpr::Name("x".to_string()),
        }],
    };
    let d_class_def = HirClassDef {
        class_attrs: Vec::new(),
        exception_type_tag: None,
        name: "D".to_string(),
        bases: Vec::new(),
        mro: vec!["D".to_string()],
        attrs: vec![("x".to_string(), Ty::Int)],
        methods: vec![("__init__".to_string(), "D.__init__".to_string())],
        type_param: None, // Not generic!
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
    // A generic function — prevents `monomorphize` from returning early.
    let identity = HirItem::Function {
        name: "identity".to_string(),
        params: vec![("x".to_string(), Ty::Param(Box::new("T".to_string())))],
        return_ty: Ty::Param(Box::new("T".to_string())),
        body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            d_init,
            identity,
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::GenericClassInstantiate {
                class: "D".to_string(),
                type_arg: Ty::Int,
                args: vec![HirExpr::IntLiteral(1)],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: vec![("D".to_string(), d_class_def)],
    };
    // `check` should pass (the class exists and the GCI is valid at
    // type-checking time — `type_check_expr` only checks class existence).
    assert!(check(&hir).is_ok());
    // `check_and_resolve` should fail (T0042: class not generic).
    let err = check_and_resolve(&hir).unwrap_err();
    assert_eq!(err.code, "T0042");
    assert!(err.message.contains("class `D` is not generic"));
}
