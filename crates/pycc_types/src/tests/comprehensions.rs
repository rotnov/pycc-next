//! List, set and dict comprehensions at module scope.

use super::*;

// -- PR-12 Task 3 (D-117): comprehension type-checking, module scope --

#[test]
fn a_list_comprehension_over_range_type_checks_and_binds_target_as_list_int() {
    // Also pins that `var` is bound (via `check_assignment`) *before*
    // `elt` is checked: `elt` references the loop variable, so this
    // would fail as an unbound local if the ordering were wrong.
    let mut env = Environment::new();
    let stmt = HirStmt::ListCompAssign {
        target: "y".to_string(),
        var: "0comp_11_i".to_string(),
        iter: CompIter::Range {
            start: HirExpr::IntLiteral(0),
            stop: HirExpr::IntLiteral(3),
            step: HirExpr::IntLiteral(1),
        },
        cond: None,
        elt: Box::new(HirExpr::Name("0comp_11_i".to_string())),
    };
    check_stmt(&mut env, &stmt).unwrap();
    assert_eq!(env.lookup("y"), Some(Ty::List(Box::new(Ty::Int))));
    assert_eq!(env.lookup("0comp_11_i"), Some(Ty::Int));
}

#[test]
fn check_accepts_a_module_level_comprehension_whose_target_is_read_afterward() {
    // End-to-end pipeline pin, distinct from the hand-held-`Environment`
    // test above: every other comprehension test in this file either
    // drives `check_stmt`/`check_function` directly against a throwaway
    // `Environment`, or runs the full `check`/`check_and_resolve`
    // pipeline over a module expected to *fail*. This is the one test
    // confirming the full `check` pipeline (`module_function_names` ->
    // `concrete_function_environment` -> `check_with_environment_all` ->
    // `check_stmt`) accepts a *valid* module-level comprehension and
    // that its `target` binding genuinely survives into the next
    // top-level statement's environment, exactly like an ordinary
    // `Assign`'s binding already does.
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
                elt: Box::new(HirExpr::Name("0comp_11_i".to_string())),
            }),
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
                callee: "len".to_string(),
                args: vec![HirExpr::Name("xs".to_string())],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert!(check(&hir).is_ok());
}

#[test]
fn a_list_comprehension_s_if_filter_of_a_non_bool_type_still_type_checks() {
    // Mirrors `If`/`While`: any type is accepted as truthy, no static
    // bool-convertibility restriction.
    let mut env = Environment::new();
    let stmt = HirStmt::ListCompAssign {
        target: "y".to_string(),
        var: "0comp_11_i".to_string(),
        iter: CompIter::Range {
            start: HirExpr::IntLiteral(0),
            stop: HirExpr::IntLiteral(3),
            step: HirExpr::IntLiteral(1),
        },
        cond: Some(Box::new(HirExpr::StringLiteral("truthy".to_string()))),
        elt: Box::new(HirExpr::IntLiteral(1)),
    };
    check_stmt(&mut env, &stmt).unwrap();
    assert_eq!(env.lookup("y"), Some(Ty::List(Box::new(Ty::Int))));
}

#[test]
fn a_list_comprehension_producing_str_is_a_list_of_str() {
    let mut env = Environment::new();
    let stmt = HirStmt::ListCompAssign {
        target: "y".to_string(),
        var: "0comp_11_i".to_string(),
        iter: CompIter::Range {
            start: HirExpr::IntLiteral(0),
            stop: HirExpr::IntLiteral(3),
            step: HirExpr::IntLiteral(1),
        },
        cond: None,
        elt: Box::new(HirExpr::StringLiteral("x".to_string())),
    };
    check_stmt(&mut env, &stmt).expect("a list comprehension of any element type");
    assert_eq!(env.lookup("y"), Some(Ty::List(Box::new(Ty::Str))));
}

#[test]
fn a_list_comprehension_producing_float_is_a_list_of_float() {
    let mut env = Environment::new();
    let stmt = HirStmt::ListCompAssign {
        target: "y".to_string(),
        var: "0comp_11_i".to_string(),
        iter: CompIter::Range {
            start: HirExpr::IntLiteral(0),
            stop: HirExpr::IntLiteral(3),
            step: HirExpr::IntLiteral(1),
        },
        cond: None,
        elt: Box::new(HirExpr::FloatLiteral(1.5)),
    };
    check_stmt(&mut env, &stmt).expect("a list comprehension of any element type");
    assert_eq!(env.lookup("y"), Some(Ty::List(Box::new(Ty::Float))));
}

