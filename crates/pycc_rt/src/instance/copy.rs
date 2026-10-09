//! The `--ext` host's `copy.copy` of an instance (#1455): a new
//! `PyInstanceObj` holding the same slot words as the original.
//!
//! The C shim's `pycc_ext_instance_copy` (`src/ext/pycc_ext_module.c`), the
//! `__copy__` every carrier type carries, calls this. The slot words carry
//! no kind at run time, so the shim passes a per-class kind string the build
//! computed from the class's flat slot layout (`src/ext_build/instance_copy.rs`),
//! one byte per slot:
//!
//! * `s` -- a `str` slot. The slot owns a `PyStrObj` reference that a later
//!   store releases (`decref_str_slot_before_store`), so the copy takes its
//!   own with [`crate::pycc_rt_str_incref`]. Without it, a store to the
//!   original's slot would free the copy's string.
//! * `i` -- an `int` slot. The same ownership for a heap bigint word
//!   ([`crate::pycc_rt_bigint_retain`]); an inline int is a no-op there.
//! * `o` -- an opaque CPython object. `pycc_rt` keeps no CPython dependency
//!   (D-244 rule 2), so the shim takes that reference itself.
//! * `w` -- anything else the build admits (`float`, `bool`, a leak-only
//!   container, another instance): a plain word copy, with no reference
//!   traffic, because none exists for those words.
//!
//! An unassigned slot stays unassigned, so the copy's checked read raises
//! the same `AttributeError` CPython raises for an attribute its copy lacks.
//! The copy has no carrier yet: the shim links it to the one it allocates.
//! Like every instance, the copy is never freed (D-107, D-154).

use std::cell::Cell;

use super::PyInstanceObj;

/// The clone [`pycc_rt_ext_instance_copy`] allocates, or `None` when
/// `kinds` does not describe `instance`'s slots one byte each.
fn instance_copy(instance: &PyInstanceObj, kinds: &[u8]) -> Option<PyInstanceObj> {
    let slots = instance.slots.take();
    if kinds.len() != slots.len() {
        instance.slots.set(slots);
        return None;
    }
    for (slot, kind) in slots.iter().zip(kinds) {
        match (slot, kind) {
            // SAFETY: an assigned `str` slot holds a live `PyStrObj` pointer:
            // the slot's own reference keeps it alive.
            (Some(word), b's') => unsafe { crate::pycc_rt_str_incref(*word as *mut _) },
            (Some(word), b'i') => crate::pycc_rt_bigint_retain(*word),
            _ => {}
        }
    }
    let copy = PyInstanceObj {
        slots: Cell::new(slots.clone()),
        layout: instance.layout,
        carrier: Cell::new(std::ptr::null_mut()),
    };
    instance.slots.set(slots);
    Some(copy)
}

/// Allocates a copy of `instance`, taking the slot references the
/// `kinds_ptr`/`kinds_len` kind string calls for (see this module's doc
/// comment), and returns it with no carrier. Returns null, allocating
/// nothing, for a null `instance`, a null `kinds_ptr` with a non-zero
/// `kinds_len`, or a kind string whose length is not the instance's slot
/// count; the shim turns that into a `SystemError`.
///
/// # Safety
/// `instance` must be null or a live `PyInstanceObj` pointer, and
/// `kinds_ptr` must be null or point at `kinds_len` readable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_ext_instance_copy(
    instance: *mut PyInstanceObj,
    kinds_ptr: *const u8,
    kinds_len: usize,
) -> *mut PyInstanceObj {
    if instance.is_null() || (kinds_ptr.is_null() && kinds_len != 0) {
        return std::ptr::null_mut();
    }
    let kinds: &[u8] = if kinds_ptr.is_null() {
        &[]
    } else {
        unsafe { std::slice::from_raw_parts(kinds_ptr, kinds_len) }
    };
    instance_copy(unsafe { &*instance }, kinds)
        .map_or(std::ptr::null_mut(), |copy| Box::into_raw(Box::new(copy)))
}

#[cfg(test)]
mod tests {
    use super::super::{
        pycc_rt_instance_get_slot, pycc_rt_instance_get_slot_checked, pycc_rt_instance_new,
        pycc_rt_instance_set_slot,
    };
    use super::*;
    use crate::int_encoding::{BigIntObj, tag_bigint, tag_smallint};
    use crate::{PyStrObj, pycc_rt_ext_instance_carrier, pycc_rt_ext_instance_set_carrier};

    static LAYOUT: &[u8] = b"Conf\0a\0b";

    fn new_instance(slots: i64) -> *mut PyInstanceObj {
        unsafe { pycc_rt_instance_new(slots, LAYOUT.as_ptr(), LAYOUT.len()) }
    }

    fn copy_of(instance: *mut PyInstanceObj, kinds: &[u8]) -> *mut PyInstanceObj {
        unsafe { pycc_rt_ext_instance_copy(instance, kinds.as_ptr(), kinds.len()) }
    }

