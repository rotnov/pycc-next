//! Emission for storing and deleting an attribute of a CPython object:
//! `o.x = v` (`MirStmt::ObjAttrSet`) and `del o.x` (`MirStmt::ObjDelAttr`),
//! #1457 (Part 2 of #1443).
//!
//! **Evaluation order.** For a store, the value, then the base -- CPython's
//! own order for an attribute assignment (`v` is evaluated before `o` in
//! `o.x = v`). An `int` value temporary is protected while the base
//! evaluates and released after the call, as `foreign_slice.rs` does for a
//! bound. A deletion evaluates only its base.
//!
//! **Packer contract.** `foreign_pack.rs`'s: the value goes through exactly
//! one `pycc_ext_obj_pack_*` helper (a `None` value becomes CPython's own
//! `None` through [`none_pointer`], as a `None` call argument does), and
//! `pycc_ext_obj_setattr` consumes that new reference on every path,
//! including a `NULL` from a failed packer, which it reports as `-1`. The
//! base is borrowed.
//!
//! **Temporaries** (Part 1 of #1092): a produced value (`o.x = p.y`) is
//! held while the base evaluates, and a produced base (`o.a.x = v`) is
//! held across the call; both are released right after the call, before
//! its status is tested, so neither path out of the store releases twice.
//!
//! **Failure.** Both helpers return `0`, or `-1` with the CPython exception
//! set, and a negative status takes the foreign-failure edge
//! ([`ForeignFailEdge`]): the module-exec failure return in a module body
//! outside every module-level `try`, and the error bridge everywhere else
//! (#1096).

use super::*;
use crate::foreign_attr::expect_object_pointer;
use crate::foreign_fail::{ForeignFailEdge, route_negative};
use crate::foreign_pack::{emit_pack, none_pointer, shim_fn};
use inkwell::builder::Builder;
use pycc_mir::Ty;

/// The attribute name as a private global C string, shared by both helpers.
fn attr_name<'ctx>(builder: &Builder<'ctx>, attr: &str) -> PointerValue<'ctx> {
    builder
        .build_global_string_ptr(attr, &format!("pycc_foreign_attr_{attr}"))
        .expect("build_global_string_ptr should not fail")
        .as_pointer_value()
}

/// Emits one `base.attr = value` on a CPython object (#1457).
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_set_attr<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    base: &MirExpr,
    attr: &str,
    value: &MirExpr,
) {
    let value_scalar = crate::object_unbox::emit_pack_operand(
        context,
        builder,
        module,
        rt,
        user_functions,
        locals,
        value,
    );
    let held_value = crate::object_release::hold(context, module, rt, value, &value_scalar);
    let pending = push_pending_int_release_if_scalar_temporary(rt, value, &value_scalar);
    let base_scalar = emit_expr(context, builder, module, rt, user_functions, locals, base);
    let held_base = crate::object_release::hold(context, module, rt, base, &base_scalar);
    pop_pending_int_release(rt, pending);
    let edge = ForeignFailEdge::for_current(builder);
    let packed = if value.ty() == Ty::None {
        let none = none_pointer(context, builder, module);
        emit_pack(
            context,
            builder,
            module,
            Scalar::Object(none),
            "foreign_store_value",
        )
    } else {
        emit_pack(
            context,
            builder,
            module,
            value_scalar,
            "foreign_store_value",
        )
    };
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    let setattr = shim_fn(
        module,
        EXT_OBJ_SETATTR_SYMBOL,
        context
            .i32_type()
            .fn_type(&[ptr.into(), ptr.into(), ptr.into()], false),
    );
    let name = attr_name(builder, attr);
    let status = builder
        .build_call(
            setattr,
            &[
                expect_object_pointer(base_scalar).into(),
                name.into(),
                packed.into(),
            ],
            "foreign_set_attr",
        )
        .expect("build_call should not fail for pycc_ext_obj_setattr")
        .try_as_basic_value()
        .expect_basic("pycc_ext_obj_setattr returns int")
        .into_int_value();
    held_base.release(builder, rt);
    held_value.release(builder, rt);
    release_scalar_if_int_temporary(context, builder, rt, value, &value_scalar);
    route_negative(
        context,
        builder,
        module,
        rt,
        edge,
        status,
        "foreign_set_attr",
    );
}

/// Emits one `del base.attr` on a CPython object (#1457).
pub(super) fn emit_del_attr<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    base: Scalar<'ctx>,
    attr: &str,
) {
    let edge = ForeignFailEdge::for_current(builder);
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    let delattr = shim_fn(
        module,
        EXT_OBJ_DELATTR_SYMBOL,
        context.i32_type().fn_type(&[ptr.into(), ptr.into()], false),
    );
    let name = attr_name(builder, attr);
    let status = builder
        .build_call(
            delattr,
            &[expect_object_pointer(base).into(), name.into()],
            "foreign_del_attr",
        )
        .expect("build_call should not fail for pycc_ext_obj_delattr")
        .try_as_basic_value()
        .expect_basic("pycc_ext_obj_delattr returns int")
        .into_int_value();
    route_negative(
        context,
        builder,
        module,
        rt,
        edge,
        status,
        "foreign_del_attr",
    );
}
