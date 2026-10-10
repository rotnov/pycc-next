//! Float and str binops/comparisons and their internal-consistency checks.

use super::*;

#[test]
fn compiles_true_division_of_two_ints_as_float_arithmetic() {
    // `x = 7 / 2` -- must promote both operands to float and use
    // `fdiv`, not integer division (`pycc_types` already types this
    // `Ty::Float`; this proves codegen honors that, not `int`'s own
    // `//`).
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::Assign {
            target: "x".to_string(),
            value: MirExpr::BinOp {
                op: pycc_mir::BinOpKind::Div,
                left: Box::new(MirExpr::IntLiteral(7)),
                right: Box::new(MirExpr::IntLiteral(2)),
                ty: pycc_mir::Ty::Float,
            },
        })],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("true_div").expect("failed to create scratch dir");
    let obj_path = dir.join("true_div.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
}

#[test]
fn compiles_mixed_int_and_float_addition() {
    // `y = 1 + 1.5` -- promotes the `int` operand to `float`.
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::Assign {
            target: "y".to_string(),
            value: MirExpr::BinOp {
                op: BinOpKind::Add,
                left: Box::new(MirExpr::IntLiteral(1)),
                right: Box::new(MirExpr::FloatLiteral(1.5)),
                ty: pycc_mir::Ty::Float,
            },
        })],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("mixed_add").expect("failed to create scratch dir");
    let obj_path = dir.join("mixed_add.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
}

#[test]
fn compiles_bool_arithmetic_promoted_to_int() {
    // `z = True + True` -- Python's `bool` is an `int` subtype; the
    // result is `2` (`int`), not a `bool`.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "z".to_string(),
                value: MirExpr::BinOp {
                    op: BinOpKind::Add,
                    left: Box::new(MirExpr::BoolLiteral(true)),
                    right: Box::new(MirExpr::BoolLiteral(true)),
                    ty: pycc_mir::Ty::Int,
                },
            }),
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
                callee: "print".to_string(),
                args: vec![MirExpr::Name {
                    name: "z".to_string(),
                    ty: pycc_mir::Ty::Int,
                }],
                ty: pycc_mir::Ty::None,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("bool_arith").expect("failed to create scratch dir");
    let obj_path = dir.join("bool_arith.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("bool_arith");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"2\n");
}

#[test]
fn compiles_a_float_comparison() {
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::Assign {
            target: "b".to_string(),
            value: MirExpr::Compare {
                op: pycc_mir::CmpOpKind::Lt,
                left: Box::new(MirExpr::FloatLiteral(1.5)),
                right: Box::new(MirExpr::FloatLiteral(2.5)),
                ty: pycc_mir::Ty::Bool,
            },
        })],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("float_cmp").expect("failed to create scratch dir");
    let obj_path = dir.join("float_cmp.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
}

#[test]
fn compiles_an_if_test_on_a_float_expression() {
    // `if 0.0: print(1)` -- must print nothing (`0.0` is falsy).
    // `if 1.5: print(1)` -- must print `1`.
    for (test, expected) in [(0.0, ""), (1.5, "1\n")] {
        let mir = MirModule {
            items: vec![MirItem::TopLevelStmt(MirStmt::If {
                test: MirExpr::FloatLiteral(test),
                body: vec![MirStmt::ExprStmt(MirExpr::Call {
                    callee: "print".to_string(),
                    args: vec![MirExpr::IntLiteral(1)],
                    ty: pycc_mir::Ty::None,
                })],
                orelse: vec![],
            })],
            class_defs: Vec::new(),
        };
        let dir = pycc_scratch::ScratchDir::new(&format!("float_truthy_{test}"))
            .expect("failed to create scratch dir");
        let obj_path = dir.join("float_truthy.o");
        compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
        let bin_path = dir.join("float_truthy");
        link_object_with_runtime(&obj_path, &bin_path);
        let output = Command::new(&bin_path).output().expect("binary should run");
        assert_eq!(output.stdout, expected.as_bytes(), "test value {test}");
    }
}