#[test]
fn a_list_comprehension_producing_bool_is_a_list_of_bool() {
    let mut env = Environment::new();
    let stmt = HirStmt::ListCompAssign {
        target: "y".to_string(),
        var: "0comp_11_i".to_string(),
        iter: CompIter::Range {
            start: HirExpr::IntLiteral(0),
            stop: HirExpr::IntLiteral(3),
            step: HirExpr::IntLiteral(1),
        },
        cond: None,
        elt: Box::new(HirExpr::BoolLiteral(true)),
    };
    check_stmt(&mut env, &stmt).expect("a list comprehension of any element type");
    assert_eq!(env.lookup("y"), Some(Ty::List(Box::new(Ty::Bool))));
}

#[test]
fn a_list_comprehension_over_a_bare_list_name_type_checks_and_binds_target_as_list_int() {
    // Exercises `resolve_comp_iter`'s `CompIter::Name` branch resolving
    // to `Ty::List` (the `ListCompAssign` tests above all use
    // `CompIter::Range`, never a bare-name list iterable).
    let mut env = Environment::new();
    env.bind("xs".to_string(), Ty::List(Box::new(Ty::Int)));
    let stmt = HirStmt::ListCompAssign {
        target: "y".to_string(),
        var: "0comp_20_x".to_string(),
        iter: CompIter::Name("xs".to_string()),
        cond: None,
        elt: Box::new(HirExpr::Name("0comp_20_x".to_string())),
    };
    check_stmt(&mut env, &stmt).unwrap();
    assert_eq!(env.lookup("y"), Some(Ty::List(Box::new(Ty::Int))));
    assert_eq!(env.lookup("0comp_20_x"), Some(Ty::Int));
}

#[test]
fn a_set_comprehension_over_a_bare_set_name_type_checks_and_binds_target_as_set_int() {
    // Exercises `resolve_comp_iter`'s `CompIter::Name` branch resolving
    // to `Ty::Set`.
    let mut env = Environment::new();
    env.bind("s".to_string(), Ty::Set(Box::new(Ty::Int)));
    let stmt = HirStmt::SetCompAssign {
        target: "y".to_string(),
        var: "0comp_20_x".to_string(),
        iter: CompIter::Name("s".to_string()),
        cond: None,
        elt: Box::new(HirExpr::Name("0comp_20_x".to_string())),
    };
    check_stmt(&mut env, &stmt).unwrap();
    assert_eq!(env.lookup("y"), Some(Ty::Set(Box::new(Ty::Int))));
    assert_eq!(env.lookup("0comp_20_x"), Some(Ty::Int));
}

#[test]
fn a_set_comprehension_producing_an_unhashable_element_is_rejected_as_t0038() {
    let mut env = Environment::new();
    let stmt = HirStmt::SetCompAssign {
        target: "y".to_string(),
        var: "0comp_11_i".to_string(),
        iter: CompIter::Range {
            start: HirExpr::IntLiteral(0),
            stop: HirExpr::IntLiteral(3),
            step: HirExpr::IntLiteral(1),
        },
        cond: None,
        elt: Box::new(HirExpr::NoneLiteral),
    };
    let err = check_stmt(&mut env, &stmt).unwrap_err();
    assert_eq!(err.code, "T0038");
    assert!(err.message.contains("set[None]"));
}

#[test]
fn a_set_comprehension_s_if_filter_of_a_non_bool_type_still_type_checks() {
    let mut env = Environment::new();
    let stmt = HirStmt::SetCompAssign {
        target: "y".to_string(),
        var: "0comp_11_i".to_string(),
        iter: CompIter::Range {
            start: HirExpr::IntLiteral(0),
            stop: HirExpr::IntLiteral(3),
            step: HirExpr::IntLiteral(1),
        },
        cond: Some(Box::new(HirExpr::StringLiteral("truthy".to_string()))),
        elt: Box::new(HirExpr::IntLiteral(1)),
    };
    check_stmt(&mut env, &stmt).unwrap();
    assert_eq!(env.lookup("y"), Some(Ty::Set(Box::new(Ty::Int))));
}

