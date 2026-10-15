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
//! each value it leaves in the value area ([`consume`]). The rest of the
//! generator protocol -- `send()`, `throw()`, `close()`, a `yield` used as
//! a value, `return value` and `yield from` -- is [`protocol`].

mod consume;
mod protocol;

pub(crate) use consume::{emit_any_all, emit_for_gen_stmt, emit_next, emit_to_list, release_owned};
pub(crate) use protocol::emit_gen_return;

/// A generator-protocol expression: `MirExpr::GenYield`, `GenYieldFrom`,
/// `GenSend`, `GenThrow`, `GenClose` or `StopIterationValue`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_protocol_expr<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    expr: &MirExpr,
) -> Scalar<'ctx> {
    match expr {
        MirExpr::GenYield { value, .. } => emit_yield(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            value,
            true,
        ),
        MirExpr::GenYieldFrom { sub, ty } => protocol::emit_yield_from(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            sub,
            ty,
        ),
        MirExpr::GenSend {
            generator, value, ..
        } => protocol::emit_send(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            generator,
            value,
        ),
        MirExpr::GenThrow {
            generator,
            exception,
            ..
        } => protocol::emit_throw(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            generator,
            exception,
        ),
        MirExpr::GenClose { generator } => protocol::emit_close(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            generator,
        ),
        MirExpr::StopIterationValue { exception, ty } => {
            let scalar = emit_expr(
                context,
                builder,
                module,
                rt,
                user_functions,
                locals,
                exception,
            );
            let pointer = match scalar {
                Scalar::Instance(ptr) | Scalar::Object(ptr) => ptr,
                _ => panic!("pycc_codegen: internal error: `e.value` of a non-exception"),
            };
            protocol::emit_stop_iteration_value(context, builder, rt, pointer, ty)
        }
        _ => unreachable!("emit_protocol_expr is called for generator-protocol nodes only"),
    }
}

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
/// `pycc_rt::generator::MODE_OFFSET`.
const MODE_OFFSET: u64 = 17;
/// `pycc_rt::generator::SENT_OFFSET`.
const SENT_OFFSET: u64 = 19;
/// `pycc_rt::generator::HAS_RET_OFFSET`.
const HAS_RET_OFFSET: u64 = 20;
/// `pycc_rt::generator::VALUE_OFFSET`.
const VALUE_OFFSET: u64 = 64;
/// `pycc_rt::generator`'s `DONE` state.
const DONE: u64 = 2;

/// The runtime functions of generator objects.
pub(crate) struct GenFns<'ctx> {
    pub(crate) new: FunctionValue<'ctx>,
    pub(crate) resume: FunctionValue<'ctx>,
    pub(crate) step: FunctionValue<'ctx>,
    pub(crate) can_send: FunctionValue<'ctx>,
    pub(crate) throw: FunctionValue<'ctx>,
    pub(crate) close: FunctionValue<'ctx>,
    pub(crate) delegate: FunctionValue<'ctx>,
    pub(crate) raise_mode: FunctionValue<'ctx>,
    pub(crate) raise_nothing_sent: FunctionValue<'ctx>,
    pub(crate) raise_stop: FunctionValue<'ctx>,
    pub(crate) raise_no_return: FunctionValue<'ctx>,
    pub(crate) stop_unless_raised: FunctionValue<'ctx>,
    pub(crate) exception_payload: FunctionValue<'ctx>,
    pub(crate) incref: FunctionValue<'ctx>,
    pub(crate) decref: FunctionValue<'ctx>,
    pub(crate) frame_alloc: FunctionValue<'ctx>,
    pub(crate) frame_free: FunctionValue<'ctx>,
    /// `pycc_rt_gen_repr`: `str(g)`.
    pub(crate) repr: FunctionValue<'ctx>,
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
                ptr_type.fn_type(
                    &[
                        ptr_type.into(),
                        i64_type.into(),
                        ptr_type.into(),
                        ptr_type.into(),
                        ptr_type.into(),
                    ],
                    false,
                ),
            ),
            resume: declare(
                "pycc_rt_gen_resume",
                i8_type.fn_type(&[ptr_type.into()], false),
            ),
            repr: declare(
                "pycc_rt_gen_repr",
                ptr_type.fn_type(&[ptr_type.into()], false),
            ),
            step: declare(
                "pycc_rt_gen_step",
                i8_type.fn_type(&[ptr_type.into(), i8_type.into()], false),
            ),
            can_send: declare(
                "pycc_rt_gen_can_send",
                i8_type.fn_type(&[ptr_type.into(), i8_type.into()], false),
            ),
            throw: declare(
                "pycc_rt_gen_throw",
                i8_type.fn_type(&[ptr_type.into(), ptr_type.into()], false),
            ),
            close: declare(
                "pycc_rt_gen_close",
                i8_type.fn_type(&[ptr_type.into()], false),
            ),
            delegate: declare(
                "pycc_rt_gen_delegate",
                i8_type.fn_type(
                    &[
                        ptr_type.into(),
                        ptr_type.into(),
                        i8_type.into(),
                        i8_type.into(),
                        i64_type.into(),
                        i64_type.into(),
                    ],
                    false,
                ),
            ),
            raise_mode: declare(
                "pycc_rt_gen_raise_mode",
                void_type.fn_type(&[ptr_type.into()], false),
            ),
            raise_nothing_sent: declare(
                "pycc_rt_gen_raise_nothing_sent",
                void_type.fn_type(&[], false),
            ),
            raise_stop: declare(
                "pycc_rt_raise_stop_iteration",
                void_type.fn_type(
                    &[
                        i64_type.into(),
                        ptr_type.into(),
                        i64_type.into(),
                        context.i8_type().into(),
                    ],
                    false,
                ),
            ),
            raise_no_return: declare("pycc_rt_gen_raise_no_return", void_type.fn_type(&[], false)),
            stop_unless_raised: declare(
                "pycc_rt_gen_stop_unless_raised",
                void_type.fn_type(&[], false),
            ),
            exception_payload: declare(
                "pycc_rt_exception_payload",
                i64_type.fn_type(&[ptr_type.into(), i64_type.into()], false),
            ),
            incref: declare(
                "pycc_rt_gen_incref",
                void_type.fn_type(&[ptr_type.into()], false),
            ),
            decref: declare(
                "pycc_rt_gen_decref",
                void_type.fn_type(&[ptr_type.into()], false),
            ),
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
    /// The generator's `(yield, send, return)` types.
    sig: (Ty, Ty, Ty),
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

