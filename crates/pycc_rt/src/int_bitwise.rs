//! `int`'s bitwise and shift operators (#1210): `<<`, `>>`, `&`, `|`, `^`.
//!
//! Its own module rather than more of `lib.rs`, under this repository's
//! "keep source files decomposable" rule. Each operation follows
//! `int_add`/`int_sub`'s model rather than #1040's inline-only one: a heap
//! bigint operand is accepted and a result that leaves the inline range is
//! a heap bigint, so the values match CPython's arbitrary-precision `int`.
//!
//! Every result goes through [`from_sign_magnitude`], which returns an
//! ordinary smallint when the value fits and a *fresh* `BigIntObj`
//! otherwise, so no operation hands back an operand's own heap word (the
//! D-181 invariant `pycc_codegen`'s operand releases rely on). The one word
//! a result may share with an operand is a D-141 bool marker: `True & True`
//! is `True`, and `&`, `|` and `^` over two markers return a marker. That
//! is safe because a marker is not a heap object and `bigint_release` is a
//! no-op on it.
//!
//! Values are decided by value, never by encoding. `int_add`/`int_sub` can
//! hand back a heap object holding a small value, so the slow paths decode
//! every operand through `to_sign_and_magnitude` and test zero and sign with
//! `magnitude_sign`; only the fast paths, taken when both operands are
//! inline, key on the encoding.
//!
//! One recorded deviation from CPython (the 2026-09-23 amendment to D-244):
//! where CPython raises `MemoryError` for `<<` -- an allocation the system
//! refuses, or a count from `2**62` up to its own `OverflowError` bound --
//! pycc raises `OverflowError("too many digits in integer")`, because pycc
//! has no `MemoryError` class.

use crate::exception::raise_builtin;
use crate::int_encoding::*;
use crate::{EXCEPTION_TYPE_OVERFLOW_ERROR, EXCEPTION_TYPE_VALUE_ERROR};

/// The encoded word for a sign-magnitude value: an ordinary smallint when it
/// fits, a fresh heap bigint otherwise. Zero is never negative.
fn from_sign_magnitude(negative: bool, magnitude: &[u32]) -> i64 {
    let magnitude = trim(magnitude);
    if magnitude.len() <= 2 {
        let raw = u64::from(magnitude[0]) | (u64::from(*magnitude.get(1).unwrap_or(&0)) << 32);
        let value = if negative {
            -i128::from(raw)
        } else {
            i128::from(raw)
        };
        if let Some(word) = i64::try_from(value).ok().and_then(fits_smallint) {
            return word;
        }
    }
    let negative = negative && magnitude != [0];
    tag_bigint(BigIntObj::new(negative, magnitude))
}

/// Negates `limbs` in place as a `limbs.len()`-limb two's-complement number.
fn negate_twos_complement(limbs: &mut [u32]) {
    let mut carry = 1u64;
    for limb in limbs {
        let sum = u64::from(!*limb) + carry;
        *limb = sum as u32;
        carry = sum >> 32;
    }
}

/// `value`'s `width`-limb two's-complement form. `width` must exceed the
/// magnitude's length so the top limb holds the sign.
fn to_twos_complement(negative: bool, magnitude: &[u32], width: usize) -> Vec<u32> {
    let mut limbs: Vec<u32> = (0..width)
        .map(|i| *magnitude.get(i).unwrap_or(&0))
        .collect();
    if negative {
        negate_twos_complement(&mut limbs);
    }
    limbs
}

/// The encoded word for a two's-complement limb vector.
fn from_twos_complement(mut limbs: Vec<u32>) -> i64 {
    let negative = limbs.last().is_some_and(|top| top >> 31 == 1);
    if negative {
        negate_twos_complement(&mut limbs);
    }
    from_sign_magnitude(negative, &limbs)
}

#[derive(Clone, Copy)]
enum BitOp {
    And,
    Or,
    Xor,
}

impl BitOp {
    fn apply(self, a: u64, b: u64) -> u64 {
        match self {
            BitOp::And => a & b,
            BitOp::Or => a | b,
            BitOp::Xor => a ^ b,
        }
    }
}

