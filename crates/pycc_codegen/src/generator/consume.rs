//! The consumers of a generator: `for x in gen`, `next()`, `list()`,
//! `any()`/`all()`.

use super::*;

/// Resumes the generator `object` and branches on the answer: to `got`
/// with a value in its value area, to `done` once it is exhausted. An
/// exception the body raised leaves through the innermost exception target.
fn resume_branch<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    object: PointerValue<'ctx>,
    got: BasicBlock<'ctx>,
    done: BasicBlock<'ctx>,
) {
    resume_branch_with(context, builder, rt, rt.gens.resume, object, got, done);
}

/// [`resume_branch`] through `resume`: `pycc_rt_gen_resume`, or
/// `pycc_rt_gen_resume_raw` for a generator pycc makes of a call's own
/// argument (a keyed `min`/`max`'s pairs), whose `StopIteration` CPython
/// would not convert.
pub(crate) fn resume_branch_with<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    resume: FunctionValue<'ctx>,
    object: PointerValue<'ctx>,
    got: BasicBlock<'ctx>,
    done: BasicBlock<'ctx>,
) {
    let answer = builder
        .build_call(resume, &[object.into()], "gen_resume")
        .expect("build_call should not fail for pycc_rt_gen_resume")
        .try_as_basic_value()
        .expect_basic("pycc_rt_gen_resume returns an i8")
        .into_int_value();
    guard_statement_effects(context, builder, rt);
    let has_value = builder
        .build_int_compare(
            IntPredicate::NE,
            answer,
            context.i8_type().const_zero(),
            "gen_has_value",
        )
        .expect("build_int_compare should not fail for an i8");
    builder
        .build_conditional_branch(has_value, got, done)
        .expect("build_conditional_branch should not fail for an i1");
}

/// `for var in gen:` (`MirStmt::ForGen`).
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_for_gen_stmt<'ctx>(
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
    let MirStmt::ForGen {
        var,
        var_ty,
        generator,
        hold,
        consume,
        body,
    } = stmt
    else {
        unreachable!()
    };
    let ptr_type = context.ptr_type(inkwell::AddressSpace::default());
    let source = locals
        .get(generator)
        .cloned()
        .expect("a generator local has a slot");
    let object = builder
        .build_load(ptr_type, source.ptr, "gen_loop_object")
        .expect("build_load should not fail for a generator slot")
        .into_pointer_value();
    if *consume {
        // The temporary bound only for this loop gives its reference up.
        builder
            .build_store(source.ptr, ptr_type.const_null())
            .expect("build_store should not fail for a generator slot");
    } else {
        heap_rc::emit_incref(context, builder, rt, heap_rc::HeapKind::Generator, object);
    }
    let hold_slot = locals
        .get(hold)
        .cloned()
        .expect("the loop's hold has a slot");
    heap_rc::release_slot(context, builder, rt, &hold_slot.ty, hold_slot.ptr);
    builder
        .build_store(hold_slot.ptr, object)
        .expect("build_store should not fail for the loop's hold");
    let function = builder.get_insert_block().unwrap().get_parent().unwrap();
    let test_bb = context.append_basic_block(function, "for_gen_test");
    let body_bb = context.append_basic_block(function, "for_gen_body");
    let after_bb = context.append_basic_block(function, "for_gen_after");
    // An exception leaving the loop releases the generator on its way out,
    // before any enclosing handler or `finally` runs: CPython drops the
    // loop's iterator while unwinding to the handler.
    let unwind_bb = context.append_basic_block(function, "for_gen_unwind");
    let outer_target = *rt
        .exceptions
        .targets
        .borrow()
        .last()
        .expect("statement emission always has an installed exception target");
    rt.exceptions.targets.borrow_mut().push(unwind_bb);
    builder
        .build_unconditional_branch(test_bb)
        .expect("build_unconditional_branch should not fail entering the loop test");
    builder.position_at_end(test_bb);
    resume_branch(context, builder, rt, object, body_bb, after_bb);
    builder.position_at_end(body_bb);
    // The value is the generator's: the target owns it, releasing what it
    // held before.
    let value = load_value(context, builder, object, var_ty);
    if let Some(slot) = locals.get(var) {
        heap_rc::release_slot(context, builder, rt, &slot.ty, slot.ptr);
    }
    emit_assign(context, builder, rt, locals, var, value);
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
    rt.exceptions.targets.borrow_mut().pop();
    // Whatever jumped here already released its own pending temporaries.
    builder.position_at_end(unwind_bb);
    heap_rc::release_slot(context, builder, rt, &hold_slot.ty, hold_slot.ptr);
    builder
        .build_store(hold_slot.ptr, ptr_type.const_null())
        .expect("build_store should not fail for the loop's hold");
    builder
        .build_unconditional_branch(outer_target)
        .expect("build_unconditional_branch should not fail for the loop's unwind");
    builder.position_at_end(after_bb);
    // The loop is over (exhausted, or left by `break`): release the
    // generator now, closing it if it is unfinished, as CPython does.
    heap_rc::release_slot(context, builder, rt, &hold_slot.ty, hold_slot.ptr);
    builder
        .build_store(hold_slot.ptr, ptr_type.const_null())
        .expect("build_store should not fail for the loop's hold");
    Ok(())
}

