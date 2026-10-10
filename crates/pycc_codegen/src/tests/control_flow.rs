//! `if`/`while`/`for range` control flow and unreachable-merge termination.

use super::*;

#[test]
fn compiles_an_if_else_choosing_the_correct_branch_at_runtime() {
    // `x = 1; if x < 2: print(10) else: print(20)` -- must print 10.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "x".to_string(),
                value: MirExpr::IntLiteral(1),
            }),
            MirItem::TopLevelStmt(MirStmt::If {
                test: MirExpr::Compare {
                    op: CmpOpKind::Lt,
                    left: Box::new(MirExpr::Name {
                        name: "x".to_string(),
                        ty: Ty::Int,
                    }),
                    right: Box::new(MirExpr::IntLiteral(2)),
                    ty: Ty::Bool,
                },
                body: vec![MirStmt::ExprStmt(MirExpr::Call {
                    callee: "print".to_string(),
                    args: vec![MirExpr::IntLiteral(10)],
                    ty: Ty::None,
                })],
                orelse: vec![MirStmt::ExprStmt(MirExpr::Call {
                    callee: "print".to_string(),
                    args: vec![MirExpr::IntLiteral(20)],
                    ty: Ty::None,
                })],
            }),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("if_else").expect("failed to create scratch dir");
    let obj_path = dir.join("if_else.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("if_else");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"10\n");
}

#[test]
fn an_if_whose_both_branches_return_terminates_its_unreachable_merge() {
    let mir = MirModule {
        items: vec![
            MirItem::Function {
                name: "choose".to_string(),
                params: vec![("flag".to_string(), Ty::Bool)],
                return_ty: Ty::Int,
                body: vec![MirStmt::If {
                    test: MirExpr::Name {
                        name: "flag".to_string(),
                        ty: Ty::Bool,
                    },
                    body: vec![MirStmt::Return(Some(MirExpr::IntLiteral(1)))],
                    orelse: vec![MirStmt::Return(Some(MirExpr::IntLiteral(2)))],
                }],
            },
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
                callee: "print".to_string(),
                args: vec![MirExpr::Call {
                    callee: "choose".to_string(),
                    args: vec![MirExpr::BoolLiteral(true)],
                    ty: Ty::Int,
                }],
                ty: Ty::None,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("if_both_branches_return")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("if_both_branches_return.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("if_both_branches_return");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"1\n");
}

#[test]
fn a_statically_unreachable_match_tail_terminates_its_function() {
    let mir = MirModule {
        items: vec![MirItem::Function {
            name: "exhaustive_match_tail".to_string(),
            params: vec![],
            return_ty: Ty::None,
            body: vec![MirStmt::Unreachable],
        }],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("statically_unreachable_match_tail")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("statically_unreachable_match_tail.o");
    compile_to_object(&mir, &obj_path, None, false)
        .expect("a statically unreachable match tail must produce valid LLVM IR");
}

#[test]
fn compiles_an_if_with_no_else_and_a_false_test_prints_nothing() {
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::If {
            test: MirExpr::BoolLiteral(false),
            body: vec![MirStmt::ExprStmt(MirExpr::Call {
                callee: "print".to_string(),
                args: vec![MirExpr::IntLiteral(1)],
                ty: Ty::None,
            })],
            orelse: vec![],
        })],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("if_no_else").expect("failed to create scratch dir");
    let obj_path = dir.join("if_no_else.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("if_no_else");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"");
}

#[test]
fn compiles_a_while_loop_that_counts_down() {
    // `i = 3; while i > 0: print(i); i = i - 1` -- prints 3, 2, 1.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "i".to_string(),
                value: MirExpr::IntLiteral(3),
            }),
            MirItem::TopLevelStmt(MirStmt::While {
                test: MirExpr::Compare {
                    op: CmpOpKind::Gt,
                    left: Box::new(MirExpr::Name {
                        name: "i".to_string(),
                        ty: Ty::Int,
                    }),
                    right: Box::new(MirExpr::IntLiteral(0)),
                    ty: Ty::Bool,
                },
                body: vec![
                    MirStmt::ExprStmt(MirExpr::Call {
                        callee: "print".to_string(),
                        args: vec![MirExpr::Name {
                            name: "i".to_string(),
                            ty: Ty::Int,
                        }],
                        ty: Ty::None,
                    }),
                    MirStmt::Assign {
                        target: "i".to_string(),
                        value: MirExpr::BinOp {
                            op: BinOpKind::Sub,
                            left: Box::new(MirExpr::Name {
                                name: "i".to_string(),
                                ty: Ty::Int,
                            }),
                            right: Box::new(MirExpr::IntLiteral(1)),
                            ty: Ty::Int,
                        },
                    },
                ],
            }),
        ],
        class_defs: Vec::new(),
    };
    let dir =
        pycc_scratch::ScratchDir::new("while_countdown").expect("failed to create scratch dir");
    let obj_path = dir.join("while_countdown.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("while_countdown");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"3\n2\n1\n");
}

