//! Pieces of the module entry point's source-order item loop in
//! `compile_to_object_with_observer`: the pending-exception check after
//! each top-level statement, and a `def`'s runtime binding.

use super::*;
use inkwell::basic_block::BasicBlock;

/// #382: after a top-level statement, branches to `top_exception_exit` when
/// an exception is pending.
///
/// A `raise` (or a converted runtime failure like division by zero) sets
/// the pending exception state and terminates the block with
/// `unreachable`. Erase the `unreachable` so the exception check can run
/// here. This check intercepts it at the top level: if an exception is
/// active, print it and exit. Inside a `try`, the try's own post-body check
/// intercepts first. `emit_stmt` rejects a `Return` whose parent is this
/// synthetic `main`, so after erasing an exception's `unreachable` the
/// current block is guaranteed to accept this check.
pub(super) fn emit_exception_check<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    entry_fn: FunctionValue<'ctx>,
    top_exception_exit: BasicBlock<'ctx>,
) {
    erase_unreachable_if_present(builder);
    let active = crate::int_fast::emit_exception_pending(context, builder, rt, "top_exc_active");
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

/// A `def`'s runtime binding at its source position: stores the function
/// pointer of definition `ordinal` (`name`, `f`) -- and of every
/// inherited-method copy bound with it -- into its global slot, then
/// publishes what it made callable (`ext_publish`).
///
/// Monomorphized generic specializations (`0gen_...` names) have no
/// `fn_ptr_global` (they dispatch directly), so no store is made for them.
#[allow(clippy::too_many_arguments)]
pub(super) fn bind_def<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    function_defs_in_order: &[(&str, FunctionValue<'ctx>)],
    copy_slots: &copy_slots::CopySlots,
    publish_plan: &ext_publish::PublishPlan,
    (name, f, ordinal): (&str, FunctionValue<'ctx>, usize),
    (entry_fn, top_exception_exit): (FunctionValue<'ctx>, BasicBlock<'ctx>),
) {
    // #1337 (D-254): an inherited-method copy is bound with the origin
    // `def` it copies, not at its own (appended) position -- see
    // `copy_slots`.
    if copy_slots.is_copy(ordinal) {
        return;
    }
    let bound = std::iter::once((name, f)).chain(
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
    // #1199: publish what this `def` (and its copies) made callable -- see
    // `ext_publish`. A `def` is refused inside any module-level block, so
    // no module-level `try` encloses this point, and a failed publish takes
    // the direct module-exec `-1` edge, as a foreign failure here does
    // (Part 1 of #1096).
    debug_assert_eq!(
        rt.exceptions.targets.borrow().last(),
        Some(&top_exception_exit)
    );
    ext_publish::emit(
        context,
        builder,
        module,
        entry_fn,
        publish_plan.names_at(ordinal),
    );
}

/// Emits the module entry point's exceptional exit: every module-scope
/// exception edge branches to `top_exception_exit`.
pub(super) fn emit_top_exception_exit<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    top_exception_exit: BasicBlock<'ctx>,
    ext: bool,
) {
    let i64_type = context.i64_type();
    // Every exceptional module-scope edge converges here. Keeping one
    // target alive while top-level expressions are emitted lets recursive
    // expression guards stop later operands and effects immediately.
    builder.position_at_end(top_exception_exit);
    if ext {
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
}
