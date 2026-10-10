//! `if`/`while`/`for` lowering, `TYPE_CHECKING` folding, jumps out of `finally`, `yield`.

use super::*;

/// Sibling of `assert_capability_error_message` for the new `L0001`
/// context-invalidity diagnostic (issue #141, D-148) -- kept separate
/// rather than parameterizing the existing helper so every other
/// existing `C0001` call site stays untouched.
fn assert_context_invalid_error_message(source: &str, expected_message: &str) {
    let module = pycc_parser_test_helper::parse(source);
    let diagnostic = lower_checked(&module).unwrap_err();

    assert_eq!(diagnostic.code, "L0001");
    assert!(diagnostic.message.contains(expected_message));
    assert!(diagnostic.span.is_some());
}

#[test]
fn lowers_an_if_with_no_else() {
    let module = pycc_parser_test_helper::parse("if True:\n    print(1)\n");
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items,
        vec![HirItem::TopLevelStmt(HirStmt::If {
            test: HirExpr::BoolLiteral(true),
            body: vec![HirStmt::ExprStmt(HirExpr::Call {
                callee: "print".to_string(),
                args: vec![HirExpr::IntLiteral(1)],
            })],
            orelse: vec![],
        })]
    );
}

#[test]
fn lowers_an_if_with_an_else() {
    let module = pycc_parser_test_helper::parse("if True:\n    print(1)\nelse:\n    print(2)\n");
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items,
        vec![HirItem::TopLevelStmt(HirStmt::If {
            test: HirExpr::BoolLiteral(true),
            body: vec![HirStmt::ExprStmt(HirExpr::Call {
                callee: "print".to_string(),
                args: vec![HirExpr::IntLiteral(1)],
            })],
            orelse: vec![HirStmt::ExprStmt(HirExpr::Call {
                callee: "print".to_string(),
                args: vec![HirExpr::IntLiteral(2)],
            })],
        })]
    );
}

#[test]
fn lowers_an_elif_as_a_nested_if_in_orelse() {
    let module = pycc_parser_test_helper::parse(
        "if False:\n    print(1)\nelif True:\n    print(2)\nelse:\n    print(3)\n",
    );
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items,
        vec![HirItem::TopLevelStmt(HirStmt::If {
            test: HirExpr::BoolLiteral(false),
            body: vec![HirStmt::ExprStmt(HirExpr::Call {
                callee: "print".to_string(),
                args: vec![HirExpr::IntLiteral(1)],
            })],
            orelse: vec![HirStmt::If {
                test: HirExpr::BoolLiteral(true),
                body: vec![HirStmt::ExprStmt(HirExpr::Call {
                    callee: "print".to_string(),
                    args: vec![HirExpr::IntLiteral(2)],
                })],
                orelse: vec![HirStmt::ExprStmt(HirExpr::Call {
                    callee: "print".to_string(),
                    args: vec![HirExpr::IntLiteral(3)],
                })],
            }],
        })]
    );
}

#[test]
fn folds_a_bare_type_checking_guard_to_an_empty_dead_body() {
    // #790: `from typing import TYPE_CHECKING; if TYPE_CHECKING: ...` must
    // fold to a literal-false test with an empty body -- the guarded body's
    // `import` (unsupported when nested inside a block) never reaches
    // `lower_stmt`, so `lower_checked` succeeds instead of failing with
    // `C0001`.
    let module = pycc_parser_test_helper::parse(
        "from typing import TYPE_CHECKING\nif TYPE_CHECKING:\n    import some_module_that_does_not_exist_at_runtime_or_compile_time\n",
    );
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items,
        vec![HirItem::TopLevelStmt(HirStmt::If {
            test: HirExpr::BoolLiteral(false),
            body: vec![],
            orelse: vec![],
        })]
    );
}

#[test]
fn folds_a_qualified_type_checking_guard_to_an_empty_dead_body() {
    // #790: the qualified `import typing; if typing.TYPE_CHECKING:` spelling
    // gets the identical fold as the bare-name form.
    let module = pycc_parser_test_helper::parse(
        "import typing\nif typing.TYPE_CHECKING:\n    import some_module_that_does_not_exist_at_runtime_or_compile_time\n",
    );
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items,
        vec![HirItem::TopLevelStmt(HirStmt::If {
            test: HirExpr::BoolLiteral(false),
            body: vec![],
            orelse: vec![],
        })]
    );
}