#[test]
fn compiles_a_while_loop_using_a_bare_int_condition_via_truthy() {
    // `i = 3; while i: print(i); i = i - 1` -- prints 3, 2, 1, same
    // countdown as the test above, but the loop test is a plain
    // `int`-typed `Name` (not a `Compare`), so this is the only test
    // in this file exercising `truthy`'s `Scalar::Int` arm (every
    // other `If`/`While` test's condition is a `Compare`, which always
    // evaluates to `Scalar::Bool`) -- `pycc_rt_int_truthy` genuinely
    // gets called from generated code here, not just unit-tested
    // directly in `pycc_rt`.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "i".to_string(),
                value: MirExpr::IntLiteral(3),
            }),
            MirItem::TopLevelStmt(MirStmt::While {
                test: MirExpr::Name {
                    name: "i".to_string(),
                    ty: Ty::Int,
                },
                body: vec![
                    MirStmt::ExprStmt(MirExpr::Call {
                        callee: "print".to_string(),
                        args: vec![MirExpr::Name {
                            name: "i".to_string(),
                            ty: Ty::Int,
                        }],
                        ty: Ty::None,
                    }),
                    MirStmt::Assign {
                        target: "i".to_string(),
                        value: MirExpr::BinOp {
                            op: BinOpKind::Sub,
                            left: Box::new(MirExpr::Name {
                                name: "i".to_string(),
                                ty: Ty::Int,
                            }),
                            right: Box::new(MirExpr::IntLiteral(1)),
                            ty: Ty::Int,
                        },
                    },
                ],
            }),
        ],
        class_defs: Vec::new(),
    };
    let dir =
        pycc_scratch::ScratchDir::new("while_int_truthy").expect("failed to create scratch dir");
    let obj_path = dir.join("while_int_truthy.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("while_int_truthy");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"3\n2\n1\n");
}

#[test]
fn emit_expr_evaluates_not_over_a_non_literal_int_operand() {
    // `i = 0; print(not i)` then `j = 5; print(not j)` -- prints `True`
    // then `False`. #604 (Part 3 of #573) adds `MirExpr::Not` as its own
    // dedicated `emit_expr` arm rather than folding into an existing
    // `BinOp`/`Compare` arm (unlike `~`, which rewrites to `-x - 1` at the
    // MIR level and so never needs a codegen arm of its own). The D-014
    // region gate scores every compilation of this crate separately, and
    // this arm was previously exercised only through the `pycc` binary and
    // the `tests/issue_604_unary_not_invert.rs` integration suite, never
    // through this crate's own `cfg(test)` build -- so this test, built
    // directly against `emit_expr`/`compile_to_object` the same way every
    // other test in this file is, is what actually reaches it there.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "i".to_string(),
                value: MirExpr::IntLiteral(0),
            }),
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
                callee: "print".to_string(),
                args: vec![MirExpr::Not(Box::new(MirExpr::Name {
                    name: "i".to_string(),
                    ty: Ty::Int,
                }))],
                ty: Ty::None,
            })),
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "j".to_string(),
                value: MirExpr::IntLiteral(5),
            }),
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
                callee: "print".to_string(),
                args: vec![MirExpr::Not(Box::new(MirExpr::Name {
                    name: "j".to_string(),
                    ty: Ty::Int,
                }))],
                ty: Ty::None,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir =
        pycc_scratch::ScratchDir::new("not_over_int_name").expect("failed to create scratch dir");
    let obj_path = dir.join("not_over_int_name.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("not_over_int_name");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"True\nFalse\n");
}

