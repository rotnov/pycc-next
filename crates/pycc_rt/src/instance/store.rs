//! The `--ext` host's store into, and `del` of, an instance slot (Part 1 of
//! #1443): what the setter of a slot's `Py_tp_getset` descriptor
//! (`src/ext_build/getset.rs`) calls after converting the host value with
//! the parameter row of the boundary table.
//!
//! The slot words carry no kind at run time, so the generated setter passes
//! the slot's kind byte, the same one #1455's copy uses
//! (`src/ext_build/instance_copy.rs`):
//!
//! * `s` -- a `str` slot. The slot owns its `PyStrObj` reference, so the
//!   replaced (or deleted) string is released with
//!   [`crate::pycc_rt_str_decref`], exactly what a compiled `self.s = v`
//!   does before it stores.
//! * `i` -- an `int` slot. The same for a heap bigint word
//!   ([`crate::pycc_rt_bigint_release`]); an inline int or a `bool` marker
//!   is a no-op there.
//! * `o` -- an opaque CPython object, and `w` -- any other word. Nothing is
//!   released: `pycc_rt` keeps no CPython dependency (D-244 rule 2), and
//!   under the leak-only rule (#1092) the reference a compiled object slot
//!   holds may be the very one compiled code still uses, so the compiled
//!   store does not release it either.
//!
//! The new word is stored as given: the setter hands over a reference it
//! owns (a fresh `PyStrObj`, a new CPython reference), or a plain word.

use super::{PyInstanceObj, instance_get_slot_checked, instance_set_slot, read_slot};
use crate::PyStrObj;

/// Releases the reference a slot of kind `kind` held in `word`, per this
/// module's doc comment.
fn release_slot_word(word: i64, kind: u8) {
    match kind {
        // SAFETY: an assigned `str` slot holds a live `PyStrObj` pointer the
        // slot's own reference keeps alive; that reference is retired here.
        b's' => unsafe { crate::pycc_rt_str_decref(word as *mut PyStrObj) },
        b'i' => crate::pycc_rt_bigint_release(word),
        _ => {}
    }
}

/// Stores `word` into `slot`, then releases the replaced word, if any.
/// Storing first keeps the slot from ever naming a released object.
fn store_slot(instance: &PyInstanceObj, slot: i64, kind: u8, word: i64) {
    let old = read_slot(instance, slot);
    instance_set_slot(instance, slot, word);
    if let Some(old) = old {
        release_slot_word(old, kind);
    }
}

/// Un-assigns `slot` and releases its word, returning `true`; or, for a slot
/// not assigned, raises the checked read's own `AttributeError` (CPython's
/// wording for `del` of a missing attribute) and returns `false`.
fn delete_slot(instance: &PyInstanceObj, slot: i64, kind: u8) -> bool {
    let Some(old) = read_slot(instance, slot) else {
        instance_get_slot_checked(instance, slot);
        return false;
    };
    let mut slots = instance.slots.take();
    slots[slot as usize] = None;
    instance.slots.set(slots);
    release_slot_word(old, kind);
    true
}

/// Stores `word` into slot `slot` of `instance`, releasing the replaced
/// word as kind `kind` calls for (see this module's doc comment). An
/// out-of-range `slot` is the same internal-error abort every slot accessor
/// has.
///
/// # Safety
/// `instance` must be a live `PyInstanceObj` pointer, and `word` a value of
/// the slot's kind whose reference, if any, the slot now owns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_ext_instance_store_slot(
    instance: *mut PyInstanceObj,
    slot: i64,
    kind: u8,
    word: i64,
) {
    store_slot(unsafe { &*instance }, slot, kind, word);
}

/// `del instance.<slot>`: returns `0` after un-assigning the slot and
/// releasing its word as kind `kind` calls for, or `-1` with CPython's
/// ``AttributeError: '<C>' object has no attribute '<x>'`` pending when the
/// slot is not assigned. The shim turns that into the CPython exception.
///
/// # Safety
/// `instance` must be a live `PyInstanceObj` pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_ext_instance_delete_slot(
    instance: *mut PyInstanceObj,
    slot: i64,
    kind: u8,
) -> i32 {
    if delete_slot(unsafe { &*instance }, slot, kind) {
        0
    } else {
        -1
    }
}

#[cfg(test)]
mod tests {
    use super::super::{
        pycc_rt_instance_get_slot, pycc_rt_instance_get_slot_checked, pycc_rt_instance_new,
        pycc_rt_instance_set_slot,
    };
    use super::*;
    use crate::int_encoding::{BigIntObj, tag_bigint, tag_smallint};

    static LAYOUT: &[u8] = b"Conf\0a\0b";

    fn new_instance() -> *mut PyInstanceObj {
        unsafe { pycc_rt_instance_new(2, LAYOUT.as_ptr(), LAYOUT.len()) }
    }

