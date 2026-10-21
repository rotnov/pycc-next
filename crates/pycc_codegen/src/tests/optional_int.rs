//! `Optional[int]` construction, narrowing, truthiness and `is None`.

use super::*;

/// A hand-built `Scalar::Optional` for the defensive-panic tests below,
/// mirroring `tuple_scalar`'s own `undef`-struct shape immediately above:
/// every function under test rejects the value before doing anything with
/// its contents, so an `undef` `{ i64, i8 }` struct is exactly as good as a
/// real one and needs no builder to construct.
fn optional_scalar(context: &Context) -> Scalar<'_> {
    Scalar::Optional(
        context
            .struct_type(
                &[context.i64_type().into(), context.i8_type().into()],
                false,
            )
            .get_undef(),
    )
}

#[test]
#[should_panic(expected = "internal error: expected an int-or-bool operand, got optional")]
fn to_numeric_encoded_int_rejects_an_optional_operand() {
    // `pycc_types`' `numeric_result_type` maps no `Ty::Optional` to a
    // numeric type (T0021 rejects arithmetic on `Optional[int]` before
    // codegen runs), so this is defensive, exactly like the sibling
    // `Tuple`/`Instance` tests immediately above.
    let context = Context::create();
    let (_module, _rt) = list_scalar_panic_fixture(&context);
    let builder = context.create_builder();
    to_numeric_encoded_int(&context, &builder, optional_scalar(&context));
}

#[test]
#[should_panic(expected = "internal error: expected a numeric operand, got optional")]
fn to_float_rejects_an_optional_operand() {
    // Same defensive-arm rationale as the test directly above, for
    // `to_float`'s own match.
    let context = Context::create();
    let (_module, rt) = list_scalar_panic_fixture(&context);
    let builder = context.create_builder();
    to_float(&context, &builder, &rt, optional_scalar(&context));
}

#[test]
#[should_panic(expected = "range() start did not evaluate to int")]
fn an_optional_operand_is_rejected_as_a_range_bound() {
    // `range()`'s own operand check folds `Optional` into its existing
    // or-pattern, mirroring `a_tuple_operand_is_rejected_as_a_range_bound`
    // immediately above: `range()` arguments are type-checked as plain
    // numeric types before codegen, and an `Optional[int]` is never one.
    let context = Context::create();
    let builder = context.create_builder();
    let module = context.create_module("optional_range_bound");
    let rt = declare_rt_functions(&context, &module);
    range_operand_to_normalized_int(&context, &builder, &rt, optional_scalar(&context), "start");
}

#[test]
#[should_panic(
    expected = "internal error: cannot store this value into an instance attribute slot"
)]
fn scalar_to_slot_word_rejects_an_optional_scalar() {
    // `Optional[int]` joins `scalar_to_slot_word`'s existing defensive
    // or-pattern: this raw-`i64`-word slot encoding has no room for the
    // struct's extra present/absent byte, and this PR ships no
    // class-attribute use of `Optional[int]`.
    let context = Context::create();
    let builder = context.create_builder();
    scalar_to_slot_word(&context, &builder, optional_scalar(&context));
}

