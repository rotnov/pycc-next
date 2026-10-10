//! Definite/maybe binding joins through `if`, `while` and `for` at module scope.

use super::*;

#[test]
fn an_if_s_test_must_be_bool_like_and_both_branches_are_checked() {
    let mut env = Environment::new();
    let stmt = HirStmt::If {
        test: HirExpr::BoolLiteral(true),
        body: vec![HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::IntLiteral(1),
        }],
        orelse: vec![HirStmt::Assign {
            target: "y".to_string(),
            value: HirExpr::IntLiteral(2),
        }],
    };
    check_stmt(&mut env, &stmt).unwrap();
    // Issue #118 Part 1: each branch is checked in an independent clone
    // of env, then joined. `x` is bound only in the body, `y` only in
    // orelse -- both are `Maybe` after the join, so `lookup` (which
    // returns `Some` only for `Definitely`) returns `None` for both.
    // Use `lookup_any` to verify the types are retained.
    assert_eq!(env.lookup("x"), None);
    assert_eq!(env.lookup("y"), None);
    assert_eq!(env.lookup_any("x"), Some(Ty::Int));
    assert_eq!(env.lookup_any("y"), Some(Ty::Int));
}

#[test]
fn an_if_with_both_branches_binding_the_same_name_joins_to_definitely() {
    let mut env = Environment::new();
    let stmt = HirStmt::If {
        test: HirExpr::BoolLiteral(true),
        body: vec![HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::IntLiteral(1),
        }],
        orelse: vec![HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::IntLiteral(2),
        }],
    };
    check_stmt(&mut env, &stmt).unwrap();
    // Both branches bind `x` as `int` -> join is `Definitely`.
    assert_eq!(env.lookup("x"), Some(Ty::Int));
}

#[test]
fn a_no_else_if_makes_body_bindings_maybe() {
    let mut env = Environment::new();
    let stmt = HirStmt::If {
        test: HirExpr::BoolLiteral(true),
        body: vec![HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::IntLiteral(1),
        }],
        orelse: vec![],
    };
    check_stmt(&mut env, &stmt).unwrap();
    // No else -> body-only binding is `Maybe`.
    assert_eq!(env.lookup("x"), None);
    assert_eq!(env.lookup_any("x"), Some(Ty::Int));
}

#[test]
fn a_nested_no_else_if_makes_bindings_maybe() {
    // `if a: if b: x = 1` -- x is Maybe after the outer if (no else on
    // either if).
    let mut env = Environment::new();
    let stmt = HirStmt::If {
        test: HirExpr::BoolLiteral(true),
        body: vec![HirStmt::If {
            test: HirExpr::BoolLiteral(true),
            body: vec![HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::IntLiteral(1),
            }],
            orelse: vec![],
        }],
        orelse: vec![],
    };
    check_stmt(&mut env, &stmt).unwrap();
    assert_eq!(env.lookup("x"), None);
    assert_eq!(env.lookup_any("x"), Some(Ty::Int));
}

#[test]
fn an_if_else_with_nested_if_in_orelse_joins_to_maybe() {
    // `if a: x = 1 else: if b: x = 2` -- x is Maybe (orelse's if has no
    // else, so x is maybe in orelse; join of definite and maybe = maybe).
    let mut env = Environment::new();
    let stmt = HirStmt::If {
        test: HirExpr::BoolLiteral(true),
        body: vec![HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::IntLiteral(1),
        }],
        orelse: vec![HirStmt::If {
            test: HirExpr::BoolLiteral(true),
            body: vec![HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::IntLiteral(2),
            }],
            orelse: vec![],
        }],
    };
    check_stmt(&mut env, &stmt).unwrap();
    assert_eq!(env.lookup("x"), None);
    assert_eq!(env.lookup_any("x"), Some(Ty::Int));
}

