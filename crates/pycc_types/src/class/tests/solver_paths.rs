//! Attribute and method access on the constraint solver and generic paths; internal panics.

use super::*;

#[test]
fn solver_path_attr_get_propagates_an_ill_typed_base_s_error() {
    // Exercises `collect_expr_constraints`'s own `HirExpr::AttrGet`
    // arm's `?` on `base`'s own constraint collection -- as opposed to
    // `an_unannotated_private_method_forces_the_solver_to_walk_attribute_and_method_access`
    // above, which only exercises `base` collecting successfully. `base`
    // must be a name that is genuinely a *local* referenced before its
    // own first assignment (`collect_expr_constraints`'s `HirExpr::Name`
    // arm only errors for `is_local`-registered names -- an entirely
    // undeclared name silently resolves to `Ok(None)`, never reaching
    // this `?` at all).
    let self_ty = Ty::Instance(Box::new("Point".to_string()));
    let init = HirItem::Function {
        name: "Point.__init__".to_string(),
        params: vec![("self".to_string(), self_ty.clone())],
        return_ty: Ty::None,
        body: vec![
            HirStmt::AttrSet {
                base: HirExpr::Name("self".to_string()),
                attr: "x".to_string(),
                value: HirExpr::IntLiteral(0),
            },
            HirStmt::Return(None),
        ],
    };
    let bad = HirItem::Function {
        name: "_bad".to_string(),
        params: vec![("y".to_string(), Ty::Infer)],
        return_ty: Ty::Infer,
        body: vec![
            HirStmt::ExprStmt(HirExpr::AttrGet {
                base: Box::new(HirExpr::Name("z".to_string())),
                attr: "x".to_string(),
            }),
            HirStmt::Assign {
                target: "z".to_string(),
                value: HirExpr::IntLiteral(0),
            },
            HirStmt::Return(Some(HirExpr::Name("y".to_string()))),
        ],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![init, bad],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: vec![(
            "Point".to_string(),
            HirClassDef {
                class_attrs: Vec::new(),
                exception_type_tag: None,
                name: "Point".to_string(),
                bases: Vec::new(),
                mro: vec!["Point".to_string()],
                attrs: vec![("x".to_string(), Ty::Int)],
                methods: vec![("__init__".to_string(), "Point.__init__".to_string())],
                type_param: None,
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
            },
        )],
    };
    assert!(check(&hir).is_err());
}

#[test]
fn solver_path_method_call_propagates_an_ill_typed_base_and_argument_error() {
    // Exercises `collect_expr_constraints`'s own `HirExpr::MethodCall`
    // arm's `?` on both `base`'s own constraint collection and its
    // per-argument loop's. As in
    // `solver_path_attr_get_propagates_an_ill_typed_base_s_error` above,
    // the erroring name must be a genuine local referenced before its
    // own first assignment -- an entirely undeclared name resolves to
    // `Ok(None)` and never reaches either `?`.
    let bad_base = HirItem::Function {
        name: "_bad_base".to_string(),
        params: vec![("y".to_string(), Ty::Infer)],
        return_ty: Ty::Infer,
        body: vec![
            HirStmt::ExprStmt(HirExpr::MethodCall {
                base: Box::new(HirExpr::Name("z".to_string())),
                method: "bump".to_string(),
                args: vec![],
            }),
            HirStmt::Assign {
                target: "z".to_string(),
                value: HirExpr::IntLiteral(0),
            },
            HirStmt::Return(Some(HirExpr::Name("y".to_string()))),
        ],
    };
    let hir1 = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![bad_base],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert!(check(&hir1).is_err());

    let bad_arg = HirItem::Function {
        name: "_bad_arg".to_string(),
        params: vec![("p".to_string(), Ty::Infer), ("y".to_string(), Ty::Infer)],
        return_ty: Ty::Infer,
        body: vec![
            HirStmt::ExprStmt(HirExpr::MethodCall {
                base: Box::new(HirExpr::Name("p".to_string())),
                method: "bump".to_string(),
                args: vec![HirExpr::Name("z".to_string())],
            }),
            HirStmt::Assign {
                target: "z".to_string(),
                value: HirExpr::IntLiteral(0),
            },
            HirStmt::Return(Some(HirExpr::Name("y".to_string()))),
        ],
    };
    let hir2 = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![bad_arg],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert!(check(&hir2).is_err());
}