#[test]
fn optional_int_annotated_assignment_constructs_a_present_struct_and_reads_its_payload() {
    // `x: int | None = 5` followed by `print(x is None)` and
    // `print(x is not None)` -- end to end through `compile_to_object`,
    // exercising `MirExpr::OptionalWrap`'s construction (via
    // `coerce_scalar_to_type`'s bare-payload-widening arm) and the
    // `Compare` arm's `Is`/`IsNot` codegen reading the present flag back
    // out of a real (not placeholder) struct. Expected output verified
    // against CPython on the equivalent source.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "x".to_string(),
                value: MirExpr::OptionalWrap(Box::new(MirExpr::IntLiteral(5)), Box::new(Ty::Int)),
            }),
            MirItem::TopLevelStmt(print_expr(MirExpr::Compare {
                op: pycc_mir::CmpOpKind::Is,
                left: Box::new(MirExpr::Name {
                    name: "x".to_string(),
                    ty: optional_int(),
                }),
                right: Box::new(MirExpr::NoneLiteral),
                ty: Ty::Bool,
            })),
            MirItem::TopLevelStmt(print_expr(MirExpr::Compare {
                op: pycc_mir::CmpOpKind::IsNot,
                left: Box::new(MirExpr::Name {
                    name: "x".to_string(),
                    ty: optional_int(),
                }),
                right: Box::new(MirExpr::NoneLiteral),
                ty: Ty::Bool,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("optional_int_present_is_none")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("optional_int_present_is_none.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("optional_int_present_is_none");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(pycc_scratch::lf(&output.stdout), b"False\nTrue\n");
}

#[test]
fn optional_int_annotated_assignment_with_bare_none_constructs_an_absent_struct() {
    // `x: int | None = None` followed by the same `is`/`is not` checks --
    // the mirror-image case of the test above, exercising
    // `coerce_scalar_to_type`'s *other* `Scalar::Optional` arm (the
    // placeholder-to-real-struct-shape branch) instead of the bare-payload
    // one.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "x".to_string(),
                value: MirExpr::OptionalWrap(Box::new(MirExpr::NoneLiteral), Box::new(Ty::Int)),
            }),
            MirItem::TopLevelStmt(print_expr(MirExpr::Compare {
                op: pycc_mir::CmpOpKind::Is,
                left: Box::new(MirExpr::Name {
                    name: "x".to_string(),
                    ty: optional_int(),
                }),
                right: Box::new(MirExpr::NoneLiteral),
                ty: Ty::Bool,
            })),
            MirItem::TopLevelStmt(print_expr(MirExpr::Compare {
                op: pycc_mir::CmpOpKind::IsNot,
                left: Box::new(MirExpr::Name {
                    name: "x".to_string(),
                    ty: optional_int(),
                }),
                right: Box::new(MirExpr::NoneLiteral),
                ty: Ty::Bool,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("optional_int_absent_is_none")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("optional_int_absent_is_none.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("optional_int_absent_is_none");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(pycc_scratch::lf(&output.stdout), b"True\nFalse\n");
}

#[test]
fn optional_int_reassignment_from_present_to_absent_updates_the_is_none_reading() {
    // `x: int | None = 5` then a later plain `x = None` -- the case
    // `coerce_scalar_to_type`'s own doc comment calls out specifically:
    // `pycc_mir::stmt::lower_stmt`'s `Assign` arm never wraps a later
    // reassignment in `OptionalWrap`, so this MIR intentionally assigns a
    // bare `MirExpr::NoneLiteral` directly to the already-`Optional[int]`
    // slot, exercising `coerce_scalar_to_type` being invoked again at
    // `emit_assign` time (not just at the first `OptionalWrap` site).
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "x".to_string(),
                value: MirExpr::OptionalWrap(Box::new(MirExpr::IntLiteral(5)), Box::new(Ty::Int)),
            }),
            MirItem::TopLevelStmt(print_expr(MirExpr::Compare {
                op: pycc_mir::CmpOpKind::Is,
                left: Box::new(MirExpr::Name {
                    name: "x".to_string(),
                    ty: optional_int(),
                }),
                right: Box::new(MirExpr::NoneLiteral),
                ty: Ty::Bool,
            })),
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "x".to_string(),
                value: MirExpr::NoneLiteral,
            }),
            MirItem::TopLevelStmt(print_expr(MirExpr::Compare {
                op: pycc_mir::CmpOpKind::Is,
                left: Box::new(MirExpr::Name {
                    name: "x".to_string(),
                    ty: optional_int(),
                }),
                right: Box::new(MirExpr::NoneLiteral),
                ty: Ty::Bool,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("optional_int_reassign_to_none")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("optional_int_reassign_to_none.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("optional_int_reassign_to_none");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(pycc_scratch::lf(&output.stdout), b"False\nTrue\n");
}

#[test]
fn optional_int_truthiness_follows_cpython_for_present_and_absent_values() {
    // `if x:` for three `Optional[int]` values -- absent (`None`), present
    // with a falsy payload (`0`), and present with a truthy payload (`5`)
    // -- exercising `truthy`'s own branch-free `Scalar::Optional` arm
    // (present AND payload-truthy) end to end. Matches CPython's
    // `bool(x)` for `x: int | None` exactly: `False` only for `None` or a
    // present `0`.
    fn if_prints_one_else_zero(name: &str) -> MirItem {
        MirItem::TopLevelStmt(MirStmt::If {
            test: MirExpr::Name {
                name: name.to_string(),
                ty: optional_int(),
            },
            body: vec![print_expr(MirExpr::IntLiteral(1))],
            orelse: vec![print_expr(MirExpr::IntLiteral(0))],
        })
    }
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "a".to_string(),
                value: MirExpr::OptionalWrap(Box::new(MirExpr::NoneLiteral), Box::new(Ty::Int)),
            }),
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "b".to_string(),
                value: MirExpr::OptionalWrap(Box::new(MirExpr::IntLiteral(0)), Box::new(Ty::Int)),
            }),
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "c".to_string(),
                value: MirExpr::OptionalWrap(Box::new(MirExpr::IntLiteral(5)), Box::new(Ty::Int)),
            }),
            if_prints_one_else_zero("a"),
            if_prints_one_else_zero("b"),
            if_prints_one_else_zero("c"),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("optional_int_truthiness")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("optional_int_truthiness.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("optional_int_truthiness");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(pycc_scratch::lf(&output.stdout), b"0\n0\n1\n");
}

