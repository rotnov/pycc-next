//! Generators as LLVM coroutines (switch-resumed ABI).
//!
//! A generator function (`MirItem::Function` whose return type is
//! `Ty::Generator(T)`) is emitted by the ordinary function emitter, as if it
//! returned `None`, with four additions:
//!
//! * [`begin`], at the very start of its entry block, allocates the
//!   coroutine frame (`llvm.coro.id`/`coro.size`/`coro.begin`);
//! * [`initial_suspend`], once the parameters are bound and right before
//!   the body, creates the generator object (`pycc_rt_gen_new`) and
//!   suspends, so calling the function runs none of its body;
//! * every `yield` ([`emit_yield`]) stores its value into the object's value
//!   area and suspends;
//! * [`finish`], once the body is emitted, turns every `ret void` the
//!   function's own exits produced -- the owned-slot epilogue's, the
//!   exception exit's, a bare `return`'s -- into a branch to the final
//!   suspend, which marks the object done.
//!
//! LLVM's coroutine passes (`CoroSplit`) then split the function into the
//! ramp (what callers call) and the resume and destroy functions, moving
//! every value live across a suspend point -- the frame's slots, a loop's
//! iterator, a pending `finally` -- into the frame. The frame's own release
//! code is unchanged: on resume after a `yield`, a generator being closed
//! raises `GeneratorExit` there, and the body unwinds through its `finally`
//! clauses and its owned-slot epilogue to the final suspend
//! (`pycc_rt::generator` drives that).
//!
//! The consumers -- `for x in gen`, `next()`, `list()`, `any()`/`all()` --
//! resume the object through `pycc_rt_gen_resume` and take ownership of
//! each value it leaves in the value area.

use super::*;
use inkwell::attributes::{Attribute, AttributeLoc};
use inkwell::basic_block::BasicBlock;
use inkwell::intrinsics::Intrinsic;
use inkwell::llvm_sys::core as llc;
use inkwell::llvm_sys::prelude::LLVMValueRef;
use inkwell::types::AsTypeRef;
use inkwell::values::{AsValueRef, BasicValueEnum, InstructionOpcode};
use pycc_mir::Ty;

/// `pycc_rt::generator::STATE_OFFSET`.
const STATE_OFFSET: u64 = 16;
/// `pycc_rt::generator::CLOSING_OFFSET`.
const CLOSING_OFFSET: u64 = 17;
/// `pycc_rt::generator::VALUE_OFFSET`.
const VALUE_OFFSET: u64 = 24;
/// `pycc_rt::generator`'s `DONE` state.
const DONE: u64 = 2;

/// The runtime functions of generator objects.
pub(crate) struct GenFns<'ctx> {
    pub(crate) new: FunctionValue<'ctx>,
    pub(crate) resume: FunctionValue<'ctx>,
    pub(crate) next: FunctionValue<'ctx>,
    pub(crate) incref: FunctionValue<'ctx>,
    pub(crate) decref: FunctionValue<'ctx>,
    pub(crate) raise_exit: FunctionValue<'ctx>,
    pub(crate) frame_alloc: FunctionValue<'ctx>,
    pub(crate) frame_free: FunctionValue<'ctx>,
}

impl<'ctx> GenFns<'ctx> {
    pub(crate) fn declare(context: &'ctx Context, module: &inkwell::module::Module<'ctx>) -> Self {
        let i64_type = context.i64_type();
        let i8_type = context.i8_type();
        let void_type = context.void_type();
        let ptr_type = context.ptr_type(inkwell::AddressSpace::default());
        let declare = |name: &str, fn_type: inkwell::types::FunctionType<'ctx>| {
            module
                .get_function(name)
                .unwrap_or_else(|| module.add_function(name, fn_type, Some(Linkage::External)))
        };
        GenFns {
            new: declare(
                "pycc_rt_gen_new",
                ptr_type.fn_type(&[ptr_type.into(), i64_type.into()], false),
            ),
            resume: declare(
                "pycc_rt_gen_resume",
                i8_type.fn_type(&[ptr_type.into()], false),
            ),
            next: declare(
                "pycc_rt_gen_next",
                i8_type.fn_type(&[ptr_type.into()], false),
            ),
            incref: declare(
                "pycc_rt_gen_incref",
                void_type.fn_type(&[ptr_type.into()], false),
            ),
            decref: declare(
                "pycc_rt_gen_decref",
                void_type.fn_type(&[ptr_type.into()], false),
            ),
            raise_exit: declare("pycc_rt_gen_raise_exit", void_type.fn_type(&[], false)),
            frame_alloc: declare(
                "pycc_rt_gen_frame_alloc",
                ptr_type.fn_type(&[i64_type.into()], false),
            ),
            frame_free: declare(
                "pycc_rt_gen_frame_free",
                void_type.fn_type(&[ptr_type.into()], false),
            ),
        }
    }
}

