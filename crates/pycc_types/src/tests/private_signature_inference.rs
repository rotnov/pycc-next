//! Private-helper signature inference and nested constraint errors.

use super::*;

#[test]
fn private_parameter_is_inferred_by_forwarding_into_an_annotated_callee() {
    // Regression test (self-review finding, pre-merge): the solver used
    // to only unify a call argument against a callee's parameter when
    // the callee's own parameter term was itself unresolved. When the
    // callee is fully annotated (its parameter term is already
    // `Ok(Ty::Int)`), an unresolved *caller* argument variable never got
    // constrained in that direction, even though `unify_terms` itself
    // already supports it symmetrically -- so `_forward` below used to
    // fail with a spurious "add an annotation" T0021 instead of
    // correctly inferring `x: int` from forwarding into `_sink`.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "_sink".to_string(),
                params: vec![("value".to_string(), Ty::Int)],
                return_ty: Ty::None,
                body: vec![HirStmt::Return(None)],
            },
            HirItem::Function {
                name: "_forward".to_string(),
                params: vec![("x".to_string(), Ty::Infer)],
                return_ty: Ty::Infer,
                body: vec![HirStmt::ExprStmt(HirExpr::Call {
                    callee: "_sink".to_string(),
                    args: vec![HirExpr::Name("x".to_string())],
                })],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    check(&hir).unwrap();
}

#[test]
fn private_binary_helper_signature_is_inferred_across_operator_constraints() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "_add".to_string(),
                params: vec![
                    ("left".to_string(), Ty::Infer),
                    ("right".to_string(), Ty::Infer),
                ],
                return_ty: Ty::Infer,
                body: vec![HirStmt::Return(Some(HirExpr::BinOp {
                    op: BinOpKind::Add,
                    left: Box::new(HirExpr::Name("left".to_string())),
                    right: Box::new(HirExpr::Name("right".to_string())),
                }))],
            },
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
                callee: "_add".to_string(),
                args: vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    check(&hir).unwrap();
}

#[test]
fn private_true_division_helper_infers_a_float_return() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "_ratio".to_string(),
                params: vec![("value".to_string(), Ty::Infer)],
                return_ty: Ty::Infer,
                body: vec![HirStmt::Return(Some(HirExpr::BinOp {
                    op: BinOpKind::Div,
                    left: Box::new(HirExpr::Name("value".to_string())),
                    right: Box::new(HirExpr::IntLiteral(2)),
                }))],
            },
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "ratio".to_string(),
                value: HirExpr::Call {
                    callee: "_ratio".to_string(),
                    args: vec![HirExpr::IntLiteral(1)],
                },
            }),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    check(&hir).unwrap();
}

#[test]
fn private_helper_without_a_return_infers_none() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_log".to_string(),
            params: vec![],
            return_ty: Ty::Infer,
            body: vec![HirStmt::ExprStmt(HirExpr::Call {
                callee: "print".to_string(),
                args: vec![HirExpr::FString(vec![
                    FStringPart::Literal("equal=".to_string()),
                    FStringPart::Interpolation(Box::new(HirExpr::Compare {
                        op: CmpOpKind::Eq,
                        left: Box::new(HirExpr::IntLiteral(1)),
                        right: Box::new(HirExpr::IntLiteral(1)),
                    })),
                ])],
            })],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    check(&hir).unwrap();
}

#[test]
fn private_helper_with_a_bare_return_infers_none() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_stop".to_string(),
            params: vec![],
            return_ty: Ty::Infer,
            body: vec![HirStmt::Return(None)],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    check(&hir).unwrap();
}

#[test]
fn private_constant_helper_infers_a_float_return() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_constant".to_string(),
            params: vec![],
            return_ty: Ty::Infer,
            body: vec![HirStmt::Return(Some(HirExpr::FloatLiteral(1.5)))],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    check(&hir).unwrap();
}

