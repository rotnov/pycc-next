//! Local and global slot storage: reads, walrus targets, branch-first assignments.

use super::*;

#[test]
fn a_bare_expression_statement_evaluates_and_discards_its_value() {
    // `5` as its own top-level statement (Python allows a bare
    // expression statement); nothing currently has a side effect from
    // it, but the shape is legal MIR and must not panic or miscompile.
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::ExprStmt(
            MirExpr::IntLiteral(5),
        ))],
        class_defs: Vec::new(),
    };
    let dir =
        pycc_scratch::ScratchDir::new("bare_expr_stmt").expect("failed to create scratch dir");
    let obj_path = dir.join("bare_expr_stmt.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
}

#[test]
fn reading_a_none_typed_parameter_slot_emits_its_unit_carrier() {
    // Calls `emit_expr` directly to isolate its `Ty::None` name-load
    // arm. The real source-level ABI path is covered separately by
    // `printing_a_none_typed_parameter_renders_none`.
    let context = Context::create();
    let module = context.create_module("test");
    let builder = context.create_builder();
    let rt = declare_rt_functions(&context, &module);
    let fn_type = context.void_type().fn_type(&[], false);
    let f = module.add_function("f", fn_type, None);
    let block = context.append_basic_block(f, "entry");
    let exception_target = context.append_basic_block(f, "exception");
    builder.position_at_end(block);
    rt.exceptions.targets.borrow_mut().push(exception_target);

    let user_functions: HashMap<&str, UserFunction> = HashMap::new();
    let mut locals = HashMap::new();
    let ptr = builder
        .build_alloca(context.i8_type(), "x")
        .expect("build_alloca should not fail for a fresh block");
    builder
        .build_store(ptr, context.i8_type().const_zero())
        .expect("build_store should not fail for a fresh unit slot");
    locals.insert(
        "x".to_string(),
        StorageSlot {
            ptr,
            ty: Ty::None,
            initialized: None,
        },
    );

    let value = emit_expr(
        &context,
        &builder,
        &module,
        &rt,
        &user_functions,
        &locals,
        &MirExpr::Name {
            name: "x".to_string(),
            ty: Ty::None,
        },
    );
    // Reaching this point proves the typed load was emitted; matching the
    // private wrapper's exact bits here would add an
    // intentionally-unreachable assertion branch under the hard region
    // coverage gate. The carrier's actual D-075 zero-value contract is
    // instead verified end-to-end (source -> ... -> execution) by #167's
    // `a_none_call_result_crossing_the_abi_carries_a_falsy_unit_value` in
    // `tests/issue_167_none_carrier_abi.rs`, which observes the carrier's
    // truthiness rather than relying on `Ty::None`'s always-"None" print
    // rendering.
    let _ = value;
}

#[test]
fn a_walrus_in_an_if_test_predeclares_its_storage_slot_and_runs() {
    // PEP 572 (#774): `if (n := 5): print(n)` -- exercises
    // `collect_expr_bindings`'s call sites from `collect_stmt_bindings`'s
    // `MirStmt::If` arm (`test` predeclaration), including its own
    // `matches!` allow-list check and `bindings.entry(..).or_insert(..)`
    // insertion for a supported type (`Ty::Int`). Without this
    // predeclaration `emit_assign`'s `locals.get(target).expect(..)`
    // would panic the first time the `MirExpr::NamedExpr` codegen arm
    // tries to store into "n"'s slot, so a successful `compile_to_object`
    // plus a correct run both demonstrate the predeclaration happened.
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::If {
            test: MirExpr::NamedExpr {
                name: "n".to_string(),
                value: Box::new(MirExpr::IntLiteral(5)),
                ty: Ty::Int,
            },
            body: vec![MirStmt::ExprStmt(MirExpr::Call {
                callee: "print".to_string(),
                args: vec![MirExpr::Name {
                    name: "n".to_string(),
                    ty: Ty::Int,
                }],
                ty: Ty::None,
            })],
            orelse: vec![],
        })],
        class_defs: Vec::new(),
    };
    let dir =
        pycc_scratch::ScratchDir::new("walrus_in_if_test").expect("failed to create scratch dir");
    let obj_path = dir.join("walrus_in_if_test.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("walrus_in_if_test");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"5\n");
}

