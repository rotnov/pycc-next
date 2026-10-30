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
            exc_depth: owned_slots::depth(rt),
            loop_depth: crate::object_release::loop_depth(rt),
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
        let has_exc = crate::int_fast::emit_exception_pending_test(
            context,
            builder,
            rt,
            "try_body_has_exc",
        );
        let branch = builder
            .build_conditional_branch(has_exc, handler_dispatch_bb, else_bb)
            .expect("build_conditional_branch should not fail");
        crate::int_fast::weight_branch(context, branch, false);
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

        for (i, handler) in handlers.iter().enumerate() {
            builder.position_at_end(handler_body_bbs[i]);
            emit_handler(
                context,
                builder,
                module,
                rt,
                user_functions,
                locals,
                handler,
                expected_return_ty.clone(),
                finally_stack,
                finally_bb,
            )?;
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

    // Pop the FinallyTarget before emitting the finally body -- a `return`
    // inside the finally body must not be intercepted by this same finally
    // (it would loop).
    if has_finally {
        finally_stack.pop();
    }
    super::finally::emit_finally_tail(
        context,
        builder,
        module,
        rt,
        user_functions,
        locals,
        finalbody,
        expected_return_ty,
        finally_stack,
        super::finally::FinallyTail {
            finally_bb,
            after_bb,
            is_returning: saved_is_returning,
            ret_slot: saved_ret_slot,
            prefix: "try",
        },
    )
}

/// Emits one `except` handler's body at the builder's position, the
/// handler having matched the pending exception.
///
/// The handler takes the pending exception: its saved slot (what a bare
/// `raise` re-raises) owns the reference, and an `as` binding owns one of
/// its own. Both are exception slots (`owned_slots`): released when the
/// handler falls through to `finally_bb`, by a cleanup block on its
/// exception edge, and by a `return`, `break` or `continue` out of it --
/// so `e` is unbound after the handler, as in CPython.
#[allow(clippy::too_many_arguments)]
fn emit_handler<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &mut HashMap<String, StorageSlot<'ctx>>,
    handler: &pycc_mir::MirExceptHandler,
    expected_return_ty: pycc_mir::Ty,
    finally_stack: &mut Vec<FinallyTarget<'ctx>>,
    finally_bb: inkwell::basic_block::BasicBlock<'ctx>,
) -> Result<(), String> {
    let depth = owned_slots::depth(rt);
    let exc_val = builder
        .build_call(rt.exception_take, &[], "handler_exc_val")
        .expect("build_call should not fail for exception_take")
        .try_as_basic_value()
        .expect_basic("pycc_rt_exception_take returns a pointer")
        .into_pointer_value();
    let saved_exc = owned_slots::new_slot(context, builder, "handler_saved_exc");
    owned_slots::hold(builder, rt, saved_exc, exc_val);
    if let Some(binding_name) = &handler.binding_name {
        let exc_slot = owned_slots::new_slot(context, builder, binding_name);
        builder
            .build_call(rt.exc_incref, &[exc_val.into()], "")
            .expect("build_call should not fail for pycc_rt_exception_incref");
        owned_slots::hold(builder, rt, exc_slot, exc_val);
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
    let cleanup = owned_slots::push_cleanup_target(context, builder, rt, "handler_exc_cleanup");
    emit_body(
        context,
        builder,
        module,
        rt,
        user_functions,
        locals,
        &handler.body,
        expected_return_ty,
        finally_stack,
    )?;
    rt.exceptions.targets.borrow_mut().pop();
    rt.exceptions.reraise_values.borrow_mut().pop();
    owned_slots::fill_cleanup_target(context, builder, rt, cleanup, depth, finally_bb);
    // A `raise`/`reraise` inside the handler body terminates the block with
    // `unreachable`. Erase it so the handler-completion check can run here.
    erase_unreachable_if_present(builder);
    let handler_falls_through = builder
        .get_insert_block()
        .unwrap()
        .get_terminator()
        .is_none();
    if handler_falls_through {
        owned_slots::release_above(context, builder, rt, depth);
        builder
            .build_unconditional_branch(finally_bb)
            .expect("build_unconditional_branch should not fail");
    }
    owned_slots::pop_to(rt, depth);
    Ok(())
}
