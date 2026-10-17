//! `str`, `format`, `hash` and `round` of an `int | float` word: each is
//! the `int` or the `float` operation of the value the word holds.

use super::*;
use num_integer::Integer;

/// `str(x)` / `repr(x)` / `print(x)`.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_num_to_str(word: i64) -> *mut PyStrObj {
    match decode(word) {
        Num::Float(f) => crate::float_arith::float_to_str(f),
        Num::Int(int) => crate::int_arith::int_to_str(int),
    }
}

/// The text `str(x)` gives, for the container renderers (`crate::value`).
pub(crate) fn repr(word: i64, out: &mut String) -> bool {
    match decode(word) {
        Num::Float(f) => {
            out.push_str(&crate::float_arith::float_repr(f));
            true
        }
        Num::Int(int) => match int_decimal(int) {
            Some(text) => {
                out.push_str(&text);
                true
            }
            None => false,
        },
    }
}

/// `format(x, spec)` / `f"{x:spec}"`.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_format_num(word: i64, spec: *mut PyStrObj) -> *mut PyStrObj {
    match decode(word) {
        Num::Float(f) => crate::format::pycc_rt_format_float(f, spec),
        Num::Int(BOOL_FALSE_MARKER) => crate::format::pycc_rt_format_bool(0, spec),
        Num::Int(BOOL_TRUE_MARKER) => crate::format::pycc_rt_format_bool(1, spec),
        Num::Int(int) => crate::format::pycc_rt_format_int(int, spec),
    }
}

/// `hash(x)`: CPython's, so an integral `float` hashes as the equal `int`.
pub(crate) fn hash(word: i64) -> i64 {
    match decode(word) {
        Num::Float(f) => crate::value::hash_float(f),
        Num::Int(int) => crate::pycc_rt_hash_int(int),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_hash_num(word: i64) -> i64 {
    hash(word)
}

/// `round(x)`: an `int` (a fresh `int` word).
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_num_round(word: i64) -> i64 {
    match decode(word) {
        Num::Float(f) => crate::round::pycc_rt_float_round_to_int(f),
        Num::Int(int) => round_int(int, 0),
    }
}

/// `round(x, ndigits)`: an `int` stays an `int` (rounded half to even to
/// `10 ** -ndigits` when `ndigits` is negative), a `float` is
/// `float.__round__`'s.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_num_round_digits(word: i64, ndigits: i64) -> i64 {
    match decode(word) {
        Num::Float(f) => from_f64(crate::round::pycc_rt_float_round(f, ndigits)),
        Num::Int(int) => round_int(int, ndigits),
    }
}

/// `round(n, ndigits)` of an `int` word, as a fresh `int` word.
pub(crate) fn round_int(int: i64, ndigits: i64) -> i64 {
    if ndigits >= 0 {
        // `round(True, 2)` is `1`.
        return match inline_int_value(int) {
            Some(value) => tag_smallint(value),
            None => {
                bigint_retain(int);
                int
            }
        };
    }
    let Some(exponent) = ndigits.checked_neg().and_then(|e| u32::try_from(e).ok()) else {
        return tag_smallint(0);
    };
    let value = decode_big(int);
    // `|value| < 10 ** (digits + 1) / 2`: every such value rounds to `0`,
    // so a huge `-ndigits` never builds its power.
    if u64::from(exponent) > value.bits() / 3 + 2 {
        return tag_smallint(0);
    }
    let power = num_traits::pow(num_bigint::BigInt::from(10), exponent as usize);
    let (quotient, remainder) = value.div_mod_floor(&power);
    let twice: num_bigint::BigInt = &remainder * 2u32;
    let rounded = match twice.cmp(&power) {
        Ordering::Less => quotient,
        Ordering::Greater => quotient + 1,
        Ordering::Equal => {
            if quotient.is_odd() {
                quotient + 1
            } else {
                quotient
            }
        }
    };
    let result = rounded * power;
    encode_big(result)
}

/// `round(n, ndigits)` of an `int`, for an `int` argument.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_int_round_digits(int: i64, ndigits: i64) -> i64 {
    round_int(int, ndigits)
}

/// `math.floor(x)`: an `int` (a fresh `int` word).
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_num_floor(word: i64) -> i64 {
    match decode(word) {
        Num::Float(f) => crate::int_arith::int_from_float(f.floor()),
        Num::Int(int) => round_int(int, 0),
    }
}

/// `math.ceil(x)`: an `int` (a fresh `int` word).
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_num_ceil(word: i64) -> i64 {
    match decode(word) {
        Num::Float(f) => crate::int_arith::int_from_float(f.ceil()),
        Num::Int(int) => round_int(int, 0),
    }
}