#[test]
fn a_dict_comprehension_over_a_bare_dict_name_type_checks_and_binds_target_as_dict_str_int() {
    // Exercises `resolve_comp_iter`'s `CompIter::Name` branch resolving
    // to `Ty::Dict`, binding `var` as the dict's *key* type (mirroring
    // `ForList`'s own `for k in d:` behavior, D-123).
    let mut env = Environment::new();
    env.bind("d".to_string(), Ty::Dict(Box::new((Ty::Str, Ty::Int))));
    let stmt = HirStmt::DictCompAssign {
        target: "y".to_string(),
        var: "0comp_20_k".to_string(),
        iter: CompIter::Name("d".to_string()),
        cond: None,
        key: Box::new(HirExpr::Name("0comp_20_k".to_string())),
        value: Box::new(HirExpr::IntLiteral(1)),
    };
    check_stmt(&mut env, &stmt).unwrap();
    assert_eq!(
        env.lookup("y"),
        Some(Ty::Dict(Box::new((Ty::Str, Ty::Int))))
    );
    assert_eq!(env.lookup("0comp_20_k"), Some(Ty::Str));
}

#[test]
fn a_dict_comprehension_producing_an_unstorable_value_is_rejected_as_t0036() {
    let mut env = Environment::new();
    let stmt = HirStmt::DictCompAssign {
        target: "y".to_string(),
        var: "0comp_11_i".to_string(),
        iter: CompIter::Range {
            start: HirExpr::IntLiteral(0),
            stop: HirExpr::IntLiteral(3),
            step: HirExpr::IntLiteral(1),
        },
        cond: None,
        key: Box::new(HirExpr::IntLiteral(1)),
        value: Box::new(HirExpr::NoneLiteral),
    };
    let err = check_stmt(&mut env, &stmt).unwrap_err();
    assert_eq!(err.code, "T0036");
    assert!(err.message.contains("dict[int, None]"));
}

#[test]
fn a_dict_comprehension_s_if_filter_of_a_non_bool_type_still_type_checks() {
    let mut env = Environment::new();
    let stmt = HirStmt::DictCompAssign {
        target: "y".to_string(),
        var: "0comp_11_i".to_string(),
        iter: CompIter::Range {
            start: HirExpr::IntLiteral(0),
            stop: HirExpr::IntLiteral(3),
            step: HirExpr::IntLiteral(1),
        },
        cond: Some(Box::new(HirExpr::StringLiteral("truthy".to_string()))),
        key: Box::new(HirExpr::StringLiteral("k".to_string())),
        value: Box::new(HirExpr::IntLiteral(1)),
    };
    check_stmt(&mut env, &stmt).unwrap();
    assert_eq!(
        env.lookup("y"),
        Some(Ty::Dict(Box::new((Ty::Str, Ty::Int))))
    );
}

#[test]
fn a_comprehension_over_a_non_list_dict_set_iterable_is_rejected_as_t0033() {
    // Reuses `ForList`'s own existing message
    // (`a_for_list_loop_over_a_non_list_value_is_rejected`).
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::Int);
    let stmt = HirStmt::ListCompAssign {
        target: "y".to_string(),
        var: "0comp_11_i".to_string(),
        iter: CompIter::Name("x".to_string()),
        cond: None,
        elt: Box::new(HirExpr::IntLiteral(1)),
    };
    let err = check_stmt(&mut env, &stmt).unwrap_err();
    assert_eq!(err.code, "T0033");
    assert!(err.message.contains("cannot be iterated"));
}

