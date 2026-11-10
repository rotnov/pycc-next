//! A `KeyError` caught where nothing can see it is never made
//! (docs/design/compile-little.md, section 2.7).
//!
//! ```python
//! try:
//!     return self.effective_config[name]
//! except KeyError:
//!     return config_defaults[name]
//! ```
//!
//! A subscript of an exact `dict` that misses raises `KeyError(key)` and
//! nothing else: an exact dict has no `__missing__`. When the subscript sits
//! directly in a `try` body (the innermost exception target is that `try`'s
//! dispatch) and the first handler catches a plain `KeyError` and cannot see
//! the exception, the miss goes straight into that handler instead, with no
//! `KeyError`, no bridged pycc exception and no traceback record made
//! (`pycc_ext_obj_getitem_lazy`, which answers `PYCC_EXT_LAZY_MISS`).
//!
//! The `KeyError` would have held the key until the handler is left, so the
//! handler holds it instead (`owned_slots::hold_object`): a key that is a
//! temporary, or that the handler rebinds, is finalized where CPython
//! finalizes it.
//!
//! A handler sees the exception only through an `as` binding or a bare
//! `raise` of its own (one not inside a nested handler, which re-raises
//! that handler's exception). Nothing else can: a pycc handler sets neither
//! CPython's handled exception (`sys.exc_info()`) nor the `__context__` of
//! what it raises (`pycc_ext_obj_raise`). So in such a handler the two
//! entries -- the real `KeyError` taken from the pending state, and the lazy
//! miss with no exception -- run the same statements with the same results.
//! If handlers come to publish the exception they handle, this lowering
//! must make the `KeyError` wherever the handler can then observe it (a
//! call, a `raise`).
//!
//! With the fast paths off (`PYCC_NO_FAST_OPS=1`) the helper raises on every
//! miss, which is how `tests/idiom_lowerings.rs` compares the two paths.
//!
//! Every other subscript, and a miss of any other mapping, raises as
//! before, and the `try` dispatches it as before.

use super::*;
use inkwell::basic_block::BasicBlock;

/// The runtime tag of `KeyError` (`pycc_ext_obj_error_tag`).
const KEY_ERROR_TAG: u8 = 3;
/// The runtime tag of `Exception`, which `pycc_rt_exception_type_matches`
/// treats as a catch-all for every `Exception`.
const EXCEPTION_TAG: u8 = 0;

/// `pycc_ext_obj_getitem_lazy`: `o[k]` answering [`LAZY_MISS`] for a key
/// missing from an exact `dict`.
pub(crate) const EXT_OBJ_GETITEM_LAZY_SYMBOL: &str = "pycc_ext_obj_getitem_lazy";
/// The pointer `pycc_ext_obj_getitem_lazy` answers for a lazy miss.
const LAZY_MISS: u64 = 1;

/// One `try` whose `KeyError` handler takes lazy misses.
#[derive(Clone, Copy)]
pub(crate) struct LazyMiss<'ctx> {
    /// The `try`'s handler dispatch: a subscript takes the lazy path only
    /// when this is its innermost exception target.
    dispatch: BasicBlock<'ctx>,
    /// Where a lazy miss enters the handler.
    pub(crate) entry: BasicBlock<'ctx>,
    /// The slot the missing key is kept in while the handler runs: null
    /// when the handler was entered by an exception.
    pub(crate) key_slot: PointerValue<'ctx>,
}

/// Whether a plain `KeyError` reaches the first handler, and that handler
/// cannot see the exception. Only the first handler is considered: an
/// earlier one of another class (`LookupError`, a class another module
/// defines) may catch a `KeyError` too, which only the runtime dispatch
/// knows.
pub(crate) fn eligible_handler(handlers: &[pycc_mir::MirExceptHandler]) -> Option<usize> {
    let handler = handlers.first()?;
    let catches = match handler.exc_type_tag.as_deref() {
        None => true,
        Some(tags) => tags.contains(&KEY_ERROR_TAG) || tags.contains(&EXCEPTION_TAG),
    };
    (catches && handler.binding_name.is_none() && !reraises(&handler.body)).then_some(0)
}

/// Whether `body` holds a bare `raise` re-raising the handler it belongs to:
/// one outside every nested handler.
fn reraises(body: &[MirStmt]) -> bool {
    body.iter().any(|stmt| match stmt {
        MirStmt::Reraise => true,
        MirStmt::Try {
            body,
            orelse,
            finalbody,
            ..
        }
        | MirStmt::TryStar {
            body,
            orelse,
            finalbody,
            ..
        } => reraises(body) || reraises(orelse) || reraises(finalbody),
        other => other.bodies().into_iter().any(reraises),
    })
}

