//! `print` lowering: argument evaluation order, `None` rendering, void calls.

use super::*;

#[test]
fn compiles_a_zero_argument_print_producing_just_a_newline() {
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
            callee: "print".to_string(),
            args: vec![],
            ty: Ty::None,
        }))],
        class_defs: Vec::new(),
    };
    let dir =
        pycc_scratch::ScratchDir::new("print_zero_args").expect("failed to create scratch dir");
    let obj_path = dir.join("print_zero_args.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("print_zero_args");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(pycc_scratch::lf(&output.stdout), b"\n");
}

#[test]
fn compiles_a_multi_argument_print_with_mixed_types_space_separated() {
    // `print(1, 2.5, True, "hi")` -- prints `1 2.5 True hi\n`.
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
            callee: "print".to_string(),
            args: vec![
                MirExpr::IntLiteral(1),
                MirExpr::FloatLiteral(2.5),
                MirExpr::BoolLiteral(true),
                MirExpr::StringLiteral("hi".to_string()),
            ],
            ty: Ty::None,
        }))],
        class_defs: Vec::new(),
    };
    let dir =
        pycc_scratch::ScratchDir::new("print_mixed_multi").expect("failed to create scratch dir");
    let obj_path = dir.join("print_mixed_multi.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("print_mixed_multi");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(pycc_scratch::lf(&output.stdout), b"1 2.5 True hi\n");
}

#[test]
fn compiles_print_of_a_bool_false() {
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
            callee: "print".to_string(),
            args: vec![MirExpr::BoolLiteral(false)],
            ty: Ty::None,
        }))],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("print_false").expect("failed to create scratch dir");
    let obj_path = dir.join("print_false.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("print_false");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(pycc_scratch::lf(&output.stdout), b"False\n");
}

