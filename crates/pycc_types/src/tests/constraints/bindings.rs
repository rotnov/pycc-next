//! Constraint collection over local bindings and annotations.

use super::*;

#[test]
fn constraint_collection_rejects_bound_and_unbound_local_call_targets() {
    for (body, expected_message) in [
        (
            vec![
                HirStmt::Assign {
                    target: "helper".to_string(),
                    value: HirExpr::IntLiteral(1),
                },
                HirStmt::ExprStmt(HirExpr::Call {
                    callee: "helper".to_string(),
                    args: vec![],
                }),
            ],
            "name `helper` is bound to a non-callable value",
        ),
        (
            vec![
                HirStmt::ExprStmt(HirExpr::Call {
                    callee: "helper".to_string(),
                    args: vec![],
                }),
                HirStmt::Assign {
                    target: "helper".to_string(),
                    value: HirExpr::IntLiteral(1),
                },
            ],
            "local name `helper` is not bound before this use",
        ),
    ] {
        let hir = HirModule {
            seeded_builtin_exception_classes: false,
            items: vec![HirItem::Function {
                name: "_caller".to_string(),
                params: vec![],
                return_ty: Ty::Infer,
                body,
            }],
            type_aliases: Vec::new(),
            imports: Vec::new(),
            class_defs: Vec::new(),
        };

        assert_eq!(check(&hir).unwrap_err().message, expected_message);
    }
}
#[test]
fn constraint_collection_skips_already_concrete_call_arguments() {
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
                name: "_caller".to_string(),
                params: vec![],
                return_ty: Ty::Infer,
                body: vec![HirStmt::ExprStmt(HirExpr::Call {
                    callee: "takes_int".to_string(),
                    args: vec![HirExpr::IntLiteral(1)],
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
fn constraint_collection_reuses_a_top_level_for_binding() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "item".to_string(),
                value: HirExpr::IntLiteral(0),
            }),
            HirItem::TopLevelStmt(HirStmt::ForRange {
                var: "item".to_string(),
                start: HirExpr::IntLiteral(0),
                stop: HirExpr::IntLiteral(3),
                step: HirExpr::IntLiteral(1),
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
#[test]
fn constraint_collection_rejects_a_non_integer_top_level_for_binding() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "item".to_string(),
                value: HirExpr::StringLiteral("not an integer".to_string()),
            }),
            HirItem::TopLevelStmt(HirStmt::ForRange {
                var: "item".to_string(),
                start: HirExpr::IntLiteral(0),
                stop: HirExpr::IntLiteral(3),
                step: HirExpr::IntLiteral(1),
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

    assert_eq!(check(&hir).unwrap_err().code, "T0023");
}
#[test]
fn collect_block_constraints_binds_the_annotation_and_records_the_initializer_default() {
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment::empty(&[]);
    let body = vec![HirStmt::AnnAssign {
        is_final: false,
        target: "y".to_string(),
        annotation: Ty::Int,
        value: Some(HirExpr::IntLiteral(5)),
    }];

    collect_block_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut constraints,
        &mut env,
        &body,
        None,
    )
    .unwrap();

    assert_eq!(env.bindings.get("y"), Some(&Ok(Ty::Int)));
    assert_eq!(
        constraints.annotation_defaults,
        vec![AnnotationDefaultConstraint {
            initializer: Ok(Ty::Int),
            annotation: Ty::Int,
        }]
    );
}
#[test]
fn collect_block_constraints_preserves_an_existing_annotated_target_binding() {
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment {
        bindings: HashMap::from([("y".to_string(), Ok(Ty::Str))]),
        ..ConstraintEnvironment::empty(&["y"])
    };
    let body = vec![HirStmt::AnnAssign {
        is_final: false,
        target: "y".to_string(),
        annotation: Ty::Str,
        value: Some(HirExpr::StringLiteral("again".to_string())),
    }];

    collect_block_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut constraints,
        &mut env,
        &body,
        None,
    )
    .unwrap();

    assert_eq!(env.bindings.get("y"), Some(&Ok(Ty::Str)));
}
#[test]
fn collect_block_constraints_binds_the_annotation_when_the_initializer_has_no_term() {
    // `unresolved_global` is neither already bound nor a declared local of
    // this scope, so `collect_expr_constraints` returns `Ok(None)` for it
    // (the same "punt, nothing to unify yet" case a plain `Assign` to an
    // unresolved global would hit). The new `AnnAssign` arm cannot record
    // an initializer default without a term, but the annotation still
    // gives `y` a concrete solver binding.
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment::empty(&[]);
    let body = vec![HirStmt::AnnAssign {
        is_final: false,
        target: "y".to_string(),
        annotation: Ty::Int,
        value: Some(HirExpr::Name("unresolved_global".to_string())),
    }];

    collect_block_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut constraints,
        &mut env,
        &body,
        None,
    )
    .unwrap();

    assert_eq!(env.bindings.get("y"), Some(&Ok(Ty::Int)));
    assert!(constraints.annotation_defaults.is_empty());
}
#[test]
fn collect_block_constraints_ignores_a_value_less_annotated_assignment() {
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment::empty(&["y"]);
    let body = vec![HirStmt::AnnAssign {
        is_final: false,
        target: "y".to_string(),
        annotation: Ty::Int,
        value: None,
    }];

    collect_block_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut constraints,
        &mut env,
        &body,
        None,
    )
    .unwrap();

    assert!(!env.bindings.contains_key("y"));
    assert!(constraints.annotation_defaults.is_empty());
}
#[test]
fn collect_block_constraints_propagates_an_error_from_the_initializer_expression() {
    // `z` is declared local to this scope but never bound, so
    // `collect_expr_constraints` returns `Err(unbound_local)` for it --
    // the `?` inside the new arm must propagate that error rather than
    // being reachable only via the `Some`/`None` term paths above.
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment::empty(&["z"]);
    let body = vec![HirStmt::AnnAssign {
        is_final: false,
        target: "y".to_string(),
        annotation: Ty::Int,
        value: Some(HirExpr::Name("z".to_string())),
    }];

    let err = collect_block_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut constraints,
        &mut env,
        &body,
        None,
    )
    .unwrap_err();

    assert_eq!(err.code, "T0021");
}
