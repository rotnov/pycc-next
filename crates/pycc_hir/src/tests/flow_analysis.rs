//! Builtin exception parents, `Optional` tests, `definitely_terminates` and `killed_names`.

use super::*;

// -- #382 coverage tests --

#[test]
fn builtin_exception_parent_returns_none_for_exception_root() {
    assert_eq!(builtin_exception_parent("Exception"), None);
}

#[test]
fn builtin_exception_parent_returns_exception_for_subclasses() {
    assert_eq!(builtin_exception_parent("ValueError"), Some("Exception"));
    assert_eq!(builtin_exception_parent("TypeError"), Some("Exception"));
    assert_eq!(builtin_exception_parent("KeyError"), Some("Exception"));
    assert_eq!(builtin_exception_parent("IndexError"), Some("Exception"));
    assert_eq!(
        builtin_exception_parent("ZeroDivisionError"),
        Some("Exception")
    );
    assert_eq!(builtin_exception_parent("RuntimeError"), Some("Exception"));
}

#[test]
fn builtin_exception_parent_returns_none_for_unknown_class() {
    assert_eq!(builtin_exception_parent("NotAnException"), None);
}

#[test]
fn is_builtin_exception_class_recognizes_all_builtins() {
    assert!(is_builtin_exception_class("Exception"));
    assert!(is_builtin_exception_class("ValueError"));
    assert!(is_builtin_exception_class("TypeError"));
    assert!(is_builtin_exception_class("KeyError"));
    assert!(is_builtin_exception_class("IndexError"));
    assert!(is_builtin_exception_class("ZeroDivisionError"));
    assert!(is_builtin_exception_class("RuntimeError"));
}

#[test]
fn is_builtin_exception_class_rejects_unknown_names() {
    assert!(!is_builtin_exception_class("NotAnException"));
    assert!(!is_builtin_exception_class(""));
}

// -- #769 (Part 2 of #747) `optional_none_test` recognizer coverage --

#[test]
fn optional_none_test_recognizes_name_is_none() {
    let test = HirExpr::Compare {
        op: CmpOpKind::Is,
        left: Box::new(HirExpr::Name("x".to_string())),
        right: Box::new(HirExpr::NoneLiteral),
    };
    assert_eq!(optional_none_test(&test), Some(("x", NoneTestPolarity::Is)));
}

#[test]
fn optional_none_test_recognizes_none_is_name_reversed_operand_order() {
    let test = HirExpr::Compare {
        op: CmpOpKind::Is,
        left: Box::new(HirExpr::NoneLiteral),
        right: Box::new(HirExpr::Name("x".to_string())),
    };
    assert_eq!(optional_none_test(&test), Some(("x", NoneTestPolarity::Is)));
}

#[test]
fn optional_none_test_recognizes_name_is_not_none() {
    let test = HirExpr::Compare {
        op: CmpOpKind::IsNot,
        left: Box::new(HirExpr::Name("x".to_string())),
        right: Box::new(HirExpr::NoneLiteral),
    };
    assert_eq!(
        optional_none_test(&test),
        Some(("x", NoneTestPolarity::IsNot))
    );
}

#[test]
fn optional_none_test_reads_a_name_as_its_truthiness() {
    let test = HirExpr::Name("flag".to_string());
    assert_eq!(optional_none_test(&test), Some(("flag", NoneTestPolarity::IsNot)));
}

#[test]
fn optional_none_test_rejects_a_non_name_non_compare_test() {
    let test = HirExpr::IntLiteral(1);
    assert_eq!(optional_none_test(&test), None);
}

#[test]
fn optional_none_test_rejects_non_is_compare_op() {
    let test = HirExpr::Compare {
        op: CmpOpKind::Lt,
        left: Box::new(HirExpr::Name("x".to_string())),
        right: Box::new(HirExpr::NoneLiteral),
    };
    assert_eq!(optional_none_test(&test), None);
}

#[test]
fn optional_none_test_rejects_neither_side_a_bare_name() {
    let test = HirExpr::Compare {
        op: CmpOpKind::Is,
        left: Box::new(HirExpr::NoneLiteral),
        right: Box::new(HirExpr::NoneLiteral),
    };
    assert_eq!(optional_none_test(&test), None);
}

