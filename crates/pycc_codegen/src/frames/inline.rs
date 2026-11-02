//! A live frame's inline link and unlink (`frames`): the work of
//! `pycc_ext_frame_enter` and `pycc_ext_frame_exit` without the call once a
//! function's code object exists, for CPython 3.13's and 3.15's
//! `_PyInterpreterFrame` layouts (`src/ext/module/frames.c`'s `PyccIFrame`),
//! picked at run time by the shim's `pycc_ext_frames_v315` flag.

use super::*;

/// A field of the frame at `frame` (`PyccIFrame`, `src/ext/module/frames.c`)
/// at byte `offset`.
fn field<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    base: inkwell::values::PointerValue<'ctx>,
    offset: u64,
) -> inkwell::values::PointerValue<'ctx> {
    unsafe {
        builder.build_in_bounds_gep(
            context.i8_type(),
            base,
            &[context.i64_type().const_int(offset, false)],
            "frame_field",
        )
    }
    .expect("build_in_bounds_gep should not fail for a frame field")
}

/// Whether the running interpreter's frames are CPython 3.15's
/// (`pycc_ext_frames_v315`, set by the shim's frames probe before any frame
/// is linked inline), as an `i1`.
fn frames_v315<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    state: &FrameState<'ctx>,
) -> inkwell::values::IntValue<'ctx> {
    let i8_type = context.i8_type();
    let flag = state.frames_v315;
    let value = builder
        .build_load(i8_type, flag.as_pointer_value(), "frames_v315")
        .expect("build_load should not fail for the frame layout flag")
        .into_int_value();
    builder
        .build_int_compare(
            inkwell::IntPredicate::NE,
            value,
            i8_type.const_zero(),
            "is_v315",
        )
        .expect("build_int_compare should not fail")
}

/// The field at `offsets` (3.13's, 3.15's) of the frame at `base`, per
/// `v315` ([`frames_v315`]).
fn layout_field<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    base: inkwell::values::PointerValue<'ctx>,
    v315: inkwell::values::IntValue<'ctx>,
    offsets: (u64, u64),
) -> inkwell::values::PointerValue<'ctx> {
    let i64_type = context.i64_type();
    let offset = builder
        .build_select(
            v315,
            i64_type.const_int(offsets.1, false),
            i64_type.const_int(offsets.0, false),
            "frame_offset",
        )
        .expect("build_select should not fail for a frame offset")
        .into_int_value();
    unsafe { builder.build_in_bounds_gep(context.i8_type(), base, &[offset], "frame_field") }
        .expect("build_in_bounds_gep should not fail for a frame field")
}

/// Byte offsets in `_PyInterpreterFrame` (`PyccIFrame`), the same in
/// CPython 3.13 and 3.15 up to `instr_ptr`.
const F_EXECUTABLE: u64 = 0;
const F_PREVIOUS: u64 = 8;
const F_FUNCOBJ: u64 = 16;
const F_GLOBALS: u64 = 24;
const F_BUILTINS: u64 = 32;
const F_LOCALS: u64 = 40;
const F_FRAME_OBJ: u64 = 48;
const F_INSTR_PTR: u64 = 56;
/// Past `instr_ptr` the versions differ (3.13, 3.15): 3.13's `stacktop`,
/// `return_offset` and `owner` share the word at 64, which 3.15's
/// `stackpointer` fills; 3.15's `return_offset`, `owner` and `visited`
/// share the word at 72, 3.13's `localsplus[0]`.
const F_TAIL: u64 = 64;
const F_TAIL_315: u64 = 72;
const F_OWNER: (u64, u64) = (70, 74);
/// `localsplus[0]` and `[1]`: the thread state, and the top it replaced.
const F_TSTATE: (u64, u64) = (72, 80);
const F_SAVED_TOP: (u64, u64) = (80, 88);
/// `PyThreadState.current_frame`, and `PyCodeObject.co_flags`.
const TSTATE_CURRENT_FRAME: u64 = 72;
const CODE_FLAGS: u64 = 48;
/// The `co_flags` bit of a signature wrapper's or forwarder's code
/// (`pycc_ast::ext_signatures::WRAPPER_FLAG`).
const CO_SIGNATURE: u64 = 0x4000_0000;
/// `PyccIFrame.owner` of a frame `pycc_ext_frame_enter` did not link.
const OWNER_INACTIVE: u64 = 0x7F;

