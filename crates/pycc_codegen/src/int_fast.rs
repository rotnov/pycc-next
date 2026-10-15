//! Inline fast paths for `int` arithmetic, comparison and conversion.
//!
//! An `int` is one D-061 word, and codegen knows exactly one fact about its
//! bits: an *odd* word is a smallint whose value is `word >> 1`. Every
//! operator below tests that tag on its operands, computes the result on
//! the tagged words directly when both are smallints, and calls the
//! `pycc_rt_int_*` function only for a bigint or bool-marker operand, a
//! result that leaves the 63-bit range, or an operation that raises (a zero
//! divisor, a negative shift count). The runtime functions accept every
//! encoded kind, so the slow path is always correct; the fast path is an
//! optimization that must agree with it bit for bit. Fast branches carry
//! `!prof` branch weights so LLVM lays the smallint path out as the
//! fall-through.
//!
//! Tagged-word identities used (`a = 2x+1`, `b = 2y+1`):
//! - `a + (b - 1) = 2(x+y) + 1`, `a - (b - 1) = 2(x-y) + 1`; the `i64`
//!   add/sub overflows exactly when `x±y` leaves the 63-bit range.
//! - `(a - 1) * y = 2xy`; the `i64` multiply overflows exactly when `xy`
//!   does, and `| 1` re-tags.
//! - `a & b`, `a | b` are already tagged; `(a ^ b) | 1` re-tags `x ^ y`.
//! - signed comparison of the words orders the values.

use super::RtFns;
use inkwell::basic_block::BasicBlock;
use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::values::{BasicValue as _, FloatValue, FunctionValue, InstructionValue, IntValue};
use inkwell::{FloatPredicate, IntPredicate};
use pycc_mir::BinOpKind;

/// Weight given to the likely edge of a fast-path branch (the other edge
/// gets 1).
const LIKELY_WEIGHT: u64 = 2000;

/// Marks the conditional branch `branch` as taking its `then` edge
/// (`likely_then`) or its `else` edge almost always.
pub(crate) fn weight_branch<'ctx>(context: &'ctx Context, branch: InstructionValue<'ctx>, likely_then: bool) {
    let i32_type = context.i32_type();
    let (then_w, else_w) = if likely_then {
        (LIKELY_WEIGHT, 1)
    } else {
        (1, LIKELY_WEIGHT)
    };
    let node = context.metadata_node(&[
        context.metadata_string("branch_weights").into(),
        i32_type.const_int(then_w, false).into(),
        i32_type.const_int(else_w, false).into(),
    ]);
    branch
        .set_metadata(node, context.get_kind_id("prof"))
        .expect("a conditional branch accepts `!prof` metadata");
}

/// `br cond, then, else` with `then` marked likely.
pub(super) fn likely_branch<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    cond: IntValue<'ctx>,
    then_bb: BasicBlock<'ctx>,
    else_bb: BasicBlock<'ctx>,
) {
    let branch = builder
        .build_conditional_branch(cond, then_bb, else_bb)
        .expect("build_conditional_branch should not fail for an i1 condition");
    weight_branch(context, branch, true);
}

fn current_function<'ctx>(builder: &Builder<'ctx>) -> FunctionValue<'ctx> {
    builder
        .get_insert_block()
        .and_then(|block| block.get_parent())
        .expect("int fast paths are emitted inside a function body")
}

/// `i1`: whether every word in `words` is a smallint (odd).
pub(crate) fn all_small<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    words: &[IntValue<'ctx>],
) -> IntValue<'ctx> {
    let i64_type = context.i64_type();
    let mut acc = words[0];
    for word in &words[1..] {
        acc = builder
            .build_and(acc, *word, "tag_and")
            .expect("and of two i64");
    }
    let low = builder
        .build_and(acc, i64_type.const_int(1, false), "tag_bit")
        .expect("and with a constant");
    builder
        .build_int_compare(IntPredicate::NE, low, i64_type.const_zero(), "all_small")
        .expect("icmp of two i64")
}

/// `i1`: whether `word` is a smallint.
pub(super) fn is_small<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    word: IntValue<'ctx>,
) -> IntValue<'ctx> {
    all_small(context, builder, &[word])
}

fn call_i64<'ctx>(
    builder: &Builder<'ctx>,
    function: FunctionValue<'ctx>,
    args: &[IntValue<'ctx>],
    name: &str,
) -> IntValue<'ctx> {
    let args: Vec<_> = args.iter().map(|a| (*a).into()).collect();
    builder
        .build_call(function, &args, name)
        .expect("build_call should not fail for a runtime int function")
        .try_as_basic_value()
        .expect_basic("the runtime int function returns a value")
        .into_int_value()
}

