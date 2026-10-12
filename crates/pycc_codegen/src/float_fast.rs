//! Inline fast paths for `float` `/` and `**`, and the `int` operand
//! conversion of a `float` operation.
//!
//! Each keeps the runtime function (`pycc_rt_float_div`,
//! `pycc_rt_float_pow`, `pycc_rt_int_to_float`) as its cold path and emits
//! the pending-exception guard there, and only there: the fast path cannot
//! raise. `exception::expression_can_set_exception` therefore leaves these
//! operations out of the nodes `emit_expr` guards after the fact, which
//! keeps a global load and a branch out of every float division in a loop.
//!
//! `**` stays a call to libm's `pow` -- the function CPython's `float_pow`
//! and `pycc_rt_float_pow` both use -- on the domain where CPython raises
//! nothing (a positive finite base, a finite exponent, a finite result).
//! An exponent of `0.5` is *not* simply `sqrt`: glibc's `pow(x, 0.5)` and
//! `sqrt(x)` differ in the last bit for roughly one input in a thousand, and
//! CPython prints `pow`'s. [`emit_pow_half`] uses `sqrt` only where glibc's
//! documented error bound proves `pow` returns the same double, and calls
//! `pow` elsewhere. Every `pow` call carries `nobuiltin` because LLVM's
//! library-call simplifier rewrites `pow(x, 0.5)` to `sqrt(x)` as soon as it
//! can prove `x` finite, which the fast path's own guard proves.

use crate::Scalar;
use crate::int_fast::likely_branch;
use crate::rt_fns::RtFns;
use inkwell::FloatPredicate;
use inkwell::attributes::{Attribute, AttributeLoc};
use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::values::{FloatValue, FunctionValue, IntValue};

/// A numeric operand of a `float` operation as `f64`: an `int` through the
/// guarded conversion, anything else through `crate::to_float`.
pub(super) fn operand<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    rt: &RtFns<'ctx>,
    scalar: Scalar<'ctx>,
) -> FloatValue<'ctx> {
    match scalar {
        Scalar::Int(word) => crate::int_fast::emit_int_to_float_guarded(context, builder, rt, word),
        other => crate::to_float(context, builder, rt, other),
    }
}

fn current_function<'ctx>(builder: &Builder<'ctx>) -> FunctionValue<'ctx> {
    builder
        .get_insert_block()
        .and_then(|block| block.get_parent())
        .expect("float fast paths are emitted inside a function body")
}

/// Branches on `fast_ok`: the `fast` block's value, or the runtime
/// function `slow_fn(l, r)` followed by the pending-exception guard. `fast`
/// may return a second `i1` that sends the fast path to the runtime too.
fn emit_guarded<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    rt: &RtFns<'ctx>,
    slow_fn: FunctionValue<'ctx>,
    l: FloatValue<'ctx>,
    r: FloatValue<'ctx>,
    fast_ok: IntValue<'ctx>,
    fast: impl FnOnce(&Builder<'ctx>) -> (FloatValue<'ctx>, Option<IntValue<'ctx>>),
) -> FloatValue<'ctx> {
    let function = current_function(builder);
    let fast_bb = context.append_basic_block(function, "float_fast");
    let slow_bb = context.append_basic_block(function, "float_slow");
    let join_bb = context.append_basic_block(function, "float_join");
    likely_branch(context, builder, fast_ok, fast_bb, slow_bb);

    builder.position_at_end(fast_bb);
    let (fast_value, bail) = fast(builder);
    let fast_end = builder.get_insert_block().expect("positioned");
    match bail {
        Some(bail) => {
            let not_bail = builder.build_not(bail, "float_fast_done").expect("not");
            likely_branch(context, builder, not_bail, join_bb, slow_bb);
        }
        None => {
            builder.build_unconditional_branch(join_bb).expect("branch");
        }
    }

    builder.position_at_end(slow_bb);
    let slow_value = builder
        .build_call(slow_fn, &[l.into(), r.into()], "float_slow_call")
        .expect("build_call should not fail for a runtime float function")
        .try_as_basic_value()
        .expect_basic("pycc_rt_float_* functions return f64")
        .into_float_value();
    crate::exception::guard_statement_effects(context, builder, rt);
    let slow_end = builder.get_insert_block().expect("positioned");
    builder.build_unconditional_branch(join_bb).expect("branch");

    builder.position_at_end(join_bb);
    let phi = builder
        .build_phi(context.f64_type(), "float_result")
        .expect("build_phi in a fresh block");
    phi.add_incoming(&[(&fast_value, fast_end), (&slow_value, slow_end)]);
    phi.as_basic_value().into_float_value()
}

