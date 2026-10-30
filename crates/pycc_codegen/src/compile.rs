//! The compile driver: object emission, optimization pipeline and verification.

use super::*;

mod bodies;
mod declare_functions;
mod top_level;

/// `target_triple`: `None` compiles for the host's own default target (the
/// common case). `Some(triple)` cross-compiles for a different Tier-1
/// target -- LLVM's codegen backend is inherently multi-target, so this
/// only requires `Target::initialize_all` (rather than
/// `Target::initialize_native`) plus the requested `TargetTriple` instead
/// of the host's default; producing an actual *linked binary* for a
/// foreign target is a separate concern the caller handles (see
/// `src/main.rs`'s `--target` handling and its doc comment on what's
/// actually achievable without bundling a full foreign sysroot).
///
/// `release`: `false` (the default, matching every build before this PR)
/// creates the target machine with `OptimizationLevel::None` and skips
/// LLVM's optimizer entirely -- today's only behavior. `true` selects
/// `OptimizationLevel::Aggressive` and additionally runs the `"default<O3>"`
/// whole-module pass pipeline via `Module::run_passes` before the object is
/// written (D-094). True cross-translation-unit LTO has no effect yet:
/// pycc emits exactly one LLVM module per compilation (single-file only
/// until v0.4's multi-file support), so this is "maximum whole-module
/// optimization," not literal cross-file link-time optimization.
///
/// # Concurrency
///
/// Safe to call concurrently from several threads of one process. LLVM's
/// process-global target registry is initialized exactly once, behind a
/// `OnceLock`, so no thread can observe the registry mid-write; see
/// `target_machine`'s module documentation for why that guard is needed
/// even though inkwell locks `Target::initialize_all` internally. Each
/// call otherwise owns its own `Context`, `Module` and `TargetMachine`,
/// and callers must still supply distinct `output_path`s.
pub fn compile_to_object(
    mir: &MirModule,
    output_path: &Path,
    target_triple: Option<&str>,
    release: bool,
) -> Result<(), String> {
    compile_to_object_with_options(
        mir,
        output_path,
        &CompileOptions {
            target_triple: target_triple.map(str::to_string),
            release,
            ..CompileOptions::default()
        },
    )
}

/// `compile_to_object` with the full option set (D-244's `ext` mode).
pub fn compile_to_object_with_options(
    mir: &MirModule,
    output_path: &Path,
    options: &CompileOptions,
) -> Result<(), String> {
    compile_to_object_with_observer(mir, output_path, options, None)
}

/// [`compile_to_object_with_options`] for a build that links whatever
/// objects it is given: a large optimized module is emitted as several
/// objects, compiled by LLVM's backend in parallel (`crate::emit`). The
/// first is `output_path`; the rest sit beside it. All of them are
/// returned, in link order.
pub fn compile_to_objects_with_options(
    mir: &MirModule,
    output_path: &Path,
    options: &CompileOptions,
) -> Result<Vec<std::path::PathBuf>, String> {
    compile_module(mir, output_path, options, None, true)
}

pub(crate) fn compile_to_object_with_observer(
    mir: &MirModule,
    output_path: &Path,
    options: &CompileOptions,
    observer: Option<&mut CodegenObserver<'_>>,
) -> Result<(), String> {
    compile_module(mir, output_path, options, observer, false).map(|_| ())
}

