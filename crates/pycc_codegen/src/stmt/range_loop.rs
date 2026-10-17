//! `if`, `while`, `for ... in range(...)` and loop-`else` lowering.

use super::*;

/// The `i1` an `if`/`while` branches on: the test's truth. A rich
/// comparison with an object operand answers it in one call
/// (`object_expr::emit_obj_compare_condition`); anything else is evaluated
/// and tested.
fn emit_condition<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    test: &MirExpr,
) -> inkwell::values::IntValue<'ctx> {
    if let Some(cond) = crate::expr::object_expr::emit_obj_compare_condition(
        context,
        builder,
        module,
        rt,
        user_functions,
        locals,
        test,
    ) {
        return cond;
    }
    let scalar = emit_expr(context, builder, module, rt, user_functions, locals, test);
    let held = object_release::hold(context, module, rt, test, &scalar);
    let cond = crate::optional_ptr::truthy(context, builder, module, rt, &test.ty(), scalar)
        .unwrap_or_else(|| truthy(context, builder, module, rt, scalar));
    held.release(builder, rt);
    // #146 Part 2 (D-181): released *after* `truthy`, which
    // reads a bigint operand's limbs -- releasing first could
    // free the very word being tested.
    release_scalar_if_temporary(context, builder, rt, test, &scalar);
    cond
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_if_stmt<'ctx>(
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
    let MirStmt::If { test, body, orelse } = stmt else {
        unreachable!()
    };
    let function = builder.get_insert_block().unwrap().get_parent().unwrap();
    let cond = emit_condition(context, builder, module, rt, user_functions, locals, test);
    let then_bb = context.append_basic_block(function, "if_then");
    let merge_bb = context.append_basic_block(function, "if_merge");
    let else_bb = if orelse.is_empty() {
        merge_bb
    } else {
        context.append_basic_block(function, "if_else")
    };
    builder
        .build_conditional_branch(cond, then_bb, else_bb)
        .expect("build_conditional_branch should not fail for a well-formed i1 condition");

    builder.position_at_end(then_bb);
    let then_falls_through = emit_body_then_branch(
        context,
        builder,
        module,
        rt,
        user_functions,
        locals,
        body,
        merge_bb,
        expected_return_ty.clone(),
        finally_stack,
    )?;

    let else_falls_through = if orelse.is_empty() {
        true
    } else {
        builder.position_at_end(else_bb);
        emit_body_then_branch(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            orelse,
            merge_bb,
            expected_return_ty,
            finally_stack,
        )?
    };

    builder.position_at_end(merge_bb);
    if !then_falls_through && !else_falls_through {
        builder
            .build_unreachable()
            .expect("build_unreachable should terminate a merge with no predecessors");
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_while_stmt<'ctx>(
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
    let MirStmt::While { test, body } = stmt else {
        unreachable!()
    };
    let function = builder.get_insert_block().unwrap().get_parent().unwrap();
    let test_bb = context.append_basic_block(function, "while_test");
    let body_bb = context.append_basic_block(function, "while_body");
    let after_bb = context.append_basic_block(function, "while_after");

    builder
        .build_unconditional_branch(test_bb)
        .expect("build_unconditional_branch should not fail entering the loop test");
    builder.position_at_end(test_bb);
    let cond = emit_condition(context, builder, module, rt, user_functions, locals, test);
    builder
        .build_conditional_branch(cond, body_bb, after_bb)
        .expect("build_conditional_branch should not fail for a well-formed i1 condition");

    builder.position_at_end(body_bb);
    rt.loops.enter(
        stmt,
        after_bb,
        test_bb,
        finally_stack.len(),
        crate::exception::owned_slots::depth(rt),
    );
    emit_body_then_branch(
        context,
        builder,
        module,
        rt,
        user_functions,
        locals,
        body,
        test_bb,
        expected_return_ty,
        finally_stack,
    )?;
    rt.loops.exit();

    builder.position_at_end(after_bb);
    Ok(())
}

