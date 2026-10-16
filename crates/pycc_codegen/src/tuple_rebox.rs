//! `MirExpr::TupleRebox`: a native tuple converted to a tuple type of the
//! same arity whose `object` slots box the elements that are native
//! there (v0.5, native containers of objects).
//!
//! A tuple whose elements are all native (`tuple[int, str]`) is returned
//! from a function declared `-> tuple[int, Any]`, which holds its second
//! element as the CPython object itself (`pycc_rt::list::LIST_KIND_OBJECT`).
//! The operand is boxed into a tuple object first when it is a struct tuple
//! of `int`/`float`/`bool` (`containers::box_struct_tuple`); the shim then
//! builds the converted tuple object, packing each element whose target
//! kind is the object one and sharing every other word
//! (`pycc_ext_obj_tuple_rebox`). The result is a new reference; the operand
//! is released when it is a temporary, and a packer's failure (an `int`
//! too large for CPython's own conversion) takes the foreign failure edge.

use super::*;
use crate::foreign_fail::{ForeignFailEdge, route_null};
use crate::foreign_pack::shim_fn;
use pycc_mir::Ty;

/// `void *pycc_ext_obj_tuple_rebox(void *tuple, const unsigned char *kinds,
/// long long n)`.
pub const EXT_OBJ_TUPLE_REBOX_SYMBOL: &str = "pycc_ext_obj_tuple_rebox";

/// Emits `value` (a native tuple) converted to the tuple type `ty`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    value: &MirExpr,
    ty: &Ty,
) -> Scalar<'ctx> {
    let Ty::Tuple(targets) = ty else {
        panic!("pycc_codegen: internal error: a tuple rebox to `{}`", ty.name())
    };
    let scalar = emit_expr(context, builder, module, rt, user_functions, locals, value);
    // The source as a tuple object, and whether it is one this node made.
    let (source, boxed_here) = match (value.ty(), scalar) {
        (Ty::Tuple(elements), Scalar::Tuple(tuple)) => (
            crate::containers::box_struct_tuple(context, builder, rt, &elements, tuple),
            true,
        ),
        (_, Scalar::List(tuple)) => (tuple, false),
        _ => panic!(
            "pycc_codegen: internal error: a tuple rebox of a `{}` value",
            value.ty().name()
        ),
    };
    let i8_type = context.i8_type();
    let kinds: Vec<IntValue<'ctx>> = targets
        .iter()
        .map(|t| i8_type.const_int(crate::containers::word_kind(rt, t), false))
        .collect();
    let array_type = i8_type.array_type(kinds.len() as u32);
    let slot = crate::entry_slots::alloca_at_entry(builder, array_type, "rebox_kinds");
    builder
        .build_store(slot, i8_type.const_array(&kinds))
        .expect("build_store should not fail for a constant byte array");
    let edge = ForeignFailEdge::for_current(builder);
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    let rebox = shim_fn(
        module,
        EXT_OBJ_TUPLE_REBOX_SYMBOL,
        ptr.fn_type(&[ptr.into(), ptr.into(), context.i64_type().into()], false),
    );
    let result = builder
        .build_call(
            rebox,
            &[
                source.into(),
                slot.into(),
                context
                    .i64_type()
                    .const_int(targets.len() as u64, false)
                    .into(),
            ],
            "tuple_rebox",
        )
        .expect("build_call should not fail for pycc_ext_obj_tuple_rebox")
        .try_as_basic_value()
        .expect_basic("pycc_ext_obj_tuple_rebox returns a pointer")
        .into_pointer_value();
    if boxed_here {
        crate::heap_rc::emit_decref(context, builder, rt, crate::heap_rc::HeapKind::List, source);
    } else {
        release_scalar_if_temporary(context, builder, rt, value, &Scalar::List(source));
    }
    route_null(context, builder, module, rt, edge, result, "tuple_rebox");
    Scalar::List(result)
}