/// Opens a lazy-miss entry for the `try` whose dispatch is `dispatch`, for
/// the body about to be emitted; [`close`] ends it. The key slot is cleared
/// here, on the way into the body, so a handler entered by an exception
/// finds no key.
pub(crate) fn open<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    function: FunctionValue<'ctx>,
    dispatch: BasicBlock<'ctx>,
) {
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    let entry = context.append_basic_block(function, "try_lazy_miss");
    let key_slot = alloca_at_entry(builder, ptr, "lazy_miss_key");
    builder
        .build_store(key_slot, ptr.const_null())
        .expect("build_store should not fail for the missing key's slot");
    rt.exceptions.lazy_misses.borrow_mut().push(LazyMiss {
        dispatch,
        entry,
        key_slot,
    });
}

/// Ends the entry [`open`] began: `None` when no subscript took it (the
/// block is removed), else the entry, its block still without a terminator,
/// for the handler to join.
pub(crate) fn close<'ctx>(rt: &RtFns<'ctx>) -> Option<LazyMiss<'ctx>> {
    let lazy = rt
        .exceptions
        .lazy_misses
        .borrow_mut()
        .pop()
        .expect("close follows open");
    if lazy.entry.get_first_use().is_none() {
        // SAFETY: the block has no predecessors and no instructions.
        unsafe { lazy.entry.delete() }.expect("an unused block is deleted");
        return None;
    }
    Some(lazy)
}

/// The lazy-miss entry a subscript emitted now may branch to: the innermost
/// open one, when its `try` is also the innermost exception target.
fn active_entry<'ctx>(rt: &RtFns<'ctx>) -> Option<(BasicBlock<'ctx>, PointerValue<'ctx>)> {
    let lazy = rt.exceptions.lazy_misses.borrow();
    let last = lazy.last()?;
    let target = *rt.exceptions.targets.borrow().last()?;
    (target == last.dispatch).then_some((last.entry, last.key_slot))
}

/// Emits `o[k]` (`base` borrowed, `key` a packed new reference the helper
/// consumes) through `pycc_ext_obj_getitem_lazy` when a lazy-miss entry is
/// active, and answers `None`, emitting nothing, otherwise. On a lazy miss
/// the helper has moved the key into the entry's key slot, the pending
/// temporaries are released as an exception edge releases them, and the
/// code branches to the entry. The result is otherwise the value or
/// `NULL` with CPython's exception set, which the caller routes.
pub(crate) fn emit_subscript<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    base: PointerValue<'ctx>,
    key: impl FnOnce() -> PointerValue<'ctx>,
) -> Option<PointerValue<'ctx>> {
    let (entry, key_slot) = active_entry(rt)?;
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    let function = builder
        .get_insert_block()
        .and_then(|b| b.get_parent())
        .expect("the builder is positioned inside a function");
    let key = key();
    let getitem = crate::foreign_pack::shim_fn(
        module,
        EXT_OBJ_GETITEM_LAZY_SYMBOL,
        ptr.fn_type(&[ptr.into(), ptr.into(), ptr.into()], false),
    );
    let result = builder
        .build_call(getitem, &[base.into(), key.into(), key_slot.into()], "lazy_subscript")
        .expect("build_call should not fail for pycc_ext_obj_getitem_lazy")
        .try_as_basic_value()
        .expect_basic("pycc_ext_obj_getitem_lazy returns PyObject *")
        .into_pointer_value();
    let miss_value = context
        .i64_type()
        .const_int(LAZY_MISS, false)
        .const_to_pointer(ptr);
    let is_miss = builder
        .build_int_compare(inkwell::IntPredicate::EQ, result, miss_value, "lazy_is_miss")
        .expect("build_int_compare should not fail for two pointers");
    let miss_bb = context.append_basic_block(function, "lazy_miss");
    let cont_bb = context.append_basic_block(function, "lazy_found");
    builder
        .build_conditional_branch(is_miss, miss_bb, cont_bb)
        .expect("build_conditional_branch should not fail for an i1 condition");
    builder.position_at_end(miss_bb);
    let snapshot: Vec<IntValue<'ctx>> = rt.exceptions.pending_int_releases.borrow().clone();
    for word in snapshot {
        emit_bigint_refcount_call(context, builder, rt, word, BigIntRefcount::Release);
    }
    crate::object_release::release_pending(builder, rt);
    crate::heap_rc::release_pending(context, builder, rt);
    builder
        .build_unconditional_branch(entry)
        .expect("build_unconditional_branch should not fail for a fresh block");
    builder.position_at_end(cont_bb);
    Some(result)
}
