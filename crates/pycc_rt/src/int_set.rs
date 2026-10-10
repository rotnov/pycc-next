//! A set of user-class instances (#1343), `PyIntSetObj`: every other set
//! is a `set.rs` `PySetObj`.
//!
//! The runtime cannot call a user `__hash__` or `__eq__`, so compiled code
//! drives insertion: it computes the element's hash, asks
//! [`pycc_rt_obj_set_candidate`] for each stored element with that hash,
//! calls `__eq__` itself, and appends with [`pycc_rt_obj_set_push`] when no
//! candidate was equal. Elements stay in insertion order.
//!
//! The object shares `PySetObj`'s header (`len` at offset 0, a marker at
//! offset 24, the count at `rc::RC_OFFSET`), so `len(s)`, truth testing,
//! iteration and reference counting (`set::pycc_rt_set_len`/
//! `pycc_rt_set_get`/`pycc_rt_set_incref`/`pycc_rt_set_decref`) serve both
//! kinds of set. A set of reference-counted instances
//! ([`pycc_rt_obj_set_new`]) owns one reference to each stored instance,
//! released when the set is freed; a set of exception instances (which
//! carry no count) owns none.

use std::cell::Cell;
use std::sync::atomic::Ordering;

/// `PySetObj`'s header marker of an instance set.
pub(crate) const INSTANCE_SET_MARKER: i64 = 0x0b5e7;

/// See the module doc comment.
#[repr(C)]
pub struct PyIntSetObj {
    len: Cell<i64>,
    /// `PySetObj::frozen`'s slot (`set::pycc_rt_set_freeze`).
    _frozen: i64,
    _reserved: i64,
    marker: i64,
    rc: Cell<i64>,
    items: Cell<Vec<i64>>,
    hashes: Cell<Vec<i64>>,
    counted: bool,
}

const _: () = assert!(std::mem::offset_of!(PyIntSetObj, rc) == crate::rc::RC_OFFSET);

fn new_set(counted: bool, items: Vec<i64>, hashes: Vec<i64>) -> *mut PyIntSetObj {
    crate::rc::SET_LIVE.fetch_add(1, Ordering::Relaxed);
    Box::into_raw(Box::new(PyIntSetObj {
        len: Cell::new(items.len() as i64),
        _frozen: 0,
        _reserved: 0,
        marker: INSTANCE_SET_MARKER,
        rc: Cell::new(1),
        items: Cell::new(items),
        hashes: Cell::new(hashes),
        counted,
    }))
}

/// Frees an instance set whose count reached zero, releasing the instances
/// a counted set owns.
pub(crate) fn free(set: *mut PyIntSetObj, dead: &mut crate::rc::Worklist) {
    let set = unsafe { Box::from_raw(set) };
    if set.counted {
        for word in set.items.take() {
            crate::rc::release_word_into(b'c', word, dead);
        }
    }
    crate::rc::SET_LIVE.fetch_sub(1, Ordering::Relaxed);
}

/// The element at insertion position `index` (`index` in range).
pub(crate) fn element_at(set: *mut PyIntSetObj, index: i64) -> i64 {
    let items = unsafe { &*set }.items.take();
    let value = items[index as usize];
    unsafe { &*set }.items.set(items);
    value
}

/// The stored element words in insertion order (a copy).
pub(crate) fn items(set: *mut PyIntSetObj) -> Vec<i64> {
    let items = unsafe { &*set }.items.take();
    let copy = items.clone();
    unsafe { &*set }.items.set(items);
    copy
}

/// A fresh, empty set of uncounted (exception) instances.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_int_set_new() -> *mut PyIntSetObj {
    new_set(false, Vec::new(), Vec::new())
}

/// A fresh, empty set of reference-counted instances: it owns a reference
/// to every element it stores ([`pycc_rt_obj_set_push`] adopts the one it is
/// handed).
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_obj_set_new() -> *mut PyIntSetObj {
    new_set(true, Vec::new(), Vec::new())
}

/// A copy of `src` with its stored hashes, so `frozenset(s)` calls no user
/// `__hash__` or `__eq__`; a counted copy takes its own references.
///
/// # Safety
/// `src` must be a live instance set.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_int_set_copy(src: *mut PyIntSetObj) -> *mut PyIntSetObj {
    let src = unsafe { &*src };
    let items = src.items.take();
    let hashes = src.hashes.take();
    if src.counted {
        for word in &items {
            crate::rc::retain_word(b'c', *word);
        }
    }
    let copy = new_set(src.counted, items.clone(), hashes.clone());
    src.items.set(items);
    src.hashes.set(hashes);
    copy
}