fn is_bool_marker(word: i64) -> bool {
    word == BOOL_FALSE_MARKER || word == BOOL_TRUE_MARKER
}

fn int_bitop(a: i64, b: i64, op: BitOp) -> i64 {
    if let (Some(x), Some(y)) = (inline_int_value(a), inline_int_value(b)) {
        // Two values in the inline range combine to a value in it.
        let result = op.apply(x as u64, y as u64) as i64;
        if is_bool_marker(a) && is_bool_marker(b) {
            return if result == 0 {
                BOOL_FALSE_MARKER
            } else {
                BOOL_TRUE_MARKER
            };
        }
        return tag_smallint(result);
    }
    let (a_negative, a_magnitude) = to_sign_and_magnitude(a);
    let (b_negative, b_magnitude) = to_sign_and_magnitude(b);
    let width = a_magnitude.len().max(b_magnitude.len()) + 1;
    let left = to_twos_complement(a_negative, &a_magnitude, width);
    let right = to_twos_complement(b_negative, &b_magnitude, width);
    let limbs = left
        .iter()
        .zip(&right)
        .map(|(&x, &y)| op.apply(u64::from(x), u64::from(y)) as u32)
        .collect();
    from_twos_complement(limbs)
}

/// A shift count, decided by value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Count {
    Negative,
    /// A count within the inline range, `0..2**62`.
    Small(u64),
    /// A count whose magnitude exceeds the inline range.
    Huge,
}

const INLINE_MAX: u64 = (1 << 62) - 1;

fn decode_count(count: i64) -> Count {
    let (negative, magnitude) = to_sign_and_magnitude(count);
    if magnitude_sign(negative, &magnitude) < 0 {
        return Count::Negative;
    }
    let magnitude = trim(&magnitude);
    if magnitude.len() > 2 {
        return Count::Huge;
    }
    let value = u64::from(magnitude[0]) | (u64::from(*magnitude.get(1).unwrap_or(&0)) << 32);
    if value > INLINE_MAX {
        Count::Huge
    } else {
        Count::Small(value)
    }
}

fn raise_negative_shift_count() -> i64 {
    raise_builtin(
        EXCEPTION_TYPE_VALUE_ERROR,
        "ValueError",
        "negative shift count",
    );
    tag_smallint(0)
}

fn raise_too_many_digits() -> i64 {
    raise_builtin(
        EXCEPTION_TYPE_OVERFLOW_ERROR,
        "OverflowError",
        "too many digits in integer",
    );
    tag_smallint(0)
}

/// `magnitude << count`, or `None` when the result cannot be allocated.
/// Reserving with `try_reserve_exact` is what turns a refused allocation
/// into an error instead of a process abort.
fn shift_magnitude_left(magnitude: &[u32], count: u64) -> Option<Vec<u32>> {
    let limb_shift = usize::try_from(count / 32).ok()?;
    let bit = (count % 32) as u32;
    let len = limb_shift.checked_add(magnitude.len())?.checked_add(1)?;
    let mut limbs: Vec<u32> = Vec::new();
    limbs.try_reserve_exact(len).ok()?;
    limbs.resize(limb_shift, 0);
    let mut carry = 0u32;
    for &limb in magnitude {
        let wide = u64::from(limb) << bit;
        limbs.push(wide as u32 | carry);
        carry = (wide >> 32) as u32;
    }
    limbs.push(carry);
    Some(limbs)
}

/// `magnitude >> count`, truncating toward zero.
fn shift_magnitude_right(magnitude: &[u32], count: u64) -> Vec<u32> {
    let limb_shift = usize::try_from(count / 32).unwrap_or(usize::MAX);
    if limb_shift >= magnitude.len() {
        return vec![0];
    }
    let bit = count % 32;
    (limb_shift..magnitude.len())
        .map(|i| {
            let wide =
                u64::from(magnitude[i]) | (u64::from(*magnitude.get(i + 1).unwrap_or(&0)) << 32);
            (wide >> bit) as u32
        })
        .collect()
}

