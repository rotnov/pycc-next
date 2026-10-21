//! Validation-only fast path vs. the constraint solver path; solver unification.

use super::*;

#[test]
fn concrete_signatures_take_the_validation_only_fast_path() {
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

    assert_eq!(
        concrete_function_signatures(&hir).unwrap()["identity"],
        (vec![Ty::Int], Ty::Int)
    );
    assert!(check(&hir).is_ok());
}

#[test]
fn inferred_signatures_keep_the_constraint_solver_path() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_identity".to_string(),
            params: vec![("value".to_string(), Ty::Infer)],
            return_ty: Ty::Infer,
            body: vec![HirStmt::Return(Some(HirExpr::Name("value".to_string())))],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    assert!(concrete_function_signatures(&hir).is_none());
}

#[test]
fn concrete_fast_path_preserves_solver_first_diagnostic_selection() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "takes_int".to_string(),
                params: vec![("value".to_string(), Ty::Int)],
                return_ty: Ty::None,
                body: vec![HirStmt::Return(None)],
            },
            HirItem::Function {
                name: "broken".to_string(),
                params: vec![],
                return_ty: Ty::Int,
                body: vec![
                    HirStmt::ExprStmt(HirExpr::Call {
                        callee: "takes_int".to_string(),
                        args: vec![HirExpr::StringLiteral("wrong".to_string())],
                    }),
                    HirStmt::Return(Some(HirExpr::StringLiteral("wrong".to_string()))),
                ],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let local_names = module_function_local_names(&hir);
    let concrete = concrete_function_signatures(&hir).unwrap();
    let validation_first = check_with_signatures(&hir, &concrete, &local_names).unwrap_err();
    let solver_first = infer_function_signatures_with_solver(&hir, &local_names).unwrap_err();
    let fast_path = checked_function_signatures(&hir, &local_names).unwrap_err();
    let public_check = check(&hir).unwrap_err();

    assert_eq!(validation_first.code, "T0021");
    assert_eq!(solver_first.code, "T0022");
    assert_eq!(fast_path.code, solver_first.code);
    assert_eq!(fast_path.message, solver_first.message);
    assert_eq!(public_check.code, solver_first.code);
    assert_eq!(public_check.message, solver_first.message);
    // #868 (D-220): the per-function list keeps the same selection at index
    // 0, and `broken` is reported exactly once (both phases flag it; the
    // solver's entry wins by key, the checker's `Some(1)` entry is dropped).
    let all = check_all(&hir).unwrap_err();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].code, solver_first.code);
    assert_eq!(all[0].message, solver_first.message);
    let fast_path_all =
        crate::module::checked_function_signatures_all(&hir, &local_names).unwrap_err();
    assert_eq!(fast_path_all.len(), 1);
    assert_eq!(fast_path_all[0].1.message, solver_first.message);
}

#[test]
fn a_list_literal_still_type_checks_correctly_when_an_unrelated_private_helper_forces_the_solver_path()
 {
    // Any `Ty::Infer` signature anywhere in the module routes `check`
    // through `infer_function_signatures_with_solver_all` first (see
    // `check_all`), which runs `collect_expr_constraints`
    // over every expression in the module, including this list literal.
    // That solver-side pass must stay lenient (`Ok(None)`, recurse only)
    // for list forms -- confirms it doesn't wrongly reject a valid list.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "xs".to_string(),
                value: HirExpr::ListLiteral(vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)]),
            }),
            HirItem::Function {
                name: "_constant".to_string(),
                params: vec![],
                return_ty: Ty::Infer,
                body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(1)))],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert!(check(&hir).is_ok());
}

#[test]
fn a_heterogeneous_list_literal_is_still_rejected_when_the_solver_path_runs_first() {
    // The load-bearing counterpart to the test above: the solver's
    // leniency must not swallow a genuine list error -- it has to fall
    // through to the real, list-aware check pass (`check_with_signatures_all`)
    // that runs after the solver, which is what actually raises T0032.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "xs".to_string(),
                value: HirExpr::ListLiteral(vec![
                    HirExpr::IntLiteral(1),
                    HirExpr::BoolLiteral(true),
                ]),
            }),
            HirItem::Function {
                name: "_constant".to_string(),
                params: vec![],
                return_ty: Ty::Infer,
                body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(1)))],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert_eq!(check(&hir).unwrap_err().code, "T0032");
}

