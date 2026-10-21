//! List iteration, append and module-level list bindings.

use super::*;

#[test]
fn a_return_inside_a_for_list_body_returns_immediately_without_looping() {
    // The `MirStmt::ForList` counterpart of `a_return_inside_a_for_
    // range_body_returns_immediately_without_looping` above, for the
    // same reason: `ForList`'s arm carries its own inline copy of the
    // terminator-safety guard, and without it the increment-and-branch-
    // back would build a second terminator onto a block `body`'s
    // `Return` already terminated -- IR `module.verify()` rejects.
    // Prints "1", not "1\n2\n3\n".
    let mir = MirModule {
        items: vec![
            MirItem::Function {
                name: "first_of_list".to_string(),
                params: vec![],
                return_ty: Ty::Int,
                body: vec![
                    assign_list_literal("xs"),
                    MirStmt::ForList {
                        var_ty: pycc_mir::Ty::Int,
                        var: "v".to_string(),
                        list: "xs".to_string(),
                        body: vec![MirStmt::Return(Some(MirExpr::Name {
                            name: "v".to_string(),
                            ty: Ty::Int,
                        }))],
                    },
                    // Unreachable in practice (the loop always returns
                    // on its first iteration for this non-empty list),
                    // but required to keep this hand-built MIR
                    // well-formed for a non-`None`-returning function --
                    // exactly the same caveat the `ForRange` version of
                    // this test documents.
                    MirStmt::Return(Some(MirExpr::IntLiteral(-1))),
                ],
            },
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
                callee: "print".to_string(),
                args: vec![MirExpr::Call {
                    callee: "first_of_list".to_string(),
                    args: vec![],
                    ty: Ty::Int,
                }],
                ty: Ty::None,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("for_list_return_inside_body")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("for_list_return_inside_body.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("for_list_return_inside_body");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(pycc_scratch::lf(&output.stdout), b"1\n");
}

#[test]
fn a_return_inside_a_for_set_body_returns_immediately_without_looping() {
    // The `MirStmt::ForSet` counterpart of `a_return_inside_a_for_list_
    // body_returns_immediately_without_looping` above, for the same
    // reason: `ForSet`'s arm carries its own inline copy of the
    // terminator-safety guard, and without it the increment-and-branch-
    // back would build a second terminator onto a block `body`'s
    // `Return` already terminated -- IR `module.verify()` rejects. This
    // is the one shape none of this task's other `ForSet` tests
    // exercise (they all either fall through normally or error before
    // reaching a `Return`), so it is the dedicated regression coverage
    // for that specific skip branch. Prints one element only, not
    // every element in the set.
    let mir = MirModule {
        items: vec![
            MirItem::Function {
                name: "first_of_set".to_string(),
                params: vec![],
                return_ty: Ty::Int,
                body: vec![
                    MirStmt::Assign {
                        target: "xs".to_string(),
                        value: MirExpr::SetLiteral {
                            elements: vec![
                                MirExpr::IntLiteral(1),
                                MirExpr::IntLiteral(2),
                                MirExpr::IntLiteral(3),
                            ],
                            ops: None,
                        },
                    },
                    MirStmt::ForSet {
                        var_ty: pycc_mir::Ty::Int,
                        var: "v".to_string(),
                        set: "xs".to_string(),
                        body: vec![MirStmt::Return(Some(MirExpr::Name {
                            name: "v".to_string(),
                            ty: Ty::Int,
                        }))],
                    },
                    // Unreachable in practice (the loop always returns
                    // on its first iteration for this non-empty set),
                    // but required to keep this hand-built MIR
                    // well-formed for a non-`None`-returning function --
                    // exactly the same caveat the `ForList`/`ForRange`
                    // versions of this test document.
                    MirStmt::Return(Some(MirExpr::IntLiteral(-1))),
                ],
            },
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
                callee: "print".to_string(),
                args: vec![MirExpr::Call {
                    callee: "first_of_set".to_string(),
                    args: vec![],
                    ty: Ty::Int,
                }],
                ty: Ty::None,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("for_set_return_inside_body")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("for_set_return_inside_body.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("for_set_return_inside_body");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(pycc_scratch::lf(&output.stdout), b"1\n");
}

#[test]
fn a_for_list_loop_visits_every_element_in_order() {
    // The full `MirStmt::ForList` loop, run to completion: unlike
    // `a_return_inside_a_for_list_body_returns_immediately_without_
    // looping` above, this one reaches the arm's increment-and-branch-
    // back block on every iteration and its loop test's exhaustion
    // edge, and proves encoded elements are forwarded unchanged.
    //
    // Kept as a `pycc_codegen` unit test even though
    // `tests/slice1_codegen_depth.rs` already covers the same behavior
    // from real source, because empirically that was not enough: with
    // this test and `a_module_level_list_binding_gets_a_null_
    // initialized_pointer_global` below absent, `cargo llvm-cov
    // --workspace` reported `lib.rs` at 99.68% regions with no
    // uncovered line to point at. That integration suite drives the
    // separate `pycc` binary, which links its own copy of this crate,
    // and llvm-cov's per-instantiation accounting does not always let
    // that copy stand in for the one this crate's own test binary
    // uses.
    let mir = list_fixture_module(vec![
        assign_list_literal("xs"),
        MirStmt::ForList {
            var_ty: pycc_mir::Ty::Int,
            var: "v".to_string(),
            list: "xs".to_string(),
            body: vec![MirStmt::ExprStmt(MirExpr::Call {
                callee: "print".to_string(),
                args: vec![MirExpr::Name {
                    name: "v".to_string(),
                    ty: Ty::Int,
                }],
                ty: Ty::None,
            })],
        },
    ]);
    let dir = pycc_scratch::ScratchDir::new("for_list_visits_every_element")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("for_list_visits_every_element.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("for_list_visits_every_element");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(pycc_scratch::lf(&output.stdout), b"1\n2\n3\n");
}

#[test]
fn a_for_list_loop_keeps_its_per_iteration_length_read_under_release_optimization() {
    // `MirStmt::ForList` calls `pycc_rt_int_list_len` inside its
    // loop-test block on purpose, so appending during iteration extends
    // the loop exactly as CPython's list iterator does.
    // `iterating_a_list_rereads_its_length_each_step_like_cpython` in
    // `tests/slice1_codegen_depth.rs` pins that from real source, but
    // only for an unoptimized build, where nothing could hoist the call
    // anyway. `--release` additionally runs LLVM's `"default<O3>"`
    // pipeline (D-094), whose LICM pass is precisely the transform that
    // would lift a loop-invariant-looking call out of the loop and
    // silently restore the hoisted behavior. It does not today --
    // `declare_rt_functions` gives the declaration no attributes, so
    // LLVM must assume the call may write memory -- but that is an
    // inference from an absence, and a future PR adding
    // `readonly`/`willreturn` to these externs for performance would
    // invalidate it with no other release-profile test noticing.
    //
    // The fixture therefore has to *mutate* `xs` mid-loop. An earlier
    // version of this test iterated a fixed `[1, 2, 3]` and asserted
    // `1\n2\n3\n`, which a hoisted length read satisfies just as well --
    // `len(xs)` is loop-invariant at 3 either way, so the test could not
    // fail for the reason it is named after. Confirmed by patching this
    // arm to compute the length in the preheader: that version still
    // passed. This one grows the list from 1 element to 3 while
    // iterating, so a length read hoisted out of the loop sees 1, runs a
    // single iteration, and prints only "1".
    //
    // `if len(xs) < 3: xs.append(v + 1)` then `print(v)`, the same
    // program the end-to-end test above uses.
    let list_int = || Ty::List(Box::new(Ty::Int));
    let mir = list_fixture_module(vec![
        MirStmt::Assign {
            target: "xs".to_string(),
            value: MirExpr::ListLiteral(vec![MirExpr::IntLiteral(1)]),
        },
        MirStmt::ForList {
            var_ty: pycc_mir::Ty::Int,
            var: "v".to_string(),
            list: "xs".to_string(),
            body: vec![
                MirStmt::If {
                    test: MirExpr::Compare {
                        op: CmpOpKind::Lt,
                        left: Box::new(MirExpr::Call {
                            callee: "len".to_string(),
                            args: vec![MirExpr::Name {
                                name: "xs".to_string(),
                                ty: list_int(),
                            }],
                            ty: Ty::Int,
                        }),
                        right: Box::new(MirExpr::IntLiteral(3)),
                        ty: Ty::Bool,
                    },
                    body: vec![MirStmt::ExprStmt(MirExpr::ListAppend {
                        list: pycc_mir::MirContainerReceiver::Name("xs".to_string()),
                        value: Box::new(MirExpr::BinOp {
                            op: BinOpKind::Add,
                            left: Box::new(MirExpr::Name {
                                name: "v".to_string(),
                                ty: Ty::Int,
                            }),
                            right: Box::new(MirExpr::IntLiteral(1)),
                            ty: Ty::Int,
                        }),
                    })],
                    orelse: vec![],
                },
                MirStmt::ExprStmt(MirExpr::Call {
                    callee: "print".to_string(),
                    args: vec![MirExpr::Name {
                        name: "v".to_string(),
                        ty: Ty::Int,
                    }],
                    ty: Ty::None,
                }),
            ],
        },
    ]);
    let dir =
        pycc_scratch::ScratchDir::new("for_list_release").expect("failed to create scratch dir");
    let obj_path = dir.join("for_list_release.o");
    compile_to_object(&mir, &obj_path, None, true).expect("release codegen should succeed");
    let bin_path = dir.join("for_list_release");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(pycc_scratch::lf(&output.stdout), b"1\n2\n3\n");
}

#[test]
fn appending_to_a_list_validates_and_preserves_the_encoded_value() {
    // `MirExpr::ListAppend`'s success path, the one new arm whose body
    // no other unit test in this file reaches (`appending_to_a_non_
    // list_local_is_an_internal_error` above panics inside
    // `emit_list_name_read` before any of it runs). Reading the
    // appended element straight back out pins D-141's round trip for
    // this arm: ingress validates but stores the encoded word unchanged.
    let mir = list_fixture_module(vec![
        assign_list_literal("xs"),
        MirStmt::ExprStmt(MirExpr::ListAppend {
            list: pycc_mir::MirContainerReceiver::Name("xs".to_string()),
            value: Box::new(MirExpr::IntLiteral(4)),
        }),
        MirStmt::ExprStmt(MirExpr::Call {
            callee: "print".to_string(),
            args: vec![MirExpr::Subscript {
                base: Box::new(MirExpr::Name {
                    name: "xs".to_string(),
                    ty: Ty::List(Box::new(Ty::Int)),
                }),
                index: Box::new(MirExpr::IntLiteral(3)),
            }],
            ty: Ty::None,
        }),
    ]);
    let dir = pycc_scratch::ScratchDir::new("list_append_round_trip")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("list_append_round_trip.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("list_append_round_trip");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(pycc_scratch::lf(&output.stdout), b"4\n");
}

#[test]
fn a_module_level_list_binding_gets_a_null_initialized_pointer_global() {
    // `declare_module_globals`' `Ty::List(_)` arm: a module-scope
    // `xs = [1, 2, 3]` (one of the two places D-105's first scope cut
    // says a `list[int]` value may live) becomes an LLVM global rather
    // than a function-local alloca. Task 5 (D-089) deliberately left
    // this arm out while no real source could build a list value, and
    // flagged re-deriving it for Task 11; without it this exact MIR
    // panics with "a `list[int]`-typed module binding is not supported
    // yet".
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(assign_list_literal("xs")),
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
                callee: "print".to_string(),
                args: vec![MirExpr::Call {
                    callee: "len".to_string(),
                    args: vec![MirExpr::Name {
                        name: "xs".to_string(),
                        ty: Ty::List(Box::new(Ty::Int)),
                    }],
                    ty: Ty::Int,
                }],
                ty: Ty::None,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("module_level_list_global")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("module_level_list_global.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("module_level_list_global");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(pycc_scratch::lf(&output.stdout), b"3\n");
}

#[test]
fn an_error_inside_a_for_list_body_propagates_out_of_codegen() {
    // `MirStmt::ForList`'s arm emits its body through `emit_body`,
    // whose `Result` it must propagate rather than swallow -- the same
    // `?`-propagation every other body-emitting arm relies on (see
    // `public_codegen_api_propagates_an_error_from_a_function_body` in
    // `tests/slice1_codegen_depth.rs`). A call to an undefined function
    // is the one failure `emit_stmt` reports as a clean `Err` instead
    // of a panic.
    let mir = list_fixture_module(vec![
        assign_list_literal("xs"),
        MirStmt::ForList {
            var_ty: pycc_mir::Ty::Int,
            var: "v".to_string(),
            list: "xs".to_string(),
            body: vec![call_user_fn("missing")],
        },
    ]);
    let dir =
        pycc_scratch::ScratchDir::new("for_list_body_error").expect("failed to create scratch dir");
    let error = compile_to_object(&mir, &dir.join("for_list_body_error.o"), None, false)
        .expect_err("the undefined call inside the loop body should fail");
    assert!(error.contains("missing"));
}

#[test]
fn a_bool_list_element_keeps_its_identity_in_encoded_storage() {
    // `xs = [True, 1]` never reaches codegen from real source
    // (`pycc_types` rejects a mixed literal with T0032, and an
    // all-`bool` one with T0034), but `MirExpr::ListLiteral`'s arm
    // deliberately routes each element through `to_encoded_int` rather
    // than requiring a `Scalar::Int` outright -- Python's `bool` is an
    // `int` subtype, so widening is the correct answer if a `bool`
    // element ever does reach it, exactly as `emit_expr`'s own
    // `BinOp`/`Ty::Int` arm treats a `bool` operand. D-141 requires the
    // stored marker to print `True`, while arithmetic still produces `1`.
    let mir = list_fixture_module(vec![
        MirStmt::Assign {
            target: "xs".to_string(),
            value: MirExpr::ListLiteral(vec![MirExpr::BoolLiteral(true)]),
        },
        MirStmt::ExprStmt(MirExpr::Call {
            callee: "print".to_string(),
            args: vec![MirExpr::Subscript {
                base: Box::new(MirExpr::Name {
                    name: "xs".to_string(),
                    ty: Ty::List(Box::new(Ty::Bool)),
                }),
                index: Box::new(MirExpr::IntLiteral(0)),
            }],
            ty: Ty::None,
        }),
    ]);
    let dir = pycc_scratch::ScratchDir::new("bool_list_element_identity")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("bool_list_element_identity.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("bool_list_element_identity");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(pycc_scratch::lf(&output.stdout), b"True\n");
}