#[test]
fn a_while_loop_body_that_always_returns_skips_its_own_trailing_branch() {
    // `def f() -> int:\n    while True:\n        return 1\n    return 2`
    // ; `print(f())` -- must print `1`. The trailing `return 2` is
    // unreachable dead code. Since #1370 a bare `while True: return 1`
    // with nothing after it is accepted source too (the checker's
    // `pycc_hir::block_always_returns` accepts it,
    // and `fallthrough::block_always_terminates` is the codegen
    // counterpart that ends the block with `unreachable`); this shape,
    // with an explicit trailing `return`, remains valid source as well.
    // Distinct region from every other `while` test in this file, all
    // of whose *loop bodies* fall through normally and so always take
    // `emit_body_then_branch`'s own trailing
    // `build_unconditional_branch(test_bb)` back to the loop test: here
    // the loop body's own `return` already terminates it, so that
    // helper's terminator check must skip building a second (invalid)
    // terminator on top of it.
    let mir = MirModule {
        items: vec![
            MirItem::Function {
                name: "f".to_string(),
                params: vec![],
                return_ty: Ty::Int,
                body: vec![
                    MirStmt::While {
                        test: MirExpr::BoolLiteral(true),
                        body: vec![MirStmt::Return(Some(MirExpr::IntLiteral(1)))],
                    },
                    MirStmt::Return(Some(MirExpr::IntLiteral(2))),
                ],
            },
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
                callee: "print".to_string(),
                args: vec![MirExpr::Call {
                    callee: "f".to_string(),
                    args: vec![],
                    ty: Ty::Int,
                }],
                ty: Ty::None,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("while_body_always_returns")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("while_body_always_returns.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("while_body_always_returns");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"1\n");
}

#[test]
fn compiles_a_for_range_loop_with_a_positive_step() {
    // `for i in range(0, 6, 2): print(i)` -- prints 0, 2, 4.
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::ForRange {
            var: "i".to_string(),
            start: MirExpr::IntLiteral(0),
            stop: MirExpr::IntLiteral(6),
            step: MirExpr::IntLiteral(2),
            body: vec![MirStmt::ExprStmt(MirExpr::Call {
                callee: "print".to_string(),
                args: vec![MirExpr::Name {
                    name: "i".to_string(),
                    ty: Ty::Int,
                }],
                ty: Ty::None,
            })],
        })],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("for_range_pos").expect("failed to create scratch dir");
    let obj_path = dir.join("for_range_pos.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("for_range_pos");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"0\n2\n4\n");
}

#[test]
fn a_second_top_level_for_range_loop_reusing_a_loop_variable_name_is_not_redeclared() {
    // `for i in range(0, 2, 1): print(i)` followed by a second, separate
    // `for i in range(0, 3, 1): print(i)` -- both loops share the same
    // module-level loop variable name `i`. Exercises
    // `collect_stmt_bindings`'s `ForRange` arm with an already-present
    // `BTreeMap` entry: the second occurrence must not re-declare or
    // change the type of its global slot.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::ForRange {
                var: "i".to_string(),
                start: MirExpr::IntLiteral(0),
                stop: MirExpr::IntLiteral(2),
                step: MirExpr::IntLiteral(1),
                body: vec![MirStmt::ExprStmt(MirExpr::Call {
                    callee: "print".to_string(),
                    args: vec![MirExpr::Name {
                        name: "i".to_string(),
                        ty: Ty::Int,
                    }],
                    ty: Ty::None,
                })],
            }),
            MirItem::TopLevelStmt(MirStmt::ForRange {
                var: "i".to_string(),
                start: MirExpr::IntLiteral(0),
                stop: MirExpr::IntLiteral(3),
                step: MirExpr::IntLiteral(1),
                body: vec![MirStmt::ExprStmt(MirExpr::Call {
                    callee: "print".to_string(),
                    args: vec![MirExpr::Name {
                        name: "i".to_string(),
                        ty: Ty::Int,
                    }],
                    ty: Ty::None,
                })],
            }),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("for_range_reused_loop_var")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("for_range_reused_loop_var.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("for_range_reused_loop_var");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"0\n1\n0\n1\n2\n");
}

#[test]
fn compiles_a_for_range_loop_with_a_negative_step() {
    // `for i in range(3, 0, -1): print(i)` -- prints 3, 2, 1.
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::ForRange {
            var: "i".to_string(),
            start: MirExpr::IntLiteral(3),
            stop: MirExpr::IntLiteral(0),
            step: MirExpr::IntLiteral(-1),
            body: vec![MirStmt::ExprStmt(MirExpr::Call {
                callee: "print".to_string(),
                args: vec![MirExpr::Name {
                    name: "i".to_string(),
                    ty: Ty::Int,
                }],
                ty: Ty::None,
            })],
        })],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("for_range_neg").expect("failed to create scratch dir");
    let obj_path = dir.join("for_range_neg.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("for_range_neg");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"3\n2\n1\n");
}

