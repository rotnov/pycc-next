//! Constraint collection over builtin calls (`len`, `float`, `bool`, conversions) and C0001 classification.

use super::*;

#[test]
fn constraint_collection_len_call_returns_int_for_a_concretely_bound_list() {
    // `lst`'s binding is a directly concrete `TypeTerm` (`Ok(Ty::List(_))`)
    // rather than one produced by `ListLiteral` (which always returns
    // `Ok(None)` in this solver, per its own comment in `lib.rs`) -- this is
    // the only way to get a concrete `Ty::List` term to validate against
    // at constraint-collection time.
    let signatures = HashMap::default();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut binops = DeferredConstraints::default();
    let env = ConstraintEnvironment {
        bindings: HashMap::from_iter([("lst".to_string(), Ok(Ty::List(Box::new(Ty::Int))))]),
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
    let signatures = HashMap::default();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut binops = DeferredConstraints::default();
    let unresolved = fresh_term(&mut parents, &mut concrete);
    let env = ConstraintEnvironment {
        bindings: HashMap::from_iter([("lst".to_string(), unresolved)]),
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
    let signatures = HashMap::default();
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
    let signatures = HashMap::default();
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
    let signatures = HashMap::default();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut binops = DeferredConstraints::default();
    let env = ConstraintEnvironment {
        bindings: HashMap::from_iter([("x".to_string(), Ok(Ty::Int))]),
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
    let signatures = HashMap::default();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut binops = DeferredConstraints::default();
    let unresolved = fresh_term(&mut parents, &mut concrete);
    let env = ConstraintEnvironment {
        bindings: HashMap::from_iter([("x".to_string(), unresolved)]),
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
    let signatures = HashMap::default();
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
    let signatures = HashMap::default();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut binops = DeferredConstraints::default();
    let env = ConstraintEnvironment::empty(&[]);
    let expr = HirExpr::Call {
        callee: "float".to_string(),
        args: vec![HirExpr::ListLiteral(vec![HirExpr::IntLiteral(1)])],
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
        "`float` expects an `int`, `float`, or `bool` argument, got `list[int]`"
    );
}
#[test]
fn constraint_collection_honors_a_user_defined_float_signature_over_the_builtin() {
    // Same post-merge review finding as `infer_expr_in`'s own
    // `a_user_defined_float_function_takes_priority_over_the_builtin`:
    // a registered `float` signature (e.g. one accepting `str`, which
    // the builtin itself would reject) must resolve through the normal
    // signature lookup, not the hand-recognized builtin arm.
    let signatures = HashMap::from_iter([(
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
fn constraint_collection_classifies_value_error_as_c0001() {
    // The private-helper inference path (`collect_expr_constraints`)
    // must apply the same C0001 classification for a known callable
    // builtin, rather than deferring with `Ok(None)` -- a private
    // helper calling `ValueError` gets C0001 directly.
    let signatures = HashMap::default();
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
    let signatures = HashMap::default();
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
    let signatures = HashMap::default();
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
    let signatures = HashMap::from_iter([(
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
    let signatures = HashMap::default();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut binops = DeferredConstraints::default();
    let env = ConstraintEnvironment {
        bindings: HashMap::from_iter([(
            "d".to_string(),
            Ok(Ty::Dict(Box::new((Ty::Str, Ty::Int)))),
        )]),
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
fn constraint_collection_len_call_returns_int_for_a_concretely_bound_set() {
    // Mirrors `constraint_collection_len_call_returns_int_for_a_concretely_bound_dict`
    // above, proving the solver's own relaxed `len()` arm (PR-11 Task 7)
    // also accepts a concretely-bound `Ty::Set` term.
    let signatures = HashMap::default();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut binops = DeferredConstraints::default();
    let env = ConstraintEnvironment {
        bindings: HashMap::from_iter([("s".to_string(), Ok(Ty::Set(Box::new(Ty::Int))))]),
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
    let signatures = HashMap::default();
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
fn constraint_collection_bool_call_of_a_native_argument_is_bool() {
    let signatures = HashMap::default();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut binops = DeferredConstraints::default();
    let env = ConstraintEnvironment::empty(&[]);
    let expr = HirExpr::Call {
        callee: "bool".to_string(),
        args: vec![HirExpr::IntLiteral(1)],
    };
    let ty = collect_expr_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut binops,
        &env,
        &expr,
    )
    .unwrap();
    assert_eq!(ty, Some(Ok(Ty::Bool)));
}

#[test]
fn constraint_collection_honors_a_user_defined_bool_signature_over_the_builtin() {
    // The C4 guard, mirrored: a registered `bool` signature must resolve
    // through the ordinary signature lookup rather than through the
    // hand-recognized builtin arm -- `float`'s own case immediately above.
    let signatures = HashMap::from_iter([(
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
fn constraint_collection_honors_a_user_defined_int_or_str_signature_over_the_builtin() {
    // The C4 guard, mirrored for PR 4b's two names: a registered signature
    // must resolve through the ordinary lookup rather than through the
    // hand-recognized builtin arm.
    for callee in ["int", "str"] {
        let signatures = HashMap::from_iter([(
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
