//! The generator protocol beyond iteration: `send()`, `throw()`, `close()`,
//! `return value` (`StopIteration.value`) and `yield from`.
//!
//! Every resume answers like `pycc_rt::generator::pycc_rt_gen_step`: `1`
//! with a yielded value in the object's value area, `2` with the value the
//! body returned there, `0` when it finished without one or raised.
//! [`yielded_or_stop`] turns that answer into the operation's result: the
//! yielded value, or `StopIteration` carrying the returned value (CPython's
//! `StopIteration.value`).

use super::*;
use crate::exception_value::emit_exception_value;
use pycc_mir::MirExceptionValue;

/// After a resume of `object` that answered `answer`: leaves the builder
/// where the yielded value (of type `sig.0`, owned) was loaded and returns
/// it; on a return or the end raises `StopIteration` (with the returned
/// value of type `sig.2`) and leaves through the innermost exception
/// target.
#[allow(clippy::too_many_arguments)]
pub(super) fn yielded_or_stop<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    object: PointerValue<'ctx>,
    answer: IntValue<'ctx>,
    sig: &(Ty, Ty, Ty),
) -> Scalar<'ctx> {
    let function = builder.get_insert_block().unwrap().get_parent().unwrap();
    let got_bb = context.append_basic_block(function, "gen_op_got");
    let returned_bb = context.append_basic_block(function, "gen_op_returned");
    let finished_bb = context.append_basic_block(function, "gen_op_finished");
    builder
        .build_switch(
            answer,
            finished_bb,
            &[
                (context.i8_type().const_int(1, false), got_bb),
                (context.i8_type().const_int(2, false), returned_bb),
            ],
        )
        .expect("build_switch should not fail for a generator answer");
    builder.position_at_end(returned_bb);
    raise_stop_with_return(context, builder, rt, object, &sig.2);
    crate::exception::jump_to_exception_target(context, builder, rt);
    builder.position_at_end(finished_bb);
    builder
        .build_call(rt.gens.stop_unless_raised, &[], "")
        .expect("build_call should not fail for pycc_rt_gen_stop_unless_raised");
    crate::exception::jump_to_exception_target(context, builder, rt);
    builder.position_at_end(got_bb);
    load_value(context, builder, object, &sig.0)
}

/// Raises `StopIteration` carrying the value of type `ret` the generator
/// `object`'s body returned (the area's value, whose reference passes to
/// the exception).
pub(super) fn raise_stop_with_return<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    object: PointerValue<'ctx>,
    ret: &Ty,
) {
    store_byte(context, builder, object, HAS_RET_OFFSET, 0);
    let ptr_type = context.ptr_type(inkwell::AddressSpace::default());
    let (word, message) = if matches!(ret, Ty::None) {
        (context.i64_type().const_zero(), ptr_type.const_null())
    } else {
        let value = load_value(context, builder, object, ret);
        if let Scalar::Str(text) = value {
            // `str(value)` is the value itself: the message takes a
            // reference of its own.
            heap_rc::emit_incref(context, builder, rt, heap_rc::HeapKind::Str, text);
        }
        let message = to_str(builder, rt, value);
        (scalar_to_slot_word(context, builder, value), message)
    };
    let tag = context.i64_type().const_int(stop_value_tag(ret), false);
    builder
        .build_call(
            rt.gens.raise_stop,
            &[word.into(), message.into(), tag.into()],
            "",
        )
        .expect("build_call should not fail for pycc_rt_raise_stop_iteration");
}

/// The runtime tag of a `StopIteration` value of type `ty`
/// (`pycc_rt::exception::pycc_rt_raise_stop_iteration`): `0` for `None`,
/// else a nonzero FNV-1a hash of the type's full structure, so `e.value`
/// read as one type never reinterprets the word of another
/// (`pycc_rt_exception_payload`).
fn stop_value_tag(ty: &Ty) -> u64 {
    if matches!(ty, Ty::None) {
        return 0;
    }
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in format!("{ty:?}").bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash | 1
}

