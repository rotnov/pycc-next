//! Function calls, parameters and return-type contracts.

use super::*;

#[test]
fn a_function_body_is_now_checked() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "f".to_string(),
            params: vec![],
            return_ty: Ty::None,
            body: vec![HirStmt::ExprStmt(HirExpr::Name("undefined".to_string()))],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
}

#[test]
fn a_bare_call_infers_none() {
    let env = Environment::new();
    let expr = HirExpr::Call {
        callee: "print".to_string(),
        args: vec![],
    };
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::None));
}

#[test]
fn calling_an_undefined_function_is_a_clean_error() {
    let env = Environment::new();
    let expr = HirExpr::Call {
        callee: "undefined".to_string(),
        args: vec![],
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("undefined"));
}

#[test]
fn calling_a_defined_function_infers_its_declared_return_type() {
    let mut env = Environment::new();
    env.bind_function("add".to_string(), vec![Ty::Int, Ty::Int], Ty::Int);
    let expr = HirExpr::Call {
        callee: "add".to_string(),
        args: vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)],
    };
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::Int));
}

#[test]
fn a_wide_call_uses_the_heap_argument_type_buffer() {
    let mut env = Environment::new();
    env.bind_function("wide".to_string(), vec![Ty::Int; 5], Ty::Int);
    let expr = HirExpr::Call {
        callee: "wide".to_string(),
        args: vec![HirExpr::IntLiteral(1); 5],
    };
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::Int));
}

#[test]
fn calling_a_function_with_a_bool_argument_for_an_int_parameter_succeeds() {
    let mut env = Environment::new();
    env.bind_function("f".to_string(), vec![Ty::Int], Ty::None);
    let expr = HirExpr::Call {
        callee: "f".to_string(),
        args: vec![HirExpr::BoolLiteral(true)],
    };
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::None));
}

#[test]
fn calling_a_function_with_the_wrong_number_of_arguments_is_a_clean_error() {
    let mut env = Environment::new();
    env.bind_function("add".to_string(), vec![Ty::Int, Ty::Int], Ty::Int);
    let expr = HirExpr::Call {
        callee: "add".to_string(),
        args: vec![HirExpr::IntLiteral(1)],
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("expects 2 argument"));
}

#[test]
fn an_int_argument_for_a_float_parameter_is_a_clean_error() {
    // D-086: pycc requires an explicit `float(...)` conversion at typed
    // boundaries (parameters, returns, assignments) rather than
    // following the Python typing spec's numeric-tower rule that `int`
    // is accepted wherever `float` is annotated. This is a deliberate
    // deviation from `mypy --strict` (which accepts this call), not an
    // oversight -- see D-086's rationale in
    // docs/decisions/D-086-two-type-boundary-strictness-decisions-equality.md.
    let mut env = Environment::new();
    env.bind_function("identity".to_string(), vec![Ty::Float], Ty::Float);
    let expr = HirExpr::Call {
        callee: "identity".to_string(),
        args: vec![HirExpr::IntLiteral(1)],
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(
        err.message
            .contains("argument 1 of `identity` expects `float`, got `int`")
    );
}

#[test]
fn calling_a_function_with_a_wrong_typed_argument_is_a_clean_error() {
    let mut env = Environment::new();
    env.bind_function("add".to_string(), vec![Ty::Int, Ty::Int], Ty::Int);
    let expr = HirExpr::Call {
        callee: "add".to_string(),
        args: vec![HirExpr::IntLiteral(1), HirExpr::FloatLiteral(2.5)],
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(
        err.message.contains("argument 2")
            && err.message.contains("int")
            && err.message.contains("float")
    );
}

#[test]
fn a_wide_calls_later_inference_error_precedes_an_earlier_type_mismatch() {
    let mut env = Environment::new();
    env.bind_function("f".to_string(), vec![Ty::Int; 5], Ty::None);
    let expr = HirExpr::Call {
        callee: "f".to_string(),
        args: vec![
            HirExpr::StringLiteral("wrong".to_string()),
            HirExpr::IntLiteral(2),
            HirExpr::IntLiteral(3),
            HirExpr::IntLiteral(4),
            HirExpr::Name("undefined".to_string()),
        ],
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(err.message, "name `undefined` is not defined");
}

#[test]
fn calling_a_function_with_an_undefined_argument_propagates_the_error() {
    let mut env = Environment::new();
    env.bind_function("f".to_string(), vec![Ty::Int], Ty::None);
    let expr = HirExpr::Call {
        callee: "f".to_string(),
        args: vec![HirExpr::Name("undefined".to_string())],
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
}

#[test]
fn a_function_s_body_is_checked_against_its_declared_param_types() {
    let function = HirItem::Function {
        name: "add".to_string(),
        params: vec![("a".to_string(), Ty::Int), ("b".to_string(), Ty::Int)],
        return_ty: Ty::Int,
        body: vec![HirStmt::Return(Some(HirExpr::BinOp {
            op: BinOpKind::Add,
            left: Box::new(HirExpr::Name("a".to_string())),
            right: Box::new(HirExpr::Name("b".to_string())),
        }))],
    };
    check_function(&function).unwrap();
}

#[test]
fn a_return_with_no_value_when_none_is_expected_succeeds() {
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![HirStmt::Return(None)],
    };
    check_function(&function).unwrap();
}

#[test]
fn a_return_with_no_value_when_a_value_is_expected_is_a_clean_error() {
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::Int,
        body: vec![HirStmt::Return(None)],
    };
    let err = check_function(&function).unwrap_err();
    assert_eq!(err.code, "T0022");
}

#[test]
fn a_return_type_mismatch_is_a_clean_error() {
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::Str,
        body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(1)))],
    };
    let err = check_function(&function).unwrap_err();
    assert_eq!(err.code, "T0022");
}

