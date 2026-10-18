//! Module globals rebound from outside (v0.8 accept 3): a non-strict
//! `--ext` module's compiled function reads a published global through the
//! module's `__dict__` version tag, so a rebinding by other code
//! (`mod.VERSION = ...`, `mock.patch`) reaches it (`global_sync.c` in the
//! shim owns the lookup).
//!
//! The fast path is three loads and a compare: the module dict (null
//! outside CPython 3.13, where every read takes the slot as it is), its
//! version tag, and the version this read last saw. Only a changed tag
//! calls `pycc_ext_global_sync`, which updates the slot when the dict
//! holds another object than last time.

use crate::RtFns;
use inkwell::context::Context;
use inkwell::module::{Linkage, Module};
use inkwell::values::PointerValue;
use pycc_mir::Ty;

/// `PyDictObject.ma_version_tag` in CPython 3.13 (`global_sync.c`).
const DICT_VERSION_TAG: u64 = 24;

/// The shim's slot kind for a global of type `ty`, or `None` for a type
/// whose slot is not synced.
fn kind(ty: &Ty) -> Option<u64> {
    match ty {
        Ty::Object => Some(0),
        Ty::Str => Some(1),
        Ty::Int => Some(2),
        Ty::Float => Some(3),
        Ty::Bool => Some(4),
        _ => None,
    }
}

/// Before a compiled function reads the global `name` (slot `slot`, bound
/// flag `initialized`, type `ty`): brings the slot up to date with the
/// module's `__dict__` when the dict changed since the last read.
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_sync<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &Module<'ctx>,
    rt: &RtFns<'ctx>,
    name: &str,
    slot: PointerValue<'ctx>,
    initialized: PointerValue<'ctx>,
    ty: &Ty,
) {
    if !rt.frames.syncs_globals() || name.starts_with("__pycc") {
        return;
    }
    let Some(kind) = kind(ty) else {
        return;
    };
    let Some(function) = builder.get_insert_block().and_then(|b| b.get_parent()) else {
        return;
    };
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    let i64_type = context.i64_type();
    let dict_global = module.get_global("pycc_ext_ns_dict").unwrap_or_else(|| {
        let global = module.add_global(ptr, None, "pycc_ext_ns_dict");
        global.set_linkage(Linkage::External);
        global
    });
    let cache_name = format!("pycc_gsync.{name}");
    let cache = module.get_global(&cache_name).unwrap_or_else(|| {
        let cache_ty = context.struct_type(&[i64_type.into(), ptr.into()], false);
        let global = module.add_global(cache_ty, None, &cache_name);
        global.set_initializer(&cache_ty.const_zero());
        global.set_linkage(Linkage::Private);
        global
    });
    let sync = module
        .get_function("pycc_ext_global_sync")
        .unwrap_or_else(|| {
            let sync = module.add_function(
                "pycc_ext_global_sync",
                context.void_type().fn_type(
                    &[
                        ptr.into(),
                        ptr.into(),
                        context.i32_type().into(),
                        ptr.into(),
                        ptr.into(),
                    ],
                    false,
                ),
                Some(Linkage::External),
            );
            crate::frames::mark_cold(context, sync);
            sync
        });
    let check = context.append_basic_block(function, "gsync_check");
    let slow = context.append_basic_block(function, "gsync_slow");
    let done = context.append_basic_block(function, "gsync_done");
    let dict = builder
        .build_load(ptr, dict_global.as_pointer_value(), "gsync_dict")
        .expect("build_load should not fail for the module dict")
        .into_pointer_value();
    let absent = builder
        .build_is_null(dict, "gsync_absent")
        .expect("build_is_null should not fail");
    let branch = builder
        .build_conditional_branch(absent, done, check)
        .expect("build_conditional_branch should not fail");
    crate::int_fast::weight_branch(context, branch, false);
    builder.position_at_end(check);
    let tag_ptr = unsafe {
        builder.build_in_bounds_gep(
            context.i8_type(),
            dict,
            &[i64_type.const_int(DICT_VERSION_TAG, false)],
            "gsync_tag_ptr",
        )
    }
    .expect("build_in_bounds_gep should not fail for the dict version");
    let tag = builder
        .build_load(i64_type, tag_ptr, "gsync_tag")
        .expect("build_load should not fail for the dict version")
        .into_int_value();
    let seen = builder
        .build_load(i64_type, cache.as_pointer_value(), "gsync_seen")
        .expect("build_load should not fail for a global cache")
        .into_int_value();
    let same = builder
        .build_int_compare(inkwell::IntPredicate::EQ, tag, seen, "gsync_same")
        .expect("build_int_compare should not fail");
    let branch = builder
        .build_conditional_branch(same, done, slow)
        .expect("build_conditional_branch should not fail");
    crate::int_fast::weight_branch(context, branch, true);
    builder.position_at_end(slow);
    let name_ptr = builder
        .build_global_string_ptr(name, "gsync_name")
        .expect("build_global_string_ptr should not fail")
        .as_pointer_value();
    builder
        .build_call(
            sync,
            &[
                name_ptr.into(),
                slot.into(),
                context.i32_type().const_int(kind, false).into(),
                cache.as_pointer_value().into(),
                initialized.into(),
            ],
            "",
        )
        .expect("build_call should not fail for pycc_ext_global_sync");
    builder
        .build_unconditional_branch(done)
        .expect("build_unconditional_branch should not fail");
    builder.position_at_end(done);
}
