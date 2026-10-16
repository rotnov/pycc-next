//! Dict, set and tuple literals, dict item stores and tuple indexing.

use super::*;

// -- PR-11 Task 3 (D-123): dict[str, int] type-checking --------------

#[test]
fn an_empty_dict_literal_cannot_be_inferred() {
    // Since #1021 (D-245) an empty literal that survives to the check
    // phase is one the pre-pass found no evidence for, so it reports
    // T0003 rather than borrowing T0021.
    let env = Environment::new();
    let err = infer_expr(&env, &HirExpr::DictLiteral(vec![])).unwrap_err();
    assert_eq!(err.code, "T0003");
    assert!(err.message.contains("empty dict literal"));
}

#[test]
fn a_homogeneous_dict_literal_infers_dict_of_str_int() {
    let env = Environment::new();
    let expr = HirExpr::DictLiteral(vec![
        (
            HirExpr::StringLiteral("a".to_string()),
            HirExpr::IntLiteral(1),
        ),
        (
            HirExpr::StringLiteral("b".to_string()),
            HirExpr::IntLiteral(2),
        ),
    ]);
    assert_eq!(
        infer_expr(&env, &expr),
        Ok(Ty::Dict(Box::new((Ty::Str, Ty::Int))))
    );
}

#[test]
fn a_dict_literal_with_mismatched_value_types_is_rejected_as_t0035() {
    let env = Environment::new();
    let expr = HirExpr::DictLiteral(vec![
        (
            HirExpr::StringLiteral("a".to_string()),
            HirExpr::IntLiteral(1),
        ),
        (
            HirExpr::StringLiteral("b".to_string()),
            HirExpr::StringLiteral("oops".to_string()),
        ),
    ]);
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0035");
    assert_eq!(
        err.message,
        "dict entry type mismatch: expected str: int (from the first pair), found str: str"
    );
}

#[test]
fn a_dict_literal_with_mismatched_key_types_is_rejected_as_t0035() {
    // Same code as the value-mismatch case above, but exercises the
    // `this_key_ty != key_ty` half of the homogeneity check's `||`
    // independently of the value half.
    let env = Environment::new();
    let expr = HirExpr::DictLiteral(vec![
        (
            HirExpr::StringLiteral("a".to_string()),
            HirExpr::IntLiteral(1),
        ),
        (HirExpr::IntLiteral(2), HirExpr::IntLiteral(2)),
    ]);
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0035");
    assert_eq!(
        err.message,
        "dict entry type mismatch: expected str: int (from the first pair), found int: int"
    );
}

#[test]
fn a_dict_literal_of_an_unstorable_value_is_rejected_as_t0036() {
    let env = Environment::new();
    let expr = HirExpr::DictLiteral(vec![(
        HirExpr::StringLiteral("a".to_string()),
        HirExpr::NoneLiteral,
    )]);
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0036");
    assert_eq!(
        err.message,
        "dict[str, None] is not compiled yet -- a dict key must be int, float, bool, str or a tuple of those, and a value anything a list can hold"
    );
}

#[test]
fn a_bad_dict_literal_is_still_rejected_when_the_solver_path_runs_first() {
    // The dict-shaped counterpart to
    // `a_heterogeneous_list_literal_is_still_rejected_when_the_solver_path_runs_first`
    // above: `collect_expr_constraints`'s own `DictLiteral` arm always returns
    // `Ok(None)` (it has no unification-friendly representation for `Ty::Dict`
    // either), so an unrelated private helper with an unresolved (`Ty::Infer`)
    // signature must not let the solver's own leniency swallow a genuine T0036 --
    // it has to fall through to the real, dict-aware check pass.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::DictLiteral(vec![(
                    HirExpr::StringLiteral("a".to_string()),
                    HirExpr::NoneLiteral,
                )]),
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
    assert_eq!(check(&hir).unwrap_err().code, "T0036");
}

#[test]
fn a_dict_literal_propagates_an_ill_typed_first_key_s_error() {
    let env = Environment::new();
    let expr = HirExpr::DictLiteral(vec![(
        HirExpr::Name("undefined".to_string()),
        HirExpr::IntLiteral(1),
    )]);
    assert_eq!(infer_expr(&env, &expr).unwrap_err().code, "T0021");
}

#[test]
fn a_dict_literal_propagates_an_ill_typed_first_value_s_error() {
    let env = Environment::new();
    let expr = HirExpr::DictLiteral(vec![(
        HirExpr::StringLiteral("a".to_string()),
        HirExpr::Name("undefined".to_string()),
    )]);
    assert_eq!(infer_expr(&env, &expr).unwrap_err().code, "T0021");
}