// Issue #769 (Part 2 of #747): `definitely_terminates` direct unit tests.
// `pycc_types::narrow`'s and `pycc_mir`'s own test suites only ever exercise
// this shared predicate indirectly (through a full `check_source`/`build`
// call), which happens to reach every *reachable* code path in it but never
// isolates each one -- these tests pin `definitely_terminates`'s own
// branches directly, covering every arm of both the outer `match` and the
// inner `&&`-chain's short-circuit structure.

#[test]
fn definitely_terminates_is_false_for_an_empty_body() {
    assert!(!definitely_terminates(&[]));
}

#[test]
fn definitely_terminates_is_true_when_the_last_statement_is_a_bare_return() {
    assert!(definitely_terminates(&[HirStmt::Return(None)]));
}

#[test]
fn definitely_terminates_is_false_when_the_last_statement_is_an_unrelated_stmt() {
    assert!(!definitely_terminates(&[HirStmt::ExprStmt(HirExpr::Name(
        "x".to_string()
    ))]));
}

#[test]
fn definitely_terminates_is_false_for_a_trailing_if_with_no_else() {
    // `!orelse.is_empty()` is the first conjunct of the `&&`-chain and is
    // false here, short-circuiting before either recursive call runs.
    let body = [HirStmt::If {
        test: HirExpr::Name("flag".to_string()),
        body: vec![HirStmt::Return(None)],
        orelse: vec![],
    }];
    assert!(!definitely_terminates(&body));
}

#[test]
fn definitely_terminates_is_false_when_the_if_body_does_not_terminate() {
    // `orelse` is non-empty (first conjunct true), but
    // `definitely_terminates(body)` (the second conjunct) is false.
    let body = [HirStmt::If {
        test: HirExpr::Name("flag".to_string()),
        body: vec![HirStmt::ExprStmt(HirExpr::Name("x".to_string()))],
        orelse: vec![HirStmt::Return(None)],
    }];
    assert!(!definitely_terminates(&body));
}

#[test]
fn definitely_terminates_is_false_when_the_if_orelse_does_not_terminate() {
    // Both `orelse.is_empty()` is false and `definitely_terminates(body)`
    // is true, but the third conjunct, `definitely_terminates(orelse)`, is
    // false.
    let body = [HirStmt::If {
        test: HirExpr::Name("flag".to_string()),
        body: vec![HirStmt::Return(None)],
        orelse: vec![HirStmt::ExprStmt(HirExpr::Name("x".to_string()))],
    }];
    assert!(!definitely_terminates(&body));
}

#[test]
fn definitely_terminates_is_true_when_both_if_branches_terminate() {
    // All three conjuncts true: `orelse` is non-empty, and both the body
    // and the orelse themselves recursively terminate.
    let body = [HirStmt::If {
        test: HirExpr::Name("flag".to_string()),
        body: vec![HirStmt::Return(None)],
        orelse: vec![HirStmt::Return(None)],
    }];
    assert!(definitely_terminates(&body));
}

// Issue #769 follow-up (D-068 re-review of #780, third round): direct unit
// tests on `killed_names`/`collect_killed_names` itself, pinning every
// match arm the way `definitely_terminates`'s own tests above pin every
// arm of that predicate. `pycc_types::narrow`'s and `pycc_mir`'s own test
// suites only ever exercise this shared function indirectly (through a
// full `check_source`/`build` call over a real `.py`-shaped program),
// which happens to reach most but not all of its recursive branches --
// these tests isolate each `HirStmt` variant's own contribution to the
// killed-name set directly.

/// #1291: a nested foreign import binds its name through
/// `HirModule::imports`, so it is not a kill.
#[test]
fn killed_names_ignores_a_foreign_import() {
    let body = vec![HirStmt::ForeignImport {
        bindings: vec![("colorsys".to_string(), "colorsys".to_string(), None)],
        span: Span::new(0, 15),
    }];
    assert!(killed_names(&body).is_empty());
}

#[test]
fn killed_names_is_empty_for_an_empty_body() {
    assert!(killed_names(&[]).is_empty());
}

#[test]
fn killed_names_includes_a_plain_assign_target() {
    let body = [HirStmt::Assign {
        target: "x".to_string(),
        value: HirExpr::NoneLiteral,
    }];
    let killed = killed_names(&body);
    assert_eq!(killed, HashSet::from(["x".to_string()]));
}

