//! Emission for `type(o)` on a CPython object value (`MirExpr::ObjType`,
//! Part 11 of #1371).
//!
//! One call to the shim's `pycc_ext_obj_type`, which wraps CPython's
//! `PyObject_Type`. The operand is borrowed.
//!
//! **Ownership** (`docs/RUNTIME.md`). The result is a new reference to the
//! operand's class, owned exactly as an attribute load's result is: its
//! consumer releases it when it is an unbound temporary (Part 1 of #1092,
//! `object_release.rs`), a module global that binds it owns it
//! (`object_slot.rs`, Part 1 of #1499), and it is otherwise leaked.
//!
//! **Failure.** `PyObject_Type` cannot fail for a live object. The helper
//! answers `NULL` only for a `NULL` operand, as the defence in depth
//! `pycc_ext_obj_len` documents; that `NULL` takes the
//! foreign failure edge through `foreign_fail::route_null`, so
//! `exception::expression_can_set_exception` answers `true` for the node.

use super::*;
use crate::foreign_attr::expect_object_pointer;
use crate::foreign_fail::{ForeignFailEdge, route_null};
use crate::foreign_pack::shim_fn;
use inkwell::builder::Builder;

/// Emits one `type(o)`, yielding the class as an opaque [`Scalar::Object`].
pub(super) fn emit_type<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    base: Scalar<'ctx>,
) -> Scalar<'ctx> {
    let edge = ForeignFailEdge::for_current(builder);
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    let helper = shim_fn(
        module,
        EXT_OBJ_TYPE_SYMBOL,
        ptr.fn_type(&[ptr.into()], false),
    );
    let result = builder
        .build_call(
            helper,
            &[expect_object_pointer(base).into()],
            "foreign_type",
        )
        .expect("build_call should not fail for pycc_ext_obj_type")
        .try_as_basic_value()
        .expect_basic("pycc_ext_obj_type returns PyObject *")
        .into_pointer_value();
    route_null(context, builder, module, rt, edge, result, "foreign_type");
    Scalar::Object(result)
}
