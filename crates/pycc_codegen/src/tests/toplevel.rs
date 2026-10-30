//! Module-level bindings, `main`, top-level statements, release mode and target triples.

use super::*;

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
    assert_eq!(pycc_scratch::lf(&output.stdout), b"");
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
    assert_eq!(pycc_scratch::lf(&output.stdout), b"42\n");
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
    assert_eq!(pycc_scratch::lf(&output.stdout), b"7\n");
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
    assert_eq!(pycc_scratch::lf(&output.stdout), b"42\n");
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
    assert_eq!(pycc_scratch::lf(&output.stdout), b"1\n");
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
    assert_eq!(pycc_scratch::lf(&output.stdout), b"1\n2\n");
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

/// One section of an ELF64 object: its name, `sh_flags`, `sh_size` and
/// `sh_addralign`.
struct ElfSection {
    name: String,
    flags: u64,
    size: u64,
    align: u64,
}

/// The sections of a little-endian ELF64 object, in header order.
fn elf64_sections(object: &[u8]) -> Vec<ElfSection> {
    let u16_at = |at: usize| u64::from(u16::from_le_bytes([object[at], object[at + 1]]));
    let u32_at = |at: usize| {
        u64::from(u32::from_le_bytes(
            object[at..at + 4].try_into().expect("4 bytes"),
        ))
    };
    let u64_at = |at: usize| u64::from_le_bytes(object[at..at + 8].try_into().expect("8 bytes"));
    assert_eq!(&object[..4], b"\x7fELF", "an ELF object");
    let (shoff, shentsize, shnum, shstrndx) =
        (u64_at(0x28), u16_at(0x3A), u16_at(0x3C), u16_at(0x3E));
    let header = |index: u64| (shoff + index * shentsize) as usize;
    let names = u64_at(header(shstrndx) + 0x18) as usize;
    (0..shnum)
        .map(|index| {
            let at = header(index);
            let name = &object[names + u32_at(at) as usize..];
            let end = name.iter().position(|&b| b == 0).expect("a NUL-ended name");
            ElfSection {
                name: String::from_utf8_lossy(&name[..end]).into_owned(),
                flags: u64_at(at + 0x08),
                size: u64_at(at + 0x20),
                align: u64_at(at + 0x30),
            }
        })
        .collect()
}

#[test]
fn x86_objects_keep_jumps_within_32_byte_boundaries() {
    // `target_machine::set_backend_options`: the x86 assembler pads so that
    // no jump crosses or ends on a 32-byte boundary, which it can only
    // guarantee in a code section aligned to 32 bytes (LLVM's default is 16).
    // Without it, diff-match-patch's `diff_bisect` ran 7-13% slower on a
    // Skylake-derived Xeon when unrelated functions moved its loops.
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(call_print(42))],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("x86_branch_alignment")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("aligned.o");
    compile_to_object(&mir, &obj_path, Some("x86_64-unknown-linux-gnu"), true)
        .expect("an x86_64 Linux object should compile on any host");
    let object = std::fs::read(&obj_path).expect("the object file");
    // The sections that hold code: executable (`SHF_EXECINSTR`) and not
    // empty. Which ones they are depends on how the functions are placed
    // (`crate::function_sections`); each must be aligned to 32 bytes.
    const SHF_EXECINSTR: u64 = 0x4;
    let sections = elf64_sections(&object);
    let code: Vec<&ElfSection> = sections
        .iter()
        .filter(|section| section.flags & SHF_EXECINSTR != 0 && section.size > 0)
        .collect();
    let listing: Vec<String> = sections
        .iter()
        .map(|s| format!("{} flags={:#x} size={} align={}", s.name, s.flags, s.size, s.align))
        .collect();
    assert!(!code.is_empty(), "no code section in {listing:#?}");
    assert!(
        code.iter().all(|section| section.align >= 32),
        "a code section aligned below 32 bytes: {listing:#?}"
    );
}