#[test]
fn a_comprehension_over_an_undefined_iterable_name_is_rejected_as_not_defined() {
    let mut env = Environment::new();
    let stmt = HirStmt::ListCompAssign {
        target: "y".to_string(),
        var: "0comp_11_i".to_string(),
        iter: CompIter::Name("undefined".to_string()),
        cond: None,
        elt: Box::new(HirExpr::IntLiteral(1)),
    };
    let err = check_stmt(&mut env, &stmt).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("not defined"));
}
#[test]
fn a_comprehension_over_a_maybe_bound_iterable_is_t0041() {
    // `if cond: xs = [1, 2, 3]` then `[x for x in xs]` -- xs is Maybe
    // after the if (no else), so `lookup_bound_name` raises T0041
    // (issue #118 Part 1).
    let mut env = Environment::new();
    check_stmt(
        &mut env,
        &HirStmt::If {
            test: HirExpr::BoolLiteral(true),
            body: vec![HirStmt::Assign {
                target: "xs".to_string(),
                value: HirExpr::ListLiteral(vec![
                    HirExpr::IntLiteral(1),
                    HirExpr::IntLiteral(2),
                    HirExpr::IntLiteral(3),
                ]),
            }],
            orelse: vec![],
        },
    )
    .unwrap();
    let stmt = HirStmt::ListCompAssign {
        target: "y".to_string(),
        var: "0comp_11_i".to_string(),
        iter: CompIter::Name("xs".to_string()),
        cond: None,
        elt: Box::new(HirExpr::IntLiteral(1)),
    };
    let err = check_stmt(&mut env, &stmt).unwrap_err();
    assert_eq!(err.code, "T0041");
}

#[test]
fn an_entirely_unannotated_private_helper_returning_a_comprehension_target_infers_list_int() {
    // #1254: the solver now binds a comprehension statement's `target` to
    // its container type (`bind_comp_target`), exactly as a plain
    // assignment of a `list[int]` literal already was (`def _h(): xs =
    // [1, 2]; return xs` resolves on the baseline). This test used to pin
    // the gap as a `T0021` "local name `xs` is not bound before this use";
    // CPython runs the program, so the return type is now inferred.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_h".to_string(),
            params: vec![],
            return_ty: Ty::Infer,
            body: vec![
                HirStmt::ListCompAssign {
                    target: "xs".to_string(),
                    var: "0comp_11_i".to_string(),
                    iter: CompIter::Range {
                        start: HirExpr::IntLiteral(0),
                        stop: HirExpr::IntLiteral(3),
                        step: HirExpr::IntLiteral(1),
                    },
                    cond: None,
                    elt: Box::new(HirExpr::Name("0comp_11_i".to_string())),
                },
                HirStmt::Return(Some(HirExpr::Name("xs".to_string()))),
            ],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let resolved = check_and_resolve(&hir).unwrap();
    assert!(matches!(
        &resolved.items[0],
        HirItem::Function { return_ty, .. } if *return_ty == Ty::List(Box::new(Ty::Int))
    ));
}

// `resolve_comp_iter`'s own `CompIter::Range` operand checks (shared by
// every comprehension kind and both scopes) -- exercised once here via
// a list comprehension, mirroring `a_for_range_loop_whose_start_is_
// undefined_propagates_the_error` and its stop/step siblings.

#[test]
fn a_comprehension_over_a_range_with_a_non_int_start_propagates_the_error() {
    let mut env = Environment::new();
    let stmt = HirStmt::ListCompAssign {
        target: "y".to_string(),
        var: "0comp_11_i".to_string(),
        iter: CompIter::Range {
            start: HirExpr::StringLiteral("x".to_string()),
            stop: HirExpr::IntLiteral(3),
            step: HirExpr::IntLiteral(1),
        },
        cond: None,
        elt: Box::new(HirExpr::IntLiteral(1)),
    };
    let err = check_stmt(&mut env, &stmt).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("range start expects"));
}

#[test]
fn a_comprehension_over_a_range_with_a_non_int_stop_propagates_the_error() {
    let mut env = Environment::new();
    let stmt = HirStmt::ListCompAssign {
        target: "y".to_string(),
        var: "0comp_11_i".to_string(),
        iter: CompIter::Range {
            start: HirExpr::IntLiteral(0),
            stop: HirExpr::StringLiteral("x".to_string()),
            step: HirExpr::IntLiteral(1),
        },
        cond: None,
        elt: Box::new(HirExpr::IntLiteral(1)),
    };
    let err = check_stmt(&mut env, &stmt).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("range stop expects"));
}

#[test]
fn a_comprehension_over_a_range_with_a_non_int_step_propagates_the_error() {
    let mut env = Environment::new();
    let stmt = HirStmt::ListCompAssign {
        target: "y".to_string(),
        var: "0comp_11_i".to_string(),
        iter: CompIter::Range {
            start: HirExpr::IntLiteral(0),
            stop: HirExpr::IntLiteral(3),
            step: HirExpr::StringLiteral("x".to_string()),
        },
        cond: None,
        elt: Box::new(HirExpr::IntLiteral(1)),
    };
    let err = check_stmt(&mut env, &stmt).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("range step expects"));
}

