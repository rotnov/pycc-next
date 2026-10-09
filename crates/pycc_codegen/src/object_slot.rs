//! Module-global `object` slots own their reference (Part 1 of #1499,
//! [#1501](https://github.com/rotnov/pycc/issues/1501)).
//!
//! **The invariant.** A module-global `object` slot (`pyglobal_<name>`)
//! holds exactly one owned `PyObject *` reference, or null. Every store into
//! one goes through [`store_owned`]:
//!
//! - a *produced* value -- a new reference [`is_owned`] recognises -- moves
//!   into the slot;
//! - every other source is borrowed, and [`retain_if_borrowed`] takes a
//!   reference of its own through the shim's `pycc_ext_obj_retain`
//!   (`Py_XINCREF`) before the store. An unlisted source is borrowed, so a
//!   source this module misclassifies over-retains and leaks; it never
//!   frees an object something else still names.
//! - a rebind stores the new value first and releases the old one after,
//!   the `Py_XSETREF` order. The release can run an arbitrary `__del__`,
//!   and that finalizer can call a compiled function that reads the global:
//!   it must see the new value, never a freed one. Storing first also makes
//!   `x = x` correct.
//! - the release runs only while the rebinding exec is the artifact's only
//!   live compiled activation (`pycc_ext_obj_rebind_may_release`, #1501);
//!   otherwise the old value leaks, because another activation -- a nested
//!   exec, an exec on another thread, a compiled call a host entered
//!   through a wrapper -- may still use it borrowed (see [`owned_bit`]).
//!
//! The stores are a module-level `x = <object>` (`MirStmt::Assign`), a
//! module-level `for x in <object>:` target ([`store_new_reference`], whose
//! per-trip item is `pycc_ext_obj_iter_next`'s new reference), and a
//! foreign import's binding (`foreign_import.rs`, the module or attribute
//! the import helpers return as a new reference).
//!
//! **The owned bit.** Each `object` global has a companion `i8` global,
//! `pyglobal.owned.<name>` (a dot no Python identifier contains, so it
//! cannot collide with a user global). The module-exec entry clears every
//! bit before its first statement, with a raw store ([`declare_owned_bits`]);
//! [`store_owned`] sets it, and releases the old value only when the bit it
//! loaded was set. A value an *earlier* exec stored -- a failed import
//! retried, a module re-imported after `del sys.modules[name]` -- was
//! stored before this exec's bits existed, so it is left unreleased, as it
//! was before this part: that exec is no longer running, and a frame or a
//! host object it handed the pointer to may still hold it borrowed. An owned
//! bit that is set implies the slot's `initialized` flag is set and that the
//! slot holds a reference the current exec stored.
//!
//! **The slot table.** A global is classified by the pointer its
//! [`StorageSlot`] carries, recorded when the globals are declared, never by
//! symbol name: `pyglobal_init_x` is a valid name for a user global
//! `init_x`'s value slot, and `Module::add_global` silently renames a
//! duplicate, so a name lookup can return the wrong global.
//!
//! **Frame slots are untouched.** A function body's locals, parameters and
//! returns keep #1092's leak-only, borrowed-pointer rule until Part 2
//! ([#1502](https://github.com/rotnov/pycc/issues/1502)): a parameter
//! rebind would otherwise release a reference its caller still owns. A
//! function-local name never resolves to a global's slot, because a
//! function cannot assign a global (`global` is refused with `C0001`), so a
//! local target always gets its own alloca. Compiled-instance `object`
//! attributes retain a borrowed source ([`retain_if_borrowed`] in the
//! `MirStmt::AttrSet` arm) but never release the replaced word; that slot
//! class is Part 4 ([#1504](https://github.com/rotnov/pycc/issues/1504)).

use super::*;
use crate::ext::{
    EXT_OBJ_REBIND_MAY_RELEASE_SYMBOL, EXT_OBJ_RELEASE_SYMBOL, EXT_OBJ_RETAIN_SYMBOL,
};
use inkwell::builder::Builder;
use std::cell::RefCell;

