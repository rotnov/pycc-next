//! Arithmetic and comparison over `int | float` words.
//!
//! Two `int`s compute as `int`s (arbitrary precision, `crate::int_arith`);
//! anything else converts the `int` side with `float(n)` (CPython's
//! `OverflowError: int too large to convert to float`) and computes as
//! `float`s (`crate::float_arith`), which is exactly what CPython's
//! `float.__add__` and friends do with an `int` operand. An `int ** int`
//! with a negative exponent is CPython's `float` power of the two.

use super::*;
use crate::float_arith::{FLOORDIV_BY_ZERO, MOD_BY_ZERO, float_div, float_divmod, float_pow};

/// The operator codes `pycc_codegen` passes to [`pycc_rt_num_binop`].
pub const NUM_ADD: i32 = 0;
pub const NUM_SUB: i32 = 1;
pub const NUM_MUL: i32 = 2;
pub const NUM_FLOORDIV: i32 = 3;
pub const NUM_MOD: i32 = 4;
pub const NUM_POW: i32 = 5;

/// The comparison codes `pycc_codegen` passes to [`pycc_rt_num_compare`].
pub const NUM_EQ: i32 = 0;
pub const NUM_NE: i32 = 1;
pub const NUM_LT: i32 = 2;
pub const NUM_LE: i32 = 3;
pub const NUM_GT: i32 = 4;
pub const NUM_GE: i32 = 5;

fn float_binop(op: i32, x: f64, y: f64) -> f64 {
    match op {
        NUM_ADD => x + y,
        NUM_SUB => x - y,
        NUM_MUL => x * y,
        NUM_FLOORDIV => float_divmod(x, y, FLOORDIV_BY_ZERO).0,
        NUM_MOD => float_divmod(x, y, MOD_BY_ZERO).1,
        NUM_POW => float_pow(x, y),
        _ => unreachable!("pycc_rt: unknown `int | float` operator {op}"),
    }
}

fn int_binop(op: i32, a: i64, b: i64) -> i64 {
    match op {
        NUM_ADD => int_add(a, b),
        NUM_SUB => int_sub(a, b),
        NUM_MUL => int_mul(a, b),
        NUM_FLOORDIV => int_floordiv(a, b),
        NUM_MOD => int_floormod(a, b),
        NUM_POW => int_pow(a, b),
        _ => unreachable!("pycc_rt: unknown `int | float` operator {op}"),
    }
}

