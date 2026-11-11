//! Reference counting of exception objects (`lifetime.rs`), observed
//! through the counts of `str`s an exception owns: each test keeps one
//! reference of its own to every message, so a freed exception shows as
//! that message's count dropping back to one.

use super::*;

/// A fresh `str` with a count of two: one for the test, one to hand over.
fn shared_str(text: &str) -> *mut PyStrObj {
    let s = crate::new_pystr(text.as_bytes());
    unsafe { crate::pycc_rt_str_incref(s) };
    s
}

fn count(s: *mut PyStrObj) -> i64 {
    unsafe { (*s).rc.get() as i64 }
}

fn exc(name: &'static str, message: *mut PyStrObj) -> *mut PyExceptionObj {
    pycc_rt_exception_alloc(
        EXCEPTION_TYPE_VALUE_ERROR,
        name.as_ptr(),
        name.len(),
        message,
    )
}

#[test]
fn the_last_release_frees_the_exception_and_its_message() {
    let message = shared_str("m");
    let e = exc("ValueError", message);
    unsafe { pycc_rt_exception_incref(e) };
    pycc_rt_exception_decref(e);
    assert_eq!(count(message), 2, "one release of two keeps it alive");
    pycc_rt_exception_decref(e);
    assert_eq!(count(message), 1);
    pycc_rt_exception_decref(std::ptr::null_mut());
}

#[test]
fn take_hands_over_the_pending_reference_and_clear_releases_it() {
    pycc_rt_exception_clear();
    let message = shared_str("pending");
    let e = exc("ValueError", message);
    pycc_rt_exception_raise(e);
    assert_eq!(pycc_rt_exception_take(), e);
    assert_eq!(pycc_rt_exception_active(), 0);
    assert!(pycc_rt_exception_take().is_null());
    pycc_rt_exception_raise(e);
    pycc_rt_exception_clear();
    assert_eq!(count(message), 1);
}

#[test]
fn raising_over_a_pending_exception_releases_it() {
    pycc_rt_exception_clear();
    let first = shared_str("first");
    let second = shared_str("second");
    pycc_rt_exception_raise(exc("ValueError", first));
    let replacement = exc("ValueError", second);
    pycc_rt_exception_raise(replacement);
    assert_eq!(count(first), 1, "the replaced exception is freed");
    // Raising the pending exception again hands over a second reference.
    unsafe { pycc_rt_exception_incref(replacement) };
    pycc_rt_exception_raise(replacement);
    assert_eq!(pycc_rt_exception_value(), replacement);
    pycc_rt_exception_clear();
    assert_eq!(count(second), 1);
}

#[test]
fn a_cause_chain_is_released_with_its_effect() {
    pycc_rt_exception_clear();
    let messages: Vec<*mut PyStrObj> = (0..3).map(|i| shared_str(&format!("c{i}"))).collect();
    let root = exc("ValueError", messages[0]);
    let middle = exc("ValueError", messages[1]);
    unsafe { pycc_rt_exception_raise_with_cause(middle, root) };
    let middle = pycc_rt_exception_take();
    let leaf = exc("ValueError", messages[2]);
    unsafe { pycc_rt_exception_raise_with_cause(leaf, middle) };
    pycc_rt_exception_clear();
    for message in messages {
        assert_eq!(count(message), 1);
    }
    // A null primary object releases the cause it was handed.
    let orphan = shared_str("orphan");
    unsafe { pycc_rt_exception_raise_with_cause(std::ptr::null_mut(), exc("ValueError", orphan)) };
    pycc_rt_exception_clear();
    assert_eq!(count(orphan), 1);
}

#[test]
fn an_exception_that_is_its_own_cause_is_freed() {
    // `except ValueError as e: raise e from e`: two references handed over,
    // one for the raise and one for the cause.
    pycc_rt_exception_clear();
    let message = shared_str("self");
    let e = exc("ValueError", message);
    unsafe { pycc_rt_exception_incref(e) };
    unsafe { pycc_rt_exception_raise_with_cause(e, e) };
    assert_eq!(unsafe { lifetime::pycc_rt_exception_cause(e) }, e);
    // Taken back and raised `from None`: the self link goes, owning nothing.
    let e = pycc_rt_exception_take();
    unsafe { lifetime::pycc_rt_exception_suppress_context(e) };
    assert!(unsafe { lifetime::pycc_rt_exception_cause(e) }.is_null());
    // And `from e` once more, then the last holder lets go.
    unsafe { pycc_rt_exception_incref(e) };
    unsafe { pycc_rt_exception_raise_with_cause(e, e) };
    pycc_rt_exception_clear();
    assert_eq!(count(message), 1, "freed with its last holder");
}

