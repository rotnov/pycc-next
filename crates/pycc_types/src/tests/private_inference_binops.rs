//! Private-helper inference through binops and string repetition.

use super::*;

#[test]
fn private_identity_signature_is_inferred_from_its_call_site_and_return() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "_identity".to_string(),
                params: vec![("value".to_string(), Ty::Infer)],
                return_ty: Ty::Infer,
                body: vec![HirStmt::Return(Some(HirExpr::Name("value".to_string())))],
            },
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
                callee: "_identity".to_string(),
                args: vec![HirExpr::IntLiteral(1)],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    check(&hir).unwrap();
}

#[test]
fn check_and_resolve_takes_the_fast_path_for_an_already_concrete_valid_module() {
    // Pre-existing gap noticed while chasing this task's own 100%
    // coverage requirement: every other `check_and_resolve` test in this
    // file uses a `Ty::Infer` signature, so `checked_function_signatures_all`'s
    // fast-path *success* return (`concrete_function_signatures` is
    // `Some` and `check_with_signatures_all` succeeds) was never exercised.
    // Unrelated to this task's list-typing work, but cheap and in-scope
    // to close here rather than leave dangling.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "identity".to_string(),
            params: vec![("value".to_string(), Ty::Int)],
            return_ty: Ty::Int,
            body: vec![HirStmt::Return(Some(HirExpr::Name("value".to_string())))],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let resolved = check_and_resolve(&hir).unwrap();
    assert_eq!(resolved, hir);
}

#[test]
fn check_and_resolve_materializes_private_signatures_without_mutating_input() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "_identity".to_string(),
                params: vec![("value".to_string(), Ty::Infer)],
                return_ty: Ty::Infer,
                body: vec![HirStmt::Return(Some(HirExpr::Name("value".to_string())))],
            },
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
                callee: "_identity".to_string(),
                args: vec![HirExpr::IntLiteral(1)],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let resolved = check_and_resolve(&hir).unwrap();
    assert_eq!(
        resolved.items[0],
        HirItem::Function {
            name: "_identity".to_string(),
            params: vec![("value".to_string(), Ty::Int)],
            return_ty: Ty::Int,
            body: vec![HirStmt::Return(Some(HirExpr::Name("value".to_string())))],
        }
    );
    assert_eq!(
        hir.items[0],
        HirItem::Function {
            name: "_identity".to_string(),
            params: vec![("value".to_string(), Ty::Infer)],
            return_ty: Ty::Infer,
            body: vec![HirStmt::Return(Some(HirExpr::Name("value".to_string())))],
        }
    );
}

#[test]
fn annotated_int_result_propagates_back_to_a_private_binary_parameter() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_inc".to_string(),
            params: vec![("value".to_string(), Ty::Infer)],
            return_ty: Ty::Int,
            body: vec![HirStmt::Return(Some(HirExpr::BinOp {
                op: BinOpKind::Add,
                left: Box::new(HirExpr::Name("value".to_string())),
                right: Box::new(HirExpr::IntLiteral(1)),
            }))],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let resolved = check_and_resolve(&hir).unwrap();
    assert_eq!(
        resolved.items[0],
        HirItem::Function {
            name: "_inc".to_string(),
            params: vec![("value".to_string(), Ty::Int)],
            return_ty: Ty::Int,
            body: vec![HirStmt::Return(Some(HirExpr::BinOp {
                op: BinOpKind::Add,
                left: Box::new(HirExpr::Name("value".to_string())),
                right: Box::new(HirExpr::IntLiteral(1)),
            }))],
        }
    );
}

// ---- #574: string repetition through both consumers of
// `numeric_result_type` — the validation pass (`infer_expr_in`'s
// `HirExpr::BinOp` arm) and the private-helper constraint solver
// (`propagate_binop_constraints`). ----

#[test]
fn a_public_function_may_return_string_repetition() {
    // Validation-pass consumer: `check` never runs the solver for a
    // fully annotated module.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "banner".to_string(),
            params: vec![("count".to_string(), Ty::Int)],
            return_ty: Ty::Str,
            body: vec![HirStmt::Return(Some(HirExpr::BinOp {
                op: BinOpKind::Mul,
                left: Box::new(HirExpr::StringLiteral("ab".to_string())),
                right: Box::new(HirExpr::Name("count".to_string())),
            }))],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    assert!(check(&hir).is_ok());
}

