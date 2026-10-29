//! `sorted(xs)`, `xs.sort()`, `min(xs)` and `max(xs)` over class instances
//! (`MirExpr::InstanceOrder`, ROADMAP v0.4 accept 5).
//!
//! The runtime orders the elements (`pycc_rt::list::order`) but cannot call
//! the user comparison, so this module emits the loop driving it:
//!
//! ```text
//! state = pycc_rt_order_begin(list, mode)     ; guarded: min/max of []
//! less = 0
//! loop:
//!   if !pycc_rt_order_step(state, less): goto done
//!   less = callee(left, right)                ; or (right, left) when swapped
//!   on a pending exception: pycc_rt_order_abort(state), unwind
//!   goto loop
//! done:
//!   result = pycc_rt_order_finish(state, list) ; guarded: list modified
//! ```
//!
//! The list operand is held on the pending-release stack across the loop,
//! so an unwinding comparison releases a fresh one. A keyed `min`/`max`
//! pulls its pairs from a generator instead, comparing each key as it is
//! computed (`emit_streamed_extreme`).
//!
//! `element in set` for a `set[C]` (`MirExpr::InstanceSetContains`) is here
//! too: the set insertion's probe without the insertion
//! (`set_instance::SetEmitter::contains`).

use super::*;
use crate::heap_rc;
use crate::int_fast::emit_exception_pending;
use crate::rt_fns::RtFns;
use inkwell::types::BasicMetadataTypeEnum;
use inkwell::values::BasicMetadataValueEnum;

/// The runtime function `name` of type `ret(params)`, declared on first use.
fn runtime<'ctx>(
    context: &'ctx Context,
    module: &inkwell::module::Module<'ctx>,
    name: &str,
    ret: Option<inkwell::types::BasicTypeEnum<'ctx>>,
    params: &[BasicMetadataTypeEnum<'ctx>],
) -> FunctionValue<'ctx> {
    module.get_function(name).unwrap_or_else(|| {
        let fn_type = match ret {
            Some(ret) => ret.fn_type(params, false),
            None => context.void_type().fn_type(params, false),
        };
        module.add_function(name, fn_type, Some(Linkage::External))
    })
}

fn call<'ctx>(
    builder: &inkwell::builder::Builder<'ctx>,
    function: FunctionValue<'ctx>,
    args: &[BasicMetadataValueEnum<'ctx>],
) -> Option<inkwell::values::BasicValueEnum<'ctx>> {
    builder
        .build_call(function, args, "")
        .expect("build_call should not fail for a declared runtime function")
        .try_as_basic_value()
        .basic()
}

