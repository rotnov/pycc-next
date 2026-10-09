//! #1388: the instance-slot entry points generated code calls --
//! `pycc_rt_instance_new` with its layout descriptor and the checked read
//! `pycc_rt_instance_get_slot_checked` -- and #1435's `--ext` carrier
//! accessors, #1455's `copy.copy` clone and #1443's slot store and delete the
//! shim calls, exercised through the `rlib`.
//!
//! `crates/pycc_rt/src/instance.rs`'s unit tests own the semantics; this
//! file pins them from outside the crate, which is also where the
//! diff-coverage gate measures these exported functions (`docs/TESTING.md`,
//! "A runtime function an integration test links is measured from that
//! binary").

use std::ffi::c_void;

use pycc_rt::{
    EXCEPTION_TYPE_ATTRIBUTE_ERROR, pycc_rt_exception_active, pycc_rt_exception_clear,
    pycc_rt_ext_instance_carrier, pycc_rt_ext_instance_class, pycc_rt_ext_instance_copy,
    pycc_rt_ext_instance_delete_slot, pycc_rt_ext_instance_set_carrier,
    pycc_rt_ext_instance_store_slot, pycc_rt_ext_pending_message, pycc_rt_ext_pending_type,
    pycc_rt_instance_get_slot, pycc_rt_instance_get_slot_checked, pycc_rt_instance_new,
    pycc_rt_instance_set_slot,
};

/// The pending exception's tag and message; clears it afterwards so the
/// thread-local state never leaks into the next test on this thread.
fn take_pending() -> (i32, String) {
    assert_eq!(pycc_rt_exception_active(), 1, "an exception is pending");
    let tag = pycc_rt_ext_pending_type();
    let mut len = 0usize;
    let bytes = unsafe { pycc_rt_ext_pending_message(&mut len) };
    assert!(!bytes.is_null());
    let message = String::from_utf8(unsafe { std::slice::from_raw_parts(bytes, len) }.to_vec())
        .expect("the message is UTF-8");
    pycc_rt_exception_clear();
    (tag, message)
}

#[test]
fn a_checked_read_names_the_class_and_slot_from_the_descriptor() {
    static LAYOUT: &[u8] = b"Point\0x\0y";
    pycc_rt_exception_clear();
    let point = unsafe { pycc_rt_instance_new(2, LAYOUT.as_ptr(), LAYOUT.len()) };
    unsafe { pycc_rt_instance_set_slot(point, 0, 7) };
    assert_eq!(unsafe { pycc_rt_instance_get_slot_checked(point, 0) }, 7);
    assert_eq!(pycc_rt_exception_active(), 0);
    // Slot 1 is unassigned: the checked read raises and returns `0`.
    assert_eq!(unsafe { pycc_rt_instance_get_slot_checked(point, 1) }, 0);
    let (tag, message) = take_pending();
    assert_eq!(tag, i32::from(EXCEPTION_TYPE_ATTRIBUTE_ERROR));
    assert_eq!(message, "'Point' object has no attribute 'y'");
    // The unchecked read releases-before-store relies on stays silent.
    assert_eq!(unsafe { pycc_rt_instance_get_slot(point, 1) }, 0);
    assert_eq!(pycc_rt_exception_active(), 0);
}

#[test]
fn an_assigned_zero_word_reads_without_raising() {
    static LAYOUT: &[u8] = b"Flag\0on";
    pycc_rt_exception_clear();
    let flag = unsafe { pycc_rt_instance_new(1, LAYOUT.as_ptr(), LAYOUT.len()) };
    unsafe { pycc_rt_instance_set_slot(flag, 0, 0) };
    assert_eq!(unsafe { pycc_rt_instance_get_slot_checked(flag, 0) }, 0);
    assert_eq!(pycc_rt_exception_active(), 0);
}

#[test]
fn a_null_descriptor_still_raises_with_placeholder_names() {
    pycc_rt_exception_clear();
    let bare = unsafe { pycc_rt_instance_new(1, std::ptr::null(), 0) };
    assert_eq!(unsafe { pycc_rt_instance_get_slot_checked(bare, 0) }, 0);
    let (tag, message) = take_pending();
    assert_eq!(tag, i32::from(EXCEPTION_TYPE_ATTRIBUTE_ERROR));
    assert_eq!(message, "'?' object has no attribute '?'");
}