// Per-arm error-propagation pins (module scope): each of `resolve_comp_iter`'s
// call, the loop-variable `check_assignment`, `cond`, and `elt`/`key`/`value`
// is its own `?`-guarded call site in each of the three comprehension arms,
// so each needs its own error-propagating test for D-014's coverage gate,
// distinct from the "rejected with a specific diagnostic" tests above.

#[test]
fn a_set_comprehension_over_a_non_list_dict_set_iterable_is_rejected_as_t0033() {
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::Int);
    let stmt = HirStmt::SetCompAssign {
        target: "y".to_string(),
        var: "0comp_11_i".to_string(),
        iter: CompIter::Name("x".to_string()),
        cond: None,
        elt: Box::new(HirExpr::IntLiteral(1)),
    };
    let err = check_stmt(&mut env, &stmt).unwrap_err();
    assert_eq!(err.code, "T0033");
    assert!(err.message.contains("cannot be iterated"));
}

#[test]
fn a_dict_comprehension_over_a_non_list_dict_set_iterable_is_rejected_as_t0033() {
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::Int);
    let stmt = HirStmt::DictCompAssign {
        target: "y".to_string(),
        var: "0comp_11_i".to_string(),
        iter: CompIter::Name("x".to_string()),
        cond: None,
        key: Box::new(HirExpr::StringLiteral("k".to_string())),
        value: Box::new(HirExpr::IntLiteral(1)),
    };
    let err = check_stmt(&mut env, &stmt).unwrap_err();
    assert_eq!(err.code, "T0033");
    assert!(err.message.contains("cannot be iterated"));
}

#[test]
fn a_list_comprehension_rejects_a_conflicting_reassignment_of_its_loop_variable() {
    let mut env = Environment::new();
    env.bind("0comp_11_i".to_string(), Ty::Str);
    let stmt = HirStmt::ListCompAssign {
        target: "y".to_string(),
        var: "0comp_11_i".to_string(),
        iter: CompIter::Range {
            start: HirExpr::IntLiteral(0),
            stop: HirExpr::IntLiteral(3),
            step: HirExpr::IntLiteral(1),
        },
        cond: None,
        elt: Box::new(HirExpr::IntLiteral(1)),
    };
    assert_eq!(check_stmt(&mut env, &stmt).unwrap_err().code, "T0023");
}

#[test]
fn a_set_comprehension_rejects_a_conflicting_reassignment_of_its_loop_variable() {
    let mut env = Environment::new();
    env.bind("0comp_11_i".to_string(), Ty::Str);
    let stmt = HirStmt::SetCompAssign {
        target: "y".to_string(),
        var: "0comp_11_i".to_string(),
        iter: CompIter::Range {
            start: HirExpr::IntLiteral(0),
            stop: HirExpr::IntLiteral(3),
            step: HirExpr::IntLiteral(1),
        },
        cond: None,
        elt: Box::new(HirExpr::IntLiteral(1)),
    };
    assert_eq!(check_stmt(&mut env, &stmt).unwrap_err().code, "T0023");
}

#[test]
fn a_dict_comprehension_rejects_a_conflicting_reassignment_of_its_loop_variable() {
    let mut env = Environment::new();
    env.bind("0comp_11_i".to_string(), Ty::Str);
    let stmt = HirStmt::DictCompAssign {
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
    };
    assert_eq!(check_stmt(&mut env, &stmt).unwrap_err().code, "T0023");
}

#[test]
fn a_list_comprehension_s_if_filter_propagates_an_ill_typed_error() {
    let mut env = Environment::new();
    let stmt = HirStmt::ListCompAssign {
        target: "y".to_string(),
        var: "0comp_11_i".to_string(),
        iter: CompIter::Range {
            start: HirExpr::IntLiteral(0),
            stop: HirExpr::IntLiteral(3),
            step: HirExpr::IntLiteral(1),
        },
        cond: Some(Box::new(HirExpr::Name("undefined".to_string()))),
        elt: Box::new(HirExpr::IntLiteral(1)),
    };
    assert_eq!(check_stmt(&mut env, &stmt).unwrap_err().code, "T0021");
}

