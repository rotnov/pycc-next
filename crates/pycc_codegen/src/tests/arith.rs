//! Integer/bool arithmetic and comparisons, int literals and bigint promotion.

use super::*;

#[test]
fn compiles_local_variable_arithmetic_comparisons_and_floor_division() {
    // `x = 7; y = 2; print(x // y)` at the MIR level, exercising: a fresh
    // `alloca` per local, `BinOp::FloorDiv` codegen, and reading a `Name`
    // back out of its local for a later statement -- everything Task 2's
    // temporary `emit_stmt` explicitly could not do yet.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "x".to_string(),
                value: MirExpr::IntLiteral(7),
            }),
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "y".to_string(),
                value: MirExpr::IntLiteral(2),
            }),
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
                callee: "print".to_string(),
                args: vec![MirExpr::BinOp {
                    op: BinOpKind::FloorDiv,
                    left: Box::new(MirExpr::Name {
                        name: "x".to_string(),
                        ty: Ty::Int,
                    }),
                    right: Box::new(MirExpr::Name {
                        name: "y".to_string(),
                        ty: Ty::Int,
                    }),
                    ty: Ty::Int,
                }],
                ty: Ty::None,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("locals_arith").expect("failed to create scratch dir");
    let obj_path = dir.join("locals_arith.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("locals_arith");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(pycc_scratch::lf(&output.stdout), b"3\n");
}

#[test]
fn compiles_a_comparison_result_stored_in_a_bool_local() {
    // `b = 1 < 2` -- exercises `Compare` codegen and a `bool`-typed
    // (`i8`) local's own `alloca`, distinct from `int`'s tagged `i64`.
    // Nothing here reads `b` back out (a dedicated runtime `print(bool)`
    // test exists separately -- `compiles_print_of_a_bool_false`), so
    // this only proves the assignment itself doesn't crash/miscompile;
    // `verify_module`'s `module.verify()` call (non-Windows) is the
    // actual proof the generated IR is well-formed.
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::Assign {
            target: "b".to_string(),
            value: MirExpr::Compare {
                op: CmpOpKind::Lt,
                left: Box::new(MirExpr::IntLiteral(1)),
                right: Box::new(MirExpr::IntLiteral(2)),
                ty: Ty::Bool,
            },
        })],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("bool_local").expect("failed to create scratch dir");
    let obj_path = dir.join("bool_local.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
}

#[test]
fn reassigning_a_local_reuses_its_existing_alloca() {
    // `x = 1; x = 2; print(x)` -- the second `Assign` must reuse `x`'s
    // existing slot (not allocate a second, shadowing one), matching
    // ordinary Python rebinding semantics.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "x".to_string(),
                value: MirExpr::IntLiteral(1),
            }),
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "x".to_string(),
                value: MirExpr::IntLiteral(2),
            }),
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
                callee: "print".to_string(),
                args: vec![MirExpr::Name {
                    name: "x".to_string(),
                    ty: Ty::Int,
                }],
                ty: Ty::None,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir =
        pycc_scratch::ScratchDir::new("reassign_local").expect("failed to create scratch dir");
    let obj_path = dir.join("reassign_local.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("reassign_local");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(pycc_scratch::lf(&output.stdout), b"2\n");
}

