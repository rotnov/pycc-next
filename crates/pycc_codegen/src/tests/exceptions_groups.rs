//! Exception terminal analysis, `raise`, exception groups and `try*`.

use super::*;

// -- #382 exception handling codegen tests --

#[test]
fn exception_terminal_analysis_covers_structured_paths() {
    let returned = || MirStmt::Return(Some(MirExpr::IntLiteral(1)));
    let falls_through = || MirStmt::NoOp;

    assert!(fallthrough::block_always_terminates(&[MirStmt::If {
        test: MirExpr::BoolLiteral(true),
        body: vec![returned()],
        orelse: vec![returned()],
    }]));
    assert!(!fallthrough::block_always_terminates(&[MirStmt::If {
        test: MirExpr::BoolLiteral(true),
        body: vec![returned()],
        orelse: vec![],
    }]));
    assert!(fallthrough::block_always_terminates(&[MirStmt::Seq(vec![
        returned(),
    ])]));

    let terminal_handler = MirExceptHandler {
        exc_type_tag: Some(vec![1]),
        binding_name: None,
        binding_ty: None,
        body: vec![returned()],
    };
    assert!(fallthrough::block_always_terminates(&[MirStmt::Try {
        body: vec![falls_through()],
        handlers: vec![terminal_handler.clone()],
        orelse: vec![returned()],
        finalbody: vec![],
    }]));
    assert!(!fallthrough::block_always_terminates(&[MirStmt::Try {
        body: vec![falls_through()],
        handlers: vec![terminal_handler],
        orelse: vec![],
        finalbody: vec![],
    }]));
    assert!(fallthrough::block_always_terminates(&[MirStmt::Try {
        body: vec![falls_through()],
        handlers: vec![],
        orelse: vec![],
        finalbody: vec![returned()],
    }]));
    assert!(!fallthrough::block_always_terminates(&[falls_through()]));

    // `except*` (#542) shares `Try`'s exact fallthrough shape via the
    // combined `MirStmt::Try { .. } | MirStmt::TryStar { .. }` arm above --
    // the `Try` alternative's own field-destructure lines are already
    // exercised by the assertions above, but the `TryStar` alternative's
    // own lines are only reached when the analyzed statement is actually a
    // `TryStar`.
    let terminal_handler = MirExceptHandler {
        exc_type_tag: Some(vec![1]),
        binding_name: None,
        binding_ty: None,
        body: vec![returned()],
    };
    assert!(fallthrough::block_always_terminates(&[MirStmt::TryStar {
        body: vec![falls_through()],
        handlers: vec![terminal_handler.clone()],
        orelse: vec![returned()],
        finalbody: vec![],
    }]));
    assert!(!fallthrough::block_always_terminates(&[MirStmt::TryStar {
        body: vec![falls_through()],
        handlers: vec![terminal_handler],
        orelse: vec![],
        finalbody: vec![],
    }]));
}

#[test]
fn bare_except_codegen_builds_and_runs() {
    // Tests the bare `except:` path (exc_type_tag = None) in codegen.
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::Try {
            body: vec![MirStmt::Raise {
                exception: MirExceptionValue::Constructed {
                    type_tag: 1, // ValueError
                    class_name: "ValueError".to_string(),
                    message: MirExpr::StringLiteral("test".to_string()),
                },
                frame_function: "test_fn".to_string(),
            }],
            handlers: vec![MirExceptHandler {
                exc_type_tag: None, // bare except
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
        pycc_scratch::ScratchDir::new("bare_except_codegen").expect("failed to create scratch dir");
    let obj_path = dir.join("bare_except.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("bare_except");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"caught\n");
    assert!(output.status.success());
}

#[test]
fn raise_with_non_string_message_is_a_codegen_error() {
    // Tests the error path for a non-string raise message.
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::Raise {
            exception: MirExceptionValue::Constructed {
                type_tag: 1,
                class_name: "ValueError".to_string(),
                message: MirExpr::IntLiteral(42),
            },
            frame_function: "test_fn".to_string(),
        })],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("raise_non_str").expect("failed to create scratch dir");
    let obj_path = dir.join("raise_non_str.o");
    let result = compile_to_object(&mir, &obj_path, None, false);
    assert!(
        result.is_err(),
        "codegen should fail for non-string raise message"
    );
}

#[test]
fn raising_a_non_instance_existing_value_is_a_codegen_error() {
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::Raise {
            exception: MirExceptionValue::Existing(MirExpr::IntLiteral(42)),
            frame_function: "test_fn".to_string(),
        })],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("raise_existing_non_instance")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("raise_existing_non_instance.o");
    let err = compile_to_object(&mir, &obj_path, None, false)
        .expect_err("a non-instance cannot be an existing exception");
    assert!(err.contains("must be an exception instance"), "{err}");
}