    fn str_rc(s: *mut PyStrObj) -> u32 {
        unsafe { &*s }.rc.get()
    }

    #[test]
    fn a_null_instance_copies_to_null() {
        assert!(copy_of(std::ptr::null_mut(), b"").is_null());
    }

    #[test]
    fn a_kind_string_of_the_wrong_length_copies_to_null() {
        let instance = new_instance(2);
        assert!(copy_of(instance, b"w").is_null());
        assert!(copy_of(instance, b"www").is_null());
    }

    #[test]
    fn a_null_kind_string_copies_only_a_slotless_instance() {
        let slotless = unsafe { pycc_rt_instance_new(0, LAYOUT.as_ptr(), 4) };
        assert!(
            unsafe { pycc_rt_ext_instance_copy(new_instance(1), std::ptr::null(), 1) }.is_null()
        );
        let copy = unsafe { pycc_rt_ext_instance_copy(slotless, std::ptr::null(), 0) };
        assert!(!copy.is_null());
        assert_eq!(unsafe { &*copy }.layout, unsafe { &*slotless }.layout);
    }

    #[test]
    fn an_unassigned_slot_stays_unassigned_in_the_copy() {
        crate::pycc_rt_exception_clear();
        let instance = new_instance(2);
        unsafe { pycc_rt_instance_set_slot(instance, 0, 5) };
        let copy = copy_of(instance, b"ww");
        assert_eq!(unsafe { pycc_rt_instance_get_slot_checked(copy, 0) }, 5);
        assert_eq!(crate::pycc_rt_exception_active(), 0);
        assert_eq!(unsafe { pycc_rt_instance_get_slot_checked(copy, 1) }, 0);
        let (_, message) = crate::tests::pending_tag_and_message();
        assert_eq!(message, "'Conf' object has no attribute 'b'");
        crate::pycc_rt_exception_clear();
    }

    #[test]
    fn a_str_slot_is_shared_with_its_own_reference() {
        let instance = new_instance(1);
        let s = crate::new_pystr(b"hello");
        unsafe { pycc_rt_instance_set_slot(instance, 0, s as i64) };
        let copy = copy_of(instance, b"s");
        assert_eq!(str_rc(s), 2);
        assert_eq!(unsafe { pycc_rt_instance_get_slot(copy, 0) }, s as i64);
        // What a compiled store to the original's slot does: release the old
        // string, then store the new one. The copy still holds `s`.
        unsafe { crate::pycc_rt_str_decref(s) };
        let replacement = crate::new_pystr(b"bye");
        unsafe { pycc_rt_instance_set_slot(instance, 0, replacement as i64) };
        assert_eq!(str_rc(s), 1);
        assert_eq!(unsafe { &*s }.bytes(), b"hello");
    }

    #[test]
    fn an_int_slot_retains_a_heap_bigint_and_not_an_inline_int() {
        let instance = new_instance(2);
        let big = tag_bigint(BigIntObj::new(false, vec![1, 2, 3]));
        let small = tag_smallint(7);
        unsafe { pycc_rt_instance_set_slot(instance, 0, big) };
        unsafe { pycc_rt_instance_set_slot(instance, 1, small) };
        let copy = copy_of(instance, b"ii");
        assert_eq!(unsafe { &*(big as *const BigIntObj) }.rc.get(), 2);
        assert_eq!(unsafe { pycc_rt_instance_get_slot(copy, 0) }, big);
        assert_eq!(unsafe { pycc_rt_instance_get_slot(copy, 1) }, small);
    }

    #[test]
    fn a_word_or_object_slot_is_copied_verbatim() {
        let instance = new_instance(2);
        unsafe { pycc_rt_instance_set_slot(instance, 0, 0x1234) };
        unsafe { pycc_rt_instance_set_slot(instance, 1, 0x5678) };
        let copy = copy_of(instance, b"wo");
        assert_eq!(unsafe { pycc_rt_instance_get_slot(copy, 0) }, 0x1234);
        assert_eq!(unsafe { pycc_rt_instance_get_slot(copy, 1) }, 0x5678);
        // The copy's slots are its own: a store to one is not seen by the
        // other.
        unsafe { pycc_rt_instance_set_slot(copy, 0, 1) };
        assert_eq!(unsafe { pycc_rt_instance_get_slot(instance, 0) }, 0x1234);
    }

    #[test]
    fn the_copy_shares_the_layout_and_has_no_carrier() {
        let instance = new_instance(2);
        let mut host = 0u8;
        let carrier = (&mut host as *mut u8).cast::<std::ffi::c_void>();
        unsafe { pycc_rt_ext_instance_set_carrier(instance, carrier) };
        let copy = copy_of(instance, b"ww");
        assert!(unsafe { pycc_rt_ext_instance_carrier(copy) }.is_null());
        assert_eq!(unsafe { pycc_rt_ext_instance_carrier(instance) }, carrier);
        assert!(std::ptr::eq(
            unsafe { &*copy }.layout,
            unsafe { &*instance }.layout
        ));
    }
}