#[test]
fn a_value_returning_function_must_return_on_every_path() {
    let function = HirItem::Function {
        name: "answer".to_string(),
        params: vec![],
        return_ty: Ty::Int,
        body: vec![HirStmt::ExprStmt(HirExpr::Call {
            callee: "print".to_string(),
            args: vec![HirExpr::IntLiteral(42)],
        })],
    };
    let err = check_function(&function).unwrap_err();
    assert_eq!(err.code, "T0022");
    assert!(err.message.contains("can exit without returning"));
}

#[test]
fn an_if_with_returns_in_both_branches_satisfies_the_return_contract() {
    let function = HirItem::Function {
        name: "choose".to_string(),
        params: vec![("condition".to_string(), Ty::Bool)],
        return_ty: Ty::Int,
        body: vec![HirStmt::If {
            test: HirExpr::Name("condition".to_string()),
            body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(1)))],
            orelse: vec![HirStmt::Return(Some(HirExpr::IntLiteral(2)))],
        }],
    };
    check_function(&function).unwrap();
}

#[test]
fn an_if_with_only_one_returning_branch_is_t0022() {
    let function = HirItem::Function {
        name: "choose".to_string(),
        params: vec![("condition".to_string(), Ty::Bool)],
        return_ty: Ty::Int,
        body: vec![HirStmt::If {
            test: HirExpr::Name("condition".to_string()),
            body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(1)))],
            orelse: vec![],
        }],
    };
    assert_eq!(check_function(&function).unwrap_err().code, "T0022");
}

#[test]
fn a_return_whose_value_is_undefined_propagates_the_error() {
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::Int,
        body: vec![HirStmt::Return(Some(HirExpr::Name(
            "undefined".to_string(),
        )))],
    };
    let err = check_function(&function).unwrap_err();
    assert_eq!(err.code, "T0021");
}

#[test]
fn recursion_is_supported_since_the_function_s_own_signature_is_in_scope() {
    let function = HirItem::Function {
        name: "count".to_string(),
        params: vec![("n".to_string(), Ty::Int)],
        return_ty: Ty::Int,
        body: vec![HirStmt::Return(Some(HirExpr::Call {
            callee: "count".to_string(),
            args: vec![HirExpr::Name("n".to_string())],
        }))],
    };
    check_function(&function).unwrap();
}

#[test]
fn a_function_s_if_while_and_for_bodies_are_checked_against_its_return_type() {
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::Int,
        body: vec![
            HirStmt::If {
                test: HirExpr::BoolLiteral(true),
                body: vec![HirStmt::While {
                    test: HirExpr::BoolLiteral(true),
                    body: vec![HirStmt::ForRange {
                        var: "i".to_string(),
                        start: HirExpr::IntLiteral(0),
                        stop: HirExpr::IntLiteral(1),
                        step: HirExpr::IntLiteral(1),
                        body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(1)))],
                    }],
                }],
                orelse: vec![HirStmt::Return(Some(HirExpr::IntLiteral(0)))],
            },
            HirStmt::Return(Some(HirExpr::IntLiteral(2))),
        ],
    };
    check_function(&function).unwrap();
}