#[test]
fn an_attribute_named_type_checking_on_another_receiver_is_not_folded() {
    // #790 (D-068 review): `is_type_checking_guard`'s qualified-form arm
    // requires *both* the attribute name `TYPE_CHECKING` *and* a `typing`
    // receiver -- an attribute access that happens to be named
    // `TYPE_CHECKING` on some other object must not be folded, and must
    // lower as an ordinary `if` whose test is a plain attribute read.
    let module =
        pycc_parser_test_helper::parse("if some_other_module.TYPE_CHECKING:\n    print(1)\n");
    let hir = lower_checked(&module).unwrap();
    let HirItem::TopLevelStmt(HirStmt::If { test, body, orelse }) = &hir.items[0] else {
        panic!(
            "expected the `if` statement to lower to `HirStmt::If`, got {:?}",
            hir.items[0]
        );
    };
    assert_eq!(
        *test,
        HirExpr::AttrGet {
            base: Box::new(HirExpr::Name("some_other_module".to_string())),
            attr: "TYPE_CHECKING".to_string(),
        }
    );
    assert_eq!(
        *body,
        vec![HirStmt::ExprStmt(HirExpr::Call {
            callee: "print".to_string(),
            args: vec![HirExpr::IntLiteral(1)],
        })]
    );
    assert_eq!(*orelse, Vec::<HirStmt>::new());
}

#[test]
fn an_attribute_named_type_checking_on_a_nested_receiver_is_not_folded() {
    // #962: the qualified-form arm only consults the alias table when the
    // receiver is a bare name. A nested attribute receiver
    // (`outer.inner.TYPE_CHECKING`) can never be a module alias, so it must
    // fall through to ordinary lowering exactly like any other attribute
    // read named `TYPE_CHECKING`.
    let module = pycc_parser_test_helper::parse("if outer.inner.TYPE_CHECKING:\n    print(1)\n");
    let hir = lower_checked(&module).unwrap();
    let HirItem::TopLevelStmt(HirStmt::If { test, body, orelse }) = &hir.items[0] else {
        panic!(
            "expected the `if` statement to lower to `HirStmt::If`, got {:?}",
            hir.items[0]
        );
    };
    assert_eq!(
        *test,
        HirExpr::AttrGet {
            base: Box::new(HirExpr::AttrGet {
                base: Box::new(HirExpr::Name("outer".to_string())),
                attr: "inner".to_string(),
            }),
            attr: "TYPE_CHECKING".to_string(),
        }
    );
    assert_eq!(
        *body,
        vec![HirStmt::ExprStmt(HirExpr::Call {
            callee: "print".to_string(),
            args: vec![HirExpr::IntLiteral(1)],
        })]
    );
    assert_eq!(*orelse, Vec::<HirStmt>::new());
}

#[test]
fn lowers_the_else_branch_of_a_type_checking_guard_normally() {
    // #790: only the `TYPE_CHECKING` branch itself is dead code -- an
    // `else` clause is live at runtime whenever the guard is skipped, so it
    // must still be lowered exactly like any other `if`/`else`.
    let module = pycc_parser_test_helper::parse(
        "from typing import TYPE_CHECKING\nif TYPE_CHECKING:\n    import some_module_that_does_not_exist_at_runtime_or_compile_time\nelse:\n    print(1)\n",
    );
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items,
        vec![HirItem::TopLevelStmt(HirStmt::If {
            test: HirExpr::BoolLiteral(false),
            body: vec![],
            orelse: vec![HirStmt::ExprStmt(HirExpr::Call {
                callee: "print".to_string(),
                args: vec![HirExpr::IntLiteral(1)],
            })],
        })]
    );
}

