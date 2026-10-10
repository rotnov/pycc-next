//! Emission for arithmetic and item stores on a CPython object (non-strict
//! mode, #3): `MirExpr::ObjBinOp`, `MirExpr::ObjUnaryOp`,
//! `MirStmt::ObjSetItem` and `MirStmt::ObjDelItem`.
//!
//! Each is one shim helper (`src/ext/pycc_ext_object_ops.c`) over CPython's
//! own C API -- `PyNumber_*`, `PyObject_SetItem`, `PyObject_DelItem` -- so the
//! result and every exception are CPython's. The packer contract is
//! `foreign_compare.rs`'s: a native operand is boxed by a
//! `pycc_ext_obj_pack_*` helper, the shim helper consumes every packed
//! reference on every path, and an object operand is borrowed. A `NULL`
//! result (or a `-1` status) takes the single foreign failure edge.
//!
//! Evaluation order is CPython's: left then right for a binary operator;
//! value, then base, then key for `o[k] = v`; base then key for `del o[k]`.
//! A produced operand is held until the call returns, and an `int`
//! temporary is protected across a later operand's evaluation, exactly as
//! the `ObjCompare` arm in `lib.rs` does.

use super::*;
use crate::foreign_attr::expect_object_pointer;
use crate::foreign_fail::{ForeignFailEdge, route_negative, route_null};
use crate::foreign_pack::{emit_pack, none_pointer, shim_fn};
use inkwell::builder::Builder;
use pycc_mir::{BinOpKind, Ty};

/// `PyObject *pycc_ext_obj_binop(PyObject *l, PyObject *r, int op, int owned)`.
pub const EXT_OBJ_BINOP_SYMBOL: &str = "pycc_ext_obj_binop";
/// `PyObject *pycc_ext_obj_unaryop(PyObject *o, int op)`.
pub const EXT_OBJ_UNARYOP_SYMBOL: &str = "pycc_ext_obj_unaryop";
/// `int pycc_ext_obj_setitem(PyObject *o, PyObject *k, PyObject *v)`.
pub const EXT_OBJ_SETITEM_SYMBOL: &str = "pycc_ext_obj_setitem";
/// `int pycc_ext_obj_delitem(PyObject *o, PyObject *k)`.
pub const EXT_OBJ_DELITEM_SYMBOL: &str = "pycc_ext_obj_delitem";
/// `PyObject *pycc_ext_obj_build_display(PyObject **items, long long n, int kind)`.
pub const EXT_OBJ_BUILD_DISPLAY_SYMBOL: &str = "pycc_ext_obj_build_display";

/// The selector `pycc_ext_obj_binop` switches on; the C side lists the same
/// order, and adds 12 for the in-place form.
fn binop_selector(op: BinOpKind) -> u64 {
    match op {
        BinOpKind::Add => 0,
        BinOpKind::Sub => 1,
        BinOpKind::Mul => 2,
        BinOpKind::Div => 3,
        BinOpKind::FloorDiv => 4,
        BinOpKind::Mod => 5,
        BinOpKind::Pow => 6,
        BinOpKind::LShift => 7,
        BinOpKind::RShift => 8,
        BinOpKind::BitAnd => 9,
        BinOpKind::BitOr => 10,
        BinOpKind::BitXor => 11,
    }
}

/// The selector `pycc_ext_obj_unaryop` switches on.
fn unary_selector(op: pycc_mir::UnaryOpKind) -> u64 {
    match op {
        pycc_mir::UnaryOpKind::USub => 0,
        pycc_mir::UnaryOpKind::UAdd => 1,
        pycc_mir::UnaryOpKind::Invert => 2,
        pycc_mir::UnaryOpKind::Not => panic!(
            "pycc_codegen: internal error: `not` reached `ObjUnaryOp` -- pycc_mir lowers it to `Not`"
        ),
    }
}

/// The `PyObject *` for an evaluated operand and whether a packer made it.
fn operand_pointer<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    operand: Scalar<'ctx>,
) -> (PointerValue<'ctx>, bool) {
    match operand {
        Scalar::Object(pointer) => (pointer, false),
        scalar => (
            emit_pack(context, builder, module, scalar, "foreign_arith_operand"),
            true,
        ),
    }
}

/// A packed value for a store: `None` is CPython's own `None`.
fn packed_value<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    value: &MirExpr,
    scalar: Scalar<'ctx>,
) -> PointerValue<'ctx> {
    let scalar = if value.ty() == Ty::None {
        Scalar::Object(none_pointer(context, builder, module))
    } else {
        scalar
    };
    emit_pack(context, builder, module, scalar, "foreign_store_item_value")
}

/// Evaluates two operands in order, the first held (and an `int` temporary
/// protected) across the second, and hands both to `emit`; then releases.
#[allow(clippy::too_many_arguments)]
fn with_two_operands<'ctx, R>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    first: &MirExpr,
    second: &MirExpr,
    emit: impl FnOnce(Scalar<'ctx>, Scalar<'ctx>) -> R,
) -> R {
    let operand = |expr: &MirExpr| {
        crate::object_unbox::emit_pack_operand(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            expr,
        )
    };
    let a = operand(first);
    let held_a = crate::object_release::hold(context, module, rt, first, &a);
    let pending_a = push_pending_int_release_if_scalar_temporary(rt, first, &a);
    let b = operand(second);
    let held_b = crate::object_release::hold(context, module, rt, second, &b);
    pop_pending_int_release(rt, pending_a);
    let result = emit(a, b);
    held_b.release(builder, rt);
    held_a.release(builder, rt);
    release_scalar_if_int_temporary(context, builder, rt, first, &a);
    release_scalar_if_int_temporary(context, builder, rt, second, &b);
    result
}

