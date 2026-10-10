//! `try`/`except`/`else`/`finally` routing, `raise from` and handler dispatch.

use super::*;

#[test]
fn raise_from_with_non_string_message_is_a_codegen_error() {
    // Tests the error path for a non-string raise message in RaiseFrom.
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::RaiseFrom {
            exception: MirExceptionValue::Constructed {
                type_tag: 1,
                class_name: "ValueError".to_string(),
                message: MirExpr::IntLiteral(42),
            },
            cause: MirExceptionValue::Constructed {
                type_tag: 2,
                class_name: "TypeError".to_string(),
                message: MirExpr::StringLiteral("cause".to_string()),
            },
            frame_function: "test_fn".to_string(),
        })],
        class_defs: Vec::new(),
    };
    let dir =
        pycc_scratch::ScratchDir::new("raise_from_non_str").expect("failed to create scratch dir");
    let obj_path = dir.join("raise_from_non_str.o");
    let result = compile_to_object(&mir, &obj_path, None, false);
    assert!(
        result.is_err(),
        "codegen should fail for non-string raise message"
    );
}

#[test]
fn raise_from_with_non_string_cause_is_a_codegen_error() {
    // Tests the error path for a non-string cause message in RaiseFrom.
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::RaiseFrom {
            exception: MirExceptionValue::Constructed {
                type_tag: 1,
                class_name: "ValueError".to_string(),
                message: MirExpr::StringLiteral("msg".to_string()),
            },
            cause: MirExceptionValue::Constructed {
                type_tag: 2,
                class_name: "TypeError".to_string(),
                message: MirExpr::IntLiteral(42),
            },
            frame_function: "test_fn".to_string(),
        })],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("raise_from_non_str_cause")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("raise_from_non_str_cause.o");
    let result = compile_to_object(&mir, &obj_path, None, false);
    assert!(
        result.is_err(),
        "codegen should fail for non-string cause message"
    );
}

// -- #382: try/except/finally `?` error propagation and fall-through --

#[test]
fn try_body_emit_error_propagates() {
    // An undefined function call in the try body causes `emit_body` to
    // return `Err`, which propagates through the `?` at the try body
    // `emit_body` call site.
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::Try {
            body: vec![call_user_fn("nonexistent_in_try_body")],
            handlers: vec![MirExceptHandler {
                exc_type_tag: Some(vec![1]),
                binding_name: None,
                binding_ty: None,
                body: vec![call_print(99)],
            }],
            orelse: Vec::new(),
            finalbody: Vec::new(),
        })],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("try_body_err").expect("failed to create scratch dir");
    let obj_path = dir.join("try_body_err.o");
    let err = compile_to_object(&mir, &obj_path, None, false)
        .expect_err("codegen should fail for undefined fn in try body");
    assert!(
        err.contains("nonexistent_in_try_body"),
        "error should name the offending function: {err}"
    );
}

#[test]
fn try_handler_body_emit_error_propagates() {
    // An undefined function call in the handler body causes `emit_body`
    // to return `Err`, which propagates through the `?` at the handler
    // body `emit_body` call site.
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::Try {
            body: vec![call_print(1)],
            handlers: vec![MirExceptHandler {
                exc_type_tag: Some(vec![1]),
                binding_name: None,
                binding_ty: None,
                body: vec![call_user_fn("nonexistent_in_handler")],
            }],
            orelse: Vec::new(),
            finalbody: Vec::new(),
        })],
        class_defs: Vec::new(),
    };
    let dir =
        pycc_scratch::ScratchDir::new("try_handler_err").expect("failed to create scratch dir");
    let obj_path = dir.join("try_handler_err.o");
    let err = compile_to_object(&mir, &obj_path, None, false)
        .expect_err("codegen should fail for undefined fn in handler body");
    assert!(
        err.contains("nonexistent_in_handler"),
        "error should name the offending function: {err}"
    );
}

