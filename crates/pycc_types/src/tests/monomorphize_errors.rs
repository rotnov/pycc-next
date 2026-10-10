//! Monomorphization error propagation and defensive paths.

use super::*;

#[test]
fn monomorphize_propagates_an_instantiation_error_from_inside_a_function_body() {
    // Bypasses ordinary validation deliberately, same rationale as the
    // `ForList`/`CompIter` defensive-fallback tests above: a real
    // program with this wrong-arity call would already be rejected by
    // `check`/`check_and_resolve` before `monomorphize` ever ran. This
    // exercises the `?` propagation out of `rewrite_generic_calls_in_stmt`
    // inside `monomorphize`'s own `HirItem::Function` branch.
    let param = Ty::Param(Box::new("T".to_string()));
    let identity = generic_identity_fn(param.clone(), param);
    let f = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![HirStmt::ExprStmt(HirExpr::Call {
            callee: "identity".to_string(),
            args: vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)],
        })],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![identity, f],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert_eq!(monomorphize(&hir).unwrap_err().code, "T0021");
}

#[test]
fn monomorphize_propagates_an_instantiation_error_from_a_top_level_statement() {
    // Same rationale as the function-body variant above, for
    // `monomorphize`'s `HirItem::TopLevelStmt` branch.
    let param = Ty::Param(Box::new("T".to_string()));
    let identity = generic_identity_fn(param.clone(), param);
    let top = HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
        callee: "identity".to_string(),
        args: vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)],
    }));
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![identity, top],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert_eq!(monomorphize(&hir).unwrap_err().code, "T0021");
}

#[test]
fn monomorphize_propagates_an_instantiation_error_from_a_dict_comp_assign_cond() {
    // Bypasses ordinary validation deliberately, same rationale as the
    // other direct `monomorphize` tests in this group -- exercises the
    // `?` propagation out of `rewrite_generic_calls_in_stmt`'s
    // `DictCompAssign` arm specifically for its `cond` sub-expression.
    let param = Ty::Param(Box::new("T".to_string()));
    let identity = generic_identity_fn(param.clone(), param);
    let f = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![
            HirStmt::Assign {
                target: "xs".to_string(),
                value: HirExpr::ListLiteral(vec![HirExpr::IntLiteral(1)]),
            },
            HirStmt::DictCompAssign {
                target: "zs".to_string(),
                var: "0comp_v".to_string(),
                iter: CompIter::Name("xs".to_string()),
                cond: Some(Box::new(HirExpr::Call {
                    callee: "identity".to_string(),
                    args: vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)],
                })),
                key: Box::new(HirExpr::StringLiteral("k".to_string())),
                value: Box::new(HirExpr::IntLiteral(1)),
            },
        ],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![identity, f],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert_eq!(monomorphize(&hir).unwrap_err().code, "T0021");
}

/// A generic call that always fails `instantiate_generic_call` with
/// `T0021` (wrong arity) -- used throughout the error-propagation group
/// below to prove every structural recursion position in
/// `rewrite_generic_calls_in_expr`/`rewrite_generic_calls_in_stmt`/
/// `rewrite_comp_iter` actually propagates a nested failure instead of
/// silently swallowing it.
fn bad_generic_call() -> HirExpr {
    HirExpr::Call {
        callee: "identity".to_string(),
        args: vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)],
    }
}

fn assert_monomorphize_propagates_error(body: Vec<HirStmt>) {
    let param = Ty::Param(Box::new("T".to_string()));
    let identity = generic_identity_fn(param.clone(), param);
    let f = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body,
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![identity, f],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert_eq!(monomorphize(&hir).unwrap_err().code, "T0021");
}