#[test]
fn compiles_and_runs_add_sub_mul_mod_and_pow_binops() {
    // Exercises every remaining `BinOpKind` arm `emit_expr`'s `BinOp`
    // codegen selects a `pycc_rt` function for (`FloorDiv` already has
    // its own dedicated test above) end to end: compiled, linked, run,
    // with real stdout checked -- not just that codegen didn't panic.
    fn print_binop(op: BinOpKind, left: i64, right: i64) -> MirStmt {
        MirStmt::ExprStmt(MirExpr::Call {
            callee: "print".to_string(),
            args: vec![MirExpr::BinOp {
                op,
                left: Box::new(MirExpr::IntLiteral(left)),
                right: Box::new(MirExpr::IntLiteral(right)),
                ty: Ty::Int,
            }],
            ty: Ty::None,
        })
    }
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(print_binop(BinOpKind::Add, 3, 4)),
            MirItem::TopLevelStmt(print_binop(BinOpKind::Sub, 10, 3)),
            MirItem::TopLevelStmt(print_binop(BinOpKind::Mul, 6, 7)),
            MirItem::TopLevelStmt(print_binop(BinOpKind::Mod, 7, 2)),
            MirItem::TopLevelStmt(print_binop(BinOpKind::Pow, 2, 5)),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("int_binops").expect("failed to create scratch dir");
    let obj_path = dir.join("int_binops.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("int_binops");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(pycc_scratch::lf(&output.stdout), b"7\n7\n42\n1\n32\n");
}

#[test]
#[should_panic(expected = "true division")]
fn true_division_binop_codegen_panics_via_its_dedicated_arm() {
    // `pycc_mir::binop_result_ty` always types `BinOpKind::Div` as
    // `Ty::Float` (`5 / 2 == 2.5`, never `Ty::Int`), so no real
    // `pycc_types`-produced MIR can construct `BinOp { op: Div, ty:
    // Ty::Int, .. }` -- a real float-division `BinOp` (`ty: Ty::Float`)
    // is now correctly handled by `to_float`/`build_float_div` (Task 6;
    // see `compiles_true_division_of_two_ints_as_float_arithmetic`
    // above), not a catch-all. The *only* way to reach this dedicated
    // (now `unreachable!`) `Div` arm inside the `BinOp { ty: Ty::Int,
    // .. }` match is to hand-construct this deliberately mislabeled
    // shape, matching this crate's existing convention (see
    // `printing_a_mistyped_compare_expression_hits_the_internal_consistency_check`
    // below) for testing defensive arms real MIR can't reach.
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::Assign {
            target: "x".to_string(),
            value: MirExpr::BinOp {
                op: BinOpKind::Div,
                left: Box::new(MirExpr::IntLiteral(4)),
                right: Box::new(MirExpr::IntLiteral(2)),
                ty: Ty::Int,
            },
        })],
        class_defs: Vec::new(),
    };
    let dir =
        pycc_scratch::ScratchDir::new("true_div_panics").expect("failed to create scratch dir");
    let obj_path = dir.join("true_div_panics.o");
    let _ = compile_to_object(&mir, &obj_path, None, false);
}