#[test]
fn an_if_with_maybe_in_body_and_definite_in_orelse_joins_to_maybe() {
    // `if a: if b: x = 1 else: x = 2` -- x is Maybe (body's inner if has
    // no else, so x is maybe in body; join of maybe and definite = maybe).
    let mut env = Environment::new();
    let stmt = HirStmt::If {
        test: HirExpr::BoolLiteral(true),
        body: vec![HirStmt::If {
            test: HirExpr::BoolLiteral(true),
            body: vec![HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::IntLiteral(1),
            }],
            orelse: vec![],
        }],
        orelse: vec![HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::IntLiteral(2),
        }],
    };
    check_stmt(&mut env, &stmt).unwrap();
    assert_eq!(env.lookup("x"), None);
    assert_eq!(env.lookup_any("x"), Some(Ty::Int));
}

#[test]
fn an_if_with_maybe_in_both_branches_joins_to_maybe() {
    // `if a: if b: x = 1 else: if c: x = 2` -- x is Maybe in both
    // branches (each branch's inner if has no else, so x is Maybe in
    // each). The join of Maybe and Maybe is Maybe. Exercises the
    // (BindingState::Maybe(_), Some(BindingState::Maybe(_))) arm.
    let mut env = Environment::new();
    let stmt = HirStmt::If {
        test: HirExpr::BoolLiteral(true),
        body: vec![HirStmt::If {
            test: HirExpr::BoolLiteral(true),
            body: vec![HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::IntLiteral(1),
            }],
            orelse: vec![],
        }],
        orelse: vec![HirStmt::If {
            test: HirExpr::BoolLiteral(true),
            body: vec![HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::IntLiteral(2),
            }],
            orelse: vec![],
        }],
    };
    check_stmt(&mut env, &stmt).unwrap();
    assert_eq!(env.lookup("x"), None);
    assert_eq!(env.lookup_any("x"), Some(Ty::Int));
}

#[test]
fn an_if_with_maybe_in_orelse_only_joins_to_maybe() {
    // `if a: pass else: if b: x = 1` -- x is Maybe (orelse's inner if
    // has no else, so x is maybe in orelse; body doesn't bind x at all).
    // This exercises join_if_branches' pass 2 with a Maybe binding.
    let mut env = Environment::new();
    let stmt = HirStmt::If {
        test: HirExpr::BoolLiteral(true),
        body: vec![],
        orelse: vec![HirStmt::If {
            test: HirExpr::BoolLiteral(true),
            body: vec![HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::IntLiteral(1),
            }],
            orelse: vec![],
        }],
    };
    check_stmt(&mut env, &stmt).unwrap();
    assert_eq!(env.lookup("x"), None);
    assert_eq!(env.lookup_any("x"), Some(Ty::Int));
}

#[test]
fn an_if_with_a_while_inside_that_assigns_takes_the_clone_path() {
    // `if a: while b: x = 1` -- the while inside the if body introduces
    // bindings, so the fast path is NOT taken and the clone+join path
    // runs. Exercises introduces_bindings' While arm.
    let mut env = Environment::new();
    let stmt = HirStmt::If {
        test: HirExpr::BoolLiteral(true),
        body: vec![HirStmt::While {
            test: HirExpr::BoolLiteral(true),
            body: vec![HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::IntLiteral(1),
            }],
        }],
        orelse: vec![],
    };
    check_stmt(&mut env, &stmt).unwrap();
    assert_eq!(env.lookup("x"), None);
    assert_eq!(env.lookup_any("x"), Some(Ty::Int));
}

#[test]
fn an_if_with_a_for_list_inside_that_assigns_takes_the_clone_path() {
    // `if a: for x in items: y = 1` -- the for-list inside the if body
    // introduces bindings, so the fast path is NOT taken. Exercises
    // introduces_bindings' ForList arm.
    let mut env = Environment::new();
    env.bind("items".to_string(), Ty::List(Box::new(Ty::Int)));
    let stmt = HirStmt::If {
        test: HirExpr::BoolLiteral(true),
        body: vec![HirStmt::ForList {
            var: "x".to_string(),
            list: "items".to_string(),
            body: vec![HirStmt::Assign {
                target: "y".to_string(),
                value: HirExpr::IntLiteral(1),
            }],
        }],
        orelse: vec![],
    };
    check_stmt(&mut env, &stmt).unwrap();
}

