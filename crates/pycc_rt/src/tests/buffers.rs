//! memoryview/buffer element access, length, allocation and free.

use super::*;

/// Part C of #1038 (#1065): a heap bigint word, the operand every
/// converted site below rejects.
pub(crate) fn a_bigint_word() -> i64 {
    tag_bigint(bigint_from_i128(1i128 << 80))
}

/// Part C of #1038 (#1065): asserts the pending exception is the
/// converted `OverflowError`, carries `context`, and does not carry the
/// retired `pycc_rt: ` panic prefix (the Part B convention).
pub(crate) fn assert_overflow_raised(context: &str) {
    let (tag, message) = pending_tag_and_message();
    assert_eq!(tag, EXCEPTION_TYPE_OVERFLOW_ERROR, "{message}");
    assert!(
        message == format!("{context} a bigint-valued `int` is not supported yet"),
        "{message}"
    );
    assert!(!message.contains("pycc_rt: "), "{message}");
}

/// Part 2 of #1027: the Rust `PyccExtBufferView` is the same two words,
/// in the same order and at the same offsets, as the C struct in
/// `src/ext/module/` -- `{ void *ptr; long long len; }`. A
/// silent disagreement here would misread every element, so it is pinned
/// rather than assumed.
#[test]
fn ext_buffer_view_layout_matches_the_c_struct() {
    assert_eq!(
        core::mem::size_of::<PyccExtBufferView>(),
        core::mem::size_of::<*mut core::ffi::c_void>() + core::mem::size_of::<i64>()
    );
    assert_eq!(core::mem::align_of::<PyccExtBufferView>(), 8);
    let storage = [0.0f64; 1];
    let view = PyccExtBufferView {
        ptr: storage.as_ptr() as *mut core::ffi::c_void,
        len: 1,
    };
    let base = &view as *const PyccExtBufferView as usize;
    assert_eq!(&view.ptr as *const _ as usize - base, 0);
    assert_eq!(
        &view.len as *const _ as usize - base,
        core::mem::size_of::<*mut core::ffi::c_void>()
    );
}

/// Builds a view over `storage` for the buffer-read tests below.
fn buffer_view_over(storage: &[f64]) -> PyccExtBufferView {
    PyccExtBufferView {
        ptr: storage.as_ptr() as *mut core::ffi::c_void,
        len: storage.len() as i64,
    }
}

#[test]
fn buffer_f64_get_reads_every_in_range_element() {
    pycc_rt_exception_clear();
    let storage = [1.5f64, 2.5, 3.5];
    let view = buffer_view_over(&storage);
    unsafe {
        assert_eq!(buffer_f64_get(&view, 0), 1.5);
        assert_eq!(buffer_f64_get(&view, 1), 2.5);
        assert_eq!(buffer_f64_get(&view, 2), 3.5);
    }
    assert_eq!(pycc_rt_exception_active(), 0);
}

#[test]
fn buffer_f64_get_past_the_end_sets_the_index_error_flag() {
    // D-173: the pending-exception flag plus a type-valid sentinel, not
    // a panic across the FFI boundary.
    pycc_rt_exception_clear();
    let storage = [1.5f64, 2.5];
    let view = buffer_view_over(&storage);
    let result = unsafe { buffer_f64_get(&view, 2) };
    assert_eq!(result, 0.0);
    assert_eq!(pycc_rt_exception_active(), 1);
    assert_eq!(
        pycc_rt_ext_pending_type(),
        i32::from(EXCEPTION_TYPE_INDEX_ERROR)
    );
    pycc_rt_exception_clear();
}

#[test]
fn buffer_f64_get_rejects_negative_indices_with_exception_flag() {
    // D-108: `b[-1]` is refused rather than resolved from the end.
    pycc_rt_exception_clear();
    let storage = [1.5f64, 2.5];
    let view = buffer_view_over(&storage);
    let result = unsafe { buffer_f64_get(&view, -1) };
    assert_eq!(result, 0.0);
    assert_eq!(pycc_rt_exception_active(), 1);
    pycc_rt_exception_clear();
}

#[test]
fn buffer_f64_get_on_an_empty_view_refuses_index_zero() {
    pycc_rt_exception_clear();
    let view = PyccExtBufferView {
        ptr: core::ptr::null_mut(),
        len: 0,
    };
    let result = unsafe { buffer_f64_get(&view, 0) };
    assert_eq!(result, 0.0);
    assert_eq!(pycc_rt_exception_active(), 1);
    pycc_rt_exception_clear();
}

