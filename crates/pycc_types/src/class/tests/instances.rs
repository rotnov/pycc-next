//! Instantiation, attribute reads/writes and method calls on class instances.

use super::*;

#[test]
fn instantiation_attribute_read_and_method_call_all_type_check() {
    let hir = point_module(vec![
        top_level(HirStmt::Assign {
            target: "p".to_string(),
            value: HirExpr::Call {
                callee: "Point".to_string(),
                args: vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)],
            },
        }),
        top_level(HirStmt::ExprStmt(HirExpr::MethodCall {
            base: Box::new(HirExpr::Name("p".to_string())),
            method: "bump".to_string(),
            args: vec![],
        })),
        top_level(HirStmt::ExprStmt(HirExpr::Call {
            callee: "print".to_string(),
            args: vec![HirExpr::AttrGet {
                base: Box::new(HirExpr::Name("p".to_string())),
                attr: "x".to_string(),
            }],
        })),
    ]);
    check(&hir).expect(
        "a well-typed class instantiation/method-call/attribute-read program should check",
    );
}

#[test]
fn instantiating_with_the_wrong_argument_count_is_rejected() {
    let hir = point_module(vec![top_level(HirStmt::ExprStmt(HirExpr::Call {
        callee: "Point".to_string(),
        args: vec![HirExpr::IntLiteral(1)],
    }))]);
    let diagnostic = check(&hir).unwrap_err();
    assert_eq!(diagnostic.code, "T0021");
}

#[test]
fn instantiating_with_a_wrong_argument_type_is_rejected() {
    let hir = point_module(vec![top_level(HirStmt::ExprStmt(HirExpr::Call {
        callee: "Point".to_string(),
        args: vec![
            HirExpr::IntLiteral(1),
            HirExpr::StringLiteral("y".to_string()),
        ],
    }))]);
    let diagnostic = check(&hir).unwrap_err();
    assert_eq!(diagnostic.code, "T0021");
}

#[test]
fn reading_an_attribute_on_a_non_instance_value_is_rejected() {
    let hir = point_module(vec![top_level(HirStmt::ExprStmt(HirExpr::Call {
        callee: "print".to_string(),
        args: vec![HirExpr::AttrGet {
            base: Box::new(HirExpr::IntLiteral(1)),
            attr: "x".to_string(),
        }],
    }))]);
    let diagnostic = check(&hir).unwrap_err();
    assert_eq!(diagnostic.code, "T0043");
}

#[test]
fn reading_an_undeclared_attribute_is_rejected() {
    let hir = point_module(vec![
        top_level(HirStmt::Assign {
            target: "p".to_string(),
            value: HirExpr::Call {
                callee: "Point".to_string(),
                args: vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)],
            },
        }),
        top_level(HirStmt::ExprStmt(HirExpr::Call {
            callee: "print".to_string(),
            args: vec![HirExpr::AttrGet {
                base: Box::new(HirExpr::Name("p".to_string())),
                attr: "z".to_string(),
            }],
        })),
    ]);
    let diagnostic = check(&hir).unwrap_err();
    assert_eq!(diagnostic.code, "T0044");
}

#[test]
fn calling_a_method_on_a_non_instance_value_is_rejected() {
    let hir = point_module(vec![top_level(HirStmt::ExprStmt(HirExpr::MethodCall {
        base: Box::new(HirExpr::IntLiteral(1)),
        method: "bump".to_string(),
        args: vec![],
    }))]);
    let diagnostic = check(&hir).unwrap_err();
    assert_eq!(diagnostic.code, "T0043");
}

#[test]
fn calling_an_undeclared_method_is_rejected() {
    let hir = point_module(vec![
        top_level(HirStmt::Assign {
            target: "p".to_string(),
            value: HirExpr::Call {
                callee: "Point".to_string(),
                args: vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)],
            },
        }),
        top_level(HirStmt::ExprStmt(HirExpr::MethodCall {
            base: Box::new(HirExpr::Name("p".to_string())),
            method: "fly".to_string(),
            args: vec![],
        })),
    ]);
    let diagnostic = check(&hir).unwrap_err();
    assert_eq!(diagnostic.code, "T0044");
}

#[test]
fn calling_a_method_with_a_wrong_argument_count_is_rejected() {
    let hir = point_module(vec![
        top_level(HirStmt::Assign {
            target: "p".to_string(),
            value: HirExpr::Call {
                callee: "Point".to_string(),
                args: vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)],
            },
        }),
        top_level(HirStmt::ExprStmt(HirExpr::MethodCall {
            base: Box::new(HirExpr::Name("p".to_string())),
            method: "bump".to_string(),
            args: vec![HirExpr::IntLiteral(1)],
        })),
    ]);
    let diagnostic = check(&hir).unwrap_err();
    assert_eq!(diagnostic.code, "T0021");
}