#[test]
fn a_dict_literal_propagates_an_ill_typed_later_key_s_error() {
    let env = Environment::new();
    let expr = HirExpr::DictLiteral(vec![
        (
            HirExpr::StringLiteral("a".to_string()),
            HirExpr::IntLiteral(1),
        ),
        (
            HirExpr::Name("undefined".to_string()),
            HirExpr::IntLiteral(2),
        ),
    ]);
    assert_eq!(infer_expr(&env, &expr).unwrap_err().code, "T0021");
}

#[test]
fn a_dict_literal_propagates_an_ill_typed_later_value_s_error() {
    let env = Environment::new();
    let expr = HirExpr::DictLiteral(vec![
        (
            HirExpr::StringLiteral("a".to_string()),
            HirExpr::IntLiteral(1),
        ),
        (
            HirExpr::StringLiteral("b".to_string()),
            HirExpr::Name("undefined".to_string()),
        ),
    ]);
    assert_eq!(infer_expr(&env, &expr).unwrap_err().code, "T0021");
}

#[test]
fn subscripting_a_dict_infers_its_value_type() {
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::Dict(Box::new((Ty::Str, Ty::Int))));
    let expr = HirExpr::Subscript {
        base: Box::new(HirExpr::Name("x".to_string())),
        index: Box::new(HirExpr::StringLiteral("a".to_string())),
    };
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::Int));
}

#[test]
fn subscripting_a_dict_with_a_mismatched_key_type_is_rejected() {
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::Dict(Box::new((Ty::Str, Ty::Int))));
    let expr = HirExpr::Subscript {
        base: Box::new(HirExpr::Name("x".to_string())),
        index: Box::new(HirExpr::IntLiteral(1)),
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(
        err.message,
        "dict key type mismatch: expected `str`, found `int`"
    );
}

#[test]
fn len_of_a_dict_infers_int() {
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::Dict(Box::new((Ty::Str, Ty::Int))));
    let expr = HirExpr::Call {
        callee: "len".to_string(),
        args: vec![HirExpr::Name("x".to_string())],
    };
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::Int));
}

#[test]
fn a_dict_set_item_with_matching_value_type_checks_cleanly() {
    let mut env = Environment::new();
    check_stmt(
        &mut env,
        &HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::DictLiteral(vec![(
                HirExpr::StringLiteral("a".to_string()),
                HirExpr::IntLiteral(1),
            )]),
        },
    )
    .unwrap();
    check_stmt(
        &mut env,
        &HirStmt::DictSet {
            dict: "x".to_string(),
            key: HirExpr::StringLiteral("b".to_string()),
            value: HirExpr::IntLiteral(2),
        },
    )
    .unwrap();
    // `d[k] = v` never rebinds `x`'s own environment type, unlike an
    // ordinary `Assign`.
    assert_eq!(
        env.lookup("x"),
        Some(Ty::Dict(Box::new((Ty::Str, Ty::Int))))
    );
}

#[test]
fn a_dict_set_item_accepts_a_bool_value_since_bool_is_an_int_subtype() {
    // Mirrors `appending_a_bool_to_a_list_of_int_is_accepted_since_bool_is_an_int_subtype`
    // (D-086): `d[k] = True` is ordinary, CPython-valid Python.
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::Dict(Box::new((Ty::Str, Ty::Int))));
    check_stmt(
        &mut env,
        &HirStmt::DictSet {
            dict: "x".to_string(),
            key: HirExpr::StringLiteral("a".to_string()),
            value: HirExpr::BoolLiteral(true),
        },
    )
    .unwrap();
}

#[test]
fn a_dict_set_item_with_wrong_key_type_is_t0021() {
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::Dict(Box::new((Ty::Str, Ty::Int))));
    let err = check_stmt(
        &mut env,
        &HirStmt::DictSet {
            dict: "x".to_string(),
            key: HirExpr::IntLiteral(1),
            value: HirExpr::IntLiteral(2),
        },
    )
    .unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(
        err.message,
        "dict key type mismatch: expected `str`, found `int`"
    );
}

#[test]
fn a_dict_set_item_with_wrong_value_type_is_t0021() {
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::Dict(Box::new((Ty::Str, Ty::Int))));
    let err = check_stmt(
        &mut env,
        &HirStmt::DictSet {
            dict: "x".to_string(),
            key: HirExpr::StringLiteral("a".to_string()),
            value: HirExpr::StringLiteral("oops".to_string()),
        },
    )
    .unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(err.message, "cannot assign `str` to a dict value of `int`");
}