/// `rewrite_generic_calls_in_stmt`'s `TryStar` arm only runs at all when the
/// module contains a generic function/class/protocol elsewhere (`monomorphize`
/// short-circuits to a plain clone otherwise, per its own leading gate) --
/// unlike the `assert_monomorphize_propagates_error` group below, this
/// exercises the *success* path, including the `handler.name` binding to
/// `ExceptionGroup` (Part 3 of #382, #542, PEP 654), which no error-only
/// fixture reaches since every one of those uses `name: None`.
#[test]
fn monomorphize_rewrites_a_try_star_handler_binding_when_the_module_has_a_generic_function() {
    let param = Ty::Param(Box::new("T".to_string()));
    let identity = generic_identity_fn(param.clone(), param);
    let f = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![HirStmt::TryStar {
            body: vec![HirStmt::ExprStmt(HirExpr::IntLiteral(1))],
            handlers: vec![pycc_hir::HirExceptHandler {
                exc_type: Some(vec!["ValueError".to_string()]),
                name: Some("e".to_string()),
                body: vec![HirStmt::ExprStmt(HirExpr::IntLiteral(2))],
            }],
            orelse: vec![HirStmt::ExprStmt(HirExpr::IntLiteral(3))],
            finalbody: vec![HirStmt::ExprStmt(HirExpr::IntLiteral(4))],
        }],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![identity, f],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    monomorphize(&hir).expect(
        "a module with a generic function and an unrelated try*/except* elsewhere \
         should monomorphize successfully",
    );
}

// PR-13 Task 3: every structural recursion position in
// `rewrite_generic_calls_in_expr`/`rewrite_generic_calls_in_stmt`/
// `rewrite_comp_iter` needs its own covering error-propagation case for
// the 100%-region D-014 gate -- grouped into a few multi-assertion
// tests (rather than one `#[test]` per position) since each case is a
// one-line `bad_generic_call()` substitution, not independent behavior
// worth its own test name.

#[test]
fn rewrite_generic_calls_in_expr_propagates_errors_from_every_recursive_position() {
    // `Call` args (line ~3213) and the `arg_tys` collection step (~3219,
    // via an unresolved argument name rather than `instantiate_generic_call`
    // itself).
    assert_monomorphize_propagates_error(vec![HirStmt::ExprStmt(HirExpr::Call {
        callee: "identity".to_string(),
        args: vec![bad_generic_call()],
    })]);
    assert_monomorphize_propagates_error(vec![HirStmt::ExprStmt(HirExpr::Call {
        callee: "identity".to_string(),
        args: vec![HirExpr::Name("undefined".to_string())],
    })]);
    // `BinOp`/`Compare`.
    assert_monomorphize_propagates_error(vec![HirStmt::ExprStmt(HirExpr::BinOp {
        op: BinOpKind::Add,
        left: Box::new(bad_generic_call()),
        right: Box::new(HirExpr::IntLiteral(1)),
    })]);
    // `FString`.
    assert_monomorphize_propagates_error(vec![HirStmt::ExprStmt(HirExpr::FString(vec![
        FStringPart::Interpolation(Box::new(bad_generic_call())),
    ]))]);
    // `ListLiteral`/`SetLiteral`/`TupleLiteral`.
    assert_monomorphize_propagates_error(vec![HirStmt::ExprStmt(HirExpr::ListLiteral(vec![
        bad_generic_call(),
    ]))]);
    // `DictLiteral`.
    assert_monomorphize_propagates_error(vec![HirStmt::ExprStmt(HirExpr::DictLiteral(vec![(
        HirExpr::StringLiteral("a".to_string()),
        bad_generic_call(),
    )]))]);
    // `Subscript`.
    assert_monomorphize_propagates_error(vec![HirStmt::ExprStmt(HirExpr::Subscript {
        base: Box::new(bad_generic_call()),
        index: Box::new(HirExpr::IntLiteral(0)),
    })]);
    // `Slice`.
    assert_monomorphize_propagates_error(vec![HirStmt::ExprStmt(HirExpr::Slice {
        base: Box::new(bad_generic_call()),
        start: None,
        stop: None,
        step: None,
    })]);
    // `ListAppend`/`SetAdd`.
    assert_monomorphize_propagates_error(vec![HirStmt::ExprStmt(HirExpr::ListAppend {
        list: pycc_hir::ContainerReceiver::Name("xs".to_string()),
        value: Box::new(bad_generic_call()),
    })]);
    // `DictGetOrDefault`.
    assert_monomorphize_propagates_error(vec![HirStmt::ExprStmt(HirExpr::DictGetOrDefault {
        dict: pycc_hir::ContainerReceiver::Name("d".to_string()),
        key: Box::new(bad_generic_call()),
        default: Box::new(HirExpr::IntLiteral(0)),
    })]);
    // `AttrGet`'s own `base` (D-154).
    assert_monomorphize_propagates_error(vec![HirStmt::ExprStmt(HirExpr::AttrGet {
        base: Box::new(bad_generic_call()),
        attr: "x".to_string(),
    })]);
    // `MethodCall`'s own `base` and its per-argument loop (D-154).
    assert_monomorphize_propagates_error(vec![HirStmt::ExprStmt(HirExpr::MethodCall {
        base: Box::new(bad_generic_call()),
        method: "m".to_string(),
        args: vec![],
    })]);
    assert_monomorphize_propagates_error(vec![HirStmt::ExprStmt(HirExpr::MethodCall {
        base: Box::new(HirExpr::IntLiteral(0)),
        method: "m".to_string(),
        args: vec![bad_generic_call()],
    })]);
}