#[test]
fn a_public_function_may_return_string_repetition_with_the_count_first() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "banner".to_string(),
            params: vec![("count".to_string(), Ty::Bool)],
            return_ty: Ty::Str,
            body: vec![HirStmt::Return(Some(HirExpr::BinOp {
                op: BinOpKind::Mul,
                left: Box::new(HirExpr::Name("count".to_string())),
                right: Box::new(HirExpr::StringLiteral("ab".to_string())),
            }))],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    assert!(check(&hir).is_ok());
}

#[test]
fn a_public_function_still_cannot_multiply_a_str_by_a_float() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "banner".to_string(),
            params: vec![("count".to_string(), Ty::Float)],
            return_ty: Ty::Str,
            body: vec![HirStmt::Return(Some(HirExpr::BinOp {
                op: BinOpKind::Mul,
                left: Box::new(HirExpr::StringLiteral("ab".to_string())),
                right: Box::new(HirExpr::Name("count".to_string())),
            }))],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
}

#[test]
fn a_private_helper_infers_str_from_string_repetition() {
    // Constraint-solver consumer: the helper's parameter and return type
    // are both `Ty::Infer`, so `check_and_resolve` reaches
    // `propagate_binop_constraints`.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "_rep".to_string(),
                params: vec![("count".to_string(), Ty::Infer)],
                return_ty: Ty::Infer,
                body: vec![HirStmt::Return(Some(HirExpr::BinOp {
                    op: BinOpKind::Mul,
                    left: Box::new(HirExpr::StringLiteral("ab".to_string())),
                    right: Box::new(HirExpr::Name("count".to_string())),
                }))],
            },
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
                callee: "_rep".to_string(),
                args: vec![HirExpr::IntLiteral(3)],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let resolved = check_and_resolve(&hir).unwrap();
    // Extracted with `assert!(matches!(..))` rather than a `let ... else
    // { unreachable!() }`, because the `else` arm would have been an
    // unexercised line and region under the D-014 gate while these tests
    // lived in `lib.rs`. This is the idiom the surrounding
    // private-inference tests already use.
    assert!(matches!(
        &resolved.items[0],
        HirItem::Function { params, return_ty, .. }
            if params == &vec![("count".to_string(), Ty::Int)] && *return_ty == Ty::Str
    ));
}

#[test]
fn a_private_helper_infers_str_with_the_count_on_the_left() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "_rep".to_string(),
                params: vec![("count".to_string(), Ty::Infer)],
                return_ty: Ty::Infer,
                body: vec![HirStmt::Return(Some(HirExpr::BinOp {
                    op: BinOpKind::Mul,
                    left: Box::new(HirExpr::Name("count".to_string())),
                    right: Box::new(HirExpr::StringLiteral("ab".to_string())),
                }))],
            },
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
                callee: "_rep".to_string(),
                args: vec![HirExpr::IntLiteral(3)],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let resolved = check_and_resolve(&hir).unwrap();
    assert!(matches!(
        &resolved.items[0],
        HirItem::Function { return_ty, .. } if *return_ty == Ty::Str
    ));
}

#[test]
fn a_private_helper_infers_str_from_a_bool_repetition_count() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "_rep".to_string(),
                params: vec![("count".to_string(), Ty::Infer)],
                return_ty: Ty::Infer,
                body: vec![HirStmt::Return(Some(HirExpr::BinOp {
                    op: BinOpKind::Mul,
                    left: Box::new(HirExpr::StringLiteral("ab".to_string())),
                    right: Box::new(HirExpr::Name("count".to_string())),
                }))],
            },
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
                callee: "_rep".to_string(),
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
        HirItem::Function { return_ty, .. } if *return_ty == Ty::Str
    ));
}

#[test]
fn a_private_helper_still_rejects_a_float_repetition_count() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "_rep".to_string(),
                params: vec![("count".to_string(), Ty::Infer)],
                return_ty: Ty::Infer,
                body: vec![HirStmt::Return(Some(HirExpr::BinOp {
                    op: BinOpKind::Mul,
                    left: Box::new(HirExpr::StringLiteral("ab".to_string())),
                    right: Box::new(HirExpr::Name("count".to_string())),
                }))],
            },
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
                callee: "_rep".to_string(),
                args: vec![HirExpr::FloatLiteral(1.5)],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let err = check_and_resolve(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
}