#[test]
fn solver_path_attr_set_propagates_an_ill_typed_base_and_value_error() {
    // Exercises `collect_block_constraints`'s own `HirStmt::AttrSet`
    // arm's `?` on both `base`'s and `value`'s own constraint
    // collection. As in the `AttrGet`/`MethodCall` solver-path tests
    // above, the erroring name must be a genuine local referenced
    // before its own first assignment -- an entirely undeclared name
    // resolves to `Ok(None)` and never reaches either `?`.
    let bad_base = HirItem::Function {
        name: "_bad_base".to_string(),
        params: vec![("y".to_string(), Ty::Infer)],
        return_ty: Ty::Infer,
        body: vec![
            HirStmt::AttrSet {
                base: HirExpr::Name("z".to_string()),
                attr: "x".to_string(),
                value: HirExpr::IntLiteral(0),
            },
            HirStmt::Assign {
                target: "z".to_string(),
                value: HirExpr::IntLiteral(0),
            },
            HirStmt::Return(Some(HirExpr::Name("y".to_string()))),
        ],
    };
    let hir1 = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![bad_base],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert!(check(&hir1).is_err());

    let bad_value = HirItem::Function {
        name: "_bad_value".to_string(),
        params: vec![("p".to_string(), Ty::Infer), ("y".to_string(), Ty::Infer)],
        return_ty: Ty::Infer,
        body: vec![
            HirStmt::AttrSet {
                base: HirExpr::Name("p".to_string()),
                attr: "x".to_string(),
                value: HirExpr::Name("z".to_string()),
            },
            HirStmt::Assign {
                target: "z".to_string(),
                value: HirExpr::IntLiteral(0),
            },
            HirStmt::Return(Some(HirExpr::Name("y".to_string()))),
        ],
    };
    let hir2 = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![bad_value],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert!(check(&hir2).is_err());
}

