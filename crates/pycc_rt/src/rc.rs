//! Reference counting of lists and class instances.
//!
//! Both objects keep their count in an `i64` at byte offset [`RC_OFFSET`],
//! the same offset in both, so compiled code increments and decrements it
//! inline with one helper (`pycc_codegen::heap_rc`) and calls
//! [`pycc_rt_list_free`] or [`pycc_rt_instance_free`] only when a decrement
//! reaches zero. A new object starts at `1`, owned by whoever created it.
//!
//! Freeing an object releases every reference it holds: a list its
//! elements, an instance its slots. Those releases can free more objects,
//! so a free runs a worklist rather than recursing -- a long linked chain of
//! instances is freed in constant stack, as CPython's trashcan does.
//!
//! **What a word owns** is named by a *kind byte*, shared with the `--ext`
//! shim's instance copy and slot store (`src/ext_build/instance_copy.rs`):
//!
//! * `s` -- a `str` (`PyStrObj` reference);
//! * `i` -- an `int` word (a heap bigint reference, nothing for an inline
//!   int);
//! * `l` -- a list; `c` -- a class instance (counted here);
//! * anything else -- no reference released here (`float`, `bool`, a dict
//!   or set, which are still leak-only, and an opaque CPython object `o`).
//!   An `o` slot's object is deliberately leaked when the instance is
//!   freed: a function local read from the slot (`y = h.o`) holds the same
//!   pointer without a reference of its own, so releasing the slot's
//!   reference could free an object the frame still uses. It can be
//!   released once object locals own their values (#1499's frame-slot
//!   part).
//!
//! **Cycles are not collected.** CPython's cycle GC frees an unreachable
//! cycle (`a.other = b; b.other = a`); here its objects keep each other's
//! counts above zero and stay allocated (`docs/DEVIATIONS.md`).

use std::sync::atomic::{AtomicI64, Ordering};

use crate::instance::PyInstanceObj;
use crate::list::PyListObj;

/// Byte offset of the reference count in both [`PyListObj`] and
/// [`PyInstanceObj`]; `pycc_codegen` mirrors it.
pub const RC_OFFSET: usize = 32;

/// Net live lists and instances, for leak tests (the
/// `pycc_rt_str_live_objects` counterpart).
pub(crate) static LIST_LIVE: AtomicI64 = AtomicI64::new(0);
pub(crate) static INSTANCE_LIVE: AtomicI64 = AtomicI64::new(0);

/// The number of lists allocated and not yet freed.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_list_live_objects() -> i64 {
    LIST_LIVE.load(Ordering::Relaxed)
}

/// The number of class instances allocated and not yet freed.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_instance_live_objects() -> i64 {
    INSTANCE_LIVE.load(Ordering::Relaxed)
}

/// The count field of an object with the shared header layout.
fn rc_cell<T>(object: *mut T) -> *mut i64 {
    // SAFETY (caller): `object` is a live list or instance, both of which
    // hold an `i64` count at `RC_OFFSET`.
    unsafe { object.cast::<u8>().add(RC_OFFSET).cast::<i64>() }
}

fn incref<T>(object: *mut T) {
    if !object.is_null() {
        unsafe { *rc_cell(object) += 1 };
    }
}

/// Decrements `object`'s count, answering whether it reached zero.
fn decref_reaches_zero<T>(object: *mut T) -> bool {
    if object.is_null() {
        return false;
    }
    let cell = rc_cell(object);
    unsafe {
        *cell -= 1;
        *cell == 0
    }
}

/// An object whose count reached zero, waiting to be freed.
pub(crate) enum Dead {
    List(*mut PyListObj),
    Instance(*mut PyInstanceObj),
}

/// The objects a free found dead and has not freed yet.
pub(crate) type Worklist = Vec<Dead>;

/// Takes one more reference to the word `word` of kind `kind`.
pub(crate) fn retain_word(kind: u8, word: i64) {
    match kind {
        b's' => unsafe { crate::pycc_rt_str_incref(word as *mut crate::PyStrObj) },
        b'i' => crate::pycc_rt_bigint_retain(word),
        b'l' => incref(word as *mut PyListObj),
        b'c' => incref(word as *mut PyInstanceObj),
        _ => {}
    }
}

/// Drops one reference to the word `word` of kind `kind`. A list or
/// instance whose count reaches zero is pushed onto `dead` rather than
/// freed here.
pub(crate) fn release_word_into(kind: u8, word: i64, dead: &mut Worklist) {
    match kind {
        b's' => unsafe { crate::pycc_rt_str_decref(word as *mut crate::PyStrObj) },
        b'i' => crate::pycc_rt_bigint_release(word),
        b'l' if decref_reaches_zero(word as *mut PyListObj) => {
            dead.push(Dead::List(word as *mut PyListObj));
        }
        b'c' if decref_reaches_zero(word as *mut PyInstanceObj) => {
            dead.push(Dead::Instance(word as *mut PyInstanceObj));
        }
        _ => {}
    }
}

/// [`release_word_into`], freeing whatever dies before returning.
pub(crate) fn release_word(kind: u8, word: i64) {
    let mut dead = Worklist::new();
    release_word_into(kind, word, &mut dead);
    free_all(dead);
}