/// The size of a value of `ty` in a generator's value area (`None`: none).
pub(super) fn area_size<'ctx>(context: &'ctx Context, ty: &Ty) -> IntValue<'ctx> {
    if matches!(ty, Ty::None) {
        return context.i64_type().const_zero();
    }
    ty_to_basic_type(context, ty.clone())
        .size_of()
        .expect("a sized generator value type")
}

/// The function releasing a value of `ty` stored at its pointer argument
/// (`pycc_rt::generator`'s drop hooks), or a null pointer when `ty` holds
/// no reference. One function per type, shared by every generator.
fn drop_fn<'ctx>(
    context: &'ctx Context,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    ty: &Ty,
) -> PointerValue<'ctx> {
    let ptr_type = context.ptr_type(inkwell::AddressSpace::default());
    let counted = matches!(ty, Ty::Int) || heap_rc::heap_kind(rt, ty).is_some();
    if !counted || matches!(ty, Ty::None) {
        return ptr_type.const_null();
    }
    let name = format!("pycc_gen_drop.{}", ty.name());
    if let Some(existing) = module.get_function(&name) {
        return existing.as_global_value().as_pointer_value();
    }
    let function = module.add_function(
        &name,
        context.void_type().fn_type(&[ptr_type.into()], false),
        Some(Linkage::Internal),
    );
    let builder = context.create_builder();
    builder.position_at_end(context.append_basic_block(function, "entry"));
    let slot = function
        .get_nth_param(0)
        .expect("the drop function takes the value's address")
        .into_pointer_value();
    let value = builder
        .build_load(
            ty_to_basic_type(context, ty.clone()),
            slot,
            "gen_drop_value",
        )
        .expect("build_load should not fail for a generator value");
    release_owned(context, &builder, rt, ty, &basic_scalar(ty, value));
    builder
        .build_return(None)
        .expect("build_return should not fail for a drop function");
    function.as_global_value().as_pointer_value()
}

/// The larger of two sizes.
fn max_size<'ctx>(
    builder: &inkwell::builder::Builder<'ctx>,
    a: IntValue<'ctx>,
    b: IntValue<'ctx>,
) -> IntValue<'ctx> {
    let a_larger = builder
        .build_int_compare(IntPredicate::UGT, a, b, "gen_size_cmp")
        .expect("build_int_compare should not fail for sizes");
    builder
        .build_select(a_larger, a, b, "gen_size_max")
        .expect("build_select should not fail for sizes")
        .into_int_value()
}