#[test]
fn a_generic_function_body_containing_attribute_and_method_access_type_checks() {
    // Exercises `reject_generic_calls_in_stmt/expr`'s and
    // `rewrite_generic_calls_in_stmt/expr`'s own new `AttrSet`/
    // `AttrGet`/`MethodCall` arms (D-154): none of this module's other
    // tests combine a class with a PEP 695 generic function, so a
    // generic function whose own body reads/writes an instance
    // attribute and calls a method is needed to walk those recursive-
    // descent helpers into these new node shapes at all.
    // `check_and_resolve` exercises both: `checked_function_signatures_all`
    // routes `helper` through `check_generic_function_in` (which calls
    // `reject_generic_calls_in_stmt` to reject self-recursion).
    // `monomorphize`'s own Pass 2, however, explicitly *skips* a generic
    // function's own body (`if generics.contains_key(name) { continue; }`
    // -- only a call *site* that instantiates a generic gets its
    // `substitute_body`-produced specialization, which is appended
    // as-is without ever being re-walked by `rewrite_generic_calls_in_stmt`
    // itself), so `helper`'s own `AttrSet`/`MethodCall`/`AttrGet` nodes
    // never reach `rewrite_generic_calls_in_stmt/expr` at all. `use_counter`
    // below is the ordinary (non-generic) twin of `helper`'s body,
    // existing purely so Pass 2 actually walks this exact node shape
    // -- `helper` itself still exists to keep `monomorphize`'s early
    // "no generics" return from short-circuiting the whole pass, and to
    // keep exercising `reject_generic_calls_in_stmt/expr`'s own walk of
    // a generic function's body.
    // A standalone `Counter` class, not `point_module`'s shared
    // `Point` fixture: `add` takes a real `n: int` argument, needed to
    // exercise `reject_generic_calls_in_expr`'s/
    // `rewrite_generic_calls_in_expr`'s own `MethodCall` arm's
    // per-argument loop body at all -- `point_module`'s own `bump`
    // takes no arguments.
    let counter_ty = Ty::Instance(Box::new("Counter".to_string()));
    let counter_init = HirItem::Function {
        name: "Counter.__init__".to_string(),
        params: vec![("self".to_string(), counter_ty.clone())],
        return_ty: Ty::None,
        body: vec![
            HirStmt::AttrSet {
                base: HirExpr::Name("self".to_string()),
                attr: "n".to_string(),
                value: HirExpr::IntLiteral(0),
            },
            HirStmt::Return(None),
        ],
    };
    let counter_add = HirItem::Function {
        name: "Counter.add".to_string(),
        params: vec![
            ("self".to_string(), counter_ty.clone()),
            ("n".to_string(), Ty::Int),
        ],
        return_ty: Ty::None,
        body: vec![
            HirStmt::AttrSet {
                base: HirExpr::Name("self".to_string()),
                attr: "n".to_string(),
                value: HirExpr::Name("n".to_string()),
            },
            HirStmt::Return(None),
        ],
    };
    let helper = HirItem::Function {
        name: "helper".to_string(),
        params: vec![("x".to_string(), Ty::Param(Box::new("T".to_string())))],
        return_ty: Ty::Param(Box::new("T".to_string())),
        body: vec![
            HirStmt::Assign {
                target: "c".to_string(),
                value: HirExpr::Call {
                    callee: "Counter".to_string(),
                    args: vec![],
                },
            },
            HirStmt::AttrSet {
                base: HirExpr::Name("c".to_string()),
                attr: "n".to_string(),
                value: HirExpr::IntLiteral(1),
            },
            HirStmt::ExprStmt(HirExpr::MethodCall {
                base: Box::new(HirExpr::Name("c".to_string())),
                method: "add".to_string(),
                args: vec![HirExpr::IntLiteral(5)],
            }),
            HirStmt::ExprStmt(HirExpr::Call {
                callee: "print".to_string(),
                args: vec![HirExpr::AttrGet {
                    base: Box::new(HirExpr::Name("c".to_string())),
                    attr: "n".to_string(),
                }],
            }),
            HirStmt::Return(Some(HirExpr::Name("x".to_string()))),
        ],
    };
    let use_counter = HirItem::Function {
        name: "use_counter".to_string(),
        params: vec![("c".to_string(), counter_ty.clone())],
        return_ty: Ty::None,
        body: vec![
            HirStmt::AttrSet {
                base: HirExpr::Name("c".to_string()),
                attr: "n".to_string(),
                value: HirExpr::IntLiteral(2),
            },
            HirStmt::ExprStmt(HirExpr::MethodCall {
                base: Box::new(HirExpr::Name("c".to_string())),
                method: "add".to_string(),
                args: vec![HirExpr::IntLiteral(5)],
            }),
            HirStmt::ExprStmt(HirExpr::Call {
                callee: "print".to_string(),
                args: vec![HirExpr::AttrGet {
                    base: Box::new(HirExpr::Name("c".to_string())),
                    attr: "n".to_string(),
                }],
            }),
            HirStmt::Return(None),
        ],
    };
    let mut hir = point_module(vec![
        counter_init,
        counter_add,
        helper,
        use_counter,
        top_level(HirStmt::ExprStmt(HirExpr::Call {
            callee: "print".to_string(),
            args: vec![HirExpr::Call {
                callee: "helper".to_string(),
                args: vec![HirExpr::IntLiteral(5)],
            }],
        })),
    ]);
    hir.class_defs.push((
        "Counter".to_string(),
        HirClassDef {
            class_attrs: Vec::new(),
            exception_type_tag: None,
            name: "Counter".to_string(),
            bases: Vec::new(),
            mro: vec!["Counter".to_string()],
            attrs: vec![("n".to_string(), Ty::Int)],
            methods: vec![
                ("__init__".to_string(), "Counter.__init__".to_string()),
                ("add".to_string(), "Counter.add".to_string()),
            ],
            type_param: None,
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
        },
    ));
    check_and_resolve(&hir).expect(
        "a well-typed generic function body using class instance attribute/method \
         access should check",
    );
}