#[test]
fn buffer_f64_set_writes_every_in_range_element() {
    pycc_rt_exception_clear();
    let mut storage = [0.0f64; 3];
    let view = PyccExtBufferView {
        ptr: storage.as_mut_ptr() as *mut core::ffi::c_void,
        len: 3,
    };
    unsafe {
        buffer_f64_set(&view, 0, 1.5);
        buffer_f64_set(&view, 1, 2.5);
        buffer_f64_set(&view, 2, 3.5);
    }
    assert_eq!(storage, [1.5, 2.5, 3.5]);
    assert_eq!(pycc_rt_exception_active(), 0);
}

/// The four properties the `--ext` wrapper checks -- writable, one
/// dimension, C-contiguous, format `"d"` -- say nothing about the
/// alignment of the exporter's storage.
/// `memoryview(bytearray(17))[1:].cast("d")` passes all four on CPython
/// and reports a data address that is `1 mod 8`, so both accessors have
/// to use the unaligned primitives; a plain `f64` dereference there is
/// undefined behavior. An unaligned exporter must keep working, so this
/// asserts a round trip rather than a refusal.
#[test]
fn buffer_f64_round_trips_through_a_misaligned_view() {
    pycc_rt_exception_clear();
    #[repr(align(8))]
    struct AlignedBytes([u8; 24]);
    let mut storage = AlignedBytes([0u8; 24]);
    // One byte past an 8-byte-aligned base, so every element address is
    // `1 mod 8` -- the same residue the `bytearray(17)[1:]` witness has.
    let base = unsafe { storage.0.as_mut_ptr().add(1) };
    assert_eq!(base as usize % core::mem::align_of::<f64>(), 1);
    let view = PyccExtBufferView {
        ptr: base as *mut core::ffi::c_void,
        len: 2,
    };
    unsafe {
        buffer_f64_set(&view, 0, 1.5);
        buffer_f64_set(&view, 1, -2.25);
        assert_eq!(buffer_f64_get(&view, 0), 1.5);
        assert_eq!(buffer_f64_get(&view, 1), -2.25);
    }
    // The writes landed at the misaligned offsets themselves, not at a
    // rounded-down aligned address: byte 0 is untouched.
    assert_eq!(storage.0[0], 0);
    assert_eq!(storage.0[1..9], 1.5f64.to_ne_bytes());
    assert_eq!(storage.0[9..17], (-2.25f64).to_ne_bytes());
    assert_eq!(pycc_rt_exception_active(), 0);
}

#[test]
fn buffer_f64_set_past_the_end_sets_the_index_error_flag_and_writes_nothing() {
    // D-173's flag, and the "returns without dereferencing `ptr`" half
    // of the contract: the storage is untouched.
    pycc_rt_exception_clear();
    let mut storage = [7.0f64, 8.0];
    let view = PyccExtBufferView {
        ptr: storage.as_mut_ptr() as *mut core::ffi::c_void,
        len: 2,
    };
    unsafe { buffer_f64_set(&view, 2, 99.0) };
    assert_eq!(storage, [7.0, 8.0]);
    assert_eq!(pycc_rt_exception_active(), 1);
    assert_eq!(
        pycc_rt_ext_pending_type(),
        i32::from(EXCEPTION_TYPE_INDEX_ERROR)
    );
    pycc_rt_exception_clear();
}

#[test]
fn buffer_f64_set_rejects_negative_indices_with_exception_flag() {
    // D-108: `b[-1] = v` is refused rather than resolved from the end,
    // exactly as the load refuses `b[-1]`.
    pycc_rt_exception_clear();
    let mut storage = [7.0f64, 8.0];
    let view = PyccExtBufferView {
        ptr: storage.as_mut_ptr() as *mut core::ffi::c_void,
        len: 2,
    };
    unsafe { buffer_f64_set(&view, -1, 99.0) };
    assert_eq!(storage, [7.0, 8.0]);
    assert_eq!(pycc_rt_exception_active(), 1);
    pycc_rt_exception_clear();
}

#[test]
fn buffer_f64_set_on_an_empty_view_refuses_index_zero() {
    pycc_rt_exception_clear();
    let view = PyccExtBufferView {
        ptr: core::ptr::null_mut(),
        len: 0,
    };
    unsafe { buffer_f64_set(&view, 0, 1.0) };
    assert_eq!(pycc_rt_exception_active(), 1);
    pycc_rt_exception_clear();
}