#[test]
fn rewrite_generic_calls_in_stmt_propagates_errors_from_every_recursive_position() {
    // `Assign`.
    assert_monomorphize_propagates_error(vec![HirStmt::Assign {
        target: "y".to_string(),
        value: bad_generic_call(),
    }]);
    // `AnnAssign` (`Some(value)`).
    assert_monomorphize_propagates_error(vec![HirStmt::AnnAssign {
        is_final: false,
        target: "y".to_string(),
        annotation: Ty::Int,
        value: Some(bad_generic_call()),
    }]);
    // `If`'s own `test`, `body`, and `orelse`.
    assert_monomorphize_propagates_error(vec![HirStmt::If {
        test: bad_generic_call(),
        body: vec![],
        orelse: vec![],
    }]);
    assert_monomorphize_propagates_error(vec![HirStmt::If {
        test: HirExpr::BoolLiteral(true),
        body: vec![HirStmt::ExprStmt(bad_generic_call())],
        orelse: vec![],
    }]);
    assert_monomorphize_propagates_error(vec![HirStmt::If {
        test: HirExpr::BoolLiteral(true),
        body: vec![],
        orelse: vec![HirStmt::ExprStmt(bad_generic_call())],
    }]);
    // `While`'s own `test` and `body`.
    assert_monomorphize_propagates_error(vec![HirStmt::While {
        test: bad_generic_call(),
        body: vec![],
    }]);
    assert_monomorphize_propagates_error(vec![HirStmt::While {
        test: HirExpr::BoolLiteral(true),
        body: vec![HirStmt::ExprStmt(bad_generic_call())],
    }]);
    // `ForRange`'s own bounds and `body`.
    assert_monomorphize_propagates_error(vec![HirStmt::ForRange {
        var: "i".to_string(),
        start: bad_generic_call(),
        stop: HirExpr::IntLiteral(1),
        step: HirExpr::IntLiteral(1),
        body: vec![],
    }]);
    assert_monomorphize_propagates_error(vec![HirStmt::ForRange {
        var: "i".to_string(),
        start: HirExpr::IntLiteral(0),
        stop: HirExpr::IntLiteral(1),
        step: HirExpr::IntLiteral(1),
        body: vec![HirStmt::ExprStmt(bad_generic_call())],
    }]);
    // `ForList`'s own `body`.
    assert_monomorphize_propagates_error(vec![
        HirStmt::Assign {
            target: "xs".to_string(),
            value: HirExpr::ListLiteral(vec![HirExpr::IntLiteral(1)]),
        },
        HirStmt::ForList {
            var: "x".to_string(),
            list: "xs".to_string(),
            body: vec![HirStmt::ExprStmt(bad_generic_call())],
        },
    ]);
    // `DictSet`'s own `key`/`value`.
    assert_monomorphize_propagates_error(vec![
        HirStmt::Assign {
            target: "d".to_string(),
            value: HirExpr::DictLiteral(vec![(
                HirExpr::StringLiteral("a".to_string()),
                HirExpr::IntLiteral(1),
            )]),
        },
        HirStmt::DictSet {
            dict: "d".to_string(),
            key: bad_generic_call(),
            value: HirExpr::IntLiteral(1),
        },
    ]);
    // `ListCompAssign`/`SetCompAssign`/`DictCompAssign`'s own
    // `rewrite_comp_iter` propagation (a bad `CompIter::Range` bound)
    // and their own `cond`/`elt`/`key`/`value` propagation.
    assert_monomorphize_propagates_error(vec![HirStmt::ListCompAssign {
        target: "xs".to_string(),
        var: "v".to_string(),
        iter: CompIter::Range {
            start: bad_generic_call(),
            stop: HirExpr::IntLiteral(1),
            step: HirExpr::IntLiteral(1),
        },
        cond: None,
        elt: Box::new(HirExpr::IntLiteral(1)),
    }]);
    assert_monomorphize_propagates_error(vec![HirStmt::ListCompAssign {
        target: "xs".to_string(),
        var: "v".to_string(),
        iter: CompIter::Range {
            start: HirExpr::IntLiteral(0),
            stop: HirExpr::IntLiteral(1),
            step: HirExpr::IntLiteral(1),
        },
        cond: None,
        elt: Box::new(bad_generic_call()),
    }]);
    assert_monomorphize_propagates_error(vec![HirStmt::SetCompAssign {
        target: "xs".to_string(),
        var: "v".to_string(),
        iter: CompIter::Range {
            start: bad_generic_call(),
            stop: HirExpr::IntLiteral(1),
            step: HirExpr::IntLiteral(1),
        },
        cond: None,
        elt: Box::new(HirExpr::IntLiteral(1)),
    }]);
    assert_monomorphize_propagates_error(vec![HirStmt::SetCompAssign {
        target: "xs".to_string(),
        var: "v".to_string(),
        iter: CompIter::Range {
            start: HirExpr::IntLiteral(0),
            stop: HirExpr::IntLiteral(1),
            step: HirExpr::IntLiteral(1),
        },
        cond: None,
        elt: Box::new(bad_generic_call()),
    }]);
    assert_monomorphize_propagates_error(vec![HirStmt::DictCompAssign {
        target: "xs".to_string(),
        var: "v".to_string(),
        iter: CompIter::Range {
            start: bad_generic_call(),
            stop: HirExpr::IntLiteral(1),
            step: HirExpr::IntLiteral(1),
        },
        cond: None,
        key: Box::new(HirExpr::StringLiteral("k".to_string())),
        value: Box::new(HirExpr::IntLiteral(1)),
    }]);
    assert_monomorphize_propagates_error(vec![HirStmt::DictCompAssign {
        target: "xs".to_string(),
        var: "v".to_string(),
        iter: CompIter::Range {
            start: HirExpr::IntLiteral(0),
            stop: HirExpr::IntLiteral(1),
            step: HirExpr::IntLiteral(1),
        },
        cond: None,
        key: Box::new(bad_generic_call()),
        value: Box::new(HirExpr::IntLiteral(1)),
    }]);
    // `Return(Some(value))`.
    assert_monomorphize_propagates_error(vec![HirStmt::Return(Some(bad_generic_call()))]);
    // `AttrSet`'s own `base`/`value` (D-154) -- both share the same
    // `for sub in [base, value]` loop, so a single erroring `base` case
    // exercises the loop's own `?`.
    assert_monomorphize_propagates_error(vec![HirStmt::AttrSet {
        base: bad_generic_call(),
        attr: "x".to_string(),
        value: HirExpr::IntLiteral(0),
    }]);
    assert_monomorphize_propagates_error(vec![HirStmt::AttrSet {
        base: HirExpr::IntLiteral(0),
        attr: "x".to_string(),
        value: bad_generic_call(),
    }]);
    // `Match`'s own `subject`, `guard`, and `body` (#381).
    assert_monomorphize_propagates_error(vec![HirStmt::Match {
        subject: bad_generic_call(),
        cases: vec![HirMatchCase {
            pattern: HirPattern::Wildcard,
            guard: None,
            body: vec![],
        }],
    }]);
    assert_monomorphize_propagates_error(vec![HirStmt::Match {
        subject: HirExpr::IntLiteral(0),
        cases: vec![HirMatchCase {
            pattern: HirPattern::Wildcard,
            guard: Some(bad_generic_call()),
            body: vec![],
        }],
    }]);
    assert_monomorphize_propagates_error(vec![HirStmt::Match {
        subject: HirExpr::IntLiteral(0),
        cases: vec![HirMatchCase {
            pattern: HirPattern::Wildcard,
            guard: None,
            body: vec![HirStmt::ExprStmt(bad_generic_call())],
        }],
    }]);
    // `Try`'s own body, handler body, else body, and finally body (#382).
    assert_monomorphize_propagates_error(vec![HirStmt::Try {
        body: vec![HirStmt::ExprStmt(bad_generic_call())],
        handlers: vec![],
        orelse: vec![],
        finalbody: vec![],
    }]);
    assert_monomorphize_propagates_error(vec![HirStmt::Try {
        body: vec![],
        handlers: vec![pycc_hir::HirExceptHandler {
            exc_type: Some(vec!["ValueError".to_string()]),
            name: None,
            body: vec![HirStmt::ExprStmt(bad_generic_call())],
        }],
        orelse: vec![],
        finalbody: vec![],
    }]);
    assert_monomorphize_propagates_error(vec![HirStmt::Try {
        body: vec![],
        handlers: vec![],
        orelse: vec![HirStmt::ExprStmt(bad_generic_call())],
        finalbody: vec![],
    }]);
    assert_monomorphize_propagates_error(vec![HirStmt::Try {
        body: vec![],
        handlers: vec![],
        orelse: vec![],
        finalbody: vec![HirStmt::ExprStmt(bad_generic_call())],
    }]);
    // `TryStar`'s own body, handler body, else body, and finally body (Part
    // 3 of #382, #542, PEP 654) -- mirrors the four `Try` cases directly
    // above, since `collect_block_constraints` propagates each of the four
    // blocks' errors with its own `?` rather than sharing `Try`'s.
    assert_monomorphize_propagates_error(vec![HirStmt::TryStar {
        body: vec![HirStmt::ExprStmt(bad_generic_call())],
        handlers: vec![],
        orelse: vec![],
        finalbody: vec![],
    }]);
    assert_monomorphize_propagates_error(vec![HirStmt::TryStar {
        body: vec![],
        handlers: vec![pycc_hir::HirExceptHandler {
            exc_type: Some(vec!["ValueError".to_string()]),
            name: None,
            body: vec![HirStmt::ExprStmt(bad_generic_call())],
        }],
        orelse: vec![],
        finalbody: vec![],
    }]);
    assert_monomorphize_propagates_error(vec![HirStmt::TryStar {
        body: vec![],
        handlers: vec![],
        orelse: vec![HirStmt::ExprStmt(bad_generic_call())],
        finalbody: vec![],
    }]);
    assert_monomorphize_propagates_error(vec![HirStmt::TryStar {
        body: vec![],
        handlers: vec![],
        orelse: vec![],
        finalbody: vec![HirStmt::ExprStmt(bad_generic_call())],
    }]);
    // `Raise`'s own exc and cause (#382).
    assert_monomorphize_propagates_error(vec![HirStmt::Raise {
        exc: Some(bad_generic_call()),
        cause: None,
    }]);
    assert_monomorphize_propagates_error(vec![HirStmt::Raise {
        exc: None,
        cause: Some(bad_generic_call()),
    }]);
    // The builtin exception constructor is deliberately not re-checked
    // during monomorphization, but a bad generic call nested in its
    // message still has to propagate its instantiation diagnostic.
    assert_monomorphize_propagates_error(vec![HirStmt::Raise {
        exc: Some(HirExpr::Call {
            callee: "ValueError".to_string(),
            args: vec![bad_generic_call()],
        }),
        cause: None,
    }]);
}

