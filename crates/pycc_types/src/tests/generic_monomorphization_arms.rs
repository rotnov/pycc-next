//! Residual monomorphization-internals unit tests for the type-checking
//! crate root.
//!
//! Extracted verbatim from `tests.rs` under AGENTS.md's decomposability rule
//! (part of #695, which tracks decomposing that oversized file). These are the
//! remaining small coverage runs over monomorphization internals: `monomorphize`
//! Pass 2b, `collect_generic_class_instantiations_from_comp_iter`, the
//! `is_assignable` `from == Ty::Param` clause, the two non-`Function` rewrite
//! arms, the `bind_local_types_in_stmt`/mangle gap region, and #436
//! static/class-method monomorphization. The
//! `generic_class_with_static_and_class_methods` fixture moved along with
//! them because only they use it; the `gci_expr` fixture stays in the parent,
//! shared with `tests/generic_class_substitution.rs`. As a child module this
//! still sees the parent's private items directly through `use super::*`, so
//! nothing needed widened visibility; only the tests' location changed.

use super::*;

// -- Pass 2b: GCI inside a monomorphized generic-class method body -----

#[test]
fn check_and_resolve_rewrites_a_generic_class_instantiate_inside_a_generic_class_method_body() {
    // Exercises Pass 2b in `monomorphize` (the `while i < instantiations.len()`
    // loop): a generic class method body contains a `GenericClassInstantiate`
    // that survives `substitute_body_with_class` (which only substitutes
    // `Ty::Param`/`Ty::Instance` in annotations, not GCI expressions).
    // Pass 2b rewrites that GCI into an ordinary `HirExpr::Call` to the
    // mangled class name before appending the instantiation.
    //
    // `Box[T]` with `__init__(self, v: T) -> None` and `self.v = v`.
    // `Maker[T]` with `__init__(self, v: T) -> None`, `self.v = v`, and a
    // method `fetch(self) -> T` whose body contains `b = Box[int](42);
    // return b.v`. `fetch` is itself generic (return type `T`), so Pass 2
    // skips it (it's in `generics`) and only Pass 2b rewrites its body.
    let param = Ty::Param(Box::new("T".to_string()));
    let box_init = HirItem::Function {
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
            attr: "v".to_string(),
            value: HirExpr::Name("v".to_string()),
        }],
    };
    let box_class_def = HirClassDef {
        class_attrs: Vec::new(),
        exception_type_tag: None,
        name: "Box".to_string(),
        bases: Vec::new(),
        mro: vec!["Box".to_string()],
        attrs: vec![("v".to_string(), Ty::Param(Box::new("T".to_string())))],
        methods: vec![("__init__".to_string(), "Box.__init__".to_string())],
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
    let maker_init = HirItem::Function {
        name: "Maker.__init__".to_string(),
        params: vec![
            (
                "self".to_string(),
                Ty::Instance(Box::new("Maker".to_string())),
            ),
            ("v".to_string(), param.clone()),
        ],
        return_ty: Ty::None,
        body: vec![HirStmt::AttrSet {
            base: HirExpr::Name("self".to_string()),
            attr: "v".to_string(),
            value: HirExpr::Name("v".to_string()),
        }],
    };
    let maker_fetch = HirItem::Function {
        name: "Maker.fetch".to_string(),
        params: vec![(
            "self".to_string(),
            Ty::Instance(Box::new("Maker".to_string())),
        )],
        return_ty: param,
        body: vec![
            HirStmt::Assign {
                target: "b".to_string(),
                value: HirExpr::GenericClassInstantiate {
                    class: "Box".to_string(),
                    type_arg: Ty::Int,
                    args: vec![HirExpr::IntLiteral(42)],
                },
            },
            HirStmt::Return(Some(HirExpr::AttrGet {
                base: Box::new(HirExpr::Name("b".to_string())),
                attr: "v".to_string(),
            })),
        ],
    };
    let maker_class_def = HirClassDef {
        class_attrs: Vec::new(),
        exception_type_tag: None,
        name: "Maker".to_string(),
        bases: Vec::new(),
        mro: vec!["Maker".to_string()],
        attrs: vec![("v".to_string(), Ty::Param(Box::new("T".to_string())))],
        methods: vec![
            ("__init__".to_string(), "Maker.__init__".to_string()),
            ("fetch".to_string(), "Maker.fetch".to_string()),
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
            box_init,
            maker_init,
            maker_fetch,
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "m".to_string(),
                value: HirExpr::GenericClassInstantiate {
                    class: "Maker".to_string(),
                    type_arg: Ty::Int,
                    args: vec![HirExpr::IntLiteral(7)],
                },
            }),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: vec![
            ("Box".to_string(), box_class_def),
            ("Maker".to_string(), maker_class_def),
        ],
    };
    // `check` must accept the module.
    assert!(check(&hir).is_ok());
    // `check_and_resolve` must monomorphize without panicking or erroring.
    let resolved = check_and_resolve(&hir).unwrap();
    // Both monomorphized classes should exist.
    assert!(
        find_function(&resolved, "0gen_Box__T_int.__init__").is_some(),
        "monomorphized Box.__init__ should exist"
    );
    assert!(
        find_function(&resolved, "0gen_Maker__T_int.__init__").is_some(),
        "monomorphized Maker.__init__ should exist"
    );
    // The monomorphized `fetch` should exist and its body's GCI should
    // have been rewritten to an ordinary `HirExpr::Call` to the mangled
    // Box class name.
    let mono_fetch = find_function(&resolved, "0gen_Maker__T_int.fetch")
        .expect("monomorphized Maker.fetch should exist");
    assert!(matches!(
        mono_fetch,
        HirItem::Function { body, .. }
        if matches!(&body[0], HirStmt::Assign { value: HirExpr::Call { callee, .. }, .. }
            if callee == "0gen_Box__T_int")
    ));
}