#[test]
fn private_range_helper_infers_its_parameter_as_int() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_loop".to_string(),
            params: vec![("limit".to_string(), Ty::Infer)],
            return_ty: Ty::Infer,
            body: vec![HirStmt::ForRange {
                var: "item".to_string(),
                start: HirExpr::IntLiteral(0),
                stop: HirExpr::Name("limit".to_string()),
                step: HirExpr::IntLiteral(1),
                body: vec![HirStmt::While {
                    test: HirExpr::BoolLiteral(false),
                    body: vec![HirStmt::If {
                        test: HirExpr::BoolLiteral(true),
                        body: vec![HirStmt::ExprStmt(HirExpr::Call {
                            callee: "print".to_string(),
                            args: vec![HirExpr::Name("item".to_string())],
                        })],
                        orelse: vec![],
                    }],
                }],
            }],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    check(&hir).unwrap();
}

#[test]
fn unresolved_private_parameter_requests_an_annotation() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_constant".to_string(),
            params: vec![("unused".to_string(), Ty::Infer)],
            return_ty: Ty::Infer,
            body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(1)))],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("parameter `unused`"));
}

#[test]
fn unresolved_private_return_requests_an_annotation() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_unknown".to_string(),
            params: vec![],
            return_ty: Ty::Infer,
            body: vec![HirStmt::Return(Some(HirExpr::Name("missing".to_string())))],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("return type"));
}

#[test]
fn undefined_call_cannot_silently_resolve_a_private_return() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_unknown".to_string(),
            params: vec![],
            return_ty: Ty::Infer,
            body: vec![HirStmt::Return(Some(HirExpr::Call {
                callee: "missing".to_string(),
                args: vec![],
            }))],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert_eq!(check(&hir).unwrap_err().code, "T0021");
}

#[test]
fn unresolved_binary_operand_cannot_silently_resolve_a_private_return() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_unknown".to_string(),
            params: vec![],
            return_ty: Ty::Infer,
            body: vec![HirStmt::Return(Some(HirExpr::BinOp {
                op: BinOpKind::Add,
                left: Box::new(HirExpr::Name("missing".to_string())),
                right: Box::new(HirExpr::IntLiteral(1)),
            }))],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert_eq!(check(&hir).unwrap_err().code, "T0021");
}

#[test]
fn unresolved_private_binary_parameters_request_annotations() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_add".to_string(),
            params: vec![
                ("left".to_string(), Ty::Infer),
                ("right".to_string(), Ty::Infer),
            ],
            return_ty: Ty::Infer,
            body: vec![HirStmt::Return(Some(HirExpr::BinOp {
                op: BinOpKind::Add,
                left: Box::new(HirExpr::Name("left".to_string())),
                right: Box::new(HirExpr::Name("right".to_string())),
            }))],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert_eq!(check(&hir).unwrap_err().code, "T0021");
}

#[test]
fn unresolved_call_argument_does_not_invent_a_private_parameter_type() {
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
                args: vec![HirExpr::Name("missing".to_string())],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert_eq!(check(&hir).unwrap_err().code, "T0021");
}

#[test]
fn private_parameter_inference_rejects_conflicting_call_sites() {
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
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
                callee: "_identity".to_string(),
                args: vec![HirExpr::StringLiteral("one".to_string())],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert_eq!(check(&hir).unwrap_err().code, "T0021");
}

#[test]
fn private_return_inference_rejects_conflicting_return_types() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_choose".to_string(),
            params: vec![("condition".to_string(), Ty::Bool)],
            return_ty: Ty::Infer,
            body: vec![HirStmt::If {
                test: HirExpr::Name("condition".to_string()),
                body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(1)))],
                orelse: vec![HirStmt::Return(Some(HirExpr::StringLiteral(
                    "one".to_string(),
                )))],
            }],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert_eq!(check(&hir).unwrap_err().code, "T0022");
}

fn nested_private_call_conflict() -> HirExpr {
    HirExpr::Call {
        callee: "_sink".to_string(),
        args: vec![HirExpr::Call {
            callee: "_identity".to_string(),
            args: vec![HirExpr::StringLiteral("wrong".to_string())],
        }],
    }
}

