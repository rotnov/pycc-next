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
    (matches!(slot_ty, Ty::Int | Ty::Float | Ty::Bool | Ty::Str) || coercible_frozenset(slot_ty))
        && !matches!(value, MirExpr::Subscript { .. })
        && value.ty() == Ty::Object
}

/// A `frozenset[str]` or `frozenset[int]` slot, which an object reaches as
/// a native copy (`pycc_types::non_strict::is_coercible_frozenset`).
fn coercible_frozenset(ty: &Ty) -> bool {
    matches!(ty, Ty::FrozenSet(element) if matches!(**element, Ty::Str | Ty::Int))
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
    let kind = match slot_ty {
        Ty::Int => 0,
        Ty::Float => 1,
        Ty::Bool => 2,
        Ty::Str => 3,
        Ty::FrozenSet(element) if **element == Ty::Str => 9,
        Ty::FrozenSet(element) if **element == Ty::Int => 10,
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
    match slot_ty {
        Ty::Int => Scalar::Int(loaded.into_int_value()),
        Ty::Float => Scalar::Float(loaded.into_float_value()),
        Ty::Bool => Scalar::Bool(loaded.into_int_value()),
        Ty::FrozenSet(_) => Scalar::Set(loaded.into_pointer_value()),
        _ => Scalar::Str(loaded.into_pointer_value()),
    }
}
