//! The MIR-level drivers for a call on a CPython object: the argument
//! evaluation every object call shares, and the whole emission of
//! `MirExpr::ObjMethodCall`, `MirExpr::ObjCall` and `MirExpr::ObjKeywordCall`
//! (Part 8 of #1371).
//!
//! `foreign_call.rs` owns the marshalling primitives and works on
//! already-evaluated [`Scalar`]s; this module is the layer above it that
//! recurses into the MIR through `emit_expr`, kept out of `lib.rs`, which
//! is far past the ~1,000-line threshold.
//!
//! **Evaluation order** is CPython's for every shape: the callee (for a
//! method call, its receiver and then the method lookup), then the
//! positional arguments left to right, then the keyword values left to
//! right. `o.missing(k=1 // 0)` therefore raises `AttributeError`, as the
//! positional `o.missing(1 // 0)` does (`foreign_call.rs`'s module doc).
//!
//! **Temporaries** (Part 1 of #1092, `object_release.rs`): a produced
//! receiver is released once the method lookup is done; the bound method,
//! or a produced callee, is held across the arguments and then consumed by
//! the call; a produced argument is held from its evaluation until after
//! the call, because the packer gives the call a reference of its own.

use super::*;
use crate::foreign_attr::expect_object_pointer;
use pycc_mir::{ObjKeywordCall, Ty};

/// The evaluated arguments of a call on a CPython object, and the holds on
/// the produced ones (Part 1 of #1092).
pub(super) struct ObjectArgs<'ctx> {
    pub(super) scalars: Vec<Scalar<'ctx>>,
    held: Vec<object_release::Held<'ctx>>,
}

impl<'ctx> ObjectArgs<'ctx> {
    /// Releases every produced argument, last first, once the call is done.
    pub(super) fn release(self, builder: &inkwell::builder::Builder<'ctx>, rt: &RtFns<'ctx>) {
        for held in self.held.into_iter().rev() {
            held.release(builder, rt);
        }
    }

    fn extend(&mut self, other: Self) {
        self.scalars.extend(other.scalars);
        self.held.extend(other.held);
    }
}

/// Evaluates each argument of a call on a CPython object, left to right,
/// into the [`Scalar`] `foreign_pack::emit_pack` marshals, holding each
/// produced one ([`ObjectArgs`]).
///
/// A `None`-typed argument (Part 8 of #1371) is still evaluated, for its
/// effects, and then replaced by CPython's own `None`
/// (`foreign_pack::none_pointer`), which packs as an `object`: pycc's
/// `None` placeholder has no `PyObject *` of its own.
pub(super) fn emit_object_args<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    args: &[MirExpr],
) -> ObjectArgs<'ctx> {
    let mut evaluated = ObjectArgs {
        scalars: Vec::with_capacity(args.len()),
        held: Vec::new(),
    };
    for arg in args {
        let value = crate::object_unbox::emit_pack_operand(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            arg,
        );
        if arg.ty() == Ty::None {
            let none = foreign_pack::none_pointer(context, builder, module);
            evaluated.scalars.push(Scalar::Object(none));
        } else {
            let held = object_release::hold(context, module, rt, arg, &value);
            evaluated.held.push(held);
            evaluated.scalars.push(value);
        }
    }
    evaluated
}

/// Emits `base.method(args)` on a CPython object (Part 2 of #1026, PR 2b of
/// #1081): the receiver, the method lookup, then each argument -- CPython's
/// own order, which is why the lookup is a step of its own
/// (`foreign_call.rs`'s module doc).
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_method_call<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    base: &MirExpr,
    method: &str,
    args: &[MirExpr],
) -> Scalar<'ctx> {
    let bound = emit_bound_method(
        context,
        builder,
        module,
        rt,
        user_functions,
        locals,
        base,
        method,
    );
    let held_bound = object_release::hold_new_reference(context, module, rt, bound);
    let args = emit_object_args(context, builder, module, rt, user_functions, locals, args);
    held_bound.consumed(rt);
    let result = foreign_call::emit_call(context, builder, module, rt, bound, &args.scalars);
    args.release(builder, rt);
    result
}