/// `min(xs, key=f)` / `max(xs, key=f)` with instance keys over a generator
/// of `(key, element)` pairs (`pycc_ast::desugar::keys`): each pair is
/// pulled (one key call), then its key compared with the best one so far,
/// as CPython's `min`/`max` do:
///
/// ```text
/// holder = []                                  ; the best pair, then the new one
/// loop:
///   pull a pair or goto done                   ; the key call, an exception unwinds
///                                              ; (a StopIteration as it is)
///   append it to holder
///   if len(holder) == 2:
///     better = callee(new key, best key)       ; or swapped; an exception unwinds
///     pycc_rt_order_stream_settle(holder, better)
///   goto loop
/// done:
///   result = pycc_rt_order_stream_finish(holder) ; guarded: empty
/// ```
///
/// The generator and the holder are held on the pending-release stack
/// across the loop.
fn emit_streamed_extreme<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    order: &pycc_mir::InstanceOrderExpr,
) -> Scalar<'ctx> {
    let i64_type = context.i64_type();
    let i8_type = context.i8_type();
    let ptr_type = context.ptr_type(inkwell::AddressSpace::default());
    let pycc_mir::Ty::Generator(sig) = order.list.ty() else {
        unreachable!("a streamed ordering pulls from a generator")
    };
    let pair_ty = sig.0.clone();
    let pycc_mir::Ty::Tuple(pair_items) = &pair_ty else {
        panic!("pycc_codegen: internal error: a keyed `min`/`max` pulls `(key, element)` pairs")
    };
    let mark = heap_rc::pending_mark(rt);
    let (object, fresh) =
        crate::generator::operand(context, builder, module, rt, user_functions, locals, &order.list);
    let holder = crate::list::build_list_new(context, builder, rt, &pair_ty, 2);
    heap_rc::hold_owned(rt, heap_rc::HeapKind::List, holder);
    let key = runtime(
        context,
        module,
        "pycc_rt_order_stream_key",
        Some(i64_type.into()),
        &[ptr_type.into(), i64_type.into()],
    );
    let settle = runtime(
        context,
        module,
        "pycc_rt_order_stream_settle",
        None,
        &[ptr_type.into(), i8_type.into()],
    );
    let finish = runtime(
        context,
        module,
        "pycc_rt_order_stream_finish",
        Some(i64_type.into()),
        &[ptr_type.into(), i64_type.into(), i8_type.into()],
    );

    let function = builder
        .get_insert_block()
        .and_then(|block| block.get_parent())
        .expect("an ordering is emitted inside a function");
    let test_bb = context.append_basic_block(function, "stream_test");
    let got_bb = context.append_basic_block(function, "stream_got");
    let compare_bb = context.append_basic_block(function, "stream_compare");
    let raise_bb = context.append_basic_block(function, "stream_raise");
    let settle_bb = context.append_basic_block(function, "stream_settle");
    let done_bb = context.append_basic_block(function, "stream_done");
    builder
        .build_unconditional_branch(test_bb)
        .expect("build_unconditional_branch should not fail");

    builder.position_at_end(test_bb);
    // The pairs' generator runs the key calls; a `StopIteration` one raises
    // propagates as it is (`pycc_rt_gen_resume_raw`), as from CPython's
    // `min`/`max`.
    crate::generator::resume_branch_with(
        context,
        builder,
        rt,
        rt.gens.resume_raw,
        object,
        got_bb,
        done_bb,
    );

    builder.position_at_end(got_bb);
    let pair = crate::generator::load_value(context, builder, object, &pair_ty);
    // The holder adopts the reference the generator handed over (a tuple
    // held by value is boxed, as `list(gen)` stores it).
    let word = match pair {
        Scalar::Tuple(tuple) => {
            let boxed = crate::containers::box_struct_tuple(context, builder, rt, pair_items, tuple);
            builder
                .build_ptr_to_int(boxed, i64_type, "stream_pair_word")
                .expect("build_ptr_to_int should not fail for a pointer")
        }
        other => crate::attr_slot::scalar_to_slot_word(context, builder, other),
    };
    crate::list::build_list_append(builder, rt, holder, word);
    let len = crate::list::build_list_len(context, builder, holder);
    let two = builder
        .build_int_compare(IntPredicate::EQ, len, i64_type.const_int(2, false), "stream_two")
        .expect("build_int_compare should not fail");
    builder
        .build_conditional_branch(two, compare_bb, test_bb)
        .expect("build_conditional_branch should not fail");

    builder.position_at_end(compare_bb);
    let key_at = |index: u64, name: &str| {
        let word = call(builder, key, &[holder.into(), i64_type.const_int(index, false).into()])
            .expect("pycc_rt_order_stream_key returns an i64")
            .into_int_value();
        builder
            .build_int_to_ptr(word, ptr_type, name)
            .expect("build_int_to_ptr should not fail")
    };
    let (new_key, best_key) = (key_at(1, "stream_new_key"), key_at(0, "stream_best_key"));
    // `new < best` for `min`, `new > best` for `max` (or the reflected
    // method with the operands swapped).
    let (first, second) = if order.swap {
        (best_key, new_key)
    } else {
        (new_key, best_key)
    };
    let user_function = &user_functions[order.callee.as_str()];
    let result = build_call_to_with_leading_args(
        context,
        builder,
        module,
        rt,
        user_functions,
        locals,
        user_function,
        &order.callee,
        &[first.into(), second.into()],
        &[],
    )
    .try_as_basic_value()
    .expect_basic("a comparison method returns a `bool`")
    .into_int_value();
    let pending = emit_exception_pending(context, builder, rt, "stream_exc");
    let raised = builder
        .build_int_compare(IntPredicate::NE, pending, i8_type.const_zero(), "stream_raised")
        .expect("build_int_compare should not fail");
    let better = builder
        .build_int_compare(
            IntPredicate::NE,
            result,
            result.get_type().const_zero(),
            "stream_better",
        )
        .expect("build_int_compare should not fail");
    let better = builder
        .build_int_z_extend(better, i8_type, "stream_better_byte")
        .expect("build_int_z_extend should not fail");
    builder
        .build_conditional_branch(raised, raise_bb, settle_bb)
        .expect("build_conditional_branch should not fail");

    builder.position_at_end(raise_bb);
    crate::exception::jump_to_exception_target(context, builder, rt);

    builder.position_at_end(settle_bb);
    call(builder, settle, &[holder.into(), better.into()]);
    builder
        .build_unconditional_branch(test_bb)
        .expect("build_unconditional_branch should not fail");

    builder.position_at_end(done_bb);
    let word = call(
        builder,
        finish,
        &[
            holder.into(),
            i64_type
                .const_int(crate::list::list_kind(rt, &order.elem), false)
                .into(),
            i8_type
                .const_int(u64::from(order.kind == pycc_mir::OrderKind::Max), false)
                .into(),
        ],
    )
    .expect("pycc_rt_order_stream_finish returns an i64")
    .into_int_value();
    // An empty generator raised CPython's `ValueError`.
    guard_statement_effects(context, builder, rt);
    heap_rc::emit_decref(context, builder, rt, heap_rc::HeapKind::List, holder);
    crate::generator::release_operand(context, builder, rt, object, fresh, mark);
    heap_rc::truncate_pending(rt, mark);
    crate::list::word_element(context, builder, rt, &order.elem, word)
}