/// The coroutine of the generator function being emitted.
#[derive(Clone)]
pub(crate) struct GenFrame<'ctx> {
    /// The `llvm.coro.id` token.
    id: LLVMValueRef,
    /// The frame handle `llvm.coro.begin` returned.
    handle: PointerValue<'ctx>,
    /// The generator object.
    object: PointerValue<'ctx>,
    /// The type of the values the generator yields.
    element: Ty,
    /// Where every suspend point's "suspended" case returns to the caller.
    suspend_bb: BasicBlock<'ctx>,
    /// Where the destroy case frees the frame.
    cleanup_bb: BasicBlock<'ctx>,
}

/// The coroutine's frame allocation, emitted by [`begin`].
pub(crate) struct GenBegin<'ctx> {
    id: LLVMValueRef,
    handle: PointerValue<'ctx>,
}

fn intrinsic<'ctx>(
    module: &inkwell::module::Module<'ctx>,
    name: &str,
    overloads: &[inkwell::types::BasicTypeEnum<'ctx>],
) -> FunctionValue<'ctx> {
    Intrinsic::find(name)
        .and_then(|i| i.get_declaration(module, overloads))
        .unwrap_or_else(|| panic!("LLVM provides `{name}`"))
}

/// A raw call of `function` with `args` (needed for the token operands
/// inkwell's typed builder cannot pass).
fn raw_call<'ctx>(
    builder: &inkwell::builder::Builder<'ctx>,
    function: FunctionValue<'ctx>,
    args: &mut [LLVMValueRef],
    name: &str,
) -> LLVMValueRef {
    let name = std::ffi::CString::new(name).expect("an LLVM value name has no NUL");
    // SAFETY: every reference is a live value of the module the builder
    // emits into, and `args` matches `function`'s parameter list.
    unsafe {
        llc::LLVMBuildCall2(
            builder.as_mut_ptr(),
            function.get_type().as_type_ref(),
            function.as_value_ref(),
            args.as_mut_ptr(),
            args.len() as u32,
            name.as_ptr(),
        )
    }
}

fn token_none(context: &Context) -> LLVMValueRef {
    // SAFETY: a constant of the context's token type.
    unsafe { llc::LLVMConstNull(llc::LLVMTokenTypeInContext(context.raw())) }
}

/// Marks `function` as a coroutine and allocates its frame, at the
/// builder's position: the start of its entry block.
pub(crate) fn begin<'ctx>(
    context: &'ctx Context,
    module: &inkwell::module::Module<'ctx>,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    function: FunctionValue<'ctx>,
) -> GenBegin<'ctx> {
    let kind = Attribute::get_named_enum_kind_id("presplitcoroutine");
    function.add_attribute(
        AttributeLoc::Function,
        context.create_enum_attribute(kind, 0),
    );
    let ptr_type = context.ptr_type(inkwell::AddressSpace::default());
    let null = ptr_type.const_null().as_value_ref();
    let i32_zero = context.i32_type().const_zero().as_value_ref();
    let id = raw_call(
        builder,
        intrinsic(module, "llvm.coro.id", &[]),
        &mut [i32_zero, null, null, null],
        "gen_id",
    );
    let size = builder
        .build_call(
            intrinsic(module, "llvm.coro.size", &[context.i64_type().into()]),
            &[],
            "gen_size",
        )
        .expect("build_call should not fail for llvm.coro.size")
        .try_as_basic_value()
        .expect_basic("llvm.coro.size returns i64");
    let memory = builder
        .build_call(rt.gens.frame_alloc, &[size.into()], "gen_frame")
        .expect("build_call should not fail for the frame allocation")
        .try_as_basic_value()
        .expect_basic("the frame allocation returns a pointer");
    let handle = raw_call(
        builder,
        intrinsic(module, "llvm.coro.begin", &[]),
        &mut [id, memory.as_value_ref()],
        "gen_handle",
    );
    GenBegin {
        id,
        // SAFETY: `llvm.coro.begin` returns a pointer.
        handle: unsafe { PointerValue::new(handle) },
    }
}