#[test]
fn an_elif_chain_without_else_joins_to_maybe() {
    // `if a: x = 1 elif b: x = 2` (lowered to nested If in orelse) --
    // x is Maybe (inner if has no else).
    let mut env = Environment::new();
    let stmt = HirStmt::If {
        test: HirExpr::BoolLiteral(true),
        body: vec![HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::IntLiteral(1),
        }],
        orelse: vec![HirStmt::If {
            test: HirExpr::BoolLiteral(true),
            body: vec![HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::IntLiteral(2),
            }],
            orelse: vec![],
        }],
    };
    check_stmt(&mut env, &stmt).unwrap();
    assert_eq!(env.lookup("x"), None);
    assert_eq!(env.lookup_any("x"), Some(Ty::Int));
}

#[test]
fn an_elif_chain_with_else_joins_to_definitely() {
    // `if a: x = 1 elif b: x = 2 else: x = 3` -- all paths bind x.
    let mut env = Environment::new();
    let stmt = HirStmt::If {
        test: HirExpr::BoolLiteral(true),
        body: vec![HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::IntLiteral(1),
        }],
        orelse: vec![HirStmt::If {
            test: HirExpr::BoolLiteral(true),
            body: vec![HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::IntLiteral(2),
            }],
            orelse: vec![HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::IntLiteral(3),
            }],
        }],
    };
    check_stmt(&mut env, &stmt).unwrap();
    assert_eq!(env.lookup("x"), Some(Ty::Int));
}

#[test]
fn a_pre_bound_name_in_if_stays_definitely() {
    // `x = 1; if cond: x = 2` -- x was definite before, stays definite.
    let mut env = Environment::new();
    check_stmt(
        &mut env,
        &HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::IntLiteral(1),
        },
    )
    .unwrap();
    check_stmt(
        &mut env,
        &HirStmt::If {
            test: HirExpr::BoolLiteral(true),
            body: vec![HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::IntLiteral(2),
            }],
            orelse: vec![],
        },
    )
    .unwrap();
    assert_eq!(env.lookup("x"), Some(Ty::Int));
}

#[test]
fn a_type_mismatch_at_if_join_is_t0023() {
    // `if cond: x = 1 else: x = "hello"` -- incompatible types at join.
    let mut env = Environment::new();
    let stmt = HirStmt::If {
        test: HirExpr::BoolLiteral(true),
        body: vec![HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::IntLiteral(1),
        }],
        orelse: vec![HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::StringLiteral("hello".to_string()),
        }],
    };
    let err = check_stmt(&mut env, &stmt).unwrap_err();
    assert_eq!(err.code, "T0023");
}

#[test]
fn a_maybe_bound_callee_is_t0041() {
    // `if cond: f = 1` then `f()` -- f is maybe-bound, not callable.
    let mut env = Environment::new();
    check_stmt(
        &mut env,
        &HirStmt::If {
            test: HirExpr::BoolLiteral(true),
            body: vec![HirStmt::Assign {
                target: "f".to_string(),
                value: HirExpr::IntLiteral(1),
            }],
            orelse: vec![],
        },
    )
    .unwrap();
    let err = check_stmt(
        &mut env,
        &HirStmt::ExprStmt(HirExpr::Call {
            callee: "f".to_string(),
            args: vec![],
        }),
    )
    .unwrap_err();
    assert_eq!(err.code, "T0041");
}

#[test]
fn an_if_whose_test_is_undefined_propagates_the_error() {
    let mut env = Environment::new();
    let stmt = HirStmt::If {
        test: HirExpr::Name("undefined".to_string()),
        body: vec![],
        orelse: vec![],
    };
    assert_eq!(check_stmt(&mut env, &stmt).unwrap_err().code, "T0021");
}

