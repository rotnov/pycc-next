//! Publishing the exception an `except` handler handles (an `--ext`
//! module): `sys.exception()`, `sys.exc_info()` and
//! `traceback.format_exc()` see it while the handler runs, and an exception
//! raised there gets it as its `__context__`, as in CPython.
//!
//! A handler's entry does not publish. It keeps the caught exception in the
//! handler's record ([`State`], the shim's `PyccExtHandler`), and the
//! handler's body publishes it the first time something could see it: once
//! the body is emitted, [`Region::publish_where_seen`] walks the
//! instructions it added and puts a call of `pycc_ext_handler_publish`
//! before every call that can run other code or raise. So neither the path
//! without an exception nor a handler that only returns a default pays for
//! a CPython exception, a traceback or the thread's handled-exception
//! state.
//!
//! What cannot see it, and gets no call:
//!
//! * a retain, a native `str` count, the pending-exception test, a
//!   traceback record, a bare `raise`, the bridge of an exception a call
//!   before it raised (the calls of [`SILENT`]);
//! * the handler's own way out, which releases the record's slots
//!   ([`exits`]): CPython restores the previous exception before it drops
//!   the handled one;
//! * a release that frees nothing: in a handler's body a release is
//!   `pycc_ext_handler_release`, which publishes only when the object is
//!   about to be freed, since only its finalizer could look;
//! * a dict lookup of the lazy-miss lowering (`lazy_miss`) that provably
//!   runs no code and raises nothing, which
//!   `pycc_ext_obj_getitem_lazy_in` decides.
//!
//! A handler a lazy miss entered has no exception at all until it
//! publishes: `pycc_ext_handler_publish` then makes the `KeyError` the
//! lookup would have raised, from the key the record holds and the
//! position of the subscript.
//!
//! An exception leaving the body goes through the handler's cleanup block,
//! which calls `pycc_ext_handler_unwind` first: an exception compiled code
//! raised itself becomes a CPython object there, while the handler's is
//! still the handled one, and so gets its context.
//!
//! Not published: a handler of a generator function (a `yield` inside it
//! would have to hand the thread's state back and forth), an `except*`
//! clause and a `finally` clause (docs/DEVIATIONS.md), and every handler of
//! a native program, which has no CPython.

use super::*;
use inkwell::values::{BasicValue, CallSiteValue, InstructionOpcode, InstructionValue};

/// `void pycc_ext_handler_publish(PyccExtHandler *h)`.
const PUBLISH_SYMBOL: &str = "pycc_ext_handler_publish";
/// `void pycc_ext_handler_release(PyccExtHandler *h, PyObject *o)`:
/// `pycc_ext_obj_release` in a handler's body.
const RELEASE_SYMBOL: &str = "pycc_ext_handler_release";
/// `void pycc_ext_handler_leave(PyObject *prev)`: the release of
/// [`State::prev`].
const LEAVE_SYMBOL: &str = "pycc_ext_handler_leave";
/// `void pycc_ext_handler_unwind(PyccExtHandler *h)`.
const UNWIND_SYMBOL: &str = "pycc_ext_handler_unwind";

/// The calls nothing can see the handled exception from: they run no other
/// code and make no CPython exception. Every other call of a handler's body
/// publishes first.
///
/// The packers of a constant or a native scalar are among them (`return 0`,
/// `return None`, a `str` literal): they make or look up an object of a
/// builtin type, and fail only for want of memory.
const SILENT: &[&str] = &[
    crate::ext::EXT_OBJ_RETAIN_SYMBOL,
    crate::ext::EXT_OBJ_NONE_SYMBOL,
    crate::ext::EXT_OBJ_PACK_INT_SYMBOL,
    crate::ext::EXT_OBJ_PACK_FLOAT_SYMBOL,
    crate::ext::EXT_OBJ_PACK_BOOL_SYMBOL,
    crate::fast_ops::EXT_OBJ_PACK_I64_SYMBOL,
    crate::py_literal::EXT_OBJ_STR_LITERAL_SYMBOL,
    PUBLISH_SYMBOL,
    RELEASE_SYMBOL,
    LEAVE_SYMBOL,
    UNWIND_SYMBOL,
    lazy_miss::EXT_OBJ_GETITEM_LAZY_IN_SYMBOL,
    "pycc_ext_tb_here_live",
    "pycc_rt_tb_here",
    "pycc_rt_exception_active",
    "pycc_rt_exception_value",
    "pycc_rt_exception_take",
    "pycc_rt_exception_restore",
    "pycc_rt_exception_incref",
    "pycc_rt_exception_raise",
    // What it bridges was raised by the call before it, which published.
    crate::EXT_OBJ_ERROR_BRIDGE_SYMBOL,
    "pycc_rt_str_incref",
    "pycc_rt_str_decref",
];