/// The owned bit of every module-global `object` slot, keyed by the slot's
/// value pointer. Empty until [`declare_owned_bits`] fills it, which is also
/// what a hand-built test fixture sees: every store then keeps the
/// pass-through it had before this part.
#[derive(Default)]
pub(super) struct ModuleObjectSlots<'ctx> {
    owned: RefCell<HashMap<PointerValue<'ctx>, PointerValue<'ctx>>>,
}

/// Declares the owned bit of every `object`-typed global in `globals`,
/// records it in `rt`'s slot table, and emits the exec-start clear of each
/// bit at the builder's position -- the module-exec entry's first block,
/// before its first statement.
///
/// The clear is a raw `store i8 0` into the bit only. It deliberately leaves
/// the value and its `initialized` flag alone, and it is deliberately not a
/// [`store_owned`]: a value an earlier exec stored stays readable by a
/// compiled function that escaped that exec, exactly as CPython keeps the
/// old module's globals, and it is never released (see the module doc). A
/// module with no `object` global declares nothing and emits nothing.
pub(super) fn declare_owned_bits<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    globals: &BTreeMap<String, StorageSlot<'ctx>>,
) {
    let i8t = context.i8_type();
    for (name, slot) in globals {
        if slot.ty != pycc_mir::Ty::Object {
            continue;
        }
        let bit = module.add_global(i8t, None, &format!("pyglobal.owned.{name}"));
        bit.set_linkage(Linkage::Internal);
        bit.set_initializer(&i8t.const_zero());
        let bit = bit.as_pointer_value();
        builder
            .build_store(bit, i8t.const_zero())
            .expect("build_store should not fail for a declared owned bit");
        rt.object_slots.owned.borrow_mut().insert(slot.ptr, bit);
    }
}

/// The owned bit of `slot` when it is a module-global `object` slot, and
/// `None` for every other slot -- a function local, a parameter, a
/// non-`object` global.
///
/// This predicate becomes obsolete in Part 2 (#1502), when every `object`
/// slot owns its reference.
///
/// SAFETY: a rebind may release the old value only when no frame can still
/// use it borrowed. [`store_owned`] therefore releases only when the shim's
/// `pycc_ext_obj_rebind_may_release()` reports the rebinding exec as the
/// artifact's only live compiled activation (`docs/RUNTIME.md`, "A module
/// global owns its reference"). Another activation -- a nested exec of the
/// same shared object, an exec on another thread, or a compiled function a
/// host entered through a wrapper, possibly on a thread that released the
/// GIL mid-call -- may hold the old value borrowed, so the rebind leaks it
/// instead. Within the rebinding exec's own activation no frame holds a
/// borrowed copy across the store: only the module body writes a global, and
/// every compiled callee has returned before the body's next statement.
///
/// The count is the shim's `pycc_ext_live_activations`. The bridge watermark
/// mark every CPython-to-compiled entry already takes counts the frame in:
/// `pycc_ext_exec_module`, and every generated wrapper (`wrapper_for`'s
/// exports, methods, field descriptors, comparison and hash slots and PEP
/// 562 hooks, and `Py_tp_init`). Each of its exits counts it out through
/// `pycc_ext_activation_exit` (or `_status`), around the `return` itself --
/// not in the watermark release -- so a wrapper stays counted until its
/// return value holds its own host reference and every cleanup that can run
/// a finalizer (the watermark release, a `PyBuffer_Release`) has finished. A
/// finalizer may switch the GIL; counted out before it, a suspended wrapper
/// would let a paused exec's rebind release the borrowed result it is about
/// to pack. No other entry runs compiled
/// code: a carrier's `__copy__` and `tp_dealloc` run only the runtime. It is
/// per artifact (a file static in each artifact's own shim), read with the
/// GIL held, and a free-threaded host is refused both at compile time
/// (`Python.h` rejects `Py_LIMITED_API` under `Py_GIL_DISABLED`) and at
/// import (the shim's `PyInit_` refuses an interpreter whose
/// `Py_GetVersion()` names a free-threading build).
///
/// Each of these invalidates the argument and must revisit this part first:
/// lowering `global` (a function would then write the slot), generator
/// support (a suspended frame of the same activation could keep a borrowed
/// copy across a rebind), publishing object globals on the host module (the
/// host could then write the slot), and any new entry into compiled code
/// that does not take the bridge watermark (it would be uncounted), or an
/// exit that counts its frame out before its pack and cleanup.
pub(super) fn owned_bit<'ctx>(
    rt: &RtFns<'ctx>,
    slot: &StorageSlot<'ctx>,
) -> Option<PointerValue<'ctx>> {
    rt.object_slots.owned.borrow().get(&slot.ptr).copied()
}