/// One `llvm.*.with.overflow.i64` call: `(result, overflowed)`.
pub(crate) fn with_overflow<'ctx>(
    builder: &Builder<'ctx>,
    intrinsic: FunctionValue<'ctx>,
    a: IntValue<'ctx>,
    b: IntValue<'ctx>,
) -> (IntValue<'ctx>, IntValue<'ctx>) {
    let pair = builder
        .build_call(intrinsic, &[a.into(), b.into()], "ov")
        .expect("build_call should not fail for an overflow intrinsic")
        .try_as_basic_value()
        .expect_basic("an overflow intrinsic returns a struct")
        .into_struct_value();
    let value = builder
        .build_extract_value(pair, 0, "ov_value")
        .expect("field 0");
    let overflow = builder
        .build_extract_value(pair, 1, "ov_flag")
        .expect("field 1");
    (value.into_int_value(), overflow.into_int_value())
}

/// The shape every operator shares: test the operands' tags, run `fast` in
/// the smallint block, fall back to `slow_fn(l, r)` otherwise. `fast`
/// returns the tagged result and, optionally, an `i1` that sends the
/// smallint path to the slow call too (overflow, zero divisor, ...).
///
/// `owned` lists the operand words whose birth reference the caller hands
/// over for release. They are released only after the slow call: on the
/// fast path both operands are smallints, which own nothing.
fn emit_guarded<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    rt: &RtFns<'ctx>,
    slow_fn: FunctionValue<'ctx>,
    l: IntValue<'ctx>,
    r: IntValue<'ctx>,
    owned: &[IntValue<'ctx>],
    fast: impl FnOnce(&Builder<'ctx>) -> (IntValue<'ctx>, Option<IntValue<'ctx>>),
) -> IntValue<'ctx> {
    let function = current_function(builder);
    let fast_bb = context.append_basic_block(function, "int_fast");
    let slow_bb = context.append_basic_block(function, "int_slow");
    let join_bb = context.append_basic_block(function, "int_join");
    let small = all_small(context, builder, &[l, r]);
    likely_branch(context, builder, small, fast_bb, slow_bb);

    builder.position_at_end(fast_bb);
    let (fast_value, bail) = fast(builder);
    let fast_end = builder.get_insert_block().expect("positioned");
    match bail {
        Some(bail) => {
            let branch = builder
                .build_conditional_branch(bail, slow_bb, join_bb)
                .expect("build_conditional_branch should not fail for an i1 condition");
            weight_branch(context, branch, false);
        }
        None => {
            builder
                .build_unconditional_branch(join_bb)
                .expect("branch to the join");
        }
    }

    builder.position_at_end(slow_bb);
    let slow_value = call_i64(builder, slow_fn, &[l, r], "int_slow_call");
    release_owned(context, builder, rt, owned);
    if slow_fn_raises(rt, slow_fn) {
        // Only the runtime path can raise (a zero divisor, a negative shift
        // count), so the pending-exception guard lives here and `emit_expr`
        // adds none after the node (`expression_can_set_exception`).
        crate::exception::guard_statement_effects(context, builder, rt);
    }
    let slow_end = builder.get_insert_block().expect("positioned");
    builder
        .build_unconditional_branch(join_bb)
        .expect("branch to the join");

    builder.position_at_end(join_bb);
    let phi = builder
        .build_phi(context.i64_type(), "int_result")
        .expect("build_phi in a fresh block");
    phi.add_incoming(&[(&fast_value, fast_end), (&slow_value, slow_end)]);
    phi.as_basic_value().into_int_value()
}

/// Whether `slow_fn` is one of the runtime `int` operations that can raise:
/// `//` and `%` (zero divisor) and `<<`/`>>` (negative count, or a result
/// too large). These guard on their own cold path.
fn slow_fn_raises<'ctx>(rt: &RtFns<'ctx>, slow_fn: FunctionValue<'ctx>) -> bool {
    [rt.int_floordiv, rt.int_floormod, rt.int_lshift, rt.int_rshift].contains(&slow_fn)
}

fn release_owned<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    rt: &RtFns<'ctx>,
    owned: &[IntValue<'ctx>],
) {
    for word in owned {
        super::emit_bigint_refcount_call(
            context,
            builder,
            rt,
            *word,
            super::BigIntRefcount::Release,
        );
    }
}

