//! `int | float`: one word holding either an `int` or a `float`.
//!
//! A variable that holds an `int` at one point and a `float` at another
//! (`best = float('inf')` later replaced by an `int`, an accumulator that
//! starts as `0` and receives floats) has the static type `int | float`
//! (`pycc_hir::Ty::Num`). Its value is one 64-bit word that extends the
//! `int` word (D-141, `int_encoding`):
//!
//! - **an `int`** is its own `int` word, unchanged: an odd smallint, a bool
//!   marker (`2`/`6`, so a `bool` keeps its identity) or a heap bigint
//!   pointer. Converting an `int` to `int | float`, or back once it is known
//!   to be one, is free;
//! - **a `float`** whose biased exponent lies in `0x300..=0x4FF` (every
//!   magnitude from about `1.7e-77` to `1.2e77`) is stored inline in the
//!   word patterns no `int` uses, those ending in binary `10` (Ruby's
//!   "flonum" layout): the bit pattern rotated left by three, its two top
//!   exponent bits below the sign replaced by the tag. The two patterns
//!   the bool markers take and eight others ([`SPECIAL_BASE`]) hold the
//!   common floats outside that range -- `0.0`, `-0.0`, both infinities,
//!   NaN of either sign -- and `2.0`/`-2.0`, whose rotation the markers
//!   took; the eight tiny floats whose patterns were taken are boxed;
//! - **every other `float`** is a heap [`FloatBoxObj`], counted exactly as
//!   a bigint (the two share a header, and `bigint_retain`/`_release` free
//!   either).
//!
//! So every `int` refcount path (`pycc_codegen`'s `bigint_rc`, a slot's
//! release before a store, a container's element kinds) serves an
//! `int | float` word unchanged, and pure `int` and pure `float` code never
//! sees a `float` word at all.
//!
//! A NaN keeps only its sign, the one NaN property CPython lets a program
//! observe (`math.copysign`).
//!
//! **Ownership** is the `int` word's: a heap word owns one reference.
//! Every function here borrows its operands and returns a fresh word (or
//! value); `pycc_codegen` releases an operand that was a temporary.
//!
//! **Errors** are pending exceptions with CPython 3.13's messages; the
//! function then returns a type-valid sentinel the generated code never
//! reads (it tests the pending flag first).

use crate::exception::raise_builtin;
use crate::int_arith::{
    decode_big, encode_big, int_abs, int_add, int_decimal, int_float_cmp, int_floordiv,
    int_floormod, int_from_float, int_mul, int_pow, int_sub, int_to_float, int_truediv,
};
use crate::int_encoding::*;
use crate::{EXCEPTION_TYPE_TYPE_ERROR, PyStrObj, new_pystr};
use std::cmp::Ordering;

mod ops;
mod text;

pub use ops::*;
pub use text::*;

/// The first of the ten special inline `float` words: the word `i * 4`
/// past it holds `SPECIALS[i]`. They are the encodings of the eight tiny
/// floats `0x3000_0000_0000_000k` and `0xB000_0000_0000_000k` (`k < 4`),
/// which are boxed instead.
pub(crate) const SPECIAL_BASE: u64 = 0x8000_0000_0000_0002;
/// The floats the special words hold, in word order.
const SPECIALS: [f64; 8] = [
    0.0,
    -0.0,
    f64::INFINITY,
    f64::NEG_INFINITY,
    f64::NAN,
    // `-NAN` is not a `const` expression on every toolchain this crate
    // builds with; this is its bit pattern.
    f64::from_bits(0xFFF8_0000_0000_0000),
    2.0,
    -2.0,
];
/// The span of the special words.
const SPECIAL_SPAN: u64 = 4 * SPECIALS.len() as u64;
/// The tag of an inline `float`: the low two bits.
const FLOAT_TAG: i64 = 0b10;

/// What a word holds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Num {
    Float(f64),
    /// The word itself, an `int` word (borrowed).
    Int(i64),
}

/// The inline `float` in `word`, when it holds one.
fn inline_float(word: i64) -> Option<f64> {
    if word & LOW_TAG_MASK != FLOAT_TAG || word == BOOL_FALSE_MARKER || word == BOOL_TRUE_MARKER {
        return None;
    }
    let offset = (word as u64).wrapping_sub(SPECIAL_BASE);
    if offset < SPECIAL_SPAN {
        return Some(SPECIALS[(offset >> 2) as usize]);
    }
    // Bits 62..61 of the float were `01` when bit 60 (now the word's top
    // bit) was set, else `10`.
    let top = if word < 0 { 0b01 } else { 0b10 };
    Some(f64::from_bits(((word & !LOW_TAG_MASK) | top).rotate_right(3) as u64))
}