/// `l / r`: an inline division unless `r` is a zero (of either sign), which
/// raises `ZeroDivisionError` in the runtime. A NaN divisor divides inline.
pub(super) fn emit_div<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    rt: &RtFns<'ctx>,
    l: FloatValue<'ctx>,
    r: FloatValue<'ctx>,
) -> FloatValue<'ctx> {
    let zero = context.f64_type().const_zero();
    let nonzero = builder
        .build_float_compare(FloatPredicate::UNE, r, zero, "divisor_nonzero")
        .expect("fcmp");
    let function = current_function(builder);
    let fast_bb = context.append_basic_block(function, "fdiv_fast");
    let zero_bb = context.append_basic_block(function, "fdiv_by_zero");
    likely_branch(context, builder, nonzero, fast_bb, zero_bb);
    // A zero divisor always raises `ZeroDivisionError`, so this cold path
    // leaves for the exception target and never rejoins the hot path.
    builder.position_at_end(zero_bb);
    builder
        .build_call(rt.float_div, &[l.into(), r.into()], "float_div_raise")
        .expect("build_call should not fail for pycc_rt_float_div");
    crate::exception::jump_to_exception_target(context, builder, rt);
    builder.position_at_end(fast_bb);
    builder.build_float_div(l, r, "fdiv").expect("fdiv")
}

/// Whether the target's libm is glibc (`None` is the host): the only libm
/// whose `pow` error bound [`emit_pow`]'s square-root proof relies on.
pub(super) fn target_libm_is_glibc(target_triple: Option<&str>) -> bool {
    match target_triple {
        Some(triple) => triple.contains("linux-gnu"),
        None => cfg!(all(target_os = "linux", target_env = "gnu")),
    }
}

/// glibc documents its `pow` (`sysdeps/ieee754/dbl-64/e_pow.c`) as
/// "Worst-case error: 0.54 ULP", with no slow exact path. If the exact
/// square root of `x` lies within 0.44 ULP of `s = sqrt(x)` (correctly
/// rounded), every other double is more than 0.56 ULP from it, so
/// `pow(x, 0.5)` can only return `s`.
///
/// The test is exact integer arithmetic. With `s = S * 2^(e-52)` (`S` the
/// 53-bit significand) and `x = X * 2^(f-52)`, the residual `x - s^2` in
/// units of `2^(2e-104)` is the integer `R = X * 2^k - S^2`, `k = f - 2e + 52`
/// (52 or 53, as `x` is within an ULP of `s^2`). `s` correctly rounded
/// bounds `|R| <= S + 1/4 < 2^53`, so `R` is its own value modulo `2^64`:
/// `(x_bits << k) - S*S` with wrapping 64-bit arithmetic (`k >= 52` shifts
/// out every bit above the significand's low ones). `|sqrt(x) - s| < 0.44`
/// ULP is `|R| < 0.88 * S`, tested as `|R| <= S * 225 / 256` (0.8789).
/// The bound holds for every positive normal `x` (`k` is 51 when `s`
/// rounds up to a power of two). Checked against glibc 2.39 on 620 million
/// inputs across every normal binade, a quarter of them next to a rounding
/// midpoint and a quarter just below an odd power of two: no fast-path
/// result differed from `pow`.
const SQRT_PROOF_NUMERATOR: u64 = 225;