#[test]
fn rewrite_generic_calls_in_stmt_raise_with_no_generic_calls_succeeds() {
    // Cover the `Ok(())` return in `rewrite_generic_calls_in_stmt`'s
    // `Raise` arm when both `exc` and `cause` are `None` (bare raise).
    // The module includes a generic function so `monomorphize` does not
    // take its early-return path.
    let param = Ty::Param(Box::new("T".to_string()));
    let identity = generic_identity_fn(param.clone(), param);
    let f = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![HirStmt::Raise {
            exc: None,
            cause: None,
        }],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![identity, f],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    monomorphize(&hir).expect("monomorphize should succeed for bare raise");
}

#[test]
fn rewrite_generic_calls_in_stmt_raise_with_non_generic_cause_succeeds() {
    // Cover the `if let Some(cause)` block's normal completion and the
    // `Ok(())` return in `rewrite_generic_calls_in_stmt`'s `Raise` arm
    // when `cause` is `Some` with a non-generic expression (IntLiteral).
    // `rewrite_generic_calls_in_expr` succeeds for `IntLiteral`, so the
    // block completes normally and `Ok(())` is reached.
    let param = Ty::Param(Box::new("T".to_string()));
    let identity = generic_identity_fn(param.clone(), param);
    let f = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![HirStmt::Raise {
            exc: Some(HirExpr::IntLiteral(0)),
            cause: Some(HirExpr::IntLiteral(1)),
        }],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![identity, f],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    monomorphize(&hir).expect("monomorphize should succeed for raise with literal cause");
}