#[test]
fn check_and_resolve_propagates_an_error_from_pass_2b_rewrite() {
    // Exercises the `?` error propagation in Pass 2b's
    // `rewrite_generic_calls_in_stmt(...)?` call. A monomorphized generic
    // class method body contains a `GenericClassInstantiate` for a
    // non-generic class `D` (which exists but has `type_param: None`).
    // `check` passes because `type_check_expr`'s GCI arm only verifies
    // class existence, not genericity. `monomorphize`'s
    // `instantiate_generic_class_methods` skips `D` (no `type_param`), so
    // the GCI survives into the monomorphized `Maker.fetch` body. Pass 2b
    // then calls `rewrite_generic_calls_in_expr`, which finds `D` in
    // `env`, sees `type_param: None`, and returns T0042 — the `?`
    // propagates it out of `monomorphize`.
    let param = Ty::Param(Box::new("T".to_string()));
    let d_init = HirItem::Function {
        name: "D.__init__".to_string(),
        params: vec![
            ("self".to_string(), Ty::Instance(Box::new("D".to_string()))),
            ("v".to_string(), Ty::Int),
        ],
        return_ty: Ty::None,
        body: vec![HirStmt::AttrSet {
            base: HirExpr::Name("self".to_string()),
            attr: "v".to_string(),
            value: HirExpr::Name("v".to_string()),
        }],
    };
    let d_class_def = HirClassDef {
        class_attrs: Vec::new(),
        exception_type_tag: None,
        name: "D".to_string(),
        bases: Vec::new(),
        mro: vec!["D".to_string()],
        attrs: vec![("v".to_string(), Ty::Int)],
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
    let maker_init = HirItem::Function {
        name: "Maker.__init__".to_string(),
        params: vec![
            (
                "self".to_string(),
                Ty::Instance(Box::new("Maker".to_string())),
            ),
            ("v".to_string(), param.clone()),
        ],
        return_ty: Ty::None,
        body: vec![HirStmt::AttrSet {
            base: HirExpr::Name("self".to_string()),
            attr: "v".to_string(),
            value: HirExpr::Name("v".to_string()),
        }],
    };
    let maker_fetch = HirItem::Function {
        name: "Maker.fetch".to_string(),
        params: vec![(
            "self".to_string(),
            Ty::Instance(Box::new("Maker".to_string())),
        )],
        return_ty: param,
        body: vec![
            HirStmt::Assign {
                target: "b".to_string(),
                value: HirExpr::GenericClassInstantiate {
                    class: "D".to_string(),
                    type_arg: Ty::Int,
                    args: vec![HirExpr::IntLiteral(42)],
                },
            },
            HirStmt::Return(Some(HirExpr::AttrGet {
                base: Box::new(HirExpr::Name("b".to_string())),
                attr: "v".to_string(),
            })),
        ],
    };
    let maker_class_def = HirClassDef {
        class_attrs: Vec::new(),
        exception_type_tag: None,
        name: "Maker".to_string(),
        bases: Vec::new(),
        mro: vec!["Maker".to_string()],
        attrs: vec![("v".to_string(), Ty::Param(Box::new("T".to_string())))],
        methods: vec![
            ("__init__".to_string(), "Maker.__init__".to_string()),
            ("fetch".to_string(), "Maker.fetch".to_string()),
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
            d_init,
            maker_init,
            maker_fetch,
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "m".to_string(),
                value: HirExpr::GenericClassInstantiate {
                    class: "Maker".to_string(),
                    type_arg: Ty::Int,
                    args: vec![HirExpr::IntLiteral(7)],
                },
            }),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: vec![
            ("D".to_string(), d_class_def),
            ("Maker".to_string(), maker_class_def),
        ],
    };
    // `check` should pass — `D` exists, and `is_assignable(Int, Param("T"))`
    // accepts the `return b.v` (D.v is Int, return type is Param("T")).
    assert!(check(&hir).is_ok());
    // `check_and_resolve` should fail with T0042 — Pass 2b's
    // `rewrite_generic_calls_in_expr` finds `D` is not generic.
    let err = check_and_resolve(&hir).unwrap_err();
    assert_eq!(err.code, "T0042");
    assert!(err.message.contains("class `D` is not generic"));
}

