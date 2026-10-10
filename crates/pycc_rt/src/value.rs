//! What one container word means, by its kind: Python's `==`, `<`, `hash()`,
//! `repr()` and truth value of an element of a list, a tuple, a dict or a
//! set, whatever the container.
//!
//! A word is the element's native representation in 8 bytes (see
//! `list.rs`): an encoded `int`, `f64` bits, `0`/`1` for a `bool`, or a
//! pointer to a `str`, an instance, a list, a tuple, a dict or a set. The
//! kind (`LIST_KIND_*`) says which. A tuple is a `PyListObj` whose own kind is
//! [`TUPLE_OBJECT`] and which records one kind per element (`tuple.rs`).
//!
//! Hashes are CPython's for `int`, `bool`, `float` and tuples of them, so a
//! set of those iterates in CPython's order (`set.rs`). A `str` hash is
//! randomized per process in CPython, so no program can depend on it; this
//! one is FNV-1a.

use crate::dict::PyDictObj;
use crate::instance::PyInstanceObj;
use crate::list::{
    LIST_KIND_BOOL, LIST_KIND_DICT, LIST_KIND_FLOAT, LIST_KIND_INSTANCE, LIST_KIND_INT,
    LIST_KIND_LIST, LIST_KIND_SET, LIST_KIND_STR, LIST_KIND_TUPLE, PyListObj,
};
use crate::set::PySetObj;
use crate::{PyStrObj, encoded_int_cmp, inline_int_value};
use std::cmp::Ordering;

/// `PyListObj::kind` of a tuple object: its elements carry their own kinds.
pub const TUPLE_OBJECT: i64 = 16;

/// The `crate::rc` kind byte of a word of kind `kind`.
pub(crate) fn rc_kind(kind: i64) -> u8 {
    match kind {
        LIST_KIND_STR => b's',
        LIST_KIND_LIST | LIST_KIND_TUPLE => b'l',
        LIST_KIND_INSTANCE => b'c',
        LIST_KIND_DICT => b'd',
        LIST_KIND_SET => b'e',
        _ => b'w',
    }
}

/// Takes one more reference to `word`.
pub(crate) fn retain(kind: i64, word: i64) {
    crate::rc::retain_word(rc_kind(kind), word);
}

/// Drops one reference to `word`.
pub(crate) fn release(kind: i64, word: i64) {
    crate::rc::release_word(rc_kind(kind), word);
}

fn str_bytes<'a>(word: i64) -> &'a [u8] {
    unsafe { &*(word as *mut PyStrObj) }.bytes()
}

/// The elements of a list or tuple object with each element's kind.
pub(crate) fn sequence_items(word: i64) -> Vec<(i64, i64)> {
    unsafe { &*(word as *mut PyListObj) }.kinded_items()
}

/// Python `a == b` for two words of `kind`.
pub(crate) fn eq(kind: i64, a: i64, b: i64) -> bool {
    match kind {
        LIST_KIND_INT => encoded_int_cmp(a, b) == Ordering::Equal,
        LIST_KIND_FLOAT => f64::from_bits(a as u64) == f64::from_bits(b as u64),
        LIST_KIND_STR => a == b || str_bytes(a) == str_bytes(b),
        LIST_KIND_LIST | LIST_KIND_TUPLE => {
            if a == b {
                return true;
            }
            let (xs, ys) = (sequence_items(a), sequence_items(b));
            xs.len() == ys.len()
                && xs
                    .iter()
                    .zip(ys.iter())
                    .all(|((kx, x), (ky, y))| kx == ky && eq(*kx, *x, *y))
        }
        LIST_KIND_DICT => {
            a == b || crate::dict::dicts_equal(a as *mut PyDictObj, b as *mut PyDictObj)
        }
        LIST_KIND_SET => a == b || crate::set::sets_equal(a as *mut PySetObj, b as *mut PySetObj),
        // `bool` words and instances (identity).
        _ => a == b,
    }
}

/// Python `a < b` for two words of `kind`; `None` when the kind has no
/// order (the caller raises `TypeError`).
pub(crate) fn lt(kind: i64, a: i64, b: i64) -> Option<bool> {
    Some(match kind {
        LIST_KIND_INT => encoded_int_cmp(a, b).is_lt(),
        LIST_KIND_FLOAT => f64::from_bits(a as u64) < f64::from_bits(b as u64),
        LIST_KIND_BOOL => a < b,
        LIST_KIND_STR => str_bytes(a) < str_bytes(b),
        LIST_KIND_LIST | LIST_KIND_TUPLE => {
            let (xs, ys) = (sequence_items(a), sequence_items(b));
            // CPython: the first position whose items differ decides,
            // else the shorter sequence is smaller.
            for ((kx, x), (_, y)) in xs.iter().zip(ys.iter()) {
                if !eq(*kx, *x, *y) {
                    return lt(*kx, *x, *y);
                }
            }
            xs.len() < ys.len()
        }
        _ => return None,
    })
}

