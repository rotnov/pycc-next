//! `str(exc)`: the text an exception shows in `print(e)`, `f"{e}"` and the
//! uncaught-exception traceback.
//!
//! For every exception but `KeyError` that is its message. `KeyError.__str__`
//! renders the `repr` of its single argument, so `str(KeyError("k"))` is
//! `'k'` -- the message itself stays the raw argument, which is what the
//! `ext` boundary hands CPython to rebuild the exception from.

use super::{EXCEPTION_TYPE_KEY_ERROR, PyExceptionObj, alloc_exception_message};
use crate::PyStrObj;

/// `exc`'s `str()` form, borrowed from `exc` like its message: for a
/// `KeyError`, the `repr` of its message, built on first use and cached on
/// the exception object, which releases it when it is freed.
pub(super) fn exception_str(exc: &PyExceptionObj) -> *mut PyStrObj {
    if exc.type_tag != EXCEPTION_TYPE_KEY_ERROR || exc.message.is_null() {
        return exc.message;
    }
    let cached = exc.str_form.get();
    if !cached.is_null() {
        return cached;
    }
    let message = String::from_utf8_lossy(unsafe { (*exc.message).bytes() });
    let rendered = alloc_exception_message(&str_repr(&message));
    exc.str_form.set(rendered);
    rendered
}

/// CPython's `repr` of a `str` ([`crate::list::str_repr`]).
fn str_repr(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    crate::list::str_repr(text, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exception::{EXCEPTION_TYPE_VALUE_ERROR, pycc_rt_exception_alloc};

    #[test]
    fn str_repr_matches_cpython() {
        assert_eq!(str_repr("k"), "'k'");
        assert_eq!(str_repr(""), "''");
        assert_eq!(str_repr("it's"), "\"it's\"");
        assert_eq!(str_repr("a'b\"c"), "'a\\'b\"c'");
        assert_eq!(str_repr("q\""), "'q\"'");
        assert_eq!(str_repr("t\tn\nr\r\\"), "'t\\tn\\nr\\r\\\\'");
        assert_eq!(str_repr("\u{1}\u{7f}\u{85}é"), "'\\x01\\x7f\\x85é'");
        // Not printable beyond ASCII: \xNN, \uNNNN, \UNNNNNNNN (fuzzer finding).
        assert_eq!(
            str_repr("\u{a0}\u{ad}\u{200b}\u{2028}\u{e0001}\u{10ffff} 日😀"),
            "'\\xa0\\xad\\u200b\\u2028\\U000e0001\\U0010ffff 日😀'"
        );
    }

    fn alloc(type_tag: u8, name: &'static str, msg: &str) -> PyExceptionObj {
        let obj = pycc_rt_exception_alloc(
            type_tag,
            name.as_ptr(),
            name.len(),
            alloc_exception_message(msg),
        );
        *unsafe { Box::from_raw(obj) }
    }

    #[test]
    fn only_a_key_error_renders_its_message_as_a_repr() {
        let key = alloc(EXCEPTION_TYPE_KEY_ERROR, "KeyError", "k");
        let first = exception_str(&key);
        assert_eq!(unsafe { (*first).bytes() }, b"'k'");
        assert_eq!(exception_str(&key), first, "the rendering is cached");
        assert_eq!(unsafe { (*key.message).bytes() }, b"k");

        let value = alloc(EXCEPTION_TYPE_VALUE_ERROR, "ValueError", "k");
        assert_eq!(exception_str(&value), value.message);
    }
}
