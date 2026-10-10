//! Annotated locals inside private helpers during signature inference.

use super::*;

#[test]
fn a_private_helper_can_use_an_annotated_assignment_during_signature_inference() {
    // End-to-end: an annotated assignment inside a private helper whose
    // own signature is still `Ty::Infer` exercises the solver path
    // (`collect_block_constraints`) as well as the later concrete
    // re-check (`check_stmt_in_function`), not just the direct unit
    // tests above.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_helper".to_string(),
            params: vec![],
            return_ty: Ty::Infer,
            body: vec![HirStmt::AnnAssign {
                is_final: false,
                target: "x".to_string(),
                annotation: Ty::Int,
                value: Some(HirExpr::IntLiteral(1)),
            }],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    assert!(check(&hir).is_ok());
}

#[test]
fn an_annotated_private_local_preserves_known_bool_initializer_evidence() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "_identity".to_string(),
                params: vec![("x".to_string(), Ty::Infer)],
                return_ty: Ty::Infer,
                body: vec![
                    HirStmt::AnnAssign {
                        is_final: false,
                        target: "y".to_string(),
                        annotation: Ty::Int,
                        value: Some(HirExpr::Name("x".to_string())),
                    },
                    HirStmt::Return(Some(HirExpr::Name("y".to_string()))),
                ],
            },
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
                callee: "_identity".to_string(),
                args: vec![HirExpr::BoolLiteral(true)],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    check(&hir).unwrap();
    let resolved = check_and_resolve(&hir).unwrap();
    assert!(matches!(
        &resolved.items[0],
        HirItem::Function { params, return_ty, .. }
            if params[0].1 == Ty::Bool && *return_ty == Ty::Int
    ));
}

#[test]
fn an_annotated_private_local_does_not_retag_the_returned_initializer() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "_echo".to_string(),
                params: vec![("x".to_string(), Ty::Infer)],
                return_ty: Ty::Infer,
                body: vec![
                    HirStmt::AnnAssign {
                        is_final: false,
                        target: "y".to_string(),
                        annotation: Ty::Int,
                        value: Some(HirExpr::Name("x".to_string())),
                    },
                    HirStmt::Return(Some(HirExpr::Name("x".to_string()))),
                ],
            },
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
                callee: "_echo".to_string(),
                args: vec![HirExpr::BoolLiteral(true)],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let resolved = check_and_resolve(&hir).unwrap();
    assert!(matches!(
        &resolved.items[0],
        HirItem::Function { params, return_ty, .. }
            if params[0].1 == Ty::Bool && *return_ty == Ty::Bool
    ));
}

#[test]
fn an_annotated_private_local_supplies_a_fallback_without_call_site_evidence() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_helper".to_string(),
            params: vec![("x".to_string(), Ty::Infer)],
            return_ty: Ty::Infer,
            body: vec![
                HirStmt::AnnAssign {
                    is_final: false,
                    target: "y".to_string(),
                    annotation: Ty::Int,
                    value: Some(HirExpr::Name("x".to_string())),
                },
                HirStmt::Return(Some(HirExpr::Name("y".to_string()))),
            ],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let resolved = check_and_resolve(&hir).unwrap();
    assert!(matches!(
        &resolved.items[0],
        HirItem::Function { params, return_ty, .. }
            if params[0].1 == Ty::Int && *return_ty == Ty::Int
    ));
}

#[test]
fn a_container_annotation_cannot_escape_the_scalar_only_private_solver() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_helper".to_string(),
            params: vec![("x".to_string(), Ty::Infer)],
            return_ty: Ty::Infer,
            body: vec![
                HirStmt::AnnAssign {
                    is_final: false,
                    target: "y".to_string(),
                    annotation: Ty::List(Box::new(Ty::Int)),
                    value: Some(HirExpr::Name("x".to_string())),
                },
                HirStmt::Return(Some(HirExpr::Name("y".to_string()))),
            ],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let err = check_and_resolve(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("cannot infer type of parameter `x`"));
}