/// `e.value` of a caught `StopIteration` `exception` (an exception object
/// pointer): the generator's return value of type `ty`, a new reference.
/// A value of another type -- the `StopIteration` came from a generator
/// some call in the `try` body resumed, not one the body resumes itself --
/// raises `TypeError` instead (`pycc_rt_exception_payload`).
pub(crate) fn emit_stop_iteration_value<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    exception: PointerValue<'ctx>,
    ty: &Ty,
) -> Scalar<'ctx> {
    let tag = context.i64_type().const_int(stop_value_tag(ty), false);
    let word = builder
        .build_call(
            rt.gens.exception_payload,
            &[exception.into(), tag.into()],
            "stop_value",
        )
        .expect("build_call should not fail for pycc_rt_exception_payload")
        .try_as_basic_value()
        .expect_basic("pycc_rt_exception_payload returns an i64")
        .into_int_value();
    guard_statement_effects(context, builder, rt);
    if matches!(ty, Ty::None) {
        return none_scalar(context);
    }
    let value = slot_word_to_scalar(context, builder, word, ty);
    heap_rc::retain_scalar(context, builder, rt, ty, &value);
    value
}

/// `gen.send(value)` (`MirExpr::GenSend`): the value the generator yields
/// next, owned.
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_send<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    generator: &MirExpr,
    value: &MirExpr,
) -> Scalar<'ctx> {
    let Ty::Generator(sig) = generator.ty() else {
        panic!("pycc_codegen: internal error: send() of a non-generator")
    };
    let mark = heap_rc::pending_mark(rt);
    let (object, fresh) = consume::operand(
        context,
        builder,
        module,
        rt,
        user_functions,
        locals,
        generator,
    );
    let sent_ty = sig.1.clone();
    let scalar = emit_expr(context, builder, module, rt, user_functions, locals, value);
    let sent = if matches!(sent_ty, Ty::None) {
        None
    } else {
        let scalar = heap_rc::retain_if_heap_duplicate(context, builder, rt, value, scalar);
        let scalar = retain_if_int_duplicate(context, builder, rt, value, scalar);
        Some(coerce_scalar_to_type(
            context,
            builder,
            scalar,
            sent_ty.clone(),
        ))
    };
    let held = heap_rc::pending_mark(rt);
    if let Some(sent) = &sent {
        heap_rc::hold_owned_value(rt, &sent_ty, sent);
    }
    let ok = builder
        .build_call(
            rt.gens.can_send,
            &[
                object.into(),
                context
                    .i8_type()
                    .const_int(u64::from(sent.is_none()), false)
                    .into(),
            ],
            "gen_can_send",
        )
        .expect("build_call should not fail for pycc_rt_gen_can_send")
        .try_as_basic_value()
        .expect_basic("pycc_rt_gen_can_send returns an i8")
        .into_int_value();
    let _ = ok;
    guard_statement_effects(context, builder, rt);
    heap_rc::truncate_pending(rt, held);
    if let Some(sent) = sent {
        let slot = byte_ptr(context, builder, object, VALUE_OFFSET);
        builder
            .build_store(slot, scalar_basic(sent))
            .expect("build_store should not fail for the value area");
    }
    let answer = builder
        .build_call(
            rt.gens.step,
            &[object.into(), context.i8_type().const_int(1, false).into()],
            "gen_send",
        )
        .expect("build_call should not fail for pycc_rt_gen_step")
        .try_as_basic_value()
        .expect_basic("pycc_rt_gen_step returns an i8")
        .into_int_value();
    let result = yielded_or_stop(context, builder, rt, object, answer, &sig);
    consume::release_operand(context, builder, rt, object, fresh, mark);
    result
}