#[test]
fn killed_names_includes_a_valued_ann_assign_target() {
    let body = [HirStmt::AnnAssign {
        target: "x".to_string(),
        annotation: Ty::Int,
        value: Some(HirExpr::IntLiteral(1)),
        is_final: false,
    }];
    let killed = killed_names(&body);
    assert_eq!(killed, HashSet::from(["x".to_string()]));
}

#[test]
fn killed_names_excludes_a_value_less_ann_assign_target() {
    // A value-less `AnnAssign` (`x: int` with no `= value`) is a bare
    // declaration, not an assignment -- `check_assignment` is never
    // reached for it, so it must not count as a kill.
    let body = [HirStmt::AnnAssign {
        target: "x".to_string(),
        annotation: Ty::Int,
        value: None,
        is_final: false,
    }];
    assert!(killed_names(&body).is_empty());
}

#[test]
fn killed_names_includes_the_for_range_loop_variable_and_recurses_into_its_body() {
    let body = [HirStmt::ForRange {
        var: "i".to_string(),
        start: HirExpr::IntLiteral(0),
        stop: HirExpr::IntLiteral(3),
        step: HirExpr::IntLiteral(1),
        body: vec![HirStmt::Assign {
            target: "y".to_string(),
            value: HirExpr::NoneLiteral,
        }],
    }];
    let killed = killed_names(&body);
    assert_eq!(killed, HashSet::from(["i".to_string(), "y".to_string()]));
}

#[test]
fn killed_names_includes_the_for_list_loop_variable_and_recurses_into_its_body() {
    let body = [HirStmt::ForList {
        var: "elt".to_string(),
        list: "xs".to_string(),
        body: vec![HirStmt::Assign {
            target: "y".to_string(),
            value: HirExpr::NoneLiteral,
        }],
    }];
    let killed = killed_names(&body);
    assert_eq!(killed, HashSet::from(["elt".to_string(), "y".to_string()]));
}

#[test]
fn killed_names_includes_both_target_and_var_for_a_list_comprehension_assign() {
    let body = [HirStmt::ListCompAssign {
        target: "result".to_string(),
        var: "v".to_string(),
        iter: CompIter::Name("xs".to_string()),
        cond: None,
        elt: Box::new(HirExpr::Name("v".to_string())),
    }];
    let killed = killed_names(&body);
    assert_eq!(
        killed,
        HashSet::from(["result".to_string(), "v".to_string()])
    );
}

#[test]
fn killed_names_includes_both_target_and_var_for_a_set_comprehension_assign() {
    let body = [HirStmt::SetCompAssign {
        target: "result".to_string(),
        var: "v".to_string(),
        iter: CompIter::Name("xs".to_string()),
        cond: None,
        elt: Box::new(HirExpr::Name("v".to_string())),
    }];
    let killed = killed_names(&body);
    assert_eq!(
        killed,
        HashSet::from(["result".to_string(), "v".to_string()])
    );
}

#[test]
fn killed_names_includes_both_target_and_var_for_a_dict_comprehension_assign() {
    let body = [HirStmt::DictCompAssign {
        target: "result".to_string(),
        var: "v".to_string(),
        iter: CompIter::Name("xs".to_string()),
        cond: None,
        key: Box::new(HirExpr::Name("v".to_string())),
        value: Box::new(HirExpr::Name("v".to_string())),
    }];
    let killed = killed_names(&body);
    assert_eq!(
        killed,
        HashSet::from(["result".to_string(), "v".to_string()])
    );
}

#[test]
fn killed_names_recurses_into_both_an_ifs_body_and_orelse() {
    let body = [HirStmt::If {
        test: HirExpr::Name("flag".to_string()),
        body: vec![HirStmt::Assign {
            target: "a".to_string(),
            value: HirExpr::NoneLiteral,
        }],
        orelse: vec![HirStmt::Assign {
            target: "b".to_string(),
            value: HirExpr::NoneLiteral,
        }],
    }];
    let killed = killed_names(&body);
    assert_eq!(killed, HashSet::from(["a".to_string(), "b".to_string()]));
}

#[test]
fn killed_names_recurses_into_a_whiles_body() {
    let body = [HirStmt::While {
        test: HirExpr::Name("flag".to_string()),
        body: vec![HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::NoneLiteral,
        }],
    }];
    let killed = killed_names(&body);
    assert_eq!(killed, HashSet::from(["x".to_string()]));
}