#[test]
fn a_container_annotated_target_cannot_resolve_an_inferred_private_return() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_helper".to_string(),
            params: vec![],
            return_ty: Ty::Infer,
            body: vec![
                HirStmt::AnnAssign {
                    is_final: false,
                    target: "y".to_string(),
                    annotation: Ty::List(Box::new(Ty::Int)),
                    value: Some(HirExpr::ListLiteral(vec![HirExpr::IntLiteral(1)])),
                },
                HirStmt::Return(Some(HirExpr::Name("y".to_string()))),
            ],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let err = check_and_resolve(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(
        err.message
            .contains("cannot infer return type of private helper `_helper`")
    );
}

#[test]
fn hard_call_evidence_cannot_turn_a_container_local_into_an_inferred_signature() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "_sink".to_string(),
                params: vec![("xs".to_string(), Ty::List(Box::new(Ty::Int)))],
                return_ty: Ty::None,
                body: vec![],
            },
            HirItem::Function {
                name: "_helper".to_string(),
                params: vec![],
                return_ty: Ty::Infer,
                body: vec![
                    HirStmt::AnnAssign {
                        is_final: false,
                        target: "y".to_string(),
                        annotation: Ty::List(Box::new(Ty::Int)),
                        value: Some(HirExpr::ListLiteral(vec![HirExpr::IntLiteral(1)])),
                    },
                    HirStmt::ExprStmt(HirExpr::Call {
                        callee: "_sink".to_string(),
                        args: vec![HirExpr::Name("y".to_string())],
                    }),
                    HirStmt::Return(Some(HirExpr::Name("y".to_string()))),
                ],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let err = check_and_resolve(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(
        err.message
            .contains("cannot infer return type of private helper `_helper`")
    );
}

#[test]
fn an_unrelated_container_local_does_not_block_scalar_signature_inference() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "_helper".to_string(),
                params: vec![("x".to_string(), Ty::Infer)],
                return_ty: Ty::Infer,
                body: vec![
                    HirStmt::AnnAssign {
                        is_final: false,
                        target: "y".to_string(),
                        annotation: Ty::List(Box::new(Ty::Int)),
                        value: Some(HirExpr::ListLiteral(vec![HirExpr::IntLiteral(1)])),
                    },
                    HirStmt::Return(Some(HirExpr::Name("x".to_string()))),
                ],
            },
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
                callee: "_helper".to_string(),
                args: vec![HirExpr::IntLiteral(1)],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let resolved = check_and_resolve(&hir).unwrap();
    assert!(matches!(
        &resolved.items[0],
        HirItem::Function { params, return_ty, .. }
            if params[0].1 == Ty::Int && *return_ty == Ty::Int
    ));
}

#[test]
fn scalar_hard_evidence_on_a_container_local_reaches_final_validation() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "_sink".to_string(),
                params: vec![("value".to_string(), Ty::Int)],
                return_ty: Ty::None,
                body: vec![],
            },
            HirItem::Function {
                name: "_helper".to_string(),
                params: vec![],
                return_ty: Ty::Infer,
                body: vec![
                    HirStmt::AnnAssign {
                        is_final: false,
                        target: "y".to_string(),
                        annotation: Ty::List(Box::new(Ty::Int)),
                        value: Some(HirExpr::ListLiteral(vec![HirExpr::IntLiteral(1)])),
                    },
                    HirStmt::ExprStmt(HirExpr::Call {
                        callee: "_sink".to_string(),
                        args: vec![HirExpr::Name("y".to_string())],
                    }),
                    HirStmt::Return(Some(HirExpr::Name("y".to_string()))),
                ],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let err = check_and_resolve(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(
        err.message
            .contains("argument 1 of `_sink` expects `int`, got `list[int]`")
    );
}

#[test]
fn an_annotated_private_local_fallback_propagates_through_a_binary_expression() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_helper".to_string(),
            params: vec![("x".to_string(), Ty::Infer)],
            return_ty: Ty::Infer,
            body: vec![
                HirStmt::AnnAssign {
                    is_final: false,
                    target: "y".to_string(),
                    annotation: Ty::Int,
                    value: Some(HirExpr::BinOp {
                        op: BinOpKind::Add,
                        left: Box::new(HirExpr::Name("x".to_string())),
                        right: Box::new(HirExpr::IntLiteral(1)),
                    }),
                },
                HirStmt::Return(Some(HirExpr::Name("y".to_string()))),
            ],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let resolved = check_and_resolve(&hir).unwrap();
    assert!(matches!(
        &resolved.items[0],
        HirItem::Function { params, .. } if params[0].1 == Ty::Int
    ));
}

