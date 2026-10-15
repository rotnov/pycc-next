//! The `finally` clause and the statement's exit, shared by `try` and
//! `try*`.

use super::*;

/// Where a `try`/`try*` statement's `finally` clause stands: the blocks it
/// joins and the `return` record its `FinallyTarget` held (when it has a
/// `finally` body).
pub(super) struct FinallyTail<'ctx> {
    pub(super) finally_bb: inkwell::basic_block::BasicBlock<'ctx>,
    pub(super) after_bb: inkwell::basic_block::BasicBlock<'ctx>,
    pub(super) is_returning: Option<PointerValue<'ctx>>,
    pub(super) ret_slot: Option<PointerValue<'ctx>>,
    /// The prefix of the emitted block names (`try` or `trystar`).
    pub(super) prefix: &'static str,
}

/// Emits the `finally` clause at `tail.finally_bb` and the statement's exit
/// at `tail.after_bb`.
///
/// A pending exception must not keep `finally` from running: it is taken
/// out of the pending state into an exception slot (`owned_slots`) while
/// the body runs. Normal completion raises it again; an exception raised by
/// the body, or a `return`, `break` or `continue` out of it, replaces it,
/// and the slot's release frees it. A `return` routed through this
/// `finally` then continues outwards, releasing the exception slots of the
/// handlers it leaves.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_finally_tail<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &mut HashMap<String, StorageSlot<'ctx>>,
    finalbody: &[MirStmt],
    expected_return_ty: pycc_mir::Ty,
    finally_stack: &mut Vec<FinallyTarget<'ctx>>,
    tail: FinallyTail<'ctx>,
) -> Result<(), String> {
    let prefix = tail.prefix;
    let function = builder.get_insert_block().unwrap().get_parent().unwrap();
    builder.position_at_end(tail.finally_bb);
    let finally_exception_bb =
        context.append_basic_block(function, &format!("{prefix}_finally_exception"));
    let pending_slot = if finalbody.is_empty() {
        None
    } else {
        let depth = owned_slots::depth(rt);
        let slot = owned_slots::new_slot(context, builder, &format!("{prefix}_finally_pending"));
        let value = builder
            .build_call(
                rt.exception_take,
                &[],
                &format!("{prefix}_finally_pending_value"),
            )
            .expect("build_call should not fail for exception_take")
            .try_as_basic_value()
            .expect_basic("pycc_rt_exception_take returns a pointer")
            .into_pointer_value();
        owned_slots::hold(builder, rt, slot, value);
        let cleanup = owned_slots::push_cleanup_target(
            context,
            builder,
            rt,
            &format!("{prefix}_finally_cleanup"),
        );
        emit_body(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            finalbody,
            expected_return_ty.clone(),
            finally_stack,
        )?;
        rt.exceptions.targets.borrow_mut().pop();
        owned_slots::fill_cleanup_target(
            context,
            builder,
            rt,
            cleanup,
            depth,
            finally_exception_bb,
        );
        owned_slots::pop_to(rt, depth);
        Some(slot)
    };
    // A `raise` inside the finally body terminates the block with
    // `unreachable`. Erase it so the finally-completion check can run here.
    erase_unreachable_if_present(builder);
    let finally_falls_through = builder
        .get_insert_block()
        .unwrap()
        .get_terminator()
        .is_none();
    if finally_falls_through {
        if let Some(slot) = pending_slot {
            restore_pending(context, builder, rt, slot, prefix);
        }
        match tail.is_returning {
            Some(is_returning) => emit_return_dispatch(
                context,
                builder,
                rt,
                is_returning,
                tail.ret_slot,
                tail.after_bb,
                expected_return_ty,
                finally_stack,
                prefix,
            ),
            // No finally body: the structured-statement guard at
            // `after_bb` propagates any pending exception.
            None => {
                builder
                    .build_unconditional_branch(tail.after_bb)
                    .expect("build_unconditional_branch should not fail");
            }
        }
    }

    // An exception raised by `finally` overrides an intercepted return. It
    // keeps the runtime exception active while skipping the return-flag
    // dispatch.
    builder.position_at_end(finally_exception_bb);
    builder
        .build_unconditional_branch(tail.after_bb)
        .expect("build_unconditional_branch should propagate a finally exception");

    builder.position_at_end(tail.after_bb);
    // A locally unmatched exception (or one restored after `finally`) must
    // leave this structured statement immediately.
    guard_statement_effects(context, builder, rt);
    Ok(())
}

