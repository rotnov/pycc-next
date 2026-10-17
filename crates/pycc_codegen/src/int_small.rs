//! `int` operations over operands `int_range` proves are smallints, with a
//! result it proves is one too: computed on the tagged words directly, with
//! no tag test, no overflow check and no runtime fallback (compare
//! `int_fast`, whose identities these share: `a = 2x + 1`, `b = 2y + 1`).

use crate::int_range::{IntRanges, Range};
use inkwell::IntPredicate;
use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::values::IntValue;
use pycc_mir::{BinOpKind, MirExpr, Ty};

/// The operand ranges of `left op right` when both operands and the result
/// are proven smallints and `op` is one [`emit_binop`] computes.
pub(crate) fn binop_operands(
    ranges: &IntRanges,
    op: BinOpKind,
    left: &MirExpr,
    right: &MirExpr,
    ty: &Ty,
) -> Option<(Range, Range)> {
    if *ty != Ty::Int || left.ty() != Ty::Int || right.ty() != Ty::Int || op == BinOpKind::Pow {
        return None;
    }
    let operands = (ranges.of(left)?, ranges.of(right)?);
    crate::int_range::binop_range(op, operands.0, operands.1)?;
    Some(operands)
}

/// `l op r` for two smallint words whose result is a smallint
/// ([`binop_operands`] holds; `right` is `r`'s range).
pub(crate) fn emit_binop<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    op: BinOpKind,
    l: IntValue<'ctx>,
    r: IntValue<'ctx>,
    right: Range,
) -> IntValue<'ctx> {
    let i64_type = context.i64_type();
    let one = i64_type.const_int(1, false);
    let untag = |w: IntValue<'ctx>, name: &str| {
        builder
            .build_right_shift(w, one, true, name)
            .expect("ashr by a constant")
    };
    let retag = |v: IntValue<'ctx>| {
        let shifted = builder
            .build_left_shift(v, one, "small_shl")
            .expect("shl by a constant");
        builder
            .build_or(shifted, one, "small_tag")
            .expect("or with a constant")
    };
    match op {
        BinOpKind::Add => {
            let r_even = builder.build_int_sub(r, one, "r_even").expect("sub");
            builder
                .build_int_nsw_add(l, r_even, "small_add")
                .expect("add")
        }
        BinOpKind::Sub => {
            let r_even = builder.build_int_sub(r, one, "r_even").expect("sub");
            builder
                .build_int_nsw_sub(l, r_even, "small_sub")
                .expect("sub")
        }
        BinOpKind::Mul => {
            let l_even = builder.build_int_sub(l, one, "l_even").expect("sub");
            let product = builder
                .build_int_nsw_mul(l_even, untag(r, "y"), "small_mul")
                .expect("mul");
            builder.build_or(product, one, "small_mul_tag").expect("or")
        }
        BinOpKind::BitAnd => builder.build_and(l, r, "small_and").expect("and"),
        BinOpKind::BitOr => builder.build_or(l, r, "small_or").expect("or"),
        BinOpKind::BitXor => {
            let x = builder.build_xor(l, r, "small_xor").expect("xor");
            builder.build_or(x, one, "small_xor_tag").expect("or")
        }
        BinOpKind::RShift => {
            // Any count of 63 or more leaves only the sign.
            let n = untag(r, "n");
            let n = if right.hi > 63 {
                let max = i64_type.const_int(63, false);
                let far = builder
                    .build_int_compare(IntPredicate::SGT, n, max, "far")
                    .expect("icmp");
                builder
                    .build_select(far, max, n, "count")
                    .expect("select")
                    .into_int_value()
            } else {
                n
            };
            let shifted = builder
                .build_right_shift(untag(l, "x"), n, true, "small_shr")
                .expect("ashr");
            retag(shifted)
        }
        BinOpKind::LShift => {
            let l_even = builder.build_int_sub(l, one, "l_even").expect("sub");
            let shifted = builder
                .build_left_shift(l_even, untag(r, "n"), "small_shl")
                .expect("shl");
            builder.build_or(shifted, one, "small_shl_tag").expect("or")
        }
        BinOpKind::FloorDiv | BinOpKind::Mod => {
            // A positive divisor: floor when the remainder is negative.
            let x = untag(l, "x");
            let y = untag(r, "y");
            let q = builder.build_int_signed_div(x, y, "q").expect("sdiv");
            let rem = builder.build_int_signed_rem(x, y, "rem").expect("srem");
            let negative = builder
                .build_int_compare(IntPredicate::SLT, rem, i64_type.const_zero(), "rem_neg")
                .expect("icmp");
            let value = if op == BinOpKind::FloorDiv {
                let adj = builder
                    .build_int_z_extend(negative, i64_type, "adj")
                    .expect("zext");
                builder.build_int_sub(q, adj, "floor_q").expect("sub")
            } else {
                let fixed = builder.build_int_add(rem, y, "rem_fixed").expect("add");
                builder
                    .build_select(negative, fixed, rem, "floor_r")
                    .expect("select")
                    .into_int_value()
            };
            retag(value)
        }
        BinOpKind::Pow | BinOpKind::Div => {
            unreachable!("binop_operands admits neither `**` nor `/`")
        }
    }
}

/// Whether `left` and `right` are both proven smallints, so that an
/// ordering comparison is one compare of their words.
pub(crate) fn compares_directly(ranges: &IntRanges, left: &MirExpr, right: &MirExpr) -> bool {
    left.ty() == Ty::Int
        && right.ty() == Ty::Int
        && ranges.of(left).is_some()
        && ranges.of(right).is_some()
}

/// `abs` of a smallint word: `2 - word` when negative.
pub(crate) fn emit_abs<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    word: IntValue<'ctx>,
) -> IntValue<'ctx> {
    let i64_type = context.i64_type();
    let negative = builder
        .build_int_compare(
            IntPredicate::SLT,
            word,
            i64_type.const_zero(),
            "small_abs_neg",
        )
        .expect("icmp");
    let flipped = builder
        .build_int_sub(i64_type.const_int(2, false), word, "small_abs_flip")
        .expect("sub");
    builder
        .build_select(negative, flipped, word, "small_abs")
        .expect("select")
        .into_int_value()
}