#[test]
fn an_if_whose_body_statement_is_ill_typed_propagates_the_error() {
    let mut env = Environment::new();
    let stmt = HirStmt::If {
        test: HirExpr::BoolLiteral(true),
        body: vec![HirStmt::ExprStmt(HirExpr::Name("undefined".to_string()))],
        orelse: vec![],
    };
    assert_eq!(check_stmt(&mut env, &stmt).unwrap_err().code, "T0021");
}

#[test]
fn an_if_whose_orelse_statement_is_ill_typed_propagates_the_error() {
    let mut env = Environment::new();
    let stmt = HirStmt::If {
        test: HirExpr::BoolLiteral(true),
        body: vec![],
        orelse: vec![HirStmt::ExprStmt(HirExpr::Name("undefined".to_string()))],
    };
    assert_eq!(check_stmt(&mut env, &stmt).unwrap_err().code, "T0021");
}

#[test]
fn an_if_with_assignments_whose_body_has_an_error_propagates_via_clone_path() {
    // Body has an assignment (so fast path is NOT taken) AND an error.
    let mut env = Environment::new();
    let stmt = HirStmt::If {
        test: HirExpr::BoolLiteral(true),
        body: vec![
            HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::IntLiteral(1),
            },
            HirStmt::ExprStmt(HirExpr::Name("undefined".to_string())),
        ],
        orelse: vec![],
    };
    assert_eq!(check_stmt(&mut env, &stmt).unwrap_err().code, "T0021");
}

#[test]
fn an_if_with_assignments_whose_orelse_has_an_error_propagates_via_clone_path() {
    // Body has an assignment (so fast path is NOT taken), orelse has error.
    let mut env = Environment::new();
    let stmt = HirStmt::If {
        test: HirExpr::BoolLiteral(true),
        body: vec![HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::IntLiteral(1),
        }],
        orelse: vec![HirStmt::ExprStmt(HirExpr::Name("undefined".to_string()))],
    };
    assert_eq!(check_stmt(&mut env, &stmt).unwrap_err().code, "T0021");
}

#[test]
fn a_while_with_assignments_whose_body_has_an_error_propagates_via_clone_path() {
    // Body has an assignment (so fast path is NOT taken) AND an error.
    let mut env = Environment::new();
    let stmt = HirStmt::While {
        test: HirExpr::BoolLiteral(true),
        body: vec![
            HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::IntLiteral(1),
            },
            HirStmt::ExprStmt(HirExpr::Name("undefined".to_string())),
        ],
    };
    assert_eq!(check_stmt(&mut env, &stmt).unwrap_err().code, "T0021");
}

#[test]
fn a_while_loop_s_test_and_body_are_checked() {
    let mut env = Environment::new();
    let stmt = HirStmt::While {
        test: HirExpr::BoolLiteral(true),
        body: vec![HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::IntLiteral(1),
        }],
    };
    check_stmt(&mut env, &stmt).unwrap();
    // Issue #118 Part 1: the loop body may execute zero times, so
    // body-only binding `x` is `Maybe` after the loop.
    assert_eq!(env.lookup("x"), None);
    assert_eq!(env.lookup_any("x"), Some(Ty::Int));
}

#[test]
fn a_while_loop_body_then_read_is_t0041() {
    // `while True: x = 1` then `x` read -- T0041 (the loop body may
    // execute zero times, so `x` is maybe-bound).
    let mut env = Environment::new();
    check_stmt(
        &mut env,
        &HirStmt::While {
            test: HirExpr::BoolLiteral(true),
            body: vec![HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::IntLiteral(1),
            }],
        },
    )
    .unwrap();
    let err = check_stmt(&mut env, &HirStmt::ExprStmt(HirExpr::Name("x".to_string()))).unwrap_err();
    assert_eq!(err.code, "T0041");
}