/// Emits `order` (see the module doc) and returns its value.
pub(super) fn emit_instance_order<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    order: &pycc_mir::InstanceOrderExpr,
) -> Scalar<'ctx> {
    if matches!(order.list.ty(), pycc_mir::Ty::Generator(_)) {
        return emit_streamed_extreme(context, builder, module, rt, user_functions, locals, order);
    }
    let i64_type = context.i64_type();
    let i8_type = context.i8_type();
    let ptr_type = context.ptr_type(inkwell::AddressSpace::default());
    let mark = heap_rc::pending_mark(rt);
    // A keyed `xs.sort(key=...)` evaluates `xs` before its pairs.
    let target = order.target.as_ref().map(|target| {
        let scalar = emit_expr(context, builder, module, rt, user_functions, locals, target);
        heap_rc::hold_temporary(rt, target, &scalar);
        (target, scalar)
    });
    let list_scalar = emit_expr(context, builder, module, rt, user_functions, locals, &order.list);
    let list = crate::container_access::expect_list_pointer(list_scalar, "an ordered list");
    heap_rc::hold_temporary(rt, &order.list, &list_scalar);
    let mut mode = i64_type.const_int(
        order.kind.mode() as u64 + if order.keyed { 4 } else { 0 },
        false,
    );
    if let Some(reverse) = &order.reverse {
        let flag = emit_expr(context, builder, module, rt, user_functions, locals, reverse);
        let Scalar::Bool(flag) = flag else {
            panic!("pycc_codegen: internal error: a `reverse=` flag is a `bool`")
        };
        let flag = builder
            .build_int_z_extend(flag, i64_type, "order_reverse")
            .expect("build_int_z_extend should not fail");
        let flag = builder
            .build_int_mul(flag, i64_type.const_int(8, false), "order_reverse_bit")
            .expect("build_int_mul should not fail");
        mode = builder
            .build_int_add(mode, flag, "order_mode")
            .expect("build_int_add should not fail");
    }
    let target_list = match &target {
        Some((_, scalar)) => {
            crate::container_access::expect_list_pointer(*scalar, "a sorted list")
        }
        None => list,
    };

    let begin = runtime(
        context,
        module,
        "pycc_rt_order_begin",
        Some(ptr_type.into()),
        &[ptr_type.into(), i64_type.into(), i64_type.into(), ptr_type.into()],
    );
    let step = runtime(
        context,
        module,
        "pycc_rt_order_step",
        Some(i8_type.into()),
        &[ptr_type.into(), i8_type.into()],
    );
    let left = runtime(context, module, "pycc_rt_order_left", Some(i64_type.into()), &[ptr_type.into()]);
    let right = runtime(context, module, "pycc_rt_order_right", Some(i64_type.into()), &[ptr_type.into()]);
    let finish = runtime(
        context,
        module,
        "pycc_rt_order_finish",
        Some(i64_type.into()),
        &[ptr_type.into()],
    );
    let abort = runtime(context, module, "pycc_rt_order_abort", None, &[ptr_type.into()]);

    let state = call(
        builder,
        begin,
        &[
            list.into(),
            mode.into(),
            i64_type
                .const_int(crate::list::list_kind(rt, &order.elem), false)
                .into(),
            if order.kind == pycc_mir::OrderKind::Sort {
                target_list
            } else {
                ptr_type.const_null()
            }
            .into(),
        ],
    )
    .expect("pycc_rt_order_begin returns a pointer")
    .into_pointer_value();
    // `min()`/`max()` of an empty list raised.
    guard_statement_effects(context, builder, rt);

    let function = builder
        .get_insert_block()
        .and_then(|block| block.get_parent())
        .expect("an ordering is emitted inside a function");
    let less_slot = build_at_entry_block(builder, function, |b| {
        b.build_alloca(i8_type, "order_less")
            .expect("build_alloca should not fail")
    });
    builder
        .build_store(less_slot, i8_type.const_zero())
        .expect("build_store should not fail");
    let test_bb = context.append_basic_block(function, "order_test");
    let compare_bb = context.append_basic_block(function, "order_compare");
    let abort_bb = context.append_basic_block(function, "order_abort");
    let next_bb = context.append_basic_block(function, "order_next");
    let done_bb = context.append_basic_block(function, "order_done");
    builder
        .build_unconditional_branch(test_bb)
        .expect("build_unconditional_branch should not fail");

    builder.position_at_end(test_bb);
    let less = builder
        .build_load(i8_type, less_slot, "order_less_v")
        .expect("build_load should not fail")
        .into_int_value();
    let more = call(builder, step, &[state.into(), less.into()])
        .expect("pycc_rt_order_step returns an i8")
        .into_int_value();
    let more = builder
        .build_int_compare(IntPredicate::NE, more, i8_type.const_zero(), "order_more")
        .expect("build_int_compare should not fail");
    builder
        .build_conditional_branch(more, compare_bb, done_bb)
        .expect("build_conditional_branch should not fail");

    builder.position_at_end(compare_bb);
    let operand = |f: FunctionValue<'ctx>, name: &str| {
        let word = call(builder, f, &[state.into()])
            .expect("an order operand is an i64")
            .into_int_value();
        builder
            .build_int_to_ptr(word, ptr_type, name)
            .expect("build_int_to_ptr should not fail")
    };
    let (l, r) = (operand(left, "order_left"), operand(right, "order_right"));
    let (first, second) = if order.swap { (r, l) } else { (l, r) };
    // MIR resolved the callee from the class's method table, which codegen
    // declares in full before any body.
    let user_function = &user_functions[order.callee.as_str()];
    let result = build_call_to_with_leading_args(
        context,
        builder,
        module,
        rt,
        user_functions,
        locals,
        user_function,
        &order.callee,
        &[first.into(), second.into()],
        &[],
    )
    .try_as_basic_value()
    .expect_basic("a comparison method returns a `bool`")
    .into_int_value();
    let pending = emit_exception_pending(context, builder, rt, "order_exc");
    let raised = builder
        .build_int_compare(IntPredicate::NE, pending, i8_type.const_zero(), "order_raised")
        .expect("build_int_compare should not fail");
    let is_less = builder
        .build_int_compare(
            IntPredicate::NE,
            result,
            result.get_type().const_zero(),
            "order_is_less",
        )
        .expect("build_int_compare should not fail");
    let is_less = builder
        .build_int_z_extend(is_less, i8_type, "order_less_byte")
        .expect("build_int_z_extend should not fail");
    builder
        .build_store(less_slot, is_less)
        .expect("build_store should not fail");
    builder
        .build_conditional_branch(raised, abort_bb, next_bb)
        .expect("build_conditional_branch should not fail");

    builder.position_at_end(abort_bb);
    call(builder, abort, &[state.into()]);
    crate::exception::jump_to_exception_target(context, builder, rt);

    builder.position_at_end(next_bb);
    builder
        .build_unconditional_branch(test_bb)
        .expect("build_unconditional_branch should not fail");

    builder.position_at_end(done_bb);
    let word = call(builder, finish, &[state.into()])
        .expect("pycc_rt_order_finish returns an i64")
        .into_int_value();
    heap_rc::truncate_pending(rt, mark);
    // `xs.sort()` of a list changed while sorting raised; the result word
    // of `sorted`/`min`/`max` is owned, released by the unwind as a
    // temporary would be once it is held.
    let value = match order.kind {
        pycc_mir::OrderKind::Sorted => Scalar::List(
            builder
                .build_int_to_ptr(word, ptr_type, "order_sorted")
                .expect("build_int_to_ptr should not fail"),
        ),
        pycc_mir::OrderKind::Sort => crate::call_result::none_scalar(context),
        pycc_mir::OrderKind::Min | pycc_mir::OrderKind::Max => {
            crate::list::word_element(context, builder, rt, &order.elem, word)
        }
    };
    release_scalar_if_temporary(context, builder, rt, &order.list, &list_scalar);
    if let Some((target, scalar)) = &target {
        release_scalar_if_temporary(context, builder, rt, target, scalar);
    }
    value
}