#[test]
fn a_bad_return_nested_in_if_while_and_for_is_still_caught() {
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::Str,
        body: vec![HirStmt::If {
            test: HirExpr::BoolLiteral(true),
            body: vec![HirStmt::While {
                test: HirExpr::BoolLiteral(true),
                body: vec![HirStmt::ForRange {
                    var: "i".to_string(),
                    start: HirExpr::IntLiteral(0),
                    stop: HirExpr::IntLiteral(1),
                    step: HirExpr::IntLiteral(1),
                    body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(1)))],
                }],
            }],
            orelse: vec![],
        }],
    };
    let err = check_function(&function).unwrap_err();
    assert_eq!(err.code, "T0022");
}

#[test]
#[should_panic(expected = "check_function called with a non-Function HirItem")]
fn check_function_panics_on_a_non_function_item() {
    let _ = check_function(&HirItem::TopLevelStmt(HirStmt::Return(None)));
}

#[test]
fn an_if_s_test_undefined_in_a_function_body_propagates_the_error() {
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![HirStmt::If {
            test: HirExpr::Name("undefined".to_string()),
            body: vec![],
            orelse: vec![],
        }],
    };
    assert_eq!(check_function(&function).unwrap_err().code, "T0021");
}

#[test]
fn an_if_s_orelse_ill_typed_in_a_function_body_propagates_the_error() {
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![HirStmt::If {
            test: HirExpr::BoolLiteral(true),
            body: vec![],
            orelse: vec![HirStmt::ExprStmt(HirExpr::Name("undefined".to_string()))],
        }],
    };
    assert_eq!(check_function(&function).unwrap_err().code, "T0021");
}

#[test]
fn an_if_with_assignments_in_a_function_whose_body_has_an_error_propagates_via_clone_path() {
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![HirStmt::If {
            test: HirExpr::BoolLiteral(true),
            body: vec![
                HirStmt::Assign {
                    target: "x".to_string(),
                    value: HirExpr::IntLiteral(1),
                },
                HirStmt::ExprStmt(HirExpr::Name("undefined".to_string())),
            ],
            orelse: vec![],
        }],
    };
    assert_eq!(check_function(&function).unwrap_err().code, "T0021");
}

#[test]
fn an_if_with_assignments_in_a_function_whose_orelse_has_an_error_propagates_via_clone_path() {
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![HirStmt::If {
            test: HirExpr::BoolLiteral(true),
            body: vec![HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::IntLiteral(1),
            }],
            orelse: vec![HirStmt::ExprStmt(HirExpr::Name("undefined".to_string()))],
        }],
    };
    assert_eq!(check_function(&function).unwrap_err().code, "T0021");
}

#[test]
fn a_while_with_assignments_in_a_function_whose_body_has_an_error_propagates_via_clone_path() {
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![HirStmt::While {
            test: HirExpr::BoolLiteral(true),
            body: vec![
                HirStmt::Assign {
                    target: "x".to_string(),
                    value: HirExpr::IntLiteral(1),
                },
                HirStmt::ExprStmt(HirExpr::Name("undefined".to_string())),
            ],
        }],
    };
    assert_eq!(check_function(&function).unwrap_err().code, "T0021");
}

#[test]
fn a_while_s_test_undefined_in_a_function_body_propagates_the_error() {
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![HirStmt::While {
            test: HirExpr::Name("undefined".to_string()),
            body: vec![],
        }],
    };
    assert_eq!(check_function(&function).unwrap_err().code, "T0021");
}

#[test]
fn a_for_range_s_start_undefined_in_a_function_body_propagates_the_error() {
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![HirStmt::ForRange {
            var: "i".to_string(),
            start: HirExpr::Name("undefined".to_string()),
            stop: HirExpr::IntLiteral(3),
            step: HirExpr::IntLiteral(1),
            body: vec![],
        }],
    };
    assert_eq!(check_function(&function).unwrap_err().code, "T0021");
}

