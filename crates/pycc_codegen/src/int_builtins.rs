//! `abs(x)`, `divmod(a, b)` and `int(x)` over native numbers, as
//! `pycc_types::int_builtins` admits them.
//!
//! Each operand is evaluated once; a fresh `int` operand's birth reference is
//! protected across a sibling's evaluation and released after the runtime
//! call, exactly as `emit_expr`'s `BinOp` arm does. A user function of the
//! same name wins, as in `pycc_types` and `pycc_mir`.

use super::{
    RtFns, Scalar, StorageSlot, UserFunction, emit_expr, pop_pending_int_release,
    push_pending_int_release_if_scalar_temporary, release_if_int_temporary,
    release_scalar_if_int_temporary, retain_if_int_duplicate, to_numeric_encoded_int,
    ty_to_basic_type,
};
use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::values::IntValue;
use pycc_mir::{MirExpr, Ty};
use std::collections::HashMap;

fn call_int<'ctx>(
    builder: &Builder<'ctx>,
    function: inkwell::values::FunctionValue<'ctx>,
    args: &[inkwell::values::BasicMetadataValueEnum<'ctx>],
    name: &str,
) -> IntValue<'ctx> {
    builder
        .build_call(function, args, name)
        .expect("build_call should not fail for a runtime int builtin")
        .try_as_basic_value()
        .expect_basic("the runtime int builtin returns an encoded int")
        .into_int_value()
}

/// `abs` of the `int` word `word`: inline for a smallint (`2v + 1`, whose
/// absolute value is `2 - word` when negative), except the most negative
/// one, whose absolute value leaves the smallint range; through
/// `pycc_rt_int_abs` for that, a bigint or a bool marker.
fn emit_int_abs<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    rt: &RtFns<'ctx>,
    word: IntValue<'ctx>,
) -> IntValue<'ctx> {
    use inkwell::IntPredicate;
    let i64_type = context.i64_type();
    let function = builder
        .get_insert_block()
        .and_then(|b| b.get_parent())
        .expect("inside a function");
    let odd = builder
        .build_and(word, i64_type.const_int(1, false), "abs_low")
        .expect("build_and should not fail");
    let small = builder
        .build_int_compare(IntPredicate::NE, odd, i64_type.const_zero(), "abs_small")
        .expect("build_int_compare should not fail");
    let not_min = builder
        .build_int_compare(
            IntPredicate::NE,
            word,
            i64_type.const_int((i64::MIN + 1) as u64, true),
            "abs_not_min",
        )
        .expect("build_int_compare should not fail");
    let fast = builder
        .build_and(small, not_min, "abs_fast")
        .expect("build_and should not fail");
    let fast_bb = context.append_basic_block(function, "abs_inline");
    let slow_bb = context.append_basic_block(function, "abs_call");
    let join_bb = context.append_basic_block(function, "abs_join");
    crate::int_fast::likely_branch(context, builder, fast, fast_bb, slow_bb);
    builder.position_at_end(fast_bb);
    let negative = builder
        .build_int_compare(IntPredicate::SLT, word, i64_type.const_zero(), "abs_neg")
        .expect("build_int_compare should not fail");
    let flipped = builder
        .build_int_sub(i64_type.const_int(2, false), word, "abs_flip")
        .expect("build_int_sub should not fail");
    let inline = builder
        .build_select(negative, flipped, word, "abs_inline_value")
        .expect("build_select should not fail")
        .into_int_value();
    builder
        .build_unconditional_branch(join_bb)
        .expect("br should not fail");
    builder.position_at_end(slow_bb);
    let called = call_int(builder, rt.int_abs, &[word.into()], "int_abs");
    let slow_end = builder.get_insert_block().expect("positioned");
    builder
        .build_unconditional_branch(join_bb)
        .expect("br should not fail");
    builder.position_at_end(join_bb);
    let phi = builder
        .build_phi(i64_type, "abs")
        .expect("build_phi should not fail");
    phi.add_incoming(&[(&inline, fast_bb), (&called, slow_end)]);
    phi.as_basic_value().into_int_value()
}