#[test]
#[should_panic(expected = "range() start did not evaluate to int")]
fn for_range_with_a_non_int_start_is_rejected() {
    // `bool` is accepted as an `int` subtype; float is not. Hand-built
    // malformed MIR exercises the defensive backend check directly.
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::ForRange {
            var: "i".to_string(),
            start: MirExpr::FloatLiteral(1.0),
            stop: MirExpr::IntLiteral(3),
            step: MirExpr::IntLiteral(1),
            body: vec![],
        })],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("for_range_bad_start_panics")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("for_range_bad_start_panics.o");
    let _ = compile_to_object(&mir, &obj_path, None, false);
}

#[test]
#[should_panic(expected = "range() stop did not evaluate to int")]
fn for_range_with_a_non_int_stop_is_rejected() {
    // Distinct region from the `start` case above.
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::ForRange {
            var: "i".to_string(),
            start: MirExpr::IntLiteral(0),
            stop: MirExpr::StringLiteral("3".to_string()),
            step: MirExpr::IntLiteral(1),
            body: vec![],
        })],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("for_range_bad_stop_panics")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("for_range_bad_stop_panics.o");
    let _ = compile_to_object(&mir, &obj_path, None, false);
}

#[test]
#[should_panic(expected = "range() step did not evaluate to int")]
fn for_range_with_a_non_int_step_is_rejected() {
    // Distinct region from the `start`/`stop` cases above.
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::ForRange {
            var: "i".to_string(),
            start: MirExpr::IntLiteral(0),
            stop: MirExpr::IntLiteral(3),
            step: MirExpr::FloatLiteral(1.0),
            body: vec![],
        })],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("for_range_bad_step_panics")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("for_range_bad_step_panics.o");
    let _ = compile_to_object(&mir, &obj_path, None, false);
}

#[test]
fn for_range_with_a_bool_start_stop_and_step_all_widen_to_int() {
    // `for i in range(True, 4, True): print(i)` -- `bool` is an `int`
    // subtype (`pycc_types::is_assignable`), and
    // `a_for_range_loop_accepts_bool_as_an_int_subtype` proves
    // `pycc_types` genuinely accepts a bool-typed `range()` argument for
    // any of its three positions, so this reaches codegen with
    // `Scalar::Bool` `start`/`stop`/`step` operands (`stop` is `4`, a
    // plain int literal, to keep this a short, checkable loop; `start`
    // and `step` are both `True` to exercise both of that arm's other
    // two match sites). Before this fix, each position's
    // `let Scalar::Int(..) = ... else { panic!(...) }` destructure
    // rejected any non-`Int` scalar outright, crashing the compiler on
    // this legitimate, accepted program instead of applying range's
    // numeric normalization from `True` to the ordinary tagged int `1`.
    // Identity-preserving int boundaries intentionally keep D-141's
    // encoded `True` marker instead.
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::ForRange {
            var: "i".to_string(),
            start: MirExpr::BoolLiteral(true),
            stop: MirExpr::IntLiteral(4),
            step: MirExpr::BoolLiteral(true),
            body: vec![MirStmt::ExprStmt(MirExpr::Call {
                callee: "print".to_string(),
                args: vec![MirExpr::Name {
                    name: "i".to_string(),
                    ty: Ty::Int,
                }],
                ty: Ty::None,
            })],
        })],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("for_range_bool_start_stop_step")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("for_range_bool_start_stop_step.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("for_range_bool_start_stop_step");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"1\n2\n3\n");
}

