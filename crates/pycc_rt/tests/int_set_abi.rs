//! Part 1 of #1319: the `pycc_rt_int_set_*` entry points `set[int]` and
//! `frozenset[int]` share, called through the `rlib` exactly as generated
//! code links them from the `staticlib`, plus the #1343 `pycc_rt_obj_set_*`
//! probe, push and identity-insert points a `set[C]` of a hashable user
//! class shares with them.
//!
//! The unit tests in `crates/pycc_rt/src/int_set.rs` own the semantics; this file pins the exported ABI from outside the crate. It is
//! also where the diff-coverage gate sees these functions: the
//! `--workspace` export keeps an integration-test binary's copy of a
//! `pycc_rt` symbol and discards the unit-test binary's counters for it
//! (`docs/TESTING.md`, "A runtime function an integration test links is
//! measured from that binary").

use pycc_rt::{
    EXCEPTION_TYPE_OVERFLOW_ERROR, EXCEPTION_TYPE_RUNTIME_ERROR, pycc_rt_exception_clear,
    pycc_rt_ext_pending_type, pycc_rt_int_list_append, pycc_rt_int_list_new, pycc_rt_int_lshift,
    pycc_rt_int_set_add, pycc_rt_int_set_check_not_resized, pycc_rt_int_set_copy,
    pycc_rt_int_set_decref, pycc_rt_int_set_from_int_list, pycc_rt_int_set_get,
    pycc_rt_int_set_incref, pycc_rt_int_set_len, pycc_rt_int_set_new, pycc_rt_obj_set_add_identity,
    pycc_rt_obj_set_candidate, pycc_rt_obj_set_push,
};

/// D-061's inline smallint word for `value`.
fn small(value: i64) -> i64 {
    (value << 1) | 1
}

/// D-141's `True` marker.
const TRUE: i64 = 6;

/// A heap bigint word (`1 << 100`), built through the exported shift.
fn bigint_word() -> i64 {
    let word = pycc_rt_int_lshift(small(1), small(100));
    assert_eq!(
        pycc_rt_ext_pending_type(),
        -1,
        "the shift itself raises nothing"
    );
    word
}

fn items(set: *mut pycc_rt::PyIntSetObj) -> Vec<i64> {
    let len = unsafe { pycc_rt_int_set_len(set) };
    (0..len)
        .map(|index| unsafe { pycc_rt_int_set_get(set, index) })
        .collect()
}

#[test]
fn add_dedups_by_numeric_value_keeping_the_first_word() {
    pycc_rt_exception_clear();
    let set = pycc_rt_int_set_new();
    unsafe {
        pycc_rt_int_set_add(set, TRUE);
        pycc_rt_int_set_add(set, small(1));
        pycc_rt_int_set_add(set, small(4));
        pycc_rt_int_set_add(set, small(4));
    }
    assert_eq!(items(set), vec![TRUE, small(4)]);
    assert_eq!(pycc_rt_ext_pending_type(), -1);
    unsafe { pycc_rt_int_set_decref(set) };
}

#[test]
fn add_of_a_bigint_raises_overflow_and_leaves_the_set_intact() {
    pycc_rt_exception_clear();
    let big = bigint_word();
    let set = pycc_rt_int_set_new();
    unsafe {
        pycc_rt_int_set_add(set, small(7));
        pycc_rt_int_set_add(set, big);
    }
    assert_eq!(
        pycc_rt_ext_pending_type(),
        i32::from(EXCEPTION_TYPE_OVERFLOW_ERROR)
    );
    assert_eq!(items(set), vec![small(7)]);
    pycc_rt_exception_clear();
    unsafe { pycc_rt_int_set_decref(set) };
}

#[test]
fn check_not_resized_raises_runtime_error_only_on_a_length_change() {
    pycc_rt_exception_clear();
    pycc_rt_int_set_check_not_resized(2, 2);
    assert_eq!(pycc_rt_ext_pending_type(), -1);
    pycc_rt_int_set_check_not_resized(3, 2);
    assert_eq!(
        pycc_rt_ext_pending_type(),
        i32::from(EXCEPTION_TYPE_RUNTIME_ERROR)
    );
    pycc_rt_exception_clear();
}

#[test]
fn incref_and_decref_tolerate_null_and_keep_a_shared_set_alive() {
    unsafe {
        pycc_rt_int_set_incref(std::ptr::null_mut());
        pycc_rt_int_set_decref(std::ptr::null_mut());
        let set = pycc_rt_int_set_new();
        pycc_rt_int_set_add(set, small(5));
        pycc_rt_int_set_incref(set);
        pycc_rt_int_set_decref(set);
        // Still live after one decref of two references.
        assert_eq!(items(set), vec![small(5)]);
        pycc_rt_int_set_decref(set);
    }
}

#[test]
fn copy_is_an_independent_set_in_source_order() {
    pycc_rt_exception_clear();
    unsafe {
        let src = pycc_rt_int_set_new();
        pycc_rt_int_set_add(src, small(3));
        pycc_rt_int_set_add(src, small(1));
        let copy = pycc_rt_int_set_copy(src);
        assert_ne!(src, copy);
        pycc_rt_int_set_add(src, small(9));
        assert_eq!(items(copy), vec![small(3), small(1)]);
        assert_eq!(items(src), vec![small(3), small(1), small(9)]);
        pycc_rt_int_set_decref(src);
        pycc_rt_int_set_decref(copy);
    }
}

#[test]
fn from_int_list_dedups_keeping_the_first_occurrence() {
    pycc_rt_exception_clear();
    unsafe {
        let list = pycc_rt_int_list_new();
        for value in [2, 1, 2, 5] {
            pycc_rt_int_list_append(list, small(value));
        }
        let set = pycc_rt_int_set_from_int_list(list);
        assert_eq!(items(set), vec![small(2), small(1), small(5)]);
        assert_eq!(pycc_rt_ext_pending_type(), -1);
        pycc_rt_int_set_decref(set);
    }
}

/// The defensive path: a bigint word in the source list raises the same
/// `OverflowError` `pycc_rt_int_set_add` does and stops copying there.
#[test]
fn from_int_list_raises_on_a_bigint_word_and_stops() {
    pycc_rt_exception_clear();
    let big = bigint_word();
    unsafe {
        let list = pycc_rt_int_list_new();
        pycc_rt_int_list_append(list, small(4));
        pycc_rt_int_list_append(list, big);
        pycc_rt_int_list_append(list, small(7));
        let set = pycc_rt_int_set_from_int_list(list);
        assert_eq!(
            pycc_rt_ext_pending_type(),
            i32::from(EXCEPTION_TYPE_OVERFLOW_ERROR)
        );
        assert_eq!(items(set), vec![small(4)]);
        pycc_rt_exception_clear();
        pycc_rt_int_set_decref(set);
    }
}

/// #1343: the instance-set entry points `set[C]` of a hashable user class
/// uses. The item word is an opaque instance pointer, so plain integers
/// stand in for it here; the unit tests own the full semantics.
#[test]
fn instance_set_entry_points_probe_push_and_dedup_by_identity() {
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
        // The same word with the same hash is kept once; an equal hash on
        // another word and a different hash are both new entries.
        pycc_rt_obj_set_add_identity(set, 100, 5);
        pycc_rt_obj_set_add_identity(set, 400, 5);
        pycc_rt_obj_set_add_identity(set, 500, 9);
        assert_eq!(items(set), vec![100, 200, 300, 400, 500]);
        assert_eq!(pycc_rt_obj_set_candidate(set, 9, 0), 4);
        pycc_rt_int_set_decref(set);
    }
}
