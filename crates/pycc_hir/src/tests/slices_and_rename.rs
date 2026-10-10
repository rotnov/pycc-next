//! Slice lowering and `rename_name_in_expr`.

use super::*;

// -- Task 6 (D-118): list[int] slicing frontend HIR forms ------------

#[test]
fn lowers_a_slice_expression_with_both_bounds_present() {
    let module = pycc_parser_test_helper::parse("xs = [1, 2, 3]\ny = xs[1:3]\n");
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items[1],
        HirItem::TopLevelStmt(HirStmt::Assign {
            target: "y".to_string(),
            value: HirExpr::Slice {
                base: Box::new(HirExpr::Name("xs".to_string())),
                start: Some(Box::new(HirExpr::IntLiteral(1))),
                stop: Some(Box::new(HirExpr::IntLiteral(3))),
                step: None,
            },
        })
    );
}

#[test]
fn lowers_a_slice_expression_with_only_the_stop_bound() {
    let module = pycc_parser_test_helper::parse("xs = [1, 2, 3]\ny = xs[:3]\n");
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items[1],
        HirItem::TopLevelStmt(HirStmt::Assign {
            target: "y".to_string(),
            value: HirExpr::Slice {
                base: Box::new(HirExpr::Name("xs".to_string())),
                start: None,
                stop: Some(Box::new(HirExpr::IntLiteral(3))),
                step: None,
            },
        })
    );
}

#[test]
fn lowers_a_slice_expression_with_only_the_start_bound() {
    let module = pycc_parser_test_helper::parse("xs = [1, 2, 3]\ny = xs[2:]\n");
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items[1],
        HirItem::TopLevelStmt(HirStmt::Assign {
            target: "y".to_string(),
            value: HirExpr::Slice {
                base: Box::new(HirExpr::Name("xs".to_string())),
                start: Some(Box::new(HirExpr::IntLiteral(2))),
                stop: None,
                step: None,
            },
        })
    );
}

#[test]
fn lowers_a_slice_expression_with_all_bounds_omitted() {
    let module = pycc_parser_test_helper::parse("xs = [1, 2, 3]\ny = xs[:]\n");
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items[1],
        HirItem::TopLevelStmt(HirStmt::Assign {
            target: "y".to_string(),
            value: HirExpr::Slice {
                base: Box::new(HirExpr::Name("xs".to_string())),
                start: None,
                stop: None,
                step: None,
            },
        })
    );
}

#[test]
fn lowers_a_slice_expression_with_a_step() {
    let module = pycc_parser_test_helper::parse("xs = [1, 2, 3]\ny = xs[::2]\n");
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items[1],
        HirItem::TopLevelStmt(HirStmt::Assign {
            target: "y".to_string(),
            value: HirExpr::Slice {
                base: Box::new(HirExpr::Name("xs".to_string())),
                start: None,
                stop: None,
                step: Some(Box::new(HirExpr::IntLiteral(2))),
            },
        })
    );
}

#[test]
fn an_ordinary_single_expression_subscript_still_lowers_to_subscript_not_slice() {
    // Regression pin for Step 2's new `match sub.slice.as_ref() { ... }`
    // dispatch (D-118): a colon-free subscript must keep taking the `_`
    // arm and producing the pre-existing `HirExpr::Subscript` shape,
    // unaffected by the new `Expr::Slice` arm added alongside it.
    // (Already exercised incidentally by `lowers_a_read_subscript`
    // above; pinned again here, by name, as the dedicated regression
    // test for this specific change.)
    let module = pycc_parser_test_helper::parse("xs = [1, 2, 3]\ny = xs[0]\n");
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items[1],
        HirItem::TopLevelStmt(HirStmt::Assign {
            target: "y".to_string(),
            value: HirExpr::Subscript {
                base: Box::new(HirExpr::Name("xs".to_string())),
                index: Box::new(HirExpr::IntLiteral(0)),
            },
        })
    );
}

#[test]
fn a_slice_expressions_base_and_bounds_are_recursively_lowered() {
    // `f()`/`g()` stand in for "some already-lowerable non-literal
    // shape" -- confirms `base`/`start`/`stop` are each passed through
    // the real `lower_expr` recursively, not merely accepted as raw
    // literals.
    let module = pycc_parser_test_helper::parse("xs = [1, 2, 3]\ny = xs[f():g()]\n");
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items[1],
        HirItem::TopLevelStmt(HirStmt::Assign {
            target: "y".to_string(),
            value: HirExpr::Slice {
                base: Box::new(HirExpr::Name("xs".to_string())),
                start: Some(Box::new(HirExpr::Call {
                    callee: "f".to_string(),
                    args: vec![],
                })),
                stop: Some(Box::new(HirExpr::Call {
                    callee: "g".to_string(),
                    args: vec![],
                })),
                step: None,
            },
        })
    );
}