// -- collect_generic_class_instantiations_from_comp_iter --------------

#[test]
fn collect_generic_class_instantiations_from_comp_iter_finds_a_gci_in_a_range_start() {
    // Exercises `collect_generic_class_instantiations_from_comp_iter`'s
    // `CompIter::Range` arm with a `GenericClassInstantiate` in the
    // `start` position (e.g. `range(C[int](0), 10)`). Called both
    // directly and through `collect_generic_class_instantiations_from_stmt`
    // (via a `ListCompAssign`), since the latter is the real call site.
    let gci = gci_expr();
    let iter = CompIter::Range {
        start: gci.clone(),
        stop: HirExpr::IntLiteral(10),
        step: HirExpr::IntLiteral(1),
    };
    // Direct call.
    let mut out = Vec::new();
    collect_generic_class_instantiations_from_comp_iter(&iter, &mut out);
    assert_eq!(out, vec![("C".to_string(), Ty::Int)]);
    // Indirect call through the statement-level collector.
    let stmt = HirStmt::ListCompAssign {
        target: "xs".to_string(),
        var: "_v0".to_string(),
        iter: CompIter::Range {
            start: gci,
            stop: HirExpr::IntLiteral(10),
            step: HirExpr::IntLiteral(1),
        },
        cond: None,
        elt: Box::new(HirExpr::Name("_v0".to_string())),
    };
    let mut out2 = Vec::new();
    collect_generic_class_instantiations_from_stmt(&stmt, &mut out2);
    assert_eq!(out2, vec![("C".to_string(), Ty::Int)]);
    // `CompIter::Name` holds no expression, so it yields nothing.
    let name_iter = CompIter::Name("xs".to_string());
    let mut out3 = Vec::new();
    collect_generic_class_instantiations_from_comp_iter(&name_iter, &mut out3);
    assert!(out3.is_empty());
}

