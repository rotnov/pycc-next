//! The `round()` builtin over `float`, matching CPython's `float.__round__`.
//!
//! CPython rounds the float's *exact* binary value to `ndigits` decimal
//! places, ties to even (`_Py_dg_dtoa` mode 3), then converts the decimal
//! string back to the nearest double. Here the exact value comes from
//! Rust's exact decimal formatting with enough digits to be lossless (an
//! `f64` has at most 1074 fractional decimal digits), the tie-to-even step
//! is done on that digit string, and `str::parse::<f64>` is correctly
//! rounded, so every step is exact and the result is CPython's.

use super::*;

/// CPython's `NDIGITS_MAX`: past this many digits every double is already
/// exactly representable, so the value is returned unchanged.
const NDIGITS_MAX: i64 = 323;
/// CPython's `NDIGITS_MIN`: below this the result is a (signed) zero.
const NDIGITS_MIN: i64 = -308;

/// Rounds the decimal digit string `digits` (no sign, no point) to its first
/// `keep` digits, ties to even. Returns the kept digits (one longer when the
/// rounding carries out of the top digit).
fn round_digits(digits: &[u8], keep: usize) -> Vec<u8> {
    let mut kept = digits[..keep.min(digits.len())].to_vec();
    let Some(&next) = digits.get(keep) else {
        return kept;
    };
    let rest_nonzero = digits[keep + 1..].iter().any(|d| *d != b'0');
    let last_odd = kept.last().is_some_and(|d| (d - b'0') % 2 == 1);
    let round_up = next > b'5' || (next == b'5' && (rest_nonzero || last_odd));
    if round_up {
        let mut i = kept.len();
        loop {
            if i == 0 {
                kept.insert(0, b'1');
                break;
            }
            i -= 1;
            if kept[i] == b'9' {
                kept[i] = b'0';
            } else {
                kept[i] += 1;
                break;
            }
        }
    }
    kept
}

/// `round(x, ndigits)` for a `float` `x`.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_float_round(x: f64, ndigits: i64) -> f64 {
    if !x.is_finite() || ndigits > NDIGITS_MAX {
        return x;
    }
    if ndigits < NDIGITS_MIN {
        return 0.0 * x;
    }
    let exact = format!("{:.1100}", x.abs());
    let (int_part, frac_part) = exact.split_once('.').expect("a fixed-point rendering");
    let digits: Vec<u8> = int_part.bytes().chain(frac_part.bytes()).collect();
    // The rounding position, counted in digits from the start of `digits`.
    let position = int_part.len() as i64 + ndigits;
    // The kept digits `K` (one longer after a carry out of the top digit)
    // stand for `K * 10^(int_len - position)`.
    let kept = if position < 0 {
        Vec::new()
    } else {
        round_digits(&digits, position as usize)
    };
    let text = if kept.is_empty() {
        "0".to_string()
    } else {
        format!(
            "{}e{}",
            String::from_utf8(kept).expect("ASCII digits"),
            int_part.len() as i64 - position
        )
    };
    let magnitude: f64 = text.parse().expect("a decimal float literal");
    if x.is_sign_negative() {
        -magnitude
    } else {
        magnitude
    }
}

/// `round(x)` for a `float` `x`: the nearest integer, ties to even, as an
/// encoded `int`. A NaN raises `ValueError`, an infinity `OverflowError`,
/// exactly as CPython; an integer past the inline range is a bigint.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_float_round_to_int(x: f64) -> i64 {
    if x.is_nan() {
        raise_builtin(
            EXCEPTION_TYPE_VALUE_ERROR,
            "ValueError",
            "cannot convert float NaN to integer",
        );
        return tag_smallint(0);
    }
    if x.is_infinite() {
        raise_builtin(
            EXCEPTION_TYPE_OVERFLOW_ERROR,
            "OverflowError",
            "cannot convert float infinity to integer",
        );
        return tag_smallint(0);
    }
    let rounded = x.round_ties_even();
    if rounded.abs() >= 9.2e18 {
        // Exact: a float this large is already an integer.
        return crate::int_arith::int_from_float(rounded);
    }
    pycc_rt_int_from_i64(rounded as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_to_digits_matches_cpython() {
        let cases: [(f64, i64, f64); 10] = [
            (2.675, 2, 2.67),
            (0.5, 0, 0.0),
            (1.5, 0, 2.0),
            (2.5, 0, 2.0),
            (-0.1234567891234, 9, -0.123456789),
            (1234.5678, -2, 1200.0),
            (1250.0, -2, 1200.0),
            (1350.0, -2, 1400.0),
            (99.96, 1, 100.0),
            (0.0001, 2, 0.0),
        ];
        for (x, n, expected) in cases {
            assert_eq!(pycc_rt_float_round(x, n), expected, "round({x}, {n})");
        }
        assert!(pycc_rt_float_round(f64::NAN, 2).is_nan());
        assert_eq!(pycc_rt_float_round(5.0, 400), 5.0);
        assert!(pycc_rt_float_round(-5.0, -400).is_sign_negative());
        assert_eq!(pycc_rt_float_round(5.0, -3), 0.0);
    }

    #[test]
    fn round_to_int_is_ties_to_even_and_raises_like_cpython() {
        assert_eq!(untag_smallint(pycc_rt_float_round_to_int(2.5)), 2);
        assert_eq!(untag_smallint(pycc_rt_float_round_to_int(-3.5)), -4);
        crate::pycc_rt_exception_clear();
        pycc_rt_float_round_to_int(f64::NAN);
        assert_eq!(
            crate::tests::pending_tag_and_message().1,
            "cannot convert float NaN to integer"
        );
        crate::pycc_rt_exception_clear();
        pycc_rt_float_round_to_int(f64::INFINITY);
        assert_eq!(
            crate::tests::pending_tag_and_message().1,
            "cannot convert float infinity to integer"
        );
        crate::pycc_rt_exception_clear();
        pycc_rt_float_round_to_int(1e19);
        assert_ne!(crate::pycc_rt_exception_active(), 0);
        crate::pycc_rt_exception_clear();
    }
}