#[test]
#[should_panic(expected = "expected an int-or-bool operand, got float")]
fn an_int_result_binop_with_a_float_operand_hits_to_numeric_encoded_int_defensive_panic() {
    // Deliberately malformed MIR: `pycc_types::numeric_result_type`
    // always promotes an expression with any `float` operand to
    // `Ty::Float` (`5 + 1.0` types as `float`, never `int`), so no real
    // pipeline could ever produce a `BinOp { ty: Ty::Int, .. }` with a
    // `float` operand. Exercises `to_numeric_encoded_int`'s own defensive
    // `Scalar::Float` arm -- same "hand-construct the otherwise
    // unreachable shape" convention as
    // `printing_a_mistyped_compare_expression_hits_the_internal_consistency_check`
    // and `true_division_binop_codegen_panics_via_its_dedicated_arm`.
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::Assign {
            target: "x".to_string(),
            value: MirExpr::BinOp {
                op: BinOpKind::Add,
                left: Box::new(MirExpr::FloatLiteral(1.5)),
                right: Box::new(MirExpr::IntLiteral(1)),
                ty: Ty::Int,
            },
        })],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("binop_int_result_float_operand_panics")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("binop_int_result_float_operand_panics.o");
    let _ = compile_to_object(&mir, &obj_path, None, false);
}

#[test]
#[should_panic(expected = "a `None`-result BinOp is not supported yet")]
fn a_none_result_binop_is_not_yet_supported() {
    // No real Python operator returns `None` from a `BinOp`, so
    // `pycc_types`/`pycc_mir` never produce this shape -- hand-crafted
    // MIR exercises the `BinOp` arm's own defensive catch-all directly
    // instead, using `int` operands under a mislabeled `ty`, same
    // "hand-construct the otherwise-unreachable shape" convention as
    // `true_division_binop_codegen_panics_via_its_dedicated_arm` above.
    // This test's earlier (Task 3-era) incarnation used `Ty::Str` here
    // (real string concatenation was Task 7's job then); Task 7 now
    // implements `Ty::Str` for real, so `Ty::None` is the placeholder
    // that keeps this catch-all covered.
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::Assign {
            target: "x".to_string(),
            value: MirExpr::BinOp {
                op: BinOpKind::Add,
                left: Box::new(MirExpr::IntLiteral(1)),
                right: Box::new(MirExpr::IntLiteral(2)),
                ty: Ty::None,
            },
        })],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("binop_none_result_panics")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("binop_none_result_panics.o");
    let _ = compile_to_object(&mir, &obj_path, None, false);
}

#[test]
#[should_panic(expected = "internal error: str BinOp operand did not evaluate to str")]
fn a_str_result_binop_with_a_non_str_left_operand_hits_the_internal_consistency_check() {
    // Deliberately malformed MIR: `pycc_types`/`pycc_mir` produce a
    // `Ty::Str`-typed `BinOp` only for `str + str` (see `pycc_mir`'s own
    // `adding_two_strings_infers_str` test) and for `Mul` string
    // repetition (#574), and the repetition shape is caught by this
    // arm's own named D-072 boundary above before either destructure
    // runs -- so no real pipeline could reach this `Add` shape with a
    // non-`str` left operand.
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::Assign {
            target: "x".to_string(),
            value: MirExpr::BinOp {
                op: BinOpKind::Add,
                left: Box::new(MirExpr::IntLiteral(1)),
                right: Box::new(MirExpr::StringLiteral("b".to_string())),
                ty: Ty::Str,
            },
        })],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("str_binop_left_mismatch_panics")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("str_binop_left_mismatch_panics.o");
    let _ = compile_to_object(&mir, &obj_path, None, false);
}

#[test]
#[should_panic(expected = "internal error: str BinOp operand did not evaluate to str")]
fn a_str_result_binop_with_a_non_str_right_operand_hits_the_internal_consistency_check() {
    // Same rationale as the left-operand version above, isolating the
    // `BinOp` arm's *second* `let Scalar::Str(r) = r else { .. }` check
    // -- the left operand must genuinely be `str` (so the first check
    // passes) for this one to be reached at all.
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::Assign {
            target: "x".to_string(),
            value: MirExpr::BinOp {
                op: BinOpKind::Add,
                left: Box::new(MirExpr::StringLiteral("a".to_string())),
                right: Box::new(MirExpr::IntLiteral(2)),
                ty: Ty::Str,
            },
        })],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("str_binop_right_mismatch_panics")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("str_binop_right_mismatch_panics.o");
    let _ = compile_to_object(&mir, &obj_path, None, false);
}