/// The name CPython's ordering `TypeError` uses for a word of `kind`.
pub(crate) fn type_name(kind: i64) -> &'static str {
    match kind {
        LIST_KIND_INT => "int",
        LIST_KIND_FLOAT => "float",
        LIST_KIND_BOOL => "bool",
        LIST_KIND_STR => "str",
        LIST_KIND_LIST => "list",
        LIST_KIND_TUPLE => "tuple",
        LIST_KIND_DICT => "dict",
        LIST_KIND_SET => "set",
        _ => "object",
    }
}

/// `a < b`, raising CPython's `TypeError` for an unordered kind (the result
/// is then `false`).
pub(crate) fn lt_or_raise(kind: i64, a: i64, b: i64) -> bool {
    lt(kind, a, b).unwrap_or_else(|| {
        let name = type_name(kind);
        crate::raise_builtin(
            crate::exception::EXCEPTION_TYPE_TYPE_ERROR,
            "TypeError",
            &format!("'<' not supported between instances of '{name}' and '{name}'"),
        );
        false
    })
}

/// `_PyHASH_MODULUS` and `_PyHASH_BITS`.
const MODULUS: u64 = (1 << 61) - 1;
const HASH_BITS: i32 = 61;

/// `frexp`: `v == m * 2**e` with `0.5 <= |m| < 1` (finite, non-zero `v`).
fn frexp(v: f64) -> (f64, i32) {
    if v == 0.0 {
        return (v, 0);
    }
    let bits = v.to_bits();
    let exp = ((bits >> 52) & 0x7ff) as i32;
    if exp == 0 {
        // Subnormal: scale into the normal range first.
        let (m, e) = frexp(v * 2f64.powi(64));
        return (m, e - 64);
    }
    let m = f64::from_bits((bits & !(0x7ff << 52)) | (1022 << 52));
    (m, exp - 1022)
}

/// CPython's `_Py_HashDouble` (a NaN, whose CPython hash is its address,
/// hashes to 0).
pub(crate) fn hash_float(v: f64) -> i64 {
    if !v.is_finite() {
        if v.is_infinite() {
            return if v > 0.0 { 314_159 } else { -314_159 };
        }
        return 0;
    }
    let (mut m, mut e) = frexp(v);
    let mut sign: i64 = 1;
    if m < 0.0 {
        sign = -1;
        m = -m;
    }
    let mut x: u64 = 0;
    while m != 0.0 {
        x = ((x << 28) & MODULUS) | (x >> (HASH_BITS - 28));
        m *= 268_435_456.0;
        e -= 28;
        let y = m as u64;
        m -= y as f64;
        x += y;
        if x >= MODULUS {
            x -= MODULUS;
        }
    }
    let e = if e >= 0 {
        e % HASH_BITS
    } else {
        HASH_BITS - 1 - ((-1 - e) % HASH_BITS)
    };
    x = ((x << e) & MODULUS) | (x >> (HASH_BITS - e));
    let x = (x as i64).wrapping_mul(sign);
    if x == -1 { -2 } else { x }
}

fn hash_str(bytes: &[u8]) -> i64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    let h = h as i64;
    if h == -1 { -2 } else { h }
}

/// Python `hash(word)`; `None` for an unhashable kind (a list, dict or
/// set).
pub(crate) fn hash(kind: i64, word: i64) -> Option<i64> {
    Some(match kind {
        LIST_KIND_INT => crate::pycc_rt_hash_int(word),
        LIST_KIND_BOOL => word,
        LIST_KIND_FLOAT => hash_float(f64::from_bits(word as u64)),
        LIST_KIND_STR => hash_str(str_bytes(word)),
        LIST_KIND_TUPLE => {
            let mut lanes = Vec::new();
            for (k, w) in sequence_items(word) {
                lanes.push(hash(k, w)?);
            }
            unsafe { crate::pycc_rt_hash_tuple(lanes.as_ptr(), lanes.len()) }
        }
        LIST_KIND_INSTANCE | crate::list::LIST_KIND_UNCOUNTED_INSTANCE => {
            crate::pycc_rt_hash_pointer(word as *const std::ffi::c_void)
        }
        _ => return None,
    })
}

/// `hash(word)`, raising CPython's `TypeError` for an unhashable kind.
pub(crate) fn hash_or_raise(kind: i64, word: i64) -> Option<i64> {
    let found = hash(kind, word);
    if found.is_none() {
        crate::raise_builtin(
            crate::exception::EXCEPTION_TYPE_TYPE_ERROR,
            "TypeError",
            &format!("unhashable type: '{}'", type_name(kind)),
        );
    }
    found
}

/// Python truth value of `word`.
pub(crate) fn truthy(kind: i64, word: i64) -> bool {
    match kind {
        LIST_KIND_INT => inline_int_value(word) != Some(0),
        LIST_KIND_FLOAT => f64::from_bits(word as u64) != 0.0,
        LIST_KIND_STR => !str_bytes(word).is_empty(),
        LIST_KIND_LIST | LIST_KIND_TUPLE => !sequence_items(word).is_empty(),
        LIST_KIND_DICT => crate::dict::dict_len(word as *mut PyDictObj) != 0,
        LIST_KIND_SET => crate::set::set_len(word as *mut PySetObj) != 0,
        _ => word != 0,
    }
}