/// The generator operand of a consumer, and whether the consumer must
/// release it once done (a fresh one: `next(make())`).
pub(crate) fn operand<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    generator: &MirExpr,
) -> (PointerValue<'ctx>, bool) {
    let Scalar::Generator(object) = emit_expr(
        context,
        builder,
        module,
        rt,
        user_functions,
        locals,
        generator,
    ) else {
        panic!("pycc_codegen: internal error: a generator operand is not a generator")
    };
    let fresh = !heap_rc::is_borrowed_read(generator);
    if fresh {
        heap_rc::hold_owned(rt, heap_rc::HeapKind::Generator, object);
    }
    (object, fresh)
}

/// Releases a fresh generator operand held by [`operand`].
pub(crate) fn release_operand<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    object: PointerValue<'ctx>,
    fresh: bool,
    mark: usize,
) {
    if fresh {
        heap_rc::truncate_pending(rt, mark);
        heap_rc::emit_decref(context, builder, rt, heap_rc::HeapKind::Generator, object);
    }
}

/// `next(gen)` / `next(gen, default)` (`MirExpr::GenNext`).
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_next<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    generator: &MirExpr,
    default: Option<&MirExpr>,
    ty: &Ty,
) -> Scalar<'ctx> {
    let mark = heap_rc::pending_mark(rt);
    let (object, fresh) = operand(
        context,
        builder,
        module,
        rt,
        user_functions,
        locals,
        generator,
    );
    let function = builder.get_insert_block().unwrap().get_parent().unwrap();
    let Some(default) = default else {
        let Ty::Generator(sig) = generator.ty() else {
            panic!("pycc_codegen: internal error: next() of a non-generator")
        };
        let answer = builder
            .build_call(
                rt.gens.step,
                &[object.into(), context.i8_type().const_zero().into()],
                "gen_next",
            )
            .expect("build_call should not fail for pycc_rt_gen_step")
            .try_as_basic_value()
            .expect_basic("pycc_rt_gen_step returns an i8")
            .into_int_value();
        let value = super::protocol::yielded_or_stop(context, builder, rt, object, answer, &sig);
        release_operand(context, builder, rt, object, fresh, mark);
        return value;
    };
    let got_bb = context.append_basic_block(function, "gen_next_got");
    let exhausted_bb = context.append_basic_block(function, "gen_next_exhausted");
    let join_bb = context.append_basic_block(function, "gen_next_join");
    resume_branch(context, builder, rt, object, got_bb, exhausted_bb);
    builder.position_at_end(got_bb);
    let got = scalar_basic(load_value(context, builder, object, ty));
    let got_end = builder.get_insert_block().unwrap();
    builder
        .build_unconditional_branch(join_bb)
        .expect("build_unconditional_branch should not fail");
    builder.position_at_end(exhausted_bb);
    let fallback = emit_expr(
        context,
        builder,
        module,
        rt,
        user_functions,
        locals,
        default,
    );
    let fallback = heap_rc::retain_if_heap_duplicate(context, builder, rt, default, fallback);
    let fallback = retain_if_int_duplicate(context, builder, rt, default, fallback);
    let fallback = scalar_basic(coerce_scalar_to_type(
        context,
        builder,
        rt,
        fallback,
        ty.clone(),
    ));
    let fallback_end = builder.get_insert_block().unwrap();
    builder
        .build_unconditional_branch(join_bb)
        .expect("build_unconditional_branch should not fail");
    builder.position_at_end(join_bb);
    let phi = builder
        .build_phi(ty_to_basic_type(context, ty.clone()), "gen_next_value")
        .expect("build_phi should not fail in a fresh join block");
    phi.add_incoming(&[(&got, got_end), (&fallback, fallback_end)]);
    let value = basic_scalar(ty, phi.as_basic_value());
    release_operand(context, builder, rt, object, fresh, mark);
    value
}

