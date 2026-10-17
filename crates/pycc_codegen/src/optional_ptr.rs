//! Text and truth of a pointer-represented `T | None`
//! (`scalar::pointer_optional_inner`): the value is the bare `T` pointer,
//! null for `None`, so `str()`/`print`/an f-string and a truth test branch
//! on the null before reaching `T`'s own path.

use super::*;
use inkwell::values::IntValue;
use pycc_mir::Ty;

/// `pycc_rt_optional_word_to_str(word, kind) -> str`.
fn word_to_str<'ctx>(
    context: &'ctx Context,
    module: &inkwell::module::Module<'ctx>,
) -> inkwell::values::FunctionValue<'ctx> {
    const NAME: &str = "pycc_rt_optional_word_to_str";
    module.get_function(NAME).unwrap_or_else(|| {
        let i64_type = context.i64_type();
        let ptr = context.ptr_type(inkwell::AddressSpace::default());
        module.add_function(
            NAME,
            ptr.fn_type(&[i64_type.into(), i64_type.into()], false),
            Some(inkwell::module::Linkage::External),
        )
    })
}

/// `str(value)` (a new `str`) when `ty` is a pointer-represented `T | None`;
/// `None` for any other type. `scalar` is borrowed.
pub(crate) fn to_str<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    ty: &Ty,
    scalar: &Scalar<'ctx>,
) -> Option<PointerValue<'ctx>> {
    let inner = crate::scalar::pointer_optional_inner(ty)?;
    let ptr = crate::scalar::scalar_pointer(scalar)?;
    let word = builder
        .build_ptr_to_int(ptr, context.i64_type(), "opt_word")
        .expect("build_ptr_to_int should not fail for a pointer");
    let kind = context
        .i64_type()
        .const_int(crate::containers::word_kind(rt, inner), false);
    Some(
        builder
            .build_call(word_to_str(context, module), &[word.into(), kind.into()], "opt_str")
            .expect("build_call should not fail for a declared runtime function")
            .try_as_basic_value()
            .expect_basic("pycc_rt_optional_word_to_str returns a pointer")
            .into_pointer_value(),
    )
}

/// The truth of `scalar` (an `i1`, as `str_truthy::truthy`'s) when `ty` is a pointer-represented
/// `T | None`: false for `None`, else `T`'s own truth; `None` for any other
/// type.
pub(crate) fn truthy<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    ty: &Ty,
    scalar: Scalar<'ctx>,
) -> Option<IntValue<'ctx>> {
    crate::scalar::pointer_optional_inner(ty)?;
    let ptr = crate::scalar::scalar_pointer(&scalar)?;
    let function = builder
        .get_insert_block()
        .and_then(|block| block.get_parent())
        .expect("a truth test is emitted inside a function");
    let entry = builder.get_insert_block().expect("an insertion block");
    let present_bb = context.append_basic_block(function, "opt_truthy_present");
    let join_bb = context.append_basic_block(function, "opt_truthy_join");
    let is_null = builder
        .build_is_null(ptr, "opt_truthy_null")
        .expect("build_is_null should not fail for a pointer");
    builder
        .build_conditional_branch(is_null, join_bb, present_bb)
        .expect("build_conditional_branch should not fail");
    builder.position_at_end(present_bb);
    let present = crate::str_truthy::truthy(context, builder, module, rt, scalar);
    let present_end = builder.get_insert_block().expect("an insertion block");
    builder
        .build_unconditional_branch(join_bb)
        .expect("build_unconditional_branch should not fail");
    builder.position_at_end(join_bb);
    let phi = builder
        .build_phi(context.bool_type(), "opt_truthy")
        .expect("build_phi should not fail in a fresh join block");
    phi.add_incoming(&[
        (&context.bool_type().const_zero(), entry),
        (&present, present_end),
    ]);
    Some(phi.as_basic_value().into_int_value())
}