#[test]
fn killed_names_recurses_into_every_match_case_body() {
    let body = [HirStmt::Match {
        subject: HirExpr::Name("flag".to_string()),
        cases: vec![
            HirMatchCase {
                pattern: HirPattern::Literal(HirExpr::IntLiteral(0)),
                guard: None,
                body: vec![HirStmt::Assign {
                    target: "a".to_string(),
                    value: HirExpr::NoneLiteral,
                }],
            },
            HirMatchCase {
                pattern: HirPattern::Wildcard,
                guard: None,
                body: vec![HirStmt::Assign {
                    target: "b".to_string(),
                    value: HirExpr::NoneLiteral,
                }],
            },
        ],
    }];
    let killed = killed_names(&body);
    assert_eq!(killed, HashSet::from(["a".to_string(), "b".to_string()]));
}

#[test]
fn killed_names_includes_a_match_cases_own_pattern_capture_names() {
    // D-068 re-review of #780 (fourth round, blocker finding 2): a case's
    // pattern can itself bind a bare name (`case x:`) exactly like an
    // `Assign` does -- `check_match` routes every pattern capture through
    // `check_assignment` (see `collect_killed_names`'s `Match` arm's own
    // doc comment). This must be visible even when the case body itself
    // kills nothing.
    let body = [HirStmt::Match {
        subject: HirExpr::Name("y".to_string()),
        cases: vec![HirMatchCase {
            pattern: HirPattern::Capture("x".to_string()),
            guard: None,
            body: vec![],
        }],
    }];
    let killed = killed_names(&body);
    assert_eq!(killed, HashSet::from(["x".to_string()]));
}

#[test]
fn killed_names_includes_capture_names_nested_inside_a_sequence_pattern() {
    let body = [HirStmt::Match {
        subject: HirExpr::Name("y".to_string()),
        cases: vec![HirMatchCase {
            pattern: HirPattern::SequenceStar(
                vec![HirPattern::Capture("a".to_string())],
                Some("rest".to_string()),
            ),
            guard: None,
            body: vec![],
        }],
    }];
    let killed = killed_names(&body);
    assert_eq!(killed, HashSet::from(["a".to_string(), "rest".to_string()]));
}

#[test]
fn killed_names_covers_every_pattern_kind_and_the_no_rest_branches() {
    // Exercises every `HirPattern` variant `collect_pattern_capture_names_as_killed`
    // matches on, including both arms of its `SequenceStar`/`Mapping`
    // `rest: Option<String>` branches (`Some`/`None`), mirroring
    // `pycc_types::collect_pattern_capture_names_covers_all_pattern_kinds`'s
    // own exhaustive coverage of its sibling function.
    let body = [HirStmt::Match {
        subject: HirExpr::Name("y".to_string()),
        cases: vec![
            HirMatchCase {
                pattern: HirPattern::Wildcard,
                guard: None,
                body: vec![],
            },
            HirMatchCase {
                pattern: HirPattern::Literal(HirExpr::IntLiteral(1)),
                guard: None,
                body: vec![],
            },
            HirMatchCase {
                pattern: HirPattern::Singleton(true),
                guard: None,
                body: vec![],
            },
            HirMatchCase {
                pattern: HirPattern::NoneSingleton,
                guard: None,
                body: vec![],
            },
            HirMatchCase {
                pattern: HirPattern::Sequence(vec![HirPattern::Capture("a".to_string())]),
                guard: None,
                body: vec![],
            },
            HirMatchCase {
                pattern: HirPattern::SequenceStar(vec![HirPattern::Capture("b".to_string())], None),
                guard: None,
                body: vec![],
            },
            HirMatchCase {
                pattern: HirPattern::Mapping(
                    vec![(
                        HirExpr::StringLiteral("k".to_string()),
                        HirPattern::Capture("c".to_string()),
                    )],
                    Some("mrest".to_string()),
                ),
                guard: None,
                body: vec![],
            },
            HirMatchCase {
                pattern: HirPattern::Mapping(vec![], None),
                guard: None,
                body: vec![],
            },
            HirMatchCase {
                pattern: HirPattern::Class {
                    class_name: "P".to_string(),
                    positional: vec![HirPattern::Capture("d".to_string())],
                    keyword: vec![("a".to_string(), HirPattern::Capture("e".to_string()))],
                },
                guard: None,
                body: vec![],
            },
            HirMatchCase {
                pattern: HirPattern::Or(vec![HirPattern::Capture("f".to_string())]),
                guard: None,
                body: vec![],
            },
            HirMatchCase {
                pattern: HirPattern::As(
                    Box::new(HirPattern::Capture("g".to_string())),
                    "h".to_string(),
                ),
                guard: None,
                body: vec![],
            },
        ],
    }];
    let killed = killed_names(&body);
    for expected in ["a", "b", "c", "mrest", "d", "e", "f", "g", "h"] {
        assert!(
            killed.contains(expected),
            "expected `killed_names` to include capture name `{expected}`, got {killed:?}"
        );
    }
}