#[test]
#[should_panic(expected = "`str Sub str` is not supported yet (only concatenation is)")]
fn a_str_binop_other_than_concatenation_is_not_yet_supported() {
    // `"a" - "b"` -- real `str` operands on both sides, but only `Add`
    // (concatenation) is implemented; Python doesn't define `str - str`
    // either, so `pycc_types` would reject this long before codegen --
    // this exercises the `Str` arm's own `op != Add` guard directly.
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::Assign {
            target: "x".to_string(),
            value: MirExpr::BinOp {
                op: BinOpKind::Sub,
                left: Box::new(MirExpr::StringLiteral("a".to_string())),
                right: Box::new(MirExpr::StringLiteral("b".to_string())),
                ty: Ty::Str,
            },
        })],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("str_binop_unsupported_op_panics")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("str_binop_unsupported_op_panics.o");
    let _ = compile_to_object(&mir, &obj_path, None, false);
}

#[test]
fn compiles_string_repetition_in_both_operand_orders() {
    // `x = "a" * 3` and `y = 3 * "a"` -- #575 (Part 2 of #123) replaces
    // Part 1's named D-072 boundary with real emission, so this MIR
    // (exactly what a real `pycc build` produces) now compiles. Both
    // operand orders go through the same `match (l, r)` in the `Ty::Str`
    // arm, so both are exercised here rather than only the `str`-first
    // one.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "x".to_string(),
                value: MirExpr::BinOp {
                    op: BinOpKind::Mul,
                    left: Box::new(MirExpr::StringLiteral("a".to_string())),
                    right: Box::new(MirExpr::IntLiteral(3)),
                    ty: Ty::Str,
                },
            }),
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "y".to_string(),
                value: MirExpr::BinOp {
                    op: BinOpKind::Mul,
                    left: Box::new(MirExpr::IntLiteral(3)),
                    right: Box::new(MirExpr::StringLiteral("a".to_string())),
                    ty: Ty::Str,
                },
            }),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("str_binop_repetition")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("str_binop_repetition.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
}

#[test]
fn compiles_string_repetition_with_a_bool_count() {
    // `x = "a" * True` -- `bool` is an `int` subtype, so `pycc_types`
    // types this `str` too. Exercises `to_numeric_encoded_int`'s own
    // `Scalar::Bool` arm from the repetition site, which the `int`-count
    // test above cannot reach.
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::Assign {
            target: "x".to_string(),
            value: MirExpr::BinOp {
                op: BinOpKind::Mul,
                left: Box::new(MirExpr::StringLiteral("a".to_string())),
                right: Box::new(MirExpr::BoolLiteral(true)),
                ty: Ty::Str,
            },
        })],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("str_binop_repetition_bool")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("str_binop_repetition_bool.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
}

#[test]
#[should_panic(expected = "pycc_codegen: internal error: a str-result `*` had no str operand")]
fn a_str_result_multiplication_without_a_str_operand_is_an_internal_error() {
    // `2 * 3` claiming a `str` result -- unreachable from any
    // type-checked program (`pycc_types` types `int * int` as `int`), so
    // this is a genuine internal-error arm, reached only through
    // hand-built MIR. Covered rather than left as an unexecuted region,
    // per D-014.
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::Assign {
            target: "x".to_string(),
            value: MirExpr::BinOp {
                op: BinOpKind::Mul,
                left: Box::new(MirExpr::IntLiteral(2)),
                right: Box::new(MirExpr::IntLiteral(3)),
                ty: Ty::Str,
            },
        })],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("str_repetition_without_str_operand")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("str_repetition_without_str_operand.o");
    let _ = compile_to_object(&mir, &obj_path, None, false);
}