#[test]
fn raising_a_bound_existing_exception_builds_successfully() {
    let exception_ty = Ty::Instance(Box::new("ValueError".to_string()));
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::Try {
            body: vec![MirStmt::Raise {
                exception: MirExceptionValue::Constructed {
                    type_tag: 1,
                    class_name: "ValueError".to_string(),
                    message: MirExpr::StringLiteral("original".to_string()),
                },
                frame_function: "test_fn".to_string(),
            }],
            handlers: vec![MirExceptHandler {
                exc_type_tag: Some(vec![1]),
                binding_name: Some("error".to_string()),
                binding_ty: Some(exception_ty.clone()),
                body: vec![MirStmt::Raise {
                    exception: MirExceptionValue::Existing(MirExpr::Name {
                        name: "error".to_string(),
                        ty: exception_ty,
                    }),
                    frame_function: "test_fn".to_string(),
                }],
            }],
            orelse: vec![],
            finalbody: vec![],
        })],
        class_defs: vec![],
    };
    let dir = pycc_scratch::ScratchDir::new("raise_existing_instance")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("raise_existing_instance.o");
    compile_to_object(&mir, &obj_path, None, false)
        .expect("a bound exception instance can be raised again");
}

/// Part 3 of #382 (#542, PEP 654, D-202): `pycc_types::check` (T0021) rejects
/// a non-`str` `ExceptionGroup`/`BaseExceptionGroup` message long before HIR
/// reaches codegen, so this internal-invariant guard in
/// `pycc_codegen::exception::emit_raise_value`'s `ConstructedGroup` arm is
/// unreachable through any real Python fixture -- exercise it directly with
/// hand-built MIR, mirroring `raise_with_non_string_message_is_a_codegen_error`
/// above for the plain (non-group) `Constructed` variant.
#[test]
fn constructed_group_with_non_string_message_is_a_codegen_error() {
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::Raise {
            exception: MirExceptionValue::ConstructedGroup {
                type_tag: EXCEPTION_GROUP_TYPE_TAG,
                class_name: "ExceptionGroup".to_string(),
                message: MirExpr::IntLiteral(42),
                members: vec![],
            },
            frame_function: "test_fn".to_string(),
        })],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("constructed_group_non_str_message")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("constructed_group_non_str_message.o");
    let err = compile_to_object(&mir, &obj_path, None, false)
        .expect_err("a non-string ExceptionGroup message must be a codegen error");
    assert!(err.contains("message must be a string"), "{err}");
}

/// Same rationale as `constructed_group_with_non_string_message_is_a_codegen_error`
/// above, but for the member-must-be-an-exception-instance guard: `T0021`
/// rejects a non-exception `ExceptionGroup` member before codegen ever sees
/// it, so this branch needs hand-built MIR too.
#[test]
fn constructed_group_with_a_non_instance_member_is_a_codegen_error() {
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::Raise {
            exception: MirExceptionValue::ConstructedGroup {
                type_tag: EXCEPTION_GROUP_TYPE_TAG,
                class_name: "ExceptionGroup".to_string(),
                message: MirExpr::StringLiteral("multi".to_string()),
                members: vec![MirExpr::IntLiteral(1)],
            },
            frame_function: "test_fn".to_string(),
        })],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("constructed_group_non_instance_member")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("constructed_group_non_instance_member.o");
    let err = compile_to_object(&mir, &obj_path, None, false)
        .expect_err("a non-instance ExceptionGroup member must be a codegen error");
    assert!(
        err.contains("group member must be an exception instance"),
        "{err}"
    );
}