#[test]
fn reject_generic_calls_in_expr_propagates_a_method_call_s_base_and_argument_errors() {
    // Exercises `reject_generic_calls_in_expr`'s own `HirExpr::MethodCall`
    // arm's `?` on both `base`'s own rejection walk and its
    // per-argument loop's -- as opposed to
    // `a_generic_function_body_containing_attribute_and_method_access_type_checks`
    // above, which only exercises both succeeding. A self-recursive
    // call nested inside a `MethodCall`'s `base` (first case) or one of
    // its `args` (second case) is rejected by `reject_generic_calls_in_expr`'s
    // own `HirExpr::Call` arm and must propagate back up through
    // `MethodCall`'s own two recursive positions. Neither case needs a
    // real class -- `reject_generic_calls_in_expr` never resolves
    // `MethodCall`'s own `base`/`method` against any `Environment`,
    // only walks the expression tree structurally.
    let bad_base = HirItem::Function {
        name: "bad_base".to_string(),
        params: vec![("x".to_string(), Ty::Param(Box::new("T".to_string())))],
        return_ty: Ty::Param(Box::new("T".to_string())),
        body: vec![HirStmt::Return(Some(HirExpr::MethodCall {
            base: Box::new(HirExpr::Call {
                callee: "bad_base".to_string(),
                args: vec![HirExpr::Name("x".to_string())],
            }),
            method: "whatever".to_string(),
            args: vec![],
        }))],
    };
    let hir1 = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![bad_base],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert_eq!(check(&hir1).unwrap_err().code, "T0042");

    let bad_arg = HirItem::Function {
        name: "bad_arg".to_string(),
        params: vec![("x".to_string(), Ty::Param(Box::new("T".to_string())))],
        return_ty: Ty::Param(Box::new("T".to_string())),
        body: vec![HirStmt::Return(Some(HirExpr::MethodCall {
            base: Box::new(HirExpr::Name("x".to_string())),
            method: "whatever".to_string(),
            args: vec![HirExpr::Call {
                callee: "bad_arg".to_string(),
                args: vec![HirExpr::Name("x".to_string())],
            }],
        }))],
    };
    let hir2 = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![bad_arg],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert_eq!(check(&hir2).unwrap_err().code, "T0042");
}

// -- internal-consistency panics ----------------------------------------
//
// Every test below bypasses the normal `check`/`check_and_resolve`
// entry points, building an inconsistent `Environment` by hand (a
// `Ty::Instance` payload naming a class the `Environment` was never
// told about, or a class whose `__init__`/method was never registered
// as an ordinary function) -- exactly the "a class's declared shape and
// its `Environment` disagree" scenario each of these functions' own doc
// comments name as unreachable from any real `check`-validated program,
// mirroring `pycc_mir`'s own established convention for the identical
// kind of internal-consistency panic (see e.g. that crate's
// `referencing_an_unbound_name_panics_with_an_internal_error` test).

#[test]
#[should_panic(expected = "class `Ghost` has no registered HirClassDef")]
fn resolve_attr_get_panics_when_the_class_is_not_registered() {
    let env = crate::Environment::new();
    let _ = super::super::resolve_attr_get(&env, &Ty::Instance(Box::new("Ghost".to_string())), "x");
}