/// `x ** 0.5` for a positive normal `x` on glibc: `s = sqrt(x)` when the
/// residual `x - s*s` proves glibc's `pow` returns `s` (see
/// [`SQRT_PROOF_NUMERATOR`]), libm `pow` otherwise -- bit-identical to
/// CPython either way.
fn emit_pow_half<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    rt: &RtFns<'ctx>,
    x: FloatValue<'ctx>,
    half: FloatValue<'ctx>,
) -> FloatValue<'ctx> {
    let f64_type = context.f64_type();
    let i64_type = context.i64_type();
    let s = builder
        .build_call(rt.sqrt_f64, &[x.into()], "sqrt")
        .expect("build_call should not fail for an f64 intrinsic")
        .try_as_basic_value()
        .expect_basic("the intrinsic returns f64")
        .into_float_value();
    let k = |n: u64| i64_type.const_int(n, false);
    let int = |v: FloatValue<'ctx>, name| {
        builder
            .build_bit_cast(v, i64_type, name)
            .expect("bitcast")
            .into_int_value()
    };
    let x_bits = int(x, "x_bits");
    let s_bits = int(s, "s_bits");
    let mantissa = builder
        .build_and(s_bits, k((1 << 52) - 1), "s_frac")
        .expect("and");
    let sig = builder.build_or(mantissa, k(1 << 52), "s_sig").expect("or");
    let sig_sq = builder.build_int_mul(sig, sig, "s_sig_sq").expect("mul");
    // `k = f - 2e + 52` from the biased exponents (both operands positive).
    let x_exp = builder
        .build_right_shift(x_bits, k(52), false, "x_exp")
        .expect("lshr");
    let s_exp = builder
        .build_right_shift(s_bits, k(52), false, "s_exp")
        .expect("lshr");
    let twice = builder.build_int_add(s_exp, s_exp, "s_exp2").expect("add");
    let diff = builder.build_int_sub(x_exp, twice, "exp_diff").expect("sub");
    let shift = builder.build_int_add(diff, k(1075), "res_shift").expect("add");
    // Always 52 or 53; the mask keeps the shift defined in LLVM's eyes.
    let shift = builder.build_and(shift, k(63), "res_shift_m").expect("and");
    let scaled_x = builder
        .build_left_shift(x_bits, shift, "x_scaled")
        .expect("shl");
    let residual = builder
        .build_int_sub(scaled_x, sig_sq, "sqrt_residual")
        .expect("sub");
    let limit = builder
        .build_right_shift(
            builder
                .build_int_mul(sig, k(SQRT_PROOF_NUMERATOR), "lim_mul")
                .expect("mul"),
            k(8),
            false,
            "residual_limit",
        )
        .expect("lshr");
    // `-limit <= residual <= limit` as one unsigned compare.
    let biased = builder.build_int_add(residual, limit, "res_biased").expect("add");
    let span = builder.build_int_add(limit, limit, "res_span").expect("add");
    let proven = builder
        .build_int_compare(inkwell::IntPredicate::ULE, biased, span, "sqrt_proven")
        .expect("icmp");

    let function = current_function(builder);
    let sqrt_bb = builder.get_insert_block().expect("positioned");
    let pow_bb = context.append_basic_block(function, "pow_half_libm");
    let done_bb = context.append_basic_block(function, "pow_half_done");
    likely_branch(context, builder, proven, done_bb, pow_bb);
    builder.position_at_end(pow_bb);
    let from_pow = emit_libm_pow(context, builder, rt, x, half);
    builder.build_unconditional_branch(done_bb).expect("branch");
    builder.position_at_end(done_bb);
    let phi = builder.build_phi(f64_type, "pow_half").expect("phi");
    phi.add_incoming(&[(&s, sqrt_bb), (&from_pow, pow_bb)]);
    phi.as_basic_value().into_float_value()
}

/// A direct call of libm `pow`, marked `nobuiltin` so LLVM keeps it a call
/// (see this module's doc comment), and `memory(none)`: the only memory it
/// writes is `errno`, which compiled code never reads, so loads stay
/// reusable across it.
fn emit_libm_pow<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    rt: &RtFns<'ctx>,
    l: FloatValue<'ctx>,
    r: FloatValue<'ctx>,
) -> FloatValue<'ctx> {
    let call = builder
        .build_call(rt.libm_pow, &[l.into(), r.into()], "pow")
        .expect("build_call should not fail for libm pow");
    for (name, value) in [("nobuiltin", 0), ("memory", 0), ("nounwind", 0), ("willreturn", 0)] {
        let attribute = context.create_enum_attribute(Attribute::get_named_enum_kind_id(name), value);
        call.add_attribute(AttributeLoc::Function, attribute);
    }
    call.try_as_basic_value()
        .expect_basic("pow returns f64")
        .into_float_value()
}