/// `a op b` for two `int | float` words (`op` is a `NUM_*` code).
pub(crate) fn binop(op: i32, a: i64, b: i64) -> i64 {
    match (decode(a), decode(b)) {
        (Num::Float(x), Num::Float(y)) => float_result(float_binop(op, x, y)),
        (Num::Int(x), Num::Int(y)) => {
            if op == NUM_POW && decode_big(y).sign() == num_bigint::Sign::Minus {
                let base = int_to_float(x);
                if raised() {
                    return SENTINEL;
                }
                let exponent = int_to_float(y);
                if raised() {
                    return SENTINEL;
                }
                return float_result(float_pow(base, exponent));
            }
            int_result(int_binop(op, x, y))
        }
        (Num::Int(x), Num::Float(y)) => {
            let x = int_to_float(x);
            if raised() {
                return SENTINEL;
            }
            float_result(float_binop(op, x, y))
        }
        (Num::Float(x), Num::Int(y)) => {
            let y = int_to_float(y);
            if raised() {
                return SENTINEL;
            }
            float_result(float_binop(op, x, y))
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_num_binop(op: i32, a: i64, b: i64) -> i64 {
    binop(op, a, b)
}

/// `a / b`: always a `float`. Two `int`s divide exactly, then round once
/// (CPython's `long_true_divide`).
pub(crate) fn truediv(a: i64, b: i64) -> f64 {
    match (decode(a), decode(b)) {
        (Num::Int(x), Num::Int(y)) => int_truediv(x, y),
        _ => {
            let x = to_f64(a);
            if raised() {
                return 0.0;
            }
            let y = to_f64(b);
            if raised() {
                return 0.0;
            }
            float_div(x, y)
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_num_truediv(a: i64, b: i64) -> f64 {
    truediv(a, b)
}

/// `a op b` for a comparison (`op` is a `NUM_EQ`..`NUM_GE` code), exact
/// between an `int` and a `float`; every ordering with NaN is false.
pub(crate) fn compare_op(op: i32, a: i64, b: i64) -> bool {
    let ordering = compare(a, b);
    match op {
        NUM_EQ => ordering == Some(Ordering::Equal),
        NUM_NE => ordering != Some(Ordering::Equal),
        NUM_LT => ordering == Some(Ordering::Less),
        NUM_LE => matches!(ordering, Some(Ordering::Less | Ordering::Equal)),
        NUM_GT => ordering == Some(Ordering::Greater),
        NUM_GE => matches!(ordering, Some(Ordering::Greater | Ordering::Equal)),
        _ => unreachable!("pycc_rt: unknown `int | float` comparison {op}"),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_num_compare(op: i32, a: i64, b: i64) -> i8 {
    i8::from(compare_op(op, a, b))
}

/// `abs(x)`.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_num_abs(word: i64) -> i64 {
    match decode(word) {
        Num::Float(f) => from_f64(f.abs()),
        Num::Int(int) => int_result(int_abs(int)),
    }
}

/// `-x`.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_num_neg(word: i64) -> i64 {
    match decode(word) {
        Num::Float(f) => from_f64(-f),
        Num::Int(int) => int_result(int_sub(tag_smallint(0), int)),
    }
}

/// `+x`: the value itself, a `bool` as its `int`.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_num_pos(word: i64) -> i64 {
    match decode(word) {
        Num::Float(f) => from_f64(f),
        Num::Int(int) => int_result(int_add(tag_smallint(0), int)),
    }
}

/// `min(a, b)`: `b` when `b < a`, else `a` (the first of equal values,
/// as CPython's `min` keeps), as a fresh word.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_num_min2(a: i64, b: i64) -> i64 {
    pycc_rt_num_copy(if compare(b, a) == Some(Ordering::Less) { b } else { a })
}

/// `max(a, b)`: `b` when `b > a`, else `a`.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_num_max2(a: i64, b: i64) -> i64 {
    pycc_rt_num_copy(if compare(b, a) == Some(Ordering::Greater) { b } else { a })
}

/// A fresh copy of `word`: the value of an `int | float` read that is
/// kept (an `int` copy shares a heap object, which takes a reference).
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_num_copy(word: i64) -> i64 {
    pycc_rt_num_retain(word);
    word
}

/// `sum(xs)` of a `list[int | float]`, CPython's `builtin_sum` step for
/// step: `int`s accumulate exactly while they fit 64 bits; the first item
/// that does not (a `float`, or an overflow) is added to the `int` total as
/// an ordinary `+`. A `float` total then continues with Neumaier's
/// compensated summation over `float` items and adds 64-bit `int` items
/// uncompensated, and anything else is ordinary `+` from there on. The
/// list's words hold no heap payload (container ingress refuses one).
///
/// # Safety
/// `list` must be a live `PyListObj` pointer of `LIST_KIND_NUM`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_list_sum_num(list: *mut crate::list::PyListObj) -> i64 {
    let words = unsafe { &*list }.kinded_items();
    let mut items = words.iter().map(|(_, word)| *word);
    let mut total: i64 = 0;
    let mut result = loop {
        let Some(word) = items.next() else {
            return encode_big(num_bigint::BigInt::from(total));
        };
        if let Num::Int(int) = decode(word)
            && let Some(next) = inline_int_value(int).and_then(|b| total.checked_add(b))
        {
            total = next;
            continue;
        }
        let start = encode_big(num_bigint::BigInt::from(total));
        let sum = binop(NUM_ADD, start, word);
        bigint_release(start);
        if raised() {
            return SENTINEL;
        }
        break sum;
    };
    if let Num::Float(start) = decode(result) {
        pycc_rt_num_release(result);
        let (mut sum, mut c) = (start, 0.0f64);
        let finish = |sum: f64, c: f64| if c != 0.0 && c.is_finite() { sum + c } else { sum };
        loop {
            let Some(word) = items.next() else {
                return from_f64(finish(sum, c));
            };
            match decode(word) {
                Num::Float(x) => {
                    let t = sum + x;
                    if sum.abs() >= x.abs() {
                        c += (sum - t) + x;
                    } else {
                        c += (x - t) + sum;
                    }
                    sum = t;
                }
                Num::Int(int) if inline_int_value(int).is_some() => {
                    sum += inline_int_value(int).unwrap_or(0) as f64;
                }
                Num::Int(_) => {
                    let partial = from_f64(finish(sum, c));
                    result = binop(NUM_ADD, partial, word);
                    pycc_rt_num_release(partial);
                    if raised() {
                        return SENTINEL;
                    }
                    break;
                }
            }
        }
    }
    for word in items {
        let next = binop(NUM_ADD, result, word);
        pycc_rt_num_release(result);
        if raised() {
            return SENTINEL;
        }
        result = next;
    }
    result
}

/// `x in xs` for an `int | float` needle `word` and a list or tuple of
/// numbers of another kind: equal by value, as CPython's `==` between an
/// `int` and a `float`.
///
/// # Safety
/// `list` must be a live `PyListObj` pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_list_contains_num(list: *mut crate::list::PyListObj, word: i64) -> i8 {
    let needle = decode(word);
    let items = unsafe { &*list }.kinded_items();
    i8::from(items.iter().any(|(kind, item)| {
        of_kind(*kind, *item)
            .is_some_and(|value| compare_values(value, needle) == Some(Ordering::Equal))
    }))
}