#[test]
fn a_pre_bound_variable_survives_a_while_loop_as_definite() {
    // `x = 1; while True: x = 2` -- `x` was definitely bound before the
    // loop, so it stays `Definitely` after the loop.
    let mut env = Environment::new();
    check_stmt(
        &mut env,
        &HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::IntLiteral(1),
        },
    )
    .unwrap();
    check_stmt(
        &mut env,
        &HirStmt::While {
            test: HirExpr::BoolLiteral(true),
            body: vec![HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::IntLiteral(2),
            }],
        },
    )
    .unwrap();
    assert_eq!(env.lookup("x"), Some(Ty::Int));
}

#[test]
fn a_while_loop_with_nested_if_body_joins_to_maybe() {
    // `while True: if True: x = 1` -- x is Maybe after the loop (the
    // inner if has no else, so x is Maybe in the body; join_loop_body
    // downgrades it to Maybe). Exercises the Maybe alternative in
    // join_loop_body's inner match.
    let mut env = Environment::new();
    check_stmt(
        &mut env,
        &HirStmt::While {
            test: HirExpr::BoolLiteral(true),
            body: vec![HirStmt::If {
                test: HirExpr::BoolLiteral(true),
                body: vec![HirStmt::Assign {
                    target: "x".to_string(),
                    value: HirExpr::IntLiteral(1),
                }],
                orelse: vec![],
            }],
        },
    )
    .unwrap();
    assert_eq!(env.lookup("x"), None);
    assert_eq!(env.lookup_any("x"), Some(Ty::Int));
}

#[test]
fn a_for_range_var_read_after_loop_is_t0041() {
    // `for i in range(3): pass` then `i` read -- T0041 (the loop variable
    // is maybe-bound after the loop).
    let mut env = Environment::new();
    check_stmt(
        &mut env,
        &HirStmt::ForRange {
            var: "i".to_string(),
            start: HirExpr::IntLiteral(0),
            stop: HirExpr::IntLiteral(3),
            step: HirExpr::IntLiteral(1),
            body: vec![],
        },
    )
    .unwrap();
    let err = check_stmt(&mut env, &HirStmt::ExprStmt(HirExpr::Name("i".to_string()))).unwrap_err();
    assert_eq!(err.code, "T0041");
}

#[test]
fn a_pre_bound_for_range_var_survives_as_definite() {
    // `i = 0; for i in range(3): pass` then `i` read -- succeeds because
    // `i` was definitely bound before the loop.
    let mut env = Environment::new();
    check_stmt(
        &mut env,
        &HirStmt::Assign {
            target: "i".to_string(),
            value: HirExpr::IntLiteral(0),
        },
    )
    .unwrap();
    check_stmt(
        &mut env,
        &HirStmt::ForRange {
            var: "i".to_string(),
            start: HirExpr::IntLiteral(0),
            stop: HirExpr::IntLiteral(3),
            step: HirExpr::IntLiteral(1),
            body: vec![],
        },
    )
    .unwrap();
    assert_eq!(env.lookup("i"), Some(Ty::Int));
}

#[test]
fn a_while_loop_whose_test_is_undefined_propagates_the_error() {
    let mut env = Environment::new();
    let stmt = HirStmt::While {
        test: HirExpr::Name("undefined".to_string()),
        body: vec![],
    };
    assert_eq!(check_stmt(&mut env, &stmt).unwrap_err().code, "T0021");
}

#[test]
fn a_while_loop_whose_body_statement_is_ill_typed_propagates_the_error() {
    let mut env = Environment::new();
    let stmt = HirStmt::While {
        test: HirExpr::BoolLiteral(true),
        body: vec![HirStmt::ExprStmt(HirExpr::Name("undefined".to_string()))],
    };
    assert_eq!(check_stmt(&mut env, &stmt).unwrap_err().code, "T0021");
}

