//! Generic-method instantiation continue/dedup unit tests for the
//! type-checking crate root.
//!
//! Extracted verbatim from `tests.rs` under AGENTS.md's decomposability rule
//! (part of #695, which tracks decomposing that oversized file). These are the
//! `instantiate_generic_class_methods` continue/dedup and `seen.insert` paths,
//! every test covering the `is_assignable` `Ty::Param` clause at line 3229
//! (the separate `from == Ty::Param` clause at line 3240 is covered in
//! `tests/generic_monomorphization_arms.rs`), and the
//! `reject_generic_calls_in_expr` generic-class-instantiate path. They carry
//! no helper of their own; the fixtures they use stay in the parent. As a
//! child module this still sees the parent's private items directly through
//! `use super::*`, so nothing needed widened visibility; only the tests'
//! location changed.

use super::*;

// -- instantiate_generic_class_methods continue/dedup paths -----------

#[test]
fn instantiate_generic_class_methods_skips_non_generic_class_instantiation() {
    // Exercises `instantiate_generic_class_methods`' `method not found`
    // continue path (line 5499). A class_def references a method name
    // that doesn't exist in `hir.items`. `instantiate_generic_class_methods`
    // finds the class_def, finds the type_param, but can't find the
    // method's HirItem, so it continues.
    let self_ty = Ty::Instance(Box::new("E".to_string()));
    let class_def = HirClassDef {
        class_attrs: Vec::new(),
        exception_type_tag: None,
        name: "E".to_string(),
        bases: Vec::new(),
        mro: vec!["E".to_string()],
        attrs: vec![("x".to_string(), Ty::Param(Box::new("T".to_string())))],
        methods: vec![
            ("__init__".to_string(), "E.__init__".to_string()),
            ("ghost".to_string(), "E.ghost".to_string()), // No matching HirItem!
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
        name: "E.__init__".to_string(),
        params: vec![
            ("self".to_string(), self_ty),
            ("x".to_string(), Ty::Param(Box::new("T".to_string()))),
        ],
        return_ty: Ty::None,
        body: vec![HirStmt::AttrSet {
            base: HirExpr::Name("self".to_string()),
            attr: "x".to_string(),
            value: HirExpr::Name("x".to_string()),
        }],
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
            init,
            identity,
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::GenericClassInstantiate {
                class: "E".to_string(),
                type_arg: Ty::Int,
                args: vec![HirExpr::IntLiteral(1)],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: vec![("E".to_string(), class_def)],
    };
    // `check` should pass (the GCI is valid), and
    // `check_and_resolve` should succeed — the missing `ghost` method
    // is simply skipped (continue at line 5499).
    assert!(check(&hir).is_ok());
    let resolved = check_and_resolve(&hir).unwrap();
    // The __init__ should be monomorphized, but ghost should not.
    assert!(find_function(&resolved, "0gen_E__T_int.__init__").is_some());
    assert!(find_function(&resolved, "0gen_E__T_int.ghost").is_none());
}

#[test]
fn rewrite_generic_calls_in_expr_rejects_undefined_class_in_gci() {
    // Exercises `rewrite_generic_calls_in_expr`'s GCI arm's class-not-found
    // error path (lines 5038-5044). This path is unreachable through
    // `check_and_resolve` because `check` already rejects undefined
    // classes. We call `rewrite_generic_calls_in_expr` directly with a
    // hand-built environment that doesn't have the class.
    let mut env = Environment::new();
    let mut expr = HirExpr::GenericClassInstantiate {
        class: "Ghost".to_string(),
        type_arg: Ty::Int,
        args: vec![HirExpr::IntLiteral(1)],
    };
    let mut instantiations = Vec::new();
    let mut seen = HashSet::new();
    let err =
        rewrite_generic_calls_in_expr(&mut env, &[], &mut expr, &mut instantiations, &mut seen)
            .unwrap_err();
    assert_eq!(err.code, "T0001");
    assert!(err.message.contains("class `Ghost` is not defined"));
}

#[test]
fn rewrite_generic_calls_in_expr_propagates_arg_error_in_gci() {
    // Exercises `rewrite_generic_calls_in_expr`'s GCI arm's `?` on the
    // recursive call for args (line 5036). A GCI whose arg is itself a
    // GCI for an undefined class — the recursive call fails with T0001.
    let mut env = Environment::new();
    let mut expr = HirExpr::GenericClassInstantiate {
        class: "C".to_string(),
        type_arg: Ty::Int,
        args: vec![HirExpr::GenericClassInstantiate {
            class: "Ghost".to_string(),
            type_arg: Ty::Int,
            args: vec![HirExpr::IntLiteral(1)],
        }],
    };
    let mut instantiations = Vec::new();
    let mut seen = HashSet::new();
    let err =
        rewrite_generic_calls_in_expr(&mut env, &[], &mut expr, &mut instantiations, &mut seen)
            .unwrap_err();
    assert_eq!(err.code, "T0001");
}

#[test]
fn instantiate_generic_class_methods_skips_class_not_in_class_defs() {
    // Exercises `instantiate_generic_class_methods`' `class not in
    // class_defs` continue path (line 5482). This path is unreachable
    // through `check_and_resolve` because `check` rejects undefined
    // classes. We call `instantiate_generic_class_methods` directly
    // with a hand-built HIR that has a GCI for a class not in
    // `class_defs`.
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
        class_defs: Vec::new(), // No class_defs!
    };
    let mut env = Environment::new();
    let mut instantiations = Vec::new();
    let mut seen = HashSet::new();
    let mut new_class_defs = Vec::new();
    // Should not panic — just continues past the Ghost pair.
    instantiate_generic_class_methods(
        &hir,
        &mut env,
        &mut instantiations,
        &mut seen,
        &mut new_class_defs,
    );
    assert!(instantiations.is_empty());
    assert!(new_class_defs.is_empty());
}

#[test]
fn instantiate_generic_class_methods_skips_non_function_method_item() {
    // The `find` filter in `instantiate_generic_class_methods` already
    // matches only `HirItem::Function` items, so the non-Function
    // continue path (formerly line 5502) was dead code and has been
    // refactored away. This test verifies that a class_def with a
    // method referencing a non-existent function name is safely
    // skipped (the `find` returns None → continue at the `else`).
    let self_ty = Ty::Instance(Box::new("F".to_string()));
    let class_def = HirClassDef {
        class_attrs: Vec::new(),
        exception_type_tag: None,
        name: "F".to_string(),
        bases: Vec::new(),
        mro: vec!["F".to_string()],
        attrs: vec![("x".to_string(), Ty::Param(Box::new("T".to_string())))],
        methods: vec![
            ("__init__".to_string(), "F.__init__".to_string()),
            ("ghost".to_string(), "F.ghost".to_string()), // No matching HirItem!
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
        name: "F.__init__".to_string(),
        params: vec![
            ("self".to_string(), self_ty),
            ("x".to_string(), Ty::Param(Box::new("T".to_string()))),
        ],
        return_ty: Ty::None,
        body: vec![HirStmt::AttrSet {
            base: HirExpr::Name("self".to_string()),
            attr: "x".to_string(),
            value: HirExpr::Name("x".to_string()),
        }],
    };
    let identity = HirItem::Function {
        name: "identity".to_string(),
        params: vec![("x".to_string(), Ty::Param(Box::new("T".to_string())))],
        return_ty: Ty::Param(Box::new("T".to_string())),
        body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            init,
            identity,
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::GenericClassInstantiate {
                class: "F".to_string(),
                type_arg: Ty::Int,
                args: vec![HirExpr::IntLiteral(1)],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: vec![("F".to_string(), class_def)],
    };
    assert!(check(&hir).is_ok());
    let resolved = check_and_resolve(&hir).unwrap();
    assert!(find_function(&resolved, "0gen_F__T_int.__init__").is_some());
    assert!(find_function(&resolved, "0gen_F__T_int.ghost").is_none());
}

#[test]
fn instantiate_generic_class_methods_skips_nonexistent_static_method_function() {
    // Exercises the `continue` at the `else` branch of the static-method
    // `rfind` in `instantiate_generic_class_methods` — the static_methods
    // table references a mangled name that has no matching `HirItem`.
    let self_ty = Ty::Instance(Box::new("F".to_string()));
    let class_def = HirClassDef {
        class_attrs: Vec::new(),
        exception_type_tag: None,
        name: "F".to_string(),
        bases: Vec::new(),
        mro: vec!["F".to_string()],
        attrs: vec![("x".to_string(), Ty::Param(Box::new("T".to_string())))],
        methods: vec![("__init__".to_string(), "F.__init__".to_string())],
        type_param: Some("T".to_string()),
        properties: Vec::new(),
        static_methods: vec![("ghost".to_string(), "F.ghost.static".to_string())],
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
        name: "F.__init__".to_string(),
        params: vec![
            ("self".to_string(), self_ty),
            ("x".to_string(), Ty::Param(Box::new("T".to_string()))),
        ],
        return_ty: Ty::None,
        body: vec![HirStmt::AttrSet {
            base: HirExpr::Name("self".to_string()),
            attr: "x".to_string(),
            value: HirExpr::Name("x".to_string()),
        }],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            init,
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::GenericClassInstantiate {
                class: "F".to_string(),
                type_arg: Ty::Int,
                args: vec![HirExpr::IntLiteral(1)],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: vec![("F".to_string(), class_def)],
    };
    assert!(check(&hir).is_ok());
    let resolved = check_and_resolve(&hir).unwrap();
    assert!(find_function(&resolved, "0gen_F__T_int.__init__").is_some());
    assert!(find_function(&resolved, "0gen_F__T_int.ghost.static").is_none());
}

#[test]
fn instantiate_generic_class_methods_skips_nonexistent_class_method_function() {
    // Exercises the `continue` at the `else` branch of the class-method
    // `rfind` in `instantiate_generic_class_methods` — the class_methods
    // table references a mangled name that has no matching `HirItem`.
    let self_ty = Ty::Instance(Box::new("F".to_string()));
    let class_def = HirClassDef {
        class_attrs: Vec::new(),
        exception_type_tag: None,
        name: "F".to_string(),
        bases: Vec::new(),
        mro: vec!["F".to_string()],
        attrs: vec![("x".to_string(), Ty::Param(Box::new("T".to_string())))],
        methods: vec![("__init__".to_string(), "F.__init__".to_string())],
        type_param: Some("T".to_string()),
        properties: Vec::new(),
        static_methods: Vec::new(),
        class_methods: vec![("ghost".to_string(), "F.ghost.classmethod".to_string())],
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
        name: "F.__init__".to_string(),
        params: vec![
            ("self".to_string(), self_ty),
            ("x".to_string(), Ty::Param(Box::new("T".to_string()))),
        ],
        return_ty: Ty::None,
        body: vec![HirStmt::AttrSet {
            base: HirExpr::Name("self".to_string()),
            attr: "x".to_string(),
            value: HirExpr::Name("x".to_string()),
        }],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            init,
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::GenericClassInstantiate {
                class: "F".to_string(),
                type_arg: Ty::Int,
                args: vec![HirExpr::IntLiteral(1)],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: vec![("F".to_string(), class_def)],
    };
    assert!(check(&hir).is_ok());
    let resolved = check_and_resolve(&hir).unwrap();
    assert!(find_function(&resolved, "0gen_F__T_int.__init__").is_some());
    assert!(find_function(&resolved, "0gen_F__T_int.ghost.classmethod").is_none());
}

// -- is_assignable Ty::Param branch (line 3229) -----------------------

#[test]
fn is_assignable_accepts_a_scalar_assigned_to_a_param_typed_attribute() {
    // Exercises `is_assignable`'s `Ty::Param` clause (line 3229):
    // `matches!(to, Ty::Param(_)) && matches!(from, Ty::Int | ...)`.
    // A generic class `C` has attribute `x: T`; its `__init__` takes
    // `x: int` (a concrete scalar, not `T`) and does `self.x = x`.
    // `check_attr_set` calls `is_assignable(Ty::Int, Ty::Param("T"))`,
    // which hits the `Ty::Param` branch and returns true.
    let self_ty = Ty::Instance(Box::new("C".to_string()));
    let init = HirItem::Function {
        name: "C.__init__".to_string(),
        params: vec![("self".to_string(), self_ty), ("x".to_string(), Ty::Int)],
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
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            init,
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
    // `check` should succeed — `is_assignable(Ty::Int, Ty::Param("T"))`
    // returns true via the `Ty::Param` branch at line 3229.
    assert!(check(&hir).is_ok());
}

#[test]
fn is_assignable_param_branch_direct() {
    // Directly exercises the `Ty::Param` clause at line 3229:
    // `matches!(to, Ty::Param(_)) && matches!(from, Ty::Int | ...)`.
    assert!(is_assignable(Ty::Int, Ty::Param(Box::new("T".to_string()))));
    assert!(is_assignable(
        Ty::Float,
        Ty::Param(Box::new("T".to_string()))
    ));
    assert!(is_assignable(
        Ty::Bool,
        Ty::Param(Box::new("T".to_string()))
    ));
    assert!(is_assignable(Ty::Str, Ty::Param(Box::new("T".to_string()))));
    // A non-scalar `from` should NOT match the Param branch.
    assert!(!is_assignable(
        Ty::List(Box::new(Ty::Int)),
        Ty::Param(Box::new("T".to_string()))
    ));
}

#[test]
fn is_assignable_param_clause_accepts_scalar_assignment_in_generic_method() {
    // A generic class `C[T]` whose `__init__` assigns a scalar literal
    // (`1`, type `Int`) to `self.x` (attr type `Ty::Param("T")`). This
    // exercises `is_assignable`'s `Ty::Param` clause at line 3229:
    // `matches!(to, Ty::Param(_)) && matches!(from, Ty::Int | ...)`.
    // The method has no `Ty::Param` parameter (only `self`), so it is
    // NOT a generic function and goes through `check_function_in`,
    // which type-checks the body and calls `is_assignable(Int, Param("T"))`.
    let self_ty = Ty::Instance(Box::new("C".to_string()));
    let init = HirItem::Function {
        name: "C.__init__".to_string(),
        params: vec![("self".to_string(), self_ty)],
        return_ty: Ty::None,
        body: vec![HirStmt::AttrSet {
            base: HirExpr::Name("self".to_string()),
            attr: "x".to_string(),
            value: HirExpr::IntLiteral(1),
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
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![init],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: vec![("C".to_string(), class_def)],
    };
    // `check` must accept this — `is_assignable(Int, Param("T"))` returns
    // true via the new clause.
    assert!(check(&hir).is_ok());
}

// -- reject_generic_calls_in_expr GCI ? path (line 4569) --------------

#[test]
fn reject_generic_calls_in_expr_propagates_error_from_generic_class_instantiate_arg() {
    // Exercises `reject_generic_calls_in_expr`'s GCI arm's `?` on the
    // recursive call (line 4569). A generic function whose body contains
    // a GCI whose arg is itself a generic function self-call (which
    // `reject_generic_calls_in_expr` rejects with T0042).
    let self_ty = Ty::Instance(Box::new("C".to_string()));
    let init = HirItem::Function {
        name: "C.__init__".to_string(),
        params: vec![
            ("self".to_string(), self_ty),
            ("x".to_string(), Ty::Param(Box::new("T".to_string()))),
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
    // Generic function `g[T]` whose body contains `C[int](g(1))` —
    // the arg `g(1)` is a self-call, which `reject_generic_calls_in_expr`
    // rejects with T0042. The error propagates through the GCI arm's `?`.
    let g = HirItem::Function {
        name: "g".to_string(),
        params: vec![("x".to_string(), Ty::Param(Box::new("T".to_string())))],
        return_ty: Ty::Param(Box::new("T".to_string())),
        body: vec![
            HirStmt::ExprStmt(HirExpr::GenericClassInstantiate {
                class: "C".to_string(),
                type_arg: Ty::Int,
                args: vec![HirExpr::Call {
                    callee: "g".to_string(),
                    args: vec![HirExpr::IntLiteral(1)],
                }],
            }),
            HirStmt::Return(Some(HirExpr::Name("x".to_string()))),
        ],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![init, g],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: vec![("C".to_string(), class_def)],
    };
    // `check` should fail with T0042 — the generic function `g` calls
    // itself inside a GCI arg, which `reject_generic_calls_in_expr`
    // catches.
    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0042");
}

// -- instantiate_generic_class_methods seen.insert false (line 5549) --

#[test]
fn instantiate_generic_class_methods_skips_duplicate_method_entry() {
    // Exercises the `seen.insert` false branch at line 5549 in
    // `instantiate_generic_class_methods`. A class_def with a duplicate
    // method entry causes the second iteration to produce the same
    // `new_mangled` name, so `seen.insert` returns false and the
    // `instantiations.push` is skipped (but `mangled_methods.push`
    // still runs, since it's outside the `if`).
    let self_ty = Ty::Instance(Box::new("D".to_string()));
    let init = HirItem::Function {
        name: "D.__init__".to_string(),
        params: vec![
            ("self".to_string(), self_ty),
            ("x".to_string(), Ty::Param(Box::new("T".to_string()))),
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
        name: "D".to_string(),
        bases: Vec::new(),
        mro: vec!["D".to_string()],
        attrs: vec![("x".to_string(), Ty::Param(Box::new("T".to_string())))],
        methods: vec![
            ("__init__".to_string(), "D.__init__".to_string()),
            // Duplicate entry — same method name and mangled name.
            ("__init__".to_string(), "D.__init__".to_string()),
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
    // A generic function forces `check_and_resolve` → `monomorphize`
    // → `instantiate_generic_class_methods`.
    let identity = HirItem::Function {
        name: "identity".to_string(),
        params: vec![("x".to_string(), Ty::Param(Box::new("T".to_string())))],
        return_ty: Ty::Param(Box::new("T".to_string())),
        body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            init,
            identity,
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::GenericClassInstantiate {
                class: "D".to_string(),
                type_arg: Ty::Int,
                args: vec![HirExpr::IntLiteral(1)],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: vec![("D".to_string(), class_def)],
    };
    assert!(check(&hir).is_ok());
    let resolved = check_and_resolve(&hir).unwrap();
    // The monomorphized __init__ should exist exactly once despite the
    // duplicate method entry — `seen.insert` prevented the second push.
    assert_eq!(count_function(&resolved, "0gen_D__T_int.__init__"), 1);
}
