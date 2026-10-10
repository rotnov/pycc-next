//! `try`/`except`/`else`/`finally` lowering.

use super::*;

#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_try<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &mut HashMap<String, StorageSlot<'ctx>>,
    body: &[MirStmt],
    handlers: &[pycc_mir::MirExceptHandler],
    orelse: &[MirStmt],
    finalbody: &[MirStmt],
    expected_return_ty: pycc_mir::Ty,
    finally_stack: &mut Vec<FinallyTarget<'ctx>>,
) -> Result<(), String> {
    let function = builder.get_insert_block().unwrap().get_parent().unwrap();

    // Allocate blocks for the try body, handler dispatch, else,
    // finally, and merge (after-try).
    let try_body_bb = context.append_basic_block(function, "try_body");
    let handler_dispatch_bb = context.append_basic_block(function, "try_handler_dispatch");
    let else_bb = context.append_basic_block(function, "try_else");
    let finally_bb = context.append_basic_block(function, "try_finally");
    let after_bb = context.append_basic_block(function, "try_after");

    // #382 (PR-22 Part 2): If there is a finally body, set up a
    // `FinallyTarget` so `return` statements inside the try body,
    // handlers, and else body route through the finally block
    // before completing. The `is_returning` flag distinguishes a
    // return (which should emit `ret` after finally) from normal
    // completion (which branches to `after_bb`).
    let has_finally = !finalbody.is_empty();
    // Saved fields from the FinallyTarget, needed after the finally
    // body runs (the target itself is popped before emitting the
    // finally body to prevent self-interception).
    let saved_is_returning: Option<PointerValue<'ctx>>;
    let saved_ret_slot: Option<PointerValue<'ctx>>;
    if has_finally {
        let is_returning = alloca_at_entry(builder, context.i8_type(), "try_is_returning");
        builder
            .build_store(is_returning, context.i8_type().const_zero())
            .expect("build_store should not fail for is_returning init");
        let ret_slot = if expected_return_ty == pycc_mir::Ty::None {
            None
        } else {
            let slot = alloca_at_entry(
                builder,
                ty_to_basic_type(context, expected_return_ty.clone()),
                "try_ret_slot",
            );
            Some(slot)
        };
        saved_is_returning = Some(is_returning);
        saved_ret_slot = ret_slot;
        finally_stack.push(FinallyTarget {
            finally_bb,
            ret_slot,
            is_returning,
        });
    } else {
        saved_is_returning = None;
        saved_ret_slot = None;
    }

    // Enter the try body.
    builder
        .build_unconditional_branch(try_body_bb)
        .expect("build_unconditional_branch should not fail entering try body");
    builder.position_at_end(try_body_bb);

    // Emit the try body. After it completes, check for an active
    // exception and branch to handler dispatch or else.
    rt.exceptions.targets.borrow_mut().push(handler_dispatch_bb);
    emit_body(
        context,
        builder,
        module,
        rt,
        user_functions,
        locals,
        body,
        expected_return_ty.clone(),
        finally_stack,
    )?;
    rt.exceptions.targets.borrow_mut().pop();
    // #382: A `raise` inside the body terminates the block with
    // `unreachable`. Erase it so the exception check can run here.
    erase_unreachable_if_present(builder);
    let body_falls_through = builder
        .get_insert_block()
        .unwrap()
        .get_terminator()
        .is_none();
    if body_falls_through {
        let active =
            crate::int_fast::emit_exception_pending(context, builder, rt, "try_body_exc_active");
        let has_exc = builder
            .build_int_compare(
                inkwell::IntPredicate::NE,
                active,
                context.i8_type().const_zero(),
                "try_body_has_exc",
            )
            .expect("build_int_compare should not fail");
        builder
            .build_conditional_branch(has_exc, handler_dispatch_bb, else_bb)
            .expect("build_conditional_branch should not fail");
    }

    // Handler dispatch: check each handler in order.
    builder.position_at_end(handler_dispatch_bb);
    if handlers.is_empty() {
        // No handlers — branch to finally (exception remains active,
        // which will cause the finally to re-check and propagate).
        builder
            .build_unconditional_branch(finally_bb)
            .expect("build_unconditional_branch should not fail");
    } else {
        // Build separate dispatch blocks and handler body blocks.
        // The dispatch chain checks type_matches in order; each
        // dispatch block branches to its handler body (match) or
        // the next dispatch block (no match).
        let exc_val = builder
            .build_call(rt.exception_value, &[], "try_exc_val")
            .expect("build_call should not fail for exception_value")
            .try_as_basic_value()
            .expect_basic("pycc_rt_exception_value returns a pointer")
            .into_pointer_value();
        let mut dispatch_bbs: Vec<inkwell::basic_block::BasicBlock> = Vec::new();
        let mut handler_body_bbs: Vec<inkwell::basic_block::BasicBlock> = Vec::new();
        for i in 0..handlers.len() {
            dispatch_bbs.push(context.append_basic_block(function, &format!("try_dispatch_{i}")));
            handler_body_bbs
                .push(context.append_basic_block(function, &format!("try_handler_{i}")));
        }
        let no_match_bb = context.append_basic_block(function, "try_no_match");

        // Entry into dispatch chain from handler_dispatch_bb.
        builder
            .build_unconditional_branch(dispatch_bbs[0])
            .expect("build_unconditional_branch should not fail");

        // Dispatch chain: for each handler, check type_matches.
        for (i, handler) in handlers.iter().enumerate() {
            let next_bb = if i + 1 < handlers.len() {
                dispatch_bbs[i + 1]
            } else {
                no_match_bb
            };
            builder.position_at_end(dispatch_bbs[i]);
            let matches = if let Some(tags) = handler.exc_type_tag.as_deref() {
                // Part 2 of #541 (D-189): a handler naming a class accepts
                // that class and every user-defined subclass of it, each of
                // which carries its own tag, so the test is an OR over the
                // whole set. A single-tag handler -- every builtin one --
                // emits exactly the call it emitted before, with no `or`.
                let mut accumulated: Option<inkwell::values::IntValue<'ctx>> = None;
                for tag in tags {
                    let tag_val = context.i8_type().const_int(u64::from(*tag), false);
                    let one = builder
                        .build_call(
                            rt.exception_type_matches,
                            &[exc_val.into(), tag_val.into()],
                            "exc_matches",
                        )
                        .expect("build_call should not fail for exception_type_matches")
                        .try_as_basic_value()
                        .expect_basic("pycc_rt_exception_type_matches returns i8")
                        .into_int_value();
                    accumulated = Some(match accumulated {
                        Some(previous) => builder
                            .build_or(previous, one, "exc_matches_any")
                            .expect("build_or should not fail"),
                        None => one,
                    });
                }
                accumulated.expect("pycc_mir never emits an empty handler tag set")
            } else {
                // Bare `except:` — always matches.
                context.i8_type().const_int(1, false)
            };
            let is_match = builder
                .build_int_compare(
                    inkwell::IntPredicate::NE,
                    matches,
                    context.i8_type().const_zero(),
                    "is_match",
                )
                .expect("build_int_compare should not fail");
            builder
                .build_conditional_branch(is_match, handler_body_bbs[i], next_bb)
                .expect("build_conditional_branch should not fail");
        }

        // No handler matched — branch to finally (exception stays
        // active, will propagate after finally).
        builder.position_at_end(no_match_bb);
        builder
            .build_unconditional_branch(finally_bb)
            .expect("build_unconditional_branch should not fail");

        // Emit each handler body. Save the current exception value
        // to a handler-local slot, then clear the exception state so the
        // handler body runs with no active exception. A bare `raise`
        // (Reraise) inside the handler loads the saved value and
        // re-raises it. After the handler body, if it completes
        // normally (no active exception), just branch to finally.
        for (i, handler) in handlers.iter().enumerate() {
            builder.position_at_end(handler_body_bbs[i]);
            // Save the current exception value and clear the state.
            let exc_val = builder
                .build_call(rt.exception_value, &[], "handler_exc_val")
                .expect("build_call should not fail for exception_value")
                .try_as_basic_value()
                .expect_basic("pycc_rt_exception_value returns a pointer")
                .into_pointer_value();
            let saved_exc = alloca_at_entry(
                builder,
                context.ptr_type(inkwell::AddressSpace::default()),
                "handler_saved_exc",
            );
            builder
                .build_store(saved_exc, exc_val)
                .expect("build_store should not fail for a handler exception slot");
            builder
                .build_call(rt.exception_clear, &[], "")
                .expect("build_call should not fail for exception_clear");
            // #382: If the handler has an `as` binding (e.g.
            // `except ValueError as e:`), allocate a local slot
            // for it and store the exception value there so the
            // handler body can reference `e`. The slot is a
            // pointer-typed alloca (all exception objects are
            // pointer-represented, like class instances).
            if let Some(binding_name) = &handler.binding_name {
                let exc_slot = alloca_at_entry(
                    builder,
                    context.ptr_type(inkwell::AddressSpace::default()),
                    binding_name,
                );
                builder
                    .build_store(exc_slot, exc_val)
                    .expect("build_store should not fail for an exception binding");
                locals.insert(
                    binding_name.clone(),
                    StorageSlot {
                        ptr: exc_slot,
                        ty: handler
                            .binding_ty
                            .clone()
                            .expect("an exception binding always carries its static type"),
                        initialized: None,
                    },
                );
            }
            rt.exceptions.reraise_values.borrow_mut().push(saved_exc);
            rt.exceptions.targets.borrow_mut().push(finally_bb);
            emit_body(
                context,
                builder,
                module,
                rt,
                user_functions,
                locals,
                &handler.body,
                expected_return_ty.clone(),
                finally_stack,
            )?;
            rt.exceptions.targets.borrow_mut().pop();
            rt.exceptions.reraise_values.borrow_mut().pop();
            // #382: A `raise`/`reraise` inside the handler body
            // terminates the block with `unreachable`. Erase it so
            // the handler-completion check can run here.
            erase_unreachable_if_present(builder);
            let handler_falls_through = builder
                .get_insert_block()
                .unwrap()
                .get_terminator()
                .is_none();
            if handler_falls_through {
                // Handler completed normally — exception was already
                // cleared before the handler body. Just branch to
                // finally.
                builder
                    .build_unconditional_branch(finally_bb)
                    .expect("build_unconditional_branch should not fail");
            }
        }
    }

    // Else body: runs only if no exception was raised.
    builder.position_at_end(else_bb);
    if orelse.is_empty() {
        builder
            .build_unconditional_branch(finally_bb)
            .expect("build_unconditional_branch should not fail");
    } else {
        rt.exceptions.targets.borrow_mut().push(finally_bb);
        emit_body(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            orelse,
            expected_return_ty.clone(),
            finally_stack,
        )?;
        rt.exceptions.targets.borrow_mut().pop();
        // #382: A `raise` inside the else body terminates the
        // block with `unreachable`. Erase it so the
        // else-completion check can run here.
        erase_unreachable_if_present(builder);
        let else_falls_through = builder
            .get_insert_block()
            .unwrap()
            .get_terminator()
            .is_none();
        if else_falls_through {
            builder
                .build_unconditional_branch(finally_bb)
                .expect("build_unconditional_branch should not fail");
        }
    }

    // Pop the FinallyTarget before emitting the finally body —
    // a `return` inside the finally body should NOT be intercepted
    // by this same finally (it would loop).
    if has_finally {
        finally_stack.pop();
    }

    // Finally body: always runs. After finally, check if a return
    // was intercepted (is_returning flag) or branch to after_bb
    // for normal completion. The structured-statement guard at
    // after_bb propagates any pending exception.
    builder.position_at_end(finally_bb);
    let finally_exception_bb = context.append_basic_block(function, "try_finally_exception");
    let pending_exception = if finalbody.is_empty() {
        None
    } else {
        // A pending exception must not prevent `finally` from
        // executing. Preserve it in SSA values and clear the
        // pending state while the final body runs. Normal
        // completion restores it; a new exception or a return from
        // `finally` deliberately replaces/suppresses it.
        let active =
            crate::int_fast::emit_exception_pending(context, builder, rt, "finally_pending_active");
        let value = builder
            .build_call(rt.exception_value, &[], "finally_pending_value")
            .expect("build_call should not fail for exception_value")
            .try_as_basic_value()
            .expect_basic("pycc_rt_exception_value returns a pointer")
            .into_pointer_value();
        builder
            .build_call(rt.exception_clear, &[], "")
            .expect("build_call should not fail for exception_clear");
        Some((active, value))
    };
    if finalbody.is_empty() {
        // No finally body — just check is_returning / branch.
    } else {
        rt.exceptions
            .targets
            .borrow_mut()
            .push(finally_exception_bb);
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
    }
    // #382: A `raise` inside the finally body terminates the
    // block with `unreachable`. Erase it so the
    // finally-completion check can run here.
    erase_unreachable_if_present(builder);
    let finally_falls_through = builder
        .get_insert_block()
        .unwrap()
        .get_terminator()
        .is_none();
    if finally_falls_through {
        if let Some((pending_active, pending_value)) = pending_exception {
            let had_pending = builder
                .build_int_compare(
                    inkwell::IntPredicate::NE,
                    pending_active,
                    context.i8_type().const_zero(),
                    "finally_had_pending",
                )
                .expect("build_int_compare should not fail");
            let restore_bb = context.append_basic_block(function, "try_finally_restore_exception");
            let restored_bb =
                context.append_basic_block(function, "try_finally_exception_restored");
            builder
                .build_conditional_branch(had_pending, restore_bb, restored_bb)
                .expect("build_conditional_branch should not fail");
            builder.position_at_end(restore_bb);
            builder
                .build_call(rt.exception_raise, &[pending_value.into()], "")
                .expect("build_call should not fail for exception_raise");
            builder
                .build_unconditional_branch(restored_bb)
                .expect("build_unconditional_branch should not fail");
            builder.position_at_end(restored_bb);
        }
        if has_finally {
            // Check if a return was intercepted: if so, load the
            // return value and emit `ret` (or propagate to an
            // enclosing finally). Otherwise, branch to after_bb.
            let is_returning = saved_is_returning.unwrap();
            let ret_slot = saved_ret_slot;
            let flag_val = builder
                .build_load(context.i8_type(), is_returning, "finally_ret_flag")
                .expect("build_load should not fail for is_returning flag")
                .into_int_value();
            let is_ret = builder
                .build_int_compare(
                    inkwell::IntPredicate::NE,
                    flag_val,
                    context.i8_type().const_zero(),
                    "finally_is_ret",
                )
                .expect("build_int_compare should not fail");
            let ret_bb = context.append_basic_block(function, "try_finally_ret");
            builder
                .build_conditional_branch(is_ret, ret_bb, after_bb)
                .expect("build_conditional_branch should not fail");
            // Return block: load the saved return value and emit
            // `ret`, or propagate to an enclosing finally.
            builder.position_at_end(ret_bb);
            if let Some(slot) = ret_slot {
                let ret_val = builder
                    .build_load(
                        ty_to_basic_type(context, expected_return_ty.clone()),
                        slot,
                        "finally_ret_val",
                    )
                    .expect("build_load should not fail for ret_slot");
                if let Some(outer) = finally_stack.last_mut() {
                    // Propagate to the enclosing finally: store the
                    // return value to the outer's ret_slot, set its
                    // is_returning flag, and branch to its finally_bb.
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
                } else {
                    builder
                        .build_return(Some(&ret_val))
                        .expect("build_return should not fail for a finally-routed return");
                }
            } else {
                // Void function (Ty::None) or top-level code (main
                // returns i64). `ret_slot` is None because
                // `expected_return_ty` is `Ty::None`. For a void
                // function, emit `ret void`. For top-level `main`
                // (which returns i64), this block is dead code (no
                // `return` is allowed at top level), so emit
                // `unreachable` to satisfy the verifier.
                if let Some(outer) = finally_stack.last_mut() {
                    builder
                        .build_store(outer.is_returning, context.i8_type().const_int(1, false))
                        .expect("build_store should not fail for outer is_returning");
                    builder
                        .build_unconditional_branch(outer.finally_bb)
                        .expect("build_unconditional_branch should not fail");
                } else if function.get_type().get_return_type().is_none() {
                    // LLVM void function — emit `ret void`.
                    builder
                        .build_return(None)
                        .expect("build_return should not fail for a finally-routed void return");
                } else {
                    // Non-void LLVM function (e.g. `main` returns
                    // i64) with no `ret_slot` — dead code (no
                    // `return` was intercepted). Emit
                    // `unreachable` to satisfy the verifier.
                    builder
                        .build_unreachable()
                        .expect("build_unreachable should not fail for dead ret_bb");
                }
            }
        } else {
            // No finally body — branch to after_bb unconditionally.
            // The structured-statement guard at after_bb propagates any
            // pending exception before the enclosing suite can continue.
            builder
                .build_unconditional_branch(after_bb)
                .expect("build_unconditional_branch should not fail");
        }
    }

    // An exception raised by `finally` overrides an intercepted
    // return. It keeps the runtime exception active while skipping
    // the return-flag dispatch.
    builder.position_at_end(finally_exception_bb);
    builder
        .build_unconditional_branch(after_bb)
        .expect("build_unconditional_branch should propagate a finally exception");

    builder.position_at_end(after_bb);
    // A locally unmatched exception (or one restored after `finally`) must
    // leave this structured statement immediately. Expression nodes already
    // guard their own raising operations; this is the corresponding boundary
    // for a complete try statement.
    guard_statement_effects(context, builder, rt);
    Ok(())
}