#[test]
fn a_for_range_loop_binds_its_variable_as_int_and_checks_its_body() {
    let mut env = Environment::new();
    let stmt = HirStmt::ForRange {
        var: "i".to_string(),
        start: HirExpr::IntLiteral(0),
        stop: HirExpr::IntLiteral(3),
        step: HirExpr::IntLiteral(1),
        body: vec![HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::Name("i".to_string()),
        }],
    };
    check_stmt(&mut env, &stmt).unwrap();
    // Issue #118 Part 1: the loop may execute zero times, so both the
    // loop variable `i` and body-only binding `x` are `Maybe`.
    assert_eq!(env.lookup("i"), None);
    assert_eq!(env.lookup("x"), None);
    assert_eq!(env.lookup_any("i"), Some(Ty::Int));
    assert_eq!(env.lookup_any("x"), Some(Ty::Int));
}

#[test]
fn a_pre_bound_name_survives_a_for_range_loop_as_definitely() {
    // `x = 1; for i in range(3): pass` -- x stays Definitely, i is Maybe.
    let mut env = Environment::new();
    check_stmt(
        &mut env,
        &HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::IntLiteral(1),
        },
    )
    .unwrap();
    check_stmt(
        &mut env,
        &HirStmt::ForRange {
            var: "i".to_string(),
            start: HirExpr::IntLiteral(0),
            stop: HirExpr::IntLiteral(3),
            step: HirExpr::IntLiteral(1),
            body: vec![],
        },
    )
    .unwrap();
    assert_eq!(env.lookup("x"), Some(Ty::Int));
    assert_eq!(env.lookup("i"), None);
    assert_eq!(env.lookup_any("i"), Some(Ty::Int));
}

#[test]
fn a_for_range_loop_whose_start_is_undefined_propagates_the_error() {
    let mut env = Environment::new();
    let stmt = HirStmt::ForRange {
        var: "i".to_string(),
        start: HirExpr::Name("undefined".to_string()),
        stop: HirExpr::IntLiteral(3),
        step: HirExpr::IntLiteral(1),
        body: vec![],
    };
    assert_eq!(check_stmt(&mut env, &stmt).unwrap_err().code, "T0021");
}

#[test]
fn a_for_range_loop_whose_stop_is_undefined_propagates_the_error() {
    let mut env = Environment::new();
    let stmt = HirStmt::ForRange {
        var: "i".to_string(),
        start: HirExpr::IntLiteral(0),
        stop: HirExpr::Name("undefined".to_string()),
        step: HirExpr::IntLiteral(1),
        body: vec![],
    };
    assert_eq!(check_stmt(&mut env, &stmt).unwrap_err().code, "T0021");
}

#[test]
fn a_for_range_loop_whose_step_is_undefined_propagates_the_error() {
    let mut env = Environment::new();
    let stmt = HirStmt::ForRange {
        var: "i".to_string(),
        start: HirExpr::IntLiteral(0),
        stop: HirExpr::IntLiteral(3),
        step: HirExpr::Name("undefined".to_string()),
        body: vec![],
    };
    assert_eq!(check_stmt(&mut env, &stmt).unwrap_err().code, "T0021");
}

#[test]
fn a_for_range_loop_whose_body_statement_is_ill_typed_propagates_the_error() {
    let mut env = Environment::new();
    let stmt = HirStmt::ForRange {
        var: "i".to_string(),
        start: HirExpr::IntLiteral(0),
        stop: HirExpr::IntLiteral(3),
        step: HirExpr::IntLiteral(1),
        body: vec![HirStmt::ExprStmt(HirExpr::Name("undefined".to_string()))],
    };
    assert_eq!(check_stmt(&mut env, &stmt).unwrap_err().code, "T0021");
}

#[test]
fn a_for_range_loop_rejects_a_non_int_operand() {
    let mut env = Environment::new();
    let stmt = HirStmt::ForRange {
        var: "i".to_string(),
        start: HirExpr::IntLiteral(0),
        stop: HirExpr::StringLiteral("three".to_string()),
        step: HirExpr::IntLiteral(1),
        body: vec![],
    };
    let err = check_stmt(&mut env, &stmt).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("range stop"));
    assert_eq!(env.lookup("i"), None);
}

