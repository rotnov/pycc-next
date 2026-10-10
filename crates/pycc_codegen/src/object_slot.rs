//! Binding a new `object` reference to a slot.
//!
//! An `object` slot -- a module global, a function local or parameter --
//! owns one reference, or null, exactly like every other counted slot
//! (`heap_rc`). Most stores reach it through `MirStmt::Assign`'s generic
//! path. The two that have no `MirExpr` value go through
//! [`store_new_reference`]: a `for x in <object>:` target, whose per-trip
//! item is `pycc_ext_obj_iter_next`'s new reference, and a foreign import's
//! binding (`foreign_import.rs`), the module or attribute the import
//! helpers return as a new reference.

use super::*;
use inkwell::builder::Builder;

/// Moves `new`, a new reference, into `slot`, raising its definite-assignment
/// flag, and releases the value the slot held -- after the store, in the
/// `Py_XSETREF` order (`heap_rc::take_slot_old`), so a finalizer the release
/// runs never sees the freed value. Storing first also makes a rebind to
/// the same object correct.
pub(super) fn store_new_reference<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    rt: &RtFns<'ctx>,
    slot: &StorageSlot<'ctx>,
    new: PointerValue<'ctx>,
) {
    let old = crate::heap_rc::take_slot_old(context, builder, rt, &pycc_mir::Ty::Object, slot.ptr);
    builder
        .build_store(slot.ptr, new)
        .expect("build_store should not fail for a declared slot");
    if let Some(initialized) = slot.initialized {
        builder
            .build_store(initialized, context.i8_type().const_int(1, false))
            .expect("build_store should not fail for a declared slot flag");
    }
    crate::heap_rc::release_old(context, builder, rt, old);
}