/// The two tests above only exercise `ConstructedGroup`'s `Err` paths (a
/// non-`str` message, a non-instance member); neither reaches the success
/// path -- the member-pointer-array build loop (`build_gep`/`build_store`
/// for each member) or the final `pycc_rt_exception_group_alloc` call. This
/// test drives a real, successful multi-member `ExceptionGroup` build: two
/// exceptions are caught and bound (`e1`, `e2`), then combined into an
/// `ExceptionGroup`, raised, and caught by an outer handler that prints the
/// group's own message -- covering the loop and the success-path tail.
#[test]
fn a_successful_multi_member_exception_group_construction_compiles_and_runs() {
    let value_error_ty = Ty::Instance(Box::new("ValueError".to_string()));
    let type_error_ty = Ty::Instance(Box::new("TypeError".to_string()));
    let group_ty = Ty::Instance(Box::new("ExceptionGroup".to_string()));

    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::Try {
            body: vec![MirStmt::Try {
                body: vec![MirStmt::Raise {
                    exception: MirExceptionValue::Constructed {
                        type_tag: 1,
                        class_name: "ValueError".to_string(),
                        message: MirExpr::StringLiteral("v1".to_string()),
                    },
                    frame_function: "test_fn".to_string(),
                }],
                handlers: vec![MirExceptHandler {
                    exc_type_tag: Some(vec![1]),
                    binding_name: Some("e1".to_string()),
                    binding_ty: Some(value_error_ty.clone()),
                    body: vec![MirStmt::Try {
                        body: vec![MirStmt::Raise {
                            exception: MirExceptionValue::Constructed {
                                type_tag: 2,
                                class_name: "TypeError".to_string(),
                                message: MirExpr::StringLiteral("v2".to_string()),
                            },
                            frame_function: "test_fn".to_string(),
                        }],
                        handlers: vec![MirExceptHandler {
                            exc_type_tag: Some(vec![2]),
                            binding_name: Some("e2".to_string()),
                            binding_ty: Some(type_error_ty.clone()),
                            body: vec![MirStmt::Raise {
                                exception: MirExceptionValue::ConstructedGroup {
                                    type_tag: EXCEPTION_GROUP_TYPE_TAG,
                                    class_name: "ExceptionGroup".to_string(),
                                    message: MirExpr::StringLiteral("multi".to_string()),
                                    members: vec![
                                        MirExpr::Name {
                                            name: "e1".to_string(),
                                            ty: value_error_ty.clone(),
                                        },
                                        MirExpr::Name {
                                            name: "e2".to_string(),
                                            ty: type_error_ty.clone(),
                                        },
                                    ],
                                },
                                frame_function: "test_fn".to_string(),
                            }],
                        }],
                        orelse: vec![],
                        finalbody: vec![],
                    }],
                }],
                orelse: vec![],
                finalbody: vec![],
            }],
            handlers: vec![MirExceptHandler {
                exc_type_tag: Some(vec![EXCEPTION_GROUP_TYPE_TAG]),
                binding_name: Some("eg".to_string()),
                binding_ty: Some(group_ty.clone()),
                body: vec![print_expr(MirExpr::ExceptionMessage(Box::new(
                    MirExpr::Name {
                        name: "eg".to_string(),
                        ty: group_ty,
                    },
                )))],
            }],
            orelse: vec![],
            finalbody: vec![],
        })],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("successful_multi_member_exception_group")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("successful_multi_member_exception_group.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("successful_multi_member_exception_group");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"multi\n");
    assert!(output.status.success());
}

/// Same rationale as `constructed_group_with_non_string_message_is_a_codegen_error`
/// above, but for the `role == "cause"` branch of the error message: that
/// test uses a plain `MirStmt::Raise`, which always passes `role ==
/// "exception"` into `emit_exception_value`, so the group variant's
/// `"raise cause message must be a string"` wording (as opposed to `"raise
/// message must be a string"`) has never been exercised. A `RaiseFrom` whose
/// `cause` (not `exception`) is the non-string-message `ConstructedGroup`
/// reaches that call with `role == "cause"`, mirroring
/// `raise_from_with_non_string_cause_is_a_codegen_error`'s coverage of the
/// same branch for the plain (non-group) `Constructed` variant.
#[test]
fn constructed_group_cause_with_non_string_message_is_a_codegen_error() {
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::RaiseFrom {
            exception: MirExceptionValue::Constructed {
                type_tag: 1,
                class_name: "ValueError".to_string(),
                message: MirExpr::StringLiteral("msg".to_string()),
            },
            cause: MirExceptionValue::ConstructedGroup {
                type_tag: EXCEPTION_GROUP_TYPE_TAG,
                class_name: "ExceptionGroup".to_string(),
                message: MirExpr::IntLiteral(42),
                members: vec![],
            },
            frame_function: "test_fn".to_string(),
        })],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("constructed_group_cause_non_str_message")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("constructed_group_cause_non_str_message.o");
    let err = compile_to_object(&mir, &obj_path, None, false)
        .expect_err("a non-string ExceptionGroup cause message must be a codegen error");
    assert!(
        err.contains("raise cause message must be a string"),
        "{err}"
    );
}