/// Issue #769 (Part 2 of #747): `x: int | None = 5; if x is not None:
/// print(x)` end to end through `compile_to_object`, exercising
/// `MirExpr::OptionalUnwrap`'s codegen arm (the `Scalar::Optional` ->
/// `Scalar::Int` field-extraction path) for a present, smallint-valued
/// payload. Expected output verified against CPython on the equivalent
/// source.
#[test]
fn optional_int_narrowed_read_of_a_present_smallint_prints_the_payload() {
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "x".to_string(),
                value: MirExpr::OptionalWrap(Box::new(MirExpr::IntLiteral(5)), Box::new(Ty::Int)),
            }),
            MirItem::TopLevelStmt(MirStmt::If {
                test: MirExpr::Compare {
                    op: pycc_mir::CmpOpKind::IsNot,
                    left: Box::new(MirExpr::Name {
                        name: "x".to_string(),
                        ty: optional_int(),
                    }),
                    right: Box::new(MirExpr::NoneLiteral),
                    ty: Ty::Bool,
                },
                body: vec![print_expr(MirExpr::OptionalUnwrap(
                    Box::new(MirExpr::Name {
                        name: "x".to_string(),
                        ty: optional_int(),
                    }),
                    Box::new(Ty::Int),
                ))],
                orelse: vec![],
            }),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("optional_int_narrowed_smallint")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("optional_int_narrowed_smallint.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("optional_int_narrowed_smallint");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(pycc_scratch::lf(&output.stdout), b"5\n");
}