/// The words of a handler's record (`PyccExtHandler` in
/// `src/ext/module/object_except.c`).
const EXC: u64 = 0;
const PREV: u64 = 1;
const KEY: u64 = 2;
const PARENT: u64 = 3;
const INFO: u64 = 4;
const FRAME: u64 = 5;
const POS: u64 = 6;
const WORDS: u32 = 7;

/// One handler's record: an entry-block array, and its slots.
#[derive(Clone, Copy)]
pub(crate) struct State<'ctx> {
    /// The record itself, what the shim's functions take.
    pub(crate) record: PointerValue<'ctx>,
    /// The caught exception's slot (`owned_slots`): null while a lazy miss
    /// has made none.
    pub(crate) exc: PointerValue<'ctx>,
    /// The previous handled exception's slot, non-null once published.
    pub(crate) prev: PointerValue<'ctx>,
    /// The missing key's slot of a lazy miss.
    pub(crate) key: PointerValue<'ctx>,
    /// The position of the subscript that missed.
    pub(crate) pos: PointerValue<'ctx>,
    parent: PointerValue<'ctx>,
    info: PointerValue<'ctx>,
    frame: PointerValue<'ctx>,
}

/// The handler whose body is being emitted, and the calls of its way out.
pub(crate) struct Open<'ctx> {
    state: State<'ctx>,
}

/// Whether the handlers of the function being emitted publish.
pub(super) fn publishes(rt: &RtFns<'_>) -> bool {
    rt.publish_handled.get() && rt.gen_frame.borrow().is_none()
}

/// Allocates a handler's record in the entry block.
pub(super) fn new_state<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
) -> State<'ctx> {
    let function = builder
        .get_insert_block()
        .and_then(|block| block.get_parent())
        .expect("a handler is always emitted inside a function");
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    let words = ptr.array_type(WORDS);
    crate::build_at_entry_block(builder, function, |b| {
        let record = b
            .build_alloca(words, "handler_record")
            .expect("build_alloca should not fail for a handler's record");
        let field = |index: u64, name: &str| {
            // SAFETY: `index` is below `WORDS`.
            unsafe {
                b.build_in_bounds_gep(
                    ptr,
                    record,
                    &[context.i64_type().const_int(index, false)],
                    name,
                )
            }
            .expect("build_in_bounds_gep should not fail for a handler's record")
        };
        // The fields are named after the record, so the instructions are
        // emitted (and dominate every use) whether or not each is read.
        State {
            record,
            exc: field(EXC, "handler_saved_exc"),
            prev: field(PREV, "handler_prev"),
            key: field(KEY, "handler_key"),
            pos: field(POS, "handler_pos"),
            parent: field(PARENT, "handler_parent"),
            info: field(INFO, "handler_info"),
            frame: field(FRAME, "handler_frame"),
        }
    })
}

/// The record of the innermost handler whose body is being emitted.
pub(super) fn innermost<'ctx>(rt: &RtFns<'ctx>) -> Option<State<'ctx>> {
    rt.exceptions.handlers.borrow().last().map(|open| open.state)
}

