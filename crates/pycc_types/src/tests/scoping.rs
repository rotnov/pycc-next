//! Module/function scoping: globals, sibling functions and local classification.

use super::*;

#[test]
fn a_top_level_binary_addition_type_checks() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::BinOp {
            op: BinOpKind::Add,
            left: Box::new(HirExpr::IntLiteral(1)),
            right: Box::new(HirExpr::IntLiteral(2)),
        }))],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert!(check(&hir).is_ok());
}

#[test]
fn a_top_level_reference_to_an_undefined_name_is_a_clean_error() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Name(
            "undefined".to_string(),
        )))],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
}

#[test]
fn a_top_level_call_to_a_previously_defined_function_type_checks() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "main".to_string(),
                params: vec![],
                return_ty: Ty::None,
                body: vec![HirStmt::Return(None)],
            },
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
                callee: "main".to_string(),
                args: vec![],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert!(check(&hir).is_ok());
}

#[test]
fn a_function_can_call_a_sibling_function_defined_before_it() {
    // Regression test for D-040: `check_function`'s own env used to be
    // seeded empty, so `main` couldn't see `helper` even though both are
    // ordinary module-level functions -- a valid, non-recursive call
    // between two sibling functions was wrongly rejected with T0021.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "helper".to_string(),
                params: vec![("x".to_string(), Ty::Int)],
                return_ty: Ty::Int,
                body: vec![HirStmt::Return(Some(HirExpr::BinOp {
                    op: BinOpKind::Add,
                    left: Box::new(HirExpr::Name("x".to_string())),
                    right: Box::new(HirExpr::IntLiteral(1)),
                }))],
            },
            HirItem::Function {
                name: "main".to_string(),
                params: vec![],
                return_ty: Ty::None,
                body: vec![HirStmt::ExprStmt(HirExpr::Call {
                    callee: "print".to_string(),
                    args: vec![HirExpr::Call {
                        callee: "helper".to_string(),
                        args: vec![HirExpr::IntLiteral(5)],
                    }],
                })],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert!(check(&hir).is_ok());
}

#[test]
fn a_function_can_call_a_sibling_function_defined_after_it() {
    // Same gap as above, but exercising the pre-registration pass (D-039)
    // from the *other* direction: `main` is checked first (it's first in
    // the module) yet still must see `helper`, which is defined later.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "main".to_string(),
                params: vec![],
                return_ty: Ty::None,
                body: vec![HirStmt::ExprStmt(HirExpr::Call {
                    callee: "print".to_string(),
                    args: vec![HirExpr::Call {
                        callee: "helper".to_string(),
                        args: vec![HirExpr::IntLiteral(5)],
                    }],
                })],
            },
            HirItem::Function {
                name: "helper".to_string(),
                params: vec![("x".to_string(), Ty::Int)],
                return_ty: Ty::Int,
                body: vec![HirStmt::Return(Some(HirExpr::BinOp {
                    op: BinOpKind::Add,
                    left: Box::new(HirExpr::Name("x".to_string())),
                    right: Box::new(HirExpr::IntLiteral(1)),
                }))],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert!(check(&hir).is_ok());
}

#[test]
fn a_function_can_read_a_module_level_global_defined_before_it() {
    // Regression test for D-041: reading a module global from a function
    // body needs no `global` declaration in real Python (that's only
    // required to *rebind* one) -- child_for_function used to reset
    // bindings to empty, so `f`'s body couldn't see `x` even though it's
    // an ordinary module-level constant, not some caller's local.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::IntLiteral(5),
            }),
            HirItem::Function {
                name: "f".to_string(),
                params: vec![],
                return_ty: Ty::Int,
                body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert!(check(&hir).is_ok());
}

#[test]
fn a_function_can_read_a_module_level_global_defined_after_it() {
    // Same gap, other direction: a function is only ever *called* after
    // the module has (typically) finished running top to bottom, so a
    // global defined later in the file is still visible inside an
    // earlier function's body.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "f".to_string(),
                params: vec![],
                return_ty: Ty::Int,
                body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
            },
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::IntLiteral(5),
            }),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert!(check(&hir).is_ok());
}

