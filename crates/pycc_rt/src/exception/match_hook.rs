//! The fallback of compiled `except` dispatch (v0.8, the whole-package
//! build): a handler's tag set is computed when the catching module is
//! compiled, so it cannot name the exception classes another module of the
//! same extension defines. The `ext` shim installs a hook that answers, for a
//! tag the set left out, CPython's own `isinstance` of the exception against
//! the handler's class (`pycc_ext_exception_tag_matches`).

use super::PyExceptionObj;
use std::sync::atomic::{AtomicPtr, Ordering};

/// `(exception's tag, its CPython original or null, handler's tag)` -> 1 when
/// the exception is an instance of the handler's class.
type ExceptionMatchHook = unsafe extern "C" fn(u8, *mut std::ffi::c_void, u8) -> i32;

static EXCEPTION_MATCH: AtomicPtr<()> = AtomicPtr::new(std::ptr::null_mut());

/// Installs the `ext` shim's dispatch fallback. Without one (a native
/// program) a tag matches only by [`super::tag_matches`].
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_exception_match_install(hook: ExceptionMatchHook) {
    EXCEPTION_MATCH.store(hook as *mut (), Ordering::Relaxed);
}

/// Whether the installed hook matches `exc` against `handler_tag`; false
/// when none is installed.
pub(super) fn hook_matches(exc: &PyExceptionObj, handler_tag: u8) -> bool {
    let hook = EXCEPTION_MATCH.load(Ordering::Relaxed);
    if hook.is_null() {
        return false;
    }
    // SAFETY: only `pycc_rt_exception_match_install` stores here, and it
    // stores an `ExceptionMatchHook`.
    let hook = unsafe { std::mem::transmute::<*mut (), ExceptionMatchHook>(hook) };
    unsafe { hook(exc.type_tag, exc.foreign.get(), handler_tag) != 0 }
}