/// The `extern "C"` wrapper generated code actually calls, exercised
/// directly for the same reason the load's is.
#[test]
fn pycc_rt_buffer_f64_set_wrapper_writes_and_raises() {
    pycc_rt_exception_clear();
    let mut storage = [0.0f64, 0.0];
    let view = PyccExtBufferView {
        ptr: storage.as_mut_ptr() as *mut core::ffi::c_void,
        len: 2,
    };
    unsafe {
        pycc_rt_buffer_f64_set(&view, 1, 4.25);
        assert_eq!(pycc_rt_exception_active(), 0);
        pycc_rt_buffer_f64_set(&view, 99, 1.0);
    }
    assert_eq!(storage, [0.0, 4.25]);
    assert_eq!(pycc_rt_exception_active(), 1);
    pycc_rt_exception_clear();
}

/// The `extern "C"` wrapper is what generated code actually calls, so it
/// is exercised directly rather than only through its private half.
#[test]
fn pycc_rt_buffer_f64_get_wrapper_reads_and_raises() {
    pycc_rt_exception_clear();
    let storage = [4.25f64, 8.5];
    let view = buffer_view_over(&storage);
    unsafe {
        assert_eq!(pycc_rt_buffer_f64_get(&view, 1), 8.5);
        assert_eq!(pycc_rt_exception_active(), 0);
        assert_eq!(pycc_rt_buffer_f64_get(&view, 99), 0.0);
    }
    assert_eq!(pycc_rt_exception_active(), 1);
    pycc_rt_exception_clear();
}

/// `len(b)` is the `len` word verbatim, in elements, and never raises.
#[test]
fn pycc_rt_buffer_len_returns_the_element_count_and_never_raises() {
    pycc_rt_exception_clear();
    let storage = [1.0f64, 2.0, 3.0, 4.0];
    let view = buffer_view_over(&storage);
    let empty = PyccExtBufferView {
        ptr: core::ptr::null_mut(),
        len: 0,
    };
    unsafe {
        assert_eq!(pycc_rt_buffer_len(&view), 4);
        assert_eq!(pycc_rt_buffer_len(&empty), 0);
    }
    assert_eq!(pycc_rt_exception_active(), 0);
}

/// The allocator pairing of #1165, pinned as a round trip: allocate,
/// write every element through the same `buffer_f64_set` a compiled body
/// uses, read them all back, and free. A mismatched deallocation layout
/// is undefined behavior that no test notices by accident, so this test
/// exists to give the debug allocator a well-formed pairing to check.
#[test]
fn buffer_alloc_untag_len_raises_instead_of_aborting_on_a_bigint_length() {
    // #1166 round 8's P1. `ndarray(2 ** 62 - 1)` on a built `--ext`
    // module used to decode its length through
    // `pycc_rt_int_untag_checked`, whose `panic!` becomes a process
    // abort at its own `extern "C"` boundary: the hosted repro exited
    // 134, killing the host CPython interpreter.
    //
    // Called through the `pub extern "C"` wrapper rather than the
    // private function -- the opposite of this module's convention for
    // `int_pow`/`int_to_float`, and deliberately so. That convention
    // exists because those wrappers *could* abort; this one cannot, and
    // calling it is what proves that at the exact boundary the defect
    // lived on. Both arms are asserted in both directions: the value and
    // the pending state.
    pycc_rt_exception_clear();
    assert_eq!(pycc_rt_buffer_alloc_untag_len(tag_smallint(4)), 4);
    assert_eq!(pycc_rt_exception_active(), 0);

    // D-086: a bool marker is a valid length and still decodes to 0/1.
    assert_eq!(pycc_rt_buffer_alloc_untag_len(tag_smallint(0)), 0);
    assert_eq!(pycc_rt_exception_active(), 0);

    let sentinel = pycc_rt_buffer_alloc_untag_len(a_bigint_word());
    assert_eq!(sentinel, 0, "the refusal returns the type-valid sentinel");
    assert_overflow_raised("sizing a buffer with");
    pycc_rt_exception_clear();
}

#[test]
fn a_buffer_allocation_of_the_refusal_sentinel_would_itself_succeed() {
    // Why the decoder's raise is not self-guarding, and therefore why
    // `pycc_codegen` must branch away between the decode and the
    // allocator: the sentinel `0` is a *valid* length. The allocator
    // accepts it, raises nothing, and hands back a live view -- which on
    // the refusal path no `MirStmt::Assign` would ever store into the
    // frame's owned slot. This is the leak the emitted guard prevents,
    // pinned as a runtime property so the codegen ordering test above
    // has something to be an ordering *of*.
    pycc_rt_exception_clear();
    let view = pycc_rt_buffer_f64_alloc(0);
    assert!(!view.is_null(), "a zero length is admitted, not refused");
    assert_eq!(pycc_rt_exception_active(), 0);
    unsafe {
        assert_eq!(pycc_rt_buffer_len(view), 0);
        pycc_rt_buffer_f64_free(view);
    }
}

