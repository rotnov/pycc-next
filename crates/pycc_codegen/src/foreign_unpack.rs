//! Emission for `MirExpr::ObjUnpack` (Part 1 of #891): the run-time unpack
//! of a CPython object into the fixed number of targets a tuple-unpacking
//! assignment names.
//!
//! One call to [`EXT_OBJ_UNPACK_SYMBOL`] does the whole of CPython's
//! protocol -- iterate, take exactly `arity` items, refuse a short or long
//! iterable -- and answers a fresh `tuple` the unpacking temporary is bound
//! to; each target is then an ordinary `MirExpr::ObjSubscript` of that
//! tuple. The only failure edge is the `NULL` result, routed through
//! `foreign_fail.rs` like every other object operation: the module-exec
//! return in a module body outside every module-level `try`, the error
//! bridge plus the innermost exception target anywhere else (#1096). Because the targets are assigned only after this
//! call succeeds, a raising unpack binds none of them -- CPython's own
//! order.
//!
//! The tuple is a new reference bound to the unpacking temporary. In a
//! function body it is leaked on the #1092 rule for a bound value
//! (`docs/RUNTIME.md`); at module level the temporary is a module global,
//! which owns it and releases it on rebind (Part 1 of #1499,
//! `object_slot.rs`).

use super::*;
use crate::ext::EXT_OBJ_UNPACK_SYMBOL;
use crate::foreign_attr::expect_object_pointer;
use crate::foreign_fail::{ForeignFailEdge, route_null};
use crate::foreign_pack::shim_fn;
use inkwell::builder::Builder;

/// Emits the unpack of the object `value` into `arity` items, yielding the
/// items' `tuple` as an opaque [`Scalar::Object`].
pub(super) fn emit_unpack<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    value: Scalar<'ctx>,
    arity: usize,
) -> Scalar<'ctx> {
    let edge = ForeignFailEdge::for_current(builder);
    let value_ptr = expect_object_pointer(value);
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    let unpack = shim_fn(
        module,
        EXT_OBJ_UNPACK_SYMBOL,
        ptr.fn_type(&[ptr.into(), context.i64_type().into()], false),
    );
    let result = builder
        .build_call(
            unpack,
            &[
                value_ptr.into(),
                context.i64_type().const_int(arity as u64, false).into(),
            ],
            "foreign_unpack",
        )
        .expect("build_call should not fail for pycc_ext_obj_unpack")
        .try_as_basic_value()
        .expect_basic("pycc_ext_obj_unpack returns PyObject *")
        .into_pointer_value();
    route_null(context, builder, module, rt, edge, result, "foreign_unpack");
    Scalar::Object(result)
}