#[test]
fn a_set_comprehension_s_if_filter_propagates_an_ill_typed_error() {
    let mut env = Environment::new();
    let stmt = HirStmt::SetCompAssign {
        target: "y".to_string(),
        var: "0comp_11_i".to_string(),
        iter: CompIter::Range {
            start: HirExpr::IntLiteral(0),
            stop: HirExpr::IntLiteral(3),
            step: HirExpr::IntLiteral(1),
        },
        cond: Some(Box::new(HirExpr::Name("undefined".to_string()))),
        elt: Box::new(HirExpr::IntLiteral(1)),
    };
    assert_eq!(check_stmt(&mut env, &stmt).unwrap_err().code, "T0021");
}

#[test]
fn a_dict_comprehension_s_if_filter_propagates_an_ill_typed_error() {
    let mut env = Environment::new();
    let stmt = HirStmt::DictCompAssign {
        target: "y".to_string(),
        var: "0comp_11_i".to_string(),
        iter: CompIter::Range {
            start: HirExpr::IntLiteral(0),
            stop: HirExpr::IntLiteral(3),
            step: HirExpr::IntLiteral(1),
        },
        cond: Some(Box::new(HirExpr::Name("undefined".to_string()))),
        key: Box::new(HirExpr::StringLiteral("k".to_string())),
        value: Box::new(HirExpr::IntLiteral(1)),
    };
    assert_eq!(check_stmt(&mut env, &stmt).unwrap_err().code, "T0021");
}

#[test]
fn a_list_comprehension_s_elt_propagates_an_ill_typed_error() {
    let mut env = Environment::new();
    let stmt = HirStmt::ListCompAssign {
        target: "y".to_string(),
        var: "0comp_11_i".to_string(),
        iter: CompIter::Range {
            start: HirExpr::IntLiteral(0),
            stop: HirExpr::IntLiteral(3),
            step: HirExpr::IntLiteral(1),
        },
        cond: None,
        elt: Box::new(HirExpr::Name("undefined".to_string())),
    };
    assert_eq!(check_stmt(&mut env, &stmt).unwrap_err().code, "T0021");
}

#[test]
fn a_set_comprehension_s_elt_propagates_an_ill_typed_error() {
    let mut env = Environment::new();
    let stmt = HirStmt::SetCompAssign {
        target: "y".to_string(),
        var: "0comp_11_i".to_string(),
        iter: CompIter::Range {
            start: HirExpr::IntLiteral(0),
            stop: HirExpr::IntLiteral(3),
            step: HirExpr::IntLiteral(1),
        },
        cond: None,
        elt: Box::new(HirExpr::Name("undefined".to_string())),
    };
    assert_eq!(check_stmt(&mut env, &stmt).unwrap_err().code, "T0021");
}

#[test]
fn a_dict_comprehension_s_key_propagates_an_ill_typed_error() {
    let mut env = Environment::new();
    let stmt = HirStmt::DictCompAssign {
        target: "y".to_string(),
        var: "0comp_11_i".to_string(),
        iter: CompIter::Range {
            start: HirExpr::IntLiteral(0),
            stop: HirExpr::IntLiteral(3),
            step: HirExpr::IntLiteral(1),
        },
        cond: None,
        key: Box::new(HirExpr::Name("undefined".to_string())),
        value: Box::new(HirExpr::IntLiteral(1)),
    };
    assert_eq!(check_stmt(&mut env, &stmt).unwrap_err().code, "T0021");
}

#[test]
fn a_dict_comprehension_s_value_propagates_an_ill_typed_error() {
    let mut env = Environment::new();
    let stmt = HirStmt::DictCompAssign {
        target: "y".to_string(),
        var: "0comp_11_i".to_string(),
        iter: CompIter::Range {
            start: HirExpr::IntLiteral(0),
            stop: HirExpr::IntLiteral(3),
            step: HirExpr::IntLiteral(1),
        },
        cond: None,
        key: Box::new(HirExpr::StringLiteral("k".to_string())),
        value: Box::new(HirExpr::Name("undefined".to_string())),
    };
    assert_eq!(check_stmt(&mut env, &stmt).unwrap_err().code, "T0021");
}