/// `gen.throw(exception)` (`MirExpr::GenThrow`): the value the generator
/// yields after handling it, owned.
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_throw<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    generator: &MirExpr,
    exception: &MirExceptionValue,
) -> Scalar<'ctx> {
    let Ty::Generator(sig) = generator.ty() else {
        panic!("pycc_codegen: internal error: throw() of a non-generator")
    };
    let mark = heap_rc::pending_mark(rt);
    let (object, fresh) = consume::operand(
        context,
        builder,
        module,
        rt,
        user_functions,
        locals,
        generator,
    );
    let thrown = emit_exception_value(
        context,
        builder,
        module,
        rt,
        user_functions,
        locals,
        exception,
        "thrown",
    )
    .expect("a thrown exception value lowers like a raised one");
    let answer = builder
        .build_call(rt.gens.throw, &[object.into(), thrown.into()], "gen_throw")
        .expect("build_call should not fail for pycc_rt_gen_throw")
        .try_as_basic_value()
        .expect_basic("pycc_rt_gen_throw returns an i8")
        .into_int_value();
    let result = yielded_or_stop(context, builder, rt, object, answer, &sig);
    consume::release_operand(context, builder, rt, object, fresh, mark);
    result
}

/// `gen.close()` (`MirExpr::GenClose`): `None`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_close<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    generator: &MirExpr,
) -> Scalar<'ctx> {
    let mark = heap_rc::pending_mark(rt);
    let (object, fresh) = consume::operand(
        context,
        builder,
        module,
        rt,
        user_functions,
        locals,
        generator,
    );
    builder
        .build_call(rt.gens.close, &[object.into()], "gen_close")
        .expect("build_call should not fail for pycc_rt_gen_close");
    guard_statement_effects(context, builder, rt);
    consume::release_operand(context, builder, rt, object, fresh, mark);
    none_scalar(context)
}

/// `return value` in a generator (`MirStmt::GenReturn`): leaves the value,
/// owned, in the object's value area for `StopIteration.value` (the
/// statement after it is the bare `return` that ends the body).
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_gen_return<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    value: &MirExpr,
) {
    let frame = current_frame(rt);
    let scalar = emit_expr(context, builder, module, rt, user_functions, locals, value);
    if matches!(frame.sig.2, Ty::None) {
        return;
    }
    let scalar = heap_rc::retain_if_heap_duplicate(context, builder, rt, value, scalar);
    let scalar = retain_if_int_duplicate(context, builder, rt, value, scalar);
    let scalar = coerce_scalar_to_type(context, builder, scalar, frame.sig.2.clone());
    let slot = byte_ptr(context, builder, frame.object, VALUE_OFFSET);
    builder
        .build_store(slot, scalar_basic(scalar))
        .expect("build_store should not fail for the value area");
    store_byte(context, builder, frame.object, HAS_RET_OFFSET, 1);
}