#[test]
fn compiles_the_remaining_comparison_operators() {
    // `Lt` already has its own dedicated test above; this exercises the
    // rest of `IntPredicate`'s match arms (`Eq`/`NotEq`/`LtE`/`Gt`/`GtE`).
    fn assign_compare(target: &str, op: CmpOpKind) -> MirStmt {
        MirStmt::Assign {
            target: target.to_string(),
            value: MirExpr::Compare {
                op,
                left: Box::new(MirExpr::IntLiteral(1)),
                right: Box::new(MirExpr::IntLiteral(2)),
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
    let dir =
        pycc_scratch::ScratchDir::new("remaining_cmp_ops").expect("failed to create scratch dir");
    let obj_path = dir.join("remaining_cmp_ops.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
}

#[test]
fn ge_and_ne_execute_with_the_correct_boolean_value_for_int() {
    // `compiles_the_remaining_comparison_operators` above only proves
    // `GtE`/`NotEq` produce IR that compiles -- it never links, runs, or
    // checks a value, so a predicate swap (e.g. `IntPredicate::SGE` ->
    // `IntPredicate::SGT`) would still pass every existing test. This
    // links and runs, asserting the actual computed booleans.
    fn print_compare(op: CmpOpKind, left: i64, right: i64) -> MirStmt {
        MirStmt::ExprStmt(MirExpr::Call {
            callee: "print".to_string(),
            args: vec![MirExpr::Compare {
                op,
                left: Box::new(MirExpr::IntLiteral(left)),
                right: Box::new(MirExpr::IntLiteral(right)),
                ty: Ty::Bool,
            }],
            ty: Ty::None,
        })
    }
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(print_compare(CmpOpKind::GtE, 5, 5)),
            MirItem::TopLevelStmt(print_compare(CmpOpKind::GtE, 5, 6)),
            MirItem::TopLevelStmt(print_compare(CmpOpKind::NotEq, 5, 5)),
            MirItem::TopLevelStmt(print_compare(CmpOpKind::NotEq, 5, 6)),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("ge_ne_values").expect("failed to create scratch dir");
    let obj_path = dir.join("ge_ne_values.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("ge_ne_values");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(pycc_scratch::lf(&output.stdout), b"True\nFalse\nFalse\nTrue\n");
}

#[test]
fn a_nan_float_is_not_equal_to_itself_and_is_truthy() {
    // `float('nan') != float('nan')` and `bool(float('nan'))` are both
    // `True` in CPython (IEEE-754 unordered-comparison semantics) --
    // `Compare`'s `NotEq` arm and `truthy`'s `Float` arm both
    // deliberately use `FloatPredicate::UNE` (not the ordered `ONE`) to
    // match this. v0.1's Python-source surface has no NaN-producing
    // expression (this same diff's `float_pow` domain guards ensure
    // `**` panics before a NaN could leak out that path either), so a
    // hand-built `FloatLiteral(f64::NAN)` is the only way to exercise
    // it -- without this test, swapping either `UNE` for `ONE` would
    // still pass every other test in the suite.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
                callee: "print".to_string(),
                args: vec![MirExpr::Compare {
                    op: CmpOpKind::NotEq,
                    left: Box::new(MirExpr::FloatLiteral(f64::NAN)),
                    right: Box::new(MirExpr::FloatLiteral(f64::NAN)),
                    ty: Ty::Bool,
                }],
                ty: Ty::None,
            })),
            MirItem::TopLevelStmt(MirStmt::If {
                test: MirExpr::FloatLiteral(f64::NAN),
                body: vec![MirStmt::ExprStmt(MirExpr::Call {
                    callee: "print".to_string(),
                    args: vec![MirExpr::StringLiteral("nan_is_truthy".to_string())],
                    ty: Ty::None,
                })],
                orelse: vec![],
            }),
        ],
        class_defs: Vec::new(),
    };
    let dir =
        pycc_scratch::ScratchDir::new("nan_comparisons").expect("failed to create scratch dir");
    let obj_path = dir.join("nan_comparisons.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("nan_comparisons");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(pycc_scratch::lf(&output.stdout), b"True\nnan_is_truthy\n");
}

#[test]
fn reading_a_bool_local_back_out_of_its_alloca() {
    // `b = 1 < 2; c = b` -- exercises `emit_expr`'s `Name` arm on a
    // `Ty::Bool` local (the existing bool-local test above only ever
    // assigns one, never reads it back).
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "b".to_string(),
                value: MirExpr::Compare {
                    op: CmpOpKind::Lt,
                    left: Box::new(MirExpr::IntLiteral(1)),
                    right: Box::new(MirExpr::IntLiteral(2)),
                    ty: Ty::Bool,
                },
            }),
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "c".to_string(),
                value: MirExpr::Name {
                    name: "b".to_string(),
                    ty: Ty::Bool,
                },
            }),
        ],
        class_defs: Vec::new(),
    };
    let dir =
        pycc_scratch::ScratchDir::new("read_bool_local").expect("failed to create scratch dir");
    let obj_path = dir.join("read_bool_local.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
}

