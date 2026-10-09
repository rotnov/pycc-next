//! Constraint-collection unit tests for the type-checking crate root.
//!
//! Extracted verbatim from `tests.rs` under AGENTS.md's decomposability rule
//! (part of #695, which tracks decomposing that oversized file). These are the
//! tests that exercise the constraint-collection pass in the production sibling
//! `crate::constraints`, so the split mirrors the source layout. As a child
//! module this still sees the parent's private items directly through
//! `use super::*`, so nothing needed widened visibility; only the tests'
//! location changed.

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
#[test]
fn constraint_collection_carries_none_literal_as_ty_none() {
    // D-197 (#763, Part 1 of #747): the constraint solver's own
    // `collect_expr_constraints` mirrors `infer_expr_in`'s identical
    // `NoneLiteral` arm -- both return `Ty::None` directly, with no
    // unification variable and no interaction with the generic-function
    // solver's own machinery.
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut binops = DeferredConstraints::default();
    let env = ConstraintEnvironment::empty(&[]);
    let expr = HirExpr::NoneLiteral;

    let term = collect_expr_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut binops,
        &env,
        &expr,
    )
    .unwrap();

    assert_eq!(term, Some(Ok(Ty::None)));
}
#[test]
fn constraint_collection_carries_a_homogeneous_scalar_list_literal_as_an_element_type_carrier() {
    // D-146 (#239): a homogeneous scalar-element list literal now produces
    // `Some(Ok(Ty::List(...)))` as a destructured element-type carrier --
    // never unified, only destructured by the `Subscript`/`ListPop` arms
    // to extract the scalar element type for scalar return-type inference.
    // Exact `Ty` equality (not `merge_inferred_types`) determines
    // homogeneity, matching `infer_expr_in`'s own rule.
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut binops = DeferredConstraints::default();
    let env = ConstraintEnvironment::empty(&[]);
    let expr = HirExpr::ListLiteral(vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)]);

    let term = collect_expr_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut binops,
        &env,
        &expr,
    )
    .unwrap();

    assert_eq!(term, Some(Ok(Ty::List(Box::new(Ty::Int)))));
}
#[test]
fn constraint_collection_propagates_an_error_from_a_list_literal_element() {
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut binops = DeferredConstraints::default();
    let env = ConstraintEnvironment::empty(&["missing"]);
    let expr = HirExpr::ListLiteral(vec![HirExpr::Name("missing".to_string())]);

    let err = collect_expr_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut binops,
        &env,
        &expr,
    )
    .unwrap_err();

    assert_eq!(err.code, "T0021");
}
#[test]
fn constraint_collection_treats_a_subscript_as_unconstrained_but_recurses_into_base_and_index() {
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut binops = DeferredConstraints::default();
    let env = ConstraintEnvironment::empty(&[]);
    let expr = HirExpr::Subscript {
        base: Box::new(HirExpr::IntLiteral(1)),
        index: Box::new(HirExpr::IntLiteral(0)),
    };

    let term = collect_expr_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut binops,
        &env,
        &expr,
    )
    .unwrap();

    assert!(term.is_none());
}
#[test]
fn constraint_collection_propagates_an_error_from_a_subscript_base() {
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut binops = DeferredConstraints::default();
    let env = ConstraintEnvironment::empty(&["missing"]);
    let expr = HirExpr::Subscript {
        base: Box::new(HirExpr::Name("missing".to_string())),
        index: Box::new(HirExpr::IntLiteral(0)),
    };

    let err = collect_expr_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut binops,
        &env,
        &expr,
    )
    .unwrap_err();

    assert_eq!(err.code, "T0021");
}
#[test]
fn constraint_collection_propagates_an_error_from_a_subscript_index() {
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut binops = DeferredConstraints::default();
    let env = ConstraintEnvironment::empty(&["missing"]);
    let expr = HirExpr::Subscript {
        base: Box::new(HirExpr::IntLiteral(1)),
        index: Box::new(HirExpr::Name("missing".to_string())),
    };

    let err = collect_expr_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut binops,
        &env,
        &expr,
    )
    .unwrap_err();

    assert_eq!(err.code, "T0021");
}
#[test]
fn constraint_collection_carries_a_homogeneous_float_list_literal_as_an_element_type_carrier() {
    // D-146 (#239): a homogeneous `float`-element list literal produces
    // `Some(Ok(Ty::List(Box::new(Ty::Float))))`, proving the carrier is
    // generic over any private-solver scalar, not `Ty::Int`-specific.
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut binops = DeferredConstraints::default();
    let env = ConstraintEnvironment::empty(&[]);
    let expr = HirExpr::ListLiteral(vec![HirExpr::FloatLiteral(1.0), HirExpr::FloatLiteral(2.0)]);

    let term = collect_expr_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut binops,
        &env,
        &expr,
    )
    .unwrap();

    assert_eq!(term, Some(Ok(Ty::List(Box::new(Ty::Float)))));
}
#[test]
fn constraint_collection_carries_a_single_element_scalar_list_literal() {
    // D-146 (#239): a single-element list is trivially homogeneous -- the
    // carrier is produced for arity 1 just as for arity 2+.
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut binops = DeferredConstraints::default();
    let env = ConstraintEnvironment::empty(&[]);
    let expr = HirExpr::ListLiteral(vec![HirExpr::IntLiteral(1)]);

    let term = collect_expr_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut binops,
        &env,
        &expr,
    )
    .unwrap();

    assert_eq!(term, Some(Ok(Ty::List(Box::new(Ty::Int)))));
}
#[test]
fn constraint_collection_does_not_carry_a_heterogeneous_list_literal() {
    // D-146 (#239): a heterogeneous `int`/`float` list keeps the
    // historical `Ok(None)` behavior -- exact `Ty` equality (not
    // `merge_inferred_types`) determines homogeneity, so `int` and
    // `float` are not merged even though `merge_inferred_types` would
    // reject them anyway.
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut binops = DeferredConstraints::default();
    let env = ConstraintEnvironment::empty(&[]);
    let expr = HirExpr::ListLiteral(vec![HirExpr::IntLiteral(1), HirExpr::FloatLiteral(2.0)]);

    let term = collect_expr_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut binops,
        &env,
        &expr,
    )
    .unwrap();

    assert!(term.is_none());
}
#[test]
fn constraint_collection_does_not_carry_a_bool_int_heterogeneous_list_literal() {
    // D-146 (#239): a heterogeneous `int`/`bool` list keeps the
    // historical `Ok(None)` behavior -- exact `Ty` equality (not
    // `merge_inferred_types`) determines homogeneity, so `bool` and
    // `int` are not merged even though `merge_inferred_types` would
    // widen `bool` to `int`.  This mirrors
    // `infer_expr_in`'s own list homogeneity rule (D-105).
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut binops = DeferredConstraints::default();
    let env = ConstraintEnvironment::empty(&[]);
    let expr = HirExpr::ListLiteral(vec![HirExpr::IntLiteral(1), HirExpr::BoolLiteral(true)]);

    let term = collect_expr_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut binops,
        &env,
        &expr,
    )
    .unwrap();

    assert!(term.is_none());
}
#[test]
fn constraint_collection_does_not_carry_an_empty_list_literal() {
    // D-146 (#239): an empty list has no element type to carry -- keeps
    // the historical `Ok(None)` behavior.
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut binops = DeferredConstraints::default();
    let env = ConstraintEnvironment::empty(&[]);
    let expr = HirExpr::ListLiteral(vec![]);

    let term = collect_expr_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut binops,
        &env,
        &expr,
    )
    .unwrap();

    assert!(term.is_none());
}
#[test]
fn constraint_collection_does_not_carry_a_list_literal_with_a_non_scalar_element() {
    // D-146 (#239): a nested-container element (`list[list[int]]`) is
    // rejected by the `is_private_solver_scalar` gate -- the carrier is
    // scalar-element-only to prevent nested-container carriers this
    // solver has no representation for.
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut binops = DeferredConstraints::default();
    let env = ConstraintEnvironment::empty(&[]);
    let expr = HirExpr::ListLiteral(vec![HirExpr::ListLiteral(vec![HirExpr::IntLiteral(1)])]);

    let term = collect_expr_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut binops,
        &env,
        &expr,
    )
    .unwrap();

    assert!(term.is_none());
}
#[test]
fn constraint_collection_does_not_carry_a_list_literal_when_an_element_has_no_term() {
    // D-146 (#239): when an element produces `None` (here an unbound
    // module-global name, not a local so no `T0021`), the list keeps the
    // historical `Ok(None)` behavior -- the carrier requires every
    // element to produce `Some(Ok(ty))`.
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut binops = DeferredConstraints::default();
    let env = ConstraintEnvironment::empty(&[]);
    let expr = HirExpr::ListLiteral(vec![
        HirExpr::IntLiteral(1),
        HirExpr::Name("unbound_global".to_string()),
    ]);

    let term = collect_expr_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut binops,
        &env,
        &expr,
    )
    .unwrap();

    assert!(term.is_none());
}
#[test]
fn constraint_collection_subscript_on_a_list_literal_base_extracts_the_element_type() {
    // D-146 (#239): `xs[0]` where `xs` is a `Ty::List`-bound name
    // extracts the scalar element type. This is the core reproduction
    // for #239: before this fix the `Subscript` arm returned `Ok(None)`
    // regardless of the base's resolved type.
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut binops = DeferredConstraints::default();
    let env = ConstraintEnvironment {
        bindings: HashMap::from([("xs".to_string(), Ok(Ty::List(Box::new(Ty::Int))))]),
        ..ConstraintEnvironment::empty(&[])
    };
    let expr = HirExpr::Subscript {
        base: Box::new(HirExpr::Name("xs".to_string())),
        index: Box::new(HirExpr::IntLiteral(0)),
    };

    let term = collect_expr_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut binops,
        &env,
        &expr,
    )
    .unwrap();

    assert_eq!(term, Some(Ok(Ty::Int)));
}
#[test]
fn constraint_collection_subscript_on_a_non_list_bound_base_keeps_ok_none() {
    // D-146 (#239): a `Ty::Int`-bound name is not a `Ty::List` carrier,
    // so the `Subscript` arm keeps the historical `Ok(None)` behavior --
    // the real check pass (`infer_expr_in`) validates it later.
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut binops = DeferredConstraints::default();
    let env = ConstraintEnvironment {
        bindings: HashMap::from([("x".to_string(), Ok(Ty::Int))]),
        ..ConstraintEnvironment::empty(&[])
    };
    let expr = HirExpr::Subscript {
        base: Box::new(HirExpr::Name("x".to_string())),
        index: Box::new(HirExpr::IntLiteral(0)),
    };

    let term = collect_expr_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut binops,
        &env,
        &expr,
    )
    .unwrap();

    assert!(term.is_none());
}
#[test]
fn constraint_collection_subscript_on_an_unresolved_list_base_keeps_ok_none() {
    // D-146 (#239): a genuinely unresolved inference variable (a fresh
    // term, not yet unified) is not a `Ty::List` carrier, so the
    // `Subscript` arm keeps the historical `Ok(None)` behavior -- the
    // real check pass validates it once its type is actually known.
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut binops = DeferredConstraints::default();
    let unresolved = fresh_term(&mut parents, &mut concrete);
    let env = ConstraintEnvironment {
        bindings: HashMap::from([("xs".to_string(), unresolved)]),
        ..ConstraintEnvironment::empty(&[])
    };
    let expr = HirExpr::Subscript {
        base: Box::new(HirExpr::Name("xs".to_string())),
        index: Box::new(HirExpr::IntLiteral(0)),
    };

    let term = collect_expr_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut binops,
        &env,
        &expr,
    )
    .unwrap();

    assert!(term.is_none());
}
#[test]
fn constraint_collection_list_pop_on_a_list_typed_bound_name_extracts_the_element_type() {
    // D-146 (#239): `xs.pop()` where `xs` is a `Ty::List`-bound name
    // extracts the scalar element type -- the carrier is destructured,
    // never unified.
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut binops = DeferredConstraints::default();
    let env = ConstraintEnvironment {
        bindings: HashMap::from([("xs".to_string(), Ok(Ty::List(Box::new(Ty::Str))))]),
        ..ConstraintEnvironment::empty(&[])
    };
    let expr = HirExpr::ListPop {
        list: pycc_hir::ContainerReceiver::Name("xs".to_string()),
    };

    let term = collect_expr_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut binops,
        &env,
        &expr,
    )
    .unwrap();

    assert_eq!(term, Some(Ok(Ty::Str)));
}
#[test]
fn constraint_collection_list_pop_on_an_unbound_name_keeps_ok_none() {
    // D-146 (#239): `xs.pop()` where `xs` is not in `env.bindings` keeps
    // the historical `Ok(None)` behavior -- the real check pass
    // (`infer_expr_in`) validates it later.
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut binops = DeferredConstraints::default();
    let env = ConstraintEnvironment::empty(&[]);
    let expr = HirExpr::ListPop {
        list: pycc_hir::ContainerReceiver::Name("xs".to_string()),
    };

    let term = collect_expr_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut binops,
        &env,
        &expr,
    )
    .unwrap();

    assert!(term.is_none());
}
#[test]
fn constraint_collection_list_pop_on_a_non_list_bound_name_keeps_ok_none() {
    // D-146 (#239): `xs.pop()` where `xs` is a `Ty::Int`-bound name is
    // not a `Ty::List` carrier, so the `ListPop` arm keeps the
    // historical `Ok(None)` behavior.
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut binops = DeferredConstraints::default();
    let env = ConstraintEnvironment {
        bindings: HashMap::from([("xs".to_string(), Ok(Ty::Int))]),
        ..ConstraintEnvironment::empty(&[])
    };
    let expr = HirExpr::ListPop {
        list: pycc_hir::ContainerReceiver::Name("xs".to_string()),
    };

    let term = collect_expr_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut binops,
        &env,
        &expr,
    )
    .unwrap();

    assert!(term.is_none());
}
#[test]
fn constraint_collection_list_append_after_a_list_binding_still_keeps_ok_none() {
    // D-146 (#239): `ListAppend` is unchanged -- it still returns
    // `Ok(None)` and recurses into `value` only, even when the list is
    // bound. The carrier is for element-type extraction, not for
    // append's own void (`Ty::None`) result.
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut binops = DeferredConstraints::default();
    let env = ConstraintEnvironment {
        bindings: HashMap::from([("xs".to_string(), Ok(Ty::List(Box::new(Ty::Int))))]),
        ..ConstraintEnvironment::empty(&[])
    };
    let expr = HirExpr::ListAppend {
        list: pycc_hir::ContainerReceiver::Name("xs".to_string()),
        value: Box::new(HirExpr::IntLiteral(2)),
    };

    let term = collect_expr_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut binops,
        &env,
        &expr,
    )
    .unwrap();

    assert!(term.is_none());
}
#[test]
fn constraint_collection_inline_subscript_on_a_list_literal_extracts_the_element_type() {
    // D-146 (#239): `[1][0]` -- an inline subscript on a list literal --
    // extracts the element type. The `ListLiteral` arm produces the
    // carrier, and the `Subscript` arm destructures it.
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut binops = DeferredConstraints::default();
    let env = ConstraintEnvironment::empty(&[]);
    let expr = HirExpr::Subscript {
        base: Box::new(HirExpr::ListLiteral(vec![HirExpr::IntLiteral(1)])),
        index: Box::new(HirExpr::IntLiteral(0)),
    };

    let term = collect_expr_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut binops,
        &env,
        &expr,
    )
    .unwrap();

    assert_eq!(term, Some(Ok(Ty::Int)));
}
#[test]
fn constraint_collection_treats_a_slice_as_unconstrained_but_recurses_into_base_and_bounds() {
    // PR-12 Task 7 (D-118): mirrors `Subscript`'s own solver arm in `lib.rs` --
    // structurally identical recursion, no term produced for the slice
    // itself.
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut binops = DeferredConstraints::default();
    let env = ConstraintEnvironment::empty(&[]);
    let expr = HirExpr::Slice {
        base: Box::new(HirExpr::IntLiteral(1)),
        start: Some(Box::new(HirExpr::IntLiteral(0))),
        stop: Some(Box::new(HirExpr::IntLiteral(2))),
        step: Some(Box::new(HirExpr::IntLiteral(1))),
    };

    let term = collect_expr_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut binops,
        &env,
        &expr,
    )
    .unwrap();

    assert!(term.is_none());
}
#[test]
fn constraint_collection_treats_a_slice_with_every_bound_omitted_as_unconstrained() {
    // Proves the `Option` loop's `None` branch is exercised too, not
    // just the `Some` branch above (`xs[:]`).
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut binops = DeferredConstraints::default();
    let env = ConstraintEnvironment::empty(&[]);
    let expr = HirExpr::Slice {
        base: Box::new(HirExpr::IntLiteral(1)),
        start: None,
        stop: None,
        step: None,
    };

    let term = collect_expr_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut binops,
        &env,
        &expr,
    )
    .unwrap();

    assert!(term.is_none());
}
#[test]
fn constraint_collection_propagates_an_error_from_a_slice_base() {
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut binops = DeferredConstraints::default();
    let env = ConstraintEnvironment::empty(&["missing"]);
    let expr = HirExpr::Slice {
        base: Box::new(HirExpr::Name("missing".to_string())),
        start: None,
        stop: None,
        step: None,
    };

    let err = collect_expr_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut binops,
        &env,
        &expr,
    )
    .unwrap_err();

    assert_eq!(err.code, "T0021");
}
#[test]
fn constraint_collection_propagates_an_error_from_a_slice_bound() {
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut binops = DeferredConstraints::default();
    let env = ConstraintEnvironment::empty(&["missing"]);
    let expr = HirExpr::Slice {
        base: Box::new(HirExpr::IntLiteral(1)),
        start: Some(Box::new(HirExpr::Name("missing".to_string()))),
        stop: None,
        step: None,
    };

    let err = collect_expr_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut binops,
        &env,
        &expr,
    )
    .unwrap_err();

    assert_eq!(err.code, "T0021");
}
#[test]
fn constraint_collection_treats_a_list_append_as_unconstrained_but_recurses_into_value() {
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut binops = DeferredConstraints::default();
    let env = ConstraintEnvironment::empty(&[]);
    let expr = HirExpr::ListAppend {
        list: pycc_hir::ContainerReceiver::Name("lst".to_string()),
        value: Box::new(HirExpr::IntLiteral(1)),
    };

    let term = collect_expr_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut binops,
        &env,
        &expr,
    )
    .unwrap();

    assert!(term.is_none());
}
#[test]
fn constraint_collection_propagates_an_error_from_a_list_append_value() {
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut binops = DeferredConstraints::default();
    let env = ConstraintEnvironment::empty(&["missing"]);
    let expr = HirExpr::ListAppend {
        list: pycc_hir::ContainerReceiver::Name("lst".to_string()),
        value: Box::new(HirExpr::Name("missing".to_string())),
    };

    let err = collect_expr_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut binops,
        &env,
        &expr,
    )
    .unwrap_err();

    assert_eq!(err.code, "T0021");
}