// See `loop_control::emit_loop_else`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_loop_else_stmt<'ctx>(
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
    let MirStmt::LoopElse { loop_stmt, orelse } = stmt else {
        unreachable!()
    };
    loop_control::emit_loop_else(
        context,
        builder,
        module,
        rt,
        user_functions,
        locals,
        loop_stmt,
        orelse,
        expected_return_ty,
        finally_stack,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_for_range_stmt<'ctx>(
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
    let MirStmt::ForRange {
        var,
        start,
        stop,
        step,
        body,
    } = stmt
    else {
        unreachable!()
    };
    let function = builder.get_insert_block().unwrap().get_parent().unwrap();
    let (start_v, stop_v, step_v) = emit_range_operands_with_exception_safety(
        context,
        builder,
        module,
        rt,
        user_functions,
        locals,
        start,
        stop,
        step,
    );
    // #146 Part 1, the `ForRange` ownership contract.
    //
    // `range_operand_to_normalized_int` passes a bigint operand
    // through `pycc_rt_range_normalize_operand` *unchanged*, so
    // `start_v`/`stop_v`/`step_v` alias whatever the source
    // expression already owned -- for `for i in range(b, b, b)`,
    // one single heap object under three names. None of the three
    // has been retained by `retain_if_int_duplicate`: that helper
    // is deliberately not applied to `range` operands, and the
    // three retains below are the loop's own.
    //
    // WARNING for a later change: do *not* route `range`'s operands
    // through `retain_if_int_duplicate` as well. `start_v` would
    // then be retained twice while still being released once, by
    // the ordinary `current` machinery -- a permanent leak of every
    // named `range` bound, invisible to every value assertion.
    //
    // `stop_v` and `step_v` are read on every trip through
    // `for_test` and are released once in `for_after`. `start_v` is
    // retained here as `current`'s *first owner*, not as a third
    // independent operand: on the first trip the induction phi's
    // incoming value *is* `start_v`, so the ordinary per-iteration
    // `current` release below (or `for_after`'s final one, for an
    // empty range) is what balances this retain. Adding a third
    // `for_after` release for `start_v` would free a word the
    // source name still holds.
    //
    // The whole invariant, for `n` executed iterations: `n + 1`
    // owned `current` values (`start_v` plus one `pycc_rt_int_add`
    // result per iteration), matched by `n` per-iteration releases
    // plus one `for_after` release. Separately, each of the `n`
    // binds of the visible target retains `current`, matched by the
    // *next* bind's release-before-store inside `emit_assign`, with
    // the final one matched whenever that slot is next overwritten.
    emit_bigint_refcount_call(context, builder, rt, start_v, BigIntRefcount::Retain);
    emit_bigint_refcount_call(context, builder, rt, stop_v, BigIntRefcount::Retain);
    emit_bigint_refcount_call(context, builder, rt, step_v, BigIntRefcount::Retain);
    // #146 Part 2 (D-181): retire the *source expression's* birth
    // reference for any operand that was freshly built rather than
    // borrowed (`range(a + a, ...)`, or a literal outside D-061's
    // tagged range). Safe immediately, and only immediately after
    // the loop's own retains directly above: the loop is already an
    // owner by this point, so the object survives to `for_after`.
    // Deliberately does *not* conditionalize those retains -- D-180
    // warns that making the loop's ownership depend on the operand's
    // shape is what breaks the `n + 1`/`n + 1` release arithmetic.
    release_if_int_temporary(context, builder, rt, start, start_v);
    release_if_int_temporary(context, builder, rt, stop, stop_v);
    release_if_int_temporary(context, builder, rt, step, step_v);
    // Re-read *after* the retains above: each one splits the block,
    // and the phi's incoming edge must name the block that actually
    // branches into `for_test`.
    let preheader = builder.get_insert_block().unwrap();

    let test_bb = context.append_basic_block(function, "for_test");
    let body_bb = context.append_basic_block(function, "for_body");
    let after_bb = context.append_basic_block(function, "for_after");

    // `for_range_fast`: smallint bounds and a literal step take a
    // raw-counter copy of the loop; this generic loop stays for
    // bigint bounds. Both leave through `exit_bb`.
    let small_version = for_range_fast::versionable_step(rt, stmt, step, body).map(|k| {
        let small_bb = context.append_basic_block(function, "small_range_entry");
        let exit_bb = context.append_basic_block(function, "for_exit");
        let small = for_range_fast::bounds_are_small(context, builder, start_v, stop_v);
        int_fast::likely_branch(context, builder, small, small_bb, test_bb);
        (k, small_bb, exit_bb)
    });
    if small_version.is_none() {
        builder
            .build_unconditional_branch(test_bb)
            .expect("build_unconditional_branch should not fail entering the loop test");
    }
    builder.position_at_end(test_bb);
    let induction = builder
        .build_phi(context.i64_type(), "for_current")
        .expect("build_phi should not fail in a fresh loop-test block");
    induction.add_incoming(&[(&start_v, preheader)]);
    let current = induction.as_basic_value().into_int_value();
    let cont_i1 = int_fast::emit_range_continue(context, builder, rt, current, stop_v, step_v);
    builder
        .build_conditional_branch(cont_i1, body_bb, after_bb)
        .expect("build_conditional_branch should not fail for a well-formed i1 condition");

    builder.position_at_end(body_bb);
    // Python binds a `for` target only after an element actually
    // exists. The visible target is separate from the hidden SSA
    // induction value so an empty range leaves it unbound, the final
    // target remains the last element, and body reassignment cannot
    // corrupt the next iteration.
    // Retain before `emit_assign`, which releases the slot's
    // previous word: when the slot already holds this very object
    // (a body that does not reassign the target cannot produce
    // that, but an aliasing source such as `b` can), releasing
    // first would free the word this bind is about to store.
    emit_bigint_refcount_call(context, builder, rt, current, BigIntRefcount::Retain);
    emit_assign(context, builder, rt, locals, var, Scalar::Int(current));
    // The body falls through, and every `continue` jumps, to this
    // latch, which advances `current`. A `break` leaves through
    // `after_bb`, whose release of `current` balances this
    // iteration's owned value exactly as the normal exit's does.
    let latch_bb = context.append_basic_block(function, "for_latch");
    rt.loops.enter(
        stmt,
        after_bb,
        latch_bb,
        finally_stack.len(),
        crate::exception::owned_slots::depth(rt),
    );
    emit_body_then_branch(
        context,
        builder,
        module,
        rt,
        user_functions,
        locals,
        body,
        latch_bb,
        expected_return_ty.clone(),
        finally_stack,
    )?;
    rt.loops.exit();
    // `ForRange`'s own inline copy of `emit_body_then_branch`'s
    // terminator-safety guard (see that function's own doc comment
    // for why): a `Return` reached inside `body` already terminates
    // `body_bb`, and with no `continue` either nothing reaches the
    // latch, so the increment-and-branch-back below is skipped --
    // the latch is deleted rather than left as a dead increment.
    if loop_control::position_at_latch_if_used(builder, latch_bb) {
        let next = int_fast::emit_int_binop(
            context,
            builder,
            rt,
            pycc_mir::BinOpKind::Add,
            current,
            step_v,
            &[],
        );
        // This iteration's `current` is dead once `next` exists.
        // A `Return` inside `body` skips this release (and both
        // `for_after` releases below) and leaks -- an accepted,
        // documented Part 1 concession, not an oversight.
        emit_bigint_refcount_call(context, builder, rt, current, BigIntRefcount::Release);
        // Re-read after the release: it splits the block, and the
        // phi's incoming edge must name the block that branches.
        let body_end = builder.get_insert_block().unwrap();
        induction.add_incoming(&[(&next, body_end)]);
        builder
            .build_unconditional_branch(test_bb)
            .expect("build_unconditional_branch should not fail on a block with no terminator yet");
    }

    builder.position_at_end(after_bb);
    // The `current` that failed `range_continue` was never bound to
    // the visible target and has no per-iteration release; plus the
    // two operand retains from the preheader.
    emit_bigint_refcount_call(context, builder, rt, current, BigIntRefcount::Release);
    emit_bigint_refcount_call(context, builder, rt, stop_v, BigIntRefcount::Release);
    emit_bigint_refcount_call(context, builder, rt, step_v, BigIntRefcount::Release);
    if let Some((k, small_bb, exit_bb)) = small_version {
        builder.build_unconditional_branch(exit_bb).expect("branch");
        // Smallint bounds and step own nothing, so the smallint
        // loop leaves straight to `exit_bb`, with no releases.
        builder.position_at_end(small_bb);
        for_range_fast::emit(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            stmt,
            var,
            start_v,
            stop_v,
            k,
            body,
            exit_bb,
            expected_return_ty,
            finally_stack,
        )?;
        builder.position_at_end(exit_bb);
    }
    Ok(())
}