#[test]
fn adding_a_bool_left_operand_to_an_int_promotes_bool_to_int() {
    // `x = True + 1; print(x)` -- `pycc_types` accepts this (`bool` is
    // numeric-like, see its own `a_binop_treats_bool_as_int` test) and
    // infers `Ty::Int`. This file's earlier (Task 3) version of this
    // test proved that `emit_expr` did not yet implement the
    // bool-to-int promotion this needs, and hit a defensive check
    // mislabeled "internal error" for what a prior review correctly
    // flagged is actually reachable from real, legitimate source (this
    // exact case). Task 6's `to_numeric_encoded_int` (see its own doc comment)
    // now implements that promotion for real, so this is rewritten
    // into what it always should have been: a positive test proving
    // `True + 1` correctly computes `2`, not a panic.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "x".to_string(),
                value: MirExpr::BinOp {
                    op: BinOpKind::Add,
                    left: Box::new(MirExpr::BoolLiteral(true)),
                    right: Box::new(MirExpr::IntLiteral(1)),
                    ty: Ty::Int,
                },
            }),
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
                callee: "print".to_string(),
                args: vec![MirExpr::Name {
                    name: "x".to_string(),
                    ty: Ty::Int,
                }],
                ty: Ty::None,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("binop_bool_left_promotes")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("binop_bool_left_promotes.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("binop_bool_left_promotes");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(pycc_scratch::lf(&output.stdout), b"2\n");
}

#[test]
fn adding_an_int_and_a_bool_right_operand_promotes_bool_to_int() {
    // `x = 1 + True; print(x)` -- distinct region from the
    // left-operand case above (`to_numeric_encoded_int` is called once per
    // operand).
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "x".to_string(),
                value: MirExpr::BinOp {
                    op: BinOpKind::Add,
                    left: Box::new(MirExpr::IntLiteral(1)),
                    right: Box::new(MirExpr::BoolLiteral(true)),
                    ty: Ty::Int,
                },
            }),
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
                callee: "print".to_string(),
                args: vec![MirExpr::Name {
                    name: "x".to_string(),
                    ty: Ty::Int,
                }],
                ty: Ty::None,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("binop_bool_right_promotes")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("binop_bool_right_promotes.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("binop_bool_right_promotes");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(pycc_scratch::lf(&output.stdout), b"2\n");
}

#[test]
fn comparing_a_bool_left_operand_to_an_int_promotes_bool_to_int() {
    // `True < 2` -- `pycc_types` accepts comparing `bool` and `int`
    // (`bool` is a subtype of `int`, see its own
    // `comparing_a_bool_and_an_int_succeeds_since_bool_is_a_subtype_of_int`
    // test); Task 6's `Compare` codegen now promotes the `bool`
    // operand via `to_numeric_encoded_int` instead of rejecting it (same
    // rewrite rationale as the `BinOp` tests above). Nothing reads
    // the result back here (see `compiles_print_of_a_bool_false` for a
    // dedicated runtime `print(bool)` test), so this only proves the
    // comparison itself doesn't crash/miscompile, same as
    // `compiles_a_comparison_result_stored_in_a_bool_local`.
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::Assign {
            target: "x".to_string(),
            value: MirExpr::Compare {
                op: CmpOpKind::Lt,
                left: Box::new(MirExpr::BoolLiteral(true)),
                right: Box::new(MirExpr::IntLiteral(2)),
                ty: Ty::Bool,
            },
        })],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("compare_bool_left_promotes")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("compare_bool_left_promotes.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
}

#[test]
fn comparing_an_int_and_a_bool_right_operand_promotes_bool_to_int() {
    // Distinct region from the left-operand case above (`1 < True`).
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::Assign {
            target: "x".to_string(),
            value: MirExpr::Compare {
                op: CmpOpKind::Lt,
                left: Box::new(MirExpr::IntLiteral(1)),
                right: Box::new(MirExpr::BoolLiteral(true)),
                ty: Ty::Bool,
            },
        })],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("compare_bool_right_promotes")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("compare_bool_right_promotes.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
}