/// Creates the generator object and suspends before the body, at the
/// builder's position (the parameters bound, the exception exit installed);
/// leaves the builder where the body starts. Records the frame in `rt` for
/// [`emit_yield`] and [`finish`].
#[allow(clippy::too_many_arguments)]
pub(crate) fn initial_suspend<'ctx>(
    context: &'ctx Context,
    module: &inkwell::module::Module<'ctx>,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    function: FunctionValue<'ctx>,
    name: &str,
    begun: GenBegin<'ctx>,
    sig: (Ty, Ty, Ty),
) {
    let value_size = max_size(
        builder,
        area_size(context, &sig.0),
        max_size(
            builder,
            area_size(context, &sig.1),
            area_size(context, &sig.2),
        ),
    );
    let drop_yield = drop_fn(context, module, rt, &sig.0);
    let drop_return = drop_fn(context, module, rt, &sig.2);
    let object = builder
        .build_call(
            rt.gens.new,
            &[
                begun.handle.into(),
                value_size.into(),
                drop_yield.into(),
                drop_return.into(),
                crate::closure::qualname_ptr(module, name).into(),
            ],
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
        sig,
        suspend_bb,
        cleanup_bb,
    };
    *rt.gen_frame.borrow_mut() = Some(frame);
    suspend_and_resume(context, module, builder, rt, "gen_start");
}

/// The frame of the generator function being emitted.
pub(super) fn current_frame<'ctx>(rt: &RtFns<'ctx>) -> GenFrame<'ctx> {
    rt.gen_frame
        .borrow()
        .clone()
        .expect("generator code is emitted inside a generator function")
}

/// Emits a non-final suspend point and leaves the builder where the body
/// resumes, without looking at why it was resumed (`yield from`, which
/// forwards that to the generator it delegates to).
pub(super) fn suspend_raw<'ctx>(
    context: &'ctx Context,
    module: &inkwell::module::Module<'ctx>,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    label: &str,
) {
    let frame = current_frame(rt);
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
}

/// Emits a non-final suspend point and leaves the builder where the body
/// resumes; a generator being closed raises `GeneratorExit` there, one a
/// value is thrown into raises that value.
fn suspend_and_resume<'ctx>(
    context: &'ctx Context,
    module: &inkwell::module::Module<'ctx>,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    label: &str,
) {
    suspend_raw(context, module, builder, rt, label);
    let frame = current_frame(rt);
    let function = builder.get_insert_block().unwrap().get_parent().unwrap();
    let mode = load_byte(context, builder, frame.object, MODE_OFFSET, "gen_mode");
    let raising = builder
        .build_int_compare(
            IntPredicate::NE,
            mode,
            context.i8_type().const_zero(),
            "gen_is_raising",
        )
        .expect("build_int_compare should not fail for an i8 flag");
    let raise_bb = context.append_basic_block(function, "gen_raise_mode");
    let run_bb = context.append_basic_block(function, "gen_run");
    builder
        .build_conditional_branch(raising, raise_bb, run_bb)
        .expect("build_conditional_branch should not fail for an i1");
    builder.position_at_end(raise_bb);
    builder
        .build_call(rt.gens.raise_mode, &[frame.object.into()], "")
        .expect("build_call should not fail for pycc_rt_gen_raise_mode");
    crate::exception::jump_to_exception_target(context, builder, rt);
    builder.position_at_end(run_bb);
}

/// At the end of a native program, releases every module global holding a
/// generator, in the order the module first assigns them -- CPython clears
/// the module's globals at shutdown, closing each unfinished generator
/// (running its `finally` clauses) in that order.
pub(crate) fn release_generator_globals<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    mir: &pycc_mir::MirModule,
    module_globals: &std::collections::BTreeMap<String, StorageSlot<'ctx>>,
) {
    fn assigned(stmts: &[MirStmt], out: &mut Vec<String>) {
        for stmt in stmts {
            if let MirStmt::Assign { target, .. } = stmt
                && !out.contains(target)
            {
                out.push(target.clone());
            }
            for body in stmt.bodies() {
                assigned(body, out);
            }
        }
    }
    let mut order = Vec::new();
    for item in &mir.items {
        if let pycc_mir::MirItem::TopLevelStmt(stmt) = item {
            assigned(std::slice::from_ref(stmt), &mut order);
        }
    }
    for name in order {
        let Some(slot) = module_globals.get(&name) else {
            continue;
        };
        if !matches!(slot.ty, Ty::Generator(_)) {
            continue;
        }
        heap_rc::release_slot(context, builder, rt, &slot.ty, slot.ptr);
        builder
            .build_store(
                slot.ptr,
                context
                    .ptr_type(inkwell::AddressSpace::default())
                    .const_null(),
            )
            .expect("build_store should not fail for a released global");
    }
}