#[test]
fn a_slice_with_an_unsupported_base_propagates_the_base_error() {
    // Exercises the `Expr::Slice(slice) => ...` arm's own
    // `lower_expr(&sub.value)?` call site specifically -- a distinct
    // region from the `_` arm's identically-worded call site, already
    // covered by `a_subscript_with_an_unsupported_base_propagates_the_base_error`.
    assert_capability_error_message(
        "y = (lambda: 1)[0:1]\n",
        "expression kind not supported yet",
    );
}

#[test]
fn a_slice_with_an_unsupported_start_bound_propagates_the_start_error() {
    assert_capability_error_message(
        "xs = [1, 2, 3]\ny = xs[(lambda: 1):3]\n",
        "expression kind not supported yet",
    );
}

#[test]
fn a_slice_with_an_unsupported_stop_bound_propagates_the_stop_error() {
    assert_capability_error_message(
        "xs = [1, 2, 3]\ny = xs[1:(lambda: 1)]\n",
        "expression kind not supported yet",
    );
}

#[test]
fn a_slice_with_an_unsupported_step_propagates_the_step_error() {
    assert_capability_error_message(
        "xs = [1, 2, 3]\ny = xs[1:3:(lambda: 1)]\n",
        "expression kind not supported yet",
    );
}

#[test]
fn a_slice_assignment_target_is_not_specially_recognized() {
    // Pins `HirExpr::Slice`'s own documented "Load position only"
    // restriction (D-118): unlike a plain-index assignment target
    // (`xs[0] = 1`, which reaches `HirStmt::DictSet` via `Stmt::Assign`'s
    // own `Expr::Subscript` target arm), a slice assignment target
    // (`xs[1:3] = value`) calls `lower_expr` directly on the bare
    // `Expr::Slice` node -- which has no top-level arm -- and falls
    // through to the same generic catch-all as any other unsupported
    // expression kind. This is intentional (slicing ships read-only in
    // this PR), not a regression to fix later without revisiting D-118.
    assert_capability_error_message(
        "xs = [1, 2, 3]\nxs[1:3] = [4, 5]\n",
        "expression kind not supported yet",
    );
}

