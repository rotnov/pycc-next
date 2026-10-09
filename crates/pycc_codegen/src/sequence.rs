//! Emission for `MirExpr::Sequence` (#1346): a receiver evaluated for its
//! effects ahead of an attribute that does not consume it --
//! `make().h(1)` for a `@staticmethod` `h`, or `make().exists(p)` for a
//! `staticmethod(<foreign callable>)` class attribute.
//!
//! Carved out of `lib.rs` per AGENTS.md's "Keep source files decomposable"
//! (tracker #545); `emit_expr_unchecked`'s arm is a one-line dispatch here.
//!
//! **Exceptions.** `discard` is emitted through the guarded `emit_expr`, so
//! a receiver that raises (`boom().h(1)`) branches to the pending-exception
//! edge before `value` -- its arguments and the call -- ever runs, which is
//! CPython's order. `value` gets its own guard the same way, which is why
//! `exception::expression_can_set_exception` answers `false` for the node
//! itself.
//!
//! **Ownership.** The discarded value is retired exactly as a bare
//! expression statement's is (`MirStmt::ExprStmt`). Today the receiver is
//! always a class instance, and class instances are not reference counted,
//! so the int-only retire is a no-op for every program that reaches here;
//! when instances gain a reference count, this retire has to grow with them.
//! The yielded value is `value`'s own, and the `bigint_rc` / `str_rc`
//! ownership classifiers answer for a `Sequence` by delegating to `value`.

use super::*;

/// `emit_expr_unchecked`'s `MirExpr::Sequence` arm: `discard`, retired,
/// then `value`, whose scalar is the result.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_sequence<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    discard: &MirExpr,
    value: &MirExpr,
) -> Scalar<'ctx> {
    let scalar = emit_expr(
        context,
        builder,
        module,
        rt,
        user_functions,
        locals,
        discard,
    );
    release_scalar_if_int_temporary(context, builder, rt, discard, &scalar);
    emit_expr(context, builder, module, rt, user_functions, locals, value)
}