/// Links `frame` for `info` onto the thread's frame chain, inline:
/// `pycc_ext_frame_enter`'s work without the call once `info`'s code
/// object exists (that first call makes it).
pub(super) fn link_frame<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    state: &FrameState<'ctx>,
    function: FunctionValue<'ctx>,
    frame: inkwell::values::PointerValue<'ctx>,
    info: GlobalValue<'ctx>,
) -> Option<()> {
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    let i8_type = context.i8_type();
    let i32_type = context.i32_type();
    let info = info.as_pointer_value();
    let load = |at: inkwell::values::PointerValue<'ctx>, name: &str| {
        builder
            .build_load(ptr, at, name)
            .expect("build_load should not fail for a frame word")
            .into_pointer_value()
    };
    let slow = context.append_basic_block(function, "frame_enter_slow");
    let fast = context.append_basic_block(function, "frame_enter_fast");
    let check = context.append_basic_block(function, "frame_enter_wrapper");
    let skip = context.append_basic_block(function, "frame_enter_skip");
    let link = context.append_basic_block(function, "frame_enter_link");
    let done = context.append_basic_block(function, "frame_enter_done");
    let code = load(info, "frame_code");
    let ready = builder.build_is_not_null(code, "frame_ready").ok()?;
    let branch = builder.build_conditional_branch(ready, fast, slow).ok()?;
    crate::int_fast::weight_branch(context, branch, true);
    builder.position_at_end(slow);
    builder
        .build_call(state.frame_enter, &[frame.into(), info.into()], "")
        .ok()?;
    builder.build_unconditional_branch(done).ok()?;
    builder.position_at_end(fast);
    let tstate = builder
        .build_call(state.thread_state, &[], "tstate")
        .ok()?
        .try_as_basic_value()
        .basic()?
        .into_pointer_value();
    let v315 = frames_v315(context, builder, state);
    let current = field(context, builder, tstate, TSTATE_CURRENT_FRAME);
    let top = load(current, "frame_top");
    let has_top = builder.build_is_not_null(top, "frame_has_top").ok()?;
    builder
        .build_conditional_branch(has_top, check, link)
        .ok()?;
    // A signature wrapper or forwarder on top stands for this function:
    // link past it.
    builder.position_at_end(check);
    let owner = builder
        .build_load(
            i8_type,
            layout_field(context, builder, top, v315, F_OWNER),
            "top_owner",
        )
        .ok()?
        .into_int_value();
    let threaded = builder
        .build_int_compare(
            inkwell::IntPredicate::EQ,
            owner,
            i8_type.const_zero(),
            "top_thread",
        )
        .ok()?;
    let probe = context.append_basic_block(function, "frame_enter_flags");
    builder
        .build_conditional_branch(threaded, probe, link)
        .ok()?;
    builder.position_at_end(probe);
    let top_code = load(field(context, builder, top, F_EXECUTABLE), "top_code");
    let flags = builder
        .build_load(
            i32_type,
            field(context, builder, top_code, CODE_FLAGS),
            "top_flags",
        )
        .ok()?
        .into_int_value();
    let wrapper = builder
        .build_and(
            flags,
            i32_type.const_int(CO_SIGNATURE, false),
            "top_wrapper",
        )
        .ok()?;
    let is_wrapper = builder
        .build_int_compare(
            inkwell::IntPredicate::NE,
            wrapper,
            i32_type.const_zero(),
            "is_wrapper",
        )
        .ok()?;
    builder
        .build_conditional_branch(is_wrapper, skip, link)
        .ok()?;
    builder.position_at_end(skip);
    let below = load(field(context, builder, top, F_PREVIOUS), "top_previous");
    builder.build_unconditional_branch(link).ok()?;
    builder.position_at_end(link);
    let previous = builder.build_phi(ptr, "frame_previous").ok()?;
    previous.add_incoming(&[(&top, fast), (&top, check), (&top, probe), (&below, skip)]);
    let store = |offset: u64, value: inkwell::values::BasicValueEnum<'ctx>| {
        builder
            .build_store(field(context, builder, frame, offset), value)
            .expect("build_store should not fail for a frame field");
    };
    let word = |index: u32, name: &str| -> inkwell::values::BasicValueEnum<'ctx> {
        let at = builder
            .build_struct_gep(info_type(context), info, index, name)
            .expect("build_struct_gep should not fail for PyccFnInfo");
        load(at, name).into()
    };
    store(F_EXECUTABLE, code.into());
    store(F_PREVIOUS, previous.as_basic_value());
    store(F_FUNCOBJ, word(1, "fninfo_func"));
    store(F_GLOBALS, word(2, "fninfo_globals"));
    store(F_BUILTINS, word(3, "fninfo_builtins"));
    store(F_LOCALS, ptr.const_null().into());
    store(F_FRAME_OBJ, ptr.const_null().into());
    store(F_INSTR_PTR, word(4, "fninfo_ip0"));
    // An empty stack, no call in progress, owned by the thread: 3.13's
    // `stacktop`, `return_offset` and `owner` all zero, or 3.15's
    // `stackpointer` at `localsplus` and the next word (`return_offset`,
    // `owner`, `visited`) zero -- 3.13's `localsplus[0]`, stored below.
    let i64_type = context.i64_type();
    let localsplus = builder
        .build_ptr_to_int(
            field(context, builder, frame, F_TSTATE.1),
            i64_type,
            "localsplus",
        )
        .ok()?;
    let tail = builder
        .build_select(v315, localsplus, i64_type.const_zero(), "frame_tail")
        .ok()?;
    store(F_TAIL, tail);
    store(F_TAIL_315, i64_type.const_zero().into());
    builder
        .build_store(
            layout_field(context, builder, frame, v315, F_TSTATE),
            tstate,
        )
        .ok()?;
    builder
        .build_store(
            layout_field(context, builder, frame, v315, F_SAVED_TOP),
            top,
        )
        .ok()?;
    builder.build_store(current, frame).ok()?;
    builder.build_unconditional_branch(done).ok()?;
    builder.position_at_end(done);
    Some(())
}

