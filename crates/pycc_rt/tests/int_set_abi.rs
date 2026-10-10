//! The exported set entry points, called through the `rlib` exactly as
//! generated code links them from the `staticlib`: the generic `PySetObj`
//! (`pycc_rt_set_*`) and the instance set `set[C]` of a hashable user class
//! uses (`pycc_rt_int_set_new` plus the `pycc_rt_obj_set_*` probe, push and
//! identity insert), which share `len` and iteration.
//!
//! The unit tests in `crates/pycc_rt/src/set.rs` and `int_set.rs` own the
//! semantics; this file pins the exported ABI from outside the crate. It is
//! also where the diff-coverage gate sees these functions (`docs/TESTING.md`,
//! "A runtime function an integration test links is measured from that
//! binary").

use pycc_rt::set::{
    pycc_rt_set_add, pycc_rt_set_check_not_resized, pycc_rt_set_contains, pycc_rt_set_copy,
    pycc_rt_set_from_list, pycc_rt_set_get, pycc_rt_set_len, pycc_rt_set_new,
};
use pycc_rt::{
    EXCEPTION_TYPE_RUNTIME_ERROR, LIST_KIND_INT, PySetObj, pycc_rt_exception_clear,
    pycc_rt_ext_pending_type, pycc_rt_int_set_copy, pycc_rt_int_set_new, pycc_rt_list_append,
    pycc_rt_list_new, pycc_rt_obj_set_add_identity, pycc_rt_obj_set_candidate,
    pycc_rt_obj_set_push,
};

/// D-061's inline smallint word for `value`.
fn small(value: i64) -> i64 {
    (value << 1) | 1
}

/// D-141's `True` marker.
const TRUE: i64 = 6;

fn items(set: *mut PySetObj) -> Vec<i64> {
    let len = unsafe { pycc_rt_set_len(set) };
    (0..len)
        .map(|index| unsafe { pycc_rt_set_get(set, index) })
        .collect()
}

#[test]
fn add_dedups_by_numeric_value_keeping_the_first_word() {
    pycc_rt_exception_clear();
    let set = pycc_rt_set_new(LIST_KIND_INT);
    unsafe {
        pycc_rt_set_add(set, TRUE);
        pycc_rt_set_add(set, small(1));
        pycc_rt_set_add(set, small(4));
        pycc_rt_set_add(set, small(4));
        assert_eq!(pycc_rt_set_contains(set, small(1)), 1);
        assert_eq!(pycc_rt_set_contains(set, small(2)), 0);
    }
    assert_eq!(items(set), vec![TRUE, small(4)]);
    assert_eq!(pycc_rt_ext_pending_type(), -1);
}

#[test]
fn check_not_resized_raises_runtime_error_only_on_a_length_change() {
    pycc_rt_exception_clear();
    pycc_rt_set_check_not_resized(2, 2);
    assert_eq!(pycc_rt_ext_pending_type(), -1);
    pycc_rt_set_check_not_resized(3, 2);
    assert_eq!(
        pycc_rt_ext_pending_type(),
        i32::from(EXCEPTION_TYPE_RUNTIME_ERROR)
    );
    pycc_rt_exception_clear();
}

#[test]
fn copy_and_from_list_keep_cpython_order() {
    pycc_rt_exception_clear();
    unsafe {
        let list = pycc_rt_list_new(LIST_KIND_INT, 0);
        for value in [9, 2, 1, 2, 5] {
            pycc_rt_list_append(list, small(value));
        }
        let set = pycc_rt_set_from_list(list, LIST_KIND_INT);
        assert_eq!(items(set), vec![small(9), small(2), small(5), small(1)]);
        let copy = pycc_rt_set_copy(set);
        pycc_rt_set_add(set, small(3));
        assert_eq!(items(copy), vec![small(9), small(2), small(5), small(1)]);
        assert_eq!(pycc_rt_set_len(set), 5);
    }
}

/// #1343: the instance-set entry points. The item word is an opaque
/// instance pointer, so plain integers stand in for it here.
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
        pycc_rt_obj_set_add_identity(set, 100, 5);
        pycc_rt_obj_set_add_identity(set, 400, 5);
        pycc_rt_obj_set_add_identity(set, 500, 9);
        assert_eq!(items(set.cast()), vec![100, 200, 300, 400, 500]);
        assert_eq!(pycc_rt_obj_set_candidate(set, 9, 0), 4);
        let copy = pycc_rt_int_set_copy(set);
        assert_eq!(items(copy.cast()).len(), 5);
    }
}