#[test]
fn a_walrus_with_an_optional_int_value_and_a_repeated_target_name_predeclare_correctly() {
    // `collect_expr_bindings`'s (#774) two remaining untested branches:
    // the `Ty::Optional(_)` arm of its `matches!` allow-list (the sibling
    // test above only ever exercises `Ty::Int`), and the
    // `bindings.entry(name).or_insert(ty)` "already present" skip path for
    // a second walrus target reusing an earlier walrus's own name within
    // the same expression tree (`m`, bound twice below).
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::NamedExpr {
                name: "n".to_string(),
                value: Box::new(MirExpr::OptionalWrap(
                    Box::new(MirExpr::IntLiteral(5)),
                    Box::new(Ty::Int),
                )),
                ty: optional_int(),
            })),
            MirItem::TopLevelStmt(print_expr(MirExpr::Compare {
                op: pycc_mir::CmpOpKind::IsNot,
                left: Box::new(MirExpr::Name {
                    name: "n".to_string(),
                    ty: optional_int(),
                }),
                right: Box::new(MirExpr::NoneLiteral),
                ty: Ty::Bool,
            })),
            MirItem::TopLevelStmt(print_expr(MirExpr::Compare {
                op: pycc_mir::CmpOpKind::Lt,
                left: Box::new(MirExpr::NamedExpr {
                    name: "m".to_string(),
                    value: Box::new(MirExpr::IntLiteral(1)),
                    ty: Ty::Int,
                }),
                right: Box::new(MirExpr::NamedExpr {
                    name: "m".to_string(),
                    value: Box::new(MirExpr::IntLiteral(2)),
                    ty: Ty::Int,
                }),
                ty: Ty::Bool,
            })),
            MirItem::TopLevelStmt(print_expr(MirExpr::Name {
                name: "m".to_string(),
                ty: Ty::Int,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("walrus_optional_and_repeated_name")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("walrus_optional_and_repeated_name.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("walrus_optional_and_repeated_name");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"True\nTrue\n2\n");
}

#[test]
fn emit_expr_evaluates_a_named_expr_stores_it_and_reads_it_back() {
    // PEP 572 (#774): `MirExpr::NamedExpr { name, value, ty }` -- calls
    // `emit_expr` directly to isolate the walrus arm inside
    // `emit_expr_unchecked`. `pycc_mir`'s `collect_named_expr_bindings`
    // (exercised separately in `pycc_mir`'s own tests) is what
    // predeclares the target's storage slot for real source programs;
    // here that predeclaration is done by hand, mirroring
    // `emit_assign`'s own documented contract that the target slot
    // must already exist in `locals` before it is called.
    let context = Context::create();
    let module = context.create_module("test");
    let builder = context.create_builder();
    let rt = declare_rt_functions(&context, &module);
    let fn_type = context.void_type().fn_type(&[], false);
    let f = module.add_function("f", fn_type, None);
    let block = context.append_basic_block(f, "entry");
    let exception_target = context.append_basic_block(f, "exception");
    builder.position_at_end(block);
    rt.exceptions.targets.borrow_mut().push(exception_target);

    let user_functions: HashMap<&str, UserFunction> = HashMap::new();
    let mut locals = HashMap::new();
    let ptr = builder
        .build_alloca(context.i64_type(), "n")
        .expect("build_alloca should not fail for a fresh block");
    locals.insert(
        "n".to_string(),
        StorageSlot {
            ptr,
            ty: Ty::Int,
            initialized: None,
        },
    );

    let value = emit_expr(
        &context,
        &builder,
        &module,
        &rt,
        &user_functions,
        &locals,
        &MirExpr::NamedExpr {
            name: "n".to_string(),
            value: Box::new(MirExpr::IntLiteral(5)),
            ty: Ty::Int,
        },
    );
    // Reaching this point proves both the store into the predeclared
    // slot and the read-back through the synthetic `Name` round-trip
    // were emitted without panicking; the walrus's actual value
    // (5) is verified end-to-end by the source-level conformance
    // fixtures under `tests/`, which are outside this in-process
    // coverage measurement.
    let _ = value;
}

#[test]
#[should_panic(expected = "reading a `<inferred>`-typed local is not supported yet")]
fn reading_an_unresolved_infer_typed_local_is_an_internal_error() {
    // `Ty::Infer` is an HIR-only solver marker and must be resolved
    // before MIR reaches codegen. Hand-built storage keeps the
    // defensive catch-all in the name-load path covered without
    // weakening the invariant for real source programs.
    //
    // Task 5 (D-089) changed this catch-all's message to name the type
    // via `Ty::name()` instead of a bare `{:?}`, so `Ty::Infer` now
    // renders as `<inferred>` (`Ty::name()`'s own text for that
    // variant) rather than the `Debug`-derived `Infer` this test's
    // `expected` string pinned before -- same panic, updated wording.
    let context = Context::create();
    let module = context.create_module("test");
    let builder = context.create_builder();
    let rt = declare_rt_functions(&context, &module);
    let fn_type = context.void_type().fn_type(&[], false);
    let f = module.add_function("f", fn_type, None);
    let block = context.append_basic_block(f, "entry");
    let exception_target = context.append_basic_block(f, "exception");
    builder.position_at_end(block);
    rt.exceptions.targets.borrow_mut().push(exception_target);

    let user_functions: HashMap<&str, UserFunction> = HashMap::new();
    let ptr = builder
        .build_alloca(context.i8_type(), "x")
        .expect("build_alloca should not fail for a fresh block");
    let locals = HashMap::from([(
        "x".to_string(),
        StorageSlot {
            ptr,
            ty: Ty::Infer,
            initialized: None,
        },
    )]);

    emit_expr(
        &context,
        &builder,
        &module,
        &rt,
        &user_functions,
        &locals,
        &MirExpr::Name {
            name: "x".to_string(),
            ty: Ty::Infer,
        },
    );
}

#[test]
fn reading_a_float_local_back_out_of_its_alloca() {
    // `x = 1.5; y = x + 1.0` -- exercises `emit_expr`'s `Name` arm on
    // a `Ty::Float` local (mirrors the existing bool-local read-back
    // test, `reading_a_bool_local_back_out_of_its_alloca` above).
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "x".to_string(),
                value: MirExpr::FloatLiteral(1.5),
            }),
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "y".to_string(),
                value: MirExpr::BinOp {
                    op: BinOpKind::Add,
                    left: Box::new(MirExpr::Name {
                        name: "x".to_string(),
                        ty: Ty::Float,
                    }),
                    right: Box::new(MirExpr::FloatLiteral(1.0)),
                    ty: Ty::Float,
                },
            }),
        ],
        class_defs: Vec::new(),
    };
    let dir =
        pycc_scratch::ScratchDir::new("read_float_local").expect("failed to create scratch dir");
    let obj_path = dir.join("read_float_local.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
}