/// Emits `element in set` (`negate`: `not in`) for a set of instances.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_instance_set_contains<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    element: &MirExpr,
    set: &MirExpr,
    ops: &pycc_mir::SetElementOps,
    negate: bool,
) -> Scalar<'ctx> {
    let element_scalar = emit_expr(context, builder, module, rt, user_functions, locals, element);
    let mark = heap_rc::pending_mark(rt);
    heap_rc::hold_temporary(rt, element, &element_scalar);
    let set_scalar = emit_expr(context, builder, module, rt, user_functions, locals, set);
    heap_rc::hold_temporary(rt, set, &set_scalar);
    let element_ptr =
        crate::container_access::expect_instance_pointer(element_scalar, "a set needle");
    let set_ptr = crate::set_instance::expect_set_pointer(set_scalar, "a searched set");
    let emitter = crate::set_instance::SetEmitter {
        context,
        builder,
        module,
        rt,
        user_functions,
        locals,
    };
    let found = emitter.contains(set_ptr, element_ptr, ops);
    let found = if negate {
        builder
            .build_not(found, "set_not_found")
            .expect("build_not should not fail")
    } else {
        found
    };
    heap_rc::truncate_pending(rt, mark);
    release_scalar_if_temporary(context, builder, rt, set, &set_scalar);
    release_scalar_if_temporary(context, builder, rt, element, &element_scalar);
    Scalar::Bool(
        builder
            .build_int_z_extend(found, context.i8_type(), "set_found_byte")
            .expect("build_int_z_extend should not fail"),
    )
}
