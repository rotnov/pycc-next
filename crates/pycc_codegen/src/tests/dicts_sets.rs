//! Dict and set construction, iteration, parameters and key ownership.

use super::*;

#[test]
fn dict_get_codegens_and_runs() {
    // `x = {"a": 1, "b": 2}\nprint(x["b"])\n` end to end -- real
    // `MirExpr::DictGet` read codegen (PR-11 Task 5). Expected output
    // verified against `python3` on this exact source.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "x".to_string(),
                value: MirExpr::DictLiteral(vec![
                    (
                        MirExpr::StringLiteral("a".to_string()),
                        MirExpr::IntLiteral(1),
                    ),
                    (
                        MirExpr::StringLiteral("b".to_string()),
                        MirExpr::IntLiteral(2),
                    ),
                ]),
            }),
            MirItem::TopLevelStmt(print_expr(MirExpr::DictGet {
                dict: Box::new(dict_name("x")),
                key: Box::new(MirExpr::StringLiteral("b".to_string())),
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("dict_get").expect("failed to create scratch dir");
    let obj_path = dir.join("dict_get.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("dict_get");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(pycc_scratch::lf(&output.stdout), b"2\n");
}

#[test]
fn dict_set_item_updates_an_existing_key_in_place() {
    // `x = {"a": 1}\nx["a"] = 5\nprint(x["a"])\nprint(len(x))\n` end to
    // end -- real `MirStmt::DictSet` insert-or-update codegen (PR-11
    // Task 5, D-123), exercising its update-in-place half: `len(x)`
    // staying `1` (not growing to `2`) is exactly what distinguishes
    // an update from an append. Expected output verified against
    // `python3` on this exact source.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "x".to_string(),
                value: MirExpr::DictLiteral(vec![(
                    MirExpr::StringLiteral("a".to_string()),
                    MirExpr::IntLiteral(1),
                )]),
            }),
            MirItem::TopLevelStmt(MirStmt::DictSet {
                dict: "x".to_string(),
                key: MirExpr::StringLiteral("a".to_string()),
                value: MirExpr::IntLiteral(5),
            }),
            MirItem::TopLevelStmt(print_expr(MirExpr::DictGet {
                dict: Box::new(dict_name("x")),
                key: Box::new(MirExpr::StringLiteral("a".to_string())),
            })),
            MirItem::TopLevelStmt(print_expr(MirExpr::Call {
                callee: "len".to_string(),
                args: vec![dict_name("x")],
                ty: Ty::Int,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir =
        pycc_scratch::ScratchDir::new("dict_set_update").expect("failed to create scratch dir");
    let obj_path = dir.join("dict_set_update.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("dict_set_update");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(pycc_scratch::lf(&output.stdout), b"5\n1\n");
}

#[test]
fn dict_set_item_appends_a_new_key() {
    // `x = {"a": 1}\nx["b"] = 2\nprint(len(x))\n` end to end --
    // `MirStmt::DictSet`'s append-a-new-key half (PR-11 Task 5,
    // D-123): `len(x)` growing to `2` is exactly what distinguishes an
    // append from an update. Expected output verified against
    // `python3` on this exact source.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "x".to_string(),
                value: MirExpr::DictLiteral(vec![(
                    MirExpr::StringLiteral("a".to_string()),
                    MirExpr::IntLiteral(1),
                )]),
            }),
            MirItem::TopLevelStmt(MirStmt::DictSet {
                dict: "x".to_string(),
                key: MirExpr::StringLiteral("b".to_string()),
                value: MirExpr::IntLiteral(2),
            }),
            MirItem::TopLevelStmt(print_expr(MirExpr::Call {
                callee: "len".to_string(),
                args: vec![dict_name("x")],
                ty: Ty::Int,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir =
        pycc_scratch::ScratchDir::new("dict_set_append").expect("failed to create scratch dir");
    let obj_path = dir.join("dict_set_append.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("dict_set_append");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(pycc_scratch::lf(&output.stdout), b"2\n");
}

#[test]
fn for_k_in_dict_iterates_keys_in_insertion_order() {
    // `x = {"b": 2, "a": 1}\nfor k in x:\n    print(k)\n` end to end --
    // real `MirStmt::ForDict` iteration codegen (PR-11 Task 5, D-123).
    // "b" printing before "a" (insertion order, not sorted order) is
    // the actual point of this test: `PyDictObj`'s own insertion-order
    // guarantee (D-121) surviving through `pycc_rt_dict_key_at`.
    // Expected output verified against `python3` on this exact source.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "x".to_string(),
                value: MirExpr::DictLiteral(vec![
                    (
                        MirExpr::StringLiteral("b".to_string()),
                        MirExpr::IntLiteral(2),
                    ),
                    (
                        MirExpr::StringLiteral("a".to_string()),
                        MirExpr::IntLiteral(1),
                    ),
                ]),
            }),
            MirItem::TopLevelStmt(MirStmt::ForDict {
                var: "k".to_string(),
                var_ty: pycc_mir::Ty::Str,
                dict: "x".to_string(),
                body: vec![print_expr(MirExpr::Name {
                    name: "k".to_string(),
                    ty: Ty::Str,
                })],
            }),
        ],
        class_defs: Vec::new(),
    };
    let dir =
        pycc_scratch::ScratchDir::new("for_dict_iteration").expect("failed to create scratch dir");
    let obj_path = dir.join("for_dict_iteration.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("for_dict_iteration");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(pycc_scratch::lf(&output.stdout), b"b\na\n");
}

#[test]
fn reassigning_the_for_dict_loop_variable_inside_the_body_does_not_corrupt_the_dict() {
    // The regression test for this arm's own doc comment (point 3):
    // `for k in x:\n    k = "z"\n    print(k)\nprint(len(x))\nprint(x["a"])\n`
    // reassigns the loop variable on every iteration, which -- without
    // the `pycc_rt_str_incref` `MirStmt::ForDict`'s codegen gives the
    // loop variable's own slot -- would decref (and, on the last
    // iteration, free) a key `x` itself still points to, corrupting
    // the dict for the two `print`s that follow the loop. Both survive
    // intact if the incref is doing its job. Expected output verified
    // against `python3` on this exact source (CPython, of course, has
    // no such hazard at all -- this test is pinning pycc's own
    // representation-specific safety property, not a language
    // semantic).
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "x".to_string(),
                value: MirExpr::DictLiteral(vec![(
                    MirExpr::StringLiteral("a".to_string()),
                    MirExpr::IntLiteral(1),
                )]),
            }),
            MirItem::TopLevelStmt(MirStmt::ForDict {
                var: "k".to_string(),
                var_ty: pycc_mir::Ty::Str,
                dict: "x".to_string(),
                body: vec![
                    MirStmt::Assign {
                        target: "k".to_string(),
                        value: MirExpr::StringLiteral("z".to_string()),
                    },
                    print_expr(MirExpr::Name {
                        name: "k".to_string(),
                        ty: Ty::Str,
                    }),
                ],
            }),
            MirItem::TopLevelStmt(print_expr(MirExpr::Call {
                callee: "len".to_string(),
                args: vec![dict_name("x")],
                ty: Ty::Int,
            })),
            MirItem::TopLevelStmt(print_expr(MirExpr::DictGet {
                dict: Box::new(dict_name("x")),
                key: Box::new(MirExpr::StringLiteral("a".to_string())),
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("for_dict_var_reassignment")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("for_dict_var_reassignment.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("for_dict_var_reassignment");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(pycc_scratch::lf(&output.stdout), b"z\n1\n1\n");
}

#[test]
fn set_literal_construction_dedups_and_reports_correct_len() {
    // `x = {1, 2, 2, 3}\nprint(len(x))\n` end to end through
    // `compile_to_object` -- real `MirExpr::SetLiteral` construction
    // (PR-11 Task 9) plus `len()`'s new `Ty::Set` branch. The dedup
    // check lives entirely in `pycc_rt_int_set_add` (Task 6); this test
    // proves codegen actually calls it per element, unconditionally,
    // and that the repeated `2` collapses to one element. Expected
    // output verified against `python3` on this exact source (a set
    // literal's own construction order does not affect its `len`).
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "x".to_string(),
                value: MirExpr::SetLiteral {
                    elements: vec![
                        MirExpr::IntLiteral(1),
                        MirExpr::IntLiteral(2),
                        MirExpr::IntLiteral(2),
                        MirExpr::IntLiteral(3),
                    ],
                    ops: None,
                },
            }),
            MirItem::TopLevelStmt(print_expr(MirExpr::Call {
                callee: "len".to_string(),
                args: vec![set_name("x")],
                ty: Ty::Int,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir =
        pycc_scratch::ScratchDir::new("set_literal_and_len").expect("failed to create scratch dir");
    let obj_path = dir.join("set_literal_and_len.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("set_literal_and_len");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(pycc_scratch::lf(&output.stdout), b"3\n");
}

#[test]
fn for_x_in_set_iterates_in_cpython_order() {
    // `x = {2, 1, 2}\nfor v in x:\n    print(v)\n` end to end -- real
    // `MirStmt::ForSet` iteration codegen. `pycc_rt::set` keeps CPython's
    // own hash table, so the order is CPython's: `1` before `2`, as
    // `python3` prints for this exact source.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "x".to_string(),
                value: MirExpr::SetLiteral {
                    elements: vec![
                        MirExpr::IntLiteral(2),
                        MirExpr::IntLiteral(1),
                        MirExpr::IntLiteral(2),
                    ],
                    ops: None,
                },
            }),
            MirItem::TopLevelStmt(MirStmt::ForSet {
                var_ty: pycc_mir::Ty::Int,
                var: "v".to_string(),
                set: "x".to_string(),
                body: vec![print_expr(MirExpr::Name {
                    name: "v".to_string(),
                    ty: Ty::Int,
                })],
            }),
        ],
        class_defs: Vec::new(),
    };
    let dir =
        pycc_scratch::ScratchDir::new("for_set_iteration").expect("failed to create scratch dir");
    let obj_path = dir.join("for_set_iteration.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("for_set_iteration");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(pycc_scratch::lf(&output.stdout), b"1\n2\n");
}

#[test]
#[should_panic(expected = "`n` did not evaluate to a set")]
fn for_set_over_a_non_set_local_is_an_internal_error() {
    // `n = 1` then `for v in n:`: `pycc_types` rejects this with T0033
    // ("not iterable"), so codegen only sees it as hand-built malformed
    // MIR -- mirrors `for_dict_over_an_unbound_name_is_an_internal_
    // error`/`appending_to_a_non_list_local_is_an_internal_error`
    // above, for the identical reason. Covers `emit_set_name_read`'s
    // own use of `expect_set_pointer`.
    let mir = list_fixture_module(vec![
        MirStmt::Assign {
            target: "n".to_string(),
            value: MirExpr::IntLiteral(1),
        },
        MirStmt::ForSet {
            var_ty: pycc_mir::Ty::Int,
            var: "v".to_string(),
            set: "n".to_string(),
            body: vec![],
        },
    ]);
    let dir = pycc_scratch::ScratchDir::new("for_set_on_non_set_panics")
        .expect("failed to create scratch dir");
    let _ = compile_to_object(&mir, &dir.join("for_set_on_non_set_panics.o"), None, false);
}

#[test]
#[should_panic(expected = "`never_bound` has no local slot")]
fn for_set_over_an_unbound_name_is_an_internal_error() {
    // `for v in never_bound:` where nothing ever bound `never_bound` --
    // mirrors `iterating_a_name_with_no_local_slot_is_an_internal_
    // error`/`for_dict_over_an_unbound_name_is_an_internal_error`
    // above, for the identical reason: codegen's own defensive
    // backstop for `emit_set_name_read`'s slot lookup, the one branch
    // `for_set_over_a_non_set_local_is_an_internal_error` above does
    // not reach.
    let mir = list_fixture_module(vec![MirStmt::ForSet {
        var_ty: pycc_mir::Ty::Int,
        var: "v".to_string(),
        set: "never_bound".to_string(),
        body: vec![],
    }]);
    let dir = pycc_scratch::ScratchDir::new("for_set_unbound_name_panics")
        .expect("failed to create scratch dir");
    let _ = compile_to_object(
        &mir,
        &dir.join("for_set_unbound_name_panics.o"),
        None,
        false,
    );
}

#[test]
fn compiles_a_function_with_a_set_int_parameter_and_set_int_return_value() {
    // The `set[int]` counterpart of `compiles_a_function_with_a_dict_
    // str_int_parameter_and_dict_str_int_return_value` above, for the
    // identical reason. Since D-228 (issue #918) an annotated `set[int]`
    // *parameter* reaches codegen from real source, and since #925 a
    // `-> set[int]` return annotation does too; the combined shape below
    // is kept hand-built as the direct codegen-level pin, independent of
    // the front end. This MIR shape must still compile
    // *cleanly* -- `ty_to_basic_type`'s `Set(_)` arm and `emit_expr`'s
    // `Name` arm's `Set(_)` arm must agree on the same pointer
    // representation, and `MirStmt::Return`'s own `Scalar::Set`
    // pass-through arm must actually build a valid `ret` instruction.
    // Deliberately does not link or run the resulting object, for the
    // identical reason the dict counterpart doesn't.
    let mir = MirModule {
        items: vec![MirItem::Function {
            name: "f".to_string(),
            params: vec![("x".to_string(), set_int())],
            return_ty: set_int(),
            body: vec![MirStmt::Return(Some(MirExpr::Name {
                name: "x".to_string(),
                ty: set_int(),
            }))],
        }],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("set_int_param_and_return")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("set_int_param_and_return.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
}

#[test]
fn passing_a_set_value_as_a_function_argument_marshals_it_like_a_pointer() {
    // The `set[int]` counterpart of `passing_a_dict_value_as_a_
    // function_argument_marshals_it_like_a_pointer` above: the caller
    // adds the one shape the test directly above does not reach --
    // `build_call_to`'s argument-marshalling match, whose `Scalar::Set`
    // arm is also in the pass-through bucket. Same not-linked, not-run
    // caveat as the dict counterpart: neither `f` nor `g` is ever
    // called, and their annotated `set[int]` parameters are
    // unreachable from real source, so this proves the codegen shape
    // only.
    let mir = MirModule {
        items: vec![
            MirItem::Function {
                name: "f".to_string(),
                params: vec![("x".to_string(), set_int())],
                return_ty: Ty::Int,
                body: vec![MirStmt::Return(Some(MirExpr::IntLiteral(0)))],
            },
            MirItem::Function {
                name: "g".to_string(),
                params: vec![("x".to_string(), set_int())],
                return_ty: Ty::Int,
                body: vec![MirStmt::Return(Some(MirExpr::Call {
                    callee: "f".to_string(),
                    args: vec![MirExpr::Name {
                        name: "x".to_string(),
                        ty: set_int(),
                    }],
                    ty: Ty::Int,
                }))],
            },
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("set_int_passed_as_argument")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("set_int_passed_as_argument.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
}

#[test]
fn an_error_inside_a_for_set_body_propagates_out_of_codegen() {
    // `MirStmt::ForSet`'s arm emits its body through `emit_body`, whose
    // `Result` it must propagate rather than swallow -- mirrors
    // `an_error_inside_a_for_dict_body_propagates_out_of_codegen`
    // above, for the identical reason. A call to an undefined function
    // is the one failure `emit_stmt` reports as a clean `Err` instead
    // of a panic.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "x".to_string(),
                value: MirExpr::SetLiteral {
                    elements: vec![MirExpr::IntLiteral(1)],
                    ops: None,
                },
            }),
            MirItem::TopLevelStmt(MirStmt::ForSet {
                var_ty: pycc_mir::Ty::Int,
                var: "v".to_string(),
                set: "x".to_string(),
                body: vec![call_user_fn("missing")],
            }),
        ],
        class_defs: Vec::new(),
    };
    let dir =
        pycc_scratch::ScratchDir::new("for_set_body_error").expect("failed to create scratch dir");
    let error = compile_to_object(&mir, &dir.join("for_set_body_error.o"), None, false)
        .expect_err("the undefined call inside the loop body should fail");
    assert!(error.contains("missing"));
}

#[test]
fn truthiness_of_a_set_value_is_its_non_emptiness() {
    // Part 1 of #1319 replaced the honest panic this test used to pin:
    // `if s:` for a `set[int]`/`frozenset[int]` value now reads the
    // runtime length and compares it with zero; `truthy` then narrows the
    // arm's `i8` to the `i1` a branch consumes.
    let context = Context::create();
    let (module, rt) = list_scalar_panic_fixture(&context);
    let builder = context.create_builder();
    let f = module.add_function("f", context.void_type().fn_type(&[], false), None);
    builder.position_at_end(context.append_basic_block(f, "entry"));
    let ptr = context
        .ptr_type(inkwell::AddressSpace::default())
        .const_null();
    let bit = truthy(&context, &builder, &module, &rt, Scalar::Set(ptr));
    assert_eq!(bit.get_type().get_bit_width(), 1);
    let ir = llvm_string_to_owned(module.print_to_string());
    assert!(ir.contains("call i64 @pycc_rt_set_len"), "{ir}");
}

#[test]
#[should_panic(expected = "internal error: expected an int-or-bool operand, got set")]
fn to_numeric_encoded_int_rejects_a_set_operand() {
    // The `set[T]` counterpart of `to_numeric_encoded_int_rejects_a_dict_
    // operand` above -- genuinely defensive for the identical reason:
    // `pycc_types`' `numeric_result_type` has no `as_numeric` mapping
    // for `Ty::Set` either, so no real MIR reaches this arm.
    let context = Context::create();
    let builder = context.create_builder();
    let ptr = context
        .ptr_type(inkwell::AddressSpace::default())
        .const_null();
    to_numeric_encoded_int(&context, &builder, Scalar::Set(ptr));
}

#[test]
#[should_panic(expected = "internal error: expected a numeric operand, got set")]
fn to_float_rejects_a_set_operand() {
    // Same defensive-arm rationale as `to_numeric_encoded_int_rejects_a_set_
    // operand` directly above, for `to_float`'s own match.
    let context = Context::create();
    let (_module, rt) = list_scalar_panic_fixture(&context);
    let builder = context.create_builder();
    let ptr = context
        .ptr_type(inkwell::AddressSpace::default())
        .const_null();
    to_float(&context, &builder, &rt, Scalar::Set(ptr));
}

#[test]
#[should_panic(expected = "internal error: expected a dict, got `int`")]
fn dict_set_item_on_a_non_dict_local_is_an_internal_error() {
    // `n = 1` then `n["a"] = 2`: `pycc_types` rejects this with T0033
    // ("does not support item assignment"), so codegen only sees it as
    // hand-built malformed MIR -- mirrors `appending_to_a_non_list_
    // local_is_an_internal_error` above, for the identical reason.
    // Covers `emit_dict_name_read`'s own use of `expect_dict_pointer`,
    // the shared check `MirStmt::ForDict`'s dict operand also goes
    // through.
    let mir = list_fixture_module(vec![
        MirStmt::Assign {
            target: "n".to_string(),
            value: MirExpr::IntLiteral(1),
        },
        MirStmt::DictSet {
            dict: "n".to_string(),
            key: MirExpr::StringLiteral("a".to_string()),
            value: MirExpr::IntLiteral(2),
        },
    ]);
    let dir = pycc_scratch::ScratchDir::new("dict_set_on_non_dict_panics")
        .expect("failed to create scratch dir");
    let _ = compile_to_object(
        &mir,
        &dir.join("dict_set_on_non_dict_panics.o"),
        None,
        false,
    );
}

#[test]
#[should_panic(expected = "`never_bound` has no local slot")]
fn for_dict_over_an_unbound_name_is_an_internal_error() {
    // `for k in never_bound:` where nothing ever bound `never_bound` --
    // mirrors `iterating_a_name_with_no_local_slot_is_an_internal_error`
    // above, for the identical reason: codegen's own defensive backstop
    // for `emit_dict_name_read`'s slot lookup, the one branch
    // `dict_set_item_on_a_non_dict_local_is_an_internal_error` above
    // does not reach.
    let mir = list_fixture_module(vec![MirStmt::ForDict {
        var: "k".to_string(),
        var_ty: pycc_mir::Ty::Str,
        dict: "never_bound".to_string(),
        body: vec![],
    }]);
    let dir = pycc_scratch::ScratchDir::new("for_dict_unbound_name_panics")
        .expect("failed to create scratch dir");
    let _ = compile_to_object(
        &mir,
        &dir.join("for_dict_unbound_name_panics.o"),
        None,
        false,
    );
}

#[test]
fn compiles_a_function_with_a_dict_str_int_parameter_and_dict_str_int_return_value() {
    // The `dict[str, int]` counterpart of `compiles_a_function_with_a_
    // list_int_parameter_and_list_int_return_value` above, for the
    // identical reason. Since D-228 (issue #918) an annotated
    // `dict[str, int]` *parameter* reaches codegen from real source, and
    // since #925 a `-> dict[str, int]` return annotation does too; the
    // combined shape below is kept hand-built as the direct
    // codegen-level pin. This MIR shape must still
    // compile *cleanly* -- `ty_to_basic_type`'s `Dict(_)` arm and
    // `emit_expr`'s `Name` arm's `Dict(_)` arm must agree on the same
    // pointer representation, and `MirStmt::Return`'s own `Scalar::
    // Dict` pass-through arm must actually build a valid `ret`
    // instruction. Deliberately does not link or run the resulting
    // object, for the identical reason the list counterpart doesn't.
    let dict_str_int = || Ty::Dict(Box::new((Ty::Str, Ty::Int)));
    let mir = MirModule {
        items: vec![MirItem::Function {
            name: "f".to_string(),
            params: vec![("x".to_string(), dict_str_int())],
            return_ty: dict_str_int(),
            body: vec![MirStmt::Return(Some(MirExpr::Name {
                name: "x".to_string(),
                ty: dict_str_int(),
            }))],
        }],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("dict_str_int_param_and_return")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("dict_str_int_param_and_return.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
}

#[test]
fn passing_a_dict_value_as_a_function_argument_marshals_it_like_a_pointer() {
    // The `dict[str, int]` counterpart of `passing_a_list_value_as_a_
    // function_argument_marshals_it_like_a_pointer` above: the caller
    // adds the one shape the test directly above does not reach --
    // `build_call_to`'s argument-marshalling match, whose `Scalar::
    // Dict` arm is also in the pass-through bucket. Same not-linked,
    // not-run caveat as the list counterpart: neither `f` nor `g` is
    // ever called, and their annotated `dict[str, int]` parameters are
    // unreachable from real source, so this proves the codegen shape
    // only.
    let dict_str_int = || Ty::Dict(Box::new((Ty::Str, Ty::Int)));
    let mir = MirModule {
        items: vec![
            MirItem::Function {
                name: "f".to_string(),
                params: vec![("x".to_string(), dict_str_int())],
                return_ty: Ty::Int,
                body: vec![MirStmt::Return(Some(MirExpr::IntLiteral(0)))],
            },
            MirItem::Function {
                name: "g".to_string(),
                params: vec![("x".to_string(), dict_str_int())],
                return_ty: Ty::Int,
                body: vec![MirStmt::Return(Some(MirExpr::Call {
                    callee: "f".to_string(),
                    args: vec![MirExpr::Name {
                        name: "x".to_string(),
                        ty: dict_str_int(),
                    }],
                    ty: Ty::Int,
                }))],
            },
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("dict_str_int_passed_as_argument")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("dict_str_int_passed_as_argument.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
}

#[test]
fn an_error_inside_a_for_dict_body_propagates_out_of_codegen() {
    // `MirStmt::ForDict`'s arm emits its body through `emit_body`,
    // whose `Result` it must propagate rather than swallow -- mirrors
    // `an_error_inside_a_for_list_body_propagates_out_of_codegen`
    // above, for the identical reason. A call to an undefined function
    // is the one failure `emit_stmt` reports as a clean `Err` instead
    // of a panic.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "x".to_string(),
                value: MirExpr::DictLiteral(vec![(
                    MirExpr::StringLiteral("a".to_string()),
                    MirExpr::IntLiteral(1),
                )]),
            }),
            MirItem::TopLevelStmt(MirStmt::ForDict {
                var: "k".to_string(),
                var_ty: pycc_mir::Ty::Str,
                dict: "x".to_string(),
                body: vec![call_user_fn("missing")],
            }),
        ],
        class_defs: Vec::new(),
    };
    let dir =
        pycc_scratch::ScratchDir::new("for_dict_body_error").expect("failed to create scratch dir");
    let error = compile_to_object(&mir, &dir.join("for_dict_body_error.o"), None, false)
        .expect_err("the undefined call inside the loop body should fail");
    assert!(error.contains("missing"));
}

#[test]
fn a_dict_literal_key_read_from_a_variable_is_increfed_before_storage() {
    // Regression test for a confirmed use-after-free a pinned-reviewer
    // pass on this task caught: `pycc_rt_dict_set` adopts whatever key
    // pointer it is given as `PyDictObj`'s own permanent reference,
    // without incref'ing it itself (D-124: "neither increfed on insert
    // nor decrefed ... ever"). `{k: 1}` where `k` is a bare `str`
    // variable is exactly the *duplicate*-reference shape `incref_if_
    // str_duplicate` exists to protect at every other ownership-taking
    // boundary (`MirStmt::Assign`, `build_call_to`'s argument
    // marshalling, `MirStmt::Return`) -- without incref'ing it here
    // too, a later `k = ...` reassignment would decref (and,at
    // refcount 1, free) the exact `PyStrObj` the dict still points to.
    // Checked via the same object-symbol-presence technique
    // `passing_a_list_value_as_a_function_argument_marshals_it_like_a_
    // pointer` above uses: this minimal fixture has no other reason to
    // reference `pycc_rt_str_incref` at all, so the assertion is a
    // deterministic proxy for "the fix's `incref_if_str_duplicate` call
    // actually ran," not dependent on allocator behavior the way
    // observing an actual corrupted read would be.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "k".to_string(),
                value: MirExpr::StringLiteral("a".to_string()),
            }),
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "x".to_string(),
                value: MirExpr::DictLiteral(vec![(
                    MirExpr::Name {
                        name: "k".to_string(),
                        ty: Ty::Str,
                    },
                    MirExpr::IntLiteral(1),
                )]),
            }),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("dict_literal_variable_key_incref")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("dict_literal_variable_key_incref.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let obj_bytes = std::fs::read(&obj_path).expect("object file should be readable");
    let references_symbol =
        |name: &str| obj_bytes.windows(name.len()).any(|w| w == name.as_bytes());
    assert!(
        references_symbol("pycc_rt_str_incref"),
        "a variable-keyed dict literal must incref its key's shared PyStrObj \
             before handing it to pycc_rt_dict_set, or a later reassignment of the \
             source variable could free memory the dict still points to"
    );
}

#[test]
fn dict_set_item_key_read_from_a_variable_is_increfed_before_storage() {
    // Same regression coverage as `a_dict_literal_key_read_from_a_
    // variable_is_increfed_before_storage` above, for `MirStmt::
    // DictSet`'s own key -- the statement-level `d[k] = v` counterpart
    // of the expression-level dict literal.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "x".to_string(),
                value: MirExpr::DictLiteral(vec![(
                    MirExpr::StringLiteral("z".to_string()),
                    MirExpr::IntLiteral(0),
                )]),
            }),
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "k".to_string(),
                value: MirExpr::StringLiteral("a".to_string()),
            }),
            MirItem::TopLevelStmt(MirStmt::DictSet {
                dict: "x".to_string(),
                key: MirExpr::Name {
                    name: "k".to_string(),
                    ty: Ty::Str,
                },
                value: MirExpr::IntLiteral(1),
            }),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("dict_set_variable_key_incref")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("dict_set_variable_key_incref.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let obj_bytes = std::fs::read(&obj_path).expect("object file should be readable");
    let references_symbol =
        |name: &str| obj_bytes.windows(name.len()).any(|w| w == name.as_bytes());
    assert!(
        references_symbol("pycc_rt_str_incref"),
        "a variable-keyed d[k] = v must incref its key's shared PyStrObj before \
             handing it to pycc_rt_dict_set, or a later reassignment of the source \
             variable could free memory the dict still points to"
    );
}