#[test]
fn for_range_with_a_bool_stop_widens_to_int() {
    // `for i in range(0, True, 1): print(i)` -- distinct region from
    // the `start`/`step` coverage above: exercises `stop`'s own
    // `range_operand_to_normalized_int`'s `Scalar::Bool` arm specifically.
    // `True` normalizes to the ordinary tagged int `1`, so this loop runs once.
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::ForRange {
            var: "i".to_string(),
            start: MirExpr::IntLiteral(0),
            stop: MirExpr::BoolLiteral(true),
            step: MirExpr::IntLiteral(1),
            body: vec![MirStmt::ExprStmt(MirExpr::Call {
                callee: "print".to_string(),
                args: vec![MirExpr::Name {
                    name: "i".to_string(),
                    ty: Ty::Int,
                }],
                ty: Ty::None,
            })],
        })],
        class_defs: Vec::new(),
    };
    let dir =
        pycc_scratch::ScratchDir::new("for_range_bool_stop").expect("failed to create scratch dir");
    let obj_path = dir.join("for_range_bool_stop.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("for_range_bool_stop");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"0\n");
}

#[test]
fn for_range_normalizes_int_typed_bool_markers_before_the_induction_phi() {
    // Unlike the literal-bool range tests above, both helpers return an
    // `i64`-carried `Ty::Int`: their bool results have already crossed
    // the return boundary and become D-141 markers before `ForRange`
    // sees them. All three operands therefore exercise
    // `range_operand_to_normalized_int`'s `Scalar::Int` path. The first
    // visible target must print ordinary integer `0`, not `False`.
    let mir = MirModule {
        items: vec![
            MirItem::Function {
                name: "false_int".to_string(),
                params: vec![],
                return_ty: Ty::Int,
                body: vec![MirStmt::Return(Some(MirExpr::BoolLiteral(false)))],
            },
            MirItem::Function {
                name: "true_int".to_string(),
                params: vec![],
                return_ty: Ty::Int,
                body: vec![MirStmt::Return(Some(MirExpr::BoolLiteral(true)))],
            },
            MirItem::TopLevelStmt(MirStmt::ForRange {
                var: "i".to_string(),
                start: MirExpr::Call {
                    callee: "false_int".to_string(),
                    args: vec![],
                    ty: Ty::Int,
                },
                stop: MirExpr::Call {
                    callee: "true_int".to_string(),
                    args: vec![],
                    ty: Ty::Int,
                },
                step: MirExpr::Call {
                    callee: "true_int".to_string(),
                    args: vec![],
                    ty: Ty::Int,
                },
                body: vec![MirStmt::ExprStmt(MirExpr::Call {
                    callee: "print".to_string(),
                    args: vec![MirExpr::Name {
                        name: "i".to_string(),
                        ty: Ty::Int,
                    }],
                    ty: Ty::None,
                })],
            }),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("for_range_encoded_bool_markers")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("for_range_encoded_bool_markers.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("for_range_encoded_bool_markers");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"0\n");
}

#[test]
fn compiles_nested_control_flow_with_a_statement_after_it_in_the_same_body() {
    // `for i in range(0, 3, 1): (if i == 1: print(100)); print(i)` --
    // exercises two things no other test in this file does: control
    // flow (`If`) nested inside other control flow (`ForRange`), and a
    // statement following a control-flow statement in the *same*
    // `body` list. Every other test's `If`/`While`/`ForRange` is the
    // last statement of its enclosing body -- so nothing else proves
    // that `emit_stmt`'s `If` arm correctly leaves the builder
    // positioned at `merge_bb` in a state where a *subsequent*
    // statement resumes into it correctly (right `locals`, right
    // block, no invalid IR from double-terminating or orphaning a
    // block) -- exactly the invariant `emit_body`'s own doc comment
    // relies on to justify never needing an early-terminator-stop
    // check in Task 4's scope. Expected: i=0 -> "0"; i=1 -> "100" then
    // "1"; i=2 -> "2".
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::ForRange {
            var: "i".to_string(),
            start: MirExpr::IntLiteral(0),
            stop: MirExpr::IntLiteral(3),
            step: MirExpr::IntLiteral(1),
            body: vec![
                MirStmt::If {
                    test: MirExpr::Compare {
                        op: CmpOpKind::Eq,
                        left: Box::new(MirExpr::Name {
                            name: "i".to_string(),
                            ty: Ty::Int,
                        }),
                        right: Box::new(MirExpr::IntLiteral(1)),
                        ty: Ty::Bool,
                    },
                    body: vec![MirStmt::ExprStmt(MirExpr::Call {
                        callee: "print".to_string(),
                        args: vec![MirExpr::IntLiteral(100)],
                        ty: Ty::None,
                    })],
                    orelse: vec![],
                },
                MirStmt::ExprStmt(MirExpr::Call {
                    callee: "print".to_string(),
                    args: vec![MirExpr::Name {
                        name: "i".to_string(),
                        ty: Ty::Int,
                    }],
                    ty: Ty::None,
                }),
            ],
        })],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("nested_control_flow_resume")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("nested_control_flow_resume.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("nested_control_flow_resume");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"0\n100\n1\n2\n");
}

// The four tests below are not in the brief's own Step 3 list -- added
// because `cargo llvm-cov`'s region coverage showed each of `If`'s
// `then`/`orelse` arms, `While`'s body, and `ForRange`'s body has its
// own distinct `?`-propagation region (one per `emit_body`/`emit_body_
// then_branch` call site inside `emit_stmt`, not shared across arms):
// every prior test's nested body only ever contains statements that
// succeed, so none of these four `?` operators had ever actually
// propagated an `Err`. Mirrors the existing top-level/function-body
// `calling_an_undefined_function_..._is_rejected` tests above, just
// with the undefined call nested one level deeper.
#[test]
fn calling_an_undefined_function_inside_an_if_then_body_is_rejected() {
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::If {
            test: MirExpr::BoolLiteral(true),
            body: vec![call_user_fn("does_not_exist_in_if_then")],
            orelse: vec![],
        })],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("if_then_undefined_fn")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("if_then_undefined_fn.o");
    let err = compile_to_object(&mir, &obj_path, None, false).expect_err("should be rejected");
    assert!(
        err.contains("does_not_exist_in_if_then"),
        "error should name the offending function: {err}"
    );
}