#[test]
fn compiles_bool_promoted_to_float_in_mixed_arithmetic() {
    // `y = True + 0.5` -- `bool` is `int`-compatible, and any `float`
    // operand promotes the whole expression to `float`
    // (`pycc_types`' `numeric_or_bool_compatible`); exercises
    // `to_float`'s own `Scalar::Bool` arm, not otherwise reached by
    // this task's other fixtures (`compiles_mixed_int_and_float_
    // addition` above only ever passes `to_float` an `Int` or `Float`
    // operand).
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::Assign {
            target: "y".to_string(),
            value: MirExpr::BinOp {
                op: BinOpKind::Add,
                left: Box::new(MirExpr::BoolLiteral(true)),
                right: Box::new(MirExpr::FloatLiteral(0.5)),
                ty: Ty::Float,
            },
        })],
        class_defs: Vec::new(),
    };
    let dir =
        pycc_scratch::ScratchDir::new("bool_float_mixed").expect("failed to create scratch dir");
    let obj_path = dir.join("bool_float_mixed.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
}

#[test]
fn compiles_the_remaining_float_binop_kinds() {
    // `compiles_true_division_of_two_ints_as_float_arithmetic` already
    // covers `Div`; this exercises every other `BinOpKind` arm under a
    // `Ty::Float` result -- `Add`/`Sub`/`Mul` go through `build_float_*`
    // directly, `FloorDiv`/`Mod`/`Pow` through the `pycc_rt_float_*`
    // runtime calls -- mirroring `compiles_and_runs_add_sub_mul_mod_
    // and_pow_binops`'s `int` coverage. This test itself doesn't print
    // any of these results (same limitation as `compiles_true_division_
    // of_two_ints_as_float_arithmetic`/`compiles_mixed_int_and_float_
    // addition` above -- `print(float)` runtime output is exercised
    // separately, e.g. via `compiles_a_multi_argument_print_with_mixed_
    // types_space_separated`'s `2.5` argument), so this only proves
    // each arm compiles and verifies, not a runtime stdout value.
    fn float_binop(op: BinOpKind, left: f64, right: f64) -> MirStmt {
        MirStmt::Assign {
            target: format!("{op:?}").to_lowercase(),
            value: MirExpr::BinOp {
                op,
                left: Box::new(MirExpr::FloatLiteral(left)),
                right: Box::new(MirExpr::FloatLiteral(right)),
                ty: Ty::Float,
            },
        }
    }
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(float_binop(BinOpKind::Add, 3.0, 4.0)),
            MirItem::TopLevelStmt(float_binop(BinOpKind::Sub, 10.0, 3.0)),
            MirItem::TopLevelStmt(float_binop(BinOpKind::Mul, 6.0, 7.0)),
            MirItem::TopLevelStmt(float_binop(BinOpKind::FloorDiv, 7.0, 2.0)),
            MirItem::TopLevelStmt(float_binop(BinOpKind::Mod, 7.0, 2.0)),
            MirItem::TopLevelStmt(float_binop(BinOpKind::Pow, 2.0, 5.0)),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("float_binops").expect("failed to create scratch dir");
    let obj_path = dir.join("float_binops.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
}

#[test]
fn compiles_a_mixed_int_and_float_comparison() {
    // `1 < 1.5` -- exercises the `Compare` arm's `left_ty == Ty::Float
    // || right_ty == Ty::Float` promotion check's right-hand disjunct:
    // `left_ty` alone is `Ty::Int` here, so only evaluating `right_ty`
    // decides this comparison promotes to `float` (distinct from
    // `compiles_a_float_comparison`, where `left_ty == Ty::Float`
    // alone already decides it, short-circuiting before `right_ty` is
    // even considered).
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::Assign {
            target: "b".to_string(),
            value: MirExpr::Compare {
                op: CmpOpKind::Lt,
                left: Box::new(MirExpr::IntLiteral(1)),
                right: Box::new(MirExpr::FloatLiteral(1.5)),
                ty: Ty::Bool,
            },
        })],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("mixed_cmp").expect("failed to create scratch dir");
    let obj_path = dir.join("mixed_cmp.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
}