/// Creates the generator object and suspends before the body, at the
/// builder's position (the parameters bound, the exception exit installed);
/// leaves the builder where the body starts. Records the frame in `rt` for
/// [`emit_yield`] and [`finish`].
pub(crate) fn initial_suspend<'ctx>(
    context: &'ctx Context,
    module: &inkwell::module::Module<'ctx>,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    function: FunctionValue<'ctx>,
    begun: GenBegin<'ctx>,
    element: Ty,
) {
    let value_size = ty_to_basic_type(context, element.clone())
        .size_of()
        .expect("a sized element type");
    let object = builder
        .build_call(
            rt.gens.new,
            &[begun.handle.into(), value_size.into()],
            "gen_object",
        )
        .expect("build_call should not fail for pycc_rt_gen_new")
        .try_as_basic_value()
        .expect_basic("pycc_rt_gen_new returns a pointer")
        .into_pointer_value();
    let suspend_bb = context.append_basic_block(function, "gen_suspend");
    let cleanup_bb = context.append_basic_block(function, "gen_cleanup");
    let frame = GenFrame {
        id: begun.id,
        handle: begun.handle,
        object,
        element,
        suspend_bb,
        cleanup_bb,
    };
    *rt.gen_frame.borrow_mut() = Some(frame);
    suspend_and_resume(context, module, builder, rt, "gen_start");
}

/// Emits a non-final suspend point and leaves the builder where the body
/// resumes; a generator being closed raises `GeneratorExit` there.
fn suspend_and_resume<'ctx>(
    context: &'ctx Context,
    module: &inkwell::module::Module<'ctx>,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    label: &str,
) {
    let frame = rt
        .gen_frame
        .borrow()
        .clone()
        .expect("a suspend point is emitted inside a generator function");
    let function = builder.get_insert_block().unwrap().get_parent().unwrap();
    let state = raw_call(
        builder,
        intrinsic(module, "llvm.coro.suspend", &[]),
        &mut [
            token_none(context),
            context.bool_type().const_zero().as_value_ref(),
        ],
        "gen_suspended",
    );
    // SAFETY: `llvm.coro.suspend` returns an `i8`.
    let state = unsafe { IntValue::new(state) };
    let resumed = context.append_basic_block(function, label);
    builder
        .build_switch(
            state,
            frame.suspend_bb,
            &[
                (context.i8_type().const_zero(), resumed),
                (context.i8_type().const_int(1, false), frame.cleanup_bb),
            ],
        )
        .expect("build_switch should not fail for a suspend point");
    builder.position_at_end(resumed);
    let closing_ptr = byte_ptr(context, builder, frame.object, CLOSING_OFFSET);
    let closing = builder
        .build_load(context.i8_type(), closing_ptr, "gen_closing")
        .expect("build_load should not fail for the closing flag")
        .into_int_value();
    let is_closing = builder
        .build_int_compare(
            IntPredicate::NE,
            closing,
            context.i8_type().const_zero(),
            "gen_is_closing",
        )
        .expect("build_int_compare should not fail for an i8 flag");
    let close_bb = context.append_basic_block(function, "gen_close");
    let run_bb = context.append_basic_block(function, "gen_run");
    builder
        .build_conditional_branch(is_closing, close_bb, run_bb)
        .expect("build_conditional_branch should not fail for an i1");
    builder.position_at_end(close_bb);
    builder
        .build_call(rt.gens.raise_exit, &[], "")
        .expect("build_call should not fail for pycc_rt_gen_raise_exit");
    crate::exception::jump_to_exception_target(context, builder, rt);
    builder.position_at_end(run_bb);
}