#[test]
fn calling_an_undefined_function_inside_an_if_orelse_body_is_rejected() {
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::If {
            test: MirExpr::BoolLiteral(false),
            body: vec![],
            orelse: vec![call_user_fn("does_not_exist_in_if_orelse")],
        })],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("if_orelse_undefined_fn")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("if_orelse_undefined_fn.o");
    let err = compile_to_object(&mir, &obj_path, None, false).expect_err("should be rejected");
    assert!(
        err.contains("does_not_exist_in_if_orelse"),
        "error should name the offending function: {err}"
    );
}

#[test]
fn calling_an_undefined_function_inside_a_while_body_is_rejected() {
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::While {
            test: MirExpr::BoolLiteral(true),
            body: vec![call_user_fn("does_not_exist_in_while")],
        })],
        class_defs: Vec::new(),
    };
    let dir =
        pycc_scratch::ScratchDir::new("while_undefined_fn").expect("failed to create scratch dir");
    let obj_path = dir.join("while_undefined_fn.o");
    let err = compile_to_object(&mir, &obj_path, None, false).expect_err("should be rejected");
    assert!(
        err.contains("does_not_exist_in_while"),
        "error should name the offending function: {err}"
    );
}

#[test]
fn calling_an_undefined_function_inside_a_for_range_body_is_rejected() {
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::ForRange {
            var: "i".to_string(),
            start: MirExpr::IntLiteral(0),
            stop: MirExpr::IntLiteral(3),
            step: MirExpr::IntLiteral(1),
            body: vec![call_user_fn("does_not_exist_in_for_range")],
        })],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("for_range_undefined_fn")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("for_range_undefined_fn.o");
    let err = compile_to_object(&mir, &obj_path, None, false).expect_err("should be rejected");
    assert!(
        err.contains("does_not_exist_in_for_range"),
        "error should name the offending function: {err}"
    );
}
