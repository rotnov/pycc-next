//! #1210: the five `extern "C"` entry points for `<< >> & | ^`, called
//! through the `rlib` exactly as generated code links them from the
//! `staticlib`.
//!
//! The unit tests in `crates/pycc_rt/src/int_bitwise.rs` own the semantics;
//! this file pins the exported ABI -- each symbol's name, its two encoded
//! `i64` words in and one out, and the D-173 sentinel-and-pending-exception
//! contract of the two raising shifts -- from outside the crate.
//! It is also where the diff-coverage gate sees these five functions: the
//! `--workspace` export keeps an integration-test binary's copy of a
//! `pycc_rt` symbol and discards the unit-test binary's counters for it
//! (`docs/TESTING.md`, "A runtime function an integration test links is
//! measured from that binary").

use pycc_rt::{
    EXCEPTION_TYPE_VALUE_ERROR, pycc_rt_exception_clear, pycc_rt_ext_pending_type, pycc_rt_int_and,
    pycc_rt_int_lshift, pycc_rt_int_or, pycc_rt_int_rshift, pycc_rt_int_xor,
};

/// D-061's inline smallint word for `value`.
fn small(value: i64) -> i64 {
    (value << 1) | 1
}

/// D-141's `True` marker.
const TRUE: i64 = 6;

#[test]
fn every_entry_point_computes_its_operator() {
    pycc_rt_exception_clear();
    assert_eq!(pycc_rt_int_lshift(small(3), small(4)), small(48));
    assert_eq!(pycc_rt_int_rshift(small(-49), small(4)), small(-4));
    assert_eq!(pycc_rt_int_and(small(12), small(10)), small(8));
    assert_eq!(pycc_rt_int_or(small(12), small(10)), small(14));
    assert_eq!(pycc_rt_int_xor(small(12), small(10)), small(6));
    assert_eq!(pycc_rt_int_and(TRUE, TRUE), TRUE);
    assert_eq!(pycc_rt_ext_pending_type(), -1, "nothing raised");
}

#[test]
fn a_negative_count_returns_the_sentinel_with_value_error_pending() {
    for shift in [pycc_rt_int_lshift, pycc_rt_int_rshift] {
        pycc_rt_exception_clear();
        assert_eq!(
            shift(small(1), small(-1)),
            small(0),
            "the D-173 sentinel int 0"
        );
        assert_eq!(
            pycc_rt_ext_pending_type(),
            i32::from(EXCEPTION_TYPE_VALUE_ERROR)
        );
        pycc_rt_exception_clear();
    }
}
