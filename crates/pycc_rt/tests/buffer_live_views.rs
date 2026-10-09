//! #1165: the live artifact-owned-buffer counter pairs
//! `pycc_rt_buffer_f64_alloc` with the free that releases its storage.
//!
//! **This file must hold exactly one test, and nothing added to it may
//! allocate or free a buffer outside that test.** `BUFFER_LIVE`, the counter
//! behind [`pycc_rt::pycc_rt_buffer_live_views`], is a single process-wide
//! atomic, so an absolute-value assertion about it is only sound in a
//! process where no other code is moving it. Cargo's isolation unit is the
//! test *binary*: threads interleave within one binary, never across two.
//! Hosting the probe in its own integration-test file therefore gives it the
//! process to itself and makes the assertions below race-free by
//! construction -- exactly the reasoning `tests/str_live_objects.rs` records
//! for the `str` counter, and exactly why an inline `#[cfg(test)]` module in
//! `crates/pycc_rt/src/lib.rs` could not carry this assertion.
//!
//! `cargo test -p pycc_rt --test buffer_live_views -- --list` is the check
//! that this file still holds one test only.

use pycc_rt::{
    pycc_rt_buffer_f64_alloc, pycc_rt_buffer_f64_free, pycc_rt_buffer_live_views,
    pycc_rt_exception_clear,
};

/// `pycc_rt_buffer_live_views` tracks the *net* number of live artifact-owned
/// buffers -- one up per allocation, one down per free. Because this test
/// owns the process (see the module comment), the counter starts at zero and
/// every reading below is an exact statement about the calls this test made.
#[test]
fn buffer_live_views_counts_allocation_and_the_freeing_release() {
    pycc_rt_exception_clear();
    assert_eq!(
        pycc_rt_buffer_live_views(),
        0,
        "this test owns the process, so nothing has allocated a buffer yet"
    );

    let a = pycc_rt_buffer_f64_alloc(3);
    assert_eq!(
        pycc_rt_buffer_live_views(),
        1,
        "allocating a buffer must raise the live count by exactly one"
    );

    let b = pycc_rt_buffer_f64_alloc(0);
    assert_eq!(
        pycc_rt_buffer_live_views(),
        2,
        "a zero-length buffer is still an allocation the artifact owns"
    );

    // A refused allocation allocates nothing, so it must not move the
    // counter -- the null it returns is what the epilogue's null arm sees.
    let refused = pycc_rt_buffer_f64_alloc(-1);
    assert!(refused.is_null());
    assert_eq!(
        pycc_rt_buffer_live_views(),
        2,
        "a refused allocation must not move the counter"
    );
    pycc_rt_exception_clear();

    // #1166 round 8's second refusal, on the same counter invariant: a
    // length whose storage cannot even be reserved (`i64::MAX` overflows
    // the byte arithmetic before any allocator is asked) used to `panic!`
    // inside `Vec`, which at this `extern "C"` boundary is a process
    // abort rather than an unwind -- `ndarray(2 ** 62 - 1)` on a built
    // `--ext` module exited 134 and took the host interpreter with it.
    // It now raises, and like every other refusal must leave the counter
    // where it found it.
    //
    // It is also the arm that has to be made from *this* binary rather
    // than from `lib.rs`'s own unit tests to be visible to the coverage
    // gate at all; `docs/TESTING.md`'s "A runtime function an integration
    // test links is measured from that binary" states why.
    let unreservable = pycc_rt_buffer_f64_alloc(i64::MAX);
    assert!(unreservable.is_null());
    assert_eq!(
        pycc_rt_buffer_live_views(),
        2,
        "a length that cannot be reserved must not move the counter"
    );
    pycc_rt_exception_clear();

    // The null free is a no-op on the counter as well as on memory.
    unsafe { pycc_rt_buffer_f64_free(core::ptr::null_mut()) };
    assert_eq!(
        pycc_rt_buffer_live_views(),
        2,
        "freeing null must not move the counter"
    );

    unsafe {
        pycc_rt_buffer_f64_free(a);
        pycc_rt_buffer_f64_free(b);
    }
    assert_eq!(
        pycc_rt_buffer_live_views(),
        0,
        "every free must restore the live count"
    );
}