/// Emits `l + e` or `l - e` (`op`) for an encoded `int` `l` and a machine
/// `i64` `e`, and returns the encoded result, releasing the `owned`
/// operand words as [`emit_guarded`] does.
///
/// The fast path never encodes `e`: for a smallint `l` (`2x + 1`) the
/// result is `l + 2e` (`l - 2e`), and it is taken when `l` is a smallint
/// and neither `2e` nor the sum overflows -- exactly when `x + e` is in the
/// inline range. Otherwise the cold path encodes `e`
/// (`pycc_rt_int_from_i64`, a heap bigint past the inline range), calls the
/// runtime operation and releases it.
pub(crate) fn emit_int_addsub_i64<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    rt: &RtFns<'ctx>,
    op: BinOpKind,
    l: IntValue<'ctx>,
    e: IntValue<'ctx>,
    owned: &[IntValue<'ctx>],
) -> IntValue<'ctx> {
    let (intrinsic, slow_fn) = match op {
        BinOpKind::Add => (rt.sadd_overflow, rt.int_add),
        BinOpKind::Sub => (rt.ssub_overflow, rt.int_sub),
        _ => panic!("pycc_codegen: internal error: emit_int_addsub_i64 on {op:?}"),
    };
    let function = current_function(builder);
    let fast_bb = context.append_basic_block(function, "int_fast");
    let slow_bb = context.append_basic_block(function, "int_slow");
    let join_bb = context.append_basic_block(function, "int_join");
    let small = is_small(context, builder, l);
    let (twice, twice_overflow) = with_overflow(builder, rt.sadd_overflow, e, e);
    let (sum, sum_overflow) = with_overflow(builder, intrinsic, l, twice);
    let overflow = builder
        .build_or(twice_overflow, sum_overflow, "raw_overflow")
        .expect("or of two i1");
    let no_overflow = builder.build_not(overflow, "raw_fits").expect("not");
    let fast = builder
        .build_and(small, no_overflow, "raw_fast")
        .expect("and of two i1");
    likely_branch(context, builder, fast, fast_bb, slow_bb);

    builder.position_at_end(fast_bb);
    builder
        .build_unconditional_branch(join_bb)
        .expect("branch to the join");

    builder.position_at_end(slow_bb);
    let r = call_i64(builder, rt.int_from_i64, &[e], "int_from_raw");
    let slow_value = call_i64(builder, slow_fn, &[l, r], "int_slow_call");
    release_owned(context, builder, rt, owned);
    release_owned(context, builder, rt, &[r]);
    let slow_end = builder.get_insert_block().expect("positioned");
    builder
        .build_unconditional_branch(join_bb)
        .expect("branch to the join");

    builder.position_at_end(join_bb);
    let phi = builder
        .build_phi(context.i64_type(), "int_result")
        .expect("build_phi in a fresh block");
    phi.add_incoming(&[(&sum, fast_bb), (&slow_value, slow_end)]);
    phi.as_basic_value().into_int_value()
}