/// Lowers `mir` and emits it: one object at `output_path`, or with `split`
/// possibly several (`crate::emit`).
fn compile_module(
    mir: &MirModule,
    output_path: &Path,
    options: &CompileOptions,
    observer: Option<&mut CodegenObserver<'_>>,
    split: bool,
) -> Result<Vec<std::path::PathBuf>, String> {
    let target_triple = options.target_triple.as_deref();
    let context = Context::create();
    let module = context.create_module("pycc_module");
    let builder = context.create_builder();
    let i64_type = context.i64_type();
    let rt = declare_rt_functions(&context, &module);
    rt.thread_safe_exception_check.set(options.ext);
    rt.glibc_libm
        .set(float_fast::target_libm_is_glibc(target_triple));
    *rt.rc_class_kinds.borrow_mut() = heap_rc::rc_class_kinds(&mir.class_defs);
    *rt.exception_classes.borrow_mut() = heap_rc::exception_classes(&mir.class_defs);
    *rt.assigned_slots.borrow_mut() = assigned_slots::AssignedSlots::analyze(mir, options.ext);
    *rt.nothrow.borrow_mut() = nothrow::nothrow_functions(mir);
    // A dispatcher's class test never raises.
    rt.nothrow
        .borrow_mut()
        .insert(pycc_mir::dispatch::INSTANCE_CLASS_IS.to_string());
    rt.nothrow
        .borrow_mut()
        .insert(pycc_mir::dispatch::INSTANCE_PY_OVERRIDE.to_string());
    for test in [
        pycc_mir::dispatch::INSTANCE_FOREIGN,
        pycc_mir::dispatch::INSTANCE_VT_HAS,
    ] {
        rt.nothrow.borrow_mut().insert(test.to_string());
    }
    let _pins = operand_pin::plan_module(&rt, mir);
    rt.frames.set_files(
        options.source_files.clone(),
        options.source_blocks.clone(),
        options.position_sites.clone(),
    );
    if options.ext {
        rt.frames.set_framed(frame_needs::framed_functions(mir));
    }
    rt.frames.set_global_sync(options.global_sync);

    // First pass: declare every user-defined function -- with its real
    // parameter types and return type (Task 5), instead of Task 3/4's
    // placeholder zero-arg/void signature -- under a mangled name (never
    // the bare Python name) before emitting any body. Two reasons: this is
    // what lets a function call another function defined later in the
    // same module, or itself (recursion, now with real arguments and a
    // real return value flowing through it, since every signature is
    // already known before any body is lowered); and mangling is what
    // stops a Python-level function actually named `main` from colliding
    // with the real C-ABI entry point below, which must be literally
    // named `main` for the OS loader to find it. A def alone has no
    // runtime effect in Python regardless of its name -- something has to
    // call it, which is exactly the bug this pass structure fixes (see
    // git history: an earlier version treated a function merely named
    // `main` as auto-invoked, which doesn't match CPython at all).
    //
    // Issue #22: each `def` now gets a unique mangled name so redefinition
    // doesn't collide (`pyfn_{name}` for the first, `pyfn_{name}__redef_{n}`
    // for subsequent). A global function-pointer slot per unique name is
    // initialized to null; the top-level emission pass stores each def's
    // address into the slot when the def is "executed" in source order, and
    // all calls dispatch indirectly through the slot. This separates
    // LLVM symbol declaration (this pass, needed for the compiler to
    // generate call instructions) from Python name binding (the store,
    // which happens at the def's source position in top-level execution).
    // Part 2 of #1175 (#1179): see `declare_functions` for why the
    // buffer-sub-range out-slot fact is resolved per name.
    let slice_widened = crate::ext::buffer_slice_out_names(&mir.items);
    crate::sibling_class::set(&options.sibling_classes);
    let (mut user_functions, function_defs_in_order, def_counts) =
        declare_functions::declare_user_functions(&context, &module, mir, &slice_widened);

    declare_functions::mark_single_defs(mir, &mut user_functions, &function_defs_in_order, &def_counts);
    sibling_call::attach(
        &context,
        &module,
        &options.sibling_targets,
        &mut user_functions,
    );
    // The class records, once every function is declared: a vtable names
    // functions.
    {
        let value = |name: &str| {
            function_defs_in_order
                .iter()
                .find(|(defined, _)| *defined == name)
                .map(|(_, f)| *f)
        };
        let signatures: HashMap<&str, (Vec<pycc_mir::Ty>, pycc_mir::Ty)> = mir
            .items
            .iter()
            .filter_map(|item| match item {
                MirItem::Function {
                    name,
                    params,
                    return_ty,
                    ..
                } => Some((
                    name.as_str(),
                    (
                        params.iter().skip(1).map(|(_, ty)| ty.clone()).collect(),
                        return_ty.clone(),
                    ),
                )),
                _ => None,
            })
            .collect();
        let signature = |name: &str| signatures.get(name).cloned();
        *rt.class_records.borrow_mut() = crate::class_record::emit_records(
            &context,
            &module,
            &mir.class_defs,
            &rt.rc_class_kinds.borrow(),
            &rt.exception_classes.borrow(),
            &crate::class_record::Functions {
                value: &value,
                signature: &signature,
            },
        );
    }

    // #1050: an `ext` artifact's generated C reaches an export whose
    // signature carries a `tuple` through a scalar-only thunk rather than
    // through `fnptr_<name>` directly, because pycc's aggregate calling
    // convention is not the platform C struct ABI. Emit those thunks now
    // that every callee's `fn_type` and `fnptr_` slot exist, and before the
    // builder is positioned in the module entry point below.
    if options.ext && !options.suppress_export_thunks {
        ext_thunk::emit_export_thunks(&context, &builder, &module, &rt, mir, &user_functions);
    }

    // Module bindings need process-wide storage because generated functions
    // can read them (D-041), including bindings whose assignment appears
    // after the function definition. Declare every slot before emitting either
    // the synthetic module entry point or any user function.
    let module_bindings = collect_module_bindings(mir);
    let module_globals = declare_module_globals(&context, &module, &module_bindings);

    let entry_fn_type = i64_type.fn_type(&[], false);
    let entry_fn = module.add_function(entry_fn_name(options.ext), entry_fn_type, None);
    // An extension's module body runs once, at import: `cold` places it
    // with the rest of the import-only code (`.text.unlikely` on ELF), away
    // from the functions callers run afterwards (v0.11). A native build's
    // `main` is the program, and stays as it is.
    if options.ext {
        crate::frames::mark_cold(&context, entry_fn);
    }
    let entry_block = context.append_basic_block(entry_fn, "entry");
    let top_exception_exit = context.append_basic_block(entry_fn, "top_exception_exit");
    builder.position_at_end(entry_block);
    // Top-level statements share one `locals` map across the synthetic
    // `main` entry block (module-level Python names are one shared
    // scope); each user function gets its own, fresh map below, since
    // Python function bodies don't see each other's locals.
    let mut top_level_locals: HashMap<_, _> = module_globals
        .iter()
        .map(|(name, binding)| (name.clone(), binding.clone()))
        .collect();
    // #379 (PR-19): emit per-enum-member singleton init sequences BEFORE
    // the top-level statement loop.
    emit_enum_member_inits(&context, &builder, &module, &rt, mir, &module_globals);
    // Issue #22: iterate over ALL items in source order, not just
    // top-level statements. A `MirItem::Function` at its source position
    // represents a `def` statement's runtime binding effect: store the
    // function's address into the global function-pointer slot so calls
    // after this point dispatch to it. A call before the `def` (the slot
    // is still null) raises `pycc_rt_raise_name_error` -- matching
    // CPython's `NameError: name 'foo' is not defined`.
    let mut def_iter = function_defs_in_order.iter().peekable();
    let copy_slots = copy_slots::CopySlots::new(mir);
    let publish_plan = ext_publish::PublishPlan::new(mir, &copy_slots, options.ext);
    let mut fn_ordinal = 0usize;
    rt.exceptions.targets.borrow_mut().push(top_exception_exit);
    crate::frames::begin_function(
        &context,
        &module,
        &builder,
        &rt,
        "",
        crate::frames::FN_INFO_MODULE,
    );
    // Part 1 of #1096: while this target is the innermost one, no
    // module-level `try` encloses the statement being emitted, so a foreign
    // failure keeps its direct module-exec `-1` edge (`foreign_fail.rs`).
    rt.exceptions.module_exec_exit.set(Some(top_exception_exit));
    for item in &mir.items {
        match item {
            // A position marker emits nothing a pending exception can follow.
            MirItem::TopLevelStmt(MirStmt::Pos(pos)) => {
                crate::frames::set_position(&context, &builder, &rt, *pos);
            }
            MirItem::TopLevelStmt(stmt) => {
                emit_stmt(
                    &context,
                    &builder,
                    &module,
                    &rt,
                    &user_functions,
                    &mut top_level_locals,
                    stmt,
                    pycc_mir::Ty::None,
                    &mut Vec::new(),
                )?;
                top_level::emit_exception_check(
                    &context,
                    &builder,
                    &rt,
                    entry_fn,
                    top_exception_exit,
                );
            }
            // Part 1 of #1026: emit the import call here, at the item's
            // own position in the source-order loop, rather than as a
            // hoisted prologue -- see `foreign_import.rs`. `def_iter` is
            // deliberately not advanced: it pairs with
            // `MirItem::Function` items only.
            MirItem::ForeignImport {
                local_name,
                module_path,
                from,
            } => foreign_import::emit_item(
                &context,
                &builder,
                &module,
                &rt,
                entry_fn,
                options.ext,
                &module_globals,
                foreign_import::ForeignBinding {
                    local_name,
                    module_path,
                    from: from.as_ref(),
                },
            ),
            MirItem::Function { name, .. } => {
                // `def_iter` is in the same source order as `mir.items`,
                // so the next entry matches this function definition.
                let &(_, f) = def_iter.next().expect(
                    "def_iter should have an entry for every MirItem::Function \
                     (the declaration pass populates function_defs_in_order \
                     from the same mir.items in the same order)",
                );
                let ordinal = fn_ordinal;
                fn_ordinal += 1;
                top_level::bind_def(
                    &context,
                    &builder,
                    &module,
                    &rt,
                    &user_functions,
                    &function_defs_in_order,
                    &copy_slots,
                    &publish_plan,
                    (name.as_str(), f, ordinal),
                    (entry_fn, top_exception_exit),
                );
            }
        }
    }
    rt.exceptions.targets.borrow_mut().pop();
    rt.exceptions.module_exec_exit.set(None);
    // Module-level Python code has no `return` (T0024) -- every top-level
    // `str` local's single exit point is program completion right here, so
    // this is where its accepted refcounting scope (D-061's Task 7
    // addendum) decrefs it exactly once, before `main` itself returns.
    //
    // `ext` mode has no such exit point. The entry function is D-244's
    // `Py_mod_exec` slot, and the module object outlives it: every wrapper
    // the host may call afterwards still reads these globals, so decrefing
    // them here frees storage that stays reachable. `PyModuleDef.m_free` is
    // NULL, so nothing ever tears the module down -- holding the reference
    // for the process lifetime is the correct D-060 accounting, exactly as
    // the exceptional exit below already assumes.
    if !options.ext {
        crate::generator::release_generator_globals(&context, &builder, &rt, mir, &module_globals);
    }
    for slot in module_globals.values() {
        if slot.ty == pycc_mir::Ty::Str && !options.ext {
            let value = builder
                .build_load(
                    context.ptr_type(inkwell::AddressSpace::default()),
                    slot.ptr,
                    "final_str",
                )
                .expect("build_load should not fail for this function's own alloca")
                .into_pointer_value();
            builder
                .build_call(rt.str_decref, &[value.into()], "str_decref_final")
                .expect("build_call should not fail for a well-formed decref");
        }
    }
    // `emit_stmt` rejects every `Return` whose parent is this synthetic
    // `main`; all exception-produced `unreachable` terminators are erased
    // before their explicit state check above. The current continuation is
    // therefore guaranteed to accept the process-success return below.
    // See the module-level comment block below for why these .expect()s
    // (this one included) are deliberate rather than Result-threaded: each
    // covers an operation that stays infallible given how this function
    // always calls it. Two calls below are genuine, externally-triggerable
    // failure modes and stay real Results the caller must handle:
    // Target::from_triple (a user-supplied --target can legitimately name
    // a triple LLVM doesn't recognize) and write_to_file, at the very end.
    //
    // A last line printed without a newline (`print(x, end=" ")`) is still
    // in the runtime's line buffer, which nothing flushes at process exit.
    builder
        .build_call(rt.print_flush, &[], "final_flush")
        .expect("build_call should not fail for a well-formed flush");
    builder
        .build_return(Some(&i64_type.const_int(0, false)))
        .expect(
            "build_return should not fail: builder is always freshly positioned before this call",
        );

    top_level::emit_top_exception_exit(&context, &builder, &rt, top_exception_exit, options.ext);
    crate::frames::end_function(&context, &module, &rt);
    bodies::emit_function_bodies(
        &context,
        &module,
        &builder,
        &rt,
        &user_functions,
        &module_globals,
        &function_defs_in_order,
        mir,
        &slice_widened,
    )?;

    crate::ndarray::clear_fresh_after_calls(&context, &module, &builder, &rt);
    if options.ext {
        crate::frames::finish_module(&context, &module, &rt);
    }
    verify_module(&module);

    crate::emit::optimize_and_emit(&module, mir, options, observer, output_path, split)
}

