//! Emission for a slice of a CPython object: the load `o[a:b:c]`
//! (`MirExpr::ObjSlice`, Part 2b of #1371) and the deletion `del o[a:b:c]`
//! (`MirStmt::ObjDelSlice`, Part 2c of #1371).
//!
//! Both share one evaluation order and one packer contract, so both live
//! here rather than forking into two spellings.
//!
//! **Evaluation order.** The base, then each present bound in source
//! order -- CPython's own. Every evaluated `int` temporary is protected
//! across the evaluations after it and released after the call
//! ([`emit_with_operands`]).
//!
//! **Packer contract.** The packer contract is `foreign_pack.rs`'s. Every
//! present bound is packed and handed to the shim helper, which consumes it
//! on every path, a failed packer's `NULL` included. So each operation keeps
//! exactly one failure edge: a `NULL` result for the load, and a negative
//! status for the deletion. An absent bound is passed as a null pointer with
//! its `present` bit clear (bit 0 start, bit 1 stop, bit 2 step), and the
//! helper hands `PySlice_New` CPython's `None` for it. The base is borrowed.
//!
//! **Ownership.** The load's result is a new reference
//! (`object_release::is_produced`). A produced base or bound (`o.a[p.i:]`)
//! is held across the later operands and the operation and released after
//! it (Part 1 of #1092). The deletion produces nothing that outlives the
//! call.

use super::*;
use crate::foreign_attr::expect_object_pointer;
use crate::foreign_fail::{ForeignFailEdge, route_negative, route_null};
use crate::foreign_pack::{emit_pack, shim_fn};
use inkwell::builder::Builder;
use inkwell::values::BasicMetadataValueEnum;

/// Evaluates `base` and the present `bounds` in CPython's order, hands the
/// scalars to `operation`, then releases every evaluated `int` temporary
/// and every produced CPython object operand.
///
/// Each bound's `int` temporary stays protected while the later operands
/// evaluate. The protections are popped in LIFO order before `operation`
/// runs, and the temporaries are released after it. A produced object
/// operand is held until after `operation`, whose failure edge releases it
/// (Part 1 of #1092).
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_with_operands<'ctx, T>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    base: &MirExpr,
    bounds: [Option<&MirExpr>; 3],
    operation: impl FnOnce(Scalar<'ctx>, [Option<Scalar<'ctx>>; 3]) -> T,
) -> T {
    let base_scalar = emit_expr(context, builder, module, rt, user_functions, locals, base);
    let mut holds = vec![crate::object_release::hold(
        context,
        module,
        rt,
        base,
        &base_scalar,
    )];
    let mut pendings = Vec::new();
    let scalars = bounds.map(|bound| {
        bound.map(|bound| {
            let scalar = crate::object_unbox::emit_pack_operand(
                context,
                builder,
                module,
                rt,
                user_functions,
                locals,
                bound,
            );
            holds.push(crate::object_release::hold(
                context, module, rt, bound, &scalar,
            ));
            pendings.push(push_pending_int_release_if_scalar_temporary(
                rt, bound, &scalar,
            ));
            scalar
        })
    });
    for pending in pendings.into_iter().rev() {
        pop_pending_int_release(rt, pending);
    }
    let result = operation(base_scalar, scalars);
    for held in holds.into_iter().rev() {
        held.release(builder, rt);
    }
    for (bound, scalar) in bounds.into_iter().zip(scalars) {
        if let (Some(bound), Some(scalar)) = (bound, scalar) {
            release_scalar_if_int_temporary(context, builder, rt, bound, &scalar);
        }
    }
    result
}

/// Packs `base` (borrowed) and each present bound into the shim helpers'
/// shared `(base, start, stop, step, present)` argument list.
fn slice_arguments<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    base: Scalar<'ctx>,
    bounds: [Option<Scalar<'ctx>>; 3],
) -> Vec<BasicMetadataValueEnum<'ctx>> {
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    let mut present = 0u64;
    let mut args: Vec<BasicMetadataValueEnum<'ctx>> = vec![expect_object_pointer(base).into()];
    for (bit, bound) in bounds.into_iter().enumerate() {
        let pointer = match bound {
            Some(scalar) => {
                present |= 1 << bit;
                emit_pack(context, builder, module, scalar, "foreign_slice_bound")
            }
            None => ptr.const_null(),
        };
        args.push(pointer.into());
    }
    args.push(context.i32_type().const_int(present, false).into());
    args
}

/// Declares one slice helper, `<ret> symbol(ptr, ptr, ptr, ptr, i32)`.
fn slice_helper<'ctx>(
    context: &'ctx Context,
    module: &inkwell::module::Module<'ctx>,
    symbol: &str,
    returns_status: bool,
) -> FunctionValue<'ctx> {
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    let params = [
        ptr.into(),
        ptr.into(),
        ptr.into(),
        ptr.into(),
        context.i32_type().into(),
    ];
    let fn_type = if returns_status {
        context.i32_type().fn_type(&params, false)
    } else {
        ptr.fn_type(&params, false)
    };
    shim_fn(module, symbol, fn_type)
}

/// Emits one `o[start:stop:step]` load (Part 2b of #1371), yielding the
/// result as an opaque [`Scalar::Object`]; `None` is an absent bound.
/// `pycc_ext_obj_getslice` returns `NULL` with the CPython exception set on
/// failure, which takes the failure edge.
pub(super) fn emit_slice<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    base: Scalar<'ctx>,
    bounds: [Option<Scalar<'ctx>>; 3],
) -> Scalar<'ctx> {
    let edge = ForeignFailEdge::for_current(builder);
    let args = slice_arguments(context, builder, module, base, bounds);
    let getslice = slice_helper(context, module, EXT_OBJ_GETSLICE_SYMBOL, false);
    let result = builder
        .build_call(getslice, &args, "foreign_slice")
        .expect("build_call should not fail for pycc_ext_obj_getslice")
        .try_as_basic_value()
        .expect_basic("pycc_ext_obj_getslice returns PyObject *")
        .into_pointer_value();
    route_null(context, builder, module, rt, edge, result, "foreign_slice");
    Scalar::Object(result)
}

/// Emits one `del o[start:stop:step]` (Part 2c of #1371). It has no value.
/// `pycc_ext_obj_delslice` returns `0`, or `-1` with the CPython exception
/// set, and a negative status takes the failure edge.
pub(super) fn emit_del_slice<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    base: Scalar<'ctx>,
    bounds: [Option<Scalar<'ctx>>; 3],
) {
    let edge = ForeignFailEdge::for_current(builder);
    let args = slice_arguments(context, builder, module, base, bounds);
    let delslice = slice_helper(context, module, EXT_OBJ_DELSLICE_SYMBOL, true);
    let status = builder
        .build_call(delslice, &args, "foreign_del_slice")
        .expect("build_call should not fail for pycc_ext_obj_delslice")
        .try_as_basic_value()
        .expect_basic("pycc_ext_obj_delslice returns int")
        .into_int_value();
    route_negative(
        context,
        builder,
        module,
        rt,
        edge,
        status,
        "foreign_del_slice",
    );
}