#[test]
fn a_for_list_loop_still_type_checks_correctly_when_the_solver_path_runs_first() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "xs".to_string(),
                value: HirExpr::ListLiteral(vec![HirExpr::IntLiteral(1)]),
            }),
            HirItem::TopLevelStmt(HirStmt::ForList {
                var: "i".to_string(),
                list: "xs".to_string(),
                body: vec![],
            }),
            HirItem::Function {
                name: "_constant".to_string(),
                params: vec![],
                return_ty: Ty::Infer,
                body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(1)))],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert!(check(&hir).is_ok());
}

// PR-12 Task 7 (D-118): whole-module `check()` tests for `HirExpr::Slice`,
// exercising the real statement walkers (`check_with_environment_all`'s
// `check_stmt`, and `collect_block_constraints`'s `Assign` arm under the
// solver path) rather than calling `infer_expr`/`collect_expr_constraints`
// directly on a hand-built expression. Mirrors the list-literal pair
// immediately above: Task 3's own ledger records a real regression of
// exactly this shape (a solver arm that looked correct in isolation but
// was never actually reached by the block walker) -- these confirm the
// `Slice` arm added in this task is genuinely wired into both paths, not
// just correct when invoked directly.

#[test]
fn slicing_type_checks_through_the_full_check_pipeline_on_the_fast_path() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "xs".to_string(),
                value: HirExpr::ListLiteral(vec![
                    HirExpr::IntLiteral(1),
                    HirExpr::IntLiteral(2),
                    HirExpr::IntLiteral(3),
                ]),
            }),
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "ys".to_string(),
                value: HirExpr::Slice {
                    base: Box::new(HirExpr::Name("xs".to_string())),
                    start: Some(Box::new(HirExpr::IntLiteral(1))),
                    stop: None,
                    step: None,
                },
            }),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert!(check(&hir).is_ok());
}

#[test]
fn slicing_a_list_of_str_type_checks_through_the_full_check_pipeline() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "xs".to_string(),
                value: HirExpr::ListLiteral(vec![HirExpr::StringLiteral("a".to_string())]),
            }),
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "ys".to_string(),
                value: HirExpr::Slice {
                    base: Box::new(HirExpr::Name("xs".to_string())),
                    start: None,
                    stop: None,
                    step: None,
                },
            }),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert!(check(&hir).is_ok());
}

#[test]
fn slicing_type_checks_correctly_when_an_unrelated_private_helper_forces_the_solver_path() {
    // Companion to `a_list_literal_still_type_checks_correctly_when_an_unrelated_private_helper_forces_the_solver_path`
    // above: proves `collect_block_constraints`'s ordinary `Assign` arm
    // reaches this task's new `HirExpr::Slice` constraint-collection arm
    // (which must stay lenient, `Ok(None)`, recursing only) without
    // wrongly rejecting a valid slice, and that the real check pass
    // (`check_with_signatures_all`) run afterward still type-checks it.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "xs".to_string(),
                value: HirExpr::ListLiteral(vec![
                    HirExpr::IntLiteral(1),
                    HirExpr::IntLiteral(2),
                    HirExpr::IntLiteral(3),
                ]),
            }),
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "ys".to_string(),
                value: HirExpr::Slice {
                    base: Box::new(HirExpr::Name("xs".to_string())),
                    start: Some(Box::new(HirExpr::IntLiteral(1))),
                    stop: Some(Box::new(HirExpr::IntLiteral(3))),
                    step: None,
                },
            }),
            HirItem::Function {
                name: "_constant".to_string(),
                params: vec![],
                return_ty: Ty::Infer,
                body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(1)))],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert!(check(&hir).is_ok());
}

#[test]
fn slicing_a_list_of_str_still_type_checks_when_the_solver_path_runs_first() {
    // The solver's leniency for `Slice` leaves a `list[str]` slice to the
    // real, list-aware check pass that runs after it.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "xs".to_string(),
                value: HirExpr::ListLiteral(vec![HirExpr::StringLiteral("a".to_string())]),
            }),
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "ys".to_string(),
                value: HirExpr::Slice {
                    base: Box::new(HirExpr::Name("xs".to_string())),
                    start: None,
                    stop: None,
                    step: None,
                },
            }),
            HirItem::Function {
                name: "_constant".to_string(),
                params: vec![],
                return_ty: Ty::Infer,
                body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(1)))],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert!(check(&hir).is_ok());
}

