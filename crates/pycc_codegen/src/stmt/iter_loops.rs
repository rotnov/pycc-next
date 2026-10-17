//! `for` loops over lists, dicts, sets and CPython objects.

use super::*;

// An intentional inline duplicate of `ForRange`'s loop-building
// logic directly above -- exactly as that arm is itself a
// deliberate inline copy of `emit_body_then_branch`'s (see both
// their own comments). Not factored into a shared helper: the two
// arms differ in their bound, their induction step, and their
// per-iteration prologue, and this file's established position is
// that a third consumer, not a second, is what justifies extracting
// shared loop-building machinery.
//
// Three real differences from `ForRange`, all of them consequences
// of iterating a container rather than an arithmetic range:
//
// 1. The induction variable is a plain, raw LLVM `i64` index, never
//    a D-061-tagged `Ty::Int` -- so the increment is
//    `build_int_add`, not `pycc_rt_int_add`, and the loop test is a
//    plain `icmp slt`, not `pycc_rt_range_continue`. Both of those
//    runtime functions take *tagged* operands (D-061); calling
//    either with this raw counter would silently compute on the
//    wrong values.
// 2. The bound is `pycc_rt_list_len`, re-read on every
//    iteration inside the test block rather than hoisted into the
//    preheader. This matches CPython, whose list iterator compares
//    its cursor against the list's *current* length on every
//    `__next__` -- so `for v in xs: xs.append(v)` runs forever
//    there, and would terminate here if the length were hoisted.
// 3. Each iteration prepends one `pycc_rt_list_get` read.
//
// D-141 leaves the induction variable and raw `len` as private LLVM
// counters. The element read is already encoded and is assigned to
// the user-visible `Ty::Int` loop target unchanged.
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_for_list_stmt<'ctx>(
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
    let MirStmt::ForList {
        var,
        var_ty,
        list,
        body,
    } = stmt
    else {
        unreachable!()
    };
    let function = builder.get_insert_block().unwrap().get_parent().unwrap();
    // Read once, in the preheader, not per iteration: Python binds
    // its iterator to the object the `for` statement evaluated, so
    // a body-level rebinding of the same name (`xs = [9]`) must not
    // retarget the loop.
    //
    // The loop holds `list_ptr` across arbitrary body code. A local
    // the body never rebinds keeps the list alive itself; otherwise
    // (`xs = [9]` in the body, or a module global any call may
    // rebind) the loop takes its own reference, released when the
    // loop ends. In a function that reference lives in a hidden
    // owned slot (`loop_list_slots`), so a `return` or an exception
    // out of the loop releases it through the frame's epilogue.
    let list_ptr = emit_list_name_read(context, builder, module, rt, user_functions, locals, list);
    let hold = LoopHold::enter(
        context,
        builder,
        rt,
        locals,
        stmt,
        body,
        list,
        heap_rc::HeapKind::List,
        list_ptr,
    );
    let preheader = builder.get_insert_block().unwrap();

    let test_bb = context.append_basic_block(function, "for_list_test");
    let body_bb = context.append_basic_block(function, "for_list_body");
    let after_bb = context.append_basic_block(function, "for_list_after");

    builder
        .build_unconditional_branch(test_bb)
        .expect("build_unconditional_branch should not fail entering the loop test");
    builder.position_at_end(test_bb);
    let induction = builder
        .build_phi(context.i64_type(), "for_list_index")
        .expect("build_phi should not fail in a fresh loop-test block");
    let zero = context.i64_type().const_zero();
    induction.add_incoming(&[(&zero, preheader)]);
    let current = induction.as_basic_value().into_int_value();
    let len = list::build_list_len(context, builder, list_ptr);
    let cont = builder
        .build_int_compare(IntPredicate::SLT, current, len, "for_list_cont")
        .expect("build_int_compare should not fail comparing two i64 operands");
    builder
        .build_conditional_branch(cont, body_bb, after_bb)
        .expect("build_conditional_branch should not fail for a well-formed i1 condition");

    builder.position_at_end(body_bb);
    // Same Python target-binding semantics `ForRange`'s own
    // comment describes: the visible target is a separate storage
    // slot written once per iteration, so the final target keeps
    // the last element and body reassignment cannot corrupt the
    // next iteration (both pinned by
    // `list_targets_keep_the_last_element_and_ignore_body_reassignment`
    // in `tests/slice1_codegen_depth.rs`). `ForRange`'s third
    // property -- an empty sequence leaving the target unbound --
    // holds here structurally for the same reason, and since #1021
    // (D-245) it *is* reachable from real source: `xs: list[int] =
    // []` now resolves its element type from the annotation instead
    // of failing, so `for x in xs` over an empty list compiles and
    // runs zero iterations (pinned by
    // `iterating_an_inferred_empty_list_runs_zero_iterations` in
    // `tests/issue_1021_empty_container_inference.rs`).
    let word = list::build_list_get_unchecked(context, builder, var_ty, list_ptr, current);
    let element = list::word_element(context, builder, rt, var_ty, word);
    // The element is borrowed from the list; the loop variable owns
    // its value like any other slot, unless it only ever borrows.
    if !rt.borrowed_locals.borrow().contains(var) {
        heap_rc::retain_scalar(context, builder, rt, var_ty, &element);
        if let Some(slot) = locals.get(var) {
            heap_rc::release_slot(context, builder, rt, &slot.ty, slot.ptr);
        }
    }
    emit_assign(context, builder, rt, locals, var, element);
    // `ForRange`'s latch shape: fall-through and `continue` advance
    // the index here, `break` leaves through `after_bb`.
    let latch_bb = context.append_basic_block(function, "for_list_latch");
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
        expected_return_ty,
        finally_stack,
    )?;
    rt.loops.exit();
    // `ForRange`'s own terminator-safety guard, for the identical
    // reason (see that arm's comment).
    if loop_control::position_at_latch_if_used(builder, latch_bb) {
        let next = builder
            .build_int_add(
                current,
                context.i64_type().const_int(1, false),
                "for_list_next",
            )
            .expect("build_int_add should not fail for two i64 operands");
        let body_end = builder.get_insert_block().unwrap();
        induction.add_incoming(&[(&next, body_end)]);
        builder
            .build_unconditional_branch(test_bb)
            .expect("build_unconditional_branch should not fail on a block with no terminator yet");
    }

    builder.position_at_end(after_bb);
    hold.exit(context, builder, rt);
    Ok(())
}