fn int_lshift(a: i64, n: i64) -> i64 {
    let count = decode_count(n);
    if count == Count::Negative {
        return raise_negative_shift_count();
    }
    if let (Some(x), Some(s)) = (inline_int_value(a), inline_int_value(n))
        && s < 64
    {
        // `|x| < 2**62` and `s <= 63` keep the result under `2**126`.
        let value = i128::from(x) << s;
        return i64::try_from(value)
            .ok()
            .and_then(fits_smallint)
            .unwrap_or_else(|| tag_bigint(bigint_from_i128(value)));
    }
    let (negative, magnitude) = to_sign_and_magnitude(a);
    if magnitude_sign(negative, &magnitude) == 0 {
        return tag_smallint(0);
    }
    let Count::Small(s) = count else {
        return raise_too_many_digits();
    };
    match shift_magnitude_left(&magnitude, s) {
        Some(limbs) => from_sign_magnitude(negative, &limbs),
        None => raise_too_many_digits(),
    }
}

fn int_rshift(a: i64, n: i64) -> i64 {
    let count = decode_count(n);
    if count == Count::Negative {
        return raise_negative_shift_count();
    }
    if let (Some(x), Some(s)) = (inline_int_value(a), inline_int_value(n)) {
        // An arithmetic shift already floors; the clamp keeps Rust's `>>`
        // from panicking at `s >= 64`, where the result is `0` or `-1`.
        return tag_smallint(x >> s.min(63));
    }
    let (negative, magnitude) = to_sign_and_magnitude(a);
    let sign = magnitude_sign(negative, &magnitude);
    let Count::Small(s) = count else {
        return tag_smallint(if sign < 0 { -1 } else { 0 });
    };
    if sign >= 0 {
        return from_sign_magnitude(false, &shift_magnitude_right(&magnitude, s));
    }
    // Floor for a negative value: `a >> s == -(((|a| - 1) >> s) + 1)`.
    let less_one = trim(&magnitude_sub(&magnitude, &[1]));
    let shifted = shift_magnitude_right(&less_one, s);
    from_sign_magnitude(true, &magnitude_add(&shifted, &[1]))
}

/// `a << n`. A negative `n` raises `ValueError`; a result too large to
/// allocate raises `OverflowError` (D-244's recorded deviation from
/// CPython's `MemoryError`). Returns the D-173 sentinel `0` after a raise.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_int_lshift(a: i64, n: i64) -> i64 {
    int_lshift(a, n)
}

/// `a >> n`, flooring. A negative `n` raises `ValueError` and returns the
/// D-173 sentinel `0`.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_int_rshift(a: i64, n: i64) -> i64 {
    int_rshift(a, n)
}

/// `a & b`; two bool markers give a bool marker.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_int_and(a: i64, b: i64) -> i64 {
    int_bitop(a, b, BitOp::And)
}

/// `a | b`; two bool markers give a bool marker.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_int_or(a: i64, b: i64) -> i64 {
    int_bitop(a, b, BitOp::Or)
}