/// Raises the exception `finally` set aside in `slot` again, if any,
/// handing the slot's reference to the pending state.
fn restore_pending<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    slot: PointerValue<'ctx>,
    prefix: &str,
) {
    let function = builder.get_insert_block().unwrap().get_parent().unwrap();
    let ptr_type = context.ptr_type(inkwell::AddressSpace::default());
    let pending = builder
        .build_load(ptr_type, slot, &format!("{prefix}_finally_pending_held"))
        .expect("build_load should not fail for an exception slot")
        .into_pointer_value();
    builder
        .build_store(slot, ptr_type.const_null())
        .expect("build_store should not fail for an exception slot");
    let had_pending = builder
        .build_is_not_null(pending, &format!("{prefix}_finally_had_pending"))
        .expect("build_is_not_null should not fail");
    let restore_bb =
        context.append_basic_block(function, &format!("{prefix}_finally_restore_exception"));
    let restored_bb =
        context.append_basic_block(function, &format!("{prefix}_finally_exception_restored"));
    builder
        .build_conditional_branch(had_pending, restore_bb, restored_bb)
        .expect("build_conditional_branch should not fail");
    builder.position_at_end(restore_bb);
    builder
        .build_call(rt.exception_raise, &[pending.into()], "")
        .expect("build_call should not fail for exception_raise");
    builder
        .build_unconditional_branch(restored_bb)
        .expect("build_unconditional_branch should not fail");
    builder.position_at_end(restored_bb);
}

/// After a `finally` body with a `FinallyTarget`: completes an intercepted
/// `return` -- `ret`, or on to the enclosing `finally` -- or continues at
/// `after_bb`.
#[allow(clippy::too_many_arguments)]
fn emit_return_dispatch<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    is_returning: PointerValue<'ctx>,
    ret_slot: Option<PointerValue<'ctx>>,
    after_bb: inkwell::basic_block::BasicBlock<'ctx>,
    expected_return_ty: pycc_mir::Ty,
    finally_stack: &mut [FinallyTarget<'ctx>],
    prefix: &str,
) {
    let function = builder.get_insert_block().unwrap().get_parent().unwrap();
    let flag_val = builder
        .build_load(
            context.i8_type(),
            is_returning,
            &format!("{prefix}_finally_ret_flag"),
        )
        .expect("build_load should not fail for is_returning flag")
        .into_int_value();
    let is_ret = builder
        .build_int_compare(
            inkwell::IntPredicate::NE,
            flag_val,
            context.i8_type().const_zero(),
            &format!("{prefix}_finally_is_ret"),
        )
        .expect("build_int_compare should not fail");
    let ret_bb = context.append_basic_block(function, &format!("{prefix}_finally_ret"));
    builder
        .build_conditional_branch(is_ret, ret_bb, after_bb)
        .expect("build_conditional_branch should not fail");
    builder.position_at_end(ret_bb);
    // The return leaves the handlers this `try` sits in, up to the next
    // `finally` it passes through.
    owned_slots::release_above(
        context,
        builder,
        rt,
        owned_slots::return_depth(finally_stack),
    );
    let outer = finally_stack.last();
    match (ret_slot, outer) {
        (Some(slot), Some(outer)) => {
            let ret_val = builder
                .build_load(
                    ty_to_basic_type(context, expected_return_ty),
                    slot,
                    &format!("{prefix}_finally_ret_val"),
                )
                .expect("build_load should not fail for ret_slot");
            let outer_slot = outer
                .ret_slot
                .expect("nested finally in the same non-None function has a return slot");
            builder
                .build_store(outer_slot, ret_val)
                .expect("build_store should not fail for outer ret_slot");
            builder
                .build_store(outer.is_returning, context.i8_type().const_int(1, false))
                .expect("build_store should not fail for outer is_returning");
            builder
                .build_unconditional_branch(outer.finally_bb)
                .expect("build_unconditional_branch should not fail");
        }
        (Some(slot), None) => {
            let ret_val = builder
                .build_load(
                    ty_to_basic_type(context, expected_return_ty),
                    slot,
                    &format!("{prefix}_finally_ret_val"),
                )
                .expect("build_load should not fail for ret_slot");
            builder
                .build_return(Some(&ret_val))
                .expect("build_return should not fail for a finally-routed return");
        }
        (None, Some(outer)) => {
            builder
                .build_store(outer.is_returning, context.i8_type().const_int(1, false))
                .expect("build_store should not fail for outer is_returning");
            builder
                .build_unconditional_branch(outer.finally_bb)
                .expect("build_unconditional_branch should not fail");
        }
        // Void function (`Ty::None`): `ret void`. Top-level `main` returns
        // `i64` and cannot `return`, so its dispatch block is dead.
        (None, None) if function.get_type().get_return_type().is_none() => {
            builder
                .build_return(None)
                .expect("build_return should not fail for a finally-routed void return");
        }
        (None, None) => {
            builder
                .build_unreachable()
                .expect("build_unreachable should not fail for dead ret_bb");
        }
    }
}