#[test]
fn a_for_range_s_stop_undefined_in_a_function_body_propagates_the_error() {
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![HirStmt::ForRange {
            var: "i".to_string(),
            start: HirExpr::IntLiteral(0),
            stop: HirExpr::Name("undefined".to_string()),
            step: HirExpr::IntLiteral(1),
            body: vec![],
        }],
    };
    assert_eq!(check_function(&function).unwrap_err().code, "T0021");
}

#[test]
fn a_for_range_s_step_undefined_in_a_function_body_propagates_the_error() {
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![HirStmt::ForRange {
            var: "i".to_string(),
            start: HirExpr::IntLiteral(0),
            stop: HirExpr::IntLiteral(3),
            step: HirExpr::Name("undefined".to_string()),
            body: vec![],
        }],
    };
    assert_eq!(check_function(&function).unwrap_err().code, "T0021");
}

#[test]
fn a_for_list_loop_binds_its_variable_as_the_list_element_type_in_a_function_body() {
    // Direct `check_stmt_in_function` call (matching this file's own
    // established pattern, e.g. the `AnnAssign` tests just above) rather
    // than `check_function`: a for-loop body might never execute (an
    // empty list), so `block_always_returns` always treats it as "does
    // not always return" (same as `ForRange`) -- a function whose only
    // statement is a `for` loop can never satisfy a non-`None` return
    // type, regardless of what's inside the loop body.
    let mut env = Environment::new();
    env.bind("xs".to_string(), Ty::List(Box::new(Ty::Int)));
    let stmt = HirStmt::ForList {
        var: "i".to_string(),
        list: "xs".to_string(),
        body: vec![HirStmt::Assign {
            target: "y".to_string(),
            value: HirExpr::Name("i".to_string()),
        }],
    };
    check_stmt_in_function(&mut env, &["xs", "i", "y"], &stmt, Ty::None).unwrap();
    // Issue #118 Part 1: loop variable and body-only binding are Maybe.
    assert_eq!(env.lookup("i"), None);
    assert_eq!(env.lookup("y"), None);
    assert_eq!(env.lookup_any("i"), Some(Ty::Int));
    assert_eq!(env.lookup_any("y"), Some(Ty::Int));
}

#[test]
fn a_for_dict_loop_binds_its_variable_as_the_key_type_in_a_function_body() {
    // Mirrors the list-shaped test above (PR-11 Task 3, D-123): `for k in
    // d:` binds `k` as the dict's key type, not its value type.
    let mut env = Environment::new();
    env.bind("d".to_string(), Ty::Dict(Box::new((Ty::Str, Ty::Int))));
    let stmt = HirStmt::ForList {
        var: "k".to_string(),
        list: "d".to_string(),
        body: vec![HirStmt::Assign {
            target: "y".to_string(),
            value: HirExpr::Name("k".to_string()),
        }],
    };
    check_stmt_in_function(&mut env, &["d", "k", "y"], &stmt, Ty::None).unwrap();
    // Issue #118 Part 1: loop variable and body-only binding are Maybe.
    assert_eq!(env.lookup("k"), None);
    assert_eq!(env.lookup("y"), None);
    assert_eq!(env.lookup_any("k"), Some(Ty::Str));
    assert_eq!(env.lookup_any("y"), Some(Ty::Str));
}

#[test]
fn a_for_set_loop_binds_its_variable_as_the_element_type_in_a_function_body() {
    // Mirrors the dict-shaped test above (PR-11 Task 7, D-123): `for x in
    // s:` binds `x` as the set's element type.
    let mut env = Environment::new();
    env.bind("s".to_string(), Ty::Set(Box::new(Ty::Int)));
    let stmt = HirStmt::ForList {
        var: "x".to_string(),
        list: "s".to_string(),
        body: vec![HirStmt::Assign {
            target: "y".to_string(),
            value: HirExpr::Name("x".to_string()),
        }],
    };
    check_stmt_in_function(&mut env, &["s", "x", "y"], &stmt, Ty::None).unwrap();
    // Issue #118 Part 1: loop variable and body-only binding are Maybe.
    assert_eq!(env.lookup("x"), None);
    assert_eq!(env.lookup("y"), None);
    assert_eq!(env.lookup_any("x"), Some(Ty::Int));
    assert_eq!(env.lookup_any("y"), Some(Ty::Int));
}