#[test]
fn an_annotated_private_local_fallback_rechecks_binary_operand_conflicts() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_helper".to_string(),
            params: vec![("x".to_string(), Ty::Infer)],
            return_ty: Ty::Infer,
            body: vec![
                HirStmt::AnnAssign {
                    is_final: false,
                    target: "y".to_string(),
                    annotation: Ty::Int,
                    value: Some(HirExpr::BinOp {
                        op: BinOpKind::Add,
                        left: Box::new(HirExpr::Name("x".to_string())),
                        right: Box::new(HirExpr::StringLiteral("bad".to_string())),
                    }),
                },
                HirStmt::Return(Some(HirExpr::Name("y".to_string()))),
            ],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let err = check_and_resolve(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(
        err.message
            .contains("right operand of int binary expression")
    );
}

#[test]
fn later_nested_call_evidence_wins_over_an_earlier_annotation_fallback() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "_echo".to_string(),
                params: vec![("x".to_string(), Ty::Infer)],
                return_ty: Ty::Infer,
                body: vec![
                    HirStmt::AnnAssign {
                        is_final: false,
                        target: "y".to_string(),
                        annotation: Ty::Int,
                        value: Some(HirExpr::Name("x".to_string())),
                    },
                    HirStmt::Return(Some(HirExpr::Name("x".to_string()))),
                ],
            },
            HirItem::Function {
                name: "_caller".to_string(),
                params: vec![],
                return_ty: Ty::None,
                body: vec![
                    HirStmt::ExprStmt(HirExpr::Call {
                        callee: "_echo".to_string(),
                        args: vec![HirExpr::BoolLiteral(true)],
                    }),
                    HirStmt::Return(None),
                ],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let resolved = check_and_resolve(&hir).unwrap();
    assert!(matches!(
        &resolved.items[0],
        HirItem::Function { params, return_ty, .. }
            if params[0].1 == Ty::Bool && *return_ty == Ty::Bool
    ));
}

#[test]
fn multiple_annotation_bounds_choose_bool_in_either_declaration_order() {
    for bool_first in [false, true] {
        let int_assignment = HirStmt::AnnAssign {
            is_final: false,
            target: "a".to_string(),
            annotation: Ty::Int,
            value: Some(HirExpr::Name("x".to_string())),
        };
        let bool_assignment = HirStmt::AnnAssign {
            is_final: false,
            target: "b".to_string(),
            annotation: Ty::Bool,
            value: Some(HirExpr::Name("x".to_string())),
        };
        let assignments = if bool_first {
            vec![bool_assignment, int_assignment]
        } else {
            vec![int_assignment, bool_assignment]
        };
        let mut body = assignments;
        body.push(HirStmt::Return(Some(HirExpr::Name("b".to_string()))));
        let hir = HirModule {
            seeded_builtin_exception_classes: false,
            items: vec![HirItem::Function {
                name: "_helper".to_string(),
                params: vec![("x".to_string(), Ty::Infer)],
                return_ty: Ty::Infer,
                body,
            }],
            type_aliases: Vec::new(),
            imports: Vec::new(),
            class_defs: Vec::new(),
        };

        let resolved = check_and_resolve(&hir).unwrap();
        assert!(matches!(
            &resolved.items[0],
            HirItem::Function { params, .. } if params[0].1 == Ty::Bool
        ));
    }
}

