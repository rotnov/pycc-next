//! Name, binop, list literal, subscript and slice inference.

use super::*;

#[test]
fn referencing_an_assigned_name_infers_its_bound_type() {
    let mut env = Environment::new();
    check_stmt(
        &mut env,
        &HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::IntLiteral(1),
        },
    )
    .unwrap();
    assert_eq!(
        infer_expr(&env, &HirExpr::Name("x".to_string())),
        Ok(Ty::Int)
    );
}

#[test]
fn adding_two_ints_infers_int() {
    let env = Environment::new();
    let expr = HirExpr::BinOp {
        op: BinOpKind::Add,
        left: Box::new(HirExpr::IntLiteral(1)),
        right: Box::new(HirExpr::IntLiteral(2)),
    };
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::Int));
}

#[test]
fn a_binop_with_an_undefined_left_operand_propagates_the_error() {
    let env = Environment::new();
    let expr = HirExpr::BinOp {
        op: BinOpKind::Add,
        left: Box::new(HirExpr::Name("undefined".to_string())),
        right: Box::new(HirExpr::IntLiteral(1)),
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
}

#[test]
fn a_binop_with_an_undefined_right_operand_propagates_the_error() {
    let env = Environment::new();
    let expr = HirExpr::BinOp {
        op: BinOpKind::Add,
        left: Box::new(HirExpr::IntLiteral(1)),
        right: Box::new(HirExpr::Name("undefined".to_string())),
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
}

#[test]
fn true_division_of_two_ints_infers_float() {
    assert_eq!(
        numeric_result_type(BinOpKind::Div, Ty::Int, Ty::Int),
        Ok(Ty::Float)
    );
    assert_eq!(
        numeric_result_type(BinOpKind::Div, Ty::Bool, Ty::Bool),
        Ok(Ty::Float)
    );
}

#[test]
fn floor_division_of_two_ints_still_infers_int() {
    assert_eq!(
        numeric_result_type(BinOpKind::FloorDiv, Ty::Int, Ty::Int),
        Ok(Ty::Int)
    );
}

#[test]
fn referencing_an_undefined_name_is_a_clean_error_not_a_panic() {
    let env = Environment::new();
    let err = infer_expr(&env, &HirExpr::Name("undefined".to_string())).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("undefined"));
}

#[test]
fn adding_an_int_and_a_float_promotes_to_float() {
    let env = Environment::new();
    let expr = HirExpr::BinOp {
        op: BinOpKind::Add,
        left: Box::new(HirExpr::IntLiteral(1)),
        right: Box::new(HirExpr::FloatLiteral(2.5)),
    };
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::Float));
}

#[test]
fn an_empty_list_literal_cannot_be_inferred() {
    // Since #1021 (D-245) an empty literal that survives to the check
    // phase is one the pre-pass found no evidence for, so it reports
    // T0003 rather than borrowing T0021.
    let env = Environment::new();
    let err = infer_expr(&env, &HirExpr::ListLiteral(vec![])).unwrap_err();
    assert_eq!(err.code, "T0003");
    assert!(err.message.contains("empty list literal"));
}

#[test]
fn a_homogeneous_int_list_literal_infers_list_of_int() {
    let env = Environment::new();
    let expr = HirExpr::ListLiteral(vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)]);
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::List(Box::new(Ty::Int))));
}

#[test]
fn a_heterogeneous_list_literal_is_rejected_as_t0032() {
    let env = Environment::new();
    let expr = HirExpr::ListLiteral(vec![
        HirExpr::IntLiteral(1),
        HirExpr::StringLiteral("two".to_string()),
        HirExpr::IntLiteral(3),
    ]);
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0032");
    // Exact match, not a substring check: this is the same message text
    // baked into `tests/diagnostics/d0032_heterogeneous_list_literal
    // .expected.txt`, which can't currently run end-to-end (`pycc_mir`
    // is still non-exhaustive against Task 7's HIR forms) -- this unit
    // test is what actually guards that fixture's wording today.
    assert_eq!(
        err.message,
        "list element type mismatch: expected int (from the first element), found str"
    );
}

#[test]
fn a_list_literal_of_bool_and_int_is_rejected_since_homogeneity_uses_exact_ty_equality() {
    // D-105 requires the *exact same* `Ty` for every element -- unlike
    // `is_assignable`'s bool-is-an-int-subtype rule used elsewhere in
    // the crate, `[1, True]` is still T0032.
    let env = Environment::new();
    let expr = HirExpr::ListLiteral(vec![HirExpr::IntLiteral(1), HirExpr::BoolLiteral(true)]);
    assert_eq!(infer_expr(&env, &expr).unwrap_err().code, "T0032");
}