/// Whether `value`, an expression evaluated to a `Scalar::Object`, is a new
/// reference nothing else holds: a shim producer
/// (`object_release::is_produced`), or a native value boxed into a new
/// CPython object (`MirExpr::ObjectBox`), every packer of which returns a
/// new reference -- `pycc_ext_obj_pack_instance` included, which hands back
/// the live carrier with `Py_INCREF` or a freshly allocated one. A boxed
/// `None` is the exception: `pycc_ext_obj_none` returns `Py_None` borrowed.
pub(super) fn is_owned(value: &MirExpr) -> bool {
    match value {
        MirExpr::ObjectBox(inner) => inner.ty() != pycc_mir::Ty::None,
        other => crate::object_release::is_produced(crate::object_release::operand_source(other)),
    }
}

/// Declares `void pycc_ext_obj_retain(PyObject *)` once per module.
fn retain_fn<'ctx>(
    context: &'ctx Context,
    module: &inkwell::module::Module<'ctx>,
) -> FunctionValue<'ctx> {
    crate::foreign_pack::shim_fn(
        module,
        EXT_OBJ_RETAIN_SYMBOL,
        context.void_type().fn_type(
            &[context.ptr_type(inkwell::AddressSpace::default()).into()],
            false,
        ),
    )
}

/// `pointer`, evaluated from `value`, as a reference the store may move
/// into longer-lived storage: unchanged when [`is_owned`] says it is a new
/// reference, and otherwise retained first with `pycc_ext_obj_retain`.
pub(super) fn retain_if_borrowed<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    value: &MirExpr,
    pointer: PointerValue<'ctx>,
) -> PointerValue<'ctx> {
    if !is_owned(value) {
        builder
            .build_call(retain_fn(context, module), &[pointer.into()], "obj_retain")
            .expect("build_call should not fail for pycc_ext_obj_retain");
    }
    pointer
}

