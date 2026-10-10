//! D-244 rule 2's `PyObject*` boundary, runtime half: the one place that
//! decides how a CPython `int`/`bool` becomes a D-141 encoded word and back.
//!
//! **What lives here and what does not, and why.** Rule 2 requires the
//! unpack/pack layer to live in `pycc_rt` rather than being reinvented per
//! mode, so that `--ext` (#1025) and the embedded executable (#1028) share
//! one boundary. It does *not* require `pycc_rt` to call CPython, and it must
//! not: `libpycc_rt.a` is linked into every `native` executable too, where no
//! interpreter exists. A `PyErr_SetString` reference reachable from an
//! ordinary archive member would make every native link fail on an undefined
//! symbol -- or, worse, succeed only by accident of which archive members the
//! linker happened to pull in. So the split is by *dependency*, not by
//! convenience: every rule the boundary has (the inline-integer range, D-141's
//! bool markers, the four-way classification of an encoded word, the pending
//! exception's type and message) is decided here, in ordinary Rust with no
//! foreign symbols, and the fixed C shim (`src/ext/module/`) does
//! nothing but move `PyObject*`s according to the answers.
//!
//! That split is also what makes this boundary testable at all: everything in
//! this module is a pure function over `i64`s, so every arm -- including the
//! two `OverflowError` arms the D-244 amendment introduces -- is executed by
//! ordinary unit tests on a host with no CPython headers, which is exactly the
//! environment the coverage gate runs in.
//!
//! **Any magnitude crosses.** A CPython `int` that fits a C `long long`
//! enters through `pycc_rt_int_from_i64` (inline when it fits D-061's 63-bit
//! range, a heap bigint otherwise); a larger one travels as CPython's own
//! hex text through [`pycc_rt_ext_int_from_hex`], and a heap bigint result
//! leaves as hex text through [`pycc_rt_ext_bigint_hex`] for
//! `PyLong_FromString` (#1040). Text keeps this crate free of any CPython
//! symbol and of CPython's digit layout.

use crate::int_arith::{decode_big, encode_big};
use crate::int_encoding::{
    BOOL_FALSE_MARKER, BOOL_TRUE_MARKER, LOW_TAG_MASK, is_smallint, untag_smallint,
};
use num_bigint::BigInt;

/// [`pycc_rt_ext_int_classify`]: an ordinary tagged smallint; the payload is
/// [`pycc_rt_ext_int_decode`]'s result.
pub const PYCC_EXT_INT_SMALLINT: i32 = 0;
/// [`pycc_rt_ext_int_classify`]: D-141's `False` marker word; pack as
/// `Py_False` so `f(False) is False` holds.
pub const PYCC_EXT_INT_FALSE: i32 = 1;
/// [`pycc_rt_ext_int_classify`]: D-141's `True` marker word.
pub const PYCC_EXT_INT_TRUE: i32 = 2;
/// [`pycc_rt_ext_int_classify`]: a heap bigint pointer, packed through
/// [`pycc_rt_ext_bigint_hex`].
pub const PYCC_EXT_INT_BIGINT: i32 = 3;
/// [`pycc_rt_ext_int_classify`]: not a valid encoded `int` word at all.
/// Fails closed rather than being dereferenced as an attacker-chosen pointer
/// -- the same posture `classify_encoded_int` takes, except that this
/// boundary returns the verdict instead of panicking, because a panic here
/// would cross a plain `extern "C"` frame and abort the host interpreter.
pub const PYCC_EXT_INT_INVALID: i32 = -1;

/// Ingress for a CPython `int` too large for a C `long long`, as the text
/// `PyNumber_ToBase(obj, 16)` produces (`0x1f`, `-0x1f`). Returns the
/// encoded word, a fresh heap bigint carrying one reference.
///
/// Text that is not such a literal is an internal error of the C shim and
/// fails closed.
///
/// # Safety
///
/// `text` must point to `len` readable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_ext_int_from_hex(text: *const u8, len: usize) -> i64 {
    int_from_hex(unsafe { std::slice::from_raw_parts(text, len) })
}