#[test]
fn a_dict_set_item_on_a_non_dict_value_is_t0033() {
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::Int);
    let err = check_stmt(
        &mut env,
        &HirStmt::DictSet {
            dict: "x".to_string(),
            key: HirExpr::StringLiteral("a".to_string()),
            value: HirExpr::IntLiteral(1),
        },
    )
    .unwrap_err();
    assert_eq!(err.code, "T0033");
    assert_eq!(err.message, "`int` does not support item assignment");
}

#[test]
fn a_dict_set_item_on_a_list_value_is_a_list_item_store() {
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::List(Box::new(Ty::Int)));
    let store = |value| HirStmt::DictSet {
        dict: "x".to_string(),
        key: HirExpr::IntLiteral(0),
        value,
    };
    check_stmt(&mut env, &store(HirExpr::BoolLiteral(true))).expect("bool is an int");
    let err = check_stmt(&mut env, &store(HirExpr::StringLiteral("a".to_string()))).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(err.message, "cannot assign `str` to a list of `int`");
}

#[test]
fn a_dict_set_item_over_an_undefined_name_is_rejected_as_not_defined() {
    let mut env = Environment::new();
    let err = check_stmt(
        &mut env,
        &HirStmt::DictSet {
            dict: "x".to_string(),
            key: HirExpr::StringLiteral("a".to_string()),
            value: HirExpr::IntLiteral(1),
        },
    )
    .unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("is not defined"));
}

#[test]
fn a_dict_set_item_propagates_an_ill_typed_key_expression_s_error() {
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::Dict(Box::new((Ty::Str, Ty::Int))));
    let err = check_stmt(
        &mut env,
        &HirStmt::DictSet {
            dict: "x".to_string(),
            key: HirExpr::Name("undefined".to_string()),
            value: HirExpr::IntLiteral(1),
        },
    )
    .unwrap_err();
    assert_eq!(err.code, "T0021");
}

#[test]
fn a_dict_set_item_propagates_an_ill_typed_value_expression_s_error() {
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::Dict(Box::new((Ty::Str, Ty::Int))));
    let err = check_stmt(
        &mut env,
        &HirStmt::DictSet {
            dict: "x".to_string(),
            key: HirExpr::StringLiteral("a".to_string()),
            value: HirExpr::Name("undefined".to_string()),
        },
    )
    .unwrap_err();
    assert_eq!(err.code, "T0021");
}

#[test]
fn function_scope_dict_set_item_with_matching_value_type_checks_cleanly() {
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::Dict(Box::new((Ty::Str, Ty::Int))));
    check_stmt_in_function(
        &mut env,
        &["x"],
        &HirStmt::DictSet {
            dict: "x".to_string(),
            key: HirExpr::StringLiteral("a".to_string()),
            value: HirExpr::IntLiteral(1),
        },
        Ty::None,
    )
    .unwrap();
}

#[test]
fn function_scope_dict_set_item_over_an_unbound_local_is_unbound_local() {
    // Mirrors the local-vs-global unbound distinction `lookup_bound_name`
    // already gives every other function-scope name lookup (e.g.
    // `ForList`'s own `list` field) -- a declared-but-unassigned local
    // reports `unbound_local`, not "not defined".
    let mut env = Environment::new();
    let err = check_stmt_in_function(
        &mut env,
        &["x"],
        &HirStmt::DictSet {
            dict: "x".to_string(),
            key: HirExpr::StringLiteral("a".to_string()),
            value: HirExpr::IntLiteral(1),
        },
        Ty::None,
    )
    .unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("is not bound before this use"));
}

// -- PR-11 Task 7 (D-123): set[int] type-checking ---------------------

#[test]
fn an_empty_set_literal_cannot_be_inferred() {
    // Unlike `DictLiteral(vec![])`, which is reachable from real source
    // (`{}` parses as an empty dict), an empty `SetLiteral` can only
    // ever be hand-built HIR -- Python has no empty-set literal spelling
    // at all.
    let env = Environment::new();
    let err = infer_expr(&env, &HirExpr::SetLiteral(vec![])).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("empty set literal"));
}

#[test]
fn a_homogeneous_set_literal_infers_set_of_int() {
    let env = Environment::new();
    let expr = HirExpr::SetLiteral(vec![
        HirExpr::IntLiteral(1),
        HirExpr::IntLiteral(2),
        HirExpr::IntLiteral(3),
    ]);
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::Set(Box::new(Ty::Int))));
}