// -- is_assignable from == Ty::Param clause (line 3240) ----------------

#[test]
fn is_assignable_accepts_a_param_typed_value_assigned_to_a_scalar() {
    // Directly exercises the `from == Ty::Param` clause at line 3240:
    // `matches!(from, Ty::Param(_)) && matches!(to, Ty::Int | Ty::Float |
    // Ty::Bool | Ty::Str)`. This is the `return self.v` direction where
    // the attribute read yields `Ty::Param` and the function's return
    // type is a concrete scalar.
    let param = Ty::Param(Box::new("T".to_string()));
    assert!(is_assignable(param.clone(), Ty::Int));
    assert!(is_assignable(param.clone(), Ty::Float));
    assert!(is_assignable(param.clone(), Ty::Bool));
    assert!(is_assignable(param, Ty::Str));
    // A non-scalar `to` should NOT match the Param branch.
    assert!(!is_assignable(
        Ty::Param(Box::new("T".to_string())),
        Ty::List(Box::new(Ty::Int)),
    ));
}

// -- rewrite_generic_calls_in_instantiation non-Function arm -----------

#[test]
fn rewrite_generic_calls_in_instantiation_skips_a_non_function_item() {
    // Exercises the defense-in-depth `_ => return Ok(())` arm in
    // `rewrite_generic_calls_in_instantiation`. All instantiations are
    // `HirItem::Function` in practice (created by
    // `instantiate_generic_call` or `instantiate_generic_class_methods`),
    // so this arm is unreachable through `check_and_resolve`. Calling the
    // helper directly with a `GenericInstantiation` whose `specialized` is
    // a `HirItem::TopLevelStmt` covers the arm without affecting the
    // monomorphization pipeline.
    let mut env = Environment::new();
    let mut instantiations = vec![GenericInstantiation {
        mangled_name: "ghost".to_string(),
        specialized: HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::IntLiteral(1))),
        return_ty: Ty::None,
    }];
    let mut seen = HashSet::new();
    let result =
        rewrite_generic_calls_in_instantiation(&mut env, 0, &mut instantiations, &mut seen);
    assert!(result.is_ok());
    // The non-Function item should be unchanged (the helper returns
    // early without modifying it). Verified via the mangled name rather
    // than a `matches!` on the variant, which avoided an uncovered
    // `_ => false` arm back when these tests lived in `lib.rs` and counted
    // toward the D-014 denominator.
    assert_eq!(instantiations[0].mangled_name, "ghost");
}

// -- rewrite_protocol_calls_in_specialization non-Function arm ---------

#[test]
fn rewrite_protocol_calls_in_specialization_skips_a_non_function_item() {
    // Exercises the defense-in-depth `_ => return` arm in
    // `rewrite_protocol_calls_in_specialization`. All specializations are
    // `HirItem::Function` in practice (created by
    // `rewrite_protocol_calls_in_expr`), so this arm is unreachable through
    // `check_and_resolve`. Calling the helper directly with a
    // `HirItem::TopLevelStmt` covers the arm without affecting the
    // monomorphization pipeline.
    let env = Environment::new();
    let protocol_funcs = HashMap::new();
    let mut specializations = Vec::new();
    let mut seen = HashSet::new();
    let mut item = HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::IntLiteral(1)));
    rewrite_protocol_calls_in_specialization(
        &env,
        &mut item,
        &protocol_funcs,
        &mut specializations,
        &mut seen,
    );
    assert!(specializations.is_empty());
}

// -- bind_local_types_in_stmt / mangle gap-region coverage --------------