#[test]
fn rename_name_in_expr_rewrites_every_hir_expr_variant() {
    // Exhaustiveness-pinning test for `rename_name_in_expr`'s own
    // "let the compiler enumerate every site" discipline (D-117,
    // mirroring D-107's `Scalar::List` precedent) -- every arm must be
    // hit by at least one case, and every conditional inside an arm
    // (name matches `from` vs. doesn't) must be hit on both sides.

    // Name: renamed when it matches `from`, left alone otherwise.
    assert_eq!(
        rename_name_in_expr(HirExpr::Name("old".to_string()), "old", "new"),
        HirExpr::Name("new".to_string())
    );
    assert_eq!(
        rename_name_in_expr(HirExpr::Name("other".to_string()), "old", "new"),
        HirExpr::Name("other".to_string())
    );

    // The four grouped literal variants are returned unchanged.
    assert_eq!(
        rename_name_in_expr(HirExpr::IntLiteral(1), "old", "new"),
        HirExpr::IntLiteral(1)
    );
    assert_eq!(
        rename_name_in_expr(HirExpr::FloatLiteral(1.5), "old", "new"),
        HirExpr::FloatLiteral(1.5)
    );
    assert_eq!(
        rename_name_in_expr(HirExpr::BoolLiteral(true), "old", "new"),
        HirExpr::BoolLiteral(true)
    );
    assert_eq!(
        rename_name_in_expr(HirExpr::StringLiteral("s".to_string()), "old", "new"),
        HirExpr::StringLiteral("s".to_string())
    );
    // `NoneLiteral` (D-197, #763, Part 1 of #747) joined this same
    // returned-unchanged group.
    assert_eq!(
        rename_name_in_expr(HirExpr::NoneLiteral, "old", "new"),
        HirExpr::NoneLiteral
    );

    // Call: renames every argument.
    assert_eq!(
        rename_name_in_expr(
            HirExpr::Call {
                callee: "print".to_string(),
                args: vec![HirExpr::Name("old".to_string())],
            },
            "old",
            "new",
        ),
        HirExpr::Call {
            callee: "print".to_string(),
            args: vec![HirExpr::Name("new".to_string())],
        }
    );

    // BinOp: renames both sides.
    assert_eq!(
        rename_name_in_expr(
            HirExpr::BinOp {
                op: BinOpKind::Add,
                left: Box::new(HirExpr::Name("old".to_string())),
                right: Box::new(HirExpr::Name("old".to_string())),
            },
            "old",
            "new",
        ),
        HirExpr::BinOp {
            op: BinOpKind::Add,
            left: Box::new(HirExpr::Name("new".to_string())),
            right: Box::new(HirExpr::Name("new".to_string())),
        }
    );

    // Compare: renames both sides.
    assert_eq!(
        rename_name_in_expr(
            HirExpr::Compare {
                op: CmpOpKind::Eq,
                left: Box::new(HirExpr::Name("old".to_string())),
                right: Box::new(HirExpr::Name("old".to_string())),
            },
            "old",
            "new",
        ),
        HirExpr::Compare {
            op: CmpOpKind::Eq,
            left: Box::new(HirExpr::Name("new".to_string())),
            right: Box::new(HirExpr::Name("new".to_string())),
        }
    );

    // FString: covers both a `Literal` part (passed through unchanged)
    // and an `Interpolation` part (recursed into) in the same tree.
    assert_eq!(
        rename_name_in_expr(
            HirExpr::FString(vec![
                FStringPart::Literal("text".to_string()),
                FStringPart::Interpolation(Box::new(HirExpr::Name("old".to_string()))),
            ]),
            "old",
            "new",
        ),
        HirExpr::FString(vec![
            FStringPart::Literal("text".to_string()),
            FStringPart::Interpolation(Box::new(HirExpr::Name("new".to_string()))),
        ])
    );

    // ListLiteral: renames every element.
    assert_eq!(
        rename_name_in_expr(
            HirExpr::ListLiteral(vec![HirExpr::Name("old".to_string())]),
            "old",
            "new",
        ),
        HirExpr::ListLiteral(vec![HirExpr::Name("new".to_string())])
    );

    // Subscript: renames both base and index.
    assert_eq!(
        rename_name_in_expr(
            HirExpr::Subscript {
                base: Box::new(HirExpr::Name("old".to_string())),
                index: Box::new(HirExpr::Name("old".to_string())),
            },
            "old",
            "new",
        ),
        HirExpr::Subscript {
            base: Box::new(HirExpr::Name("new".to_string())),
            index: Box::new(HirExpr::Name("new".to_string())),
        }
    );

    // Slice: renames `base` and every present bound. Only the `Some`
    // side of each `Option::map` is exercised here -- the `None` side
    // has no closure body of its own to cover (it is just the same
    // `start`/`stop`/`step` value flowing through unchanged), and is
    // separately pinned by `lowers_a_slice_expression_with_all_bounds_omitted`
    // above at the `lower_expr` level.
    assert_eq!(
        rename_name_in_expr(
            HirExpr::Slice {
                base: Box::new(HirExpr::Name("old".to_string())),
                start: Some(Box::new(HirExpr::Name("old".to_string()))),
                stop: Some(Box::new(HirExpr::Name("old".to_string()))),
                step: Some(Box::new(HirExpr::Name("old".to_string()))),
            },
            "old",
            "new",
        ),
        HirExpr::Slice {
            base: Box::new(HirExpr::Name("new".to_string())),
            start: Some(Box::new(HirExpr::Name("new".to_string()))),
            stop: Some(Box::new(HirExpr::Name("new".to_string()))),
            step: Some(Box::new(HirExpr::Name("new".to_string()))),
        }
    );

    // ListAppend: covers both the `list` field matching `from` and not
    // matching it, plus renaming `value` in both cases.
    assert_eq!(
        rename_name_in_expr(
            HirExpr::ListAppend {
                list: crate::ContainerReceiver::Name("old".to_string()),
                value: Box::new(HirExpr::Name("old".to_string())),
            },
            "old",
            "new",
        ),
        HirExpr::ListAppend {
            list: crate::ContainerReceiver::Name("new".to_string()),
            value: Box::new(HirExpr::Name("new".to_string())),
        }
    );
    assert_eq!(
        rename_name_in_expr(
            HirExpr::ListAppend {
                list: crate::ContainerReceiver::Name("other".to_string()),
                value: Box::new(HirExpr::Name("old".to_string())),
            },
            "old",
            "new",
        ),
        HirExpr::ListAppend {
            list: crate::ContainerReceiver::Name("other".to_string()),
            value: Box::new(HirExpr::Name("new".to_string())),
        }
    );

    // DictLiteral: renames both key and value of every pair.
    assert_eq!(
        rename_name_in_expr(
            HirExpr::DictLiteral(vec![(
                HirExpr::Name("old".to_string()),
                HirExpr::Name("old".to_string()),
            )]),
            "old",
            "new",
        ),
        HirExpr::DictLiteral(vec![(
            HirExpr::Name("new".to_string()),
            HirExpr::Name("new".to_string()),
        )])
    );

    // SetLiteral: renames every element.
    assert_eq!(
        rename_name_in_expr(
            HirExpr::SetLiteral(vec![HirExpr::Name("old".to_string())]),
            "old",
            "new",
        ),
        HirExpr::SetLiteral(vec![HirExpr::Name("new".to_string())])
    );

    // TupleLiteral: renames every element.
    assert_eq!(
        rename_name_in_expr(
            HirExpr::TupleLiteral(vec![HirExpr::Name("old".to_string())]),
            "old",
            "new",
        ),
        HirExpr::TupleLiteral(vec![HirExpr::Name("new".to_string())])
    );

    // ListPop: covers both the `list` field matching `from` and not
    // matching it (PR-12 Task 10, D-119; mirrors `ListAppend` above).
    assert_eq!(
        rename_name_in_expr(
            HirExpr::ListPop {
                list: crate::ContainerReceiver::Name("old".to_string()),
            },
            "old",
            "new",
        ),
        HirExpr::ListPop {
            list: crate::ContainerReceiver::Name("new".to_string()),
        }
    );
    assert_eq!(
        rename_name_in_expr(
            HirExpr::ListPop {
                list: crate::ContainerReceiver::Name("other".to_string()),
            },
            "old",
            "new",
        ),
        HirExpr::ListPop {
            list: crate::ContainerReceiver::Name("other".to_string()),
        }
    );

    // DictGetOrDefault: covers both the `dict` field matching `from` and
    // not matching it, plus renaming `key`/`default` in both cases.
    assert_eq!(
        rename_name_in_expr(
            HirExpr::DictGetOrDefault {
                dict: crate::ContainerReceiver::Name("old".to_string()),
                key: Box::new(HirExpr::Name("old".to_string())),
                default: Box::new(HirExpr::Name("old".to_string())),
            },
            "old",
            "new",
        ),
        HirExpr::DictGetOrDefault {
            dict: crate::ContainerReceiver::Name("new".to_string()),
            key: Box::new(HirExpr::Name("new".to_string())),
            default: Box::new(HirExpr::Name("new".to_string())),
        }
    );
    assert_eq!(
        rename_name_in_expr(
            HirExpr::DictGetOrDefault {
                dict: crate::ContainerReceiver::Name("other".to_string()),
                key: Box::new(HirExpr::Name("old".to_string())),
                default: Box::new(HirExpr::Name("old".to_string())),
            },
            "old",
            "new",
        ),
        HirExpr::DictGetOrDefault {
            dict: crate::ContainerReceiver::Name("other".to_string()),
            key: Box::new(HirExpr::Name("new".to_string())),
            default: Box::new(HirExpr::Name("new".to_string())),
        }
    );

    // SetAdd: covers both the `set` field matching `from` and not
    // matching it, plus renaming `value` in both cases.
    assert_eq!(
        rename_name_in_expr(
            HirExpr::SetAdd {
                set: "old".to_string(),
                value: Box::new(HirExpr::Name("old".to_string())),
            },
            "old",
            "new",
        ),
        HirExpr::SetAdd {
            set: "new".to_string(),
            value: Box::new(HirExpr::Name("new".to_string())),
        }
    );
    assert_eq!(
        rename_name_in_expr(
            HirExpr::SetAdd {
                set: "other".to_string(),
                value: Box::new(HirExpr::Name("old".to_string())),
            },
            "old",
            "new",
        ),
        HirExpr::SetAdd {
            set: "other".to_string(),
            value: Box::new(HirExpr::Name("new".to_string())),
        }
    );

    // AttrGet (D-154): renames `base`, `attr` is untouched (it names a
    // field, never a local variable this rename could shadow).
    assert_eq!(
        rename_name_in_expr(
            HirExpr::AttrGet {
                base: Box::new(HirExpr::Name("old".to_string())),
                attr: "x".to_string(),
            },
            "old",
            "new",
        ),
        HirExpr::AttrGet {
            base: Box::new(HirExpr::Name("new".to_string())),
            attr: "x".to_string(),
        }
    );

    // MethodCall (D-154): renames `base` and every argument; `method`
    // is untouched for the same reason `attr` is above.
    assert_eq!(
        rename_name_in_expr(
            HirExpr::MethodCall {
                base: Box::new(HirExpr::Name("old".to_string())),
                method: "bump".to_string(),
                args: vec![HirExpr::Name("old".to_string())],
            },
            "old",
            "new",
        ),
        HirExpr::MethodCall {
            base: Box::new(HirExpr::Name("new".to_string())),
            method: "bump".to_string(),
            args: vec![HirExpr::Name("new".to_string())],
        }
    );

    // Super (#433): carries no names to rename — returned unchanged.
    assert_eq!(
        rename_name_in_expr(HirExpr::Super, "old", "new"),
        HirExpr::Super
    );
}