#[test]
fn assigning_a_wrong_typed_value_to_an_attribute_is_rejected() {
    let hir = point_module(vec![
        top_level(HirStmt::Assign {
            target: "p".to_string(),
            value: HirExpr::Call {
                callee: "Point".to_string(),
                args: vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)],
            },
        }),
        top_level(HirStmt::AttrSet {
            base: HirExpr::Name("p".to_string()),
            attr: "x".to_string(),
            value: HirExpr::StringLiteral("nope".to_string()),
        }),
    ]);
    let diagnostic = check(&hir).unwrap_err();
    assert_eq!(diagnostic.code, "T0021");
}

#[test]
fn setting_an_attribute_on_a_non_instance_value_is_rejected() {
    let hir = point_module(vec![top_level(HirStmt::AttrSet {
        base: HirExpr::IntLiteral(1),
        attr: "x".to_string(),
        value: HirExpr::IntLiteral(1),
    })]);
    let diagnostic = check(&hir).unwrap_err();
    assert_eq!(diagnostic.code, "T0043");
}

#[test]
fn attribute_set_propagates_an_ill_typed_base_s_error() {
    // Exercises `check_attr_set`'s own `?` on `base`'s own inference --
    // as opposed to `setting_an_attribute_on_a_non_instance_value_is_rejected`
    // above, which only exercises `base` resolving successfully to a
    // non-instance type.
    let hir = point_module(vec![top_level(HirStmt::AttrSet {
        base: HirExpr::Name("undefined".to_string()),
        attr: "x".to_string(),
        value: HirExpr::IntLiteral(1),
    })]);
    let diagnostic = check(&hir).unwrap_err();
    assert_eq!(diagnostic.code, "T0021");
}

#[test]
fn attribute_set_propagates_an_ill_typed_value_s_error() {
    // Exercises `check_attr_set`'s own `?` on `value`'s own inference.
    let hir = point_module(vec![
        top_level(HirStmt::Assign {
            target: "p".to_string(),
            value: HirExpr::Call {
                callee: "Point".to_string(),
                args: vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)],
            },
        }),
        top_level(HirStmt::AttrSet {
            base: HirExpr::Name("p".to_string()),
            attr: "x".to_string(),
            value: HirExpr::Name("undefined".to_string()),
        }),
    ]);
    let diagnostic = check(&hir).unwrap_err();
    assert_eq!(diagnostic.code, "T0021");
}

#[test]
fn a_class_instance_is_rejected_as_a_numeric_operand() {
    // Task 3's own explicit rule: reject arithmetic on `Ty::Instance`
    // (this crate's existing D-116/D-124-style precedent), exercised
    // through the ordinary `numeric_result_type` catch-all (no
    // class-specific code needed there -- see that function's own
    // `as_numeric` closure).
    let hir = point_module(vec![
        top_level(HirStmt::Assign {
            target: "p".to_string(),
            value: HirExpr::Call {
                callee: "Point".to_string(),
                args: vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)],
            },
        }),
        top_level(HirStmt::ExprStmt(HirExpr::BinOp {
            op: BinOpKind::Add,
            left: Box::new(HirExpr::Name("p".to_string())),
            right: Box::new(HirExpr::IntLiteral(1)),
        })),
    ]);
    let diagnostic = check(&hir).unwrap_err();
    assert_eq!(diagnostic.code, "T0021");
}

#[test]
fn attribute_read_propagates_an_ill_typed_base_s_error() {
    // Exercises `infer_expr_in`'s own `HirExpr::AttrGet` arm's `?` on
    // `base`'s own inference -- as opposed to
    // `reading_an_attribute_on_a_non_instance_value_is_rejected` above,
    // which only exercises `base` itself resolving successfully to a
    // non-instance type.
    let hir = point_module(vec![top_level(HirStmt::ExprStmt(HirExpr::Call {
        callee: "print".to_string(),
        args: vec![HirExpr::AttrGet {
            base: Box::new(HirExpr::Name("undefined".to_string())),
            attr: "x".to_string(),
        }],
    }))]);
    let diagnostic = check(&hir).unwrap_err();
    assert_eq!(diagnostic.code, "T0021");
}

#[test]
fn method_call_propagates_an_ill_typed_base_s_error() {
    // Exercises `infer_expr_in`'s own `HirExpr::MethodCall` arm's `?`
    // on `base`'s own inference.
    let hir = point_module(vec![top_level(HirStmt::ExprStmt(HirExpr::MethodCall {
        base: Box::new(HirExpr::Name("undefined".to_string())),
        method: "bump".to_string(),
        args: vec![],
    }))]);
    let diagnostic = check(&hir).unwrap_err();
    assert_eq!(diagnostic.code, "T0021");
}

