//! #1054: the live-`PyStrObj` counter pairs construction with the freeing
//! decref.
//!
//! **This file must hold exactly one test, and nothing added to it may
//! construct or release a `PyStrObj`.** `STR_LIVE`, the counter behind
//! [`pycc_rt::pycc_rt_str_live_objects`], is a single process-wide atomic, so
//! an absolute-value assertion about it is only sound in a process where no
//! other code is moving it. Cargo's isolation unit is the test *binary*:
//! threads interleave within one binary, never across two. Hosting the probe
//! in its own integration-test file therefore gives it the process to itself
//! and makes the assertions below race-free by construction -- which the
//! equivalent unit test inside `crates/pycc_rt/src/lib.rs` was not, because
//! dozens of sibling `#[test]` functions there allocate and free `str`
//! objects concurrently under the default multi-threaded harness. A lock
//! would not have fixed that: it would have had to be taken by every one of
//! those siblings, and a future test that forgot it would reintroduce the
//! flake silently.
//!
//! `cargo test -p pycc_rt --test str_live_objects -- --list` is the check
//! that this file still holds one test only.

use pycc_rt::{
    pycc_rt_str_decref, pycc_rt_str_from_literal, pycc_rt_str_incref, pycc_rt_str_live_objects,
};

/// `pycc_rt_str_live_objects` tracks the *net* number of live `PyStrObj`
/// allocations -- one up per construction, one down only at the decref that
/// actually frees. Because this test owns the process (see the module
/// comment), the counter starts at zero and every reading below is an exact
/// statement about the calls this test made.
#[test]
fn str_live_objects_counts_construction_and_the_freeing_decref() {
    unsafe {
        assert_eq!(
            pycc_rt_str_live_objects(),
            0,
            "this test owns the process, so nothing has allocated a str yet"
        );

        let s = pycc_rt_str_from_literal(b"leak-probe".as_ptr(), 10);
        assert_eq!(
            pycc_rt_str_live_objects(),
            1,
            "constructing a str must raise the live count by exactly one"
        );

        // A non-freeing decref leaves the count alone: only the release that
        // retires the last reference is an object going away.
        pycc_rt_str_incref(s);
        pycc_rt_str_decref(s);
        assert_eq!(
            pycc_rt_str_live_objects(),
            1,
            "a decref that does not free must not move the counter"
        );

        pycc_rt_str_decref(s);
        assert_eq!(
            pycc_rt_str_live_objects(),
            0,
            "the freeing decref must restore the live count"
        );
    }
}