/// `yield from sub` (`MirExpr::GenYieldFrom`): delegates to the generator
/// `sub` until it finishes -- each value it yields is yielded by this
/// generator, each value sent to (exception thrown into, close of) this
/// generator is forwarded to it (`pycc_rt_gen_delegate`) -- and is the value
/// `sub` returned, of type `ty`, owned.
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_yield_from<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    sub: &MirExpr,
    ty: &Ty,
) -> Scalar<'ctx> {
    let Ty::Generator(sub_sig) = sub.ty() else {
        panic!("pycc_codegen: internal error: `yield from` of a non-generator")
    };
    let frame = current_frame(rt);
    let mark = heap_rc::pending_mark(rt);
    let Scalar::Generator(object) =
        emit_expr(context, builder, module, rt, user_functions, locals, sub)
    else {
        panic!("pycc_codegen: internal error: a `yield from` operand is not a generator")
    };
    // The delegation holds its own reference for as long as it runs, so
    // that every way out -- the end, an exception, a close -- releases it.
    if heap_rc::is_borrowed_read(sub) {
        heap_rc::emit_incref(context, builder, rt, heap_rc::HeapKind::Generator, object);
    }
    heap_rc::hold_owned(rt, heap_rc::HeapKind::Generator, object);
    let function = builder.get_insert_block().unwrap().get_parent().unwrap();
    let entry_bb = builder.get_insert_block().unwrap();
    let step_bb = context.append_basic_block(function, "gen_from_step");
    let yielded_bb = context.append_basic_block(function, "gen_from_yielded");
    let after_bb = context.append_basic_block(function, "gen_from_after");
    builder
        .build_unconditional_branch(step_bb)
        .expect("build_unconditional_branch should not fail");
    builder.position_at_end(step_bb);
    let first = builder
        .build_phi(context.i8_type(), "gen_from_first")
        .expect("build_phi should not fail at the head of a block");
    first.add_incoming(&[(&context.i8_type().const_int(1, false), entry_bb)]);
    let forward_send = !matches!(sub_sig.1, Ty::None);
    let answer = builder
        .build_call(
            rt.gens.delegate,
            &[
                frame.object.into(),
                object.into(),
                first.as_basic_value().into_int_value().into(),
                context
                    .i8_type()
                    .const_int(u64::from(forward_send), false)
                    .into(),
                area_size(context, &sub_sig.0).into(),
                area_size(context, &sub_sig.1).into(),
            ],
            "gen_from",
        )
        .expect("build_call should not fail for pycc_rt_gen_delegate")
        .try_as_basic_value()
        .expect_basic("pycc_rt_gen_delegate returns an i8")
        .into_int_value();
    guard_statement_effects(context, builder, rt);
    let yielded = builder
        .build_int_compare(
            IntPredicate::EQ,
            answer,
            context.i8_type().const_int(1, false),
            "gen_from_did_yield",
        )
        .expect("build_int_compare should not fail for an i8");
    builder
        .build_conditional_branch(yielded, yielded_bb, after_bb)
        .expect("build_conditional_branch should not fail for an i1");
    builder.position_at_end(yielded_bb);
    // The value `sub` yielded is in this generator's area: suspend with it,
    // and on resume let the delegation forward whatever resumed us.
    suspend_raw(context, module, builder, rt, "gen_from_resumed");
    let resumed_end = builder.get_insert_block().unwrap();
    builder
        .build_unconditional_branch(step_bb)
        .expect("build_unconditional_branch should not fail");
    first.add_incoming(&[(&context.i8_type().const_zero(), resumed_end)]);
    builder.position_at_end(after_bb);
    let result = if matches!(ty, Ty::None) {
        none_scalar(context)
    } else {
        // `2`: the value `sub` returned is in its area. `0` with nothing
        // pending: `sub` had finished before -- CPython's `None`, which a
        // non-`None` return type cannot hold (`docs/DEVIATIONS.md`).
        let returned_bb = context.append_basic_block(function, "gen_from_returned");
        let none_bb = context.append_basic_block(function, "gen_from_no_value");
        let returned = builder
            .build_int_compare(
                IntPredicate::EQ,
                answer,
                context.i8_type().const_int(2, false),
                "gen_from_did_return",
            )
            .expect("build_int_compare should not fail for an i8");
        builder
            .build_conditional_branch(returned, returned_bb, none_bb)
            .expect("build_conditional_branch should not fail for an i1");
        builder.position_at_end(none_bb);
        builder
            .build_call(rt.gens.raise_no_return, &[], "")
            .expect("build_call should not fail for pycc_rt_gen_raise_no_return");
        crate::exception::jump_to_exception_target(context, builder, rt);
        builder.position_at_end(returned_bb);
        store_byte(context, builder, object, HAS_RET_OFFSET, 0);
        load_value(context, builder, object, ty)
    };
    heap_rc::truncate_pending(rt, mark);
    heap_rc::emit_decref(context, builder, rt, heap_rc::HeapKind::Generator, object);
    result
}
