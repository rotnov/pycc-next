//! The `ext` shim's dealloc pointer (`pycc_rt_exception_dealloc_install`)
//! follows the pending state: the set-aside function while an exception is
//! pending, the plain one otherwise.
//!
//! A test binary of its own, with this one test: the pointer and the count
//! of threads with an exception pending are process-wide, so no other
//! test's thread may raise or clear meanwhile.

use pycc_rt::{
    EXCEPTION_TYPE_VALUE_ERROR, pycc_rt_exception_alloc, pycc_rt_exception_clear,
    pycc_rt_exception_dealloc_install, pycc_rt_exception_raise, pycc_rt_exception_restore,
    pycc_rt_exception_take, pycc_rt_str_from_literal,
};
use std::ffi::c_void;
use std::sync::atomic::{AtomicPtr, Ordering};

#[test]
fn the_dealloc_pointer_follows_the_pending_state() {
    static SLOT: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
    static FUNCTIONS: [u8; 2] = [0; 2];
    let plain = (&raw const FUNCTIONS[0]).cast_mut().cast::<c_void>();
    let aside = (&raw const FUNCTIONS[1]).cast_mut().cast::<c_void>();
    let slot = || SLOT.load(Ordering::Relaxed);

    unsafe { pycc_rt_exception_dealloc_install(SLOT.as_ptr(), plain, aside) };
    assert_eq!(slot(), plain, "installed with nothing pending");

    let name = "ValueError";
    let message = unsafe { pycc_rt_str_from_literal(name.as_ptr(), name.len() as i64) };
    let raised =
        pycc_rt_exception_alloc(EXCEPTION_TYPE_VALUE_ERROR, name.as_ptr(), name.len(), message);
    pycc_rt_exception_raise(raised);
    assert_eq!(slot(), aside);
    // Raising over a pending exception changes nothing.
    pycc_rt_exception_raise(pycc_rt_exception_take());
    assert_eq!(slot(), aside);

    let taken = pycc_rt_exception_take();
    assert_eq!(taken, raised);
    assert_eq!(slot(), plain);
    pycc_rt_exception_restore(taken);
    assert_eq!(slot(), aside);
    pycc_rt_exception_clear();
    assert_eq!(slot(), plain);

    // Installed while an exception is pending, the slot starts set aside.
    let message = unsafe { pycc_rt_str_from_literal(name.as_ptr(), name.len() as i64) };
    pycc_rt_exception_raise(pycc_rt_exception_alloc(
        EXCEPTION_TYPE_VALUE_ERROR,
        name.as_ptr(),
        name.len(),
        message,
    ));
    unsafe { pycc_rt_exception_dealloc_install(SLOT.as_ptr(), plain, aside) };
    assert_eq!(slot(), aside);
    pycc_rt_exception_clear();
    assert_eq!(slot(), plain);
}
