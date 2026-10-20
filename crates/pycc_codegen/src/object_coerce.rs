//! Non-strict mode's run-time conversion (#3): a CPython object passed to a
//! compiled function's `int`/`float`/`bool`/`str` parameter, or returned from
//! a function declared to return one (`pycc_types::non_strict` admits it).
//!
//! The object is evaluated, handed to `pycc_ext_obj_coerce`
//! (`src/ext/pycc_ext_object_ops.c`) with the native kind and a description
//! of the slot, and the native value is read back from an entry-block
//! out-slot, as `object_unbox::emit_unboxed` does for a narrowed read. A
//! value of the wrong type raises `TypeError: f() argument 1 must be int,
//! not str` on the foreign failure edge. A produced object is held until the
//! conversion returns.

use super::*;
use crate::foreign_fail::{ForeignFailEdge, route_negative};
use crate::foreign_pack::shim_fn;
use pycc_mir::Ty;

/// `int pycc_ext_obj_coerce(PyObject *o, int kind, const char *what, void *out)`.
pub const EXT_OBJ_COERCE_SYMBOL: &str = "pycc_ext_obj_coerce";

/// Whether `value` moved into a slot of `slot_ty` is converted: the value is
/// an object and the slot one of the four scalars.
///
/// A native subscript is never an object (an object base lowers to
/// `ObjSubscript`), so its `ty()` -- which panics on malformed hand-built
/// MIR that `emit_expr` reports with its own message -- is not asked.
pub(crate) fn coerces_into(value: &MirExpr, slot_ty: &Ty) -> bool {
    (matches!(slot_ty, Ty::Int | Ty::Float | Ty::Bool | Ty::Str | Ty::Instance(_))
        || coercible_frozenset(slot_ty)
        || optional_scalar(slot_ty).is_some())
        && !matches!(value, MirExpr::Subscript { .. })
        && value.ty() == Ty::Object
}

/// The scalar of an `int | None`, `float | None`, `bool | None` or
/// `str | None` slot, which an object reaches as `None` or that scalar.
fn optional_scalar(ty: &Ty) -> Option<&Ty> {
    match ty {
        Ty::Optional(inner) if matches!(**inner, Ty::Int | Ty::Float | Ty::Bool | Ty::Str) => {
            Some(inner)
        }
        _ => None,
    }
}

/// The shim's flag on a kind code: `None` is admitted too, answered `1`
/// with the out-slot untouched.
const OR_NONE: u64 = 256;

/// A `frozenset[str]` or `frozenset[int]` slot, which an object reaches as
/// a native copy (`pycc_types::non_strict::is_coercible_frozenset`).
fn coercible_frozenset(ty: &Ty) -> bool {
    matches!(ty, Ty::FrozenSet(element) if matches!(**element, Ty::Str | Ty::Int))
        || matches!(ty, Ty::VarTuple(element) if **element == Ty::Str)
}

/// Evaluates `value` (an object) and converts it to `slot_ty`; `what` names
/// the slot in the `TypeError` (`"f() argument 1"`).
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_coerced<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    value: &MirExpr,
    slot_ty: &Ty,
    what: &str,
) -> Scalar<'ctx> {
    if let Ty::Instance(class) = slot_ty {
        return emit_instance_conversion(
            context, builder, module, rt, user_functions, locals, value, class, what,
        );
    }
    if let Some(inner) = optional_scalar(slot_ty) {
        let kind = scalar_kind(inner) + OR_NONE;
        let (loaded, status) = emit_conversion_status(
            context, builder, module, rt, user_functions, locals, value, kind, inner, what,
        );
        return optional_of(context, builder, inner, slot_ty, loaded, status);
    }
    let kind = match slot_ty {
        Ty::Int => 0,
        Ty::Float => 1,
        Ty::Bool => 2,
        Ty::Str => 3,
        Ty::FrozenSet(element) if **element == Ty::Str => 9,
        Ty::FrozenSet(element) if **element == Ty::Int => 10,
        // `tuple[str, ...]`: a native copy sharing the CPython tuple.
        Ty::VarTuple(element) if **element == Ty::Str => 11,
        other => panic!(
            "pycc_codegen: internal error: no run-time conversion of an object to `{}`",
            other.name()
        ),
    };
    emit_conversion(context, builder, module, rt, user_functions, locals, value, kind, slot_ty, what)
}

