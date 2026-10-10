//! The integer functions of `math` (`gcd`, `lcm`, `isqrt`, `floor`,
//! `ceil`). Arguments are borrowed encoded `int` words; results are new
//! references, as for every other `pycc_rt_int_*` entry point.

use crate::exception::{EXCEPTION_TYPE_VALUE_ERROR, raise_builtin};
use crate::int_arith::{decode_big, encode_big, int_from_float};
use crate::int_encoding::{inline_int_value, tag_smallint};
use num_integer::{Integer, Roots};
use num_traits::{Signed, Zero};

/// `math.gcd(a, b)`: never negative.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_math_gcd(a: i64, b: i64) -> i64 {
    if let (Some(x), Some(y)) = (inline_int_value(a), inline_int_value(b)) {
        // Inline values are 63-bit, so their magnitudes fit an `i64`.
        return tag_smallint(x.abs().gcd(&y.abs()));
    }
    encode_big(decode_big(a).gcd(&decode_big(b)))
}

/// `math.lcm(a, b)`: never negative, `0` when either is `0`.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_math_lcm(a: i64, b: i64) -> i64 {
    let (a, b) = (decode_big(a), decode_big(b));
    if a.is_zero() || b.is_zero() {
        return tag_smallint(0);
    }
    encode_big(a.lcm(&b).abs())
}

/// `math.isqrt(n)`: the floor of the square root of a non-negative `n`.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_math_isqrt(n: i64) -> i64 {
    let n = decode_big(n);
    if n.is_negative() {
        raise_builtin(
            EXCEPTION_TYPE_VALUE_ERROR,
            "ValueError",
            "isqrt() argument must be nonnegative",
        );
        return tag_smallint(0);
    }
    encode_big(n.sqrt())
}

/// `math.floor(x)` of a `float` (`ValueError`/`OverflowError` for NaN and
/// infinities, with `int()`'s messages, as CPython).
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_math_floor(x: f64) -> i64 {
    int_from_float(x.floor())
}

/// `math.ceil(x)` of a `float`.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_math_ceil(x: f64) -> i64 {
    int_from_float(x.ceil())
}

/// `math.floor`/`math.ceil` of an `int`: the value itself.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_math_int_identity(n: i64) -> i64 {
    crate::int_encoding::bigint_retain(n);
    n
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::int_encoding::untag_smallint;

    #[test]
    fn gcd_lcm_and_isqrt_of_small_values() {
        assert_eq!(untag_smallint(pycc_rt_math_gcd(tag_smallint(-12), tag_smallint(18))), 6);
        assert_eq!(untag_smallint(pycc_rt_math_gcd(tag_smallint(0), tag_smallint(0))), 0);
        assert_eq!(untag_smallint(pycc_rt_math_lcm(tag_smallint(-4), tag_smallint(6))), 12);
        assert_eq!(untag_smallint(pycc_rt_math_lcm(tag_smallint(0), tag_smallint(6))), 0);
        assert_eq!(untag_smallint(pycc_rt_math_isqrt(tag_smallint(99))), 9);
        assert_eq!(untag_smallint(pycc_rt_math_floor(-2.5)), -3);
        assert_eq!(untag_smallint(pycc_rt_math_ceil(-2.5)), -2);
    }
}
