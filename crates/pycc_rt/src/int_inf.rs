//! `float('inf')` held in an `int` slot.
//!
//! CPython lets one variable hold `float('inf')` until a `min` (or a plain
//! rebinding) replaces it with an `int` (`best = float('inf')` ...
//! `best = min(best, cost)`). `pycc_types` types such a variable as `int`
//! (its `int_inf` pass) and binds the infinity as the word made here: a
//! heap bigint with no limbs, which no canonical bigint is (zero is the
//! single limb `[0]`, and a small value is never a heap word). The
//! operations CPython defines on that infinity as a float with an `int`
//! result type here -- ordering and equality against ints and floats,
//! `min`/`max`, `str`/`repr`, `hash`, truth, `float()`, `abs`, `+`/`-`/`*`
//! by a finite `int` and `/` -- answer exactly as CPython's float infinity
//! does; every other operation reaches [`crate::int_arith::decode_big`],
//! which raises `OverflowError: cannot convert float infinity to integer`
//! where CPython would have produced a float (`docs/DEVIATIONS.md`).

use crate::int_encoding::{BigIntObj, inline_int_value, tag_bigint};

/// The sign of the infinity `word` holds (`Some(true)` for `-inf`), or
/// `None` for an ordinary int word.
pub(crate) fn inf_sign(word: i64) -> Option<bool> {
    if word == 0 || inline_int_value(word).is_some() || word & crate::int_encoding::LOW_TAG_MASK != 0 {
        return None;
    }
    // SAFETY: a non-zero aligned word of an `int` slot is a live heap bigint.
    let big = unsafe { &*(word as *const BigIntObj) };
    big.limbs.is_empty().then_some(big.negative)
}

/// A fresh word holding `-inf` when `negative`, else `inf`.
pub(crate) fn make_inf(negative: bool) -> i64 {
    tag_bigint(BigIntObj::new(negative, Vec::new()))
}

/// `float('inf')` bound to an `int` slot.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_int_inf_pos() -> i64 {
    make_inf(false)
}

/// `float('-inf')` bound to an `int` slot.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_int_inf_neg() -> i64 {
    make_inf(true)
}

/// The ordering of `a` and `b` when either is an infinity.
pub(crate) fn cmp(a: i64, b: i64) -> Option<std::cmp::Ordering> {
    let rank = |word: i64| match inf_sign(word) {
        Some(true) => -1,
        Some(false) => 1,
        None => 0,
    };
    let (ra, rb) = (rank(a), rank(b));
    (ra != 0 || rb != 0).then(|| ra.cmp(&rb))
}

/// The float value of an infinity word.
pub(crate) fn as_float(word: i64) -> Option<f64> {
    inf_sign(word).map(|negative| if negative { f64::NEG_INFINITY } else { f64::INFINITY })
}

/// `a + b` (`negate` false) or `a - b` (`negate` true) when either is an
/// infinity: the infinity, unless both are infinities that cancel (CPython's
/// `nan`), which falls through to the raising decode.
pub(crate) fn add(a: i64, b: i64, negate: bool) -> Option<i64> {
    let sa = inf_sign(a);
    let sb = inf_sign(b).map(|negative| negative != negate);
    match (sa, sb) {
        (Some(x), Some(y)) if x == y => Some(make_inf(x)),
        (Some(x), None) => Some(make_inf(x)),
        (None, Some(y)) => Some(make_inf(y)),
        _ => None,
    }
}

/// `a * b` when one side is an infinity and the other a non-zero finite
/// int, or both are infinities.
pub(crate) fn mul(a: i64, b: i64) -> Option<i64> {
    let sign_of = |word: i64| match inf_sign(word) {
        Some(negative) => Some(negative),
        None => crate::int_encoding::to_sign_and_magnitude(word)
            .1
            .iter()
            .any(|limb| *limb != 0)
            .then(|| crate::int_encoding::to_sign_and_magnitude(word).0),
    };
    if inf_sign(a).is_none() && inf_sign(b).is_none() {
        return None;
    }
    Some(make_inf(sign_of(a)? != sign_of(b)?))
}

/// `a / b` when either is an infinity (`inf / inf` is CPython's `nan`).
pub(crate) fn truediv(a: i64, b: i64) -> Option<f64> {
    if as_float(a).is_some() && inline_int_value(b) == Some(0) {
        crate::exception::raise_builtin(
            crate::EXCEPTION_TYPE_ZERO_DIV_ERROR,
            "ZeroDivisionError",
            "float division by zero",
        );
        return Some(0.0);
    }
    match (as_float(a), as_float(b)) {
        (Some(_), Some(_)) => Some(f64::NAN),
        (Some(x), None) => Some(x / crate::int_arith::int_to_float(b)),
        (None, Some(y)) => Some(crate::int_arith::int_to_float(a) / y),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::int_arith::{int_add, int_decimal, int_float_cmp, int_mul, int_sub, int_to_float};
    use crate::int_encoding::{encoded_int_cmp, tag_smallint};
    use std::cmp::Ordering;

    #[test]
    fn an_infinity_orders_above_and_below_every_int() {
        let (inf, ninf, five) = (make_inf(false), make_inf(true), tag_smallint(5));
        assert_eq!(encoded_int_cmp(inf, five), Ordering::Greater);
        assert_eq!(encoded_int_cmp(ninf, five), Ordering::Less);
        assert_eq!(encoded_int_cmp(inf, make_inf(false)), Ordering::Equal);
        assert_eq!(int_float_cmp(inf, f64::INFINITY), 0);
        assert_eq!(int_float_cmp(ninf, 1.5), -1);
        assert_eq!(int_to_float(ninf), f64::NEG_INFINITY);
        assert_eq!(int_decimal(inf).as_deref(), Some("inf"));
        assert_eq!(int_decimal(ninf).as_deref(), Some("-inf"));
        assert_eq!(inf_sign(five), None);
    }

    #[test]
    fn exact_arithmetic_keeps_the_infinity() {
        let (inf, five) = (make_inf(false), tag_smallint(5));
        assert_eq!(inf_sign(int_add(inf, five)), Some(false));
        assert_eq!(inf_sign(int_sub(five, inf)), Some(true));
        assert_eq!(inf_sign(int_mul(inf, tag_smallint(-2))), Some(true));
        assert_eq!(truediv(five, inf), Some(0.0));
        assert!(truediv(inf, make_inf(false)).is_some_and(f64::is_nan));
        assert_eq!(add(inf, make_inf(false), true), None);
        assert_eq!(mul(inf, tag_smallint(0)), None);
    }
}