/// Emits the call when `callee` is one of the three builtins over native
/// operands, `None` otherwise (including `int(o)` of a CPython object).
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_call<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    callee: &str,
    args: &[MirExpr],
) -> Option<Scalar<'ctx>> {
    if user_functions.contains_key(callee) {
        return None;
    }
    let emit =
        |expr: &MirExpr| emit_expr(context, builder, module, rt, user_functions, locals, expr);
    match (callee, args) {
        ("abs", [x]) if x.ty() == Ty::Float => {
            let Scalar::Float(v) = emit(x) else {
                panic!("pycc_codegen: internal error: a float `abs` operand was not a float")
            };
            let fabs = inkwell::intrinsics::Intrinsic::find("llvm.fabs")
                .and_then(|i| i.get_declaration(module, &[context.f64_type().into()]))
                .expect("LLVM provides llvm.fabs");
            let result = builder
                .build_call(fabs, &[v.into()], "fabs")
                .expect("build_call should not fail for llvm.fabs")
                .try_as_basic_value()
                .expect_basic("llvm.fabs returns f64")
                .into_float_value();
            Some(Scalar::Float(result))
        }
        ("abs", [x]) if matches!(x.ty(), Ty::Int | Ty::Bool) => {
            let scalar = emit(x);
            let word = to_numeric_encoded_int(context, builder, scalar);
            let result = if x.ty() == Ty::Int && rt.int_ranges.borrow().of(x).is_some() {
                // A smallint (`int_range`): `2v + 1` negated is `2 - word`.
                crate::int_small::emit_abs(context, builder, word)
            } else {
                emit_int_abs(context, builder, rt, word)
            };
            release_if_int_temporary(context, builder, rt, x, word);
            Some(Scalar::Int(result))
        }
        ("int", [x]) if x.ty() == Ty::Float => {
            let Scalar::Float(v) = emit(x) else {
                panic!("pycc_codegen: internal error: a float `int` operand was not a float")
            };
            Some(Scalar::Int(call_int(
                builder,
                rt.int_from_float,
                &[v.into()],
                "int_from_float",
            )))
        }
        // `int(n)` of an int is the value itself; of a bool its smallint.
        // The operand's reference, if it owns one, becomes the result's.
        ("int", [x]) if matches!(x.ty(), Ty::Int | Ty::Bool) => {
            let scalar = retain_if_int_duplicate(context, builder, rt, x, emit(x));
            Some(Scalar::Int(to_numeric_encoded_int(
                context, builder, scalar,
            )))
        }
        ("divmod", [a, b])
            if matches!(a.ty(), Ty::Int | Ty::Bool) && matches!(b.ty(), Ty::Int | Ty::Bool) =>
        {
            let left = emit(a);
            let pending = push_pending_int_release_if_scalar_temporary(rt, a, &left);
            let right = emit(b);
            pop_pending_int_release(rt, pending);
            let lw = to_numeric_encoded_int(context, builder, left);
            let rw = to_numeric_encoded_int(context, builder, right);
            // The quotient raises CPython's `divmod` message for a zero
            // divisor; the remainder function never raises.
            let quotient = call_int(
                builder,
                rt.int_floordiv,
                &[lw.into(), rw.into()],
                "divmod_q",
            );
            let remainder = call_int(
                builder,
                rt.int_mod_noraise,
                &[lw.into(), rw.into()],
                "divmod_r",
            );
            release_scalar_if_int_temporary(context, builder, rt, a, &Scalar::Int(lw));
            release_scalar_if_int_temporary(context, builder, rt, b, &Scalar::Int(rw));
            let tuple_ty = ty_to_basic_type(context, Ty::Tuple(Box::new(vec![Ty::Int, Ty::Int])))
                .into_struct_type();
            let with_q = builder
                .build_insert_value(tuple_ty.get_undef(), quotient, 0, "divmod_q_field")
                .expect("insert field 0");
            let with_r = builder
                .build_insert_value(with_q, remainder, 1, "divmod_r_field")
                .expect("insert field 1");
            Some(Scalar::Tuple(with_r.into_struct_value()))
        }
        _ => None,
    }
}