// Inverted for #148/D-178: this module is exactly the one the retired
// `an_oversized_int_literal_is_not_yet_supported` compiled, and it must
// now succeed by materializing the literal through
// `pycc_rt_int_from_i64` rather than panicking in `tag_smallint_const`.
#[test]
fn an_oversized_int_literal_materializes_a_runtime_bigint() {
    for value in [
        i64::MAX,
        4_611_686_018_427_387_904_i64,
        -4_611_686_018_427_387_905_i64,
        i64::MIN,
    ] {
        let mir = MirModule {
            items: vec![MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
                callee: "print".to_string(),
                args: vec![MirExpr::IntLiteral(value)],
                ty: Ty::None,
            }))],
            class_defs: Vec::new(),
        };
        let dir = pycc_scratch::ScratchDir::new("oversized_int_literal_materializes")
            .expect("failed to create scratch dir");
        let obj_path = dir.join("oversized_int_literal_materializes.o");
        let mut saw_call = false;
        // `Module::print_to_string` returns an `inkwell` `LLVMString`,
        // whose `Drop` calls `LLVMDisposeMessage` -- which faults with
        // `STATUS_ACCESS_VIOLATION` on Windows against this LLVM
        // release (D-029). Route it through `llvm_string_to_owned`,
        // exactly as this crate's error paths already do, instead of
        // letting the temporary drop.
        let mut observer = |module: &inkwell::module::Module<'_>, _| {
            saw_call |= llvm_string_to_owned(module.print_to_string())
                .contains("call i64 @pycc_rt_int_from_i64");
        };
        compile_to_object_with_observer(
            &mir,
            &obj_path,
            &CompileOptions::default(),
            Some(&mut observer),
        )
        .expect("an out-of-range int literal should compile");
        assert!(saw_call, "{value} should be materialized at run time");
    }
}

// D-029 static guard. Every inkwell API that hands back an `LLVMString`
// is a Windows crash waiting to happen: the wrapper's `Drop` calls
// `LLVMDisposeMessage`, which faults against the prebuilt LLVM this
// project links there. Reaching past this crate's own protections to
// the raw inkwell API compiles and passes every local gate, then kills
// the whole test binary in CI, because the Windows job is the only
// place this class is ever executed. This test is the mechanical
// stand-in for that missing local gate.
//
// D-029 records three distinct protections, and this test covers them
// to three different depths -- state that plainly rather than letting
// the name imply uniform coverage:
//
//  1. `llvm_string_to_owned`, which forgets the wrapper instead of
//     dropping it. Fully checked: every `print_to_string` call must
//     name it on the same line.
//  2. `verify_module`, a no-op under `#[cfg(windows)]`. Fully checked:
//     exactly one direct `verify` call may exist, the wrapper's own.
//  3. `ManuallyDrop` at the point a `TargetTriple` is created, which
//     covers every exit path including the early `?`. Only tripwired:
//     the wrapping is structural and spans several lines, so a
//     line-oriented scan cannot confirm it. What it can do is pin the
//     number of triple-producing call sites, so adding one fails here
//     and sends the author to this comment.
//
// `Target::from_triple`'s and `write_to_file`'s `.map_err` sites fall
// under (1) and are checked only insofar as they name the wrapper.
//
// The needles are assembled at run time so this test's own body is not
// counted as a violation of itself. Comment lines are excluded, since
// the crate discusses all three APIs at length in prose.
//
// The three checks live in `d029_guard::d029_violations`, extracted
// from this test (#619) into that cohesion-driven submodule -- see
// AGENTS.md's decomposability rule -- so the exact same logic can be
// driven against synthetic sources instead of only the crate's real
// ones. See that function's own doc comment for why the extraction
// needs its own care, and `d029_guard::d029_violations_tests` for the
// automated proof this test used to require running by hand
// (`.harden/incidents/platform-wrapper-bypassed-by-new-code/incident.md`).
#[test]
fn every_inkwell_llvm_string_call_routes_through_a_d029_wrapper() {
    // Read the whole crate rather than this one file, so a module added
    // later is covered without anyone remembering to extend a list.
    let sources: Vec<(String, String)> =
        std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/src"))
            .expect("the crate's own source directory should be readable")
            .map(|entry| entry.expect("a readable directory entry").path())
            .filter(|path| path.extension() == Some(std::ffi::OsStr::new("rs")))
            .map(|path| {
                let contents = std::fs::read_to_string(&path).expect("a readable source file");
                (path.display().to_string(), contents)
            })
            .collect();
    let sources: Vec<(&str, &str)> = sources
        .iter()
        .map(|(path, contents)| (path.as_str(), contents.as_str()))
        .collect();

    let violations = d029_guard::d029_violations(&sources, 2);
    assert!(
        violations.is_empty(),
        "D-029 guard violation(s) found in the crate's own sources:\n{}",
        violations.join("\n")
    );
}