/// Frees every object on `dead` and everything its releases kill.
pub(crate) fn free_all(mut dead: Worklist) {
    while let Some(object) = dead.pop() {
        match object {
            Dead::List(list) => crate::list::free_list(list, &mut dead),
            Dead::Instance(instance) => crate::instance::free_instance(instance, &mut dead),
        }
    }
}

/// `pycc_rt_list_incref`: one more reference to `list` (null is a no-op).
///
/// # Safety
/// `list` must be null or a live list.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_list_incref(list: *mut PyListObj) {
    incref(list);
}

/// Drops one reference to `list`, freeing it at zero (null is a no-op).
///
/// # Safety
/// `list` must be null or a live list the caller owns a reference to.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_list_decref(list: *mut PyListObj) {
    release_word(b'l', list as i64);
}

/// Frees `list`, whose count compiled code has just brought to zero.
///
/// # Safety
/// `list` must be a list with a count of zero that nothing references.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_list_free(list: *mut PyListObj) {
    free_all(vec![Dead::List(list)]);
}

/// One more reference to `instance` (null is a no-op).
///
/// # Safety
/// `instance` must be null or a live instance.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_instance_incref(instance: *mut PyInstanceObj) {
    incref(instance);
}

/// Drops one reference to `instance`, freeing it at zero (null is a no-op).
///
/// # Safety
/// `instance` must be null or a live instance the caller owns a reference
/// to.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_instance_decref(instance: *mut PyInstanceObj) {
    release_word(b'c', instance as i64);
}

/// Frees `instance`, whose count compiled code has just brought to zero.
///
/// # Safety
/// `instance` must be an instance with a count of zero that nothing
/// references.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_instance_free(instance: *mut PyInstanceObj) {
    free_all(vec![Dead::Instance(instance)]);
}

/// The current count of a list or instance, for tests.
#[cfg(test)]
pub(crate) fn count_of<T>(object: *mut T) -> i64 {
    unsafe { *rc_cell(object) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        LIST_KIND_INSTANCE, LIST_KIND_LIST, LIST_KIND_STR, pycc_rt_instance_new,
        pycc_rt_instance_set_slot, pycc_rt_list_append, pycc_rt_list_new, pycc_rt_str_from_literal,
        pycc_rt_str_incref,
    };

    fn str_count(s: *mut crate::PyStrObj) -> u32 {
        unsafe { &*s }.rc.get()
    }

    /// Freeing a list releases each element it owns, and an element another
    /// owner still holds survives.
    #[test]
    fn freeing_a_list_releases_its_elements() {
        let s = unsafe { pycc_rt_str_from_literal(b"kept".as_ptr(), 4) };
        unsafe { pycc_rt_str_incref(s) };
        let list = pycc_rt_list_new(LIST_KIND_STR, 0);
        unsafe { pycc_rt_list_append(list, s as i64) };
        assert_eq!(str_count(s), 2);
        unsafe { pycc_rt_list_decref(list) };
        assert_eq!(str_count(s), 1);
        unsafe { crate::pycc_rt_str_decref(s) };
    }

    /// An instance's descriptor kinds say which slots it releases: here a
    /// list slot (`l`) holding the only reference to a list of instances.
    #[test]
    fn freeing_an_instance_releases_its_slots_by_kind() {
        static OUTER: &[u8] = b"Outer\0items\0n\0lw";
        static INNER: &[u8] = b"Inner\0";
        let inner = unsafe { pycc_rt_instance_new(0, INNER.as_ptr(), INNER.len()) };
        unsafe { pycc_rt_instance_incref(inner) };
        let items = pycc_rt_list_new(LIST_KIND_INSTANCE, 0);
        unsafe { pycc_rt_list_append(items, inner as i64) };
        let outer = unsafe { pycc_rt_instance_new(2, OUTER.as_ptr(), OUTER.len()) };
        unsafe { pycc_rt_instance_set_slot(outer, 0, items as i64) };
        unsafe { pycc_rt_instance_set_slot(outer, 1, 7) };
        assert_eq!(count_of(inner), 2);
        unsafe { pycc_rt_instance_decref(outer) };
        assert_eq!(count_of(inner), 1);
        unsafe { pycc_rt_instance_decref(inner) };
    }

    /// A long chain of nested lists is freed by the worklist, not by
    /// recursion one native frame per level.
    #[test]
    fn a_deep_chain_frees_in_constant_stack() {
        let mut head = pycc_rt_list_new(LIST_KIND_LIST, 0);
        for _ in 0..200_000 {
            let next = pycc_rt_list_new(LIST_KIND_LIST, 1);
            unsafe { pycc_rt_list_append(next, head as i64) };
            head = next;
        }
        unsafe { pycc_rt_list_decref(head) };
    }

    #[test]
    fn null_is_a_no_op_for_every_entry_point() {
        unsafe {
            pycc_rt_list_incref(std::ptr::null_mut());
            pycc_rt_list_decref(std::ptr::null_mut());
            pycc_rt_instance_incref(std::ptr::null_mut());
            pycc_rt_instance_decref(std::ptr::null_mut());
        }
    }
}