#[test]
fn folds_an_elif_type_checking_guard_to_an_empty_dead_body() {
    // #790: `elif TYPE_CHECKING:` gets the same constant-fold as a leading
    // `if TYPE_CHECKING:`, nested inside the enclosing `else` the same way
    // `lowers_an_elif_as_a_nested_if_in_orelse` shows for an ordinary `elif`.
    let module = pycc_parser_test_helper::parse(
        "from typing import TYPE_CHECKING\nif False:\n    print(1)\nelif TYPE_CHECKING:\n    import some_module_that_does_not_exist_at_runtime_or_compile_time\nelse:\n    print(2)\n",
    );
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items,
        vec![HirItem::TopLevelStmt(HirStmt::If {
            test: HirExpr::BoolLiteral(false),
            body: vec![HirStmt::ExprStmt(HirExpr::Call {
                callee: "print".to_string(),
                args: vec![HirExpr::IntLiteral(1)],
            })],
            orelse: vec![HirStmt::If {
                test: HirExpr::BoolLiteral(false),
                body: vec![],
                orelse: vec![HirStmt::ExprStmt(HirExpr::Call {
                    callee: "print".to_string(),
                    args: vec![HirExpr::IntLiteral(2)],
                })],
            }],
        })]
    );
}

#[test]
fn a_lowering_error_in_the_else_after_an_if_type_checking_guard_propagates() {
    // #790: the leading `if TYPE_CHECKING:` arm still lowers its `orelse`
    // chain normally (only the guard's own body is dead) -- proves the `?`
    // on that recursive `lower_elif_else_clauses` call actually propagates
    // a genuine error from a live `else`, not just the empty-`orelse`
    // success path every other test above exercises.
    assert_capability_error_message(
        "from typing import TYPE_CHECKING\nif TYPE_CHECKING:\n    pass\nelse:\n    import some_module_that_does_not_exist_at_runtime_or_compile_time\n",
        "import of module `some_module_that_does_not_exist_at_runtime_or_compile_time` is not supported yet",
    );
}

#[test]
fn a_lowering_error_after_an_elif_type_checking_guard_propagates() {
    // #790: same as above, but for the `elif TYPE_CHECKING:` fold's own
    // recursive `orelse` lowering (a separate `?` in
    // `lower_elif_else_clauses`, reached only once an earlier ordinary
    // branch is already live).
    assert_capability_error_message(
        "from typing import TYPE_CHECKING\nif False:\n    pass\nelif TYPE_CHECKING:\n    pass\nelse:\n    import some_module_that_does_not_exist_at_runtime_or_compile_time\n",
        "import of module `some_module_that_does_not_exist_at_runtime_or_compile_time` is not supported yet",
    );
}

#[test]
fn lowers_a_while_loop() {
    let module = pycc_parser_test_helper::parse("while True:\n    print(1)\n");
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items,
        vec![HirItem::TopLevelStmt(HirStmt::While {
            test: HirExpr::BoolLiteral(true),
            body: vec![HirStmt::ExprStmt(HirExpr::Call {
                callee: "print".to_string(),
                args: vec![HirExpr::IntLiteral(1)],
            })],
        })]
    );
}

#[test]
fn a_while_else_lowers_to_a_loop_else_wrapper() {
    let module = pycc_parser_test_helper::parse("while True:\n    break\nelse:\n    print(2)\n");
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items,
        vec![HirItem::TopLevelStmt(HirStmt::LoopElse {
            loop_stmt: Box::new(HirStmt::While {
                test: HirExpr::BoolLiteral(true),
                body: vec![HirStmt::Break],
            }),
            orelse: vec![HirStmt::ExprStmt(HirExpr::Call {
                callee: "print".to_string(),
                args: vec![HirExpr::IntLiteral(2)],
            })],
        })]
    );
}

#[test]
fn an_else_pass_clause_leaves_the_loop_unwrapped() {
    let module = pycc_parser_test_helper::parse("while True:\n    continue\nelse:\n    pass\n");
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items,
        vec![HirItem::TopLevelStmt(HirStmt::While {
            test: HirExpr::BoolLiteral(true),
            body: vec![HirStmt::Continue],
        })]
    );
}