#[test]
fn bind_local_types_in_stmt_skips_assign_when_inference_fails() {
    let mut env = Environment::new();
    let local_names = ["x"];
    let stmt = HirStmt::Assign {
        target: "x".to_string(),
        value: HirExpr::Name("unbound".to_string()),
    };
    bind_local_types_in_stmt(&mut env, &local_names, &stmt);
    assert_eq!(env.lookup("x"), None);
}

/// #1021 review round 1: an `AnnAssign` whose *value* does not infer still
/// seeds the declared annotation, exactly as the valueless
/// `x: int` form already did. Leaving the name unbound made
/// `xs: list[int] = []` invisible to the empty-container pre-pass
/// environment -- the raw literal cannot infer before the pass rewrites it --
/// so a later resolution derived from `xs` reported a spurious `T0003`.
#[test]
fn bind_local_types_in_stmt_annassign_falls_back_to_the_annotation() {
    let mut env = Environment::new();
    let local_names = ["x"];
    let stmt = HirStmt::AnnAssign {
        target: "x".to_string(),
        annotation: Ty::Int,
        value: Some(HirExpr::Name("unbound".to_string())),
        is_final: false,
    };
    bind_local_types_in_stmt(&mut env, &local_names, &stmt);
    assert_eq!(env.lookup("x"), Some(Ty::Int));
}

#[test]
fn bind_local_types_in_stmt_for_list_skips_when_list_not_bound() {
    let mut env = Environment::new();
    let local_names = ["i"];
    let stmt = HirStmt::ForList {
        var: "i".to_string(),
        list: "xs".to_string(),
        body: vec![],
    };
    bind_local_types_in_stmt(&mut env, &local_names, &stmt);
    assert_eq!(env.lookup("i"), None);
}

/// #1445: the flat binder walks every suite of a `try` and a `try`/`except*`
/// -- the body, each handler, the `else` and the `finally` -- in source
/// order, exactly as it walks an `if`'s two arms. Before, the statement fell
/// into the catch-all arm, so a name bound inside any suite stayed unbound
/// and every binding derived from it (lark's `size = len(rule.expansion)`
/// after `action, arg = ...` inside a `try`) failed to infer.
#[test]
fn bind_local_types_in_stmt_walks_every_suite_of_a_try() {
    let assign = |target: &str, value: HirExpr| HirStmt::Assign {
        target: target.to_string(),
        value,
    };
    let suites = |body, handler, orelse, finalbody| {
        (
            body,
            vec![pycc_hir::HirExceptHandler {
                exc_type: Some(vec!["ValueError".to_string()]),
                name: None,
                body: handler,
            }],
            orelse,
            finalbody,
        )
    };
    let local_names = ["a", "b", "c", "d"];
    let (body, handlers, orelse, finalbody) = suites(
        vec![assign("a", HirExpr::IntLiteral(1))],
        vec![assign("b", HirExpr::StringLiteral("s".to_string()))],
        vec![assign("c", HirExpr::BoolLiteral(true))],
        // A later suite reads an earlier suite's binding.
        vec![assign("d", HirExpr::Name("a".to_string()))],
    );
    let try_stmt = HirStmt::Try {
        body: body.clone(),
        handlers: handlers.clone(),
        orelse: orelse.clone(),
        finalbody: finalbody.clone(),
    };
    let try_star = HirStmt::TryStar {
        body,
        handlers,
        orelse,
        finalbody,
    };
    for stmt in [try_stmt, try_star] {
        let mut env = Environment::new();
        bind_local_types_in_stmt(&mut env, &local_names, &stmt);
        assert_eq!(env.lookup("a"), Some(Ty::Int), "{stmt:?}");
        assert_eq!(env.lookup("b"), Some(Ty::Str), "{stmt:?}");
        assert_eq!(env.lookup("c"), Some(Ty::Bool), "{stmt:?}");
        assert_eq!(env.lookup("d"), Some(Ty::Int), "{stmt:?}");
    }
}