/// The smallest index `i >= from` whose stored hash equals `hash`, or `-1`
/// when there is none. An instance set is append-only, so an index returned
/// here stays valid across the user `__eq__` call codegen makes next.
///
/// # Safety
/// `set` must be a live instance set; `from >= 0`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_obj_set_candidate(
    set: *mut PyIntSetObj,
    hash: i64,
    from: i64,
) -> i64 {
    let hashes = unsafe { &*set }.hashes.take();
    let found = hashes
        .iter()
        .skip(from as usize)
        .position(|stored| *stored == hash)
        .map_or(-1, |offset| from + offset as i64);
    unsafe { &*set }.hashes.set(hashes);
    found
}

/// Appends `word` with its reduced `hash`, once codegen's probe found no
/// equal element. A counted set adopts the reference the caller hands over
/// with `word`.
///
/// # Safety
/// `set` must be a live instance set.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_obj_set_push(set: *mut PyIntSetObj, word: i64, hash: i64) {
    let set = unsafe { &*set };
    let mut items = set.items.take();
    items.push(word);
    set.len.set(items.len() as i64);
    set.items.set(items);
    let mut hashes = set.hashes.take();
    hashes.push(hash);
    set.hashes.set(hashes);
}

/// The insert for a class whose `__eq__` is `object.__eq__` (identity):
/// appends `word` unless the same word with an equal hash is stored.
///
/// # Safety
/// `set` must be a live instance set.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_obj_set_add_identity(set: *mut PyIntSetObj, word: i64, hash: i64) {
    let items = unsafe { &*set }.items.take();
    let hashes = unsafe { &*set }.hashes.take();
    let present = items
        .iter()
        .zip(hashes.iter())
        .any(|(stored, stored_hash)| *stored_hash == hash && *stored == word);
    unsafe { &*set }.items.set(items);
    unsafe { &*set }.hashes.set(hashes);
    if !present {
        unsafe { pycc_rt_obj_set_push(set, word, hash) };
    } else {
        unsafe { pycc_rt_obj_set_discard_candidate(set, word) };
    }
}

/// Releases the reference a counted set was handed with `word` when the
/// insertion found an equal element already stored and so does not keep
/// it (a set of uncounted instances owns nothing to release).
///
/// # Safety
/// `set` must be a live `PyIntSetObj` pointer holding instances.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_obj_set_discard_candidate(set: *mut PyIntSetObj, word: i64) {
    if unsafe { &*set }.counted {
        crate::rc::release_word(b'c', word);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::set::{pycc_rt_set_get, pycc_rt_set_len};

    fn set_items(set: *mut PyIntSetObj) -> Vec<i64> {
        let set = set.cast::<crate::set::PySetObj>();
        let len = unsafe { pycc_rt_set_len(set) };
        (0..len)
            .map(|index| unsafe { pycc_rt_set_get(set, index) })
            .collect()
    }

    fn set_hashes(set: *mut PyIntSetObj) -> Vec<i64> {
        let hashes = unsafe { &*set }.hashes.take();
        let copy = hashes.clone();
        unsafe { &*set }.hashes.set(hashes);
        copy
    }

    #[test]
    fn candidate_finds_no_match_the_first_match_and_a_match_from_an_offset() {
        unsafe {
            let set = pycc_rt_int_set_new();
            assert_eq!(pycc_rt_obj_set_candidate(set, 5, 0), -1);
            pycc_rt_obj_set_push(set, 100, 5);
            pycc_rt_obj_set_push(set, 200, 6);
            pycc_rt_obj_set_push(set, 300, 5);
            assert_eq!(pycc_rt_obj_set_candidate(set, 5, 0), 0);
            assert_eq!(pycc_rt_obj_set_candidate(set, 5, 1), 2);
            assert_eq!(pycc_rt_obj_set_candidate(set, 5, 3), -1);
            assert_eq!(pycc_rt_obj_set_candidate(set, 7, 0), -1);
        }
    }

    #[test]
    fn add_identity_dedups_only_the_same_word_with_the_same_hash() {
        unsafe {
            let set = pycc_rt_int_set_new();
            pycc_rt_obj_set_add_identity(set, 100, 5);
            pycc_rt_obj_set_add_identity(set, 100, 5);
            pycc_rt_obj_set_add_identity(set, 200, 5);
            pycc_rt_obj_set_add_identity(set, 300, 9);
            assert_eq!(set_items(set), vec![100, 200, 300]);
            assert_eq!(set_hashes(set), vec![5, 5, 9]);
        }
    }

    #[test]
    fn copy_of_an_instance_set_keeps_its_hashes() {
        unsafe {
            let src = pycc_rt_int_set_new();
            pycc_rt_obj_set_push(src, 100, 5);
            pycc_rt_obj_set_push(src, 200, 6);
            let copy = pycc_rt_int_set_copy(src);
            pycc_rt_obj_set_push(src, 300, 7);
            assert_eq!(set_items(copy), vec![100, 200]);
            assert_eq!(set_hashes(copy), vec![5, 6]);
            assert_eq!(pycc_rt_obj_set_candidate(copy, 6, 0), 1);
            assert_eq!(set_items(src).len(), 3);
        }
    }
}