// PR 3c of #1082: `for x in o.attr:` / `for x in o.method(...):`.
// `foreign_call::emit_iter_loop` builds the preheader and header --
// the `pycc_ext_obj_get_iter` call with its NULL failure edge, and
// the `pycc_ext_obj_iter_next` call with the three-way switch whose
// default is a second failure edge -- and leaves the builder in the
// body block with this iteration's item loaded. Only the target
// store, the body and the back-edge are emitted here.
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_for_object_stmt<'ctx>(
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
    let MirStmt::ForObject { var, iter, body } = stmt else {
        unreachable!()
    };
    let iterable = emit_expr(context, builder, module, rt, user_functions, locals, iter);
    let held = object_release::hold(context, module, rt, iter, &iterable);
    let (loop_blocks, iterator) =
        foreign_call::emit_iter_loop(context, builder, module, rt, iterable, held);
    // The item is `pycc_ext_obj_iter_next`'s new reference. The
    // target owns it, so each trip releases the previous item (or
    // the value bound before the loop) and the slot keeps the last
    // one, as in CPython.
    let slot = locals
        .get(var)
        .expect("every for-target must have a predeclared storage slot");
    object_slot::store_new_reference(context, builder, rt, slot, loop_blocks.item);
    // A `continue` asks the iterator for the next item; a `break`
    // leaves through `after_bb`, which releases the iterator below
    // exactly as the normal exit does.
    rt.loops.enter(
        stmt,
        loop_blocks.after_bb,
        loop_blocks.header_bb,
        finally_stack.len(),
        crate::exception::owned_slots::depth(rt),
    );
    // `emit_body_then_branch` carries `ForList`'s own
    // terminator-safety guard: a `Return` inside `body` already
    // terminated the block and a second terminator would be
    // invalid IR.
    emit_body_then_branch(
        context,
        builder,
        module,
        rt,
        user_functions,
        locals,
        body,
        loop_blocks.header_bb,
        expected_return_ty,
        finally_stack,
    )?;
    rt.loops.exit();
    builder.position_at_end(loop_blocks.after_bb);
    // Part 3 of #1092: the normal exit releases the iterator; every
    // exception edge already did through its cleanup target.
    iterator.exit(builder, rt);
    Ok(())
}

