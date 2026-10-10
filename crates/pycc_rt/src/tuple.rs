//! Tuple objects: a boxed tuple is a `PyListObj` whose kind is
//! [`TUPLE_OBJECT`] and which records each element's kind, so one runtime
//! object serves every tuple type, its elements of any kind.
//!
//! Compiled code holds a tuple of only `int`/`bool`/`float` elements by
//! value (an LLVM struct) and boxes it into one of these only to store it
//! in a container; every other tuple type is always one of these. Being a
//! `PyListObj`, a tuple object is reference counted exactly as a list is
//! (`crate::rc`): it owns one reference to each counted element, and
//! `pycc_codegen` reads and writes its words inline through the shared
//! `{len, data}` prefix. Its words are written once, right after
//! [`pycc_rt_tuple_new`], and never change.

use crate::list::PyListObj;
use crate::value::TUPLE_OBJECT;

/// A tuple object's per-element kinds.
pub(crate) fn kinds_of(tuple: &PyListObj) -> &[u8] {
    if tuple.kinds.is_null() {
        return &[];
    }
    unsafe { std::slice::from_raw_parts(tuple.kinds, tuple.len() as usize) }
}

/// Frees a dying tuple object's kinds array.
pub(crate) fn free_kinds(tuple: &mut PyListObj) {
    if !tuple.kinds.is_null() {
        let len = tuple.len() as usize;
        drop(unsafe { Box::from_raw(std::ptr::slice_from_raw_parts_mut(tuple.kinds, len)) });
        tuple.kinds = std::ptr::null_mut();
    }
}

/// A new tuple object of `len` elements whose kinds are `kinds[..len]`,
/// every word zero until the caller writes it (adopting one reference per
/// counted element). Count one.
///
/// # Safety
/// `kinds` must be valid for `len` reads.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_tuple_new(len: i64, kinds: *const u8) -> *mut PyListObj {
    let len = len.max(0) as usize;
    let kinds: Box<[u8]> = unsafe { std::slice::from_raw_parts(kinds, len) }.into();
    tuple_from(vec![0; len], kinds)
}

/// A tuple object owning `words` (one reference each) of the given kinds.
pub(crate) fn tuple_from(words: Vec<i64>, kinds: Box<[u8]>) -> *mut PyListObj {
    debug_assert_eq!(words.len(), kinds.len());
    let tuple = crate::list::new_raw(TUPLE_OBJECT, words);
    unsafe { (*tuple).kinds = Box::into_raw(kinds).cast::<u8>() };
    tuple
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::list::{LIST_KIND_INT, LIST_KIND_STR, LIST_KIND_TUPLE};
    use crate::value;

    fn str_word(text: &str) -> i64 {
        crate::new_pystr(text.as_bytes()) as i64
    }

    fn int(v: i64) -> i64 {
        crate::tag_smallint(v)
    }

    fn pair(a: i64, b: &str) -> i64 {
        tuple_from(
            vec![int(a), str_word(b)],
            vec![LIST_KIND_INT as u8, LIST_KIND_STR as u8].into(),
        ) as i64
    }

    fn render(kind: i64, word: i64) -> String {
        let mut out = String::new();
        assert!(value::repr(kind, word, &mut out));
        out
    }

    #[test]
    fn tuples_render_compare_and_hash_like_cpython() {
        let a = pair(1, "x");
        let b = pair(1, "y");
        let c = pair(1, "x");
        assert_eq!(render(LIST_KIND_TUPLE, a), "(1, 'x')");
        assert!(value::eq(LIST_KIND_TUPLE, a, c));
        assert!(!value::eq(LIST_KIND_TUPLE, a, b));
        assert_eq!(value::lt(LIST_KIND_TUPLE, a, b), Some(true));
        assert_eq!(value::lt(LIST_KIND_TUPLE, b, a), Some(false));
        assert_eq!(
            value::hash(LIST_KIND_TUPLE, a),
            value::hash(LIST_KIND_TUPLE, c)
        );
        let one = tuple_from(vec![int(7)], vec![LIST_KIND_INT as u8].into()) as i64;
        assert_eq!(render(LIST_KIND_TUPLE, one), "(7,)");
        // hash((1, 2)) on CPython 3.13.
        let ints = tuple_from(
            vec![int(1), int(2)],
            vec![LIST_KIND_INT as u8, LIST_KIND_INT as u8].into(),
        ) as i64;
        assert_eq!(
            value::hash(LIST_KIND_TUPLE, ints),
            Some(-3_550_055_125_485_641_917)
        );
        for word in [a, b, c, one, ints] {
            value::release(LIST_KIND_TUPLE, word);
        }
    }
}