/// A pointer `offset` bytes into `object`.
fn byte_ptr<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    object: PointerValue<'ctx>,
    offset: u64,
) -> PointerValue<'ctx> {
    // SAFETY: every offset used is inside the generator object's header or
    // value area.
    unsafe {
        builder
            .build_in_bounds_gep(
                context.i8_type(),
                object,
                &[context.i64_type().const_int(offset, false)],
                "gen_field",
            )
            .expect("build_in_bounds_gep should not fail for a byte offset")
    }
}

/// `yield value`: stores an owned reference to the value, converted to the
/// element type, into the object's value area and suspends.
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_yield<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    value: &MirExpr,
) {
    let frame = rt
        .gen_frame
        .borrow()
        .clone()
        .expect("`yield` is emitted inside a generator function");
    let scalar = emit_expr(context, builder, module, rt, user_functions, locals, value);
    let scalar = heap_rc::retain_if_heap_duplicate(context, builder, rt, value, scalar);
    let scalar = retain_if_int_duplicate(context, builder, rt, value, scalar);
    let scalar = coerce_scalar_to_type(context, builder, scalar, frame.element.clone());
    let slot = byte_ptr(context, builder, frame.object, VALUE_OFFSET);
    builder
        .build_store(slot, scalar_basic(scalar))
        .expect("build_store should not fail for the value area");
    suspend_and_resume(context, module, builder, rt, "gen_resumed");
}

/// The LLVM value of `scalar`.
fn scalar_basic(scalar: Scalar<'_>) -> BasicValueEnum<'_> {
    match scalar {
        Scalar::Int(v) | Scalar::Bool(v) => v.into(),
        Scalar::Float(v) => v.into(),
        Scalar::Str(v)
        | Scalar::List(v)
        | Scalar::Dict(v)
        | Scalar::Set(v)
        | Scalar::Instance(v)
        | Scalar::Object(v)
        | Scalar::MemoryView(v)
        | Scalar::Generator(v)
        | Scalar::Closure(v) => v.into(),
        Scalar::Tuple(v) | Scalar::Optional(v) => v.into(),
    }
}

/// The `Scalar` of a value of type `ty` loaded as `value`.
fn basic_scalar<'ctx>(ty: &Ty, value: BasicValueEnum<'ctx>) -> Scalar<'ctx> {
    match ty {
        Ty::Int => Scalar::Int(value.into_int_value()),
        Ty::Bool | Ty::None => Scalar::Bool(value.into_int_value()),
        Ty::Float => Scalar::Float(value.into_float_value()),
        Ty::Str => Scalar::Str(value.into_pointer_value()),
        Ty::List(_) => Scalar::List(value.into_pointer_value()),
        Ty::Tuple(elements) if pycc_hir::tuple_is_boxed(elements) => {
            Scalar::List(value.into_pointer_value())
        }
        Ty::Tuple(_) => Scalar::Tuple(value.into_struct_value()),
        Ty::Optional(_) => Scalar::Optional(value.into_struct_value()),
        Ty::Dict(_) => Scalar::Dict(value.into_pointer_value()),
        Ty::Set(_) | Ty::FrozenSet(_) => Scalar::Set(value.into_pointer_value()),
        Ty::Instance(_) | Ty::Protocol(_) => Scalar::Instance(value.into_pointer_value()),
        Ty::Object => Scalar::Object(value.into_pointer_value()),
        Ty::Generator(_) => Scalar::Generator(value.into_pointer_value()),
        Ty::Callable(_) => Scalar::Closure(value.into_pointer_value()),
        Ty::MemoryView => Scalar::MemoryView(value.into_pointer_value()),
        Ty::Infer | Ty::Param(_) => {
            panic!("pycc_codegen: internal error: an unresolved generator element type")
        }
    }
}

/// The value the generator `object` last yielded (an owned reference).
pub(crate) fn load_value<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    object: PointerValue<'ctx>,
    element: &Ty,
) -> Scalar<'ctx> {
    let slot = byte_ptr(context, builder, object, VALUE_OFFSET);
    let value = builder
        .build_load(
            ty_to_basic_type(context, element.clone()),
            slot,
            "gen_value",
        )
        .expect("build_load should not fail for the value area");
    basic_scalar(element, value)
}