#[test]
fn buffer_f64_alloc_raises_instead_of_aborting_on_an_unreservable_length() {
    // #1166 round 8's second abort door, found while writing the hosted
    // arm for the first: `ndarray(2 ** 62 - 1)` needs no bigint at all --
    // the length is an ordinary inline smallint that decodes fine and
    // then overflows `Vec`'s capacity, whose `panic!` this
    // `extern "C" fn` turns into a process abort. Measured at exit 134
    // before the fix.
    //
    // `i64::MAX` is chosen so `try_reserve_exact` reports capacity
    // overflow arithmetically, without attempting a real allocation.
    pycc_rt_exception_clear();
    let view = pycc_rt_buffer_f64_alloc(i64::MAX);
    assert!(view.is_null(), "the refusal must not hand back a view");
    let (tag, message) = pending_tag_and_message();
    assert_eq!(tag, EXCEPTION_TYPE_RUNTIME_ERROR, "{message}");
    assert!(message.contains("cannot be allocated"), "{message}");
    // The deviation is stated in the message rather than in the class,
    // because this runtime has no `MemoryError` tag to raise.
    assert!(message.contains("MemoryError"), "{message}");
    pycc_rt_exception_clear();

    // The other direction: an ordinary length is unaffected by the new
    // fallible reservation, and still zero-fills.
    let view = pycc_rt_buffer_f64_alloc(3);
    assert!(!view.is_null());
    assert_eq!(pycc_rt_exception_active(), 0);
    unsafe {
        assert_eq!(pycc_rt_buffer_len(view), 3);
        assert_eq!(pycc_rt_buffer_f64_get(view, 2), 0.0);
        pycc_rt_buffer_f64_free(view);
    }
}

#[test]
fn buffer_f64_alloc_free_round_trips_through_the_element_helpers() {
    pycc_rt_exception_clear();
    let view = pycc_rt_buffer_f64_alloc(4);
    assert!(!view.is_null());
    assert_eq!(pycc_rt_exception_active(), 0);
    unsafe {
        assert_eq!(pycc_rt_buffer_len(view), 4);
        // The zero-fill deviation from `numpy.ndarray(n)`, asserted
        // before any store.
        for i in 0..4 {
            assert_eq!(pycc_rt_buffer_f64_get(view, i), 0.0);
        }
        for i in 0..4 {
            pycc_rt_buffer_f64_set(view, i, 1.5 + i as f64);
        }
        for i in 0..4 {
            assert_eq!(pycc_rt_buffer_f64_get(view, i), 1.5 + i as f64);
        }
        pycc_rt_buffer_f64_free(view);
    }
    assert_eq!(pycc_rt_exception_active(), 0);
}

/// `ndarray(0)` is admitted: a zero-length view whose `len` is `0`, which
/// every existing helper already handles, and whose free is the same
/// `Box<[f64]>` round trip.
#[test]
fn buffer_f64_alloc_admits_a_zero_length_request() {
    pycc_rt_exception_clear();
    let view = pycc_rt_buffer_f64_alloc(0);
    assert!(!view.is_null());
    unsafe {
        assert_eq!(pycc_rt_buffer_len(view), 0);
        pycc_rt_buffer_f64_free(view);
    }
    assert_eq!(pycc_rt_exception_active(), 0);
}

/// `ndarray(-1)` raises `ValueError` through D-173's pending-exception
/// protocol and returns null, matching CPython's own
/// `ValueError: negative dimensions are not allowed`.
#[test]
fn buffer_f64_alloc_refuses_a_negative_length() {
    pycc_rt_exception_clear();
    let view = pycc_rt_buffer_f64_alloc(-1);
    assert!(view.is_null());
    assert_eq!(pycc_rt_exception_active(), 1);
    assert_eq!(
        pycc_rt_ext_pending_type(),
        i32::from(EXCEPTION_TYPE_VALUE_ERROR)
    );
    pycc_rt_exception_clear();
}

/// The null arm of the free is a no-op, which is what makes a generated
/// epilogue safe over a slot a path never assigned.
#[test]
fn buffer_f64_free_is_a_no_op_on_null() {
    pycc_rt_exception_clear();
    unsafe { pycc_rt_buffer_f64_free(core::ptr::null_mut()) };
    assert_eq!(pycc_rt_exception_active(), 0);
}