/// Emits `left op right` with at least one object operand.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_binop<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    op: BinOpKind,
    in_place: bool,
    left: &MirExpr,
    right: &MirExpr,
) -> Scalar<'ctx> {
    with_two_operands(
        context,
        builder,
        module,
        rt,
        user_functions,
        locals,
        left,
        right,
        |l, r| {
            let edge = ForeignFailEdge::for_current(builder);
            let (l, l_owned) = operand_pointer(context, builder, module, l);
            let (r, r_owned) = operand_pointer(context, builder, module, r);
            let owned = u64::from(l_owned) | (u64::from(r_owned) << 1);
            let ptr = context.ptr_type(inkwell::AddressSpace::default());
            let i32_type = context.i32_type();
            let binop = shim_fn(
                module,
                EXT_OBJ_BINOP_SYMBOL,
                ptr.fn_type(
                    &[ptr.into(), ptr.into(), i32_type.into(), i32_type.into()],
                    false,
                ),
            );
            let result = builder
                .build_call(
                    binop,
                    &[
                        l.into(),
                        r.into(),
                        i32_type
                            .const_int(binop_selector(op) + 12 * u64::from(in_place), false)
                            .into(),
                        i32_type.const_int(owned, false).into(),
                    ],
                    "foreign_binop",
                )
                .expect("build_call should not fail for pycc_ext_obj_binop")
                .try_as_basic_value()
                .expect_basic("pycc_ext_obj_binop returns PyObject *")
                .into_pointer_value();
            route_null(context, builder, module, rt, edge, result, "foreign_binop");
            Scalar::Object(result)
        },
    )
}

/// Emits `-o`, `+o` or `~o` on an object operand.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_unary<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    op: pycc_mir::UnaryOpKind,
    operand: &MirExpr,
) -> Scalar<'ctx> {
    let value = emit_expr(
        context,
        builder,
        module,
        rt,
        user_functions,
        locals,
        operand,
    );
    let held = crate::object_release::hold(context, module, rt, operand, &value);
    let edge = ForeignFailEdge::for_current(builder);
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    let i32_type = context.i32_type();
    let unary = shim_fn(
        module,
        EXT_OBJ_UNARYOP_SYMBOL,
        ptr.fn_type(&[ptr.into(), i32_type.into()], false),
    );
    let result = builder
        .build_call(
            unary,
            &[
                expect_object_pointer(value).into(),
                i32_type.const_int(unary_selector(op), false).into(),
            ],
            "foreign_unaryop",
        )
        .expect("build_call should not fail for pycc_ext_obj_unaryop")
        .try_as_basic_value()
        .expect_basic("pycc_ext_obj_unaryop returns PyObject *")
        .into_pointer_value();
    held.release(builder, rt);
    route_null(
        context,
        builder,
        module,
        rt,
        edge,
        result,
        "foreign_unaryop",
    );
    Scalar::Object(result)
}

/// Emits `base[key] = value` on a CPython object.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_set_item<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    base: &MirExpr,
    key: &MirExpr,
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
    with_two_operands(
        context,
        builder,
        module,
        rt,
        user_functions,
        locals,
        base,
        key,
        |base_scalar, key_scalar| {
            pop_pending_int_release(rt, pending);
            let edge = ForeignFailEdge::for_current(builder);
            let packed_key = emit_pack(
                context,
                builder,
                module,
                key_scalar,
                "foreign_store_item_key",
            );
            let packed = packed_value(context, builder, module, value, value_scalar);
            let ptr = context.ptr_type(inkwell::AddressSpace::default());
            let setitem = shim_fn(
                module,
                EXT_OBJ_SETITEM_SYMBOL,
                context
                    .i32_type()
                    .fn_type(&[ptr.into(), ptr.into(), ptr.into()], false),
            );
            let status = builder
                .build_call(
                    setitem,
                    &[
                        expect_object_pointer(base_scalar).into(),
                        packed_key.into(),
                        packed.into(),
                    ],
                    "foreign_set_item",
                )
                .expect("build_call should not fail for pycc_ext_obj_setitem")
                .try_as_basic_value()
                .expect_basic("pycc_ext_obj_setitem returns int")
                .into_int_value();
            route_negative(
                context,
                builder,
                module,
                rt,
                edge,
                status,
                "foreign_set_item",
            );
        },
    );
    held_value.release(builder, rt);
    release_scalar_if_int_temporary(context, builder, rt, value, &value_scalar);
}

/// Emits `del base[key]` on a CPython object.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_del_item<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    base: &MirExpr,
    key: &MirExpr,
) {
    with_two_operands(
        context,
        builder,
        module,
        rt,
        user_functions,
        locals,
        base,
        key,
        |base_scalar, key_scalar| {
            let edge = ForeignFailEdge::for_current(builder);
            let packed_key =
                emit_pack(context, builder, module, key_scalar, "foreign_del_item_key");
            let ptr = context.ptr_type(inkwell::AddressSpace::default());
            let delitem = shim_fn(
                module,
                EXT_OBJ_DELITEM_SYMBOL,
                context.i32_type().fn_type(&[ptr.into(), ptr.into()], false),
            );
            let status = builder
                .build_call(
                    delitem,
                    &[expect_object_pointer(base_scalar).into(), packed_key.into()],
                    "foreign_del_item",
                )
                .expect("build_call should not fail for pycc_ext_obj_delitem")
                .try_as_basic_value()
                .expect_basic("pycc_ext_obj_delitem returns int")
                .into_int_value();
            route_negative(
                context,
                builder,
                module,
                rt,
                edge,
                status,
                "foreign_del_item",
            );
        },
    );
}