#[test]
fn mangle_protocol_instantiation_skips_non_instance_substitutions() {
    let substitutions = vec![("P".to_string(), Ty::Int)];
    let mangled = mangle_protocol_instantiation("foo", &substitutions);
    assert_eq!(mangled, "0gen_foo");
}

// -- #436: generic class monomorphization of static/class methods ------

/// Builds a generic class `C[T]` with `__init__`, a static method
/// `factory(x: int) -> int`, and a class method `greet(cls, x: int) ->
/// int`, plus an instantiation `C[int](1)`.
fn generic_class_with_static_and_class_methods() -> HirModule {
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
    let static_fn = HirItem::Function {
        name: "C.factory.static".to_string(),
        params: vec![("x".to_string(), Ty::Int)],
        return_ty: Ty::Int,
        body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
    };
    let class_fn = HirItem::Function {
        name: "C.greet.classmethod".to_string(),
        params: vec![
            ("cls".to_string(), self_ty.clone()),
            ("x".to_string(), Ty::Int),
        ],
        return_ty: Ty::Int,
        body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
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
        static_methods: vec![("factory".to_string(), "C.factory.static".to_string())],
        class_methods: vec![("greet".to_string(), "C.greet.classmethod".to_string())],
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
    HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            init,
            static_fn,
            class_fn,
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::GenericClassInstantiate {
                class: "C".to_string(),
                type_arg: Ty::Int,
                args: vec![HirExpr::IntLiteral(1)],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: vec![("C".to_string(), class_def)],
    }
}

#[test]
fn check_and_resolve_monomorphizes_a_generic_class_static_method() {
    let hir = generic_class_with_static_and_class_methods();
    assert!(check(&hir).is_ok());
    let resolved = check_and_resolve(&hir).unwrap();
    // The monomorphized static method should exist.
    assert!(
        find_function(&resolved, "0gen_C__T_int.factory.static").is_some(),
        "monomorphized static method should exist"
    );
    // The monomorphized class def should list the static method.
    let mono_class = resolved
        .class_defs
        .iter()
        .find(|(name, _)| name == "0gen_C__T_int")
        .expect("monomorphized class def should exist");
    assert!(
        mono_class
            .1
            .static_methods
            .iter()
            .any(|(name, _)| name == "factory"),
        "monomorphized class def should list the static method"
    );
}

#[test]
fn check_and_resolve_monomorphizes_a_generic_class_class_method() {
    let hir = generic_class_with_static_and_class_methods();
    assert!(check(&hir).is_ok());
    let resolved = check_and_resolve(&hir).unwrap();
    // The monomorphized class method should exist.
    assert!(
        find_function(&resolved, "0gen_C__T_int.greet.classmethod").is_some(),
        "monomorphized class method should exist"
    );
    // The monomorphized class def should list the class method.
    let mono_class = resolved
        .class_defs
        .iter()
        .find(|(name, _)| name == "0gen_C__T_int")
        .expect("monomorphized class def should exist");
    assert!(
        mono_class
            .1
            .class_methods
            .iter()
            .any(|(name, _)| name == "greet"),
        "monomorphized class def should list the class method"
    );
}

#[test]
fn instantiate_generic_class_methods_skips_duplicate_static_method_entry() {
    // Exercises the `seen.insert` false branch for static methods at
    // line 5896 in `instantiate_generic_class_methods`. A class_def with
    // a duplicate static_methods entry causes the second iteration to
    // produce the same `new_mangled` name, so `seen.insert` returns
    // false and the `instantiations.push` is skipped (but
    // `mangled_static_methods.push` still runs, since it's outside the
    // `if`).
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
    let static_fn = HirItem::Function {
        name: "D.factory.static".to_string(),
        params: vec![("x".to_string(), Ty::Int)],
        return_ty: Ty::Int,
        body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
    };
    let class_def = HirClassDef {
        class_attrs: Vec::new(),
        exception_type_tag: None,
        name: "D".to_string(),
        bases: Vec::new(),
        mro: vec!["D".to_string()],
        attrs: vec![("x".to_string(), Ty::Param(Box::new("T".to_string())))],
        methods: vec![("__init__".to_string(), "D.__init__".to_string())],
        type_param: Some("T".to_string()),
        properties: Vec::new(),
        static_methods: vec![
            ("factory".to_string(), "D.factory.static".to_string()),
            // Duplicate entry — same method name and mangled name.
            ("factory".to_string(), "D.factory.static".to_string()),
        ],
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
            static_fn,
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
    // The monomorphized static method should exist exactly once despite
    // the duplicate entry — `seen.insert` prevented the second push.
    assert_eq!(count_function(&resolved, "0gen_D__T_int.factory.static"), 1);
}

#[test]
fn instantiate_generic_class_methods_skips_duplicate_class_method_entry() {
    // Exercises the `seen.insert` false branch for class methods at
    // line 5942 in `instantiate_generic_class_methods`. A class_def with
    // a duplicate class_methods entry causes the second iteration to
    // produce the same `new_mangled` name, so `seen.insert` returns
    // false and the `instantiations.push` is skipped (but
    // `mangled_class_methods.push` still runs, since it's outside the
    // `if`).
    let self_ty = Ty::Instance(Box::new("D".to_string()));
    let init = HirItem::Function {
        name: "D.__init__".to_string(),
        params: vec![
            ("self".to_string(), self_ty.clone()),
            ("x".to_string(), Ty::Param(Box::new("T".to_string()))),
        ],
        return_ty: Ty::None,
        body: vec![HirStmt::AttrSet {
            base: HirExpr::Name("self".to_string()),
            attr: "x".to_string(),
            value: HirExpr::Name("x".to_string()),
        }],
    };
    let class_fn = HirItem::Function {
        name: "D.greet.classmethod".to_string(),
        params: vec![("cls".to_string(), self_ty), ("x".to_string(), Ty::Int)],
        return_ty: Ty::Int,
        body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
    };
    let class_def = HirClassDef {
        class_attrs: Vec::new(),
        exception_type_tag: None,
        name: "D".to_string(),
        bases: Vec::new(),
        mro: vec!["D".to_string()],
        attrs: vec![("x".to_string(), Ty::Param(Box::new("T".to_string())))],
        methods: vec![("__init__".to_string(), "D.__init__".to_string())],
        type_param: Some("T".to_string()),
        properties: Vec::new(),
        static_methods: Vec::new(),
        class_methods: vec![
            ("greet".to_string(), "D.greet.classmethod".to_string()),
            // Duplicate entry — same method name and mangled name.
            ("greet".to_string(), "D.greet.classmethod".to_string()),
        ],
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
            class_fn,
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
    // The monomorphized class method should exist exactly once despite
    // the duplicate entry — `seen.insert` prevented the second push.
    assert_eq!(
        count_function(&resolved, "0gen_D__T_int.greet.classmethod"),
        1
    );
}

/// Part 1 of #1255: a `CompIter::Iterable` never reaches a statement-form
/// comprehension (`comp_assign_stmt` routes it to a plain `Assign`), but the
/// statement walkers still visit its expression, so a generic call or a
/// generic-class instantiation inside one cannot escape them. Pinned
/// directly, since no source reaches these arms.
#[test]
fn the_statement_walkers_visit_an_iterable_comprehension_source() {
    let iter = CompIter::Iterable(Box::new(gci_expr()));
    let mut out = Vec::new();
    collect_generic_class_instantiations_from_comp_iter(&iter, &mut out);
    assert_eq!(out, vec![("C".to_string(), Ty::Int)]);
    let mut exprs = Vec::new();
    comp_iter_exprs(&iter, &mut exprs);
    assert_eq!(exprs, vec![&gci_expr()]);
}