#[test]
fn monomorphize_defends_a_for_list_over_a_non_container_binding_defensively() {
    // Bypasses ordinary validation deliberately -- `for x in xs` where
    // `xs: int` would already be rejected by `check`/`check_and_resolve`
    // before `monomorphize` ever ran. Calling `monomorphize` directly
    // exercises its own defensive fallback for an already-invalid
    // binding, matching this file's existing "defense in depth" test
    // convention (see e.g. `instantiate_generic_call_rejects_two_distinct_type_parameters`).
    let param = Ty::Param(Box::new("T".to_string()));
    let identity = generic_identity_fn(param.clone(), param);
    let f = HirItem::Function {
        name: "f".to_string(),
        params: vec![("xs".to_string(), Ty::Int)],
        return_ty: Ty::None,
        body: vec![HirStmt::ForList {
            var: "x".to_string(),
            list: "xs".to_string(),
            body: vec![HirStmt::ExprStmt(HirExpr::Call {
                callee: "identity".to_string(),
                args: vec![HirExpr::IntLiteral(1)],
            })],
        }],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![identity, f],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let resolved = monomorphize(&hir).unwrap();
    assert_eq!(count_function(&resolved, "0gen_identity__T_int"), 1);
}

#[test]
fn monomorphize_defends_a_comprehension_over_a_non_container_binding_defensively() {
    // Same defensive-fallback rationale as the `ForList` test above,
    // for `rewrite_comp_iter`'s own `CompIter::Name` fallback arm.
    let param = Ty::Param(Box::new("T".to_string()));
    let identity = generic_identity_fn(param.clone(), param);
    let f = HirItem::Function {
        name: "f".to_string(),
        params: vec![("xs".to_string(), Ty::Int)],
        return_ty: Ty::None,
        body: vec![HirStmt::ListCompAssign {
            target: "ys".to_string(),
            var: "0comp_v".to_string(),
            iter: CompIter::Name("xs".to_string()),
            cond: None,
            elt: Box::new(HirExpr::Call {
                callee: "identity".to_string(),
                args: vec![HirExpr::IntLiteral(1)],
            }),
        }],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![identity, f],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let resolved = monomorphize(&hir).unwrap();
    assert_eq!(count_function(&resolved, "0gen_identity__T_int"), 1);
}