#[test]
fn a_break_in_a_loop_else_clause_targets_the_enclosing_loop() {
    // The `else:` clause is outside its own loop: with no enclosing loop
    // its `break` is CPython's `'break' outside loop`.
    assert_context_invalid_error_message(
        "while True:\n    pass\nelse:\n    break\n",
        "'break' outside loop",
    );
    let module = pycc_parser_test_helper::parse(
        "for i in range(2):\n    for j in range(2):\n        pass\n    else:\n        break\n",
    );
    lower_checked(&module).expect("a break in an inner loop's else clause targets the outer loop");
}

#[test]
fn a_break_out_of_a_try_with_a_finally_clause_is_not_supported_yet() {
    for source in [
        "for i in range(3):\n    try:\n        break\n    finally:\n        print(i)\n",
        "for i in range(3):\n    try:\n        pass\n    except ValueError:\n        if i:\n            continue\n    finally:\n        print(i)\n",
        "for i in range(3):\n    try:\n        pass\n    except ValueError:\n        pass\n    else:\n        break\n    finally:\n        print(i)\n",
        "while True:\n    try:\n        for i in range(3):\n            pass\n        else:\n            break\n    finally:\n        print(1)\n",
        "while True:\n    try:\n        match 1:\n            case 1:\n                break\n    finally:\n        print(1)\n",
        "while True:\n    try:\n        try:\n            pass\n        finally:\n            pass\n        continue\n    finally:\n        print(1)\n",
    ] {
        assert_capability_error_message(source, "out of a `try` block that has a `finally` clause");
    }
}

#[test]
fn a_jump_inside_a_loop_nested_in_a_try_with_finally_lowers() {
    for source in [
        "try:\n    for i in range(3):\n        break\nfinally:\n    print(1)\n",
        "for i in range(3):\n    try:\n        break\n    except ValueError:\n        continue\n",
    ] {
        let module = pycc_parser_test_helper::parse(source);
        lower_checked(&module).expect("the jump stays inside the loop or crosses no finally");
    }
}

#[test]
fn lowers_a_for_range_loop_with_one_argument() {
    let module = pycc_parser_test_helper::parse("for i in range(3):\n    print(i)\n");
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items,
        vec![HirItem::TopLevelStmt(HirStmt::ForRange {
            var: "i".to_string(),
            start: HirExpr::IntLiteral(0),
            stop: HirExpr::IntLiteral(3),
            step: HirExpr::IntLiteral(1),
            body: vec![HirStmt::ExprStmt(HirExpr::Call {
                callee: "print".to_string(),
                args: vec![HirExpr::Name("i".to_string())],
            })],
        })]
    );
}

#[test]
fn lowers_a_for_range_loop_with_start_and_stop() {
    let module = pycc_parser_test_helper::parse("for i in range(1, 3):\n    print(i)\n");
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items,
        vec![HirItem::TopLevelStmt(HirStmt::ForRange {
            var: "i".to_string(),
            start: HirExpr::IntLiteral(1),
            stop: HirExpr::IntLiteral(3),
            step: HirExpr::IntLiteral(1),
            body: vec![HirStmt::ExprStmt(HirExpr::Call {
                callee: "print".to_string(),
                args: vec![HirExpr::Name("i".to_string())],
            })],
        })]
    );
}

#[test]
fn lowers_a_for_range_loop_with_start_stop_and_step() {
    let module = pycc_parser_test_helper::parse("for i in range(0, 6, 2):\n    print(i)\n");
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items,
        vec![HirItem::TopLevelStmt(HirStmt::ForRange {
            var: "i".to_string(),
            start: HirExpr::IntLiteral(0),
            stop: HirExpr::IntLiteral(6),
            step: HirExpr::IntLiteral(2),
            body: vec![HirStmt::ExprStmt(HirExpr::Call {
                callee: "print".to_string(),
                args: vec![HirExpr::Name("i".to_string())],
            })],
        })]
    );
}

#[test]
fn iterating_a_non_call_expression_is_not_supported_yet() {
    // A list *literal* iterable is still rejected (Task 7/D-105 only
    // added support for a bare-name iterable, i.e. `for v in some_list:`
    // -- see `lowers_for_over_a_list_name_to_for_list`); the rejection
    // message itself changed to mention that new bare-name-list form.
    assert_capability_error_message(
        "for i in [1, 2, 3]:\n    print(i)\n",
        "only `for x in range(...)` or `for x in <list>` is supported so far",
    );
}