/// Unlinks the live frame before the return `terminator`, inline: the
/// top it replaced goes back unless the frame is inactive or a frame
/// object was made for it, which `pycc_ext_frame_exit` handles.
pub(super) fn unlink_frame<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    live: LiveFrame<'ctx>,
    terminator: inkwell::values::InstructionValue<'ctx>,
) {
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    let i8_type = context.i8_type();
    let Some(block) = terminator.get_parent() else {
        return;
    };
    let function = live.function;
    // The return moves to a block of its own, after the unlink.
    let tail = block.get_next_basic_block();
    let ret = match tail {
        Some(next) => context.prepend_basic_block(next, "frame_exit_ret"),
        None => context.append_basic_block(function, "frame_exit_ret"),
    };
    let slow = context.append_basic_block(function, "frame_exit_slow");
    let fast = context.append_basic_block(function, "frame_exit_fast");
    let check = context.append_basic_block(function, "frame_exit_check");
    builder.position_before(&terminator);
    builder
        .build_unconditional_branch(check)
        .expect("build_unconditional_branch should not fail");
    terminator.remove_from_basic_block();
    builder.position_at_end(ret);
    builder.insert_instruction(&terminator, None);
    builder.position_at_end(check);
    let v315 = frames_v315(context, builder, &rt.frames);
    let owner = builder
        .build_load(
            i8_type,
            layout_field(context, builder, live.frame, v315, F_OWNER),
            "frame_owner",
        )
        .expect("build_load should not fail")
        .into_int_value();
    let inactive = builder
        .build_int_compare(
            inkwell::IntPredicate::EQ,
            owner,
            i8_type.const_int(OWNER_INACTIVE, false),
            "frame_inactive",
        )
        .expect("build_int_compare should not fail");
    let object = builder
        .build_load(
            ptr,
            field(context, builder, live.frame, F_FRAME_OBJ),
            "frame_obj",
        )
        .expect("build_load should not fail")
        .into_pointer_value();
    let has_object = builder
        .build_is_not_null(object, "frame_has_obj")
        .expect("build_is_not_null should not fail");
    let unusual = builder
        .build_or(inactive, has_object, "frame_exit_unusual")
        .expect("build_or should not fail");
    let branch = builder
        .build_conditional_branch(unusual, slow, fast)
        .expect("build_conditional_branch should not fail");
    crate::int_fast::weight_branch(context, branch, false);
    builder.position_at_end(slow);
    builder
        .build_call(rt.frames.frame_exit, &[live.frame.into()], "")
        .expect("build_call should not fail for pycc_ext_frame_exit");
    builder
        .build_unconditional_branch(ret)
        .expect("build_unconditional_branch should not fail");
    builder.position_at_end(fast);
    let tstate = builder
        .build_load(
            ptr,
            layout_field(context, builder, live.frame, v315, F_TSTATE),
            "frame_tstate",
        )
        .expect("build_load should not fail")
        .into_pointer_value();
    let top = builder
        .build_load(
            ptr,
            layout_field(context, builder, live.frame, v315, F_SAVED_TOP),
            "frame_saved_top",
        )
        .expect("build_load should not fail");
    builder
        .build_store(field(context, builder, tstate, TSTATE_CURRENT_FRAME), top)
        .expect("build_store should not fail");
    builder
        .build_unconditional_branch(ret)
        .expect("build_unconditional_branch should not fail");
}
