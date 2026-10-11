//! Function values (`Ty::Callable`).
//!
//! A function value is a module-level function with its leading parameters
//! bound to captured values: a nested function's enclosing variables, or
//! none for a module-level function used as a value. The object holds the
//! captured values as words (`pycc_codegen::closure` encodes each by its
//! type) and two compiled functions: `call(obj, args...)`, which passes the
//! captured values and the arguments to the function, and `drop(obj)`, which
//! releases the captured values the object owns.

use std::sync::atomic::{AtomicI64, Ordering};

/// Byte offset of the `call` function; mirrored by `pycc_codegen::closure`.
pub const CALL_OFFSET: usize = 8;
/// Byte offset of the first captured word; mirrored by codegen.
pub const CAPTURES_OFFSET: usize = 32;

/// The header of a function value, followed by its captured words.
#[repr(C)]
pub struct PyClosureObj {
    rc: i64,
    call: *const u8,
    drop: Option<unsafe extern "C" fn(*mut PyClosureObj)>,
    captures: i64,
}

const _: () = {
    assert!(std::mem::offset_of!(PyClosureObj, call) == CALL_OFFSET);
    assert!(std::mem::size_of::<PyClosureObj>() == CAPTURES_OFFSET);
};

static CLOSURES_LIVE: AtomicI64 = AtomicI64::new(0);

/// The number of function values allocated and not yet freed.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_closure_live_objects() -> i64 {
    CLOSURES_LIVE.load(Ordering::Relaxed)
}

fn layout(captures: usize) -> std::alloc::Layout {
    std::alloc::Layout::from_size_align(CAPTURES_OFFSET + 8 * captures, 8)
        .expect("a function value fits a layout")
}

/// A new function value with `captures` zeroed captured words (the caller
/// stores them) and one reference.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_closure_new(
    call: *const u8,
    drop: Option<unsafe extern "C" fn(*mut PyClosureObj)>,
    captures: i64,
) -> *mut PyClosureObj {
    let count = usize::try_from(captures).unwrap_or(0);
    // SAFETY: a non-zero-size layout.
    let obj = unsafe { std::alloc::alloc_zeroed(layout(count)) }.cast::<PyClosureObj>();
    if obj.is_null() {
        std::alloc::handle_alloc_error(layout(count));
    }
    // SAFETY: freshly allocated and large enough for the header.
    unsafe {
        (*obj).rc = 1;
        (*obj).call = call;
        (*obj).drop = drop;
        (*obj).captures = captures;
    }
    CLOSURES_LIVE.fetch_add(1, Ordering::Relaxed);
    obj
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_closure_incref(obj: *mut PyClosureObj) {
    if !obj.is_null() {
        // SAFETY: a non-null `obj` is live.
        unsafe { (*obj).rc += 1 };
    }
}

/// Releases one reference; the last one releases the captured values and
/// frees the object.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_closure_decref(obj: *mut PyClosureObj) {
    if obj.is_null() {
        return;
    }
    // SAFETY: a non-null `obj` is live, and this is the only place it is
    // freed.
    unsafe {
        (*obj).rc -= 1;
        if (*obj).rc > 0 {
            return;
        }
        if let Some(drop) = (*obj).drop {
            drop(obj);
        }
        let count = usize::try_from((*obj).captures).unwrap_or(0);
        std::alloc::dealloc(obj.cast::<u8>(), layout(count));
    }
    CLOSURES_LIVE.fetch_sub(1, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;

    unsafe extern "C" fn count_drop(obj: *mut PyClosureObj) {
        // SAFETY: the test's object has one captured word.
        unsafe {
            let word = obj.cast::<u8>().add(CAPTURES_OFFSET).cast::<i64>();
            *word += 1;
        }
    }

    #[test]
    fn the_last_release_runs_drop_and_frees() {
        let before = pycc_rt_closure_live_objects();
        let obj = pycc_rt_closure_new(std::ptr::null(), Some(count_drop), 1);
        pycc_rt_closure_incref(obj);
        pycc_rt_closure_decref(obj);
        assert_eq!(pycc_rt_closure_live_objects(), before + 1);
        pycc_rt_closure_decref(obj);
        assert_eq!(pycc_rt_closure_live_objects(), before);
        pycc_rt_closure_decref(std::ptr::null_mut());
        pycc_rt_closure_incref(std::ptr::null_mut());
    }
}
