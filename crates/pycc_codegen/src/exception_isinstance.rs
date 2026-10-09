//! Codegen for `MirExpr::ExceptionTypeTest` (#1337, WI-6a): `isinstance` on
//! a value whose static type is a builtin exception class, decided from the
//! caught `PyExceptionObj`'s runtime type tag.
//!
//! The test is the one an `except` handler's dispatch uses
//! (`exception::emit_try`'s handler chain): an `or` over
//! `pycc_rt_exception_type_matches` for every tag in the set.

use super::{RtFns, Scalar, expect_instance_pointer};
use inkwell::values::IntValue;

/// Emits `obj_scalar`'s tag test against `tags` and returns the `bool`
/// scalar (an `i8` of 0 or 1, the runtime primitive's own encoding).
pub(super) fn emit_exception_type_test<'ctx>(
    context: &'ctx inkwell::context::Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    obj_scalar: Scalar<'ctx>,
    tags: &[u8],
) -> Scalar<'ctx> {
    let obj = expect_instance_pointer(obj_scalar, "exception isinstance test");
    let mut accumulated: Option<IntValue<'ctx>> = None;
    for tag in tags {
        let tag_val = context.i8_type().const_int(u64::from(*tag), false);
        let one = builder
            .build_call(
                rt.exception_type_matches,
                &[obj.into(), tag_val.into()],
                "isinstance_tag",
            )
            .expect("build_call should not fail for exception_type_matches")
            .try_as_basic_value()
            .expect_basic("pycc_rt_exception_type_matches returns i8")
            .into_int_value();
        accumulated = Some(match accumulated {
            Some(previous) => builder
                .build_or(previous, one, "isinstance_any")
                .expect("build_or should not fail"),
            None => one,
        });
    }
    Scalar::Bool(accumulated.expect("pycc_mir never emits an empty isinstance tag set"))
}
