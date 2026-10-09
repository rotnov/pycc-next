//! The read of an `object` name an `isinstance` guard narrowed back to a
//! native type (#1476, Part 3 of #1387): `MirExpr::ObjectUnbox`.
//!
//! [`emit_unboxed`] evaluates the `object` slot and hands the pointer to
//! one of the shim's `pycc_ext_obj_unbox_*` helpers
//! ([`EXT_OBJ_UNBOX_INT_SYMBOL`] and its siblings), which writes the native
//! value through an out-slot hoisted into the entry block. A `-1` status
//! takes the foreign failure edge (`foreign_fail::route_negative`): an
//! `int` outside the inline range is `OverflowError` (#1040), and an object
//! the guard admitted but the read cannot unbox (a carrier whose `__init__`
//! never ran) is `TypeError`.
//!
//! Ownership of the result: an `int`, `float` or `bool` is a plain value; a
//! `str` is a fresh pycc copy at refcount 1, which `str_rc` already
//! classifies as owning (it names only `Name`, `AttrGet` and
//! `ExceptionMessage` as borrowed); a compiled instance is borrowed and
//! never freed (D-107, D-154).
//!
//! **The identity peephole.** Packing a narrowed read back into a
//! `PyObject *` -- a foreign call argument, a rich-comparison operand, a
//! subscript key, an attribute store, a boxing into an `object` slot --
//! would unbox and re-pack, which loses a `str` subclass's type and a
//! value's identity, and adds an `OverflowError` CPython never raises.
//! [`emit_pack_operand`] evaluates such an operand as the object it still
//! is instead; every `foreign_pack::emit_pack` caller and
//! `object_box::emit_boxed` evaluate their operand through it.
//!
//! The peephole looks only at the operand itself. A narrowed read inside a
//! native expression (a conditional expression's arm, an annotated
//! binding) is a native value by then and is packed anew, so a `str`
//! subclass is flattened there. That is D-258's documented `str`-subclass
//! deviation, pinned by `tests/issue_1476_isinstance_narrowing.rs`, not a
//! gap to close here. An arm that yields the borrowed object and one that
//! yields a fresh native value share no representation, so they cannot be
//! merged into one operand.

use super::*;
use crate::foreign_fail::{ForeignFailEdge, route_negative};
use crate::foreign_pack::shim_fn;

/// Evaluates an operand bound for `foreign_pack::emit_pack`: a narrowed
/// `object` read yields the object itself (see the module doc), anything
/// else is `emit_expr`.
pub(crate) fn emit_pack_operand<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    value: &MirExpr,
) -> Scalar<'ctx> {
    let value = match value {
        MirExpr::ObjectUnbox(object, _) => object.as_ref(),
        other => other,
    };
    emit_expr(context, builder, module, rt, user_functions, locals, value)
}

/// Emits the narrowed read of `object` (a `Scalar::Object` once evaluated)
/// as `inner`, one of `int`, `float`, `bool`, `str` or a compiled class
/// instance -- the only targets `pycc_hir::isinstance_narrow_target`
/// produces.
pub(crate) fn emit_unboxed<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    object: Scalar<'ctx>,
    inner: &pycc_mir::Ty,
) -> Scalar<'ctx> {
    let edge = ForeignFailEdge::for_current(builder);
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    let object_ptr = crate::foreign_attr::expect_object_pointer(object);
    let (symbol, slot_ty, class_name): (&str, inkwell::types::BasicTypeEnum<'ctx>, Option<&str>) =
        match inner {
            pycc_mir::Ty::Int => (EXT_OBJ_UNBOX_INT_SYMBOL, context.i64_type().into(), None),
            pycc_mir::Ty::Float => (EXT_OBJ_UNBOX_FLOAT_SYMBOL, context.f64_type().into(), None),
            pycc_mir::Ty::Bool => (EXT_OBJ_UNBOX_BOOL_SYMBOL, context.i8_type().into(), None),
            pycc_mir::Ty::Str => (EXT_OBJ_UNBOX_STR_SYMBOL, ptr.into(), None),
            pycc_mir::Ty::Instance(class) => (
                EXT_OBJ_UNBOX_INSTANCE_SYMBOL,
                ptr.into(),
                Some(class.as_str()),
            ),
            other => panic!(
                "pycc_codegen: internal error: ObjectUnbox narrows to `{}`, which no isinstance \
             guard produces -- pycc_hir::isinstance_narrow_target admits only int, float, \
             bool, str and a compiled class",
                other.name()
            ),
        };
    let out = super::build_at_entry_block(builder, edge.function(), |b| {
        b.build_alloca(slot_ty, "object_unbox_out")
            .expect("build_alloca should not fail")
    });
    let i32_type = context.i32_type();
    let status = match class_name {
        None => {
            let unbox = shim_fn(
                module,
                symbol,
                i32_type.fn_type(&[ptr.into(), ptr.into()], false),
            );
            builder.build_call(unbox, &[object_ptr.into(), out.into()], "object_unbox")
        }
        Some(class) => {
            let name_ptr = builder
                .build_global_string_ptr(class, &format!("pycc_unbox_class_{class}"))
                .expect("build_global_string_ptr should not fail")
                .as_pointer_value();
            let unbox = shim_fn(
                module,
                symbol,
                i32_type.fn_type(&[ptr.into(), ptr.into(), ptr.into()], false),
            );
            builder.build_call(
                unbox,
                &[object_ptr.into(), name_ptr.into(), out.into()],
                "object_unbox",
            )
        }
    }
    .unwrap_or_else(|_| panic!("build_call should not fail for {symbol}"))
    .try_as_basic_value()
    .expect_basic("a pycc_ext_obj_unbox_* helper returns int")
    .into_int_value();
    route_negative(context, builder, module, rt, edge, status, "object_unbox");
    let value = builder
        .build_load(slot_ty, out, "object_unbox_value")
        .expect("build_load should not fail");
    match inner {
        pycc_mir::Ty::Int => Scalar::Int(value.into_int_value()),
        pycc_mir::Ty::Float => Scalar::Float(value.into_float_value()),
        pycc_mir::Ty::Bool => Scalar::Bool(value.into_int_value()),
        pycc_mir::Ty::Str => Scalar::Str(value.into_pointer_value()),
        _ => Scalar::Instance(value.into_pointer_value()),
    }
}