    fn store(instance: *mut PyInstanceObj, slot: i64, kind: u8, word: i64) {
        unsafe { pycc_rt_ext_instance_store_slot(instance, slot, kind, word) }
    }

    fn delete(instance: *mut PyInstanceObj, slot: i64, kind: u8) -> i32 {
        unsafe { pycc_rt_ext_instance_delete_slot(instance, slot, kind) }
    }

    fn str_rc(s: *mut PyStrObj) -> u32 {
        unsafe { &*s }.rc.get()
    }

    fn bigint_rc(word: i64) -> u32 {
        unsafe { &*(word as *const BigIntObj) }.rc.get()
    }

    #[test]
    fn a_store_into_an_unassigned_slot_assigns_it_and_releases_nothing() {
        crate::pycc_rt_exception_clear();
        let instance = new_instance();
        let s = crate::new_pystr(b"new");
        store(instance, 0, b's', s as i64);
        assert_eq!(str_rc(s), 1);
        assert_eq!(
            unsafe { pycc_rt_instance_get_slot_checked(instance, 0) },
            s as i64
        );
        assert_eq!(crate::pycc_rt_exception_active(), 0);
    }

    #[test]
    fn a_str_store_releases_the_replaced_string() {
        let instance = new_instance();
        let old = crate::new_pystr(b"old");
        unsafe { crate::pycc_rt_str_incref(old) };
        unsafe { pycc_rt_instance_set_slot(instance, 0, old as i64) };
        let new = crate::new_pystr(b"new");
        store(instance, 0, b's', new as i64);
        assert_eq!(str_rc(old), 1);
        assert_eq!(str_rc(new), 1);
        assert_eq!(
            unsafe { pycc_rt_instance_get_slot(instance, 0) },
            new as i64
        );
    }

    #[test]
    fn an_int_store_releases_a_heap_bigint_and_not_an_inline_int() {
        let instance = new_instance();
        let big = tag_bigint(BigIntObj::new(false, vec![1, 2, 3]));
        crate::pycc_rt_bigint_retain(big);
        unsafe { pycc_rt_instance_set_slot(instance, 0, big) };
        store(instance, 0, b'i', tag_smallint(1));
        assert_eq!(bigint_rc(big), 1);
        store(instance, 0, b'i', tag_smallint(2));
        assert_eq!(
            unsafe { pycc_rt_instance_get_slot(instance, 0) },
            tag_smallint(2)
        );
    }

    #[test]
    fn an_object_or_word_store_releases_nothing() {
        let instance = new_instance();
        // A `PyStrObj` stands in for a referenced word: a kind other than
        // `s` must leave its count alone.
        let held = crate::new_pystr(b"held");
        unsafe { pycc_rt_instance_set_slot(instance, 0, held as i64) };
        unsafe { pycc_rt_instance_set_slot(instance, 1, held as i64) };
        store(instance, 0, b'o', 0x10);
        store(instance, 1, b'w', 0x20);
        assert_eq!(str_rc(held), 1);
        assert_eq!(unsafe { pycc_rt_instance_get_slot(instance, 0) }, 0x10);
        assert_eq!(unsafe { pycc_rt_instance_get_slot(instance, 1) }, 0x20);
    }

    #[test]
    fn a_delete_unassigns_the_slot_and_releases_its_word() {
        crate::pycc_rt_exception_clear();
        let instance = new_instance();
        let s = crate::new_pystr(b"gone");
        unsafe { crate::pycc_rt_str_incref(s) };
        store(instance, 1, b's', s as i64);
        assert_eq!(delete(instance, 1, b's'), 0);
        assert_eq!(crate::pycc_rt_exception_active(), 0);
        assert_eq!(str_rc(s), 1);
        assert_eq!(unsafe { pycc_rt_instance_get_slot_checked(instance, 1) }, 0);
        let (_, message) = crate::tests::pending_tag_and_message();
        assert_eq!(message, "'Conf' object has no attribute 'b'");
        crate::pycc_rt_exception_clear();
    }

    #[test]
    fn a_delete_of_an_unassigned_slot_raises_attribute_error() {
        crate::pycc_rt_exception_clear();
        let instance = new_instance();
        assert_eq!(delete(instance, 0, b'w'), -1);
        let (_, message) = crate::tests::pending_tag_and_message();
        assert_eq!(message, "'Conf' object has no attribute 'a'");
        crate::pycc_rt_exception_clear();
        // A second `del` after a first one raises again, as in CPython.
        store(instance, 0, b'w', 5);
        assert_eq!(delete(instance, 0, b'w'), 0);
        assert_eq!(delete(instance, 0, b'w'), -1);
        crate::pycc_rt_exception_clear();
    }
}
