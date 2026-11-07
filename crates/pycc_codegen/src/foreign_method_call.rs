//! A method call on a CPython object without a bound method, as CPython's
//! `LOAD_ATTR` of a method and its `CALL`/`CALL_KW` make none: the load
//! ([`emit_load_method`]: the function and its receiver when the receiver's
//! class holds it) before the arguments, and the call ([`emit_call_loaded`],
//! positional or with keywords) after them, both sharing one call site's
//! cache record; or one fused call when evaluating the arguments can neither
//! raise nor run code ([`emit_method_call_fused`]). Split out of
//! `foreign_call.rs`, whose packer contract these follow (its module doc).

use super::*;
use crate::foreign_attr::expect_object_pointer;
use crate::foreign_call::{alloca_in_entry_block, emit_name_array, emit_packed_array};
use crate::foreign_fail::{ForeignFailEdge, route_null};
use crate::foreign_pack::{emit_pack, shim_fn};
use inkwell::builder::Builder;
use inkwell::values::BasicMetadataValueEnum;

/// v0.9: the static record of one method call site's cache
/// (`PyccExtCallSite` in the shim's `call_site.c`: six words, zero at
/// first), passed to the method-call entries.
fn call_site_record<'ctx>(
    context: &'ctx Context,
    module: &inkwell::module::Module<'ctx>,
) -> inkwell::values::PointerValue<'ctx> {
    let ty = context.i64_type().array_type(6);
    let global = module.add_global(ty, None, "pycc_call_site");
    global.set_initializer(&ty.const_zero());
    global.set_linkage(inkwell::module::Linkage::Private);
    global.as_pointer_value()
}

/// The lookup half of a positional method call, as CPython's `LOAD_ATTR`
/// of a method: [`crate::ext::EXT_OBJ_LOAD_METHOD_SYMBOL`] answers the
/// callable and, for a method of the receiver's class the attribute caches
/// hold, the receiver to pass first (both new references; the receiver
/// `NULL` otherwise), with no bound method made. [`emit_call_loaded`]
/// consumes both.
pub(super) fn emit_load_method<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    base: Scalar<'ctx>,
    method: &str,
) -> (
    inkwell::values::PointerValue<'ctx>,
    inkwell::values::PointerValue<'ctx>,
    inkwell::values::PointerValue<'ctx>,
) {
    let edge = ForeignFailEdge::for_current(builder);
    let base_ptr = expect_object_pointer(base);
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    let self_slot = alloca_in_entry_block(context, builder, edge.function(), 1);
    let name = builder
        .build_global_string_ptr(method, &format!("pycc_foreign_method_{method}"))
        .expect("build_global_string_ptr should not fail")
        .as_pointer_value();
    let load = shim_fn(
        module,
        crate::ext::EXT_OBJ_LOAD_METHOD_SYMBOL,
        ptr.fn_type(&[ptr.into(), ptr.into(), ptr.into(), ptr.into()], false),
    );
    let site = call_site_record(context, module);
    let callable = builder
        .build_call(
            load,
            &[base_ptr.into(), name.into(), self_slot.into(), site.into()],
            "foreign_call_bound",
        )
        .expect("build_call should not fail for pycc_ext_obj_load_method")
        .try_as_basic_value()
        .expect_basic("pycc_ext_obj_load_method returns PyObject *")
        .into_pointer_value();
    route_null(
        context,
        builder,
        module,
        rt,
        edge,
        callable,
        "foreign_call_lookup",
    );
    let receiver = builder
        .build_load(ptr, self_slot, "foreign_call_self")
        .expect("build_load should not fail")
        .into_pointer_value();
    (callable, receiver, site)
}