#[test]
fn homogeneous_private_solver_scalar_list_element_rejects_an_empty_slice() {
    assert_eq!(homogeneous_private_solver_scalar_list_element(&[]), None);
}

#[test]
fn homogeneous_private_solver_scalar_list_element_rejects_a_none_element_term() {
    let terms = vec![Some(Ok(Ty::Int)), None];
    assert_eq!(homogeneous_private_solver_scalar_list_element(&terms), None);
}

#[test]
fn homogeneous_private_solver_scalar_list_element_rejects_a_none_first_element() {
    // Covers the `.as_ref()?` None path when the first element itself is
    // `None` (distinct from the test above where `None` is at index 1).
    let terms = vec![None, Some(Ok(Ty::Int))];
    assert_eq!(homogeneous_private_solver_scalar_list_element(&terms), None);
}

#[test]
fn homogeneous_private_solver_scalar_list_element_rejects_an_err_element_term() {
    let terms = vec![Some(Ok(Ty::Int)), Some(Err(0))];
    assert_eq!(homogeneous_private_solver_scalar_list_element(&terms), None);
}

#[test]
fn homogeneous_private_solver_scalar_list_element_rejects_an_err_first_element() {
    // Covers the `.ok()?` None path when the first element is `Some(Err(_))`
    // (distinct from the test above where `Err` is at index 1).
    let terms = vec![Some(Err(0)), Some(Ok(Ty::Int))];
    assert_eq!(homogeneous_private_solver_scalar_list_element(&terms), None);
}

#[test]
fn homogeneous_private_solver_scalar_list_element_rejects_a_non_scalar_first_element() {
    let terms = vec![Some(Ok(Ty::List(Box::new(Ty::Int))))];
    assert_eq!(homogeneous_private_solver_scalar_list_element(&terms), None);
}

#[test]
fn homogeneous_private_solver_scalar_list_element_rejects_heterogeneous_scalars() {
    let terms = vec![Some(Ok(Ty::Int)), Some(Ok(Ty::Bool))];
    // Exact `Ty` equality, NOT `merge_inferred_types` -- `bool` and `int`
    // are not merged even though `merge_inferred_types` would widen them.
    assert_eq!(homogeneous_private_solver_scalar_list_element(&terms), None);
}

#[test]
fn homogeneous_private_solver_scalar_list_element_accepts_homogeneous_scalars() {
    let terms = vec![Some(Ok(Ty::Int)), Some(Ok(Ty::Int))];
    assert_eq!(
        homogeneous_private_solver_scalar_list_element(&terms),
        Some(Ty::Int)
    );
}

#[test]
fn homogeneous_private_solver_scalar_list_element_accepts_a_single_scalar() {
    let terms = vec![Some(Ok(Ty::Str))];
    assert_eq!(
        homogeneous_private_solver_scalar_list_element(&terms),
        Some(Ty::Str)
    );
}