#[test]
fn releasing_a_cause_frees_it_and_keeps_the_context_shown() {
    // The `ext` shim's, once a CPython object holds the link: the cause is
    // released, and the exception is not marked raised `from None`.
    pycc_rt_exception_clear();
    let cause_text = shared_str("cause");
    let e = exc("ValueError", shared_str("e"));
    unsafe { pycc_rt_exception_raise_with_cause(e, exc("KeyError", cause_text)) };
    let e = pycc_rt_exception_take();
    unsafe { lifetime::pycc_rt_exception_release_cause(e) };
    assert!(unsafe { lifetime::pycc_rt_exception_cause(e) }.is_null());
    assert_eq!(count(cause_text), 1, "the cause is freed");
    assert_eq!(unsafe { lifetime::pycc_rt_exception_suppresses_context(e) }, 0);
    // A self-cause is a borrowed link: releasing it releases nothing.
    unsafe { pycc_rt_exception_incref(e) };
    unsafe { pycc_rt_exception_raise_with_cause(e, e) };
    let e = pycc_rt_exception_take();
    unsafe { lifetime::pycc_rt_exception_release_cause(e) };
    assert!(unsafe { lifetime::pycc_rt_exception_cause(e) }.is_null());
    unsafe { lifetime::pycc_rt_exception_release_cause(std::ptr::null_mut()) };
    pycc_rt_exception_decref(e);
}

#[test]
fn groups_and_partitions_own_their_members_and_message() {
    let a_text = shared_str("a");
    let b_text = shared_str("b");
    let group_text = shared_str("group");
    let a = exc("ValueError", a_text);
    let b = pycc_rt_exception_alloc(EXCEPTION_TYPE_KEY_ERROR, "KeyError".as_ptr(), 8, b_text);
    let members = [a, b];
    let group = unsafe {
        pycc_rt_exception_group_alloc(
            24,
            "ExceptionGroup".as_ptr(),
            14,
            group_text,
            members.as_ptr(),
            members.len(),
        )
    };
    // The group took its own references: the caller's go.
    pycc_rt_exception_decref(a);
    pycc_rt_exception_decref(b);
    assert_eq!(count(a_text), 2);
    let mut matched = std::ptr::null_mut();
    let mut rest = std::ptr::null_mut();
    let tags = [EXCEPTION_TYPE_VALUE_ERROR];
    unsafe {
        pycc_rt_exception_group_partition(
            group,
            tags.as_ptr(),
            tags.len(),
            24,
            "ExceptionGroup".as_ptr(),
            14,
            &raw mut matched,
            &raw mut rest,
        );
    }
    assert!(!matched.is_null() && !rest.is_null());
    assert_eq!(count(group_text), 4, "each subgroup holds the message");
    for g in [group, matched, rest] {
        pycc_rt_exception_decref(g);
    }
    for text in [a_text, b_text, group_text] {
        assert_eq!(count(text), 1);
    }
}

#[test]
fn a_stop_iteration_releases_its_value_and_a_str_form_is_owned() {
    pycc_rt_exception_clear();
    // A value-less `StopIteration` owns no value.
    pycc_rt_raise_stop_iteration(0, std::ptr::null_mut(), 0, b'w');
    pycc_rt_exception_clear();
    let value = shared_str("returned");
    let message = shared_str("v");
    pycc_rt_raise_stop_iteration(value as u64, message, 7, b's');
    pycc_rt_exception_clear();
    assert_eq!(count(value), 1);
    assert_eq!(count(message), 1);

    let text = shared_str("form");
    let e = exc("ValueError", shared_str("m"));
    unsafe { pycc_rt_exception_set_str_form(e, text) };
    unsafe { pycc_rt_exception_set_str_form(e, text) };
    assert_eq!(count(text), 3);
    pycc_rt_exception_decref(e);
    assert_eq!(count(text), 2);
}

#[test]
fn a_container_word_of_kind_x_counts_the_exception() {
    let message = shared_str("held");
    let e = exc("ValueError", message);
    crate::rc::retain_word(b'x', e as i64);
    crate::rc::release_word(b'x', e as i64);
    crate::rc::release_word(b'x', e as i64);
    assert_eq!(count(message), 1);
}