#[test]
fn a_private_helper_still_rejects_subtracting_an_int_from_a_str() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "_rep".to_string(),
                params: vec![("count".to_string(), Ty::Infer)],
                return_ty: Ty::Infer,
                body: vec![HirStmt::Return(Some(HirExpr::BinOp {
                    op: BinOpKind::Sub,
                    left: Box::new(HirExpr::StringLiteral("ab".to_string())),
                    right: Box::new(HirExpr::Name("count".to_string())),
                }))],
            },
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
                callee: "_rep".to_string(),
                args: vec![HirExpr::IntLiteral(3)],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let err = check_and_resolve(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
}

#[test]
fn a_repetition_over_an_undefined_operand_is_still_reported() {
    // Validation pass: repetition acceptance must not swallow operand
    // diagnostics raised before `numeric_result_type` ever sees a pair
    // of types.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "banner".to_string(),
            params: vec![("count".to_string(), Ty::Int)],
            return_ty: Ty::Str,
            body: vec![HirStmt::Return(Some(HirExpr::BinOp {
                op: BinOpKind::Mul,
                left: Box::new(HirExpr::Name("missing".to_string())),
                right: Box::new(HirExpr::Name("count".to_string())),
            }))],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("missing"));
}

#[test]
fn a_private_helper_repetition_over_an_undefined_operand_fails_closed() {
    // Solver path: an operand that never resolves leaves the helper's
    // return term unresolved, and `resolved_private_signature_term`
    // fails closed with "cannot infer return type" rather than
    // inventing a signature. Repetition does not change that.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "_rep".to_string(),
                params: vec![("count".to_string(), Ty::Infer)],
                return_ty: Ty::Infer,
                body: vec![HirStmt::Return(Some(HirExpr::BinOp {
                    op: BinOpKind::Mul,
                    left: Box::new(HirExpr::Name("missing".to_string())),
                    right: Box::new(HirExpr::Name("count".to_string())),
                }))],
            },
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
                callee: "_rep".to_string(),
                args: vec![HirExpr::IntLiteral(3)],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let err = check_and_resolve(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("cannot infer return type"));
    assert!(err.message.contains("_rep"));
}

#[test]
fn annotated_int_result_propagates_back_to_a_right_binary_parameter() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_inc".to_string(),
            params: vec![("value".to_string(), Ty::Infer)],
            return_ty: Ty::Int,
            body: vec![HirStmt::Return(Some(HirExpr::BinOp {
                op: BinOpKind::Add,
                left: Box::new(HirExpr::IntLiteral(1)),
                right: Box::new(HirExpr::Name("value".to_string())),
            }))],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let resolved = check_and_resolve(&hir).unwrap();
    assert_eq!(
        resolved.items[0],
        HirItem::Function {
            name: "_inc".to_string(),
            params: vec![("value".to_string(), Ty::Int)],
            return_ty: Ty::Int,
            body: vec![HirStmt::Return(Some(HirExpr::BinOp {
                op: BinOpKind::Add,
                left: Box::new(HirExpr::IntLiteral(1)),
                right: Box::new(HirExpr::Name("value".to_string())),
            }))],
        }
    );
}

#[test]
fn annotated_int_result_rejects_a_known_string_left_operand() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_bad".to_string(),
            params: vec![("value".to_string(), Ty::Infer)],
            return_ty: Ty::Int,
            body: vec![HirStmt::Return(Some(HirExpr::BinOp {
                op: BinOpKind::Add,
                left: Box::new(HirExpr::StringLiteral("wrong".to_string())),
                right: Box::new(HirExpr::Name("value".to_string())),
            }))],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert_eq!(check(&hir).unwrap_err().code, "T0021");
}

#[test]
fn annotated_int_result_rejects_a_known_string_right_operand() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_bad".to_string(),
            params: vec![("value".to_string(), Ty::Infer)],
            return_ty: Ty::Int,
            body: vec![HirStmt::Return(Some(HirExpr::BinOp {
                op: BinOpKind::Add,
                left: Box::new(HirExpr::Name("value".to_string())),
                right: Box::new(HirExpr::StringLiteral("wrong".to_string())),
            }))],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert_eq!(check(&hir).unwrap_err().code, "T0021");
}