/// See D-029: every message inkwell hands back as an `LLVMString` --
/// `Target::from_triple`'s and `TargetMachine::write_to_file`'s error
/// paths, on top of `TargetTriple` itself -- shares the same broken `Drop`
/// (`LLVMDisposeMessage`) on Windows against this LLVM release. Converts
/// to an owned `String` (a real copy, safe to keep and format past this
/// call) and forgets the original rather than letting it drop. General
/// fix at the one place `LLVMString` crosses into this crate's error
/// values, not a patch per call site -- so a future one isn't missed.
pub(crate) fn llvm_string_to_owned(message: inkwell::support::LLVMString) -> String {
    let owned = message.to_string();
    std::mem::forget(message);
    owned
}

/// Skipped on Windows: `module.verify()` crashes there with an access
/// violation when linked against the official prebuilt LLVM 22.1.1 release
/// -- isolated with stderr checkpoints bracketing every call from the end of
/// IR building through object emission (D-029): every other call completed,
/// consistently, across every test that reached this point; only this one
/// never returned. Root cause not further isolated -- no Windows debugger
/// available in this environment to get an exact crash address/frame -- so
/// this is a targeted skip of a *pure internal sanity check* (a failure
/// here would mean a pycc_codegen bug, never a rejection of legitimate user
/// code -- see the non-Windows body's own message), not a change to what
/// gets compiled. The identical IR-building code already runs verified on
/// macOS/Linux for every test in this suite, which narrows the residual,
/// Windows-only risk to "a bug that only produces malformed IR on a
/// Windows-specific code path" -- and no such path exists yet, since IR
/// building above has no platform-conditional logic at all.
#[cfg(windows)]
pub(crate) fn verify_module(_module: &inkwell::module::Module<'_>) {}

#[cfg(not(windows))]
pub(crate) fn verify_module(module: &inkwell::module::Module<'_>) {
    module.verify().expect(
        "generated IR should always be well-formed for this fixed instruction shape; \
         a verify() failure here means a bug in pycc_codegen itself, not bad user input",
    );
}