#[test]
fn a_for_range_loop_accepts_bool_as_an_int_subtype() {
    let mut env = Environment::new();
    let stmt = HirStmt::ForRange {
        var: "i".to_string(),
        start: HirExpr::BoolLiteral(false),
        stop: HirExpr::IntLiteral(3),
        step: HirExpr::BoolLiteral(true),
        body: vec![],
    };
    check_stmt(&mut env, &stmt).unwrap();
    // Issue #118 Part 1: loop variable is Maybe after the loop.
    assert_eq!(env.lookup("i"), None);
    assert_eq!(env.lookup_any("i"), Some(Ty::Int));
}

#[test]
fn a_for_list_loop_binds_its_variable_as_the_list_element_type_and_checks_its_body() {
    let mut env = Environment::new();
    check_stmt(
        &mut env,
        &HirStmt::Assign {
            target: "xs".to_string(),
            value: HirExpr::ListLiteral(vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)]),
        },
    )
    .unwrap();
    let stmt = HirStmt::ForList {
        var: "i".to_string(),
        list: "xs".to_string(),
        body: vec![HirStmt::Assign {
            target: "y".to_string(),
            value: HirExpr::Name("i".to_string()),
        }],
    };
    check_stmt(&mut env, &stmt).unwrap();
    // Issue #118 Part 1: the loop may execute zero times, so both the
    // loop variable `i` and body-only binding `y` are `Maybe`.
    assert_eq!(env.lookup("i"), None);
    assert_eq!(env.lookup("y"), None);
    assert_eq!(env.lookup_any("i"), Some(Ty::Int));
    assert_eq!(env.lookup_any("y"), Some(Ty::Int));
}

#[test]
fn a_for_list_loop_binds_its_variable_as_str_for_a_list_of_str() {
    // Proves the `ForList` inference isn't hardcoded to `Ty::Int` --
    // it's generic over whatever scalar element type the list actually
    // has (see the `HirExpr::Subscript`/`ListAppend` genericity notes).
    // Bound directly rather than via a `str` list literal: T0034 gates
    // every non-`int` list literal at creation time, so no source-level
    // program can ever produce a live `Ty::List(Ty::Str)` binding --
    // this is the only way to reach this arm with one.
    let mut env = Environment::new();
    env.bind("xs".to_string(), Ty::List(Box::new(Ty::Str)));
    let stmt = HirStmt::ForList {
        var: "i".to_string(),
        list: "xs".to_string(),
        body: vec![],
    };
    check_stmt(&mut env, &stmt).unwrap();
    // Issue #118 Part 1: loop variable is Maybe after the loop.
    assert_eq!(env.lookup("i"), None);
    assert_eq!(env.lookup_any("i"), Some(Ty::Str));
}

#[test]
fn a_for_dict_loop_binds_its_variable_as_the_key_type() {
    // PR-11 Task 3 (D-123): `for k in d:` iterates a dict's own keys, so
    // `var` binds as the dict's key type (`Ty::Str`), not its value type.
    let mut env = Environment::new();
    env.bind("d".to_string(), Ty::Dict(Box::new((Ty::Str, Ty::Int))));
    let stmt = HirStmt::ForList {
        var: "k".to_string(),
        list: "d".to_string(),
        body: vec![],
    };
    check_stmt(&mut env, &stmt).unwrap();
    // Issue #118 Part 1: loop variable is Maybe after the loop.
    assert_eq!(env.lookup("k"), None);
    assert_eq!(env.lookup_any("k"), Some(Ty::Str));
}