#[test]
fn try_else_body_emit_error_propagates() {
    // An undefined function call in the else body causes `emit_body` to
    // return `Err`, which propagates through the `?` at the else body
    // `emit_body` call site.
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::Try {
            body: vec![call_print(1)],
            handlers: vec![MirExceptHandler {
                exc_type_tag: Some(vec![1]),
                binding_name: None,
                binding_ty: None,
                body: vec![call_print(99)],
            }],
            orelse: vec![call_user_fn("nonexistent_in_else")],
            finalbody: Vec::new(),
        })],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("try_else_err").expect("failed to create scratch dir");
    let obj_path = dir.join("try_else_err.o");
    let err = compile_to_object(&mir, &obj_path, None, false)
        .expect_err("codegen should fail for undefined fn in else body");
    assert!(
        err.contains("nonexistent_in_else"),
        "error should name the offending function: {err}"
    );
}

#[test]
fn try_finally_body_emit_error_propagates() {
    // An undefined function call in the finally body causes `emit_body`
    // to return `Err`, which propagates through the `?` at the finally
    // body `emit_body` call site.
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::Try {
            body: vec![call_print(1)],
            handlers: vec![MirExceptHandler {
                exc_type_tag: Some(vec![1]),
                binding_name: None,
                binding_ty: None,
                body: vec![call_print(99)],
            }],
            orelse: Vec::new(),
            finalbody: vec![call_user_fn("nonexistent_in_finally")],
        })],
        class_defs: Vec::new(),
    };
    let dir =
        pycc_scratch::ScratchDir::new("try_finally_err").expect("failed to create scratch dir");
    let obj_path = dir.join("try_finally_err.o");
    let err = compile_to_object(&mir, &obj_path, None, false)
        .expect_err("codegen should fail for undefined fn in finally body");
    assert!(
        err.contains("nonexistent_in_finally"),
        "error should name the offending function: {err}"
    );
}

#[test]
fn try_body_with_return_does_not_fall_through() {
    // A `Return` in the try body terminates the block, so
    // `body_falls_through` is false and the exception-check branch is
    // skipped. The function has `return_ty: Ty::None` so the implicit
    // void return on the unreachable `after_bb` is valid.
    let mir = MirModule {
        items: vec![MirItem::Function {
            name: "f".to_string(),
            params: vec![],
            return_ty: Ty::None,
            body: vec![MirStmt::Try {
                body: vec![return_int(1)],
                handlers: vec![MirExceptHandler {
                    exc_type_tag: Some(vec![1]),
                    binding_name: None,
                    binding_ty: None,
                    body: vec![call_print(99)],
                }],
                orelse: Vec::new(),
                finalbody: Vec::new(),
            }],
        }],
        class_defs: Vec::new(),
    };
    let dir =
        pycc_scratch::ScratchDir::new("try_body_return").expect("failed to create scratch dir");
    let obj_path = dir.join("try_body_return.o");
    compile_to_object(&mir, &obj_path, None, false)
        .expect("codegen should succeed for try body with return");
}

#[test]
fn try_handler_body_with_return_does_not_fall_through() {
    // A `Return` in the handler body terminates the block, so
    // `handler_falls_through` is false and the branch to finally is
    // skipped.
    let mir = MirModule {
        items: vec![MirItem::Function {
            name: "f".to_string(),
            params: vec![],
            return_ty: Ty::None,
            body: vec![MirStmt::Try {
                body: vec![call_print(1)],
                handlers: vec![MirExceptHandler {
                    exc_type_tag: Some(vec![1]),
                    binding_name: None,
                    binding_ty: None,
                    body: vec![return_int(2)],
                }],
                orelse: Vec::new(),
                finalbody: Vec::new(),
            }],
        }],
        class_defs: Vec::new(),
    };
    let dir =
        pycc_scratch::ScratchDir::new("try_handler_return").expect("failed to create scratch dir");
    let obj_path = dir.join("try_handler_return.o");
    compile_to_object(&mir, &obj_path, None, false)
        .expect("codegen should succeed for handler body with return");
}

