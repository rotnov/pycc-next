//! `int` arithmetic, comparison, conversion and formatting over D-061/D-141's
//! one-word encoding, with arbitrary precision.
//!
//! Every operation has two halves. When both operands are inline (smallints
//! or bool markers) it works on plain `i64`s and promotes only an
//! overflowing result. Otherwise it decodes both operands to a
//! `num_bigint::BigInt` ([`decode_big`]), computes exactly, and re-encodes
//! with [`encode_big`]. `encode_big` is canonical: a value that fits the
//! 63-bit tagged range is *always* returned as a smallint, so a heap word
//! produced here never holds a small value. `pycc_codegen` emits inline fast
//! paths for the common operators and calls these functions only for
//! non-smallint operands or on overflow, so they must accept every encoded
//! kind, not only bigints.
//!
//! # Ownership ([D-181])
//! No operation returns an operand's own encoded word: every heap result is
//! a fresh `BigIntObj` built by `encode_big`. `pycc_codegen` releases both
//! operands right after the call, so an aliasing fast path (`a + 0 -> a`)
//! would be a use-after-free.
//!
//! # Errors
//! Failures are D-173 pending exceptions with CPython 3.13's messages; each
//! function then returns a type-valid sentinel (`tag_smallint(0)`, `0`,
//! `0.0`) that generated code never reads, because it checks the pending
//! flag first. Each `extern "C"` symbol is a thin wrapper over a private
//! ordinary-ABI function: a panic unwinding past a plain `extern "C"` fn
//! aborts, so the crate's own tests call the private half.
//!
//! [D-181]: ../../../docs/decisions/D-181-release-a-heap-bigint-s-birth-reference-at-every.md

use crate::exception::raise_builtin;
use crate::int_encoding::*;
use crate::{
    EXCEPTION_TYPE_OVERFLOW_ERROR, EXCEPTION_TYPE_RUNTIME_ERROR, EXCEPTION_TYPE_VALUE_ERROR,
    EXCEPTION_TYPE_ZERO_DIV_ERROR, PyStrObj, new_pystr,
};
use num_bigint::{BigInt, Sign};
use num_integer::Integer;
use num_traits::{FromPrimitive, Signed, ToPrimitive, Zero};

/// CPython's default `sys.get_int_max_str_digits()`: `str()` of an `int`
/// with more decimal digits raises `ValueError`.
const MAX_STR_DIGITS: usize = 4300;

/// The largest result, in bits, `**` attempts to build. CPython has no such
/// bound and raises `MemoryError` when allocation fails; pycc has no
/// `MemoryError` class, so it raises the same `OverflowError` `<<` does for
/// an unallocatable result (D-244's 2026-09-23 amendment).
const MAX_POW_BITS: u64 = 1 << 36;

/// The exact value of any valid encoded word.
pub(crate) fn decode_big(word: i64) -> BigInt {
    match inline_int_value(word) {
        Some(v) => BigInt::from(v),
        None => {
            let b = unsafe { bigint_ref(word) };
            let sign = if b.negative { Sign::Minus } else { Sign::Plus };
            BigInt::from_slice(sign, &b.limbs)
        }
    }
}

/// The canonical encoded word for `value`: a smallint when it fits the
/// tagged range, otherwise a fresh heap bigint holding one reference.
pub(crate) fn encode_big(value: BigInt) -> i64 {
    if let Some(word) = value.to_i64().and_then(fits_smallint) {
        return word;
    }
    let (sign, digits) = value.to_u32_digits();
    tag_bigint(BigIntObj::new(sign == Sign::Minus, digits))
}

fn encode_i128(value: i128) -> i64 {
    i64::try_from(value)
        .ok()
        .and_then(fits_smallint)
        .unwrap_or_else(|| encode_big(BigInt::from(value)))
}

/// Decodes `encoded` to its inline value, or raises `OverflowError` naming
/// `context` and yields `None` for a heap bigint. Used only by the runtime
/// boundaries that still require a machine-word value (a `set[int]` element,
/// a buffer size); arithmetic accepts every value.
pub(crate) fn decode_inline_or_raise(encoded: i64, context: &str) -> Option<i64> {
    let value = inline_int_value(encoded);
    if value.is_none() {
        raise_builtin(
            EXCEPTION_TYPE_OVERFLOW_ERROR,
            "OverflowError",
            &format!("{context} a bigint-valued `int` is not supported yet"),
        );
    }
    value
}