/// Issue #769 (Part 2 of #747), the mandatory bigint case: `x: int | None
/// = <a value that does not fit D-061's tagged smallint range>; if x is
/// not None: print(x)`. `i64::MAX` (per `fits_tagged_smallint`'s own
/// `(n << 1) | 1` round-trip check) always falls outside that range and is
/// therefore materialized as a genuine heap `BigIntObj` by
/// `emit_int_constant`, not merely a large-looking smallint -- so this
/// exercises `OptionalUnwrap`'s codegen arm on a real heap payload, not
/// just its `Scalar::Int` shape.
#[test]
fn optional_int_narrowed_read_of_a_present_bigint_prints_the_payload() {
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "x".to_string(),
                value: MirExpr::OptionalWrap(
                    Box::new(MirExpr::IntLiteral(i64::MAX)),
                    Box::new(Ty::Int),
                ),
            }),
            MirItem::TopLevelStmt(MirStmt::If {
                test: MirExpr::Compare {
                    op: pycc_mir::CmpOpKind::IsNot,
                    left: Box::new(MirExpr::Name {
                        name: "x".to_string(),
                        ty: optional_int(),
                    }),
                    right: Box::new(MirExpr::NoneLiteral),
                    ty: Ty::Bool,
                },
                body: vec![print_expr(MirExpr::OptionalUnwrap(
                    Box::new(MirExpr::Name {
                        name: "x".to_string(),
                        ty: optional_int(),
                    }),
                    Box::new(Ty::Int),
                ))],
                orelse: vec![],
            }),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("optional_int_narrowed_bigint")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("optional_int_narrowed_bigint.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("optional_int_narrowed_bigint");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(pycc_scratch::lf(&output.stdout), b"9223372036854775807\n");
}

/// Issue #769 (Part 2 of #747), the retain-in-practice half of the mandatory
/// bigint case: proves `OptionalUnwrap`'s retain (added to
/// `retain_if_int_duplicate`'s own inline classification, `bigint_rc.rs`)
/// is not merely emitted but load-bearing at runtime. `x: int | None =
/// <heap bigint>` is narrowed and duplicated into `y` inside the `if`, `x`
/// is then reassigned to `None` (retiring `x`'s own reference via
/// `release_optional_int_slot_before_store`), and `y` is printed
/// afterward. Without the retain this test's own fix adds, `x`'s
/// reassignment would free the bigint out from under `y` while `y` is
/// still live, and this test would either crash or print a corrupted
/// value instead of the correct one.
#[test]
fn a_narrowed_bigint_duplicated_into_a_second_binding_survives_the_original_slots_death() {
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "x".to_string(),
                value: MirExpr::OptionalWrap(
                    Box::new(MirExpr::IntLiteral(i64::MAX)),
                    Box::new(Ty::Int),
                ),
            }),
            MirItem::TopLevelStmt(MirStmt::If {
                test: MirExpr::Compare {
                    op: pycc_mir::CmpOpKind::IsNot,
                    left: Box::new(MirExpr::Name {
                        name: "x".to_string(),
                        ty: optional_int(),
                    }),
                    right: Box::new(MirExpr::NoneLiteral),
                    ty: Ty::Bool,
                },
                body: vec![
                    MirStmt::Assign {
                        target: "y".to_string(),
                        value: MirExpr::OptionalUnwrap(
                            Box::new(MirExpr::Name {
                                name: "x".to_string(),
                                ty: optional_int(),
                            }),
                            Box::new(Ty::Int),
                        ),
                    },
                    MirStmt::Assign {
                        target: "x".to_string(),
                        value: MirExpr::OptionalWrap(
                            Box::new(MirExpr::NoneLiteral),
                            Box::new(Ty::Int),
                        ),
                    },
                    print_expr(MirExpr::Name {
                        name: "y".to_string(),
                        ty: Ty::Int,
                    }),
                ],
                orelse: vec![],
            }),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("optional_int_narrowed_bigint_survives_reassign")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("optional_int_narrowed_bigint_survives_reassign.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("optional_int_narrowed_bigint_survives_reassign");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(pycc_scratch::lf(&output.stdout), b"9223372036854775807\n");
}