/// Stores `new`, an owned reference, into the module-global `object` slot
/// `slot` whose owned bit is `bit`, in the `Py_XSETREF` order: load the old
/// value and the bit, store the new value and raise the `initialized` flag
/// and the bit, then -- on a branch taken only when the loaded bit was set
/// and `pycc_ext_obj_rebind_may_release()` answers non-zero (see the SAFETY
/// note on [`owned_bit`]) -- release the old value through
/// `pycc_ext_obj_release`. When the gate is closed the old value leaks; the
/// new value and the bit are stored either way. The release cannot fail: a
/// finalizer's exception is CPython's unraisable hook's, not this
/// statement's.
///
/// Panics outside the module-exec entry: a module global is written only
/// there, so a store anywhere else is a misclassified frame slot, which must
/// fail the build rather than release a borrowed reference.
pub(super) fn store_owned<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    slot: &StorageSlot<'ctx>,
    bit: PointerValue<'ctx>,
    new: PointerValue<'ctx>,
) {
    let function = crate::foreign_attr::expect_module_exec_entry(builder);
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    let i8t = context.i8_type();
    let old = builder
        .build_load(ptr, slot.ptr, "global_old")
        .expect("build_load should not fail for a declared module global")
        .into_pointer_value();
    let was_owned = builder
        .build_load(i8t, bit, "global_owned")
        .expect("build_load should not fail for a declared owned bit")
        .into_int_value();
    builder
        .build_store(slot.ptr, new)
        .expect("build_store should not fail for a declared module global");
    if let Some(initialized) = slot.initialized {
        builder
            .build_store(initialized, i8t.const_int(1, false))
            .expect("build_store should not fail for a declared global flag");
    }
    builder
        .build_store(bit, i8t.const_int(1, false))
        .expect("build_store should not fail for a declared owned bit");
    let owned = builder
        .build_int_compare(
            IntPredicate::NE,
            was_owned,
            i8t.const_zero(),
            "global_was_owned",
        )
        .expect("build_int_compare should not fail");
    let i32t = context.i32_type();
    let gate = crate::foreign_pack::shim_fn(
        module,
        EXT_OBJ_REBIND_MAY_RELEASE_SYMBOL,
        i32t.fn_type(&[], false),
    );
    let may = builder
        .build_call(gate, &[], "rebind_may_release")
        .expect("build_call should not fail for pycc_ext_obj_rebind_may_release")
        .try_as_basic_value()
        .expect_basic("pycc_ext_obj_rebind_may_release returns int")
        .into_int_value();
    let alone = builder
        .build_int_compare(IntPredicate::NE, may, i32t.const_zero(), "global_alone")
        .expect("build_int_compare should not fail");
    let owned = builder
        .build_and(owned, alone, "global_release_gate")
        .expect("build_and should not fail");
    let release_bb = context.append_basic_block(function, "global_release_old");
    let cont_bb = context.append_basic_block(function, "global_stored");
    builder
        .build_conditional_branch(owned, release_bb, cont_bb)
        .expect("build_conditional_branch should not fail");
    builder.position_at_end(release_bb);
    let release = crate::foreign_pack::shim_fn(
        module,
        EXT_OBJ_RELEASE_SYMBOL,
        context.void_type().fn_type(&[ptr.into()], false),
    );
    builder
        .build_call(release, &[old.into()], "global_release")
        .expect("build_call should not fail for pycc_ext_obj_release");
    builder
        .build_unconditional_branch(cont_bb)
        .expect("build_unconditional_branch should not fail");
    builder.position_at_end(cont_bb);
}

/// `MirStmt::Assign`'s store into a module-global `object` slot: retains a
/// borrowed `scalar` (evaluated from `value`) and stores it through
/// [`store_owned`]. Answers `false`, emitting nothing, for every other
/// target, which keeps `emit_assign`'s store.
#[allow(clippy::too_many_arguments)]
pub(super) fn assign<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    target: &str,
    value: &MirExpr,
    scalar: Scalar<'ctx>,
) -> bool {
    let (Some(slot), Scalar::Object(pointer)) = (locals.get(target), scalar) else {
        return false;
    };
    let Some(bit) = owned_bit(rt, slot) else {
        return false;
    };
    let pointer = retain_if_borrowed(context, builder, module, value, pointer);
    store_owned(context, builder, module, slot, bit, pointer);
    true
}

/// Binds `new`, a new reference -- a `for x in <object>:` item, or a
/// foreign import's result -- to `slot`: moved in through [`store_owned`]
/// when `slot` is a module-global `object` slot, and otherwise a plain
/// store that raises the `initialized` flag, the frame-slot rule until
/// Part 2.
pub(super) fn store_new_reference<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    slot: &StorageSlot<'ctx>,
    new: PointerValue<'ctx>,
) {
    if let Some(bit) = owned_bit(rt, slot) {
        store_owned(context, builder, module, slot, bit, new);
        return;
    }
    builder
        .build_store(slot.ptr, new)
        .expect("build_store should not fail for a declared slot");
    if let Some(initialized) = slot.initialized {
        builder
            .build_store(initialized, context.i8_type().const_int(1, false))
            .expect("build_store should not fail for a declared slot flag");
    }
}

#[cfg(test)]
#[path = "object_slot_tests.rs"]
mod tests;