#[test]
fn a_function_parameter_shadows_a_module_level_global_of_the_same_name() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::StringLiteral("global".to_string()),
            }),
            HirItem::Function {
                name: "f".to_string(),
                params: vec![("x".to_string(), Ty::Int)],
                return_ty: Ty::Int,
                body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    // If the global (Ty::Str) leaked through instead of the parameter
    // (Ty::Int), this would fail with a T0022 return-type mismatch.
    assert!(check(&hir).is_ok());
}

#[test]
fn a_later_local_assignment_blocks_fallback_to_a_same_named_global() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::IntLiteral(1),
            }),
            HirItem::Function {
                name: "f".to_string(),
                params: vec![],
                return_ty: Ty::Int,
                body: vec![
                    HirStmt::ExprStmt(HirExpr::Name("x".to_string())),
                    HirStmt::Assign {
                        target: "x".to_string(),
                        value: HirExpr::IntLiteral(2),
                    },
                    HirStmt::Return(Some(HirExpr::Name("x".to_string()))),
                ],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(err.message, "local name `x` is not bound before this use");

    let err = check_and_resolve(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(err.message, "local name `x` is not bound before this use");
}

#[test]
fn a_read_before_local_assignment_is_local_even_without_a_global() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "f".to_string(),
            params: vec![],
            return_ty: Ty::Int,
            body: vec![
                HirStmt::ExprStmt(HirExpr::Name("x".to_string())),
                HirStmt::Assign {
                    target: "x".to_string(),
                    value: HirExpr::IntLiteral(2),
                },
                HirStmt::Return(Some(HirExpr::Name("x".to_string()))),
            ],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(err.message, "local name `x` is not bound before this use");
}

#[test]
fn local_name_collection_deduplicates_assignment_and_for_targets() {
    let body = vec![
        HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::IntLiteral(1),
        },
        HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::IntLiteral(2),
        },
        HirStmt::ForRange {
            var: "i".to_string(),
            start: HirExpr::IntLiteral(0),
            stop: HirExpr::IntLiteral(1),
            step: HirExpr::IntLiteral(1),
            body: vec![],
        },
        HirStmt::ForRange {
            var: "i".to_string(),
            start: HirExpr::IntLiteral(0),
            stop: HirExpr::IntLiteral(1),
            step: HirExpr::IntLiteral(1),
            body: vec![],
        },
    ];

    assert_eq!(function_local_names(&[], &body), vec!["x", "i"]);
}

#[test]
fn local_name_collection_deduplicates_for_list_targets() {
    let body = vec![
        HirStmt::ForList {
            var: "i".to_string(),
            list: "xs".to_string(),
            body: vec![],
        },
        HirStmt::ForList {
            var: "i".to_string(),
            list: "ys".to_string(),
            body: vec![],
        },
    ];

    assert_eq!(function_local_names(&[], &body), vec!["i"]);
}

#[test]
fn local_name_collection_deduplicates_annotated_assignment_targets() {
    let body = vec![
        HirStmt::AnnAssign {
            is_final: false,
            target: "x".to_string(),
            annotation: Ty::Int,
            value: Some(HirExpr::IntLiteral(1)),
        },
        HirStmt::AnnAssign {
            is_final: false,
            target: "x".to_string(),
            annotation: Ty::Int,
            value: None,
        },
    ];

    assert_eq!(function_local_names(&[], &body), vec!["x"]);
}

#[test]
fn contains_return_treats_an_annotated_assignment_as_not_a_return() {
    let body = vec![HirStmt::AnnAssign {
        is_final: false,
        target: "x".to_string(),
        annotation: Ty::Int,
        value: Some(HirExpr::IntLiteral(1)),
    }];
    assert!(!contains_return(&body));
}

#[test]
fn block_always_returns_treats_an_annotated_assignment_as_not_a_return() {
    let body = vec![HirStmt::AnnAssign {
        is_final: false,
        target: "x".to_string(),
        annotation: Ty::Int,
        value: None,
    }];
    assert!(!block_always_returns(&body));
}

#[test]
fn contains_return_finds_a_return_inside_a_for_list_loop_body() {
    let body = vec![HirStmt::ForList {
        var: "i".to_string(),
        list: "xs".to_string(),
        body: vec![HirStmt::Return(Some(HirExpr::Name("i".to_string())))],
    }];
    assert!(contains_return(&body));
}