/// Emits `l op r` for two encoded `int` operands and returns the encoded
/// result, releasing the `owned` operand words (see [`emit_guarded`]).
pub(super) fn emit_int_binop<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    rt: &RtFns<'ctx>,
    op: BinOpKind,
    l: IntValue<'ctx>,
    r: IntValue<'ctx>,
    owned: &[IntValue<'ctx>],
) -> IntValue<'ctx> {
    let i64_type = context.i64_type();
    let one = i64_type.const_int(1, false);
    let zero = i64_type.const_zero();
    let untag = |b: &Builder<'ctx>, w: IntValue<'ctx>, name: &str| {
        b.build_right_shift(w, one, true, name)
            .expect("ashr by a constant")
    };
    let retag = |b: &Builder<'ctx>, v: IntValue<'ctx>| {
        let shifted = b
            .build_left_shift(v, one, "tag_shl")
            .expect("shl by a constant");
        b.build_or(shifted, one, "tag_or")
            .expect("or with a constant")
    };
    match op {
        BinOpKind::Add | BinOpKind::Sub => {
            let (intrinsic, slow) = if op == BinOpKind::Add {
                (rt.sadd_overflow, rt.int_add)
            } else {
                (rt.ssub_overflow, rt.int_sub)
            };
            emit_guarded(context, builder, rt, slow, l, r, owned, |b| {
                let r_even = b
                    .build_int_sub(r, one, "r_even")
                    .expect("sub of a constant");
                let (value, overflow) = with_overflow(b, intrinsic, l, r_even);
                (value, Some(overflow))
            })
        }
        BinOpKind::Mul => emit_guarded(context, builder, rt, rt.int_mul, l, r, owned, |b| {
            let l_even = b
                .build_int_sub(l, one, "l_even")
                .expect("sub of a constant");
            let y = untag(b, r, "y");
            let (product, overflow) = with_overflow(b, rt.smul_overflow, l_even, y);
            let tagged = b
                .build_or(product, one, "mul_tag")
                .expect("or with a constant");
            (tagged, Some(overflow))
        }),
        BinOpKind::FloorDiv | BinOpKind::Mod => {
            let slow = if op == BinOpKind::FloorDiv {
                rt.int_floordiv
            } else {
                rt.int_floormod
            };
            emit_guarded(context, builder, rt, slow, l, r, owned, |b| {
                let x = untag(b, l, "x");
                let y = untag(b, r, "y");
                // A zero divisor raises in the runtime. `y == -1` goes there
                // too: `-2**62 // -1` is the one quotient that leaves the
                // tagged range, and the slow path promotes it.
                let y_plus_one = b.build_int_add(y, one, "y_plus_one").expect("add");
                let bail = b
                    .build_int_compare(IntPredicate::ULE, y_plus_one, one, "div_bail")
                    .expect("icmp");
                // `sdiv`/`srem` on a zero divisor would be UB even on the
                // untaken path once LLVM hoists them, so divide by a safe
                // operand when bailing.
                let safe_y = b
                    .build_select(bail, one, y, "safe_y")
                    .expect("select")
                    .into_int_value();
                let q = b.build_int_signed_div(x, safe_y, "q").expect("sdiv");
                let rem = b.build_int_signed_rem(x, safe_y, "rem").expect("srem");
                // Python floors: adjust when the remainder is non-zero and
                // its sign differs from the divisor's.
                let rem_nonzero = b
                    .build_int_compare(IntPredicate::NE, rem, zero, "rem_nz")
                    .expect("icmp");
                let sign_mix = b.build_xor(rem, safe_y, "sign_mix").expect("xor");
                let signs_differ = b
                    .build_int_compare(IntPredicate::SLT, sign_mix, zero, "signs_differ")
                    .expect("icmp");
                let adjust = b
                    .build_and(rem_nonzero, signs_differ, "adjust")
                    .expect("and");
                let value = if op == BinOpKind::FloorDiv {
                    let adj = b.build_int_z_extend(adjust, i64_type, "adj").expect("zext");
                    b.build_int_sub(q, adj, "floor_q").expect("sub")
                } else {
                    let fixed = b.build_int_add(rem, safe_y, "rem_fixed").expect("add");
                    b.build_select(adjust, fixed, rem, "floor_r")
                        .expect("select")
                        .into_int_value()
                };
                (retag(b, value), Some(bail))
            })
        }
        BinOpKind::LShift => emit_guarded(context, builder, rt, rt.int_lshift, l, r, owned, |b| {
            let n = untag(b, r, "n");
            // Counts 0..=62 shift inline; a negative or larger count, or a
            // shift that loses bits, takes the runtime path.
            let bad_count = b
                .build_int_compare(
                    IntPredicate::UGT,
                    n,
                    i64_type.const_int(62, false),
                    "bad_count",
                )
                .expect("icmp");
            let safe_n = b
                .build_select(bad_count, zero, n, "safe_n")
                .expect("select")
                .into_int_value();
            let l_even = b.build_int_sub(l, one, "l_even").expect("sub");
            let shifted = b.build_left_shift(l_even, safe_n, "shl").expect("shl");
            let back = b
                .build_right_shift(shifted, safe_n, true, "shl_back")
                .expect("ashr");
            let lossy = b
                .build_int_compare(IntPredicate::NE, back, l_even, "lossy")
                .expect("icmp");
            let bail = b.build_or(bad_count, lossy, "shl_bail").expect("or");
            let tagged = b.build_or(shifted, one, "shl_tag").expect("or");
            (tagged, Some(bail))
        }),
        BinOpKind::RShift => emit_guarded(context, builder, rt, rt.int_rshift, l, r, owned, |b| {
            let n = untag(b, r, "n");
            let negative = b
                .build_int_compare(IntPredicate::SLT, n, zero, "neg_count")
                .expect("icmp");
            // Any count of 63 or more leaves only the sign.
            let max = i64_type.const_int(63, false);
            let too_far = b
                .build_int_compare(IntPredicate::SGT, n, max, "too_far")
                .expect("icmp");
            let count = b
                .build_select(too_far, max, n, "count")
                .expect("select")
                .into_int_value();
            let safe_count = b
                .build_select(negative, zero, count, "safe_count")
                .expect("select")
                .into_int_value();
            let x = untag(b, l, "x");
            let value = b
                .build_right_shift(x, safe_count, true, "shr")
                .expect("ashr");
            (retag(b, value), Some(negative))
        }),
        BinOpKind::BitAnd => emit_guarded(context, builder, rt, rt.int_and, l, r, owned, |b| {
            (b.build_and(l, r, "and").expect("and"), None)
        }),
        BinOpKind::BitOr => emit_guarded(context, builder, rt, rt.int_or, l, r, owned, |b| {
            (b.build_or(l, r, "or").expect("or"), None)
        }),
        BinOpKind::BitXor => emit_guarded(context, builder, rt, rt.int_xor, l, r, owned, |b| {
            let x = b.build_xor(l, r, "xor").expect("xor");
            (b.build_or(x, one, "xor_tag").expect("or"), None)
        }),
        BinOpKind::Pow => {
            let value = call_i64(builder, rt.int_pow, &[l, r], "int_pow");
            release_owned(context, builder, rt, owned);
            value
        }
        BinOpKind::Div => {
            unreachable!("pycc_types/pycc_mir always type true division as Ty::Float")
        }
    }
}