/// #1435: the shim reads the run-time class from descriptor field 0 and
/// records, reads back and clears the weak carrier pointer.
#[test]
fn the_ext_carrier_accessors_round_trip() {
    static LAYOUT: &[u8] = b"Q\0n";
    let q = unsafe { pycc_rt_instance_new(1, LAYOUT.as_ptr(), LAYOUT.len()) };
    let mut len = usize::MAX;
    let name = unsafe { pycc_rt_ext_instance_class(q, &mut len) };
    assert_eq!(unsafe { std::slice::from_raw_parts(name, len) }, b"Q");
    assert!(unsafe { pycc_rt_ext_instance_carrier(q) }.is_null());
    let mut host = 0u8;
    let carrier = (&mut host as *mut u8).cast::<c_void>();
    unsafe { pycc_rt_ext_instance_set_carrier(q, carrier) };
    assert_eq!(unsafe { pycc_rt_ext_instance_carrier(q) }, carrier);
    unsafe { pycc_rt_ext_instance_set_carrier(q, std::ptr::null_mut()) };
    assert!(unsafe { pycc_rt_ext_instance_carrier(q) }.is_null());
}

/// #1455: the shim's `__copy__` clone copies the slot words into a new
/// instance with no carrier, keeps an unassigned slot unassigned, and
/// answers null for a kind string that does not match the slot count.
#[test]
fn the_ext_instance_copy_clones_the_slots() {
    static LAYOUT: &[u8] = b"Q\0n\0m";
    let q = unsafe { pycc_rt_instance_new(2, LAYOUT.as_ptr(), LAYOUT.len()) };
    unsafe { pycc_rt_instance_set_slot(q, 0, 41) };
    let mut host = 0u8;
    unsafe { pycc_rt_ext_instance_set_carrier(q, (&mut host as *mut u8).cast::<c_void>()) };
    assert!(unsafe { pycc_rt_ext_instance_copy(q, b"w".as_ptr(), 1) }.is_null());
    assert!(unsafe { pycc_rt_ext_instance_copy(std::ptr::null_mut(), b"".as_ptr(), 0) }.is_null());
    let copy = unsafe { pycc_rt_ext_instance_copy(q, b"ww".as_ptr(), 2) };
    assert!(!copy.is_null());
    assert!(unsafe { pycc_rt_ext_instance_carrier(copy) }.is_null());
    assert_eq!(unsafe { pycc_rt_instance_get_slot(copy, 0) }, 41);
    assert_eq!(unsafe { pycc_rt_instance_get_slot_checked(copy, 1) }, 0);
    let (_, message) = take_pending();
    assert_eq!(message, "'Q' object has no attribute 'm'");
    // A slotless class's kind string is empty, and the shim may hand it over
    // as a null pointer; only a non-zero length needs readable bytes.
    let slotless = unsafe { pycc_rt_instance_new(0, LAYOUT.as_ptr(), 1) };
    assert!(!unsafe { pycc_rt_ext_instance_copy(slotless, std::ptr::null(), 0) }.is_null());
    assert!(unsafe { pycc_rt_ext_instance_copy(q, std::ptr::null(), 2) }.is_null());
}

/// #1443: the host's store into a slot assigns it, and its `del` un-assigns
/// it, raising the checked read's `AttributeError` when there is nothing to
/// delete.
#[test]
fn the_ext_slot_store_and_delete_assign_and_unassign() {
    static LAYOUT: &[u8] = b"R\0x";
    let r = unsafe { pycc_rt_instance_new(1, LAYOUT.as_ptr(), LAYOUT.len()) };
    unsafe { pycc_rt_ext_instance_store_slot(r, 0, b'w', 7) };
    assert_eq!(unsafe { pycc_rt_instance_get_slot_checked(r, 0) }, 7);
    assert_eq!(pycc_rt_exception_active(), 0);
    assert_eq!(unsafe { pycc_rt_ext_instance_delete_slot(r, 0, b'w') }, 0);
    assert_eq!(pycc_rt_exception_active(), 0);
    assert_eq!(unsafe { pycc_rt_ext_instance_delete_slot(r, 0, b'w') }, -1);
    let (tag, message) = take_pending();
    assert_eq!(tag, i32::from(EXCEPTION_TYPE_ATTRIBUTE_ERROR));
    assert_eq!(message, "'R' object has no attribute 'x'");
}