fn private_constraint_error_fixture(stmt: HirStmt) -> HirModule {
    HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "_identity".to_string(),
                params: vec![("value".to_string(), Ty::Infer)],
                return_ty: Ty::Infer,
                body: vec![HirStmt::Return(Some(HirExpr::Name("value".to_string())))],
            },
            HirItem::Function {
                name: "_sink".to_string(),
                params: vec![("value".to_string(), Ty::Int)],
                return_ty: Ty::None,
                body: vec![HirStmt::Return(None)],
            },
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
                callee: "_identity".to_string(),
                args: vec![HirExpr::IntLiteral(1)],
            })),
            HirItem::Function {
                name: "_probe".to_string(),
                params: vec![],
                return_ty: Ty::Infer,
                body: vec![stmt],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    }
}

#[test]
fn private_f_string_and_return_propagate_nested_constraint_errors() {
    let stmt = HirStmt::Return(Some(HirExpr::FString(vec![FStringPart::Interpolation(
        Box::new(nested_private_call_conflict()),
    )])));
    assert_eq!(
        check(&private_constraint_error_fixture(stmt))
            .unwrap_err()
            .code,
        "T0021"
    );
}

#[test]
fn private_slice_base_wrapping_a_conflict_is_rejected_end_to_end() {
    // PR-12 Task 7 (D-118). NOTE what this test does and does NOT pin:
    // it proves `check()`'s real, end-to-end pipeline still rejects a
    // nested private-helper call conflict when it sits inside a
    // `Slice`'s `base`, *inside a private helper's own body* (not just
    // at module top level, which the pair above this whole group
    // already covers) -- a real and valuable thing to test on its own.
    // It does NOT, on its own, prove that `collect_expr_constraints`'s
    // `Slice` arm itself recurses into `base`: `check_with_environment_all`'s
    // Pass 3 independently re-type-checks every function body via
    // `infer_expr_in` regardless of what the solver's own
    // constraint-collection pass did or didn't visit, so this exact
    // conflict is still caught by Pass 3 even with the solver arm's
    // recursion mutated away to a bare `Ok(None)` no-op (confirmed by
    // temporarily making that mutation and rerunning this test: it kept
    // passing). `private_slice_base_recursion_pins_an_otherwise_unconstrained_parameter`
    // below is the test that actually is sensitive to the solver arm's
    // own recursion, using a construction where Pass 3 cannot mask a
    // missing visit (a `Ty::Infer` parameter whose *only* constraining
    // use anywhere in the module is inside a `Slice`'s `base`, so
    // signature resolution itself -- which gates whether Pass 3 ever
    // runs at all -- depends on the solver reaching it).
    let stmt = HirStmt::Return(Some(HirExpr::Slice {
        base: Box::new(nested_private_call_conflict()),
        start: None,
        stop: None,
        step: None,
    }));
    assert_eq!(
        check(&private_constraint_error_fixture(stmt))
            .unwrap_err()
            .code,
        "T0021"
    );
}

#[test]
fn private_slice_bound_wrapping_a_conflict_is_rejected_end_to_end() {
    // Same as above, but the conflict is nested inside a bound
    // (`start`), not `base`. Same caveat applies: this alone doesn't
    // pin the solver arm's own recursion (Pass 3 masks it identically),
    // see `private_slice_start_bound_recursion_pins_an_otherwise_unconstrained_parameter`
    // below for the test that actually does.
    let stmt = HirStmt::Return(Some(HirExpr::Slice {
        base: Box::new(HirExpr::IntLiteral(1)),
        start: Some(Box::new(nested_private_call_conflict())),
        stop: None,
        step: None,
    }));
    assert_eq!(
        check(&private_constraint_error_fixture(stmt))
            .unwrap_err()
            .code,
        "T0021"
    );
}