/// `emit_try_star` calls `emit_body` once for each of its four constituent
/// blocks (try body, handler body, `else` body, `finally` body), and each
/// call is immediately propagated with `?`. Every real `except*` fixture
/// used elsewhere in this workspace only ever produces `Ok` from all four
/// calls, so the `?` operator's own error-propagation branch on each of
/// those four call sites is otherwise never taken. These four tests place a
/// deliberately invalid raise (hand-built MIR bypassing `pycc_types::check`,
/// exactly like `raise_with_non_string_message_is_a_codegen_error` above) in
/// each of the four positions in turn, to exercise every one of those
/// branches once.
#[test]
fn a_codegen_error_in_a_try_star_body_propagates_out_of_emit_try_star() {
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::TryStar {
            body: vec![MirStmt::Raise {
                exception: MirExceptionValue::Constructed {
                    type_tag: 1,
                    class_name: "ValueError".to_string(),
                    message: MirExpr::IntLiteral(42),
                },
                frame_function: "test_fn".to_string(),
            }],
            handlers: vec![],
            orelse: vec![],
            finalbody: vec![],
        })],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("try_star_body_codegen_error")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("try_star_body_codegen_error.o");
    let err = compile_to_object(&mir, &obj_path, None, false)
        .expect_err("a codegen error in the try* body must propagate");
    assert!(err.contains("message must be a string"), "{err}");
}

#[test]
fn a_codegen_error_in_a_try_star_handler_body_propagates_out_of_emit_try_star() {
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::TryStar {
            body: vec![],
            handlers: vec![MirExceptHandler {
                exc_type_tag: Some(vec![1]),
                binding_name: None,
                binding_ty: None,
                body: vec![MirStmt::Raise {
                    exception: MirExceptionValue::Constructed {
                        type_tag: 1,
                        class_name: "ValueError".to_string(),
                        message: MirExpr::IntLiteral(42),
                    },
                    frame_function: "test_fn".to_string(),
                }],
            }],
            orelse: vec![],
            finalbody: vec![],
        })],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("try_star_handler_codegen_error")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("try_star_handler_codegen_error.o");
    let err = compile_to_object(&mir, &obj_path, None, false)
        .expect_err("a codegen error in a try* handler body must propagate");
    assert!(err.contains("message must be a string"), "{err}");
}

#[test]
fn a_codegen_error_in_a_try_star_else_body_propagates_out_of_emit_try_star() {
    // `except*` always has at least one clause (a bare `except*:` is a
    // parse-time rejection under PEP 654, and `emit_try_star` assumes at
    // least one dispatch block exists), so this fixture -- unlike the body
    // and handler-body cases above -- needs one ordinary, non-erroring
    // handler alongside the erroring `else` body.
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::TryStar {
            body: vec![],
            handlers: vec![MirExceptHandler {
                exc_type_tag: Some(vec![1]),
                binding_name: None,
                binding_ty: None,
                body: vec![],
            }],
            orelse: vec![MirStmt::Raise {
                exception: MirExceptionValue::Constructed {
                    type_tag: 1,
                    class_name: "ValueError".to_string(),
                    message: MirExpr::IntLiteral(42),
                },
                frame_function: "test_fn".to_string(),
            }],
            finalbody: vec![],
        })],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("try_star_else_codegen_error")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("try_star_else_codegen_error.o");
    let err = compile_to_object(&mir, &obj_path, None, false)
        .expect_err("a codegen error in a try* else body must propagate");
    assert!(err.contains("message must be a string"), "{err}");
}