#[test]
fn try_else_body_with_return_does_not_fall_through() {
    // A `Return` in the else body terminates the block, so
    // `else_falls_through` is false and the branch to finally is
    // skipped.
    let mir = MirModule {
        items: vec![MirItem::Function {
            name: "f".to_string(),
            params: vec![],
            return_ty: Ty::None,
            body: vec![MirStmt::Try {
                body: vec![call_print(1)],
                handlers: vec![MirExceptHandler {
                    exc_type_tag: Some(vec![1]),
                    binding_name: None,
                    binding_ty: None,
                    body: vec![call_print(99)],
                }],
                orelse: vec![return_int(3)],
                finalbody: Vec::new(),
            }],
        }],
        class_defs: Vec::new(),
    };
    let dir =
        pycc_scratch::ScratchDir::new("try_else_return").expect("failed to create scratch dir");
    let obj_path = dir.join("try_else_return.o");
    compile_to_object(&mir, &obj_path, None, false)
        .expect("codegen should succeed for else body with return");
}

#[test]
fn try_finally_body_with_return_does_not_fall_through() {
    // A `Return` in the finally body terminates the block, so
    // `finally_falls_through` is false and the exception-check branch
    // is skipped.
    let mir = MirModule {
        items: vec![MirItem::Function {
            name: "f".to_string(),
            params: vec![],
            return_ty: Ty::None,
            body: vec![MirStmt::Try {
                body: vec![call_print(1)],
                handlers: vec![MirExceptHandler {
                    exc_type_tag: Some(vec![1]),
                    binding_name: None,
                    binding_ty: None,
                    body: vec![call_print(99)],
                }],
                orelse: Vec::new(),
                finalbody: vec![return_int(4)],
            }],
        }],
        class_defs: Vec::new(),
    };
    let dir =
        pycc_scratch::ScratchDir::new("try_finally_return").expect("failed to create scratch dir");
    let obj_path = dir.join("try_finally_return.o");
    compile_to_object(&mir, &obj_path, None, false)
        .expect("codegen should succeed for finally body with return");
}