// The in-range arm of `fits_tagged_smallint`: still folded to an
// immediate, with no runtime call at all.
#[test]
fn an_in_range_int_literal_is_still_folded_at_compile_time() {
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
            callee: "print".to_string(),
            args: vec![MirExpr::IntLiteral(4_611_686_018_427_387_903_i64)],
            ty: Ty::None,
        }))],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("in_range_int_literal_folds")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("in_range_int_literal_folds.o");
    let mut saw_call = false;
    // D-029 again: `print_to_string`'s `LLVMString` must not be dropped
    // on Windows. See the sibling test above.
    let mut observer = |module: &inkwell::module::Module<'_>, _| {
        saw_call |= llvm_string_to_owned(module.print_to_string())
            .contains("call i64 @pycc_rt_int_from_i64");
    };
    compile_to_object_with_observer(
        &mir,
        &obj_path,
        &CompileOptions::default(),
        Some(&mut observer),
    )
    .expect("an in-range int literal should compile");
    assert!(
        !saw_call,
        "an in-range literal needs no runtime materialization"
    );
}

// `declare_module_globals` builds a constant initializer with no
// `Builder`, so `tag_smallint_const` survives the #148 refactor with
// its defensive panic. Generated code only ever reaches it with `0`;
// this test calls it directly, matching this file's existing
// defensive-arm convention.
#[test]
#[should_panic(expected = "too large for the constant-only")]
fn tag_smallint_const_still_rejects_an_out_of_range_constant() {
    let context = Context::create();
    let _ = tag_smallint_const(&context, i64::MAX);
}

#[test]
#[should_panic(expected = "an f-string with zero parts should not be reachable")]
fn assigning_a_zero_part_fstring_hits_the_defensive_internal_panic() {
    // Renamed and re-targeted for Task 8: this test's previous
    // (Task 3/7-era) incarnation exercised `emit_expr`'s final
    // catch-all arm (`other => panic!("...this expression kind's
    // codegen is not supported yet...")`), back when `MirExpr::FString`
    // had no arm of its own at all. Task 8 gives `FString` a real arm,
    // and with every other `MirExpr` variant already handled by its own
    // named arm, that catch-all became dead code (unreachable for any
    // input) and was removed rather than kept as untestable dead
    // weight -- the same "remove a provably dead arm" convention this
    // file already applies elsewhere (see `emit_expr`'s `Name` arm's
    // own doc comment).
    //
    // `MirExpr::FString(vec![])` (zero parts) is still not a real,
    // reachable program shape -- `pycc_hir`'s own f-string lowering
    // always produces at least one `Literal` part, even for a literal
    // empty f-string `f""` -- but `emit_expr`'s new `FString` arm
    // guards that assumption defensively instead of silently returning
    // a dangling/null pointer if it's ever wrong (see that arm's own
    // doc comment). This test is what exercises that guard: deliberately
    // malformed MIR no real pipeline produces, same convention as this
    // file's other "internal error" tests (e.g.
    // `referencing_a_name_with_no_bound_local_is_an_internal_error`).
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::Assign {
            target: "x".to_string(),
            value: MirExpr::FString(vec![]),
        })],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("fstring_zero_parts_panics")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("fstring_zero_parts_panics.o");
    let _ = compile_to_object(&mir, &obj_path, None, false);
}