/// `a ^ b`; two bool markers give a bool marker.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_int_xor(a: i64, b: i64) -> i64 {
    int_bitop(a, b, BitOp::Xor)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{pycc_rt_exception_clear, pycc_rt_ext_pending_message, pycc_rt_ext_pending_type};

    /// A heap word for `value`, built directly rather than through an
    /// operation so the tests do not depend on the code they test.
    fn big(value: i128) -> i64 {
        tag_bigint(bigint_from_i128(value))
    }

    /// The value of an encoded word, as `i128` (every test value fits).
    fn value_of(word: i64) -> i128 {
        let (negative, magnitude) = to_sign_and_magnitude(word);
        let magnitude = trim(&magnitude);
        assert!(magnitude.len() <= 4, "test values fit i128");
        let raw = magnitude
            .iter()
            .rev()
            .fold(0u128, |acc, &limb| (acc << 32) | u128::from(limb));
        if negative {
            -(raw as i128)
        } else {
            raw as i128
        }
    }

    /// A non-canonical heap word: a `BigIntObj` holding a small value, as
    /// `int_add`/`int_sub` produce for `(1 << 70) - (1 << 70) + 5`.
    fn non_canonical(value: i64) -> i64 {
        big(i128::from(value))
    }

    /// The pending exception's type tag and message, clearing it.
    /// A deferred raising call, run only once the previous case's
    /// pending exception has been read.
    type Case = fn() -> i64;

    fn take_pending() -> Option<(i32, String)> {
        let tag = pycc_rt_ext_pending_type();
        if tag < 0 {
            return None;
        }
        let mut len = 0usize;
        let bytes = unsafe { pycc_rt_ext_pending_message(&raw mut len) };
        let message =
            String::from_utf8_lossy(unsafe { std::slice::from_raw_parts(bytes, len) }).into_owned();
        pycc_rt_exception_clear();
        Some((tag, message))
    }

    /// Asserts `word` is `expected` and that nothing was raised.
    fn assert_value(word: i64, expected: i128, what: &str) {
        assert_eq!(take_pending(), None, "{what} raised");
        assert_eq!(value_of(word), expected, "{what}");
    }

    #[test]
    fn the_inline_fast_path_matches_cpython() {
        pycc_rt_exception_clear();
        for (word, expected, what) in [
            (
                pycc_rt_int_and(tag_smallint(12), tag_smallint(10)),
                8,
                "12 & 10",
            ),
            (
                pycc_rt_int_or(tag_smallint(12), tag_smallint(10)),
                14,
                "12 | 10",
            ),
            (
                pycc_rt_int_xor(tag_smallint(12), tag_smallint(10)),
                6,
                "12 ^ 10",
            ),
            (
                int_bitop(tag_smallint(-6), tag_smallint(3), BitOp::And),
                2,
                "-6 & 3",
            ),
            (
                int_bitop(tag_smallint(-6), tag_smallint(3), BitOp::Or),
                -5,
                "-6 | 3",
            ),
            (
                int_bitop(tag_smallint(-6), tag_smallint(-3), BitOp::Xor),
                7,
                "-6 ^ -3",
            ),
            (
                pycc_rt_int_lshift(tag_smallint(5), tag_smallint(3)),
                40,
                "5 << 3",
            ),
            (
                int_lshift(tag_smallint(-5), tag_smallint(3)),
                -40,
                "-5 << 3",
            ),
            (
                pycc_rt_int_rshift(tag_smallint(-7), tag_smallint(1)),
                -4,
                "-7 >> 1",
            ),
            (int_rshift(tag_smallint(7), tag_smallint(1)), 3, "7 >> 1"),
            (
                int_rshift(tag_smallint(-7), tag_smallint(64)),
                -1,
                "-7 >> 64",
            ),
            (
                int_rshift(tag_smallint(7), tag_smallint(1000)),
                0,
                "7 >> 1000",
            ),
            (
                int_lshift(tag_smallint(0), tag_smallint(1000)),
                0,
                "0 << 1000",
            ),
        ] {
            assert_value(word, expected, what);
        }
    }

    /// An inline shift whose result leaves the inline range promotes to a
    /// fresh heap bigint, in both directions of sign.
    #[test]
    fn an_inline_left_shift_promotes_past_the_inline_range() {
        pycc_rt_exception_clear();
        let word = int_lshift(tag_smallint(1), tag_smallint(62));
        assert_eq!(classify_encoded_int(word), EncodedIntKind::BigInt);
        assert_value(word, 1i128 << 62, "1 << 62");
        let word = int_lshift(tag_smallint(-3), tag_smallint(63));
        assert_value(word, -3i128 << 63, "-3 << 63");
        // `-(1 << 62)` is the smallest inline value, and stays inline.
        let word = int_lshift(tag_smallint(-1), tag_smallint(62));
        assert!(is_smallint(word));
        assert_value(word, -(1i128 << 62), "-1 << 62");
        // A count of 64 or more leaves the `i128` fast path.
        assert_value(
            int_lshift(tag_smallint(3), tag_smallint(64)),
            3i128 << 64,
            "3 << 64",
        );
        assert_value(
            int_lshift(tag_smallint(-1), tag_smallint(90)),
            -(1i128 << 90),
            "-1 << 90",
        );
    }

    /// Bigint operands of both signs, cross-checked against CPython 3.14.7
    /// (`python3.14 -c "print(...)"` for each expression below).
    #[test]
    fn bigint_operands_match_cpython() {
        pycc_rt_exception_clear();
        let cases = [
            (
                int_bitop(big(-(1i128 << 70)), big((1i128 << 65) + 12345), BitOp::And),
                0,
                "-(1<<70) & ((1<<65)+12345)",
            ),
            (
                int_bitop(big(-(1i128 << 70) + 7), big(1i128 << 66), BitOp::Or),
                -1106804644422573096953,
                "(-(1<<70)+7) | (1<<66)",
            ),
            (
                int_bitop(
                    big(-(1i128 << 70) - 3),
                    big(-((1i128 << 68) + 99)),
                    BitOp::Xor,
                ),
                1475739525896764129376,
                "(-(1<<70)-3) ^ -((1<<68)+99)",
            ),
            (
                int_rshift(big(-((1i128 << 70) + 1)), tag_smallint(3)),
                -147573952589676412929,
                "-((1<<70)+1) >> 3",
            ),
            (
                int_rshift(big(-((1i128 << 70) + 1)), tag_smallint(70)),
                -2,
                "-((1<<70)+1) >> 70",
            ),
            (
                int_rshift(big(-(1i128 << 70)), tag_smallint(70)),
                -1,
                "-(1<<70) >> 70",
            ),
            (
                int_rshift(big(1i128 << 70), tag_smallint(35)),
                1i128 << 35,
                "(1<<70) >> 35",
            ),
            (
                int_rshift(big(1i128 << 70), tag_smallint(200)),
                0,
                "(1<<70) >> 200",
            ),
            (
                int_rshift(big(-(1i128 << 70)), tag_smallint(200)),
                -1,
                "-(1<<70) >> 200",
            ),
            (
                int_lshift(big((1i128 << 70) + 5), tag_smallint(33)),
                ((1i128 << 70) + 5) << 33,
                "((1<<70)+5) << 33",
            ),
            (
                int_lshift(big(-(1i128 << 64)), tag_smallint(32)),
                -(1i128 << 96),
                "-(1<<64) << 32",
            ),
            (
                int_bitop(big(1i128 << 70), tag_smallint(-1), BitOp::And),
                1i128 << 70,
                "(1<<70) & -1",
            ),
            (
                int_bitop(big((1i128 << 70) + 3), big(1i128 << 70), BitOp::Xor),
                3,
                "((1<<70)+3) ^ (1<<70)",
            ),
            (
                int_bitop(big(-(1i128 << 70)), big(1i128 << 70), BitOp::Xor),
                -(1i128 << 71),
                "-(1<<70) ^ (1<<70)",
            ),
        ];
        for (word, expected, what) in cases {
            assert_value(word, expected, what);
        }
    }

    /// A result that fits comes back as a smallint even from heap operands.
    #[test]
    fn a_small_result_from_bigint_operands_is_a_smallint() {
        pycc_rt_exception_clear();
        let word = int_bitop(big((1i128 << 70) + 3), big(1i128 << 70), BitOp::Xor);
        assert!(is_smallint(word));
        let word = int_rshift(big(1i128 << 70), tag_smallint(69));
        assert!(is_smallint(word));
        assert_value(word, 2, "(1<<70) >> 69");
    }

    /// A two-limb result past the inline range stays a heap bigint, of
    /// either sign.
    #[test]
    fn a_two_limb_result_past_the_inline_range_is_a_bigint() {
        pycc_rt_exception_clear();
        let word = int_rshift(big(1i128 << 70), tag_smallint(7));
        assert!(!is_smallint(word));
        assert_value(word, 1i128 << 63, "(1<<70) >> 7");
        let word = int_rshift(big(-(1i128 << 70)), tag_smallint(7));
        assert!(!is_smallint(word));
        assert_value(word, -(1i128 << 63), "-(1<<70) >> 7");
    }

    /// `&`, `|` and `^` over two bool markers keep bool identity; a marker
    /// with a smallint, and every shift, give an ordinary int.
    #[test]
    fn two_bool_markers_give_a_bool_marker_and_nothing_else_does() {
        let (f, t) = (BOOL_FALSE_MARKER, BOOL_TRUE_MARKER);
        for (word, expected) in [
            (int_bitop(t, t, BitOp::And), t),
            (int_bitop(t, f, BitOp::And), f),
            (int_bitop(f, f, BitOp::Or), f),
            (int_bitop(t, f, BitOp::Or), t),
            (int_bitop(t, t, BitOp::Xor), f),
            (int_bitop(f, t, BitOp::Xor), t),
            (int_bitop(t, tag_smallint(1), BitOp::And), tag_smallint(1)),
            (int_bitop(tag_smallint(0), f, BitOp::Or), tag_smallint(0)),
            (int_lshift(t, tag_smallint(1)), tag_smallint(2)),
            (int_lshift(t, f), tag_smallint(1)),
            (int_rshift(t, f), tag_smallint(1)),
            (int_rshift(f, t), tag_smallint(0)),
        ] {
            assert_eq!(word, expected);
        }
    }

    /// A negative count raises before anything else, whether it is inline,
    /// a heap bigint, or a non-canonical heap word.
    #[test]
    fn a_negative_count_raises_value_error() {
        pycc_rt_exception_clear();
        // Each case runs only when checked: a second raise before the first
        // is read would replace it.
        let cases: [(Case, &str); 5] = [
            (|| int_lshift(tag_smallint(1), tag_smallint(-1)), "1 << -1"),
            (|| int_rshift(tag_smallint(1), tag_smallint(-1)), "1 >> -1"),
            (
                || int_lshift(tag_smallint(0), big(-(1i128 << 70))),
                "0 << -(1<<70)",
            ),
            (
                || int_rshift(big(1i128 << 70), big(-(1i128 << 70))),
                "(1<<70) >> -(1<<70)",
            ),
            (
                || int_rshift(tag_smallint(1), non_canonical(-2)),
                "1 >> non-canonical -2",
            ),
        ];
        for (case, what) in cases {
            assert_eq!(case(), tag_smallint(0), "{what} must return the sentinel");
            assert_eq!(
                take_pending(),
                Some((
                    i32::from(EXCEPTION_TYPE_VALUE_ERROR),
                    "negative shift count".to_string()
                )),
                "{what}"
            );
        }
    }

    /// A count past the inline range: `>>` gives `0` or `-1`, `<<` of zero
    /// gives `0`, and `<<` of anything else raises CPython's
    /// `OverflowError`.
    #[test]
    fn a_huge_count_follows_cpython() {
        pycc_rt_exception_clear();
        let huge = || big(1i128 << 70);
        assert_value(int_rshift(tag_smallint(5), huge()), 0, "5 >> (1<<70)");
        assert_value(int_rshift(tag_smallint(-5), huge()), -1, "-5 >> (1<<70)");
        assert_value(
            int_rshift(big(-(1i128 << 70)), huge()),
            -1,
            "-(1<<70) >> (1<<70)",
        );
        assert_value(int_lshift(tag_smallint(0), huge()), 0, "0 << (1<<70)");
        assert_value(
            int_lshift(non_canonical(0), huge()),
            0,
            "non-canonical 0 << (1<<70)",
        );
        // `1 << 62` is past the inline range too, though it fits `u64`.
        assert_value(
            int_rshift(tag_smallint(9), big(1i128 << 62)),
            0,
            "9 >> (1<<62)",
        );
        let cases: [(Case, &str); 2] = [
            (
                || int_lshift(tag_smallint(1), big(1i128 << 70)),
                "1 << (1<<70)",
            ),
            (
                || int_lshift(big(-(1i128 << 70)), big(1i128 << 62)),
                "-(1<<70) << (1<<62)",
            ),
        ];
        for (case, what) in cases {
            assert_eq!(case(), tag_smallint(0), "{what} must return the sentinel");
            assert_eq!(
                take_pending(),
                Some((
                    i32::from(EXCEPTION_TYPE_OVERFLOW_ERROR),
                    "too many digits in integer".to_string()
                )),
                "{what}"
            );
        }
    }

    /// Counts and bases are read by value: a heap word holding a small
    /// count shifts by that count, and a heap zero base is zero.
    #[test]
    fn non_canonical_operands_are_read_by_value() {
        pycc_rt_exception_clear();
        assert_eq!(decode_count(non_canonical(5)), Count::Small(5));
        assert_eq!(decode_count(non_canonical(0)), Count::Small(0));
        assert_eq!(decode_count(big(1i128 << 62)), Count::Huge);
        assert_eq!(
            decode_count(big((1i128 << 62) - 1)),
            Count::Small((1 << 62) - 1)
        );
        assert_value(
            int_lshift(tag_smallint(3), non_canonical(5)),
            96,
            "3 << nc 5",
        );
        assert_value(
            int_rshift(tag_smallint(96), non_canonical(5)),
            3,
            "96 >> nc 5",
        );
        assert_value(
            int_lshift(tag_smallint(3), non_canonical(0)),
            3,
            "3 << nc 0",
        );
        assert_value(
            int_rshift(non_canonical(-96), tag_smallint(5)),
            -3,
            "nc -96 >> 5",
        );
        assert_value(
            int_rshift(non_canonical(0), tag_smallint(5)),
            0,
            "nc 0 >> 5",
        );
        assert_value(
            int_lshift(non_canonical(0), tag_smallint(5)),
            0,
            "nc 0 << 5",
        );
        assert_value(
            int_bitop(non_canonical(12), non_canonical(-3), BitOp::And),
            12,
            "nc 12 & nc -3",
        );
        let word = int_bitop(non_canonical(12), non_canonical(10), BitOp::Or);
        assert!(is_smallint(word), "a small result is canonicalized");
        assert_value(word, 14, "nc 12 | nc 10");
    }

    /// The allocation-failure arm: a count just inside the inline range asks
    /// for about `2**59` bytes, which no allocator grants, so `<<` raises
    /// `OverflowError` rather than aborting. A unit test only: CPython
    /// raises `MemoryError` here (D-244's recorded deviation).
    #[test]
    fn a_refused_allocation_raises_overflow_error() {
        pycc_rt_exception_clear();
        assert_eq!(shift_magnitude_left(&[1], INLINE_MAX), None);
        let word = int_lshift(tag_smallint(1), tag_smallint(INLINE_MAX as i64));
        assert_eq!(word, tag_smallint(0));
        assert_eq!(
            take_pending(),
            Some((
                i32::from(EXCEPTION_TYPE_OVERFLOW_ERROR),
                "too many digits in integer".to_string()
            ))
        );
    }

    /// D-181: the sibling of `lib.rs`'s
    /// `an_int_operation_never_returns_an_operand_s_own_word`, for these
    /// five operations over heap operands, including the identity-shaped
    /// cases (`x << 0`, `x >> 0`, `x & x`, `x | 0`, `x ^ 0`) an identity
    /// fast path would short-circuit.
    #[test]
    fn a_bitwise_operation_never_returns_an_operand_s_own_word() {
        pycc_rt_exception_clear();
        let big_word = big(1i128 << 70);
        let zero = tag_smallint(0);
        for (name, result) in [
            ("lshift-zero", pycc_rt_int_lshift(big_word, zero)),
            ("rshift-zero", pycc_rt_int_rshift(big_word, zero)),
            ("and-self", pycc_rt_int_and(big_word, big_word)),
            ("or-zero", pycc_rt_int_or(big_word, zero)),
            ("or-zero-left", pycc_rt_int_or(zero, big_word)),
            ("xor-zero", pycc_rt_int_xor(big_word, zero)),
        ] {
            assert_ne!(result, big_word, "`{name}` returned the operand's own word");
            assert_eq!(value_of(result), 1i128 << 70, "{name}");
            bigint_release(result);
        }
        bigint_release(big_word);
    }
}
