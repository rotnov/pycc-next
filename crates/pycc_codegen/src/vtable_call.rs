//! A dispatcher's foreign arm (v0.10, `pycc_mir::dispatch`): the receiver's
//! class record (`crate::class_record`) read through its layout pointer.
//!
//! An instance's layout pointer is its class record's descriptor; the
//! record's header sits before it, so the vtable pointer is at
//! `layout - 24` and its length at `layout - 16`. Only a record another
//! module of the package compiled for a subclass of one of this module's
//! classes carries a vtable.

use std::collections::HashMap;

use inkwell::context::Context;
use inkwell::values::{IntValue, PointerValue};

use crate::expr::emit_expr;
use crate::{RtFns, Scalar, StorageSlot, UserFunction};
use pycc_mir::MirExpr;

/// Offset of a record's vtable pointer from its descriptor.
const VTABLE_FROM_DESCRIPTOR: i64 = 24;
/// Offset of a record's vtable length from its descriptor.
const ENTRIES_FROM_DESCRIPTOR: i64 = 16;

fn receiver<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    expr: &MirExpr,
) -> PointerValue<'ctx> {
    match emit_expr(context, builder, module, rt, user_functions, locals, expr) {
        Scalar::Instance(receiver) => receiver,
        _ => panic!("pycc_codegen: internal error: a vtable test's receiver is an instance"),
    }
}

/// The word at `offset` bytes before `receiver`'s descriptor.
fn header_word<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    receiver: PointerValue<'ctx>,
    offset: i64,
    name: &str,
) -> PointerValue<'ctx> {
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    let layout = builder
        .build_load(ptr, receiver, "vt_layout")
        .expect("a layout load is well formed")
        .into_pointer_value();
    // SAFETY: every instance's descriptor follows a record header
    // (`crate::class_record`).
    let at = unsafe {
        builder
            .build_gep(
                context.i8_type(),
                layout,
                &[context.i64_type().const_int((-offset) as u64, true)],
                "vt_header",
            )
            .expect("a byte offset into a record is well formed")
    };
    builder
        .build_load(ptr, at, name)
        .expect("a header load is well formed")
        .into_pointer_value()
}

fn to_bool<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    bit: IntValue<'ctx>,
) -> Scalar<'ctx> {
    Scalar::Bool(
        builder
            .build_int_z_extend(bit, context.i8_type(), "vt_bool")
            .expect("a zero extension is well formed"),
    )
}

/// `$instance_foreign(recv)`: the receiver's record carries a vtable.
pub(crate) fn emit_foreign<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    args: &[MirExpr],
) -> Scalar<'ctx> {
    let [recv] = args else {
        panic!("pycc_codegen: internal error: a foreign test takes a receiver")
    };
    let recv = receiver(context, builder, module, rt, user_functions, locals, recv);
    let vtable = header_word(context, builder, recv, VTABLE_FROM_DESCRIPTOR, "vt");
    let set = builder
        .build_is_not_null(vtable, "vt_set")
        .expect("a null test is well formed");
    to_bool(context, builder, set)
}

/// Entry `index` of `recv`'s vtable (its length checked; a vtable of a
/// record is never empty), null when absent.
fn entry<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    recv: PointerValue<'ctx>,
    index: i64,
) -> PointerValue<'ctx> {
    let i64_type = context.i64_type();
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    let vtable = header_word(context, builder, recv, VTABLE_FROM_DESCRIPTOR, "vt");
    let entries = builder
        .build_ptr_to_int(
            header_word(context, builder, recv, ENTRIES_FROM_DESCRIPTOR, "vt_len"),
            i64_type,
            "vt_entries",
        )
        .expect("a pointer cast is well formed");
    let wanted = i64_type.const_int(index as u64, false);
    let inside = builder
        .build_int_compare(inkwell::IntPredicate::ULT, wanted, entries, "vt_inside")
        .expect("a comparison is well formed");
    let at = builder
        .build_select(inside, wanted, i64_type.const_zero(), "vt_at")
        .expect("a select is well formed")
        .into_int_value();
    // SAFETY: `at` is inside the vtable, which has at least one entry.
    let slot = unsafe {
        builder
            .build_gep(ptr, vtable, &[at], "vt_slot")
            .expect("an index into a vtable is well formed")
    };
    let loaded = builder
        .build_load(ptr, slot, "vt_entry")
        .expect("a vtable load is well formed")
        .into_pointer_value();
    builder
        .build_select(inside, loaded, ptr.const_null(), "vt_fn")
        .expect("a select is well formed")
        .into_pointer_value()
}

fn index_of(expr: &MirExpr) -> i64 {
    match expr {
        MirExpr::IntLiteral(index) => *index,
        _ => panic!("pycc_codegen: internal error: a vtable index is a literal"),
    }
}

/// `$instance_vt_has(recv, k)`.
pub(crate) fn emit_has<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    args: &[MirExpr],
) -> Scalar<'ctx> {
    let [recv, index] = args else {
        panic!("pycc_codegen: internal error: a vtable test takes a receiver and an index")
    };
    let recv = receiver(context, builder, module, rt, user_functions, locals, recv);
    let function = entry(context, builder, recv, index_of(index));
    let set = builder
        .build_is_not_null(function, "vt_has")
        .expect("a null test is well formed");
    to_bool(context, builder, set)
}

/// `$instance_vt_call("C.m", k, recv, args...)`: entry `k` called as `C.m`
/// is, the arguments marshalled for `C.m`'s parameters.
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_call<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    args: &[MirExpr],
    ty: &pycc_mir::Ty,
) -> Scalar<'ctx> {
    let [MirExpr::StringLiteral(default), index, rest @ ..] = args else {
        panic!("pycc_codegen: internal error: a vtable call names its signature and index")
    };
    let user_function = user_functions.get(default.as_str()).unwrap_or_else(|| {
        panic!("pycc_codegen: internal error: a vtable call's signature `{default}` is unknown")
    });
    let [recv, ..] = rest else {
        panic!("pycc_codegen: internal error: a vtable call takes a receiver")
    };
    // The entry is read before any argument is evaluated (the receiver is
    // the dispatcher's parameter, which evaluating reads only).
    let recv = receiver(context, builder, module, rt, user_functions, locals, recv);
    let function = entry(context, builder, recv, index_of(index));
    let marshalled = crate::user_call::marshal_args(
        context,
        builder,
        module,
        rt,
        user_functions,
        locals,
        &user_function.param_tys,
        default,
        rest,
    );
    let call_site = builder
        .build_indirect_call(user_function.fn_type, function, &marshalled, "vt_call")
        .expect("an indirect call is well formed");
    crate::call_result::call_result_scalar(context, call_site, ty)
}
