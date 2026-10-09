//! Unit tests for the codegen crate root.
//!
//! Relocated verbatim from `lib.rs`'s inline `#[cfg(test)] mod tests` under
//! AGENTS.md's decomposability rule (#545 Part 1). As a child module it still
//! sees the crate root's private items directly, so nothing needed widened
//! visibility; only the module's own location changed.

use super::*;
use pycc_mir::{
    BinOpKind, CmpOpKind, CompSource, InstantiateExpr, MirExceptHandler, MirExceptionValue,
    MirExpr, MirFStringPart, MirItem, MirModule, MirStmt, Ty,
};
use std::process::Command;

// #619 / AGENTS.md decomposability rule: the D-029 mechanical guard's
// checking logic and its automated synthetic-source tests live in their
// own cohesion-driven submodule instead of growing this already
// far-over-threshold file further. See that module's own doc comment.
mod d029_guard;

// Part 2 of #1175 (#1179): the buffer sub-range egress's own tests, in a
// cohesion-driven submodule for the same reason `d029_guard` has one.
mod buffer_slice_egress;

// #1212: chained-comparison codegen, in its own submodule for the same
// reason.
mod compare_chain;

// #1254: the shared entry-block placement helper.
mod entry_block;

// #1298: a caught exception's message ownership (print/f-string renderings
// and borrowed raise messages).
mod exception_message_rc;

// #1325: binding a CPython object value to a module-level name.
mod object_binding;

// Part 1 of #1333: passing a CPython object to a pycc function.
mod object_argument;
// #1418: CPython's `NotImplemented` singleton as a return value.
mod not_implemented;
mod object_compare;
// Part 1 of #1255: a list or set comprehension over a CPython object.
mod object_comprehension;
// Part 2d of #1371: a list display built as a CPython `list`.
mod object_list_display;
// #1457: storing and deleting an attribute of a CPython object.
mod object_attr_store;
// #1475: boxing a native value into an `object` slot.
mod object_box;
mod object_unbox;
// Part 2b of #1371: membership in, and slices of, a CPython object.
mod object_membership_slice;
mod object_raise;
mod object_return;
// Part 1 of #891: the run-time unpack of a CPython object.
mod object_unpack;
// Part 6 of #1371: `and`/`or` with a CPython object operand.
mod object_bool_op;
// Part 10 of #1371: `not` on a CPython object.
mod object_not;
// Part 11 of #1371: `type(o)` on a CPython object.
mod object_type;

/// `print(<n>)` as a `MirStmt` -- a convenience single-int-argument
/// shape reused by many of this file's older tests (`emit_stmt`'s
/// `print` dispatch itself now handles any number of arguments of any
/// v0.1 scalar type, plus `None` from direct user-function results,
/// D-075 parameter values, and assignment storage; see its own doc comment).
fn call_print(n: i64) -> MirStmt {
    MirStmt::ExprStmt(MirExpr::Call {
        callee: "print".to_string(),
        args: vec![MirExpr::IntLiteral(n)],
        ty: Ty::None,
    })
}

/// A zero-arg call to a user-defined function as a `MirStmt`.
fn call_user_fn(name: &str) -> MirStmt {
    MirStmt::ExprStmt(MirExpr::Call {
        callee: name.to_string(),
        args: vec![],
        ty: Ty::None,
    })
}

#[test]
fn a_none_typed_module_binding_gets_a_zero_unit_global_slot() {
    let context = Context::create();
    let module = context.create_module("none_global");
    let bindings = BTreeMap::from([("x".to_string(), Ty::None)]);
    let globals = declare_module_globals(&context, &module, &bindings);
    let slot = globals
        .get("x")
        .expect("declare_module_globals should bind `x`");
    assert_eq!(slot.ty, Ty::None);
    assert!(
        slot.initialized.is_some(),
        "zero is the None carrier, not proof that the binding was assigned"
    );
    let initializer = module
        .get_global("pyglobal_x")
        .expect("the global is named pyglobal_<name>")
        .get_initializer()
        .expect("declare_module_globals always sets an initializer")
        .into_int_value();
    assert_eq!(initializer.get_zero_extended_constant(), Some(0));
}

#[test]
#[should_panic(expected = "a `<inferred>`-typed module binding is not supported yet")]
fn an_infer_typed_module_binding_remains_an_internal_error() {
    let context = Context::create();
    let module = context.create_module("infer_global");
    let bindings = BTreeMap::from([("x".to_string(), Ty::Infer)]);
    let _ = declare_module_globals(&context, &module, &bindings);
}

#[test]
fn a_dict_typed_module_binding_gets_a_real_pointer_backed_global_slot() {
    // Superseded by PR-11 Task 5: this test used to assert that a
    // `dict[str, int]` module binding panicked here (mirroring the
    // `None`/`tuple` catch-all tests above) -- that was accurate for
    // Task 4's own scope, but D-123 names module scope as one of the
    // places a `dict[str, int]` value is expected to live, and every
    // one of this task's own CLI repro programs assigns `x = {...}` at
    // module scope. `declare_module_globals` now gives `Ty::Dict(_)` a
    // real arm (mirrors `Ty::List(_)`'s own arm exactly), so this test
    // instead proves that: a pointer-backed global slot with the
    // `initialized` guard flag every module global gets, not a panic.
    let context = Context::create();
    let module = context.create_module("dict_global");
    let bindings = BTreeMap::from([("x".to_string(), Ty::Dict(Box::new((Ty::Str, Ty::Int))))]);
    let globals = declare_module_globals(&context, &module, &bindings);
    let slot = globals
        .get("x")
        .expect("declare_module_globals should bind `x`");
    assert_eq!(slot.ty, Ty::Dict(Box::new((Ty::Str, Ty::Int))));
    assert!(slot.initialized.is_some());
}

#[test]
fn a_set_typed_module_binding_gets_a_real_pointer_backed_global_slot() {
    // The `set[int]` counterpart of `a_dict_typed_module_binding_gets_a_
    // real_pointer_backed_global_slot` above, for the identical reason
    // (PR-11 Task 9): `declare_module_globals` now gives `Ty::Set(_)` a
    // real arm (mirrors `Ty::List(_)`/`Ty::Dict(_)`'s own arms exactly),
    // so a `set[int]` module binding gets a pointer-backed global slot
    // with the `initialized` guard flag every module global gets, not a
    // panic.
    let context = Context::create();
    let module = context.create_module("set_global");
    let bindings = BTreeMap::from([("x".to_string(), Ty::Set(Box::new(Ty::Int)))]);
    let globals = declare_module_globals(&context, &module, &bindings);
    let slot = globals
        .get("x")
        .expect("declare_module_globals should bind `x`");
    assert_eq!(slot.ty, Ty::Set(Box::new(Ty::Int)));
    assert!(slot.initialized.is_some());
}

#[test]
fn defining_main_without_calling_it_produces_no_output() {
    // The regression test for a bug fixed before this file was split out
    // of `lib.rs` by #545, so `git log --follow` is needed to reach it:
    // a function definition alone must never run, regardless of its
    // name -- matches CPython exactly (confirmed empirically against
    // python3.14 on this exact source: zero bytes of stdout).
    let mir = MirModule {
        items: vec![MirItem::Function {
            name: "main".to_string(),
            params: vec![],
            return_ty: Ty::None,
            body: vec![call_print(42)],
        }],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("slice0_uncalled_main")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("slice0_uncalled_main.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("slice0_uncalled_main");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"");
}

#[test]
fn compiles_an_explicit_call_to_main_to_a_running_binary() {
    let mir = MirModule {
        items: vec![
            MirItem::Function {
                name: "main".to_string(),
                params: vec![],
                return_ty: Ty::None,
                body: vec![call_print(42)],
            },
            MirItem::TopLevelStmt(call_user_fn("main")),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("slice0").expect("failed to create scratch dir");
    let obj_path = dir.join("slice0.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("slice0");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"42\n");
}

#[test]
fn monomorphized_generic_function_dispatches_directly_without_fn_ptr_global() {
    // A monomorphized generic specialization (`0gen_...` name) has no
    // `fn_ptr_global` -- it dispatches directly through `direct_value`.
    // This test covers the `None` path of `if let Some(ref fn_ptr_global)`
    // in the top-level binding pass and the `direct_value` path in
    // `build_call_to_with_leading_args`.
    let mir = MirModule {
        items: vec![
            MirItem::Function {
                name: "0gen_identity__T_int".to_string(),
                params: vec![("x".to_string(), Ty::Int)],
                return_ty: Ty::Int,
                body: vec![MirStmt::Return(Some(MirExpr::Name {
                    name: "x".to_string(),
                    ty: Ty::Int,
                }))],
            },
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
                callee: "print".to_string(),
                args: vec![MirExpr::Call {
                    callee: "0gen_identity__T_int".to_string(),
                    args: vec![MirExpr::IntLiteral(7)],
                    ty: Ty::Int,
                }],
                ty: Ty::None,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("monomorphized_direct_dispatch")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("monomorphized_direct_dispatch.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("monomorphized_direct_dispatch");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"7\n");
}

#[test]
fn compiles_top_level_statement_with_no_main() {
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(call_print(42))],
        class_defs: Vec::new(),
    };
    let dir =
        pycc_scratch::ScratchDir::new("slice0_toplevel").expect("failed to create scratch dir");
    let obj_path = dir.join("slice0_toplevel.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("slice0_toplevel");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"42\n");
}

#[test]
fn a_no_op_statement_compiles_and_produces_no_observable_output() {
    // `MirStmt::NoOp` is produced only by lowering a value-less PEP 526
    // annotation (`y: int`, Task 5) -- it must compile to nothing at
    // all: no store, no allocation, and it must not swallow or block
    // the statements sequenced around it.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::NoOp),
            MirItem::TopLevelStmt(call_print(1)),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("slice0_no_op").expect("failed to create scratch dir");
    let obj_path = dir.join("slice0_no_op.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("slice0_no_op");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"1\n");
}

#[test]
fn top_level_statements_run_in_order_including_a_call_to_main() {
    // RUNTIME.md's ordering guarantee ("top-level code ... runs once
    // ... at process start") applies to top-level statements
    // themselves running in source order -- which now includes an
    // explicit call to a user function as just another top-level
    // statement, not a special auto-invoked case.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(call_print(1)),
            MirItem::Function {
                name: "main".to_string(),
                params: vec![],
                return_ty: Ty::None,
                body: vec![call_print(2)],
            },
            MirItem::TopLevelStmt(call_user_fn("main")),
        ],
        class_defs: Vec::new(),
    };
    let dir =
        pycc_scratch::ScratchDir::new("slice0_combined").expect("failed to create scratch dir");
    let obj_path = dir.join("slice0_combined.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("slice0_combined");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"1\n2\n");
}

#[test]
fn calling_an_undefined_function_at_top_level_is_rejected() {
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(call_user_fn("does_not_exist"))],
        class_defs: Vec::new(),
    };
    let dir =
        pycc_scratch::ScratchDir::new("slice0_undefined_fn").expect("failed to create scratch dir");
    let obj_path = dir.join("slice0_undefined_fn.o");
    let err = compile_to_object(&mir, &obj_path, None, false).expect_err("should be rejected");
    assert!(
        err.contains("does_not_exist"),
        "error should name the offending function: {err}"
    );
}

#[test]
fn calling_an_undefined_function_inside_a_function_body_is_rejected() {
    let mir = MirModule {
        items: vec![MirItem::Function {
            name: "main".to_string(),
            params: vec![],
            return_ty: Ty::None,
            body: vec![call_user_fn("also_does_not_exist")],
        }],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("slice0_undefined_fn_nested")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("slice0_undefined_fn_nested.o");
    let err = compile_to_object(&mir, &obj_path, None, false).expect_err("should be rejected");
    assert!(
        err.contains("also_does_not_exist"),
        "error should name the offending function: {err}"
    );
}

#[test]
fn a_function_can_be_defined_under_any_name_without_being_called() {
    // There is no longer a "must be named main" restriction: any
    // function name is legal to *define*; only calling one runs it.
    let mir = MirModule {
        items: vec![MirItem::Function {
            name: "helper".to_string(),
            params: vec![],
            return_ty: Ty::None,
            body: vec![],
        }],
        class_defs: Vec::new(),
    };
    let dir =
        pycc_scratch::ScratchDir::new("slice0_any_fn_name").expect("failed to create scratch dir");
    let obj_path = dir.join("slice0_any_fn_name.o");
    compile_to_object(&mir, &obj_path, None, false)
        .expect("defining a function under any name should succeed");
}

#[test]
fn write_to_file_failure_is_reported_as_an_error() {
    // A real, reachable failure mode (unlike the internal invariants
    // asserted via .expect() in compile_to_object): the output path's
    // parent directory doesn't exist. an_unknown_target_triple_is_a_
    // clean_error below covers this function's other genuine failure
    // mode, Target::from_triple.
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(call_print(42))],
        class_defs: Vec::new(),
    };
    let scratch = pycc_scratch::ScratchDir::new("codegen_test_nonexistent_dir")
        .expect("failed to create scratch dir");
    let bad_path = scratch.join("does_not_exist").join("out.o");
    let err = compile_to_object(&mir, &bad_path, None, false)
        .expect_err("should fail: parent dir doesn't exist");
    assert!(!err.is_empty());
}

/// A hand-built compute-heavy loop whose final `print(float)` references
/// `pycc_rt_float_to_str`, while nothing references `pycc_rt_int_sub`.
/// Debug codegen preserves both declarations; the release pipeline must
/// retain the used declaration and remove the unused one.
fn release_flag_fixture() -> MirModule {
    MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "x".to_string(),
                value: MirExpr::FloatLiteral(1.0),
            }),
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
                    right: Box::new(MirExpr::IntLiteral(1000)),
                    ty: Ty::Bool,
                },
                body: vec![
                    MirStmt::Assign {
                        target: "x".to_string(),
                        value: MirExpr::BinOp {
                            op: BinOpKind::Mul,
                            left: Box::new(MirExpr::Name {
                                name: "x".to_string(),
                                ty: Ty::Float,
                            }),
                            right: Box::new(MirExpr::FloatLiteral(1.0000001)),
                            ty: Ty::Float,
                        },
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
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
                callee: "print".to_string(),
                args: vec![MirExpr::Name {
                    name: "x".to_string(),
                    ty: Ty::Float,
                }],
                ty: Ty::None,
            })),
        ],
        class_defs: Vec::new(),
    }
}

#[test]
fn release_mode_actually_runs_llvm_optimization_passes() {
    let mir = release_flag_fixture();

    let debug_dir =
        pycc_scratch::ScratchDir::new("release_flag_debug").expect("failed to create scratch dir");
    let debug_obj_path = debug_dir.join("release_flag_debug.o");
    let mut debug_observations = Vec::new();
    let mut debug_observer = |module: &inkwell::module::Module<'_>, applied_pipeline| {
        debug_observations.push((
            applied_pipeline,
            module.get_function("pycc_rt_float_to_str").is_some(),
            module.get_function("pycc_rt_int_sub").is_some(),
        ));
    };
    compile_to_object_with_observer(
        &mir,
        &debug_obj_path,
        &CompileOptions::default(),
        Some(&mut debug_observer),
    )
    .expect("debug codegen should succeed");

    let release_dir = pycc_scratch::ScratchDir::new("release_flag_release")
        .expect("failed to create scratch dir");
    let release_obj_path = release_dir.join("release_flag_release.o");
    let mut release_observations = Vec::new();
    let mut release_observer = |module: &inkwell::module::Module<'_>, applied_pipeline| {
        release_observations.push((
            applied_pipeline,
            module.get_function("pycc_rt_float_to_str").is_some(),
            module.get_function("pycc_rt_int_sub").is_some(),
        ));
    };
    compile_to_object_with_observer(
        &mir,
        &release_obj_path,
        &CompileOptions {
            release: true,
            ..CompileOptions::default()
        },
        Some(&mut release_observer),
    )
    .expect("release codegen should succeed");

    assert_eq!(debug_observations, vec![(None, true, true)]);
    assert_eq!(
        release_observations,
        vec![(Some("default<O3>"), true, false)]
    );
}

#[test]
fn cross_compiles_object_code_for_a_different_target_triple() {
    // This host is aarch64-apple-darwin; request the other macOS Tier-1
    // architecture. LLVM's codegen backend is inherently multi-target,
    // so this only needs Target::initialize_all (see the target_machine
    // module) plus the requested triple -- verified by checking the emitted
    // object file's actual architecture, not just that codegen didn't
    // error.
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(call_print(42))],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("cross_x64").expect("failed to create scratch dir");
    let obj_path = dir.join("cross_x64.o");
    compile_to_object(&mir, &obj_path, Some("x86_64-apple-darwin"), false)
        .expect("cross-compiling to a different Tier-1 target should succeed");

    assert!(
        object_file_cpu_type_is_x86_64(&obj_path),
        "expected a Mach-O object file with cputype x86_64"
    );
}

/// Reads the Mach-O header directly instead of shelling out to the
/// `file` utility, which this test used to do: fragile on Windows,
/// where `file` isn't a standard tool and only worked because Git's
/// bundled `usr/bin/file.exe` happened to be on `PATH` there -- an
/// environment coincidence, not a guarantee. This test only ever
/// emits Mach-O (`--target x86_64-apple-darwin`), so a full
/// multi-format parser isn't needed -- just enough of
/// `mach_header_64`'s fixed layout (magic, then cputype, both
/// little-endian on every Tier-1 target this project builds for) to
/// assert the emitted object's architecture is genuinely x86_64, not
/// a copy-paste no-op.
fn object_file_cpu_type_is_x86_64(path: &std::path::Path) -> bool {
    const MH_MAGIC_64: [u8; 4] = 0xfeed_facf_u32.to_le_bytes();
    const CPU_TYPE_X86_64: [u8; 4] = 0x0100_0007_u32.to_le_bytes();
    let bytes = std::fs::read(path).expect("object file should be readable");
    bytes.len() >= 8 && bytes[0..4] == MH_MAGIC_64 && bytes[4..8] == CPU_TYPE_X86_64
}

#[test]
fn an_unknown_target_triple_is_a_clean_error() {
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(call_print(42))],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("bad_triple").expect("failed to create scratch dir");
    let obj_path = dir.join("bad_triple.o");
    let err = compile_to_object(&mir, &obj_path, Some("not-a-real-target-triple"), false)
        .expect_err("an unrecognized target triple should be rejected");
    assert!(!err.is_empty());
}

#[test]
fn target_initialization_happens_once_across_concurrent_compiles() {
    // #628: LLVM's target registry is process-global mutable state, and
    // inkwell's create_target_machine reads it without taking the lock it
    // holds while initialize_all writes it. The crate's OnceLock guard is
    // what makes concurrent compilation safe, so this test binds on the
    // guard's own observable effect -- the initializer body ran exactly
    // once -- rather than merely on every thread returning Ok, which would
    // pass identically with the guard removed.
    //
    // The count is checked for equality with 1, not for "no growth across
    // this test": every other test in the harness also compiles, so any
    // interleaving still leaves the process-wide total at exactly 1. That
    // makes the assertion independent of test ordering and of whether the
    // harness runs single- or multi-threaded (D-029's Windows job uses
    // --test-threads=1; the four threads below are this test's own and are
    // unaffected by that flag). No sleeps, barriers or timing assumptions
    // are involved.
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(call_print(42))],
        class_defs: Vec::new(),
    };
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..4)
            .map(|index| {
                let mir = &mir;
                scope.spawn(move || {
                    // A scratch directory per thread: concurrent compiles
                    // must not share an output path.
                    let dir = pycc_scratch::ScratchDir::new("concurrent_target_init")
                        .expect("failed to create scratch dir");
                    let obj_path = dir.join(format!("concurrent_{index}.o"));
                    compile_to_object(mir, &obj_path, None, false)
                        .expect("concurrent compilation to an object file should succeed");
                })
            })
            .collect();
        for handle in handles {
            handle
                .join()
                .expect("no concurrent compilation thread should panic");
        }
    });

    assert_eq!(
        crate::target_machine::target_init_count(),
        1,
        "LLVM's target registry must be initialized exactly once per process"
    );
}

#[test]
fn compiles_a_zero_argument_print_producing_just_a_newline() {
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
            callee: "print".to_string(),
            args: vec![],
            ty: Ty::None,
        }))],
        class_defs: Vec::new(),
    };
    let dir =
        pycc_scratch::ScratchDir::new("print_zero_args").expect("failed to create scratch dir");
    let obj_path = dir.join("print_zero_args.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("print_zero_args");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"\n");
}

#[test]
fn compiles_a_multi_argument_print_with_mixed_types_space_separated() {
    // `print(1, 2.5, True, "hi")` -- prints `1 2.5 True hi\n`.
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
            callee: "print".to_string(),
            args: vec![
                MirExpr::IntLiteral(1),
                MirExpr::FloatLiteral(2.5),
                MirExpr::BoolLiteral(true),
                MirExpr::StringLiteral("hi".to_string()),
            ],
            ty: Ty::None,
        }))],
        class_defs: Vec::new(),
    };
    let dir =
        pycc_scratch::ScratchDir::new("print_mixed_multi").expect("failed to create scratch dir");
    let obj_path = dir.join("print_mixed_multi.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("print_mixed_multi");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"1 2.5 True hi\n");
}

#[test]
fn compiles_print_of_a_bool_false() {
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
            callee: "print".to_string(),
            args: vec![MirExpr::BoolLiteral(false)],
            ty: Ty::None,
        }))],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("print_false").expect("failed to create scratch dir");
    let obj_path = dir.join("print_false.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("print_false");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"False\n");
}

#[test]
fn compiles_print_of_a_void_returning_call_as_none() {
    // `def f() -> None: return` ; `print(f())` -- prints `None`.
    let mir = MirModule {
        items: vec![
            MirItem::Function {
                name: "f".to_string(),
                params: vec![],
                return_ty: Ty::None,
                body: vec![MirStmt::Return(None)],
            },
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
                callee: "print".to_string(),
                args: vec![MirExpr::Call {
                    callee: "f".to_string(),
                    args: vec![],
                    ty: Ty::None,
                }],
                ty: Ty::None,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("print_none_from_call")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("print_none_from_call.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("print_none_from_call");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"None\n");
}

#[test]
fn compiles_print_evaluating_all_args_before_output_with_a_side_effecting_call() {
    // #145: `def side_effect() -> int: print(2); return 3` ;
    // `print(1, side_effect())` -- the later argument's side effect
    // (printing `2`) must complete before the outer `print`'s own
    // output begins, so stdout is `2\n1 3\n` (CPython's order), not
    // the interleaved `1 2\n3\n` the old single-phase emit produced.
    let mir = MirModule {
        items: vec![
            MirItem::Function {
                name: "side_effect".to_string(),
                params: vec![],
                return_ty: Ty::Int,
                body: vec![
                    MirStmt::ExprStmt(MirExpr::Call {
                        callee: "print".to_string(),
                        args: vec![MirExpr::IntLiteral(2)],
                        ty: Ty::None,
                    }),
                    MirStmt::Return(Some(MirExpr::IntLiteral(3))),
                ],
            },
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
                callee: "print".to_string(),
                args: vec![
                    MirExpr::IntLiteral(1),
                    MirExpr::Call {
                        callee: "side_effect".to_string(),
                        args: vec![],
                        ty: Ty::Int,
                    },
                ],
                ty: Ty::None,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("print_eval_order_side_effect")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("print_eval_order_side_effect.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("print_eval_order_side_effect");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"2\n1 3\n");
}

#[test]
fn compiles_print_with_a_failing_later_argument_produces_no_partial_output() {
    // #145/#382: `def fail_later() -> int: return 1 // 0` ;
    // `print(1, fail_later())` -- the later argument's zero-divisor
    // now sets the pending exception state (D-173, #382) instead of
    // panicking. The function returns a neutral 0, `print` evaluates
    // all args before output, and the top-level exception check after
    // the `print` statement detects the active exception and exits
    // with non-zero. stdout may contain partial output because the
    // process no longer aborts mid-statement — the exception is
    // caught at the next top-level check point.
    let mir = MirModule {
        items: vec![
            MirItem::Function {
                name: "fail_later".to_string(),
                params: vec![],
                return_ty: Ty::Int,
                body: vec![MirStmt::Return(Some(MirExpr::BinOp {
                    op: BinOpKind::FloorDiv,
                    left: Box::new(MirExpr::IntLiteral(1)),
                    right: Box::new(MirExpr::IntLiteral(0)),
                    ty: Ty::Int,
                }))],
            },
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
                callee: "print".to_string(),
                args: vec![
                    MirExpr::IntLiteral(1),
                    MirExpr::Call {
                        callee: "fail_later".to_string(),
                        args: vec![],
                        ty: Ty::Int,
                    },
                ],
                ty: Ty::None,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("print_eval_order_fail_later")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("print_eval_order_fail_later.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("print_eval_order_fail_later");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    // #382: The process exits with non-zero because the top-level
    // exception check catches the ZeroDivisionError set by `1 // 0`.
    assert!(
        !output.status.success(),
        "process should exit non-zero on zero-divisor exception: {:?}",
        output.status
    );
}

#[test]
fn returning_a_void_returning_call_result_from_a_none_returning_function_runs_the_callee_and_returns_void()
 {
    // `def helper() -> None: return` ; `def outer() -> None: return
    // helper()` ; `outer()` -- must not build `ret i8 0` inside
    // `outer`'s `void` LLVM signature (previously failed `verify_module`
    // with "Found return instr that returns non-void in Function of
    // void return type!").
    let mir = MirModule {
        items: vec![
            MirItem::Function {
                name: "helper".to_string(),
                params: vec![],
                return_ty: Ty::None,
                body: vec![MirStmt::Return(None)],
            },
            MirItem::Function {
                name: "outer".to_string(),
                params: vec![],
                return_ty: Ty::None,
                body: vec![MirStmt::Return(Some(MirExpr::Call {
                    callee: "helper".to_string(),
                    args: vec![],
                    ty: Ty::None,
                }))],
            },
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
                callee: "outer".to_string(),
                args: vec![],
                ty: Ty::None,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir =
        pycc_scratch::ScratchDir::new("return_none_call").expect("failed to create scratch dir");
    let obj_path = dir.join("return_none_call.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("return_none_call");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert!(output.status.success());
    assert_eq!(output.stdout, b"");
}

#[test]
fn printing_a_none_typed_parameter_renders_none() {
    // `def source() -> None: return`; `def sink(y: None) -> None:
    // print(y)`; `sink(source())` -- exercises the canonical unit
    // carrier as a call argument, parameter slot, name read, and print
    // input without ever confusing it for the physically-identical
    // `False` carrier.
    let mir = MirModule {
        items: vec![
            MirItem::Function {
                name: "source".to_string(),
                params: vec![],
                return_ty: Ty::None,
                body: vec![MirStmt::Return(None)],
            },
            MirItem::Function {
                name: "sink".to_string(),
                params: vec![("y".to_string(), Ty::None)],
                return_ty: Ty::None,
                body: vec![MirStmt::ExprStmt(MirExpr::Call {
                    callee: "print".to_string(),
                    args: vec![MirExpr::Name {
                        name: "y".to_string(),
                        ty: Ty::None,
                    }],
                    ty: Ty::None,
                })],
            },
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
                callee: "sink".to_string(),
                args: vec![MirExpr::Call {
                    callee: "source".to_string(),
                    args: vec![],
                    ty: Ty::None,
                }],
                ty: Ty::None,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("print_none_typed_parameter")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("print_none_typed_parameter.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("print_none_typed_parameter");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"None\n");
}

#[test]
fn a_bare_return_with_no_value_exits_a_none_returning_function_early() {
    // `def f() -> None:\n    return\n    print(999)` ; `f(); print(1)`
    // -- supersedes this test's earlier (Task 3/4) incarnation,
    // `a_return_statement_is_not_yet_supported_by_codegen`, which
    // proved `Return` had no codegen at all yet (via `emit_stmt`'s
    // then-catch-all, since removed -- the match is now exhaustive
    // over `MirStmt`, see Task 5's own doc comment on `emit_stmt`).
    // Now that `Return` is fully implemented, this instead exercises
    // its `None` arm (a bare `return`, as opposed to `return <expr>`,
    // which the two dedicated function-call tests above already
    // cover) and proves `emit_body`'s terminator-safety early-stop
    // (re-added by this task, see its own doc comment) really does
    // skip the unreachable `print(999)` after the `return`, rather
    // than trying to emit into an already-terminated block. Only "1"
    // should print.
    let mir = MirModule {
        items: vec![
            MirItem::Function {
                name: "f".to_string(),
                params: vec![],
                return_ty: Ty::None,
                body: vec![
                    MirStmt::Return(None),
                    MirStmt::ExprStmt(MirExpr::Call {
                        callee: "print".to_string(),
                        args: vec![MirExpr::IntLiteral(999)],
                        ty: Ty::None,
                    }),
                ],
            },
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
                callee: "f".to_string(),
                args: vec![],
                ty: Ty::None,
            })),
            MirItem::TopLevelStmt(call_print(1)),
        ],
        class_defs: Vec::new(),
    };
    let dir =
        pycc_scratch::ScratchDir::new("bare_return_none").expect("failed to create scratch dir");
    let obj_path = dir.join("bare_return_none.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("bare_return_none");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"1\n");
}

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
    assert_eq!(output.stdout, b"3\n");
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
    assert_eq!(output.stdout, b"2\n");
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
    assert_eq!(output.stdout, b"7\n7\n42\n1\n32\n");
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
    assert_eq!(output.stdout, b"True\nFalse\nFalse\nTrue\n");
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
    assert_eq!(output.stdout, b"True\nnan_is_truthy\n");
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
    assert_eq!(output.stdout, b"2\n");
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
    assert_eq!(output.stdout, b"2\n");
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
    assert_eq!(output.stdout, b"True\n");
}

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
fn compiles_an_if_else_choosing_the_correct_branch_at_runtime() {
    // `x = 1; if x < 2: print(10) else: print(20)` -- must print 10.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "x".to_string(),
                value: MirExpr::IntLiteral(1),
            }),
            MirItem::TopLevelStmt(MirStmt::If {
                test: MirExpr::Compare {
                    op: CmpOpKind::Lt,
                    left: Box::new(MirExpr::Name {
                        name: "x".to_string(),
                        ty: Ty::Int,
                    }),
                    right: Box::new(MirExpr::IntLiteral(2)),
                    ty: Ty::Bool,
                },
                body: vec![MirStmt::ExprStmt(MirExpr::Call {
                    callee: "print".to_string(),
                    args: vec![MirExpr::IntLiteral(10)],
                    ty: Ty::None,
                })],
                orelse: vec![MirStmt::ExprStmt(MirExpr::Call {
                    callee: "print".to_string(),
                    args: vec![MirExpr::IntLiteral(20)],
                    ty: Ty::None,
                })],
            }),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("if_else").expect("failed to create scratch dir");
    let obj_path = dir.join("if_else.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("if_else");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"10\n");
}

#[test]
fn an_if_whose_both_branches_return_terminates_its_unreachable_merge() {
    let mir = MirModule {
        items: vec![
            MirItem::Function {
                name: "choose".to_string(),
                params: vec![("flag".to_string(), Ty::Bool)],
                return_ty: Ty::Int,
                body: vec![MirStmt::If {
                    test: MirExpr::Name {
                        name: "flag".to_string(),
                        ty: Ty::Bool,
                    },
                    body: vec![MirStmt::Return(Some(MirExpr::IntLiteral(1)))],
                    orelse: vec![MirStmt::Return(Some(MirExpr::IntLiteral(2)))],
                }],
            },
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
                callee: "print".to_string(),
                args: vec![MirExpr::Call {
                    callee: "choose".to_string(),
                    args: vec![MirExpr::BoolLiteral(true)],
                    ty: Ty::Int,
                }],
                ty: Ty::None,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("if_both_branches_return")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("if_both_branches_return.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("if_both_branches_return");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"1\n");
}

#[test]
fn a_statically_unreachable_match_tail_terminates_its_function() {
    let mir = MirModule {
        items: vec![MirItem::Function {
            name: "exhaustive_match_tail".to_string(),
            params: vec![],
            return_ty: Ty::None,
            body: vec![MirStmt::Unreachable],
        }],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("statically_unreachable_match_tail")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("statically_unreachable_match_tail.o");
    compile_to_object(&mir, &obj_path, None, false)
        .expect("a statically unreachable match tail must produce valid LLVM IR");
}

#[test]
fn compiles_an_if_with_no_else_and_a_false_test_prints_nothing() {
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::If {
            test: MirExpr::BoolLiteral(false),
            body: vec![MirStmt::ExprStmt(MirExpr::Call {
                callee: "print".to_string(),
                args: vec![MirExpr::IntLiteral(1)],
                ty: Ty::None,
            })],
            orelse: vec![],
        })],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("if_no_else").expect("failed to create scratch dir");
    let obj_path = dir.join("if_no_else.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("if_no_else");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"");
}

#[test]
fn compiles_a_while_loop_that_counts_down() {
    // `i = 3; while i > 0: print(i); i = i - 1` -- prints 3, 2, 1.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "i".to_string(),
                value: MirExpr::IntLiteral(3),
            }),
            MirItem::TopLevelStmt(MirStmt::While {
                test: MirExpr::Compare {
                    op: CmpOpKind::Gt,
                    left: Box::new(MirExpr::Name {
                        name: "i".to_string(),
                        ty: Ty::Int,
                    }),
                    right: Box::new(MirExpr::IntLiteral(0)),
                    ty: Ty::Bool,
                },
                body: vec![
                    MirStmt::ExprStmt(MirExpr::Call {
                        callee: "print".to_string(),
                        args: vec![MirExpr::Name {
                            name: "i".to_string(),
                            ty: Ty::Int,
                        }],
                        ty: Ty::None,
                    }),
                    MirStmt::Assign {
                        target: "i".to_string(),
                        value: MirExpr::BinOp {
                            op: BinOpKind::Sub,
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
    let dir =
        pycc_scratch::ScratchDir::new("while_countdown").expect("failed to create scratch dir");
    let obj_path = dir.join("while_countdown.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("while_countdown");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"3\n2\n1\n");
}

#[test]
fn compiles_a_while_loop_using_a_bare_int_condition_via_truthy() {
    // `i = 3; while i: print(i); i = i - 1` -- prints 3, 2, 1, same
    // countdown as the test above, but the loop test is a plain
    // `int`-typed `Name` (not a `Compare`), so this is the only test
    // in this file exercising `truthy`'s `Scalar::Int` arm (every
    // other `If`/`While` test's condition is a `Compare`, which always
    // evaluates to `Scalar::Bool`) -- `pycc_rt_int_truthy` genuinely
    // gets called from generated code here, not just unit-tested
    // directly in `pycc_rt`.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "i".to_string(),
                value: MirExpr::IntLiteral(3),
            }),
            MirItem::TopLevelStmt(MirStmt::While {
                test: MirExpr::Name {
                    name: "i".to_string(),
                    ty: Ty::Int,
                },
                body: vec![
                    MirStmt::ExprStmt(MirExpr::Call {
                        callee: "print".to_string(),
                        args: vec![MirExpr::Name {
                            name: "i".to_string(),
                            ty: Ty::Int,
                        }],
                        ty: Ty::None,
                    }),
                    MirStmt::Assign {
                        target: "i".to_string(),
                        value: MirExpr::BinOp {
                            op: BinOpKind::Sub,
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
    let dir =
        pycc_scratch::ScratchDir::new("while_int_truthy").expect("failed to create scratch dir");
    let obj_path = dir.join("while_int_truthy.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("while_int_truthy");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"3\n2\n1\n");
}

#[test]
fn emit_expr_evaluates_not_over_a_non_literal_int_operand() {
    // `i = 0; print(not i)` then `j = 5; print(not j)` -- prints `True`
    // then `False`. #604 (Part 3 of #573) adds `MirExpr::Not` as its own
    // dedicated `emit_expr` arm rather than folding into an existing
    // `BinOp`/`Compare` arm (unlike `~`, which rewrites to `-x - 1` at the
    // MIR level and so never needs a codegen arm of its own). The D-014
    // region gate scores every compilation of this crate separately, and
    // this arm was previously exercised only through the `pycc` binary and
    // the `tests/issue_604_unary_not_invert.rs` integration suite, never
    // through this crate's own `cfg(test)` build -- so this test, built
    // directly against `emit_expr`/`compile_to_object` the same way every
    // other test in this file is, is what actually reaches it there.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "i".to_string(),
                value: MirExpr::IntLiteral(0),
            }),
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
                callee: "print".to_string(),
                args: vec![MirExpr::Not(Box::new(MirExpr::Name {
                    name: "i".to_string(),
                    ty: Ty::Int,
                }))],
                ty: Ty::None,
            })),
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "j".to_string(),
                value: MirExpr::IntLiteral(5),
            }),
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
                callee: "print".to_string(),
                args: vec![MirExpr::Not(Box::new(MirExpr::Name {
                    name: "j".to_string(),
                    ty: Ty::Int,
                }))],
                ty: Ty::None,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir =
        pycc_scratch::ScratchDir::new("not_over_int_name").expect("failed to create scratch dir");
    let obj_path = dir.join("not_over_int_name.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("not_over_int_name");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"True\nFalse\n");
}

#[test]
fn a_while_loop_body_that_always_returns_skips_its_own_trailing_branch() {
    // `def f() -> int:\n    while True:\n        return 1\n    return 2`
    // ; `print(f())` -- must print `1`. The trailing `return 2` is
    // unreachable dead code. Since #1370 a bare `while True: return 1`
    // with nothing after it is accepted source too (the checker's
    // `pycc_hir::block_always_returns` accepts it,
    // and `fallthrough::block_always_terminates` is the codegen
    // counterpart that ends the block with `unreachable`); this shape,
    // with an explicit trailing `return`, remains valid source as well.
    // Distinct region from every other `while` test in this file, all
    // of whose *loop bodies* fall through normally and so always take
    // `emit_body_then_branch`'s own trailing
    // `build_unconditional_branch(test_bb)` back to the loop test: here
    // the loop body's own `return` already terminates it, so that
    // helper's terminator check must skip building a second (invalid)
    // terminator on top of it.
    let mir = MirModule {
        items: vec![
            MirItem::Function {
                name: "f".to_string(),
                params: vec![],
                return_ty: Ty::Int,
                body: vec![
                    MirStmt::While {
                        test: MirExpr::BoolLiteral(true),
                        body: vec![MirStmt::Return(Some(MirExpr::IntLiteral(1)))],
                    },
                    MirStmt::Return(Some(MirExpr::IntLiteral(2))),
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
    let dir = pycc_scratch::ScratchDir::new("while_body_always_returns")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("while_body_always_returns.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("while_body_always_returns");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"1\n");
}

#[test]
fn compiles_a_for_range_loop_with_a_positive_step() {
    // `for i in range(0, 6, 2): print(i)` -- prints 0, 2, 4.
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::ForRange {
            var: "i".to_string(),
            start: MirExpr::IntLiteral(0),
            stop: MirExpr::IntLiteral(6),
            step: MirExpr::IntLiteral(2),
            body: vec![MirStmt::ExprStmt(MirExpr::Call {
                callee: "print".to_string(),
                args: vec![MirExpr::Name {
                    name: "i".to_string(),
                    ty: Ty::Int,
                }],
                ty: Ty::None,
            })],
        })],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("for_range_pos").expect("failed to create scratch dir");
    let obj_path = dir.join("for_range_pos.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("for_range_pos");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"0\n2\n4\n");
}

#[test]
fn a_second_top_level_for_range_loop_reusing_a_loop_variable_name_is_not_redeclared() {
    // `for i in range(0, 2, 1): print(i)` followed by a second, separate
    // `for i in range(0, 3, 1): print(i)` -- both loops share the same
    // module-level loop variable name `i`. Exercises
    // `collect_stmt_bindings`'s `ForRange` arm with an already-present
    // `BTreeMap` entry: the second occurrence must not re-declare or
    // change the type of its global slot.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::ForRange {
                var: "i".to_string(),
                start: MirExpr::IntLiteral(0),
                stop: MirExpr::IntLiteral(2),
                step: MirExpr::IntLiteral(1),
                body: vec![MirStmt::ExprStmt(MirExpr::Call {
                    callee: "print".to_string(),
                    args: vec![MirExpr::Name {
                        name: "i".to_string(),
                        ty: Ty::Int,
                    }],
                    ty: Ty::None,
                })],
            }),
            MirItem::TopLevelStmt(MirStmt::ForRange {
                var: "i".to_string(),
                start: MirExpr::IntLiteral(0),
                stop: MirExpr::IntLiteral(3),
                step: MirExpr::IntLiteral(1),
                body: vec![MirStmt::ExprStmt(MirExpr::Call {
                    callee: "print".to_string(),
                    args: vec![MirExpr::Name {
                        name: "i".to_string(),
                        ty: Ty::Int,
                    }],
                    ty: Ty::None,
                })],
            }),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("for_range_reused_loop_var")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("for_range_reused_loop_var.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("for_range_reused_loop_var");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"0\n1\n0\n1\n2\n");
}

#[test]
fn compiles_a_for_range_loop_with_a_negative_step() {
    // `for i in range(3, 0, -1): print(i)` -- prints 3, 2, 1.
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::ForRange {
            var: "i".to_string(),
            start: MirExpr::IntLiteral(3),
            stop: MirExpr::IntLiteral(0),
            step: MirExpr::IntLiteral(-1),
            body: vec![MirStmt::ExprStmt(MirExpr::Call {
                callee: "print".to_string(),
                args: vec![MirExpr::Name {
                    name: "i".to_string(),
                    ty: Ty::Int,
                }],
                ty: Ty::None,
            })],
        })],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("for_range_neg").expect("failed to create scratch dir");
    let obj_path = dir.join("for_range_neg.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("for_range_neg");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"3\n2\n1\n");
}

#[test]
#[should_panic(expected = "range() start did not evaluate to int")]
fn for_range_with_a_non_int_start_is_rejected() {
    // `bool` is accepted as an `int` subtype; float is not. Hand-built
    // malformed MIR exercises the defensive backend check directly.
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::ForRange {
            var: "i".to_string(),
            start: MirExpr::FloatLiteral(1.0),
            stop: MirExpr::IntLiteral(3),
            step: MirExpr::IntLiteral(1),
            body: vec![],
        })],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("for_range_bad_start_panics")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("for_range_bad_start_panics.o");
    let _ = compile_to_object(&mir, &obj_path, None, false);
}

#[test]
#[should_panic(expected = "range() stop did not evaluate to int")]
fn for_range_with_a_non_int_stop_is_rejected() {
    // Distinct region from the `start` case above.
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::ForRange {
            var: "i".to_string(),
            start: MirExpr::IntLiteral(0),
            stop: MirExpr::StringLiteral("3".to_string()),
            step: MirExpr::IntLiteral(1),
            body: vec![],
        })],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("for_range_bad_stop_panics")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("for_range_bad_stop_panics.o");
    let _ = compile_to_object(&mir, &obj_path, None, false);
}

#[test]
#[should_panic(expected = "range() step did not evaluate to int")]
fn for_range_with_a_non_int_step_is_rejected() {
    // Distinct region from the `start`/`stop` cases above.
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::ForRange {
            var: "i".to_string(),
            start: MirExpr::IntLiteral(0),
            stop: MirExpr::IntLiteral(3),
            step: MirExpr::FloatLiteral(1.0),
            body: vec![],
        })],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("for_range_bad_step_panics")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("for_range_bad_step_panics.o");
    let _ = compile_to_object(&mir, &obj_path, None, false);
}

#[test]
fn for_range_with_a_bool_start_stop_and_step_all_widen_to_int() {
    // `for i in range(True, 4, True): print(i)` -- `bool` is an `int`
    // subtype (`pycc_types::is_assignable`), and
    // `a_for_range_loop_accepts_bool_as_an_int_subtype` proves
    // `pycc_types` genuinely accepts a bool-typed `range()` argument for
    // any of its three positions, so this reaches codegen with
    // `Scalar::Bool` `start`/`stop`/`step` operands (`stop` is `4`, a
    // plain int literal, to keep this a short, checkable loop; `start`
    // and `step` are both `True` to exercise both of that arm's other
    // two match sites). Before this fix, each position's
    // `let Scalar::Int(..) = ... else { panic!(...) }` destructure
    // rejected any non-`Int` scalar outright, crashing the compiler on
    // this legitimate, accepted program instead of applying range's
    // numeric normalization from `True` to the ordinary tagged int `1`.
    // Identity-preserving int boundaries intentionally keep D-141's
    // encoded `True` marker instead.
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::ForRange {
            var: "i".to_string(),
            start: MirExpr::BoolLiteral(true),
            stop: MirExpr::IntLiteral(4),
            step: MirExpr::BoolLiteral(true),
            body: vec![MirStmt::ExprStmt(MirExpr::Call {
                callee: "print".to_string(),
                args: vec![MirExpr::Name {
                    name: "i".to_string(),
                    ty: Ty::Int,
                }],
                ty: Ty::None,
            })],
        })],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("for_range_bool_start_stop_step")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("for_range_bool_start_stop_step.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("for_range_bool_start_stop_step");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"1\n2\n3\n");
}

#[test]
fn for_range_with_a_bool_stop_widens_to_int() {
    // `for i in range(0, True, 1): print(i)` -- distinct region from
    // the `start`/`step` coverage above: exercises `stop`'s own
    // `range_operand_to_normalized_int`'s `Scalar::Bool` arm specifically.
    // `True` normalizes to the ordinary tagged int `1`, so this loop runs once.
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::ForRange {
            var: "i".to_string(),
            start: MirExpr::IntLiteral(0),
            stop: MirExpr::BoolLiteral(true),
            step: MirExpr::IntLiteral(1),
            body: vec![MirStmt::ExprStmt(MirExpr::Call {
                callee: "print".to_string(),
                args: vec![MirExpr::Name {
                    name: "i".to_string(),
                    ty: Ty::Int,
                }],
                ty: Ty::None,
            })],
        })],
        class_defs: Vec::new(),
    };
    let dir =
        pycc_scratch::ScratchDir::new("for_range_bool_stop").expect("failed to create scratch dir");
    let obj_path = dir.join("for_range_bool_stop.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("for_range_bool_stop");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"0\n");
}

#[test]
fn for_range_normalizes_int_typed_bool_markers_before_the_induction_phi() {
    // Unlike the literal-bool range tests above, both helpers return an
    // `i64`-carried `Ty::Int`: their bool results have already crossed
    // the return boundary and become D-141 markers before `ForRange`
    // sees them. All three operands therefore exercise
    // `range_operand_to_normalized_int`'s `Scalar::Int` path. The first
    // visible target must print ordinary integer `0`, not `False`.
    let mir = MirModule {
        items: vec![
            MirItem::Function {
                name: "false_int".to_string(),
                params: vec![],
                return_ty: Ty::Int,
                body: vec![MirStmt::Return(Some(MirExpr::BoolLiteral(false)))],
            },
            MirItem::Function {
                name: "true_int".to_string(),
                params: vec![],
                return_ty: Ty::Int,
                body: vec![MirStmt::Return(Some(MirExpr::BoolLiteral(true)))],
            },
            MirItem::TopLevelStmt(MirStmt::ForRange {
                var: "i".to_string(),
                start: MirExpr::Call {
                    callee: "false_int".to_string(),
                    args: vec![],
                    ty: Ty::Int,
                },
                stop: MirExpr::Call {
                    callee: "true_int".to_string(),
                    args: vec![],
                    ty: Ty::Int,
                },
                step: MirExpr::Call {
                    callee: "true_int".to_string(),
                    args: vec![],
                    ty: Ty::Int,
                },
                body: vec![MirStmt::ExprStmt(MirExpr::Call {
                    callee: "print".to_string(),
                    args: vec![MirExpr::Name {
                        name: "i".to_string(),
                        ty: Ty::Int,
                    }],
                    ty: Ty::None,
                })],
            }),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("for_range_encoded_bool_markers")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("for_range_encoded_bool_markers.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("for_range_encoded_bool_markers");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"0\n");
}

#[test]
fn compiles_nested_control_flow_with_a_statement_after_it_in_the_same_body() {
    // `for i in range(0, 3, 1): (if i == 1: print(100)); print(i)` --
    // exercises two things no other test in this file does: control
    // flow (`If`) nested inside other control flow (`ForRange`), and a
    // statement following a control-flow statement in the *same*
    // `body` list. Every other test's `If`/`While`/`ForRange` is the
    // last statement of its enclosing body -- so nothing else proves
    // that `emit_stmt`'s `If` arm correctly leaves the builder
    // positioned at `merge_bb` in a state where a *subsequent*
    // statement resumes into it correctly (right `locals`, right
    // block, no invalid IR from double-terminating or orphaning a
    // block) -- exactly the invariant `emit_body`'s own doc comment
    // relies on to justify never needing an early-terminator-stop
    // check in Task 4's scope. Expected: i=0 -> "0"; i=1 -> "100" then
    // "1"; i=2 -> "2".
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::ForRange {
            var: "i".to_string(),
            start: MirExpr::IntLiteral(0),
            stop: MirExpr::IntLiteral(3),
            step: MirExpr::IntLiteral(1),
            body: vec![
                MirStmt::If {
                    test: MirExpr::Compare {
                        op: CmpOpKind::Eq,
                        left: Box::new(MirExpr::Name {
                            name: "i".to_string(),
                            ty: Ty::Int,
                        }),
                        right: Box::new(MirExpr::IntLiteral(1)),
                        ty: Ty::Bool,
                    },
                    body: vec![MirStmt::ExprStmt(MirExpr::Call {
                        callee: "print".to_string(),
                        args: vec![MirExpr::IntLiteral(100)],
                        ty: Ty::None,
                    })],
                    orelse: vec![],
                },
                MirStmt::ExprStmt(MirExpr::Call {
                    callee: "print".to_string(),
                    args: vec![MirExpr::Name {
                        name: "i".to_string(),
                        ty: Ty::Int,
                    }],
                    ty: Ty::None,
                }),
            ],
        })],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("nested_control_flow_resume")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("nested_control_flow_resume.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("nested_control_flow_resume");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"0\n100\n1\n2\n");
}

// The four tests below are not in the brief's own Step 3 list -- added
// because `cargo llvm-cov`'s region coverage showed each of `If`'s
// `then`/`orelse` arms, `While`'s body, and `ForRange`'s body has its
// own distinct `?`-propagation region (one per `emit_body`/`emit_body_
// then_branch` call site inside `emit_stmt`, not shared across arms):
// every prior test's nested body only ever contains statements that
// succeed, so none of these four `?` operators had ever actually
// propagated an `Err`. Mirrors the existing top-level/function-body
// `calling_an_undefined_function_..._is_rejected` tests above, just
// with the undefined call nested one level deeper.
#[test]
fn calling_an_undefined_function_inside_an_if_then_body_is_rejected() {
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::If {
            test: MirExpr::BoolLiteral(true),
            body: vec![call_user_fn("does_not_exist_in_if_then")],
            orelse: vec![],
        })],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("if_then_undefined_fn")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("if_then_undefined_fn.o");
    let err = compile_to_object(&mir, &obj_path, None, false).expect_err("should be rejected");
    assert!(
        err.contains("does_not_exist_in_if_then"),
        "error should name the offending function: {err}"
    );
}

#[test]
fn calling_an_undefined_function_inside_an_if_orelse_body_is_rejected() {
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::If {
            test: MirExpr::BoolLiteral(false),
            body: vec![],
            orelse: vec![call_user_fn("does_not_exist_in_if_orelse")],
        })],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("if_orelse_undefined_fn")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("if_orelse_undefined_fn.o");
    let err = compile_to_object(&mir, &obj_path, None, false).expect_err("should be rejected");
    assert!(
        err.contains("does_not_exist_in_if_orelse"),
        "error should name the offending function: {err}"
    );
}

#[test]
fn calling_an_undefined_function_inside_a_while_body_is_rejected() {
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::While {
            test: MirExpr::BoolLiteral(true),
            body: vec![call_user_fn("does_not_exist_in_while")],
        })],
        class_defs: Vec::new(),
    };
    let dir =
        pycc_scratch::ScratchDir::new("while_undefined_fn").expect("failed to create scratch dir");
    let obj_path = dir.join("while_undefined_fn.o");
    let err = compile_to_object(&mir, &obj_path, None, false).expect_err("should be rejected");
    assert!(
        err.contains("does_not_exist_in_while"),
        "error should name the offending function: {err}"
    );
}

#[test]
fn calling_an_undefined_function_inside_a_for_range_body_is_rejected() {
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::ForRange {
            var: "i".to_string(),
            start: MirExpr::IntLiteral(0),
            stop: MirExpr::IntLiteral(3),
            step: MirExpr::IntLiteral(1),
            body: vec![call_user_fn("does_not_exist_in_for_range")],
        })],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("for_range_undefined_fn")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("for_range_undefined_fn.o");
    let err = compile_to_object(&mir, &obj_path, None, false).expect_err("should be rejected");
    assert!(
        err.contains("does_not_exist_in_for_range"),
        "error should name the offending function: {err}"
    );
}

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
    assert_eq!(output.stdout, b"5\n");
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
    assert_eq!(output.stdout, b"7\n");
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
    assert_eq!(output.stdout, b"120\n");
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
    assert_eq!(output.stdout, b"1\n");
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
    assert_eq!(output.stdout, b"None\n");
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

#[test]
fn ty_to_basic_type_gives_a_memoryview_a_pointer_representation() {
    // Part 1 of #1027. The one source shape that reaches this arm is a
    // `memoryview` parameter of a `pycc build --ext` export, which needs
    // CPython development headers and so lives in an `#[ignore]`d hosted
    // test that contributes no coverage. The property is header-free
    // though, and it is the one the whole call convention rests on: what
    // crosses the boundary is a pointer to the wrapper's own
    // `PyccExtBufferView`, not CPython's `Py_buffer`, so the emitted IR
    // depends on no CPython struct layout and the representation is the
    // same plain pointer `Ty::Str`/`Ty::Object` already get. Since Part 2
    // of #1175 (#1179) `ext_thunk_required` no longer answers `false` for
    // every such export -- a body that returns `b[start:stop]` carries
    // three out-pointers and so does need a thunk -- but that turns on the
    // *body*, never on this representation.
    let context = Context::create();
    assert!(ty_to_basic_type(&context, Ty::MemoryView).is_pointer_type());
}

#[test]
fn ty_to_basic_type_gives_an_opaque_object_a_pointer_representation() {
    // Part 1 of #1026. No *source* reaches this arm today: an `object`
    // is only ever a module binding, `declare_module_globals` decides a
    // module global's storage with its own match rather than through
    // this function, and the two positions that would route an `object`
    // here -- a function parameter and a return -- are refused before
    // codegen (`I0404` inside the module, `C0003` at the `ext` export
    // boundary). The arm exists anyway because this function's catch-all
    // is a `panic!`, so the first construct that does give an `object` a
    // signature position would turn a supported program into an internal
    // compiler error rather than a diagnostic; this test is what keeps
    // the arm honest until then. `PyObject *` is opaque to pycc, so the
    // only property worth pinning is that it is the same plain pointer
    // `Ty::Str`/`Ty::List` already get.
    let context = Context::create();
    assert!(ty_to_basic_type(&context, Ty::Object).is_pointer_type());
}

#[test]
fn ty_to_basic_type_gives_tuple_a_struct_representation_positionally() {
    // Supersedes `ty_to_basic_type_panics_clearly_for_tuple`, which
    // asserted that `Ty::Tuple` had no LLVM representation at all --
    // accurate through PR-11a, but PR-11b Task 5 gives it a real arm,
    // so a test demanding a panic would now be asserting the absence of
    // this task's own feature.
    //
    // D-115: unlike every other container (all pointer-represented),
    // a tuple's LLVM type is a real `struct` built field-by-field from
    // each element's own `ty_to_basic_type`. Traces the actual field
    // types rather than just "is a struct", so this pins that each
    // position maps to that element type's own scalar representation
    // exactly (`i64`/`i8`/`f64`, mirroring `Ty::Int`/`Ty::Bool`/
    // `Ty::Float`'s own arms) -- a positional mix-up or a widened
    // `bool` field would still be "a struct" but would fail here.
    let context = Context::create();
    let basic_type = ty_to_basic_type(
        &context,
        Ty::Tuple(Box::new(vec![Ty::Int, Ty::Bool, Ty::Float])),
    );
    let struct_ty = basic_type.into_struct_type();
    assert_eq!(struct_ty.count_fields(), 3);
    assert_eq!(
        struct_ty.get_field_type_at_index(0),
        Some(context.i64_type().into()),
        "a tuple's int field keeps Ty::Int's own i64 representation"
    );
    assert_eq!(
        struct_ty.get_field_type_at_index(1),
        Some(context.i8_type().into()),
        "a tuple's bool field keeps Ty::Bool's own i8 representation (D-061, not i1)"
    );
    assert_eq!(
        struct_ty.get_field_type_at_index(2),
        Some(context.f64_type().into()),
        "a tuple's float field keeps Ty::Float's own f64 representation"
    );
    assert!(
        !struct_ty.is_packed(),
        "tuple fields stay naturally aligned -- ty_to_basic_type passes packed=false"
    );
}

#[test]
fn ty_to_basic_type_builds_a_nested_tuple_struct_recursively() {
    // The `Ty::Tuple` arm is deliberately not narrowed to D-116's
    // current int/bool/float element gate (matching how `List(_)`/
    // `Dict(_)`/`Set(_)` ignore their own element types) -- it recurses
    // through `ty_to_basic_type` for whatever element types it is
    // handed. `pycc_types`' T0039 means no such type reaches codegen
    // from real source today; this pins the representation rule itself
    // so a future widening of T0039 needs no change here.
    let context = Context::create();
    let basic_type = ty_to_basic_type(
        &context,
        Ty::Tuple(Box::new(vec![
            Ty::Int,
            Ty::Tuple(Box::new(vec![Ty::Float])),
        ])),
    );
    let struct_ty = basic_type.into_struct_type();
    assert_eq!(struct_ty.count_fields(), 2);
    let inner = struct_ty
        .get_field_type_at_index(1)
        .expect("the nested tuple field exists")
        .into_struct_type();
    assert_eq!(inner.count_fields(), 1);
    assert_eq!(
        inner.get_field_type_at_index(0),
        Some(context.f64_type().into())
    );
}

#[test]
fn ty_to_basic_type_gives_list_dict_and_set_a_pointer_representation_like_str() {
    // Task 5 (D-089) gave `List(_)` a *real* arm here -- the runtime
    // list object is heap-allocated and pointer-referenced exactly
    // like `Str`'s `PyStrObj`, so both must produce the same LLVM
    // representation. PR-11 Task 5 gave `Dict(_)` the identical
    // treatment for the identical reason (`PyDictObj` is heap-
    // allocated and pointer-referenced too), and PR-11 Task 9 gives
    // `Set(_)` the same treatment again (`PyIntSetObj` is heap-
    // allocated and pointer-referenced too) -- unlike `Tuple`, which
    // PR-11b Task 5 gives a by-value *struct* representation instead,
    // precisely because it has no heap object to point at (D-115; see
    // `ty_to_basic_type_gives_tuple_a_struct_representation_
    // positionally` above). Traces through the actual return value
    // (not just
    // "doesn't panic") to prove all four really match, for a
    // `list[int]`/`list[str]` element type, a `dict[str, int]`
    // key/value pair, and a `set[int]` element type --
    // `ty_to_basic_type`'s own `List(_)`/`Dict(_)`/`Set(_)` arms ignore
    // the element/key/value types entirely.
    let context = Context::create();
    let str_repr = ty_to_basic_type(&context, Ty::Str);
    let list_int_repr = ty_to_basic_type(&context, Ty::List(Box::new(Ty::Int)));
    let list_str_repr = ty_to_basic_type(&context, Ty::List(Box::new(Ty::Str)));
    let dict_repr = ty_to_basic_type(&context, Ty::Dict(Box::new((Ty::Str, Ty::Int))));
    let set_repr = ty_to_basic_type(&context, Ty::Set(Box::new(Ty::Int)));
    assert!(str_repr.is_pointer_type());
    assert!(list_int_repr.is_pointer_type());
    assert!(list_str_repr.is_pointer_type());
    assert!(dict_repr.is_pointer_type());
    assert!(set_repr.is_pointer_type());
    assert_eq!(str_repr, list_int_repr);
    assert_eq!(str_repr, list_str_repr);
    assert_eq!(str_repr, dict_repr);
    assert_eq!(str_repr, set_repr);
}

#[test]
fn reading_a_list_typed_local_back_out_of_its_alloca_produces_a_list_scalar() {
    // Exercises `emit_expr`'s `Name` arm's `Ty::List(_)` arm directly
    // (added by Task 5, D-089; retargeted from the `Scalar::Str` reuse
    // onto `Scalar::List` by Task 11a, D-107) -- same hand-built-
    // `StorageSlot` convention as `reading_an_unresolved_infer_typed_
    // local_is_an_internal_error` above: hand-building the slot is what
    // lets one `emit_expr` call read a `list[int]` local and the next
    // read a `str` one, with nothing else in the fixture to confuse
    // which variant came from which.
    //
    // D-107's entire point is that a `list[T]` pointer must stop being
    // *indistinguishable* from a `str` pointer at the `Scalar` level,
    // so this reads one `list[int]`-typed and one `str`-typed local
    // through the same `emit_expr` entry point and proves the two now
    // produce different variants.
    //
    // The variant is extracted through a helper exercised with *both*
    // values rather than a single-value `let Scalar::List(ptr) = value
    // else { panic!(..) }`: this arm returns `Scalar::List`
    // unconditionally for a `Ty::List` slot, so a single-value
    // `else`/`_` arm would still be statically unreachable and
    // permanently uncovered under this crate's 100%-region gate
    // (D-014) -- exactly the objection this test's own pre-Task-11a
    // comment raised, which giving `list[T]` its own variant does not
    // by itself remove. Feeding the same helper a `str` read covers
    // its other arm with a real, meaningful assertion instead.
    fn loaded_pointer_type<'ctx>(
        scalar: &Scalar<'ctx>,
    ) -> Option<inkwell::types::PointerType<'ctx>> {
        match scalar {
            Scalar::List(ptr) => Some(ptr.get_type()),
            _ => None,
        }
    }

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
        .build_alloca(context.ptr_type(inkwell::AddressSpace::default()), "xs")
        .expect("build_alloca should not fail for a fresh block");
    builder
        .build_store(
            ptr,
            context
                .ptr_type(inkwell::AddressSpace::default())
                .const_null(),
        )
        .expect("build_store should not fail immediately after this function's own alloca");
    let str_ptr = builder
        .build_alloca(context.ptr_type(inkwell::AddressSpace::default()), "s")
        .expect("build_alloca should not fail for a fresh block");
    builder
        .build_store(
            str_ptr,
            context
                .ptr_type(inkwell::AddressSpace::default())
                .const_null(),
        )
        .expect("build_store should not fail immediately after this function's own alloca");
    let locals = HashMap::from([
        (
            "xs".to_string(),
            StorageSlot {
                ptr,
                ty: Ty::List(Box::new(Ty::Int)),
                initialized: None,
            },
        ),
        (
            "s".to_string(),
            StorageSlot {
                ptr: str_ptr,
                ty: Ty::Str,
                initialized: None,
            },
        ),
    ]);

    let list_value = emit_expr(
        &context,
        &builder,
        &module,
        &rt,
        &user_functions,
        &locals,
        &MirExpr::Name {
            name: "xs".to_string(),
            ty: Ty::List(Box::new(Ty::Int)),
        },
    );
    let str_value = emit_expr(
        &context,
        &builder,
        &module,
        &rt,
        &user_functions,
        &locals,
        &MirExpr::Name {
            name: "s".to_string(),
            ty: Ty::Str,
        },
    );
    // The `list[int]` read produces `Scalar::List` carrying the loaded
    // pointer (whose LLVM type must match what `ty_to_basic_type`
    // allocated the slot as); the `str` read, through the very same
    // entry point, does not -- which is the property D-107 exists to
    // establish and the one `Scalar::Str` reuse could never provide.
    assert_eq!(
        loaded_pointer_type(&list_value),
        Some(context.ptr_type(inkwell::AddressSpace::default()))
    );
    assert_eq!(loaded_pointer_type(&str_value), None);
}

/// Builds the minimal `Context`/`Module`/`Builder`/`RtFns` set the
/// `Scalar::List` defensive-panic tests below need. None of them build
/// any IR -- every one panics inside its function's own `match` before
/// reaching a `build_*` call -- so unlike the hand-built-`StorageSlot`
/// tests above, none needs a function or a positioned basic block.
pub(crate) fn list_scalar_panic_fixture(
    context: &Context,
) -> (inkwell::module::Module<'_>, RtFns<'_>) {
    let module = context.create_module("test");
    let rt = declare_rt_functions(context, &module);
    (module, rt)
}

#[test]
#[should_panic(expected = "pycc_codegen: truthiness of a list[T] value is not supported yet")]
fn truthiness_of_a_list_value_panics_honestly() {
    // D-107 confirmed this path is genuinely reachable, not defensive:
    // `pycc_types` accepts any type in a boolean context
    // (`crates/pycc_types/src/lib.rs`'s `if`/`while` handling calls
    // `infer_expr` with no type restriction at all), so `if xs:` for a
    // `list[int]` local type-checks today. v0.2 has no `bool(list)`
    // semantics (D-105 ships only `len(x)`/`x[i]`/iteration/`.append()`),
    // so an honest panic naming the gap is the correct behavior -- the
    // alternative this replaces was `pycc_rt_str_truthy` reading a
    // `PyIntListObj` as a `PyStrObj`.
    //
    // Calls `truthy` directly with a hand-built `Scalar::List` rather
    // than compiling `if xs:` from real MIR: this pins the panic to
    // `truthy` itself, where a future `bool(list)` implementation would
    // land, instead of to whichever caller happens to reach it first.
    // (`docs/ARCHITECTURE.md` records the same gap as user-visible
    // behavior -- `if xs:` type-checks and then stops codegen here.)
    let context = Context::create();
    let (module, rt) = list_scalar_panic_fixture(&context);
    let builder = context.create_builder();
    let ptr = context
        .ptr_type(inkwell::AddressSpace::default())
        .const_null();
    truthy(&context, &builder, &module, &rt, Scalar::List(ptr));
}

#[test]
#[should_panic(
    expected = "pycc_codegen: internal error: an Optional[_] payload had an unsupported LLVM representation"
)]
fn truthy_of_an_optional_with_a_pointer_payload_panics_as_an_internal_error() {
    // `truthy`'s `Scalar::Optional` arm dispatches on the extracted
    // payload's own LLVM type (`FloatValue` for `Ty::Float`, an `i8`-wide
    // `IntValue` for `Ty::Bool`, any other `IntValue` for `Ty::Int`) --
    // #809 widened the accepted inner types to exactly `{int, float,
    // bool}`, so every real `Scalar::Optional` this function ever sees
    // has a payload matching one of those three shapes. The match's
    // catch-all `other => panic!(...)` arm exists purely as a defensive
    // backstop for a payload shape none of the three real inner types can
    // produce -- e.g. a pointer payload, which is what `Optional[str]`
    // would need if `pycc_types`' T0049 gate (which rejects `Optional[T]`
    // for every `T` outside `{int, float, bool}` before codegen) were
    // ever somehow bypassed. This pins that backstop directly with a
    // hand-built `Scalar::Optional` carrying a pointer-typed payload,
    // mirroring `truthiness_of_a_list_value_panics_honestly` above's use
    // of `truthy` as a direct call rather than compiling real MIR.
    let context = Context::create();
    let (module, rt) = list_scalar_panic_fixture(&context);
    let builder = context.create_builder();
    // `truthy`'s `Scalar::Optional` arm calls `build_extract_value`
    // before reaching the defensive match, so -- unlike
    // `truthiness_of_a_tuple_value_panics_honestly` above, whose arm
    // panics immediately -- the builder needs a real positioned block to
    // extract from, not just a bare `Context`/`Builder` pair.
    let fn_type = context.void_type().fn_type(&[], false);
    let f = module.add_function("f", fn_type, None);
    let block = context.append_basic_block(f, "entry");
    builder.position_at_end(block);
    let ptr_ty = context.ptr_type(inkwell::AddressSpace::default());
    let struct_ty = context.struct_type(&[ptr_ty.into(), context.i8_type().into()], false);
    let optional_with_pointer_payload = struct_ty.const_named_struct(&[
        ptr_ty.const_null().into(),
        context.i8_type().const_int(1, false).into(),
    ]);
    truthy(
        &context,
        &builder,
        &module,
        &rt,
        Scalar::Optional(optional_with_pointer_payload),
    );
}

#[test]
#[should_panic(
    expected = "pycc_codegen: string conversion of a list[T] value is not supported yet"
)]
fn string_conversion_of_a_list_value_panics_honestly() {
    // The `to_str` half of the same D-107 pair: `pycc_types` type-checks
    // `print(xs)` for a `list[int]` local unconditionally (its `print`
    // arm returns `Ok(Ty::None)` for any argument type), and `to_str` is
    // what `print` hands its evaluated argument to. Same honest-panic
    // reasoning as `truthiness_of_a_list_value_panics_honestly` above --
    // this replaces handing a `PyIntListObj` pointer to a
    // `pycc_rt_*_to_str` function expecting a `PyStrObj`.
    let context = Context::create();
    let (_module, rt) = list_scalar_panic_fixture(&context);
    let builder = context.create_builder();
    let ptr = context
        .ptr_type(inkwell::AddressSpace::default())
        .const_null();
    to_str(&builder, &rt, Scalar::List(ptr));
}

#[test]
#[should_panic(expected = "internal error: expected an int-or-bool operand, got list")]
fn to_numeric_encoded_int_rejects_a_list_operand() {
    // Unlike `truthy`/`to_str` above, this one really is defensive:
    // `pycc_types`' `numeric_result_type` has no `as_numeric` mapping
    // for `Ty::List`, so any arithmetic with a list operand is rejected
    // as `T0021` long before codegen. Hence the "internal error"
    // wording (matching this function's own neighbouring `Float`/`Str`
    // arms) rather than the "not supported yet" feature-gap wording
    // `truthy`/`to_str` use. Exercised by calling `to_numeric_encoded_int`
    // directly, since a list operand cannot reach it through any MIR --
    // `emit_expr`'s `BinOp` arm claims a `Ty::List` result with its own
    // container-specific panic first (see
    // `a_list_result_binop_is_not_yet_supported` below), so no
    // arithmetic shape gets this far.
    let context = Context::create();
    let builder = context.create_builder();
    let ptr = context
        .ptr_type(inkwell::AddressSpace::default())
        .const_null();
    to_numeric_encoded_int(&context, &builder, Scalar::List(ptr));
}

#[test]
#[should_panic(expected = "internal error: expected an int-or-bool operand, got float")]
fn to_encoded_int_rejects_a_non_int_compatible_operand() {
    // `MirExpr::IntBoundary` is only produced for the valid Bool -> Int
    // subtype boundary. Keep its defensive fallback fail-closed if a
    // malformed MIR module ever supplies another scalar kind.
    let context = Context::create();
    let builder = context.create_builder();
    to_encoded_int(
        &context,
        &builder,
        Scalar::Float(context.f64_type().const_float(1.0)),
    );
}

#[test]
#[should_panic(expected = "internal error: expected a numeric operand, got list")]
fn to_float_rejects_a_list_operand() {
    // Same defensive-arm rationale as `to_numeric_encoded_int_rejects_a_list_
    // operand` directly above, for `to_float`'s own match.
    let context = Context::create();
    let (_module, rt) = list_scalar_panic_fixture(&context);
    let builder = context.create_builder();
    let ptr = context
        .ptr_type(inkwell::AddressSpace::default())
        .const_null();
    to_float(&context, &builder, &rt, Scalar::List(ptr));
}

#[test]
#[should_panic(expected = "pycc_codegen: truthiness of a dict[K, V] value is not supported yet")]
fn truthiness_of_a_dict_value_panics_honestly() {
    // The `dict[K, V]` counterpart of `truthiness_of_a_list_value_
    // panics_honestly` above (D-107's reasoning, per D-124): `pycc_types`
    // accepts any type in a boolean context, so `if x:` for a
    // `dict[str, int]` local type-checks today. v0.2 has no
    // `bool(dict)` semantics (D-123), so an honest panic naming the gap
    // is the correct behavior. Calls `truthy` directly with a hand-built
    // `Scalar::Dict`, for the identical reason that test gives.
    let context = Context::create();
    let (module, rt) = list_scalar_panic_fixture(&context);
    let builder = context.create_builder();
    let ptr = context
        .ptr_type(inkwell::AddressSpace::default())
        .const_null();
    truthy(&context, &builder, &module, &rt, Scalar::Dict(ptr));
}

#[test]
#[should_panic(
    expected = "pycc_codegen: string conversion of a dict[K, V] value is not supported yet"
)]
fn string_conversion_of_a_dict_value_panics_honestly() {
    // The `dict[K, V]` counterpart of `string_conversion_of_a_list_
    // value_panics_honestly` above, for the identical reason.
    let context = Context::create();
    let (_module, rt) = list_scalar_panic_fixture(&context);
    let builder = context.create_builder();
    let ptr = context
        .ptr_type(inkwell::AddressSpace::default())
        .const_null();
    to_str(&builder, &rt, Scalar::Dict(ptr));
}

#[test]
#[should_panic(expected = "internal error: expected an int-or-bool operand, got dict")]
fn to_numeric_encoded_int_rejects_a_dict_operand() {
    // The `dict[K, V]` counterpart of `to_numeric_encoded_int_rejects_a_list_
    // operand` above -- genuinely defensive for the identical reason:
    // `pycc_types`' `numeric_result_type` has no `as_numeric` mapping
    // for `Ty::Dict` either, so no real MIR reaches this arm.
    let context = Context::create();
    let builder = context.create_builder();
    let ptr = context
        .ptr_type(inkwell::AddressSpace::default())
        .const_null();
    to_numeric_encoded_int(&context, &builder, Scalar::Dict(ptr));
}

#[test]
#[should_panic(expected = "internal error: expected a numeric operand, got dict")]
fn to_float_rejects_a_dict_operand() {
    // Same defensive-arm rationale as `to_numeric_encoded_int_rejects_a_dict_
    // operand` directly above, for `to_float`'s own match.
    let context = Context::create();
    let (_module, rt) = list_scalar_panic_fixture(&context);
    let builder = context.create_builder();
    let ptr = context
        .ptr_type(inkwell::AddressSpace::default())
        .const_null();
    to_float(&context, &builder, &rt, Scalar::Dict(ptr));
}

#[test]
fn collect_expr_bindings_skips_a_walrus_target_whose_type_is_outside_the_allow_list() {
    // `collect_expr_bindings`'s `matches!` allow-list (#774) mirrors
    // `collect_stmt_bindings`'s own `MirStmt::Assign` arm allow-list, and
    // its doc comment explains that every `ty` reaching it in practice is
    // expected to already satisfy T0050's stricter upstream restriction
    // (`pycc_types::expr::is_walrus_value_ty_supported` only ever lets a
    // walrus value be `Int`/`Float`/`Bool`/`None`, or `Optional` of one of
    // those) -- so the `false` (skip) branch is not reachable through any
    // real, `check_source`-accepted program, only defensively present "if
    // that restriction ever changes." `Ty::Protocol` is one such
    // currently-unreachable-in-practice type this function's allow-list
    // does not include; calling `collect_expr_bindings` directly with a
    // hand-built `MirExpr::NamedExpr` typed `Protocol` (bypassing
    // `pycc_types`/`pycc_hir` validation entirely, the same pattern this
    // workspace already uses for `pycc_types`'s own similarly-defensive
    // `Ty::Optional(Ty::Str)` walrus fixture) reaches this branch directly
    // and confirms the binding is skipped rather than inserted.
    let expr = MirExpr::NamedExpr {
        name: "p".to_string(),
        value: Box::new(MirExpr::Name {
            name: "p_src".to_string(),
            ty: Ty::Protocol(Box::new("Comparable".to_string())),
        }),
        ty: Ty::Protocol(Box::new("Comparable".to_string())),
    };
    let mut bindings = BTreeMap::new();
    collect_expr_bindings(&expr, &mut bindings);
    assert!(
        bindings.is_empty(),
        "a walrus target typed outside the allow-list must not be predeclared: {bindings:?}"
    );
}

// PEP 572 (#774): `pycc_mir::MirExpr::collect_named_expr_bindings` is a
// shared function statically linked into both `pycc_mir`'s own test binary
// and this crate's -- each links its own separate compiled copy, so this
// crate's test binary needs its own direct exercise of every recursive arm
// independent of `pycc_mir`'s own tests for the same method (see that
// crate's `collect_named_expr_bindings_walks_into_*` tests for the
// counterpart coverage in its own binary). These four tests mirror the
// `IntBoundary`/`ListLiteral`/`SetLiteral`/`Slice`/`Instantiate` arms this
// crate's `collect_expr_bindings` recurses through via that shared method.
#[test]
fn collect_expr_bindings_walks_into_an_int_boundary() {
    let expr = MirExpr::IntBoundary(Box::new(MirExpr::NamedExpr {
        name: "z".to_string(),
        value: Box::new(MirExpr::IntLiteral(1)),
        ty: Ty::Int,
    }));
    let mut bindings = BTreeMap::new();
    collect_expr_bindings(&expr, &mut bindings);
    assert_eq!(bindings.get("z"), Some(&Ty::Int));
}

#[test]
fn collect_expr_bindings_walks_into_a_list_and_set_literal_element() {
    let list_expr = MirExpr::ListLiteral(vec![MirExpr::NamedExpr {
        name: "l".to_string(),
        value: Box::new(MirExpr::IntLiteral(1)),
        ty: Ty::Int,
    }]);
    let mut bindings = BTreeMap::new();
    collect_expr_bindings(&list_expr, &mut bindings);
    assert_eq!(bindings.get("l"), Some(&Ty::Int));

    let set_expr = MirExpr::SetLiteral {
        elements: vec![MirExpr::NamedExpr {
            name: "s".to_string(),
            value: Box::new(MirExpr::IntLiteral(1)),
            ty: Ty::Int,
        }],
        ops: None,
    };
    let mut bindings = BTreeMap::new();
    collect_expr_bindings(&set_expr, &mut bindings);
    assert_eq!(bindings.get("s"), Some(&Ty::Int));
}

#[test]
fn collect_expr_bindings_walks_into_a_slice_bound() {
    let expr = MirExpr::Slice {
        base: Box::new(MirExpr::Name {
            name: "lst".to_string(),
            ty: Ty::List(Box::new(Ty::Int)),
        }),
        start: Some(Box::new(MirExpr::NamedExpr {
            name: "b".to_string(),
            value: Box::new(MirExpr::IntLiteral(0)),
            ty: Ty::Int,
        })),
        stop: None,
        step: None,
    };
    let mut bindings = BTreeMap::new();
    collect_expr_bindings(&expr, &mut bindings);
    assert_eq!(bindings.get("b"), Some(&Ty::Int));
}

#[test]
fn collect_expr_bindings_walks_into_an_instantiate_arg() {
    let expr = MirExpr::Instantiate(Box::new(InstantiateExpr {
        ctor: "C.__init__".to_string(),
        class_name: "C".to_string(),
        slot_names: vec!["s0".to_string()],
        args: vec![MirExpr::NamedExpr {
            name: "a".to_string(),
            value: Box::new(MirExpr::IntLiteral(1)),
            ty: Ty::Int,
        }],
        ty: Ty::Instance(Box::new("C".to_string())),
    }));
    let mut bindings = BTreeMap::new();
    collect_expr_bindings(&expr, &mut bindings);
    assert_eq!(bindings.get("a"), Some(&Ty::Int));
}

#[test]
fn collect_stmt_bindings_includes_a_list_typed_assignment_target() {
    // Task 5 (D-089) added `Ty::List(_)` to this allow-list -- Task 11
    // depends on a `list[int]` local's binding already being collected
    // here, so this is a real, deliberate inclusion to verify, not
    // just a louder panic elsewhere.
    let stmt = MirStmt::Assign {
        target: "xs".to_string(),
        value: MirExpr::Name {
            name: "xs".to_string(),
            ty: Ty::List(Box::new(Ty::Int)),
        },
    };
    let mut bindings = BTreeMap::new();
    collect_stmt_bindings(&stmt, &mut bindings);
    assert_eq!(
        bindings.get("xs"),
        Some(&Ty::List(Box::new(Ty::Int))),
        "a list[int]-typed assignment target's binding should be collected"
    );
}

#[test]
fn collect_stmt_bindings_includes_a_none_typed_assignment_target() {
    let stmt = MirStmt::Assign {
        target: "result".to_string(),
        value: MirExpr::Name {
            name: "result".to_string(),
            ty: Ty::None,
        },
    };
    let mut bindings = BTreeMap::new();
    collect_stmt_bindings(&stmt, &mut bindings);
    assert_eq!(
        bindings.get("result"),
        Some(&Ty::None),
        "a None-typed assignment target needs a real storage slot"
    );
}

#[test]
fn collect_stmt_bindings_includes_a_dict_typed_assignment_target() {
    // PR-11 Task 5 joins `Ty::Dict(_)` to this allow-list, mirroring
    // `Ty::List(_)`'s own inclusion directly above for the identical
    // reason: this task's own codegen (`declare_module_globals`/
    // `storage_slot_at_entry`) depends on a `dict[str, int]` local's
    // binding already being collected here.
    let stmt = MirStmt::Assign {
        target: "x".to_string(),
        value: MirExpr::Name {
            name: "x".to_string(),
            ty: Ty::Dict(Box::new((Ty::Str, Ty::Int))),
        },
    };
    let mut bindings = BTreeMap::new();
    collect_stmt_bindings(&stmt, &mut bindings);
    assert_eq!(
        bindings.get("x"),
        Some(&Ty::Dict(Box::new((Ty::Str, Ty::Int)))),
        "a dict[str, int]-typed assignment target's binding should be collected"
    );
}

#[test]
fn collect_stmt_bindings_includes_a_set_typed_assignment_target() {
    // PR-11 Task 9 joins `Ty::Set(_)` to this allow-list, mirroring
    // `Ty::List(_)`/`Ty::Dict(_)`'s own inclusion above for the
    // identical reason: this task's own codegen
    // (`declare_module_globals`/`storage_slot_at_entry`) depends on a
    // `set[int]` local's binding already being collected here.
    let stmt = MirStmt::Assign {
        target: "xs".to_string(),
        value: MirExpr::Name {
            name: "xs".to_string(),
            ty: Ty::Set(Box::new(Ty::Int)),
        },
    };
    let mut bindings = BTreeMap::new();
    collect_stmt_bindings(&stmt, &mut bindings);
    assert_eq!(
        bindings.get("xs"),
        Some(&Ty::Set(Box::new(Ty::Int))),
        "a set[int]-typed assignment target's binding should be collected"
    );
}

#[test]
fn collect_stmt_bindings_includes_a_tuple_typed_assignment_target() {
    // Inverts `collect_stmt_bindings_excludes_a_tuple_typed_assignment_
    // target`, which asserted the opposite: `Tuple` was excluded from
    // the allow-list while no codegen existed for it. PR-11b Task 5
    // joins `Ty::Tuple(_)` to that list, mirroring `Ty::List(_)`/
    // `Ty::Dict(_)`/`Ty::Set(_)`'s own inclusion, for the identical
    // reason -- `declare_module_globals`/`storage_slot_at_entry` can
    // only allocate a slot for a binding that was collected here, so
    // without this `x = (1, 2)` would reach codegen with no storage at
    // all.
    let stmt = MirStmt::Assign {
        target: "xs".to_string(),
        value: MirExpr::Name {
            name: "xs".to_string(),
            ty: Ty::Tuple(Box::new(vec![Ty::Int, Ty::Float])),
        },
    };
    let mut bindings = BTreeMap::new();
    collect_stmt_bindings(&stmt, &mut bindings);
    assert_eq!(
        bindings.get("xs"),
        Some(&Ty::Tuple(Box::new(vec![Ty::Int, Ty::Float]))),
        "a tuple[int, float]-typed assignment target's binding should be collected"
    );
}

#[test]
fn collect_stmt_bindings_excludes_a_dict_set_target() {
    // `d[k] = v` (PR-11 Task 4) reassigns an existing binding's
    // contents, not a name -- mirrors `pycc_types::collect_local_names`'s
    // own identical `HirStmt::DictSet` test. `dict` itself must not
    // pick up an entry from this statement.
    let stmt = MirStmt::DictSet {
        dict: "x".to_string(),
        key: MirExpr::StringLiteral("a".to_string()),
        value: MirExpr::IntLiteral(1),
    };
    let mut bindings = BTreeMap::new();
    collect_stmt_bindings(&stmt, &mut bindings);
    assert_eq!(bindings.get("x"), None);
}

#[test]
fn collect_stmt_bindings_binds_a_for_dict_loop_variable_as_str_and_recurses_into_its_body() {
    // `MirStmt::ForDict` now binds its own loop variable (PR-11 Task 5),
    // mirroring `MirStmt::ForList`'s own `Ty::Int` hardcode -- but
    // `Ty::Str`, since a dict's iterated key type is always exactly
    // `Ty::Str` (see this arm's own doc comment). It must also still
    // recurse into `body`, exactly like `ForList`/`ForRange`/`If`/
    // `While` above, so a nested, ordinary statement's binding is not
    // silently dropped.
    let stmt = MirStmt::ForDict {
        var: "k".to_string(),
        dict: "d".to_string(),
        body: vec![MirStmt::Assign {
            target: "y".to_string(),
            value: MirExpr::IntLiteral(1),
        }],
    };
    let mut bindings = BTreeMap::new();
    collect_stmt_bindings(&stmt, &mut bindings);
    assert_eq!(bindings.get("y"), Some(&Ty::Int));
    assert_eq!(bindings.get("k"), Some(&Ty::Str));
}

#[test]
fn collect_stmt_bindings_binds_a_for_set_loop_variable_as_int_and_recurses_into_its_body() {
    // `MirStmt::ForSet` now binds its own loop variable (PR-11 Task 9),
    // mirroring `MirStmt::ForList`'s own `Ty::Int` hardcode exactly --
    // a set's iterated element type is always exactly `Ty::Int` (see
    // this arm's own doc comment), unlike `ForDict`'s `Ty::Str` key.
    // Superseded Task 8's own version of this test (`collect_stmt_
    // bindings_recurses_into_a_for_set_loop_body_without_binding_its_
    // own_var`), which pinned the pre-Task-9 deferred-decision
    // behavior. It must also still recurse into `body`, exactly like
    // `ForList`/`ForDict`/`ForRange`/`If`/`While` above, so a nested,
    // ordinary statement's binding is not silently dropped.
    let stmt = MirStmt::ForSet {
        var_ty: pycc_mir::Ty::Int,
        var: "v".to_string(),
        set: "s".to_string(),
        body: vec![MirStmt::Assign {
            target: "y".to_string(),
            value: MirExpr::IntLiteral(1),
        }],
    };
    let mut bindings = BTreeMap::new();
    collect_stmt_bindings(&stmt, &mut bindings);
    assert_eq!(bindings.get("y"), Some(&Ty::Int));
    assert_eq!(bindings.get("v"), Some(&Ty::Int));
}

#[test]
#[should_panic(expected = "binary operators are not supported on list[int] yet")]
fn a_list_result_binop_is_not_yet_supported() {
    // No real Python operator produces a `BinOp` typed `list[int]`
    // (D-105 defers even `+` list concatenation past v0.2), so
    // `pycc_types`/`pycc_mir` never produce this shape -- hand-crafted
    // MIR exercises `emit_expr`'s `BinOp` arm's new explicit container
    // arm directly, same "hand-construct the otherwise-unreachable
    // shape" convention as `a_none_result_binop_is_not_yet_supported`
    // above.
    //
    // Deliberately a function-local assignment, not a top-level one:
    // `collect_stmt_bindings`'s allow-list now includes `Ty::List(_)`
    // (Task 5, D-089, see the two `collect_stmt_bindings_*` tests
    // above), so a top-level `list[int]` binding would be collected
    // and routed to `declare_module_globals` -- which does *not* get a
    // `List(_)` arm (that catch-all's own test covers it) -- panicking
    // there first and never reaching this arm at all. A function-local
    // binding's slot is instead declared via `storage_slot_at_entry`/
    // `ty_to_basic_type`, which *does* give `List(_)` a real pointer
    // representation, so codegen gets past slot allocation and
    // actually reaches this `BinOp` arm.
    let mir = MirModule {
        items: vec![MirItem::Function {
            name: "f".to_string(),
            params: vec![],
            return_ty: Ty::None,
            body: vec![MirStmt::Assign {
                target: "x".to_string(),
                value: MirExpr::BinOp {
                    op: BinOpKind::Add,
                    left: Box::new(MirExpr::IntLiteral(1)),
                    right: Box::new(MirExpr::IntLiteral(2)),
                    ty: Ty::List(Box::new(Ty::Int)),
                },
            }],
        }],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("binop_list_result_panics")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("binop_list_result_panics.o");
    let _ = compile_to_object(&mir, &obj_path, None, false);
}

/// Part 1 of #1319: `frozenset[int]` joins the named container arm, so a
/// hand-built `frozenset[int]`-result `BinOp` (which `pycc_types` refuses
/// as `T0021`) names the type instead of reaching the generic catch-all.
#[test]
#[should_panic(expected = "binary operators are not supported on frozenset[int] yet")]
fn a_frozenset_result_binop_is_not_yet_supported() {
    let mir = MirModule {
        items: vec![MirItem::Function {
            name: "f".to_string(),
            params: vec![],
            return_ty: Ty::None,
            body: vec![MirStmt::Assign {
                target: "x".to_string(),
                value: MirExpr::BinOp {
                    op: BinOpKind::BitOr,
                    left: Box::new(MirExpr::IntLiteral(1)),
                    right: Box::new(MirExpr::IntLiteral(2)),
                    ty: Ty::FrozenSet(Box::new(Ty::Int)),
                },
            }],
        }],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("binop_frozenset_result_panics")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("binop_frozenset_result_panics.o");
    let _ = compile_to_object(&mir, &obj_path, None, false);
}

#[test]
fn compiles_a_function_with_a_list_int_parameter_and_list_int_return_value() {
    // `def f(x: list[int]) -> list[int]: return x`. Both halves are
    // producible from real source now: the *parameter* half since D-228
    // (issue #918), and the `-> list[int]` half since #925 removed the
    // return-position `C0001` (see
    // `tests/issue_925_container_returns.rs`). The MIR shape below is
    // kept hand-built as the direct codegen-level pin, and Task 5
    // (D-089) requires this MIR shape to compile *cleanly* rather than
    // panic: `ty_to_basic_type`'s `List(_)` arm (parameter type, and
    // transitively the return type via `compile_to_object`'s `fn_type`
    // delegation) and `emit_expr`'s `Name` arm's `List(_)` arm must
    // agree on the same pointer representation, or `module.verify()`
    // inside `compile_to_object` would reject the mismatched IR.
    // Deliberately does not link or run the resulting object: `f` is
    // never called, and no caller could construct the annotated
    // `list[int]` argument it wants, so this only proves the codegen
    // shape is internally consistent, not that the program is
    // meaningful to run.
    //
    // Also the regression test for a review finding on this task's
    // first pass: `return x` is a bare `Name` read of a `list[int]`
    // parameter, which is exactly the shape `incref_if_str_duplicate`
    // dispatches on -- before `str_value_is_a_duplicate_reference` was
    // gated on `ty: Ty::Str` (see that function's own doc comment),
    // this test's own generated object emitted a spurious call to
    // `pycc_rt_str_incref` on the list pointer. Asserted against
    // directly below, using the equivalently-shaped
    // `compiles_a_function_with_a_float_parameter_and_float_return_value`
    // above as the known-clean baseline (a bare `float`-typed `Name`
    // return, which has never referenced any `pycc_rt_str_*` symbol).
    let mir = MirModule {
        items: vec![MirItem::Function {
            name: "f".to_string(),
            params: vec![("x".to_string(), Ty::List(Box::new(Ty::Int)))],
            return_ty: Ty::List(Box::new(Ty::Int)),
            body: vec![MirStmt::Return(Some(MirExpr::Name {
                name: "x".to_string(),
                ty: Ty::List(Box::new(Ty::Int)),
            }))],
        }],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("list_int_param_and_return")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("list_int_param_and_return.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let obj_bytes = std::fs::read(&obj_path).expect("object file should be readable");
    let references_symbol =
        |name: &str| obj_bytes.windows(name.len()).any(|w| w == name.as_bytes());
    assert!(
        !references_symbol("pycc_rt_str_incref"),
        "a list[int]-typed bare-Name read must not be treated as a duplicate str \
             reference and incref'd as if it were one"
    );
    assert!(
        !references_symbol("pycc_rt_str_decref"),
        "a list[int]-typed value must not be decref'd as if it were str"
    );
}

#[test]
fn passing_a_list_value_as_a_function_argument_marshals_it_like_a_pointer() {
    // `def f(x: list[int]) -> list[int]: return x` plus
    // `def g(x: list[int]) -> list[int]: return f(x)` -- the caller adds
    // the one shape the test directly above does not reach:
    // `build_call_to`'s argument-marshalling match, whose `Scalar::List`
    // arm Task 11a (D-107) put in the *pass-through* bucket. That claim
    // ("a list pointer marshals identically to a str pointer -- it's an
    // opaque pointer either way") is exactly what `module.verify()`
    // inside `compile_to_object` checks here: if the marshalled argument
    // disagreed with `ty_to_basic_type`'s parameter type, LLVM would
    // reject the call instruction outright.
    //
    // Same not-linked, not-run caveat as the test above: neither `f`
    // nor `g` is ever called, and their annotated `list[int]`
    // parameters are unreachable from real source, so this proves the
    // codegen shape only. The `pycc_rt_str_*` assertion carries over for
    // the same reason -- a `list[int]` argument is a bare `Name` read,
    // the exact shape `incref_if_str_duplicate` dispatches on, and
    // `build_call_to` calls that helper on every argument.
    //
    // Both functions return `int`, not `list[int]`: this test is about
    // the *argument* side, which is the side that has a `Scalar::List`
    // arm to verify here. The result side gained its own `Ty::List` arm
    // in #925 (`call_result_scalar`), covered by
    // `a_list_typed_call_result_is_returned_as_a_pointer` below.
    let list_int = || Ty::List(Box::new(Ty::Int));
    let mir = MirModule {
        items: vec![
            MirItem::Function {
                name: "f".to_string(),
                params: vec![("x".to_string(), list_int())],
                return_ty: Ty::Int,
                body: vec![MirStmt::Return(Some(MirExpr::IntLiteral(0)))],
            },
            MirItem::Function {
                name: "g".to_string(),
                params: vec![("x".to_string(), list_int())],
                return_ty: Ty::Int,
                body: vec![MirStmt::Return(Some(MirExpr::Call {
                    callee: "f".to_string(),
                    args: vec![MirExpr::Name {
                        name: "x".to_string(),
                        ty: list_int(),
                    }],
                    ty: Ty::Int,
                }))],
            },
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("list_int_passed_as_argument")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("list_int_passed_as_argument.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let obj_bytes = std::fs::read(&obj_path).expect("object file should be readable");
    let references_symbol =
        |name: &str| obj_bytes.windows(name.len()).any(|w| w == name.as_bytes());
    assert!(
        !references_symbol("pycc_rt_str_incref"),
        "a list[int]-typed argument must not be incref'd as if it were a duplicate str \
             reference"
    );
}

#[test]
fn assigning_a_list_value_stores_the_raw_pointer() {
    // Covers `emit_assign`'s `Scalar::List` arm, the third member of
    // Task 11a's pass-through bucket (D-107), in isolation: it calls
    // `emit_assign` directly with a hand-built `Scalar::List` and a
    // hand-built `StorageSlot` so the store instruction itself is what
    // `f.verify(true)` judges, with no surrounding list construction to
    // fail first. (`xs = [1, 2, 3]` reaches this same arm through real
    // MIR in the list tests further below; this one pins the store's IR
    // shape rather than the program's output.)
    //
    // `f.verify(true)` is the real assertion: `ty_to_basic_type`
    // allocated this slot as a pointer, so a `store` of anything but a
    // pointer-typed value would be rejected as malformed IR. Also
    // proves no `str`-style refcount traffic is emitted -- D-107 keeps
    // `list[T]` leak-only for v0.2.
    let context = Context::create();
    let module = context.create_module("test");
    let builder = context.create_builder();
    let fn_type = context.void_type().fn_type(&[], false);
    let f = module.add_function("f", fn_type, None);
    let block = context.append_basic_block(f, "entry");
    builder.position_at_end(block);

    let ptr = builder
        .build_alloca(context.ptr_type(inkwell::AddressSpace::default()), "xs")
        .expect("build_alloca should not fail for a fresh block");
    let locals = HashMap::from([(
        "xs".to_string(),
        StorageSlot {
            ptr,
            ty: Ty::List(Box::new(Ty::Int)),
            initialized: None,
        },
    )]);
    let value = context
        .ptr_type(inkwell::AddressSpace::default())
        .const_null();
    let rt = declare_rt_functions(&context, &module);
    emit_assign(&context, &builder, &rt, &locals, "xs", Scalar::List(value));
    builder
        .build_return(None)
        .expect("build_return should not fail for a void function");

    assert!(
        f.verify(true),
        "storing a list[T] pointer into its own pointer-typed slot must be valid IR"
    );
}

/// Wraps `body` in a `None`-returning function `f` that is then called
/// at top level -- the shape every `list[int]` MIR fixture below needs,
/// since a `list[int]` binding only becomes a function-local (rather
/// than a module global) when its assignment lives inside a function
/// body. Kept as a helper because each of these tests differs only in
/// the statements it puts inside.
fn list_fixture_module(body: Vec<MirStmt>) -> MirModule {
    MirModule {
        items: vec![
            MirItem::Function {
                name: "f".to_string(),
                params: vec![],
                return_ty: Ty::None,
                body,
            },
            MirItem::TopLevelStmt(call_user_fn("f")),
        ],
        class_defs: Vec::new(),
    }
}

/// `xs = [1, 2, 3]` as a `MirStmt`, the prelude most of the `list[int]`
/// fixtures below open with.
fn assign_list_literal(target: &str) -> MirStmt {
    MirStmt::Assign {
        target: target.to_string(),
        value: MirExpr::ListLiteral(vec![
            MirExpr::IntLiteral(1),
            MirExpr::IntLiteral(2),
            MirExpr::IntLiteral(3),
        ]),
    }
}

#[test]
#[should_panic(expected = "`len` takes exactly 1 argument, got 0")]
fn a_len_call_with_the_wrong_argument_count_is_an_internal_error() {
    // `pycc_types` already rejects a mis-arity `len` call with T0033
    // (its own `len` arm checks `arg_tys.len() != 1` before anything
    // else), so this is hand-built malformed MIR exercising codegen's
    // own defensive backstop -- the same convention as
    // `referencing_a_name_with_no_bound_local_is_an_internal_error`
    // above.
    let mir = list_fixture_module(vec![MirStmt::ExprStmt(MirExpr::Call {
        callee: "len".to_string(),
        args: vec![],
        ty: Ty::Int,
    })]);
    let dir = pycc_scratch::ScratchDir::new("len_wrong_arity_panics")
        .expect("failed to create scratch dir");
    let _ = compile_to_object(&mir, &dir.join("len_wrong_arity_panics.o"), None, false);
}

#[test]
#[should_panic(expected = "`float` takes exactly 1 argument, got 0")]
fn a_float_call_with_the_wrong_argument_count_is_an_internal_error() {
    // `pycc_types` already rejects a mis-arity `float` call with T0021
    // (its own `float` arm checks `arg_tys.len() != 1` before anything
    // else), so this is hand-built malformed MIR exercising codegen's
    // own defensive backstop, mirroring
    // `a_len_call_with_the_wrong_argument_count_is_an_internal_error`
    // immediately above.
    let mir = list_fixture_module(vec![MirStmt::ExprStmt(MirExpr::Call {
        callee: "float".to_string(),
        args: vec![],
        ty: Ty::Float,
    })]);
    let dir = pycc_scratch::ScratchDir::new("float_wrong_arity_panics")
        .expect("failed to create scratch dir");
    let _ = compile_to_object(&mir, &dir.join("float_wrong_arity_panics.o"), None, false);
}

#[test]
#[should_panic(expected = "`bool` takes exactly 1 argument, got 0")]
fn a_bool_call_with_the_wrong_argument_count_is_an_internal_error() {
    // Part 4 of #1026 (PR 4a of #1083). `pycc_types` admits the `bool`
    // builtin only for exactly one `Ty::Object` argument and leaves every
    // other shape to C0001, so this is hand-built malformed MIR exercising
    // codegen's own defensive backstop, mirroring
    // `a_float_call_with_the_wrong_argument_count_is_an_internal_error`
    // immediately above.
    let mir = list_fixture_module(vec![MirStmt::ExprStmt(MirExpr::Call {
        callee: "bool".to_string(),
        args: vec![],
        ty: Ty::Bool,
    })]);
    let dir = pycc_scratch::ScratchDir::new("bool_wrong_arity_panics")
        .expect("failed to create scratch dir");
    let _ = compile_to_object(&mir, &dir.join("bool_wrong_arity_panics.o"), None, false);
}

#[test]
#[should_panic(expected = "`bool` takes a CPython object argument")]
fn a_bool_call_on_a_non_object_argument_is_an_internal_error() {
    // Part 4 of #1026 (PR 4a of #1083). `bool(1)` keeps its C0001 refusal in
    // `pycc_types`, so reaching codegen at all means the MIR is malformed;
    // this pins the second of the `bool` arm's two backstops.
    let mir = list_fixture_module(vec![MirStmt::ExprStmt(MirExpr::Call {
        callee: "bool".to_string(),
        args: vec![MirExpr::IntLiteral(1)],
        ty: Ty::Bool,
    })]);
    let dir = pycc_scratch::ScratchDir::new("bool_non_object_panics")
        .expect("failed to create scratch dir");
    let _ = compile_to_object(&mir, &dir.join("bool_non_object_panics.o"), None, false);
}

#[test]
#[should_panic(expected = "`int` takes exactly 1 argument, got 0")]
fn an_int_call_with_the_wrong_argument_count_is_an_internal_error() {
    // Part 4 of #1026 (PR 4b of #1083). `int` and `str` share one emission
    // arm, so this pins its arity backstop; the `str` spelling of the same
    // call reaches the identical `panic!` and needs no second test, while
    // the *operand* backstop below is a different line and does.
    let mir = list_fixture_module(vec![MirStmt::ExprStmt(MirExpr::Call {
        callee: "int".to_string(),
        args: vec![],
        ty: Ty::Int,
    })]);
    let dir = pycc_scratch::ScratchDir::new("int_wrong_arity_panics")
        .expect("failed to create scratch dir");
    let _ = compile_to_object(&mir, &dir.join("int_wrong_arity_panics.o"), None, false);
}

#[test]
#[should_panic(expected = "`str` takes a CPython object argument")]
fn a_str_call_on_a_non_object_argument_is_an_internal_error() {
    // Part 4 of #1026 (PR 4b of #1083). `str(1)` keeps its C0001 refusal in
    // `pycc_types`, so reaching codegen at all means the MIR is malformed.
    // Spelled with `str` rather than `int` so the shared arm's `{callee}`
    // interpolation is observed on both names across the two tests.
    let mir = list_fixture_module(vec![MirStmt::ExprStmt(MirExpr::Call {
        callee: "str".to_string(),
        args: vec![MirExpr::IntLiteral(1)],
        ty: Ty::Str,
    })]);
    let dir = pycc_scratch::ScratchDir::new("str_non_object_panics")
        .expect("failed to create scratch dir");
    let _ = compile_to_object(&mir, &dir.join("str_non_object_panics.o"), None, false);
}

#[test]
#[should_panic(expected = "`math.sqrt` takes exactly 1 argument, got 0")]
fn a_math_sqrt_call_with_the_wrong_argument_count_is_an_internal_error() {
    // `pycc_types` already rejects a mis-arity `math.sqrt` call with
    // T0021, so this is hand-built malformed MIR exercising codegen's
    // own defensive backstop, mirroring
    // `a_float_call_with_the_wrong_argument_count_is_an_internal_error`
    // immediately above.
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
            callee: "math.sqrt".to_string(),
            args: vec![],
            ty: Ty::Float,
        }))],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("math_sqrt_wrong_arity_panics")
        .expect("failed to create scratch dir");
    let _ = compile_to_object(
        &mir,
        &dir.join("math_sqrt_wrong_arity_panics.o"),
        None,
        false,
    );
}

#[test]
#[should_panic(expected = "`math.sqrt`'s argument was not a float")]
fn a_math_sqrt_call_on_a_non_float_argument_is_an_internal_error() {
    // The other half of `pycc_types`' own T0021 `math.sqrt` check (a
    // non-`float` argument) -- hand-built malformed MIR, since
    // `pycc_types::std_qualified_symbol`'s call-site check already
    // rejects this before codegen runs for any legitimate source.
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
            callee: "math.sqrt".to_string(),
            args: vec![MirExpr::IntLiteral(1)],
            ty: Ty::Float,
        }))],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("math_sqrt_non_float_panics")
        .expect("failed to create scratch dir");
    let _ = compile_to_object(&mir, &dir.join("math_sqrt_non_float_panics.o"), None, false);
}

#[test]
#[should_panic(expected = "`len`'s argument did not evaluate to a list")]
fn a_len_call_on_a_non_list_argument_is_an_internal_error() {
    // The other half of `pycc_types`' own T0033 `len` check (a
    // non-`list[T]` argument), and the naturally reachable cover for
    // `expect_list_pointer`'s shared panic: `emit_expr` really does
    // return a non-`List` scalar here, no self-inconsistent MIR needed.
    let mir = list_fixture_module(vec![MirStmt::ExprStmt(MirExpr::Call {
        callee: "len".to_string(),
        args: vec![MirExpr::IntLiteral(1)],
        ty: Ty::Int,
    })]);
    let dir =
        pycc_scratch::ScratchDir::new("len_non_list_panics").expect("failed to create scratch dir");
    let _ = compile_to_object(&mir, &dir.join("len_non_list_panics.o"), None, false);
}

#[test]
#[should_panic(expected = "`n` did not evaluate to a list")]
fn appending_to_a_non_list_local_is_an_internal_error() {
    // `n = 1` then `n.append(2)`: `pycc_types` rejects this with T0033
    // ("value does not support list operations"), so codegen only sees
    // it as hand-built malformed MIR. Covers `emit_list_name_read`'s
    // own use of `expect_list_pointer`, the shared check
    // `MirStmt::ForList`'s list operand and `MirExpr::Subscript`'s base
    // also go through.
    let mir = list_fixture_module(vec![
        MirStmt::Assign {
            target: "n".to_string(),
            value: MirExpr::IntLiteral(1),
        },
        MirStmt::ExprStmt(MirExpr::ListAppend {
            list: pycc_mir::MirContainerReceiver::Name("n".to_string()),
            value: Box::new(MirExpr::IntLiteral(2)),
        }),
    ]);
    let dir = pycc_scratch::ScratchDir::new("append_to_non_list_panics")
        .expect("failed to create scratch dir");
    let _ = compile_to_object(&mir, &dir.join("append_to_non_list_panics.o"), None, false);
}

#[test]
#[should_panic(expected = "`never_bound` has no local slot")]
fn iterating_a_name_with_no_local_slot_is_an_internal_error() {
    // `for v in never_bound:` where nothing ever bound `never_bound` --
    // `pycc_types` rejects an unbound list operand (T0033/T0021, see
    // its `lookup_bound_name` helper), so this is codegen's own
    // defensive backstop for `emit_list_name_read`'s slot lookup, the
    // one branch of it `appending_to_a_non_list_local_is_an_internal_
    // error` above does not reach.
    let mir = list_fixture_module(vec![MirStmt::ForList {
        var: "v".to_string(),
        list: "never_bound".to_string(),
        body: vec![],
    }]);
    let dir = pycc_scratch::ScratchDir::new("for_list_unbound_name_panics")
        .expect("failed to create scratch dir");
    let _ = compile_to_object(
        &mir,
        &dir.join("for_list_unbound_name_panics.o"),
        None,
        false,
    );
}

/// Builds `print(<a bare expression>)` as a `MirStmt`, for the dict
/// codegen tests below -- like `call_print` above but for an arbitrary
/// argument expression rather than only an int literal.
fn print_expr(arg: MirExpr) -> MirStmt {
    MirStmt::ExprStmt(MirExpr::Call {
        callee: "print".to_string(),
        args: vec![arg],
        ty: Ty::None,
    })
}

fn dict_str_int() -> Ty {
    Ty::Dict(Box::new((Ty::Str, Ty::Int)))
}

fn dict_name(name: &str) -> MirExpr {
    MirExpr::Name {
        name: name.to_string(),
        ty: dict_str_int(),
    }
}

fn set_int() -> Ty {
    Ty::Set(Box::new(Ty::Int))
}

fn set_name(name: &str) -> MirExpr {
    MirExpr::Name {
        name: name.to_string(),
        ty: set_int(),
    }
}

#[test]
fn dict_literal_construction_codegens_and_runs() {
    // `x = {"a": 1, "b": 2}\nprint(len(x))\n` end to end through
    // `compile_to_object` -- real, previously-panicking `MirExpr::
    // DictLiteral` construction (PR-11 Task 5) plus `len()`'s new
    // `Ty::Dict` branch. Expected output verified against `python3` on
    // this exact source.
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
            MirItem::TopLevelStmt(print_expr(MirExpr::Call {
                callee: "len".to_string(),
                args: vec![dict_name("x")],
                ty: Ty::Int,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("dict_literal_and_len")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("dict_literal_and_len.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("dict_literal_and_len");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"2\n");
}

fn instance_ty(class_name: &str) -> Ty {
    Ty::Instance(Box::new(class_name.to_string()))
}

#[test]
fn class_instantiation_attribute_and_method_call_codegens_and_runs() {
    // D-154 (Part 1 of #375), end to end through `compile_to_object`:
    //
    //     class Point:
    //         def __init__(self, x: int, y: int) -> None:
    //             self.x = x
    //             self.y = y
    //         def bump(self) -> None:
    //             self.x = self.x + 1
    //
    //     p = Point(1, 2)
    //     p.bump()
    //     print(p.x)
    //     print(p.y)
    //
    // Expected output verified against `python3` on this exact source
    // (`p.x` starts `1`, `bump()` increments it once to `2`; `p.y`
    // stays `2`). Exercises every new codegen path this issue adds in
    // one program: `MirExpr::Instantiate` (allocation + `__init__`
    // call), `MirStmt::AttrSet` (both inside `__init__` and inside
    // `bump`), `MirExpr::AttrGet` (both inside `bump`'s own `self.x +
    // 1` and at module scope for the two `print` calls), and an
    // ordinary method call lowered to `MirExpr::Call` with `self`
    // prepended.
    let self_ty = instance_ty("Point");
    let init = MirItem::Function {
        name: "Point.__init__".to_string(),
        params: vec![
            ("self".to_string(), self_ty.clone()),
            ("x".to_string(), Ty::Int),
            ("y".to_string(), Ty::Int),
        ],
        return_ty: Ty::None,
        body: vec![
            MirStmt::AttrSet {
                base: MirExpr::Name {
                    name: "self".to_string(),
                    ty: self_ty.clone(),
                },
                slot: 0,
                value: MirExpr::Name {
                    name: "x".to_string(),
                    ty: Ty::Int,
                },
            },
            MirStmt::AttrSet {
                base: MirExpr::Name {
                    name: "self".to_string(),
                    ty: self_ty.clone(),
                },
                slot: 1,
                value: MirExpr::Name {
                    name: "y".to_string(),
                    ty: Ty::Int,
                },
            },
            MirStmt::Return(None),
        ],
    };
    let bump = MirItem::Function {
        name: "Point.bump".to_string(),
        params: vec![("self".to_string(), self_ty.clone())],
        return_ty: Ty::None,
        body: vec![
            MirStmt::AttrSet {
                base: MirExpr::Name {
                    name: "self".to_string(),
                    ty: self_ty.clone(),
                },
                slot: 0,
                value: MirExpr::BinOp {
                    op: BinOpKind::Add,
                    left: Box::new(MirExpr::AttrGet {
                        base: Box::new(MirExpr::Name {
                            name: "self".to_string(),
                            ty: self_ty.clone(),
                        }),
                        slot: 0,
                        ty: Ty::Int,
                    }),
                    right: Box::new(MirExpr::IntLiteral(1)),
                    ty: Ty::Int,
                },
            },
            MirStmt::Return(None),
        ],
    };
    let mir = MirModule {
        items: vec![
            init,
            bump,
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "p".to_string(),
                value: MirExpr::Instantiate(Box::new(pycc_mir::InstantiateExpr {
                    ctor: "Point.__init__".to_string(),
                    class_name: "C".to_string(),
                    slot_names: vec!["s0".to_string(), "s1".to_string()],
                    args: vec![MirExpr::IntLiteral(1), MirExpr::IntLiteral(2)],
                    ty: self_ty.clone(),
                })),
            }),
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
                callee: "Point.bump".to_string(),
                args: vec![MirExpr::Name {
                    name: "p".to_string(),
                    ty: self_ty.clone(),
                }],
                ty: Ty::None,
            })),
            MirItem::TopLevelStmt(print_expr(MirExpr::AttrGet {
                base: Box::new(MirExpr::Name {
                    name: "p".to_string(),
                    ty: self_ty.clone(),
                }),
                slot: 0,
                ty: Ty::Int,
            })),
            MirItem::TopLevelStmt(print_expr(MirExpr::AttrGet {
                base: Box::new(MirExpr::Name {
                    name: "p".to_string(),
                    ty: self_ty,
                }),
                slot: 1,
                ty: Ty::Int,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("class_instantiation_attribute_and_method_call")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("class_instantiation_attribute_and_method_call.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("class_instantiation_attribute_and_method_call");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"2\n2\n");
}

#[test]
fn bool_float_and_str_typed_attribute_slots_round_trip_correctly() {
    // `class_instantiation_attribute_and_method_call_codegens_and_runs`
    // above only ever exercises `int`-typed attribute slots --
    // `slot_word_to_scalar`/`scalar_to_slot_word`'s own `Bool`/`Float`/
    // `Str` arms (D-154) need their own exercise. `__init__` forwarding
    // each constructor parameter straight into its own attribute slot
    // exercises both the write (`scalar_to_slot_word`, during
    // `__init__`) and the read (`slot_word_to_scalar`, at each
    // `print(w.<attr>)` below) halves of all three in one program:
    //
    //     class Widget:
    //         def __init__(self, flag: bool, ratio: float, label: str) -> None:
    //             self.flag = flag
    //             self.ratio = ratio
    //             self.label = label
    //
    //     w = Widget(True, 2.5, "hi")
    //     print(w.flag)
    //     print(w.ratio)
    //     print(w.label)
    //
    // Expected output verified against `python3` on this exact source.
    let self_ty = instance_ty("Widget");
    let init = MirItem::Function {
        name: "Widget.__init__".to_string(),
        params: vec![
            ("self".to_string(), self_ty.clone()),
            ("flag".to_string(), Ty::Bool),
            ("ratio".to_string(), Ty::Float),
            ("label".to_string(), Ty::Str),
        ],
        return_ty: Ty::None,
        body: vec![
            MirStmt::AttrSet {
                base: MirExpr::Name {
                    name: "self".to_string(),
                    ty: self_ty.clone(),
                },
                slot: 0,
                value: MirExpr::Name {
                    name: "flag".to_string(),
                    ty: Ty::Bool,
                },
            },
            MirStmt::AttrSet {
                base: MirExpr::Name {
                    name: "self".to_string(),
                    ty: self_ty.clone(),
                },
                slot: 1,
                value: MirExpr::Name {
                    name: "ratio".to_string(),
                    ty: Ty::Float,
                },
            },
            MirStmt::AttrSet {
                base: MirExpr::Name {
                    name: "self".to_string(),
                    ty: self_ty.clone(),
                },
                slot: 2,
                value: MirExpr::Name {
                    name: "label".to_string(),
                    ty: Ty::Str,
                },
            },
            MirStmt::Return(None),
        ],
    };
    let mir = MirModule {
        items: vec![
            init,
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "w".to_string(),
                value: MirExpr::Instantiate(Box::new(pycc_mir::InstantiateExpr {
                    ctor: "Widget.__init__".to_string(),
                    class_name: "C".to_string(),
                    slot_names: vec!["s0".to_string(), "s1".to_string(), "s2".to_string()],
                    args: vec![
                        MirExpr::BoolLiteral(true),
                        MirExpr::FloatLiteral(2.5),
                        MirExpr::StringLiteral("hi".to_string()),
                    ],
                    ty: self_ty.clone(),
                })),
            }),
            MirItem::TopLevelStmt(print_expr(MirExpr::AttrGet {
                base: Box::new(MirExpr::Name {
                    name: "w".to_string(),
                    ty: self_ty.clone(),
                }),
                slot: 0,
                ty: Ty::Bool,
            })),
            MirItem::TopLevelStmt(print_expr(MirExpr::AttrGet {
                base: Box::new(MirExpr::Name {
                    name: "w".to_string(),
                    ty: self_ty.clone(),
                }),
                slot: 1,
                ty: Ty::Float,
            })),
            MirItem::TopLevelStmt(print_expr(MirExpr::AttrGet {
                base: Box::new(MirExpr::Name {
                    name: "w".to_string(),
                    ty: self_ty,
                }),
                slot: 2,
                ty: Ty::Str,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("bool_float_str_attribute_slots")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("bool_float_str_attribute_slots.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("bool_float_str_attribute_slots");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"True\n2.5\nhi\n");
}

#[test]
fn a_str_attribute_read_twice_and_then_reassigned_does_not_use_after_free() {
    // D-154 Part 1's own post-merge review finding: the first version
    // of `MirStmt::AttrSet`'s codegen neither incref'd a `str` value
    // read out of an instance attribute (`str_value_is_a_duplicate_
    // reference` had no `MirExpr::AttrGet` arm) nor decref'd a slot's
    // previous `str` occupant before overwriting it. Reading a `str`
    // attribute a second time reliably observed freed memory (the
    // first `print` already decrefs the read pointer to 0, freeing it,
    // since the read was wrongly treated as a fresh, unshared value),
    // and `bool_float_and_str_typed_attribute_slots_round_trip_
    // correctly` above cannot catch this: it reads `w.label` exactly
    // once, which structurally cannot observe a premature free.
    //
    //     class Widget:
    //         def __init__(self, label: str) -> None:
    //             self.label = label
    //         def relabel(self, new_label: str) -> None:
    //             self.label = new_label
    //
    //     w = Widget("hi")
    //     print(w.label)
    //     print(w.label)
    //     w.relabel("bye")
    //     print(w.label)
    //
    // Exercises both halves of the fix in one program: the two
    // `print(w.label)` calls before `relabel` exercise
    // `incref_if_str_duplicate`'s new `AttrGet` arm (without it, the
    // second `print` reads freed memory); `relabel`'s own
    // `self.label = new_label` exercises
    // `decref_str_attr_slot_before_store` (without it, `"hi"`'s
    // `PyStrObj` leaks rather than being released when overwritten --
    // not itself a crash this test can observe, but exercised by the
    // same code path the crash-causing half shares).
    let self_ty = instance_ty("Widget");
    let init = MirItem::Function {
        name: "Widget.__init__".to_string(),
        params: vec![
            ("self".to_string(), self_ty.clone()),
            ("label".to_string(), Ty::Str),
        ],
        return_ty: Ty::None,
        body: vec![
            MirStmt::AttrSet {
                base: MirExpr::Name {
                    name: "self".to_string(),
                    ty: self_ty.clone(),
                },
                slot: 0,
                value: MirExpr::Name {
                    name: "label".to_string(),
                    ty: Ty::Str,
                },
            },
            MirStmt::Return(None),
        ],
    };
    let relabel = MirItem::Function {
        name: "Widget.relabel".to_string(),
        params: vec![
            ("self".to_string(), self_ty.clone()),
            ("new_label".to_string(), Ty::Str),
        ],
        return_ty: Ty::None,
        body: vec![
            MirStmt::AttrSet {
                base: MirExpr::Name {
                    name: "self".to_string(),
                    ty: self_ty.clone(),
                },
                slot: 0,
                value: MirExpr::Name {
                    name: "new_label".to_string(),
                    ty: Ty::Str,
                },
            },
            MirStmt::Return(None),
        ],
    };
    let mir = MirModule {
        items: vec![
            init,
            relabel,
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "w".to_string(),
                value: MirExpr::Instantiate(Box::new(pycc_mir::InstantiateExpr {
                    ctor: "Widget.__init__".to_string(),
                    class_name: "C".to_string(),
                    slot_names: vec!["s0".to_string()],
                    args: vec![MirExpr::StringLiteral("hi".to_string())],
                    ty: self_ty.clone(),
                })),
            }),
            MirItem::TopLevelStmt(print_expr(MirExpr::AttrGet {
                base: Box::new(MirExpr::Name {
                    name: "w".to_string(),
                    ty: self_ty.clone(),
                }),
                slot: 0,
                ty: Ty::Str,
            })),
            MirItem::TopLevelStmt(print_expr(MirExpr::AttrGet {
                base: Box::new(MirExpr::Name {
                    name: "w".to_string(),
                    ty: self_ty.clone(),
                }),
                slot: 0,
                ty: Ty::Str,
            })),
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
                callee: "Widget.relabel".to_string(),
                args: vec![
                    MirExpr::Name {
                        name: "w".to_string(),
                        ty: self_ty.clone(),
                    },
                    MirExpr::StringLiteral("bye".to_string()),
                ],
                ty: Ty::None,
            })),
            MirItem::TopLevelStmt(print_expr(MirExpr::AttrGet {
                base: Box::new(MirExpr::Name {
                    name: "w".to_string(),
                    ty: self_ty,
                }),
                slot: 0,
                ty: Ty::Str,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("str_attribute_read_twice_and_reassigned")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("str_attribute_read_twice_and_reassigned.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("str_attribute_read_twice_and_reassigned");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"hi\nhi\nbye\n");
}

#[test]
#[should_panic(expected = "an instance attribute of type `set[int]` is not supported yet")]
fn slot_word_to_scalar_rejects_an_unsupported_attribute_type() {
    // `pycc_hir::class::init_slot::slot_ty_from_init_rhs` admits only a
    // scalar or a `list[int]`/`dict[str, int]` slot (D-154, #1262), so a
    // `set[T]`-typed slot can never reach this function from real,
    // type-checked source -- hand-built directly, matching this file's
    // own established internal-error-test convention (e.g.
    // `to_numeric_encoded_int_rejects_a_list_operand` above).
    let context = Context::create();
    let module = context.create_module("test");
    let builder = context.create_builder();
    let raw = context.i64_type().const_int(0, false);
    slot_word_to_scalar(
        &context,
        &builder,
        raw,
        &pycc_mir::Ty::Set(Box::new(pycc_mir::Ty::Int)),
    );
    let _ = module;
}

#[test]
#[should_panic(expected = "cannot store this value into an instance attribute slot")]
fn scalar_to_slot_word_rejects_an_unsupported_scalar() {
    // Mirror image of `slot_word_to_scalar_rejects_an_unsupported_attribute_type`
    // above, for the write direction: a `Scalar::Set` can never reach
    // `scalar_to_slot_word` from real, type-checked source either.
    let context = Context::create();
    let module = context.create_module("test");
    let builder = context.create_builder();
    let ptr = context
        .ptr_type(inkwell::AddressSpace::default())
        .const_null();
    scalar_to_slot_word(&context, &builder, Scalar::Set(ptr));
    let _ = module;
}

#[test]
#[should_panic(expected = "did not evaluate to a class instance")]
fn attribute_read_over_a_non_instance_base_panics_with_an_internal_error() {
    // Bypasses `pycc_types::check` (T0043 would reject this) with a
    // hand-built `MirExpr::AttrGet` over an `int`-typed base, matching
    // `pycc_mir`'s own established internal-error-test convention.
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(print_expr(MirExpr::AttrGet {
            base: Box::new(MirExpr::IntLiteral(1)),
            slot: 0,
            ty: Ty::Int,
        }))],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("attribute_read_over_a_non_instance_base")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("attribute_read_over_a_non_instance_base.o");
    let _ = compile_to_object(&mir, &obj_path, None, false);
}

#[test]
#[should_panic(expected = "should have been registered as an ordinary user function")]
fn instantiation_of_an_unregistered_constructor_panics_with_an_internal_error() {
    // Bypasses `pycc_hir::class::lower_class` (which always mangles
    // `__init__` into `HirModule::items`) with a hand-built
    // `MirExpr::Instantiate` naming a constructor no `MirItem::Function`
    // ever declares.
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::Assign {
            target: "g".to_string(),
            value: MirExpr::Instantiate(Box::new(pycc_mir::InstantiateExpr {
                ctor: "Ghost.__init__".to_string(),
                class_name: "C".to_string(),
                slot_names: vec![],
                args: vec![],
                ty: instance_ty("Ghost"),
            })),
        })],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("instantiation_of_an_unregistered_constructor")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("instantiation_of_an_unregistered_constructor.o");
    let _ = compile_to_object(&mir, &obj_path, None, false);
}

#[test]
fn a_function_returning_an_instance_codegens_and_runs() {
    // `Ty::Instance`-typed call/return results are not reachable from
    // this PR's own frontend today (a method's return type is never
    // resolved to `Ty::Instance` by `pycc_types` -- see the plan's own
    // out-of-scope list for class-typed annotations), but
    // `emit_stmt`'s `Return` arm's own `Scalar::Instance` pass-through
    // (D-154) is still real, load-bearing codegen (a future PR that
    // does support `-> Self`/`-> ClassName` needs no further work
    // here) -- exercised directly with hand-built MIR: an `identity`
    // function that takes and returns a `Point`, called once and its
    // result's own attribute printed to prove the same pointer round-
    // trips correctly.
    let self_ty = instance_ty("Point");
    let init = MirItem::Function {
        name: "Point.__init__".to_string(),
        params: vec![
            ("self".to_string(), self_ty.clone()),
            ("x".to_string(), Ty::Int),
        ],
        return_ty: Ty::None,
        body: vec![
            MirStmt::AttrSet {
                base: MirExpr::Name {
                    name: "self".to_string(),
                    ty: self_ty.clone(),
                },
                slot: 0,
                value: MirExpr::Name {
                    name: "x".to_string(),
                    ty: Ty::Int,
                },
            },
            MirStmt::Return(None),
        ],
    };
    let identity = MirItem::Function {
        name: "identity".to_string(),
        params: vec![("self".to_string(), self_ty.clone())],
        return_ty: self_ty.clone(),
        body: vec![MirStmt::Return(Some(MirExpr::Name {
            name: "self".to_string(),
            ty: self_ty.clone(),
        }))],
    };
    let mir = MirModule {
        items: vec![
            init,
            identity,
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "p".to_string(),
                value: MirExpr::Instantiate(Box::new(pycc_mir::InstantiateExpr {
                    ctor: "Point.__init__".to_string(),
                    class_name: "C".to_string(),
                    slot_names: vec!["s0".to_string()],
                    args: vec![MirExpr::IntLiteral(7)],
                    ty: self_ty.clone(),
                })),
            }),
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "p2".to_string(),
                value: MirExpr::Call {
                    callee: "identity".to_string(),
                    args: vec![MirExpr::Name {
                        name: "p".to_string(),
                        ty: self_ty.clone(),
                    }],
                    ty: self_ty.clone(),
                },
            }),
            MirItem::TopLevelStmt(print_expr(MirExpr::AttrGet {
                base: Box::new(MirExpr::Name {
                    name: "p2".to_string(),
                    ty: self_ty,
                }),
                slot: 0,
                ty: Ty::Int,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("a_function_returning_an_instance")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("a_function_returning_an_instance.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("a_function_returning_an_instance");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"7\n");
}

#[test]
fn instantiating_a_class_at_module_scope_with_a_truthiness_check_codegens_and_runs() {
    // `p = Point(1, 2)\nif p:\n    print(1)\n` -- exercises `truthy`'s
    // new `Scalar::Instance` arm (always `True`, this PR's own
    // documented default-object rule, see that arm's doc comment) and
    // `declare_module_globals`'s new `Ty::Instance` arm together.
    let self_ty = instance_ty("Point");
    let init = MirItem::Function {
        name: "Point.__init__".to_string(),
        params: vec![
            ("self".to_string(), self_ty.clone()),
            ("x".to_string(), Ty::Int),
            ("y".to_string(), Ty::Int),
        ],
        return_ty: Ty::None,
        body: vec![
            MirStmt::AttrSet {
                base: MirExpr::Name {
                    name: "self".to_string(),
                    ty: self_ty.clone(),
                },
                slot: 0,
                value: MirExpr::Name {
                    name: "x".to_string(),
                    ty: Ty::Int,
                },
            },
            MirStmt::AttrSet {
                base: MirExpr::Name {
                    name: "self".to_string(),
                    ty: self_ty.clone(),
                },
                slot: 1,
                value: MirExpr::Name {
                    name: "y".to_string(),
                    ty: Ty::Int,
                },
            },
            MirStmt::Return(None),
        ],
    };
    let mir = MirModule {
        items: vec![
            init,
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "p".to_string(),
                value: MirExpr::Instantiate(Box::new(pycc_mir::InstantiateExpr {
                    ctor: "Point.__init__".to_string(),
                    class_name: "C".to_string(),
                    slot_names: vec!["s0".to_string(), "s1".to_string()],
                    args: vec![MirExpr::IntLiteral(1), MirExpr::IntLiteral(2)],
                    ty: self_ty.clone(),
                })),
            }),
            MirItem::TopLevelStmt(MirStmt::If {
                test: MirExpr::Name {
                    name: "p".to_string(),
                    ty: self_ty,
                },
                body: vec![print_expr(MirExpr::IntLiteral(1))],
                orelse: vec![],
            }),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("class_instance_truthiness")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("class_instance_truthiness.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("class_instance_truthiness");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"1\n");
}

#[test]
#[should_panic(
    expected = "pycc_codegen: string conversion of a class instance without `__repr__` is not supported yet"
)]
fn string_conversion_of_a_class_instance_panics_honestly() {
    // Mirrors `string_conversion_of_a_list_value_panics_honestly`
    // above exactly. #378 (PR-18) added `__repr__` support for dataclass
    // instances (the MIR rewrites `print(instance)` to a `__repr__` call
    // before codegen), and since #977 (D-237) `pycc_types` rejects every
    // other instance with `C0001` before lowering, so this arm is
    // reachable only from a hand-built `Scalar::Instance` like the one
    // below -- where it still panics honestly instead of handing a
    // `PyInstanceObj` pointer to a `pycc_rt_*_to_str` function expecting
    // a `PyStrObj`.
    let context = Context::create();
    let (_module, rt) = list_scalar_panic_fixture(&context);
    let builder = context.create_builder();
    let ptr = context
        .ptr_type(inkwell::AddressSpace::default())
        .const_null();
    to_str(&builder, &rt, Scalar::Instance(ptr));
}

#[test]
#[should_panic(expected = "internal error: expected an int-or-bool operand, got instance")]
fn to_numeric_encoded_int_rejects_an_instance_operand() {
    // Mirrors `to_numeric_encoded_int_rejects_a_list_operand` above:
    // `pycc_types`' `numeric_result_type` has no `as_numeric` mapping
    // for `Ty::Instance`, so any arithmetic with a class-instance
    // operand is rejected as `T0021` long before codegen -- this is
    // genuinely defensive, not a reachable feature gap.
    let context = Context::create();
    let ptr = context
        .ptr_type(inkwell::AddressSpace::default())
        .const_null();
    let builder = context.create_builder();
    to_numeric_encoded_int(&context, &builder, Scalar::Instance(ptr));
}

#[test]
#[should_panic(expected = "internal error: expected a numeric operand, got instance")]
fn to_float_rejects_an_instance_operand() {
    let context = Context::create();
    let (_module, rt) = list_scalar_panic_fixture(&context);
    let builder = context.create_builder();
    let ptr = context
        .ptr_type(inkwell::AddressSpace::default())
        .const_null();
    to_float(&context, &builder, &rt, Scalar::Instance(ptr));
}

#[test]
#[should_panic(expected = "pycc_codegen: internal error: range() start did not evaluate to int")]
fn range_operand_to_normalized_int_rejects_an_instance_operand() {
    let context = Context::create();
    let (_module, rt) = list_scalar_panic_fixture(&context);
    let builder = context.create_builder();
    let ptr = context
        .ptr_type(inkwell::AddressSpace::default())
        .const_null();
    range_operand_to_normalized_int(&context, &builder, &rt, Scalar::Instance(ptr), "start");
}

#[test]
fn math_sqrt_call_codegens_and_runs() {
    // `print(math.sqrt(2.0))` end to end through `compile_to_object` --
    // D-136 Task 4's one lowered stdlib function, calling the real
    // libm `sqrt` symbol. Expected output verified against `python3`
    // on this exact source (`math.sqrt(2.0)` -> `1.4142135623730951`).
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(print_expr(MirExpr::Call {
            callee: "math.sqrt".to_string(),
            args: vec![MirExpr::FloatLiteral(2.0)],
            ty: Ty::Float,
        }))],
        class_defs: Vec::new(),
    };
    let dir =
        pycc_scratch::ScratchDir::new("math_sqrt_call").expect("failed to create scratch dir");
    let obj_path = dir.join("math_sqrt_call.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("math_sqrt_call");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"1.4142135623730951\n");
}

#[test]
fn math_pi_codegens_and_runs() {
    // `print(math.pi)` end to end -- D-136 Task 4's compile-time float
    // constant, no runtime call at all. Expected output verified
    // against `python3` on this exact source.
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(print_expr(MirExpr::Name {
            name: "math.pi".to_string(),
            ty: Ty::Float,
        }))],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("math_pi").expect("failed to create scratch dir");
    let obj_path = dir.join("math_pi.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("math_pi");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"3.141592653589793\n");
}

#[test]
fn float_call_codegens_and_runs() {
    // `x = 3\nprint(float(x))\n` end to end through `compile_to_object`
    // -- #181's `float()` hand-recognized builtin, mirroring
    // `dict_literal_construction_codegens_and_runs`'s own shape
    // immediately above. Expected output verified against `python3` on
    // this exact source.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "x".to_string(),
                value: MirExpr::IntLiteral(3),
            }),
            MirItem::TopLevelStmt(print_expr(MirExpr::Call {
                callee: "float".to_string(),
                args: vec![MirExpr::Name {
                    name: "x".to_string(),
                    ty: Ty::Int,
                }],
                ty: Ty::Float,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("float_call").expect("failed to create scratch dir");
    let obj_path = dir.join("float_call.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("float_call");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"3.0\n");
}

#[test]
fn a_user_defined_float_function_codegens_and_runs_instead_of_the_builtin() {
    // Post-merge review finding: `def float(x: int) -> int: return x +
    // 1` was a valid, working program on `main` immediately before this
    // builtin landed -- reproduced directly against a pristine
    // checkout, printing `6`. Without the `user_functions.contains_key`
    // guard, this would silently emit the builtin's own float
    // conversion instead of a real call to the user's function.
    let mir = MirModule {
        items: vec![
            MirItem::Function {
                name: "float".to_string(),
                params: vec![("x".to_string(), Ty::Int)],
                return_ty: Ty::Int,
                body: vec![MirStmt::Return(Some(MirExpr::BinOp {
                    op: BinOpKind::Add,
                    left: Box::new(MirExpr::Name {
                        name: "x".to_string(),
                        ty: Ty::Int,
                    }),
                    right: Box::new(MirExpr::IntLiteral(1)),
                    ty: Ty::Int,
                }))],
            },
            MirItem::TopLevelStmt(print_expr(MirExpr::Call {
                callee: "float".to_string(),
                args: vec![MirExpr::IntLiteral(5)],
                ty: Ty::Int,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("user_defined_float_call")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("user_defined_float_call.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("user_defined_float_call");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"6\n");
}

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
    assert_eq!(output.stdout, b"2\n");
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
    assert_eq!(output.stdout, b"5\n1\n");
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
    assert_eq!(output.stdout, b"2\n");
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
    assert_eq!(output.stdout, b"b\na\n");
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
    assert_eq!(output.stdout, b"z\n1\n1\n");
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
    assert_eq!(output.stdout, b"3\n");
}

#[test]
fn for_x_in_set_iterates_in_first_insertion_order() {
    // `x = {2, 1, 2}\nfor v in x:\n    print(v)\n` end to end -- real
    // `MirStmt::ForSet` iteration codegen (PR-11 Task 9, D-123). `2`
    // printing before `1` (first-insertion order, with the second `2`
    // deduped away rather than moving `2`'s position) is the actual
    // point of this test: `PyIntSetObj`'s own insertion-order guarantee
    // (D-121) surviving through `pycc_rt_int_set_get`.
    //
    // This is pycc's own internal-consistency check, NOT a conformance
    // fixture against CPython: `python3` on this exact source prints
    // `1`/`2` (CPython's own set iteration order for small ints is not
    // insertion order -- small ints hash to themselves, so CPython's
    // hash-table iteration order here happens to be numeric order, not
    // insertion order). Task 10 owns why no conformance fixture makes
    // this same assertion against CPython; this test only pins pycc's
    // own behavior against itself.
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
    assert_eq!(output.stdout, b"2\n1\n");
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
    assert!(ir.contains("call i64 @pycc_rt_int_set_len"), "{ir}");
}

#[test]
#[should_panic(expected = "pycc_codegen: string conversion of a set[T] value is not supported yet")]
fn string_conversion_of_a_set_value_panics_honestly() {
    // The `set[T]` counterpart of `string_conversion_of_a_dict_value_
    // panics_honestly` above, for the identical reason.
    let context = Context::create();
    let (_module, rt) = list_scalar_panic_fixture(&context);
    let builder = context.create_builder();
    let ptr = context
        .ptr_type(inkwell::AddressSpace::default())
        .const_null();
    to_str(&builder, &rt, Scalar::Set(ptr));
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

/// `tuple[int, bool, float]`, the heterogeneous shape most of the
/// tuple fixtures below use.
fn tuple_int_bool_float() -> Ty {
    Ty::Tuple(Box::new(vec![Ty::Int, Ty::Bool, Ty::Float]))
}

/// A hand-built `Scalar::Tuple` for the four direct-call panic tests
/// below. The `list`/`dict`/`set` fixtures all use
/// `ptr_type().const_null()`, which has no tuple analogue -- a tuple is
/// a by-value struct, not a pointer (D-115) -- so this builds an
/// `undef` single-field struct instead. No builder is needed: `undef`
/// is a constant, and every function under test rejects the value
/// before doing anything with its contents.
fn tuple_scalar(context: &Context) -> Scalar<'_> {
    Scalar::Tuple(
        context
            .struct_type(&[context.i64_type().into()], false)
            .get_undef(),
    )
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
    assert_eq!(output.stdout, b"1\nTrue\n2.5\n");
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
    assert_eq!(output.stdout, b"20\n");
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
    assert_eq!(output.stdout, b"8\n");
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
#[should_panic(expected = "pycc_codegen: truthiness of a tuple[...] value is not supported yet")]
fn truthiness_of_a_tuple_value_panics_honestly() {
    // The `tuple[...]` counterpart of `truthiness_of_a_set_value_
    // panics_honestly` above: a real, reachable feature gap, not a
    // defensive arm. `pycc_types` accepts any type in a boolean
    // context, so `if t:` for a tuple local type-checks today; D-116
    // ships no `bool(tuple)` semantics, so an honest panic naming the
    // gap is the correct behavior. Calls `truthy` directly with a
    // hand-built `Scalar::Tuple`, for the identical reason that test
    // gives.
    let context = Context::create();
    let (module, rt) = list_scalar_panic_fixture(&context);
    let builder = context.create_builder();
    truthy(&context, &builder, &module, &rt, tuple_scalar(&context));
}

#[test]
#[should_panic(
    expected = "pycc_codegen: string conversion of a tuple[...] value is not supported yet"
)]
fn string_conversion_of_a_tuple_value_panics_honestly() {
    // The `tuple[...]` counterpart of `string_conversion_of_a_set_
    // value_panics_honestly` above, for the identical reason: `print(t)`
    // and `f"{t}"` both type-check today and both land in `to_str`.
    let context = Context::create();
    let (_module, rt) = list_scalar_panic_fixture(&context);
    let builder = context.create_builder();
    to_str(&builder, &rt, tuple_scalar(&context));
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
#[should_panic(expected = "a tuple element evaluated to a non-int/bool/float value")]
fn a_non_scalar_tuple_element_is_an_internal_error() {
    // `MirExpr::TupleLiteral`'s own defensive arm: `pycc_types`' T0039
    // gate (D-116) admits only int/bool/float elements, so a `str`
    // element cannot come from type-checked source -- only from
    // hand-built MIR like this. Without the arm, a `PyStrObj` pointer
    // would be silently inserted into a struct field whose declared
    // type came from the same `Ty::Str`, storing an unrefcounted
    // duplicate reference this crate has no policy for.
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::Assign {
            target: "t".to_string(),
            value: MirExpr::TupleLiteral(vec![MirExpr::StringLiteral("a".to_string())]),
        })],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("tuple_non_scalar_element")
        .expect("failed to create scratch dir");
    let _ = compile_to_object(&mir, &dir.join("tuple_non_scalar_element.o"), None, false);
}

#[test]
#[should_panic(expected = "a tuple subscript index is not a literal int")]
fn a_tuple_subscript_with_a_non_literal_index_is_an_internal_error() {
    // `pycc_types`' T0040 rejects every non-literal tuple index before
    // codegen, so this is defensive -- but genuinely reachable from
    // hand-built MIR, because the index expression's shape is
    // independent of the base's type: nothing about `base.ty()` being
    // `Ty::Tuple` constrains `index` to be an `IntLiteral`. Fires
    // before `MirExpr::ty()` would raise `pycc_mir`'s own equivalent
    // panic, so the message pinned here is this crate's.
    let mir = MirModule {
        items: vec![MirItem::Function {
            name: "f".to_string(),
            params: vec![
                ("t".to_string(), Ty::Tuple(Box::new(vec![Ty::Int, Ty::Int]))),
                ("i".to_string(), Ty::Int),
            ],
            return_ty: Ty::Int,
            body: vec![MirStmt::Return(Some(MirExpr::Subscript {
                base: Box::new(MirExpr::Name {
                    name: "t".to_string(),
                    ty: Ty::Tuple(Box::new(vec![Ty::Int, Ty::Int])),
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
#[should_panic(expected = "reading a `str`-typed tuple element is not supported yet")]
fn reading_a_non_scalar_tuple_element_is_not_supported() {
    // `MirExpr::Subscript`'s tuple branch mirrors `TupleLiteral`'s own
    // element gate on the way back out: T0039 keeps every element
    // int/bool/float, so a `str` element reaches this only from
    // hand-built MIR. Reachable here (unlike a "base didn't evaluate to
    // a tuple" check, which is why this arm has one and that one
    // deliberately does not) because the element *type* is carried by
    // the base's `Ty`, independently of the extract itself succeeding.
    let element_ty = Ty::Tuple(Box::new(vec![Ty::Str]));
    let mir = MirModule {
        items: vec![MirItem::Function {
            name: "f".to_string(),
            params: vec![("t".to_string(), element_ty.clone())],
            return_ty: Ty::None,
            body: vec![print_expr(MirExpr::Subscript {
                base: Box::new(MirExpr::Name {
                    name: "t".to_string(),
                    ty: element_ty,
                }),
                index: Box::new(MirExpr::IntLiteral(0)),
            })],
        }],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("tuple_non_scalar_element_read")
        .expect("failed to create scratch dir");
    let _ = compile_to_object(
        &mir,
        &dir.join("tuple_non_scalar_element_read.o"),
        None,
        false,
    );
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

// --- `Optional[int]` (D-197, #763, Part 1 of #747) ---------------------

/// `int | None`, the one `Optional` shape real, type-checked source ever
/// constructs in this PR (`T0049` rejects every other inner type).
fn optional_int() -> Ty {
    Ty::Optional(Box::new(Ty::Int))
}

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
#[should_panic(
    expected = "pycc_codegen: string conversion of an Optional[int] value is not supported yet"
)]
fn string_conversion_of_an_optional_value_panics_honestly() {
    // A real, reachable feature gap (not defensive): `pycc_types` places
    // no type restriction on `print`'s argument, so `print(x)` for an
    // `Optional[int]` local type-checks today and lands in `to_str`. This
    // PR ships no `str(Optional[int])` semantics -- see `to_str`'s own
    // `Scalar::Optional` arm doc comment.
    let context = Context::create();
    let (_module, rt) = list_scalar_panic_fixture(&context);
    let builder = context.create_builder();
    to_str(&builder, &rt, optional_scalar(&context));
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
    assert_eq!(output.stdout, b"False\nTrue\n");
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
    assert_eq!(output.stdout, b"True\nFalse\n");
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
    assert_eq!(output.stdout, b"False\nTrue\n");
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
    assert_eq!(output.stdout, b"0\n0\n1\n");
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
    assert_eq!(output.stdout, b"5\n");
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
    assert_eq!(output.stdout, b"9223372036854775807\n");
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
    assert_eq!(output.stdout, b"9223372036854775807\n");
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
    assert_eq!(output.stdout, b"True\n");
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
    assert_eq!(output.stdout, b"2\n");
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
    assert_eq!(output.stdout, b"True\nFalse\n");
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
    assert_eq!(output.stdout, b"False\nTrue\n");
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
    assert_eq!(output.stdout, b"False\nTrue\n");
}

// --- `Optional[float]`/`Optional[bool]` (#809, Part 3 of #747) --------

/// `float | None`, the widened-by-#809 counterpart of `optional_int()`
/// immediately above.
fn optional_float() -> Ty {
    Ty::Optional(Box::new(Ty::Float))
}

/// `bool | None`, the widened-by-#809 counterpart of `optional_int()`
/// immediately above.
fn optional_bool() -> Ty {
    Ty::Optional(Box::new(Ty::Bool))
}

#[test]
fn optional_float_annotated_assignment_constructs_a_present_struct_and_reads_its_payload() {
    // `x: float | None = 5.5` followed by `print(x is None)` and
    // `print(x is not None)` -- the `Optional[float]` counterpart of
    // `optional_int_annotated_assignment_constructs_a_present_struct_and_
    // reads_its_payload` above, proving `declare_module_globals`'s
    // widened `Ty::Optional` arm (this PR's own fix -- the module-scope
    // initializer previously forced every inner type's payload field
    // through `tag_smallint_const`, an LLVM constant type mismatch for
    // `f64`/`i8` fields that crashed the LLVM backend with "invalid
    // number of bytes" for `bool`) and `coerce_scalar_to_type`'s
    // `(inner, coerced)`-paired bare-payload arm both handle a real
    // `float` payload correctly.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "x".to_string(),
                value: MirExpr::OptionalWrap(
                    Box::new(MirExpr::FloatLiteral(5.5)),
                    Box::new(Ty::Float),
                ),
            }),
            MirItem::TopLevelStmt(print_expr(MirExpr::Compare {
                op: pycc_mir::CmpOpKind::Is,
                left: Box::new(MirExpr::Name {
                    name: "x".to_string(),
                    ty: optional_float(),
                }),
                right: Box::new(MirExpr::NoneLiteral),
                ty: Ty::Bool,
            })),
            MirItem::TopLevelStmt(print_expr(MirExpr::Compare {
                op: pycc_mir::CmpOpKind::IsNot,
                left: Box::new(MirExpr::Name {
                    name: "x".to_string(),
                    ty: optional_float(),
                }),
                right: Box::new(MirExpr::NoneLiteral),
                ty: Ty::Bool,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("optional_float_present_is_none")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("optional_float_present_is_none.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("optional_float_present_is_none");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"False\nTrue\n");
}

#[test]
fn optional_bool_annotated_assignment_constructs_a_present_struct_and_reads_its_payload() {
    // The `Optional[bool]` counterpart of the `Optional[float]` test
    // immediately above -- `x: bool | None = True`. This exact shape is
    // what reproduced the module-global-initializer LLVM crash this PR's
    // `declare_module_globals` fix resolves (a `bool`'s `i8` payload field
    // being handed an `i64`-typed `tag_smallint_const` constant).
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "x".to_string(),
                value: MirExpr::OptionalWrap(
                    Box::new(MirExpr::BoolLiteral(true)),
                    Box::new(Ty::Bool),
                ),
            }),
            MirItem::TopLevelStmt(print_expr(MirExpr::Compare {
                op: pycc_mir::CmpOpKind::Is,
                left: Box::new(MirExpr::Name {
                    name: "x".to_string(),
                    ty: optional_bool(),
                }),
                right: Box::new(MirExpr::NoneLiteral),
                ty: Ty::Bool,
            })),
            MirItem::TopLevelStmt(print_expr(MirExpr::Compare {
                op: pycc_mir::CmpOpKind::IsNot,
                left: Box::new(MirExpr::Name {
                    name: "x".to_string(),
                    ty: optional_bool(),
                }),
                right: Box::new(MirExpr::NoneLiteral),
                ty: Ty::Bool,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("optional_bool_present_is_none")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("optional_bool_present_is_none.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("optional_bool_present_is_none");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"False\nTrue\n");
}

#[test]
fn optional_float_annotated_assignment_with_bare_none_constructs_an_absent_struct() {
    // `x: float | None = None` -- the mirror-image case of the present
    // test above, exercising `coerce_scalar_to_type`'s
    // placeholder-to-real-struct-shape branch for a `Ty::Float` inner
    // type, and `default_value_for_type`'s `Ty::Optional` arm recursing
    // into its own `Ty::Float` arm for the placeholder payload.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "x".to_string(),
                value: MirExpr::OptionalWrap(Box::new(MirExpr::NoneLiteral), Box::new(Ty::Float)),
            }),
            MirItem::TopLevelStmt(print_expr(MirExpr::Compare {
                op: pycc_mir::CmpOpKind::Is,
                left: Box::new(MirExpr::Name {
                    name: "x".to_string(),
                    ty: optional_float(),
                }),
                right: Box::new(MirExpr::NoneLiteral),
                ty: Ty::Bool,
            })),
            MirItem::TopLevelStmt(print_expr(MirExpr::Compare {
                op: pycc_mir::CmpOpKind::IsNot,
                left: Box::new(MirExpr::Name {
                    name: "x".to_string(),
                    ty: optional_float(),
                }),
                right: Box::new(MirExpr::NoneLiteral),
                ty: Ty::Bool,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("optional_float_absent_is_none")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("optional_float_absent_is_none.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("optional_float_absent_is_none");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"True\nFalse\n");
}

#[test]
fn optional_bool_annotated_assignment_with_bare_none_constructs_an_absent_struct() {
    // `x: bool | None = None`, the `Ty::Bool` counterpart of the
    // `Optional[float]` absent test immediately above -- this is also
    // the exact module-global shape that reproduced the LLVM "invalid
    // number of bytes" crash (a bare `None` literal reaches
    // `declare_module_globals`'s `Ty::Optional(Ty::Bool)` arm exactly as
    // directly as the present-value case does).
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "x".to_string(),
                value: MirExpr::OptionalWrap(Box::new(MirExpr::NoneLiteral), Box::new(Ty::Bool)),
            }),
            MirItem::TopLevelStmt(print_expr(MirExpr::Compare {
                op: pycc_mir::CmpOpKind::Is,
                left: Box::new(MirExpr::Name {
                    name: "x".to_string(),
                    ty: optional_bool(),
                }),
                right: Box::new(MirExpr::NoneLiteral),
                ty: Ty::Bool,
            })),
            MirItem::TopLevelStmt(print_expr(MirExpr::Compare {
                op: pycc_mir::CmpOpKind::IsNot,
                left: Box::new(MirExpr::Name {
                    name: "x".to_string(),
                    ty: optional_bool(),
                }),
                right: Box::new(MirExpr::NoneLiteral),
                ty: Ty::Bool,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("optional_bool_absent_is_none")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("optional_bool_absent_is_none.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("optional_bool_absent_is_none");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"True\nFalse\n");
}

#[test]
fn optional_float_truthiness_follows_cpython_for_present_and_absent_values() {
    // `if x:` for three `Optional[float]` values -- absent (`None`),
    // present with a falsy payload (`0.0`), and present with a truthy
    // payload (`5.5`) -- exercising `truthy`'s `Scalar::Optional` arm's
    // `FloatValue` dispatch branch (this PR's own fix: previously this
    // arm always treated the payload as `int` and called
    // `pycc_rt_int_truthy` on it, which would either panic or misread an
    // `f64`'s raw bits as a D-141-encoded word).
    fn if_prints_one_else_zero(name: &str, ty: Ty) -> MirItem {
        MirItem::TopLevelStmt(MirStmt::If {
            test: MirExpr::Name {
                name: name.to_string(),
                ty,
            },
            body: vec![print_expr(MirExpr::IntLiteral(1))],
            orelse: vec![print_expr(MirExpr::IntLiteral(0))],
        })
    }
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "a".to_string(),
                value: MirExpr::OptionalWrap(Box::new(MirExpr::NoneLiteral), Box::new(Ty::Float)),
            }),
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "b".to_string(),
                value: MirExpr::OptionalWrap(
                    Box::new(MirExpr::FloatLiteral(0.0)),
                    Box::new(Ty::Float),
                ),
            }),
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "c".to_string(),
                value: MirExpr::OptionalWrap(
                    Box::new(MirExpr::FloatLiteral(5.5)),
                    Box::new(Ty::Float),
                ),
            }),
            if_prints_one_else_zero("a", optional_float()),
            if_prints_one_else_zero("b", optional_float()),
            if_prints_one_else_zero("c", optional_float()),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("optional_float_truthiness")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("optional_float_truthiness.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("optional_float_truthiness");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"0\n0\n1\n");
}

#[test]
fn optional_bool_truthiness_follows_cpython_for_present_and_absent_values() {
    // The `Optional[bool]` counterpart of the `Optional[float]`
    // truthiness test above -- absent (`None`), present-`False`, and
    // present-`True` -- exercising `truthy`'s `Scalar::Optional` arm's
    // plain-`i8` (bit-width-8 `IntValue`) dispatch branch, distinct from
    // both the `f64` branch above and the D-141-encoded-`int` branch
    // `optional_int_truthiness_follows_cpython_for_present_and_absent_
    // values` already covers. Also stands in for the plan's explicit
    // "`x: bool | None = None`" absent-case requirement: the `a` case
    // below is exactly that assignment, and its `False` output proves
    // both the placeholder payload (an uninitialized/mismatched `i8`
    // payload here would still read as some fixed bit pattern, but the
    // present flag alone -- not the payload -- must correctly gate this
    // result to `False`) and the present-flag-driven `Scalar::Optional`
    // truthiness path are both correct for the absent case.
    fn if_prints_one_else_zero(name: &str, ty: Ty) -> MirItem {
        MirItem::TopLevelStmt(MirStmt::If {
            test: MirExpr::Name {
                name: name.to_string(),
                ty,
            },
            body: vec![print_expr(MirExpr::IntLiteral(1))],
            orelse: vec![print_expr(MirExpr::IntLiteral(0))],
        })
    }
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "a".to_string(),
                value: MirExpr::OptionalWrap(Box::new(MirExpr::NoneLiteral), Box::new(Ty::Bool)),
            }),
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "b".to_string(),
                value: MirExpr::OptionalWrap(
                    Box::new(MirExpr::BoolLiteral(false)),
                    Box::new(Ty::Bool),
                ),
            }),
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "c".to_string(),
                value: MirExpr::OptionalWrap(
                    Box::new(MirExpr::BoolLiteral(true)),
                    Box::new(Ty::Bool),
                ),
            }),
            if_prints_one_else_zero("a", optional_bool()),
            if_prints_one_else_zero("b", optional_bool()),
            if_prints_one_else_zero("c", optional_bool()),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("optional_bool_truthiness")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("optional_bool_truthiness.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("optional_bool_truthiness");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"0\n0\n1\n");
}

#[test]
fn optional_float_narrowed_read_of_a_present_value_prints_the_payload() {
    // `x: float | None = 5.5; if x is not None: print(x)` -- the
    // `Optional[float]` counterpart of
    // `optional_int_narrowed_read_of_a_present_smallint_prints_the_
    // payload` above, exercising `MirExpr::OptionalUnwrap`'s codegen arm
    // dispatching to `Scalar::Float(payload.into_float_value())` (this
    // PR's own fix: previously this arm always built `Scalar::Int(payload.
    // into_int_value())` regardless of inner type, which panics when the
    // extracted field is an `f64` `BasicValueEnum`).
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "x".to_string(),
                value: MirExpr::OptionalWrap(
                    Box::new(MirExpr::FloatLiteral(5.5)),
                    Box::new(Ty::Float),
                ),
            }),
            MirItem::TopLevelStmt(MirStmt::If {
                test: MirExpr::Compare {
                    op: pycc_mir::CmpOpKind::IsNot,
                    left: Box::new(MirExpr::Name {
                        name: "x".to_string(),
                        ty: optional_float(),
                    }),
                    right: Box::new(MirExpr::NoneLiteral),
                    ty: Ty::Bool,
                },
                body: vec![print_expr(MirExpr::OptionalUnwrap(
                    Box::new(MirExpr::Name {
                        name: "x".to_string(),
                        ty: optional_float(),
                    }),
                    Box::new(Ty::Float),
                ))],
                orelse: vec![],
            }),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("optional_float_narrowed")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("optional_float_narrowed.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("optional_float_narrowed");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"5.5\n");
}

#[test]
fn optional_bool_narrowed_read_of_a_present_value_prints_the_payload() {
    // `x: bool | None = True; if x is not None: print(x)` -- the
    // `Optional[bool]` counterpart of the `Optional[float]` narrowed-read
    // test above, exercising `OptionalUnwrap`'s
    // `Scalar::Bool(payload.into_int_value())` dispatch branch.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "x".to_string(),
                value: MirExpr::OptionalWrap(
                    Box::new(MirExpr::BoolLiteral(true)),
                    Box::new(Ty::Bool),
                ),
            }),
            MirItem::TopLevelStmt(MirStmt::If {
                test: MirExpr::Compare {
                    op: pycc_mir::CmpOpKind::IsNot,
                    left: Box::new(MirExpr::Name {
                        name: "x".to_string(),
                        ty: optional_bool(),
                    }),
                    right: Box::new(MirExpr::NoneLiteral),
                    ty: Ty::Bool,
                },
                body: vec![print_expr(MirExpr::OptionalUnwrap(
                    Box::new(MirExpr::Name {
                        name: "x".to_string(),
                        ty: optional_bool(),
                    }),
                    Box::new(Ty::Bool),
                ))],
                orelse: vec![],
            }),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("optional_bool_narrowed")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("optional_bool_narrowed.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("optional_bool_narrowed");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"True\n");
}

#[test]
fn an_optional_bool_function_that_raises_before_returning_still_produces_a_valid_default() {
    // The `Ty::Bool` counterpart of `an_optional_int_function_that_
    // raises_before_returning_still_produces_a_valid_default` above: a
    // function declared to return `Optional[bool]` raises mid-body
    // instead of returning normally, exercising `default_value_for_
    // type`'s `Ty::Optional` arm recursing into its `Ty::Bool` arm
    // (`context.i8_type().const_zero()`) for the exceptional-exit
    // placeholder, rather than the `Ty::Int`-specific
    // `tag_smallint_const` branch.
    let mir = MirModule {
        items: vec![
            MirItem::Function {
                name: "g".to_string(),
                params: vec![("x".to_string(), Ty::Int)],
                return_ty: optional_bool(),
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
                        ty: optional_bool(),
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
    let dir = pycc_scratch::ScratchDir::new("optional_bool_exceptional_exit")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("optional_bool_exceptional_exit.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("optional_bool_exceptional_exit");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"2\n");
}

#[test]
fn optional_bool_none_placeholder_and_real_absent_value_are_the_same_llvm_struct_type() {
    // #809's risk-log finding: `Optional[bool]`'s real representation is
    // an anonymous LLVM struct `{ i8, i8 }` (`ty_to_basic_type`'s
    // `Ty::Optional` arm, recursing into its own `Ty::Bool` arm for field
    // 0), and `MirExpr::NoneLiteral`'s own placeholder struct
    // (`coerce_scalar_to_type`'s `bare` arm building `{ i8, i8 }` before
    // any target type is known) has the *exact same* field-type list.
    // LLVM literal (non-identified) struct types are uniqued per-`Context`
    // by field-type list, so these two are not merely equal in shape --
    // they are the same `StructType` value. This test pins that finding
    // directly: it proves the collision is real, harmless (both sides are
    // built by `ty_to_basic_type`, and every `coerce_scalar_to_type` call
    // site re-derives the target type independently rather than relying
    // on the LLVM type alone to disambiguate `bool` from any other
    // `{i8,i8}`-shaped payload), and requires no `coerce_scalar_to_type`
    // fix -- discrimination always happens on the requested `Ty`, not on
    // introspecting the `StructValue`'s LLVM type.
    let context = Context::create();
    let bool_optional_struct_ty = ty_to_basic_type(&context, optional_bool()).into_struct_type();
    let none_placeholder_struct_ty =
        context.struct_type(&[context.i8_type().into(), context.i8_type().into()], false);
    assert_eq!(bool_optional_struct_ty, none_placeholder_struct_ty);
}

#[test]
fn optional_bool_absent_value_truthiness_and_narrowed_unwrap_are_both_correct() {
    // The empirical companion to the struct-type-collision test above:
    // proves that despite `Optional[bool]`'s real `{i8,i8}` shape and the
    // bare-`None` placeholder's `{i8,i8}` shape being the literal same
    // `StructType`, an absent `Optional[bool]` value still behaves
    // correctly end to end -- `x: bool | None = None` is falsy (`truthy`
    // reads the present flag, field 1, not distinguishing the collided
    // type), and narrowing with `is not None` correctly does NOT enter the
    // unwrap branch (proving the collision causes no false "present"
    // reading). Prints only `"0\n"`: the `if x:` in the `else` branch, and
    // no output at all from the `is not None` guard's body.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "x".to_string(),
                value: MirExpr::OptionalWrap(Box::new(MirExpr::NoneLiteral), Box::new(Ty::Bool)),
            }),
            MirItem::TopLevelStmt(MirStmt::If {
                test: MirExpr::Name {
                    name: "x".to_string(),
                    ty: optional_bool(),
                },
                body: vec![print_expr(MirExpr::IntLiteral(1))],
                orelse: vec![print_expr(MirExpr::IntLiteral(0))],
            }),
            MirItem::TopLevelStmt(MirStmt::If {
                test: MirExpr::Compare {
                    op: pycc_mir::CmpOpKind::IsNot,
                    left: Box::new(MirExpr::Name {
                        name: "x".to_string(),
                        ty: optional_bool(),
                    }),
                    right: Box::new(MirExpr::NoneLiteral),
                    ty: Ty::Bool,
                },
                body: vec![print_expr(MirExpr::OptionalUnwrap(
                    Box::new(MirExpr::Name {
                        name: "x".to_string(),
                        ty: optional_bool(),
                    }),
                    Box::new(Ty::Bool),
                ))],
                orelse: vec![],
            }),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("optional_bool_absent_end_to_end")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("optional_bool_absent_end_to_end.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("optional_bool_absent_end_to_end");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"0\n");
}

#[test]
#[should_panic(
    expected = "internal error: an Optional[int|float|bool] assignment's payload did not evaluate to int, float, or bool"
)]
fn coerce_scalar_to_type_rejects_a_non_int_payload_widening_into_optional_int() {
    // `pycc_hir::func`'s `T0049` gate rejects every `Optional[T]`
    // annotation for `T` outside `{int, float, bool}` before this value
    // could ever be constructed from real source (D-197, #763, Part 1 of
    // #747; widened by #809) -- this pins `coerce_scalar_to_type`'s own
    // defensive backstop directly, via a hand-built `Scalar::Float`
    // targeting `Optional[int]` specifically (not `Optional[float]`,
    // which is a real, accepted widening as of #809): the function's own
    // `(_, scalar) => scalar` catch-all passes it through its
    // `Ty::Int`-targeted recursive call unchanged (there being no `float
    // -> int` widening arm), landing back here as a payload that is
    // `Scalar::Float` while the declared inner type is `Ty::Int` -- a
    // mismatch #809's `(inner.as_ref(), coerced)` match rejects.
    let context = Context::create();
    let builder = context.create_builder();
    coerce_scalar_to_type(
        &context,
        &builder,
        Scalar::Float(context.f64_type().const_float(1.0)),
        pycc_mir::Ty::Optional(Box::new(Ty::Int)),
    );
}

#[test]
#[should_panic(
    expected = "internal error: an Optional[_] placeholder targeted an unsupported inner type"
)]
fn coerce_scalar_to_type_rejects_a_none_placeholder_targeting_an_unsupported_inner_type() {
    // The sibling defensive backstop to the test immediately above, this
    // time for the *placeholder* branch (`v.get_type() != struct_ty`) added
    // by this PR's own fix: `T0049` never admits `Optional[str]`, so a
    // hand-built call reaching this arm directly (bypassing `T0049`
    // entirely, the same way the sibling test above does) proves the
    // `match inner.as_ref() { Ty::Int | Ty::Float | Ty::Bool => .., other
    // => panic!(..) }` catch-all is reachable and fires for any inner type
    // outside the three #809/#763 admit.
    let context = Context::create();
    let builder = context.create_builder();
    let none_placeholder = context
        .struct_type(&[context.i8_type().into(), context.i8_type().into()], false)
        .const_zero();
    coerce_scalar_to_type(
        &context,
        &builder,
        Scalar::Optional(none_placeholder),
        pycc_mir::Ty::Optional(Box::new(Ty::Str)),
    );
}

#[test]
fn coerce_scalar_to_type_builds_an_i8_zero_placeholder_payload_for_a_mismatched_optional_bool_struct()
 {
    // `Ty::Bool`'s real `Optional[bool]` struct shape (`{ i8, i8 }`)
    // happens to be the literal same `StructType` as the bare-`None`
    // placeholder's own fixed `{ i8, i8 }` shape (see
    // `optional_bool_none_placeholder_and_real_absent_value_are_the_same_
    // llvm_struct_type`), so no value ever built by real codegen takes
    // this arm's `else` branch for `inner == Ty::Bool` -- every real
    // `Optional[bool]` value's `v.get_type() == struct_ty` check is
    // always true. `match inner.as_ref() { .. Ty::Bool => .. }`'s own line
    // is therefore otherwise unreachable and untestable through any real
    // MIR program. Pin it directly with a hand-built, deliberately
    // mismatched `Scalar::Optional` (an `{ i64, i8 }`-shaped struct value,
    // never producible for a `Ty::Bool` inner type by real codegen) so
    // this arm still has direct coverage and is proven to build a
    // correctly-typed `i8` zero rather than the sibling `Ty::Int` arm's
    // `i64` constant, mirroring `truthy_of_an_optional_with_a_pointer_
    // payload_panics_as_an_internal_error` above's use of a positioned
    // builder to exercise a codegen function directly.
    let context = Context::create();
    let module = context.create_module("test");
    let builder = context.create_builder();
    let fn_type = context.void_type().fn_type(&[], false);
    let f = module.add_function("f", fn_type, None);
    let block = context.append_basic_block(f, "entry");
    builder.position_at_end(block);
    let mismatched = ty_to_basic_type(&context, pycc_mir::Ty::Optional(Box::new(Ty::Int)))
        .into_struct_type()
        .get_undef();
    let result = coerce_scalar_to_type(
        &context,
        &builder,
        Scalar::Optional(mismatched),
        pycc_mir::Ty::Optional(Box::new(Ty::Bool)),
    );
    let Scalar::Optional(v) = result else {
        panic!("coerce_scalar_to_type should return Scalar::Optional for an Optional target");
    };
    assert_eq!(
        v.get_type(),
        ty_to_basic_type(&context, optional_bool()).into_struct_type()
    );
}

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
    assert_eq!(output.stdout, b"1\n");
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
    assert_eq!(output.stdout, b"1\n");
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
    assert_eq!(output.stdout, b"1\n2\n3\n");
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
    assert_eq!(output.stdout, b"1\n2\n3\n");
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
    assert_eq!(output.stdout, b"4\n");
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
    assert_eq!(output.stdout, b"3\n");
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
    assert_eq!(output.stdout, b"True\n");
}

#[test]
fn compiles_a_none_typed_parameter_and_value_return() {
    // `def f(x: None) -> None: return x` -- `None` returns stay LLVM
    // `void`, while the parameter and name read use the canonical i8
    // unit carrier. This is valid frontend input and must not fail only
    // when codegen declares the function.
    let mir = MirModule {
        items: vec![MirItem::Function {
            name: "f".to_string(),
            params: vec![("x".to_string(), Ty::None)],
            return_ty: Ty::None,
            body: vec![MirStmt::Return(Some(MirExpr::Name {
                name: "x".to_string(),
                ty: Ty::None,
            }))],
        }],
        class_defs: Vec::new(),
    };
    let dir =
        pycc_scratch::ScratchDir::new("none_param_compiles").expect("failed to create scratch dir");
    let obj_path = dir.join("none_param_compiles.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
}

#[test]
fn compiles_a_function_with_a_float_parameter_and_float_return_value() {
    // `def f(x: float) -> float: return x` ; `y = f(1.5)` -- exercises
    // `ty_to_basic_type`'s new `Ty::Float` arm (both the parameter and
    // return-type positions), `build_call_to`'s argument-marshaling
    // match's `Scalar::Float` arm, `emit_expr`'s `Call` arm's
    // `Ty::Float`-result match arm, and `emit_stmt`'s `Return` arm's
    // `Scalar::Float` match arm -- every `float`-typed position Task 5
    // could not support yet.
    let mir = MirModule {
        items: vec![
            MirItem::Function {
                name: "f".to_string(),
                params: vec![("x".to_string(), Ty::Float)],
                return_ty: Ty::Float,
                body: vec![MirStmt::Return(Some(MirExpr::Name {
                    name: "x".to_string(),
                    ty: Ty::Float,
                }))],
            },
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "y".to_string(),
                value: MirExpr::Call {
                    callee: "f".to_string(),
                    args: vec![MirExpr::FloatLiteral(1.5)],
                    ty: Ty::Float,
                },
            }),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("float_param_and_return")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("float_param_and_return.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
}

#[test]
fn a_return_inside_a_for_range_body_returns_immediately_without_looping() {
    // `def first_of_range() -> int:\n    for i in range(0, 5, 1):\n
    // return i\n    return -1` ; `print(first_of_range())` -- the
    // trailing `return -1` is unreachable in practice (every
    // legitimate call actually returns from inside the loop on its
    // first iteration) but keeps this hand-built MIR shape well-formed
    // for a non-`None`-returning function (real `pycc_types` would
    // likely reject a bare `for` loop as satisfying T0024's
    // definite-return check on its own, since a `for` loop is never
    // assumed to execute at least once). Proves `ForRange`'s own
    // inline terminator-safety guard (this task's re-add, see the
    // `ForRange` arm's own comment) correctly skips the
    // increment-and-branch-back the moment `body`'s `Return` already
    // terminates `body_bb` -- without it, this would try to build a
    // second terminator onto an already-terminated block, which
    // `module.verify()` would (correctly) reject. Prints "0", not
    // "0\n1\n2\n3\n4\n" (which would mean the loop kept running) or a
    // crash.
    let mir = MirModule {
        items: vec![
            MirItem::Function {
                name: "first_of_range".to_string(),
                params: vec![],
                return_ty: Ty::Int,
                body: vec![
                    MirStmt::ForRange {
                        var: "i".to_string(),
                        start: MirExpr::IntLiteral(0),
                        stop: MirExpr::IntLiteral(5),
                        step: MirExpr::IntLiteral(1),
                        body: vec![MirStmt::Return(Some(MirExpr::Name {
                            name: "i".to_string(),
                            ty: Ty::Int,
                        }))],
                    },
                    MirStmt::Return(Some(MirExpr::IntLiteral(-1))),
                ],
            },
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
                callee: "print".to_string(),
                args: vec![MirExpr::Call {
                    callee: "first_of_range".to_string(),
                    args: vec![],
                    ty: Ty::Int,
                }],
                ty: Ty::None,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("for_range_return_inside_body")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("for_range_return_inside_body.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("for_range_return_inside_body");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"0\n");
}

#[test]
fn compiles_a_function_call_with_a_bool_argument() {
    // `def identity_bool(b: bool) -> bool: return b` ;
    // `x = identity_bool(True)` -- exercises `build_call_to`'s
    // `Scalar::Bool` argument-marshalling arm (every other
    // function-call test in this file passes only `int` arguments).
    let mir = MirModule {
        items: vec![
            MirItem::Function {
                name: "identity_bool".to_string(),
                params: vec![("b".to_string(), Ty::Bool)],
                return_ty: Ty::Bool,
                body: vec![MirStmt::Return(Some(MirExpr::Name {
                    name: "b".to_string(),
                    ty: Ty::Bool,
                }))],
            },
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "x".to_string(),
                value: MirExpr::Call {
                    callee: "identity_bool".to_string(),
                    args: vec![MirExpr::BoolLiteral(true)],
                    ty: Ty::Bool,
                },
            }),
        ],
        class_defs: Vec::new(),
    };
    let dir =
        pycc_scratch::ScratchDir::new("call_with_bool_arg").expect("failed to create scratch dir");
    let obj_path = dir.join("call_with_bool_arg.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
}

#[test]
fn a_bool_argument_widens_to_int_when_the_parameter_is_declared_int() {
    // `def f(x: int) -> None: print(x)` ; `f(True)` -- `bool` is an
    // `int` subtype (`pycc_types::is_assignable`), so this is valid,
    // type-checked v0.1 Python. `build_call_to` previously passed the
    // evaluated `Scalar::Bool` (an `i8`) straight through with no
    // widening, so the built call's argument type didn't match `f`'s
    // declared `i64` parameter -- `module.verify()` rejected the IR.
    // `x` is statically `int`-typed, but D-141 preserves and prints the
    // source object's `True` identity.
    let mir = MirModule {
        items: vec![
            MirItem::Function {
                name: "f".to_string(),
                params: vec![("x".to_string(), Ty::Int)],
                return_ty: Ty::None,
                body: vec![MirStmt::ExprStmt(MirExpr::Call {
                    callee: "print".to_string(),
                    args: vec![MirExpr::Name {
                        name: "x".to_string(),
                        ty: Ty::Int,
                    }],
                    ty: Ty::None,
                })],
            },
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
                callee: "f".to_string(),
                args: vec![MirExpr::BoolLiteral(true)],
                ty: Ty::None,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("bool_arg_widens_to_int")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("bool_arg_widens_to_int.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("bool_arg_widens_to_int");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"True\n");
}

#[test]
fn a_bool_return_value_widens_to_int_when_the_function_declares_int() {
    // `def f() -> int: return True` ; `print(f())` -- same
    // `bool`-is-`int` widening as the argument case above, but for
    // `MirStmt::Return`'s own value-emission arm: it previously mapped
    // the returned `Scalar::Bool` straight to a `BasicValueEnum` with no
    // widening, so the built `ret` instruction's operand type didn't
    // match `f`'s declared `i64` return type -- `module.verify()`
    // rejected the IR. D-141 now preserves and prints `True`.
    let mir = MirModule {
        items: vec![
            MirItem::Function {
                name: "f".to_string(),
                params: vec![],
                return_ty: Ty::Int,
                body: vec![MirStmt::Return(Some(MirExpr::BoolLiteral(true)))],
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
    let dir = pycc_scratch::ScratchDir::new("bool_return_widens_to_int")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("bool_return_widens_to_int.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("bool_return_widens_to_int");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"True\n");
}

#[test]
fn reassigning_an_int_local_with_a_bool_value_widens_it_to_int() {
    // `x = 5; x = True; print(x)` -- `pycc_types::check_assignment`'s
    // sticky-first-type rule (T0023) keeps `x` typed `int` throughout
    // (a later `bool` value is `is_assignable` into it, never rebinding
    // it), so `pycc_mir` reports every `Name("x")` read as `Ty::Int`.
    // `emit_assign` previously reused the first assignment's `i64`
    // alloca but stored the second assignment's raw `Scalar::Bool` (an
    // `i8`) into it verbatim -- an `i8` store into an `i64`-sized slot,
    // followed by an `i64` load expecting a full encoded int word.
    // D-141 stores the `True` marker rather than ordinary integer `1`.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "x".to_string(),
                value: MirExpr::IntLiteral(5),
            }),
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "x".to_string(),
                value: MirExpr::BoolLiteral(true),
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
    let dir = pycc_scratch::ScratchDir::new("reassign_bool_into_int")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("reassign_bool_into_int.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("reassign_bool_into_int");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"True\n");
}

#[test]
fn an_explicit_int_boundary_preserves_bool_identity_in_an_int_slot() {
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "x".to_string(),
                value: MirExpr::IntBoundary(Box::new(MirExpr::BoolLiteral(true))),
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
    let dir = pycc_scratch::ScratchDir::new("int_boundary_bool_identity")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("int_boundary_bool_identity.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("int_boundary_bool_identity");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"True\n");
}

#[test]
fn bool_identity_survives_nested_int_forwarding_and_fstring_formatting() {
    let mir = MirModule {
        items: vec![
            MirItem::Function {
                name: "source".to_string(),
                params: vec![],
                return_ty: Ty::Int,
                body: vec![MirStmt::Return(Some(MirExpr::BoolLiteral(true)))],
            },
            MirItem::Function {
                name: "forward".to_string(),
                params: vec![("x".to_string(), Ty::Int)],
                return_ty: Ty::Int,
                body: vec![MirStmt::Return(Some(MirExpr::Name {
                    name: "x".to_string(),
                    ty: Ty::Int,
                }))],
            },
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
                callee: "print".to_string(),
                args: vec![MirExpr::FString(vec![
                    MirFStringPart::Literal("value=".to_string()),
                    MirFStringPart::Interpolation(Box::new(MirExpr::Call {
                        callee: "forward".to_string(),
                        args: vec![MirExpr::Call {
                            callee: "source".to_string(),
                            args: vec![],
                            ty: Ty::Int,
                        }],
                        ty: Ty::Int,
                    })),
                ])],
                ty: Ty::None,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("bool_identity_nested_fstring")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("bool_identity_nested_fstring.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("bool_identity_nested_fstring");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"value=True\n");
}

#[test]
fn a_bool_dict_value_round_trips_with_identity_preserved() {
    let dict_ty = Ty::Dict(Box::new((Ty::Str, Ty::Bool)));
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "d".to_string(),
                value: MirExpr::DictLiteral(vec![(
                    MirExpr::StringLiteral("k".to_string()),
                    MirExpr::BoolLiteral(true),
                )]),
            }),
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
                callee: "print".to_string(),
                args: vec![MirExpr::DictGet {
                    dict: Box::new(MirExpr::Name {
                        name: "d".to_string(),
                        ty: dict_ty,
                    }),
                    key: Box::new(MirExpr::StringLiteral("k".to_string())),
                }],
                ty: Ty::None,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("bool_dict_value_identity")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("bool_dict_value_identity.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("bool_dict_value_identity");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"True\n");
}

#[test]
#[should_panic(expected = "using print()'s result as a nested expression is not supported yet")]
fn nesting_a_print_call_inside_another_expression_is_not_yet_supported() {
    // `x = print(1)` remains D-072's explicit exception: ordinary
    // materializable `None` results now support assignment storage,
    // but `print()`'s own result is still rejected as a nested
    // expression. `emit_stmt`'s own `print`-call arm builds a
    // `pycc_rt_int_print` call directly and never routes the outer
    // `print(...)` itself through `emit_expr` -- so the only way a
    // `print` call can reach `emit_expr`'s `Call` arm at all is nested
    // one level deeper than that, inside another expression, exercised
    // here via `Assign`.
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::Assign {
            target: "x".to_string(),
            value: MirExpr::Call {
                callee: "print".to_string(),
                args: vec![MirExpr::IntLiteral(1)],
                ty: Ty::None,
            },
        })],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("print_result_nested_panics")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("print_result_nested_panics.o");
    let _ = compile_to_object(&mir, &obj_path, None, false);
}

#[test]
#[should_panic(expected = "has no local slot")]
fn referencing_a_name_with_no_bound_local_is_an_internal_error() {
    // Real `pycc_types` already rejects any reference to an undefined
    // name (T0021) long before codegen runs, so this is hand-built
    // malformed MIR exercising `emit_expr`'s `Name` arm's own
    // defensive backstop directly. This panic's coverage used to come
    // from this file's earlier (Task 3) `referencing_a_function_
    // parameter_is_not_yet_supported` test, which happened to hit it
    // via an *unbound* parameter; now that Task 5 binds parameters for
    // real, that test was rewritten into a positive one (see
    // `a_function_parameter_can_be_reassigned_read_back_and_printed` above),
    // leaving this exact check's own coverage to this more direct,
    // dedicated test instead.
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Name {
            name: "never_bound".to_string(),
            ty: Ty::Int,
        }))],
        class_defs: Vec::new(),
    };
    let dir =
        pycc_scratch::ScratchDir::new("unbound_name_panics").expect("failed to create scratch dir");
    let obj_path = dir.join("unbound_name_panics.o");
    let _ = compile_to_object(&mir, &obj_path, None, false);
}

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

#[test]
fn compiles_an_f_string_interpolating_an_int_between_literal_parts() {
    // `x = 5; s = f"n={x}!"` -- `s` would hold `"n=5!"`.
    //
    // Deviations from the task brief, both in this test:
    //
    // 1. The brief's own version wrote `pycc_hir::Ty::Int`/`pycc_hir::
    //    Ty::None` -- but `pycc_hir` is not a dependency of this crate
    //    (only `pycc_mir` is, per `Cargo.toml`), and Rust doesn't
    //    resolve an indirect crate's name from a `pub use` re-export
    //    alone (`pycc_mir::Ty` is the exact same type as `pycc_hir::
    //    Ty`, but the bare path `pycc_hir::` itself isn't in scope
    //    here). Fixed to use the plain `Ty` already imported from
    //    `pycc_mir` at this module's own top (`use pycc_mir::{BinOpKind,
    //    CmpOpKind, MirExpr, MirItem, MirModule, MirStmt, Ty};`),
    //    matching every other test in this file.
    //
    // 2. The brief's own version wrapped the f-string in `print(...)`
    //    instead of a plain `Assign`. `emit_stmt`'s own `print()` arm
    //    (a few hundred lines above) only accepts a *single, `Ty::Int`-
    //    typed* argument today -- any other shape, including a single
    //    `Ty::Str`-typed argument, falls through to its own documented
    //    "this print() argument shape is not supported yet (multi-arg /
    //    non-int print lands in Task 10)" panic, confirmed empirically:
    //    with the brief's own `print(f"n={x}!")` shape, this test
    //    failed with exactly that panic instead of proving f-string
    //    codegen itself works. Wiring a `str`-typed argument into
    //    `print` is explicitly Task 10's job (the crate root's own doc
    //    comment on `emit_stmt`, and the plan's own Task 10 scope) --
    //    implementing it here would reach into a later task's scope.
    //    Changed to a plain `Assign` (`s = f"..."`), the same shape
    //    already used by this brief's own next two tests below, so this
    //    test actually exercises `MirExpr::FString`'s own codegen
    //    without depending on unfinished `print` dispatch.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "x".to_string(),
                value: MirExpr::IntLiteral(5),
            }),
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "s".to_string(),
                value: MirExpr::FString(vec![
                    pycc_mir::MirFStringPart::Literal("n=".to_string()),
                    pycc_mir::MirFStringPart::Interpolation(Box::new(MirExpr::Name {
                        name: "x".to_string(),
                        ty: Ty::Int,
                    })),
                    pycc_mir::MirFStringPart::Literal("!".to_string()),
                ]),
            }),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("fstring_int").expect("failed to create scratch dir");
    let obj_path = dir.join("fstring_int.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
}

#[test]
fn compiles_an_f_string_interpolating_a_float_and_a_bool() {
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::Assign {
            target: "s".to_string(),
            value: MirExpr::FString(vec![
                pycc_mir::MirFStringPart::Interpolation(Box::new(MirExpr::FloatLiteral(2.5))),
                pycc_mir::MirFStringPart::Literal(" ".to_string()),
                pycc_mir::MirFStringPart::Interpolation(Box::new(MirExpr::BoolLiteral(true))),
            ]),
        })],
        class_defs: Vec::new(),
    };
    let dir =
        pycc_scratch::ScratchDir::new("fstring_float_bool").expect("failed to create scratch dir");
    let obj_path = dir.join("fstring_float_bool.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
}

#[test]
fn compiles_an_f_string_interpolating_a_none_returning_call_as_none() {
    let mir = MirModule {
        items: vec![
            MirItem::Function {
                name: "returns_none".to_string(),
                params: vec![],
                return_ty: Ty::None,
                body: vec![MirStmt::Return(None)],
            },
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "text".to_string(),
                value: MirExpr::FString(vec![pycc_mir::MirFStringPart::Interpolation(Box::new(
                    MirExpr::Call {
                        callee: "returns_none".to_string(),
                        args: vec![],
                        ty: Ty::None,
                    },
                ))]),
            }),
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
                callee: "print".to_string(),
                args: vec![MirExpr::Name {
                    name: "text".to_string(),
                    ty: Ty::Str,
                }],
                ty: Ty::None,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("fstring_none_call_as_none")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("fstring_none_call_as_none.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("fstring_none_call_as_none");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"None\n");
}

#[test]
fn compiles_an_f_string_with_only_literal_parts() {
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::Assign {
            target: "s".to_string(),
            value: MirExpr::FString(vec![pycc_mir::MirFStringPart::Literal(
                "no interpolation".to_string(),
            )]),
        })],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("fstring_literal_only")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("fstring_literal_only.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
}

#[test]
fn compiles_an_f_string_interpolating_an_existing_str_value() {
    // `s = "hi"; t = f"{s} there"` -- added beyond the task brief's own
    // three tests above: none of those ever interpolates an
    // already-`str`-typed value, so `to_str`'s `Scalar::Str` passthrough
    // arm (`return v` -- no `pycc_rt_*_to_str` conversion call at all)
    // would otherwise never execute, an uncovered region under this
    // project's 100%-line-and-region coverage gate (D-014). Also
    // exercises `incref_if_str_duplicate`'s true branch for a bare
    // `Name` read inside an interpolation (needed so the f-string's own
    // final decref of every non-literal part doesn't underflow `s`'s
    // refcount below what its own binding still owns).
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "s".to_string(),
                value: MirExpr::StringLiteral("hi".to_string()),
            }),
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "t".to_string(),
                value: MirExpr::FString(vec![
                    pycc_mir::MirFStringPart::Interpolation(Box::new(MirExpr::Name {
                        name: "s".to_string(),
                        ty: Ty::Str,
                    })),
                    pycc_mir::MirFStringPart::Literal(" there".to_string()),
                ]),
            }),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("fstring_str_passthrough")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("fstring_str_passthrough.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
}

#[test]
fn interpolating_a_none_returning_call_in_an_f_string_renders_none_not_false() {
    // `def f() -> None:\n    return` ; `s = f"got: {f()}"` ; `print(s)`
    // -- must print `"got: None"`. Before this fix, a `None`-typed
    // interpolation's placeholder `Scalar::Bool(0)` (see `emit_expr`'s
    // `Call` arm doc comment) flowed straight into `to_str`, which has
    // no way to tell it apart from a genuine `False` -- rendering
    // `"got: False"` instead.
    let mir = MirModule {
        items: vec![
            MirItem::Function {
                name: "f".to_string(),
                params: vec![],
                return_ty: Ty::None,
                body: vec![MirStmt::Return(None)],
            },
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "s".to_string(),
                value: MirExpr::FString(vec![
                    pycc_mir::MirFStringPart::Literal("got: ".to_string()),
                    pycc_mir::MirFStringPart::Interpolation(Box::new(MirExpr::Call {
                        callee: "f".to_string(),
                        args: vec![],
                        ty: Ty::None,
                    })),
                ]),
            }),
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
                callee: "print".to_string(),
                args: vec![MirExpr::Name {
                    name: "s".to_string(),
                    ty: Ty::Str,
                }],
                ty: Ty::None,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("fstring_none_call_renders_none")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("fstring_none_call_renders_none.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("fstring_none_call_renders_none");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"got: None\n");
}

#[test]
fn interpolating_a_none_typed_parameter_renders_none() {
    // `render(source())` exercises the same unit carrier through an
    // f-string interpolation of a parameter name rather than `print`'s
    // dedicated `None` path.
    let mir = MirModule {
        items: vec![
            MirItem::Function {
                name: "source".to_string(),
                params: vec![],
                return_ty: Ty::None,
                body: vec![MirStmt::Return(None)],
            },
            MirItem::Function {
                name: "render".to_string(),
                params: vec![("value".to_string(), Ty::None)],
                return_ty: Ty::Str,
                body: vec![MirStmt::Return(Some(MirExpr::FString(vec![
                    pycc_mir::MirFStringPart::Interpolation(Box::new(MirExpr::Name {
                        name: "value".to_string(),
                        ty: Ty::None,
                    })),
                ])))],
            },
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
                callee: "print".to_string(),
                args: vec![MirExpr::Call {
                    callee: "render".to_string(),
                    args: vec![MirExpr::Call {
                        callee: "source".to_string(),
                        args: vec![],
                        ty: Ty::None,
                    }],
                    ty: Ty::Str,
                }],
                ty: Ty::None,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("fstring_none_typed_parameter")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("fstring_none_typed_parameter.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("fstring_none_typed_parameter");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"None\n");
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
    assert_eq!(output.stdout, format!("{expected}\n").into_bytes());
}

#[test]
#[should_panic(expected = "`n` did not evaluate to a dict")]
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
#[should_panic(expected = "dict literal key did not evaluate to str")]
fn a_dict_literal_with_a_non_str_key_is_an_internal_error() {
    // `pycc_types`' T0036 gate rejects every dict literal whose key
    // isn't `Ty::Str` before codegen ever runs, so this is hand-built
    // malformed MIR, not a real-source repro -- covers `MirExpr::
    // DictLiteral`'s own inline key-extraction panic.
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::Assign {
            target: "x".to_string(),
            value: MirExpr::DictLiteral(vec![(MirExpr::IntLiteral(1), MirExpr::IntLiteral(2))]),
        })],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("dict_literal_non_str_key_panics")
        .expect("failed to create scratch dir");
    let _ = compile_to_object(
        &mir,
        &dir.join("dict_literal_non_str_key_panics.o"),
        None,
        false,
    );
}

#[test]
#[should_panic(expected = "dict subscript key did not evaluate to str")]
fn a_dict_get_with_a_non_str_key_is_an_internal_error() {
    // `pycc_types` rejects a mismatched dict-subscript key type with
    // T0021 before codegen ever runs, so this is hand-built malformed
    // MIR -- covers `MirExpr::DictGet`'s own inline key-extraction
    // panic.
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::DictGet {
            dict: Box::new(MirExpr::DictLiteral(vec![(
                MirExpr::StringLiteral("a".to_string()),
                MirExpr::IntLiteral(1),
            )])),
            key: Box::new(MirExpr::IntLiteral(1)),
        }))],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("dict_get_non_str_key_panics")
        .expect("failed to create scratch dir");
    let _ = compile_to_object(
        &mir,
        &dir.join("dict_get_non_str_key_panics.o"),
        None,
        false,
    );
}

#[test]
#[should_panic(expected = "dict item-assignment key did not evaluate to str")]
fn a_dict_set_with_a_non_str_key_is_an_internal_error() {
    // `pycc_types` rejects a mismatched `d[k] = v` key type with T0021
    // before codegen ever runs, so this is hand-built malformed MIR --
    // covers `MirStmt::DictSet`'s own inline key-extraction panic.
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
                key: MirExpr::IntLiteral(1),
                value: MirExpr::IntLiteral(2),
            }),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("dict_set_non_str_key_panics")
        .expect("failed to create scratch dir");
    let _ = compile_to_object(
        &mir,
        &dir.join("dict_set_non_str_key_panics.o"),
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

/// Builds `MirStmt::ForList { var: "v", list: <list>, body: [print(v)] }`,
/// the shape every list-comprehension test below uses to read its own
/// result back out: container `to_str`/`truthy` are unimplemented
/// (D-107/D-124), so a comprehension's own produced list cannot be
/// printed directly and must be walked element-by-element instead.
fn print_each_int(list: &str) -> MirStmt {
    MirStmt::ForList {
        var: "v".to_string(),
        list: list.to_string(),
        body: vec![MirStmt::ExprStmt(MirExpr::Call {
            callee: "print".to_string(),
            args: vec![MirExpr::Name {
                name: "v".to_string(),
                ty: Ty::Int,
            }],
            ty: Ty::None,
        })],
    }
}

/// `MirStmt::ForSet { var: "v", var_ty: int, set: <set>, body: [print(v)] }` -- the
/// `SetCompAssign` test suite's own analog of `print_each_int` above,
/// for the identical reason (container `to_str`/`truthy` remain
/// unimplemented, D-107/D-124, so a produced `set[int]` cannot be
/// printed directly and must be walked element-by-element via
/// `MirStmt::ForSet` instead, whose own insertion-order iteration this
/// crate's pre-existing `a_return_inside_a_for_set_body_returns_
/// immediately_without_looping` test already pins).
fn print_each_int_from_set(set: &str) -> MirStmt {
    MirStmt::ForSet {
        var_ty: pycc_mir::Ty::Int,
        var: "v".to_string(),
        set: set.to_string(),
        body: vec![MirStmt::ExprStmt(MirExpr::Call {
            callee: "print".to_string(),
            args: vec![MirExpr::Name {
                name: "v".to_string(),
                ty: Ty::Int,
            }],
            ty: Ty::None,
        })],
    }
}

#[test]
fn a_range_sourced_list_comprehension_with_no_filter_computes_every_element() {
    // `xs = [i * 2 for i in range(5)]` (PR-12 Task 5a, D-117):
    // `CompSource::Range`, no `cond`. Exercises the `Range` branch of
    // `MirStmt::ListCompAssign`'s own internal `match source` (mirrors
    // `MirStmt::ForRange`'s own shape), the `cond: None` unconditional-
    // append path, and confirms `elt` (`i * 2`, not a bare `Name`) is
    // evaluated fresh every iteration rather than the loop's own raw
    // induction value being appended untransformed.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::ListCompAssign {
                target: "xs".to_string(),
                var: "i".to_string(),
                var_ty: Ty::Int,
                source: CompSource::Range {
                    start: MirExpr::IntLiteral(0),
                    stop: MirExpr::IntLiteral(5),
                    step: MirExpr::IntLiteral(1),
                },
                cond: None,
                elt: Box::new(MirExpr::BinOp {
                    op: BinOpKind::Mul,
                    left: Box::new(MirExpr::Name {
                        name: "i".to_string(),
                        ty: Ty::Int,
                    }),
                    right: Box::new(MirExpr::IntLiteral(2)),
                    ty: Ty::Int,
                }),
            }),
            MirItem::TopLevelStmt(print_each_int("xs")),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("listcomp_range_no_filter")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("listcomp_range_no_filter.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("listcomp_range_no_filter");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"0\n2\n4\n6\n8\n");
}

#[test]
fn a_list_sourced_list_comprehension_with_a_filter_only_keeps_matching_elements() {
    // `xs = [i for i in range(5)]` then `ys = [x for x in xs if x > 2]`
    // (PR-12 Task 5a, D-117): the second comprehension's own
    // `CompSource::List` is sourced from the *first* comprehension's own
    // produced list, not a literal -- exercising the `List` branch of
    // `MirStmt::ListCompAssign`'s own internal `match source` (mirrors
    // `MirStmt::ForList`'s own shape, via `emit_list_name_read`) and the
    // `cond: Some(..)` filtered-append path (the `listcomp_if_taken`/
    // `listcomp_if_skip` block pair): `0` and `1` and `2` must be
    // dropped, `3` and `4` kept.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::ListCompAssign {
                target: "xs".to_string(),
                var: "i".to_string(),
                var_ty: Ty::Int,
                source: CompSource::Range {
                    start: MirExpr::IntLiteral(0),
                    stop: MirExpr::IntLiteral(5),
                    step: MirExpr::IntLiteral(1),
                },
                cond: None,
                elt: Box::new(MirExpr::Name {
                    name: "i".to_string(),
                    ty: Ty::Int,
                }),
            }),
            MirItem::TopLevelStmt(MirStmt::ListCompAssign {
                target: "ys".to_string(),
                var: "x".to_string(),
                var_ty: Ty::Int,
                source: CompSource::List("xs".to_string()),
                cond: Some(Box::new(MirExpr::Compare {
                    op: CmpOpKind::Gt,
                    left: Box::new(MirExpr::Name {
                        name: "x".to_string(),
                        ty: Ty::Int,
                    }),
                    right: Box::new(MirExpr::IntLiteral(2)),
                    ty: Ty::Bool,
                })),
                elt: Box::new(MirExpr::Name {
                    name: "x".to_string(),
                    ty: Ty::Int,
                }),
            }),
            MirItem::TopLevelStmt(print_each_int("ys")),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("listcomp_list_with_filter")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("listcomp_list_with_filter.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("listcomp_list_with_filter");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"3\n4\n");
}

#[test]
fn a_list_sourced_list_comprehension_that_rebinds_its_own_source_name_reads_the_pre_existing_value()
{
    // Regression test (review round 1, post-Task-5a): `xs = [i for i in
    // range(5)]` then `xs = [x for x in xs if x > 2]`, reusing the same
    // name for both the source *and* the target of the second
    // comprehension. Real CPython evaluates the entire RHS -- the whole
    // loop over the pre-existing `xs` (`[0, 1, 2, 3, 4]`) -- before
    // rebinding `xs` to the result (`[3, 4]`), exactly like an ordinary
    // `xs = xs + [1]` never sees its own partially-updated target mid-
    // expression. An earlier version of this arm stored the target's
    // freshly allocated *empty* list into `xs`'s own slot before
    // evaluating `source`, so `emit_list_name_read(..., "xs")` read the
    // brand-new empty list instead of the original one, and this
    // produced `len(xs) == 0` instead of `2`. Fixed by deferring
    // `emit_assign(target, ..)` until after the loop (see this arm's own
    // "point 1"/"point 5" doc comments on `ListCompAssign` in `lib.rs`).
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::ListCompAssign {
                target: "xs".to_string(),
                var: "i".to_string(),
                var_ty: Ty::Int,
                source: CompSource::Range {
                    start: MirExpr::IntLiteral(0),
                    stop: MirExpr::IntLiteral(5),
                    step: MirExpr::IntLiteral(1),
                },
                cond: None,
                elt: Box::new(MirExpr::Name {
                    name: "i".to_string(),
                    ty: Ty::Int,
                }),
            }),
            MirItem::TopLevelStmt(MirStmt::ListCompAssign {
                target: "xs".to_string(),
                var: "x".to_string(),
                var_ty: Ty::Int,
                source: CompSource::List("xs".to_string()),
                cond: Some(Box::new(MirExpr::Compare {
                    op: CmpOpKind::Gt,
                    left: Box::new(MirExpr::Name {
                        name: "x".to_string(),
                        ty: Ty::Int,
                    }),
                    right: Box::new(MirExpr::IntLiteral(2)),
                    ty: Ty::Bool,
                })),
                elt: Box::new(MirExpr::Name {
                    name: "x".to_string(),
                    ty: Ty::Int,
                }),
            }),
            MirItem::TopLevelStmt(print_each_int("xs")),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("listcomp_self_referential_source")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("listcomp_self_referential_source.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("listcomp_self_referential_source");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"3\n4\n");
}

#[test]
fn a_range_sourced_list_comprehension_whose_bound_reads_its_own_rebound_target_uses_the_pre_existing_length()
 {
    // Regression test (review round 1, post-Task-5a): `xs = [i for i in
    // range(5)]` then `xs = [i * 2 for i in range(len(xs))]`. `source`
    // is `CompSource::Range` here, not `CompSource::List` -- distinct
    // from the test directly above, since it exercises `source`'s own
    // `stop` expression (`len(xs)`) reading `target`'s pre-existing
    // value during the *preheader*, before `test_bb` even exists, not
    // `var`'s own per-iteration container read. Real CPython evaluates
    // `range(len(xs))` once, against the original 5-element `xs`,
    // before the comprehension loop runs at all, giving a 5-element
    // result (`[0, 2, 4, 6, 8]`); the same premature-rebind bug this
    // file's neighboring regression test documents would instead have
    // read the just-emptied `xs` here, giving `range(0)` and an empty
    // result.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::ListCompAssign {
                target: "xs".to_string(),
                var: "i".to_string(),
                var_ty: Ty::Int,
                source: CompSource::Range {
                    start: MirExpr::IntLiteral(0),
                    stop: MirExpr::IntLiteral(5),
                    step: MirExpr::IntLiteral(1),
                },
                cond: None,
                elt: Box::new(MirExpr::Name {
                    name: "i".to_string(),
                    ty: Ty::Int,
                }),
            }),
            MirItem::TopLevelStmt(MirStmt::ListCompAssign {
                target: "xs".to_string(),
                var: "i".to_string(),
                var_ty: Ty::Int,
                source: CompSource::Range {
                    start: MirExpr::IntLiteral(0),
                    stop: MirExpr::Call {
                        callee: "len".to_string(),
                        args: vec![MirExpr::Name {
                            name: "xs".to_string(),
                            ty: Ty::List(Box::new(Ty::Int)),
                        }],
                        ty: Ty::Int,
                    },
                    step: MirExpr::IntLiteral(1),
                },
                cond: None,
                elt: Box::new(MirExpr::BinOp {
                    op: BinOpKind::Mul,
                    left: Box::new(MirExpr::Name {
                        name: "i".to_string(),
                        ty: Ty::Int,
                    }),
                    right: Box::new(MirExpr::IntLiteral(2)),
                    ty: Ty::Int,
                }),
            }),
            MirItem::TopLevelStmt(print_each_int("xs")),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("listcomp_self_referential_range_bound")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("listcomp_self_referential_range_bound.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("listcomp_self_referential_range_bound");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"0\n2\n4\n6\n8\n");
}

#[test]
fn a_list_comprehensions_elt_reading_its_own_rebound_target_reads_the_pre_existing_value() {
    // Regression test (review round 1, post-Task-5a): `xs = [1, 2, 3]`
    // then `xs = [xs[0] for i in range(2)]`. Distinct from both tests
    // above: `elt` (not `source`) reads `target`'s own name here,
    // *inside* the loop body, once per iteration. Real CPython gives
    // `[1, 1]` (`xs[0]` is `1` throughout, since the original `xs` is
    // untouched until the whole comprehension finishes). The
    // premature-rebind bug this file's other two regression tests above
    // document made this specific shape crash outright, not merely
    // print the wrong value: `xs`'s slot held the freshly allocated
    // *empty* list for the entire loop, so the very first
    // `xs[0]` read inside `elt` panicked with `pycc_rt`'s own honest
    // "list index out of range" before any element was ever appended.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "xs".to_string(),
                value: MirExpr::ListLiteral(vec![
                    MirExpr::IntLiteral(1),
                    MirExpr::IntLiteral(2),
                    MirExpr::IntLiteral(3),
                ]),
            }),
            MirItem::TopLevelStmt(MirStmt::ListCompAssign {
                target: "xs".to_string(),
                var: "i".to_string(),
                var_ty: Ty::Int,
                source: CompSource::Range {
                    start: MirExpr::IntLiteral(0),
                    stop: MirExpr::IntLiteral(2),
                    step: MirExpr::IntLiteral(1),
                },
                cond: None,
                elt: Box::new(MirExpr::Subscript {
                    base: Box::new(MirExpr::Name {
                        name: "xs".to_string(),
                        ty: Ty::List(Box::new(Ty::Int)),
                    }),
                    index: Box::new(MirExpr::IntLiteral(0)),
                }),
            }),
            MirItem::TopLevelStmt(print_each_int("xs")),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("listcomp_self_referential_elt")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("listcomp_self_referential_elt.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("listcomp_self_referential_elt");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"1\n1\n");
}

#[test]
fn a_set_sourced_list_comprehension_with_no_filter_visits_every_element() {
    // `zs = [x for x in some_set]` (PR-12 Task 5a, D-117): `set[int]`'s
    // own element type is always `Ty::Int` (T0038), satisfying
    // `list[int]`'s own T0034 gate trivially, so this source is
    // reachable and type-safe from real Python source. Exercises the
    // `Set` branch of `MirStmt::ListCompAssign`'s own internal `match
    // source` (mirrors `MirStmt::ForSet`'s own shape, via
    // `emit_set_name_read`/`build_int_set_get`/`build_int_set_len`).
    // `{1, 2, 3}` is read back in insertion order (pinned by this
    // crate's own `a_return_inside_a_for_set_body_returns_immediately_
    // without_looping` test, which reads the same literal's first
    // element as `1`), so the produced list's own order is
    // deterministic here.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "some_set".to_string(),
                value: MirExpr::SetLiteral {
                    elements: vec![
                        MirExpr::IntLiteral(1),
                        MirExpr::IntLiteral(2),
                        MirExpr::IntLiteral(3),
                    ],
                    ops: None,
                },
            }),
            MirItem::TopLevelStmt(MirStmt::ListCompAssign {
                target: "zs".to_string(),
                var: "x".to_string(),
                var_ty: Ty::Int,
                source: CompSource::Set("some_set".to_string()),
                cond: None,
                elt: Box::new(MirExpr::Name {
                    name: "x".to_string(),
                    ty: Ty::Int,
                }),
            }),
            MirItem::TopLevelStmt(print_each_int("zs")),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("listcomp_set_no_filter")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("listcomp_set_no_filter.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("listcomp_set_no_filter");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"1\n2\n3\n");
}

#[test]
fn an_empty_range_sourced_list_comprehension_produces_a_genuinely_valid_empty_list() {
    // `zs = [i for i in range(0)]` (PR-12 Task 5a, D-117): the loop body
    // never executes, so `zs` must still be a real, non-null
    // `PyIntListObj` -- not a null/dangling pointer that merely happens
    // never to be read. `len(zs)` proves the pointer is valid enough for
    // `pycc_rt_int_list_len` to read (a null pointer would segfault, not
    // silently return `0`); appending to it afterward and reading the
    // appended element back out proves it is independently *writable*
    // too, not merely readable.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::ListCompAssign {
                target: "zs".to_string(),
                var: "i".to_string(),
                var_ty: Ty::Int,
                source: CompSource::Range {
                    start: MirExpr::IntLiteral(0),
                    stop: MirExpr::IntLiteral(0),
                    step: MirExpr::IntLiteral(1),
                },
                cond: None,
                elt: Box::new(MirExpr::Name {
                    name: "i".to_string(),
                    ty: Ty::Int,
                }),
            }),
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
                callee: "print".to_string(),
                args: vec![MirExpr::Call {
                    callee: "len".to_string(),
                    args: vec![MirExpr::Name {
                        name: "zs".to_string(),
                        ty: Ty::List(Box::new(Ty::Int)),
                    }],
                    ty: Ty::Int,
                }],
                ty: Ty::None,
            })),
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::ListAppend {
                list: pycc_mir::MirContainerReceiver::Name("zs".to_string()),
                value: Box::new(MirExpr::IntLiteral(9)),
            })),
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
                callee: "print".to_string(),
                args: vec![MirExpr::Subscript {
                    base: Box::new(MirExpr::Name {
                        name: "zs".to_string(),
                        ty: Ty::List(Box::new(Ty::Int)),
                    }),
                    index: Box::new(MirExpr::IntLiteral(0)),
                }],
                ty: Ty::None,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("listcomp_range_empty")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("listcomp_range_empty.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("listcomp_range_empty");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"0\n9\n");
}

#[test]
fn a_dict_sourced_list_comprehension_binds_its_key_without_crashing() {
    // `CompSource::Dict` reaching `MirStmt::ListCompAssign` is an
    // internal-error-panic path no type-checked program can ever
    // trigger from real source (PR-12 Task 5a's own brief, D-117): every
    // reachable `list[int]`-producing comprehension has `elt: Ty::Int`
    // (T0034), and this compiler has no `str`-to-`int` builtin of any
    // kind, so `pycc_types` can never route a dict-typed base into a
    // `list[int]` comprehension's own source. This test bypasses
    // `pycc_types` entirely (hand-built MIR, mirroring this crate's own
    // established convention for other unreachable-from-real-source
    // shapes) to confirm the dict-sourced per-iteration binding code
    // path -- the `pycc_rt_str_incref` call on the read key, exactly
    // like `MirStmt::ForDict`'s own per-iteration bind -- does not crash
    // even though nothing reachable exercises it today. `elt` is a
    // constant (`7`), not a read of the bound key, since `var`'s own
    // type here is `Ty::Str` and a `list[int]`'s own element type is
    // `Ty::Int` -- the binding itself is what this test is pinning, not
    // what `elt` does with it afterward (this crate has no `str`-typed
    // `elt` to give it that would still type as `Ty::Int`).
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "d".to_string(),
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
            MirItem::TopLevelStmt(MirStmt::ListCompAssign {
                target: "zs".to_string(),
                var: "k".to_string(),
                var_ty: Ty::Str,
                source: CompSource::Dict("d".to_string()),
                cond: None,
                elt: Box::new(MirExpr::IntLiteral(7)),
            }),
            MirItem::TopLevelStmt(print_each_int("zs")),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("listcomp_dict_source_no_crash")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("listcomp_dict_source_no_crash.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("listcomp_dict_source_no_crash");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"7\n7\n");
}

// ---- PR-12 Task 5b: `SetCompAssign` codegen tests ----

#[test]
fn a_range_sourced_set_comprehension_with_a_filter_only_keeps_matching_elements() {
    // `evens = {x for x in range(10) if x % 2 == 0}` (PR-12 Task 5b,
    // D-117): exercises `MirStmt::SetCompAssign`'s own `Range` branch
    // (mirrors `ListCompAssign`'s own `Range` branch exactly) and the
    // `cond: Some(..)` filtered-insert path. Expected output verified
    // against `python3` on this exact source (`set[int]`'s own
    // insertion order is pinned by this crate's own
    // `a_return_inside_a_for_set_body_returns_immediately_without_
    // looping` test, so the filtered evens print back in ascending
    // order here).
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::SetCompAssign {
                ops: None,
                target: "evens".to_string(),
                var: "x".to_string(),
                var_ty: Ty::Int,
                source: CompSource::Range {
                    start: MirExpr::IntLiteral(0),
                    stop: MirExpr::IntLiteral(10),
                    step: MirExpr::IntLiteral(1),
                },
                cond: Some(Box::new(MirExpr::Compare {
                    op: CmpOpKind::Eq,
                    left: Box::new(MirExpr::BinOp {
                        op: BinOpKind::Mod,
                        left: Box::new(MirExpr::Name {
                            name: "x".to_string(),
                            ty: Ty::Int,
                        }),
                        right: Box::new(MirExpr::IntLiteral(2)),
                        ty: Ty::Int,
                    }),
                    right: Box::new(MirExpr::IntLiteral(0)),
                    ty: Ty::Bool,
                })),
                elt: Box::new(MirExpr::Name {
                    name: "x".to_string(),
                    ty: Ty::Int,
                }),
            }),
            MirItem::TopLevelStmt(print_each_int_from_set("evens")),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("setcomp_range_with_filter")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("setcomp_range_with_filter.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("setcomp_range_with_filter");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"0\n2\n4\n6\n8\n");
}

#[test]
fn a_list_sourced_set_comprehension_with_no_filter_deduplicates_repeated_elements() {
    // `xs = [1, 2, 2, 3]` then `s = {x for x in xs}` (PR-12 Task 5b,
    // D-117): exercises `MirStmt::SetCompAssign`'s own `List` branch and
    // the `cond: None` unconditional-insert path, and confirms
    // `pycc_rt_int_set_add`'s own dedup check (D-121) collapses the
    // repeated `2` to a single entry -- `len(s) == 3`, not `4`.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "xs".to_string(),
                value: MirExpr::ListLiteral(vec![
                    MirExpr::IntLiteral(1),
                    MirExpr::IntLiteral(2),
                    MirExpr::IntLiteral(2),
                    MirExpr::IntLiteral(3),
                ]),
            }),
            MirItem::TopLevelStmt(MirStmt::SetCompAssign {
                ops: None,
                target: "s".to_string(),
                var: "x".to_string(),
                var_ty: Ty::Int,
                source: CompSource::List("xs".to_string()),
                cond: None,
                elt: Box::new(MirExpr::Name {
                    name: "x".to_string(),
                    ty: Ty::Int,
                }),
            }),
            MirItem::TopLevelStmt(print_expr(MirExpr::Call {
                callee: "len".to_string(),
                args: vec![set_name("s")],
                ty: Ty::Int,
            })),
            MirItem::TopLevelStmt(print_each_int_from_set("s")),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("setcomp_list_no_filter_dedup")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("setcomp_list_no_filter_dedup.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("setcomp_list_no_filter_dedup");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"3\n1\n2\n3\n");
}

#[test]
fn a_set_sourced_set_comprehension_that_rebinds_its_own_source_name_reads_the_pre_existing_value() {
    // `s = {1, 2, 3}` then `s = {x for x in s if x > 1}`, reusing the
    // same name for both the source *and* the target (PR-12 Task 5b,
    // D-117): the direct `SetCompAssign` analog of `ListCompAssign`'s
    // own review-round-1 regression (see that arm's own "point 1"/
    // "point 5" comments, and this crate's `a_list_sourced_list_
    // comprehension_that_rebinds_its_own_source_name_reads_the_pre_
    // existing_value` test) -- a premature `emit_assign(target, ..)`
    // before the loop runs would make `emit_set_name_read(..., "s")`
    // read the freshly allocated, still-empty set instead of the
    // original `{1, 2, 3}`, producing an empty result instead of
    // `{2, 3}`. Also exercises `SetCompAssign`'s own `Set` branch and
    // the `cond: Some(..)` filtered path.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "s".to_string(),
                value: MirExpr::SetLiteral {
                    elements: vec![
                        MirExpr::IntLiteral(1),
                        MirExpr::IntLiteral(2),
                        MirExpr::IntLiteral(3),
                    ],
                    ops: None,
                },
            }),
            MirItem::TopLevelStmt(MirStmt::SetCompAssign {
                ops: None,
                target: "s".to_string(),
                var: "x".to_string(),
                var_ty: Ty::Int,
                source: CompSource::Set("s".to_string()),
                cond: Some(Box::new(MirExpr::Compare {
                    op: CmpOpKind::Gt,
                    left: Box::new(MirExpr::Name {
                        name: "x".to_string(),
                        ty: Ty::Int,
                    }),
                    right: Box::new(MirExpr::IntLiteral(1)),
                    ty: Ty::Bool,
                })),
                elt: Box::new(MirExpr::Name {
                    name: "x".to_string(),
                    ty: Ty::Int,
                }),
            }),
            MirItem::TopLevelStmt(print_each_int_from_set("s")),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("setcomp_self_referential_rebind")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("setcomp_self_referential_rebind.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("setcomp_self_referential_rebind");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"2\n3\n");
}

#[test]
fn a_range_sourced_set_comprehension_whose_bound_reads_its_own_rebound_target_uses_the_pre_existing_length()
 {
    // `s = {1, 2, 3, 4, 5}` then `s = {i * 2 for i in range(len(s))}`
    // (PR-12 Task 5b, D-117): the `SetCompAssign` analog of
    // `ListCompAssign`'s own `a_range_sourced_list_comprehension_whose_
    // bound_reads_its_own_rebound_target_uses_the_pre_existing_length`
    // regression test -- distinct from the test directly above, since
    // it exercises `source`'s own `stop` expression (`len(s)`) reading
    // `target`'s pre-existing value during the *preheader*, before
    // `test_bb` even exists, not `var`'s own per-iteration container
    // read (`source` here is `CompSource::Range`, not `CompSource::
    // Set`). Real CPython evaluates `range(len(s))` once, against the
    // original 5-element `s`, before the comprehension loop runs at
    // all, giving a 5-element result (`{0, 2, 4, 6, 8}`); the premature-
    // rebind bug this file's `ListCompAssign` neighbor documents would
    // instead have read the just-emptied `s` here, giving `range(0)`
    // and an empty result.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "s".to_string(),
                value: MirExpr::SetLiteral {
                    elements: vec![
                        MirExpr::IntLiteral(1),
                        MirExpr::IntLiteral(2),
                        MirExpr::IntLiteral(3),
                        MirExpr::IntLiteral(4),
                        MirExpr::IntLiteral(5),
                    ],
                    ops: None,
                },
            }),
            MirItem::TopLevelStmt(MirStmt::SetCompAssign {
                ops: None,
                target: "s".to_string(),
                var: "i".to_string(),
                var_ty: Ty::Int,
                source: CompSource::Range {
                    start: MirExpr::IntLiteral(0),
                    stop: MirExpr::Call {
                        callee: "len".to_string(),
                        args: vec![set_name("s")],
                        ty: Ty::Int,
                    },
                    step: MirExpr::IntLiteral(1),
                },
                cond: None,
                elt: Box::new(MirExpr::BinOp {
                    op: BinOpKind::Mul,
                    left: Box::new(MirExpr::Name {
                        name: "i".to_string(),
                        ty: Ty::Int,
                    }),
                    right: Box::new(MirExpr::IntLiteral(2)),
                    ty: Ty::Int,
                }),
            }),
            MirItem::TopLevelStmt(print_each_int_from_set("s")),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("setcomp_self_referential_range_bound")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("setcomp_self_referential_range_bound.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("setcomp_self_referential_range_bound");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"0\n2\n4\n6\n8\n");
}

#[test]
fn a_dict_sourced_set_comprehension_binds_its_key_without_crashing() {
    // `CompSource::Dict` reaching `MirStmt::SetCompAssign` is
    // unreachable from real, type-checked source for the identical
    // reason `ListCompAssign`'s own analogous test gives (`set[int]`'s
    // own element type is always `Ty::Int`, T0038, and this compiler has
    // no `str`-to-`int` builtin) -- hand-built MIR, mirroring this
    // crate's own `a_dict_sourced_list_comprehension_binds_its_key_
    // without_crashing` test exactly. `elt` is a constant (`7`), not a
    // read of the bound key; `pycc_rt_int_set_add`'s own dedup collapses
    // the two (identical) inserted `7`s to one entry, confirming the
    // dict-sourced per-iteration binding path -- the
    // `pycc_rt_str_incref` call on the read key -- does not crash.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "d".to_string(),
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
            MirItem::TopLevelStmt(MirStmt::SetCompAssign {
                ops: None,
                target: "zs".to_string(),
                var: "k".to_string(),
                var_ty: Ty::Str,
                source: CompSource::Dict("d".to_string()),
                cond: None,
                elt: Box::new(MirExpr::IntLiteral(7)),
            }),
            MirItem::TopLevelStmt(print_expr(MirExpr::Call {
                callee: "len".to_string(),
                args: vec![set_name("zs")],
                ty: Ty::Int,
            })),
            MirItem::TopLevelStmt(print_each_int_from_set("zs")),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("setcomp_dict_source_no_crash")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("setcomp_dict_source_no_crash.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("setcomp_dict_source_no_crash");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"1\n7\n");
}

// ---- PR-12 Task 5b: `DictCompAssign` codegen tests ----

#[test]
fn a_range_sourced_dict_comprehension_with_an_fstring_key_computes_every_entry() {
    // `named = {f"n{i}": i for i in range(3)}` (PR-12 Task 5b, D-117):
    // the common, no-aliasing-hazard shape -- `key` is a fresh
    // `MirExpr::FString` (already owning exactly one reference from its
    // own construction), so this arm's own `incref_if_str_duplicate`
    // call on `key` is a no-op here (see that arm's own doc comment
    // above). Exercises `MirStmt::DictCompAssign`'s own `Range` branch
    // and the `cond: None` unconditional-insert path.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::DictCompAssign {
                target: "named".to_string(),
                var: "i".to_string(),
                var_ty: Ty::Int,
                source: CompSource::Range {
                    start: MirExpr::IntLiteral(0),
                    stop: MirExpr::IntLiteral(3),
                    step: MirExpr::IntLiteral(1),
                },
                cond: None,
                key: Box::new(MirExpr::FString(vec![
                    pycc_mir::MirFStringPart::Literal("n".to_string()),
                    pycc_mir::MirFStringPart::Interpolation(Box::new(MirExpr::Name {
                        name: "i".to_string(),
                        ty: Ty::Int,
                    })),
                ])),
                value: Box::new(MirExpr::Name {
                    name: "i".to_string(),
                    ty: Ty::Int,
                }),
            }),
            MirItem::TopLevelStmt(print_expr(MirExpr::Call {
                callee: "len".to_string(),
                args: vec![dict_name("named")],
                ty: Ty::Int,
            })),
            MirItem::TopLevelStmt(print_expr(MirExpr::DictGet {
                dict: Box::new(dict_name("named")),
                key: Box::new(MirExpr::StringLiteral("n0".to_string())),
            })),
            MirItem::TopLevelStmt(print_expr(MirExpr::DictGet {
                dict: Box::new(dict_name("named")),
                key: Box::new(MirExpr::StringLiteral("n1".to_string())),
            })),
            MirItem::TopLevelStmt(print_expr(MirExpr::DictGet {
                dict: Box::new(dict_name("named")),
                key: Box::new(MirExpr::StringLiteral("n2".to_string())),
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("dictcomp_range_fstring_key")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("dictcomp_range_fstring_key.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("dictcomp_range_fstring_key");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"3\n0\n1\n2\n");
}

#[test]
fn a_dict_sourced_dict_comprehension_builds_an_independent_copy_and_leaves_the_source_intact() {
    // `d = {"a": 10, "b": 20}` then `d2 = {k: 1 for k in d}` (PR-12 Task
    // 5b, D-117's own dedicated safety test, brief item (d)): `d`, a
    // pre-existing `dict[str, int]`, is the one `Dict`-sourced
    // `DictCompAssign` shape reachable from real, type-checked source
    // (T0036: `var`/`k` is `Ty::Str`, satisfying `dict[str, int]`'s own
    // key-type gate directly -- see this arm's own doc comment on
    // `DictCompAssign` in `lib.rs` for the full argument). Confirms two things: `d2`'s own contents are
    // correct and independently readable, and building `d2` leaves `d`'s
    // own contents unaffected -- exactly what this arm's own
    // `incref_if_str_duplicate` call on `key` (mirroring `MirStmt::
    // DictSet`'s own identical call, D-124) exists to guarantee: `d2`'s
    // own stored key becomes a genuinely independent reference, not a
    // bare, uncounted alias of `d`'s own key pointer.
    //
    // This is a functional-correctness test, not a use-after-free
    // detector: this compiler's container model keeps `dict[K, V]`
    // leak-only (D-124) and has no key-removal operation of any kind
    // (`del`, `.pop()`, ...) yet, so nothing currently implemented can
    // ever bring a dict's own claim on one of its keys down to zero --
    // the missing incref this test guards against cannot be forced into
    // an observable crash via any currently-reachable pycc-language
    // operation (see this task's own report for the full analysis). The
    // fix is still required by D-124's ownership contract and is
    // unconditionally applied here, independent of what this one test
    // can currently observe.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "d".to_string(),
                value: MirExpr::DictLiteral(vec![
                    (
                        MirExpr::StringLiteral("a".to_string()),
                        MirExpr::IntLiteral(10),
                    ),
                    (
                        MirExpr::StringLiteral("b".to_string()),
                        MirExpr::IntLiteral(20),
                    ),
                ]),
            }),
            MirItem::TopLevelStmt(MirStmt::DictCompAssign {
                target: "d2".to_string(),
                var: "k".to_string(),
                var_ty: Ty::Str,
                source: CompSource::Dict("d".to_string()),
                cond: None,
                key: Box::new(MirExpr::Name {
                    name: "k".to_string(),
                    ty: Ty::Str,
                }),
                value: Box::new(MirExpr::IntLiteral(1)),
            }),
            // `d`'s own contents, read back after `d2` was built.
            MirItem::TopLevelStmt(print_expr(MirExpr::DictGet {
                dict: Box::new(dict_name("d")),
                key: Box::new(MirExpr::StringLiteral("a".to_string())),
            })),
            MirItem::TopLevelStmt(print_expr(MirExpr::DictGet {
                dict: Box::new(dict_name("d")),
                key: Box::new(MirExpr::StringLiteral("b".to_string())),
            })),
            MirItem::TopLevelStmt(print_expr(MirExpr::Call {
                callee: "len".to_string(),
                args: vec![dict_name("d")],
                ty: Ty::Int,
            })),
            // `d2`'s own contents, independently.
            MirItem::TopLevelStmt(print_expr(MirExpr::DictGet {
                dict: Box::new(dict_name("d2")),
                key: Box::new(MirExpr::StringLiteral("a".to_string())),
            })),
            MirItem::TopLevelStmt(print_expr(MirExpr::DictGet {
                dict: Box::new(dict_name("d2")),
                key: Box::new(MirExpr::StringLiteral("b".to_string())),
            })),
            MirItem::TopLevelStmt(print_expr(MirExpr::Call {
                callee: "len".to_string(),
                args: vec![dict_name("d2")],
                ty: Ty::Int,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("dictcomp_dict_source_independent_copy")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("dictcomp_dict_source_independent_copy.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("dictcomp_dict_source_independent_copy");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"10\n20\n2\n1\n1\n2\n");
}

#[test]
fn a_list_sourced_dict_comprehension_with_a_filter_only_keeps_matching_entries() {
    // `xs = [10, 20, 30]` then `named2 = {f"v{x}": x for x in xs if x >
    // 15}` (PR-12 Task 5b, D-117): exercises `MirStmt::DictCompAssign`'s
    // own `List` branch (reachable from real, type-checked source:
    // `list[int]`'s element type, T0034, satisfies `var`'s own
    // `Ty::Int`, and an f-string key is always `Ty::Str`, satisfying
    // T0036) and the `cond: Some(..)` filtered-insert path -- `10` must
    // be dropped, `20`/`30` kept.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "xs".to_string(),
                value: MirExpr::ListLiteral(vec![
                    MirExpr::IntLiteral(10),
                    MirExpr::IntLiteral(20),
                    MirExpr::IntLiteral(30),
                ]),
            }),
            MirItem::TopLevelStmt(MirStmt::DictCompAssign {
                target: "named2".to_string(),
                var: "x".to_string(),
                var_ty: Ty::Int,
                source: CompSource::List("xs".to_string()),
                cond: Some(Box::new(MirExpr::Compare {
                    op: CmpOpKind::Gt,
                    left: Box::new(MirExpr::Name {
                        name: "x".to_string(),
                        ty: Ty::Int,
                    }),
                    right: Box::new(MirExpr::IntLiteral(15)),
                    ty: Ty::Bool,
                })),
                key: Box::new(MirExpr::FString(vec![
                    pycc_mir::MirFStringPart::Literal("v".to_string()),
                    pycc_mir::MirFStringPart::Interpolation(Box::new(MirExpr::Name {
                        name: "x".to_string(),
                        ty: Ty::Int,
                    })),
                ])),
                value: Box::new(MirExpr::Name {
                    name: "x".to_string(),
                    ty: Ty::Int,
                }),
            }),
            MirItem::TopLevelStmt(print_expr(MirExpr::Call {
                callee: "len".to_string(),
                args: vec![dict_name("named2")],
                ty: Ty::Int,
            })),
            MirItem::TopLevelStmt(print_expr(MirExpr::DictGet {
                dict: Box::new(dict_name("named2")),
                key: Box::new(MirExpr::StringLiteral("v20".to_string())),
            })),
            MirItem::TopLevelStmt(print_expr(MirExpr::DictGet {
                dict: Box::new(dict_name("named2")),
                key: Box::new(MirExpr::StringLiteral("v30".to_string())),
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("dictcomp_list_with_filter")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("dictcomp_list_with_filter.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("dictcomp_list_with_filter");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"2\n20\n30\n");
}

#[test]
fn a_set_sourced_dict_comprehension_with_a_filter_only_keeps_matching_entries() {
    // `ys = {5, 6, 7, 10}` then `named3 = {f"s{y}": y for y in ys if y <
    // 10}` (PR-12 Task 5b, D-117): exercises `MirStmt::DictCompAssign`'s
    // own `Set` branch (reachable from real, type-checked source:
    // `set[int]`'s element type, T0038, satisfies `var`'s own `Ty::Int`)
    // and the `cond: Some(..)` filtered-insert path -- `10` must be
    // dropped, `5`/`6`/`7` kept.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "ys".to_string(),
                value: MirExpr::SetLiteral {
                    elements: vec![
                        MirExpr::IntLiteral(5),
                        MirExpr::IntLiteral(6),
                        MirExpr::IntLiteral(7),
                        MirExpr::IntLiteral(10),
                    ],
                    ops: None,
                },
            }),
            MirItem::TopLevelStmt(MirStmt::DictCompAssign {
                target: "named3".to_string(),
                var: "y".to_string(),
                var_ty: Ty::Int,
                source: CompSource::Set("ys".to_string()),
                cond: Some(Box::new(MirExpr::Compare {
                    op: CmpOpKind::Lt,
                    left: Box::new(MirExpr::Name {
                        name: "y".to_string(),
                        ty: Ty::Int,
                    }),
                    right: Box::new(MirExpr::IntLiteral(10)),
                    ty: Ty::Bool,
                })),
                key: Box::new(MirExpr::FString(vec![
                    pycc_mir::MirFStringPart::Literal("s".to_string()),
                    pycc_mir::MirFStringPart::Interpolation(Box::new(MirExpr::Name {
                        name: "y".to_string(),
                        ty: Ty::Int,
                    })),
                ])),
                value: Box::new(MirExpr::Name {
                    name: "y".to_string(),
                    ty: Ty::Int,
                }),
            }),
            MirItem::TopLevelStmt(print_expr(MirExpr::Call {
                callee: "len".to_string(),
                args: vec![dict_name("named3")],
                ty: Ty::Int,
            })),
            MirItem::TopLevelStmt(print_expr(MirExpr::DictGet {
                dict: Box::new(dict_name("named3")),
                key: Box::new(MirExpr::StringLiteral("s5".to_string())),
            })),
            MirItem::TopLevelStmt(print_expr(MirExpr::DictGet {
                dict: Box::new(dict_name("named3")),
                key: Box::new(MirExpr::StringLiteral("s6".to_string())),
            })),
            MirItem::TopLevelStmt(print_expr(MirExpr::DictGet {
                dict: Box::new(dict_name("named3")),
                key: Box::new(MirExpr::StringLiteral("s7".to_string())),
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("dictcomp_set_with_filter")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("dictcomp_set_with_filter.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("dictcomp_set_with_filter");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"3\n5\n6\n7\n");
}

#[test]
#[should_panic(expected = "dict comprehension key did not evaluate to str")]
fn a_dict_comprehension_with_a_non_str_key_is_an_internal_error() {
    // `pycc_types` rejects a mismatched dict-comprehension key type with
    // T0036 before codegen ever runs, so this is hand-built malformed
    // MIR -- covers `MirStmt::DictCompAssign`'s own inline key-
    // extraction panic (mirrors `MirStmt::DictSet`'s own identical panic
    // and its own dedicated `a_dict_set_with_a_non_str_key_is_an_
    // internal_error` test).
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::DictCompAssign {
            target: "zs".to_string(),
            var: "i".to_string(),
            var_ty: Ty::Int,
            source: CompSource::Range {
                start: MirExpr::IntLiteral(0),
                stop: MirExpr::IntLiteral(1),
                step: MirExpr::IntLiteral(1),
            },
            cond: None,
            key: Box::new(MirExpr::IntLiteral(1)),
            value: Box::new(MirExpr::IntLiteral(2)),
        })],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("dictcomp_non_str_key_panics")
        .expect("failed to create scratch dir");
    let _ = compile_to_object(
        &mir,
        &dir.join("dictcomp_non_str_key_panics.o"),
        None,
        false,
    );
}

#[test]
#[should_panic(expected = "dict comprehension key did not evaluate to str")]
fn a_dict_comprehension_with_a_non_str_key_under_a_filter_is_an_internal_error() {
    // Same defect as `a_dict_comprehension_with_a_non_str_key_is_an_
    // internal_error` above, but with `cond: Some(..)` rather than
    // `cond: None`: this arm's own key-extraction `let-else` panic is
    // written once inside each of the two `match cond` branches (not
    // shared, mirroring `ListCompAssign`'s/`SetCompAssign`'s own
    // `elt`-evaluation-and-append duplication across those same two
    // branches), so it is two separate coverage regions -- this test
    // covers the `Some(..)` branch's own copy, which the `cond: None`
    // test above does not reach.
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::DictCompAssign {
            target: "zs".to_string(),
            var: "i".to_string(),
            var_ty: Ty::Int,
            source: CompSource::Range {
                start: MirExpr::IntLiteral(0),
                stop: MirExpr::IntLiteral(1),
                step: MirExpr::IntLiteral(1),
            },
            cond: Some(Box::new(MirExpr::BoolLiteral(true))),
            key: Box::new(MirExpr::IntLiteral(1)),
            value: Box::new(MirExpr::IntLiteral(2)),
        })],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("dictcomp_non_str_key_filtered_panics")
        .expect("failed to create scratch dir");
    let _ = compile_to_object(
        &mir,
        &dir.join("dictcomp_non_str_key_filtered_panics.o"),
        None,
        false,
    );
}

#[test]
fn a_dict_sourced_dict_comprehension_that_rebinds_its_own_source_name_reads_the_pre_existing_value()
{
    // `d = {"a": 5, "b": 6}` then `d = {k: 9 for k in d}`, reusing the
    // same name for both the source *and* the target (PR-12 Task 5b,
    // D-117): the direct `DictCompAssign` analog of `ListCompAssign`'s
    // own review-round-1 regression (see that arm's own "point 1"/
    // "point 5" comments and this crate's `a_list_sourced_list_
    // comprehension_that_rebinds_its_own_source_name_reads_the_pre_
    // existing_value` test) -- a premature `emit_assign(target, ..)`
    // before the loop runs would make `emit_dict_name_read(..., "d")`
    // read the freshly allocated, still-empty dict instead of the
    // original `{"a": 5, "b": 6}`, producing an empty result instead of
    // two entries. Distinct from `a_dict_sourced_dict_comprehension_
    // builds_an_independent_copy_and_leaves_the_source_intact` above,
    // which uses distinct names (`d`/`d2`) and therefore does not
    // exercise this ordering fix at all -- this test is what actually
    // guards against reintroducing Task 5a's own review-round-1 defect
    // in this arm.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "d".to_string(),
                value: MirExpr::DictLiteral(vec![
                    (
                        MirExpr::StringLiteral("a".to_string()),
                        MirExpr::IntLiteral(5),
                    ),
                    (
                        MirExpr::StringLiteral("b".to_string()),
                        MirExpr::IntLiteral(6),
                    ),
                ]),
            }),
            MirItem::TopLevelStmt(MirStmt::DictCompAssign {
                target: "d".to_string(),
                var: "k".to_string(),
                var_ty: Ty::Str,
                source: CompSource::Dict("d".to_string()),
                cond: None,
                key: Box::new(MirExpr::Name {
                    name: "k".to_string(),
                    ty: Ty::Str,
                }),
                value: Box::new(MirExpr::IntLiteral(9)),
            }),
            MirItem::TopLevelStmt(print_expr(MirExpr::Call {
                callee: "len".to_string(),
                args: vec![dict_name("d")],
                ty: Ty::Int,
            })),
            MirItem::TopLevelStmt(print_expr(MirExpr::DictGet {
                dict: Box::new(dict_name("d")),
                key: Box::new(MirExpr::StringLiteral("a".to_string())),
            })),
            MirItem::TopLevelStmt(print_expr(MirExpr::DictGet {
                dict: Box::new(dict_name("d")),
                key: Box::new(MirExpr::StringLiteral("b".to_string())),
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("dictcomp_self_referential_rebind")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("dictcomp_self_referential_rebind.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("dictcomp_self_referential_rebind");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"2\n9\n9\n");
}

#[test]
fn a_range_sourced_dict_comprehension_whose_bound_reads_its_own_rebound_target_uses_the_pre_existing_length()
 {
    // `d = {"a": 1, "b": 2, "c": 3}` then `d = {f"n{i}": i for i in
    // range(len(d))}` (PR-12 Task 5b, D-117): the `DictCompAssign`
    // analog of `ListCompAssign`'s own `a_range_sourced_list_
    // comprehension_whose_bound_reads_its_own_rebound_target_uses_the_
    // pre_existing_length` regression test -- distinct from
    // `a_dict_sourced_dict_comprehension_that_rebinds_its_own_source_
    // name_reads_the_pre_existing_value` above, since it exercises
    // `source`'s own `stop` expression (`len(d)`) reading `target`'s
    // pre-existing value during the *preheader*, before `test_bb` even
    // exists, not `var`'s own per-iteration container read (`source`
    // here is `CompSource::Range`, not `CompSource::Dict`). Real
    // CPython evaluates `range(len(d))` once, against the original
    // 3-entry `d`, before the comprehension loop runs at all, giving a
    // 3-entry result (`{"n0": 0, "n1": 1, "n2": 2}`); the premature-
    // rebind bug this file's `ListCompAssign` neighbor documents would
    // instead have read the just-emptied `d` here, giving `range(0)`
    // and an empty result.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "d".to_string(),
                value: MirExpr::DictLiteral(vec![
                    (
                        MirExpr::StringLiteral("a".to_string()),
                        MirExpr::IntLiteral(1),
                    ),
                    (
                        MirExpr::StringLiteral("b".to_string()),
                        MirExpr::IntLiteral(2),
                    ),
                    (
                        MirExpr::StringLiteral("c".to_string()),
                        MirExpr::IntLiteral(3),
                    ),
                ]),
            }),
            MirItem::TopLevelStmt(MirStmt::DictCompAssign {
                target: "d".to_string(),
                var: "i".to_string(),
                var_ty: Ty::Int,
                source: CompSource::Range {
                    start: MirExpr::IntLiteral(0),
                    stop: MirExpr::Call {
                        callee: "len".to_string(),
                        args: vec![dict_name("d")],
                        ty: Ty::Int,
                    },
                    step: MirExpr::IntLiteral(1),
                },
                cond: None,
                key: Box::new(MirExpr::FString(vec![
                    pycc_mir::MirFStringPart::Literal("n".to_string()),
                    pycc_mir::MirFStringPart::Interpolation(Box::new(MirExpr::Name {
                        name: "i".to_string(),
                        ty: Ty::Int,
                    })),
                ])),
                value: Box::new(MirExpr::Name {
                    name: "i".to_string(),
                    ty: Ty::Int,
                }),
            }),
            MirItem::TopLevelStmt(print_expr(MirExpr::Call {
                callee: "len".to_string(),
                args: vec![dict_name("d")],
                ty: Ty::Int,
            })),
            MirItem::TopLevelStmt(print_expr(MirExpr::DictGet {
                dict: Box::new(dict_name("d")),
                key: Box::new(MirExpr::StringLiteral("n0".to_string())),
            })),
            MirItem::TopLevelStmt(print_expr(MirExpr::DictGet {
                dict: Box::new(dict_name("d")),
                key: Box::new(MirExpr::StringLiteral("n1".to_string())),
            })),
            MirItem::TopLevelStmt(print_expr(MirExpr::DictGet {
                dict: Box::new(dict_name("d")),
                key: Box::new(MirExpr::StringLiteral("n2".to_string())),
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("dictcomp_self_referential_range_bound")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("dictcomp_self_referential_range_bound.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("dictcomp_self_referential_range_bound");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"3\n0\n1\n2\n");
}

// -- PR-12 Task 9 (D-118): `MirExpr::Slice` codegen -----------------

/// `xs = [<values>]` as a `MirStmt`, this section's own analog of
/// `assign_list_literal` above for an arbitrary element list rather
/// than the fixed `[1, 2, 3]` that helper hardcodes.
fn assign_list_literal_values(target: &str, values: &[i64]) -> MirStmt {
    MirStmt::Assign {
        target: target.to_string(),
        value: MirExpr::ListLiteral(values.iter().map(|v| MirExpr::IntLiteral(*v)).collect()),
    }
}

fn slice_list_name(name: &str) -> MirExpr {
    MirExpr::Name {
        name: name.to_string(),
        ty: Ty::List(Box::new(Ty::Int)),
    }
}

#[test]
fn a_slice_with_all_three_bounds_present_returns_the_expected_sub_range() {
    // D-118's ordinary path, at the codegen layer: `xs[1:4:1]` on
    // `[10, 20, 30, 40, 50]` is `[20, 30, 40]`. Exercises every `Some`
    // arm of the new `MirExpr::Slice` match (`start`/`stop`/`step` all
    // present), matching `tests/slice1_codegen_depth.rs`'s own identical
    // real-Python-source case (`a_basic_slice_with_explicit_bounds_
    // returns_the_expected_sub_range`) -- this hand-built-MIR version is
    // what actually counts toward `cargo llvm-cov -p pycc_codegen`,
    // since a workspace-root `tests/*.rs` integration binary is
    // attributed to the `pycc` package, not this crate.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(assign_list_literal_values("xs", &[10, 20, 30, 40, 50])),
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "ys".to_string(),
                value: MirExpr::Slice {
                    base: Box::new(slice_list_name("xs")),
                    start: Some(Box::new(MirExpr::IntLiteral(1))),
                    stop: Some(Box::new(MirExpr::IntLiteral(4))),
                    step: Some(Box::new(MirExpr::IntLiteral(1))),
                },
            }),
            MirItem::TopLevelStmt(print_each_int("ys")),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("slice_all_bounds_present")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("slice_all_bounds_present.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("slice_all_bounds_present");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"20\n30\n40\n");
}

#[test]
fn a_slice_with_every_bound_omitted_defaults_to_the_whole_list_stepped_by_one() {
    // D-118's own defaulting rule (`start`/`stop`/`step` default to
    // `0`/`len(list)`/`1`): `xs[:]` returns every element unchanged.
    // Exercises every `None` arm of the new match, including the
    // deferred `build_int_list_len` call this arm's own doc comment
    // describes -- the one line no other test in this group reaches,
    // since every other test here supplies an explicit `stop`.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(assign_list_literal_values("xs", &[10, 20, 30, 40, 50])),
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "ys".to_string(),
                value: MirExpr::Slice {
                    base: Box::new(slice_list_name("xs")),
                    start: None,
                    stop: None,
                    step: None,
                },
            }),
            MirItem::TopLevelStmt(print_each_int("ys")),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("slice_all_bounds_omitted")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("slice_all_bounds_omitted.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("slice_all_bounds_omitted");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"10\n20\n30\n40\n50\n");
}

#[test]
fn a_slice_with_only_a_step_present_skips_every_other_element() {
    // `xs[::2]` on `[0, 1, 2, 3, 4, 5]` is `[0, 2, 4]` -- `start`/`stop`
    // stay omitted (re-exercising the `None` arms the test above
    // already covers) while `step`'s own `Some` arm carries a value
    // other than `1`, pinning D-118's Step 5(c) requirement
    // specifically (a step greater than one, not just present at all).
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(assign_list_literal_values("xs", &[0, 1, 2, 3, 4, 5])),
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "ys".to_string(),
                value: MirExpr::Slice {
                    base: Box::new(slice_list_name("xs")),
                    start: None,
                    stop: None,
                    step: Some(Box::new(MirExpr::IntLiteral(2))),
                },
            }),
            MirItem::TopLevelStmt(print_each_int("ys")),
        ],
        class_defs: Vec::new(),
    };
    let dir =
        pycc_scratch::ScratchDir::new("slice_step_only").expect("failed to create scratch dir");
    let obj_path = dir.join("slice_step_only.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("slice_step_only");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"0\n2\n4\n");
}

#[test]
fn a_sliced_list_is_a_genuinely_independent_allocation_from_its_base() {
    // D-107's leak-only policy still requires the slice result to be a
    // *new* allocation, not an alias of `xs`'s own backing storage
    // (`pycc_rt_int_list_slice` itself is unit-tested for this in
    // `pycc_rt`; this pins the same guarantee through the real codegen
    // call site). Appending to `xs` after slicing must not retroactively
    // change `ys`, and vice versa.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(assign_list_literal_values("xs", &[1, 2, 3])),
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "ys".to_string(),
                value: MirExpr::Slice {
                    base: Box::new(slice_list_name("xs")),
                    start: Some(Box::new(MirExpr::IntLiteral(0))),
                    stop: Some(Box::new(MirExpr::IntLiteral(3))),
                    step: Some(Box::new(MirExpr::IntLiteral(1))),
                },
            }),
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::ListAppend {
                list: pycc_mir::MirContainerReceiver::Name("xs".to_string()),
                value: Box::new(MirExpr::IntLiteral(99)),
            })),
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::ListAppend {
                list: pycc_mir::MirContainerReceiver::Name("ys".to_string()),
                value: Box::new(MirExpr::IntLiteral(77)),
            })),
            MirItem::TopLevelStmt(print_each_int("xs")),
            MirItem::TopLevelStmt(print_each_int("ys")),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("slice_result_is_independent")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("slice_result_is_independent.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("slice_result_is_independent");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"1\n2\n3\n99\n1\n2\n3\n77\n");
}

#[test]
fn a_list_sourced_slice_that_rebinds_its_own_source_name_reads_the_pre_existing_value() {
    // Regression test in this arm's own established style (see
    // `a_list_sourced_list_comprehension_that_rebinds_its_own_source_
    // name_reads_the_pre_existing_value` above, Task 5a's confirmed
    // review-round regression): `xs = [1,2,3,4,5]` then `xs =
    // xs[1:3]`, reusing `xs` as both the slice's own `base` and the
    // assignment's own `target`. Real CPython fully evaluates the RHS
    // (reading the original 5-element `xs`, producing `[2, 3]`) before
    // rebinding `xs` to that result -- an implementation that stored
    // the slice's target pointer into `xs`'s slot before `base` had
    // been evaluated and the slice call completed would corrupt this.
    // `MirExpr::Slice`'s own arm never writes to `locals` at all (see
    // its doc comment) -- `MirStmt::Assign` is the only thing that
    // ever does, and only after this whole expression already produced
    // a pointer to a brand new, independent result object -- so there
    // is no premature-rebind window here to begin with, unlike
    // `ListCompAssign`'s own multi-block loop construction, which
    // needed a deliberate fix to get this right.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(assign_list_literal_values("xs", &[1, 2, 3, 4, 5])),
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "xs".to_string(),
                value: MirExpr::Slice {
                    base: Box::new(slice_list_name("xs")),
                    start: Some(Box::new(MirExpr::IntLiteral(1))),
                    stop: Some(Box::new(MirExpr::IntLiteral(3))),
                    step: None,
                },
            }),
            MirItem::TopLevelStmt(print_each_int("xs")),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("slice_self_referential_rebind")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("slice_self_referential_rebind.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("slice_self_referential_rebind");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"2\n3\n");
}

// -- PR-12 Task 11 (D-119): `list.pop()`/`dict.get(key, default)`/
// `set.add(value)` codegen -----------------------------------------

#[test]
fn list_pop_removes_and_returns_the_last_element_and_shrinks_len() {
    // `xs = [1,2,3]\ny = xs.pop()\nprint(y)\nprint(len(xs))\n` end to
    // end -- real `MirExpr::ListPop` codegen. Expected output verified
    // against `python3` on this exact source: `3` then `2`.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(assign_list_literal_values("xs", &[1, 2, 3])),
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "y".to_string(),
                value: MirExpr::ListPop {
                    list: pycc_mir::MirContainerReceiver::Name("xs".to_string()),
                    ty: Ty::Int,
                },
            }),
            MirItem::TopLevelStmt(print_expr(MirExpr::Name {
                name: "y".to_string(),
                ty: Ty::Int,
            })),
            MirItem::TopLevelStmt(print_expr(MirExpr::Call {
                callee: "len".to_string(),
                args: vec![slice_list_name("xs")],
                ty: Ty::Int,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir =
        pycc_scratch::ScratchDir::new("list_pop_basic").expect("failed to create scratch dir");
    let obj_path = dir.join("list_pop_basic.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("list_pop_basic");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"3\n2\n");
}

#[test]
fn pop_twice_on_the_same_list_in_one_statement_removes_in_order() {
    // `xs = [1,2,3,4,5]\nys = [xs.pop(), xs.pop()]\n` -- this task's own
    // brief flags exactly this shape as a place a Task-5a-class
    // evaluation-order bug could hide (binding a target's storage slot
    // before fully evaluating a self-referential right-hand side).
    // Verified empirically here, not just by the `MirExpr::ListPop`
    // arm's own doc comment reasoning: real CPython evaluates
    // `[xs.pop(), xs.pop()]`'s two elements strictly left to right,
    // each `.pop()` observing the previous one's mutation, so the
    // first `.pop()` removes `5` (leaving `[1,2,3,4]`) and the second
    // removes the new last element `4` (leaving `[1,2,3]`) --
    // `ys == [5, 4]`, `xs == [1,2,3]`. A codegen bug that read `xs`'s
    // pointer once and cached it, or that evaluated both `.pop()`
    // calls against a stale snapshot, would produce a different
    // result here (e.g. both calls popping the same element, or
    // popping in the wrong order) -- this test would catch either.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(assign_list_literal_values("xs", &[1, 2, 3, 4, 5])),
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "ys".to_string(),
                value: MirExpr::ListLiteral(vec![
                    MirExpr::ListPop {
                        list: pycc_mir::MirContainerReceiver::Name("xs".to_string()),
                        ty: Ty::Int,
                    },
                    MirExpr::ListPop {
                        list: pycc_mir::MirContainerReceiver::Name("xs".to_string()),
                        ty: Ty::Int,
                    },
                ]),
            }),
            MirItem::TopLevelStmt(print_each_int("ys")),
            MirItem::TopLevelStmt(print_each_int("xs")),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("list_pop_twice_same_statement")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("list_pop_twice_same_statement.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("list_pop_twice_same_statement");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"5\n4\n1\n2\n3\n");
}

#[test]
fn dict_get_or_default_on_a_present_key_returns_the_stored_value_codegens_and_runs() {
    // `d = {"a": 1}\nprint(d.get("a", -1))\n` end to end -- real
    // `MirExpr::DictGetOrDefault` codegen, found-key path. Expected
    // output verified against `python3` on this exact source: `1`.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "d".to_string(),
                value: MirExpr::DictLiteral(vec![(
                    MirExpr::StringLiteral("a".to_string()),
                    MirExpr::IntLiteral(1),
                )]),
            }),
            MirItem::TopLevelStmt(print_expr(MirExpr::DictGetOrDefault {
                dict: pycc_mir::MirContainerReceiver::Name("d".to_string()),
                key: Box::new(MirExpr::StringLiteral("a".to_string())),
                default: Box::new(MirExpr::IntLiteral(-1)),
                ty: Ty::Int,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("dict_get_or_default_present")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("dict_get_or_default_present.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("dict_get_or_default_present");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"1\n");
}

#[test]
fn dict_get_or_default_on_a_missing_key_returns_the_default_codegens_and_runs() {
    // `d = {"a": 1}\nprint(d.get("z", -1))\n` end to end -- real
    // `MirExpr::DictGetOrDefault` codegen, missing-key path (unlike
    // `MirExpr::DictGet`, this never panics). Expected output verified
    // against `python3` on this exact source: `-1`.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "d".to_string(),
                value: MirExpr::DictLiteral(vec![(
                    MirExpr::StringLiteral("a".to_string()),
                    MirExpr::IntLiteral(1),
                )]),
            }),
            MirItem::TopLevelStmt(print_expr(MirExpr::DictGetOrDefault {
                dict: pycc_mir::MirContainerReceiver::Name("d".to_string()),
                key: Box::new(MirExpr::StringLiteral("z".to_string())),
                default: Box::new(MirExpr::IntLiteral(-1)),
                ty: Ty::Int,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("dict_get_or_default_missing")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("dict_get_or_default_missing.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("dict_get_or_default_missing");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"-1\n");
}

#[test]
fn dict_get_or_default_nested_in_its_own_default_argument_resolves_correctly() {
    // `d = {"a": 1}\ny = d.get("k", d.get("a", -1))\nprint(y)\n` --
    // this task's own brief flags exactly this nested shape. Unlike
    // `list.pop()`, `dict.get()` never mutates its dict, so there is
    // no analogous ordering hazard to begin with (both the outer and
    // inner `.get()` read the same unchanged `d`) -- verified
    // empirically here rather than only by that reasoning: `"k"` is
    // absent, so the outer call's `default` sub-expression
    // (`d.get("a", -1)`) must itself be evaluated, and `"a"` is
    // present, so it resolves to `1`. Expected output verified against
    // `python3` on this exact source: `1`.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "d".to_string(),
                value: MirExpr::DictLiteral(vec![(
                    MirExpr::StringLiteral("a".to_string()),
                    MirExpr::IntLiteral(1),
                )]),
            }),
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "y".to_string(),
                value: MirExpr::DictGetOrDefault {
                    dict: pycc_mir::MirContainerReceiver::Name("d".to_string()),
                    key: Box::new(MirExpr::StringLiteral("k".to_string())),
                    default: Box::new(MirExpr::DictGetOrDefault {
                        dict: pycc_mir::MirContainerReceiver::Name("d".to_string()),
                        key: Box::new(MirExpr::StringLiteral("a".to_string())),
                        default: Box::new(MirExpr::IntLiteral(-1)),
                        ty: Ty::Int,
                    }),
                    ty: Ty::Int,
                },
            }),
            MirItem::TopLevelStmt(print_expr(MirExpr::Name {
                name: "y".to_string(),
                ty: Ty::Int,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("dict_get_or_default_nested_default")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("dict_get_or_default_nested_default.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("dict_get_or_default_nested_default");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"1\n");
}

#[test]
#[should_panic(expected = "dict.get() key did not evaluate to str")]
fn a_dict_get_or_default_with_a_non_str_key_is_an_internal_error() {
    // `pycc_types` (T0021) rejects a mismatched `dict.get()` key type
    // before codegen ever runs, so this is hand-built malformed MIR --
    // covers `MirExpr::DictGetOrDefault`'s own inline key-extraction
    // panic, mirroring `a_dict_get_with_a_non_str_key_is_an_internal_error`
    // above for `MirExpr::DictGet`.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "d".to_string(),
                value: MirExpr::DictLiteral(vec![(
                    MirExpr::StringLiteral("a".to_string()),
                    MirExpr::IntLiteral(1),
                )]),
            }),
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::DictGetOrDefault {
                dict: pycc_mir::MirContainerReceiver::Name("d".to_string()),
                key: Box::new(MirExpr::IntLiteral(1)),
                default: Box::new(MirExpr::IntLiteral(0)),
                ty: Ty::Int,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("dict_get_or_default_non_str_key_panics")
        .expect("failed to create scratch dir");
    let _ = compile_to_object(
        &mir,
        &dir.join("dict_get_or_default_non_str_key_panics.o"),
        None,
        false,
    );
}

#[test]
fn function_redefinition_uses_unique_mangled_names() {
    // Issue #22: each `def` gets a unique mangled name so redefinition
    // doesn't collide (`pyfn_{name}` for the first, `pyfn_{name}__redef_{n}`
    // for subsequent). The global function-pointer slot is initialized to
    // null and updated at each def's source position; calls dispatch
    // indirectly through the slot.
    let mir = MirModule {
        items: vec![
            MirItem::Function {
                name: "foo".to_string(),
                params: vec![("x".to_string(), Ty::Int)],
                return_ty: Ty::Int,
                body: vec![MirStmt::Return(Some(MirExpr::Name {
                    name: "x".to_string(),
                    ty: Ty::Int,
                }))],
            },
            MirItem::Function {
                name: "foo".to_string(),
                params: vec![("x".to_string(), Ty::Int)],
                return_ty: Ty::Int,
                body: vec![MirStmt::Return(Some(MirExpr::Name {
                    name: "x".to_string(),
                    ty: Ty::Int,
                }))],
            },
            // Call foo(42) and print the result -- exercises the
            // indirect call dispatch through the function-pointer slot.
            MirItem::TopLevelStmt(print_expr(MirExpr::Call {
                callee: "foo".to_string(),
                args: vec![MirExpr::IntLiteral(42)],
                ty: Ty::Int,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("fn_redef_unique_names")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("fn_redef_unique_names.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("fn_redef_unique_names");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    // The second definition (which returns its argument) is the one
    // bound at call time, so foo(42) should print 42.
    assert_eq!(
        output.stdout,
        b"42
"
    );
    assert!(output.status.success());
}

#[test]
fn set_add_grows_the_set_and_a_repeated_value_still_dedups_codegens_and_runs() {
    // `s = {1,2}\ns.add(3)\nprint(len(s))\ns.add(1)\nprint(len(s))\n`
    // end to end -- real `MirExpr::SetAdd` codegen, the second,
    // user-facing call site for the already-existing
    // `pycc_rt_int_set_add` (`SetLiteral`'s own per-element
    // construction is the first). Two separate statements, each its
    // own independent `MirStmt`, executed strictly in source order --
    // this task's own brief flags `s.add(x); s.add(x)`-shaped repeated
    // calls as a shape to verify dedup still holds for. Expected
    // output verified against `python3` on this exact source: `3`
    // then `3` again (the repeated `.add(1)` does not grow the set).
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "s".to_string(),
                value: MirExpr::SetLiteral {
                    elements: vec![MirExpr::IntLiteral(1), MirExpr::IntLiteral(2)],
                    ops: None,
                },
            }),
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::SetAdd {
                ops: None,
                set: "s".to_string(),
                value: Box::new(MirExpr::IntLiteral(3)),
            })),
            MirItem::TopLevelStmt(print_expr(MirExpr::Call {
                callee: "len".to_string(),
                args: vec![set_name("s")],
                ty: Ty::Int,
            })),
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::SetAdd {
                ops: None,
                set: "s".to_string(),
                value: Box::new(MirExpr::IntLiteral(1)),
            })),
            MirItem::TopLevelStmt(print_expr(MirExpr::Call {
                callee: "len".to_string(),
                args: vec![set_name("s")],
                ty: Ty::Int,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("set_add_grows_and_dedups")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("set_add_grows_and_dedups.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("set_add_grows_and_dedups");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"3\n3\n");
}

/// Test-only linking helper. `pycc`'s real CLI (Task 8) does this via
/// `cc`/clang (see `src/main.rs`'s `linker_command`/`effective_link_target`/
/// `add_windows_system_libs`/`add_linux_system_libs`); duplicated
/// minimally here so pycc_codegen's own tests can prove the object file
/// it produces actually links and runs, without depending on the `pycc`
/// binary crate (that would be a dependency cycle: pycc depends on
/// pycc_codegen, not the other way around). Needs the same Windows
/// handling as `main.rs`, and for the same reasons: there's no default
/// `cc` there (D-028) -- on this runner it silently resolved to
/// MinGW's `gcc`, which cannot link the MSVC-ABI `pycc_rt.lib` (the
/// exact "undefined reference to `__imp_...`"/`collect2` wall D-028
/// already diagnosed for `main.rs`, reproduced here because this
/// helper wasn't covered by that fix); clang's bare-invocation default
/// target also proved unreliable (D-028), so `-target` must be
/// explicit too. Needs the same Linux handling too, for the same
/// reason `main.rs` does (`f64::powf` -> libm's `pow`, not linked by
/// GCC's/clang's default driver invocation): this helper's own
/// `undefined reference to 'pow'` failure on both Linux architectures
/// wasn't covered by that fix either, since it's a separate linker
/// invocation from `main.rs`'s.
fn link_object_with_runtime(obj_path: &std::path::Path, bin_path: &std::path::Path) {
    // `CARGO_MANIFEST_DIR` is this *crate*'s directory, so the
    // workspace root is two levels up; `parent()` twice rather than a
    // `..` join keeps the rendered path readable in linker errors.
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("crate manifest dir has a workspace-root grandparent");
    let target_root = crate::artifact_layout::resolve_cargo_target_root(
        workspace_root,
        crate::artifact_layout::cargo_target_dir_from_env,
    );
    let rt_lib_dir = crate::artifact_layout::find_pycc_rt_lib_dir_in(
        &target_root,
        None,
        false,
        std::path::Path::exists,
    )
    .expect("pycc_rt debug build must exist before these link-and-run tests");

    #[cfg(windows)]
    let mut cmd = {
        let clang = std::path::Path::new(env!("LLVM_SYS_221_PREFIX"))
            .join("bin")
            .join("clang.exe");
        let mut cmd = Command::new(clang);
        cmd.arg("-target").arg("x86_64-pc-windows-msvc");
        cmd
    };
    #[cfg(not(windows))]
    let mut cmd = Command::new("cc");

    cmd.arg(obj_path)
        .arg("-L")
        .arg(&rt_lib_dir)
        .arg("-lpycc_rt")
        .arg("-o")
        .arg(bin_path);

    #[cfg(windows)]
    for lib in [
        "ws2_32",
        "ntdll",
        "userenv",
        "advapi32",
        "shell32",
        "ole32",
        "uuid",
        "psapi",
        "dbghelp",
        "kernel32",
        "legacy_stdio_definitions",
    ] {
        cmd.arg(format!("-l{lib}"));
    }

    #[cfg(target_os = "linux")]
    cmd.arg("-lm");

    let status = cmd.status().expect("the linker driver should run");
    assert!(status.success(), "linking failed");
}

// -- #436: NullInstance codegen (class method called through a class) --

/// #436: A `@classmethod` called through a class name (`C.greet(21)`)
/// lowers to MIR with a `MirExpr::NullInstance` as the first argument
/// (the `cls` receiver). This test verifies that codegen emits a null
/// pointer for `NullInstance` and that the resulting binary runs
/// correctly, producing the expected output.
///
///     class C:
///         def __init__(self) -> None:
///             return
///         @classmethod
///         def greet(cls, x: int) -> int:
///             return x * 2
///
///     print(C.greet(21))
///
/// Expected output: `42`.
#[test]
fn null_instance_classmethod_codegens_and_runs() {
    let self_ty = instance_ty("C");
    let init = MirItem::Function {
        name: "C.__init__".to_string(),
        params: vec![("self".to_string(), self_ty.clone())],
        return_ty: Ty::None,
        body: vec![MirStmt::Return(None)],
    };
    let greet = MirItem::Function {
        name: "C.greet.classmethod".to_string(),
        params: vec![
            ("cls".to_string(), self_ty.clone()),
            ("x".to_string(), Ty::Int),
        ],
        return_ty: Ty::Int,
        body: vec![MirStmt::Return(Some(MirExpr::BinOp {
            op: BinOpKind::Mul,
            left: Box::new(MirExpr::Name {
                name: "x".to_string(),
                ty: Ty::Int,
            }),
            right: Box::new(MirExpr::IntLiteral(2)),
            ty: Ty::Int,
        }))],
    };
    let mir = MirModule {
        items: vec![
            init,
            greet,
            MirItem::TopLevelStmt(print_expr(MirExpr::Call {
                callee: "C.greet.classmethod".to_string(),
                args: vec![
                    MirExpr::NullInstance { ty: self_ty },
                    MirExpr::IntLiteral(21),
                ],
                ty: Ty::Int,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("null_instance_classmethod")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("null_instance_classmethod.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("null_instance_classmethod");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"42\n");
}

#[test]
fn enum_member_singleton_init_emits_and_runs() {
    // #379: Exercises `emit_enum_member_inits` by building a MIR
    // module with an enum class definition and a top-level statement
    // that reads a member's value. The codegen must emit the
    // per-member singleton init sequence before the top-level
    // statement loop.
    let class_def = pycc_mir::HirClassDef {
        class_attrs: Vec::new(),
        exception_type_tag: None,
        name: "Color".to_string(),
        bases: vec![],
        mro: vec!["Color".to_string()],
        attrs: vec![
            ("value".to_string(), Ty::Int),
            ("name".to_string(), Ty::Str),
        ],
        methods: vec![],
        properties: vec![],
        static_methods: vec![],
        class_methods: vec![],
        type_param: None,
        is_enum: false,
        implicit_object_init: false,
        method_defaults: Vec::new(),
        enum_members: vec![
            ("RED".to_string(), pycc_mir::EnumMemberValue::Int(1)),
            ("GREEN".to_string(), pycc_mir::EnumMemberValue::Int(2)),
        ],
        is_dataclass: false,
        dataclass_fields: Vec::new(),
        is_protocol: false,
        runtime_checkable: false,
        protocol_members: Vec::new(),
        abstract_methods: Vec::new(),
        is_abstract: false,
    };
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
            callee: "print".to_string(),
            args: vec![MirExpr::AttrGet {
                base: Box::new(MirExpr::Name {
                    name: "Color.RED.enum_member".to_string(),
                    ty: Ty::Instance(Box::new("Color".to_string())),
                }),
                slot: 0,
                ty: Ty::Int,
            }],
            ty: Ty::None,
        }))],
        class_defs: vec![("Color".to_string(), class_def)],
    };
    let dir =
        pycc_scratch::ScratchDir::new("enum_member_init").expect("failed to create scratch dir");
    let obj_path = dir.join("enum_member_init.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("enum_member_init");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"1\n");
}

#[test]
fn str_valued_enum_member_singleton_init_emits_and_runs() {
    // #892: the `Scalar::Str` arm of `emit_enum_member_inits`. A
    // `str`-valued member's slot 0 holds a runtime string object built by
    // `emit_string_literal`, not a tagged int, and the class's `value`
    // attribute is typed `Ty::Str`. Building and *running* the binary is
    // what proves the emitted init sequence is well-formed -- a codegen-only
    // assertion would not catch a mis-tagged slot word.
    let class_def = pycc_mir::HirClassDef {
        class_attrs: Vec::new(),
        exception_type_tag: None,
        name: "Kind".to_string(),
        bases: vec![],
        mro: vec!["Kind".to_string()],
        attrs: vec![
            ("value".to_string(), Ty::Str),
            ("name".to_string(), Ty::Str),
        ],
        methods: vec![],
        properties: vec![],
        static_methods: vec![],
        class_methods: vec![],
        type_param: None,
        is_enum: false,
        implicit_object_init: false,
        method_defaults: Vec::new(),
        enum_members: vec![
            (
                "AXIAL".to_string(),
                pycc_mir::EnumMemberValue::Str("axial".to_string()),
            ),
            (
                "RADIAL".to_string(),
                pycc_mir::EnumMemberValue::Str("radial".to_string()),
            ),
        ],
        is_dataclass: false,
        dataclass_fields: Vec::new(),
        is_protocol: false,
        runtime_checkable: false,
        protocol_members: Vec::new(),
        abstract_methods: Vec::new(),
        is_abstract: false,
    };
    let read_member = |member: &str, slot: usize, ty: Ty| {
        MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
            callee: "print".to_string(),
            args: vec![MirExpr::AttrGet {
                base: Box::new(MirExpr::Name {
                    name: format!("Kind.{member}.enum_member"),
                    ty: Ty::Instance(Box::new("Kind".to_string())),
                }),
                slot,
                ty,
            }],
            ty: Ty::None,
        }))
    };
    let mir = MirModule {
        items: vec![
            read_member("AXIAL", 0, Ty::Str),
            read_member("RADIAL", 0, Ty::Str),
            read_member("AXIAL", 1, Ty::Str),
        ],
        class_defs: vec![("Kind".to_string(), class_def)],
    };
    let dir = pycc_scratch::ScratchDir::new("str_enum_member_init")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("str_enum_member_init.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("str_enum_member_init");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"axial\nradial\nAXIAL\n");
}

// -- #380: default_value_for_type covers every Mir Ty variant --------

#[test]
fn default_value_for_type_covers_every_mir_ty_variant() {
    let context = Context::create();
    // Scalar types.
    let _ = default_value_for_type(&context, pycc_mir::Ty::Int);
    let _ = default_value_for_type(&context, pycc_mir::Ty::Bool);
    let _ = default_value_for_type(&context, pycc_mir::Ty::Float);
    let _ = default_value_for_type(&context, pycc_mir::Ty::Str);
    let _ = default_value_for_type(&context, pycc_mir::Ty::None);
    let _ = default_value_for_type(&context, pycc_mir::Ty::Infer);
    let _ = default_value_for_type(&context, pycc_mir::Ty::Param(Box::new("T".to_string())));
    // Container types.
    let _ = default_value_for_type(&context, pycc_mir::Ty::List(Box::new(pycc_mir::Ty::Int)));
    let _ = default_value_for_type(
        &context,
        pycc_mir::Ty::Dict(Box::new((pycc_mir::Ty::Int, pycc_mir::Ty::Str))),
    );
    let _ = default_value_for_type(&context, pycc_mir::Ty::Set(Box::new(pycc_mir::Ty::Int)));
    let _ = default_value_for_type(
        &context,
        pycc_mir::Ty::Instance(Box::new("Foo".to_string())),
    );
    let _ = default_value_for_type(&context, pycc_mir::Ty::Protocol(Box::new("P".to_string())));
    let _ = default_value_for_type(
        &context,
        pycc_mir::Ty::Tuple(Box::new(vec![pycc_mir::Ty::Int, pycc_mir::Ty::Str])),
    );
}

#[test]
fn abstract_method_body_with_non_none_return_emits_default_value() {
    // #380 (PR-20): an abstract method has a `Return(None)` body but
    // a non-`None` declared return type. Codegen must emit a correctly
    // typed default value so the LLVM IR is well-typed. This exercises
    // the `else` branch of the `Return(None)` arm in `emit_stmt`.
    let mir = MirModule {
        items: vec![
            MirItem::Function {
                name: "Animal.sound".to_string(),
                params: vec![(
                    "self".to_string(),
                    Ty::Instance(Box::new("Animal".to_string())),
                )],
                return_ty: Ty::Str,
                body: vec![MirStmt::Return(None)],
            },
            MirItem::Function {
                name: "main".to_string(),
                params: vec![],
                return_ty: Ty::None,
                body: vec![MirStmt::Return(None)],
            },
        ],
        class_defs: vec![(
            "Animal".to_string(),
            pycc_mir::HirClassDef {
                class_attrs: Vec::new(),
                exception_type_tag: None,
                name: "Animal".to_string(),
                bases: Vec::new(),
                mro: vec!["Animal".to_string()],
                attrs: Vec::new(),
                methods: vec![("sound".to_string(), "Animal.sound".to_string())],
                type_param: None,
                properties: Vec::new(),
                static_methods: Vec::new(),
                class_methods: Vec::new(),
                is_enum: false,
                implicit_object_init: false,
                method_defaults: Vec::new(),
                enum_members: Vec::new(),
                is_dataclass: false,
                dataclass_fields: Vec::new(),
                is_protocol: false,
                runtime_checkable: false,
                protocol_members: Vec::new(),
                abstract_methods: vec!["sound".to_string()],
                is_abstract: true,
            },
        )],
    };
    let dir = pycc_scratch::ScratchDir::new("abstract_default_ret")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("abstract_default_ret.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    // The binary is never run — the abstract method is never called.
    // We only need to verify that codegen produces valid LLVM IR.
}

// -- #381: MirStmt::Seq error propagation in emit_stmt ---------------

#[test]
fn calling_an_undefined_function_inside_a_seq_is_rejected() {
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::Seq(vec![
            call_print(1),
            call_user_fn("does_not_exist_in_seq"),
        ]))],
        class_defs: Vec::new(),
    };
    let dir =
        pycc_scratch::ScratchDir::new("seq_undefined_fn").expect("failed to create scratch dir");
    let obj_path = dir.join("seq_undefined_fn.o");
    let err = compile_to_object(&mir, &obj_path, None, false).expect_err("should be rejected");
    assert!(
        err.contains("does_not_exist_in_seq"),
        "error should name the offending function: {err}"
    );
}

#[test]
fn seq_with_valid_statements_compiles_and_runs() {
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::Seq(vec![
            call_print(10),
            call_print(20),
        ]))],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("seq_valid").expect("failed to create scratch dir");
    let obj_path = dir.join("seq_valid.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("seq_valid");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"10\n20\n");
}

// -- #381: None singleton pattern comparison in emit_expr ---------------

#[test]
fn none_singleton_comparison_emits_zero_carrier() {
    // Exercises the `MirExpr::Name { name, ty: Ty::None } if name ==
    // "None"` arm of `emit_expr` (line 1633).  The MIR mirrors what
    // `lower_pattern_conds` produces for `match x: case None:`.
    let mir = MirModule {
        items: vec![
            MirItem::Function {
                name: "get_none".to_string(),
                params: vec![],
                return_ty: Ty::None,
                body: vec![MirStmt::Return(Some(MirExpr::Name {
                    name: "None".to_string(),
                    ty: Ty::None,
                }))],
            },
            MirItem::TopLevelStmt(MirStmt::If {
                test: MirExpr::Compare {
                    op: CmpOpKind::Eq,
                    left: Box::new(MirExpr::Call {
                        callee: "get_none".to_string(),
                        args: vec![],
                        ty: Ty::None,
                    }),
                    right: Box::new(MirExpr::Name {
                        name: "None".to_string(),
                        ty: Ty::None,
                    }),
                    ty: Ty::Bool,
                },
                body: vec![call_print(1)],
                orelse: vec![call_print(0)],
            }),
        ],
        class_defs: Vec::new(),
    };
    let dir =
        pycc_scratch::ScratchDir::new("none_singleton_cmp").expect("failed to create scratch dir");
    let obj_path = dir.join("none_singleton_cmp.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("none_singleton_cmp");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"1\n");
}

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

/// Helper: a `MirStmt::Return(Some(MirExpr::IntLiteral(n)))` — used in
/// function bodies to terminate the current block so that the enclosing
/// try's `*_falls_through` check is false.
fn return_int(n: i64) -> MirStmt {
    MirStmt::Return(Some(MirExpr::IntLiteral(n)))
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

// --- #812 review finding: bare `None` through the argument/return paths
// (`coerce_scalar_to_type`'s `(Ty::Optional(inner), Scalar::Optional(v))`
// arm, not the `OptionalWrap`-mediated assignment path the tests above
// exercise) -----------------------------------------------------------

#[test]
fn optional_float_bare_none_argument_constructs_a_correctly_typed_absent_struct() {
    // `def f(x: float | None): print(x is None); print(x is not None)` ;
    // `f(None)` -- a bare `None` literal argument reaches
    // `build_call_to_with_leading_args`'s own `coerce_scalar_to_type` call
    // directly, with no `MirExpr::OptionalWrap` wrapper in between (only
    // `HirStmt::Assign`/`HirStmt::AnnAssign` construct `OptionalWrap`).
    // Before this PR's fix, the mismatched-struct branch always inserted
    // `tag_smallint_const` (an `i64`) into field 0 regardless of the
    // parameter's declared inner type, producing a malformed
    // `{ f64, i8 }` constant for `Ty::Float` that neither `module.verify()`
    // nor LLVM's own IR printer rejected.
    let mir = MirModule {
        items: vec![
            MirItem::Function {
                name: "f".to_string(),
                params: vec![("x".to_string(), optional_float())],
                return_ty: Ty::None,
                body: vec![
                    print_expr(MirExpr::Compare {
                        op: pycc_mir::CmpOpKind::Is,
                        left: Box::new(MirExpr::Name {
                            name: "x".to_string(),
                            ty: optional_float(),
                        }),
                        right: Box::new(MirExpr::NoneLiteral),
                        ty: Ty::Bool,
                    }),
                    print_expr(MirExpr::Compare {
                        op: pycc_mir::CmpOpKind::IsNot,
                        left: Box::new(MirExpr::Name {
                            name: "x".to_string(),
                            ty: optional_float(),
                        }),
                        right: Box::new(MirExpr::NoneLiteral),
                        ty: Ty::Bool,
                    }),
                ],
            },
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
                callee: "f".to_string(),
                args: vec![MirExpr::NoneLiteral],
                ty: Ty::None,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("optional_float_bare_none_argument")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("optional_float_bare_none_argument.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("optional_float_bare_none_argument");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"True\nFalse\n");
}

#[test]
fn optional_bool_bare_none_argument_constructs_a_correctly_typed_absent_struct() {
    // The `Optional[bool]` counterpart of the `Optional[float]` argument
    // test immediately above. `Ty::Bool`'s real `{ i8, i8 }` shape happens
    // to collide with the placeholder's own fixed `{ i8, i8 }` (see
    // `optional_bool_none_placeholder_and_real_absent_value_are_the_same_
    // llvm_struct_type`), so this specific inner type was never at risk of
    // the field-0-type-mismatch this PR fixes -- kept for parity with the
    // rest of this file's float/bool pairing and to pin the argument path
    // for both admitted inner types.
    let mir = MirModule {
        items: vec![
            MirItem::Function {
                name: "f".to_string(),
                params: vec![("x".to_string(), optional_bool())],
                return_ty: Ty::None,
                body: vec![
                    print_expr(MirExpr::Compare {
                        op: pycc_mir::CmpOpKind::Is,
                        left: Box::new(MirExpr::Name {
                            name: "x".to_string(),
                            ty: optional_bool(),
                        }),
                        right: Box::new(MirExpr::NoneLiteral),
                        ty: Ty::Bool,
                    }),
                    print_expr(MirExpr::Compare {
                        op: pycc_mir::CmpOpKind::IsNot,
                        left: Box::new(MirExpr::Name {
                            name: "x".to_string(),
                            ty: optional_bool(),
                        }),
                        right: Box::new(MirExpr::NoneLiteral),
                        ty: Ty::Bool,
                    }),
                ],
            },
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
                callee: "f".to_string(),
                args: vec![MirExpr::NoneLiteral],
                ty: Ty::None,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("optional_bool_bare_none_argument")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("optional_bool_bare_none_argument.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("optional_bool_bare_none_argument");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"True\nFalse\n");
}

#[test]
fn optional_float_bare_none_return_constructs_a_correctly_typed_absent_struct() {
    // `def g() -> float | None: return None` ; `z = g()` ;
    // `print(z is None); print(z is not None)` -- a bare `None` literal
    // `return` reaches `coerce_scalar_to_type` from the function's own
    // return-handling call site, again with no `OptionalWrap` wrapper.
    let mir = MirModule {
        items: vec![
            MirItem::Function {
                name: "g".to_string(),
                params: vec![],
                return_ty: optional_float(),
                body: vec![MirStmt::Return(Some(MirExpr::NoneLiteral))],
            },
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "z".to_string(),
                value: MirExpr::Call {
                    callee: "g".to_string(),
                    args: vec![],
                    ty: optional_float(),
                },
            }),
            MirItem::TopLevelStmt(print_expr(MirExpr::Compare {
                op: pycc_mir::CmpOpKind::Is,
                left: Box::new(MirExpr::Name {
                    name: "z".to_string(),
                    ty: optional_float(),
                }),
                right: Box::new(MirExpr::NoneLiteral),
                ty: Ty::Bool,
            })),
            MirItem::TopLevelStmt(print_expr(MirExpr::Compare {
                op: pycc_mir::CmpOpKind::IsNot,
                left: Box::new(MirExpr::Name {
                    name: "z".to_string(),
                    ty: optional_float(),
                }),
                right: Box::new(MirExpr::NoneLiteral),
                ty: Ty::Bool,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("optional_float_bare_none_return")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("optional_float_bare_none_return.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("optional_float_bare_none_return");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"True\nFalse\n");
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
    output.stdout
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

// ---------------------------------------------------------------------------
// D-244, #1026: the `Scalar::Object` arms.
//
// Most arms below are defensive: `pycc_types`' Part 2 refusal migration
// rejects a `Ty::Object` operand at those consuming sites (see
// `docs/TYPE_SYSTEM.md`'s `object` row), so no type-checked program reaches
// them. Each is pinned directly with a hand-built `Scalar::Object` carrying
// a null `PyObject *` -- the same convention the `Scalar::List` defensive
// tests above use, and for the same reason: it pins the panic to the
// function that owns the gap rather than to whichever caller reaches it
// first. The two *reachable* `Ty::Object` paths (`MirStmt::Return`'s
// pass-through and `call_result_scalar`'s arm) are covered by real MIR at
// the end of this section instead.
//
// `truthy`'s arm is the exception, and PR 3a of #1082 is what made it one:
// it no longer panics, because the ten condition-position refusals that
// justified the panic are gone. Its test below therefore asserts the shim
// call it emits rather than a message.
// ---------------------------------------------------------------------------

/// A null `PyObject *` as a [`Scalar::Object`]. None of the defensive tests
/// below dereferences it -- each panics inside its function's own `match`
/// before any `build_*` call runs.
fn null_object_scalar(context: &Context) -> Scalar<'_> {
    Scalar::Object(
        context
            .ptr_type(inkwell::AddressSpace::default())
            .const_null(),
    )
}

#[test]
#[should_panic(expected = "expected an int-or-bool operand, got object")]
fn to_numeric_encoded_int_rejects_a_cpython_object_operand() {
    let context = Context::create();
    let builder = context.create_builder();
    to_numeric_encoded_int(&context, &builder, null_object_scalar(&context));
}

#[test]
#[should_panic(expected = "expected a numeric operand, got object")]
fn to_float_rejects_a_cpython_object_operand() {
    let context = Context::create();
    let (_module, rt) = list_scalar_panic_fixture(&context);
    let builder = context.create_builder();
    to_float(&context, &builder, &rt, null_object_scalar(&context));
}

#[test]
#[should_panic(expected = "string conversion of a CPython object value must go through the shim")]
fn to_str_rejects_a_cpython_object_operand() {
    // Rendering a foreign object needs a `pycc_ext_obj_*` shim call and its
    // failure edge, which this plain runtime-call helper cannot emit -- so
    // since #1340 both callers (`string_render.rs`) convert an object
    // operand before reaching `to_str`, and this arm is the backstop.
    let context = Context::create();
    let (_module, rt) = list_scalar_panic_fixture(&context);
    let builder = context.create_builder();
    to_str(&builder, &rt, null_object_scalar(&context));
}

/// Part 3 of #1026 (PR 3a of #1082): `truthy`'s `Scalar::Object` arm used
/// to be the defensive panic this test pinned. It is now a real capability
/// -- a `pycc_ext_obj_truthy` call plus the module-exec failure edge -- so
/// the assertion moved from a message to the emitted IR.
///
/// The direct `truthy` call is kept rather than compiling real MIR (the
/// module-level shapes are covered in `foreign_len.rs`'s own tests) because
/// it pins the arm to the function that owns it, exactly as
/// `truthiness_of_a_list_value_panics_honestly` above does. It does need a
/// positioned block inside a function spelled [`EXT_MODULE_EXEC_SYMBOL`],
/// because the arm asserts that enclosing function before appending the
/// failure edge's blocks.
#[test]
fn truthiness_of_a_cpython_object_value_calls_the_shim_helper() {
    let context = Context::create();
    let (module, rt) = list_scalar_panic_fixture(&context);
    let builder = context.create_builder();
    let entry = module.add_function(
        EXT_MODULE_EXEC_SYMBOL,
        context.i64_type().fn_type(&[], false),
        None,
    );
    builder.position_at_end(context.append_basic_block(entry, "entry"));
    truthy(
        &context,
        &builder,
        &module,
        &rt,
        null_object_scalar(&context),
    );
    let ir = {
        use inkwell::values::AnyValue;
        crate::llvm_string_to_owned(entry.print_to_string())
    };
    assert!(ir.contains(EXT_OBJ_TRUTHY_SYMBOL), "{ir}");
    assert!(ir.contains("foreign_truthy_failed"), "{ir}");
    assert!(
        ir.contains(&format!("ret i64 {EXT_MODULE_EXEC_FAILED}")),
        "{ir}"
    );
}

/// `import numpy` spliced ahead of `items`, so a `MirExpr::Name` read of
/// `numpy` typed `Ty::Object` resolves to the module global the foreign
/// import emits.
fn with_foreign_numpy(mut items: Vec<MirItem>) -> Vec<MirItem> {
    items.insert(
        0,
        MirItem::ForeignImport {
            local_name: "numpy".to_string(),
            module_path: "numpy".to_string(),
            from: None,
        },
    );
    items
}

/// `numpy.pi` as a `MirExpr`, the one producer shape PR 2a emits.
fn numpy_pi() -> MirExpr {
    MirExpr::ObjAttrGet {
        base: Box::new(MirExpr::Name {
            name: "numpy".to_string(),
            ty: Ty::Object,
        }),
        attr: "pi".to_string(),
        ty: Ty::Object,
    }
}

/// Compiles `items` as a D-244 `ext` object, which is the only mode a
/// foreign import is emitted in.
fn compile_ext_items(label: &str, items: Vec<MirItem>) {
    let dir = pycc_scratch::ScratchDir::new(label).expect("failed to create scratch dir");
    compile_to_object_with_options(
        &MirModule {
            items,
            ..Default::default()
        },
        &dir.join(format!("{label}.o")),
        &CompileOptions {
            ext: true,
            ..CompileOptions::default()
        },
    )
    .expect("ext codegen should succeed");
}

#[test]
fn a_private_helper_may_return_a_cpython_object_and_its_result_is_discarded() {
    // The two `Ty::Object` codegen paths that survive PR 2a's narrowing,
    // together in one program: `emit_stmt`'s `MirStmt::Return`
    // pass-through carries a `PyObject *` out of `_module`, and
    // `call_result_scalar`'s `Ty::Object` arm turns the call site's result
    // back into a `Scalar::Object`. Nothing dereferences it -- the
    // `ExprStmt` discards it.
    //
    // The returned value is the foreign module global itself rather than a
    // `numpy.pi` load so the test selects exactly these two arms; an
    // `ObjAttrGet` inside a function body has its own failure-edge tests in
    // `foreign_fail_tests.rs` (#1316). `pycc_types` admits returning a
    // CPython object from a function since Part 1 of #1333, so this is
    // also a shape the front end produces.
    //
    // A public function returning `object` -- spellable since Part 1 of
    // #1367 through a class a foreign import binds -- is exported since
    // #1397 (unchecked against the class, #1386), but its thunk is a
    // separate seam this test does not select.
    compile_ext_items(
        "object_returning_helper",
        with_foreign_numpy(vec![
            MirItem::Function {
                name: "_module".to_string(),
                params: vec![],
                return_ty: Ty::Object,
                body: vec![MirStmt::Return(Some(MirExpr::Name {
                    name: "numpy".to_string(),
                    ty: Ty::Object,
                }))],
            },
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
                callee: "_module".to_string(),
                args: vec![],
                ty: Ty::Object,
            })),
        ]),
    );
}

// -- PR 3c of #1082: the `ForObject` storage-slot type ----------------------

/// `collect_stmt_bindings` produces exactly one storage-slot type per name,
/// so a `for x in <object>:` target must claim an `Object` (pointer) slot
/// even when an earlier statement in the same body already bound the name to
/// a narrower scalar. An `or_insert` here would keep the `Int` and leave the
/// loop storing a `PyObject *` into an `i64` slot -- a miscompilation the
/// `local type drifted` `debug_assert` cannot catch in a release build.
/// `pycc_types` refuses that source shape with `T0023`, so this unit test is
/// the only place the choice is observable.
#[test]
fn a_for_object_target_claims_an_object_slot_over_an_earlier_int_binding() {
    let mut bindings = BTreeMap::new();
    collect_stmt_bindings(
        &MirStmt::Assign {
            target: "x".to_string(),
            value: MirExpr::IntLiteral(5),
        },
        &mut bindings,
    );
    assert_eq!(bindings.get("x"), Some(&Ty::Int));
    collect_stmt_bindings(
        &MirStmt::ForObject {
            var: "x".to_string(),
            iter: numpy_pi(),
            body: Vec::new(),
        },
        &mut bindings,
    );
    assert_eq!(bindings.get("x"), Some(&Ty::Object));
}

// ---------------------------------------------------------------------------
// Part 2 of #1027 and #1116: the `Scalar::MemoryView` arms,
// `MirExpr::BufferGet` and `MirExpr::BufferLen`.
//
// A `memoryview` reaches codegen only as a parameter of a `pycc build --ext`
// export (D-244), and `reject_memoryview_read` refuses every *use* of such a
// name except the `b[i]` element load and the `len(b)` count these parts
// admit. Every consuming arm
// below is therefore defensive, and each is pinned directly with a
// hand-built `Scalar::MemoryView` carrying a null `PyccExtBufferView *` --
// the same convention the `Scalar::List` and `Scalar::Object` defensive
// tests above use, and for the same reason: it pins the panic to the
// function that owns the gap rather than to whichever caller reaches it
// first. The *reachable* paths -- `MirExpr::Name`'s pointer-slot load and
// the `BufferGet`/`BufferLen` emissions themselves -- are covered by real
// MIR at the end of this section.
// ---------------------------------------------------------------------------

/// A null `PyccExtBufferView *` as a [`Scalar::MemoryView`]. None of the
/// defensive tests below dereferences it -- each panics inside its
/// function's own `match` before any `build_*` call runs.
fn null_memoryview_scalar(context: &Context) -> Scalar<'_> {
    Scalar::MemoryView(
        context
            .ptr_type(inkwell::AddressSpace::default())
            .const_null(),
    )
}

#[test]
#[should_panic(expected = "expected an int-or-bool operand, got memoryview")]
fn to_numeric_encoded_int_rejects_a_memoryview_operand() {
    let context = Context::create();
    let builder = context.create_builder();
    to_numeric_encoded_int(&context, &builder, null_memoryview_scalar(&context));
}

#[test]
#[should_panic(expected = "expected a numeric operand, got memoryview")]
fn to_float_rejects_a_memoryview_operand() {
    // A buffer *element* is a `float`, but the buffer itself is not: there
    // is no `float(b)` in CPython either.
    let context = Context::create();
    let (_module, rt) = list_scalar_panic_fixture(&context);
    let builder = context.create_builder();
    to_float(&context, &builder, &rt, null_memoryview_scalar(&context));
}

#[test]
#[should_panic(expected = "string conversion of a memoryview value is not supported yet")]
fn to_str_rejects_a_memoryview_operand() {
    // `print(b)` and `f"{b}"` are both bare reads of the name, which
    // `reject_memoryview_read` refuses with `C0001` before codegen.
    let context = Context::create();
    let (_module, rt) = list_scalar_panic_fixture(&context);
    let builder = context.create_builder();
    to_str(&builder, &rt, null_memoryview_scalar(&context));
}

#[test]
#[should_panic(expected = "truthiness of a memoryview value is not supported yet")]
fn truthiness_of_a_memoryview_value_panics_honestly() {
    // `if b:` is a bare read of the name, so `reject_memoryview_read`
    // refuses it with `C0001`. CPython's own answer would be `len(b) != 0`,
    // which #1027 does not ship -- an honest panic naming the gap is the
    // correct behavior until it does.
    let context = Context::create();
    let (module, rt) = list_scalar_panic_fixture(&context);
    let builder = context.create_builder();
    truthy(
        &context,
        &builder,
        &module,
        &rt,
        null_memoryview_scalar(&context),
    );
}

/// An `--ext` module holding one function whose first parameter is a
/// `memoryview`, which is the only shape that binding can have.
fn buffer_fn_items(body: Vec<MirStmt>, return_ty: Ty) -> Vec<MirItem> {
    vec![MirItem::Function {
        name: "element".to_string(),
        params: vec![
            ("b".to_string(), Ty::MemoryView),
            ("i".to_string(), Ty::Int),
        ],
        return_ty,
        body,
    }]
}

/// `b[i]` as a `MirExpr`, the one buffer shape Part 2 of #1027 emits.
fn buffer_get_b_i() -> MirExpr {
    MirExpr::BufferGet {
        base: Box::new(MirExpr::Name {
            name: "b".to_string(),
            ty: Ty::MemoryView,
        }),
        index: Box::new(MirExpr::Name {
            name: "i".to_string(),
            ty: Ty::Int,
        }),
    }
}

/// `len(b)` as a `MirExpr`, the buffer shape #1116 adds.
fn buffer_len_b() -> MirExpr {
    MirExpr::BufferLen {
        base: Box::new(MirExpr::Name {
            name: "b".to_string(),
            ty: Ty::MemoryView,
        }),
    }
}

/// Compiles `items` as a D-244 `ext` object and hands the whole module's IR
/// to `check`.
fn compile_ext_items_checking_ir(label: &str, items: Vec<MirItem>, check: impl Fn(&str)) {
    let dir = pycc_scratch::ScratchDir::new(label).expect("failed to create scratch dir");
    let mut seen = String::new();
    // D-029: never let the `LLVMString` temporary drop -- route it through
    // `llvm_string_to_owned`, exactly as this crate's error paths do.
    let mut observer = |module: &inkwell::module::Module<'_>, _| {
        seen = llvm_string_to_owned(module.print_to_string());
    };
    compile_to_object_with_observer(
        &MirModule {
            items,
            ..Default::default()
        },
        &dir.join(format!("{label}.o")),
        &CompileOptions {
            ext: true,
            ..CompileOptions::default()
        },
        Some(&mut observer),
    )
    .expect("ext codegen should succeed");
    check(&seen);
}

#[test]
fn a_buffer_element_load_untags_its_index_and_calls_the_runtime_helper() {
    // The whole admitted read surface, end to end through real MIR: the
    // `memoryview` parameter's pointer slot is loaded (`MirExpr::Name`'s
    // own `Ty::MemoryView` arm), the D-141-tagged index is decoded through
    // the shared checked untag, and the pair is handed to the runtime
    // helper that owns the bounds check. Nothing here dereferences the
    // buffer: `pycc_rt_buffer_f64_get` does, after it has compared the
    // index against the view's recorded length.
    compile_ext_items_checking_ir(
        "buffer_element_load",
        buffer_fn_items(vec![MirStmt::Return(Some(buffer_get_b_i()))], Ty::Float),
        |ir| {
            assert!(ir.contains("call i64 @pycc_rt_int_untag_checked"), "{ir}");
            assert!(ir.contains("call double @pycc_rt_buffer_f64_get"), "{ir}");
        },
    );
}

#[test]
fn a_buffer_length_read_calls_the_runtime_helper_and_tags_the_count() {
    // #1116, end to end through real MIR. Two properties, both of which a
    // `return len(b)` test alone would pass without: the raw `i64` count is
    // re-tagged into D-141's form before it becomes a user-visible `int`
    // (`list_tag_shl`/`list_tag_or`, the same pair `raw_i64_to_tagged_int`
    // emits for `pycc_rt_int_list_len`), and no untag of an index is
    // emitted, because there is no index. An untagged count would compile,
    // return plausibly, and then produce a wrong iteration count the moment
    // it is used as a `range` bound.
    compile_ext_items_checking_ir(
        "buffer_length_read",
        buffer_fn_items(vec![MirStmt::Return(Some(buffer_len_b()))], Ty::Int),
        |ir| {
            assert!(ir.contains("call i64 @pycc_rt_buffer_len"), "{ir}");
            assert!(ir.contains("list_tag_shl"), "{ir}");
            assert!(ir.contains("list_tag_or"), "{ir}");
        },
    );
}

#[test]
fn a_buffer_length_read_emits_no_pending_exception_check() {
    // `pycc_rt_buffer_len` cannot raise, so `expression_can_set_exception`
    // answers `false` for this node where it answers `true` for both its
    // siblings. Asserted as a pair so a regression in either direction is
    // caught: the same function shape emits exactly one D-173 guard when its
    // body is the fallible element load, and none at all when it is the
    // length read. A `true` classification here would emit a never-taken
    // branch after every `len(b)`.
    let guard = "call i8 @pycc_rt_exception_active()";
    compile_ext_items_checking_ir(
        "buffer_length_no_guard",
        buffer_fn_items(vec![MirStmt::Return(Some(buffer_len_b()))], Ty::Int),
        |ir| {
            assert_eq!(ir.matches(guard).count(), 0, "{ir}");
        },
    );
    compile_ext_items_checking_ir(
        "buffer_load_has_guard",
        buffer_fn_items(vec![MirStmt::Return(Some(buffer_get_b_i()))], Ty::Float),
        |ir| {
            assert_eq!(ir.matches(guard).count(), 1, "{ir}");
        },
    );
}

#[test]
#[should_panic(expected = "a buffer length read's base did not evaluate to a memoryview")]
fn a_buffer_length_read_whose_base_is_not_a_memoryview_is_an_internal_error() {
    // The same argument the element load's own defensive arm records:
    // `MirExpr::BufferLen` is produced by exactly one lowering arm, which
    // keys on the lowered argument's `Ty::MemoryView`, so a base of any
    // other type means that dispatch regressed.
    compile_ext_items_checking_ir(
        "buffer_length_bad_base",
        buffer_fn_items(
            vec![MirStmt::Return(Some(MirExpr::BufferLen {
                base: Box::new(MirExpr::Name {
                    name: "i".to_string(),
                    ty: Ty::Int,
                }),
            }))],
            Ty::Int,
        ),
        |_| unreachable!("codegen should have panicked"),
    );
}

#[test]
#[should_panic(expected = "a buffer element store's base did not evaluate to a memoryview")]
fn a_buffer_element_store_whose_base_is_not_a_memoryview_is_an_internal_error() {
    // `MirStmt::BufferSet` is produced by exactly one lowering arm, which
    // keys on the target name's `Ty::MemoryView` (`pycc_mir`'s `DictSet`
    // arm), so a base of any other type means that dispatch regressed --
    // the same argument the load's and the length read's own defensive
    // arms record. The deliberately mistyped `int` base below is a shape
    // no type-checked program can produce.
    compile_ext_items_checking_ir(
        "buffer_element_store_bad_base",
        buffer_fn_items(
            vec![MirStmt::BufferSet {
                base: MirExpr::Name {
                    name: "i".to_string(),
                    ty: Ty::Int,
                },
                index: MirExpr::IntLiteral(0),
                value: MirExpr::FloatLiteral(1.0),
            }],
            Ty::None,
        ),
        |_| unreachable!("codegen should have panicked"),
    );
}

/// The store emits exactly one D-173 guard, where the length read emits
/// none and the element load emits one: the void setter's only exception
/// path is the `IndexError` the runtime helper raises, so dropping the
/// guard would let an out-of-range store run the rest of the body.
#[test]
fn a_buffer_element_store_emits_one_pending_exception_guard() {
    compile_ext_items_checking_ir(
        "buffer_store_has_guard",
        buffer_fn_items(
            vec![MirStmt::BufferSet {
                base: MirExpr::Name {
                    name: "b".to_string(),
                    ty: Ty::MemoryView,
                },
                index: MirExpr::Name {
                    name: "i".to_string(),
                    ty: Ty::Int,
                },
                value: MirExpr::FloatLiteral(1.0),
            }],
            Ty::None,
        ),
        |ir| {
            assert_eq!(
                ir.matches("call i8 @pycc_rt_exception_active()").count(),
                1,
                "{ir}"
            );
            assert_eq!(ir.matches("@pycc_rt_buffer_f64_set").count(), 2, "{ir}");
        },
    );
}

/// CPython evaluates `b[i] = v`'s **right-hand side first**, then the
/// subscription target, then the index -- a `d[i()] = v()` probe on the
/// reference interpreter prints `v`'s effect before `i`'s. The emitted IR
/// must observe the same order, or a raising or side-effecting index
/// expression would suppress a value expression CPython would already have
/// run.
///
/// The witness is a store whose value is itself a buffer load: with the
/// value emitted first, the index's own `pycc_rt_int_untag_checked` falls
/// *between* the load and the store. Emitting the index first would put
/// both untags ahead of the load and leave that span empty.
#[test]
fn a_buffer_element_store_emits_its_value_before_its_index() {
    compile_ext_items_checking_ir(
        "buffer_store_value_before_index",
        buffer_fn_items(
            vec![MirStmt::BufferSet {
                base: MirExpr::Name {
                    name: "b".to_string(),
                    ty: Ty::MemoryView,
                },
                index: MirExpr::Name {
                    name: "i".to_string(),
                    ty: Ty::Int,
                },
                value: buffer_get_b_i(),
            }],
            Ty::None,
        ),
        |ir| {
            // Anchored on the `call` sites, not on the bare symbol: the
            // module prints its `declare` lines ahead of every definition.
            let load_at = ir
                .find("call double @pycc_rt_buffer_f64_get")
                .unwrap_or_else(|| panic!("the value's own load is missing: {ir}"));
            let store_at = ir
                .find("call void @pycc_rt_buffer_f64_set")
                .unwrap_or_else(|| panic!("the store is missing: {ir}"));
            assert!(load_at < store_at, "{ir}");
            assert!(
                ir[load_at..store_at].contains("call i64 @pycc_rt_int_untag_checked"),
                "the index was decoded before the value was evaluated: {ir}"
            );
        },
    );
}

#[test]
#[should_panic(expected = "a buffer element load's base did not evaluate to a memoryview")]
fn a_buffer_element_load_whose_base_is_not_a_memoryview_is_an_internal_error() {
    // `MirExpr::BufferGet` is produced by exactly one lowering arm, which
    // keys on the lowered base's `Ty::MemoryView` (`pycc_mir`'s
    // `Subscript` arm), so a base of any other type means that dispatch
    // regressed. The deliberately mistyped `int` base below is a shape no
    // type-checked program can produce.
    compile_ext_items_checking_ir(
        "buffer_element_load_bad_base",
        buffer_fn_items(
            vec![MirStmt::Return(Some(MirExpr::BufferGet {
                base: Box::new(MirExpr::Name {
                    name: "i".to_string(),
                    ty: Ty::Int,
                }),
                index: Box::new(MirExpr::IntLiteral(0)),
            }))],
            Ty::Float,
        ),
        |_| unreachable!("codegen should have panicked"),
    );
}

#[test]
fn returning_owned_buffer_storage_transfers_it_in_the_epilogue_not_at_the_return() {
    // Part 2b of #1142 (#1164). Ownership of a returned buffer transfers at
    // the frame's owned-slot epilogue, which is the one point control
    // actually leaves the frame -- **not** at the `return` statement, which
    // only makes the value pending. Between the two, `finally_stack`
    // interposes every user finalizer.
    //
    // Both directions are pinned, because only the pair excludes the
    // regression: the returning block must *not* clear the slot (clearing it
    // there made a finalizer that reads the name dereference null and abort
    // the hosting interpreter), and the epilogue must decide by identity
    // against the value actually being returned (without that the epilogue
    // frees storage the host is still reading).
    compile_ext_items_checking_ir(
        "buffer_returned",
        vec![MirItem::Function {
            name: "allocate".to_string(),
            params: vec![],
            return_ty: Ty::MemoryView,
            body: vec![
                buffer_alloc_a(),
                MirStmt::Return(Some(MirExpr::Name {
                    name: "a".to_string(),
                    ty: Ty::MemoryView,
                })),
            ],
        }],
        |ir| {
            // The LLVM signature returns the carrier pointer, not `void`.
            assert!(ir.contains("define ptr @pyfn_allocate()"), "{ir}");
            let block = ir
                .split("global_ready:")
                .nth(1)
                .unwrap_or_else(|| panic!("no returning block: {ir}"));
            let load_at = block
                .find("%load = load ptr, ptr %a")
                .unwrap_or_else(|| panic!("no load out of the slot: {ir}"));
            // The regression direction. A `store ptr null, ptr %a` here is
            // exactly the defect: the slot stops naming live storage while
            // user finalizers are still running.
            assert!(
                !block.contains("store ptr null, ptr %a"),
                "the returning block cleared the slot: {ir}"
            );
            // What the statement does instead: record the pointer as
            // pending, so a finalizer that rebinds the name cannot release
            // it out from under the host.
            let pending_at = block
                .find("store ptr %load, ptr %pending_return_buffer_slot")
                .unwrap_or_else(|| panic!("the pending return was not recorded: {ir}"));
            let handoff_at = block
                .find("store ptr %load, ptr %str_epilogue_ret_slot")
                .unwrap_or_else(|| panic!("the loaded view was not returned: {ir}"));
            assert!(load_at < pending_at, "{ir}");
            assert!(pending_at < handoff_at, "{ir}");
            // ...and the epilogue releases the slot only when it does not
            // hold the value being returned. The `select` is the transfer:
            // `pycc_rt_buffer_f64_free`'s documented null no-op is what the
            // returning path gets instead of a free.
            let epilogue = ir
                .split("pycc_str_epilogue:")
                .nth(1)
                .unwrap_or_else(|| panic!("no owned-slot epilogue: {ir}"));
            assert!(
                epilogue
                    .contains("%buffer_epilogue_returned = load ptr, ptr %str_epilogue_ret_slot"),
                "{ir}"
            );
            assert!(
                epilogue.contains(
                    "%buffer_epilogue_transferred = icmp eq ptr %buffer_epilogue_live, \
                     %buffer_epilogue_returned"
                ),
                "{ir}"
            );
            assert!(
                epilogue.contains(
                    "%buffer_epilogue_release = select i1 %buffer_epilogue_transferred, ptr null, \
                     ptr %buffer_epilogue_live"
                ),
                "{ir}"
            );
            assert!(
                epilogue
                    .contains("call void @pycc_rt_buffer_f64_free(ptr %buffer_epilogue_release)"),
                "{ir}"
            );
        },
    );
}

#[test]
fn an_abandoned_pending_return_is_released_only_once_nothing_else_names_it() {
    // Part 2b of #1142 (#1164), the arm the per-slot loop cannot reach. A
    // `finally` that rebinds the returned name runs
    // `free_buffer_slot_before_store` *after* the `return` statement made
    // that pointer pending, so the store site must keep its hands off it --
    // and the pending record then becomes the pointer's sole owner, which
    // the epilogue has to release when a raising finalizer abandoned the
    // return.
    //
    // The conjunction is what makes both safe. `old == pending` alone is
    // true for the *first* allocation into a never-assigned slot, where both
    // are the entry null: that raised the orphan flag on a frame that never
    // rebound anything, and the epilogue then released a pointer the slot
    // loop had already released -- a double free on every exception exit.
    //
    // Two `ndarray` allocations rather than one because the second is what
    // reaches the store site's release path at all; the shape is
    // `a_reallocating_function_frees_the_old_view_before_it_stores_the_new_one`'s,
    // lifted to a `memoryview`-returning frame.
    compile_ext_items_checking_ir(
        "buffer_returned_rebound",
        vec![MirItem::Function {
            name: "allocate".to_string(),
            params: vec![],
            return_ty: Ty::MemoryView,
            body: vec![
                buffer_alloc_a(),
                buffer_alloc_a(),
                MirStmt::Return(Some(MirExpr::Name {
                    name: "a".to_string(),
                    ty: Ty::MemoryView,
                })),
            ],
        }],
        |ir| {
            // The store site's guard, both halves.
            assert!(
                ir.contains("%buffer_store_old_is_live = icmp ne ptr %old_buffer, null"),
                "the null guard is missing: {ir}"
            );
            assert!(
                ir.contains(
                    "%buffer_store_matches_pending = icmp eq ptr %old_buffer, \
                     %pending_return_buffer"
                ),
                "{ir}"
            );
            assert!(
                ir.contains(
                    "%buffer_store_is_pending = and i1 %buffer_store_old_is_live, \
                     %buffer_store_matches_pending"
                ),
                "the guard is not a conjunction: {ir}"
            );
            assert!(
                ir.contains(
                    "%buffer_free_old_release = select i1 %buffer_store_is_pending, ptr null, \
                     ptr %old_buffer"
                ),
                "{ir}"
            );
            // ...and the orphan flag it raises, which is the only thing that
            // lets the epilogue distinguish "still in a slot" from "rebound
            // away".
            assert!(
                ir.contains(
                    "%pending_return_orphaned = select i1 %buffer_store_is_pending, i8 1, \
                     i8 %pending_return_was_orphaned"
                ),
                "{ir}"
            );
            // The epilogue's trailing release, gated on that flag *and* on
            // the pending pointer not being the returned one.
            let epilogue = ir
                .split("pycc_str_epilogue:")
                .nth(1)
                .unwrap_or_else(|| panic!("no owned-slot epilogue: {ir}"));
            assert!(
                epilogue.contains(
                    "%buffer_epilogue_was_orphaned = icmp ne i8 %buffer_epilogue_orphaned, 0"
                ),
                "{ir}"
            );
            assert!(
                epilogue.contains(
                    "%buffer_epilogue_pending_keep = select i1 \
                     %buffer_epilogue_pending_transferred, ptr null, ptr %buffer_epilogue_pending"
                ),
                "{ir}"
            );
            assert!(
                epilogue.contains(
                    "%buffer_epilogue_pending_release = select i1 %buffer_epilogue_was_orphaned, \
                     ptr %buffer_epilogue_pending_keep, ptr null"
                ),
                "the trailing release is not gated on the orphan flag: {ir}"
            );
            assert!(
                epilogue.contains(
                    "call void @pycc_rt_buffer_f64_free(ptr %buffer_epilogue_pending_release)"
                ),
                "{ir}"
            );
        },
    );
}

#[test]
fn a_superseding_return_releases_the_orphaned_record_it_replaces_and_clears_the_flag() {
    // Part 2b of #1142 (#1164), round 2: the pending record's own state
    // machine rather than the control-flow shapes that reach it.
    //
    // `(pending, orphaned)` is one two-field state whose invariant is
    // "`orphaned != 0` means the record is the pointer's sole owner". It has
    // exactly two mutators. `free_buffer_slot_before_store` is the one the
    // test above pins; this is the other. A `return` that supersedes an
    // earlier one -- which `finally: while True: return a` reaches, because
    // the HIR resets its `in_finally` state on loop entry and the `L0001`
    // refusal of a bare `return` in `finally` therefore does not cover it --
    // used to store the new pointer raw, which broke the invariant twice
    // over: the superseded pointer's sole record was overwritten and its
    // storage leaked, and the surviving flag then described a pointer that
    // was no longer pending, so the epilogue's trailing release freed the
    // *new* pointer that the owned-slot loop had already released.
    //
    // The transition is pinned rather than the shape: with one
    // invariant-preserving mutator, a third and a fourth `return` are the
    // same transition applied again, so no enumeration of syntactic paths
    // carries the correctness argument. A single-`return` frame emits the
    // same instructions (`stale` is statically false there at runtime, not
    // in the IR), which is why this fixture needs no `finally` to pin them.
    compile_ext_items_checking_ir(
        "buffer_return_supersedes",
        vec![MirItem::Function {
            name: "allocate".to_string(),
            params: vec![],
            return_ty: Ty::MemoryView,
            body: vec![
                buffer_alloc_a(),
                MirStmt::Return(Some(MirExpr::Name {
                    name: "a".to_string(),
                    ty: Ty::MemoryView,
                })),
            ],
        }],
        |ir| {
            let block = ir
                .split("global_ready:")
                .nth(1)
                .unwrap_or_else(|| panic!("no returning block: {ir}"));
            // The predecessor is read back before it can be overwritten.
            let read_at = block
                .find("%superseded_return_buffer = load ptr, ptr %pending_return_buffer_slot")
                .unwrap_or_else(|| panic!("the superseded record is not read: {ir}"));
            assert!(
                block.contains(
                    "%superseded_return_orphaned = load i8, ptr %pending_return_orphaned_slot"
                ),
                "{ir}"
            );
            // Both halves of the release guard. Gating on the flag is what
            // keeps an *unorphaned* predecessor -- still held by its own
            // slot, which the epilogue's loop releases -- from being freed
            // twice; `try: return a` with `finally: while True: return b` is
            // that shape. The inequality makes the symmetric state
            // unreachable rather than merely improbable.
            assert!(
                block.contains(
                    "%superseded_return_was_orphaned = icmp ne i8 %superseded_return_orphaned, 0"
                ),
                "the release is not gated on the orphan flag: {ir}"
            );
            assert!(
                block.contains(
                    "%superseded_return_differs = icmp ne ptr %superseded_return_buffer, %load"
                ),
                "the release does not exclude the value being returned: {ir}"
            );
            assert!(
                block.contains(
                    "%superseded_return_is_stale = and i1 %superseded_return_was_orphaned, \
                     %superseded_return_differs"
                ),
                "the guard is not a conjunction: {ir}"
            );
            assert!(
                block.contains(
                    "%superseded_return_release = select i1 %superseded_return_is_stale, \
                     ptr %superseded_return_buffer, ptr null"
                ),
                "{ir}"
            );
            let free_at = block
                .find("call void @pycc_rt_buffer_f64_free(ptr %superseded_return_release)")
                .unwrap_or_else(|| panic!("the superseded record is not released: {ir}"));
            // ...and the transition completes: the new pointer is recorded
            // and the flag it inherited is cleared. Leaving the flag raised
            // is the double-free half of the defect, so the clear is pinned
            // separately from the release and ordered after it.
            let store_at = block
                .find("store ptr %load, ptr %pending_return_buffer_slot")
                .unwrap_or_else(|| panic!("the pending return was not recorded: {ir}"));
            let clear_at = block
                .find("store i8 0, ptr %pending_return_orphaned_slot")
                .unwrap_or_else(|| panic!("the inherited orphan flag was not cleared: {ir}"));
            assert!(read_at < free_at, "{ir}");
            assert!(free_at < store_at, "{ir}");
            assert!(store_at < clear_at, "{ir}");
        },
    );
}

#[test]
fn returning_a_buffer_parameter_leaves_its_slot_alone() {
    // The other provenance. A `memoryview` **parameter**'s storage belongs
    // to the host's exporter: the slot is not in the frame's release list,
    // so the frame must neither clear it nor free it -- an edit that treated
    // every buffer slot as artifact-owned would either lose the host's
    // pointer or hand its exporter a pointer this frame had freed.
    //
    // `pycc_types` refuses this shape at the source level
    // (`reject_memoryview_read`'s parameter arm), so the MIR below is hand
    // built: the assertion guards the codegen arm, not a reachable program.
    //
    // Part 2b of #1142 (#1164) narrows what this can assert. The pending
    // record and the epilogue both exist only for a frame that owns at least
    // one buffer *local*; this frame owns none, so the assertions are that
    // no owned-slot machinery appears at all around a parameter.
    compile_ext_items_checking_ir(
        "buffer_parameter_returned",
        buffer_fn_items(
            vec![MirStmt::Return(Some(MirExpr::Name {
                name: "b".to_string(),
                ty: Ty::MemoryView,
            }))],
            Ty::MemoryView,
        ),
        |ir| {
            assert!(ir.contains("define ptr @pyfn_element("), "{ir}");
            // The slot is null-initialized at entry like every other one, so
            // the assertion is that *no second* null store appears -- and
            // that the one that does precedes the load feeding the return.
            assert_eq!(
                ir.matches("store ptr null, ptr %b").count(),
                1,
                "the host's buffer pointer was cleared: {ir}"
            );
            let null_at = ir
                .find("store ptr null, ptr %b")
                .unwrap_or_else(|| panic!("no entry initialization: {ir}"));
            let load_at = ir
                .find("%load = load ptr, ptr %b")
                .unwrap_or_else(|| panic!("no load out of the slot: {ir}"));
            assert!(null_at < load_at, "{ir}");
            // Nothing frees the host's buffer, and the frame allocates no
            // pending-return record for a parameter it does not own.
            assert!(
                !ir.contains("call void @pycc_rt_buffer_f64_free"),
                "the host's buffer was released by the callee: {ir}"
            );
            assert!(
                !ir.contains("pending_return_buffer_slot"),
                "a frame that owns no buffer local got a pending-return record: {ir}"
            );
        },
    );
}

#[test]
#[should_panic(expected = "a memoryview argument is not supported yet")]
fn passing_a_memoryview_as_a_call_argument_is_an_internal_error() {
    // Reached through real MIR rather than a direct call, because this arm
    // lives inside `build_call_to_with_leading_args`' per-argument loop and
    // only a `MirExpr` that *evaluates* to `Scalar::MemoryView` selects it.
    // A `memoryview` parameter exists only on an `--ext` export, and
    // passing the name on is a bare read refused with `C0001`, so the
    // deliberately mistyped `int` parameter below is unreachable from
    // source.
    let mut items = buffer_fn_items(
        vec![MirStmt::ExprStmt(MirExpr::Call {
            callee: "takes_int".to_string(),
            args: vec![MirExpr::Name {
                name: "b".to_string(),
                ty: Ty::MemoryView,
            }],
            ty: Ty::None,
        })],
        Ty::None,
    );
    items.push(MirItem::Function {
        name: "takes_int".to_string(),
        params: vec![("n".to_string(), Ty::Int)],
        return_ty: Ty::None,
        body: vec![MirStmt::Return(None)],
    });
    compile_ext_items_checking_ir("buffer_call_argument", items, |_| {
        unreachable!("codegen should have panicked")
    });
}

// ---------------------------------------------------------------------------
// #1054: every entry-block `str` slot is released on every exit path.
//
// These live in-crate deliberately. The hosted end-to-end probe
// (`tests/issue_1054_ext_str_release.rs`) is `#[ignore]`d and `cargo
// llvm-cov` runs without `--include-ignored`, so it contributes no line
// coverage at all; D-014/D-242's 100%-changed-line gate over the new
// `pycc_str_epilogue` emission is satisfied here and nowhere else. One test
// per exit family, matching the plan's own case list (a)-(j).
// ---------------------------------------------------------------------------

/// Compiles `items` as an ordinary (non-`ext`) object and hands the whole
/// module's IR to `check`. The non-`ext` sibling of
/// `compile_ext_items_checking_ir` above, and it observes the same D-029
/// rule: the `LLVMString` temporary must never be dropped.
fn compile_items_checking_ir(label: &str, items: Vec<MirItem>, check: impl Fn(&str)) {
    let dir = pycc_scratch::ScratchDir::new(label).expect("failed to create scratch dir");
    let mut seen = String::new();
    let mut observer = |module: &inkwell::module::Module<'_>, _| {
        seen = llvm_string_to_owned(module.print_to_string());
    };
    compile_to_object_with_observer(
        &MirModule {
            items,
            ..Default::default()
        },
        &dir.join(format!("{label}.o")),
        &CompileOptions::default(),
        Some(&mut observer),
    )
    .expect("the #1054 str-epilogue fixtures must all compile");
    check(&seen);
}

/// How many `str` slots the emitted epilogues release, module-wide. Counted
/// through the loaded operand's own name, because LLVM prints no result name
/// for a `void` call -- the `"str_epilogue_decref"` this crate passes
/// `build_call` never appears in the textual IR.
fn str_epilogue_releases(ir: &str) -> usize {
    ir.matches("@pycc_rt_str_decref(ptr %str_epilogue_live")
        .count()
}

/// How many functions got a `str` epilogue at all. Counts the block label's
/// own definition site, not the branches into it.
fn str_epilogue_blocks(ir: &str) -> usize {
    ir.matches("pycc_str_epilogue:").count()
}

fn str_name(name: &str) -> MirExpr {
    MirExpr::Name {
        name: name.to_string(),
        ty: Ty::Str,
    }
}

/// (a) The plain case: a `str` parameter, an ordinary `return`. The return
/// routes through the synthetic frame (no edit to `MirStmt::Return` was
/// needed), the epilogue releases the parameter slot, and the function's
/// single `ret` reads the epilogue's own `ret_slot`.
#[test]
fn issue_1054_a_str_parameter_is_released_on_a_normal_return() {
    compile_items_checking_ir(
        "issue_1054_a_normal_return",
        vec![MirItem::Function {
            name: "echo".to_string(),
            params: vec![("s".to_string(), Ty::Str)],
            return_ty: Ty::Str,
            body: vec![MirStmt::Return(Some(str_name("s")))],
        }],
        |ir| {
            assert_eq!(str_epilogue_blocks(ir), 1, "{ir}");
            assert_eq!(str_epilogue_releases(ir), 1, "{ir}");
            assert!(ir.contains("str_epilogue_ret_slot"), "{ir}");
            // The returned duplicate is increfd by the `return` itself, so
            // the epilogue's release balances the caller's incoming
            // reference rather than the value being handed back.
            assert!(ir.contains("pycc_rt_str_incref"), "{ir}");
        },
    );
}

/// (b) A `None`-returning function that falls off the end of its body: the
/// implicit `return None` never passes through `finally_stack`, so it is one
/// of the two sites that needed an explicit branch into the epilogue.
#[test]
fn issue_1054_b_a_none_returning_fall_through_reaches_the_epilogue() {
    compile_items_checking_ir(
        "issue_1054_b_fall_through",
        vec![MirItem::Function {
            name: "consume".to_string(),
            params: vec![("s".to_string(), Ty::Str)],
            return_ty: Ty::None,
            body: vec![MirStmt::NoOp],
        }],
        |ir| {
            assert_eq!(str_epilogue_blocks(ir), 1, "{ir}");
            assert_eq!(str_epilogue_releases(ir), 1, "{ir}");
            // A `None` ABI gets no `ret_slot`: the epilogue ends in
            // `ret void`.
            assert!(!ir.contains("str_epilogue_ret_slot"), "{ir}");
        },
    );
}

/// (c) A `return` from inside a `try`/`finally`. `emit_try`'s own `ret_bb`
/// finds the synthetic frame as its enclosing `finally_stack` entry and
/// propagates outward to it, so this path reaches the epilogue with no edit
/// to `exception.rs` at all.
#[test]
fn issue_1054_c_a_return_through_a_finally_reaches_the_epilogue() {
    compile_items_checking_ir(
        "issue_1054_c_try_finally",
        vec![MirItem::Function {
            name: "guarded".to_string(),
            params: vec![("s".to_string(), Ty::Str)],
            return_ty: Ty::Str,
            body: vec![MirStmt::Try {
                body: vec![MirStmt::Return(Some(str_name("s")))],
                handlers: vec![],
                orelse: vec![],
                finalbody: vec![MirStmt::NoOp],
            }],
        }],
        |ir| {
            assert_eq!(str_epilogue_blocks(ir), 1, "{ir}");
            assert_eq!(str_epilogue_releases(ir), 1, "{ir}");
            assert!(ir.contains("try_ret_slot"), "{ir}");
        },
    );
}

/// (d) The same for `try*` (PEP 654), whose `ret_bb` is a separate emitter
/// with its own copy of the propagate-to-the-enclosing-frame logic.
#[test]
fn issue_1054_d_a_return_through_a_try_star_finally_reaches_the_epilogue() {
    compile_items_checking_ir(
        "issue_1054_d_try_star_finally",
        vec![MirItem::Function {
            name: "guarded_star".to_string(),
            params: vec![("s".to_string(), Ty::Str)],
            return_ty: Ty::Str,
            body: vec![MirStmt::TryStar {
                body: vec![MirStmt::Return(Some(str_name("s")))],
                // `emit_try_star` requires at least one clause -- an
                // `except*` with no handler is not a shape the lowerer can
                // produce, so the fixture carries a terminating one.
                handlers: vec![MirExceptHandler {
                    exc_type_tag: Some(vec![1]),
                    binding_name: None,
                    binding_ty: None,
                    body: vec![MirStmt::Return(Some(MirExpr::StringLiteral(
                        "handled".to_string(),
                    )))],
                }],
                orelse: vec![],
                finalbody: vec![MirStmt::NoOp],
            }],
        }],
        |ir| {
            assert_eq!(str_epilogue_blocks(ir), 1, "{ir}");
            assert_eq!(str_epilogue_releases(ir), 1, "{ir}");
            assert!(ir.contains("trystar_ret_slot"), "{ir}");
        },
    );
}

/// (e) The exceptional exit. An in-flight exception does not make the
/// references this function owns somebody else's problem: `exception_exit`
/// stores its neutral carrier into the epilogue's `ret_slot` and branches in
/// rather than returning directly.
#[test]
fn issue_1054_e_the_exception_exit_reaches_the_epilogue() {
    compile_items_checking_ir(
        "issue_1054_e_exception_exit",
        vec![MirItem::Function {
            name: "always_raises".to_string(),
            params: vec![("s".to_string(), Ty::Str)],
            return_ty: Ty::Str,
            body: vec![MirStmt::Raise {
                exception: MirExceptionValue::Constructed {
                    type_tag: 1,
                    class_name: "ValueError".to_string(),
                    message: MirExpr::StringLiteral("boom".to_string()),
                },
                frame_function: "always_raises".to_string(),
            }],
        }],
        |ir| {
            assert_eq!(str_epilogue_blocks(ir), 1, "{ir}");
            assert_eq!(str_epilogue_releases(ir), 1, "{ir}");
            assert!(ir.contains("exception_exit"), "{ir}");
            // The `unreachable` fall-through arm is untouched: a body that
            // always raises still terminates statically.
            assert!(ir.contains("unreachable"), "{ir}");
        },
    );
}

/// (f) A `str` local that a given path never assigned still holds the null
/// `storage_slot_at_entry` initialized it with, and `pycc_rt_str_decref` is
/// a documented no-op on null -- so the epilogue needs no per-slot guard.
#[test]
fn issue_1054_f_an_unassigned_str_local_is_released_unconditionally() {
    compile_items_checking_ir(
        "issue_1054_f_unassigned_local",
        vec![MirItem::Function {
            name: "maybe_binds".to_string(),
            params: vec![("flag".to_string(), Ty::Bool)],
            return_ty: Ty::None,
            body: vec![MirStmt::If {
                test: MirExpr::Name {
                    name: "flag".to_string(),
                    ty: Ty::Bool,
                },
                body: vec![MirStmt::Assign {
                    target: "t".to_string(),
                    value: MirExpr::StringLiteral("bound".to_string()),
                }],
                orelse: vec![],
            }],
        }],
        |ir| {
            assert_eq!(str_epilogue_blocks(ir), 1, "{ir}");
            assert_eq!(str_epilogue_releases(ir), 1, "{ir}");
        },
    );
}

/// (g) The gate itself: a function with no `str` slot pushes no synthetic
/// frame and emits no epilogue, which is what keeps the "no outer
/// `finally_stack` entry" arms of `MirStmt::Return` and of `emit_try`/
/// `emit_try_star`'s `ret_bb` reachable for the tests that cover them.
#[test]
fn issue_1054_g_a_str_free_function_gets_no_epilogue() {
    compile_items_checking_ir(
        "issue_1054_g_no_str_slot",
        vec![MirItem::Function {
            name: "arithmetic".to_string(),
            params: vec![("n".to_string(), Ty::Int)],
            return_ty: Ty::Int,
            body: vec![MirStmt::Return(Some(MirExpr::Name {
                name: "n".to_string(),
                ty: Ty::Int,
            }))],
        }],
        |ir| {
            assert_eq!(str_epilogue_blocks(ir), 0, "{ir}");
            assert_eq!(str_epilogue_releases(ir), 0, "{ir}");
        },
    );
}

/// (h) The type-confusion hazard the entry-slot *snapshot* exists to
/// prevent. `except ValueError as s` rebinds the name `s` mid-function to a
/// slot holding an exception instance, not a `PyStrObj`. A walk of the
/// live locals at exit would decref that instance pointer as a string; the
/// snapshot released here is still the parameter's own entry-block slot, and
/// there is exactly one release, not two.
#[test]
fn issue_1054_h_an_except_binding_shadowing_a_str_parameter_is_not_released() {
    compile_items_checking_ir(
        "issue_1054_h_except_shadow",
        vec![MirItem::Function {
            name: "shadowed".to_string(),
            params: vec![("s".to_string(), Ty::Str)],
            return_ty: Ty::None,
            body: vec![MirStmt::Try {
                body: vec![MirStmt::Raise {
                    exception: MirExceptionValue::Constructed {
                        type_tag: 1,
                        class_name: "ValueError".to_string(),
                        message: MirExpr::StringLiteral("boom".to_string()),
                    },
                    frame_function: "shadowed".to_string(),
                }],
                handlers: vec![MirExceptHandler {
                    exc_type_tag: Some(vec![1]),
                    binding_name: Some("s".to_string()),
                    binding_ty: Some(Ty::Instance(Box::new("ValueError".to_string()))),
                    body: vec![MirStmt::NoOp],
                }],
                orelse: vec![],
                finalbody: vec![],
            }],
        }],
        |ir| {
            assert_eq!(str_epilogue_blocks(ir), 1, "{ir}");
            assert_eq!(
                str_epilogue_releases(ir),
                1,
                "only the parameter's own entry-block slot is in the snapshot: {ir}"
            );
        },
    );
}

/// (i) The compiled-to-compiled call boundary, duplicate-argument shape. The
/// caller reads a bare `Name`, so `incref_if_str_duplicate` increfs before
/// the call and the callee's slot owns a reference of its own -- newly
/// load-bearing, because before this change the callee simply leaked it.
#[test]
fn issue_1054_i_a_bare_name_argument_is_increfd_by_the_caller_and_released_by_the_callee() {
    compile_items_checking_ir(
        "issue_1054_i_duplicate_argument",
        vec![
            MirItem::Function {
                name: "callee".to_string(),
                params: vec![("s".to_string(), Ty::Str)],
                return_ty: Ty::None,
                body: vec![MirStmt::NoOp],
            },
            MirItem::Function {
                name: "caller".to_string(),
                params: vec![("s".to_string(), Ty::Str)],
                return_ty: Ty::None,
                body: vec![MirStmt::ExprStmt(MirExpr::Call {
                    callee: "callee".to_string(),
                    args: vec![str_name("s")],
                    ty: Ty::None,
                })],
            },
        ],
        |ir| {
            assert_eq!(str_epilogue_blocks(ir), 2, "{ir}");
            assert_eq!(str_epilogue_releases(ir), 2, "{ir}");
            assert!(
                ir.contains("pycc_rt_str_incref"),
                "the caller must incref a bare-`Name` argument it keeps: {ir}"
            );
        },
    );
}

/// (j) The same boundary, fresh-temporary shape. `a + b` is not a bare
/// `Name`, so no incref happens: the concatenation's own refcount-1 object
/// transfers into the callee's slot, which is then the only owner and the
/// only releaser.
#[test]
fn issue_1054_j_a_fresh_temporary_argument_is_owned_solely_by_the_callee() {
    compile_items_checking_ir(
        "issue_1054_j_fresh_argument",
        vec![
            MirItem::Function {
                name: "callee".to_string(),
                params: vec![("s".to_string(), Ty::Str)],
                return_ty: Ty::None,
                body: vec![MirStmt::NoOp],
            },
            MirItem::Function {
                name: "caller".to_string(),
                params: vec![("a".to_string(), Ty::Str), ("b".to_string(), Ty::Str)],
                return_ty: Ty::None,
                body: vec![MirStmt::ExprStmt(MirExpr::Call {
                    callee: "callee".to_string(),
                    args: vec![MirExpr::BinOp {
                        op: BinOpKind::Add,
                        left: Box::new(str_name("a")),
                        right: Box::new(str_name("b")),
                        ty: Ty::Str,
                    }],
                    ty: Ty::None,
                })],
            },
        ],
        |ir| {
            // Three slots: the callee's `s`, and the caller's `a` and `b`.
            assert_eq!(str_epilogue_blocks(ir), 2, "{ir}");
            assert_eq!(str_epilogue_releases(ir), 3, "{ir}");
            assert!(
                ir.contains("pycc_rt_str_concat"),
                "the argument must be a freshly built temporary: {ir}"
            );
        },
    );
}

/// (k) The `except*` mirror of (h). `emit_try_star` applies the same
/// mid-function `locals.insert(binding_name, ...)` overwrite that
/// `emit_try` does, so `except* ValueError as s` over a `str` parameter is
/// the same type-confusion hazard through a second, independently written
/// lowering path. The entry-block snapshot is what makes both safe, and this
/// test pins the `TryStar` arm of it rather than inferring it from (h).
#[test]
fn issue_1054_k_an_except_star_binding_shadowing_a_str_parameter_is_not_released() {
    compile_items_checking_ir(
        "issue_1054_k_except_star_shadow",
        vec![MirItem::Function {
            name: "shadowed_star".to_string(),
            params: vec![("s".to_string(), Ty::Str)],
            return_ty: Ty::None,
            body: vec![MirStmt::TryStar {
                body: vec![MirStmt::Raise {
                    exception: MirExceptionValue::Constructed {
                        type_tag: 1,
                        class_name: "ValueError".to_string(),
                        message: MirExpr::StringLiteral("boom".to_string()),
                    },
                    frame_function: "shadowed_star".to_string(),
                }],
                handlers: vec![MirExceptHandler {
                    exc_type_tag: Some(vec![1]),
                    binding_name: Some("s".to_string()),
                    binding_ty: Some(Ty::Instance(Box::new("ValueError".to_string()))),
                    body: vec![MirStmt::NoOp],
                }],
                orelse: vec![],
                finalbody: vec![],
            }],
        }],
        |ir| {
            assert_eq!(str_epilogue_blocks(ir), 1, "{ir}");
            assert_eq!(
                str_epilogue_releases(ir),
                1,
                "only the parameter's own entry-block slot is in the snapshot: {ir}"
            );
        },
    );
}

// ---------------------------------------------------------------------------
// Part 2a of #1142 (#1165): `MirExpr::BufferAlloc` and the owned-slot frame.
//
// The second source of a `Ty::MemoryView` value, and the first that the
// artifact itself owns. What has to hold is a lifecycle rather than a single
// emission: the slot is null-initialized at entry, the allocation stores into
// it, a reassignment frees the old view first, and the epilogue frees
// whatever the slot holds on every return path -- including a path on which
// the allocation never ran, where the slot still holds the entry null and
// `pycc_rt_buffer_f64_free`'s documented null no-op is what makes it safe.
// ---------------------------------------------------------------------------

/// `a = ndarray(4)` as a `MirStmt`, the one producer shape #1165 emits.
fn buffer_alloc_a() -> MirStmt {
    MirStmt::Assign {
        target: "a".to_string(),
        value: MirExpr::BufferAlloc {
            len: Box::new(MirExpr::IntLiteral(4)),
        },
    }
}

/// An `--ext` module holding one parameterless function with `body`.
fn owned_buffer_fn_items(body: Vec<MirStmt>) -> Vec<MirItem> {
    vec![MirItem::Function {
        name: "allocate".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body,
    }]
}

#[test]
fn a_buffer_allocation_untags_its_length_and_calls_the_runtime_allocator() {
    // The length reaches the runtime as a raw `i64`: the allocator takes an
    // element count, not a tagged `int`, and handing it the tagged form
    // would allocate twice the requested length.
    //
    // #1166 round 8: the decoder is `pycc_rt_buffer_alloc_untag_len`, *not*
    // the shared `pycc_rt_int_untag_checked` every other D-141 operand uses.
    // That one `panic!`s on a bigint, and the panic becomes a process abort
    // at its own `extern "C"` boundary -- the host CPython interpreter's,
    // for a D-244 `ext` artifact. Asserted two-directionally: the
    // non-aborting decoder is present *and* the aborting one is absent, so a
    // half-applied edit that emitted both cannot pass.
    compile_ext_items_checking_ir(
        "buffer_allocation",
        owned_buffer_fn_items(vec![buffer_alloc_a()]),
        |ir| {
            assert!(
                ir.contains("call i64 @pycc_rt_buffer_alloc_untag_len"),
                "{ir}"
            );
            // A `declare` for every runtime function is emitted whether or
            // not it is called, so the negative is on the *call*.
            assert!(!ir.contains("call i64 @pycc_rt_int_untag_checked"), "{ir}");
            assert!(ir.contains("call ptr @pycc_rt_buffer_f64_alloc"), "{ir}");
        },
    );
}

#[test]
fn a_buffer_allocations_length_decoder_can_branch_away_before_it_allocates() {
    // #1166 round 8, the P1 this arm exists to close. The decoder's own
    // bigint refusal raises `OverflowError` (D-173) and returns the
    // type-valid sentinel `0`, and `0` is a *valid* length -- so if the
    // allocator call were reachable from the decode without an intervening
    // branch, the refusal would allocate a zero-length view that
    // `MirStmt::Assign` never stores, leaking it.
    //
    // `guard_statement_effects` repositions the builder into a fresh
    // continuation block, so the property is a *block* separation, not mere
    // text order: pinned here as a conditional branch plus a new label
    // between the decode and the allocator call. A presence-only assertion
    // on the guard passes with or without that separation.
    compile_ext_items_checking_ir(
        "buffer_alloc_decoder_branch",
        owned_buffer_fn_items(vec![buffer_alloc_a()]),
        |ir| {
            let decode = ir
                .find("call i64 @pycc_rt_buffer_alloc_untag_len")
                .unwrap_or_else(|| panic!("the length decode should be emitted: {ir}"));
            let alloc = ir[decode..]
                .find("@pycc_rt_buffer_f64_alloc")
                .map(|offset| decode + offset)
                .unwrap_or_else(|| panic!("the allocator call should follow the decode: {ir}"));
            let between = &ir[decode..alloc];
            assert!(between.contains("@pycc_rt_exception_active"), "{ir}");
            assert!(between.contains("br i1 "), "{ir}");
            assert!(between.contains("effect_exc_cont"), "{ir}");
        },
    );
}

#[test]
fn an_allocating_function_frees_its_buffer_slot_in_the_epilogue() {
    // The whole reason the producer is admitted in exactly one position: the
    // value cannot escape the frame, so the frame can free it
    // unconditionally on the way out. Without the epilogue this is one
    // leaked allocation per call in a long-lived host process, which is the
    // D-107 list/dict/set precedent this issue explicitly does not follow.
    compile_ext_items_checking_ir(
        "buffer_epilogue_free",
        owned_buffer_fn_items(vec![buffer_alloc_a(), MirStmt::Return(None)]),
        |ir| {
            assert!(ir.contains("call void @pycc_rt_buffer_f64_free"), "{ir}");
        },
    );
}

#[test]
fn a_reallocating_function_frees_the_old_view_before_it_stores_the_new_one() {
    // D-074's free-before-overwrite, mirrored from `decref_str_slot_before_store`:
    // a second `a = ndarray(4)` overwrites the slot, so the view the slot
    // already holds has to be released first -- the epilogue only ever sees
    // the last one. Two frees is the signature of the fix: one before the
    // second store, one in the epilogue.
    compile_ext_items_checking_ir(
        "buffer_realloc_free",
        owned_buffer_fn_items(vec![buffer_alloc_a(), buffer_alloc_a()]),
        |ir| {
            assert!(
                ir.matches("call void @pycc_rt_buffer_f64_free").count() >= 2,
                "{ir}"
            );
        },
    );
}

#[test]
fn a_refused_reallocation_never_reaches_the_free_before_its_own_store() {
    // #1166 round 9 review, the reassign-then-refused-allocation ordering.
    // `a = ndarray(4); a = ndarray(-1)` is the one shape neither neighbour
    // covers: `a_reallocating_function_frees_the_old_view_before_it_stores_the_new_one`
    // reallocates *successfully*, and the hosted `alloc_then_raise` refuses
    // on a *different* name, where the slot being overwritten is null anyway.
    // Here the slot holds a live view when the second allocation is refused,
    // so freeing it before the refusal branched away would release a view the
    // exception-exit epilogue then frees again -- a double free.
    //
    // What makes that unreachable is block separation, not text order:
    // `expression_can_set_exception` answers `true` for `MirExpr::BufferAlloc`,
    // so `emit_expr`'s post-call `guard_statement_effects` repositions the
    // builder into a fresh continuation block before `MirStmt::Assign` reaches
    // `free_buffer_slot_before_store`. Pinned the way the round-8 decoder test
    // pins its own separation: the window between the second allocator call
    // and the next free must carry the pending-state read, a conditional
    // branch, and the continuation label. A bare "no free here" assertion
    // would be unkillable -- the mutation that matters moves the free
    // *upstream*, out of any such window -- and "a free exists afterwards"
    // is confounded by the epilogue's own free.
    compile_ext_items_checking_ir(
        "buffer_refused_realloc_order",
        owned_buffer_fn_items(vec![buffer_alloc_a(), buffer_alloc_a()]),
        |ir| {
            let first = ir
                .find("@pycc_rt_buffer_f64_alloc")
                .unwrap_or_else(|| panic!("the first allocator call should be emitted: {ir}"));
            let second = ir[first + 1..]
                .find("@pycc_rt_buffer_f64_alloc")
                .map(|offset| first + 1 + offset)
                .unwrap_or_else(|| panic!("the second allocator call should be emitted: {ir}"));
            let free = ir[second..]
                .find("call void @pycc_rt_buffer_f64_free")
                .map(|offset| second + offset)
                .unwrap_or_else(|| panic!("a free should follow the second allocation: {ir}"));
            let between = &ir[second..free];
            assert!(between.contains("@pycc_rt_exception_active"), "{ir}");
            assert!(between.contains("br i1 "), "{ir}");
            // Substring, not an exact label: LLVM uniquifies the name once
            // two statements each emit a continuation block.
            assert!(between.contains("effect_exc_cont"), "{ir}");
            // The window above pins the separation but not *which* free it
            // landed on: hoisting `free_buffer_slot_before_store` above
            // `emit_expr` in the `MirStmt::Assign` arm would move the second
            // statement's free upstream of the whole window, leaving the
            // epilogue's free to satisfy it. Counting what precedes the
            // second allocation closes that: exactly one, statement 1's
            // free-before-store over a slot the entry block null-initialized.
            assert_eq!(
                ir[..second]
                    .matches("call void @pycc_rt_buffer_f64_free")
                    .count(),
                1,
                "{ir}"
            );
        },
    );
}

/// An `--ext` module holding one `n: int` function with `body`, for the
/// arms that need a *parameter* to build a borrowed length out of.
fn owned_buffer_fn_items_taking_n(body: Vec<MirStmt>) -> Vec<MirItem> {
    vec![MirItem::Function {
        name: "allocate".to_string(),
        params: vec![("n".to_string(), Ty::Int)],
        return_ty: Ty::None,
        body,
    }]
}

/// `a = ndarray(<len>)` over an arbitrary length expression.
fn buffer_alloc_a_of(len: MirExpr) -> MirStmt {
    MirStmt::Assign {
        target: "a".to_string(),
        value: MirExpr::BufferAlloc { len: Box::new(len) },
    }
}

#[test]
fn a_buffer_allocations_length_temporary_is_released_before_the_guard() {
    // #1166 round 11, finding 1. `a = ndarray(n + 1)` with `n == 2 ** 62 - 1`
    // evaluates the length to a freshly owned `BigIntObj`, and the decoder
    // below refuses it with a *catchable* `OverflowError`. Every exception
    // edge out of this arm therefore abandons a birth reference unless the
    // arm retires it first, and repeatedly catching that exception leaked one
    // bigint per call in the host process.
    //
    // Asserted as an ordering rather than as presence, for the reason the
    // round-8 decoder arm above states: a release emitted *after*
    // `guard_statement_effects` is skipped by exactly the edges that leak,
    // and a presence-only assertion passes either way. The window is
    // therefore the decode through the pre-allocator guard's own pending
    // read -- which is also what pins the release ahead of the stale-pending
    // short circuit, not merely ahead of the allocator.
    compile_ext_items_checking_ir(
        "buffer_alloc_len_release",
        owned_buffer_fn_items_taking_n(vec![buffer_alloc_a_of(MirExpr::BinOp {
            op: BinOpKind::Add,
            left: Box::new(MirExpr::Name {
                name: "n".to_string(),
                ty: Ty::Int,
            }),
            right: Box::new(MirExpr::IntLiteral(1)),
            ty: Ty::Int,
        })]),
        |ir| {
            let decode = ir
                .find("call i64 @pycc_rt_buffer_alloc_untag_len")
                .unwrap_or_else(|| panic!("the length decode should be emitted: {ir}"));
            let guard = ir[decode..]
                .find("@pycc_rt_exception_active")
                .map(|offset| decode + offset)
                .unwrap_or_else(|| {
                    panic!("the pre-allocator guard should follow the decode: {ir}")
                });
            assert!(
                ir[decode..guard].contains("@pycc_rt_bigint_release"),
                "{ir}"
            );
        },
    );
}

#[test]
fn a_borrowed_buffer_length_is_not_released_by_the_allocation() {
    // The other direction of the arm above, and the one that makes the
    // release a classification rather than an unconditional emission: in
    // `a = ndarray(n)` the length is a plain `Name` read, which owns no
    // reference of its own -- `n`'s own storage still refers to the word.
    // Releasing it here would be a double release, not a leak, so this is
    // pinned in the same window the positive arm asserts over.
    compile_ext_items_checking_ir(
        "buffer_alloc_borrowed_len",
        owned_buffer_fn_items_taking_n(vec![buffer_alloc_a_of(MirExpr::Name {
            name: "n".to_string(),
            ty: Ty::Int,
        })]),
        |ir| {
            let decode = ir
                .find("call i64 @pycc_rt_buffer_alloc_untag_len")
                .unwrap_or_else(|| panic!("the length decode should be emitted: {ir}"));
            let alloc = ir[decode..]
                .find("@pycc_rt_buffer_f64_alloc")
                .map(|offset| decode + offset)
                .unwrap_or_else(|| panic!("the allocator call should follow the decode: {ir}"));
            assert!(
                !ir[decode..alloc].contains("@pycc_rt_bigint_release"),
                "{ir}"
            );
        },
    );
}

#[test]
fn a_buffer_allocation_checks_the_pending_state_before_it_allocates() {
    // #1166 review finding F5. `emit_expr` guards *after*
    // `emit_expr_unchecked` returns, so an exception already pending when
    // this node is reached -- set by an operation
    // `expression_can_set_exception` classifies `false` and therefore leaves
    // unguarded, such as the `ListPop` in `a = ndarray(xs.pop())` -- let the
    // allocation succeed and only then branched to the handler, before
    // `MirStmt::Assign` stored the pointer into the frame's owned slot. The
    // slot kept its entry null, so nothing freed the view: one leaked buffer
    // per call.
    //
    // Asserted as an ordering *between* the length untag and the allocator
    // call rather than as the mere presence of a guard: the post-call guard
    // `expression_can_set_exception` already emitted makes a presence-only
    // assertion pass with or without the fix.
    compile_ext_items_checking_ir(
        "buffer_alloc_pending_pre_guard",
        owned_buffer_fn_items(vec![buffer_alloc_a()]),
        |ir| {
            let untag = ir
                .find("buffer_untag_alloc_len")
                .unwrap_or_else(|| panic!("the length untag should be emitted: {ir}"));
            let alloc = ir[untag..]
                .find("@pycc_rt_buffer_f64_alloc")
                .map(|offset| untag + offset)
                .unwrap_or_else(|| panic!("the allocator call should follow the untag: {ir}"));
            assert!(
                ir[untag..alloc].contains("@pycc_rt_exception_active"),
                "{ir}"
            );
        },
    );
}

#[test]
fn a_buffer_parameter_is_never_pushed_into_the_owned_slot_frame() {
    // The load-bearing negative. A `memoryview` *parameter* is storage the
    // host lent for exactly one call, so freeing it in the epilogue would
    // hand the host's allocator a pointer it still owns. The `str` precedent
    // this frame is modelled on *does* push parameter slots, so the omission
    // is deliberate rather than incidental and is pinned here.
    compile_ext_items_checking_ir(
        "buffer_param_not_owned",
        buffer_fn_items(vec![MirStmt::Return(Some(buffer_get_b_i()))], Ty::Float),
        |ir| {
            assert!(!ir.contains("call void @pycc_rt_buffer_f64_free"), "{ir}");
        },
    );
}

#[test]
#[should_panic(expected = "has a non-buffer storage slot")]
fn freeing_a_non_buffer_slot_before_a_store_is_an_internal_error() {
    // `free_buffer_slot_before_store` is reached only from the
    // `Ty::MemoryView` arm of `MirStmt::Assign`, so a slot of any other type
    // is a lowering defect. Pinned directly, on the same convention as the
    // defensive `Scalar::MemoryView` tests above: an honest panic in the
    // function that owns the invariant beats a silent free of an `i64` slot.
    let context = Context::create();
    let (module, rt) = list_scalar_panic_fixture(&context);
    let builder = context.create_builder();
    let function = module.add_function(
        "free_wrong_slot",
        context.void_type().fn_type(&[], false),
        None,
    );
    builder.position_at_end(context.append_basic_block(function, "entry"));
    let ptr = builder
        .build_alloca(context.i64_type(), "a")
        .expect("build_alloca should not fail for a fresh block");
    let locals = HashMap::from([(
        "a".to_string(),
        StorageSlot {
            ptr,
            ty: Ty::Int,
            initialized: None,
        },
    )]);
    free_buffer_slot_before_store(&context, &builder, &rt, &locals, "a");
}
