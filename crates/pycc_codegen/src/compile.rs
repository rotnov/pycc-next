//! The compile driver: object emission, optimization pipeline and verification.

use super::*;

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

pub(crate) fn compile_to_object_with_observer(
    mir: &MirModule,
    output_path: &Path,
    options: &CompileOptions,
    mut observer: Option<&mut CodegenObserver<'_>>,
) -> Result<(), String> {
    let target_triple = options.target_triple.as_deref();
    let release = options.release;
    let context = Context::create();
    let module = context.create_module("pycc_module");
    let builder = context.create_builder();
    let i64_type = context.i64_type();
    let rt = declare_rt_functions(&context, &module);
    rt.thread_safe_exception_check.set(options.ext);
    rt.glibc_libm
        .set(float_fast::target_libm_is_glibc(target_triple));
    *rt.rc_class_kinds.borrow_mut() = heap_rc::rc_class_kinds(&mir.class_defs);
    *rt.assigned_slots.borrow_mut() = assigned_slots::AssignedSlots::analyze(mir, options.ext);
    *rt.nothrow.borrow_mut() = nothrow::nothrow_functions(mir);
    let _pins = operand_pin::plan_module(&rt, mir);

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
    let mut user_functions: HashMap<&str, UserFunction> = HashMap::new();
    // Maps function name to the number of definitions seen so far, for
    // unique mangled-name generation on redefinition.
    let mut def_counts: HashMap<&str, usize> = HashMap::new();
    // List of (function name, LLVM function value) in source order, for
    // the top-level binding pass.
    let mut function_defs_in_order: Vec<(&str, FunctionValue)> = Vec::new();
    // Part 2 of #1175 (#1179), review round 2: the buffer-sub-range
    // out-slot fact is resolved **per name, over every definition of it**,
    // not per `def`. Two `def f`s share one `fnptr_f` slot and one
    // `UserFunction::fn_type`, so giving them two arities makes the
    // indirect call ill-typed for whichever definition the slot does not
    // describe -- which is the `verify()` abort a redefinition used to hit
    // when only the slicing definition was widened. See
    // `crate::ext::buffer_slice_out_names` for why the union is the safe
    // direction and for the driver-side half that unions with it.
    let slice_widened = crate::ext::buffer_slice_out_names(&mir.items);
    for item in &mir.items {
        if let MirItem::Function {
            name,
            params,
            return_ty,
            body,
        } = item
        {
            let mut param_types: Vec<inkwell::types::BasicMetadataTypeEnum> = params
                .iter()
                .map(|(_, ty)| ty_to_basic_type(&context, ty.clone()).into())
                .collect();
            // Part 2 of #1175 (#1179): a function whose body returns a
            // sub-range of a buffer parameter hands the sub-range's
            // `has_slice`/`start`/`stop` back through three trailing
            // out-pointers, because the bounds are produced inside this
            // frame while the returned value stays the parameter's own view
            // pointer -- which is what keeps the generated wrapper's
            // `result == &a{index}` identity test exact.
            //
            // The predicate is the body fact alone, deliberately not gated
            // on `options.ext` or on `ext_thunk_required`'s own
            // exportability test: this declaration and `emit_stmt`'s own
            // `MirStmt::ReturnBufferSlice` arm must never disagree about
            // the arity, and `pycc_types` already refuses the shape
            // everywhere a wrapper cannot exist.
            if slice_widened.contains(name.as_str()) {
                let ptr_type = context.ptr_type(inkwell::AddressSpace::default());
                for _ in crate::ext::ext_thunk_out_tys(return_ty, true) {
                    param_types.push(ptr_type.into());
                }
            }
            let fn_type = match return_ty {
                pycc_mir::Ty::None => context.void_type().fn_type(&param_types, false),
                other => ty_to_basic_type(&context, other.clone()).fn_type(&param_types, false),
            };
            let count = def_counts.entry(name.as_str()).or_insert(0);
            let mangled = if *count == 0 {
                format!("pyfn_{name}")
            } else {
                format!("pyfn_{name}__redef_{count}")
            };
            *count += 1;
            let f = module.add_function(&mangled, fn_type, None);
            if leaf_inline::is_tiny_leaf(body) {
                leaf_inline::mark(&context, f);
            }
            function_defs_in_order.push((name.as_str(), f));
            // Only insert into user_functions on the first definition;
            // subsequent definitions update fn_ptr_global at their source
            // position but share the same slot and type info. The param_tys
            // and fn_type come from the first definition (the type checker
            // resolves one signature per name).
            user_functions.entry(name.as_str()).or_insert_with(|| {
                // Monomorphized generic specializations (`0gen_...` names)
                // are compiler-generated, not user-defined: they have no
                // top-level `def` whose execution order matters, so they
                // dispatch directly through `direct_value` instead of
                // through the indirect function-pointer slot.
                let is_monomorphized = name.starts_with("0gen_");
                if is_monomorphized {
                    UserFunction {
                        param_tys: params.iter().map(|(_, ty)| ty.clone()).collect(),
                        fn_ptr_global: None,
                        fn_type,
                        name_global: None,
                        direct_value: Some(f),
                        single_def: None,
                        bound_before_code: false,
                    }
                } else {
                    let fn_ptr_type = context.ptr_type(inkwell::AddressSpace::default());
                    // A method's `name` is dotted (`Grid.scale.static`), and
                    // this global is the one codegen emits that the generated
                    // C *declares* -- `extern void *fnptr_<name>;` in
                    // `pycc::ext_build`'s `wrapper_for`. A dotted C identifier
                    // is not accepted by any compiler, so the symbol carries
                    // the mangled spelling. inkwell 0.9 exposes no alias API
                    // and the workspace has no `llvm-sys` dependency, so this
                    // is a rename and not a C-safe alias beside the dotted
                    // one; nothing outside this workspace consumes the
                    // spelling. `mangle_ext_name` is the identity for a
                    // dot-free name, so every module-level function's global
                    // is byte-identical to what it was.
                    //
                    // This path is not gated on `CompileOptions::ext`, so a
                    // native build renames a method's global too, where no C
                    // ever declares it. That is deliberate: one spelling per
                    // symbol is cheaper to reason about than a mode-dependent
                    // one, and nothing outside this workspace reads it.
                    let fn_ptr_global = module.add_global(
                        fn_ptr_type,
                        None,
                        &format!("fnptr_{}", mangle_ext_name(name)),
                    );
                    fn_ptr_global.set_initializer(&fn_ptr_type.const_null());
                    // Deliberately *not* mangled, on both counts. The symbol
                    // is internal-linkage and never declared from C, so it
                    // needs no C-legal spelling; and its contents are the
                    // string `pycc_rt_name_error` prints, so the array size
                    // and initializer stay on the source dotted name --
                    // mangling them would make a method's `NameError` read
                    // `0m4_Grid5_scale6_static`.
                    let name_global = module.add_global(
                        context.i8_type().array_type(name.len() as u32 + 1),
                        None,
                        &format!("fnname_{name}"),
                    );
                    name_global.set_linkage(Linkage::Internal);
                    name_global.set_constant(true);
                    name_global.set_initializer(&context.const_string(name.as_bytes(), true));
                    UserFunction {
                        param_tys: params.iter().map(|(_, ty)| ty.clone()).collect(),
                        fn_ptr_global: Some(fn_ptr_global),
                        fn_type,
                        name_global: Some(name_global),
                        direct_value: None,
                        single_def: None,
                        bound_before_code: false,
                    }
                }
            });
        }
    }

    // A name defined once and never bound as a variable can only ever hold
    // that one definition (see `UserFunction::single_def`). The ext
    // wrappers only read the slot, so CPython rebinding the module attribute
    // does not reach compiled callers -- the same as for indirect calls.
    let rebound_names = collect_module_bindings(mir);
    for &(name, f) in &function_defs_in_order {
        if def_counts[name] == 1
            && !rebound_names.contains_key(name)
            && let Some(uf) = user_functions.get_mut(name)
            && uf.fn_ptr_global.is_some()
        {
            uf.single_def = Some(f);
        }
    }
    // A single definition whose `def` precedes every statement that can run
    // code is bound before any function body can run, so no call reaches
    // its slot while it is still null (see `UserFunction::bound_before_code`).
    // Enum member singletons are built before the first item, so a module
    // that has any keeps every check.
    if mir.class_defs.iter().all(|(_, class_def)| class_def.enum_members.is_empty()) {
        for item in &mir.items {
            let MirItem::Function { name, .. } = item else {
                break;
            };
            if let Some(uf) = user_functions.get_mut(name.as_str())
                && uf.single_def.is_some()
            {
                uf.bound_before_code = true;
            }
        }
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
    // is still null) aborts with `pycc_rt_name_error` -- matching
    // CPython's `NameError: name 'foo' is not defined`.
    let mut def_iter = function_defs_in_order.iter().peekable();
    let copy_slots = copy_slots::CopySlots::new(mir);
    let publish_plan = ext_publish::PublishPlan::new(mir, &copy_slots, options.ext);
    let mut fn_ordinal = 0usize;
    rt.exceptions.targets.borrow_mut().push(top_exception_exit);
    // Part 1 of #1096: while this target is the innermost one, no
    // module-level `try` encloses the statement being emitted, so a foreign
    // failure keeps its direct module-exec `-1` edge (`foreign_fail.rs`).
    rt.exceptions.module_exec_exit.set(Some(top_exception_exit));
    for item in &mir.items {
        match item {
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
                // #382: After each top-level statement, check for an active
                // exception. A `raise` (or a converted runtime failure like
                // division by zero) sets the pending exception state and
                // terminates the block with `unreachable`. Erase the
                // `unreachable` so the exception check can run here. This
                // check intercepts it at the top level: if an exception is
                // active, print it and exit. Inside a `try`, the try's own
                // post-body check intercepts first. `emit_stmt` rejects a
                // `Return` whose parent is this synthetic `main`, so after
                // erasing an exception's `unreachable` the current block is
                // guaranteed to accept this check.
                erase_unreachable_if_present(&builder);
                let active = crate::int_fast::emit_exception_pending(
                    &context,
                    &builder,
                    &rt,
                    "top_exc_active",
                );
                let has_exc = builder
                    .build_int_compare(
                        inkwell::IntPredicate::NE,
                        active,
                        context.i8_type().const_zero(),
                        "top_has_exc",
                    )
                    .expect("build_int_compare should not fail");
                let cont_bb = context.append_basic_block(entry_fn, "top_exc_cont");
                builder
                    .build_conditional_branch(has_exc, top_exception_exit, cont_bb)
                    .expect("build_conditional_branch should not fail");
                builder.position_at_end(cont_bb);
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
                // Store this definition's function pointer into the
                // global slot, representing the `def`'s runtime binding.
                // `def_iter` is in the same source order as `mir.items`,
                // so the next entry matches this function definition.
                // Monomorphized generic specializations (`0gen_...` names)
                // have no `fn_ptr_global` (they dispatch directly), so
                // skip the store for them.
                let &(_, f) = def_iter.next().expect(
                    "def_iter should have an entry for every MirItem::Function                      (the declaration pass populates function_defs_in_order                      from the same mir.items in the same order)",
                );
                let ordinal = fn_ordinal;
                fn_ordinal += 1;
                // #1337 (D-254): an inherited-method copy is bound with the
                // origin `def` it copies, not at its own (appended)
                // position -- see `copy_slots`.
                if copy_slots.is_copy(ordinal) {
                    continue;
                }
                let bound = std::iter::once((name.as_str(), f)).chain(
                    copy_slots
                        .bound_with(ordinal)
                        .iter()
                        .map(|&copy| function_defs_in_order[copy]),
                );
                for (name, f) in bound {
                    let uf = &user_functions[name];
                    if let Some(ref fn_ptr_global) = uf.fn_ptr_global {
                        let _ = builder.build_store(
                            fn_ptr_global.as_pointer_value(),
                            f.as_global_value().as_pointer_value(),
                        );
                    }
                }
                // #1199: publish what this `def` (and its copies) made
                // callable -- see `ext_publish`. A `def` is refused inside
                // any module-level block, so no module-level `try` encloses
                // this point, and a failed publish takes the direct
                // module-exec `-1` edge, as a foreign failure here does
                // (Part 1 of #1096).
                debug_assert_eq!(
                    rt.exceptions.targets.borrow().last(),
                    Some(&top_exception_exit)
                );
                ext_publish::emit(
                    &context,
                    &builder,
                    &module,
                    entry_fn,
                    publish_plan.names_at(ordinal),
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

    // Every exceptional module-scope edge converges here. Keeping one
    // target alive while top-level expressions are emitted lets recursive
    // expression guards stop later operands and effects immediately.
    builder.position_at_end(top_exception_exit);
    if options.ext {
        // D-244's `ext` mode: the module body runs inside the host
        // interpreter's `Py_mod_exec` slot, so an uncaught module-scope
        // exception must *fail the import*, not end the process.
        // `pycc_rt_exception_print_and_exit` is `-> !` and calls
        // `std::process::exit(1)`, which here would terminate the
        // interpreter that just imported this artifact. Return `-1`
        // instead, leaving the pending exception state set: the C shim
        // reads it through `pycc_rt_ext_pending_type`/`_message`, raises
        // the matching CPython exception, and returns `-1` from the slot.
        // `print_and_exit` stays `native` mode's handler, untouched.
        builder
            .build_return(Some(
                &i64_type.const_int(EXT_MODULE_EXEC_FAILED as u64, true),
            ))
            .expect("build_return should not fail on a freshly positioned builder");
    } else {
        let exc_val = builder
            .build_call(rt.exception_value, &[], "top_exc_val")
            .expect("build_call should not fail for exception_value")
            .try_as_basic_value()
            .expect_basic("pycc_rt_exception_value returns a pointer")
            .into_pointer_value();
        builder
            .build_call(rt.exception_print_and_exit, &[exc_val.into()], "")
            .expect("build_call should not fail for exception_print_and_exit");
        builder
            .build_unreachable()
            .expect("build_unreachable should terminate a noreturn block");
    }

    // Second pass: fill in each user function's body, now that every
    // function (including ones a body might call) is already declared.
    // Each parameter is bound into `fn_locals` by allocating a fresh slot
    // for it and storing the incoming LLVM argument into it (Task 5) --
    // the same load/store-via-`alloca` model every other local already
    // uses (see `emit_assign`), so a parameter is fully ordinary once
    // bound: reassignable, and readable via `emit_expr`'s `Name` arm with
    // no special-casing.
    // Issue #22: use `function_defs_in_order` to get the correct LLVM
    // function value for each definition -- a redefined name has multiple
    // function values (one per def, with unique mangled names), and each
    // body must be emitted into its own function value, not the first
    // definition's.
    let mut body_def_iter = function_defs_in_order.iter();
    for item in &mir.items {
        if let MirItem::Function {
            name,
            params,
            return_ty,
            body,
        } = item
        {
            // Advance the iterator in lockstep with `mir.items`'s
            // Function items -- same source order, same count.
            let f = body_def_iter
                .next()
                .map(|&(_, f)| f)
                .expect("function body has matching declaration");
            let block = context.append_basic_block(f, "entry");
            builder.position_at_end(block);
            rt.emitting_ctor.set(rt.assigned_slots.borrow().is_ctor(name));
            // A generator function is a coroutine: its frame is allocated
            // first, and its body is emitted as if it returned `None`
            // (`generator`).
            let generator_element = match return_ty {
                pycc_mir::Ty::Generator(element) => Some((**element).clone()),
                _ => None,
            };
            let generator_begin = generator_element
                .is_some()
                .then(|| crate::generator::begin(&context, &module, &builder, &rt, f));
            let return_ty = if generator_element.is_some() {
                &pycc_mir::Ty::None
            } else {
                return_ty
            };
            let mut fn_locals: HashMap<_, _> = module_globals
                .iter()
                .map(|(global_name, binding)| (global_name.clone(), binding.clone()))
                .collect();
            // #1054: the release list for this function's `str` epilogue.
            //
            // **Ownership invariant this epilogue depends on (plan §7 R2):
            // a `str` slot holds exactly one *owned* `PyStrObj` reference,
            // or null.** Every producer upholds it -- a parameter slot
            // receives an argument the caller already `pycc_rt_str_incref`d
            // (`emit_user_call_args`' own `incref_if_str_duplicate`) or a
            // freshly built object with refcount 1 (a literal, a
            // concatenation, the D-244 `ext` shim's `pycc_ext_unpack_str`);
            // a rebinding store releases the old value first
            // (`decref_str_slot_before_store`); a `return` of a bare name
            // increfs the duplicate it hands back. So exactly one
            // `pycc_rt_str_decref` per slot on the way out balances the
            // books, and the null a never-assigned local still holds is the
            // runtime's documented no-op.
            //
            // Deliberately a *snapshot* taken here rather than a walk of
            // `fn_locals` at exit: `fn_locals` is seeded above with this
            // module's globals (which this function does not own) and is
            // extended mid-function by `exception.rs`'s `except X as e`
            // bindings (whose slots are not in the entry block and so need
            // not dominate the epilogue).
            let mut owned_str_slots: Vec<PointerValue> = Vec::new();
            // The same list for counted lists and instances (`heap_rc`),
            // under the same invariant: a parameter slot owns its argument
            // and a local slot what it was last assigned, or null.
            let mut owned_heap_slots: Vec<(PointerValue, heap_rc::HeapKind)> = Vec::new();
            // #625: the same list for `int` slots. A parameter slot owns its
            // argument (the caller transfers a fresh word and retains a
            // borrowed one, `emit_user_call_args`) and a local slot the word
            // it was last assigned (`release_int_slot_before_store`), or `0`;
            // the epilogue releases them as it does a `str` slot, so a heap
            // bigint held by a parameter or a local dies with its frame.
            let mut owned_int_slots: Vec<(PointerValue, bool)> = Vec::new();
            rt.owned_int_names.borrow_mut().clear();
            // Part 2a of #1142 (#1165): the same list for artifact-owned
            // buffer storage, kept separate from `owned_str_slots` because
            // the two release through different runtime helpers.
            //
            // **Only locals are ever pushed here, never parameters.** A
            // buffer parameter holds a `PyccExtBufferView *` the *host*
            // owns and the wrapper borrowed for one call; freeing it would
            // be an invalid free. `pycc_types`' `buffer_parameter_rebinding`
            // refuses the one shape (`b = ndarray(3)` on a parameter `b`)
            // that could make a parameter slot look like it needed
            // releasing, so this asymmetry with the `str` list above is a
            // checked invariant rather than an assumption.
            let mut owned_buffer_slots: Vec<PointerValue> = Vec::new();
            for (i, (param_name, ty)) in params.iter().enumerate() {
                // `.expect(...)`, not `.unwrap_or_else(|| panic!(...))`:
                // `f`'s own `fn_type` (built above, in the first pass) was
                // constructed from this exact `params` list, so
                // `get_nth_param` always succeeds for every `i` this loop
                // produces -- an unreachable "missing parameter" case
                // would be a defensive branch this crate's own coverage
                // gate (D-014) could never legitimately exercise, so it's
                // not introduced in the first place, the same reasoning
                // already applied elsewhere in this file (e.g.
                // `emit_body_then_branch`'s removed guard, Task 4).
                let incoming = f.get_nth_param(i as u32).expect(
                    "this function was declared with exactly `params.len()` parameters above",
                );
                let slot = storage_slot_at_entry(&context, &builder, ty.clone(), param_name, false);
                builder.build_store(slot.ptr, incoming).expect(
                    "build_store should not fail for a slot this function itself allocated",
                );
                if *ty == pycc_mir::Ty::Str {
                    owned_str_slots.push(slot.ptr);
                } else if *ty == pycc_mir::Ty::Int {
                    owned_int_slots.push((slot.ptr, true));
                    rt.owned_int_names.borrow_mut().insert(param_name.clone());
                } else if let Some(kind) = heap_rc::heap_kind(&rt, ty) {
                    owned_heap_slots.push((slot.ptr, kind));
                }
                fn_locals.insert(param_name.clone(), slot);
            }
            // Part 2 of #1175 (#1179), review round 2: bind this frame's
            // three out-pointers -- present exactly when the declaration
            // pass above widened this *name* -- so both `MirStmt::Return`
            // and `MirStmt::ReturnBufferSlice` reach them by name instead
            // of by `count_params()` arithmetic, and so a bare `return`
            // can tell that it owes the wrapper a `has_slice = 0`.
            if slice_widened.contains(name.as_str()) {
                let out_base = f.count_params() - 3;
                for (offset, key) in [
                    BUFFER_SLICE_HAS_SLICE_KEY,
                    BUFFER_SLICE_START_KEY,
                    BUFFER_SLICE_STOP_KEY,
                ]
                .into_iter()
                .enumerate()
                {
                    let ptr = f
                        .get_nth_param(out_base + offset as u32)
                        .expect("the declaration pass widened this signature by three out-pointers")
                        .into_pointer_value();
                    fn_locals.insert(
                        key.to_string(),
                        StorageSlot {
                            ptr,
                            ty: pycc_mir::Ty::Int,
                            initialized: None,
                        },
                    );
                }
            }
            let mut local_bindings = BTreeMap::new();
            for stmt in body {
                collect_stmt_bindings(stmt, &mut local_bindings);
            }
            for (param_name, _) in params {
                local_bindings.remove(param_name);
            }
            *rt.borrowed_locals.borrow_mut() = borrowed_locals::borrowed_locals(
                params,
                body,
                local_bindings
                    .iter()
                    .filter(|(_, ty)| {
                        matches!(ty, pycc_mir::Ty::List(_) | pycc_mir::Ty::Instance(_))
                    })
                    .map(|(name, _)| name),
            );
            for (local_name, ty) in local_bindings {
                let is_str = ty == pycc_mir::Ty::Str;
                let heap = heap_rc::heap_kind(&rt, &ty)
                    .filter(|_| !is_str && !rt.borrowed_locals.borrow().contains(&local_name));
                // Part 2a of #1142 (#1165): a buffer *local* is artifact-owned
                // storage and joins the release list; the parameter loop above
                // deliberately has no counterpart.
                let is_buffer = ty == pycc_mir::Ty::MemoryView;
                let is_int = ty == pycc_mir::Ty::Int;
                let slot = storage_slot_at_entry(&context, &builder, ty, &local_name, true);
                if is_int {
                    owned_int_slots.push((slot.ptr, false));
                    rt.owned_int_names.borrow_mut().insert(local_name.clone());
                }
                if is_str {
                    owned_str_slots.push(slot.ptr);
                }
                if let Some(kind) = heap {
                    owned_heap_slots.push((slot.ptr, kind));
                }
                if is_buffer {
                    owned_buffer_slots.push(slot.ptr);
                }
                // A function-local target shadows a same-named module global
                // throughout the function (D-055), so this intentionally
                // replaces any global slot seeded above.
                fn_locals.insert(local_name, slot);
            }
            // A `for` loop that must hold its list keeps that reference in
            // a hidden owned slot, so a `return` or an exception out of the
            // loop releases it through the epilogue.
            {
                let mut slots = rt.loop_list_slots.borrow_mut();
                slots.clear();
                for (stmt, name, loop_body, kind) in container_loops(body) {
                    if list_may_be_rebound(&fn_locals, loop_body, name) {
                        let ptr_type = context.ptr_type(inkwell::AddressSpace::default());
                        let slot = builder
                            .build_alloca(ptr_type, "for_list_hold")
                            .expect("build_alloca should not fail for a loop hold slot");
                        builder
                            .build_store(slot, ptr_type.const_null())
                            .expect("build_store should not fail for a loop hold slot");
                        owned_heap_slots.push((slot, kind));
                        slots.insert(std::ptr::from_ref(stmt) as usize, slot);
                    }
                }
            }
            // Part 2b of #1142 (#1164): the pending-return record, seeded
            // only for a frame that can actually hand a buffer back and owns
            // at least one buffer slot to hand. Both allocas belong to the
            // entry block, where the builder still is, on the same model as
            // every storage slot above.
            //
            // Seeded *after* the binding loop above deliberately: that loop
            // is what fills `owned_buffer_slots`, and a synthetic entry
            // picked up as an ordinary `memoryview` local would join the
            // release list and make the epilogue free the returned pointer a
            // second time.
            let pending_return_slots =
                if *return_ty == pycc_mir::Ty::MemoryView && !owned_buffer_slots.is_empty() {
                    let pending = builder
                        .build_alloca(
                            context.ptr_type(inkwell::AddressSpace::default()),
                            "pending_return_buffer_slot",
                        )
                        .expect("build_alloca should not fail for the pending-return slot");
                    builder
                        .build_store(
                            pending,
                            context
                                .ptr_type(inkwell::AddressSpace::default())
                                .const_null(),
                        )
                        .expect("build_store should not fail for the pending-return slot init");
                    let orphaned = builder
                        .build_alloca(context.i8_type(), "pending_return_orphaned_slot")
                        .expect("build_alloca should not fail for the pending-return orphan flag");
                    builder
                        .build_store(orphaned, context.i8_type().const_zero())
                        .expect("build_store should not fail for the orphan flag init");
                    fn_locals.insert(
                        PENDING_RETURN_BUFFER_KEY.to_string(),
                        StorageSlot {
                            ptr: pending,
                            ty: pycc_mir::Ty::MemoryView,
                            initialized: None,
                        },
                    );
                    fn_locals.insert(
                        PENDING_RETURN_ORPHANED_KEY.to_string(),
                        StorageSlot {
                            ptr: orphaned,
                            ty: pycc_mir::Ty::Bool,
                            initialized: None,
                        },
                    );
                    Some((pending, orphaned))
                } else {
                    None
                };
            // #1054: when this function owns at least one entry-block
            // `str` slot, every way out of it is routed through a single
            // synthetic outermost `finally` frame whose block releases
            // those slots. Reusing #382's existing `FinallyTarget`
            // machinery rather than inventing a landing pad means
            // `MirStmt::Return` and `exception.rs`'s own `try`/`try*`
            // return routing need no edit at all: they already store to
            // the innermost frame's `ret_slot` and branch to its
            // `finally_bb`, and a nested frame already propagates outward
            // to this one. Only the two paths that bypass `finally_stack`
            // entirely -- the implicit `return None` fall-through and the
            // `exception_exit` carrier return, both below -- are
            // redirected here.
            //
            // Gated on a non-empty snapshot so a `str`-free function emits
            // byte-identical IR to before this change, which is also what
            // keeps the "no outer `finally_stack` entry" arms of
            // `MirStmt::Return` and of `emit_try`/`emit_try_star`'s
            // `ret_bb` reachable (their tests build `str`-free functions).
            // The module entry point (`main` / `__pycc_ext_exec`) is a
            // separate emitter above and never gets this frame.
            let mut finally_stack: Vec<FinallyTarget> = Vec::new();
            // Part 2a of #1142 (#1165): the gate widens to "this function
            // owns *any* releasable entry-block slot". Left keyed on
            // `owned_str_slots` alone, a buffer-owning but `str`-free
            // function would get no epilogue block at all and leak one
            // allocation per call into a long-lived host process -- silently,
            // since nothing else in this emitter would notice.
            let owned_slot_epilogue_bb = if owned_str_slots.is_empty()
                && owned_buffer_slots.is_empty()
                && owned_heap_slots.is_empty()
                && owned_int_slots.is_empty()
            {
                None
            } else {
                let is_returning = builder
                    .build_alloca(context.i8_type(), "str_epilogue_is_returning")
                    .expect("build_alloca should not fail for the str epilogue is_returning flag");
                builder
                    .build_store(is_returning, context.i8_type().const_zero())
                    .expect("build_store should not fail for the str epilogue is_returning init");
                let ret_slot = if *return_ty == pycc_mir::Ty::None {
                    None
                } else {
                    Some(
                        builder
                            .build_alloca(
                                ty_to_basic_type(&context, return_ty.clone()),
                                "str_epilogue_ret_slot",
                            )
                            .expect("build_alloca should not fail for the str epilogue ret_slot"),
                    )
                };
                let finally_bb = context.append_basic_block(f, "pycc_str_epilogue");
                finally_stack.push(FinallyTarget {
                    finally_bb,
                    ret_slot,
                    is_returning,
                });
                Some((finally_bb, ret_slot))
            };
            let exception_exit = context.append_basic_block(f, "exception_exit");
            rt.exceptions.targets.borrow_mut().push(exception_exit);
            if let (Some(begun), Some(element)) = (generator_begin, generator_element.clone()) {
                crate::generator::initial_suspend(
                    &context, &module, &builder, &rt, f, begun, element,
                );
            }
            emit_body(
                &context,
                &builder,
                &module,
                &rt,
                &user_functions,
                &mut fn_locals,
                body,
                return_ty.clone(),
                &mut finally_stack,
            )?;
            rt.exceptions.targets.borrow_mut().pop();
            // A `None`-returning function falling through its last
            // statement without an explicit `return` is ordinary, legal
            // Python (an implicit `return None`); a non-`None`-returning
            // function falling through is not -- `pycc_types`' T0024
            // fallthrough check rejects that HIR before it ever reaches
            // codegen, so seeing it here is this crate's own internal
            // error, not a user-facing rejection (see this task's own
            // `a_non_none_function_falling_through_is_an_internal_error_
            // not_bad_ir` test).
            match return_ty {
                pycc_mir::Ty::None => {
                    if builder
                        .get_insert_block()
                        .unwrap()
                        .get_terminator()
                        .is_none()
                    {
                        // #1054: the implicit `return None` never passes
                        // through `finally_stack`, so it is redirected to
                        // the owned-slot epilogue explicitly. No `ret_slot`
                        // store: a `None`-returning function's epilogue
                        // has none, and the epilogue does not read
                        // `is_returning` -- every path into it returns.
                        if let Some((epilogue_bb, _)) = owned_slot_epilogue_bb {
                            builder.build_unconditional_branch(epilogue_bb).expect(
                                "build_unconditional_branch should not fail for str epilogue routing",
                            );
                        } else {
                            builder.build_return(None).expect(
                                "build_return should not fail: builder is always freshly positioned before this call",
                            );
                        }
                    }
                }
                _ if builder
                    .get_insert_block()
                    .unwrap()
                    .get_terminator()
                    .is_none() =>
                {
                    if fallthrough::block_always_terminates(body) {
                        builder.build_unreachable().expect(
                            "build_unreachable should terminate a statically impossible continuation",
                        );
                    } else {
                        panic!(
                            "pycc_codegen: internal error: `{name}` is declared to return a \
                             non-`None` value but fell through without a `return` -- \
                             pycc_types::check (T0024) should have rejected this HIR before \
                             it reached codegen"
                        );
                    }
                }
                _ => {}
            }

            // A Python exception crosses the native function ABI via the
            // thread-local runtime flag. Return a neutral carrier here;
            // the caller's expression guard observes the still-active flag
            // before it can consume that carrier or evaluate another effect.
            builder.position_at_end(exception_exit);
            // #1054: the exceptional exit is the other path that bypasses
            // `finally_stack`. The slots are released here too -- an
            // in-flight exception does not make the references this
            // function owns somebody else's problem.
            match (owned_slot_epilogue_bb, *return_ty == pycc_mir::Ty::None) {
                (Some((epilogue_bb, _)), true) => {
                    builder.build_unconditional_branch(epilogue_bb).expect(
                        "build_unconditional_branch should not fail for an exceptional None exit",
                    );
                }
                (Some((epilogue_bb, ret_slot)), false) => {
                    let default = default_value_for_type(&context, return_ty.clone());
                    let slot = ret_slot.expect(
                        "a non-`None`-returning function's str epilogue always has a ret_slot",
                    );
                    builder
                        .build_store(slot, default)
                        .expect("build_store should not fail for the exceptional carrier");
                    builder.build_unconditional_branch(epilogue_bb).expect(
                        "build_unconditional_branch should not fail for an exceptional value exit",
                    );
                }
                (None, true) => {
                    builder
                        .build_return(None)
                        .expect("build_return should not fail for an exceptional None exit");
                }
                (None, false) => {
                    let default = default_value_for_type(&context, return_ty.clone());
                    builder
                        .build_return(Some(&default))
                        .expect("build_return should not fail for an exceptional value exit");
                }
            }

            // #1054: emitted last, after every branch into it exists. One
            // `pycc_rt_str_decref` per owned `str` entry-block slot -- a
            // null slot (a local on a path that never assigned it) is the
            // runtime's documented no-op -- then, since Part 2a of #1142
            // (#1165), one `pycc_rt_buffer_f64_free` per owned buffer slot
            // on the same model, and finally the function's single real
            // `ret`. The IR value names below keep their `str_` prefix so a
            // `str`-owning function still emits byte-identical IR.
            if let Some((epilogue_bb, ret_slot)) = owned_slot_epilogue_bb {
                builder.position_at_end(epilogue_bb);
                crate::slot_epilogue::release_owned_slots(
                    &context,
                    &builder,
                    &rt,
                    &owned_str_slots,
                    &owned_int_slots,
                    &owned_heap_slots,
                );
                // Part 2a of #1142 (#1165): one `pycc_rt_buffer_f64_free`
                // per artifact-owned buffer slot, on exactly the `str`
                // model above -- the null a never-assigned local still
                // holds is the runtime's documented no-op, which is what
                // covers a buffer allocated in only one arm of an `if`.
                //
                // Part 2b of #1142 (#1164): **this block is where ownership
                // of a returned buffer transfers**, and it is the only
                // place it can correctly happen. A `return` statement makes
                // a value *pending*; it is not the point control leaves the
                // frame. Between the two, `finally_stack` interposes every
                // user finalizer -- which may read the returned name, raise,
                // or return a different buffer of its own -- so a transfer
                // performed at the `return` statement is wrong for all three
                // (a finalizer reading a cleared slot dereferences null and
                // aborts the hosting interpreter; a finalizer that raises or
                // overrides the return abandons the pointer the statement
                // already disowned). Every exit routes through this one
                // block with the value that is actually being returned
                // already stored in `ret_slot`, so the transfer is decided
                // here by identity against that value.
                //
                // Branch-free by construction: `select` replaces the
                // returned pointer with the null `pycc_rt_buffer_f64_free`
                // documents as a no-op, so the release loop keeps the single
                // straight-line shape #1165 gave it. No two owned slots can
                // hold one pointer -- a bare read of an owned buffer name is
                // refused by `owned_buffer_use_unsupported` everywhere but
                // this return position, so `b = a` cannot alias -- which is
                // what makes "skip the one slot whose value is being
                // returned" release each allocation exactly once.
                let returned_buffer =
                    if owned_buffer_slots.is_empty() || *return_ty != pycc_mir::Ty::MemoryView {
                        None
                    } else {
                        let slot = ret_slot.expect(
                        "a `memoryview`-returning function's owned-slot epilogue has a ret_slot",
                    );
                        Some(
                            builder
                                .build_load(
                                    context.ptr_type(inkwell::AddressSpace::default()),
                                    slot,
                                    "buffer_epilogue_returned",
                                )
                                .expect("build_load should not fail for the epilogue ret_slot")
                                .into_pointer_value(),
                        )
                    };
                for slot_ptr in &owned_buffer_slots {
                    let live = builder
                        .build_load(
                            context.ptr_type(inkwell::AddressSpace::default()),
                            *slot_ptr,
                            "buffer_epilogue_live",
                        )
                        .expect(
                            "build_load should not fail for a buffer slot this function allocated",
                        )
                        .into_pointer_value();
                    let release = match returned_buffer {
                        None => live,
                        Some(returned) => {
                            let transferred = builder
                                .build_int_compare(
                                    inkwell::IntPredicate::EQ,
                                    live,
                                    returned,
                                    "buffer_epilogue_transferred",
                                )
                                .expect("build_int_compare should not fail for two pointers");
                            builder
                                .build_select(
                                    transferred,
                                    context
                                        .ptr_type(inkwell::AddressSpace::default())
                                        .const_null(),
                                    live,
                                    "buffer_epilogue_release",
                                )
                                .expect("build_select should not fail for two pointers")
                                .into_pointer_value()
                        }
                    };
                    builder
                        .build_call(
                            rt.buffer_f64_free,
                            &[release.into()],
                            "buffer_epilogue_free",
                        )
                        .expect("build_call should not fail for pycc_rt_buffer_f64_free");
                }
                // Part 2b of #1142 (#1164): the one pointer the loop above
                // cannot see. A finalizer that rebound the returned name left
                // the pending record as that pointer's sole owner, so it is
                // released here -- unless it is the value actually being
                // returned, in which case it belongs to the host now.
                //
                // Gated on the orphan flag rather than on the record alone:
                // unorphaned, the pending pointer is still in some local slot
                // and the loop above already released it, so freeing it again
                // here would be a double free on every exception exit.
                if let (Some((pending_ptr, orphaned_ptr)), Some(returned)) =
                    (pending_return_slots, returned_buffer)
                {
                    let pending = builder
                        .build_load(
                            context.ptr_type(inkwell::AddressSpace::default()),
                            pending_ptr,
                            "buffer_epilogue_pending",
                        )
                        .expect("build_load should not fail for the pending-return slot")
                        .into_pointer_value();
                    let orphaned = builder
                        .build_load(context.i8_type(), orphaned_ptr, "buffer_epilogue_orphaned")
                        .expect("build_load should not fail for the pending-return orphan flag")
                        .into_int_value();
                    let was_orphaned = builder
                        .build_int_compare(
                            inkwell::IntPredicate::NE,
                            orphaned,
                            context.i8_type().const_zero(),
                            "buffer_epilogue_was_orphaned",
                        )
                        .expect("build_int_compare should not fail for an i8 flag");
                    let transferred = builder
                        .build_int_compare(
                            inkwell::IntPredicate::EQ,
                            pending,
                            returned,
                            "buffer_epilogue_pending_transferred",
                        )
                        .expect("build_int_compare should not fail for two pointers");
                    let null = context
                        .ptr_type(inkwell::AddressSpace::default())
                        .const_null();
                    let unless_transferred = builder
                        .build_select(transferred, null, pending, "buffer_epilogue_pending_keep")
                        .expect("build_select should not fail for two pointers")
                        .into_pointer_value();
                    let release = builder
                        .build_select(
                            was_orphaned,
                            unless_transferred,
                            null,
                            "buffer_epilogue_pending_release",
                        )
                        .expect("build_select should not fail for two pointers")
                        .into_pointer_value();
                    builder
                        .build_call(
                            rt.buffer_f64_free,
                            &[release.into()],
                            "buffer_epilogue_pending_free",
                        )
                        .expect("build_call should not fail for pycc_rt_buffer_f64_free");
                }
                match ret_slot {
                    Some(slot) => {
                        let ret_val = builder
                            .build_load(
                                ty_to_basic_type(&context, return_ty.clone()),
                                slot,
                                "str_epilogue_ret_val",
                            )
                            .expect("build_load should not fail for the str epilogue ret_slot");
                        builder
                            .build_return(Some(&ret_val))
                            .expect("build_return should not fail for the str epilogue");
                    }
                    None => {
                        builder
                            .build_return(None)
                            .expect("build_return should not fail for a void str epilogue");
                    }
                }
            }
            if generator_element.is_some() {
                crate::generator::finish(&context, &module, &builder, &rt, f);
            }
        }
    }

    crate::ndarray::clear_fresh_after_calls(&context, &module, &builder, &rt);
    verify_module(&module);

    // See `target_machine::target_machine_for` for the LLVM target-registry
    // initialization contract this call satisfies, and for the D-029 and
    // D-073 reasoning behind the triple and relocation-model choices.
    let target_machine = target_machine::target_machine_for(target_triple, release)?;
    // `--release`'s whole-module optimization pipeline (D-094). This is
    // "maximum whole-module optimization," not literal cross-translation-
    // unit LTO: pycc emits exactly one LLVM module per compilation today
    // (single-file only until v0.4's multi-file support), so there is only
    // ever one module for `"default<O3>"` to optimize.
    //
    // Deliberately `.expect(..)`, not `.map_err(llvm_string_to_owned)?` like
    // `Target::from_triple` (in `target_machine`)/`write_to_file` below:
    // those two are genuine,
    // externally-triggerable failure modes with their own dedicated tests
    // and reachable `Err` paths this crate's 100% region-coverage gate
    // (D-014) can actually exercise. `run_passes` here runs a fixed,
    // always-valid pipeline string against a module `verify_module` has
    // already accepted (skipped only on Windows per D-029's own "no
    // Windows-specific IR-building path exists" reasoning, which applies
    // here too) -- there is no way to make this fail that a test could
    // construct, so this stays infallible given how this function always
    // calls it, the same treatment `create_target_machine` gets in
    // `target_machine` and `module.verify()` gets in `verify_module` below.
    // On the
    // vanishingly narrow chance this ever legitimately panics on Windows,
    // the `LLVMString`'s `Drop` would run during unwinding (D-029's crash)
    // -- an accepted, currently unreachable risk, not a silently ignored one.
    // A generator is an LLVM coroutine, which only LLVM's coroutine passes
    // lower: a debug build of a module with one runs the `O0` pipeline,
    // which includes them.
    let has_coroutines = mir.items.iter().any(|item| {
        matches!(item, MirItem::Function { return_ty: pycc_mir::Ty::Generator(_), .. })
    });
    if !release && has_coroutines {
        module
            .run_passes("default<O0>", &target_machine, PassBuilderOptions::create())
            .expect("LLVM's \"default<O0>\" pipeline lowers a verified module's coroutines");
    }
    // Developer hook for performance work: `PYCC_DUMP_LLVM=<path>` writes the
    // module's IR before (`<path>.pre.ll`) and after (`<path>`) the pipeline.
    let dump_path = std::env::var_os("PYCC_DUMP_LLVM").map(std::path::PathBuf::from);
    if let Some(path) = &dump_path {
        let mut pre = path.clone().into_os_string();
        pre.push(".pre.ll");
        let _ = module.print_to_file(std::path::Path::new(&pre));
    }
    let applied_pipeline = if release {
        module
            .run_passes(
                RELEASE_PASS_PIPELINE,
                &target_machine,
                PassBuilderOptions::create(),
            )
            .expect(
                "LLVM's \"default<O3>\" pipeline should never fail against a module this \
                 function has already verified, using a fixed, always-valid pipeline string",
            );
        Some(RELEASE_PASS_PIPELINE)
    } else {
        None
    };
    if let Some(path) = &dump_path {
        let _ = module.print_to_file(path);
    }
    if let Some(observer) = observer.as_mut() {
        observer(&module, applied_pipeline);
    }
    target_machine
        .write_to_file(&module, FileType::Object, output_path)
        .map_err(llvm_string_to_owned)
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