#[test]
fn a_bare_non_none_return_routes_through_finally_with_a_default_value() {
    // Raw MIR mirrors an abstract-method-style `Return(None)` paired
    // with a non-None ABI. The type checker keeps this shape out of
    // ordinary functions, but codegen still has to route its neutral
    // carrier through a finally target without producing invalid IR.
    let mir = MirModule {
        items: vec![MirItem::Function {
            name: "f".to_string(),
            params: vec![],
            return_ty: Ty::Int,
            body: vec![MirStmt::Try {
                body: vec![MirStmt::Return(None)],
                handlers: vec![],
                orelse: vec![],
                finalbody: vec![call_print(1)],
            }],
        }],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("bare_non_none_return_finally")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("bare_non_none_return_finally.o");
    compile_to_object(&mir, &obj_path, None, false)
        .expect("the defensive default return must remain valid through finally");
}

#[test]
fn nested_finally_return_routing_covers_value_and_none_abis() {
    for (name, return_ty, returned) in [
        (
            "value",
            Ty::Int,
            MirStmt::Return(Some(MirExpr::IntLiteral(7))),
        ),
        ("none", Ty::None, MirStmt::Return(None)),
        (
            "none_expr",
            Ty::None,
            MirStmt::Return(Some(MirExpr::Name {
                name: "None".to_string(),
                ty: Ty::None,
            })),
        ),
    ] {
        let mir = MirModule {
            items: vec![MirItem::Function {
                name: format!("nested_{name}"),
                params: vec![],
                return_ty,
                body: vec![MirStmt::Try {
                    body: vec![MirStmt::Try {
                        body: vec![returned],
                        handlers: vec![],
                        orelse: vec![],
                        finalbody: vec![MirStmt::NoOp],
                    }],
                    handlers: vec![],
                    orelse: vec![],
                    finalbody: vec![MirStmt::NoOp],
                }],
            }],
            class_defs: vec![],
        };
        let dir = pycc_scratch::ScratchDir::new(&format!("nested_finally_{name}"))
            .expect("failed to create scratch dir");
        let obj_path = dir.join(format!("nested_finally_{name}.o"));
        compile_to_object(&mir, &obj_path, None, false)
            .expect("nested finally return routing must produce valid object code");
    }

    let mir = MirModule {
        items: vec![MirItem::Function {
            name: "direct_none".to_string(),
            params: vec![],
            return_ty: Ty::None,
            body: vec![MirStmt::Try {
                body: vec![MirStmt::Return(None)],
                handlers: vec![],
                orelse: vec![],
                finalbody: vec![MirStmt::NoOp],
            }],
        }],
        class_defs: vec![],
    };
    let dir =
        pycc_scratch::ScratchDir::new("direct_none_finally").expect("failed to create scratch dir");
    let obj_path = dir.join("direct_none_finally.o");
    compile_to_object(&mir, &obj_path, None, false)
        .expect("a None return routed through finally must produce valid object code");
}

/// Mirrors `nested_finally_return_routing_covers_value_and_none_abis`
/// above, but for `MirStmt::TryStar` (#542): `emit_try_star`'s own
/// `ret_bb` routing only reaches its "there is an outer `finally_stack`
/// entry" branches (both the `ret_slot: Some` and `ret_slot: None`
/// halves) when a `try*` with a `finally` is itself nested inside
/// another `finally`-bearing `try*`'s body, and only reaches its
/// "no outer, function ABI is `None`" branch when a top-level `try*`'s
/// `finally` routes a bare `return` in a `None`-returning function.
#[test]
fn nested_try_star_finally_return_routing_covers_value_and_none_abis() {
    // `block_always_terminates`'s defensive fallthrough check (independent
    // of `pycc_types::check`, since this is hand-built MIR) requires every
    // clause to itself terminate, so each handler returns a default value
    // of the matching ABI rather than falling through.
    for (name, return_ty, returned, handler_return) in [
        (
            "value",
            Ty::Int,
            MirStmt::Return(Some(MirExpr::IntLiteral(7))),
            MirStmt::Return(Some(MirExpr::IntLiteral(0))),
        ),
        (
            "none",
            Ty::None,
            MirStmt::Return(None),
            MirStmt::Return(None),
        ),
        (
            "none_expr",
            Ty::None,
            MirStmt::Return(Some(MirExpr::Name {
                name: "None".to_string(),
                ty: Ty::None,
            })),
            MirStmt::Return(None),
        ),
    ] {
        let ordinary_handler = |body| MirExceptHandler {
            exc_type_tag: Some(vec![1]),
            binding_name: None,
            binding_ty: None,
            body: vec![body],
        };
        let mir = MirModule {
            items: vec![MirItem::Function {
                name: format!("nested_trystar_{name}"),
                params: vec![],
                return_ty,
                body: vec![MirStmt::TryStar {
                    body: vec![MirStmt::TryStar {
                        body: vec![returned],
                        handlers: vec![ordinary_handler(handler_return.clone())],
                        orelse: vec![],
                        finalbody: vec![MirStmt::NoOp],
                    }],
                    handlers: vec![ordinary_handler(handler_return)],
                    orelse: vec![],
                    finalbody: vec![MirStmt::NoOp],
                }],
            }],
            class_defs: vec![],
        };
        let dir = pycc_scratch::ScratchDir::new(&format!("nested_trystar_finally_{name}"))
            .expect("failed to create scratch dir");
        let obj_path = dir.join(format!("nested_trystar_finally_{name}.o"));
        compile_to_object(&mir, &obj_path, None, false)
            .expect("nested try* finally return routing must produce valid object code");
    }

    let mir = MirModule {
        items: vec![MirItem::Function {
            name: "direct_none_trystar".to_string(),
            params: vec![],
            return_ty: Ty::None,
            body: vec![MirStmt::TryStar {
                body: vec![MirStmt::Return(None)],
                handlers: vec![MirExceptHandler {
                    exc_type_tag: Some(vec![1]),
                    binding_name: None,
                    binding_ty: None,
                    body: vec![MirStmt::Return(None)],
                }],
                orelse: vec![],
                finalbody: vec![MirStmt::NoOp],
            }],
        }],
        class_defs: vec![],
    };
    let dir = pycc_scratch::ScratchDir::new("direct_none_trystar_finally")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("direct_none_trystar_finally.o");
    compile_to_object(&mir, &obj_path, None, false)
        .expect("a None return routed through a try*'s finally must produce valid object code");
}

// -- #382: RaiseFrom, Reraise, and remaining Try codegen paths --

#[test]
fn raise_from_codegen_builds_and_runs() {
    // Exercises the success path of `RaiseFrom` codegen (lines 7296-7323):
    // both message and cause are string literals, so the `Scalar::Str(p)`
    // arms succeed and the full alloc/raise-with-cause sequence runs.
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::Try {
            body: vec![MirStmt::RaiseFrom {
                exception: MirExceptionValue::Constructed {
                    type_tag: 1, // ValueError
                    class_name: "ValueError".to_string(),
                    message: MirExpr::StringLiteral("bad".to_string()),
                },
                cause: MirExceptionValue::Constructed {
                    type_tag: 2, // TypeError
                    class_name: "TypeError".to_string(),
                    message: MirExpr::StringLiteral("cause".to_string()),
                },
                frame_function: "test_fn".to_string(),
            }],
            handlers: vec![MirExceptHandler {
                exc_type_tag: Some(vec![1]),
                binding_name: None,
                binding_ty: None,
                body: vec![MirStmt::ExprStmt(MirExpr::Call {
                    callee: "print".to_string(),
                    args: vec![MirExpr::StringLiteral("caught".to_string())],
                    ty: Ty::None,
                })],
            }],
            orelse: Vec::new(),
            finalbody: Vec::new(),
        })],
        class_defs: Vec::new(),
    };
    let dir =
        pycc_scratch::ScratchDir::new("raise_from_codegen").expect("failed to create scratch dir");
    let obj_path = dir.join("raise_from.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("raise_from");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"caught\n");
    assert!(output.status.success());
}

