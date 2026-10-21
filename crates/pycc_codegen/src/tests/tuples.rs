//! Tuple construction, subscripts, parameters and returns.

use super::*;

/// `tuple[int, bool, float]`, the heterogeneous shape most of the
/// tuple fixtures below use.
fn tuple_int_bool_float() -> Ty {
    Ty::Tuple(Box::new(vec![Ty::Int, Ty::Bool, Ty::Float]))
}

#[test]
fn tuple_construction_and_literal_index_reads_codegen_and_run() {
    // `t = (1, True, 2.5)` followed by `print(t[0])`/`print(t[1])`/
    // `print(t[2])` end to end through `compile_to_object` -- the first
    // test to actually execute PR-11b Task 5's `MirExpr::TupleLiteral`
    // construction (`insertvalue`) and `MirExpr::Subscript`'s tuple
    // branch (`extractvalue`). Expected output verified against
    // `python3` on this exact source.
    //
    // Heterogeneous on purpose, and reading all three positions on
    // purpose. A single-type tuple would not catch a positional
    // mix-up, and the `int` read in particular is what pins D-115's
    // encoded-word rule: a tuple field stores the already D-141 encoded
    // value, so `Subscript`'s tuple branch must not re-tag it. List and
    // dict reads now follow the same pass-through rule. With a stray
    // `raw_i64_to_tagged_int` here this prints `3` instead of `1`,
    // while the `bool` and `float` reads would both still pass -- which
    // is exactly why this asserts the `int` line too.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "t".to_string(),
                value: MirExpr::TupleLiteral(vec![
                    MirExpr::IntLiteral(1),
                    MirExpr::BoolLiteral(true),
                    MirExpr::FloatLiteral(2.5),
                ]),
            }),
            MirItem::TopLevelStmt(print_expr(MirExpr::Subscript {
                base: Box::new(MirExpr::Name {
                    name: "t".to_string(),
                    ty: tuple_int_bool_float(),
                }),
                index: Box::new(MirExpr::IntLiteral(0)),
            })),
            MirItem::TopLevelStmt(print_expr(MirExpr::Subscript {
                base: Box::new(MirExpr::Name {
                    name: "t".to_string(),
                    ty: tuple_int_bool_float(),
                }),
                index: Box::new(MirExpr::IntLiteral(1)),
            })),
            MirItem::TopLevelStmt(print_expr(MirExpr::Subscript {
                base: Box::new(MirExpr::Name {
                    name: "t".to_string(),
                    ty: tuple_int_bool_float(),
                }),
                index: Box::new(MirExpr::IntLiteral(2)),
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("tuple_literal_and_reads")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("tuple_literal_and_reads.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("tuple_literal_and_reads");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(pycc_scratch::lf(&output.stdout), b"1\nTrue\n2.5\n");
}

#[test]
fn a_function_local_tuple_codegens_and_runs_through_its_alloca_slot() {
    // The module-level test above exercises `declare_module_globals`'
    // new `Ty::Tuple(_)` arm; this exercises the *other* storage route,
    // `storage_slot_at_entry`'s alloca, which allocates the struct type
    // directly rather than a pointer. Both routes must work for
    // `x = (...)` to be usable where D-116 says it is.
    //
    // Also the one fixture here that reads a tuple element as a
    // function's return value, so `MirStmt::Return`'s path runs with a
    // real tuple-derived scalar.
    let mir = MirModule {
        items: vec![
            MirItem::Function {
                name: "second".to_string(),
                params: vec![],
                return_ty: Ty::Int,
                body: vec![
                    MirStmt::Assign {
                        target: "u".to_string(),
                        value: MirExpr::TupleLiteral(vec![
                            MirExpr::IntLiteral(10),
                            MirExpr::IntLiteral(20),
                            MirExpr::IntLiteral(30),
                        ]),
                    },
                    MirStmt::Return(Some(MirExpr::Subscript {
                        base: Box::new(MirExpr::Name {
                            name: "u".to_string(),
                            ty: Ty::Tuple(Box::new(vec![Ty::Int, Ty::Int, Ty::Int])),
                        }),
                        index: Box::new(MirExpr::IntLiteral(1)),
                    })),
                ],
            },
            MirItem::TopLevelStmt(print_expr(MirExpr::Call {
                callee: "second".to_string(),
                args: vec![],
                ty: Ty::Int,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir =
        pycc_scratch::ScratchDir::new("tuple_local_slot").expect("failed to create scratch dir");
    let obj_path = dir.join("tuple_local_slot.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("tuple_local_slot");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(pycc_scratch::lf(&output.stdout), b"20\n");
}

#[test]
fn an_inline_tuple_literal_can_be_subscripted_without_a_named_binding() {
    // `print((7, 8)[1])` -- the `Subscript` tuple branch reached with a
    // `MirExpr::TupleLiteral` base rather than a `MirExpr::Name` one,
    // so the struct value comes straight from `insertvalue` without
    // ever being stored to or loaded from a slot. Proves the branch
    // dispatches on the *value*, not on having a backing binding.
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(print_expr(MirExpr::Subscript {
            base: Box::new(MirExpr::TupleLiteral(vec![
                MirExpr::IntLiteral(7),
                MirExpr::IntLiteral(8),
            ])),
            index: Box::new(MirExpr::IntLiteral(1)),
        }))],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("tuple_inline_subscript")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("tuple_inline_subscript.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("tuple_inline_subscript");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(pycc_scratch::lf(&output.stdout), b"8\n");
}

#[test]
fn a_tuple_typed_module_binding_gets_a_struct_backed_global_slot() {
    // The tuple counterpart of `a_dict_typed_module_binding_gets_a_
    // real_pointer_backed_global_slot` above -- but asserting the
    // *opposite* storage shape, which is the whole point of D-115: a
    // tuple global holds the struct inline, not a nullable pointer to a
    // heap object. Its zero initializer is therefore a real zeroed
    // struct rather than a null sentinel, and the separate
    // `initialized` flag is what guards a read-before-assignment.
    let context = Context::create();
    let module = context.create_module("tuple_global");
    let bindings = BTreeMap::from([("t".to_string(), tuple_int_bool_float())]);
    let slots = declare_module_globals(&context, &module, &bindings);
    let slot = slots.get("t").expect("the tuple binding gets a slot");
    assert_eq!(slot.ty, tuple_int_bool_float());
    assert!(
        slot.initialized.is_some(),
        "a tuple global still gets the initialized flag -- a zeroed struct is \
             indistinguishable from a legitimately-zero tuple, so it cannot self-signal"
    );
    let global = module
        .get_global("pyglobal_t")
        .expect("the global is named pyglobal_<name>");
    let initializer = global
        .get_initializer()
        .expect("declare_module_globals always sets an initializer");
    assert!(
        initializer.is_struct_value(),
        "a tuple global is initialized with a struct value, not a null pointer"
    );
}

#[test]
fn compiles_a_function_with_a_tuple_parameter_and_tuple_return_value() {
    // The tuple counterpart of `compiles_a_function_with_a_set_int_
    // parameter_and_set_int_return_value` above, for the identical
    // reason. Since D-228 (issue #918) an annotated `tuple[...]`
    // *parameter* reaches codegen from real source, and since #925 a
    // `-> tuple[...]` return annotation does too; the combined shape
    // below is kept hand-built as the direct codegen-level pin,
    // independent of the front end. This MIR shape must still compile *cleanly*
    // -- `ty_to_basic_type`'s `Tuple(_)` arm, `emit_expr`'s `Name` arm's
    // `Ty::Tuple(_)` arm, and `MirStmt::Return`'s own `Scalar::Tuple`
    // pass-through must all agree on the same by-value struct
    // representation, or the `ret` would be typed against a different
    // aggregate than the signature declares.
    //
    // Deliberately does not link or run the resulting object, for the
    // identical reason the set counterpart doesn't.
    let mir = MirModule {
        items: vec![MirItem::Function {
            name: "f".to_string(),
            params: vec![("x".to_string(), tuple_int_bool_float())],
            return_ty: tuple_int_bool_float(),
            body: vec![MirStmt::Return(Some(MirExpr::Name {
                name: "x".to_string(),
                ty: tuple_int_bool_float(),
            }))],
        }],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("tuple_param_and_return")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("tuple_param_and_return.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
}

#[test]
fn passing_a_tuple_value_as_a_function_argument_marshals_it_by_value() {
    // The tuple counterpart of `passing_a_set_value_as_a_function_
    // argument_marshals_it_like_a_pointer` above: the caller adds the
    // one shape the test directly above does not reach --
    // `build_call_to`'s argument-marshalling match, whose
    // `Scalar::Tuple` arm is also in the pass-through bucket, differing
    // from its neighbours only in that it hands LLVM a `StructValue`
    // rather than a `PointerValue`. Same not-linked, not-run caveat as
    // the set counterpart.
    let mir = MirModule {
        items: vec![
            MirItem::Function {
                name: "f".to_string(),
                params: vec![("x".to_string(), tuple_int_bool_float())],
                return_ty: Ty::Int,
                body: vec![MirStmt::Return(Some(MirExpr::IntLiteral(0)))],
            },
            MirItem::Function {
                name: "g".to_string(),
                params: vec![("x".to_string(), tuple_int_bool_float())],
                return_ty: Ty::Int,
                body: vec![MirStmt::Return(Some(MirExpr::Call {
                    callee: "f".to_string(),
                    args: vec![MirExpr::Name {
                        name: "x".to_string(),
                        ty: tuple_int_bool_float(),
                    }],
                    ty: Ty::Int,
                }))],
            },
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("tuple_passed_as_argument")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("tuple_passed_as_argument.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
}

#[test]
fn a_struct_tuple_is_always_truthy() {
    // A struct tuple has a fixed, non-zero arity (the empty tuple is a
    // tuple object), so its truth value is the constant `1`, as
    // `bool((1, 2))` is in CPython. Calls `truthy` directly with a
    // hand-built `Scalar::Tuple`.
    let context = Context::create();
    let (module, rt) = list_scalar_panic_fixture(&context);
    let builder = context.create_builder();
    let function = module.add_function("probe", context.void_type().fn_type(&[], false), None);
    builder.position_at_end(context.append_basic_block(function, "entry"));
    let truth = truthy(&context, &builder, &module, &rt, tuple_scalar(&context));
    assert_eq!(truth.get_zero_extended_constant(), Some(1));
}

#[test]
#[should_panic(expected = "internal error: expected an int-or-bool operand, got tuple")]
fn to_numeric_encoded_int_rejects_a_tuple_operand() {
    // The `tuple[...]` counterpart of `to_numeric_encoded_int_rejects_a_set_
    // operand` above -- genuinely defensive, unlike the two tests
    // directly above: `pycc_types`' `numeric_result_type` has no
    // `as_numeric` mapping for `Ty::Tuple`, so no real MIR reaches this
    // arm.
    let context = Context::create();
    let builder = context.create_builder();
    to_numeric_encoded_int(&context, &builder, tuple_scalar(&context));
}

#[test]
#[should_panic(expected = "internal error: expected a numeric operand, got tuple")]
fn to_float_rejects_a_tuple_operand() {
    // Same defensive-arm rationale as `to_numeric_encoded_int_rejects_a_tuple_
    // operand` directly above, for `to_float`'s own match.
    let context = Context::create();
    let (_module, rt) = list_scalar_panic_fixture(&context);
    let builder = context.create_builder();
    to_float(&context, &builder, &rt, tuple_scalar(&context));
}

#[test]
#[should_panic(expected = "a computed index into a struct tuple of mixed element types")]
fn a_computed_index_into_a_mixed_tuple_is_an_internal_error() {
    // `pycc_types` admits a computed tuple index only when every element
    // has one `int`/`bool`/`float` type (T0040 otherwise), so this is
    // defensive -- reachable only from hand-built MIR.
    let mir = MirModule {
        items: vec![MirItem::Function {
            name: "f".to_string(),
            params: vec![
                ("t".to_string(), Ty::Tuple(Box::new(vec![Ty::Int, Ty::Float]))),
                ("i".to_string(), Ty::Int),
            ],
            return_ty: Ty::Int,
            body: vec![MirStmt::Return(Some(MirExpr::Subscript {
                base: Box::new(MirExpr::Name {
                    name: "t".to_string(),
                    ty: Ty::Tuple(Box::new(vec![Ty::Int, Ty::Float])),
                }),
                index: Box::new(MirExpr::Name {
                    name: "i".to_string(),
                    ty: Ty::Int,
                }),
            }))],
        }],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("tuple_non_literal_index")
        .expect("failed to create scratch dir");
    let _ = compile_to_object(&mir, &dir.join("tuple_non_literal_index.o"), None, false);
}

#[test]
fn a_tuple_operand_is_rejected_as_a_range_bound() {
    // `range()`'s own operand check folds `Tuple` into its existing
    // or-pattern rather than giving it a separate arm (that arm's
    // message never names the offending type, so folding costs no
    // honesty and adds no permanently-unexecutable region). Pinned via
    // a direct call, matching how the arm's other variants are reached.
    let context = Context::create();
    let builder = context.create_builder();
    let module = context.create_module("tuple_range_bound");
    let rt = declare_rt_functions(&context, &module);
    let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        range_operand_to_normalized_int(&context, &builder, &rt, tuple_scalar(&context), "start");
    }));
    let payload = panicked.expect_err("a tuple range bound must be rejected");
    let message = payload
        .downcast_ref::<String>()
        .expect("this crate's panics all carry a formatted String");
    assert!(
        message.contains("range() start did not evaluate to int"),
        "unexpected panic message: {message}"
    );
}