/// Emits an ordering comparison of two encoded `int` operands as an `i1`.
pub(super) fn emit_int_compare<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    rt: &RtFns<'ctx>,
    predicate: IntPredicate,
    l: IntValue<'ctx>,
    r: IntValue<'ctx>,
) -> IntValue<'ctx> {
    let function = current_function(builder);
    let fast_bb = context.append_basic_block(function, "cmp_fast");
    let slow_bb = context.append_basic_block(function, "cmp_slow");
    let join_bb = context.append_basic_block(function, "cmp_join");
    let small = all_small(context, builder, &[l, r]);
    likely_branch(context, builder, small, fast_bb, slow_bb);

    builder.position_at_end(fast_bb);
    let fast_value = builder
        .build_int_compare(predicate, l, r, "cmp_small")
        .expect("icmp");
    builder.build_unconditional_branch(join_bb).expect("branch");

    builder.position_at_end(slow_bb);
    let ordering = builder
        .build_call(rt.int_cmp, &[l.into(), r.into()], "int_cmp")
        .expect("build_call should not fail for a comparison")
        .try_as_basic_value()
        .expect_basic("pycc_rt_int_cmp returns i32")
        .into_int_value();
    let slow_value = builder
        .build_int_compare(
            predicate,
            ordering,
            context.i32_type().const_zero(),
            "cmp_big",
        )
        .expect("icmp");
    builder.build_unconditional_branch(join_bb).expect("branch");

    builder.position_at_end(join_bb);
    let phi = builder
        .build_phi(context.bool_type(), "cmp_result")
        .expect("phi");
    phi.add_incoming(&[(&fast_value, fast_bb), (&slow_value, slow_bb)]);
    phi.as_basic_value().into_int_value()
}

/// Emits `e op w` (`w op e` when `!element_left`) for a machine `i64` `e`
/// and an encoded `int` `w`, as an `i1`: inline against the untagged `w`
/// when it is a smallint (no overflow: both are machine integers), through
/// `pycc_rt_int_cmp` on an `int` made of `e` otherwise.
pub(crate) fn emit_int_compare_i64<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    rt: &RtFns<'ctx>,
    predicate: IntPredicate,
    e: IntValue<'ctx>,
    w: IntValue<'ctx>,
    element_left: bool,
) -> IntValue<'ctx> {
    let function = current_function(builder);
    let fast_bb = context.append_basic_block(function, "cmp_fast");
    let slow_bb = context.append_basic_block(function, "cmp_slow");
    let join_bb = context.append_basic_block(function, "cmp_join");
    let small = is_small(context, builder, w);
    likely_branch(context, builder, small, fast_bb, slow_bb);

    builder.position_at_end(fast_bb);
    let raw = builder
        .build_right_shift(w, context.i64_type().const_int(1, false), true, "cmp_untag")
        .expect("ashr by a constant");
    let (a, b) = if element_left { (e, raw) } else { (raw, e) };
    let fast_value = builder
        .build_int_compare(predicate, a, b, "cmp_raw")
        .expect("icmp");
    builder.build_unconditional_branch(join_bb).expect("branch");

    builder.position_at_end(slow_bb);
    let boxed = call_i64(builder, rt.int_from_i64, &[e], "int_from_raw");
    let (a, b) = if element_left { (boxed, w) } else { (w, boxed) };
    let ordering = builder
        .build_call(rt.int_cmp, &[a.into(), b.into()], "int_cmp")
        .expect("build_call should not fail for a comparison")
        .try_as_basic_value()
        .expect_basic("pycc_rt_int_cmp returns i32")
        .into_int_value();
    let slow_value = builder
        .build_int_compare(predicate, ordering, context.i32_type().const_zero(), "cmp_big")
        .expect("icmp");
    release_owned(context, builder, rt, &[boxed]);
    let slow_end = builder.get_insert_block().expect("positioned");
    builder.build_unconditional_branch(join_bb).expect("branch");

    builder.position_at_end(join_bb);
    let phi = builder
        .build_phi(context.bool_type(), "cmp_result")
        .expect("phi");
    phi.add_incoming(&[(&fast_value, fast_bb), (&slow_value, slow_end)]);
    phi.as_basic_value().into_int_value()
}

