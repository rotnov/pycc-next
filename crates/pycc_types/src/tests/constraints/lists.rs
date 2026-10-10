//! Constraint collection over list literals, subscripts, slices and container methods.

use super::*;

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