#[test]
fn reraise_codegen_builds() {
    // Exercises the `Reraise` codegen path: loads the lexically enclosing
    // handler's saved exception value and re-raises it.
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::Try {
            body: vec![MirStmt::Raise {
                exception: MirExceptionValue::Constructed {
                    type_tag: 1,
                    class_name: "ValueError".to_string(),
                    message: MirExpr::StringLiteral("orig".to_string()),
                },
                frame_function: "test_fn".to_string(),
            }],
            handlers: vec![MirExceptHandler {
                exc_type_tag: Some(vec![1]),
                binding_name: None,
                binding_ty: None,
                body: vec![MirStmt::Reraise],
            }],
            orelse: Vec::new(),
            finalbody: Vec::new(),
        })],
        class_defs: Vec::new(),
    };
    let dir =
        pycc_scratch::ScratchDir::new("reraise_codegen").expect("failed to create scratch dir");
    let obj_path = dir.join("reraise.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("reraise");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert!(
        !output.status.success(),
        "reraise should propagate as non-zero exit"
    );
}

#[test]
fn try_with_no_handlers_branches_to_finally() {
    // Exercises the `handlers.is_empty()` branch (lines 7423-7428):
    // a try with no handlers branches directly to finally, where the
    // exception remains active and propagates after the finally body.
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::Try {
            body: vec![MirStmt::Raise {
                exception: MirExceptionValue::Constructed {
                    type_tag: 1,
                    class_name: "ValueError".to_string(),
                    message: MirExpr::StringLiteral("uncaught".to_string()),
                },
                frame_function: "test_fn".to_string(),
            }],
            handlers: Vec::new(),
            orelse: Vec::new(),
            finalbody: vec![MirStmt::ExprStmt(MirExpr::Call {
                callee: "print".to_string(),
                args: vec![MirExpr::StringLiteral("finally".to_string())],
                ty: Ty::None,
            })],
        })],
        class_defs: Vec::new(),
    };
    let dir =
        pycc_scratch::ScratchDir::new("try_no_handlers").expect("failed to create scratch dir");
    let obj_path = dir.join("try_no_handlers.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("try_no_handlers");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"finally\n");
    assert!(
        !output.status.success(),
        "uncaught exception should exit non-zero"
    );
}