/// `float(word)` for an encoded `int`: `sitofp` of the untagged value for a
/// smallint (correctly rounded, like CPython), the runtime otherwise.
pub(super) fn emit_int_to_float<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    rt: &RtFns<'ctx>,
    word: IntValue<'ctx>,
) -> FloatValue<'ctx> {
    int_to_float(context, builder, rt, word, false)
}

/// [`emit_int_to_float`] followed, on its cold runtime path only, by the
/// pending-exception guard: the conversion raises `OverflowError` past
/// `1e308`. For an operation that guards itself (`float_fast`) instead of
/// relying on `emit_expr`'s guard after the whole node.
pub(super) fn emit_int_to_float_guarded<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    rt: &RtFns<'ctx>,
    word: IntValue<'ctx>,
) -> FloatValue<'ctx> {
    int_to_float(context, builder, rt, word, true)
}

fn int_to_float<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    rt: &RtFns<'ctx>,
    word: IntValue<'ctx>,
    guard: bool,
) -> FloatValue<'ctx> {
    let function = current_function(builder);
    let fast_bb = context.append_basic_block(function, "itof_fast");
    let slow_bb = context.append_basic_block(function, "itof_slow");
    let join_bb = context.append_basic_block(function, "itof_join");
    let small = is_small(context, builder, word);
    likely_branch(context, builder, small, fast_bb, slow_bb);

    builder.position_at_end(fast_bb);
    let x = builder
        .build_right_shift(word, context.i64_type().const_int(1, false), true, "x")
        .expect("ashr");
    let fast_value = builder
        .build_signed_int_to_float(x, context.f64_type(), "itof")
        .expect("sitofp");
    builder.build_unconditional_branch(join_bb).expect("branch");

    builder.position_at_end(slow_bb);
    let slow_value = builder
        .build_call(rt.int_to_float, &[word.into()], "int_to_float")
        .expect("build_call should not fail for a conversion")
        .try_as_basic_value()
        .expect_basic("pycc_rt_int_to_float returns f64")
        .into_float_value();
    if guard {
        crate::exception::guard_statement_effects(context, builder, rt);
    }
    let slow_end = builder.get_insert_block().expect("positioned");
    builder.build_unconditional_branch(join_bb).expect("branch");

    builder.position_at_end(join_bb);
    let phi = builder
        .build_phi(context.f64_type(), "itof_result")
        .expect("phi");
    phi.add_incoming(&[(&fast_value, fast_bb), (&slow_value, slow_end)]);
    phi.as_basic_value().into_float_value()
}

/// One `range()` operand normalized for the induction phi: a smallint passes
/// through inline; anything else (a bool marker, a heap bigint) goes through
/// `pycc_rt_range_normalize_operand`.
pub(super) fn emit_range_normalize<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    rt: &RtFns<'ctx>,
    word: IntValue<'ctx>,
) -> IntValue<'ctx> {
    let function = current_function(builder);
    let fast_bb = builder.get_insert_block().expect("positioned");
    let slow_bb = context.append_basic_block(function, "range_norm_slow");
    let join_bb = context.append_basic_block(function, "range_norm_join");
    let small = is_small(context, builder, word);
    likely_branch(context, builder, small, join_bb, slow_bb);
    builder.position_at_end(slow_bb);
    let slow_value = call_i64(builder, rt.range_normalize_operand, &[word], "range_norm");
    builder.build_unconditional_branch(join_bb).expect("branch");
    builder.position_at_end(join_bb);
    let phi = builder
        .build_phi(context.i64_type(), "range_operand")
        .expect("phi");
    phi.add_incoming(&[(&word, fast_bb), (&slow_value, slow_bb)]);
    phi.as_basic_value().into_int_value()
}