/// Fills `state` on entry into its handler, the caught exception already
/// stored in `state.exc` (and, by a lazy miss, the key and the position in
/// theirs): not published yet, inside `parent`, in this function's frame.
/// Registers `state.prev` as an exception slot released by
/// `pycc_ext_handler_leave`.
pub(super) fn enter<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    state: State<'ctx>,
) {
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    let null = ptr.const_null();
    let parent = innermost(rt).map_or(null, |parent| parent.record);
    let (info, frame) = crate::frames::here(rt);
    for (slot, value) in [
        (state.prev, null),
        (state.parent, parent),
        (state.info, info.unwrap_or(null)),
        (state.frame, frame.unwrap_or(null)),
    ] {
        builder
            .build_store(slot, value)
            .expect("build_store should not fail for a handler's record");
    }
    let leave = crate::foreign_pack::shim_fn(
        module,
        LEAVE_SYMBOL,
        context.void_type().fn_type(&[ptr.into()], false),
    );
    owned_slots::hold_object(rt, state.prev, leave);
}

/// The instructions a handler's body adds: everything after `after` in
/// `first`, and every block appended after `last`.
pub(super) struct Region<'ctx> {
    state: State<'ctx>,
    first: inkwell::basic_block::BasicBlock<'ctx>,
    after: Option<InstructionValue<'ctx>>,
    last: inkwell::basic_block::BasicBlock<'ctx>,
}

/// Opens `state`'s body at the builder's position: the handler becomes the
/// innermost one, and what is emitted until [`Region::publish_where_seen`]
/// is its body.
pub(super) fn open<'ctx>(
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    state: State<'ctx>,
) -> Region<'ctx> {
    let first = builder
        .get_insert_block()
        .expect("the builder is positioned inside a block");
    let function = first
        .get_parent()
        .expect("every basic block belongs to a function");
    rt.exceptions.handlers.borrow_mut().push(Open { state });
    Region {
        state,
        first,
        after: first.get_last_instruction(),
        last: function
            .get_last_basic_block()
            .expect("a function being emitted into has a block"),
    }
}

/// Records `call`, the release of `slot` on a way out of a handler, when
/// `slot` belongs to a handler's record or is its `as` binding
/// ([`mark_exit_slot`]): such a release is not something the handled
/// exception is seen from.
pub(super) fn exits<'ctx>(rt: &RtFns<'ctx>, slot: PointerValue<'ctx>, call: CallSiteValue<'ctx>) {
    if rt.exceptions.handler_slots.borrow().contains(&slot)
        && let Some(instruction) = call.try_as_basic_value().instruction()
    {
        rt.exceptions.handler_exits.borrow_mut().push(instruction);
    }
}

/// Marks `slot` as one a publishing handler's way out releases.
pub(super) fn mark_exit_slot<'ctx>(rt: &RtFns<'ctx>, slot: PointerValue<'ctx>) {
    rt.exceptions.handler_slots.borrow_mut().push(slot);
}

/// Emits, at the top of the handler's cleanup block `cleanup`, the call
/// that gives an exception leaving the body its context
/// (`pycc_ext_handler_unwind`).
pub(super) fn unwind<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    state: State<'ctx>,
    cleanup: inkwell::basic_block::BasicBlock<'ctx>,
) {
    let resume = builder.get_insert_block();
    builder.position_at_end(cleanup);
    builder
        .build_call(record_fn(context, module, UNWIND_SYMBOL), &[state.record.into()], "")
        .expect("build_call should not fail for pycc_ext_handler_unwind");
    if let Some(block) = resume {
        builder.position_at_end(block);
    }
}

fn record_fn<'ctx>(
    context: &'ctx Context,
    module: &inkwell::module::Module<'ctx>,
    symbol: &str,
) -> FunctionValue<'ctx> {
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    crate::foreign_pack::shim_fn(module, symbol, context.void_type().fn_type(&[ptr.into()], false))
}