/// Appends `repr(word)` to `out`; `false` when rendering raised (a `float`
/// needing scientific notation).
pub(crate) fn repr(kind: i64, word: i64, out: &mut String) -> bool {
    let rendered = match kind {
        LIST_KIND_INT => crate::int_to_str(word),
        LIST_KIND_FLOAT => crate::pycc_rt_float_to_str(f64::from_bits(word as u64)),
        LIST_KIND_BOOL => crate::pycc_rt_bool_to_str(word as i8),
        LIST_KIND_STR => {
            let text = String::from_utf8_lossy(str_bytes(word));
            crate::list::str_repr(&text, out);
            return true;
        }
        LIST_KIND_LIST | LIST_KIND_TUPLE => {
            return crate::list::sequence_repr(unsafe { &*(word as *mut PyListObj) }, out);
        }
        LIST_KIND_DICT => return crate::dict::dict_repr_into(word as *mut PyDictObj, out),
        LIST_KIND_SET => return crate::set::set_repr_into(word as *mut PySetObj, out),
        _ => {
            let instance = word as *mut PyInstanceObj;
            let mut len = 0usize;
            let name = unsafe { crate::pycc_rt_ext_instance_class(instance, &mut len) };
            let name = unsafe { std::slice::from_raw_parts(name, len) };
            out.push_str(&format!(
                "<__main__.{} object at {:#x}>",
                String::from_utf8_lossy(name),
                word
            ));
            return true;
        }
    };
    out.push_str(&String::from_utf8_lossy(unsafe { &*rendered }.bytes()));
    unsafe { crate::pycc_rt_str_decref(rendered) };
    crate::pycc_rt_exception_active() == 0
}

/// Compare operators of [`pycc_rt_value_compare`], in `MirCmpOp` order.
pub const CMP_EQ: i64 = 0;
pub const CMP_NE: i64 = 1;
pub const CMP_LT: i64 = 2;
pub const CMP_LE: i64 = 3;
pub const CMP_GT: i64 = 4;
pub const CMP_GE: i64 = 5;

/// `a <op> b` for two words of `kind` (`CMP_*`), raising `TypeError` for
/// an ordering of an unordered kind. Both words are borrowed.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_value_compare(kind: i64, a: i64, b: i64, op: i64) -> i8 {
    if kind == LIST_KIND_SET && op >= CMP_LT {
        // Set ordering is inclusion: `a <= b` is `a.issubset(b)`, `a < b`
        // a proper subset.
        let (small, big) = if op == CMP_LT || op == CMP_LE {
            (a, b)
        } else {
            (b, a)
        };
        let (small, big) = (small as *mut PySetObj, big as *mut PySetObj);
        let subset = unsafe { crate::set::pycc_rt_set_issubset(small, big) } != 0;
        let proper = crate::set::set_len(small) < crate::set::set_len(big);
        return i8::from(if op == CMP_LT || op == CMP_GT {
            subset && proper
        } else {
            subset
        });
    }
    i8::from(match op {
        CMP_EQ => eq(kind, a, b),
        CMP_NE => !eq(kind, a, b),
        CMP_LT => lt_or_raise(kind, a, b),
        CMP_LE => eq(kind, a, b) || lt_or_raise(kind, a, b),
        CMP_GT => lt_or_raise(kind, b, a),
        _ => eq(kind, a, b) || lt_or_raise(kind, b, a),
    })
}

/// `hash(word)` for a word of `kind` (`TypeError` for an unhashable one).
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_value_hash(kind: i64, word: i64) -> i64 {
    hash_or_raise(kind, word).unwrap_or(0)
}

/// `repr(word)` as a new `str`.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_value_repr(kind: i64, word: i64) -> *mut PyStrObj {
    let mut out = String::new();
    if !repr(kind, word, &mut out) {
        return crate::new_pystr(b"");
    }
    crate::new_pystr(out.as_bytes())
}

/// Python truth value of a word of `kind`.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_value_truthy(kind: i64, word: i64) -> i8 {
    i8::from(truthy(kind, word))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn float_hash_matches_cpython() {
        // Values from CPython 3.13: hash(0.5), hash(1.5), hash(-2.25),
        // hash(1e300), hash(3.0).
        assert_eq!(hash_float(0.5), 1_152_921_504_606_846_976);
        assert_eq!(hash_float(1.5), 1_152_921_504_606_846_977);
        assert_eq!(hash_float(-2.25), -576_460_752_303_423_490);
        assert_eq!(hash_float(3.0), 3);
        assert_eq!(hash_float(1e300), 1_224_995_262_755_759_164);
        assert_eq!(hash_float(1e-300), 482_449_582_752_280_463);
        assert_eq!(hash_float(0.1), 230_584_300_921_369_408);
        assert_eq!(hash_float(f64::INFINITY), 314_159);
        assert_eq!(hash_float(-1.0), -2);
    }
}