#[test]
fn killed_names_recurses_into_every_part_of_a_try_statement() {
    let body = [HirStmt::Try {
        body: vec![HirStmt::Assign {
            target: "a".to_string(),
            value: HirExpr::NoneLiteral,
        }],
        handlers: vec![HirExceptHandler {
            exc_type: Some(vec!["ValueError".to_string()]),
            name: None,
            body: vec![HirStmt::Assign {
                target: "b".to_string(),
                value: HirExpr::NoneLiteral,
            }],
        }],
        orelse: vec![HirStmt::Assign {
            target: "c".to_string(),
            value: HirExpr::NoneLiteral,
        }],
        finalbody: vec![HirStmt::Assign {
            target: "d".to_string(),
            value: HirExpr::NoneLiteral,
        }],
    }];
    let killed = killed_names(&body);
    assert_eq!(
        killed,
        HashSet::from([
            "a".to_string(),
            "b".to_string(),
            "c".to_string(),
            "d".to_string(),
        ])
    );
}

#[test]
fn killed_names_includes_an_except_handlers_own_as_binding_name() {
    // D-068 re-review of #780 (fourth round, blocker finding 1's MIR/HIR
    // shared prescan half): `except ValueError as e:` binds `e` before the
    // handler body runs, exactly like an `Assign` -- this must be visible
    // even when the handler body itself kills nothing.
    let body = [HirStmt::Try {
        body: vec![],
        handlers: vec![HirExceptHandler {
            exc_type: Some(vec!["ValueError".to_string()]),
            name: Some("e".to_string()),
            body: vec![],
        }],
        orelse: vec![],
        finalbody: vec![],
    }];
    let killed = killed_names(&body);
    assert_eq!(killed, HashSet::from(["e".to_string()]));
}

#[test]
fn killed_names_recurses_into_every_part_of_a_try_star_statement() {
    // D-068 re-review of #780 (rebase onto #542's except* landing):
    // mirrors `killed_names_recurses_into_every_part_of_a_try_statement`
    // above for `TryStar` -- #542 landed independently of #769/#780's
    // narrowing overlay, so its own `HirStmt::TryStar` arm needs the same
    // coverage as the plain `Try` arm it mirrors.
    let body = [HirStmt::TryStar {
        body: vec![HirStmt::Assign {
            target: "a".to_string(),
            value: HirExpr::NoneLiteral,
        }],
        handlers: vec![HirExceptHandler {
            exc_type: Some(vec!["ValueError".to_string()]),
            name: None,
            body: vec![HirStmt::Assign {
                target: "b".to_string(),
                value: HirExpr::NoneLiteral,
            }],
        }],
        orelse: vec![HirStmt::Assign {
            target: "c".to_string(),
            value: HirExpr::NoneLiteral,
        }],
        finalbody: vec![HirStmt::Assign {
            target: "d".to_string(),
            value: HirExpr::NoneLiteral,
        }],
    }];
    let killed = killed_names(&body);
    assert_eq!(
        killed,
        HashSet::from([
            "a".to_string(),
            "b".to_string(),
            "c".to_string(),
            "d".to_string(),
        ])
    );
}