/// Issue #769 (Part 2 of #747): `emit_expr`'s `OptionalUnwrap` arm's own
/// defensive `panic!` fires when its operand does not evaluate to
/// `Scalar::Optional` -- structurally impossible from real `pycc_mir`
/// lowering (only `expr::lower_expr`'s `HirExpr::Name` arm ever constructs
/// this node, always wrapping a `Ty::Optional`-scoped `Name`), so this test
/// constructs the malformed shape directly to reach and cover that arm,
/// mirroring this file's existing `#[should_panic]` conventions for other
/// "internal error" defensive arms.
#[test]
#[should_panic(expected = "OptionalUnwrap's operand did not evaluate to Scalar::Optional")]
fn optional_unwrap_on_a_non_optional_operand_panics_defensively_in_codegen() {
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(print_expr(MirExpr::OptionalUnwrap(
            Box::new(MirExpr::IntLiteral(5)),
            Box::new(Ty::Int),
        )))],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("optional_unwrap_non_optional_operand")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("optional_unwrap_non_optional_operand.o");
    let _ = compile_to_object(&mir, &obj_path, None, false);
}

#[test]
fn an_optional_int_parameter_and_return_value_round_trip_through_a_function_call() {
    // `def g(x: int | None) -> int | None: return x` called as `g(5)`, with
    // the result compared `is None`/`is not None` -- exercises argument
    // marshalling's `Scalar::Optional` pass-through arm and `MirExpr::
    // Call`'s `Ty::Optional` result-extraction arm together, the second of
    // which this PR's own review found missing (see the fix that added it
    // to the `Call` result match).
    let mir = MirModule {
        items: vec![
            MirItem::Function {
                name: "g".to_string(),
                params: vec![("x".to_string(), optional_int())],
                return_ty: optional_int(),
                body: vec![MirStmt::Return(Some(MirExpr::Name {
                    name: "x".to_string(),
                    ty: optional_int(),
                }))],
            },
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "y".to_string(),
                value: MirExpr::Call {
                    callee: "g".to_string(),
                    args: vec![MirExpr::OptionalWrap(
                        Box::new(MirExpr::IntLiteral(5)),
                        Box::new(Ty::Int),
                    )],
                    ty: optional_int(),
                },
            }),
            MirItem::TopLevelStmt(print_expr(MirExpr::Compare {
                op: pycc_mir::CmpOpKind::IsNot,
                left: Box::new(MirExpr::Name {
                    name: "y".to_string(),
                    ty: optional_int(),
                }),
                right: Box::new(MirExpr::NoneLiteral),
                ty: Ty::Bool,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("optional_int_call_roundtrip")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("optional_int_call_roundtrip.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("optional_int_call_roundtrip");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(pycc_scratch::lf(&output.stdout), b"True\n");
}

#[test]
fn an_optional_int_function_that_raises_before_returning_still_produces_a_valid_default() {
    // The exact scenario this PR's `default_value_for_type` fix targets:
    // a function declared to return `Optional[int]` raises mid-body
    // instead of returning normally. The exceptional-exit path still
    // builds *some* `Optional[int]` value to satisfy the LLVM `ret`
    // instruction, and that value's payload field must itself be a valid
    // D-141-encoded int (not a raw zero word) -- otherwise a caller
    // performing an `is None`/`is not None` check on the (never truly
    // observed, but still materialized) result would trip
    // `classify_encoded_int`'s fail-closed panic. `raise` propagates past
    // `main`'s own `try`/`except`, so a caught `ValueError` proves the
    // function's exceptional exit ran to completion without an internal
    // panic.
    let mir = MirModule {
        items: vec![
            MirItem::Function {
                name: "g".to_string(),
                params: vec![("x".to_string(), Ty::Int)],
                return_ty: optional_int(),
                body: vec![MirStmt::Raise {
                    exception: MirExceptionValue::Constructed {
                        type_tag: 1, // ValueError
                        class_name: "ValueError".to_string(),
                        message: MirExpr::StringLiteral("boom".to_string()),
                    },
                    frame_function: "test_fn".to_string(),
                }],
            },
            MirItem::TopLevelStmt(MirStmt::Try {
                body: vec![MirStmt::Assign {
                    target: "z".to_string(),
                    value: MirExpr::Call {
                        callee: "g".to_string(),
                        args: vec![MirExpr::IntLiteral(1)],
                        ty: optional_int(),
                    },
                }],
                handlers: vec![MirExceptHandler {
                    exc_type_tag: Some(vec![1]),
                    binding_name: None,
                    binding_ty: None,
                    body: vec![print_expr(MirExpr::IntLiteral(2))],
                }],
                orelse: Vec::new(),
                finalbody: Vec::new(),
            }),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("optional_int_exceptional_exit")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("optional_int_exceptional_exit.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("optional_int_exceptional_exit");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(pycc_scratch::lf(&output.stdout), b"2\n");
}

#[test]
#[should_panic(
    expected = "internal error: an `is`/`is not` operand's non-`None` side must be `Optional[_]`"
)]
fn is_none_on_a_non_optional_non_none_operand_panics_defensively_in_codegen() {
    // `pycc_types::check`'s own `T0021` gate already rejects `x is None`
    // for a plain (non-`Optional`) `x` before codegen ever runs -- this
    // pins the codegen-level defensive arm directly, via hand-built MIR
    // that skips the type checker entirely, exactly like
    // `a_tuple_operand_is_rejected_as_a_range_bound` above pins its own
    // codegen-level defensive arm.
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(print_expr(MirExpr::Compare {
            op: pycc_mir::CmpOpKind::Is,
            left: Box::new(MirExpr::IntLiteral(5)),
            right: Box::new(MirExpr::NoneLiteral),
            ty: Ty::Bool,
        }))],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("optional_is_none_non_optional_operand")
        .expect("failed to create scratch dir");
    let _ = compile_to_object(
        &mir,
        &dir.join("optional_is_none_non_optional_operand.o"),
        None,
        false,
    );
}

#[test]
fn is_none_on_a_ty_none_typed_non_optional_operand_reads_the_ty_none_arm() {
    // `f() is None`/`f() is not None` where `f` is declared `-> None`
    // (D-197, #763, Part 1 of #747): `pycc_types::check`'s own `T0021`
    // gate accepts `Ty::None` as well as `Ty::Optional(_)` on the
    // non-`None` side of `is`/`is not` (see this crate's own `Compare`
    // arm doc comment), and a `None`-returning call's own codegen
    // (`MirExpr::Call`'s `Ty::None` arm) materializes `Scalar::Bool(0)`,
    // never a `Scalar::Optional` -- so this is the one legitimate,
    // reachable-from-real-source path into the `present` match's
    // `_ if other_ty == Ty::None` arm, distinct from every other `is`/
    // `is not` test above, which all reach the `Scalar::Optional(v)` arm
    // instead (including `None is None`, since `MirExpr::NoneLiteral`
    // itself always emits a placeholder `Scalar::Optional`, never
    // `Scalar::Bool`). Always `True`/`False` respectively, matching
    // CPython's `f() is None` for any `None`-returning `f`.
    let mir = MirModule {
        items: vec![
            MirItem::Function {
                name: "f".to_string(),
                params: vec![],
                return_ty: Ty::None,
                body: vec![MirStmt::Return(None)],
            },
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "x".to_string(),
                value: MirExpr::Call {
                    callee: "f".to_string(),
                    args: vec![],
                    ty: Ty::None,
                },
            }),
            MirItem::TopLevelStmt(print_expr(MirExpr::Compare {
                op: pycc_mir::CmpOpKind::Is,
                left: Box::new(MirExpr::Name {
                    name: "x".to_string(),
                    ty: Ty::None,
                }),
                right: Box::new(MirExpr::NoneLiteral),
                ty: Ty::Bool,
            })),
            MirItem::TopLevelStmt(print_expr(MirExpr::Compare {
                op: pycc_mir::CmpOpKind::IsNot,
                left: Box::new(MirExpr::Name {
                    name: "x".to_string(),
                    ty: Ty::None,
                }),
                right: Box::new(MirExpr::NoneLiteral),
                ty: Ty::Bool,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("optional_is_none_ty_none_operand")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("optional_is_none_ty_none_operand.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("optional_is_none_ty_none_operand");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(pycc_scratch::lf(&output.stdout), b"True\nFalse\n");
}

#[test]
fn optional_int_annotated_assignment_inside_a_function_body_uses_the_alloca_storage_route() {
    // Every other module-scope `x: int | None` test above exercises
    // `declare_module_globals`'s own `Ty::Optional` arm (D-197, #763,
    // Part 1 of #747) -- the bug this PR's own `declare_module_globals`
    // fix targeted. A function-local `Optional[int]` binding takes an
    // entirely separate storage route, `storage_slot_at_entry`'s alloca
    // path, which this test exercises directly: `def g(): x: int | None
    // = 5; print(x is None); print(x is not None)`, called once from
    // module scope.
    let mir = MirModule {
        items: vec![
            MirItem::Function {
                name: "g".to_string(),
                params: vec![],
                return_ty: Ty::None,
                body: vec![
                    MirStmt::Assign {
                        target: "x".to_string(),
                        value: MirExpr::OptionalWrap(
                            Box::new(MirExpr::IntLiteral(5)),
                            Box::new(Ty::Int),
                        ),
                    },
                    print_expr(MirExpr::Compare {
                        op: pycc_mir::CmpOpKind::Is,
                        left: Box::new(MirExpr::Name {
                            name: "x".to_string(),
                            ty: optional_int(),
                        }),
                        right: Box::new(MirExpr::NoneLiteral),
                        ty: Ty::Bool,
                    }),
                    print_expr(MirExpr::Compare {
                        op: pycc_mir::CmpOpKind::IsNot,
                        left: Box::new(MirExpr::Name {
                            name: "x".to_string(),
                            ty: optional_int(),
                        }),
                        right: Box::new(MirExpr::NoneLiteral),
                        ty: Ty::Bool,
                    }),
                    MirStmt::Return(None),
                ],
            },
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
                callee: "g".to_string(),
                args: vec![],
                ty: Ty::None,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("optional_int_function_local_alloca")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("optional_int_function_local_alloca.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("optional_int_function_local_alloca");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(pycc_scratch::lf(&output.stdout), b"False\nTrue\n");
}

#[test]
fn none_is_operand_reads_the_right_hand_side_when_none_is_written_on_the_left() {
    // Every other `is`/`is not` test above writes `x is None`/`x is not
    // None`, so `left` is always the non-`None` operand and the `if
    // matches!(left.as_ref(), MirExpr::NoneLiteral)` branch always takes
    // its `else` arm (D-197, #763, Part 1 of #747). CPython also accepts
    // the reverse order, `None is x`/`None is not x`, and this crate's
    // own `Compare` codegen handles it identically -- this test exercises
    // that `then` arm (`(r, right_ty)`) directly, the one combination the
    // other tests never reach.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "x".to_string(),
                value: MirExpr::OptionalWrap(Box::new(MirExpr::IntLiteral(5)), Box::new(Ty::Int)),
            }),
            MirItem::TopLevelStmt(print_expr(MirExpr::Compare {
                op: pycc_mir::CmpOpKind::Is,
                left: Box::new(MirExpr::NoneLiteral),
                right: Box::new(MirExpr::Name {
                    name: "x".to_string(),
                    ty: optional_int(),
                }),
                ty: Ty::Bool,
            })),
            MirItem::TopLevelStmt(print_expr(MirExpr::Compare {
                op: pycc_mir::CmpOpKind::IsNot,
                left: Box::new(MirExpr::NoneLiteral),
                right: Box::new(MirExpr::Name {
                    name: "x".to_string(),
                    ty: optional_int(),
                }),
                ty: Ty::Bool,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("optional_none_is_x_ordering")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("optional_none_is_x_ordering.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("optional_none_is_x_ordering");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(pycc_scratch::lf(&output.stdout), b"False\nTrue\n");
}