/// The inline word of `value`, when it has one.
pub(crate) fn inline_word(value: f64) -> Option<i64> {
    if value.is_nan() {
        return Some((SPECIAL_BASE + if value.is_sign_negative() { 20 } else { 16 }) as i64);
    }
    let bits = value.to_bits() as i64;
    let exponent_top = (bits >> 60) & 0b111;
    if exponent_top == 0b011 || exponent_top == 0b100 {
        let word = (bits.rotate_left(3) & !LOW_TAG_MASK) | FLOAT_TAG;
        let taken = word == BOOL_FALSE_MARKER
            || word == BOOL_TRUE_MARKER
            || (word as u64).wrapping_sub(SPECIAL_BASE) < SPECIAL_SPAN;
        if !taken {
            return Some(word);
        }
    }
    SPECIALS
        .iter()
        .position(|special| special.to_bits() == value.to_bits())
        .map(|index| (SPECIAL_BASE + 4 * index as u64) as i64)
}

/// Whether `word` is a `float`.
pub(crate) fn is_float(word: i64) -> bool {
    matches!(decode(word), Num::Float(_))
}

/// Reads `word` (no reference is taken).
pub(crate) fn decode(word: i64) -> Num {
    if let Some(value) = inline_float(word) {
        return Num::Float(value);
    }
    if word & LOW_TAG_MASK == 0
        && word != 0
        && let Some(value) = boxed_float(word)
    {
        return Num::Float(value);
    }
    Num::Int(word)
}

/// A fresh word holding the `float` `value`.
pub(crate) fn from_f64(value: f64) -> i64 {
    inline_word(value).unwrap_or_else(|| box_float(value))
}

/// Whether a runtime call made since the operation began raised.
fn raised() -> bool {
    crate::exception::pycc_rt_exception_active() != 0
}

/// A placeholder result for a raising operation: the smallint `0`.
const SENTINEL: i64 = 1;

/// Takes one more reference to `word`.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_num_retain(word: i64) {
    bigint_retain(word);
}

/// Drops one reference from `word`.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_num_release(word: i64) {
    bigint_release(word);
}

/// `float` to `int | float`: a fresh word (a boxed `float` past the inline
/// range).
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_num_from_float(value: f64) -> i64 {
    from_f64(value)
}

/// `float(x)`: `OverflowError` for an `int` beyond the `float` range.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_num_to_float(word: i64) -> f64 {
    to_f64(word)
}

pub(crate) fn to_f64(word: i64) -> f64 {
    match decode(word) {
        Num::Float(f) => f,
        Num::Int(int) => int_to_float(int),
    }
}

/// `int(x)`: a fresh `int` word, truncating a `float` toward zero
/// (`ValueError` for NaN, `OverflowError` for an infinity). `int(True)` is
/// `1`.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_num_to_int(word: i64) -> i64 {
    match decode(word) {
        Num::Float(f) => int_from_float(f),
        Num::Int(int) => plain_int(int),
    }
}

/// A fresh copy of the `int` word `int` with a bool's identity dropped.
fn plain_int(int: i64) -> i64 {
    match inline_int_value(int) {
        Some(value) => tag_smallint(value),
        None => {
            bigint_retain(int);
            int
        }
    }
}

/// The `int` the word holds, for a read narrowed by `isinstance(x, int)`
/// (borrowed: the word itself), or `TypeError` when it holds a `float`.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_num_as_int(word: i64) -> i64 {
    match decode(word) {
        Num::Int(int) => int,
        Num::Float(_) => {
            raise_builtin(EXCEPTION_TYPE_TYPE_ERROR, "TypeError", "expected an int, got a float");
            SENTINEL
        }
    }
}