/// The logic of [`pycc_rt_ext_int_from_hex`], apart so tests can observe its
/// fail-closed panic (an unwind past an `extern "C"` frame aborts).
fn int_from_hex(text: &[u8]) -> i64 {
    let parsed = std::str::from_utf8(text).ok().and_then(parse_hex);
    encode_big(parsed.expect("pycc_rt: the ext shim passes CPython's hex text"))
}

fn parse_hex(text: &str) -> Option<BigInt> {
    let (negative, rest) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    let digits = rest.strip_prefix("0x")?;
    let magnitude = BigInt::parse_bytes(digits.as_bytes(), 16)?;
    Some(if negative { -magnitude } else { magnitude })
}

/// Egress for a heap bigint word: its value as NUL-terminated hex text
/// (`-` sign, no prefix) for `PyLong_FromString(text, NULL, 16)`. Borrows the
/// word. The text must be returned through [`pycc_rt_ext_cstr_free`].
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_ext_bigint_hex(encoded: i64) -> *mut std::ffi::c_char {
    let text = decode_big(encoded).to_str_radix(16);
    std::ffi::CString::new(text)
        .expect("hex digits contain no NUL")
        .into_raw()
}

/// Frees text from [`pycc_rt_ext_bigint_hex`].
///
/// # Safety
///
/// `text` must come from [`pycc_rt_ext_bigint_hex`] and not be freed twice.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_ext_cstr_free(text: *mut std::ffi::c_char) {
    drop(unsafe { std::ffi::CString::from_raw(text) });
}

/// Ingress for a CPython `bool` at an `int` parameter (D-244 rule 7 admits
/// it). Produces D-141's marker word, not the smallint `0`/`1`, so that
/// `def f(x: int) -> int: return x` returns `True` for `f(True)` --
/// `PyLong_AsLongLongAndOverflow` would silently flatten it to `1` and destroy
/// that identity, which no numeric-equality test would ever catch.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_ext_bool_encode(value: i32) -> i64 {
    if value != 0 {
        BOOL_TRUE_MARKER
    } else {
        BOOL_FALSE_MARKER
    }
}

/// Egress classification of an encoded `int` word, mirroring
/// `classify_encoded_int`'s four-way split plus an explicit invalid verdict.
///
/// Four-way, never two-way: the markers are both *even* and both carry low tag
/// `0b10`, so a naive "odd means smallint, otherwise raise" egress would report
/// `OverflowError` for `f(True)` and silently destroy bool identity.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_ext_int_classify(encoded: i64) -> i32 {
    if is_smallint(encoded) {
        PYCC_EXT_INT_SMALLINT
    } else if encoded == BOOL_FALSE_MARKER {
        PYCC_EXT_INT_FALSE
    } else if encoded == BOOL_TRUE_MARKER {
        PYCC_EXT_INT_TRUE
    } else if encoded != 0 && encoded & LOW_TAG_MASK == 0 {
        PYCC_EXT_INT_BIGINT
    } else {
        PYCC_EXT_INT_INVALID
    }
}