/// Non-strict mode: evaluates `value` (an object) and reads it as an index
/// for `use_` (`pycc_mir::MirExpr::ObjectIndex`), an `int`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_index<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    value: &MirExpr,
    use_: pycc_mir::IndexUse,
) -> Scalar<'ctx> {
    emit_conversion(
        context,
        builder,
        module,
        rt,
        user_functions,
        locals,
        value,
        use_.coerce_kind(),
        &Ty::Int,
        "",
    )
}

/// The shim's kind code of a scalar slot.
fn scalar_kind(ty: &Ty) -> u64 {
    match ty {
        Ty::Int => 0,
        Ty::Float => 1,
        Ty::Bool => 2,
        _ => 3,
    }
}

/// The optional `slot_ty` (of the scalar `inner`) a conversion answered:
/// `None` where the shim answered `1`, else `loaded`.
fn optional_of<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    inner: &Ty,
    slot_ty: &Ty,
    loaded: inkwell::values::BasicValueEnum<'ctx>,
    status: inkwell::values::IntValue<'ctx>,
) -> Scalar<'ctx> {
    let is_none = builder
        .build_int_compare(
            inkwell::IntPredicate::EQ,
            status,
            status.get_type().const_int(1, false),
            "object_coerce_is_none",
        )
        .expect("build_int_compare should not fail");
    if *inner == Ty::Str {
        // A `str | None` is the string pointer, null for `None`.
        let null = context.ptr_type(inkwell::AddressSpace::default()).const_null();
        let value = builder
            .build_select(is_none, null, loaded.into_pointer_value(), "object_coerce_opt")
            .expect("build_select should not fail");
        return Scalar::Str(value.into_pointer_value());
    }
    let none_payload: inkwell::values::BasicValueEnum<'ctx> = match inner {
        Ty::Int => crate::int_const::tag_smallint_const(context, 0).into(),
        Ty::Float => context.f64_type().const_zero().into(),
        _ => context.i8_type().const_zero().into(),
    };
    let payload = builder
        .build_select(is_none, none_payload, loaded, "object_coerce_payload")
        .expect("build_select should not fail");
    let present = builder
        .build_not(is_none, "object_coerce_present")
        .expect("build_not should not fail");
    let flag = builder
        .build_int_z_extend(present, context.i8_type(), "object_coerce_flag")
        .expect("build_int_z_extend should not fail");
    let pair_ty = ty_to_basic_type(context, slot_ty.clone()).into_struct_type();
    let with_payload = builder
        .build_insert_value(pair_ty.get_undef(), payload, 0, "object_coerce_pair")
        .expect("build_insert_value should not fail for field 0")
        .into_struct_value();
    let pair = builder
        .build_insert_value(with_payload, flag, 1, "object_coerce_pair")
        .expect("build_insert_value should not fail for field 1")
        .into_struct_value();
    Scalar::Optional(pair)
}

/// The one `pycc_ext_obj_coerce` call [`emit_coerced`] and [`emit_index`]
/// share: `kind` is the shim's kind code, `slot_ty` the native result.
#[allow(clippy::too_many_arguments)]
fn emit_conversion<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    value: &MirExpr,
    kind: u64,
    slot_ty: &Ty,
    what: &str,
) -> Scalar<'ctx> {
    let (loaded, _) = emit_conversion_status(
        context, builder, module, rt, user_functions, locals, value, kind, slot_ty, what,
    );
    match slot_ty {
        Ty::Int => Scalar::Int(loaded.into_int_value()),
        Ty::Float => Scalar::Float(loaded.into_float_value()),
        Ty::Bool => Scalar::Bool(loaded.into_int_value()),
        Ty::FrozenSet(_) => Scalar::Set(loaded.into_pointer_value()),
        Ty::VarTuple(_) => Scalar::List(loaded.into_pointer_value()),
        _ => Scalar::Str(loaded.into_pointer_value()),
    }
}

