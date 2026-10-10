//! The float functions of `math` (`sqrt`, `exp`, `log`, the trigonometric
//! functions, ...) with CPython 3.13's error behavior: a domain error raises
//! `ValueError: math domain error`, an overflow `OverflowError: math range
//! error`. An `int` argument of the logarithms is taken exactly, as CPython's
//! `loghelper` does, so `log2(2**2000)` is `2000.0`; every other function
//! receives an `int` already converted to `float` (`OverflowError` beyond
//! its range).

use crate::exception::{
    EXCEPTION_TYPE_OVERFLOW_ERROR, EXCEPTION_TYPE_VALUE_ERROR, EXCEPTION_TYPE_ZERO_DIV_ERROR,
    raise_builtin,
};
use crate::int_arith::decode_big;
use crate::int_encoding::inline_int_value;
use num_traits::{Signed, ToPrimitive, Zero};

fn domain_error() -> f64 {
    raise_builtin(EXCEPTION_TYPE_VALUE_ERROR, "ValueError", "math domain error");
    0.0
}

fn range_error() -> f64 {
    raise_builtin(EXCEPTION_TYPE_OVERFLOW_ERROR, "OverflowError", "math range error");
    0.0
}

/// CPython's `math_1` rule: a NaN from a non-NaN argument is a domain
/// error, an infinity from a finite argument a range error.
fn checked(x: f64, r: f64, overflow_is_range: bool) -> f64 {
    if r.is_nan() && !x.is_nan() {
        return domain_error();
    }
    if r.is_infinite() && x.is_finite() {
        return if overflow_is_range {
            range_error()
        } else {
            domain_error()
        };
    }
    r
}

macro_rules! unary {
    ($name:ident, $f:expr, $range:expr) => {
        #[unsafe(no_mangle)]
        pub extern "C" fn $name(x: f64) -> f64 {
            let f: fn(f64) -> f64 = $f;
            checked(x, f(x), $range)
        }
    };
}

unary!(pycc_rt_math_sqrt, f64::sqrt, false);
unary!(pycc_rt_math_exp, f64::exp, true);
unary!(pycc_rt_math_sin, f64::sin, false);
unary!(pycc_rt_math_cos, f64::cos, false);
unary!(pycc_rt_math_tan, f64::tan, false);
unary!(pycc_rt_math_asin, f64::asin, false);
unary!(pycc_rt_math_acos, f64::acos, false);
unary!(pycc_rt_math_atan, f64::atan, false);
unary!(pycc_rt_math_fabs, f64::abs, false);

/// The natural, base-2 or base-10 logarithm of a float: non-positive
/// arguments are a domain error (`log(0.0)` included, unlike libm's `-inf`).
fn float_log(x: f64, f: fn(f64) -> f64) -> f64 {
    if x.is_nan() || x == f64::INFINITY {
        return x;
    }
    if x <= 0.0 {
        return domain_error();
    }
    f(x)
}

/// CPython's `loghelper` for an `int`: exact through `frexp` when the value
/// does not fit a float.
fn int_log(word: i64, f: fn(f64) -> f64) -> f64 {
    if let Some(v) = inline_int_value(word) {
        if v <= 0 {
            return domain_error();
        }
        return f(v as f64);
    }
    let n = decode_big(word);
    if !n.is_positive() || n.is_zero() {
        return domain_error();
    }
    if let Some(x) = n.to_f64()
        && x.is_finite()
    {
        return f(x);
    }
    // n = m * 2**e with m in [0.5, 1): keep 60 significant bits of n.
    let bits = n.bits() as i64;
    let shift = bits - 60;
    let m = (&n >> shift as usize).to_f64().expect("60 bits fit a float") / (1u64 << 60) as f64;
    f(m) + f(2.0) * bits as f64
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_math_log(x: f64) -> f64 {
    float_log(x, f64::ln)
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_math_log2(x: f64) -> f64 {
    float_log(x, f64::log2)
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_math_log10(x: f64) -> f64 {
    float_log(x, f64::log10)
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_math_log_int(n: i64) -> f64 {
    int_log(n, f64::ln)
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_math_log2_int(n: i64) -> f64 {
    int_log(n, f64::log2)
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_math_log10_int(n: i64) -> f64 {
    int_log(n, f64::log10)
}

/// `log(x, base)` from the two natural logarithms (CPython divides them).
fn log_base(num: f64, den: f64) -> f64 {
    if crate::exception::pycc_rt_exception_active() != 0 {
        return 0.0;
    }
    if den == 0.0 {
        raise_builtin(
            EXCEPTION_TYPE_ZERO_DIV_ERROR,
            "ZeroDivisionError",
            "float division by zero",
        );
        return 0.0;
    }
    num / den
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_math_log_base_ff(x: f64, base: f64) -> f64 {
    let num = pycc_rt_math_log(x);
    let den = pycc_rt_math_log(base);
    log_base(num, den)
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_math_log_base_if(x: i64, base: f64) -> f64 {
    let num = pycc_rt_math_log_int(x);
    let den = pycc_rt_math_log(base);
    log_base(num, den)
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_math_log_base_fi(x: f64, base: i64) -> f64 {
    let num = pycc_rt_math_log(x);
    let den = pycc_rt_math_log_int(base);
    log_base(num, den)
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_math_log_base_ii(x: i64, base: i64) -> f64 {
    let num = pycc_rt_math_log_int(x);
    let den = pycc_rt_math_log_int(base);
    log_base(num, den)
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_math_atan2(y: f64, x: f64) -> f64 {
    y.atan2(x)
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_math_hypot(x: f64, y: f64) -> f64 {
    let r = x.hypot(y);
    if r.is_infinite() && x.is_finite() && y.is_finite() {
        return range_error();
    }
    r
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_math_copysign(x: f64, y: f64) -> f64 {
    x.copysign(y)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::int_encoding::tag_smallint;

    #[test]
    fn logarithms_of_ints_and_floats() {
        assert_eq!(pycc_rt_math_log2_int(tag_smallint(1024)), 10.0);
        assert_eq!(pycc_rt_math_log10(1000.0), 3.0);
        assert_eq!(pycc_rt_math_log(1.0), 0.0);
        let big = crate::int_arith::int_pow(tag_smallint(2), tag_smallint(2000));
        assert_eq!(pycc_rt_math_log2_int(big), 2000.0);
        assert_eq!(pycc_rt_math_log_base_ii(tag_smallint(8), tag_smallint(2)), 3.0);
    }

    #[test]
    fn finite_results_pass_through_checked() {
        assert_eq!(pycc_rt_math_sqrt(4.0), 2.0);
        assert_eq!(pycc_rt_math_fabs(-1.5), 1.5);
        assert!(pycc_rt_math_sqrt(f64::NAN).is_nan());
        assert_eq!(pycc_rt_math_exp(f64::INFINITY), f64::INFINITY);
    }
}