#[test]
fn a_homogeneous_str_list_literal_is_a_list_of_str() {
    let env = Environment::new();
    let expr = HirExpr::ListLiteral(vec![
        HirExpr::StringLiteral("a".to_string()),
        HirExpr::StringLiteral("b".to_string()),
    ]);
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::List(Box::new(Ty::Str))));
}

#[test]
fn a_list_literal_of_an_element_no_word_holds_is_rejected_as_t0034() {
    let env = Environment::new();
    let expr = HirExpr::ListLiteral(vec![HirExpr::NoneLiteral]);
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0034");
    assert_eq!(
        err.message,
        "list[None] is not compiled yet -- a list element must be int, float, bool, str, a user-class instance, or a list, tuple, dict or set of those"
    );
}

#[test]
fn a_list_literal_propagates_an_ill_typed_element_s_error() {
    let env = Environment::new();
    let expr = HirExpr::ListLiteral(vec![HirExpr::Name("undefined".to_string())]);
    assert_eq!(infer_expr(&env, &expr).unwrap_err().code, "T0021");
}

#[test]
fn subscripting_a_list_infers_its_element_type() {
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::List(Box::new(Ty::Int)));
    let expr = HirExpr::Subscript {
        base: Box::new(HirExpr::Name("x".to_string())),
        index: Box::new(HirExpr::IntLiteral(0)),
    };
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::Int));
}

#[test]
fn subscripting_a_list_of_str_infers_str_proving_subscript_is_not_int_specific() {
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::List(Box::new(Ty::Str)));
    let expr = HirExpr::Subscript {
        base: Box::new(HirExpr::Name("x".to_string())),
        index: Box::new(HirExpr::IntLiteral(0)),
    };
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::Str));
}

#[test]
fn subscripting_with_a_bool_index_is_accepted_since_bool_is_an_int_subtype() {
    // D-086: `bool` is accepted wherever `int` is expected at an operand
    // boundary (mirroring `is_assignable`'s existing param/assignment
    // rule) -- `xs[True]` is ordinary, CPython-valid Python (PEP 285),
    // not a type error. Found by an automated PR review (PR #236) and
    // confirmed against a real `pycc build` before this fix.
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::List(Box::new(Ty::Int)));
    let expr = HirExpr::Subscript {
        base: Box::new(HirExpr::Name("x".to_string())),
        index: Box::new(HirExpr::BoolLiteral(true)),
    };
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::Int));
}

#[test]
fn subscripting_with_a_non_int_index_is_rejected() {
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::List(Box::new(Ty::Int)));
    let expr = HirExpr::Subscript {
        base: Box::new(HirExpr::Name("x".to_string())),
        index: Box::new(HirExpr::StringLiteral("zero".to_string())),
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("list index must be"));
}

#[test]
fn subscripting_a_non_list_value_is_rejected_as_t0033() {
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::Int);
    let expr = HirExpr::Subscript {
        base: Box::new(HirExpr::Name("x".to_string())),
        index: Box::new(HirExpr::IntLiteral(0)),
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0033");
    // Exact match: same wording as
    // `tests/diagnostics/d0033_subscript_on_non_list.expected.txt`
    // (currently unrunnable end-to-end -- see the T0032 test above).
    assert_eq!(err.message, "`int` does not support indexing");
}

#[test]
fn subscripting_a_non_list_value_with_a_non_int_index_reports_t0033_not_t0021() {
    // Both operands are ill-typed at once: the base doesn't support
    // indexing at all, AND the index isn't int-compatible. T0033
    // ("does this value support the operation at all") must fire
    // before T0021 ("is this operand's type wrong"). The `match
    // base_ty { Ty::List(..) => ..., Ty::Dict(..) => ..., Ty::Tuple(..)
    // => ..., other => T0033 }` dispatch above already gives this
    // precedence structurally -- a non-container base falls straight
    // to the `other` arm without ever reaching a container's own
    // index-type check -- but PR-10's own final whole-branch review
    // found and pinned this exact compound case as a regression test
    // before container types existed, so it is kept here too.
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::Int);
    let expr = HirExpr::Subscript {
        base: Box::new(HirExpr::Name("x".to_string())),
        index: Box::new(HirExpr::StringLiteral("zero".to_string())),
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0033");
    assert_eq!(err.message, "`int` does not support indexing");
}

#[test]
fn subscripting_propagates_an_ill_typed_base_s_error() {
    let env = Environment::new();
    let expr = HirExpr::Subscript {
        base: Box::new(HirExpr::Name("undefined".to_string())),
        index: Box::new(HirExpr::IntLiteral(0)),
    };
    assert_eq!(infer_expr(&env, &expr).unwrap_err().code, "T0021");
}