impl<'ctx> Region<'ctx> {
    /// Closes the body: puts a publishing call before every call in it the
    /// handled exception could be seen from (the module doc), and the
    /// handler is no longer the innermost one. The builder stays where it
    /// was.
    pub(super) fn publish_where_seen(
        self,
        context: &'ctx Context,
        builder: &inkwell::builder::Builder<'ctx>,
        module: &inkwell::module::Module<'ctx>,
        rt: &RtFns<'ctx>,
    ) {
        let resume = builder.get_insert_block();
        let ptr = context.ptr_type(inkwell::AddressSpace::default());
        let publish = record_fn(context, module, PUBLISH_SYMBOL);
        let release = crate::foreign_pack::shim_fn(
            module,
            RELEASE_SYMBOL,
            context.void_type().fn_type(&[ptr.into(), ptr.into()], false),
        );
        let mut calls = Vec::new();
        let start = match self.after {
            Some(after) => after.get_next_instruction(),
            None => self.first.get_first_instruction(),
        };
        collect_calls(start, &mut calls);
        let mut block = self.last.get_next_basic_block();
        while let Some(current) = block {
            collect_calls(current.get_first_instruction(), &mut calls);
            block = current.get_next_basic_block();
        }
        let exits = rt.exceptions.handler_exits.borrow().clone();
        // The rest of a block after the record's last slot is released is
        // past the handler.
        let mut left_in: Option<inkwell::basic_block::BasicBlock<'ctx>> = None;
        for instruction in calls {
            let call = CallSiteValue::try_from(instruction).expect("collected as a call");
            if left_in.is_some() && instruction.get_parent() == left_in {
                continue;
            }
            if exits.contains(&instruction) {
                if released_slot(instruction) == Some(self.state.exc) {
                    left_in = instruction.get_parent();
                }
                continue;
            }
            let callee = call.get_called_fn_value();
            let name = callee.map(|f| f.get_name().to_string_lossy().into_owned());
            let name = name.as_deref();
            if name.is_some_and(|name| name.starts_with("llvm.") || SILENT.contains(&name)) {
                continue;
            }
            // An inner handler's call already publishes this one first.
            if instruction
                .get_previous_instruction()
                .and_then(|previous| CallSiteValue::try_from(previous).ok())
                .and_then(|previous| previous.get_called_fn_value())
                .is_some_and(|f| f == publish)
            {
                continue;
            }
            builder.position_before(&instruction);
            if name == Some(crate::ext::EXT_OBJ_RELEASE_SYMBOL) {
                // The release itself becomes the handler's.
                let object = instruction
                    .get_operand(0)
                    .and_then(|operand| operand.value())
                    .expect("pycc_ext_obj_release takes the object");
                builder
                    .build_call(release, &[self.state.record.into(), object.into()], "")
                    .expect("build_call should not fail for pycc_ext_handler_release");
                instruction.erase_from_basic_block();
            } else {
                builder
                    .build_call(publish, &[self.state.record.into()], "")
                    .expect("build_call should not fail for pycc_ext_handler_publish");
            }
        }
        if let Some(block) = resume {
            builder.position_at_end(block);
        }
        let mut handlers = rt.exceptions.handlers.borrow_mut();
        handlers.pop();
        if handlers.is_empty() {
            rt.exceptions.handler_exits.borrow_mut().clear();
        }
    }
}

/// Appends every call from `start` to the end of its block.
fn collect_calls<'ctx>(start: Option<InstructionValue<'ctx>>, calls: &mut Vec<InstructionValue<'ctx>>) {
    let mut next = start;
    while let Some(instruction) = next {
        if instruction.get_opcode() == InstructionOpcode::Call {
            calls.push(instruction);
        }
        next = instruction.get_next_instruction();
    }
}

/// The slot the exit release `call` empties: the pointer operand of the
/// load that feeds it (`owned_slots::release`).
fn released_slot<'ctx>(call: InstructionValue<'ctx>) -> Option<PointerValue<'ctx>> {
    let held = call.get_operand(0)?.value()?.as_instruction_value()?;
    (held.get_opcode() == InstructionOpcode::Load)
        .then(|| held.get_operand(0)?.value())
        .flatten()
        .map(|slot| slot.into_pointer_value())
}