#[test]
fn block_always_returns_treats_a_for_list_loop_as_not_always_returning() {
    // A `for` loop's body might never execute (an empty list), so it can
    // never on its own guarantee a function returns -- same treatment as
    // `ForRange` gets in this same match.
    let body = vec![HirStmt::ForList {
        var: "i".to_string(),
        list: "xs".to_string(),
        body: vec![HirStmt::Return(Some(HirExpr::Name("i".to_string())))],
    }];
    assert!(!block_always_returns(&body));
}

#[test]
fn a_call_before_local_assignment_cannot_fall_back_to_a_global_function() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "helper".to_string(),
                params: vec![],
                return_ty: Ty::None,
                body: vec![],
            },
            HirItem::Function {
                name: "f".to_string(),
                params: vec![],
                return_ty: Ty::None,
                body: vec![
                    HirStmt::ExprStmt(HirExpr::Call {
                        callee: "helper".to_string(),
                        args: vec![],
                    }),
                    HirStmt::Assign {
                        target: "helper".to_string(),
                        value: HirExpr::IntLiteral(1),
                    },
                ],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(
        err.message,
        "local name `helper` is not bound before this use"
    );
}

#[test]
fn a_call_before_local_assignment_cannot_fall_back_to_print() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "f".to_string(),
            params: vec![],
            return_ty: Ty::None,
            body: vec![
                HirStmt::ExprStmt(HirExpr::Call {
                    callee: "print".to_string(),
                    args: vec![],
                }),
                HirStmt::Assign {
                    target: "print".to_string(),
                    value: HirExpr::IntLiteral(1),
                },
            ],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let err = check_and_resolve(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(
        err.message,
        "local name `print` is not bound before this use"
    );
}

#[test]
fn a_bound_local_value_cannot_fall_back_to_a_function_registry_entry() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "helper".to_string(),
                params: vec![],
                return_ty: Ty::None,
                body: vec![],
            },
            HirItem::Function {
                name: "f".to_string(),
                params: vec![],
                return_ty: Ty::None,
                body: vec![
                    HirStmt::Assign {
                        target: "helper".to_string(),
                        value: HirExpr::IntLiteral(1),
                    },
                    HirStmt::ExprStmt(HirExpr::Call {
                        callee: "helper".to_string(),
                        args: vec![],
                    }),
                ],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(
        err.message,
        "name `helper` is bound to a non-callable value"
    );
}

#[test]
fn a_parameter_cannot_fall_back_to_a_same_named_builtin() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "f".to_string(),
            params: vec![("print".to_string(), Ty::Int)],
            return_ty: Ty::None,
            body: vec![HirStmt::ExprStmt(HirExpr::Call {
                callee: "print".to_string(),
                args: vec![],
            })],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(err.message, "name `print` is bound to a non-callable value");
}

#[test]
fn direct_function_check_uses_the_same_lexical_local_classification() {
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::Int,
        body: vec![
            HirStmt::ExprStmt(HirExpr::Name("x".to_string())),
            HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::IntLiteral(2),
            },
            HirStmt::Return(Some(HirExpr::Name("x".to_string()))),
        ],
    };

    let err = check_function(&function).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(err.message, "local name `x` is not bound before this use");
}

#[test]
fn direct_function_check_treats_an_unbound_call_target_as_a_local_read() {
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![
            HirStmt::ExprStmt(HirExpr::Call {
                callee: "helper".to_string(),
                args: vec![],
            }),
            HirStmt::Assign {
                target: "helper".to_string(),
                value: HirExpr::IntLiteral(1),
            },
        ],
    };

    let err = check_function(&function).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(
        err.message,
        "local name `helper` is not bound before this use"
    );
}

#[test]
fn direct_function_check_rejects_calling_a_bound_local_value() {
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![
            HirStmt::Assign {
                target: "helper".to_string(),
                value: HirExpr::IntLiteral(1),
            },
            HirStmt::ExprStmt(HirExpr::Call {
                callee: "helper".to_string(),
                args: vec![],
            }),
        ],
    };

    let err = check_function(&function).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(
        err.message,
        "name `helper` is bound to a non-callable value"
    );
}