// -- PR-12 Task 10 (D-119): remaining container methods depth --------
// `collect_expr_constraints` coverage, mirroring `ListAppend`'s own
// direct-call test shape exactly.

#[test]
fn constraint_collection_treats_a_list_pop_as_unconstrained() {
    // `list` is a plain name, not a sub-expression -- unlike
    // `ListAppend`, there is no `value` to recurse into here.
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut binops = DeferredConstraints::default();
    let env = ConstraintEnvironment::empty(&[]);
    let expr = HirExpr::ListPop {
        list: pycc_hir::ContainerReceiver::Name("lst".to_string()),
    };

    let term = collect_expr_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut binops,
        &env,
        &expr,
    )
    .unwrap();

    assert!(term.is_none());
}
#[test]
fn constraint_collection_treats_a_dict_get_or_default_as_unconstrained_but_recurses_into_key_and_default()
 {
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut binops = DeferredConstraints::default();
    let env = ConstraintEnvironment::empty(&[]);
    let expr = HirExpr::DictGetOrDefault {
        dict: pycc_hir::ContainerReceiver::Name("d".to_string()),
        key: Box::new(HirExpr::StringLiteral("a".to_string())),
        default: Box::new(HirExpr::IntLiteral(0)),
    };

    let term = collect_expr_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut binops,
        &env,
        &expr,
    )
    .unwrap();

    assert!(term.is_none());
}
#[test]
fn constraint_collection_propagates_an_error_from_a_dict_get_or_default_key() {
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut binops = DeferredConstraints::default();
    let env = ConstraintEnvironment::empty(&["missing"]);
    let expr = HirExpr::DictGetOrDefault {
        dict: pycc_hir::ContainerReceiver::Name("d".to_string()),
        key: Box::new(HirExpr::Name("missing".to_string())),
        default: Box::new(HirExpr::IntLiteral(0)),
    };

    let err = collect_expr_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut binops,
        &env,
        &expr,
    )
    .unwrap_err();

    assert_eq!(err.code, "T0021");
}
#[test]
fn constraint_collection_propagates_an_error_from_a_dict_get_or_default_default() {
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut binops = DeferredConstraints::default();
    let env = ConstraintEnvironment::empty(&["missing"]);
    let expr = HirExpr::DictGetOrDefault {
        dict: pycc_hir::ContainerReceiver::Name("d".to_string()),
        key: Box::new(HirExpr::StringLiteral("a".to_string())),
        default: Box::new(HirExpr::Name("missing".to_string())),
    };

    let err = collect_expr_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut binops,
        &env,
        &expr,
    )
    .unwrap_err();

    assert_eq!(err.code, "T0021");
}
#[test]
fn constraint_collection_treats_a_set_add_as_unconstrained_but_recurses_into_value() {
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut binops = DeferredConstraints::default();
    let env = ConstraintEnvironment::empty(&[]);
    let expr = HirExpr::SetAdd {
        set: "s".to_string(),
        value: Box::new(HirExpr::IntLiteral(1)),
    };

    let term = collect_expr_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut binops,
        &env,
        &expr,
    )
    .unwrap();

    assert!(term.is_none());
}
#[test]
fn constraint_collection_propagates_an_error_from_a_set_add_value() {
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut binops = DeferredConstraints::default();
    let env = ConstraintEnvironment::empty(&["missing"]);
    let expr = HirExpr::SetAdd {
        set: "s".to_string(),
        value: Box::new(HirExpr::Name("missing".to_string())),
    };

    let err = collect_expr_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut binops,
        &env,
        &expr,
    )
    .unwrap_err();

    assert_eq!(err.code, "T0021");
}
#[test]
fn constraint_collection_len_call_returns_int_for_a_concretely_bound_list() {
    // `lst`'s binding is a directly concrete `TypeTerm` (`Ok(Ty::List(_))`)
    // rather than one produced by `ListLiteral` (which always returns
    // `Ok(None)` in this solver, per its own comment in `lib.rs`) -- this is
    // the only way to get a concrete `Ty::List` term to validate against
    // at constraint-collection time.
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut binops = DeferredConstraints::default();
    let env = ConstraintEnvironment {
        bindings: HashMap::from([("lst".to_string(), Ok(Ty::List(Box::new(Ty::Int))))]),
        ..ConstraintEnvironment::empty(&[])
    };
    let expr = HirExpr::Call {
        callee: "len".to_string(),
        args: vec![HirExpr::Name("lst".to_string())],
    };

    let term = collect_expr_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut binops,
        &env,
        &expr,
    )
    .unwrap();

    assert_eq!(term, Some(Ok(Ty::Int)));
}
#[test]
fn constraint_collection_len_call_defers_an_unresolved_argument_to_the_real_check_pass() {
    // `lst`'s binding is a genuinely unresolved inference variable (a
    // fresh term, not yet unified with anything) -- this solver can't
    // tell yet whether it's a list, so it must not reject it here; the
    // real check pass (`infer_expr_in`) validates it once its type is
    // actually known.
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut binops = DeferredConstraints::default();
    let unresolved = fresh_term(&mut parents, &mut concrete);
    let env = ConstraintEnvironment {
        bindings: HashMap::from([("lst".to_string(), unresolved)]),
        ..ConstraintEnvironment::empty(&[])
    };
    let expr = HirExpr::Call {
        callee: "len".to_string(),
        args: vec![HirExpr::Name("lst".to_string())],
    };

    let term = collect_expr_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut binops,
        &env,
        &expr,
    )
    .unwrap();

    assert_eq!(term, Some(Ok(Ty::Int)));
}
#[test]
fn constraint_collection_len_call_rejects_the_wrong_arity() {
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut binops = DeferredConstraints::default();
    let env = ConstraintEnvironment::empty(&[]);
    let expr = HirExpr::Call {
        callee: "len".to_string(),
        args: vec![],
    };

    let err = collect_expr_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut binops,
        &env,
        &expr,
    )
    .unwrap_err();

    assert_eq!(err.code, "T0033");
    assert_eq!(err.message, "`len` expects exactly 1 argument, got 0");
}
#[test]
fn constraint_collection_len_call_rejects_a_concretely_known_non_list_argument() {
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut binops = DeferredConstraints::default();
    let env = ConstraintEnvironment::empty(&[]);
    let expr = HirExpr::Call {
        callee: "len".to_string(),
        args: vec![HirExpr::IntLiteral(5)],
    };

    let err = collect_expr_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut binops,
        &env,
        &expr,
    )
    .unwrap_err();

    assert_eq!(err.code, "T0033");
    assert_eq!(
        err.message,
        "`len` expects a `list[T]`, `dict[K, V]`, `set[T]`, or `frozenset[T]` argument, got `int`"
    );
}
#[test]
fn constraint_collection_float_call_returns_float_regardless_of_argument_resolution() {
    // Mirrors `constraint_collection_len_call_returns_int_for_a_
    // concretely_bound_list`: a directly concrete `TypeTerm`
    // (`Ok(Ty::Int)`) validates cleanly and produces `Ty::Float`.
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut binops = DeferredConstraints::default();
    let env = ConstraintEnvironment {
        bindings: HashMap::from([("x".to_string(), Ok(Ty::Int))]),
        ..ConstraintEnvironment::empty(&[])
    };
    let expr = HirExpr::Call {
        callee: "float".to_string(),
        args: vec![HirExpr::Name("x".to_string())],
    };

    let term = collect_expr_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut binops,
        &env,
        &expr,
    )
    .unwrap();

    assert_eq!(term, Some(Ok(Ty::Float)));
}
#[test]
fn constraint_collection_float_call_defers_an_unresolved_argument_to_the_real_check_pass() {
    // Mirrors `constraint_collection_len_call_defers_an_unresolved_
    // argument_to_the_real_check_pass`: a genuinely unresolved
    // inference variable is left to `infer_expr_in`, but the call's
    // own return type (`Ty::Float`) is still produced unconditionally.
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut binops = DeferredConstraints::default();
    let unresolved = fresh_term(&mut parents, &mut concrete);
    let env = ConstraintEnvironment {
        bindings: HashMap::from([("x".to_string(), unresolved)]),
        ..ConstraintEnvironment::empty(&[])
    };
    let expr = HirExpr::Call {
        callee: "float".to_string(),
        args: vec![HirExpr::Name("x".to_string())],
    };

    let term = collect_expr_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut binops,
        &env,
        &expr,
    )
    .unwrap();

    assert_eq!(term, Some(Ok(Ty::Float)));
}
#[test]
fn constraint_collection_float_call_rejects_the_wrong_arity() {
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut binops = DeferredConstraints::default();
    let env = ConstraintEnvironment::empty(&[]);
    let expr = HirExpr::Call {
        callee: "float".to_string(),
        args: vec![],
    };

    let err = collect_expr_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut binops,
        &env,
        &expr,
    )
    .unwrap_err();

    assert_eq!(err.code, "T0021");
    assert_eq!(err.message, "`float` expects exactly 1 argument, got 0");
}
#[test]
fn constraint_collection_float_call_rejects_a_concretely_known_non_numeric_argument() {
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut binops = DeferredConstraints::default();
    let env = ConstraintEnvironment::empty(&[]);
    let expr = HirExpr::Call {
        callee: "float".to_string(),
        args: vec![HirExpr::StringLiteral("hello".to_string())],
    };

    let err = collect_expr_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut binops,
        &env,
        &expr,
    )
    .unwrap_err();

    assert_eq!(err.code, "T0021");
    assert_eq!(
        err.message,
        "`float` expects an `int`, `float`, or `bool` argument, got `str`"
    );
}
#[test]
fn constraint_collection_honors_a_user_defined_float_signature_over_the_builtin() {
    // Same post-merge review finding as `infer_expr_in`'s own
    // `a_user_defined_float_function_takes_priority_over_the_builtin`:
    // a registered `float` signature (e.g. one accepting `str`, which
    // the builtin itself would reject) must resolve through the normal
    // signature lookup, not the hand-recognized builtin arm.
    let signatures = HashMap::from([(
        "float".to_string(),
        (vec!["x".to_string()], vec![Ok(Ty::Str)], Ok(Ty::Str)),
    )]);
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut binops = DeferredConstraints::default();
    let env = ConstraintEnvironment::empty(&[]);
    let expr = HirExpr::Call {
        callee: "float".to_string(),
        args: vec![HirExpr::StringLiteral("hello".to_string())],
    };

    let term = collect_expr_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut binops,
        &env,
        &expr,
    )
    .unwrap();

    assert_eq!(term, Some(Ok(Ty::Str)));
}
#[test]
fn collect_block_constraints_gives_a_for_list_loop_variable_a_fresh_term_when_unbound() {
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment::empty(&["i"]);
    let body = vec![HirStmt::ForList {
        var: "i".to_string(),
        list: "lst".to_string(),
        body: vec![],
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

    assert!(env.bindings.contains_key("i"));
}
#[test]
fn collect_block_constraints_keeps_a_for_list_loop_variable_s_existing_term() {
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment {
        bindings: HashMap::from([("i".to_string(), Ok(Ty::Int))]),
        ..ConstraintEnvironment::empty(&["i"])
    };
    let body = vec![HirStmt::ForList {
        var: "i".to_string(),
        list: "lst".to_string(),
        body: vec![],
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

    assert_eq!(env.bindings.get("i"), Some(&Ok(Ty::Int)));
}
#[test]
fn collect_block_constraints_propagates_an_error_from_a_for_list_loop_body() {
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment::empty(&["i", "z"]);
    let body = vec![HirStmt::ForList {
        var: "i".to_string(),
        list: "lst".to_string(),
        body: vec![HirStmt::ExprStmt(HirExpr::Name("z".to_string()))],
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
#[test]
fn constraint_collection_leaves_top_level_return_to_validation() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Return(Some(HirExpr::IntLiteral(1)))),
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

    assert_eq!(check(&hir).unwrap_err().code, "T0024");
}
#[test]
fn constraint_collection_classifies_value_error_as_c0001() {
    // The private-helper inference path (`collect_expr_constraints`)
    // must apply the same C0001 classification for a known callable
    // builtin, rather than deferring with `Ok(None)` -- a private
    // helper calling `ValueError` gets C0001 directly.
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut binops = DeferredConstraints::default();
    let env = ConstraintEnvironment::empty(&[]);
    let expr = HirExpr::Call {
        callee: "ValueError".to_string(),
        args: vec![HirExpr::StringLiteral("x".to_string())],
    };
    let err = collect_expr_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut binops,
        &env,
        &expr,
    )
    .unwrap_err();
    assert_eq!(err.code, "C0001");
    assert!(err.message.contains("ValueError"));
}
#[test]
fn constraint_collection_classifies_exception_as_c0001() {
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut binops = DeferredConstraints::default();
    let env = ConstraintEnvironment::empty(&[]);
    let expr = HirExpr::Call {
        callee: "Exception".to_string(),
        args: vec![HirExpr::StringLiteral("msg".to_string())],
    };
    let err = collect_expr_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut binops,
        &env,
        &expr,
    )
    .unwrap_err();
    assert_eq!(err.code, "C0001");
    assert!(err.message.contains("Exception"));
}
#[test]
fn constraint_collection_defers_unknown_callees_to_final_validation() {
    // A genuinely unknown callee still returns `Ok(None)` and defers to
    // final validation's T0021 -- the C0001 classification only applies
    // to known callable builtins.
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut binops = DeferredConstraints::default();
    let env = ConstraintEnvironment::empty(&[]);
    let expr = HirExpr::Call {
        callee: "totally_undefined".to_string(),
        args: vec![HirExpr::IntLiteral(1)],
    };
    let term = collect_expr_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut binops,
        &env,
        &expr,
    )
    .unwrap();
    assert_eq!(term, None);
}
#[test]
fn constraint_collection_honors_user_defined_value_error_over_c0001() {
    // A registered `ValueError` signature resolves through normal
    // signature lookup, not the C0001 builtin classification -- user
    // definitions take priority in the private-helper path too.
    let signatures = HashMap::from([(
        "ValueError".to_string(),
        (vec!["x".to_string()], vec![Ok(Ty::Str)], Ok(Ty::Int)),
    )]);
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut binops = DeferredConstraints::default();
    let env = ConstraintEnvironment::empty(&[]);
    let expr = HirExpr::Call {
        callee: "ValueError".to_string(),
        args: vec![HirExpr::StringLiteral("x".to_string())],
    };
    let term = collect_expr_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut binops,
        &env,
        &expr,
    )
    .unwrap();
    assert_eq!(term, Some(Ok(Ty::Int)));
}
#[test]
fn constraint_collection_len_call_returns_int_for_a_concretely_bound_dict() {
    // Mirrors `constraint_collection_len_call_returns_int_for_a_concretely_bound_list`
    // above, proving the solver's own relaxed `len()` arm (PR-11 Task 3)
    // also accepts a concretely-bound `Ty::Dict` term.
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut binops = DeferredConstraints::default();
    let env = ConstraintEnvironment {
        bindings: HashMap::from([("d".to_string(), Ok(Ty::Dict(Box::new((Ty::Str, Ty::Int)))))]),
        ..ConstraintEnvironment::empty(&[])
    };
    let expr = HirExpr::Call {
        callee: "len".to_string(),
        args: vec![HirExpr::Name("d".to_string())],
    };

    let term = collect_expr_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut binops,
        &env,
        &expr,
    )
    .unwrap();

    assert_eq!(term, Some(Ok(Ty::Int)));
}
#[test]
fn constraint_collection_treats_a_dict_literal_as_unconstrained_but_recurses_into_pairs() {
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut binops = DeferredConstraints::default();
    let env = ConstraintEnvironment::empty(&[]);
    let expr = HirExpr::DictLiteral(vec![(
        HirExpr::StringLiteral("a".to_string()),
        HirExpr::IntLiteral(1),
    )]);

    let term = collect_expr_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut binops,
        &env,
        &expr,
    )
    .unwrap();

    assert!(term.is_none());
}
#[test]
fn constraint_collection_propagates_an_error_from_a_dict_literal_key() {
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut binops = DeferredConstraints::default();
    let env = ConstraintEnvironment::empty(&["missing"]);
    let expr = HirExpr::DictLiteral(vec![(
        HirExpr::Name("missing".to_string()),
        HirExpr::IntLiteral(1),
    )]);

    let err = collect_expr_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut binops,
        &env,
        &expr,
    )
    .unwrap_err();

    assert_eq!(err.code, "T0021");
}
#[test]
fn constraint_collection_propagates_an_error_from_a_dict_literal_value() {
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut binops = DeferredConstraints::default();
    let env = ConstraintEnvironment::empty(&["missing"]);
    let expr = HirExpr::DictLiteral(vec![(
        HirExpr::StringLiteral("a".to_string()),
        HirExpr::Name("missing".to_string()),
    )]);

    let err = collect_expr_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut binops,
        &env,
        &expr,
    )
    .unwrap_err();

    assert_eq!(err.code, "T0021");
}
#[test]
fn collect_block_constraints_recurses_into_a_dict_set_s_key_and_value() {
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment::empty(&[]);
    let body = vec![HirStmt::DictSet {
        dict: "x".to_string(),
        key: HirExpr::StringLiteral("a".to_string()),
        value: HirExpr::IntLiteral(1),
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
}
#[test]
fn collect_block_constraints_propagates_an_error_from_a_dict_set_key() {
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment::empty(&["missing"]);
    let body = vec![HirStmt::DictSet {
        dict: "x".to_string(),
        key: HirExpr::Name("missing".to_string()),
        value: HirExpr::IntLiteral(1),
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
#[test]
fn collect_block_constraints_propagates_an_error_from_a_dict_set_value() {
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment::empty(&["missing"]);
    let body = vec![HirStmt::DictSet {
        dict: "x".to_string(),
        key: HirExpr::StringLiteral("a".to_string()),
        value: HirExpr::Name("missing".to_string()),
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
/// Part 3 of #382 (#542, PEP 654): `collect_block_constraints`'s `TryStar`
/// arm collects constraints from all four of its blocks (body, handler
/// body, `else`, `finally`) and, when a handler names its binding, seeds it
/// as `ExceptionGroup` -- exactly like `check_try_star_stmt` does at
/// type-checking time. Nothing else in this file calls
/// `collect_block_constraints` (directly or via `infer_function_signatures_
/// with_solver`) with a `TryStar` body, so without this test the entire arm
/// -- including the named-handler-binding branch -- goes unexercised.
#[test]
fn collect_block_constraints_recurses_into_every_try_star_block_and_binds_a_named_handler() {
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment::empty(&[]);
    let body = vec![HirStmt::TryStar {
        body: vec![HirStmt::ExprStmt(HirExpr::IntLiteral(1))],
        handlers: vec![pycc_hir::HirExceptHandler {
            exc_type: Some(vec!["ValueError".to_string()]),
            name: Some("e".to_string()),
            body: vec![HirStmt::ExprStmt(HirExpr::IntLiteral(2))],
        }],
        orelse: vec![HirStmt::ExprStmt(HirExpr::IntLiteral(3))],
        finalbody: vec![HirStmt::ExprStmt(HirExpr::IntLiteral(4))],
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
}
/// A `DictSet` whose target name is declared local but never bound
/// (`local_names` claims it but `bindings` never seeds it) is the same
/// unresolved-local-reference shape the two `DictSet`-focused tests above
/// use to reach `T0021`. Placing it in `TryStar`'s own `body` block reaches
/// the first of that arm's four `collect_block_constraints(...)?` call
/// sites and proves the `?` actually propagates the body block's error
/// instead of only ever seeing `Ok` there.
#[test]
fn collect_block_constraints_propagates_an_error_from_a_try_star_body_block() {
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment::empty(&["missing"]);
    let body = vec![HirStmt::TryStar {
        body: vec![HirStmt::DictSet {
            dict: "x".to_string(),
            key: HirExpr::Name("missing".to_string()),
            value: HirExpr::IntLiteral(1),
        }],
        handlers: vec![],
        orelse: vec![],
        finalbody: vec![],
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
/// Same unresolved-local-reference shape as above, but placed in a
/// `TryStar` handler's body instead -- reaches the arm's second
/// `collect_block_constraints(...)?` call site (the per-handler loop).
#[test]
fn collect_block_constraints_propagates_an_error_from_a_try_star_handler_block() {
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment::empty(&["missing"]);
    let body = vec![HirStmt::TryStar {
        body: vec![],
        handlers: vec![pycc_hir::HirExceptHandler {
            exc_type: Some(vec!["ValueError".to_string()]),
            name: None,
            body: vec![HirStmt::DictSet {
                dict: "x".to_string(),
                key: HirExpr::Name("missing".to_string()),
                value: HirExpr::IntLiteral(1),
            }],
        }],
        orelse: vec![],
        finalbody: vec![],
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
/// Same shape again, placed in `TryStar`'s `else` block -- reaches the
/// arm's third `collect_block_constraints(...)?` call site.
#[test]
fn collect_block_constraints_propagates_an_error_from_a_try_star_else_block() {
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment::empty(&["missing"]);
    let body = vec![HirStmt::TryStar {
        body: vec![],
        handlers: vec![],
        orelse: vec![HirStmt::DictSet {
            dict: "x".to_string(),
            key: HirExpr::Name("missing".to_string()),
            value: HirExpr::IntLiteral(1),
        }],
        finalbody: vec![],
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
/// Issue #771 join-site follow-up, extended to `TryStar`'s own
/// `pre_existing` snapshot in the same commit that adds this test: `y` is
/// already opaquely bound *before* the `try*` (e.g. from `y = d.get("a",
/// 0)` two lines above), then reassigned to a real, solver-representable
/// term inside the `try*` body only. Mirrors
/// `solver_if_reassigns_pre_existing_opaque_binding_in_one_branch_only`'s
/// scenario but through the `TryStar` arm's `join_loop_body_solver` call
/// instead of `If`'s `join_if_branches_solver` -- before this fix,
/// `TryStar`'s `pre_existing` was computed as `env.bindings.keys()` alone
/// (unlike every sibling join site, including the `Try` arm immediately
/// above it, which all `.chain(env.opaque_bindings.iter())`), so `y` looked
/// "newly introduced" to the join helper and was wrongly folded into
/// `env.maybe_bindings` even though it was already definitely (if opaquely)
/// bound beforehand.
#[test]
fn collect_block_constraints_try_star_body_reassigns_pre_existing_opaque_binding() {
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment {
        opaque_bindings: HashSet::from(["y".to_string()]),
        ..ConstraintEnvironment::empty(&["y"])
    };
    let body = vec![HirStmt::TryStar {
        body: vec![HirStmt::Assign {
            target: "y".to_string(),
            value: HirExpr::IntLiteral(1),
        }],
        handlers: vec![],
        orelse: vec![],
        finalbody: vec![],
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

    // `y` must never become newly *maybe* bound: it was already definitely
    // (if opaquely) bound before the `try*`, and the body reassigning it
    // does not retroactively make a pre-existing name conditional.
    assert!(!env.maybe_bindings.contains("y"));
}
/// Same shape again, placed in `TryStar`'s `finally` block -- reaches the
/// arm's fourth and last `collect_block_constraints(...)?` call site.
#[test]
fn collect_block_constraints_propagates_an_error_from_a_try_star_finally_block() {
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment::empty(&["missing"]);
    let body = vec![HirStmt::TryStar {
        body: vec![],
        handlers: vec![],
        orelse: vec![],
        finalbody: vec![HirStmt::DictSet {
            dict: "x".to_string(),
            key: HirExpr::Name("missing".to_string()),
            value: HirExpr::IntLiteral(1),
        }],
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
#[test]
fn collect_block_constraints_treats_a_list_comp_assign_target_as_a_no_op() {
    // PR-12 Task 3 (D-117), Step 5: mirrors the dict/set/tuple-literal
    // solver-gap precedent directly -- `collect_block_constraints`'s
    // `ListCompAssign`/`SetCompAssign`/`DictCompAssign` arm registers no
    // term for `target` at all (though it does, unlike a bare no-op,
    // still recurse into `elt`/`cond` -- see
    // `private_helper_parameter_is_inferred_through_a_comprehension_s_elt`
    // below for that half). An unrelated private helper with an
    // unresolved (`Ty::Infer`) signature elsewhere in the same module
    // must not let the solver's own leniency swallow a genuine T0034 --
    // it has to fall through to the real, comprehension-aware check pass
    // (mirrors
    // `a_bad_dict_literal_is_still_rejected_when_the_solver_path_runs_first`).
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::ListCompAssign {
                target: "xs".to_string(),
                var: "0comp_11_i".to_string(),
                iter: CompIter::Range {
                    start: HirExpr::IntLiteral(0),
                    stop: HirExpr::IntLiteral(3),
                    step: HirExpr::IntLiteral(1),
                },
                cond: None,
                elt: Box::new(HirExpr::StringLiteral("x".to_string())),
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
    assert_eq!(check(&hir).unwrap_err().code, "T0034");
}
#[test]
fn collect_block_constraints_unifies_a_range_comprehension_s_stop_forwarded_from_an_unannotated_parameter()
 {
    // Exercises `bind_comp_loop_var`'s `CompIter::Range` operand-check
    // branch (mirrors `ForRange`'s own analogous `start`/`stop`/`step`
    // checks): the comprehension's own `stop` operand is itself an
    // unresolved solver term (an unannotated parameter forwarded
    // directly as the range bound), which this arm must still visit and
    // unify against `Ty::Int` -- exactly like a plain `for i in
    // range(n):` loop's own `ForRange` arm already does.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_h".to_string(),
            params: vec![("n".to_string(), Ty::Infer)],
            return_ty: Ty::None,
            body: vec![
                HirStmt::ListCompAssign {
                    target: "y".to_string(),
                    var: "0comp_11_i".to_string(),
                    iter: CompIter::Range {
                        start: HirExpr::IntLiteral(0),
                        stop: HirExpr::Name("n".to_string()),
                        step: HirExpr::IntLiteral(1),
                    },
                    cond: None,
                    elt: Box::new(HirExpr::IntLiteral(1)),
                },
                HirStmt::Return(None),
            ],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    check(&hir).unwrap();
}
#[test]
fn collect_block_constraints_rejects_a_range_comprehension_whose_stop_was_already_resolved_to_an_incompatible_type()
 {
    // The failure half of the operand-check branch above: `n` is first
    // forwarded into a `str`-typed parameter (resolving its solver term
    // to `Ty::Str`), then reused as the comprehension's own `stop`
    // operand -- `bind_comp_loop_var`'s `unify_terms(term, Ok(Ty::Int),
    // ...)` call must propagate the resulting conflict rather than
    // silently accepting it.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "_sink_str".to_string(),
                params: vec![("value".to_string(), Ty::Str)],
                return_ty: Ty::None,
                body: vec![HirStmt::Return(None)],
            },
            HirItem::Function {
                name: "_h".to_string(),
                params: vec![("n".to_string(), Ty::Infer)],
                return_ty: Ty::None,
                body: vec![
                    HirStmt::ExprStmt(HirExpr::Call {
                        callee: "_sink_str".to_string(),
                        args: vec![HirExpr::Name("n".to_string())],
                    }),
                    HirStmt::ListCompAssign {
                        target: "y".to_string(),
                        var: "0comp_11_i".to_string(),
                        iter: CompIter::Range {
                            start: HirExpr::IntLiteral(0),
                            stop: HirExpr::Name("n".to_string()),
                            step: HirExpr::IntLiteral(1),
                        },
                        cond: None,
                        elt: Box::new(HirExpr::IntLiteral(1)),
                    },
                    HirStmt::Return(None),
                ],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert_eq!(check(&hir).unwrap_err().code, "T0021");
}
#[test]
fn collect_block_constraints_rejects_a_range_comprehension_whose_loop_variable_conflicts_with_an_existing_binding()
 {
    // Exercises `bind_comp_loop_var`'s `CompIter::Range` "existing
    // binding" branch's own *failure* path (mirrors `ForRange`'s own
    // `T0023` branch): the comprehension's loop variable happens, in
    // this hand-built test only, to share a name with the enclosing
    // function's own `str`-typed parameter (real lowering's
    // `synthesize_comp_var_name` never produces such a collision), so
    // `Range`'s `Ty::Int` fact genuinely conflicts with the existing
    // binding.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "_h".to_string(),
                params: vec![("0comp_11_i".to_string(), Ty::Str)],
                return_ty: Ty::None,
                body: vec![
                    HirStmt::ListCompAssign {
                        target: "y".to_string(),
                        var: "0comp_11_i".to_string(),
                        iter: CompIter::Range {
                            start: HirExpr::IntLiteral(0),
                            stop: HirExpr::IntLiteral(3),
                            step: HirExpr::IntLiteral(1),
                        },
                        cond: None,
                        elt: Box::new(HirExpr::IntLiteral(1)),
                    },
                    HirStmt::Return(None),
                ],
            },
            HirItem::Function {
                name: "_trigger".to_string(),
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
fn collect_block_constraints_gives_a_name_iterable_comprehension_s_loop_variable_a_fresh_term() {
    // Exercises `bind_comp_loop_var`'s `CompIter::Name` branch (mirrors
    // `ForList`'s own analogous branch): this solver doesn't track a
    // list-typed name's element type, so the loop variable just gets an
    // unconstrained fresh term here -- real element-type checking is
    // the subsequent `check_with_signatures_all` pass's job. Every other
    // solver-path comprehension test above uses `CompIter::Range`.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "_h".to_string(),
                params: vec![("xs".to_string(), Ty::List(Box::new(Ty::Int)))],
                return_ty: Ty::None,
                body: vec![
                    HirStmt::ListCompAssign {
                        target: "y".to_string(),
                        var: "0comp_11_i".to_string(),
                        iter: CompIter::Name("xs".to_string()),
                        cond: None,
                        elt: Box::new(HirExpr::Name("0comp_11_i".to_string())),
                    },
                    HirStmt::Return(None),
                ],
            },
            HirItem::Function {
                name: "_trigger".to_string(),
                params: vec![],
                return_ty: Ty::Infer,
                body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(1)))],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    check(&hir).unwrap();
}
#[test]
fn collect_block_constraints_visits_a_set_comp_assign_s_cond_and_elt() {
    // The combined `HirStmt::ListCompAssign | HirStmt::SetCompAssign`
    // solver arm is only exercised with `ListCompAssign` and a `None`
    // `cond` by the tests above -- this pins the `SetCompAssign` half of
    // that pattern and the `cond.is_some()` branch, both distinct
    // coverage regions from the `ListCompAssign`/`cond: None` case.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "_h".to_string(),
                params: vec![],
                return_ty: Ty::None,
                body: vec![
                    HirStmt::SetCompAssign {
                        target: "y".to_string(),
                        var: "0comp_11_i".to_string(),
                        iter: CompIter::Range {
                            start: HirExpr::IntLiteral(0),
                            stop: HirExpr::IntLiteral(3),
                            step: HirExpr::IntLiteral(1),
                        },
                        cond: Some(Box::new(HirExpr::IntLiteral(1))),
                        elt: Box::new(HirExpr::Name("0comp_11_i".to_string())),
                    },
                    HirStmt::Return(None),
                ],
            },
            HirItem::Function {
                name: "_trigger".to_string(),
                params: vec![],
                return_ty: Ty::Infer,
                body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(1)))],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    check(&hir).unwrap();
}
#[test]
fn collect_block_constraints_visits_a_dict_comp_assign_s_cond_key_and_value() {
    // Pins the `HirStmt::DictCompAssign` solver arm (its own separate
    // key/value split, not shared with the list/set arm in `lib.rs`), with a
    // `cond` present so its own `cond.is_some()` branch is covered too.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "_h".to_string(),
                params: vec![],
                return_ty: Ty::None,
                body: vec![
                    HirStmt::DictCompAssign {
                        target: "y".to_string(),
                        var: "0comp_11_i".to_string(),
                        iter: CompIter::Range {
                            start: HirExpr::IntLiteral(0),
                            stop: HirExpr::IntLiteral(3),
                            step: HirExpr::IntLiteral(1),
                        },
                        cond: Some(Box::new(HirExpr::IntLiteral(1))),
                        key: Box::new(HirExpr::StringLiteral("k".to_string())),
                        value: Box::new(HirExpr::Name("0comp_11_i".to_string())),
                    },
                    HirStmt::Return(None),
                ],
            },
            HirItem::Function {
                name: "_trigger".to_string(),
                params: vec![],
                return_ty: Ty::Infer,
                body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(1)))],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    check(&hir).unwrap();
}
#[test]
fn collect_block_constraints_visits_a_dict_comp_assign_s_key_and_value_with_no_cond() {
    // Companion to the test above: pins the `DictCompAssign` solver
    // arm's `cond.is_none()` branch, which that test's `cond: Some(...)`
    // shape never reaches.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "_h".to_string(),
                params: vec![],
                return_ty: Ty::None,
                body: vec![
                    HirStmt::DictCompAssign {
                        target: "y".to_string(),
                        var: "0comp_11_i".to_string(),
                        iter: CompIter::Range {
                            start: HirExpr::IntLiteral(0),
                            stop: HirExpr::IntLiteral(3),
                            step: HirExpr::IntLiteral(1),
                        },
                        cond: None,
                        key: Box::new(HirExpr::StringLiteral("k".to_string())),
                        value: Box::new(HirExpr::Name("0comp_11_i".to_string())),
                    },
                    HirStmt::Return(None),
                ],
            },
            HirItem::Function {
                name: "_trigger".to_string(),
                params: vec![],
                return_ty: Ty::Infer,
                body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(1)))],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    check(&hir).unwrap();
}
#[test]
fn collect_block_constraints_propagates_an_error_from_a_range_comprehension_s_stop_operand() {
    // Pins `bind_comp_loop_var`'s own `collect_expr_constraints(...)?`
    // call for a `CompIter::Range` operand failing outright, distinct
    // from the operand-resolves-but-conflicts tests above. Uses a
    // forward reference to a name assigned *later* in the same body
    // (mirrors `private_helper_inference_rejects_a_read_before_local_assignment`)
    // rather than a plain undefined name: this solver's own
    // `collect_expr_constraints` is deliberately lenient about a
    // non-local undefined name (`None => Ok(None)`, no error -- real
    // "not defined" checking is `infer_expr_in`'s job in the second
    // pass), so only a genuine *local-but-not-yet-bound* read actually
    // triggers this solver's own `unbound_local` error.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_h".to_string(),
            params: vec![],
            return_ty: Ty::None,
            body: vec![
                HirStmt::ListCompAssign {
                    target: "y".to_string(),
                    var: "0comp_11_i".to_string(),
                    iter: CompIter::Range {
                        start: HirExpr::IntLiteral(0),
                        stop: HirExpr::Name("later".to_string()),
                        step: HirExpr::IntLiteral(1),
                    },
                    cond: None,
                    elt: Box::new(HirExpr::IntLiteral(1)),
                },
                HirStmt::Assign {
                    target: "later".to_string(),
                    value: HirExpr::IntLiteral(3),
                },
                HirStmt::Return(None),
            ],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert_eq!(check(&hir).unwrap_err().code, "T0021");
}
#[test]
fn collect_block_constraints_keeps_a_name_iterable_comprehension_s_loop_variable_s_existing_term() {
    // Companion to the "fresh term" test above: pins the
    // `CompIter::Name` branch's `contains_key(var)` guard's *true*
    // branch (variable already bound), mirroring
    // `collect_block_constraints_keeps_a_for_list_loop_variable_s_existing_term`'s
    // own analogous `ForList` coverage. The loop variable is given the
    // same name as a real parameter (`0comp_11_i`), whose own term is
    // seeded into the solver's environment before any statement in the
    // body runs -- unlike a plain `ExprStmt` reference to an
    // undeclared name (which this solver's own local-name tracking
    // would instead reject as unbound, never reaching the
    // comprehension at all).
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_h".to_string(),
            params: vec![
                ("0comp_11_i".to_string(), Ty::List(Box::new(Ty::Int))),
                ("ys".to_string(), Ty::List(Box::new(Ty::Int))),
            ],
            return_ty: Ty::None,
            body: vec![
                HirStmt::ListCompAssign {
                    target: "y".to_string(),
                    var: "0comp_11_i".to_string(),
                    iter: CompIter::Name("ys".to_string()),
                    cond: None,
                    elt: Box::new(HirExpr::Name("0comp_11_i".to_string())),
                },
                HirStmt::Return(None),
            ],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    // The real check pass still rejects this program overall (`0comp_11_i`
    // is a `list[int]`-typed parameter, and the comprehension tries to
    // rebind it to `int`) -- that is not what this test pins. What
    // matters is that the *solver's* own `bind_comp_loop_var` reaches
    // its `CompIter::Name` branch with `var` already a key in
    // `env.bindings` (from the parameter seeding) and takes the
    // "already bound, do nothing" path without itself erroring, which
    // is what lets the module-wide solver pass complete and fall
    // through to the real, comprehension-aware check pass that then
    // reports the actual `T0023` conflict.
    assert_eq!(check(&hir).unwrap_err().code, "T0023");
}
#[test]
fn collect_block_constraints_propagates_an_error_from_a_list_comp_assign_s_cond() {
    // See the range-stop-operand test above for why a forward reference
    // to a same-body local, not a plain undefined name, is required to
    // exercise this solver's own error path.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_h".to_string(),
            params: vec![],
            return_ty: Ty::None,
            body: vec![
                HirStmt::ListCompAssign {
                    target: "y".to_string(),
                    var: "0comp_11_i".to_string(),
                    iter: CompIter::Range {
                        start: HirExpr::IntLiteral(0),
                        stop: HirExpr::IntLiteral(3),
                        step: HirExpr::IntLiteral(1),
                    },
                    cond: Some(Box::new(HirExpr::Name("later".to_string()))),
                    elt: Box::new(HirExpr::IntLiteral(1)),
                },
                HirStmt::Assign {
                    target: "later".to_string(),
                    value: HirExpr::IntLiteral(1),
                },
                HirStmt::Return(None),
            ],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert_eq!(check(&hir).unwrap_err().code, "T0021");
}
#[test]
fn collect_block_constraints_propagates_an_error_from_a_list_comp_assign_s_elt() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_h".to_string(),
            params: vec![],
            return_ty: Ty::None,
            body: vec![
                HirStmt::ListCompAssign {
                    target: "y".to_string(),
                    var: "0comp_11_i".to_string(),
                    iter: CompIter::Range {
                        start: HirExpr::IntLiteral(0),
                        stop: HirExpr::IntLiteral(3),
                        step: HirExpr::IntLiteral(1),
                    },
                    cond: None,
                    elt: Box::new(HirExpr::Name("later".to_string())),
                },
                HirStmt::Assign {
                    target: "later".to_string(),
                    value: HirExpr::IntLiteral(1),
                },
                HirStmt::Return(None),
            ],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert_eq!(check(&hir).unwrap_err().code, "T0021");
}
#[test]
fn collect_block_constraints_propagates_an_error_from_a_dict_comp_assign_s_loop_variable_binding() {
    // Pins `DictCompAssign`'s own `bind_comp_loop_var(...)?` call site
    // (textually distinct from the list/set arm's own call site above)
    // failing outright, using the same loop-variable-collision shape as
    // the list-comprehension version above.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "_h".to_string(),
                params: vec![("0comp_11_i".to_string(), Ty::Str)],
                return_ty: Ty::None,
                body: vec![
                    HirStmt::DictCompAssign {
                        target: "y".to_string(),
                        var: "0comp_11_i".to_string(),
                        iter: CompIter::Range {
                            start: HirExpr::IntLiteral(0),
                            stop: HirExpr::IntLiteral(3),
                            step: HirExpr::IntLiteral(1),
                        },
                        cond: None,
                        key: Box::new(HirExpr::StringLiteral("k".to_string())),
                        value: Box::new(HirExpr::IntLiteral(1)),
                    },
                    HirStmt::Return(None),
                ],
            },
            HirItem::Function {
                name: "_trigger".to_string(),
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
fn collect_block_constraints_propagates_an_error_from_a_dict_comp_assign_s_cond() {
    // See `collect_block_constraints_propagates_an_error_from_a_range_comprehension_s_stop_operand`
    // above for why a forward reference to a same-body local, not a
    // plain undefined name, is required to exercise this solver's own
    // error path (rather than being leniently ignored as `Ok(None)`).
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_h".to_string(),
            params: vec![],
            return_ty: Ty::None,
            body: vec![
                HirStmt::DictCompAssign {
                    target: "y".to_string(),
                    var: "0comp_11_i".to_string(),
                    iter: CompIter::Range {
                        start: HirExpr::IntLiteral(0),
                        stop: HirExpr::IntLiteral(3),
                        step: HirExpr::IntLiteral(1),
                    },
                    cond: Some(Box::new(HirExpr::Name("later".to_string()))),
                    key: Box::new(HirExpr::StringLiteral("k".to_string())),
                    value: Box::new(HirExpr::IntLiteral(1)),
                },
                HirStmt::Assign {
                    target: "later".to_string(),
                    value: HirExpr::IntLiteral(1),
                },
                HirStmt::Return(None),
            ],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert_eq!(check(&hir).unwrap_err().code, "T0021");
}
#[test]
fn collect_block_constraints_propagates_an_error_from_a_dict_comp_assign_s_key() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_h".to_string(),
            params: vec![],
            return_ty: Ty::None,
            body: vec![
                HirStmt::DictCompAssign {
                    target: "y".to_string(),
                    var: "0comp_11_i".to_string(),
                    iter: CompIter::Range {
                        start: HirExpr::IntLiteral(0),
                        stop: HirExpr::IntLiteral(3),
                        step: HirExpr::IntLiteral(1),
                    },
                    cond: None,
                    key: Box::new(HirExpr::Name("later".to_string())),
                    value: Box::new(HirExpr::IntLiteral(1)),
                },
                HirStmt::Assign {
                    target: "later".to_string(),
                    value: HirExpr::StringLiteral("k".to_string()),
                },
                HirStmt::Return(None),
            ],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert_eq!(check(&hir).unwrap_err().code, "T0021");
}
#[test]
fn collect_block_constraints_propagates_an_error_from_a_dict_comp_assign_s_value() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_h".to_string(),
            params: vec![],
            return_ty: Ty::None,
            body: vec![
                HirStmt::DictCompAssign {
                    target: "y".to_string(),
                    var: "0comp_11_i".to_string(),
                    iter: CompIter::Range {
                        start: HirExpr::IntLiteral(0),
                        stop: HirExpr::IntLiteral(3),
                        step: HirExpr::IntLiteral(1),
                    },
                    cond: None,
                    key: Box::new(HirExpr::StringLiteral("k".to_string())),
                    value: Box::new(HirExpr::Name("later".to_string())),
                },
                HirStmt::Assign {
                    target: "later".to_string(),
                    value: HirExpr::IntLiteral(1),
                },
                HirStmt::Return(None),
            ],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert_eq!(check(&hir).unwrap_err().code, "T0021");
}
#[test]
fn constraint_collection_len_call_returns_int_for_a_concretely_bound_set() {
    // Mirrors `constraint_collection_len_call_returns_int_for_a_concretely_bound_dict`
    // above, proving the solver's own relaxed `len()` arm (PR-11 Task 7)
    // also accepts a concretely-bound `Ty::Set` term.
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut binops = DeferredConstraints::default();
    let env = ConstraintEnvironment {
        bindings: HashMap::from([("s".to_string(), Ok(Ty::Set(Box::new(Ty::Int))))]),
        ..ConstraintEnvironment::empty(&[])
    };
    let expr = HirExpr::Call {
        callee: "len".to_string(),
        args: vec![HirExpr::Name("s".to_string())],
    };

    let term = collect_expr_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut binops,
        &env,
        &expr,
    )
    .unwrap();

    assert_eq!(term, Some(Ok(Ty::Int)));
}
#[test]
fn constraint_collection_treats_a_set_literal_as_unconstrained_but_recurses_into_elements() {
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut binops = DeferredConstraints::default();
    let env = ConstraintEnvironment::empty(&[]);
    let expr = HirExpr::SetLiteral(vec![HirExpr::IntLiteral(1)]);

    let term = collect_expr_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut binops,
        &env,
        &expr,
    )
    .unwrap();

    assert!(term.is_none());
}
#[test]
fn constraint_collection_propagates_an_error_from_a_set_literal_element() {
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut binops = DeferredConstraints::default();
    let env = ConstraintEnvironment::empty(&["missing"]);
    let expr = HirExpr::SetLiteral(vec![HirExpr::Name("missing".to_string())]);

    let err = collect_expr_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut binops,
        &env,
        &expr,
    )
    .unwrap_err();

    assert_eq!(err.code, "T0021");
}
#[test]
fn constraint_collection_treats_a_tuple_literal_as_unconstrained_but_recurses_into_elements() {
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut binops = DeferredConstraints::default();
    let env = ConstraintEnvironment::empty(&[]);
    let expr = HirExpr::TupleLiteral(vec![HirExpr::IntLiteral(1)]);

    let term = collect_expr_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut binops,
        &env,
        &expr,
    )
    .unwrap();

    assert!(term.is_none());
}
#[test]
fn constraint_collection_propagates_an_error_from_a_tuple_literal_element() {
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut binops = DeferredConstraints::default();
    let env = ConstraintEnvironment::empty(&["missing"]);
    let expr = HirExpr::TupleLiteral(vec![HirExpr::Name("missing".to_string())]);

    let err = collect_expr_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut binops,
        &env,
        &expr,
    )
    .unwrap_err();

    assert_eq!(err.code, "T0021");
}

// -- collect_expr_constraints GCI ? path (line 1384) ------------------

#[test]
fn collect_expr_constraints_propagates_error_from_generic_class_instantiate_arg() {
    // Exercises `collect_expr_constraints`'s GCI arm's `?` on the
    // recursive call (line 1384). A private function `_f` with inferred
    // types whose body contains a GCI whose argument is a call to a
    // non-callable binding (`y = 1` then `y()`). The solver path calls
    // `collect_expr_constraints`, which recurses into the GCI's args;
    // the Call arm sees `y` in `env.bindings` and returns
    // `non_callable_binding`, which propagates through the GCI arm's `?`.
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
    // Private function with inferred types — forces the solver path.
    let f = HirItem::Function {
        name: "_f".to_string(),
        params: vec![],
        return_ty: Ty::Infer,
        body: vec![
            HirStmt::ExprStmt(HirExpr::GenericClassInstantiate {
                class: "C".to_string(),
                type_arg: Ty::Int,
                args: vec![HirExpr::Call {
                    callee: "y".to_string(),
                    args: vec![],
                }],
            }),
            HirStmt::Return(None),
        ],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            init,
            // Top-level assignment creates a non-callable binding `y`
            // that the solver's first pass adds to `globals.bindings`.
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "y".to_string(),
                value: HirExpr::IntLiteral(1),
            }),
            f,
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: vec![("C".to_string(), class_def)],
    };
    // `check` should fail — the solver's `collect_expr_constraints`
    // sees `y` in `env.bindings` (a non-callable int binding) and
    // returns `non_callable_binding`, which propagates through the
    // GCI arm's `?` at line 1384.
    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("non-callable"));
}
#[test]
fn collect_block_constraints_propagates_error_from_match_subject() {
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment::empty(&["z"]);
    let body = vec![HirStmt::Match {
        subject: HirExpr::Name("z".to_string()),
        cases: vec![HirMatchCase {
            pattern: HirPattern::Wildcard,
            guard: None,
            body: vec![],
        }],
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
#[test]
fn collect_block_constraints_propagates_error_from_match_case_body() {
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment::empty(&["z"]);
    let body = vec![HirStmt::Match {
        subject: HirExpr::IntLiteral(0),
        cases: vec![HirMatchCase {
            pattern: HirPattern::Wildcard,
            guard: None,
            body: vec![HirStmt::ExprStmt(HirExpr::Name("z".to_string()))],
        }],
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
#[test]
fn collect_block_constraints_propagates_an_error_from_a_logical_not_operand() {
    // #604 (Part 3 of #573): `collect_expr_constraints`'s dedicated `Not`
    // arm (`HirExpr::UnaryOp { op: UnaryOpKind::Not, operand }`) walks its
    // operand with its own `collect_expr_constraints(..., operand)?` call,
    // separate from the generic `UnaryOp` arm exercised elsewhere in this
    // file (`Not` is peeled off before that arm ever runs, since its
    // result is always `Ty::Bool` regardless of the operand's type). A
    // parsed-source forward-reference walrus doesn't reach this specific
    // `?`: `bind_named_expr_targets`'s own pre-pass (same file) rejects an
    // unbound forward reference before `collect_expr_constraints` ever
    // walks the `if` test, exactly like
    // `collect_block_constraints_propagates_an_error_from_the_initializer_
    // expression` above needed a direct `collect_block_constraints` call
    // rather than parseable source for the same reason. `z` is declared
    // local but never bound, so wrapping it in `not z` fails with
    // `unbound_local` (T0021) from the recursive call, and the `?` here
    // must propagate that unchanged.
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment::empty(&["z"]);
    let body = vec![HirStmt::AnnAssign {
        is_final: false,
        target: "y".to_string(),
        annotation: Ty::Bool,
        value: Some(HirExpr::UnaryOp {
            op: UnaryOpKind::Not,
            operand: Box::new(HirExpr::Name("z".to_string())),
        }),
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

// ---------------------------------------------------------------------
// PR 4a of #1083 (Part 4 of #1026). The solver mirrors `infer_expr_in`'s
// two arms, and the two must not drift: the solver runs first, so an arm
// missing here reports its own error before the check pass can answer.
// ---------------------------------------------------------------------

/// A solver environment in which `o` is a foreign-import-bound object.
fn foreign_object_env<'scope, 'hir>() -> ConstraintEnvironment<'scope, 'hir> {
    let mut env = ConstraintEnvironment::empty(&[]);
    env.opaque_bindings.insert("o".to_string());
    env.foreign_objects.insert("o".to_string());
    env
}

/// Collects `<callee>(o)` against [`foreign_object_env`].
fn collect_conversion_of_a_foreign_object(
    callee: &str,
) -> Result<Option<Result<Ty, usize>>, Diagnostic> {
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut binops = DeferredConstraints::default();
    let env = foreign_object_env();
    let expr = HirExpr::Call {
        callee: callee.to_string(),
        args: vec![HirExpr::Name("o".to_string())],
    };
    collect_expr_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut binops,
        &env,
        &expr,
    )
}

#[test]
fn constraint_collection_float_call_admits_a_foreign_object_argument() {
    assert_eq!(
        collect_conversion_of_a_foreign_object("float").unwrap(),
        Some(Ok(Ty::Float))
    );
}

#[test]
fn constraint_collection_bool_call_admits_a_foreign_object_argument() {
    assert_eq!(
        collect_conversion_of_a_foreign_object("bool").unwrap(),
        Some(Ok(Ty::Bool))
    );
}

#[test]
fn constraint_collection_bool_call_keeps_c0001_for_a_non_object_argument() {
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut binops = DeferredConstraints::default();
    let env = ConstraintEnvironment::empty(&[]);
    let expr = HirExpr::Call {
        callee: "bool".to_string(),
        args: vec![HirExpr::IntLiteral(1)],
    };
    let err = collect_expr_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut binops,
        &env,
        &expr,
    )
    .unwrap_err();
    assert_eq!(err.code, "C0001");
}

#[test]
fn constraint_collection_honors_a_user_defined_bool_signature_over_the_builtin() {
    // The C4 guard, mirrored: a registered `bool` signature must resolve
    // through the ordinary signature lookup rather than through the
    // hand-recognized builtin arm -- `float`'s own case immediately above.
    let signatures = HashMap::from([(
        "bool".to_string(),
        (vec!["x".to_string()], vec![Ok(Ty::Int)], Ok(Ty::Int)),
    )]);
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut binops = DeferredConstraints::default();
    let env = ConstraintEnvironment::empty(&[]);
    let expr = HirExpr::Call {
        callee: "bool".to_string(),
        args: vec![HirExpr::IntLiteral(1)],
    };
    assert_eq!(
        collect_expr_constraints(
            &signatures,
            &mut parents,
            &mut concrete,
            &mut binops,
            &env,
            &expr,
        )
        .unwrap(),
        Some(Ok(Ty::Int))
    );
}

#[test]
fn constraint_collection_int_and_str_calls_admit_a_foreign_object_argument() {
    // Part 4 of #1026 (PR 4b of #1083): the solver-side mirrors. Without
    // them a module-scope `int(o)` reaches `unsupported_callable_builtin`
    // here and is refused with `C0001` before `infer_expr_in` ever runs.
    for (callee, expected) in [("int", Ty::Int), ("str", Ty::Str)] {
        assert_eq!(
            collect_conversion_of_a_foreign_object(callee).unwrap(),
            Some(Ok(expected)),
            "{callee}"
        );
    }
}

#[test]
fn constraint_collection_int_and_str_calls_keep_c0001_for_a_non_object_argument() {
    for callee in ["int", "str"] {
        let signatures = HashMap::new();
        let mut parents = Vec::new();
        let mut concrete = Vec::new();
        let mut binops = DeferredConstraints::default();
        let env = ConstraintEnvironment::empty(&[]);
        let expr = HirExpr::Call {
            callee: callee.to_string(),
            args: vec![HirExpr::IntLiteral(1)],
        };
        let err = collect_expr_constraints(
            &signatures,
            &mut parents,
            &mut concrete,
            &mut binops,
            &env,
            &expr,
        )
        .unwrap_err();
        assert_eq!(err.code, "C0001", "{callee}");
    }
}

#[test]
fn constraint_collection_honors_a_user_defined_int_or_str_signature_over_the_builtin() {
    // The C4 guard, mirrored for PR 4b's two names: a registered signature
    // must resolve through the ordinary lookup rather than through the
    // hand-recognized builtin arm.
    for callee in ["int", "str"] {
        let signatures = HashMap::from([(
            callee.to_string(),
            (vec!["x".to_string()], vec![Ok(Ty::Int)], Ok(Ty::Float)),
        )]);
        let mut parents = Vec::new();
        let mut concrete = Vec::new();
        let mut binops = DeferredConstraints::default();
        let env = ConstraintEnvironment::empty(&[]);
        let expr = HirExpr::Call {
            callee: callee.to_string(),
            args: vec![HirExpr::IntLiteral(1)],
        };
        assert_eq!(
            collect_expr_constraints(
                &signatures,
                &mut parents,
                &mut concrete,
                &mut binops,
                &env,
                &expr,
            )
            .unwrap(),
            Some(Ok(Ty::Float)),
            "{callee}"
        );
    }
}