/// `range`'s continuation test for three encoded operands: inline signed
/// comparisons when all three are smallints and the step is non-zero, the
/// runtime (which raises for a zero step) otherwise. Returns an `i1`.
pub(super) fn emit_range_continue<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    rt: &RtFns<'ctx>,
    current: IntValue<'ctx>,
    stop: IntValue<'ctx>,
    step: IntValue<'ctx>,
) -> IntValue<'ctx> {
    let function = current_function(builder);
    let fast_bb = context.append_basic_block(function, "range_fast");
    let slow_bb = context.append_basic_block(function, "range_slow");
    let join_bb = context.append_basic_block(function, "range_join");
    let small = all_small(context, builder, &[current, stop, step]);
    // The tagged zero is `1`.
    let one = context.i64_type().const_int(1, false);
    let nonzero = builder
        .build_int_compare(IntPredicate::NE, step, one, "step_nz")
        .expect("icmp");
    let fast_ok = builder
        .build_and(small, nonzero, "range_fast_ok")
        .expect("and");
    likely_branch(context, builder, fast_ok, fast_bb, slow_bb);

    builder.position_at_end(fast_bb);
    let up = builder
        .build_int_compare(IntPredicate::SGT, step, one, "step_up")
        .expect("icmp");
    let below = builder
        .build_int_compare(IntPredicate::SLT, current, stop, "below")
        .expect("icmp");
    let above = builder
        .build_int_compare(IntPredicate::SGT, current, stop, "above")
        .expect("icmp");
    let fast_value = builder
        .build_select(up, below, above, "range_cont")
        .expect("select")
        .into_int_value();
    builder.build_unconditional_branch(join_bb).expect("branch");

    builder.position_at_end(slow_bb);
    let cont = builder
        .build_call(
            rt.range_continue,
            &[current.into(), stop.into(), step.into()],
            "range_continue",
        )
        .expect("build_call should not fail for range_continue")
        .try_as_basic_value()
        .expect_basic("pycc_rt_range_continue returns i8")
        .into_int_value();
    let slow_value = builder
        .build_int_compare(
            IntPredicate::NE,
            cont,
            context.i8_type().const_zero(),
            "range_cont_rt",
        )
        .expect("icmp");
    builder.build_unconditional_branch(join_bb).expect("branch");

    builder.position_at_end(join_bb);
    let phi = builder
        .build_phi(context.bool_type(), "range_continue_result")
        .expect("phi");
    phi.add_incoming(&[(&fast_value, fast_bb), (&slow_value, slow_bb)]);
    phi.as_basic_value().into_int_value()
}

/// Reads the pending-exception flag as an `i8`. A native program (one
/// thread) loads `pycc_rt_exc_pending`, the runtime's process-wide mirror of
/// its thread-local state, inline. An `ext` module may run on several
/// CPython threads at once, so it calls `pycc_rt_exception_active`, which
/// reads the calling thread's own state.
pub(super) fn emit_exception_pending<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    rt: &RtFns<'ctx>,
    name: &str,
) -> IntValue<'ctx> {
    if rt.thread_safe_exception_check.get() {
        return builder
            .build_call(rt.exception_active, &[], name)
            .expect("build_call should not fail for exception_active")
            .try_as_basic_value()
            .expect_basic("pycc_rt_exception_active returns i8")
            .into_int_value();
    }
    let flag = builder
        .build_load(context.i8_type(), rt.exc_pending.as_pointer_value(), name)
        .expect("build_load should not fail for the pending-exception flag");
    crate::tbaa::tag(
        context,
        flag.as_instruction_value().expect("a load is an instruction"),
        crate::tbaa::EXC_PENDING,
    );
    flag.into_int_value()
}