// `for k in d:` (PR-11 Task 5, D-123): an intentional inline
// duplicate of `MirStmt::ForList`'s own loop-building logic
// directly above, for the identical "a third consumer, not a
// second, justifies a shared helper" reason that arm's own comment
// already gives for its own `ForRange` duplication.
//
// Three differences from `ForList`, all consequences of iterating
// a dict's keys rather than a list's elements:
// 1. The bound is `pycc_rt_dict_len`, still re-read on every
//    iteration inside the test block rather than hoisted into the
//    preheader, for the identical CPython-mutation-during-iteration
//    reason `ForList`'s own comment gives: this compiler has no
//    `del`/removal for a dict yet, but `d[k] = v` inside the loop
//    body can still grow it, and a hoisted bound would silently
//    stop tracking that.
// 2. Each iteration reads the current *key*, via
//    `pycc_rt_dict_key_at`, not an element via
//    `pycc_rt_list_get`.
// 3. The loop variable is bound `Scalar::Str`, not `Scalar::Int` --
//    and, unlike every `ForList`/`ForRange` induction target (never
//    refcounted, D-061), a dict key genuinely is a refcounted
//    `PyStrObj` that `d` itself still holds a live, non-incref'd
//    pointer to (D-124: "`PyDictObj`'s own keys ... are stored
//    without incref on insert"). Without the `pycc_rt_str_incref`
//    below, an ordinary body-level reassignment of the loop
//    variable (`for k in d:\n    k = "z"\n`) would go through
//    `emit_stmt`'s own `Assign` arm, whose
//    `decref_str_slot_before_store` call fires unconditionally for
//    any `Ty::Str` target -- decref'ing `d`'s *own* only reference
//    to that key and freeing it while `d` still holds the
//    now-dangling pointer, a real premature free this project's own
//    leak-only containers are never supposed to cause (see
//    `Scalar::List`'s and `Scalar::Dict`'s own doc comments: "leak-
//    only -- never a premature free"). The incref gives the loop
//    variable's own slot a reference distinct from `d`'s, so that
//    decref (or the next iteration's own overwrite, which bypasses
//    it entirely -- see `emit_assign` below) only ever brings the
//    *duplicate* back down, never below `d`'s own copy. The
//    resulting extra, never-brought-back-down increment on the
//    loop's last key is exactly D-124's existing leak-only policy
//    playing out one level lower: an unbalanced incref only makes
//    an already-permanent leak larger, never a premature free.
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_for_dict_stmt<'ctx>(
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
    let MirStmt::ForDict {
        var, dict, body, ..
    } = stmt
    else {
        unreachable!()
    };
    let function = builder.get_insert_block().unwrap().get_parent().unwrap();
    // Read once, in the preheader, not per iteration -- identical
    // reasoning to `ForList`'s own preheader read (see that arm's
    // comment): Python binds its iterator to the object the `for`
    // statement evaluated, so a body-level rebinding of `dict`'s
    // own name must not retarget the loop, and a body that could
    // drop the dict's other owner makes the loop hold its own
    // reference (`LoopHold`), as `ForList` does.
    let dict_ptr = emit_dict_name_read(context, builder, module, rt, user_functions, locals, dict);
    let hold = LoopHold::enter(
        context,
        builder,
        rt,
        locals,
        stmt,
        body,
        dict,
        heap_rc::HeapKind::Dict,
        dict_ptr,
    );
    // CPython's dict iterator checks the size it started with at every
    // step: a body that inserts or deletes raises `RuntimeError` at the
    // next step (`ForSet`'s check).
    let initial_len = build_dict_len(builder, rt, dict_ptr);
    let preheader = builder.get_insert_block().unwrap();

    let test_bb = context.append_basic_block(function, "for_dict_test");
    let body_bb = context.append_basic_block(function, "for_dict_body");
    let after_bb = context.append_basic_block(function, "for_dict_after");

    builder
        .build_unconditional_branch(test_bb)
        .expect("build_unconditional_branch should not fail entering the loop test");
    builder.position_at_end(test_bb);
    let induction = builder
        .build_phi(context.i64_type(), "for_dict_index")
        .expect("build_phi should not fail in a fresh loop-test block");
    let zero = context.i64_type().const_zero();
    induction.add_incoming(&[(&zero, preheader)]);
    let current = induction.as_basic_value().into_int_value();
    let len = build_dict_len(builder, rt, dict_ptr);
    builder
        .build_call(
            rt.dict_check_not_resized,
            &[len.into(), initial_len.into()],
            "dict_check_not_resized",
        )
        .expect("build_call should not fail for a well-formed dict resize check");
    let in_range = builder
        .build_int_compare(IntPredicate::SLT, current, len, "for_dict_in_range")
        .expect("build_int_compare should not fail comparing two i64 operands");
    // `ForSet`'s pending-exception conjunct: the check (or the body)
    // raised, so the loop ends here.
    let exc_active =
        crate::int_fast::emit_exception_pending(context, builder, rt, "for_dict_exc_active");
    let no_exception = builder
        .build_int_compare(
            IntPredicate::EQ,
            exc_active,
            exc_active.get_type().const_zero(),
            "for_dict_no_exc",
        )
        .expect("build_int_compare should not fail comparing an i8 against zero");
    let cont = builder
        .build_and(in_range, no_exception, "for_dict_cont")
        .expect("build_and should not fail for two i1 operands");
    builder
        .build_conditional_branch(cont, body_bb, after_bb)
        .expect("build_conditional_branch should not fail for a well-formed i1 condition");

    builder.position_at_end(body_bb);
    let key_word = builder
        .build_call(
            rt.dict_key_at,
            &[dict_ptr.into(), current.into()],
            "dict_key_at",
        )
        .expect("build_call should not fail for a well-formed dict key read")
        .try_as_basic_value()
        .expect_basic("pycc_rt_dict_key_at returns a non-void i64")
        .into_int_value();
    // The key is borrowed from the dict: the loop variable takes
    // its own reference and releases the one it held, as
    // `ForList`'s does.
    let (key_ty, _) = containers::dict_kv(&locals[dict].ty);
    let key = list::word_element(context, builder, rt, &key_ty, key_word);
    heap_rc::retain_scalar(context, builder, rt, &key_ty, &key);
    if let Some(slot) = locals.get(var) {
        heap_rc::release_slot(context, builder, rt, &slot.ty, slot.ptr);
    }
    emit_assign(context, builder, rt, locals, var, key);
    // `ForList`'s latch shape (see that arm).
    let latch_bb = context.append_basic_block(function, "for_dict_latch");
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
        expected_return_ty,
        finally_stack,
    )?;
    rt.loops.exit();
    // `ForList`'s own terminator-safety guard, for the identical
    // reason (see that arm's comment).
    if loop_control::position_at_latch_if_used(builder, latch_bb) {
        let next = builder
            .build_int_add(
                current,
                context.i64_type().const_int(1, false),
                "for_dict_next",
            )
            .expect("build_int_add should not fail for two i64 operands");
        let body_end = builder.get_insert_block().unwrap();
        induction.add_incoming(&[(&next, body_end)]);
        builder
            .build_unconditional_branch(test_bb)
            .expect("build_unconditional_branch should not fail on a block with no terminator yet");
    }

    builder.position_at_end(after_bb);
    hold.exit(context, builder, rt);
    Ok(())
}