#[test]
fn calling_something_other_than_range_in_a_for_is_not_supported_yet() {
    assert_capability_error_message(
        "for i in items(3):\n    print(i)\n",
        "only iterating over `range(...)` is supported so far",
    );
}

/// PR 3c of #1082 narrowed this arm: `for i in a.b(3):` is now one of the
/// two admitted foreign-`object` iterable shapes and lowers to
/// `HirStmt::ForObject` (the type checker refuses a non-`object` one), so
/// the arm survives only for a callee that is neither a name nor an
/// attribute.
#[test]
fn calling_via_a_subscript_in_a_for_is_not_supported_yet() {
    assert_capability_error_message(
        "for i in a[0](3):\n    print(i)\n",
        "only `for x in range(...)` is supported so far",
    );
}

/// The iterable of an admitted shape is lowered as a real expression, so
/// a lowering failure inside it has to propagate rather than be swallowed
/// -- a `lambda` argument is the smallest expression `lower_expr` refuses.
#[test]
fn a_lowering_failure_inside_a_for_object_iterable_propagates() {
    assert_capability_error_message(
        "for x in a.b(lambda: 1):\n    pass\n",
        "expression kind not supported yet: a `lambda`",
    );
}

/// The same for the loop body, which the new arm lowers itself.
#[test]
fn a_lowering_failure_inside_a_for_object_body_propagates() {
    assert_capability_error_message(
        "for x in a.b:\n    import os\n",
        "an `import` inside a block body",
    );
}

/// The other half of that narrowing: both admitted shapes lower rather
/// than being refused here, since `pycc_hir` has no types to refuse them
/// with.
#[test]
fn both_foreign_object_iterable_shapes_lower_to_for_object() {
    for source in [
        "for x in a.b:\n    print(x)\n",
        "for x in a.b(3):\n    print(x)\n",
    ] {
        let module = pycc_parser_test_helper::parse(source);
        let hir = lower_checked(&module).expect("the shape lowers");
        let [HirItem::TopLevelStmt(HirStmt::ForObject { var, body, .. })] = hir.items.as_slice()
        else {
            panic!("{source:?} must lower to one `ForObject`: {:?}", hir.items);
        };
        assert_eq!(var, "x");
        assert_eq!(body.len(), 1, "{source:?}");
    }
}

#[test]
fn range_with_too_many_arguments_is_not_supported() {
    assert_capability_error_message(
        "for i in range(1, 2, 3, 4):\n    print(i)\n",
        "range() with 4 arguments is not supported",
    );
}

#[test]
fn a_tuple_for_target_is_not_supported_yet() {
    assert_capability_error_message(
        "for i, j in range(3):\n    print(i)\n",
        "only a bare name for-target is supported so far",
    );
}

#[test]
fn a_for_else_lowers_to_a_loop_else_wrapper() {
    let module =
        pycc_parser_test_helper::parse("for i in range(3):\n    continue\nelse:\n    print(0)\n");
    let hir = lower_checked(&module).unwrap();
    let [HirItem::TopLevelStmt(HirStmt::LoopElse { loop_stmt, orelse })] = hir.items.as_slice()
    else {
        panic!("expected one loop-else statement, got {:?}", hir.items);
    };
    assert!(matches!(
        loop_stmt.as_ref(),
        HirStmt::ForRange { body, .. } if body == &vec![HirStmt::Continue]
    ));
    assert_eq!(orelse.len(), 1);
}

#[test]
fn an_async_for_inside_an_async_function_is_not_supported_yet() {
    // `async for` is only valid Python syntax inside an `async def` body,
    // so this now hits the (newer, more general) async-function rejection
    // in lower_function before lower_stmt's own `for_stmt.is_async` check
    // is ever reached -- the fixture still exercises real, valid Python
    // that must be rejected, just via the outer boundary now.
    assert_capability_error_message(
        "async def f() -> None:\n    async for i in range(3):\n        print(i)\n",
        "async functions are not supported yet",
    );
}

