//! Function values (`Ty::Callable`).
//!
//! A function value is a module-level function with its leading parameters
//! bound to captured values: a nested function's enclosing variables, or
//! none for a module-level function used as a value. The object holds the
//! captured values as words (`pycc_codegen::closure` encodes each by its
//! type) and two compiled functions: `call(obj, args...)`, which passes the
//! captured values and the arguments to the function, and `drop(obj)`, which
//! releases the captured values the object owns. It also points at the
//! function's `__qualname__` (a NUL-terminated constant), which its `repr`
//! names: `<function make_adder.<locals>.add at 0x...>`.

use std::sync::atomic::{AtomicI64, Ordering};

/// Byte offset of the `call` function; mirrored by `pycc_codegen::closure`.
pub const CALL_OFFSET: usize = 8;
/// Byte offset of the first captured word; mirrored by codegen.
pub const CAPTURES_OFFSET: usize = 40;

/// The header of a function value, followed by its captured words.
#[repr(C)]
pub struct PyClosureObj {
    rc: i64,
    call: *const u8,
    drop: Option<unsafe extern "C" fn(*mut PyClosureObj)>,
    captures: i64,
    /// The function's `__qualname__`, NUL-terminated; null when unknown.
    name: *const u8,
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

/// A new function value of the function named `name` (a NUL-terminated
/// `__qualname__`, or null) with `captures` zeroed captured words (the
/// caller stores them) and one reference.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_closure_new(
    call: *const u8,
    drop: Option<unsafe extern "C" fn(*mut PyClosureObj)>,
    captures: i64,
    name: *const u8,
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
        (*obj).name = name;
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

/// The text of a NUL-terminated name, or `?` for null.
pub(crate) fn c_name(name: *const u8) -> String {
    if name.is_null() {
        return "?".to_string();
    }
    // SAFETY: a non-null name is a NUL-terminated constant.
    let name = unsafe { std::ffi::CStr::from_ptr(name.cast()) };
    String::from_utf8_lossy(name.to_bytes()).into_owned()
}

/// Python `repr(f)` of the function value `obj`: `<function NAME at 0x...>`.
pub(crate) fn closure_repr(obj: *mut PyClosureObj) -> String {
    // SAFETY (caller): `obj` is a live function value.
    let name = c_name(unsafe { (*obj).name });
    format!("<function {name} at {:#x}>", obj as usize)
}

/// `str(f)` / `repr(f)` of a function value, as a new `str`.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_closure_repr(obj: *mut PyClosureObj) -> *mut crate::PyStrObj {
    crate::str_core::new_pystr(closure_repr(obj).as_bytes())
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
        let obj = pycc_rt_closure_new(std::ptr::null(), Some(count_drop), 1, std::ptr::null());
        pycc_rt_closure_incref(obj);
        pycc_rt_closure_decref(obj);
        assert_eq!(pycc_rt_closure_live_objects(), before + 1);
        pycc_rt_closure_decref(obj);
        assert_eq!(pycc_rt_closure_live_objects(), before);
        pycc_rt_closure_decref(std::ptr::null_mut());
        pycc_rt_closure_incref(std::ptr::null_mut());
    }

    #[test]
    fn a_function_value_renders_its_qualname_and_address() {
        let obj = pycc_rt_closure_new(std::ptr::null(), None, 0, c"f.<locals>.g".as_ptr().cast());
        assert_eq!(
            closure_repr(obj),
            format!("<function f.<locals>.g at {:#x}>", obj as usize)
        );
        pycc_rt_closure_decref(obj);
        assert_eq!(c_name(std::ptr::null()), "?");
    }
}
