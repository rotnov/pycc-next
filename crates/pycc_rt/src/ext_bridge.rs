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
//! foreign symbols, and the fixed C shim (`src/ext/pycc_ext_module.c`) does
//! nothing but move `PyObject*`s according to the answers.
//!
//! That split is also what makes this boundary testable at all: everything in
//! this module is a pure function over `i64`s, so every arm -- including the
//! two `OverflowError` arms the D-244 amendment introduces -- is executed by
//! ordinary unit tests on a host with no CPython headers, which is exactly the
//! environment the coverage gate runs in.
//!
//! **The boundary is the inline-integer range, never `i64`.** `fits_smallint`
//! accepts `[-2^62, 2^62-1]`; `pycc_rt_int_from_i64` promotes anything outside
//! it to a bigint-tagged word. Before Part C of #1038
//! ([#1065](https://github.com/rotnov/pycc/issues/1065)) the compiled body's
//! first inline-int decode then panicked, which across a plain `extern "C"`
//! boundary is a process abort -- i.e. a killed interpreter; it now raises
//! `OverflowError` instead, which the `ext` wrapper reports to the host. The
//! ingress check stays written against the inline range rather than `i64` so
//! `f(4611686018427387905)` is rejected with a clean `OverflowError` at the
//! boundary instead of deep inside the body. See the D-244 amendment and
//! #1040.

use crate::int_encoding::{
    BOOL_FALSE_MARKER, BOOL_TRUE_MARKER, LOW_TAG_MASK, fits_smallint, is_smallint, untag_smallint,
};

/// [`pycc_rt_ext_int_classify`]: an ordinary tagged smallint; the payload is
/// [`pycc_rt_ext_int_decode`]'s result.
pub const PYCC_EXT_INT_SMALLINT: i32 = 0;
/// [`pycc_rt_ext_int_classify`]: D-141's `False` marker word; pack as
/// `Py_False` so `f(False) is False` holds.
pub const PYCC_EXT_INT_FALSE: i32 = 1;
/// [`pycc_rt_ext_int_classify`]: D-141's `True` marker word.
pub const PYCC_EXT_INT_TRUE: i32 = 2;
/// [`pycc_rt_ext_int_classify`]: a heap bigint pointer. The D-244 amendment
/// makes this an `OverflowError` at the wrapper until #1040 lands.
pub const PYCC_EXT_INT_BIGINT: i32 = 3;
/// [`pycc_rt_ext_int_classify`]: not a valid encoded `int` word at all.
/// Fails closed rather than being dereferenced as an attacker-chosen pointer
/// -- the same posture `classify_encoded_int` takes, except that this
/// boundary returns the verdict instead of panicking, because a panic here
/// would cross a plain `extern "C"` frame and abort the host interpreter.
pub const PYCC_EXT_INT_INVALID: i32 = -1;

/// Ingress for a CPython `int` whose value the caller has already extracted
/// as a C `long long` (`PyLong_AsLongLongAndOverflow`, *after* that call's own
/// overflow check). Writes the D-141 encoded word to `out` and returns `0`, or
/// returns `-1` without writing when `value` falls outside the inline-integer
/// range `[-2^62, 2^62-1]`, which the caller reports as `OverflowError`.
///
/// Not `pycc_rt_int_from_i64`: that function *promotes* an out-of-range value
/// to a bigint, and a bigint argument is precisely what the compiled body
/// cannot accept today.
///
/// # Safety
///
/// `out` must be non-null and point to a writable, aligned `i64`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_ext_int_encode(value: i64, out: *mut i64) -> i32 {
    match fits_smallint(value) {
        Some(encoded) => {
            unsafe { *out = encoded };
            0
        }
        None => -1,
    }
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

    /// The inline-integer range's exact edges, on both sides, plus the two
    /// values one step outside it. `i64::MAX` is deliberately included: an
    /// ingress check written against `i64` instead of the inline range would
    /// accept it.
    #[test]
    fn int_encode_accepts_the_inline_range_and_rejects_everything_outside_it() {
        const MAX_INLINE: i64 = (1 << 62) - 1;
        const MIN_INLINE: i64 = -(1 << 62);
        for value in [0, 1, -1, 42, MAX_INLINE, MIN_INLINE] {
            let mut out = 0;
            assert_eq!(unsafe { pycc_rt_ext_int_encode(value, &mut out) }, 0);
            assert_eq!(pycc_rt_ext_int_classify(out), PYCC_EXT_INT_SMALLINT);
            assert_eq!(pycc_rt_ext_int_decode(out), value);
        }
        for value in [MAX_INLINE + 1, MIN_INLINE - 1, i64::MAX, i64::MIN] {
            let mut out = 0xdead;
            assert_eq!(
                unsafe { pycc_rt_ext_int_encode(value, &mut out) },
                -1,
                "{value} is outside [-2^62, 2^62-1] and must be an OverflowError, not a bigint"
            );
            assert_eq!(out, 0xdead, "a rejected value must not write through `out`");
        }
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
    /// guard: renumbering a tag without updating `src/ext/pycc_ext_module.c`
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
