//! The exception references a `try` statement holds while one of its
//! clauses runs: an `except` handler's caught exception (kept for a bare
//! `raise`) and its `as` binding, an `except*` statement's unmatched
//! remainder and each clause's matched subgroup, and the exception a
//! `finally` clause set aside.
//!
//! Each lives in an entry-block slot that owns one reference while its
//! clause runs and is null otherwise. The slots form a stack, in the order
//! their clauses are entered, and every way out of a clause releases the
//! slots it entered (each release nulls its slot):
//!
//! * falling through -- the statement's own emitter, via [`release_above`];
//! * an exception edge -- a cleanup block installed as the clause's
//!   exception target ([`push_cleanup_target`]);
//! * `return` -- every slot above the innermost `finally` (whose body may
//!   still read them), then each `finally` the return passes through
//!   releases the slots above the next one (`FinallyTarget::exc_depth`);
//! * `break`/`continue` -- every slot entered since the loop
//!   (`loop_control`).
//!
//! So a loop that catches an exception every iteration holds at most one
//! caught exception at a time, and a function returning from a handler
//! releases what it caught.

use super::*;

/// The number of exception slots currently live.
pub(crate) fn depth(rt: &RtFns<'_>) -> usize {
    rt.exceptions.owned_slots.borrow().len()
}

/// Allocates an exception slot in the entry block.
pub(crate) fn new_slot<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    name: &str,
) -> PointerValue<'ctx> {
    alloca_at_entry(
        builder,
        context.ptr_type(inkwell::AddressSpace::default()),
        name,
    )
}

/// Stores `value`, an owned reference, into `slot` and registers the slot
/// as live.
pub(crate) fn hold<'ctx>(
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    slot: PointerValue<'ctx>,
    value: PointerValue<'ctx>,
) {
    builder
        .build_store(slot, value)
        .expect("build_store should not fail for an exception slot");
    rt.exceptions.owned_slots.borrow_mut().push(slot);
}

/// Unregisters the slots above `depth` once their clause is emitted.
pub(crate) fn pop_to(rt: &RtFns<'_>, depth: usize) {
    rt.exceptions.owned_slots.borrow_mut().truncate(depth);
}

/// Releases the reference `slot` holds and nulls it.
pub(crate) fn release<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    slot: PointerValue<'ctx>,
) {
    let ptr_type = context.ptr_type(inkwell::AddressSpace::default());
    let held = builder
        .build_load(ptr_type, slot, "exc_slot_held")
        .expect("build_load should not fail for an exception slot")
        .into_pointer_value();
    builder
        .build_store(slot, ptr_type.const_null())
        .expect("build_store should not fail for an exception slot");
    builder
        .build_call(rt.exc_decref, &[held.into()], "")
        .expect("build_call should not fail for pycc_rt_exception_decref");
}

/// Releases every live slot above `depth`, innermost first.
pub(crate) fn release_above<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    depth: usize,
) {
    let slots: Vec<PointerValue<'ctx>> = rt.exceptions.owned_slots.borrow()[depth..].to_vec();
    for slot in slots.into_iter().rev() {
        release(context, builder, rt, slot);
    }
}

/// The slot depth a `return` leaves its frame at before it branches: the
/// innermost `finally` it runs first keeps what was live when it was
/// entered.
pub(crate) fn return_depth(finally_stack: &[FinallyTarget<'_>]) -> usize {
    finally_stack.last().map_or(0, |target| target.exc_depth)
}

/// Installs, as the innermost exception target, a block that releases
/// every slot above `depth` and branches to `target`; returns it. The
/// caller pops it from `rt.exceptions.targets` once the clause is emitted
/// and the slots it covers are still registered, then calls
/// [`fill_cleanup_target`].
pub(crate) fn push_cleanup_target<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    name: &str,
) -> inkwell::basic_block::BasicBlock<'ctx> {
    let function = builder.get_insert_block().unwrap().get_parent().unwrap();
    let cleanup = context.append_basic_block(function, name);
    rt.exceptions.targets.borrow_mut().push(cleanup);
    cleanup
}

/// Emits the body of a block [`push_cleanup_target`] made, while the slots
/// it releases are still registered, leaving the builder where it was.
pub(crate) fn fill_cleanup_target<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    cleanup: inkwell::basic_block::BasicBlock<'ctx>,
    depth: usize,
    target: inkwell::basic_block::BasicBlock<'ctx>,
) {
    let resume = builder.get_insert_block();
    builder.position_at_end(cleanup);
    release_above(context, builder, rt, depth);
    builder
        .build_unconditional_branch(target)
        .expect("build_unconditional_branch should not fail for an exception cleanup");
    if let Some(block) = resume {
        builder.position_at_end(block);
    }
}
