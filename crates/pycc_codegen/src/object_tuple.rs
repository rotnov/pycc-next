//! Non-strict mode: a tuple display with an object element (`(k, f(v))`),
//! built in place, and `tuple()` and `sorted()` of a list only the call
//! holds ([`fresh_symbol`]) (docs/design/compile-little.md, section 2.6).
//!
//! The elements are packed (each a new reference, or NULL from a failed
//! packer), then `PyTuple_New` makes the tuple and each packed element is
//! stored into its item slot, where a `tuple`'s items start on this
//! interpreter (`PyccExtFast`'s `tuple_items_at`, confirmed by the shim's
//! layout probe): what `PyTuple_SET_ITEM` does, which the limited API the
//! shim is built against leaves out. When the layout is not confirmed (or
//! the fast paths are off), an element is NULL or `PyTuple_New` fails,
//! `pycc_ext_obj_build_display` builds it as before, consuming every packed
//! element on every path.

use super::foreign_fail::{ForeignFailEdge, route_null};
use super::foreign_pack::{emit_pack, shim_fn};
use super::{RtFns, Scalar};
use inkwell::context::Context;
use inkwell::module::Module;
use inkwell::values::PointerValue;

/// Emits the tuple of `elements`, already evaluated left to right.
pub(super) fn emit_tuple_display<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &Module<'ctx>,
    rt: &RtFns<'ctx>,
    elements: &[Scalar<'ctx>],
) -> Scalar<'ctx> {
    let edge = ForeignFailEdge::for_current(builder);
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    let i64_ty = context.i64_type();
    let packed: Vec<PointerValue<'ctx>> = elements
        .iter()
        .map(|element| emit_pack(context, builder, module, *element, "tuple_item"))
        .collect();
    let count = i64_ty.const_int(packed.len() as u64, false);
    let result = crate::fast_ops::guarded(
        context,
        builder,
        module,
        "tuple_display",
        |fast| {
            let items_at = fast.tuple_items_at("tuple_display");
            for item in &packed {
                let present = builder.build_is_not_null(*item, "tuple_item_present").expect("is_not_null");
                fast.require(present, "tuple_item_present");
            }
            let new = module.get_function("PyTuple_New").unwrap_or_else(|| {
                module.add_function("PyTuple_New", ptr.fn_type(&[i64_ty.into()], false), None)
            });
            let tuple = builder
                .build_call(new, &[count.into()], "tuple_new")
                .expect("build_call should not fail for PyTuple_New")
                .try_as_basic_value()
                .expect_basic("PyTuple_New returns PyObject *")
                .into_pointer_value();
            let made = builder.build_is_not_null(tuple, "tuple_made").expect("is_not_null");
            fast.require(made, "tuple_made");
            for (index, item) in packed.iter().enumerate() {
                let offset = builder
                    .build_int_add(items_at, i64_ty.const_int(8 * index as u64, false), "tuple_item_offset")
                    .expect("add");
                // SAFETY: item `index` of a tuple of `packed.len()` items.
                let slot = unsafe {
                    builder
                        .build_in_bounds_gep(context.i8_type(), tuple, &[offset], "tuple_item_slot")
                        .expect("gep")
                };
                builder.build_store(slot, *item).expect("build_store should not fail");
            }
            tuple.into()
        },
        || {
            let array = super::foreign_call::alloca_in_entry_block(
                context,
                builder,
                edge.function(),
                packed.len().max(1),
            );
            for (index, item) in packed.iter().enumerate() {
                // SAFETY: an index below the array's length.
                let slot = unsafe {
                    builder
                        .build_in_bounds_gep(ptr, array, &[i64_ty.const_int(index as u64, false)], "tuple_arg_slot")
                        .expect("gep")
                };
                builder.build_store(slot, *item).expect("build_store should not fail");
            }
            let build = shim_fn(
                module,
                crate::foreign_arith::EXT_OBJ_BUILD_DISPLAY_SYMBOL,
                ptr.fn_type(&[ptr.into(), i64_ty.into(), context.i32_type().into()], false),
            );
            builder
                .build_call(
                    build,
                    &[
                        array.into(),
                        count.into(),
                        context
                            .i32_type()
                            .const_int(pycc_mir::ObjDisplayKind::Tuple.shim_code(), false)
                            .into(),
                    ],
                    "foreign_display",
                )
                .expect("build_call should not fail for pycc_ext_obj_build_display")
                .try_as_basic_value()
                .expect_basic("pycc_ext_obj_build_display returns PyObject *")
        },
    )
    .into_pointer_value();
    route_null(context, builder, module, rt, edge, result, "foreign_display");
    Scalar::Object(result)
}

/// The helper a `tuple(a)` or `sorted(a)` of a CPython object calls instead
/// of `pycc_ext_obj_tuple` or `pycc_ext_obj_sorted` when the argument is a
/// list the call alone may hold (`src/ext/module/object_stdlib.c`, which
/// checks at run time that nothing else does):
///
/// - `tuple(a)` of a produced `a` (a comprehension, a call: not a read of
///   a variable, whose list the tuple must copy): its items move;
/// - `sorted(a)` of the list a generator expression passed to `sorted` was
///   materialized into (`pycc_hir::is_fresh_comprehension_var`), which CPython's own
///   `sorted` would have filled and sorted: sorted in place. A list display
///   or comprehension written as such is copied, as CPython copies it.
pub(super) fn fresh_symbol(call: &pycc_mir::RtCallExpr) -> Option<&'static str> {
    let [arg] = call.args.as_slice() else {
        return None;
    };
    if call.sig.ret != pycc_mir::Ty::Object {
        return None;
    }
    match call.sig.symbol {
        "pycc_ext_obj_tuple" if crate::object_release::is_produced(arg) => Some("pycc_ext_obj_tuple_fresh"),
        "pycc_ext_obj_sorted"
            if matches!(arg, pycc_mir::MirExpr::Comprehension(comp)
                if pycc_hir::is_fresh_comprehension_var(&comp.var)) =>
        {
            Some("pycc_ext_obj_sorted_fresh")
        }
        _ => None,
    }
}