fn raise_zero_division(message: &'static str) {
    raise_builtin(EXCEPTION_TYPE_ZERO_DIV_ERROR, "ZeroDivisionError", message);
}

pub(crate) fn int_add(a: i64, b: i64) -> i64 {
    if let (Some(x), Some(y)) = (inline_int_value(a), inline_int_value(b)) {
        return encode_i128(i128::from(x) + i128::from(y));
    }
    encode_big(decode_big(a) + decode_big(b))
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_int_add(a: i64, b: i64) -> i64 {
    int_add(a, b)
}

pub(crate) fn int_sub(a: i64, b: i64) -> i64 {
    if let (Some(x), Some(y)) = (inline_int_value(a), inline_int_value(b)) {
        return encode_i128(i128::from(x) - i128::from(y));
    }
    encode_big(decode_big(a) - decode_big(b))
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_int_sub(a: i64, b: i64) -> i64 {
    int_sub(a, b)
}

pub(crate) fn int_mul(a: i64, b: i64) -> i64 {
    if let (Some(x), Some(y)) = (inline_int_value(a), inline_int_value(b)) {
        // Two 63-bit values multiply exactly in `i128`.
        return encode_i128(i128::from(x) * i128::from(y));
    }
    encode_big(decode_big(a) * decode_big(b))
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_int_mul(a: i64, b: i64) -> i64 {
    int_mul(a, b)
}

/// Python's `//`: the quotient rounded toward negative infinity.
pub(crate) fn int_floordiv(a: i64, b: i64) -> i64 {
    if let (Some(x), Some(y)) = (inline_int_value(a), inline_int_value(b)) {
        if y == 0 {
            raise_zero_division("integer division or modulo by zero");
            return tag_smallint(0);
        }
        // Inline values are within +/-2**62, so no `i64` division traps;
        // only `-2**62 // -1` leaves the tagged range, which `encode_i128`
        // promotes.
        return encode_i128(i128::from(Integer::div_floor(&x, &y)));
    }
    let divisor = decode_big(b);
    if divisor.is_zero() {
        raise_zero_division("integer division or modulo by zero");
        return tag_smallint(0);
    }
    encode_big(decode_big(a).div_floor(&divisor))
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_int_floordiv(a: i64, b: i64) -> i64 {
    int_floordiv(a, b)
}

/// Python's `%`: the remainder takes the divisor's sign.
pub(crate) fn int_floormod(a: i64, b: i64) -> i64 {
    if let (Some(x), Some(y)) = (inline_int_value(a), inline_int_value(b)) {
        if y == 0 {
            raise_zero_division("integer modulo by zero");
            return tag_smallint(0);
        }
        return tag_smallint(x.mod_floor(&y));
    }
    let divisor = decode_big(b);
    if divisor.is_zero() {
        raise_zero_division("integer modulo by zero");
        return tag_smallint(0);
    }
    encode_big(decode_big(a).mod_floor(&divisor))
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_int_floormod(a: i64, b: i64) -> i64 {
    int_floormod(a, b)
}

/// `divmod`'s remainder: `a % b`, or `0` for a zero divisor, raising
/// nothing -- `divmod` reports the zero divisor through its quotient, with
/// `//`'s message.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_int_mod_noraise(a: i64, b: i64) -> i64 {
    if decode_big(b).is_zero() {
        return tag_smallint(0);
    }
    int_floormod(a, b)
}

fn raise_too_many_digits() -> i64 {
    raise_builtin(
        EXCEPTION_TYPE_OVERFLOW_ERROR,
        "OverflowError",
        "too many digits in integer",
    );
    tag_smallint(0)
}

/// `base ** exp` for a non-negative `exp`.
///
/// A negative exponent raises `RuntimeError`: CPython returns a `float`
/// there, but `pycc_types` types `int ** int` as `int` (the pre-existing
/// simplification recorded in D-244's 2026-09-13 amendment and in
/// `docs/DEVIATIONS.md`).
pub(crate) fn int_pow(base: i64, exp: i64) -> i64 {
    let exponent = decode_big(exp);
    if exponent.is_negative() {
        raise_builtin(
            EXCEPTION_TYPE_RUNTIME_ERROR,
            "RuntimeError",
            "negative exponent for `int ** int` is not supported \
             (the real result would need to be `float`, matching CPython's \
             own `int ** int` rule -- a pre-existing pycc_types simplification: \
             pycc_types::numeric_result_type always types `**` as \
             `int`-returning)",
        );
        return tag_smallint(0);
    }
    let base = decode_big(base);
    // 0, 1 and -1 stay small for any exponent, however large.
    match base.to_i64() {
        Some(0) => return tag_smallint(i64::from(exponent.is_zero())),
        Some(1) => return tag_smallint(1),
        Some(-1) => return tag_smallint(if exponent.is_odd() { -1 } else { 1 }),
        _ => {}
    }
    let Some(exponent) = exponent.to_u64() else {
        return raise_too_many_digits();
    };
    if base.bits().saturating_mul(exponent) > MAX_POW_BITS {
        return raise_too_many_digits();
    }
    let Ok(exponent) = u32::try_from(exponent) else {
        return raise_too_many_digits();
    };
    encode_big(num_traits::pow(base, exponent as usize))
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_int_pow(base: i64, exp: i64) -> i64 {
    int_pow(base, exp)
}

/// `abs(a)`.
pub(crate) fn int_abs(a: i64) -> i64 {
    if let Some(x) = inline_int_value(a) {
        return encode_i128(i128::from(x).abs());
    }
    encode_big(decode_big(a).abs())
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_int_abs(a: i64) -> i64 {
    int_abs(a)
}

pub(crate) fn int_cmp(a: i64, b: i64) -> i32 {
    let ordering = encoded_int_cmp(a, b);
    ordering as i32
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_int_cmp(a: i64, b: i64) -> i32 {
    int_cmp(a, b)
}

/// `float(a)`, correctly rounded; `OverflowError` beyond `f64`'s range.
pub(crate) fn int_to_float(tagged: i64) -> f64 {
    if let Some(v) = inline_int_value(tagged) {
        return v as f64;
    }
    match decode_big(tagged).to_f64() {
        Some(f) if f.is_finite() => f,
        _ => {
            raise_builtin(
                EXCEPTION_TYPE_OVERFLOW_ERROR,
                "OverflowError",
                "int too large to convert to float",
            );
            0.0
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_int_to_float(tagged: i64) -> f64 {
    int_to_float(tagged)
}

/// `x * 2**exp` without intermediate overflow or double rounding: each step
/// scales by at most `2**1000`, and callers only pass values whose final
/// result is exactly representable.
fn ldexp(mut x: f64, mut exp: i64) -> f64 {
    let up = f64::from_bits(((1023 + 1000) as u64) << 52);
    let down = f64::from_bits(((1023 - 1000) as u64) << 52);
    while exp > 1000 {
        x *= up;
        exp -= 1000;
    }
    while exp < -1000 {
        x *= down;
        exp += 1000;
    }
    x * f64::from_bits(((1023 + exp) as u64) << 52)
}

/// `a / b` for two ints: the correctly rounded quotient, like CPython's
/// `long_true_divide`, including operands far beyond `f64`'s range.
pub(crate) fn int_truediv(a: i64, b: i64) -> f64 {
    const EXACT: i64 = 1 << 53;
    if let (Some(x), Some(y)) = (inline_int_value(a), inline_int_value(b))
        && (-EXACT..=EXACT).contains(&x)
        && (-EXACT..=EXACT).contains(&y)
        && y != 0
    {
        // Both convert exactly, so one IEEE division rounds correctly.
        return x as f64 / y as f64;
    }
    let numerator = decode_big(a);
    let denominator = decode_big(b);
    if denominator.is_zero() {
        raise_zero_division("division by zero");
        return 0.0;
    }
    let negative = numerator.is_negative() != denominator.is_negative();
    let n = numerator.magnitude();
    let d = denominator.magnitude();
    if n.is_zero() {
        return if negative { -0.0 } else { 0.0 };
    }
    // The quotient lies in [2**(e-1), 2**(e+1)) for e = bits(n) - bits(d).
    // Pick the weight `lsb` of the last computed bit so the integer
    // quotient carries 55-56 significant bits (two or more guard bits), or
    // exactly the subnormal grid (2**-1074) plus two guard bits.
    let e = n.bits() as i64 - d.bits() as i64;
    if e > 1025 {
        raise_builtin(
            EXCEPTION_TYPE_OVERFLOW_ERROR,
            "OverflowError",
            "integer division result too large for a float",
        );
        return 0.0;
    }
    let lsb = (e - 55).max(-1074 - 2);
    let (scaled_n, scaled_d) = if lsb < 0 {
        (n << (-lsb) as usize, d.clone())
    } else {
        (n.clone(), d << lsb as usize)
    };
    let (q, r) = scaled_n.div_rem(&scaled_d);
    let sticky = !r.is_zero();
    let mut q = q.to_u64().expect("the scaled quotient has at most 57 bits");
    let magnitude = if q >= 1 << 54 || lsb > -1076 {
        // Normal range: round-to-odd keeps the one rounding in the `u64 ->
        // f64` conversion correct, and the scaling below is exact unless it
        // overflows.
        q |= u64::from(sticky);
        ldexp(q as f64, lsb)
    } else {
        // Subnormal: round to the 2**-1074 grid by hand (half to even), so
        // the scaling cannot round a second time.
        let low = q & 3;
        q >>= 2;
        if low > 2 || (low == 2 && (sticky || q & 1 == 1)) {
            q += 1;
        }
        ldexp(q as f64, -1074)
    };
    if magnitude.is_infinite() {
        raise_builtin(
            EXCEPTION_TYPE_OVERFLOW_ERROR,
            "OverflowError",
            "integer division result too large for a float",
        );
        return 0.0;
    }
    if negative { -magnitude } else { magnitude }
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_int_truediv(a: i64, b: i64) -> f64 {
    int_truediv(a, b)
}

/// Orders an int against a float exactly, as CPython does (no rounding of
/// the int): `-1`, `0` or `1`, or `2` when `f` is NaN (unordered).
pub(crate) fn int_float_cmp(a: i64, f: f64) -> i32 {
    if f.is_nan() {
        return 2;
    }
    if f.is_infinite() {
        return if f > 0.0 { -1 } else { 1 };
    }
    const EXACT: i64 = 1 << 53;
    if let Some(x) = inline_int_value(a)
        && (-EXACT..=EXACT).contains(&x)
    {
        let x = x as f64;
        return if x < f {
            -1
        } else if x > f {
            1
        } else {
            0
        };
    }
    let floor = f.floor();
    let whole = BigInt::from_f64(floor).expect("a finite float has an exact integer floor");
    match decode_big(a).cmp(&whole) {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Greater => 1,
        std::cmp::Ordering::Equal => {
            if f > floor {
                -1
            } else {
                0
            }
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_int_float_cmp(a: i64, f: f64) -> i32 {
    int_float_cmp(a, f)
}

/// `int(f)`: truncation toward zero, exact for any finite float.
pub(crate) fn int_from_float(f: f64) -> i64 {
    if f.is_nan() {
        raise_builtin(
            EXCEPTION_TYPE_VALUE_ERROR,
            "ValueError",
            "cannot convert float NaN to integer",
        );
        return tag_smallint(0);
    }
    if f.is_infinite() {
        raise_builtin(
            EXCEPTION_TYPE_OVERFLOW_ERROR,
            "OverflowError",
            "cannot convert float infinity to integer",
        );
        return tag_smallint(0);
    }
    encode_big(BigInt::from_f64(f.trunc()).expect("a finite float converts exactly"))
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_int_from_float(f: f64) -> i64 {
    int_from_float(f)
}

/// Materializes an `int` literal (or `enum` discriminant) that does not fit
/// the tagged range: `pycc_codegen` folds the tagged case at compile time,
/// but this function accepts both so the ABI contract is total (D-178).
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_int_from_i64(v: i64) -> i64 {
    encode_i128(i128::from(v))
}

/// [`int_untag_checked`]'s `use_kind`: the word is stored as a container
/// element.
pub const UNTAG_ELEMENT: i32 = 0;
/// [`int_untag_checked`]'s `use_kind`: the word indexes a sequence.
pub const UNTAG_INDEX: i32 = 1;
/// [`int_untag_checked`]'s `use_kind`: the word is a size, count or slice
/// bound.
pub const UNTAG_SIZE: i32 = 2;

/// D-141 checked decoder for an int-compatible ABI word used as a machine
/// word. Smallints decode to their value and the bool markers to `0`/`1`. A
/// heap bigint raises (D-173) and yields the sentinel `0`, which generated
/// code never consumes: it checks the pending flag right after this call.
/// An index raises CPython's own `IndexError`; a size or count CPython's
/// `OverflowError`; a container element, which CPython would store, the
/// recorded `OverflowError` gap (`docs/DEVIATIONS.md`). Raising rather than
/// aborting matters for an `--ext` module, whose callers pass ints of any
/// size: an abort would kill the host interpreter.
pub(crate) fn int_untag_checked(tagged: i64, use_kind: i32) -> i64 {
    if let Some(value) = inline_int_value(tagged) {
        return value;
    }
    let (tag, name, message) = match use_kind {
        UNTAG_INDEX => (
            crate::EXCEPTION_TYPE_INDEX_ERROR,
            "IndexError",
            "cannot fit 'int' into an index-sized integer",
        ),
        UNTAG_SIZE => (
            EXCEPTION_TYPE_OVERFLOW_ERROR,
            "OverflowError",
            "cannot fit 'int' into an index-sized integer",
        ),
        _ => (
            EXCEPTION_TYPE_OVERFLOW_ERROR,
            "OverflowError",
            "storing a bigint-valued `int` in a container is not supported yet",
        ),
    };
    raise_builtin(tag, name, message);
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_int_untag_checked(tagged: i64, use_kind: i32) -> i64 {
    int_untag_checked(tagged, use_kind)
}

/// `int`'s truthiness: canonical encoding means only a smallint can be zero,
/// but a heap word is still inspected rather than assumed non-zero.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_int_truthy(tagged: i64) -> i8 {
    if let Some(value) = inline_int_value(tagged) {
        return i8::from(value != 0);
    }
    i8::from(!decode_big(tagged).is_zero())
}

/// The decimal text of `tagged` (`True`/`False` for a bool marker), or
/// `None` after raising CPython's `ValueError` for more than
/// [`MAX_STR_DIGITS`] digits.
pub(crate) fn int_decimal(tagged: i64) -> Option<String> {
    match classify_encoded_int(tagged) {
        EncodedIntKind::SmallInt => Some(untag_smallint(tagged).to_string()),
        EncodedIntKind::BoolFalse => Some("False".to_string()),
        EncodedIntKind::BoolTrue => Some("True".to_string()),
        EncodedIntKind::BigInt => {
            let value = decode_big(tagged);
            // A cheap lower bound on the digit count first, so a huge value
            // is refused before the quadratic conversion runs.
            let min_digits = ((value.bits() - 1) as f64 * std::f64::consts::LOG10_2) as usize + 1;
            let text = if min_digits > MAX_STR_DIGITS + 1 {
                None
            } else {
                Some(value.to_string())
            };
            match text {
                Some(text) if text.trim_start_matches('-').len() <= MAX_STR_DIGITS => Some(text),
                _ => {
                    raise_builtin(
                        EXCEPTION_TYPE_VALUE_ERROR,
                        "ValueError",
                        "Exceeds the limit (4300 digits) for integer string conversion; \
                         use sys.set_int_max_str_digits() to increase the limit",
                    );
                    None
                }
            }
        }
    }
}

pub(crate) fn int_to_str(tagged: i64) -> *mut PyStrObj {
    new_pystr(int_decimal(tagged).unwrap_or_default().as_bytes())
}

/// `str(n)` / f-string interpolation of an `int`.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_int_to_str(tagged: i64) -> *mut PyStrObj {
    int_to_str(tagged)
}

pub(crate) fn format_i64_line(value: i64) -> String {
    format!("{value}\n")
}

/// `print(n)` of an `int`. Prints nothing when the conversion raises.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_int_print(tagged: i64) {
    if let Some(text) = int_decimal(tagged) {
        crate::io::out(format!("{text}\n").as_bytes());
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_print_i64(value: i64) {
    crate::io::out(format_i64_line(value).as_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exception::{pycc_rt_exception_active, pycc_rt_exception_clear};
    use crate::tests::pending_tag_and_message;

    fn big(text: &str) -> i64 {
        encode_big(text.parse::<BigInt>().unwrap())
    }

    fn text(word: i64) -> String {
        int_decimal(word).unwrap()
    }

    fn assert_raises(tag: u8, message: &str, run: impl FnOnce()) {
        pycc_rt_exception_clear();
        run();
        let (actual_tag, actual) = pending_tag_and_message();
        assert_eq!((actual_tag, actual.as_str()), (tag, message));
        pycc_rt_exception_clear();
    }

    #[test]
    fn encoding_is_canonical_so_small_values_never_live_on_the_heap() {
        let two_62 = big("4611686018427387904");
        assert_eq!(classify_encoded_int(two_62), EncodedIntKind::BigInt);
        assert_eq!(int_sub(two_62, two_62), tag_smallint(0));
        assert_eq!(
            int_sub(two_62, tag_smallint(1)),
            tag_smallint((1 << 62) - 1)
        );
        assert_eq!(
            text(int_add(tag_smallint((1 << 62) - 1), tag_smallint(1))),
            "4611686018427387904"
        );
        assert_eq!(
            text(int_sub(tag_smallint(-(1 << 62)), tag_smallint(1))),
            "-4611686018427387905"
        );
        assert_eq!(text(pycc_rt_int_from_i64(i64::MIN)), "-9223372036854775808");
        bigint_release(two_62);
    }

    #[test]
    fn arithmetic_matches_cpython_for_mixed_magnitudes() {
        let p = big("1267650600228229401496703205376"); // 2**100
        assert_eq!(
            text(int_mul(p, p)),
            "1606938044258990275541962092341162602522202993782792835301376"
        );
        assert_eq!(
            text(int_mul(p, tag_smallint(-3))),
            "-3802951800684688204490109616128"
        );
        assert_eq!(
            text(int_floordiv(p, tag_smallint(3))),
            "422550200076076467165567735125"
        );
        assert_eq!(
            text(int_floordiv(int_sub(tag_smallint(0), p), tag_smallint(3))),
            "-422550200076076467165567735126"
        );
        assert_eq!(
            text(int_floormod(int_sub(tag_smallint(0), p), tag_smallint(7))),
            "5"
        );
        assert_eq!(text(int_floormod(p, tag_smallint(-7))), "-5");
        assert_eq!(int_floordiv(tag_smallint(7), p), tag_smallint(0));
        assert_eq!(int_floordiv(tag_smallint(-7), p), tag_smallint(-1));
        assert_eq!(text(int_pow(tag_smallint(2), tag_smallint(100))), text(p));
        assert_eq!(text(int_abs(int_sub(tag_smallint(0), p))), text(p));
        assert_eq!(
            text(int_abs(tag_smallint(-(1 << 62)))),
            "4611686018427387904"
        );
        assert_eq!(int_cmp(p, tag_smallint(1)), 1);
        assert_eq!(int_cmp(int_sub(tag_smallint(0), p), tag_smallint(1)), -1);
        assert_eq!(int_cmp(p, int_mul(tag_smallint(1), p)), 0);
        assert_eq!(pycc_rt_int_truthy(p), 1);
    }

    #[test]
    fn floor_division_and_modulo_follow_the_divisor_sign() {
        for (a, b, q, r) in [
            (-7, 2, -4, 1),
            (7, -2, -4, -1),
            (-7, -2, 3, -1),
            (7, 2, 3, 1),
        ] {
            assert_eq!(
                int_floordiv(tag_smallint(a), tag_smallint(b)),
                tag_smallint(q)
            );
            assert_eq!(
                int_floormod(tag_smallint(a), tag_smallint(b)),
                tag_smallint(r)
            );
        }
        let min = i64::MIN >> 1;
        assert_eq!(
            text(int_floordiv(tag_smallint(min), tag_smallint(-1))),
            "4611686018427387904"
        );
    }

    #[test]
    fn zero_divisors_raise_cpython_messages() {
        let p = big("1267650600228229401496703205376");
        assert_raises(
            EXCEPTION_TYPE_ZERO_DIV_ERROR,
            "integer division or modulo by zero",
            || {
                assert_eq!(
                    int_floordiv(tag_smallint(1), tag_smallint(0)),
                    tag_smallint(0)
                );
            },
        );
        assert_raises(
            EXCEPTION_TYPE_ZERO_DIV_ERROR,
            "integer division or modulo by zero",
            || {
                int_floordiv(p, tag_smallint(0));
            },
        );
        assert_raises(
            EXCEPTION_TYPE_ZERO_DIV_ERROR,
            "integer modulo by zero",
            || {
                int_floormod(tag_smallint(1), BOOL_FALSE_MARKER);
            },
        );
        assert_raises(
            EXCEPTION_TYPE_ZERO_DIV_ERROR,
            "integer modulo by zero",
            || {
                int_floormod(p, tag_smallint(0));
            },
        );
        assert_raises(EXCEPTION_TYPE_ZERO_DIV_ERROR, "division by zero", || {
            int_truediv(p, tag_smallint(0));
        });
        assert_raises(EXCEPTION_TYPE_ZERO_DIV_ERROR, "division by zero", || {
            int_truediv(tag_smallint(1), tag_smallint(0));
        });
    }

    #[test]
    fn pow_handles_trivial_bases_huge_exponents_and_refusals() {
        let huge = big("100000000000000000000000000000");
        assert_eq!(int_pow(tag_smallint(1), huge), tag_smallint(1));
        assert_eq!(int_pow(tag_smallint(-1), huge), tag_smallint(1));
        assert_eq!(
            int_pow(tag_smallint(-1), int_add(huge, tag_smallint(1))),
            tag_smallint(-1)
        );
        assert_eq!(int_pow(tag_smallint(0), huge), tag_smallint(0));
        assert_eq!(int_pow(tag_smallint(0), tag_smallint(0)), tag_smallint(1));
        assert_eq!(int_pow(BOOL_TRUE_MARKER, tag_smallint(5)), tag_smallint(1));
        assert_raises(
            EXCEPTION_TYPE_OVERFLOW_ERROR,
            "too many digits in integer",
            || {
                int_pow(tag_smallint(2), huge);
            },
        );
        assert_raises(
            EXCEPTION_TYPE_OVERFLOW_ERROR,
            "too many digits in integer",
            || {
                int_pow(tag_smallint(3), tag_smallint(1 << 40));
            },
        );
        pycc_rt_exception_clear();
        int_pow(tag_smallint(2), tag_smallint(-1));
        let (tag, message) = pending_tag_and_message();
        assert_eq!(tag, EXCEPTION_TYPE_RUNTIME_ERROR);
        assert!(message.contains("negative exponent"), "{message}");
        pycc_rt_exception_clear();
    }

    #[test]
    fn int_to_float_rounds_correctly_and_overflows_like_cpython() {
        assert_eq!(int_to_float(big("9007199254740993")), 9007199254740992.0);
        assert_eq!(int_to_float(big("9007199254740995")), 9007199254740996.0);
        assert_eq!(
            int_to_float(big("1267650600228229401496703205376")),
            1.2676506002282294e30
        );
        let too_big = int_pow(tag_smallint(10), tag_smallint(400));
        assert_raises(
            EXCEPTION_TYPE_OVERFLOW_ERROR,
            "int too large to convert to float",
            || {
                assert_eq!(int_to_float(too_big), 0.0);
            },
        );
        assert_eq!(int_to_float(BOOL_TRUE_MARKER), 1.0);
    }

    #[test]
    fn true_division_is_correctly_rounded_at_every_scale() {
        let ten = |n: i64| int_pow(tag_smallint(10), tag_smallint(n));
        assert_eq!(int_truediv(ten(400), ten(399)), 10.0);
        assert_eq!(int_truediv(tag_smallint(1), ten(400)), 0.0);
        assert_eq!(
            int_truediv(int_sub(tag_smallint(0), tag_smallint(1)), ten(400)),
            -0.0
        );
        assert!(
            int_truediv(int_sub(tag_smallint(0), tag_smallint(1)), ten(400)).is_sign_negative()
        );
        assert_eq!(int_truediv(tag_smallint(0), tag_smallint(-5)), -0.0);
        assert_eq!(
            int_truediv(big("1267650600228229401496703205376"), tag_smallint(3)),
            4.2255020007607644e29
        );
        assert_eq!(int_truediv(tag_smallint(1), tag_smallint(3)), 1.0 / 3.0);
        assert_eq!(int_truediv(tag_smallint(1), ten(320)), 1e-320);
        assert_eq!(int_truediv(tag_smallint(3), ten(324)), 5e-324);
        assert_eq!(int_truediv(tag_smallint(2), ten(324)), 0.0);
        assert_eq!(int_truediv(ten(30), ten(20)), 1e10);
        assert_eq!(
            int_truediv(big("9007199254740993"), tag_smallint(1)),
            9007199254740992.0
        );
        assert_raises(
            EXCEPTION_TYPE_OVERFLOW_ERROR,
            "integer division result too large for a float",
            || {
                int_truediv(ten(400), tag_smallint(3));
            },
        );
    }

    #[test]
    fn int_float_comparison_is_exact() {
        let two_53_plus_1 = big("9007199254740993");
        assert_eq!(int_float_cmp(two_53_plus_1, 9007199254740992.0), 1);
        assert_eq!(int_float_cmp(tag_smallint(3), 3.0), 0);
        assert_eq!(int_float_cmp(tag_smallint(3), 3.5), -1);
        assert_eq!(int_float_cmp(tag_smallint(-3), -3.5), 1);
        assert_eq!(int_float_cmp(tag_smallint(3), f64::NAN), 2);
        assert_eq!(int_float_cmp(tag_smallint(3), f64::INFINITY), -1);
        assert_eq!(int_float_cmp(tag_smallint(3), f64::NEG_INFINITY), 1);
        let ten_400 = int_pow(tag_smallint(10), tag_smallint(400));
        assert_eq!(int_float_cmp(ten_400, 1e308), 1);
        assert_eq!(int_float_cmp(int_sub(tag_smallint(0), ten_400), -1e308), -1);
        assert_eq!(
            int_float_cmp(big("-9007199254740993"), -9007199254740994.0),
            1
        );
        assert_eq!(
            int_float_cmp(big("-9007199254740993"), -9007199254740992.0),
            -1
        );
    }

    #[test]
    fn int_from_float_truncates_and_rejects_non_finite_values() {
        assert_eq!(int_from_float(-2.7), tag_smallint(-2));
        assert_eq!(text(int_from_float(1e20)), "100000000000000000000");
        assert_eq!(text(int_from_float(-1e300)).len(), 302);
        assert_raises(
            EXCEPTION_TYPE_VALUE_ERROR,
            "cannot convert float NaN to integer",
            || {
                int_from_float(f64::NAN);
            },
        );
        assert_raises(
            EXCEPTION_TYPE_OVERFLOW_ERROR,
            "cannot convert float infinity to integer",
            || {
                int_from_float(f64::NEG_INFINITY);
            },
        );
    }

    #[test]
    fn str_of_an_int_honours_cpython_s_digit_limit() {
        assert_eq!(text(BOOL_TRUE_MARKER), "True");
        assert_eq!(text(tag_smallint(-42)), "-42");
        let at_limit = int_sub(
            int_pow(tag_smallint(10), tag_smallint(4300)),
            tag_smallint(1),
        );
        assert_eq!(text(at_limit).len(), 4300);
        let over = int_pow(tag_smallint(10), tag_smallint(4300));
        let message = "Exceeds the limit (4300 digits) for integer string conversion; \
                       use sys.set_int_max_str_digits() to increase the limit";
        assert_raises(EXCEPTION_TYPE_VALUE_ERROR, message, || {
            assert!(int_decimal(over).is_none());
        });
        let far_over = int_pow(tag_smallint(10), tag_smallint(100_000));
        assert_raises(EXCEPTION_TYPE_VALUE_ERROR, message, || {
            let s = int_to_str(far_over);
            unsafe { crate::pycc_rt_str_decref(s) };
        });
        pycc_rt_int_print(tag_smallint(7));
        pycc_rt_int_print(at_limit);
        assert_eq!(pycc_rt_exception_active(), 0);
    }

    #[test]
    fn untag_checked_raises_by_use_instead_of_aborting() {
        let p = big("1267650600228229401496703205376");
        assert_eq!(int_untag_checked(tag_smallint(-9), UNTAG_INDEX), -9);
        assert_eq!(pycc_rt_int_untag_checked(BOOL_TRUE_MARKER, UNTAG_SIZE), 1);
        let fit = "cannot fit 'int' into an index-sized integer";
        assert_raises(crate::EXCEPTION_TYPE_INDEX_ERROR, fit, || {
            assert_eq!(int_untag_checked(p, UNTAG_INDEX), 0);
        });
        assert_raises(EXCEPTION_TYPE_OVERFLOW_ERROR, fit, || {
            int_untag_checked(p, UNTAG_SIZE);
        });
        assert_raises(
            EXCEPTION_TYPE_OVERFLOW_ERROR,
            "storing a bigint-valued `int` in a container is not supported yet",
            || {
                int_untag_checked(p, UNTAG_ELEMENT);
            },
        );
    }

    #[test]
    fn decode_inline_or_raise_rejects_a_heap_word() {
        let p = big("1267650600228229401496703205376");
        assert_raises(
            EXCEPTION_TYPE_OVERFLOW_ERROR,
            "sizing a bigint-valued `int` is not supported yet",
            || {
                assert_eq!(decode_inline_or_raise(p, "sizing"), None);
            },
        );
        assert_eq!(decode_inline_or_raise(tag_smallint(4), "sizing"), Some(4));
    }
}
