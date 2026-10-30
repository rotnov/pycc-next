//! The second pass of `compile_module`: each user function's body, emitted
//! into the function value the declaration pass made for it. Extracted
//! from `compile.rs` per AGENTS.md's file-decomposition rule when the
//! whole-package build's sibling classes added methods no body is emitted
//! for (another module compiles them).

use super::*;

/// Emits the body of every function of `mir` (module docs).
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_function_bodies<'ctx>(
    context: &'ctx Context,
    module: &inkwell::module::Module<'ctx>,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    module_globals: &std::collections::BTreeMap<String, StorageSlot<'ctx>>,
    function_defs_in_order: &[(&str, FunctionValue<'ctx>)],
    mir: &MirModule,
    slice_widened: &std::collections::BTreeSet<String>,
) -> Result<(), String> {
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
            // Another module compiles a sibling class's method.
            if crate::sibling_class::producer_method(name).is_some() {
                continue;
            }
            let block = context.append_basic_block(f, "entry");
            builder.position_at_end(block);
            rt.emitting_ctor
                .set(rt.assigned_slots.borrow().is_ctor(name));
            // A generator function is a coroutine: its frame is allocated
            // first, and its body is emitted as if it returned `None`
            // (`generator`).
            let generator_element = match return_ty {
                pycc_mir::Ty::Generator(sig) if pycc_mir::body_yields(body) => {
                    Some((**sig).clone())
                }
                _ => None,
            };
            let generator_begin = generator_element
                .is_some()
                .then(|| crate::generator::begin(context, module, builder, rt, f));
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
                let slot = storage_slot_at_entry(context, builder, ty.clone(), param_name, false);
                builder.build_store(slot.ptr, incoming).expect(
                    "build_store should not fail for a slot this function itself allocated",
                );
                if *ty == pycc_mir::Ty::Str {
                    owned_str_slots.push(slot.ptr);
                } else if matches!(ty, pycc_mir::Ty::Int | pycc_mir::Ty::Num) {
                    owned_int_slots.push((slot.ptr, true));
                    rt.owned_int_names.borrow_mut().insert(param_name.clone());
                } else if let Some(kind) = heap_rc::heap_kind(rt, ty) {
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
            // A multi-value twin's out-pointers, by result index
            // (`multi_out::out_key`), after its parameters.
            for (offset, ty) in pycc_mir::multi_return::out_tys(body)
                .into_iter()
                .enumerate()
            {
                let ptr = f
                    .get_nth_param((params.len() + offset) as u32)
                    .expect("the declaration pass widened a twin's signature by its out-pointers")
                    .into_pointer_value();
                fn_locals.insert(
                    crate::multi_out::out_key(offset + 1),
                    StorageSlot {
                        ptr,
                        ty,
                        initialized: None,
                    },
                );
            }
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
                        matches!(
                            ty,
                            pycc_mir::Ty::List(_)
                                | pycc_mir::Ty::VarTuple(_)
                                | pycc_mir::Ty::Instance(_)
                        )
                    })
                    .map(|(name, _)| name),
            );
            // `int` locals whose every value is a smallint own nothing: no
            // release before a store or on the way out (`int_range`).
            *rt.int_ranges.borrow_mut() = crate::int_range::int_ranges(
                body,
                local_bindings
                    .iter()
                    .filter(|(_, ty)| **ty == pycc_mir::Ty::Int)
                    .map(|(name, _)| name),
                !user_functions.contains_key("abs"),
            );
            for (local_name, ty) in local_bindings {
                let is_str = ty == pycc_mir::Ty::Str;
                let heap = heap_rc::heap_kind(rt, &ty)
                    .filter(|_| !is_str && !rt.borrowed_locals.borrow().contains(&local_name));
                // Part 2a of #1142 (#1165): a buffer *local* is artifact-owned
                // storage and joins the release list; the parameter loop above
                // deliberately has no counterpart.
                let is_buffer = ty == pycc_mir::Ty::MemoryView;
                let is_int = matches!(ty, pycc_mir::Ty::Int | pycc_mir::Ty::Num);
                let slot = storage_slot_at_entry(context, builder, ty, &local_name, true);
                if is_int && !rt.int_ranges.borrow().is_small_local(&local_name) {
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
                                ty_to_basic_type(context, return_ty.clone()),
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
                    exc_depth: 0,
                    loop_depth: 0,
                });
                Some((finally_bb, ret_slot))
            };
            let exception_exit = context.append_basic_block(f, "exception_exit");
            rt.exceptions.targets.borrow_mut().push(exception_exit);
            crate::frames::begin_function(
                context,
                module,
                builder,
                rt,
                name,
                if generator_element.is_some() {
                    crate::frames::FN_INFO_GENERATOR
                } else {
                    0
                },
            );
            if let (Some(begun), Some(element)) = (generator_begin, generator_element.clone()) {
                crate::generator::initial_suspend(
                    context, module, builder, rt, f, name, begun, element,
                );
            }
            emit_body(
                context,
                builder,
                module,
                rt,
                user_functions,
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
                    let default = default_value_for_type(context, return_ty.clone());
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
                    let default = default_value_for_type(context, return_ty.clone());
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
                    context,
                    builder,
                    rt,
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
                                ty_to_basic_type(context, return_ty.clone()),
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
                crate::generator::finish(context, module, builder, rt, f);
            }
            // Every `ret` of the function exists now (`frames::exit_frame`).
            crate::frames::end_function(context, module, rt);
            *rt.int_ranges.borrow_mut() = Default::default();
        }
    }
    Ok(())
}