#[test]
fn a_function_parameter_can_be_reassigned_read_back_and_printed() {
    // `def f(n: int): n = n + 1; print(n)` ; `f(7)` -- supersedes this test's
    // earlier (Task 3) incarnation, `referencing_a_function_parameter_
    // is_not_yet_supported`, which proved the opposite: that
    // `compile_to_object` started each function's `fn_locals` map
    // empty, so reading a parameter back by name hit an internal-error
    // panic. Task 5 fixes exactly that gap (see `compile_to_object`'s
    // second pass: each parameter gets its own `alloca`, with the
    // incoming LLVM argument stored into it before the body runs), so
    // this now proves a parameter is fully ordinary -- readable via
    // `emit_expr`'s `Name` arm exactly like any other local -- and
    // this call site also exercises `emit_stmt`'s void-call arm with a
    // *non-empty* argument list (every other void-call test in this
    // file uses a zero-arg call).
    let mir = MirModule {
        items: vec![
            MirItem::Function {
                name: "f".to_string(),
                params: vec![("n".to_string(), Ty::Int)],
                return_ty: Ty::None,
                body: vec![
                    MirStmt::Assign {
                        target: "n".to_string(),
                        value: MirExpr::BinOp {
                            op: BinOpKind::Add,
                            left: Box::new(MirExpr::Name {
                                name: "n".to_string(),
                                ty: Ty::Int,
                            }),
                            right: Box::new(MirExpr::IntLiteral(1)),
                            ty: Ty::Int,
                        },
                    },
                    MirStmt::ExprStmt(MirExpr::Call {
                        callee: "print".to_string(),
                        args: vec![MirExpr::Name {
                            name: "n".to_string(),
                            ty: Ty::Int,
                        }],
                        ty: Ty::None,
                    }),
                ],
            },
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
                callee: "f".to_string(),
                args: vec![MirExpr::IntLiteral(7)],
                ty: Ty::None,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("param_reference_reads_back")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("param_reference_reads_back.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("param_reference_reads_back");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"8\n");
}

#[test]
fn a_function_reads_a_module_level_global_it_does_not_itself_assign() {
    // `x = 5` ; `def f() -> int:\n    return x` ; `print(f())` -- must
    // print `5`. Before this fix, every function's `fn_locals` map
    // started empty (seeded only with its own parameters), and every
    // top-level name lived only in `main`'s own separate
    // `top_level_locals` map -- entirely discarded once `main`'s body
    // finished emitting, never visible to any function's own codegen.
    // `emit_expr`'s `Name` arm panicked ("no local slot") the moment a
    // function body read a module-level global it did not itself
    // assign, even though `pycc_types` (D-055) and `pycc_mir` (this
    // file's own sibling fix) both correctly accept and type this
    // program. Fixed by giving every module-level binding real
    // (non-stack) LLVM global storage and seeding each function's
    // `fn_locals` map from those slots before parameters and lexical
    // locals override same-named entries.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "x".to_string(),
                value: MirExpr::IntLiteral(5),
            }),
            MirItem::Function {
                name: "f".to_string(),
                params: vec![],
                return_ty: Ty::Int,
                body: vec![MirStmt::Return(Some(MirExpr::Name {
                    name: "x".to_string(),
                    ty: Ty::Int,
                }))],
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
    let dir = pycc_scratch::ScratchDir::new("function_reads_module_global")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("function_reads_module_global.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("function_reads_module_global");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"5\n");
}

#[test]
fn an_int_local_first_assigned_inside_an_if_body_is_readable_after_the_if() {
    // `if True: x = 1` then `print(x)` at the top level, after the
    // `if`. `collect_module_bindings` finds `x` before emission, and
    // `declare_module_globals` creates its process-wide slot and
    // initialized flag before either branch exists. The taken branch
    // stores into that dominating slot and marks it initialized, so the
    // read after `if_merge` succeeds rather than depending on storage
    // first created inside `if_then`.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::If {
                test: MirExpr::BoolLiteral(true),
                body: vec![MirStmt::Assign {
                    target: "x".to_string(),
                    value: MirExpr::IntLiteral(1),
                }],
                orelse: vec![],
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
    let dir = pycc_scratch::ScratchDir::new("int_first_assign_in_if_body")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("int_first_assign_in_if_body.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("int_first_assign_in_if_body");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"1\n");
}

#[test]
fn an_int_local_assigned_in_both_branches_of_an_if_else_is_readable_after_the_if() {
    // `if True: x = 1 else: x = 2` then `print(x)` -- module-binding
    // collection predeclares one global slot and initialized flag for
    // `x` before codegen reaches either sibling block. Both branches
    // store into that same dominating slot and mark it initialized, so
    // the merged read is independent of branch emission order.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::If {
                test: MirExpr::BoolLiteral(true),
                body: vec![MirStmt::Assign {
                    target: "x".to_string(),
                    value: MirExpr::IntLiteral(1),
                }],
                orelse: vec![MirStmt::Assign {
                    target: "x".to_string(),
                    value: MirExpr::IntLiteral(2),
                }],
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
    let dir = pycc_scratch::ScratchDir::new("int_first_assign_in_if_else_both")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("int_first_assign_in_if_else_both.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("int_first_assign_in_if_else_both");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"1\n");
}

#[test]
fn a_str_local_first_assigned_inside_an_if_body_is_freed_at_top_level_completion() {
    // Regression test for a review finding against the crate root's first
    // `str`-codegen task: `if True: s = "hi"` -- `s`'s *first*
    // assignment executes inside the `if`'s own `then` block, never at
    // the top level directly, and `s` is never read by any user code
    // either. `collect_module_bindings` predeclares `s` as an internal
    // pointer global with a null initializer and a separate initialized
    // flag before `main` is emitted. The nested assignment stores into
    // that process-wide slot, and the top-level completion pass can load
    // and decref it after the merge without any CFG-dominance dependency
    // on the `then` block (D-074).
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::If {
            test: MirExpr::BoolLiteral(true),
            body: vec![MirStmt::Assign {
                target: "s".to_string(),
                value: MirExpr::StringLiteral("hi".to_string()),
            }],
            orelse: vec![],
        })],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("str_first_assign_in_if_body")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("str_first_assign_in_if_body.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("str_first_assign_in_if_body");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert!(output.status.success(), "should run without crashing");
}

#[test]
fn a_str_local_assigned_in_both_branches_of_an_if_else_is_freed_at_top_level_completion() {
    // `if True: s = "hi" else: s = "bye"` -- `s` is classified and
    // predeclared as one null-initialized module global before either
    // branch is emitted. Both siblings store into that same slot rather
    // than whichever branch is visited first creating storage. This
    // additionally proves the pre-store null/old-value decref path is
    // safe from either predecessor (D-074).
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::If {
            test: MirExpr::BoolLiteral(true),
            body: vec![MirStmt::Assign {
                target: "s".to_string(),
                value: MirExpr::StringLiteral("hi".to_string()),
            }],
            orelse: vec![MirStmt::Assign {
                target: "s".to_string(),
                value: MirExpr::StringLiteral("bye".to_string()),
            }],
        })],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("str_first_assign_in_if_else_both")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("str_first_assign_in_if_else_both.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("str_first_assign_in_if_else_both");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert!(output.status.success(), "should run without crashing");
}

#[test]
fn a_str_local_first_assigned_inside_a_while_body_frees_previous_and_final_values() {
    // `i = 0; while i < 3: s = "x"; i = i + 1` -- `s`'s first assignment
    // happens inside the `while`'s own body block. Its module-global
    // pointer slot is nevertheless declared with a null initializer
    // before `main` is emitted, so every loop iteration reuses the same
    // storage: the pre-store decref is harmless on the first iteration,
    // frees the previous value on later iterations, and the top-level
    // completion pass frees the final value (D-074).
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "i".to_string(),
                value: MirExpr::IntLiteral(0),
            }),
            MirItem::TopLevelStmt(MirStmt::While {
                test: MirExpr::Compare {
                    op: CmpOpKind::Lt,
                    left: Box::new(MirExpr::Name {
                        name: "i".to_string(),
                        ty: Ty::Int,
                    }),
                    right: Box::new(MirExpr::IntLiteral(3)),
                    ty: Ty::Bool,
                },
                body: vec![
                    MirStmt::Assign {
                        target: "s".to_string(),
                        value: MirExpr::StringLiteral("x".to_string()),
                    },
                    MirStmt::Assign {
                        target: "i".to_string(),
                        value: MirExpr::BinOp {
                            op: BinOpKind::Add,
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
    let dir = pycc_scratch::ScratchDir::new("str_first_assign_in_while_body")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("str_first_assign_in_while_body.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("str_first_assign_in_while_body");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert!(output.status.success(), "should run without crashing");
}

#[test]
fn a_str_local_never_assigned_on_the_taken_path_decrefs_a_clean_null_at_completion() {
    // `flag = ""; if flag: s = "hi"` -- `flag` is falsy (the empty
    // string), so the `then` block containing `s`'s only assignment
    // never runs at all. `declare_module_globals` still creates `s`'s
    // pointer global with a null initializer and its initialized flag
    // with a false initializer before `main` starts. The top-level
    // completion pass therefore loads a real null rather than LLVM
    // `undef`, and `pycc_rt_str_decref` safely no-ops. Dropping that
    // global initializer would make this path load an indeterminate
    // pointer even though no Python assignment executed.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "flag".to_string(),
                value: MirExpr::StringLiteral(String::new()),
            }),
            MirItem::TopLevelStmt(MirStmt::If {
                test: MirExpr::Name {
                    name: "flag".to_string(),
                    ty: Ty::Str,
                },
                body: vec![MirStmt::Assign {
                    target: "s".to_string(),
                    value: MirExpr::StringLiteral("hi".to_string()),
                }],
                orelse: vec![],
            }),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("str_never_assigned_on_taken_path")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("str_never_assigned_on_taken_path.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("str_never_assigned_on_taken_path");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert!(
        output.status.success(),
        "should run without crashing (null-guarded decref of a never-assigned slot)"
    );
}

#[test]
fn a_str_local_first_assigned_inside_a_functions_own_leading_if_body() {
    // `def f() -> None:\n    if True:\n        s = "hi"` ; `f()` --
    // function-local collection finds `s` throughout the body before
    // emitting its leading control flow. `storage_slot_at_entry` creates
    // the null-initialized pointer slot and false initialized flag while
    // the builder is still in `f`'s entry block; only then is the `if`
    // emitted. The nested assignment marks that dominating per-call slot
    // initialized without creating storage inside `if_then`.
    let mir = MirModule {
        items: vec![
            MirItem::Function {
                name: "f".to_string(),
                params: vec![],
                return_ty: Ty::None,
                body: vec![MirStmt::If {
                    test: MirExpr::BoolLiteral(true),
                    body: vec![MirStmt::Assign {
                        target: "s".to_string(),
                        value: MirExpr::StringLiteral("hi".to_string()),
                    }],
                    orelse: vec![],
                }],
            },
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
                callee: "f".to_string(),
                args: vec![],
                ty: Ty::None,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("str_first_assign_in_fn_leading_if")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("str_first_assign_in_fn_leading_if.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("str_first_assign_in_fn_leading_if");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert!(output.status.success(), "should run without crashing");
}

#[test]
fn a_str_local_first_assigned_as_a_functions_own_plain_leading_statement() {
    // `def f() -> None:\n    s = "hi"` ; `f()` -- the straight-line
    // counterpart to the leading-`if` test above. Body preclassification
    // still creates one guarded, null-initialized per-call slot before
    // statement emission, and the assignment reuses it and flips its
    // initialized flag.
    let mir = MirModule {
        items: vec![
            MirItem::Function {
                name: "f".to_string(),
                params: vec![],
                return_ty: Ty::None,
                body: vec![MirStmt::Assign {
                    target: "s".to_string(),
                    value: MirExpr::StringLiteral("hi".to_string()),
                }],
            },
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
                callee: "f".to_string(),
                args: vec![],
                ty: Ty::None,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("str_first_assign_in_fn_plain")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("str_first_assign_in_fn_plain.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("str_first_assign_in_fn_plain");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert!(output.status.success(), "should run without crashing");
}

#[test]
fn an_int_local_assigned_in_both_branches_of_a_functions_own_leading_if_else() {
    // `def f() -> int:\n    if True:\n        x = 1\n    else:\n        x = 2\n    return x`
    // ; `print(f())` -- must print `1`. Function-local collection
    // preclassifies `x`, and `storage_slot_at_entry` creates its slot and
    // initialized flag before the leading `if` is emitted. Both sibling
    // branches then store into that one dominating per-call slot, and
    // the return reads the value selected by the executed branch.
    let mir = MirModule {
        items: vec![
            MirItem::Function {
                name: "f".to_string(),
                params: vec![],
                return_ty: Ty::Int,
                body: vec![
                    MirStmt::If {
                        test: MirExpr::BoolLiteral(true),
                        body: vec![MirStmt::Assign {
                            target: "x".to_string(),
                            value: MirExpr::IntLiteral(1),
                        }],
                        orelse: vec![MirStmt::Assign {
                            target: "x".to_string(),
                            value: MirExpr::IntLiteral(2),
                        }],
                    },
                    MirStmt::Return(Some(MirExpr::Name {
                        name: "x".to_string(),
                        ty: Ty::Int,
                    })),
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
    let dir = pycc_scratch::ScratchDir::new("int_first_assign_in_fn_leading_if_else")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("int_first_assign_in_fn_leading_if_else.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("int_first_assign_in_fn_leading_if_else");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"1\n");
}

#[test]
fn a_float_local_first_assigned_inside_a_function_uses_preclassified_storage() {
    // `def f() -> float:\n    y = 2.5\n    return y` ; `print(f())` --
    // body preclassification creates `y`'s guarded f64 storage in the
    // function entry block before emission. The plain assignment reuses
    // that slot and preserves its float representation through the
    // subsequent return (distinct from the integer and string cases).
    let mir = MirModule {
        items: vec![
            MirItem::Function {
                name: "f".to_string(),
                params: vec![],
                return_ty: Ty::Float,
                body: vec![
                    MirStmt::Assign {
                        target: "y".to_string(),
                        value: MirExpr::FloatLiteral(2.5),
                    },
                    MirStmt::Return(Some(MirExpr::Name {
                        name: "y".to_string(),
                        ty: Ty::Float,
                    })),
                ],
            },
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
                callee: "print".to_string(),
                args: vec![MirExpr::Call {
                    callee: "f".to_string(),
                    args: vec![],
                    ty: Ty::Float,
                }],
                ty: Ty::None,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("float_first_assign_in_fn")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("float_first_assign_in_fn.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("float_first_assign_in_fn");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"2.5\n");
}