#[test]
fn a_top_level_async_for_is_context_invalid() {
    // Issue #141 / D-148: `async for` outside an async function is
    // syntactically well-formed but CPython rejects it as a
    // `SyntaxError`, so this is now `L0001`, not `C0001` -- there is no
    // reachable "valid, just unimplemented" case here (see Correction 2
    // in the published plan): `lower_function` already rejects any
    // `async def` before this arm could ever be reached from inside a
    // real async function.
    assert_context_invalid_error_message(
        "async for i in range(3):\n    print(i)\n",
        "'async for' outside async function",
    );
}

#[test]
fn an_async_for_inside_a_synchronous_function_is_context_invalid() {
    // The issue's own "other invalid async context" example beyond top
    // level: a synchronous `def` body reaches `lower_function`'s body
    // lowering (which resets `in_loop` but not any async-context state),
    // then this `for_stmt.is_async` arm -- unconditionally `L0001`,
    // exactly like the top-level case, since a synchronous function
    // provides no more of an "async function" context than module scope
    // does.
    assert_context_invalid_error_message(
        "def f() -> None:\n    async for i in range(3):\n        print(i)\n",
        "'async for' outside async function",
    );
}

#[test]
fn a_top_level_break_is_context_invalid() {
    assert_context_invalid_error_message("break\n", "'break' outside loop");
}

#[test]
fn a_top_level_continue_is_context_invalid() {
    assert_context_invalid_error_message("continue\n", "'continue' not properly in loop");
}

#[test]
fn a_break_inside_a_synchronous_function_with_no_loop_is_context_invalid() {
    // Regression guard for `lower_function`'s own `false` call site
    // (entering a function body resets `in_loop`): a `break` directly in
    // a function body, with no enclosing loop, must still be
    // context-invalid, not silently inherit `true` from some outer
    // caller state.
    assert_context_invalid_error_message("def f() -> None:\n    break\n", "'break' outside loop");
}

#[test]
fn a_break_or_continue_with_an_enclosing_loop_lowers() {
    // A real enclosing loop makes the jump valid, including through an
    // `if`: the `If` arm and `lower_elif_else_clauses` pass the caller's
    // `in_loop` value through rather than resetting it.
    for source in [
        "for i in range(3):\n    break\n",
        "while True:\n    continue\n",
        "for i in range(3):\n    if i:\n        break\n    elif i > 1:\n        continue\n",
    ] {
        let module = pycc_parser_test_helper::parse(source);
        lower_checked(&module).expect("a loop jump with an enclosing loop must lower");
    }
}

// -- PEP 765 (#738, Part 1 of #543): `return`/`break`/`continue` inside a
// `finally` block --
//
// The exact shape below (one `in_finally` flag shared by all three
// statement kinds, reset by the nearest enclosing loop reached from inside
// the `finally` but not by a plain conditional or a nested non-`finally`
// `try` part) was verified empirically against CPython 3.14's actual
// `SyntaxWarning` behavior rather than assumed from the PEP text alone --
// see `stmt.rs`'s own module doc comment for the full account, including
// the naive-but-wrong two-flag design this rejected (a `return` inside a
// loop defined inside the `finally` is, perhaps counter-intuitively, NOT
// flagged by CPython even though it still exits the function).

#[test]
fn a_return_directly_in_a_finally_block_is_context_invalid() {
    assert_context_invalid_error_message(
        "def f() -> int:\n    try:\n        pass\n    finally:\n        return 3\n",
        "'return' in a 'finally' block",
    );
}

#[test]
fn a_break_directly_in_a_finally_block_is_context_invalid() {
    // The classic PEP 765 pattern: a real enclosing loop exists outside
    // the `try`/`finally` (`in_loop` is `true`), but the `finally`
    // violation takes precedence over the pre-existing valid-but-
    // unimplemented `C0001` path.
    assert_context_invalid_error_message(
        "def f() -> None:\n    while True:\n        try:\n            pass\n        finally:\n            break\n",
        "'break' in a 'finally' block",
    );
}