#[test]
fn private_slice_base_recursion_pins_an_otherwise_unconstrained_parameter() {
    // Load-bearing counterpart to the pair above (added after review:
    // those two tests' original doc comments overclaimed what they
    // pinned -- Pass 3 independently re-type-checks every function body
    // regardless of solver recursion, so a genuine conflict is caught
    // either way; verified by temporarily gutting the `Slice` arm in
    // `collect_expr_constraints` to a bare `Ok(None)` and confirming
    // those two tests still passed).
    //
    // This test's construction is different in a way that matters:
    // `_forward`'s own parameter `xs` is `Ty::Infer`, and its *only*
    // appearance anywhere in this module is wrapped in a call
    // (`_id_list(xs)`) embedded in a `Slice`'s `base`. Nothing else ever
    // constrains `xs`'s type. `collect_expr_constraints`'s `Call` arm is
    // the one place that performs real unification (`unify_terms`)
    // between an argument's term and the callee's own (here: concretely
    // annotated) parameter term -- so `xs`'s term becomes concretely
    // `list[int]` if and only if the solver actually visits
    // `_id_list(xs)`, which only happens if the outer `Slice` arm
    // recurses into `base`.
    //
    // If that recursion is missing, `xs`'s term never resolves, and
    // `infer_function_signatures_with_solver_all`'s own final resolution
    // loop raises "cannot infer type of parameter `xs`... add an
    // annotation" *before `check_with_signatures_all`/Pass 3 ever runs for
    // this function* -- Pass 3 cannot mask a failure that happens
    // before Pass 3 is even reached. Confirmed empirically: gutting the
    // `Slice` arm to a bare `Ok(None)` flips this test from `Ok(())` to
    // exactly that `Err`; restoring the real arm makes it pass again.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "_id_list".to_string(),
                params: vec![("v".to_string(), Ty::List(Box::new(Ty::Int)))],
                return_ty: Ty::List(Box::new(Ty::Int)),
                body: vec![HirStmt::Return(Some(HirExpr::Name("v".to_string())))],
            },
            HirItem::Function {
                name: "_forward".to_string(),
                params: vec![("xs".to_string(), Ty::Infer)],
                return_ty: Ty::List(Box::new(Ty::Int)),
                body: vec![HirStmt::Return(Some(HirExpr::Slice {
                    base: Box::new(HirExpr::Call {
                        callee: "_id_list".to_string(),
                        args: vec![HirExpr::Name("xs".to_string())],
                    }),
                    start: Some(Box::new(HirExpr::IntLiteral(1))),
                    stop: None,
                    step: None,
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
fn private_slice_start_bound_recursion_pins_an_otherwise_unconstrained_parameter() {
    // Same load-bearing shape as the test above, but the embedded call
    // sits in a bound (`start`) rather than `base`: `_forward`'s own
    // parameter `x` is `Ty::Infer` and is used *only* wrapped in
    // `_id(x)` inside the slice's `start` bound. A bare `Name("x")`
    // bound would NOT be load-bearing here -- looking up a name in
    // `collect_expr_constraints` performs no unification of its own,
    // only the `Call` arm does, which is why this test wraps `x` in a
    // call rather than mirroring the brief-review's own bare `xs[x:]`
    // sketch literally.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "_id".to_string(),
                params: vec![("v".to_string(), Ty::Int)],
                return_ty: Ty::Int,
                body: vec![HirStmt::Return(Some(HirExpr::Name("v".to_string())))],
            },
            HirItem::Function {
                name: "_forward".to_string(),
                params: vec![("x".to_string(), Ty::Infer)],
                return_ty: Ty::List(Box::new(Ty::Int)),
                body: vec![HirStmt::Return(Some(HirExpr::Slice {
                    base: Box::new(HirExpr::ListLiteral(vec![HirExpr::IntLiteral(1)])),
                    start: Some(Box::new(HirExpr::Call {
                        callee: "_id".to_string(),
                        args: vec![HirExpr::Name("x".to_string())],
                    })),
                    stop: None,
                    step: None,
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
fn private_if_body_propagates_nested_constraint_errors() {
    let stmt = HirStmt::If {
        test: HirExpr::BoolLiteral(true),
        body: vec![HirStmt::ExprStmt(nested_private_call_conflict())],
        orelse: vec![],
    };
    assert_eq!(
        check(&private_constraint_error_fixture(stmt))
            .unwrap_err()
            .code,
        "T0021"
    );
}

#[test]
fn private_while_body_propagates_nested_constraint_errors() {
    let stmt = HirStmt::While {
        test: HirExpr::BoolLiteral(true),
        body: vec![HirStmt::ExprStmt(nested_private_call_conflict())],
    };
    assert_eq!(
        check(&private_constraint_error_fixture(stmt))
            .unwrap_err()
            .code,
        "T0021"
    );
}

#[test]
fn private_range_operand_propagates_nested_constraint_errors() {
    let stmt = HirStmt::ForRange {
        var: "item".to_string(),
        start: HirExpr::IntLiteral(0),
        stop: nested_private_call_conflict(),
        step: HirExpr::IntLiteral(1),
        body: vec![],
    };
    assert_eq!(
        check(&private_constraint_error_fixture(stmt))
            .unwrap_err()
            .code,
        "T0021"
    );
}

#[test]
fn private_range_body_propagates_nested_constraint_errors() {
    let stmt = HirStmt::ForRange {
        var: "item".to_string(),
        start: HirExpr::IntLiteral(0),
        stop: HirExpr::IntLiteral(1),
        step: HirExpr::IntLiteral(1),
        body: vec![HirStmt::ExprStmt(nested_private_call_conflict())],
    };
    assert_eq!(
        check(&private_constraint_error_fixture(stmt))
            .unwrap_err()
            .code,
        "T0021"
    );
}

#[test]
fn private_compare_left_propagates_nested_constraint_errors() {
    let stmt = HirStmt::ExprStmt(HirExpr::Compare {
        op: CmpOpKind::Eq,
        left: Box::new(nested_private_call_conflict()),
        right: Box::new(HirExpr::IntLiteral(1)),
    });
    assert_eq!(
        check(&private_constraint_error_fixture(stmt))
            .unwrap_err()
            .code,
        "T0021"
    );
}

#[test]
fn private_compare_right_propagates_nested_constraint_errors() {
    let stmt = HirStmt::ExprStmt(HirExpr::Compare {
        op: CmpOpKind::Eq,
        left: Box::new(HirExpr::IntLiteral(1)),
        right: Box::new(nested_private_call_conflict()),
    });
    assert_eq!(
        check(&private_constraint_error_fixture(stmt))
            .unwrap_err()
            .code,
        "T0021"
    );
}

#[test]
fn private_binary_left_propagates_nested_constraint_errors() {
    let stmt = HirStmt::ExprStmt(HirExpr::BinOp {
        op: BinOpKind::Add,
        left: Box::new(nested_private_call_conflict()),
        right: Box::new(HirExpr::IntLiteral(1)),
    });
    assert_eq!(
        check(&private_constraint_error_fixture(stmt))
            .unwrap_err()
            .code,
        "T0021"
    );
}

#[test]
fn private_binary_right_propagates_nested_constraint_errors() {
    let stmt = HirStmt::ExprStmt(HirExpr::BinOp {
        op: BinOpKind::Add,
        left: Box::new(HirExpr::IntLiteral(1)),
        right: Box::new(nested_private_call_conflict()),
    });
    assert_eq!(
        check(&private_constraint_error_fixture(stmt))
            .unwrap_err()
            .code,
        "T0021"
    );
}

#[test]
fn private_assignment_propagates_nested_constraint_errors() {
    let stmt = HirStmt::Assign {
        target: "value".to_string(),
        value: nested_private_call_conflict(),
    };
    assert_eq!(
        check(&private_constraint_error_fixture(stmt))
            .unwrap_err()
            .code,
        "T0021"
    );
}

#[test]
fn private_assignment_with_an_unresolved_value_is_checked_after_inference() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_assign".to_string(),
            params: vec![],
            return_ty: Ty::Infer,
            body: vec![HirStmt::Assign {
                target: "value".to_string(),
                value: HirExpr::Name("missing".to_string()),
            }],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert_eq!(check(&hir).unwrap_err().code, "T0021");
}

#[test]
fn private_if_test_propagates_nested_constraint_errors() {
    let stmt = HirStmt::If {
        test: nested_private_call_conflict(),
        body: vec![],
        orelse: vec![],
    };
    assert_eq!(
        check(&private_constraint_error_fixture(stmt))
            .unwrap_err()
            .code,
        "T0021"
    );
}

#[test]
fn private_while_test_propagates_nested_constraint_errors() {
    let stmt = HirStmt::While {
        test: nested_private_call_conflict(),
        body: vec![],
    };
    assert_eq!(
        check(&private_constraint_error_fixture(stmt))
            .unwrap_err()
            .code,
        "T0021"
    );
}

#[test]
fn private_range_parameter_rejects_a_conflicting_call_site_type() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "_loop".to_string(),
                params: vec![("limit".to_string(), Ty::Infer)],
                return_ty: Ty::Infer,
                body: vec![HirStmt::ForRange {
                    var: "item".to_string(),
                    start: HirExpr::IntLiteral(0),
                    stop: HirExpr::Name("limit".to_string()),
                    step: HirExpr::IntLiteral(1),
                    body: vec![],
                }],
            },
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
                callee: "_loop".to_string(),
                args: vec![HirExpr::StringLiteral("wrong".to_string())],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert_eq!(check(&hir).unwrap_err().code, "T0021");
}

#[test]
fn private_implicit_none_return_rejects_an_int_constrained_call_site() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "_noop".to_string(),
                params: vec![],
                return_ty: Ty::Infer,
                body: vec![],
            },
            HirItem::TopLevelStmt(HirStmt::ForRange {
                var: "item".to_string(),
                start: HirExpr::IntLiteral(0),
                stop: HirExpr::Call {
                    callee: "_noop".to_string(),
                    args: vec![],
                },
                step: HirExpr::IntLiteral(1),
                body: vec![],
            }),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert_eq!(check(&hir).unwrap_err().code, "T0022");
}

#[test]
fn private_division_return_rejects_an_int_constrained_call_site() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "_ratio".to_string(),
                params: vec![("value".to_string(), Ty::Infer)],
                return_ty: Ty::Infer,
                body: vec![HirStmt::Return(Some(HirExpr::BinOp {
                    op: BinOpKind::Div,
                    left: Box::new(HirExpr::Name("value".to_string())),
                    right: Box::new(HirExpr::IntLiteral(2)),
                }))],
            },
            HirItem::TopLevelStmt(HirStmt::ForRange {
                var: "item".to_string(),
                start: HirExpr::IntLiteral(0),
                stop: HirExpr::Call {
                    callee: "_ratio".to_string(),
                    args: vec![HirExpr::IntLiteral(4)],
                },
                step: HirExpr::IntLiteral(1),
                body: vec![],
            }),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert_eq!(check(&hir).unwrap_err().code, "T0021");
}

#[test]
fn private_binary_constraint_rejects_incompatible_resolved_operands() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "_bad_add".to_string(),
                params: vec![("value".to_string(), Ty::Infer)],
                return_ty: Ty::Infer,
                body: vec![HirStmt::Return(Some(HirExpr::BinOp {
                    op: BinOpKind::Add,
                    left: Box::new(HirExpr::Name("value".to_string())),
                    right: Box::new(HirExpr::StringLiteral("wrong".to_string())),
                }))],
            },
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
                callee: "_bad_add".to_string(),
                args: vec![HirExpr::IntLiteral(1)],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert_eq!(check(&hir).unwrap_err().code, "T0021");
}

#[test]
fn direct_check_function_rejects_an_unresolved_private_signature() {
    let function = HirItem::Function {
        name: "_identity".to_string(),
        params: vec![("value".to_string(), Ty::Infer)],
        return_ty: Ty::Infer,
        body: vec![HirStmt::Return(Some(HirExpr::Name("value".to_string())))],
    };
    assert_eq!(check_function(&function).unwrap_err().code, "T0021");
}