#[test]
fn a_local_first_assignment_does_not_inherit_the_globals_type() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::StringLiteral("global".to_string()),
            }),
            HirItem::Function {
                name: "f".to_string(),
                params: vec![],
                return_ty: Ty::Int,
                body: vec![
                    HirStmt::Assign {
                        target: "x".to_string(),
                        value: HirExpr::IntLiteral(2),
                    },
                    HirStmt::Return(Some(HirExpr::Name("x".to_string()))),
                ],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    assert!(check(&hir).is_ok());
}

#[test]
fn a_self_referential_first_local_assignment_cannot_read_the_global() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::IntLiteral(1),
            }),
            HirItem::Function {
                name: "f".to_string(),
                params: vec![],
                return_ty: Ty::Int,
                body: vec![
                    HirStmt::Assign {
                        target: "x".to_string(),
                        value: HirExpr::BinOp {
                            op: BinOpKind::Add,
                            left: Box::new(HirExpr::Name("x".to_string())),
                            right: Box::new(HirExpr::IntLiteral(1)),
                        },
                    },
                    HirStmt::Return(Some(HirExpr::Name("x".to_string()))),
                ],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(err.message, "local name `x` is not bound before this use");
}

#[test]
fn an_assignment_nested_in_if_classifies_the_name_as_function_local() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::IntLiteral(1),
            }),
            HirItem::Function {
                name: "f".to_string(),
                params: vec![],
                return_ty: Ty::None,
                body: vec![
                    HirStmt::ExprStmt(HirExpr::Name("x".to_string())),
                    HirStmt::If {
                        test: HirExpr::BoolLiteral(true),
                        body: vec![HirStmt::Assign {
                            target: "x".to_string(),
                            value: HirExpr::IntLiteral(2),
                        }],
                        orelse: vec![],
                    },
                ],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let err = check(&hir).unwrap_err();
    assert_eq!(err.message, "local name `x` is not bound before this use");
}

#[test]
fn a_nested_for_target_classifies_the_name_as_function_local() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "i".to_string(),
                value: HirExpr::IntLiteral(1),
            }),
            HirItem::Function {
                name: "f".to_string(),
                params: vec![],
                return_ty: Ty::None,
                body: vec![
                    HirStmt::ExprStmt(HirExpr::Name("i".to_string())),
                    HirStmt::While {
                        test: HirExpr::BoolLiteral(true),
                        body: vec![HirStmt::ForRange {
                            var: "i".to_string(),
                            start: HirExpr::IntLiteral(0),
                            stop: HirExpr::IntLiteral(1),
                            step: HirExpr::IntLiteral(1),
                            body: vec![],
                        }],
                    },
                ],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let err = check(&hir).unwrap_err();
    assert_eq!(err.message, "local name `i` is not bound before this use");
}

#[test]
fn a_for_target_is_local_while_its_range_operands_are_evaluated() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "i".to_string(),
                value: HirExpr::IntLiteral(3),
            }),
            HirItem::Function {
                name: "f".to_string(),
                params: vec![],
                return_ty: Ty::None,
                body: vec![HirStmt::ForRange {
                    var: "i".to_string(),
                    start: HirExpr::IntLiteral(0),
                    stop: HirExpr::Name("i".to_string()),
                    step: HirExpr::IntLiteral(1),
                    body: vec![],
                }],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(err.message, "local name `i` is not bound before this use");
}

#[test]
fn private_helper_inference_rejects_a_read_before_local_assignment() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::IntLiteral(1),
            }),
            HirItem::Function {
                name: "_f".to_string(),
                params: vec![],
                return_ty: Ty::Infer,
                body: vec![
                    HirStmt::ExprStmt(HirExpr::Name("x".to_string())),
                    HirStmt::Assign {
                        target: "x".to_string(),
                        value: HirExpr::IntLiteral(2),
                    },
                    HirStmt::Return(Some(HirExpr::Name("x".to_string()))),
                ],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let err = check_and_resolve(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(err.message, "local name `x` is not bound before this use");
}

#[test]
fn check_function_the_public_api_still_has_no_sibling_visibility() {
    // `check_function` is a standalone entry point with no module
    // context, so it must keep working exactly as before: it only ever
    // sees its own signature (needed for recursion), never a sibling's.
    let function = HirItem::Function {
        name: "main".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![HirStmt::ExprStmt(HirExpr::Call {
            callee: "helper".to_string(),
            args: vec![],
        })],
    };
    let err = check_function(&function).unwrap_err();
    assert_eq!(err.code, "T0021");
}