#[test]
fn printing_a_mistyped_compare_expression_prints_the_actual_runtime_value() {
    // Deliberately malformed MIR: `pycc_mir::build` always lowers
    // `Compare` with `ty: Ty::Bool` (see `pycc_mir`'s own
    // `builds_a_compare_expression_with_bool_type` test) -- no real
    // pipeline could ever produce `ty: Ty::Int` here.
    //
    // Before Task 10, `emit_stmt`'s `print` arm dispatched on this
    // (lied-about) declared `ty` field -- a `Ty::Int`-guarded arm then
    // pattern-matched the actual `Scalar` back out with a `let
    // Scalar::Int(v) = ... else { unreachable!(...) }`, which this test
    // used to prove panics for a mismatched `ty`. Task 10's fully
    // general dispatch removed that per-argument `ty`-based branch
    // entirely: it only ever inspects `arg.ty()` to tell a `None`-typed
    // argument apart from every other one (see that arm's own doc
    // comment), and then hands whatever `Scalar` `emit_expr` actually
    // produced straight to `to_str`, which matches on the real `Scalar`
    // variant, never the caller-declared `ty`. So this exact
    // mismatched-`ty` shape can no longer desync from reality -- it
    // just prints the real `Scalar::Bool` value `Compare` always
    // produces (`1 < 2` is `True`), regardless of what `ty` claims.
    // Kept (renamed, no longer `#[should_panic]`) as a regression test
    // documenting this behavior change rather than being deleted
    // outright, same rationale as `a_none_typed_call_result_used_as_a_
    // nested_expression_no_longer_panics` above.
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
            callee: "print".to_string(),
            args: vec![MirExpr::Compare {
                op: CmpOpKind::Lt,
                left: Box::new(MirExpr::IntLiteral(1)),
                right: Box::new(MirExpr::IntLiteral(2)),
                ty: Ty::Int,
            }],
            ty: Ty::None,
        }))],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("print_mistyped_compare_prints_actual_value")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("print_mistyped_compare_prints_actual_value.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("print_mistyped_compare_prints_actual_value");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(pycc_scratch::lf(&output.stdout), b"True\n");
}

#[test]
fn compiles_a_loop_whose_accumulator_overflows_into_a_bigint() {
    // `i = 0; acc = 4611686018427387903; while i < 3: acc = acc + acc; i = i + 1`
    // `print(acc)` -- starts at `i64::MAX >> 1` and doubles 3 times,
    // overflowing well past `i64::MAX` partway through; must print the
    // exact mathematical result via real bigint arithmetic, not a
    // wrapped/truncated one.
    let start: i64 = i64::MAX >> 1;
    let expected = (start as i128) * 8; // doubled 3 times
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "acc".to_string(),
                value: MirExpr::IntLiteral(start),
            }),
            MirItem::TopLevelStmt(MirStmt::ForRange {
                var: "i".to_string(),
                start: MirExpr::IntLiteral(0),
                stop: MirExpr::IntLiteral(3),
                step: MirExpr::IntLiteral(1),
                body: vec![MirStmt::Assign {
                    target: "acc".to_string(),
                    value: MirExpr::BinOp {
                        op: BinOpKind::Add,
                        left: Box::new(MirExpr::Name {
                            name: "acc".to_string(),
                            ty: Ty::Int,
                        }),
                        right: Box::new(MirExpr::Name {
                            name: "acc".to_string(),
                            ty: Ty::Int,
                        }),
                        ty: Ty::Int,
                    },
                }],
            }),
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
                callee: "print".to_string(),
                args: vec![MirExpr::Name {
                    name: "acc".to_string(),
                    ty: Ty::Int,
                }],
                ty: Ty::None,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("bigint_overflow_loop")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("bigint_overflow_loop.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("bigint_overflow_loop");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(pycc_scratch::lf(&output.stdout), format!("{expected}\n").into_bytes());
}