#[test]
fn type_solver_covers_concrete_and_union_merge_paths() {
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    assert!(
        !unify_terms(
            Ok(Ty::Int),
            Ok(Ty::Bool),
            &mut parents,
            &mut concrete,
            "T0021",
            "test",
        )
        .unwrap()
    );
    assert!(
        unify_terms(
            Ok(Ty::Int),
            Ok(Ty::Str),
            &mut parents,
            &mut concrete,
            "T0021",
            "test",
        )
        .is_err()
    );

    let empty_left = fresh_term(&mut parents, &mut concrete);
    let empty_right = fresh_term(&mut parents, &mut concrete);
    assert!(
        unify_terms(
            empty_left.clone(),
            empty_right.clone(),
            &mut parents,
            &mut concrete,
            "T0021",
            "test",
        )
        .unwrap()
    );
    assert!(
        !unify_terms(
            empty_left,
            empty_right,
            &mut parents,
            &mut concrete,
            "T0021",
            "test",
        )
        .unwrap()
    );

    let typed_left = fresh_term(&mut parents, &mut concrete);
    let typed_right = fresh_term(&mut parents, &mut concrete);
    unify_terms(
        typed_left.clone(),
        Ok(Ty::Bool),
        &mut parents,
        &mut concrete,
        "T0021",
        "test",
    )
    .unwrap();
    unify_terms(
        typed_right.clone(),
        Ok(Ty::Int),
        &mut parents,
        &mut concrete,
        "T0021",
        "test",
    )
    .unwrap();
    unify_terms(
        typed_left,
        typed_right.clone(),
        &mut parents,
        &mut concrete,
        "T0021",
        "test",
    )
    .unwrap();
    assert_eq!(
        resolved_term(typed_right, &mut parents, &concrete),
        Some(Ty::Int)
    );

    let typed = fresh_term(&mut parents, &mut concrete);
    let empty = fresh_term(&mut parents, &mut concrete);
    unify_terms(
        typed.clone(),
        Ok(Ty::Str),
        &mut parents,
        &mut concrete,
        "T0021",
        "test",
    )
    .unwrap();
    unify_terms(
        typed,
        empty.clone(),
        &mut parents,
        &mut concrete,
        "T0021",
        "test",
    )
    .unwrap();
    assert_eq!(resolved_term(empty, &mut parents, &concrete), Some(Ty::Str));

    let conflicting_left = fresh_term(&mut parents, &mut concrete);
    let conflicting_right = fresh_term(&mut parents, &mut concrete);
    unify_terms(
        conflicting_left.clone(),
        Ok(Ty::Int),
        &mut parents,
        &mut concrete,
        "T0021",
        "test",
    )
    .unwrap();
    unify_terms(
        conflicting_right.clone(),
        Ok(Ty::Str),
        &mut parents,
        &mut concrete,
        "T0021",
        "test",
    )
    .unwrap();
    assert!(
        unify_terms(
            conflicting_left,
            conflicting_right,
            &mut parents,
            &mut concrete,
            "T0021",
            "test",
        )
        .is_err()
    );

    let reversed = fresh_term(&mut parents, &mut concrete);
    assert!(
        unify_terms(
            Ok(Ty::Float),
            reversed.clone(),
            &mut parents,
            &mut concrete,
            "T0021",
            "test",
        )
        .unwrap()
    );
    assert_eq!(
        resolved_term(reversed, &mut parents, &concrete),
        Some(Ty::Float)
    );
}

/// #949: with a known declared type, `unify_terms_with_declared` reports a
/// declared-vs-actual mismatch instead of the ambiguous "conflicting inferred
/// types" wording, and picks the actual operand by comparing against the
/// declared type rather than by position -- the `(Ok, Ok)` arm passes the
/// declared type first, while the `(Err(var), Ok(ty)) | (Ok(ty), Err(var))`
/// arm passes the already-inferred actual first and the declared type second.
#[test]
fn unify_terms_with_declared_names_the_declared_type_in_either_operand_order() {
    let mut parents = Vec::new();
    let mut concrete = Vec::new();

    // `(Ok, Ok)`: declared is the left operand.
    let declared_first = unify_terms_with_declared(
        Ok(Ty::Int),
        Ok(Ty::Str),
        &mut parents,
        &mut concrete,
        "T0022",
        "declared return type",
        Some(&Ty::Int),
    )
    .unwrap_err();
    assert_eq!(declared_first.code, "T0022");
    assert_eq!(
        declared_first.message,
        "return type mismatch: expected `int`, found `str`"
    );
    assert_eq!(declared_first.help.as_deref(), Some("return a `int` value"));

    // `(Ok(ty), Err(var))` with the variable already resolved to `str`: the
    // conflict is raised with the *actual* type first, so a positional
    // assumption would invert the message.
    let inferred = fresh_term(&mut parents, &mut concrete);
    unify_terms(
        inferred.clone(),
        Ok(Ty::Str),
        &mut parents,
        &mut concrete,
        "T0022",
        "private helper return type",
    )
    .unwrap();
    let declared_second = unify_terms_with_declared(
        Ok(Ty::Int),
        inferred.clone(),
        &mut parents,
        &mut concrete,
        "T0022",
        "declared return type",
        Some(&Ty::Int),
    )
    .unwrap_err();
    assert_eq!(
        declared_second.message,
        "return type mismatch: expected `int`, found `str`"
    );
    assert_eq!(
        declared_second.help.as_deref(),
        Some("return a `int` value")
    );

    // Without a declared type the original ambiguous wording is unchanged,
    // including its operand order, and carries no `help` (D-152 leaves
    // genuinely ambiguous conflicts unpopulated).
    let ambiguous = unify_terms(
        Ok(Ty::Int),
        inferred,
        &mut parents,
        &mut concrete,
        "T0022",
        "private helper return type",
    )
    .unwrap_err();
    assert_eq!(
        ambiguous.message,
        "private helper return type: conflicting inferred types `str` and `int`"
    );
    assert_eq!(ambiguous.help, None);
}