#[test]
#[should_panic(expected = "was not registered as an ordinary function")]
fn resolve_attr_get_panics_when_a_property_getter_is_not_registered() {
    // #377: a property's getter is in the class's own property table
    // but was never registered in `Environment::functions` -- the
    // "declared shape and Environment disagree" scenario
    // `resolve_attr_get`'s own doc comment names as unreachable from
    // any real `check`-validated program, mirroring
    // `resolve_instantiation_panics_when_init_is_not_registered` above.
    use pycc_hir::PropertyDef;
    let mut env = crate::Environment::new();
    env.bind_class(
        "Ghost".to_string(),
        HirClassDef {
            class_attrs: Vec::new(),
            exception_type_tag: None,
            name: "Ghost".to_string(),
            bases: Vec::new(),
            mro: vec!["Ghost".to_string()],
            attrs: vec![],
            methods: vec![("__init__".to_string(), "Ghost.__init__".to_string())],
            type_param: None,
            properties: vec![PropertyDef {
                name: "x".to_string(),
                getter: "Ghost.x".to_string(),
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
        },
    );
    let _ = super::super::resolve_attr_get(&env, &Ty::Instance(Box::new("Ghost".to_string())), "x");
}

#[test]
#[should_panic(expected = "was not registered as an ordinary function")]
fn check_attr_set_panics_when_a_property_setter_is_not_registered() {
    // #377: a property's setter is in the class's own property table
    // but was never registered in `Environment::functions` -- the
    // "declared shape and Environment disagree" scenario
    // `check_attr_set`'s own doc comment names as unreachable from
    // any real `check`-validated program, mirroring
    // `class/method_call.rs`'s
    // `resolve_method_call_panics_when_the_method_is_not_registered`
    // (moved there by #815, Part 1 of #737).
    use pycc_hir::PropertyDef;
    let mut env = crate::Environment::new();
    env.bind_class(
        "Ghost".to_string(),
        HirClassDef {
            class_attrs: Vec::new(),
            exception_type_tag: None,
            name: "Ghost".to_string(),
            bases: Vec::new(),
            mro: vec!["Ghost".to_string()],
            attrs: vec![],
            methods: vec![("__init__".to_string(), "Ghost.__init__".to_string())],
            type_param: None,
            properties: vec![PropertyDef {
                name: "x".to_string(),
                getter: "Ghost.x".to_string(),
                setter: Some("Ghost.x.setter".to_string()),
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
        },
    );
    // `base` must infer as a `Ghost` instance so `check_attr_set`
    // reaches the property branch; `value` must infer successfully so
    // the `?` on `infer_expr_in` (line 228) does not short-circuit
    // before the setter lookup panic.
    env.bind_function(
        "Ghost.__init__".to_string(),
        vec![Ty::Instance(Box::new("Ghost".to_string()))],
        Ty::None,
    );
    env.bind("b".to_string(), Ty::Instance(Box::new("Ghost".to_string())));
    let _ = super::super::check_attr_set(
        &env,
        &[],
        &HirExpr::Name("b".to_string()),
        "x",
        &HirExpr::IntLiteral(42),
    );
}

#[test]
fn cast_compatibility_treats_an_unregistered_from_class_as_a_representation_mismatch() {
    // #767 (D-198, third pass): every `Ty::Instance` `cast_compatibility`
    // sees from a real `check`-validated program names a class the
    // `Environment` was told about -- either an ordinary user class or
    // a seeded builtin exception class (see
    // `crate::exception::is_user_defined_class`'s doc comment). This
    // directly exercises the function's own defensive fallback for the
    // "declared shape and Environment disagree" state that scenario
    // rules out from any real caller, matching the internal-consistency
    // convention above.
    let env = crate::Environment::new();
    let mismatch = super::super::cast_compatibility(
        &env,
        &Ty::Instance(Box::new("Ghost".to_string())),
        &Ty::Instance(Box::new("AlsoGhost".to_string())),
    )
    .unwrap_err();
    assert_eq!(mismatch, super::super::CastMismatch::Representation);
}

#[test]
fn cast_compatibility_treats_an_mro_entry_missing_its_own_class_def_as_a_layout_mismatch() {
    // Same "declared shape and Environment disagree" scenario as above,
    // but for the ancestor lookup instead of the value's own class:
    // `from`'s own MRO names a class that isn't registered. Normal class
    // registration (`validate_bases` in `pycc_hir::class::mro`) only
    // ever admits an already-defined class into a base's MRO, so this
    // state cannot arise from any real `check`-validated program either.
    let mut env = crate::Environment::new();
    env.bind_class(
        "Derived".to_string(),
        HirClassDef {
            class_attrs: Vec::new(),
            exception_type_tag: None,
            name: "Derived".to_string(),
            bases: vec!["GhostBase".to_string()],
            mro: vec!["Derived".to_string(), "GhostBase".to_string()],
            attrs: vec![],
            methods: vec![],
            type_param: None,
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
        },
    );
    let mismatch = super::super::cast_compatibility(
        &env,
        &Ty::Instance(Box::new("Derived".to_string())),
        &Ty::Instance(Box::new("GhostBase".to_string())),
    )
    .unwrap_err();
    assert_eq!(mismatch, super::super::CastMismatch::Layout);
}