/// `list(gen)` (`MirExpr::GenToList`): a new list of the generator's
/// remaining values.
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_to_list<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    generator: &MirExpr,
    ty: &Ty,
    raw: bool,
) -> Scalar<'ctx> {
    let element = list::element_ty(ty).clone();
    let mark = heap_rc::pending_mark(rt);
    let (object, fresh) = operand(
        context,
        builder,
        module,
        rt,
        user_functions,
        locals,
        generator,
    );
    let result = list::build_list_new(context, builder, rt, &element, 0);
    heap_rc::hold_owned(rt, heap_rc::HeapKind::List, result);
    let function = builder.get_insert_block().unwrap().get_parent().unwrap();
    let test_bb = context.append_basic_block(function, "gen_list_test");
    let got_bb = context.append_basic_block(function, "gen_list_got");
    let done_bb = context.append_basic_block(function, "gen_list_done");
    builder
        .build_unconditional_branch(test_bb)
        .expect("build_unconditional_branch should not fail");
    builder.position_at_end(test_bb);
    let resume = if raw { rt.gens.resume_raw } else { rt.gens.resume };
    resume_branch_with(context, builder, rt, resume, object, got_bb, done_bb);
    builder.position_at_end(got_bb);
    let value = load_value(context, builder, object, &element);
    // The list adopts the reference the generator handed over.
    let word = match (&element, value) {
        (Ty::Int, Scalar::Int(word)) => word,
        (Ty::Tuple(elements), Scalar::Tuple(tuple)) => {
            let boxed = crate::containers::box_struct_tuple(context, builder, rt, elements, tuple);
            builder
                .build_ptr_to_int(boxed, context.i64_type(), "boxed_tuple_word")
                .expect("build_ptr_to_int should not fail for a pointer")
        }
        (_, value) => scalar_to_slot_word(context, builder, value),
    };
    list::build_list_append(builder, rt, result, word);
    builder
        .build_unconditional_branch(test_bb)
        .expect("build_unconditional_branch should not fail");
    builder.position_at_end(done_bb);
    release_operand(context, builder, rt, object, fresh, mark);
    heap_rc::truncate_pending(rt, mark);
    Scalar::List(result)
}

/// `any(gen)` / `all(gen)` (`MirExpr::GenAnyAll`): resumes the generator
/// only until a value decides the answer.
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_any_all<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    generator: &MirExpr,
    all: bool,
) -> Scalar<'ctx> {
    let Ty::Generator(sig) = generator.ty() else {
        panic!("pycc_codegen: internal error: any()/all() of a non-generator")
    };
    let element = sig.0;
    let mark = heap_rc::pending_mark(rt);
    let (object, fresh) = operand(
        context,
        builder,
        module,
        rt,
        user_functions,
        locals,
        generator,
    );
    let function = builder.get_insert_block().unwrap().get_parent().unwrap();
    let test_bb = context.append_basic_block(function, "gen_any_test");
    let got_bb = context.append_basic_block(function, "gen_any_got");
    let exhausted_bb = context.append_basic_block(function, "gen_any_exhausted");
    let decided_bb = context.append_basic_block(function, "gen_any_decided");
    let join_bb = context.append_basic_block(function, "gen_any_join");
    builder
        .build_unconditional_branch(test_bb)
        .expect("build_unconditional_branch should not fail");
    builder.position_at_end(test_bb);
    resume_branch(context, builder, rt, object, got_bb, exhausted_bb);
    builder.position_at_end(got_bb);
    let value = load_value(context, builder, object, &element);
    let truth = crate::str_truthy::truthy(context, builder, module, rt, value);
    // The value was handed over; only its truth is kept.
    release_owned(context, builder, rt, &element, &value);
    let is_true = truth;
    let (on_true, on_false) = if all {
        (test_bb, decided_bb)
    } else {
        (decided_bb, test_bb)
    };
    builder
        .build_conditional_branch(is_true, on_true, on_false)
        .expect("build_conditional_branch should not fail for an i1");
    builder.position_at_end(decided_bb);
    builder
        .build_unconditional_branch(join_bb)
        .expect("build_unconditional_branch should not fail");
    builder.position_at_end(exhausted_bb);
    builder
        .build_unconditional_branch(join_bb)
        .expect("build_unconditional_branch should not fail");
    builder.position_at_end(join_bb);
    let phi = builder
        .build_phi(context.i8_type(), "gen_any_result")
        .expect("build_phi should not fail in a fresh join block");
    let decided = context.i8_type().const_int(u64::from(!all), false);
    let exhausted = context.i8_type().const_int(u64::from(all), false);
    phi.add_incoming(&[(&decided, decided_bb), (&exhausted, exhausted_bb)]);
    release_operand(context, builder, rt, object, fresh, mark);
    Scalar::Bool(phi.as_basic_value().into_int_value())
}

/// Releases an owned value of type `ty`.
pub(crate) fn release_owned<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    ty: &Ty,
    value: &Scalar<'ctx>,
) {
    match (ty, value) {
        (Ty::Int, Scalar::Int(word)) => crate::bigint_rc::emit_bigint_refcount_call(
            context,
            builder,
            rt,
            *word,
            crate::bigint_rc::BigIntRefcount::Release,
        ),
        _ => {
            if let Some(kind) = heap_rc::heap_kind(rt, ty)
                && let Some(ptr) = pointer_of(value)
            {
                heap_rc::emit_decref(context, builder, rt, kind, ptr);
            }
        }
    }
}

fn pointer_of<'ctx>(value: &Scalar<'ctx>) -> Option<PointerValue<'ctx>> {
    match value {
        Scalar::Str(v)
        | Scalar::List(v)
        | Scalar::Dict(v)
        | Scalar::Set(v)
        | Scalar::Instance(v)
        | Scalar::Object(v)
        | Scalar::Generator(v) => Some(*v),
        _ => None,
    }
}