/// [`emit_exception_pending`] for a site that branches on the result
/// right away (a statement's effect guard, the end of a `try` body), as an
/// `i1`. In an `ext` module the common case -- no thread has an exception
/// pending -- is one inline load of `pycc_rt_exc_pending_threads`; only a
/// non-zero count calls `pycc_rt_exception_active` for the calling thread's
/// own state. The builder is left in a new block, so a caller must not
/// name the block it was in before as a predecessor afterwards.
pub(super) fn emit_exception_pending_test<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    rt: &RtFns<'ctx>,
    name: &str,
) -> IntValue<'ctx> {
    if !rt.thread_safe_exception_check.get() {
        let active = emit_exception_pending(context, builder, rt, name);
        return builder
            .build_int_compare(IntPredicate::NE, active, context.i8_type().const_zero(), name)
            .expect("icmp");
    }
    let i64_type = context.i64_type();
    let threads = builder
        .build_load(
            i64_type,
            rt.exc_pending_threads.as_pointer_value(),
            &format!("{name}_threads"),
        )
        .expect("build_load should not fail for the pending-thread count");
    let load = threads.as_instruction_value().expect("a load is an instruction");
    load.set_alignment(8).expect("an i64 load takes alignment 8");
    load.set_atomic_ordering(inkwell::AtomicOrdering::Monotonic)
        .expect("an i64 load can be atomic");
    let any = builder
        .build_int_compare(
            IntPredicate::NE,
            threads.into_int_value(),
            i64_type.const_zero(),
            &format!("{name}_any"),
        )
        .expect("icmp");
    let entry_bb = builder.get_insert_block().expect("an insert block");
    let function = entry_bb.get_parent().expect("a function");
    let slow_bb = context.append_basic_block(function, &format!("{name}_slow"));
    let join_bb = context.append_basic_block(function, &format!("{name}_join"));
    let branch = builder
        .build_conditional_branch(any, slow_bb, join_bb)
        .expect("branch");
    crate::int_fast::weight_branch(context, branch, false);
    builder.position_at_end(slow_bb);
    let active = builder
        .build_call(rt.exception_active, &[], name)
        .expect("build_call should not fail for exception_active")
        .try_as_basic_value()
        .expect_basic("pycc_rt_exception_active returns i8")
        .into_int_value();
    let slow_has = builder
        .build_int_compare(IntPredicate::NE, active, context.i8_type().const_zero(), name)
        .expect("icmp");
    builder.build_unconditional_branch(join_bb).expect("branch");
    builder.position_at_end(join_bb);
    let phi = builder.build_phi(context.bool_type(), name).expect("phi");
    phi.add_incoming(&[
        (&context.bool_type().const_zero(), entry_bb),
        (&slow_has, slow_bb),
    ]);
    phi.as_basic_value().into_int_value()
}

/// `int op float`, exactly (CPython does not round the int). A smallint
/// within +/-2**53 converts exactly, so `float_predicate` decides it inline;
/// anything else asks `pycc_rt_int_float_cmp` for an ordering (`-1`/`0`/`1`,
/// `2` for NaN) and tests `ordering ordering_predicate ordering_rhs`.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_int_float_compare<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    rt: &RtFns<'ctx>,
    word: IntValue<'ctx>,
    float: FloatValue<'ctx>,
    float_predicate: FloatPredicate,
    ordering_predicate: IntPredicate,
    ordering_rhs: i64,
) -> IntValue<'ctx> {
    let i64_type = context.i64_type();
    let function = current_function(builder);
    let fast_bb = context.append_basic_block(function, "ifcmp_fast");
    let slow_bb = context.append_basic_block(function, "ifcmp_slow");
    let join_bb = context.append_basic_block(function, "ifcmp_join");
    // Exact when the word is odd and its value is within +/-2**53, i.e.
    // `word + 2**54` is in `0..=2**55` (unsigned).
    let small = is_small(context, builder, word);
    let biased = builder
        .build_int_add(word, i64_type.const_int(1 << 54, false), "biased")
        .expect("add");
    let in_range = builder
        .build_int_compare(
            IntPredicate::ULE,
            biased,
            i64_type.const_int(1 << 55, false),
            "exact",
        )
        .expect("icmp");
    let fast_ok = builder
        .build_and(small, in_range, "ifcmp_fast_ok")
        .expect("and");
    likely_branch(context, builder, fast_ok, fast_bb, slow_bb);

    builder.position_at_end(fast_bb);
    let x = builder
        .build_right_shift(word, i64_type.const_int(1, false), true, "x")
        .expect("ashr");
    let xf = builder
        .build_signed_int_to_float(x, context.f64_type(), "xf")
        .expect("sitofp");
    let fast_value = builder
        .build_float_compare(float_predicate, xf, float, "ifcmp_small")
        .expect("fcmp");
    builder.build_unconditional_branch(join_bb).expect("branch");

    builder.position_at_end(slow_bb);
    let ordering = builder
        .build_call(
            rt.int_float_cmp,
            &[word.into(), float.into()],
            "int_float_cmp",
        )
        .expect("build_call should not fail for an int/float comparison")
        .try_as_basic_value()
        .expect_basic("pycc_rt_int_float_cmp returns i32")
        .into_int_value();
    let slow_value = builder
        .build_int_compare(
            ordering_predicate,
            ordering,
            context.i32_type().const_int(ordering_rhs as u64, true),
            "ifcmp_big",
        )
        .expect("icmp");
    builder.build_unconditional_branch(join_bb).expect("branch");

    builder.position_at_end(join_bb);
    let phi = builder
        .build_phi(context.bool_type(), "ifcmp_result")
        .expect("phi");
    phi.add_incoming(&[(&fast_value, fast_bb), (&slow_value, slow_bb)]);
    phi.as_basic_value().into_int_value()
}