/// Emits `callee(args)` where `callee` is a CPython object (#1313): the
/// callee, then each argument, and no lookup step. A produced callee is
/// held across the arguments and then consumed by the call; a borrowed one
/// goes to the borrowing helper (`foreign_call::emit_object_call`).
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_direct_call<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    callee: &MirExpr,
    args: &[MirExpr],
) -> Scalar<'ctx> {
    let callee_scalar = emit_expr(context, builder, module, rt, user_functions, locals, callee);
    let held_callee = object_release::hold(context, module, rt, callee, &callee_scalar);
    let args = emit_object_args(context, builder, module, rt, user_functions, locals, args);
    held_callee.consumed(rt);
    let result = foreign_call::emit_object_call(
        context,
        builder,
        module,
        rt,
        callee,
        callee_scalar,
        &args.scalars,
    );
    args.release(builder, rt);
    result
}

/// The receiver and lookup half of a method call: evaluates `base`, holds
/// it across the lookup when it is produced, and releases it once the
/// bound method exists.
#[allow(clippy::too_many_arguments)]
fn emit_bound_method<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    base: &MirExpr,
    method: &str,
) -> PointerValue<'ctx> {
    let base_scalar = emit_expr(context, builder, module, rt, user_functions, locals, base);
    let held_base = object_release::hold(context, module, rt, base, &base_scalar);
    let bound = foreign_call::emit_lookup(context, builder, module, rt, base_scalar, method);
    held_base.release(builder, rt);
    bound
}

/// Emits `o.method(x, key=v)` or `callee(x, key=v)` on a CPython object
/// (Part 8 of #1371), yielding the result as an opaque [`Scalar::Object`].
///
/// `call.call` is the positional half (`pycc_mir::ObjKeywordCall`'s doc):
/// an `ObjMethodCall` is emitted like [`emit_method_call`] -- receiver,
/// lookup, arguments -- and its bound method is consumed; an `ObjCall`'s
/// callee is consumed or borrowed by `foreign_call::callee_is_produced`,
/// exactly as for a positional call. The keyword values are evaluated after
/// every positional argument and handed to `foreign_call::emit_call_kw`
/// behind them.
pub(super) fn emit_keyword_call<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    call: &ObjKeywordCall,
) -> Scalar<'ctx> {
    let (consume, callable, held_callable, positional) = match &call.call {
        MirExpr::ObjMethodCall {
            base, method, args, ..
        } => {
            let bound = emit_bound_method(
                context,
                builder,
                module,
                rt,
                user_functions,
                locals,
                base,
                method,
            );
            let held = object_release::hold_new_reference(context, module, rt, bound);
            (true, bound, held, args)
        }
        MirExpr::ObjCall { callee, args } => {
            let callee_scalar =
                emit_expr(context, builder, module, rt, user_functions, locals, callee);
            let held = object_release::hold(context, module, rt, callee, &callee_scalar);
            (
                foreign_call::callee_is_produced(callee),
                expect_object_pointer(callee_scalar),
                held,
                args,
            )
        }
        other => panic!(
            "pycc_codegen: internal error: a keyword call's positional half must be an \
             `ObjMethodCall` or an `ObjCall`, got {other:?}"
        ),
    };
    let mut args = emit_object_args(
        context,
        builder,
        module,
        rt,
        user_functions,
        locals,
        positional,
    );
    args.extend(emit_object_args(
        context,
        builder,
        module,
        rt,
        user_functions,
        locals,
        &call.values,
    ));
    held_callable.consumed(rt);
    let result = foreign_call::emit_call_kw(
        context,
        builder,
        module,
        rt,
        consume,
        callable,
        &args.scalars,
        &call.names,
    );
    args.release(builder, rt);
    result
}