#[test]
fn killed_names_includes_a_try_star_handlers_own_as_binding_name() {
    // D-068 re-review of #780 (rebase onto #542's except* landing):
    // mirrors `killed_names_includes_an_except_handlers_own_as_binding_name`
    // above for `except* ValueError as e:` -- binds `e` to the caught
    // `ExceptionGroup` before the handler body runs, the same kill a plain
    // `except ... as e:` performs, and must be visible even when the
    // handler body itself kills nothing.
    let body = [HirStmt::TryStar {
        body: vec![],
        handlers: vec![HirExceptHandler {
            exc_type: Some(vec!["ValueError".to_string()]),
            name: Some("e".to_string()),
            body: vec![],
        }],
        orelse: vec![],
        finalbody: vec![],
    }];
    let killed = killed_names(&body);
    assert_eq!(killed, HashSet::from(["e".to_string()]));
}

#[test]
fn killed_names_ignores_statement_kinds_that_do_not_rebind_a_bare_name() {
    // `ExprStmt`, `DictSet`, `AttrSet`, `Return`, and `Raise` all route
    // through the catch-all no-op arm: none of them ever passes a bare
    // name through `check_assignment` (`DictSet`/`AttrSet` write through
    // a container/attribute slot, not a name binding).
    let body = [
        HirStmt::ExprStmt(HirExpr::Name("x".to_string())),
        HirStmt::DictSet {
            dict: "d".to_string(),
            key: HirExpr::IntLiteral(0),
            value: HirExpr::NoneLiteral,
        },
        HirStmt::AttrSet {
            base: HirExpr::Name("self".to_string()),
            attr: "field".to_string(),
            value: HirExpr::NoneLiteral,
        },
        HirStmt::Return(Some(HirExpr::IntLiteral(0))),
        HirStmt::Raise {
            exc: Some(HirExpr::Name("err".to_string())),
            cause: None,
        },
    ];
    assert!(killed_names(&body).is_empty());
}

// D-068 review of #780/#774's interaction (blocker finding 2): a bare walrus
// (`(x := ...)`) is a reassignment exactly like `HirStmt::Assign`, but it
// never appears as its own `HirStmt` variant -- it only ever shows up nested
// inside a bare `ExprStmt`'s expression or an `If`/`While` statement's
// `test`. Before this fix, `collect_killed_names` never inspected either of
// those two expression positions, so a walrus-only kill was invisible to the
// prescan.

#[test]
fn killed_names_includes_a_walrus_target_inside_a_bare_expr_stmt() {
    let body = [HirStmt::ExprStmt(HirExpr::NamedExpr {
        name: "x".to_string(),
        value: Box::new(HirExpr::NoneLiteral),
    })];
    let killed = killed_names(&body);
    assert_eq!(killed, HashSet::from(["x".to_string()]));
}

#[test]
fn killed_names_includes_a_walrus_target_inside_an_ifs_test() {
    let body = [HirStmt::If {
        test: HirExpr::NamedExpr {
            name: "x".to_string(),
            value: Box::new(HirExpr::NoneLiteral),
        },
        body: vec![],
        orelse: vec![],
    }];
    let killed = killed_names(&body);
    assert_eq!(killed, HashSet::from(["x".to_string()]));
}

#[test]
fn killed_names_includes_a_walrus_target_inside_a_whiles_test() {
    let body = [HirStmt::While {
        test: HirExpr::NamedExpr {
            name: "x".to_string(),
            value: Box::new(HirExpr::NoneLiteral),
        },
        body: vec![],
    }];
    let killed = killed_names(&body);
    assert_eq!(killed, HashSet::from(["x".to_string()]));
}

#[test]
fn killed_names_ignores_an_expr_stmt_with_no_embedded_walrus() {
    // Regresses the pre-fix behavior for the common case: a bare `ExprStmt`
    // that contains no `NamedExpr` anywhere still contributes nothing.
    let body = [HirStmt::ExprStmt(HirExpr::Call {
        callee: "print".to_string(),
        args: vec![HirExpr::Name("x".to_string())],
    })];
    assert!(killed_names(&body).is_empty());
}

