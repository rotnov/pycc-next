//! Codegen for `MirExpr::ExceptionMessage` (Part 3A of #541, #736): renders
//! a caught exception binding as its own message string, matching CPython's
//! `str(e)` semantics. Kept in its own cohesion-driven submodule rather than
//! growing `lib.rs`'s already-oversized `emit_expr_unchecked` further
//! (D-185/AGENTS.md's "keep source files decomposable" rule) -- the logic
//! here is narrowly the new codegen this issue adds, not a home for the
//! pre-existing `to_str`/`Scalar::Instance` panic path, which is untouched.
//!
//! The rendered `str` is **borrowed**: the exception object keeps owning its
//! message (#1298). `str_value_is_a_duplicate_reference` classifies
//! `MirExpr::ExceptionMessage` as a borrowed read, so every consumer retains
//! the value before it consumes it.

use super::{RtFns, Scalar, expect_instance_pointer};
use inkwell::values::PointerValue;

/// Calls the runtime's `pycc_rt_exception_message` accessor on `base_scalar`
/// (the already-evaluated exception-typed operand `MirExpr::ExceptionMessage`
/// wraps) and returns the resulting `str` scalar. Never touches
/// `exception_print_and_exit`'s own uncaught-exception `"{type}: {message}"`
/// format -- this is the message alone.
///
/// The returned `Scalar::Str` is a borrowed pointer (the accessor returns
/// the exception's own `message` field unretained, and the exception keeps
/// owning it). Callers never consume it directly: they go through
/// `incref_if_str_duplicate`, which retains it because
/// `str_value_is_a_duplicate_reference` names `MirExpr::ExceptionMessage`.
pub(super) fn emit_exception_message<'ctx>(
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    base_scalar: Scalar<'ctx>,
) -> Scalar<'ctx> {
    let base_ptr: PointerValue<'ctx> =
        expect_instance_pointer(base_scalar, "exception message read");
    let message = builder
        .build_call(
            rt.exception_message,
            &[base_ptr.into()],
            "exception_message",
        )
        .expect("build_call should not fail for a well-formed exception-message read")
        .try_as_basic_value()
        .expect_basic("pycc_rt_exception_message returns a non-void pointer")
        .into_pointer_value();
    Scalar::Str(message)
}
