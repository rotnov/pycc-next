//! Reference counting of exception objects.
//!
//! An exception object starts with a count of one, owned by whoever
//! allocated it. Every holder owns one reference: the pending-exception
//! state ([`super::pycc_rt_exception_raise`] takes the reference it is
//! handed, [`pycc_rt_exception_take`] hands it back, `clear` releases it),
//! a compiled `except` handler's saved exception and its `as` binding, a
//! local, a list element, an instance slot, a group (each member), and an
//! exception's `__cause__`. Compiled code counts through
//! [`pycc_rt_exception_incref`] and [`pycc_rt_exception_decref`]; inside
//! containers the word's kind byte is `x` (`crate::rc`).
//!
//! The last release frees the object and releases what it owns: its
//! message, its cached `str()` form, its cause, its group members, a
//! `StopIteration`'s value, and the original CPython exception a bridged
//! exception was translated from ([`pycc_rt_exception_set_foreign`]). A
//! chain of causes is freed with a worklist, not by recursion.
//!
//! Cycles are not collected: `raise e from e` (or a longer `__cause__`
//! cycle) keeps its exceptions alive, as any reference cycle does
//! (`docs/DEVIATIONS.md`).

use super::PyExceptionObj;
use crate::PyStrObj;
use std::cell::Cell;
use std::sync::atomic::{AtomicI64, Ordering};

/// Net live exception objects, for leak tests.
static EXCEPTION_LIVE: AtomicI64 = AtomicI64::new(0);

/// The number of exception objects allocated and not yet freed.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_exception_live_objects() -> i64 {
    EXCEPTION_LIVE.load(Ordering::Relaxed)
}

/// Allocates an exception object with a count of one. `message` (owned)
/// and `exceptions` (an owned member array, each member already retained)
/// pass to the object.
pub(crate) fn new_exception(
    type_tag: u8,
    name: *const u8,
    name_len: usize,
    message: *mut PyStrObj,
    exceptions: *mut *mut PyExceptionObj,
    exceptions_len: usize,
) -> *mut PyExceptionObj {
    EXCEPTION_LIVE.fetch_add(1, Ordering::Relaxed);
    Box::into_raw(Box::new(PyExceptionObj {
        rc: Cell::new(1),
        type_tag,
        name,
        name_len,
        message,
        frame_function: std::ptr::null(),
        frame_function_len: 0,
        cause: Cell::new(std::ptr::null_mut()),
        exceptions,
        exceptions_len,
        str_form: Cell::new(std::ptr::null_mut()),
        payload: Cell::new(None),
        foreign: Cell::new(std::ptr::null_mut()),
    }))
}

/// Allocates an exception of class `name` (`name_len` UTF-8 bytes that
/// outlive it -- codegen passes a static constant) whose message is
/// `message`, whose reference passes to the exception. The new exception's
/// one reference is the caller's.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_exception_alloc(
    type_tag: u8,
    name: *const u8,
    name_len: usize,
    message: *mut PyStrObj,
) -> *mut PyExceptionObj {
    new_exception(type_tag, name, name_len, message, std::ptr::null_mut(), 0)
}

/// Takes one more reference to `obj` (null is a no-op).
///
/// # Safety
/// `obj` must be null or a live exception object.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_exception_incref(obj: *mut PyExceptionObj) {
    if !obj.is_null() {
        // SAFETY: a non-null `obj` is a live exception object.
        let rc = unsafe { &(*obj).rc };
        rc.set(rc.get() + 1);
    }
}

/// Drops one reference to `obj`, freeing it and releasing what it owns
/// when it was the last (null is a no-op).
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_exception_decref(obj: *mut PyExceptionObj) {
    let mut dead = Vec::new();
    if reaches_zero(obj) {
        dead.push(obj);
    }
    while let Some(obj) = dead.pop() {
        free_one(obj, &mut dead);
    }
}

/// Decrements `obj`'s count, answering whether it reached zero.
fn reaches_zero(obj: *mut PyExceptionObj) -> bool {
    if obj.is_null() {
        return false;
    }
    // SAFETY: a non-null `obj` is a live exception object.
    let rc = unsafe { &(*obj).rc };
    let left = rc.get() - 1;
    rc.set(left);
    left == 0
}

/// Frees `obj`, whose count reached zero, releasing what it owns; an owned
/// exception (its cause, a member) whose count reaches zero is pushed onto
/// `dead` instead of being freed recursively.
fn free_one(obj: *mut PyExceptionObj, dead: &mut Vec<*mut PyExceptionObj>) {
    EXCEPTION_LIVE.fetch_sub(1, Ordering::Relaxed);
    // SAFETY: `obj` was allocated by `new_exception` and nothing else
    // references it any more.
    let exc = unsafe { Box::from_raw(obj) };
    // SAFETY: the message and the cached `str()` form are owned `str`s (or
    // null, which `pycc_rt_str_decref` ignores).
    unsafe {
        crate::pycc_rt_str_decref(exc.message);
        crate::pycc_rt_str_decref(exc.str_form.get());
    }
    if let Some((word, _, kind)) = exc.payload.get() {
        crate::rc::release_word(kind, word as i64);
    }
    let foreign = exc.foreign.get();
    if !foreign.is_null() {
        crate::rc::release_word(b'o', foreign as i64);
    }
    if reaches_zero(exc.cause.get()) {
        dead.push(exc.cause.get());
    }
    if exc.exceptions_len != 0 {
        // SAFETY: `own_members` leaked exactly this boxed slice.
        let members = unsafe {
            Box::from_raw(std::ptr::slice_from_raw_parts_mut(
                exc.exceptions,
                exc.exceptions_len,
            ))
        };
        for member in members.iter() {
            if reaches_zero(*member) {
                dead.push(*member);
            }
        }
    }
}

/// Records `cause` as `obj`'s `__cause__`, taking over the caller's
/// reference to it and releasing the cause it replaces.
pub(super) fn set_cause(obj: &PyExceptionObj, cause: *mut PyExceptionObj) {
    let old = obj.cause.replace(cause);
    pycc_rt_exception_decref(old);
}

/// Attaches `foreign`, the original CPython exception (`PyObject *`) the
/// `ext` shim translated `obj` from, taking over the caller's reference: it
/// is released (through the installed object hooks) when `obj` is freed,
/// and [`pycc_rt_exception_foreign`] hands it back when `obj` escapes to
/// CPython unchanged.
///
/// # Safety
/// `obj` must be a live exception object with no foreign original yet.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_exception_set_foreign(
    obj: *mut PyExceptionObj,
    foreign: *mut std::ffi::c_void,
) {
    // SAFETY (caller): live.
    unsafe { (*obj).foreign.set(foreign) };
}

/// The original CPython exception `obj` was translated from, borrowed, or
/// null for an exception compiled code raised itself.
///
/// # Safety
/// A non-null `obj` must be a live exception object.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_exception_foreign(
    obj: *mut PyExceptionObj,
) -> *mut std::ffi::c_void {
    if obj.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY (caller): live.
    unsafe { (*obj).foreign.get() }
}