#[test]
fn subscripting_propagates_an_ill_typed_index_s_error() {
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::List(Box::new(Ty::Int)));
    let expr = HirExpr::Subscript {
        base: Box::new(HirExpr::Name("x".to_string())),
        index: Box::new(HirExpr::Name("undefined".to_string())),
    };
    assert_eq!(infer_expr(&env, &expr).unwrap_err().code, "T0021");
}

// PR-12 Task 7 (D-118): `HirExpr::Slice` type-checking. Only
// `list[int]` ships slicing in v0.2 -- see the arm's own doc comment in
// `infer_expr_in` for the full base-type/element-type/bound-type
// diagnostic-order rationale.

#[test]
fn slicing_a_list_of_int_with_both_bounds_infers_list_of_int() {
    let mut env = Environment::new();
    env.bind("xs".to_string(), Ty::List(Box::new(Ty::Int)));
    let expr = HirExpr::Slice {
        base: Box::new(HirExpr::Name("xs".to_string())),
        start: Some(Box::new(HirExpr::IntLiteral(1))),
        stop: Some(Box::new(HirExpr::IntLiteral(3))),
        step: None,
    };
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::List(Box::new(Ty::Int))));
}

#[test]
fn slicing_with_start_omitted_type_checks() {
    let mut env = Environment::new();
    env.bind("xs".to_string(), Ty::List(Box::new(Ty::Int)));
    let expr = HirExpr::Slice {
        base: Box::new(HirExpr::Name("xs".to_string())),
        start: None,
        stop: Some(Box::new(HirExpr::IntLiteral(3))),
        step: Some(Box::new(HirExpr::IntLiteral(1))),
    };
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::List(Box::new(Ty::Int))));
}

#[test]
fn slicing_with_stop_omitted_type_checks() {
    let mut env = Environment::new();
    env.bind("xs".to_string(), Ty::List(Box::new(Ty::Int)));
    let expr = HirExpr::Slice {
        base: Box::new(HirExpr::Name("xs".to_string())),
        start: Some(Box::new(HirExpr::IntLiteral(1))),
        stop: None,
        step: Some(Box::new(HirExpr::IntLiteral(1))),
    };
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::List(Box::new(Ty::Int))));
}

#[test]
fn slicing_with_step_omitted_type_checks() {
    let mut env = Environment::new();
    env.bind("xs".to_string(), Ty::List(Box::new(Ty::Int)));
    let expr = HirExpr::Slice {
        base: Box::new(HirExpr::Name("xs".to_string())),
        start: Some(Box::new(HirExpr::IntLiteral(1))),
        stop: Some(Box::new(HirExpr::IntLiteral(3))),
        step: None,
    };
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::List(Box::new(Ty::Int))));
}

#[test]
fn slicing_with_every_bound_omitted_type_checks_as_xs_colon() {
    let mut env = Environment::new();
    env.bind("xs".to_string(), Ty::List(Box::new(Ty::Int)));
    let expr = HirExpr::Slice {
        base: Box::new(HirExpr::Name("xs".to_string())),
        start: None,
        stop: None,
        step: None,
    };
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::List(Box::new(Ty::Int))));
}