#[test]
fn method_call_propagates_an_ill_typed_argument_s_error() {
    // Exercises `infer_expr_in`'s own `HirExpr::MethodCall` arm's `?`
    // on its own argument-collection loop -- distinct from the `base`
    // propagation test above.
    let hir = point_module(vec![
        top_level(HirStmt::Assign {
            target: "p".to_string(),
            value: HirExpr::Call {
                callee: "Point".to_string(),
                args: vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)],
            },
        }),
        top_level(HirStmt::ExprStmt(HirExpr::MethodCall {
            base: Box::new(HirExpr::Name("p".to_string())),
            method: "bump".to_string(),
            args: vec![HirExpr::Name("undefined".to_string())],
        })),
    ]);
    let diagnostic = check(&hir).unwrap_err();
    assert_eq!(diagnostic.code, "T0021");
}

#[test]
fn an_unannotated_private_method_forces_the_solver_to_walk_attribute_and_method_access() {
    // Exercises `collect_expr_constraints`'s and
    // `collect_block_constraints`'s own new `AttrGet`/`MethodCall`/
    // `AttrSet` arms (D-154): the constraint solver only runs at all
    // when at least one function in the module is not fully annotated
    // (`concrete_function_environment` returns `None`, routing `check`
    // through `infer_function_signatures_with_solver_all` instead of the
    // concrete fast path) -- every other test in this module uses only
    // fully annotated methods, so none of them exercises this path.
    // `_touch` is private (D-038: an unannotated *private* name is
    // permitted) and has no return annotation, forcing exactly that.
    let self_ty = Ty::Instance(Box::new("Point".to_string()));
    let init = HirItem::Function {
        name: "Point.__init__".to_string(),
        params: vec![
            ("self".to_string(), self_ty.clone()),
            ("x".to_string(), Ty::Int),
        ],
        return_ty: Ty::None,
        body: vec![
            HirStmt::AttrSet {
                base: HirExpr::Name("self".to_string()),
                attr: "x".to_string(),
                value: HirExpr::Name("x".to_string()),
            },
            HirStmt::Return(None),
        ],
    };
    let bump = HirItem::Function {
        name: "Point.bump".to_string(),
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
    let touch = HirItem::Function {
        name: "Point._touch".to_string(),
        params: vec![("self".to_string(), self_ty.clone())],
        // Explicitly annotated (unlike a truly unannotated private
        // method) so the solver never needs to *infer* this function's
        // own return type from `self.x` -- `AttrGet`/`MethodCall`
        // deliberately give the solver no unification term at all
        // (mirroring `ListPop`/`Subscript`'s own pre-existing
        // consequence, see `collect_expr_constraints`'s own doc
        // comment), so a truly unannotated `_touch` returning `self.x`
        // cannot be solved. This method's own job is only to put an
        // `AttrGet`/`AttrSet`/`MethodCall` inside a body the solver
        // still *walks* (see `_identity` below for what actually
        // forces solver mode for the whole module).
        return_ty: Ty::None,
        body: vec![
            HirStmt::ExprStmt(HirExpr::MethodCall {
                base: Box::new(HirExpr::Name("self".to_string())),
                method: "bump".to_string(),
                args: vec![],
            }),
            HirStmt::AttrSet {
                base: HirExpr::Name("self".to_string()),
                attr: "x".to_string(),
                value: HirExpr::IntLiteral(0),
            },
            HirStmt::ExprStmt(HirExpr::AttrGet {
                base: Box::new(HirExpr::Name("self".to_string())),
                attr: "x".to_string(),
            }),
            HirStmt::Return(None),
        ],
    };
    // Forces the whole module through the solver path (see this
    // test's own doc comment): a private, unannotated helper
    // completely unrelated to `Point`, mirroring this crate's own
    // existing `private_identity_signature_is_inferred_from_its_call_site_and_return`
    // precedent -- its own return type is inferred as `int` from its
    // one call site's argument and its own `return x`.
    let identity = HirItem::Function {
        name: "_identity".to_string(),
        params: vec![("x".to_string(), Ty::Infer)],
        return_ty: Ty::Infer,
        body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            init,
            bump,
            touch,
            identity,
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
                callee: "_identity".to_string(),
                args: vec![HirExpr::IntLiteral(1)],
            })),
        ],
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
                methods: vec![
                    ("__init__".to_string(), "Point.__init__".to_string()),
                    ("bump".to_string(), "Point.bump".to_string()),
                    ("_touch".to_string(), "Point._touch".to_string()),
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
        )],
    };
    check(&hir).expect(
        "a class method reading/writing an instance attribute and calling another \
         method should check when an unrelated unannotated helper forces the solver \
         path for the whole module",
    );
}