#[test]
fn compiles_the_remaining_float_comparison_operators() {
    // `Lt` already has its own dedicated test above
    // (`compiles_a_float_comparison`); this exercises the rest of
    // `FloatPredicate`'s match arms (`Eq`/`NotEq`/`LtE`/`Gt`/`GtE`),
    // mirroring `compiles_the_remaining_comparison_operators`'s `int`
    // coverage.
    fn assign_compare(target: &str, op: CmpOpKind) -> MirStmt {
        MirStmt::Assign {
            target: target.to_string(),
            value: MirExpr::Compare {
                op,
                left: Box::new(MirExpr::FloatLiteral(1.0)),
                right: Box::new(MirExpr::FloatLiteral(2.0)),
                ty: Ty::Bool,
            },
        }
    }
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(assign_compare("a", CmpOpKind::Eq)),
            MirItem::TopLevelStmt(assign_compare("b", CmpOpKind::NotEq)),
            MirItem::TopLevelStmt(assign_compare("c", CmpOpKind::LtE)),
            MirItem::TopLevelStmt(assign_compare("d", CmpOpKind::Gt)),
            MirItem::TopLevelStmt(assign_compare("e", CmpOpKind::GtE)),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("remaining_float_cmp_ops")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("remaining_float_cmp_ops.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
}

#[test]
fn compiles_string_concatenation_and_a_reassignment_that_frees_the_old_value() {
    // `x = "foo"; x = x + "bar"` -- the second `Assign` reads the
    // existing `x` (needs an incref before rebinding) and overwrites
    // `x`'s slot (must decref the *original* `"foo"` first). Nothing
    // observes the refcounting directly; this proves it doesn't crash
    // and that codegen for the whole sequence succeeds.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "x".to_string(),
                value: MirExpr::StringLiteral("foo".to_string()),
            }),
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "x".to_string(),
                value: MirExpr::BinOp {
                    op: BinOpKind::Add,
                    left: Box::new(MirExpr::Name {
                        name: "x".to_string(),
                        ty: Ty::Str,
                    }),
                    right: Box::new(MirExpr::StringLiteral("bar".to_string())),
                    ty: Ty::Str,
                },
            }),
        ],
        class_defs: Vec::new(),
    };
    let dir =
        pycc_scratch::ScratchDir::new("str_concat_reassign").expect("failed to create scratch dir");
    let obj_path = dir.join("str_concat_reassign.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("str_concat_reassign");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert!(output.status.success(), "should run without crashing");
}

#[test]
fn a_repeated_string_prints_its_repetition() {
    // `print("ab" * 3)`, `print(0 * "ab")`, `print("ab" * True)` and
    // `print("ab" * n)` for `n = 0 - 2` -- the executable half of #575.
    // The negative count comes from `0 - 2` rather than a `-2` literal:
    // this test predates #602's literal-sign fold. Both forms are real
    // reachable programs, and CPython prints an empty line for each.
    let repeat = |left: MirExpr, right: MirExpr| {
        MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
            callee: "print".to_string(),
            args: vec![MirExpr::BinOp {
                op: BinOpKind::Mul,
                left: Box::new(left),
                right: Box::new(right),
                ty: Ty::Str,
            }],
            ty: Ty::None,
        }))
    };
    let mir = MirModule {
        items: vec![
            repeat(
                MirExpr::StringLiteral("ab".to_string()),
                MirExpr::IntLiteral(3),
            ),
            repeat(
                MirExpr::IntLiteral(0),
                MirExpr::StringLiteral("ab".to_string()),
            ),
            repeat(
                MirExpr::StringLiteral("ab".to_string()),
                MirExpr::BoolLiteral(true),
            ),
            repeat(
                MirExpr::StringLiteral("ab".to_string()),
                MirExpr::BinOp {
                    op: BinOpKind::Sub,
                    left: Box::new(MirExpr::IntLiteral(0)),
                    right: Box::new(MirExpr::IntLiteral(2)),
                    ty: Ty::Int,
                },
            ),
        ],
        class_defs: Vec::new(),
    };
    let dir =
        pycc_scratch::ScratchDir::new("str_repeat_prints").expect("failed to create scratch dir");
    let obj_path = dir.join("str_repeat_prints.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("str_repeat_prints");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"ababab\n\nab\n\n");
}

