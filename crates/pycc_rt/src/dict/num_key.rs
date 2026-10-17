//! A `float` (or `int | float`) key looked up in an `int`-keyed dict
//! (`d[1.0]`, `d.get(x, default)`): CPython hashes and compares the number
//! by value, so an integral `float` finds the `int` key equal to it, and any
//! other finds nothing. A miss raises `KeyError` with the `float`'s own repr
//! (`KeyError: 1.5`), as CPython's does.

use super::{PyDictObj, dict, pycc_rt_dict_get};
use crate::int_encoding::bigint_release;
use crate::list::LIST_KIND_NUM;
use crate::num::{Num, decode};

/// The `int` key equal to the `int | float` word `word` (a new reference),
/// or `None` when no `int` equals it (a non-integral or non-finite float).
fn int_key(word: i64) -> Option<i64> {
    match decode(word) {
        Num::Int(int) => {
            crate::int_encoding::bigint_retain(int);
            Some(int)
        }
        Num::Float(f) if f.is_finite() && f.trunc() == f => Some(crate::int_arith::int_from_float(f)),
        Num::Float(_) => None,
    }
}

/// Whether `d` holds the `int` key `key`, or computes a missing value
/// (a `Counter` or `defaultdict`), so `pycc_rt_dict_get` is the lookup.
fn served(d: *mut PyDictObj, key: i64) -> bool {
    let d = dict(d);
    !d.extra.is_null() || matches!(d.locate(key), Ok((Some(_), _)))
}

/// `d[word]` for an `int`-keyed `d` and an `int | float` word (borrowed):
/// the value (a new reference), or `KeyError(word)`.
///
/// # Safety
/// `d` must be a live dict with `int` keys.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_dict_get_num_key(d: *mut PyDictObj, word: i64) -> i64 {
    if let Some(key) = int_key(word) {
        if served(d, key) {
            let value = unsafe { pycc_rt_dict_get(d, key) };
            bigint_release(key);
            return value;
        }
        bigint_release(key);
    }
    crate::exception::raise_key_error(LIST_KIND_NUM, word);
    dict(d).sentinel()
}

/// `d.get(word, default)` for an `int`-keyed `d` and an `int | float`
/// word (borrowed): the value or `default` (adopted), a new reference.
///
/// # Safety
/// `d` must be a live dict with `int` keys whose value kind `default` has.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_dict_get_or_default_num_key(
    d: *mut PyDictObj,
    word: i64,
    default: i64,
) -> i64 {
    if let Some(key) = int_key(word) {
        let value = unsafe { super::pycc_rt_dict_get_or_default(d, key, default) };
        bigint_release(key);
        return value;
    }
    default
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::list::LIST_KIND_INT;
    use crate::tests::pending_tag_and_message;

    #[test]
    fn an_integral_float_finds_the_equal_int_key() {
        unsafe {
            let d = super::super::pycc_rt_dict_new(LIST_KIND_INT, LIST_KIND_INT);
            super::super::pycc_rt_dict_set(d, crate::tag_smallint(1), crate::tag_smallint(10));
            let one = crate::num::from_f64(1.0);
            assert_eq!(pycc_rt_dict_get_num_key(d, one), crate::tag_smallint(10));
            assert_eq!(
                pycc_rt_dict_get_or_default_num_key(d, crate::num::from_f64(1.5), crate::tag_smallint(7)),
                crate::tag_smallint(7)
            );
            assert_eq!(
                pycc_rt_dict_get_or_default_num_key(d, one, crate::tag_smallint(7)),
                crate::tag_smallint(10)
            );
            pycc_rt_dict_get_num_key(d, crate::num::from_f64(2.0));
            let (_, message) = pending_tag_and_message();
            assert_eq!(message, "2.0");
            crate::exception::pycc_rt_exception_clear();
            pycc_rt_dict_get_num_key(d, crate::num::from_f64(f64::NAN));
            let (_, message) = pending_tag_and_message();
            assert_eq!(message, "nan");
            crate::exception::pycc_rt_exception_clear();
            super::super::pycc_rt_dict_decref(d);
        }
    }
}
