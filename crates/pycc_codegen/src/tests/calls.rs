//! User function calls: argument marshalling, return values and container returns.

use super::*;

#[test]
fn compiles_a_function_call_with_real_arguments_and_a_return_value() {
    // `def add(a: int, b: int) -> int: return a + b` ; `print(add(2, 3))`
    let mir = MirModule {
        items: vec![
            MirItem::Function {
                name: "add".to_string(),
                params: vec![("a".to_string(), Ty::Int), ("b".to_string(), Ty::Int)],
                return_ty: Ty::Int,
                body: vec![MirStmt::Return(Some(MirExpr::BinOp {
                    op: BinOpKind::Add,
                    left: Box::new(MirExpr::Name {
                        name: "a".to_string(),
                        ty: Ty::Int,
                    }),
                    right: Box::new(MirExpr::Name {
                        name: "b".to_string(),
                        ty: Ty::Int,
                    }),
                    ty: Ty::Int,
                }))],
            },
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
                callee: "print".to_string(),
                args: vec![MirExpr::Call {
                    callee: "add".to_string(),
                    args: vec![MirExpr::IntLiteral(2), MirExpr::IntLiteral(3)],
                    ty: Ty::Int,
                }],
                ty: Ty::None,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir =
        pycc_scratch::ScratchDir::new("call_with_args").expect("failed to create scratch dir");
    let obj_path = dir.join("call_with_args.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("call_with_args");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(pycc_scratch::lf(&output.stdout), b"5\n");
}

#[test]
fn a_multi_argument_call_binds_each_parameter_in_the_right_order() {
    // `def sub(a: int, b: int) -> int: return a - b` ; `print(sub(10, 3))`
    // -- `add`'s own test above is commutative (`2 + 3 == 3 + 2`), so it
    // can't tell a correct argument-to-parameter binding apart from a
    // transposed one (`get_nth_param(i)` bound to the wrong
    // `param_name`, or `build_call_to` marshaling `args` out of order).
    // `sub` isn't commutative: `10 - 3 == 7`, but the transposed
    // binding would compute `3 - 10 == -7` instead. Prints "7", not
    // "-7".
    let mir = MirModule {
        items: vec![
            MirItem::Function {
                name: "sub".to_string(),
                params: vec![("a".to_string(), Ty::Int), ("b".to_string(), Ty::Int)],
                return_ty: Ty::Int,
                body: vec![MirStmt::Return(Some(MirExpr::BinOp {
                    op: BinOpKind::Sub,
                    left: Box::new(MirExpr::Name {
                        name: "a".to_string(),
                        ty: Ty::Int,
                    }),
                    right: Box::new(MirExpr::Name {
                        name: "b".to_string(),
                        ty: Ty::Int,
                    }),
                    ty: Ty::Int,
                }))],
            },
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
                callee: "print".to_string(),
                args: vec![MirExpr::Call {
                    callee: "sub".to_string(),
                    args: vec![MirExpr::IntLiteral(10), MirExpr::IntLiteral(3)],
                    ty: Ty::Int,
                }],
                ty: Ty::None,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir =
        pycc_scratch::ScratchDir::new("call_arg_order").expect("failed to create scratch dir");
    let obj_path = dir.join("call_arg_order.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("call_arg_order");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(pycc_scratch::lf(&output.stdout), b"7\n");
}

#[test]
fn compiles_a_recursive_function_with_an_early_return() {
    // `def fact(n: int) -> int:\n    if n <= 1:\n        return 1\n    return n * fact(n - 1)`
    // `print(fact(5))` -- exercises recursion (calling `fact` from inside
    // its own not-yet-fully-emitted body works because the two-pass
    // declare-then-define structure already declares every function
    // before any body is compiled), a return nested inside an `if` with
    // no `else`, and a second `return` reached only via that `if`'s false
    // edge (Task 4's `merge_bb` handling).
    let fact_body = vec![
        MirStmt::If {
            test: MirExpr::Compare {
                op: CmpOpKind::LtE,
                left: Box::new(MirExpr::Name {
                    name: "n".to_string(),
                    ty: Ty::Int,
                }),
                right: Box::new(MirExpr::IntLiteral(1)),
                ty: Ty::Bool,
            },
            body: vec![MirStmt::Return(Some(MirExpr::IntLiteral(1)))],
            orelse: vec![],
        },
        MirStmt::Return(Some(MirExpr::BinOp {
            op: BinOpKind::Mul,
            left: Box::new(MirExpr::Name {
                name: "n".to_string(),
                ty: Ty::Int,
            }),
            right: Box::new(MirExpr::Call {
                callee: "fact".to_string(),
                args: vec![MirExpr::BinOp {
                    op: BinOpKind::Sub,
                    left: Box::new(MirExpr::Name {
                        name: "n".to_string(),
                        ty: Ty::Int,
                    }),
                    right: Box::new(MirExpr::IntLiteral(1)),
                    ty: Ty::Int,
                }],
                ty: Ty::Int,
            }),
            ty: Ty::Int,
        })),
    ];
    let mir = MirModule {
        items: vec![
            MirItem::Function {
                name: "fact".to_string(),
                params: vec![("n".to_string(), Ty::Int)],
                return_ty: Ty::Int,
                body: fact_body,
            },
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
                callee: "print".to_string(),
                args: vec![MirExpr::Call {
                    callee: "fact".to_string(),
                    args: vec![MirExpr::IntLiteral(5)],
                    ty: Ty::Int,
                }],
                ty: Ty::None,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir =
        pycc_scratch::ScratchDir::new("recursive_fact").expect("failed to create scratch dir");
    let obj_path = dir.join("recursive_fact.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("recursive_fact");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(pycc_scratch::lf(&output.stdout), b"120\n");
}

#[test]
fn a_function_returning_from_both_if_and_else_branches_compiles_and_runs() {
    // `def f(x: int) -> int:\n    if x > 0:\n        return 1\n    else:\n        return 2`
    // Every real path through `f` returns, so this is legal, ordinary
    // Python -- but before this fix, `MirStmt::If`'s codegen
    // unconditionally positioned the builder at `if_merge` after
    // emitting both branches, even when both had already terminated via
    // `return` (leaving `if_merge` an unreachable block with zero
    // predecessors and no terminator of its own). `emit_body`'s caller
    // (here, `compile_to_object`'s own end-of-function fallthrough
    // check) then saw a terminator-less current block and raised its
    // own "fell through without a `return`" internal-error panic --
    // a false positive for a function that provably always returns.
    let mir = MirModule {
        items: vec![
            MirItem::Function {
                name: "f".to_string(),
                params: vec![("x".to_string(), Ty::Int)],
                return_ty: Ty::Int,
                body: vec![MirStmt::If {
                    test: MirExpr::Compare {
                        op: CmpOpKind::Gt,
                        left: Box::new(MirExpr::Name {
                            name: "x".to_string(),
                            ty: Ty::Int,
                        }),
                        right: Box::new(MirExpr::IntLiteral(0)),
                        ty: Ty::Bool,
                    },
                    body: vec![MirStmt::Return(Some(MirExpr::IntLiteral(1)))],
                    orelse: vec![MirStmt::Return(Some(MirExpr::IntLiteral(2)))],
                }],
            },
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
                callee: "print".to_string(),
                args: vec![MirExpr::Call {
                    callee: "f".to_string(),
                    args: vec![MirExpr::IntLiteral(5)],
                    ty: Ty::Int,
                }],
                ty: Ty::None,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir =
        pycc_scratch::ScratchDir::new("if_else_both_return").expect("failed to create scratch dir");
    let obj_path = dir.join("if_else_both_return.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("if_else_both_return");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(pycc_scratch::lf(&output.stdout), b"1\n");
}

#[test]
fn a_non_none_function_falling_through_is_an_internal_error_not_bad_ir() {
    // `pycc_types`' T0024 fallthrough check should have rejected this
    // HIR already -- this proves codegen fails loudly (a clear panic)
    // rather than emitting an invalid `ret` from a function declared to
    // return `int`, if that check is ever somehow bypassed.
    let mir = MirModule {
        items: vec![MirItem::Function {
            name: "broken".to_string(),
            params: vec![],
            return_ty: Ty::Int,
            body: vec![],
        }],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("fallthrough_internal_error")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("fallthrough_internal_error.o");
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        compile_to_object(&mir, &obj_path, None, false)
    }));
    assert!(
        result.is_err(),
        "expected a panic, not a successfully-compiled object"
    );
}

#[test]
fn a_non_none_function_whose_try_raise_finally_body_always_terminates_compiles_cleanly() {
    // `block_always_terminates`'s own doc comment gives this exact shape
    // as its motivating example: `try: raise ... finally: cleanup`. The
    // `finally` block's own codegen exits back into a continuation block
    // that carries no LLVM terminator of its own -- structurally the
    // function body always terminates (the `try`'s only path raises),
    // but nothing has yet placed a `ret`/`unreachable` in that trailing
    // block. This exercises the `fallthrough::block_always_terminates(body)
    // == true` branch's `builder.build_unreachable()` call directly
    // (distinct from `a_non_none_function_falling_through_is_an_internal_
    // error_not_bad_ir` above, which covers the sibling `false` branch --
    // an ordinary fallthrough with no raise at all).
    let mir = MirModule {
        items: vec![MirItem::Function {
            name: "always_raises".to_string(),
            params: vec![],
            return_ty: Ty::Int,
            body: vec![MirStmt::Try {
                body: vec![MirStmt::Raise {
                    exception: MirExceptionValue::Constructed {
                        type_tag: 1,
                        class_name: "ValueError".to_string(),
                        message: MirExpr::StringLiteral("boom".to_string()),
                    },
                    frame_function: "test_fn".to_string(),
                }],
                handlers: vec![],
                orelse: vec![],
                finalbody: vec![print_expr(MirExpr::IntLiteral(9))],
            }],
        }],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("try_raise_finally_always_terminates")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("try_raise_finally_always_terminates.o");
    compile_to_object(&mir, &obj_path, None, false).expect(
        "a try/raise/finally body that always terminates must produce valid object code, \
         not panic on a missing terminator",
    );
}

#[test]
fn a_top_level_return_is_an_internal_error_not_bad_ir() {
    // `pycc_types`' T0024 rejects any module-level `return` already (even
    // nested in a top-level `if`/`while`/`for`) -- this proves codegen
    // fails loudly (a clear panic) rather than emitting a second
    // terminator into `main`'s entry block, which is invalid IR that
    // `module.verify()` cannot catch on Windows (D-029's no-op), if that
    // check is ever somehow bypassed. Mirrors
    // `a_non_none_function_falling_through_is_an_internal_error_not_bad_ir`
    // above for the per-function analogue of the same guard.
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::Return(Some(
            MirExpr::IntLiteral(0),
        )))],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("top_level_return_internal_error")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("top_level_return_internal_error.o");
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        compile_to_object(&mir, &obj_path, None, false)
    }));
    assert!(
        result.is_err(),
        "expected a panic, not a successfully-compiled object"
    );
}

#[test]
#[should_panic(expected = "internal error: call to undefined function")]
fn calling_an_undefined_function_as_a_nested_expression_is_an_internal_error() {
    // Unlike a bare statement-level call (see `calling_an_undefined_
    // function_at_top_level_is_rejected` and its siblings above, which
    // still return a clean `Result::Err` -- `emit_stmt`'s void-call
    // arm generalizes this crate's pre-Task-5 zero-arg-only behavior
    // rather than switching to a panic), a call used *inside* another
    // expression flows through `emit_expr`'s `Call` arm, which returns
    // a `Scalar`, not a `Result` -- there is no way to propagate a
    // graceful error from there. Real `pycc_types` already rejects any
    // call to an undefined function (T0021) long before codegen runs,
    // so this is this crate's own defensive "should never happen"
    // backstop, not a rejection of legitimate source.
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::Assign {
            target: "x".to_string(),
            value: MirExpr::Call {
                callee: "does_not_exist_as_expr".to_string(),
                args: vec![],
                ty: Ty::Int,
            },
        })],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("undefined_fn_nested_expr_panics")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("undefined_fn_nested_expr_panics.o");
    let _ = compile_to_object(&mir, &obj_path, None, false);
}

#[test]
fn compiles_a_function_call_returning_bool_used_as_an_expression() {
    // `def is_positive(n: int) -> bool: return n > 0` ;
    // `x = is_positive(5)` -- the brief's own Step 1 tests only ever
    // exercise `emit_expr`'s `Call` arm's `Ty::Int` branch; this
    // exercises its `Ty::Bool` branch instead.
    let mir = MirModule {
        items: vec![
            MirItem::Function {
                name: "is_positive".to_string(),
                params: vec![("n".to_string(), Ty::Int)],
                return_ty: Ty::Bool,
                body: vec![MirStmt::Return(Some(MirExpr::Compare {
                    op: CmpOpKind::Gt,
                    left: Box::new(MirExpr::Name {
                        name: "n".to_string(),
                        ty: Ty::Int,
                    }),
                    right: Box::new(MirExpr::IntLiteral(0)),
                    ty: Ty::Bool,
                }))],
            },
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "x".to_string(),
                value: MirExpr::Call {
                    callee: "is_positive".to_string(),
                    args: vec![MirExpr::IntLiteral(5)],
                    ty: Ty::Bool,
                },
            }),
        ],
        class_defs: Vec::new(),
    };
    let dir =
        pycc_scratch::ScratchDir::new("call_returns_bool").expect("failed to create scratch dir");
    let obj_path = dir.join("call_returns_bool.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
}

#[test]
fn a_none_typed_call_result_can_be_stored_and_printed() {
    let mir = MirModule {
        items: vec![
            MirItem::Function {
                name: "f".to_string(),
                params: vec![],
                return_ty: Ty::None,
                body: vec![MirStmt::Return(None)],
            },
            MirItem::Function {
                name: "store".to_string(),
                params: vec![],
                return_ty: Ty::None,
                body: vec![
                    MirStmt::Assign {
                        target: "x".to_string(),
                        value: MirExpr::Call {
                            callee: "f".to_string(),
                            args: vec![],
                            ty: Ty::None,
                        },
                    },
                    MirStmt::ExprStmt(MirExpr::Call {
                        callee: "print".to_string(),
                        args: vec![MirExpr::Name {
                            name: "x".to_string(),
                            ty: Ty::None,
                        }],
                        ty: Ty::None,
                    }),
                ],
            },
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
                callee: "store".to_string(),
                args: vec![],
                ty: Ty::None,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("none_typed_call_result_storage")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("none_typed_call_result_storage.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("none_typed_call_result_storage");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert!(output.status.success());
    assert_eq!(pycc_scratch::lf(&output.stdout), b"None\n");
}

#[test]
#[should_panic(expected = "a `<inferred>`-typed call result is not supported yet")]
fn an_infer_typed_call_result_used_as_a_nested_expression_is_not_supported() {
    // Exercises the first of the three unsupported variants named by
    // `call_result_scalar`'s explicit rejection arm. #925 replaced that
    // function's catch-all with `Ty::Infer | Ty::Param(_) |
    // Ty::Protocol(_)` and no `_ =>` fallback, so the compiler -- not a
    // comment -- guarantees every other variant has a real arm; the
    // other two are pinned by the two tests below. `Ty::Infer` is an
    // HIR-only inference placeholder no real MIR ever carries this far,
    // same rationale as `ty_to_basic_type`'s own
    // `an_infer_typed_return_value_is_not_yet_supported` test above.
    //
    // Task 5 (D-089) updated this catch-all to name the type via
    // `Ty::name()` instead of a bare `{:?}`, so `Ty::Infer` renders as
    // `<inferred>` here now -- same panic, updated wording.
    let mir = MirModule {
        items: vec![
            MirItem::Function {
                name: "f".to_string(),
                params: vec![],
                return_ty: Ty::Int,
                body: vec![MirStmt::Return(Some(MirExpr::IntLiteral(1)))],
            },
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "x".to_string(),
                value: MirExpr::Call {
                    callee: "f".to_string(),
                    args: vec![],
                    ty: Ty::Infer,
                },
            }),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("infer_typed_call_result_panics")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("infer_typed_call_result_panics.o");
    let _ = compile_to_object(&mir, &obj_path, None, false);
}

/// The second and third variants named by `call_result_scalar`'s explicit
/// rejection arm. Neither is reachable from real source in return position --
/// `T0042` rejects a type parameter inside a container annotation and a bare
/// `Ty::Param` call result is substituted at the call site under D-134, and a
/// protocol name is not a constructible return value -- so both are pinned at
/// the MIR level, exactly as `Ty::Infer` is above. Written as one test per
/// variant rather than one test for the whole or-pattern so that the arm's
/// coverage does not depend on how llvm-cov counts an or-pattern's regions.

#[test]
#[should_panic(expected = "a `T`-typed call result is not supported yet")]
fn a_param_typed_call_result_is_not_supported() {
    let mir = MirModule {
        items: vec![
            MirItem::Function {
                name: "f".to_string(),
                params: vec![],
                return_ty: Ty::Int,
                body: vec![MirStmt::Return(Some(MirExpr::IntLiteral(1)))],
            },
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "x".to_string(),
                value: MirExpr::Call {
                    callee: "f".to_string(),
                    args: vec![],
                    ty: Ty::Param(Box::new("T".to_string())),
                },
            }),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("param_typed_call_result_panics")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("param_typed_call_result_panics.o");
    let _ = compile_to_object(&mir, &obj_path, None, false);
}

#[test]
#[should_panic(expected = "a `P`-typed call result is not supported yet")]
fn a_protocol_typed_call_result_is_not_supported() {
    // Hand-built MIR is the only way to reach this arm since #934:
    // `pycc_hir` rejects a `-> P` annotation with `C0001`, so no
    // type-checked source produces a protocol-typed call result. The test
    // stays because the arm stays -- it is the compile-time-exhaustive
    // rejection of a `Ty` variant codegen has no representation for.
    let mir = MirModule {
        items: vec![
            MirItem::Function {
                name: "f".to_string(),
                params: vec![],
                return_ty: Ty::Int,
                body: vec![MirStmt::Return(Some(MirExpr::IntLiteral(1)))],
            },
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "x".to_string(),
                value: MirExpr::Call {
                    callee: "f".to_string(),
                    args: vec![],
                    ty: Ty::Protocol(Box::new("P".to_string())),
                },
            }),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("protocol_typed_call_result_panics")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("protocol_typed_call_result_panics.o");
    let _ = compile_to_object(&mir, &obj_path, None, false);
}

#[test]
#[should_panic(expected = "<inferred> has no LLVM representation yet")]
fn an_infer_typed_return_value_is_not_yet_supported() {
    // `ty_to_basic_type` now implements `Int`/`Bool`/`Float`/`Str`
    // (Task 7 closed the `Str` gap this test's earlier, Task 3-era
    // incarnation exercised -- see
    // `compiles_a_function_with_a_str_parameter_and_str_return_value`
    // below) plus `None`/`List(_)` (Task 5, D-089). `Ty::None` can't
    // stand in for "still unhandled" here: `compile_to_object`'s own
    // `return_ty` match special-cases `Ty::None` into
    // `void_type().fn_type(...)` *before* `ty_to_basic_type` is ever
    // called for a return type (see that match's own `Ty::None` arm)
    // -- `Ty::Infer` (an HIR-only inference placeholder no real MIR
    // ever carries this far) is the one `Ty` variant left that still
    // reaches `ty_to_basic_type`'s own defensive catch-all from the
    // return-type position.
    //
    // Task 5 (D-089) also rewrote this catch-all's whole message (not
    // just the type-name formatting) to "{name} has no LLVM
    // representation yet". Since PR-11b Task 5 gave `Ty::Tuple` a real
    // arm, `Ty::Infer` is the only variant that still reaches the
    // catch-all, so this is now the sole test pinning that wording.
    // The `expected` string here pins the rendered
    // `<inferred>` name too (`Ty::Infer`'s own `.name()` text), not
    // just the trailing message, for the same reason those two do.
    let mir = MirModule {
        items: vec![MirItem::Function {
            name: "f".to_string(),
            params: vec![],
            return_ty: Ty::Infer,
            body: vec![],
        }],
        class_defs: Vec::new(),
    };
    let dir =
        pycc_scratch::ScratchDir::new("infer_return_panics").expect("failed to create scratch dir");
    let obj_path = dir.join("infer_return_panics.o");
    let _ = compile_to_object(&mir, &obj_path, None, false);
}

// --- #925 (Part 2 of #918): container-typed call results -------------------

/// Builds `def make() -> <ty>: return <literal>` followed by a top-level
/// `x = make()` and one `print(...)` of `probe(x)`, then compiles, links and
/// runs it, returning the program's stdout.
///
/// These four tests live inline in the crate rather than only in
/// `tests/issue_925_container_returns.rs` on purpose: llvm-cov takes the
/// maximum over instantiations rather than their union, so an arm reached only
/// from an integration test can still leave a region uncovered (the #603
/// precedent recorded in `docs/TESTING.md`). Each one is what actually pins
/// its `call_result_scalar` arm.
fn run_container_return(label: &str, ty: Ty, literal: MirExpr, probe: MirExpr) -> Vec<u8> {
    let mir = MirModule {
        items: vec![
            MirItem::Function {
                name: "make".to_string(),
                params: vec![],
                return_ty: ty.clone(),
                body: vec![MirStmt::Return(Some(literal))],
            },
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "x".to_string(),
                value: MirExpr::Call {
                    callee: "make".to_string(),
                    args: vec![],
                    ty,
                },
            }),
            MirItem::TopLevelStmt(print_expr(probe)),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new(label).expect("failed to create scratch dir");
    let obj_path = dir.join(format!("{label}.o"));
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join(label);
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert!(output.status.success(), "{label} should exit successfully");
    pycc_scratch::lf(&output.stdout)
}

#[test]
fn a_list_typed_call_result_is_returned_as_a_pointer() {
    // `call_result_scalar`'s `Ty::List` arm: the call site's basic value is
    // the same opaque pointer a `list[int]` name carries, so `len(x)` on the
    // returned value works without any conversion at the seam.
    let stdout = run_container_return(
        "call_returns_list",
        Ty::List(Box::new(Ty::Int)),
        MirExpr::ListLiteral(vec![
            MirExpr::IntLiteral(1),
            MirExpr::IntLiteral(2),
            MirExpr::IntLiteral(3),
        ]),
        MirExpr::Call {
            callee: "len".to_string(),
            args: vec![MirExpr::Name {
                name: "x".to_string(),
                ty: Ty::List(Box::new(Ty::Int)),
            }],
            ty: Ty::Int,
        },
    );
    assert_eq!(stdout, b"3\n");
}

#[test]
fn a_dict_typed_call_result_is_returned_as_a_pointer() {
    // `call_result_scalar`'s `Ty::Dict` arm.
    let stdout = run_container_return(
        "call_returns_dict",
        dict_str_int(),
        MirExpr::DictLiteral(vec![(
            MirExpr::StringLiteral("a".to_string()),
            MirExpr::IntLiteral(7),
        )]),
        MirExpr::Call {
            callee: "len".to_string(),
            args: vec![dict_name("x")],
            ty: Ty::Int,
        },
    );
    assert_eq!(stdout, b"1\n");
}

#[test]
fn a_set_typed_call_result_is_returned_as_a_pointer() {
    // `call_result_scalar`'s `Ty::Set` arm.
    let stdout = run_container_return(
        "call_returns_set",
        set_int(),
        MirExpr::SetLiteral {
            elements: vec![MirExpr::IntLiteral(4), MirExpr::IntLiteral(5)],
            ops: None,
        },
        MirExpr::Call {
            callee: "len".to_string(),
            args: vec![set_name("x")],
            ty: Ty::Int,
        },
    );
    assert_eq!(stdout, b"2\n");
}

#[test]
fn a_tuple_typed_call_result_is_returned_as_a_struct() {
    // `call_result_scalar`'s `Ty::Tuple` arm -- the one family whose result
    // is `.into_struct_value()` rather than `.into_pointer_value()` (D-115's
    // by-value SSA struct), so it is the arm a copy-paste of the other three
    // would get wrong.
    let tuple_ty = Ty::Tuple(Box::new(vec![Ty::Int, Ty::Int]));
    let stdout = run_container_return(
        "call_returns_tuple",
        tuple_ty.clone(),
        MirExpr::TupleLiteral(vec![MirExpr::IntLiteral(8), MirExpr::IntLiteral(9)]),
        MirExpr::Subscript {
            base: Box::new(MirExpr::Name {
                name: "x".to_string(),
                ty: tuple_ty,
            }),
            index: Box::new(MirExpr::IntLiteral(1)),
        },
    );
    assert_eq!(stdout, b"9\n");
}