#[test]
fn a_set_literal_with_mismatched_element_types_is_rejected_as_t0037() {
    let env = Environment::new();
    let expr = HirExpr::SetLiteral(vec![
        HirExpr::IntLiteral(1),
        HirExpr::StringLiteral("oops".to_string()),
    ]);
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0037");
    assert_eq!(
        err.message,
        "set element type mismatch: expected int (from the first element), found str"
    );
}

#[test]
fn a_set_literal_of_an_unhashable_element_is_rejected_as_t0038() {
    let env = Environment::new();
    let expr = HirExpr::SetLiteral(vec![HirExpr::NoneLiteral, HirExpr::NoneLiteral]);
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0038");
    assert_eq!(
        err.message,
        "set[None] is not compiled yet -- a set element must be int, float, bool, str, a tuple of those or a user-class instance"
    );
    assert!(err.message.contains("set[None]"));
}

#[test]
fn a_bad_set_literal_is_still_rejected_when_the_solver_path_runs_first() {
    // The set-shaped counterpart to
    // `a_bad_dict_literal_is_still_rejected_when_the_solver_path_runs_first`:
    // `collect_expr_constraints`'s own `SetLiteral` arm always returns
    // `Ok(None)` (it has no unification-friendly representation for
    // `Ty::Set` either), so an unrelated private helper with an
    // unresolved (`Ty::Infer`) signature must not let the solver's own
    // leniency swallow a genuine T0038 -- it has to fall through to the
    // real, set-aware check pass.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::SetLiteral(vec![HirExpr::NoneLiteral, HirExpr::NoneLiteral]),
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
    assert_eq!(check(&hir).unwrap_err().code, "T0038");
}

#[test]
fn a_set_literal_propagates_an_ill_typed_first_element_s_error() {
    let env = Environment::new();
    let expr = HirExpr::SetLiteral(vec![HirExpr::Name("undefined".to_string())]);
    assert_eq!(infer_expr(&env, &expr).unwrap_err().code, "T0021");
}

#[test]
fn a_set_literal_propagates_an_ill_typed_later_element_s_error() {
    let env = Environment::new();
    let expr = HirExpr::SetLiteral(vec![
        HirExpr::IntLiteral(1),
        HirExpr::Name("undefined".to_string()),
    ]);
    assert_eq!(infer_expr(&env, &expr).unwrap_err().code, "T0021");
}

#[test]
fn indexing_a_set_reports_t0033() {
    // Mirrors real CPython: sets are not subscriptable either (D-123) --
    // no explicit `Ty::Set` arm exists in `Subscript`'s own match, and
    // the generic `other` fallthrough already covers it.
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::Set(Box::new(Ty::Int)));
    let expr = HirExpr::Subscript {
        base: Box::new(HirExpr::Name("x".to_string())),
        index: Box::new(HirExpr::IntLiteral(0)),
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0033");
    assert_eq!(err.message, "`set[int]` does not support indexing");
}

#[test]
fn len_of_a_set_infers_int() {
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::Set(Box::new(Ty::Int)));
    let expr = HirExpr::Call {
        callee: "len".to_string(),
        args: vec![HirExpr::Name("x".to_string())],
    };
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::Int));
}

// -- PR-11b Task 3 (D-116): tuple[...] construction + indexing --------

#[test]
fn a_tuple_literal_of_int_bool_float_infers_ty_tuple() {
    let env = Environment::new();
    let expr = HirExpr::TupleLiteral(vec![
        HirExpr::IntLiteral(1),
        HirExpr::BoolLiteral(true),
        HirExpr::FloatLiteral(2.5),
    ]);
    assert_eq!(
        infer_expr(&env, &expr),
        Ok(Ty::Tuple(Box::new(vec![Ty::Int, Ty::Bool, Ty::Float])))
    );
}

#[test]
fn an_empty_tuple_literal_is_the_empty_tuple() {
    // v0.5: `()` is the native empty tuple, assignable to any
    // `tuple[T, ...]`.
    let env = Environment::new();
    assert_eq!(
        infer_expr(&env, &HirExpr::TupleLiteral(vec![])),
        Ok(Ty::Tuple(Box::new(vec![])))
    );
}

#[test]
fn a_tuple_literal_with_a_none_element_is_rejected_as_t0039() {
    let env = Environment::new();
    let expr = HirExpr::TupleLiteral(vec![HirExpr::IntLiteral(1), HirExpr::NoneLiteral]);
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0039");
    assert_eq!(
        err.message,
        "tuple element type `None` is not compiled yet -- a tuple element must be int, float, bool, str, a user-class instance, or a list, tuple, dict or set of those"
    );
}