/// The payload of a word [`pycc_rt_ext_int_classify`] reported as
/// [`PYCC_EXT_INT_SMALLINT`]. Always in the inline-integer range, so
/// `PyLong_FromLongLong` can pack it unconditionally.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_ext_int_decode(encoded: i64) -> i64 {
    untag_smallint(encoded)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_text_round_trips_any_magnitude() {
        for text in [
            "0x8000000000000000",
            "-0x8000000000000001",
            "0xffffffffffffffffffffffffffff",
        ] {
            let word = unsafe { pycc_rt_ext_int_from_hex(text.as_ptr(), text.len()) };
            assert_eq!(pycc_rt_ext_int_classify(word), PYCC_EXT_INT_BIGINT);
            let hex = pycc_rt_ext_bigint_hex(word);
            let back = unsafe { std::ffi::CStr::from_ptr(hex) }
                .to_str()
                .unwrap()
                .to_string();
            unsafe { pycc_rt_ext_cstr_free(hex) };
            assert_eq!(back, text.replace("0x", ""));
            crate::int_encoding::bigint_release(word);
        }
        // CPython never sends a small value this way, but one still decodes
        // to its canonical smallint.
        let word = unsafe { pycc_rt_ext_int_from_hex(b"-0x2a".as_ptr(), 5) };
        assert_eq!(pycc_rt_ext_int_decode(word), -42);
    }

    #[test]
    #[should_panic(expected = "the ext shim passes CPython's hex text")]
    fn malformed_hex_text_fails_closed() {
        int_from_hex(b"12");
    }

    #[test]
    fn bool_encode_produces_the_identity_preserving_markers() {
        assert_eq!(pycc_rt_ext_bool_encode(1), BOOL_TRUE_MARKER);
        assert_eq!(pycc_rt_ext_bool_encode(0), BOOL_FALSE_MARKER);
        // Any non-zero C `int` is truthy, not just 1.
        assert_eq!(pycc_rt_ext_bool_encode(-7), BOOL_TRUE_MARKER);
        assert_eq!(
            pycc_rt_ext_int_classify(pycc_rt_ext_bool_encode(1)),
            PYCC_EXT_INT_TRUE
        );
        assert_eq!(
            pycc_rt_ext_int_classify(pycc_rt_ext_bool_encode(0)),
            PYCC_EXT_INT_FALSE
        );
    }

    #[test]
    fn classify_reports_bigints_and_fails_closed_on_an_unrecognized_word() {
        // A non-zero 4-aligned word is the bigint shape (the pointer itself
        // is never dereferenced here, so a fabricated one is safe).
        assert_eq!(pycc_rt_ext_int_classify(0x1000), PYCC_EXT_INT_BIGINT);
        // Zero and the remaining low-tag-`10` words are neither.
        assert_eq!(pycc_rt_ext_int_classify(0), PYCC_EXT_INT_INVALID);
        assert_eq!(pycc_rt_ext_int_classify(0b1010), PYCC_EXT_INT_INVALID);
    }

    /// The fixed C shim maps a pending exception's type tag to a `PyExc_*`
    /// object with a `switch` over these literal values, because it cannot
    /// see this crate's Rust constants. This test is that mapping's drift
    /// guard: renumbering a tag without updating `src/ext/module/`
    /// would otherwise raise the wrong CPython exception class silently.
    #[test]
    fn exception_type_tags_match_the_c_shims_hardcoded_switch() {
        assert_eq!(crate::EXCEPTION_TYPE_EXCEPTION, 0);
        assert_eq!(crate::EXCEPTION_TYPE_VALUE_ERROR, 1);
        assert_eq!(crate::EXCEPTION_TYPE_TYPE_ERROR, 2);
        assert_eq!(crate::EXCEPTION_TYPE_KEY_ERROR, 3);
        assert_eq!(crate::EXCEPTION_TYPE_INDEX_ERROR, 4);
        assert_eq!(crate::EXCEPTION_TYPE_ZERO_DIV_ERROR, 5);
        assert_eq!(crate::EXCEPTION_TYPE_RUNTIME_ERROR, 6);
        // Part A of #1038 (#1063). Tags 7..=24 and 26..=28 are deliberately
        // absent: this crate declares no constants for classes it never
        // raises by name, so there is nothing here to pin them against.
        assert_eq!(crate::EXCEPTION_TYPE_OVERFLOW_ERROR, 25);
        // #1388: `AttributeError`, raised by name for an unassigned slot.
        assert_eq!(crate::EXCEPTION_TYPE_ATTRIBUTE_ERROR, 29);
        // #1316: the shim's `PYCC_EXT_TAG_FOREIGN_BASE`, which
        // `src/ext_build_tests/toolchain.rs` pins against `pycc_hir`.
        assert_eq!(crate::EXCEPTION_TYPE_FOREIGN_BASE, 255);
    }
}