// `for x in s:` (PR-11 Task 9, D-123): an intentional inline
// duplicate of `MirStmt::ForList`'s own loop-building logic above,
// for the identical "a third consumer, not a second, justifies a
// shared helper" reason that arm's own comment already gives for
// its own `ForRange` duplication (and `MirStmt::ForDict`'s own
// comment gives for its `ForList` duplication).
//
// Structurally this is closer to `ForList` than to `ForDict`: a
// set's elements, like a list's, are encoded `i64` values with no
// refcounting concern (unlike `ForDict`'s own key, which is a
// refcounted `PyStrObj` needing its own `pycc_rt_str_incref` before
// the loop variable's per-iteration bind -- see that arm's own doc
// comment). Two differences from `ForList`, both consequences of
// iterating a set rather than a list:
// 1. The bound is `pycc_rt_int_set_len`, still re-read on every
//    iteration inside the test block rather than hoisted into the
//    preheader, for the identical CPython-mutation-during-iteration
//    reason `ForList`'s own comment gives -- kept identical to
//    `ForList`/`ForDict`'s own shape. Like `ForDict`'s, a mid-loop
//    `set.add()` (D-119) is
//    checked against the length captured once in the preheader and
//    raises `RuntimeError` (D-173) on any change -- see
//    `pycc_rt_int_set_check_not_resized`'s own doc comment for why
//    silently extending the iteration is not safe to accept here:
//    `for x in s: s.add(x + 1)` would never terminate.
// 2. Each iteration reads the current element via
//    `pycc_rt_int_set_get`, not `pycc_rt_list_get`.
//
// D-141 mirrors `ForList`: the induction variable and `len` are raw
// private counters, while the element is an encoded user value.
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_for_set_stmt<'ctx>(
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
    let MirStmt::ForSet {
        var,
        var_ty,
        set,
        body,
    } = stmt
    else {
        unreachable!()
    };
    let function = builder.get_insert_block().unwrap().get_parent().unwrap();
    // Read once, in the preheader, not per iteration -- identical
    // reasoning to `ForList`'s own preheader read (see that arm's
    // comment): Python binds its iterator to the object the `for`
    // statement evaluated, so a body-level rebinding of `set`'s own
    // name must not retarget the loop, and a body that could drop
    // the set's other owner makes the loop hold its own reference
    // (`LoopHold`), as `ForList` does.
    let set_ptr = emit_set_name_read(context, builder, module, rt, user_functions, locals, set);
    let hold = LoopHold::enter(
        context,
        builder,
        rt,
        locals,
        stmt,
        body,
        set,
        heap_rc::HeapKind::Set,
        set_ptr,
    );
    // Captured once, in the preheader -- P1 review fix (PR-12,
    // `pycc_rt_int_set_check_not_resized`'s own doc comment):
    // `set.add()` existing in this same commit made it possible for
    // the loop body to grow `set_ptr` out from under this loop's own
    // `len` re-read below. Comparing every iteration's fresh read
    // against this snapshot, rather than silently visiting whatever
    // `len` grows to, matches CPython's own `RuntimeError` on
    // set-changed-size-during-iteration.
    let initial_len = build_int_set_len(builder, rt, set_ptr);
    let preheader = builder.get_insert_block().unwrap();

    let test_bb = context.append_basic_block(function, "for_set_test");
    let body_bb = context.append_basic_block(function, "for_set_body");
    let after_bb = context.append_basic_block(function, "for_set_after");

    builder
        .build_unconditional_branch(test_bb)
        .expect("build_unconditional_branch should not fail entering the loop test");
    builder.position_at_end(test_bb);
    let induction = builder
        .build_phi(context.i64_type(), "for_set_index")
        .expect("build_phi should not fail in a fresh loop-test block");
    let zero = context.i64_type().const_zero();
    induction.add_incoming(&[(&zero, preheader)]);
    let current = induction.as_basic_value().into_int_value();
    let len = build_int_set_len(builder, rt, set_ptr);
    build_int_set_check_not_resized(builder, rt, len, initial_len);
    let in_range = builder
        .build_int_compare(IntPredicate::SLT, current, len, "for_set_in_range")
        .expect("build_int_compare should not fail comparing two i64 operands");
    // Part B of #1038 (#1064): `pycc_rt_int_set_check_not_resized`
    // now *raises* `RuntimeError: Set changed size during iteration`
    // (D-173) instead of aborting the process, and a D-173 raise
    // returns normally. Without this conjunct the loop-continue
    // condition would still be `current < len` -- and `len` is
    // exactly the value the body just grew -- so a `for x in s:
    // s.add(...)` loop would spin forever rather than reporting the
    // error. Reading the pending flag here, rather than returning a
    // status from the check function, keeps the runtime ABI
    // unchanged and is strictly more correct under D-173: the loop
    // also stops when anything in the *body* raised.
    //
    // What this cannot do is cut a loop short after a raise that was
    // already *handled*: `exception.rs` clears the pending state
    // before an `except` handler body runs and before a `finally`
    // body runs (restoring it afterwards), so a `for` loop in either
    // position iterates in full. What it can do -- deliberately -- is
    // exit at zero iterations when an *unhandled* raise is still
    // pending from an earlier statement that was not itself a
    // checkpoint, since `expression_can_set_exception` does not
    // classify every call as one. That is the same
    // raise-observed-at-the-next-checkpoint residual D-244's
    // 2026-09-13 amendments record, and exiting is the correct
    // response to it: a real exception is pending and is about to be
    // reported, so running the loop body would be the bug.
    //
    // Note the emission order: the resize check above runs *before*
    // this read, yet cannot relabel a body's own exception, because
    // `check_set_len_unchanged` returns early when
    // `pycc_rt_exception_active()` is already non-zero (review round
    // 2 of #1064). Suppressing inside the runtime rather than
    // branching around the call here keeps this block a single
    // straight-line test with no extra basic block, and the two
    // placements are observationally identical: the conjunct below
    // terminates the loop on this same iteration either way.
    let exc_active =
        crate::int_fast::emit_exception_pending(context, builder, rt, "for_set_exc_active");
    let no_exception = builder
        .build_int_compare(
            IntPredicate::EQ,
            exc_active,
            exc_active.get_type().const_zero(),
            "for_set_no_exc",
        )
        .expect("build_int_compare should not fail comparing an i8 against zero");
    let cont = builder
        .build_and(in_range, no_exception, "for_set_cont")
        .expect("build_and should not fail for two i1 operands");
    builder
        .build_conditional_branch(cont, body_bb, after_bb)
        .expect("build_conditional_branch should not fail for a well-formed i1 condition");

    builder.position_at_end(body_bb);
    // Same Python target-binding semantics `ForList`'s own comment
    // describes: a separate storage slot written once per
    // iteration.
    let encoded_element = build_int_set_get(builder, rt, set_ptr, current);
    let element = set_element_scalar(context, builder, rt, var_ty, encoded_element);
    // A `set[C]` element is borrowed: the loop variable takes its own
    // reference, as `ForList`'s does.
    heap_rc::retain_scalar(context, builder, rt, var_ty, &element);
    if let Some(slot) = locals.get(var) {
        heap_rc::release_slot(context, builder, rt, &slot.ty, slot.ptr);
    }
    emit_assign(context, builder, rt, locals, var, element);
    // `ForList`'s latch shape (see that arm).
    let latch_bb = context.append_basic_block(function, "for_set_latch");
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
        expected_return_ty,
        finally_stack,
    )?;
    rt.loops.exit();
    // `ForList`'s own terminator-safety guard, for the identical
    // reason (see that arm's comment).
    if loop_control::position_at_latch_if_used(builder, latch_bb) {
        let next = builder
            .build_int_add(
                current,
                context.i64_type().const_int(1, false),
                "for_set_next",
            )
            .expect("build_int_add should not fail for two i64 operands");
        let body_end = builder.get_insert_block().unwrap();
        induction.add_incoming(&[(&next, body_end)]);
        builder
            .build_unconditional_branch(test_bb)
            .expect("build_unconditional_branch should not fail on a block with no terminator yet");
    }

    builder.position_at_end(after_bb);
    hold.exit(context, builder, rt);
    // Review round 3 of #1064. `after_bb` is reached by two edges
    // now: the ordinary "the loop ran out of elements" one, and the
    // exception conjunct's own early exit above. Falling through
    // without a checkpoint would let the statement *after* the loop
    // run with a real exception already pending -- measured: a
    // `for x in s: s.add(...)` loop followed by `print("BAD")`
    // inside a function printed `BAD` before the `RuntimeError`
    // reached its handler. That is not the
    // raise-observed-at-the-next-checkpoint residual D-244's
    // 2026-09-13 amendments accept, which is a property of
    // `expression_can_set_exception`'s classification of *operands*;
    // this is a statement-level edge this same commit introduced, so
    // it is closed here rather than recorded. `guard_statement_effects`
    // is the project's one mechanism for that edge (it also unwinds
    // the D-208 pending-release stack), so the loop's exceptional
    // exit routes through exactly the block every other fallible
    // statement does.
    guard_statement_effects(context, builder, rt);
    Ok(())
}