#[test]
fn a_continue_directly_in_a_finally_block_is_context_invalid() {
    assert_context_invalid_error_message(
        "def f() -> None:\n    while True:\n        try:\n            pass\n        finally:\n            continue\n",
        "'continue' in a 'finally' block",
    );
}

#[test]
fn a_return_nested_under_if_inside_a_finally_block_is_still_context_invalid() {
    // Guards the `If` arm's pass-through of `in_finally`: a conditional
    // does not shield a `return` from the enclosing `finally`.
    assert_context_invalid_error_message(
        "def f() -> int:\n    try:\n        pass\n    finally:\n        if True:\n            return 3\n",
        "'return' in a 'finally' block",
    );
}

#[test]
fn a_return_in_the_try_body_of_a_nested_try_inside_a_finally_block_is_context_invalid() {
    // Guards the `Try` arm's pass-through of `in_finally` for its own
    // `body`/`handlers`/`orelse` (only entering a *new* `finalbody` forces
    // it back to `true` unconditionally) -- a nested try's non-`finally`
    // parts still inherit the outer `finally`'s restriction.
    assert_context_invalid_error_message(
        "def f() -> int:\n    try:\n        pass\n    finally:\n        try:\n            return 3\n        except ValueError:\n            pass\n",
        "'return' in a 'finally' block",
    );
}

#[test]
fn a_return_in_the_except_body_of_a_nested_try_inside_a_finally_block_is_context_invalid() {
    assert_context_invalid_error_message(
        "def f() -> None:\n    while True:\n        try:\n            pass\n        finally:\n            try:\n                pass\n            except ValueError:\n                break\n",
        "'break' in a 'finally' block",
    );
}

#[test]
fn a_return_in_the_else_body_of_a_nested_try_inside_a_finally_block_is_context_invalid() {
    // Guards the `Try` arm's pass-through of `in_finally` for its own
    // `orelse` (the `else:` clause), the last of the three non-`finally`
    // `Try` parts -- `body` and `handlers` are already covered by the two
    // sibling tests above.
    assert_context_invalid_error_message(
        "def f() -> int:\n    try:\n        pass\n    finally:\n        try:\n            pass\n        except ValueError:\n            pass\n        else:\n            return 3\n",
        "'return' in a 'finally' block",
    );
}

#[test]
fn a_break_in_a_nested_finally_inside_a_finally_block_is_context_invalid() {
    // A nested `try`/`finally`'s own `finalbody`, itself lexically inside
    // an outer `finally`, is a violation on its own -- entering it forces
    // `in_finally` back to `true` regardless of the outer state, so no
    // loop needs to intervene for this to already be true.
    assert_context_invalid_error_message(
        "def f() -> None:\n    while True:\n        try:\n            pass\n        finally:\n            try:\n                pass\n            finally:\n                break\n",
        "'break' in a 'finally' block",
    );
}

#[test]
fn a_return_in_a_loop_defined_inside_a_finally_block_lowers_successfully() {
    // A loop introduced *inside* the finally shields `return` from the
    // enclosing `finally`, exactly like `break`/`continue` (see the two
    // tests below) -- verified directly against CPython 3.14 rather than
    // assumed: despite a naive PEP-text-only reading predicting this
    // *should* still be rejected (a `return` inside the inner loop still
    // exits the function, same as it would without the loop), CPython's
    // actual compiler does not flag it, so this lowers all the way
    // through successfully.
    let module = pycc_parser_test_helper::parse(
        "def f() -> int:\n    try:\n        pass\n    finally:\n        for i in range(3):\n            return i\n",
    );
    lower_checked(&module).expect("a return shielded by an inner loop must lower successfully");
}

#[test]
fn a_break_targeting_a_loop_defined_inside_a_finally_block_lowers() {
    // A `break` that targets a loop defined *inside* the `finally` itself
    // does not escape the finally, so it is not the `L0001` finally
    // violation, and it lowers like any other loop jump.
    let module = pycc_parser_test_helper::parse(
        "def f() -> None:\n    try:\n        pass\n    finally:\n        for i in range(3):\n            break\n",
    );
    lower_checked(&module).expect("a break shielded by an inner loop must lower");
}

