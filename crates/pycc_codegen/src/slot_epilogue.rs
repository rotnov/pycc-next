//! The owned-slot epilogue's releases: every `str`, `int` and counted heap
//! slot a function owns is released once on its way out (#1054, #625).

use super::*;

/// Emits, at the builder's position, one release per owned slot: a `str`
/// decref (null is the runtime's no-op), an `int` release behind its guard
/// (a parameter's one-test guard, `bigint_rc::emit_param_release`), and a
/// heap decref of its kind.
pub(crate) fn release_owned_slots<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    str_slots: &[PointerValue<'ctx>],
    int_slots: &[(PointerValue<'ctx>, bool)],
    heap_slots: &[(PointerValue<'ctx>, heap_rc::HeapKind)],
) {
    let ptr_type = context.ptr_type(inkwell::AddressSpace::default());
    for slot_ptr in str_slots {
        let live = builder
            .build_load(ptr_type, *slot_ptr, "str_epilogue_live")
            .expect("build_load should not fail for a str slot this function allocated")
            .into_pointer_value();
        builder
            .build_call(rt.str_decref, &[live.into()], "str_epilogue_decref")
            .expect("build_call should not fail for pycc_rt_str_decref");
    }
    for (slot_ptr, is_param) in int_slots {
        let live = builder
            .build_load(context.i64_type(), *slot_ptr, "int_epilogue_live")
            .expect("build_load should not fail for an int slot this function allocated")
            .into_int_value();
        if *is_param {
            bigint_rc::emit_param_release(context, builder, rt, live);
        } else {
            bigint_rc::emit_bigint_refcount_call(
                context,
                builder,
                rt,
                live,
                bigint_rc::BigIntRefcount::Release,
            );
        }
    }
    for (slot_ptr, kind) in heap_slots {
        let live = builder
            .build_load(ptr_type, *slot_ptr, "heap_epilogue_live")
            .expect("build_load should not fail for a slot this function allocated")
            .into_pointer_value();
        heap_rc::emit_decref(context, builder, rt, *kind, live);
    }
}
