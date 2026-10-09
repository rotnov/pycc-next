//! Emission for `raise o` with a CPython object `o` (`MirStmt::ObjRaise`,
//! Part 9 of #1371).
//!
//! The operand is evaluated, then handed borrowed to
//! [`EXT_OBJ_RAISE_SYMBOL`]. The helper decides what is raised the way
//! CPython's own `raise` does and always leaves a pending pycc exception,
//! with the original CPython exception kept in the bridge table. So the
//! block ends with the same `unreachable` marker a native `MirStmt::Raise`
//! emits, and `emit_body` replaces it with a branch to the innermost
//! exception target: an enclosing `try`, the function's exceptional exit,
//! or the module body's exit, which hands the original object back to the
//! host. A produced operand is released right after the helper returns
//! (Part 3 of #1092).
//!
//! Unlike the other foreign-object operations this one takes no
//! [`crate::foreign_fail::ForeignFailEdge`]: every `raise` raises, so
//! there is no success path to branch around: the raise always ends in the
//! branch to the innermost exception target.

use super::*;
use crate::foreign_attr::expect_object_pointer;
use crate::foreign_pack::shim_fn;
use inkwell::builder::Builder;

/// Emits one `raise o` and terminates the current block.
pub(super) fn emit_obj_raise<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    value: &MirExpr,
) {
    let scalar = emit_expr(context, builder, module, rt, user_functions, locals, value);
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    let raise = shim_fn(
        module,
        EXT_OBJ_RAISE_SYMBOL,
        context.void_type().fn_type(&[ptr.into()], false),
    );
    builder
        .build_call(raise, &[expect_object_pointer(scalar).into()], "")
        .expect("build_call should not fail for pycc_ext_obj_raise");
    // Part 3 of #1092: a produced operand (`raise o.Error()`) is released
    // once the helper is done with it. The raised exception outlives it:
    // `PyErr_SetObject` takes its own reference to the instance, which the
    // bridge table then owns, and an instance keeps its class alive.
    crate::object_release::release_if_produced(context, builder, module, value, &scalar);
    // The same marker `MirStmt::Raise` leaves for `emit_body` to route.
    builder
        .build_unreachable()
        .expect("build_unreachable should not fail after an object raise");
}