/// Marshals `args` and calls what [`emit_load_method`] loaded:
/// [`crate::ext::EXT_OBJ_CALL_LOADED_SYMBOL`], which consumes `callable`,
/// `receiver` (stored ahead of the arguments) and every packed argument;
/// `site` is the call site's cache record [`emit_load_method`] passed too.
/// With `names`, the last `names.len()` of `args` are keyword values passed
/// by those names ([`crate::ext::EXT_OBJ_CALL_LOADED_KW_SYMBOL`], CPython's
/// `CALL_KW` after a method load); no unpacking marker may be among them.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_call_loaded<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    callable: inkwell::values::PointerValue<'ctx>,
    receiver: inkwell::values::PointerValue<'ctx>,
    site: inkwell::values::PointerValue<'ctx>,
    args: &[Scalar<'ctx>],
    names: &[String],
) -> Scalar<'ctx> {
    debug_assert!(names.len() <= args.len(), "every keyword name has a value");
    let edge = ForeignFailEdge::for_current(builder);
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    let i64_type = context.i64_type();
    // Slot 0 holds the receiver (or NULL) ahead of the arguments, so the
    // call needs no copy to put it first.
    let stack = alloca_in_entry_block(context, builder, edge.function(), args.len() + 1);
    builder
        .build_store(stack, receiver)
        .expect("build_store should not fail");
    for (index, value) in args.iter().enumerate() {
        let packed = emit_pack(context, builder, module, *value, "foreign_call_arg");
        let slot = unsafe {
            builder
                .build_in_bounds_gep(
                    ptr,
                    stack,
                    &[i64_type.const_int(index as u64 + 1, false)],
                    "foreign_call_arg_slot",
                )
                .expect("build_in_bounds_gep should not fail")
        };
        builder
            .build_store(slot, packed)
            .expect("build_store should not fail");
    }
    let nargs = i64_type.const_int((args.len() - names.len()) as u64, false);
    let (symbol, fn_type, operands): (_, _, Vec<BasicMetadataValueEnum<'ctx>>) = if names.is_empty() {
        (
            crate::ext::EXT_OBJ_CALL_LOADED_SYMBOL,
            ptr.fn_type(&[ptr.into(), ptr.into(), i64_type.into(), ptr.into()], false),
            vec![callable.into(), stack.into(), nargs.into(), site.into()],
        )
    } else {
        let name_array = emit_name_array(context, builder, edge.function(), names);
        // The site's keyword names tuple, made at its first call and kept.
        let kwnames = module.add_global(ptr, None, "pycc_call_site_kwnames");
        kwnames.set_initializer(&ptr.const_null());
        kwnames.set_linkage(inkwell::module::Linkage::Private);
        (
            crate::ext::EXT_OBJ_CALL_LOADED_KW_SYMBOL,
            ptr.fn_type(
                &[
                    ptr.into(),
                    ptr.into(),
                    i64_type.into(),
                    ptr.into(),
                    i64_type.into(),
                    ptr.into(),
                    ptr.into(),
                ],
                false,
            ),
            vec![
                callable.into(),
                stack.into(),
                nargs.into(),
                name_array.into(),
                i64_type.const_int(names.len() as u64, false).into(),
                site.into(),
                kwnames.as_pointer_value().into(),
            ],
        )
    };
    let call = shim_fn(module, symbol, fn_type);
    let result = builder
        .build_call(call, &operands, "foreign_call")
        .expect("build_call should not fail for pycc_ext_obj_call_loaded")
        .try_as_basic_value()
        .expect_basic("pycc_ext_obj_call_loaded returns PyObject *")
        .into_pointer_value();
    route_null(context, builder, module, rt, edge, result, "foreign_call");
    Scalar::Object(result)
}

/// `base.method(args)` as one `pycc_ext_obj_call_method` call: no bound
/// method is made. `base` is borrowed and `args` already evaluated; the
/// caller guarantees evaluating them could neither raise nor run code
/// (`foreign_call_emit::emit_method_call`), so the lookup may follow them.
pub(super) fn emit_method_call_fused<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    base: Scalar<'ctx>,
    method: &str,
    args: &[Scalar<'ctx>],
) -> Scalar<'ctx> {
    let edge = ForeignFailEdge::for_current(builder);
    let entry_fn = edge.function();
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    let i64_type = context.i64_type();
    let base_ptr = expect_object_pointer(base);
    let name = builder
        .build_global_string_ptr(method, &format!("pycc_foreign_method_{method}"))
        .expect("build_global_string_ptr should not fail")
        .as_pointer_value();
    let arg_array = emit_packed_array(context, builder, module, entry_fn, args);
    let call = shim_fn(
        module,
        crate::ext::EXT_OBJ_CALL_METHOD_SYMBOL,
        ptr.fn_type(
            &[ptr.into(), ptr.into(), ptr.into(), i64_type.into(), ptr.into()],
            false,
        ),
    );
    let site = call_site_record(context, module);
    let result = builder
        .build_call(
            call,
            &[
                base_ptr.into(),
                name.into(),
                arg_array.into(),
                i64_type.const_int(args.len() as u64, false).into(),
                site.into(),
            ],
            "foreign_call",
        )
        .expect("build_call should not fail for pycc_ext_obj_call_method")
        .try_as_basic_value()
        .expect_basic("pycc_ext_obj_call_method returns PyObject *")
        .into_pointer_value();
    route_null(context, builder, module, rt, edge, result, "foreign_call");
    Scalar::Object(result)
}
