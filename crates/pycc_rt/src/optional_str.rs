//! A native `str | None` local (`pycc_mir::optional_str`): a `str` slot that
//! holds a `PyStrObj` or null for `None`. These are its tests; the
//! `str`-slot reference counting it shares with every `str` local is
//! already null-safe.

use crate::PyStrObj;
use crate::set::PySetObj;

/// The null `str` standing for `None`.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_optional_str_none() -> *mut PyStrObj {
    std::ptr::null_mut()
}

/// `s is None`.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_optional_str_is_none(s: *mut PyStrObj) -> i8 {
    i8::from(s.is_null())
}

/// `s == text` (both borrowed), false for `None`.
///
/// # Safety
/// `s` must be null or a live `str`, `text` a live `str`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_optional_str_eq(s: *mut PyStrObj, text: *mut PyStrObj) -> i8 {
    if s.is_null() {
        return 0;
    }
    // SAFETY: both are live `str`s.
    i8::from(s == text || unsafe { (*s).bytes() == (*text).bytes() })
}

/// `s in set` for a set of `str` (both borrowed), false for `None`.
///
/// # Safety
/// `s` must be null or a live `str`, `set` a live set of `str`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_optional_str_in(s: *mut PyStrObj, set: *mut PySetObj) -> i8 {
    if s.is_null() {
        return 0;
    }
    // SAFETY: `set` is live and `s` a live `str` word.
    unsafe { crate::set::pycc_rt_set_contains(set, s as i64) }
}