/// Turns every `ret void` of the generator function `function` into a
/// branch to its final suspend, and emits the final suspend, the frame's
/// cleanup and the return to the caller.
pub(crate) fn finish<'ctx>(
    context: &'ctx Context,
    module: &inkwell::module::Module<'ctx>,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    function: FunctionValue<'ctx>,
) {
    let frame = rt
        .gen_frame
        .borrow_mut()
        .take()
        .expect("a generator function's frame was begun");
    let final_bb = context.append_basic_block(function, "gen_final");
    let returns: Vec<inkwell::values::InstructionValue<'ctx>> = function
        .get_basic_block_iter()
        .filter_map(|block| block.get_terminator())
        .filter(|terminator| {
            terminator.get_opcode() == InstructionOpcode::Return
                && terminator.get_num_operands() == 0
        })
        .collect();
    for ret in returns {
        builder.position_before(&ret);
        builder
            .build_unconditional_branch(final_bb)
            .expect("build_unconditional_branch should not fail replacing a return");
        ret.erase_from_basic_block();
    }
    // The final suspend: the object is done; the runtime destroys the frame.
    builder.position_at_end(final_bb);
    let state_ptr = byte_ptr(context, builder, frame.object, STATE_OFFSET);
    builder
        .build_store(state_ptr, context.i8_type().const_int(DONE, false))
        .expect("build_store should not fail for the state byte");
    let state = raw_call(
        builder,
        intrinsic(module, "llvm.coro.suspend", &[]),
        &mut [
            token_none(context),
            context.bool_type().const_all_ones().as_value_ref(),
        ],
        "gen_final_suspended",
    );
    // SAFETY: `llvm.coro.suspend` returns an `i8`.
    let state = unsafe { IntValue::new(state) };
    let never = context.append_basic_block(function, "gen_resumed_after_final");
    builder
        .build_switch(
            state,
            frame.suspend_bb,
            &[
                (context.i8_type().const_zero(), never),
                (context.i8_type().const_int(1, false), frame.cleanup_bb),
            ],
        )
        .expect("build_switch should not fail for the final suspend");
    builder.position_at_end(never);
    builder
        .build_unreachable()
        .expect("build_unreachable should not fail");
    // The destroy case: free the frame.
    builder.position_at_end(frame.cleanup_bb);
    let memory = raw_call(
        builder,
        intrinsic(module, "llvm.coro.free", &[]),
        &mut [frame.id, frame.handle.as_value_ref()],
        "gen_frame_memory",
    );
    // SAFETY: `llvm.coro.free` returns a pointer.
    let memory = unsafe { PointerValue::new(memory) };
    builder
        .build_call(rt.gens.frame_free, &[memory.into()], "")
        .expect("build_call should not fail for the frame free");
    builder
        .build_unconditional_branch(frame.suspend_bb)
        .expect("build_unconditional_branch should not fail");
    // Every suspend returns the object to whoever resumed (the ramp's
    // caller receives it; the split resume functions return nothing).
    builder.position_at_end(frame.suspend_bb);
    raw_call(
        builder,
        intrinsic(module, "llvm.coro.end", &[]),
        &mut [
            frame.handle.as_value_ref(),
            context.bool_type().const_zero().as_value_ref(),
            token_none(context),
        ],
        "",
    );
    builder
        .build_return(Some(&frame.object))
        .expect("build_return should not fail for the ramp's return");
}

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
    let answer = builder
        .build_call(rt.gens.resume, &[object.into()], "gen_resume")
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
    rt.loops.enter(stmt, after_bb, test_bb, finally_stack.len());
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
fn operand<'ctx>(
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
fn release_operand<'ctx>(
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
        builder
            .build_call(rt.gens.next, &[object.into()], "gen_next")
            .expect("build_call should not fail for pycc_rt_gen_next");
        guard_statement_effects(context, builder, rt);
        let value = load_value(context, builder, object, ty);
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
    resume_branch(context, builder, rt, object, got_bb, done_bb);
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
    let Ty::Generator(element) = generator.ty() else {
        panic!("pycc_codegen: internal error: any()/all() of a non-generator")
    };
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