#[test]
#[should_panic(expected = "string assignment target `value` has a non-string storage slot")]
fn a_string_assignment_rejects_a_predeclared_non_string_slot() {
    // Deliberately malformed MIR: the checked pipeline preserves a
    // binding's established representation, so it cannot assign `str`
    // to an `int` target. The hand-built shape verifies codegen rejects
    // the mismatch before treating an integer slot as a string pointer.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "value".to_string(),
                value: MirExpr::IntLiteral(1),
            }),
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "value".to_string(),
                value: MirExpr::StringLiteral("bad".to_string()),
            }),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("str_assignment_non_str_slot_panics")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("str_assignment_non_str_slot_panics.o");
    let _ = compile_to_object(&mir, &obj_path, None, false);
}

#[test]
fn compiles_a_string_comparison() {
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::Assign {
            target: "b".to_string(),
            value: MirExpr::Compare {
                op: CmpOpKind::Lt,
                left: Box::new(MirExpr::StringLiteral("apple".to_string())),
                right: Box::new(MirExpr::StringLiteral("banana".to_string())),
                ty: Ty::Bool,
            },
        })],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("str_cmp").expect("failed to create scratch dir");
    let obj_path = dir.join("str_cmp.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
}

#[test]
fn a_string_comparison_result_is_correct_at_runtime() {
    // `if "a" < "b": print(1)` -- unlike `compiles_a_string_comparison`
    // above (which only proves codegen for a `str` `Compare` succeeds),
    // this proves `pycc_rt_str_cmp`'s lexicographic ordering actually
    // drives a real `if` branch decision correctly, in both directions.
    for (left, right, expected) in [("a", "b", "1\n"), ("b", "a", "")] {
        let mir = MirModule {
            items: vec![MirItem::TopLevelStmt(MirStmt::If {
                test: MirExpr::Compare {
                    op: CmpOpKind::Lt,
                    left: Box::new(MirExpr::StringLiteral(left.to_string())),
                    right: Box::new(MirExpr::StringLiteral(right.to_string())),
                    ty: Ty::Bool,
                },
                body: vec![MirStmt::ExprStmt(MirExpr::Call {
                    callee: "print".to_string(),
                    args: vec![MirExpr::IntLiteral(1)],
                    ty: Ty::None,
                })],
                orelse: vec![],
            })],
            class_defs: Vec::new(),
        };
        let dir = pycc_scratch::ScratchDir::new(&format!("str_cmp_runtime_{left}_{right}"))
            .expect("failed to create scratch dir");
        let obj_path = dir.join("str_cmp_runtime.o");
        compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
        let bin_path = dir.join("str_cmp_runtime");
        link_object_with_runtime(&obj_path, &bin_path);
        let output = Command::new(&bin_path).output().expect("binary should run");
        assert_eq!(
            output.stdout,
            expected.as_bytes(),
            "comparing {left:?} < {right:?}"
        );
    }
}

#[test]
fn compiles_the_remaining_string_comparison_operators() {
    // `Lt` already has its own dedicated test above
    // (`compiles_a_string_comparison`); this exercises the rest of the
    // `str` branch's `IntPredicate` match arms
    // (`Eq`/`NotEq`/`LtE`/`Gt`/`GtE`), mirroring
    // `compiles_the_remaining_comparison_operators`'s `int` coverage and
    // `compiles_the_remaining_float_comparison_operators`'s `float` one.
    fn assign_compare(target: &str, op: CmpOpKind) -> MirStmt {
        MirStmt::Assign {
            target: target.to_string(),
            value: MirExpr::Compare {
                op,
                left: Box::new(MirExpr::StringLiteral("a".to_string())),
                right: Box::new(MirExpr::StringLiteral("b".to_string())),
                ty: Ty::Bool,
            },
        }
    }
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(assign_compare("a", CmpOpKind::Eq)),
            MirItem::TopLevelStmt(assign_compare("b", CmpOpKind::NotEq)),
            MirItem::TopLevelStmt(assign_compare("c", CmpOpKind::LtE)),
            MirItem::TopLevelStmt(assign_compare("d", CmpOpKind::Gt)),
            MirItem::TopLevelStmt(assign_compare("e", CmpOpKind::GtE)),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("remaining_str_cmp_ops")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("remaining_str_cmp_ops.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
}

#[test]
#[should_panic(expected = "internal error: str Compare operand did not evaluate to str")]
fn a_mixed_int_and_string_comparison_hits_the_internal_consistency_check() {
    // `1 < "x"` -- deliberately malformed MIR (`pycc_types` never mixes
    // `int`/`str` operands in one comparison): `left_ty` alone is
    // `Ty::Int` here, so only evaluating `right_ty` decides this enters
    // the `Compare` arm's `str` branch (exercising that `||`'s
    // right-hand disjunct, mirroring
    // `compiles_a_mixed_int_and_float_comparison`'s identical
    // left-Int/right-Float construction) -- and since the left operand
    // genuinely evaluates to `Scalar::Int`, this also isolates the
    // `str` branch's *left*-operand internal-consistency check.
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::Assign {
            target: "b".to_string(),
            value: MirExpr::Compare {
                op: CmpOpKind::Lt,
                left: Box::new(MirExpr::IntLiteral(1)),
                right: Box::new(MirExpr::StringLiteral("x".to_string())),
                ty: Ty::Bool,
            },
        })],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("mixed_int_str_cmp_panics")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("mixed_int_str_cmp_panics.o");
    let _ = compile_to_object(&mir, &obj_path, None, false);
}