/// [`emit_conversion`]'s call, with the out-slot's value as loaded and the
/// shim's status (`0`, or `1` for a `None` a kind with [`OR_NONE`] admits,
/// whose out-slot holds the zero it was initialized with).
#[allow(clippy::too_many_arguments)]
fn emit_conversion_status<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    value: &MirExpr,
    kind: u64,
    slot_ty: &Ty,
    what: &str,
) -> (
    inkwell::values::BasicValueEnum<'ctx>,
    inkwell::values::IntValue<'ctx>,
) {
    let object = emit_expr(context, builder, module, rt, user_functions, locals, value);
    let held = crate::object_release::hold(context, module, rt, value, &object);
    let edge = ForeignFailEdge::for_current(builder);
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    let slot: inkwell::types::BasicTypeEnum<'ctx> = match slot_ty {
        Ty::Int => context.i64_type().into(),
        Ty::Float => context.f64_type().into(),
        Ty::Bool => context.i8_type().into(),
        _ => ptr.into(),
    };
    let out = super::build_at_entry_block(builder, edge.function(), |b| {
        b.build_alloca(slot, "object_coerce_out")
            .expect("build_alloca should not fail")
    });
    let i32_type = context.i32_type();
    let coerce = shim_fn(
        module,
        EXT_OBJ_COERCE_SYMBOL,
        i32_type.fn_type(
            &[ptr.into(), i32_type.into(), ptr.into(), ptr.into()],
            false,
        ),
    );
    let what_ptr = builder
        .build_global_string_ptr(what, "pycc_coerce_what")
        .expect("build_global_string_ptr should not fail")
        .as_pointer_value();
    let status = builder
        .build_call(
            coerce,
            &[
                crate::foreign_attr::expect_object_pointer(object).into(),
                i32_type.const_int(kind, false).into(),
                what_ptr.into(),
                out.into(),
            ],
            "object_coerce",
        )
        .expect("build_call should not fail for pycc_ext_obj_coerce")
        .try_as_basic_value()
        .expect_basic("pycc_ext_obj_coerce returns int")
        .into_int_value();
    held.release(builder, rt);
    route_negative(context, builder, module, rt, edge, status, "object_coerce");
    let loaded = builder
        .build_load(slot, out, "object_coerce_value")
        .expect("build_load should not fail");
    (loaded, status)
}

/// The module-unit entry converting a CPython object to an instance of a
/// class compiled in this module (`pycc_ext_obj_coerce_instance`): `int
/// f(PyObject *o, const char *class_name, const char *what, void **out)`.
pub const EXT_OBJ_COERCE_INSTANCE_SYMBOL: &str = "pycc_ext_obj_coerce_instance";

/// Non-strict mode: evaluates `value` (an object passed for a parameter of
/// the compiled class `class`) and reads it as the instance its carrier
/// holds, a new reference the parameter slot owns, or raises `TypeError`
/// naming `what` (`f() argument 1`).
#[allow(clippy::too_many_arguments)]
fn emit_instance_conversion<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    value: &MirExpr,
    class: &str,
    what: &str,
) -> Scalar<'ctx> {
    let object = emit_expr(context, builder, module, rt, user_functions, locals, value);
    let held = crate::object_release::hold(context, module, rt, value, &object);
    let edge = ForeignFailEdge::for_current(builder);
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    let out = super::build_at_entry_block(builder, edge.function(), |b| {
        b.build_alloca(ptr, "object_coerce_instance_out")
            .expect("build_alloca should not fail")
    });
    let i32_type = context.i32_type();
    // Another module's class: its own entry tests its own type.
    let (symbol, class) = crate::sibling_class::route(EXT_OBJ_COERCE_INSTANCE_SYMBOL, class);
    let coerce = shim_fn(
        module,
        &symbol,
        i32_type.fn_type(&[ptr.into(), ptr.into(), ptr.into(), ptr.into()], false),
    );
    let class_ptr = builder
        .build_global_string_ptr(&class, "pycc_coerce_class")
        .expect("build_global_string_ptr should not fail")
        .as_pointer_value();
    let what_ptr = builder
        .build_global_string_ptr(what, "pycc_coerce_what")
        .expect("build_global_string_ptr should not fail")
        .as_pointer_value();
    let status = builder
        .build_call(
            coerce,
            &[
                crate::foreign_attr::expect_object_pointer(object).into(),
                class_ptr.into(),
                what_ptr.into(),
                out.into(),
            ],
            "object_coerce_instance",
        )
        .expect("build_call should not fail for pycc_ext_obj_coerce_instance")
        .try_as_basic_value()
        .expect_basic("pycc_ext_obj_coerce_instance returns int")
        .into_int_value();
    held.release(builder, rt);
    route_negative(
        context,
        builder,
        module,
        rt,
        edge,
        status,
        "object_coerce_instance",
    );
    let loaded = builder
        .build_load(ptr, out, "object_coerce_instance_value")
        .expect("build_load should not fail");
    Scalar::Instance(loaded.into_pointer_value())
}