#[test]
fn a_codegen_error_in_a_try_star_finally_body_propagates_out_of_emit_try_star() {
    // Same reason as the `else`-body case above: at least one ordinary
    // handler is required for `emit_try_star`'s own dispatch-block
    // invariant, alongside the erroring `finally` body.
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::TryStar {
            body: vec![],
            handlers: vec![MirExceptHandler {
                exc_type_tag: Some(vec![1]),
                binding_name: None,
                binding_ty: None,
                body: vec![],
            }],
            orelse: vec![],
            finalbody: vec![MirStmt::Raise {
                exception: MirExceptionValue::Constructed {
                    type_tag: 1,
                    class_name: "ValueError".to_string(),
                    message: MirExpr::IntLiteral(42),
                },
                frame_function: "test_fn".to_string(),
            }],
        })],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("try_star_finally_codegen_error")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("try_star_finally_codegen_error.o");
    let err = compile_to_object(&mir, &obj_path, None, false)
        .expect_err("a codegen error in a try* finally body must propagate");
    assert!(err.contains("message must be a string"), "{err}");
}

/// The four error-propagation tests above all use empty `orelse`/`finalbody`
/// and a bindingless handler, so they only ever exercise `emit_try_star`'s
/// early-return `?` branches -- never the ordinary successful-compile path
/// through a non-empty `else`, a non-empty `finally`, and a handler that
/// binds its matched subgroup to a name (the `if let Some(binding_name) =
/// &handler.binding_name` branch), nor the "did the finally body fall
/// through, and was there a pending exception to restore" logic that only
/// exists when `finalbody` is non-empty. This test supplies all three at
/// once so a single successful `compile_to_object` call builds every one of
/// those blocks, then actually runs the resulting binary to confirm the
/// generated code still behaves correctly: `except*` matches the raised
/// `ValueError`, binds it, prints its message, and the `finally` body still
/// runs -- while `else` (guarded by "no exception was raised") does not,
/// since the try body did raise.
#[test]
fn a_try_star_with_a_bound_handler_else_and_finally_compiles_and_runs() {
    let exception_ty = Ty::Instance(Box::new("ValueError".to_string()));
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::TryStar {
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
    let dir = pycc_scratch::ScratchDir::new("try_star_bound_handler_else_finally")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("try_star_bound_handler_else_finally.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("try_star_bound_handler_else_finally");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"boom\nfinally\n");
    assert!(output.status.success());
}

/// `emit_try_star`'s non-empty-`orelse` path only inserts its own
/// fallthrough branch to `finally_bb` when the `orelse` body itself falls
/// through without a terminator (`else_falls_through`). The fixture above
/// (`a_try_star_with_a_bound_handler_else_and_finally_compiles_and_runs`)
/// has a non-empty `orelse`, but its `body` always raises, so `orelse`
/// never actually runs at all -- this test's `body` completes without
/// raising instead, so `orelse` does run, and its own body (a bare
/// `print`, not a `return`) falls through, forcing `emit_try_star` to
/// synthesize the branch to `finally_bb`.
#[test]
fn a_try_star_else_that_falls_through_branches_to_finally() {
    let exception_ty = Ty::Instance(Box::new("ValueError".to_string()));
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::TryStar {
            body: vec![print_expr(MirExpr::StringLiteral("try".to_string()))],
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
            orelse: vec![print_expr(MirExpr::StringLiteral("else".to_string()))],
            finalbody: vec![print_expr(MirExpr::StringLiteral("finally".to_string()))],
        })],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("try_star_else_falls_through")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("try_star_else_falls_through.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("try_star_else_falls_through");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"try\nelse\nfinally\n");
    assert!(output.status.success());
}

/// Complement of `a_try_star_else_that_falls_through_branches_to_finally`
/// above: `emit_try_star`'s `if else_falls_through { .. }` wrapper only
/// synthesizes its own branch to `finally_bb` when `orelse` itself falls
/// through. This fixture's `orelse` instead ends in an explicit `Return`,
/// so `orelse`'s own body already installs a terminator and
/// `else_falls_through` is false -- exercising the wrapper's implicit
/// "false" branch (nothing further emitted after `emit_body(orelse)`).
#[test]
fn a_try_star_else_that_returns_does_not_branch_to_finally() {
    let mir = MirModule {
        items: vec![MirItem::Function {
            name: "f".to_string(),
            params: vec![],
            return_ty: Ty::None,
            body: vec![MirStmt::TryStar {
                body: vec![call_print(1)],
                handlers: vec![MirExceptHandler {
                    exc_type_tag: Some(vec![1]),
                    binding_name: None,
                    binding_ty: None,
                    body: vec![call_print(99)],
                }],
                orelse: vec![return_int(4)],
                finalbody: vec![call_print(2)],
            }],
        }],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("try_star_else_returns")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("try_star_else_returns.o");
    compile_to_object(&mir, &obj_path, None, false)
        .expect("an orelse ending in Return must codegen without a spurious branch");
}

/// `emit_try_star`'s `has_finally` return-slot allocation (only reached
/// when the enclosing function has a non-`None` ABI) and the `ret_bb`
/// value-load/store path it feeds are otherwise unexercised: the
/// `TryStar` fixtures above all live at module top level, where
/// `expected_return_ty` is always `Ty::None`. This fixture also carries
/// two `except*` clauses, so the dispatch chain's "there is a next
/// clause" branch (as opposed to falling through to the unmatched-
/// remainder reraise) is exercised too.
#[test]
fn a_try_star_returning_a_value_through_its_finally_in_a_non_none_function() {
    let mir = MirModule {
        items: vec![MirItem::Function {
            name: "f".to_string(),
            params: vec![],
            return_ty: Ty::Int,
            body: vec![MirStmt::TryStar {
                body: vec![MirStmt::Return(Some(MirExpr::IntLiteral(7)))],
                handlers: vec![
                    MirExceptHandler {
                        exc_type_tag: Some(vec![1]),
                        binding_name: None,
                        binding_ty: None,
                        body: vec![MirStmt::Return(Some(MirExpr::IntLiteral(2)))],
                    },
                    MirExceptHandler {
                        exc_type_tag: Some(vec![2]),
                        binding_name: None,
                        binding_ty: None,
                        body: vec![MirStmt::Return(Some(MirExpr::IntLiteral(3)))],
                    },
                ],
                orelse: vec![],
                finalbody: vec![call_print(1)],
            }],
        }],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("try_star_return_value_through_finally")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("try_star_return_value_through_finally.o");
    compile_to_object(&mir, &obj_path, None, false)
        .expect("a value return routed through a try*'s finally must codegen");
}

/// Mirrors `try_finally_body_with_return_does_not_fall_through` above, but
/// for `MirStmt::TryStar` (#542): a `Return` in the `finally` body itself
/// terminates the block, so `finally_falls_through` is false and the
/// `if finally_falls_through { .. }` wrapper's implicit "false" branch
/// (nothing further is emitted) is exercised instead of either of its
/// `has_finally` arms.
#[test]
fn try_star_finally_body_with_return_does_not_fall_through() {
    let mir = MirModule {
        items: vec![MirItem::Function {
            name: "f".to_string(),
            params: vec![],
            return_ty: Ty::None,
            body: vec![MirStmt::TryStar {
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
    let dir = pycc_scratch::ScratchDir::new("try_star_finally_return")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("try_star_finally_return.o");
    compile_to_object(&mir, &obj_path, None, false)
        .expect("codegen should succeed for a try*'s finally body with return");
}

/// `emit_try_star`'s `finalbody.is_empty()` branch ("no finally body --
/// just check `is_returning` / branch") is otherwise unexercised: every
/// other `TryStar` fixture in this file carries a non-empty `finally`.
/// This also exercises the unmatched-remainder reraise path, since the
/// raised `ValueError` (tag 1) matches neither clause (tags 2 and 3).
#[test]
fn a_try_star_without_a_finally_reraises_an_unmatched_remainder() {
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::TryStar {
            body: vec![MirStmt::Raise {
                exception: MirExceptionValue::Constructed {
                    type_tag: 1,
                    class_name: "ValueError".to_string(),
                    message: MirExpr::StringLiteral("boom".to_string()),
                },
                frame_function: "test_fn".to_string(),
            }],
            handlers: vec![
                MirExceptHandler {
                    exc_type_tag: Some(vec![2]),
                    binding_name: None,
                    binding_ty: None,
                    body: vec![call_print(2)],
                },
                MirExceptHandler {
                    exc_type_tag: Some(vec![3]),
                    binding_name: None,
                    binding_ty: None,
                    body: vec![call_print(3)],
                },
            ],
            orelse: vec![],
            finalbody: vec![],
        })],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("try_star_no_finally_reraise")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("try_star_no_finally_reraise.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("try_star_no_finally_reraise");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"");
    assert!(
        !output.status.success(),
        "an unmatched remainder must propagate uncaught"
    );
}