#[test]
#[should_panic(expected = "internal error: str Compare operand did not evaluate to str")]
fn a_string_comparison_with_a_lying_right_operand_hits_the_internal_consistency_check() {
    // Deliberately malformed MIR, isolating the `Compare` arm's str
    // branch's *right*-operand check specifically (the test above only
    // ever reaches the *left*-operand one, since that check runs
    // first): the right operand is a nested `Compare` node that claims
    // `ty: Ty::Str` but -- like every `Compare` node, regardless of its
    // own `ty` field -- always evaluates to `Scalar::Bool` (`emit_expr`'s
    // `Compare` arm never reads its own `ty` when constructing its
    // result; only a *parent* expression's `left.ty()`/`right.ty()`
    // call ever inspects it). This makes `right_ty == Ty::Str` true
    // (entering the branch) while `r` itself evaluates to `Scalar::Bool`
    // -- with the left operand a real `str`, so the left-operand check
    // passes and only the right-operand one fires. Same "nested lying
    // node" convention as this file's other internal-consistency tests
    // (e.g. `printing_a_mistyped_compare_expression_hits_the_internal_consistency_check`).
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::Assign {
            target: "b".to_string(),
            value: MirExpr::Compare {
                op: CmpOpKind::Eq,
                left: Box::new(MirExpr::StringLiteral("x".to_string())),
                right: Box::new(MirExpr::Compare {
                    op: CmpOpKind::Lt,
                    left: Box::new(MirExpr::IntLiteral(1)),
                    right: Box::new(MirExpr::IntLiteral(2)),
                    ty: Ty::Str,
                }),
                ty: Ty::Bool,
            },
        })],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("lying_str_cmp_panics")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("lying_str_cmp_panics.o");
    let _ = compile_to_object(&mir, &obj_path, None, false);
}

#[test]
fn compiles_an_if_test_on_a_string_expression() {
    // `if "": print(1)` prints nothing; `if "x": print(1)` prints `1`.
    for (test, expected) in [("", ""), ("x", "1\n")] {
        let mir = MirModule {
            items: vec![MirItem::TopLevelStmt(MirStmt::If {
                test: MirExpr::StringLiteral(test.to_string()),
                body: vec![MirStmt::ExprStmt(MirExpr::Call {
                    callee: "print".to_string(),
                    args: vec![MirExpr::IntLiteral(1)],
                    ty: Ty::None,
                })],
                orelse: vec![],
            })],
            class_defs: Vec::new(),
        };
        let dir = pycc_scratch::ScratchDir::new(&format!("str_truthy_{}", test.len()))
            .expect("failed to create scratch dir");
        let obj_path = dir.join("str_truthy.o");
        compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
        let bin_path = dir.join("str_truthy");
        link_object_with_runtime(&obj_path, &bin_path);
        let output = Command::new(&bin_path).output().expect("binary should run");
        assert_eq!(output.stdout, expected.as_bytes(), "test value {test:?}");
    }
}