#[test]
fn a_continue_targeting_a_loop_defined_inside_a_finally_block_lowers() {
    let module = pycc_parser_test_helper::parse(
        "def f() -> None:\n    try:\n        pass\n    finally:\n        for i in range(3):\n            continue\n",
    );
    lower_checked(&module).expect("a continue shielded by an inner loop must lower");
}

#[test]
fn a_return_in_a_match_case_inside_a_finally_block_is_context_invalid() {
    // Guards `lower_match`'s pass-through of `in_finally` to each case
    // body.
    assert_context_invalid_error_message(
        "def f(x: int) -> int:\n    try:\n        pass\n    finally:\n        match x:\n            case 1:\n                return 1\n            case _:\n                pass\n",
        "'return' in a 'finally' block",
    );
}

#[test]
fn a_return_in_a_nested_def_inside_a_finally_block_hits_capability_error_not_context_invalid() {
    // A nested function definition has its own return scope and would NOT
    // be a PEP 765 violation in real Python -- but nested `def`s are
    // wholesale unsupported in this compiler today (`Stmt::FunctionDef`
    // has no arm in `lower_stmt`'s match, so it falls to the generic
    // "statement kind not supported yet" catch-all before this check
    // could ever run). This is a negative control proving the new
    // `in_finally` check doesn't misfire on it: the nested `def` itself is
    // what's rejected, with the pre-existing `C0001`, not the new PEP 765
    // `L0001`.
    assert_capability_error_message(
        "def f() -> int:\n    try:\n        pass\n    finally:\n        def g() -> int:\n            return 1\n        return g()\n",
        "statement kind not supported yet",
    );
}

#[test]
fn a_top_level_yield_is_context_invalid() {
    // Issue #361 / D-149, this crate's expression-lowering sequel to
    // #141/D-148: `yield` outside any function is syntactically
    // well-formed but CPython rejects it as a `SyntaxError`, so this is
    // now `L0001`, not `C0001`.
    assert_context_invalid_error_message("yield 1\n", "'yield' outside function");
}

#[test]
fn a_top_level_yield_from_is_context_invalid() {
    assert_context_invalid_error_message("yield from [1, 2]\n", "'yield from' outside function");
}

#[test]
fn a_yield_nested_inside_a_top_level_if_is_still_context_invalid() {
    // Pins that `in_function` correctly stays `false` through `If`/
    // `lower_elif_else_clauses` recursion at module scope -- mirrors the
    // equivalent existing coverage for `break`/`continue` (D-148).
    assert_context_invalid_error_message("if True:\n    yield 1\n", "'yield' outside function");
}

#[test]
fn a_yield_inside_a_real_function_is_still_unsupported() {
    // Regression guard: a real enclosing function keeps `yield` on the
    // existing valid-but-unimplemented `C0001` path -- generator codegen
    // remains out of scope for this issue.
    assert_capability_error_message(
        "def f() -> None:\n    yield 1\n",
        "expression kind not supported yet",
    );
}

#[test]
fn a_yield_from_inside_a_real_function_is_still_unsupported() {
    assert_capability_error_message(
        "def f() -> None:\n    yield from [1, 2]\n",
        "expression kind not supported yet",
    );
}

#[test]
fn a_yield_inside_a_comprehension_if_filter_stays_unsupported_at_module_scope() {
    // Regression-pinning test (D-149 correction 5): a `yield` reached
    // through a comprehension's `if`-filter is governed by a third,
    // scope-independent CPython rule (`'yield' inside list
    // comprehension`) this issue does not implement. The comprehension
    // helper cluster hardcodes a literal `true` at its own internal
    // `lower_expr` call sites instead of forwarding the real ambient
    // `in_function` value, so this must stay `C0001` at module scope,
    // unchanged by this fix -- guarding against a future edit that
    // "helpfully" threads the real value through and starts emitting the
    // wrong-per-CPython `L0001: 'yield' outside function` message here.
    assert_capability_error_message(
        "y = [x for x in range(3) if (yield x)]\n",
        "expression kind not supported yet",
    );
}