/// `l ** r`: libm `pow` inline when `0 < l < inf`, `r` is finite and the
/// result is finite -- the domain where CPython's `float_pow` reaches its own
/// `pow` call and raises nothing; the runtime (zero base with a negative
/// exponent, a negative base, non-finite operands, overflow) otherwise.
/// `exponent_literal` is `r` when the source wrote a float literal: a
/// literal `0.5` on glibc takes [`emit_pow_half`].
pub(super) fn emit_pow<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    rt: &RtFns<'ctx>,
    l: FloatValue<'ctx>,
    r: FloatValue<'ctx>,
    exponent_literal: Option<f64>,
) -> FloatValue<'ctx> {
    if exponent_literal == Some(0.5) && rt.glibc_libm.get() {
        let f64_type = context.f64_type();
        let cmp = |pred, bound: f64, name| {
            builder
                .build_float_compare(pred, l, f64_type.const_float(bound), name)
                .expect("fcmp")
        };
        // A positive normal base (one unsigned compare of its bits: false
        // for zero, a subnormal, a negative, an infinity and a NaN) raises
        // nothing, and is where `emit_pow_half`'s integer residual is exact.
        // It is tested first, alone. Any other base takes the general
        // guarded `pow` below (libm for a positive finite base, the runtime
        // otherwise).
        let i64_type = context.i64_type();
        let bits = builder
            .build_bit_cast(l, i64_type, "base_bits")
            .expect("bitcast")
            .into_int_value();
        let above_min = builder
            .build_int_sub(bits, i64_type.const_int(1 << 52, false), "base_off")
            .expect("sub");
        let in_range = builder
            .build_int_compare(
                inkwell::IntPredicate::ULT,
                above_min,
                i64_type.const_int(0x7FF0_0000_0000_0000 - (1 << 52), false),
                "pow_half_range",
            )
            .expect("icmp");
        let function = current_function(builder);
        let proof_bb = context.append_basic_block(function, "pow_half_proof");
        let far_bb = context.append_basic_block(function, "pow_half_far");
        let done_bb = context.append_basic_block(function, "pow_half_join");
        likely_branch(context, builder, in_range, proof_bb, far_bb);
        builder.position_at_end(proof_bb);
        let proven = emit_pow_half(context, builder, rt, l, r);
        let proof_end = builder.get_insert_block().expect("positioned");
        builder.build_unconditional_branch(done_bb).expect("branch");
        // Every other base: a negative finite one always raises (CPython's
        // result would be complex) and leaves for the exception target, so
        // the runtime call never rejoins the hot path and LLVM may keep
        // loads across this expression; every other base (a zero, a
        // subnormal, an infinity, a NaN) is libm `pow`, which raises nothing
        // for an exponent of 0.5.
        builder.position_at_end(far_bb);
        let negative = cmp(FloatPredicate::OLT, 0.0, "base_neg");
        let finite = cmp(FloatPredicate::OGT, f64::NEG_INFINITY, "base_fin");
        let raises = builder.build_and(negative, finite, "pow_half_raises").expect("and");
        let raise_bb = context.append_basic_block(function, "pow_half_raise");
        let libm_bb = context.append_basic_block(function, "pow_half_libm_far");
        let keep = builder.build_not(raises, "pow_half_keeps").expect("not");
        likely_branch(context, builder, keep, libm_bb, raise_bb);
        builder.position_at_end(raise_bb);
        builder
            .build_call(rt.float_pow, &[l.into(), r.into()], "float_pow_raise")
            .expect("build_call should not fail for pycc_rt_float_pow");
        crate::exception::jump_to_exception_target(context, builder, rt);
        builder.position_at_end(libm_bb);
        let far = emit_libm_pow(context, builder, rt, l, r);
        let far_end = builder.get_insert_block().expect("positioned");
        builder.build_unconditional_branch(done_bb).expect("branch");
        builder.position_at_end(done_bb);
        let phi = builder.build_phi(f64_type, "pow_half_value").expect("phi");
        phi.add_incoming(&[(&proven, proof_end), (&far, far_end)]);
        return phi.as_basic_value().into_float_value();
    }
    let f64_type = context.f64_type();
    let inf = f64_type.const_float(f64::INFINITY);
    let neg_inf = f64_type.const_float(f64::NEG_INFINITY);
    let cmp = |pred, a, b, name| {
        builder
            .build_float_compare(pred, a, b, name)
            .expect("fcmp")
    };
    let base_pos = cmp(FloatPredicate::OGT, l, f64_type.const_zero(), "base_pos");
    let base_fin = cmp(FloatPredicate::OLT, l, inf, "base_fin");
    let exp_lo = cmp(FloatPredicate::OGT, r, neg_inf, "exp_lo");
    let exp_hi = cmp(FloatPredicate::OLT, r, inf, "exp_hi");
    let fast_ok = [base_fin, exp_lo, exp_hi]
        .into_iter()
        .fold(base_pos, |acc, c| builder.build_and(acc, c, "pow_fast_ok").expect("and"));
    emit_guarded(context, builder, rt, rt.float_pow, l, r, fast_ok, |b| {
        let value = emit_libm_pow(context, b, rt, l, r);
        // A positive base gives a non-negative result; only `+inf`
        // (overflow) needs the runtime's `OverflowError`.
        let overflow = b
            .build_float_compare(FloatPredicate::OEQ, value, inf, "pow_overflow")
            .expect("fcmp");
        (value, Some(overflow))
    })
}