#[test]
fn a_tuple_literal_propagates_an_ill_typed_element_s_error() {
    let env = Environment::new();
    let expr = HirExpr::TupleLiteral(vec![HirExpr::Name("undefined".to_string())]);
    assert_eq!(infer_expr(&env, &expr).unwrap_err().code, "T0021");
}

#[test]
fn a_tuple_literal_reports_an_earlier_element_s_t0039_before_a_later_undefined_name() {
    // D-228 decision 4: `check_tuple_element_ty` runs per element *inside*
    // this loop rather than as a whole-`Ty` postcheck, so the composed
    // `(1, None, undefined_name)` shape pins which diagnostic wins. The gate
    // fires on element 1 before element 2 is ever inferred, so `T0039` is
    // reported and the undefined name is never reached. Folding the
    // per-element gate into a whole-`Ty` postcheck like `T0034`/`T0036`/
    // `T0038` would silently flip this to `T0021`.
    let env = Environment::new();
    let expr = HirExpr::TupleLiteral(vec![
        HirExpr::IntLiteral(1),
        HirExpr::NoneLiteral,
        HirExpr::Name("undefined".to_string()),
    ]);
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0039");
    assert_eq!(
        err.message,
        "tuple element type `None` is not compiled yet -- a tuple element must be int, float, bool, str, a user-class instance, or a list, tuple, dict or set of those"
    );
}

#[test]
fn tuple_index_with_a_literal_in_range_int_infers_the_positional_element_type() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "t".to_string(),
                value: HirExpr::TupleLiteral(vec![
                    HirExpr::IntLiteral(1),
                    HirExpr::BoolLiteral(true),
                ]),
            }),
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "y".to_string(),
                value: HirExpr::Subscript {
                    base: Box::new(HirExpr::Name("t".to_string())),
                    index: Box::new(HirExpr::IntLiteral(1)),
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
fn subscripting_a_tuple_with_a_literal_index_infers_the_positional_element_type() {
    let mut env = Environment::new();
    env.bind(
        "t".to_string(),
        Ty::Tuple(Box::new(vec![Ty::Int, Ty::Bool, Ty::Float])),
    );
    let expr = HirExpr::Subscript {
        base: Box::new(HirExpr::Name("t".to_string())),
        index: Box::new(HirExpr::IntLiteral(2)),
    };
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::Float));
}

#[test]
fn tuple_index_with_a_non_literal_expression_is_rejected_as_t0040() {
    let mut env = Environment::new();
    env.bind(
        "t".to_string(),
        Ty::Tuple(Box::new(vec![Ty::Int, Ty::Bool])),
    );
    env.bind("i".to_string(), Ty::Int);
    let expr = HirExpr::Subscript {
        base: Box::new(HirExpr::Name("t".to_string())),
        index: Box::new(HirExpr::Name("i".to_string())),
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0040");
    assert_eq!(
        err.message,
        "tuple index must be a non-negative literal integer within range"
    );
}

#[test]
fn tuple_index_out_of_range_is_rejected_as_t0040() {
    let mut env = Environment::new();
    env.bind(
        "t".to_string(),
        Ty::Tuple(Box::new(vec![Ty::Int, Ty::Bool])),
    );
    let expr = HirExpr::Subscript {
        base: Box::new(HirExpr::Name("t".to_string())),
        index: Box::new(HirExpr::IntLiteral(5)),
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0040");
}

#[test]
fn a_negative_literal_tuple_index_is_rejected_as_t0040() {
    let mut env = Environment::new();
    env.bind(
        "t".to_string(),
        Ty::Tuple(Box::new(vec![Ty::Int, Ty::Bool])),
    );
    let expr = HirExpr::Subscript {
        base: Box::new(HirExpr::Name("t".to_string())),
        index: Box::new(HirExpr::IntLiteral(-1)),
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0040");
}

#[test]
fn a_bad_tuple_literal_is_still_rejected_when_the_solver_path_runs_first() {
    // The tuple-shaped counterpart to
    // `a_bad_set_literal_is_still_rejected_when_the_solver_path_runs_first`:
    // `collect_expr_constraints`'s own `TupleLiteral` arm always returns
    // `Ok(None)` (it has no unification-friendly representation for
    // `Ty::Tuple` either), so an unrelated private helper with an
    // unresolved (`Ty::Infer`) signature must not let the solver's own
    // leniency swallow a genuine T0039 -- it has to fall through to the
    // real, tuple-aware check pass.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::TupleLiteral(vec![HirExpr::NoneLiteral]),
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
    assert_eq!(check(&hir).unwrap_err().code, "T0039");
}