#[test]
fn slicing_a_dict_is_rejected_as_t0033() {
    // Mirrors real CPython: `d[1:2]` raises `TypeError` there too (D-118
    // -- dict/set reuse `Subscript`'s own `T0033` code, not a new one).
    let mut env = Environment::new();
    env.bind("d".to_string(), Ty::Dict(Box::new((Ty::Str, Ty::Int))));
    let expr = HirExpr::Slice {
        base: Box::new(HirExpr::Name("d".to_string())),
        start: None,
        stop: None,
        step: None,
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0033");
    assert_eq!(
        err.message,
        "`dict[str, int]` does not support slicing (only a list, a `tuple[T, ...]` or a str does)"
    );
}

#[test]
fn slicing_a_set_is_rejected_as_t0033() {
    // Mirrors real CPython: `s[1:2]` raises `TypeError` there too.
    let mut env = Environment::new();
    env.bind("s".to_string(), Ty::Set(Box::new(Ty::Int)));
    let expr = HirExpr::Slice {
        base: Box::new(HirExpr::Name("s".to_string())),
        start: None,
        stop: None,
        step: None,
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0033");
    assert_eq!(
        err.message,
        "`set[int]` does not support slicing (only a list, a `tuple[T, ...]` or a str does)"
    );
}

#[test]
fn slicing_a_tuple_is_rejected_as_t0033_as_an_explicit_deferral() {
    // D-118: unlike dict/set (never supported for slicing), `tuple[...]`
    // slicing is a real, explicit v0.2 scope cut -- but it still reuses
    // the same `T0033` code, not a distinct one.
    let mut env = Environment::new();
    env.bind("t".to_string(), Ty::Tuple(Box::new(vec![Ty::Int, Ty::Int])));
    let expr = HirExpr::Slice {
        base: Box::new(HirExpr::Name("t".to_string())),
        start: None,
        stop: None,
        step: None,
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0033");
    assert_eq!(
        err.message,
        "`tuple[int, int]` does not support slicing (only a list, a `tuple[T, ...]` or a str does)"
    );
}

#[test]
fn slicing_a_scalar_is_rejected_as_t0033() {
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::Int);
    let expr = HirExpr::Slice {
        base: Box::new(HirExpr::Name("x".to_string())),
        start: None,
        stop: None,
        step: None,
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0033");
    assert_eq!(
        err.message,
        "`int` does not support slicing (only a list, a `tuple[T, ...]` or a str does)"
    );
}

#[test]
fn slicing_a_list_of_any_element_type_keeps_its_type() {
    for elem in [Ty::Str, Ty::Float, Ty::Bool, Ty::List(Box::new(Ty::Int))] {
        let mut env = Environment::new();
        let list_ty = Ty::List(Box::new(elem));
        env.bind("xs".to_string(), list_ty.clone());
        let expr = HirExpr::Slice {
            base: Box::new(HirExpr::Name("xs".to_string())),
            start: None,
            stop: None,
            step: None,
        };
        assert_eq!(infer_expr(&env, &expr), Ok(list_ty));
    }
}

#[test]
fn slicing_with_a_non_int_start_is_rejected_as_t0021() {
    let mut env = Environment::new();
    env.bind("xs".to_string(), Ty::List(Box::new(Ty::Int)));
    let expr = HirExpr::Slice {
        base: Box::new(HirExpr::Name("xs".to_string())),
        start: Some(Box::new(HirExpr::StringLiteral("a".to_string()))),
        stop: None,
        step: None,
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(err.message, "slice start must be `int`, got `str`");
}

#[test]
fn slicing_with_a_non_int_stop_is_rejected_as_t0021() {
    let mut env = Environment::new();
    env.bind("xs".to_string(), Ty::List(Box::new(Ty::Int)));
    let expr = HirExpr::Slice {
        base: Box::new(HirExpr::Name("xs".to_string())),
        start: None,
        stop: Some(Box::new(HirExpr::FloatLiteral(1.0))),
        step: None,
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(err.message, "slice stop must be `int`, got `float`");
}

#[test]
fn slicing_with_a_non_int_step_is_rejected_as_t0021() {
    let mut env = Environment::new();
    env.bind("xs".to_string(), Ty::List(Box::new(Ty::Int)));
    let expr = HirExpr::Slice {
        base: Box::new(HirExpr::Name("xs".to_string())),
        start: None,
        stop: None,
        step: Some(Box::new(HirExpr::StringLiteral("a".to_string()))),
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(err.message, "slice step must be `int`, got `str`");
}

#[test]
fn slicing_with_a_bool_bound_is_accepted_since_bool_is_an_int_subtype() {
    // D-086, mirroring `Subscript`'s own index check: `xs[True:]` is
    // ordinary, CPython-valid Python (`bool` is an `int` subtype, PEP
    // 285), not a type error.
    let mut env = Environment::new();
    env.bind("xs".to_string(), Ty::List(Box::new(Ty::Int)));
    let expr = HirExpr::Slice {
        base: Box::new(HirExpr::Name("xs".to_string())),
        start: Some(Box::new(HirExpr::BoolLiteral(true))),
        stop: None,
        step: None,
    };
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::List(Box::new(Ty::Int))));
}

#[test]
fn slicing_reports_the_base_type_error_before_any_bound_error() {
    // D-118 diagnostic-order pin: the base-type gate (T0033) fires
    // before a bound's own type is ever inspected.
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::Int);
    let expr = HirExpr::Slice {
        base: Box::new(HirExpr::Name("x".to_string())),
        start: Some(Box::new(HirExpr::StringLiteral("bad".to_string()))),
        stop: None,
        step: None,
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0033");
}

#[test]
fn slicing_propagates_an_ill_typed_base_s_error() {
    let env = Environment::new();
    let expr = HirExpr::Slice {
        base: Box::new(HirExpr::Name("undefined".to_string())),
        start: None,
        stop: None,
        step: None,
    };
    assert_eq!(infer_expr(&env, &expr).unwrap_err().code, "T0021");
}

#[test]
fn slicing_propagates_an_ill_typed_bound_s_error() {
    let mut env = Environment::new();
    env.bind("xs".to_string(), Ty::List(Box::new(Ty::Int)));
    let expr = HirExpr::Slice {
        base: Box::new(HirExpr::Name("xs".to_string())),
        start: Some(Box::new(HirExpr::Name("undefined".to_string()))),
        stop: None,
        step: None,
    };
    assert_eq!(infer_expr(&env, &expr).unwrap_err().code, "T0021");
}