/// The `int` an `int | float` index holds (`range(x)`, `xs[x]`, `s[x]`,
/// `t[x]`, a slice bound), retained for the caller; a `float` raises
/// CPython's `TypeError` for the use `kind` names (an encoded `int`:
/// `pycc_mir::IndexUse::coerce_kind`).
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_num_index(word: i64, kind: i64) -> i64 {
    match decode(word) {
        Num::Int(int) => {
            pycc_rt_num_retain(int);
            int
        }
        Num::Float(_) => {
            let message = match inline_int_value(kind).unwrap_or(0) {
                5 => "list indices must be integers or slices, not float",
                6 => "string indices must be integers, not 'float'",
                7 => "tuple indices must be integers or slices, not float",
                8 => "slice indices must be integers or None or have an __index__ method",
                _ => "'float' object cannot be interpreted as an integer",
            };
            raise_builtin(EXCEPTION_TYPE_TYPE_ERROR, "TypeError", message);
            SENTINEL
        }
    }
}

/// `1` when the word holds a `float`, for `isinstance(x, float)`.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_num_is_float(word: i64) -> i8 {
    i8::from(is_float(word))
}

/// `isinstance(x, classes)` of a word: `mask` (an encoded `int`) has bit 1
/// for `int` (which a `bool` is too), bit 2 for `float`, bit 4 for `bool`.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_num_isinstance(word: i64, mask: i64) -> i8 {
    let mask = inline_int_value(mask).unwrap_or(0);
    let matches = match decode(word) {
        Num::Float(_) => mask & 2 != 0,
        Num::Int(BOOL_FALSE_MARKER | BOOL_TRUE_MARKER) => mask & 5 != 0,
        Num::Int(_) => mask & 1 != 0,
    };
    i8::from(matches)
}

/// `type(x).__name__`.
pub(crate) fn type_name(word: i64) -> &'static str {
    match decode(word) {
        Num::Float(_) => "float",
        Num::Int(BOOL_FALSE_MARKER | BOOL_TRUE_MARKER) => "bool",
        Num::Int(_) => "int",
    }
}

/// `type(x).__name__` as a new `str`.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_num_type_name(word: i64) -> *mut PyStrObj {
    new_pystr(type_name(word).as_bytes())
}

/// Python's ordering of two words: `None` when either is NaN.
pub(crate) fn compare(a: i64, b: i64) -> Option<Ordering> {
    compare_values(decode(a), decode(b))
}

/// Python's ordering of two numbers, exact between an `int` and a `float`.
pub(crate) fn compare_values(a: Num, b: Num) -> Option<Ordering> {
    match (a, b) {
        (Num::Float(x), Num::Float(y)) => x.partial_cmp(&y),
        (Num::Int(x), Num::Int(y)) => Some(encoded_int_cmp(x, y)),
        (Num::Int(x), Num::Float(y)) => match int_float_cmp(x, y) {
            -1 => Some(Ordering::Less),
            0 => Some(Ordering::Equal),
            1 => Some(Ordering::Greater),
            _ => None,
        },
        (Num::Float(x), Num::Int(y)) => match int_float_cmp(y, x) {
            -1 => Some(Ordering::Greater),
            0 => Some(Ordering::Equal),
            1 => Some(Ordering::Less),
            _ => None,
        },
    }
}

/// The number a container word of `kind` (`LIST_KIND_*`) holds, for the
/// comparisons across numeric kinds (`(1, 2.0) == (1.0, 2)`).
pub(crate) fn of_kind(kind: i64, word: i64) -> Option<Num> {
    Some(match kind {
        crate::list::LIST_KIND_INT => Num::Int(word),
        crate::list::LIST_KIND_BOOL => Num::Int(if word != 0 { BOOL_TRUE_MARKER } else { BOOL_FALSE_MARKER }),
        crate::list::LIST_KIND_FLOAT => Num::Float(f64::from_bits(word as u64)),
        crate::list::LIST_KIND_NUM => decode(word),
        _ => return None,
    })
}

/// Python's truth value.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_num_truthy(word: i64) -> i8 {
    i8::from(match decode(word) {
        Num::Float(f) => f != 0.0,
        Num::Int(int) => inline_int_value(int) != Some(0),
    })
}

/// The word of an `int` result computed fresh (or the sentinel when the
/// computation raised; a raising `int` operation's sentinel owns nothing).
fn int_result(word: i64) -> i64 {
    if raised() { SENTINEL } else { word }
}

/// The word of a `float` result.
fn float_result(value: f64) -> i64 {
    if raised() { SENTINEL } else { from_f64(value) }
}

#[cfg(test)]
mod tests;