/// The byte at `offset` in the generator `object`.
pub(super) fn load_byte<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    object: PointerValue<'ctx>,
    offset: u64,
    name: &str,
) -> IntValue<'ctx> {
    let ptr = byte_ptr(context, builder, object, offset);
    builder
        .build_load(context.i8_type(), ptr, name)
        .expect("build_load should not fail for a generator header byte")
        .into_int_value()
}

/// Stores `value` into the byte at `offset` in the generator `object`.
pub(super) fn store_byte<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    object: PointerValue<'ctx>,
    offset: u64,
    value: u64,
) {
    let ptr = byte_ptr(context, builder, object, offset);
    builder
        .build_store(ptr, context.i8_type().const_int(value, false))
        .expect("build_store should not fail for a generator header byte");
}

/// A pointer `offset` bytes into `object`.
pub(super) fn byte_ptr<'ctx>(
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
/// element type, into the object's value area and suspends. Returns the
/// value sent in when the body resumes (`x = yield value`), owned by the
/// body: `None` for a generator whose send type is `None`, else the value
/// `send()` stored in the area (a resume by `next()`, which sends nothing,
/// raises `TypeError` there -- `docs/DEVIATIONS.md`).
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_yield<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    value: &MirExpr,
    want_value: bool,
) -> Scalar<'ctx> {
    let frame = current_frame(rt);
    let scalar = emit_expr(context, builder, module, rt, user_functions, locals, value);
    let scalar = heap_rc::retain_if_heap_duplicate(context, builder, rt, value, scalar);
    let scalar = retain_if_int_duplicate(context, builder, rt, value, scalar);
    let scalar = coerce_scalar_to_type(context, builder, scalar, frame.sig.0.clone());
    let slot = byte_ptr(context, builder, frame.object, VALUE_OFFSET);
    builder
        .build_store(slot, scalar_basic(scalar))
        .expect("build_store should not fail for the value area");
    suspend_and_resume(context, module, builder, rt, "gen_resumed");
    let sent_ty = frame.sig.1.clone();
    if matches!(sent_ty, Ty::None) {
        return none_scalar(context);
    }
    let function = builder.get_insert_block().unwrap().get_parent().unwrap();
    let sent = load_byte(context, builder, frame.object, SENT_OFFSET, "gen_sent");
    if !want_value {
        // `yield value` as a statement: a value sent in is released.
        let release_bb = context.append_basic_block(function, "gen_release_sent");
        let run_bb = context.append_basic_block(function, "gen_after_yield");
        let was_sent = builder
            .build_int_compare(
                IntPredicate::NE,
                sent,
                context.i8_type().const_zero(),
                "gen_was_sent",
            )
            .expect("build_int_compare should not fail for an i8 flag");
        builder
            .build_conditional_branch(was_sent, release_bb, run_bb)
            .expect("build_conditional_branch should not fail for an i1");
        builder.position_at_end(release_bb);
        let value = load_value(context, builder, frame.object, &sent_ty);
        release_owned(context, builder, rt, &sent_ty, &value);
        builder
            .build_unconditional_branch(run_bb)
            .expect("build_unconditional_branch should not fail");
        builder.position_at_end(run_bb);
        return none_scalar(context);
    }
    let was_sent = builder
        .build_int_compare(
            IntPredicate::NE,
            sent,
            context.i8_type().const_zero(),
            "gen_was_sent",
        )
        .expect("build_int_compare should not fail for an i8 flag");
    let nothing_bb = context.append_basic_block(function, "gen_nothing_sent");
    let got_bb = context.append_basic_block(function, "gen_got_sent");
    builder
        .build_conditional_branch(was_sent, got_bb, nothing_bb)
        .expect("build_conditional_branch should not fail for an i1");
    builder.position_at_end(nothing_bb);
    builder
        .build_call(rt.gens.raise_nothing_sent, &[], "")
        .expect("build_call should not fail for pycc_rt_gen_raise_nothing_sent");
    crate::exception::jump_to_exception_target(context, builder, rt);
    builder.position_at_end(got_bb);
    load_value(context, builder, frame.object, &sent_ty)
}

/// The value `None`, as `MirExpr::NoneLiteral` emits it.
pub(super) fn none_scalar(context: &Context) -> Scalar<'_> {
    let placeholder_ty = context.struct_type(&[context.i8_type().into(); 2], false);
    Scalar::Optional(placeholder_ty.const_zero())
}

/// The LLVM value of `scalar`.
pub(super) fn scalar_basic(scalar: Scalar<'_>) -> BasicValueEnum<'_> {
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
pub(super) fn basic_scalar<'ctx>(ty: &Ty, value: BasicValueEnum<'ctx>) -> Scalar<'ctx> {
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
        Ty::Infer | Ty::Param(_) | Ty::NdArray(_) => {
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