/// Exercises every remaining arm of `collect_named_expr_targets_in_expr`
/// (the walrus-in-expression walker `collect_killed_names` now calls for
/// `ExprStmt`/`If`/`While`) in one deeply nested expression, pinning D-014's
/// 100% line/region coverage for a function with no wildcard arm. Each
/// non-walrus arm nests a `NamedExpr` for a distinct name one level inside
/// it, so every arm both recurses correctly and the walk still terminates
/// through the plain-literal/`Name`/`ListPop`/`Super` no-op arms at the
/// leaves.
#[test]
fn killed_names_finds_a_walrus_nested_inside_every_expression_kind() {
    fn walrus(name: &str) -> HirExpr {
        HirExpr::NamedExpr {
            name: name.to_string(),
            value: Box::new(HirExpr::NoneLiteral),
        }
    }

    let test = HirExpr::Call {
        callee: "f".to_string(),
        args: vec![
            walrus("call_arg"),
            HirExpr::BinOp {
                op: BinOpKind::Add,
                left: Box::new(walrus("binop_left")),
                right: Box::new(walrus("binop_right")),
            },
            HirExpr::Compare {
                op: CmpOpKind::Eq,
                left: Box::new(walrus("cmp_left")),
                right: Box::new(walrus("cmp_right")),
            },
            HirExpr::UnaryOp {
                op: UnaryOpKind::USub,
                operand: Box::new(walrus("unary")),
            },
            HirExpr::FString(vec![
                FStringPart::Literal("lit".to_string()),
                FStringPart::Interpolation(Box::new(walrus("fstring"))),
            ]),
            HirExpr::ListLiteral(vec![walrus("list_elt")]),
            HirExpr::SetLiteral(vec![walrus("set_elt")]),
            HirExpr::TupleLiteral(vec![walrus("tuple_elt")]),
            HirExpr::ObjectList(vec![walrus("object_list_elt")]),
            HirExpr::Subscript {
                base: Box::new(walrus("subscript_base")),
                index: Box::new(walrus("subscript_index")),
            },
            HirExpr::Slice {
                base: Box::new(walrus("slice_base")),
                start: Some(Box::new(walrus("slice_start"))),
                stop: Some(Box::new(walrus("slice_stop"))),
                step: Some(Box::new(walrus("slice_step"))),
            },
            HirExpr::ListAppend {
                list: crate::ContainerReceiver::Name("xs".to_string()),
                value: Box::new(walrus("list_append")),
            },
            HirExpr::SetAdd {
                set: "s".to_string(),
                value: Box::new(walrus("set_add")),
            },
            HirExpr::DictLiteral(vec![(walrus("dict_key"), walrus("dict_value"))]),
            HirExpr::DictGetOrDefault {
                dict: crate::ContainerReceiver::Name("d".to_string()),
                key: Box::new(walrus("dict_get_key")),
                default: Box::new(walrus("dict_get_default")),
            },
            HirExpr::AttrGet {
                base: Box::new(walrus("attr_get_base")),
                attr: "field".to_string(),
            },
            HirExpr::MethodCall {
                base: Box::new(walrus("method_call_base")),
                method: "m".to_string(),
                args: vec![walrus("method_call_arg")],
            },
            HirExpr::GenericClassInstantiate {
                class: "Box".to_string(),
                type_arg: Ty::Int,
                args: vec![walrus("generic_instantiate_arg")],
            },
            // Leaves that terminate the walk without themselves nesting a
            // walrus, exercising the no-op arm.
            HirExpr::IntLiteral(0),
            HirExpr::FloatLiteral(0.0),
            HirExpr::BoolLiteral(true),
            HirExpr::StringLiteral("s".to_string()),
            HirExpr::NoneLiteral,
            HirExpr::Name("plain_name".to_string()),
            HirExpr::ListPop {
                list: crate::ContainerReceiver::Name("xs".to_string()),
            },
            HirExpr::Super,
        ],
    };

    let body = [HirStmt::While { test, body: vec![] }];
    let killed = killed_names(&body);
    let expected: HashSet<String> = [
        "call_arg",
        "binop_left",
        "binop_right",
        "cmp_left",
        "cmp_right",
        "unary",
        "fstring",
        "list_elt",
        "set_elt",
        "tuple_elt",
        "object_list_elt",
        "subscript_base",
        "subscript_index",
        "slice_base",
        "slice_start",
        "slice_stop",
        "slice_step",
        "list_append",
        "set_add",
        "dict_key",
        "dict_value",
        "dict_get_key",
        "dict_get_default",
        "attr_get_base",
        "method_call_base",
        "method_call_arg",
        "generic_instantiate_arg",
    ]
    .into_iter()
    .map(str::to_string)
    .collect();
    assert_eq!(killed, expected);
}