#[test]
fn a_for_set_loop_binds_its_variable_as_the_element_type() {
    // PR-11 Task 7 (D-123): `for x in s:` iterates a set's own elements,
    // so `var` binds as the set's element type (`Ty::Int` for
    // `set[int]`).
    let mut env = Environment::new();
    env.bind("s".to_string(), Ty::Set(Box::new(Ty::Int)));
    let stmt = HirStmt::ForList {
        var: "x".to_string(),
        list: "s".to_string(),
        body: vec![],
    };
    check_stmt(&mut env, &stmt).unwrap();
    // Issue #118 Part 1: loop variable is Maybe after the loop.
    assert_eq!(env.lookup("x"), None);
    assert_eq!(env.lookup_any("x"), Some(Ty::Int));
}

#[test]
fn a_pre_bound_for_list_var_survives_as_definite() {
    // `i = 0; xs = [1, 2, 3]; for i in xs: pass` then `i` read --
    // succeeds because `i` was definitely bound before the loop
    // (issue #118 Part 1).
    let mut env = Environment::new();
    check_stmt(
        &mut env,
        &HirStmt::Assign {
            target: "i".to_string(),
            value: HirExpr::IntLiteral(0),
        },
    )
    .unwrap();
    env.bind("xs".to_string(), Ty::List(Box::new(Ty::Int)));
    check_stmt(
        &mut env,
        &HirStmt::ForList {
            var: "i".to_string(),
            list: "xs".to_string(),
            body: vec![],
        },
    )
    .unwrap();
    assert_eq!(env.lookup("i"), Some(Ty::Int));
}

#[test]
fn a_for_list_loop_over_an_undefined_list_is_rejected() {
    let mut env = Environment::new();
    let stmt = HirStmt::ForList {
        var: "i".to_string(),
        list: "undefined".to_string(),
        body: vec![],
    };
    let err = check_stmt(&mut env, &stmt).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("not defined"));
}

#[test]
fn a_for_list_loop_over_a_non_list_value_is_rejected() {
    let mut env = Environment::new();
    check_stmt(
        &mut env,
        &HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::IntLiteral(5),
        },
    )
    .unwrap();
    let stmt = HirStmt::ForList {
        var: "i".to_string(),
        list: "x".to_string(),
        body: vec![],
    };
    let err = check_stmt(&mut env, &stmt).unwrap_err();
    assert_eq!(err.code, "T0033");
    assert!(err.message.contains("cannot be iterated"));
}

#[test]
fn a_for_list_loop_whose_body_statement_is_ill_typed_propagates_the_error() {
    let mut env = Environment::new();
    check_stmt(
        &mut env,
        &HirStmt::Assign {
            target: "xs".to_string(),
            value: HirExpr::ListLiteral(vec![HirExpr::IntLiteral(1)]),
        },
    )
    .unwrap();
    let stmt = HirStmt::ForList {
        var: "i".to_string(),
        list: "xs".to_string(),
        body: vec![HirStmt::ExprStmt(HirExpr::Name("undefined".to_string()))],
    };
    assert_eq!(check_stmt(&mut env, &stmt).unwrap_err().code, "T0021");
}

#[test]
fn a_for_list_loop_rejects_a_conflicting_reassignment_of_its_variable() {
    // `i` is already bound to `str` before the loop -- `check_assignment`
    // itself (not the list/element-type logic above it) is what rejects
    // rebinding it to the list's `int` element type.
    let mut env = Environment::new();
    check_stmt(
        &mut env,
        &HirStmt::Assign {
            target: "i".to_string(),
            value: HirExpr::StringLiteral("not an int".to_string()),
        },
    )
    .unwrap();
    check_stmt(
        &mut env,
        &HirStmt::Assign {
            target: "xs".to_string(),
            value: HirExpr::ListLiteral(vec![HirExpr::IntLiteral(1)]),
        },
    )
    .unwrap();
    let stmt = HirStmt::ForList {
        var: "i".to_string(),
        list: "xs".to_string(),
        body: vec![],
    };
    assert_eq!(check_stmt(&mut env, &stmt).unwrap_err().code, "T0023");
}