#[test]
fn try_with_multiple_handlers_dispatches_to_next() {
    // Exercises the `dispatch_bbs[i + 1]` path (line 7455): when the
    // first handler does not match, control flows to the next dispatch
    // block.  Here a `KeyError` (tag 3) is raised but the first handler
    // checks for `ValueError` (tag 1), so the second handler (tag 3)
    // matches.
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::Try {
            body: vec![MirStmt::Raise {
                exception: MirExceptionValue::Constructed {
                    type_tag: 3, // KeyError
                    class_name: "KeyError".to_string(),
                    message: MirExpr::StringLiteral("key".to_string()),
                },
                frame_function: "test_fn".to_string(),
            }],
            handlers: vec![
                MirExceptHandler {
                    exc_type_tag: Some(vec![1]), // ValueError — does not match
                    binding_name: None,
                    binding_ty: None,
                    body: vec![MirStmt::ExprStmt(MirExpr::Call {
                        callee: "print".to_string(),
                        args: vec![MirExpr::StringLiteral("value".to_string())],
                        ty: Ty::None,
                    })],
                },
                MirExceptHandler {
                    exc_type_tag: Some(vec![3]), // KeyError — matches
                    binding_name: None,
                    binding_ty: None,
                    body: vec![MirStmt::ExprStmt(MirExpr::Call {
                        callee: "print".to_string(),
                        args: vec![MirExpr::StringLiteral("key".to_string())],
                        ty: Ty::None,
                    })],
                },
            ],
            orelse: Vec::new(),
            finalbody: Vec::new(),
        })],
        class_defs: Vec::new(),
    };
    let dir =
        pycc_scratch::ScratchDir::new("try_multi_dispatch").expect("failed to create scratch dir");
    let obj_path = dir.join("try_multi_dispatch.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("try_multi_dispatch");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"key\n");
    assert!(output.status.success());
}

#[test]
fn a_multi_tag_handler_ors_every_tag_it_accepts() {
    // Part 2 of #541 (D-189): `except AppError:` over a hierarchy where
    // `AppError` is tag 7 and its subclasses are 8 and 9 emits three
    // `pycc_rt_exception_type_matches` calls joined by `or`. Only the second
    // accumulation step reaches `build_or`, so a single-tag handler cannot
    // cover it. The raised tag is 9 -- the *last* tag in the set -- so a
    // chain that stopped short would fall through to the bare handler.
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::Try {
            body: vec![MirStmt::Raise {
                exception: MirExceptionValue::Constructed {
                    type_tag: 9,
                    class_name: "DatabaseError".to_string(),
                    message: MirExpr::StringLiteral("boom".to_string()),
                },
                frame_function: "test_fn".to_string(),
            }],
            handlers: vec![
                MirExceptHandler {
                    exc_type_tag: Some(vec![7, 8, 9]),
                    binding_name: None,
                    binding_ty: None,
                    body: vec![MirStmt::ExprStmt(MirExpr::Call {
                        callee: "print".to_string(),
                        args: vec![MirExpr::StringLiteral("app".to_string())],
                        ty: Ty::None,
                    })],
                },
                MirExceptHandler {
                    exc_type_tag: None,
                    binding_name: None,
                    binding_ty: None,
                    body: vec![MirStmt::ExprStmt(MirExpr::Call {
                        callee: "print".to_string(),
                        args: vec![MirExpr::StringLiteral("bare".to_string())],
                        ty: Ty::None,
                    })],
                },
            ],
            orelse: Vec::new(),
            finalbody: Vec::new(),
        })],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("try_multi_tag").expect("failed to create scratch dir");
    let obj_path = dir.join("try_multi_tag.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("try_multi_tag");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"app\n");
    assert!(output.status.success());
}