#[test]
fn compiles_print_of_a_void_returning_call_as_none() {
    // `def f() -> None: return` ; `print(f())` -- prints `None`.
    let mir = MirModule {
        items: vec![
            MirItem::Function {
                name: "f".to_string(),
                params: vec![],
                return_ty: Ty::None,
                body: vec![MirStmt::Return(None)],
            },
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
                callee: "print".to_string(),
                args: vec![MirExpr::Call {
                    callee: "f".to_string(),
                    args: vec![],
                    ty: Ty::None,
                }],
                ty: Ty::None,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("print_none_from_call")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("print_none_from_call.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("print_none_from_call");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(pycc_scratch::lf(&output.stdout), b"None\n");
}

#[test]
fn compiles_print_evaluating_all_args_before_output_with_a_side_effecting_call() {
    // #145: `def side_effect() -> int: print(2); return 3` ;
    // `print(1, side_effect())` -- the later argument's side effect
    // (printing `2`) must complete before the outer `print`'s own
    // output begins, so stdout is `2\n1 3\n` (CPython's order), not
    // the interleaved `1 2\n3\n` the old single-phase emit produced.
    let mir = MirModule {
        items: vec![
            MirItem::Function {
                name: "side_effect".to_string(),
                params: vec![],
                return_ty: Ty::Int,
                body: vec![
                    MirStmt::ExprStmt(MirExpr::Call {
                        callee: "print".to_string(),
                        args: vec![MirExpr::IntLiteral(2)],
                        ty: Ty::None,
                    }),
                    MirStmt::Return(Some(MirExpr::IntLiteral(3))),
                ],
            },
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
                callee: "print".to_string(),
                args: vec![
                    MirExpr::IntLiteral(1),
                    MirExpr::Call {
                        callee: "side_effect".to_string(),
                        args: vec![],
                        ty: Ty::Int,
                    },
                ],
                ty: Ty::None,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("print_eval_order_side_effect")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("print_eval_order_side_effect.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("print_eval_order_side_effect");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(pycc_scratch::lf(&output.stdout), b"2\n1 3\n");
}

#[test]
fn compiles_print_with_a_failing_later_argument_produces_no_partial_output() {
    // #145/#382: `def fail_later() -> int: return 1 // 0` ;
    // `print(1, fail_later())` -- the later argument's zero-divisor
    // now sets the pending exception state (D-173, #382) instead of
    // panicking. The function returns a neutral 0, `print` evaluates
    // all args before output, and the top-level exception check after
    // the `print` statement detects the active exception and exits
    // with non-zero. stdout may contain partial output because the
    // process no longer aborts mid-statement — the exception is
    // caught at the next top-level check point.
    let mir = MirModule {
        items: vec![
            MirItem::Function {
                name: "fail_later".to_string(),
                params: vec![],
                return_ty: Ty::Int,
                body: vec![MirStmt::Return(Some(MirExpr::BinOp {
                    op: BinOpKind::FloorDiv,
                    left: Box::new(MirExpr::IntLiteral(1)),
                    right: Box::new(MirExpr::IntLiteral(0)),
                    ty: Ty::Int,
                }))],
            },
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
                callee: "print".to_string(),
                args: vec![
                    MirExpr::IntLiteral(1),
                    MirExpr::Call {
                        callee: "fail_later".to_string(),
                        args: vec![],
                        ty: Ty::Int,
                    },
                ],
                ty: Ty::None,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("print_eval_order_fail_later")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("print_eval_order_fail_later.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("print_eval_order_fail_later");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    // #382: The process exits with non-zero because the top-level
    // exception check catches the ZeroDivisionError set by `1 // 0`.
    assert!(
        !output.status.success(),
        "process should exit non-zero on zero-divisor exception: {:?}",
        output.status
    );
}

#[test]
fn returning_a_void_returning_call_result_from_a_none_returning_function_runs_the_callee_and_returns_void()
 {
    // `def helper() -> None: return` ; `def outer() -> None: return
    // helper()` ; `outer()` -- must not build `ret i8 0` inside
    // `outer`'s `void` LLVM signature (previously failed `verify_module`
    // with "Found return instr that returns non-void in Function of
    // void return type!").
    let mir = MirModule {
        items: vec![
            MirItem::Function {
                name: "helper".to_string(),
                params: vec![],
                return_ty: Ty::None,
                body: vec![MirStmt::Return(None)],
            },
            MirItem::Function {
                name: "outer".to_string(),
                params: vec![],
                return_ty: Ty::None,
                body: vec![MirStmt::Return(Some(MirExpr::Call {
                    callee: "helper".to_string(),
                    args: vec![],
                    ty: Ty::None,
                }))],
            },
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
                callee: "outer".to_string(),
                args: vec![],
                ty: Ty::None,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir =
        pycc_scratch::ScratchDir::new("return_none_call").expect("failed to create scratch dir");
    let obj_path = dir.join("return_none_call.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("return_none_call");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert!(output.status.success());
    assert_eq!(pycc_scratch::lf(&output.stdout), b"");
}

#[test]
fn printing_a_none_typed_parameter_renders_none() {
    // `def source() -> None: return`; `def sink(y: None) -> None:
    // print(y)`; `sink(source())` -- exercises the canonical unit
    // carrier as a call argument, parameter slot, name read, and print
    // input without ever confusing it for the physically-identical
    // `False` carrier.
    let mir = MirModule {
        items: vec![
            MirItem::Function {
                name: "source".to_string(),
                params: vec![],
                return_ty: Ty::None,
                body: vec![MirStmt::Return(None)],
            },
            MirItem::Function {
                name: "sink".to_string(),
                params: vec![("y".to_string(), Ty::None)],
                return_ty: Ty::None,
                body: vec![MirStmt::ExprStmt(MirExpr::Call {
                    callee: "print".to_string(),
                    args: vec![MirExpr::Name {
                        name: "y".to_string(),
                        ty: Ty::None,
                    }],
                    ty: Ty::None,
                })],
            },
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
                callee: "sink".to_string(),
                args: vec![MirExpr::Call {
                    callee: "source".to_string(),
                    args: vec![],
                    ty: Ty::None,
                }],
                ty: Ty::None,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("print_none_typed_parameter")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("print_none_typed_parameter.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("print_none_typed_parameter");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(pycc_scratch::lf(&output.stdout), b"None\n");
}

#[test]
fn a_bare_return_with_no_value_exits_a_none_returning_function_early() {
    // `def f() -> None:\n    return\n    print(999)` ; `f(); print(1)`
    // -- supersedes this test's earlier (Task 3/4) incarnation,
    // `a_return_statement_is_not_yet_supported_by_codegen`, which
    // proved `Return` had no codegen at all yet (via `emit_stmt`'s
    // then-catch-all, since removed -- the match is now exhaustive
    // over `MirStmt`, see Task 5's own doc comment on `emit_stmt`).
    // Now that `Return` is fully implemented, this instead exercises
    // its `None` arm (a bare `return`, as opposed to `return <expr>`,
    // which the two dedicated function-call tests above already
    // cover) and proves `emit_body`'s terminator-safety early-stop
    // (re-added by this task, see its own doc comment) really does
    // skip the unreachable `print(999)` after the `return`, rather
    // than trying to emit into an already-terminated block. Only "1"
    // should print.
    let mir = MirModule {
        items: vec![
            MirItem::Function {
                name: "f".to_string(),
                params: vec![],
                return_ty: Ty::None,
                body: vec![
                    MirStmt::Return(None),
                    MirStmt::ExprStmt(MirExpr::Call {
                        callee: "print".to_string(),
                        args: vec![MirExpr::IntLiteral(999)],
                        ty: Ty::None,
                    }),
                ],
            },
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
                callee: "f".to_string(),
                args: vec![],
                ty: Ty::None,
            })),
            MirItem::TopLevelStmt(call_print(1)),
        ],
        class_defs: Vec::new(),
    };
    let dir =
        pycc_scratch::ScratchDir::new("bare_return_none").expect("failed to create scratch dir");
    let obj_path = dir.join("bare_return_none.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("bare_return_none");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(pycc_scratch::lf(&output.stdout), b"1\n");
}