#[test]
fn incompatible_annotation_bounds_fail_identically_in_either_declaration_order() {
    let mut messages = Vec::new();
    for string_first in [false, true] {
        let int_assignment = HirStmt::AnnAssign {
            is_final: false,
            target: "a".to_string(),
            annotation: Ty::Int,
            value: Some(HirExpr::Name("x".to_string())),
        };
        let string_assignment = HirStmt::AnnAssign {
            is_final: false,
            target: "b".to_string(),
            annotation: Ty::Str,
            value: Some(HirExpr::Name("x".to_string())),
        };
        let body = if string_first {
            vec![string_assignment, int_assignment]
        } else {
            vec![int_assignment, string_assignment]
        };
        let hir = HirModule {
            seeded_builtin_exception_classes: false,
            items: vec![HirItem::Function {
                name: "_helper".to_string(),
                params: vec![("x".to_string(), Ty::Infer)],
                return_ty: Ty::None,
                body,
            }],
            type_aliases: Vec::new(),
            imports: Vec::new(),
            class_defs: Vec::new(),
        };

        let err = check_and_resolve(&hir).unwrap_err();
        assert_eq!(err.code, "T0021");
        messages.push(err.message);
    }
    assert_eq!(messages[0], messages[1]);
}

#[test]
fn annotated_private_local_rejects_known_int_to_bool_narrowing_with_t0025() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "_narrow".to_string(),
                params: vec![("x".to_string(), Ty::Infer)],
                return_ty: Ty::Infer,
                body: vec![
                    HirStmt::AnnAssign {
                        is_final: false,
                        target: "y".to_string(),
                        annotation: Ty::Bool,
                        value: Some(HirExpr::Name("x".to_string())),
                    },
                    HirStmt::Return(Some(HirExpr::Name("y".to_string()))),
                ],
            },
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
                callee: "_narrow".to_string(),
                args: vec![HirExpr::IntLiteral(1)],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    assert_eq!(check_and_resolve(&hir).unwrap_err().code, "T0025");
}

#[test]
fn annotated_private_local_binds_when_a_container_subscript_has_no_solver_term() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "xs".to_string(),
                value: HirExpr::ListLiteral(vec![HirExpr::IntLiteral(1)]),
            }),
            HirItem::Function {
                name: "_first".to_string(),
                params: vec![],
                return_ty: Ty::Infer,
                body: vec![
                    HirStmt::AnnAssign {
                        is_final: false,
                        target: "y".to_string(),
                        annotation: Ty::Int,
                        value: Some(HirExpr::Subscript {
                            base: Box::new(HirExpr::Name("xs".to_string())),
                            index: Box::new(HirExpr::IntLiteral(0)),
                        }),
                    },
                    HirStmt::Return(Some(HirExpr::Name("y".to_string()))),
                ],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let resolved = check_and_resolve(&hir).unwrap();
    assert!(matches!(
        &resolved.items[1],
        HirItem::Function { return_ty, .. } if *return_ty == Ty::Int
    ));
}

#[test]
fn no_term_initializer_still_reaches_directional_final_validation() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "xs".to_string(),
                value: HirExpr::ListLiteral(vec![HirExpr::IntLiteral(1)]),
            }),
            HirItem::Function {
                name: "_first".to_string(),
                params: vec![],
                return_ty: Ty::Infer,
                body: vec![
                    HirStmt::AnnAssign {
                        is_final: false,
                        target: "y".to_string(),
                        annotation: Ty::Bool,
                        value: Some(HirExpr::Subscript {
                            base: Box::new(HirExpr::Name("xs".to_string())),
                            index: Box::new(HirExpr::IntLiteral(0)),
                        }),
                    },
                    HirStmt::Return(Some(HirExpr::Name("y".to_string()))),
                ],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    assert_eq!(check_and_resolve(&hir).unwrap_err().code, "T0025");
}

#[test]
fn a_cloned_environment_keeps_later_function_bindings_isolated() {
    let mut original = Environment::new();
    original.bind_function("original".to_string(), vec![Ty::Int], Ty::Int);

    let mut cloned = original.clone();
    cloned.bind_function("cloned".to_string(), vec![], Ty::None);

    assert!(original.lookup_function("cloned").is_none());
    assert_eq!(
        cloned.lookup_function("cloned"),
        Some(&(Vec::new(), Ty::None))
    );
    assert_eq!(
        original.lookup_function("original"),
        Some(&(vec![Ty::Int], Ty::Int))
    );
}