#[test]
fn a_multi_tag_handler_declines_a_tag_outside_its_set() {
    // The complement of the test above: every `or` operand evaluates false,
    // so the OR-chain must produce zero rather than a stray one.
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::Try {
            body: vec![MirStmt::Raise {
                exception: MirExceptionValue::Constructed {
                    type_tag: 3,
                    class_name: "KeyError".to_string(),
                    message: MirExpr::StringLiteral("key".to_string()),
                },
                frame_function: "test_fn".to_string(),
            }],
            handlers: vec![
                MirExceptHandler {
                    exc_type_tag: Some(vec![7, 8, 9]),
                    binding_name: None,
                    binding_ty: None,
                    body: vec![MirStmt::ExprStmt(MirExpr::Call {
                        callee: "print".to_string(),
                        args: vec![MirExpr::StringLiteral("app".to_string())],
                        ty: Ty::None,
                    })],
                },
                MirExceptHandler {
                    exc_type_tag: None,
                    binding_name: None,
                    binding_ty: None,
                    body: vec![MirStmt::ExprStmt(MirExpr::Call {
                        callee: "print".to_string(),
                        args: vec![MirExpr::StringLiteral("bare".to_string())],
                        ty: Ty::None,
                    })],
                },
            ],
            orelse: Vec::new(),
            finalbody: Vec::new(),
        })],
        class_defs: Vec::new(),
    };
    let dir =
        pycc_scratch::ScratchDir::new("try_multi_tag_miss").expect("failed to create scratch dir");
    let obj_path = dir.join("try_multi_tag_miss.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("try_multi_tag_miss");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"bare\n");
    assert!(output.status.success());
}

#[test]
fn a_pep_758_multi_type_handler_ors_every_named_types_tag_independently() {
    // PEP 758 (#740): `except (ValueError, KeyError, IndexError):` unions
    // three *independently named* types' tags (as MIR lowering computes,
    // via union + dedup, from `HirExceptHandler.exc_type`'s `Vec<String>`)
    // rather than one name's subclass expansion. Each named type must be
    // independently catchable through the resulting OR-chain -- run three
    // separate raises against the same tag set.
    for (raised_tag, class_name) in [(1u8, "ValueError"), (3, "KeyError"), (4, "IndexError")] {
        let mir = MirModule {
            items: vec![MirItem::TopLevelStmt(MirStmt::Try {
                body: vec![MirStmt::Raise {
                    exception: MirExceptionValue::Constructed {
                        type_tag: raised_tag,
                        class_name: class_name.to_string(),
                        message: MirExpr::StringLiteral("boom".to_string()),
                    },
                    frame_function: "test_fn".to_string(),
                }],
                handlers: vec![
                    MirExceptHandler {
                        exc_type_tag: Some(vec![1, 3, 4]),
                        binding_name: None,
                        binding_ty: None,
                        body: vec![MirStmt::ExprStmt(MirExpr::Call {
                            callee: "print".to_string(),
                            args: vec![MirExpr::StringLiteral("multi".to_string())],
                            ty: Ty::None,
                        })],
                    },
                    MirExceptHandler {
                        exc_type_tag: None,
                        binding_name: None,
                        binding_ty: None,
                        body: vec![MirStmt::ExprStmt(MirExpr::Call {
                            callee: "print".to_string(),
                            args: vec![MirExpr::StringLiteral("bare".to_string())],
                            ty: Ty::None,
                        })],
                    },
                ],
                orelse: Vec::new(),
                finalbody: Vec::new(),
            })],
            class_defs: Vec::new(),
        };
        let dir = pycc_scratch::ScratchDir::new(&format!("try_pep758_multi_{class_name}"))
            .expect("failed to create scratch dir");
        let obj_path = dir.join("try_pep758_multi.o");
        compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
        let bin_path = dir.join("try_pep758_multi");
        link_object_with_runtime(&obj_path, &bin_path);
        let output = Command::new(&bin_path).output().expect("binary should run");
        assert_eq!(
            output.stdout, b"multi\n",
            "raised tag {raised_tag} ({class_name})"
        );
        assert!(output.status.success());
    }
}

