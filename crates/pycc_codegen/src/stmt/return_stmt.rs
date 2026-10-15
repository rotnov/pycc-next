//! `return` lowering, including buffer-slice returns and `finally` routing.

use super::*;

// Part 2 of #1175 (#1179): `return b[start:stop]` for a
// `memoryview` **parameter** `b`.
//
// The two bounds are evaluated here, in the compiled frame, and
// written to the three trailing out-pointers the declaration pass
// gave this function. The returned *value* is the parameter's own
// whole view, byte-identical to what a bare `return b` returns, so
// the generated wrapper's `result == &a{index}` identity chain
// stays exact and Part 1's PEP 688 `held`-vs-probe check still runs
// against the whole window before the host derives the sub-range.
//
// `has_slice` is stored **last**, after both bounds, so a bound
// whose evaluation raises can never leave the flag set; on that
// path the frame leaves through its exception target and the
// wrapper's pending-exception bail returns before any acquire.
//
// An absent `start` is `0` and an absent `stop` is `i64::MAX`, both
// of which CPython's own `PySlice_New`/`PyObject_GetItem` clamp
// exactly as Python does; no slice arithmetic exists on this side.
// A bigint bound raises `OverflowError` in `build_untag_checked`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_return_buffer_slice_stmt<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &mut HashMap<String, StorageSlot<'ctx>>,
    stmt: &MirStmt,
    expected_return_ty: pycc_mir::Ty,
    finally_stack: &mut Vec<FinallyTarget<'ctx>>,
) -> Result<(), String> {
    let MirStmt::ReturnBufferSlice { name, start, stop } = stmt else {
        unreachable!()
    };
    let bound_or = |builder: &inkwell::builder::Builder<'ctx>,
                    locals: &mut HashMap<String, StorageSlot<'ctx>>,
                    bound: &Option<MirExpr>,
                    default: u64,
                    label: &str| match bound {
        Some(expr) => {
            let scalar = emit_expr(context, builder, module, rt, user_functions, locals, expr);
            let encoded = to_numeric_encoded_int(context, builder, scalar);
            build_untag_checked(context, builder, rt, UntagUse::Size, encoded, label)
        }
        None => context.i64_type().const_int(default, false),
    };
    let start_i64 = bound_or(builder, locals, start, 0, "buffer_slice_untag_start");
    let stop_i64 = bound_or(
        builder,
        locals,
        stop,
        i64::MAX as u64,
        "buffer_slice_untag_stop",
    );
    // The three out-pointers were bound at function entry, under
    // the reserved keys, exactly when the declaration pass widened
    // this name -- which it did, or this statement could not be
    // here.
    //
    // `has_slice` is **taken** out of `locals` rather than read
    // through it, for the duration of the delegated bare `return`
    // below: that arm reads the key's presence as "this frame owes
    // the wrapper a `has_slice = 0`", and this return's own answer
    // is 1. Restored immediately afterwards, so a later return in
    // the same frame still describes itself.
    let has_slice = locals
        .remove(BUFFER_SLICE_HAS_SLICE_KEY)
        .expect("a frame containing a buffer sub-range return was widened by the declaration pass");
    builder
        .build_store(locals[BUFFER_SLICE_START_KEY].ptr, start_i64)
        .expect("build_store should not fail through a buffer-slice out-pointer");
    builder
        .build_store(locals[BUFFER_SLICE_STOP_KEY].ptr, stop_i64)
        .expect("build_store should not fail through a buffer-slice out-pointer");
    builder
        .build_store(has_slice.ptr, context.i64_type().const_int(1, false))
        .expect("build_store should not fail through a buffer-slice out-pointer");
    // Delegated rather than duplicated: the value half of this
    // statement *is* a bare `return b`, including its `finally`
    // routing and its interaction with the frame's owned-slot
    // epilogue, and a second copy of that arm is exactly the drift
    // this node exists to avoid.
    let emitted = emit_stmt(
        context,
        builder,
        module,
        rt,
        user_functions,
        locals,
        &MirStmt::Return(Some(MirExpr::Name {
            name: name.clone(),
            ty: pycc_mir::Ty::MemoryView,
        })),
        expected_return_ty,
        finally_stack,
    );
    locals.insert(BUFFER_SLICE_HAS_SLICE_KEY.to_string(), has_slice);
    emitted
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_return_stmt<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &mut HashMap<String, StorageSlot<'ctx>>,
    stmt: &MirStmt,
    expected_return_ty: pycc_mir::Ty,
    finally_stack: &mut Vec<FinallyTarget<'ctx>>,
) -> Result<(), String> {
    let MirStmt::Return(value) = stmt else {
        unreachable!()
    };
    if is_module_entry_symbol(
        builder
            .get_insert_block()
            .unwrap()
            .get_parent()
            .unwrap()
            .get_name()
            .to_bytes(),
    ) {
        panic!(
            "pycc_codegen: internal error: a top-level statement terminated `main`'s \
             entry block -- pycc_types::check (T0024) should have rejected a module-level \
             `return` before it reached codegen"
        );
    }
    // Part 2 of #1175 (#1179), review round 2: in a frame the
    // declaration pass widened, **every** return describes itself.
    // This one hands the whole view back, so its answer is
    // `has_slice = 0`.
    //
    // Without this store the out-slots would describe "the last
    // return that executed a store" rather than "the return that
    // reached the wrapper", and the wrapper's own zero-init of the
    // flag would be load-bearing in a way it cannot be: a frame can
    // execute more than one `return`. `try: return b[1:3]` /
    // `finally: raise` / an enclosing `except` that swallows it /
    // `return b` left the abandoned sub-range's bounds in the slots
    // and applied them to the whole-buffer return.
    //
    // Stored before the value is evaluated, so a returned
    // expression that raises cannot leave a stale 1 behind either;
    // the wrapper bails on the pending exception before reading
    // either way, and 0 is the safe answer regardless.
    //
    // Absent for every frame that was not widened, which is every
    // frame in a build that has no buffer sub-range return at all.
    if let Some(has_slice) = locals.get(BUFFER_SLICE_HAS_SLICE_KEY) {
        builder
            .build_store(has_slice.ptr, context.i64_type().const_zero())
            .expect("build_store should not fail through a buffer-slice out-pointer");
    }
    // #382 (PR-22 Part 2): If inside a try-with-finally, route the
    // return through the finally block instead of emitting `ret`
    // directly. Store the return value to the finally's ret_slot,
    // set the is_returning flag, and branch to the finally block.
    // After the finally body runs, the codegen emits the `ret`.
    let finally_target = finally_stack.last().cloned();
    // #1387: in a function returning `object` (D-258), a bare
    // `return` means `return None` -- see `object_return` -- and,
    // since Part 2 of #1387, a native returned value (`None`
    // included) is boxed into the `object` the host receives -- see
    // `object_box`.
    let bare_none = MirExpr::NoneLiteral;
    let value = object_return::object_return_value(&expected_return_ty, value, &bare_none);
    match value {
        Some(expr) => {
            let scalar = if object_box::boxes_into(expr, &expected_return_ty) {
                Scalar::Object(object_box::emit_boxed(
                    context,
                    builder,
                    module,
                    rt,
                    user_functions,
                    locals,
                    expr,
                ))
            } else if object_coerce::coerces_into(expr, &expected_return_ty) {
                // Non-strict mode (#3): `object_coerce`.
                object_coerce::emit_coerced(
                    context,
                    builder,
                    module,
                    rt,
                    user_functions,
                    locals,
                    expr,
                    &expected_return_ty,
                    "return value",
                )
            } else {
                emit_expr(context, builder, module, rt, user_functions, locals, expr)
            };
            let scalar = heap_rc::retain_if_heap_duplicate(context, builder, rt, expr, scalar);
            // #625: `return n` of an `int` slot the epilogue would release
            // next, with no user `finally` in between that could read it,
            // moves the slot's reference out instead of retaining a second
            // one for the epilogue to drop. The slot keeps the smallint
            // `0` (word `1`), which the epilogue's guard skips without a
            // call (`emit_param_release`).
            let moved = match expr {
                MirExpr::Name {
                    name,
                    ty: pycc_mir::Ty::Int,
                } if finally_stack.len() == 1
                    && rt.owned_int_names.borrow().contains(name)
                    && expected_return_ty == pycc_mir::Ty::Int =>
                {
                    locals.get(name).map(|slot| slot.ptr)
                }
                _ => None,
            };
            let scalar = match moved {
                Some(slot) => {
                    builder
                        .build_store(slot, context.i64_type().const_int(1, false))
                        .expect("build_store should not fail for an int slot");
                    scalar
                }
                None => retain_if_int_duplicate(context, builder, rt, expr, scalar),
            };
            let scalar =
                coerce_scalar_to_type(context, builder, scalar, expected_return_ty.clone());
            // Part 2b of #1142 (#1164): a returned artifact-owned
            // buffer leaves through the frame's owned-slot
            // epilogue, and **nothing about ownership happens
            // here**. This statement only makes the value pending:
            // when `finally_stack` is non-empty the value is stored
            // to the innermost frame's `ret_slot` and control is
            // routed through every interposed user finalizer before
            // it reaches the epilogue. Clearing the slot here --
            // which is what this arm used to do -- made the slot
            // stop naming live storage while those finalizers were
            // still running, so `finally: a[0] = 42.0` dereferenced
            // null and aborted the hosting interpreter, and a
            // finalizer that raised or returned a buffer of its own
            // abandoned the pointer this statement had disowned.
            //
            // The transfer therefore lives in the epilogue, which is
            // the one point control actually leaves the frame; see
            // the `buffer_epilogue_returned` block there for how it
            // decides which slot not to release.
            //
            // What this statement does record is that the pointer is
            // now *pending*: a finalizer that rebinds the name it
            // came from would otherwise release it out from under the
            // host, so `free_buffer_slot_before_store` reads this
            // record to keep its hands off. Written for every
            // `memoryview` return shape rather than only a bare
            // `Name`, since the epilogue's own identity test makes a
            // record that no slot ever held a harmless no-op.
            //
            // `(pending, orphaned)` is **one two-field state** with a
            // coupling invariant: `orphaned != 0` means the pending
            // record is the pointer's *sole* owner, so nothing else
            // will release it. The record therefore has exactly two
            // mutators, and each must leave that invariant standing.
            // `free_buffer_slot_before_store` is the other one, and it
            // raises the flag in the same breath that it makes the
            // record the sole owner.
            //
            // This one supersedes the record, so its transition is
            // total: release the predecessor when this frame is its
            // only owner, store the new pointer, and clear the flag.
            // A raw store here -- which is what this arm used to do --
            // left both halves of the invariant broken whenever one
            // frame reached a second `return`: the superseded
            // pointer's only record was gone, leaking one buffer per
            // call, and the surviving flag then described a pointer
            // that was no longer pending, so the epilogue's trailing
            // release freed the *new* pointer that the owned-slot loop
            // had already released -- a double free that aborts the
            // hosting interpreter.
            //
            // A second `return` is still reachable after review round
            // 5 narrowed the egress admission
            // (`pycc_types::buffer::buffer_return_inside_finally`
            // now refuses a buffer egress from any function with a
            // `return` inside a `finally`). That narrowing removes
            // only the *simultaneous* case -- two returns in flight at
            // once -- because that requires the inner `return` to sit
            // lexically inside a `finally` the outer one's exit path
            // runs. The *sequential* case survives and is what this
            // transition serves: `try: return a` / `finally: raise`,
            // caught by an enclosing handler, then `a = ndarray(n)`
            // (which orphans the abandoned pointer) and a second
            // `return a`. `cancelled_then_rebind` and
            // `cancelled_then_rebind_then_raises` in
            // `tests/issue_1164_memoryview_egress.rs` are that shape;
            // each kills one half of this transition, the release and
            // the flag clear respectively.
            //
            // With one invariant-preserving mutator, "how many
            // syntactic paths reach a second `return`" stops being a
            // question correctness depends on: the state is restored at
            // every write, so a third and fourth write are the same
            // transition applied again.
            //
            // Clearing the flag unconditionally is sound because the
            // new pointer is, at this instant, held by a live local
            // slot: an owned buffer originates only at `a = ndarray(n)`
            // bound to a simple local name, and the read admitted at
            // this return position loads it back out of that slot.
            // "Not orphaned" is therefore true by construction of
            // every pending store.
            if let Scalar::MemoryView(returned) = scalar
                && let Some(pending) = locals.get(PENDING_RETURN_BUFFER_KEY)
            {
                let superseded = builder
                    .build_load(
                        context.ptr_type(inkwell::AddressSpace::default()),
                        pending.ptr,
                        "superseded_return_buffer",
                    )
                    .expect("build_load should not fail for the pending-return slot")
                    .into_pointer_value();
                let orphaned = &locals[PENDING_RETURN_ORPHANED_KEY];
                let was_orphaned = builder
                    .build_int_compare(
                        inkwell::IntPredicate::NE,
                        builder
                            .build_load(
                                context.i8_type(),
                                orphaned.ptr,
                                "superseded_return_orphaned",
                            )
                            .expect("build_load should not fail for the pending-return orphan flag")
                            .into_int_value(),
                        context.i8_type().const_zero(),
                        "superseded_return_was_orphaned",
                    )
                    .expect("build_int_compare should not fail for an i8 flag");
                // Both halves again. Gating on the flag rather than on
                // "a different pointer" is what keeps an unorphaned
                // predecessor -- still held by its own slot, which the
                // epilogue's loop will release -- from being freed
                // twice; a `return a` abandoned by a raising
                // finalizer, caught, and followed by a second `return
                // b` on a still-slot-held `b` is that shape. The
                // inequality makes
                // the symmetric state unreachable rather than merely
                // improbable: an orphaned record is by definition held
                // by no slot, so no later `return` can load it back and
                // name it again, but one `icmp` removes the dependence
                // on that argument entirely.
                let differs = builder
                    .build_int_compare(
                        inkwell::IntPredicate::NE,
                        superseded,
                        returned,
                        "superseded_return_differs",
                    )
                    .expect("build_int_compare should not fail for two pointers");
                let stale = builder
                    .build_and(was_orphaned, differs, "superseded_return_is_stale")
                    .expect("build_and should not fail for two i1 values");
                // Branch-free on the `select`-plus-null-free model the
                // epilogue and the free-before-store both already use,
                // so a single-`return` frame keeps the straight-line
                // shape it has today and pays one extra no-op free.
                let release = builder
                    .build_select(
                        stale,
                        superseded,
                        context
                            .ptr_type(inkwell::AddressSpace::default())
                            .const_null(),
                        "superseded_return_release",
                    )
                    .expect("build_select should not fail for two pointers")
                    .into_pointer_value();
                builder
                    .build_call(
                        rt.buffer_f64_free,
                        &[release.into()],
                        "superseded_return_free",
                    )
                    .expect("build_call should not fail for pycc_rt_buffer_f64_free");
                builder
                    .build_store(pending.ptr, returned)
                    .expect("build_store should not fail for the pending-return slot");
                builder
                    .build_store(orphaned.ptr, context.i8_type().const_zero())
                    .expect("build_store should not fail for the pending-return orphan flag");
            }
            if expected_return_ty == pycc_mir::Ty::None {
                // `None` parameters, call results, and stored names
                // use a canonical `i8 0` carrier inside expressions,
                // but a function
                // declared to return `None` has an LLVM `void`
                // signature. Evaluating above preserves any call or
                // name-load side effects; the carrier itself is
                // intentionally discarded here. Returning it as an
                // `i8` previously built invalid IR that only the
                // non-Windows verifier caught (D-029).
                // The return leaves the `except` handlers it is in, up to the
                // innermost `finally` it runs first.
                crate::exception::owned_slots::release_above(
                    context,
                    builder,
                    rt,
                    crate::exception::owned_slots::return_depth(finally_stack),
                );
                if let Some(ft) = finally_target {
                    builder
                        .build_store(ft.is_returning, context.i8_type().const_int(1, false))
                        .expect("build_store should not fail for is_returning");
                    builder
                        .build_unconditional_branch(ft.finally_bb)
                        .expect("build_unconditional_branch should not fail for finally routing");
                } else {
                    builder
                        .build_return(None)
                        .expect("build_return should not fail for a None return value");
                }
                return Ok(());
            }
            let basic_value: inkwell::values::BasicValueEnum = match scalar {
                Scalar::Generator(v) | Scalar::Closure(v) => v.into(),
                Scalar::Int(v) => v.into(),
                Scalar::Bool(v) => v.into(),
                Scalar::Float(v) => v.into(),
                Scalar::Str(v) => v.into(),
                // Pass-through, identical to `Str`'s arm directly
                // above: returning a `list[T]` returns one opaque
                // pointer, and `ty_to_basic_type` already gave the
                // function's LLVM signature the same pointer
                // return type it gives a `str`-returning one.
                Scalar::List(v) => v.into(),
                // Pass-through, identical to `List`'s arm directly
                // above: returning a `dict[K, V]` returns one
                // opaque pointer, and `ty_to_basic_type` already
                // gave the function's LLVM signature the same
                // pointer return type it gives a `str`/`list[T]`-
                // returning one.
                Scalar::Dict(v) => v.into(),
                // Pass-through, identical to `List`'s/`Dict`'s arms
                // directly above: returning a `set[T]` returns one
                // opaque pointer, and `ty_to_basic_type` already
                // gave the function's LLVM signature the same
                // pointer return type it gives a
                // `str`/`list[T]`/`dict[K, V]`-returning one.
                Scalar::Set(v) => v.into(),
                // Pass-through like the three arms above, but
                // returning a whole struct by value rather than one
                // opaque pointer (D-115) -- and
                // `ty_to_basic_type` already gave this function's
                // LLVM signature that same struct return type, so
                // the `ret` is well-typed with no tuple-specific
                // code here.
                Scalar::Tuple(v) => v.into(),
                // Pass-through, identical to `List`'s/`Dict`'s/
                // `Set`'s arms above (D-154, Part 1 of #375):
                // returning a class instance returns one opaque
                // pointer, and `ty_to_basic_type` already gave the
                // function's LLVM signature the same pointer return
                // type it gives a `str`/`list[T]`/`dict[K, V]`/
                // `set[T]`-returning one.
                Scalar::Instance(v) => v.into(),
                // Pass-through by VALUE, identical in kind to
                // `Tuple`'s arm above (D-197, #763, Part 1 of
                // #747): `coerce_scalar_to_type` above already
                // built the correctly-typed struct against
                // `expected_return_ty`, and `ty_to_basic_type`
                // already gave the function's LLVM signature that
                // same struct return type.
                Scalar::Optional(v) => v.into(),
                // Pass-through, identical to `List`'s/`Dict`'s/
                // `Set`'s/`Instance`'s arms above (D-244, Part 2 of
                // #1026): returning a foreign CPython object
                // returns one opaque `PyObject *`, and
                // `ty_to_basic_type`'s own `Ty::Object` arm already
                // gave the function's LLVM signature the same
                // pointer return type. This arm is genuinely
                // reachable: an unannotated private
                // helper whose body is `return numpy.pi` infers a
                // `Ty::Object` return type, which is the second
                // producer shape Part 2's refusal migration admits.
                // The caller owns the returned reference: a
                // borrowed value was retained above.
                Scalar::Object(v) => v.into(),
                // Part 2b of #1142 (#1164): pass-through by
                // pointer, identical in kind to `List`'s and
                // `Instance`'s arms above -- `ty_to_basic_type`'s
                // own `Ty::MemoryView` arm already gave the
                // function's LLVM signature the same pointer return
                // type, and the value is the `PyccExtBufferView *`
                // `pycc_rt_buffer_f64_alloc` produced.
                //
                // Two provenances reach here. *Artifact-owned*
                // storage is the original one. Part 1 of #1175
                // (#1178) adds the *caller-owned* one: a
                // `memoryview` **parameter** returned by name,
                // whose value is the address of the wrapper's own
                // `PyccExtBufferView` local. The earlier refusal of
                // that shape was justified here as a dangling view
                // -- the host's exporter being released on the way
                // out -- and that justification was wrong: the
                // wrapper acquires a second, independent buffer
                // export on the argument object before releasing
                // its own, and an export is what pins the storage.
                // See `pycc_types::buffer::admits_buffer_egress`.
                //
                // Nothing below has to distinguish the two. A
                // parameter never enters `owned_buffer_slots` (see
                // the comment at the slot-collection site), and a
                // local buffer slot only ever holds null or a
                // `pycc_rt_buffer_f64_alloc` heap pointer, never a
                // parameter's address -- so the orphan flag cannot
                // fire against a caller-owned pending return and
                // the epilogue cannot free one. This arm stores
                // nothing into the
                // slot: the ownership transfer happens later, in
                // the frame's owned-slot epilogue, where the
                // `buffer_epilogue_returned` identity test skips
                // exactly the pointer being returned and releases
                // every other owned slot. Clearing the slot here
                // instead is what made a `finally` block observe a
                // null name and abort the host.
                Scalar::MemoryView(v) => v.into(),
            };
            // The return leaves the `except` handlers it is in, up to the
            // innermost `finally` it runs first.
            crate::exception::owned_slots::release_above(
                context,
                builder,
                rt,
                crate::exception::owned_slots::return_depth(finally_stack),
            );
            if let Some(ft) = finally_target {
                // Route through finally: store the return value,
                // set the is_returning flag, and branch to finally.
                let slot = ft
                    .ret_slot
                    .expect("a value return routed through finally has a return slot");
                builder
                    .build_store(slot, basic_value)
                    .expect("build_store should not fail for ret_slot");
                builder
                    .build_store(ft.is_returning, context.i8_type().const_int(1, false))
                    .expect("build_store should not fail for is_returning");
                builder
                    .build_unconditional_branch(ft.finally_bb)
                    .expect("build_unconditional_branch should not fail for finally routing");
            } else {
                builder
                    .build_return(Some(&basic_value))
                    .expect("build_return should not fail for a well-formed return value");
            }
        }
        None => {
            // #380 (PR-20): an abstract method has a `Return(None)`
            // body but a non-`None` declared return type. The type
            // checker skips body checking for abstract methods, so
            // this is the only case where `Return(None)` reaches
            // codegen with a non-`None` `expected_return_ty`. Emit
            // a default value of the correct type so the LLVM IR
            // is well-typed; the abstract method is never called,
            // so the value doesn't matter.
            if expected_return_ty == pycc_mir::Ty::None {
                // The return leaves the `except` handlers it is in, up to the
                // innermost `finally` it runs first.
                crate::exception::owned_slots::release_above(
                    context,
                    builder,
                    rt,
                    crate::exception::owned_slots::return_depth(finally_stack),
                );
                if let Some(ft) = finally_target {
                    builder
                        .build_store(ft.is_returning, context.i8_type().const_int(1, false))
                        .expect("build_store should not fail for is_returning");
                    builder
                        .build_unconditional_branch(ft.finally_bb)
                        .expect("build_unconditional_branch should not fail for finally routing");
                } else {
                    builder
                        .build_return(None)
                        .expect("build_return should not fail for a bare `return`");
                }
            } else {
                let default = default_value_for_type(context, expected_return_ty.clone());
                // The return leaves the `except` handlers it is in, up to the
                // innermost `finally` it runs first.
                crate::exception::owned_slots::release_above(
                    context,
                    builder,
                    rt,
                    crate::exception::owned_slots::return_depth(finally_stack),
                );
                if let Some(ft) = finally_target {
                    let slot = ft.ret_slot.expect(
                        "a non-None default return routed through finally has a return slot",
                    );
                    builder
                        .build_store(slot, default)
                        .expect("build_store should not fail for ret_slot");
                    builder
                        .build_store(ft.is_returning, context.i8_type().const_int(1, false))
                        .expect("build_store should not fail for is_returning");
                    builder
                        .build_unconditional_branch(ft.finally_bb)
                        .expect("build_unconditional_branch should not fail for finally routing");
                } else {
                    builder
                        .build_return(Some(&default))
                        .expect("build_return should not fail for a default return value");
                }
            }
        }
    }
    Ok(())
}
