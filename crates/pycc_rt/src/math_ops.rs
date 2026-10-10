//! The integer functions of `math` (`gcd`, `lcm`, `isqrt`, `floor`,
//! `ceil`). Arguments are borrowed encoded `int` words; results are new
//! references, as for every other `pycc_rt_int_*` entry point.

use crate::exception::{EXCEPTION_TYPE_VALUE_ERROR, raise_builtin};
use crate::int_arith::{decode_big, encode_big, int_from_float};
use crate::int_encoding::{fits_smallint, inline_int_value, tag_smallint};
use num_bigint::BigInt;
use num_integer::{Integer, Roots};
use num_traits::{Signed, Zero};

/// `math.gcd(a, b)`: never negative.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_math_gcd(a: i64, b: i64) -> i64 {
    if let (Some(x), Some(y)) = (inline_int_value(a), inline_int_value(b)) {
        // Inline values are 63-bit, so their magnitudes fit an `i64`, but the
        // result can still exceed the smallint range: gcd(-2**62, 0) == 2**62.
        let g = x.abs().gcd(&y.abs());
        return fits_smallint(g).unwrap_or_else(|| encode_big(BigInt::from(g)));
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

fn value_error(message: &str) -> i64 {
    raise_builtin(EXCEPTION_TYPE_VALUE_ERROR, "ValueError", message);
    tag_smallint(0)
}

/// The product `lo * (lo + 1) * ... * hi` (1 when empty).
fn product(lo: &BigInt, hi: &BigInt) -> BigInt {
    let mut acc = BigInt::from(1);
    let mut i = lo.clone();
    while &i <= hi {
        acc *= &i;
        i += 1;
    }
    acc
}

/// `math.factorial(n)`.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_math_factorial(n: i64) -> i64 {
    let n = decode_big(n);
    if n.is_negative() {
        return value_error("factorial() not defined for negative values");
    }
    encode_big(product(&BigInt::from(2), &n))
}

/// `n` and `k` of `math.comb`/`math.perm`, or the `ValueError` they raise.
fn comb_args(n: i64, k: i64) -> Option<(BigInt, BigInt)> {
    let (n, k) = (decode_big(n), decode_big(k));
    if n.is_negative() {
        value_error("n must be a non-negative integer");
        return None;
    }
    if k.is_negative() {
        value_error("k must be a non-negative integer");
        return None;
    }
    Some((n, k))
}

/// `math.perm(n, k)`: `n! / (n - k)!`, 0 when `k > n`.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_math_perm(n: i64, k: i64) -> i64 {
    let Some((n, k)) = comb_args(n, k) else {
        return tag_smallint(0);
    };
    if k > n {
        return tag_smallint(0);
    }
    encode_big(product(&(&n - &k + 1), &n))
}

/// `math.comb(n, k)`: `n! / (k! (n - k)!)`, 0 when `k > n`.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_math_comb(n: i64, k: i64) -> i64 {
    let Some((n, k)) = comb_args(n, k) else {
        return tag_smallint(0);
    };
    if k > n {
        return tag_smallint(0);
    }
    let k = std::cmp::min(k.clone(), &n - &k);
    let mut acc = BigInt::from(1);
    let mut i = BigInt::from(0);
    while i < k {
        acc = acc * (&n - &i) / (&i + 1);
        i += 1;
    }
    encode_big(acc)
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
        let min = -(1_i64 << 62);
        let g = pycc_rt_math_gcd(tag_smallint(min), tag_smallint(0));
        assert_eq!(decode_big(g), BigInt::from(1_i64 << 62));
        assert_eq!(untag_smallint(pycc_rt_math_lcm(tag_smallint(-4), tag_smallint(6))), 12);
        assert_eq!(untag_smallint(pycc_rt_math_lcm(tag_smallint(0), tag_smallint(6))), 0);
        assert_eq!(untag_smallint(pycc_rt_math_isqrt(tag_smallint(99))), 9);
        assert_eq!(untag_smallint(pycc_rt_math_floor(-2.5)), -3);
        assert_eq!(untag_smallint(pycc_rt_math_ceil(-2.5)), -2);
    }
}