#[test]
fn compiles_a_string_literal_longer_than_the_inline_cap() {
    let long = "y".repeat(30); // exceeds D-059's 22-byte inline threshold
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::Assign {
            target: "s".to_string(),
            value: MirExpr::StringLiteral(long),
        })],
        class_defs: Vec::new(),
    };
    let dir =
        pycc_scratch::ScratchDir::new("str_long_literal").expect("failed to create scratch dir");
    let obj_path = dir.join("str_long_literal.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
}

#[test]
fn compiles_a_function_with_a_str_parameter_and_str_return_value() {
    // `def f(x: str) -> str: return x` ; `y = f("hi")` -- exercises
    // `ty_to_basic_type`'s `Ty::Str` arm (both the parameter and
    // return-type positions), `build_call_to`'s argument-marshaling
    // match's `Scalar::Str` arm (plus `incref_if_str_duplicate`'s
    // duplicate-reference branch on the `"hi"` literal argument, which
    // is *not* a duplicate reference, exercising its `else` half),
    // `emit_expr`'s `Call` arm's `Ty::Str`-result match arm, and
    // `emit_stmt`'s `Return` arm's `Scalar::Str` arm together with
    // `incref_if_str_duplicate`'s duplicate-reference branch on `return
    // x` (a bare `Name`, which *is* a duplicate reference, exercising
    // its `if` half) -- every `str`-typed position Task 5/6's
    // float-parameter precedent
    // (`compiles_a_function_with_a_float_parameter_and_float_return_value`)
    // established for `float`, now closed for `str`.
    let mir = MirModule {
        items: vec![
            MirItem::Function {
                name: "f".to_string(),
                params: vec![("x".to_string(), Ty::Str)],
                return_ty: Ty::Str,
                body: vec![MirStmt::Return(Some(MirExpr::Name {
                    name: "x".to_string(),
                    ty: Ty::Str,
                }))],
            },
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "y".to_string(),
                value: MirExpr::Call {
                    callee: "f".to_string(),
                    args: vec![MirExpr::StringLiteral("hi".to_string())],
                    ty: Ty::Str,
                },
            }),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("str_param_and_return")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("str_param_and_return.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("str_param_and_return");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert!(output.status.success(), "should run without crashing");
}

#[test]
#[should_panic(expected = "expected an int-or-bool operand, got str")]
fn an_int_result_binop_with_a_str_operand_hits_to_numeric_encoded_int_defensive_panic() {
    // Deliberately malformed MIR: `pycc_types::numeric_result_type`
    // never types a `str`-operand `BinOp` as `Ty::Int` (`str` only ever
    // combines with `str`, under `Add`, per its own
    // `adding_two_strings_infers_str` test), so no real pipeline could
    // ever produce this shape. Exercises `to_numeric_encoded_int`'s own
    // defensive `Scalar::Str` arm -- same convention as
    // `an_int_result_binop_with_a_float_operand_hits_to_numeric_encoded_int_defensive_panic`
    // above, now for `str` instead of `float`.
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::Assign {
            target: "x".to_string(),
            value: MirExpr::BinOp {
                op: BinOpKind::Add,
                left: Box::new(MirExpr::StringLiteral("x".to_string())),
                right: Box::new(MirExpr::IntLiteral(1)),
                ty: Ty::Int,
            },
        })],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("binop_int_result_str_operand_panics")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("binop_int_result_str_operand_panics.o");
    let _ = compile_to_object(&mir, &obj_path, None, false);
}

#[test]
#[should_panic(expected = "expected a numeric operand, got str")]
fn a_float_result_binop_with_a_str_operand_hits_to_float_defensive_panic() {
    // Same rationale as the `to_numeric_encoded_int` version above, exercising
    // `to_float`'s own defensive `Scalar::Str` arm instead (a brand-new
    // arm this task adds -- `to_float`'s match was previously exhaustive
    // over `Int`/`Bool`/`Float` alone, with no catch-all to fill in).
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::Assign {
            target: "x".to_string(),
            value: MirExpr::BinOp {
                op: BinOpKind::Add,
                left: Box::new(MirExpr::StringLiteral("x".to_string())),
                right: Box::new(MirExpr::FloatLiteral(1.0)),
                ty: Ty::Float,
            },
        })],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("binop_float_result_str_operand_panics")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("binop_float_result_str_operand_panics.o");
    let _ = compile_to_object(&mir, &obj_path, None, false);
}