#[test]
fn try_else_body_falls_through_to_finally() {
    // Exercises the `else_falls_through` branch (lines 7569-7571):
    // a non-empty else body that completes normally (no return/raise)
    // branches to the finally block.
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::Try {
            body: vec![MirStmt::ExprStmt(MirExpr::Call {
                callee: "print".to_string(),
                args: vec![MirExpr::StringLiteral("try".to_string())],
                ty: Ty::None,
            })],
            handlers: vec![MirExceptHandler {
                exc_type_tag: Some(vec![1]),
                binding_name: None,
                binding_ty: None,
                body: vec![MirStmt::ExprStmt(MirExpr::Call {
                    callee: "print".to_string(),
                    args: vec![MirExpr::StringLiteral("handler".to_string())],
                    ty: Ty::None,
                })],
            }],
            orelse: vec![MirStmt::ExprStmt(MirExpr::Call {
                callee: "print".to_string(),
                args: vec![MirExpr::StringLiteral("else".to_string())],
                ty: Ty::None,
            })],
            finalbody: vec![MirStmt::ExprStmt(MirExpr::Call {
                callee: "print".to_string(),
                args: vec![MirExpr::StringLiteral("finally".to_string())],
                ty: Ty::None,
            })],
        })],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("try_else_falls_through")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("try_else_falls_through.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("try_else_falls_through");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"try\nelse\nfinally\n");
    assert!(output.status.success());
}

// -- Part 3A of #541 (#736): render a caught exception binding ----------

#[test]
fn print_of_a_caught_exception_binding_prints_its_message() {
    // `except ValueError as e: print(e)` -- resolves the #705 reproducer.
    // `MirExpr::ExceptionMessage` is hand-built here exactly as
    // `pycc_mir::class::rewrite_exception_to_message` would produce it,
    // matching how `raising_a_bound_existing_exception_builds_successfully`
    // above hand-builds its own handler binding.
    let exception_ty = Ty::Instance(Box::new("ValueError".to_string()));
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::Try {
            body: vec![MirStmt::Raise {
                exception: MirExceptionValue::Constructed {
                    type_tag: 1,
                    class_name: "ValueError".to_string(),
                    message: MirExpr::StringLiteral("boom".to_string()),
                },
                frame_function: "test_fn".to_string(),
            }],
            handlers: vec![MirExceptHandler {
                exc_type_tag: Some(vec![1]),
                binding_name: Some("e".to_string()),
                binding_ty: Some(exception_ty.clone()),
                body: vec![print_expr(MirExpr::ExceptionMessage(Box::new(
                    MirExpr::Name {
                        name: "e".to_string(),
                        ty: exception_ty,
                    },
                )))],
            }],
            orelse: vec![],
            finalbody: vec![],
        })],
        class_defs: vec![],
    };
    let dir = pycc_scratch::ScratchDir::new("exception_message_print")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("exception_message_print.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("exception_message_print");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"boom\n");
}

#[test]
fn fstring_interpolation_of_a_caught_exception_binding_renders_its_message() {
    // `except ValueError as e: print(f"{e}")` -- a single interpolation
    // and no other text produces just the message, exactly like
    // `str(e)`.
    let exception_ty = Ty::Instance(Box::new("ValueError".to_string()));
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::Try {
            body: vec![MirStmt::Raise {
                exception: MirExceptionValue::Constructed {
                    type_tag: 1,
                    class_name: "ValueError".to_string(),
                    message: MirExpr::StringLiteral("boom".to_string()),
                },
                frame_function: "test_fn".to_string(),
            }],
            handlers: vec![MirExceptHandler {
                exc_type_tag: Some(vec![1]),
                binding_name: Some("e".to_string()),
                binding_ty: Some(exception_ty.clone()),
                body: vec![print_expr(MirExpr::FString(vec![
                    MirFStringPart::Interpolation(Box::new(MirExpr::ExceptionMessage(Box::new(
                        MirExpr::Name {
                            name: "e".to_string(),
                            ty: exception_ty,
                        },
                    )))),
                ]))],
            }],
            orelse: vec![],
            finalbody: vec![],
        })],
        class_defs: vec![],
    };
    let dir = pycc_scratch::ScratchDir::new("exception_message_fstring")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("exception_message_fstring.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("exception_message_fstring");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"boom\n");
}
