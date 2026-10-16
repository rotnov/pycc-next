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
//! [`pycc_rt_tuple_new`], and never change; its kinds live in the spare
//! capacity of the same data allocation ([`tuple_from`]).
//!
//! # Native containers of objects (v0.5)
//!
//! A tuple object's element may be a CPython object itself
//! (`LIST_KIND_OBJECT`, counted through the object hooks): `tuple[int, Any]`
//! holds its position as a word and its value as the `PyObject *`, boxing
//! nothing until the tuple crosses into CPython. `tuple[T, ...]` (a
//! variadic tuple of `str`, instances or containers) is the same object,
//! so a fixed tuple of matching elements is one with no conversion.
//!
//! Crossing into CPython, a tuple object is **copied**: neither a proxy
//! nor a CPython-owned value. A proxy type would not be a `tuple`
//! (`type(t) is tuple`, `isinstance`, the C API's `PyTuple_Check` in every
//! extension the value reaches), and keeping every tuple CPython-owned
//! would cost a C API call per element read in the code that builds and
//! slices them. A copy is exact for an immutable value except for
//! identity, so identity is kept where it can be: the copy is made once and
//! remembered (`PyListObj::foreign`), the same native tuple always crosses
//! as the same CPython tuple, and a tuple copied in from CPython remembers
//! the original, which is what crosses back (`f(t) is t`). The copy shares
//! every object element and packs `int`/`float`/`str` elements; a tuple
//! whose elements are mutable (lists, dicts, sets, instances) is not copied
//! at all -- the boundary refuses it (`C0003`) rather than break the
//! aliasing between the caller's list and the copy's.
//!
//! Mutable containers of objects (`list[object]`, `dict[str, Any]`) stay
//! CPython-owned for the same reason: CPython code may hold and mutate the
//! very object compiled code mutates, so a native copy, converted lazily or
//! not, would diverge. Compiled code operates on them through the object
//! hooks (`pycc_ext_obj_*`).

use crate::list::PyListObj;
use crate::value::TUPLE_OBJECT;

/// A tuple object's per-element kinds.
pub(crate) fn kinds_of(tuple: &PyListObj) -> &[u8] {
    if tuple.kinds.is_null() {
        return &[];
    }
    unsafe { std::slice::from_raw_parts(tuple.kinds, tuple.len() as usize) }
}

/// Detaches a dying tuple object's kinds, which live in its data
/// allocation's spare capacity ([`tuple_from`]) and go with it.
pub(crate) fn free_kinds(tuple: &mut PyListObj) {
    tuple.kinds = std::ptr::null_mut();
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
    let mut words = tuple_words(len);
    words.resize(len, 0);
    tuple_from(words, unsafe { std::slice::from_raw_parts(kinds, len) })
}

/// Element `i` of `seq`, a tuple object or a list, as (kind, word); `i`
/// must be in range.
fn item(seq: &PyListObj, i: usize) -> (i64, i64) {
    if seq.kind() == TUPLE_OBJECT {
        (i64::from(kinds_of(seq)[i]), seq.word_at(i))
    } else {
        (seq.kind(), seq.word_at(i))
    }
}

/// A new tuple object of the elements of `seqs` at `positions` (each a
/// (sequence, index) pair), taking one more reference to each. Builds the
/// words and kinds directly: a tuple object costs its three allocations
/// and nothing more (tomli's keys are sliced and extended in its hot loop).
fn tuple_of(len: usize, positions: impl Iterator<Item = (i64, i64)>) -> *mut PyListObj {
    let mut words = tuple_words(len);
    let mut kinds = Vec::with_capacity(len);
    for (kind, word) in positions {
        crate::value::retain(kind, word);
        words.push(word);
        kinds.push(kind as u8);
    }
    tuple_from(words, &kinds)
}

/// `t[start:stop:step]` of a tuple object (`pycc_rt_list_slice` routes a
/// `tuple[T, ...]` here): a new tuple object.
pub(crate) fn slice(t: &PyListObj, start: i64, stop: i64, step: i64, present: i64) -> *mut PyListObj {
    match crate::list::slice_indices(t.len(), start, stop, step, present) {
        Some(positions) => {
            let positions: Vec<usize> = positions.collect();
            tuple_of(positions.len(), positions.into_iter().map(|i| item(t, i)))
        }
        // A zero step raised `ValueError`; see `pycc_rt_list_slice`.
        None => std::ptr::null_mut(),
    }
}

/// `a + b` where either is a tuple object: a new tuple object holding the
/// elements of both (`tuple[T, ...] + tuple[T, T]`).
pub(crate) fn concat(a: &PyListObj, b: &PyListObj) -> *mut PyListObj {
    let (la, lb) = (a.len() as usize, b.len() as usize);
    tuple_of(
        la + lb,
        (0..la).map(|i| item(a, i)).chain((0..lb).map(|i| item(b, i))),
    )
}

/// `t * count` of a tuple object; a non-positive count gives `()`.
pub(crate) fn repeat(t: &PyListObj, count: i64) -> *mut PyListObj {
    let len = t.len() as usize;
    let count = count.max(0) as usize;
    tuple_of(len * count, (0..len * count).map(|i| item(t, i % len)))
}

/// `word in t` of a tuple object, `word` having kind `kind` (the static
/// element type of a `tuple[T, ...]`).
pub(crate) fn contains(t: &PyListObj, word: i64) -> bool {
    (0..t.len() as usize).any(|i| {
        let (kind, element) = item(t, i);
        crate::value::eq(kind, element, word)
    })
}

/// The CPython `tuple` `t` shares when it crosses into CPython, or null
/// (`PyListObj::foreign`). Only a tuple object shares one: a function's
/// `*args` is a list marked as a tuple, which the program may still append
/// to.
///
/// # Safety
/// `t` must be a live tuple object or list.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_tuple_foreign(t: *mut PyListObj) -> *mut std::ffi::c_void {
    unsafe { (*t).foreign }
}

/// Makes `object` (a CPython `tuple` of `t`'s elements, whose reference
/// this adopts) the one `t` shares, releasing any earlier one; for a list
/// (see [`pycc_rt_tuple_foreign`]) the reference is released at once.
///
/// # Safety
/// `t` must be a live tuple object or list.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_tuple_set_foreign(t: *mut PyListObj, object: *mut std::ffi::c_void) {
    let t = unsafe { &mut *t };
    if t.kind() != TUPLE_OBJECT {
        crate::rc::release_object(object);
        return;
    }
    let old = std::mem::replace(&mut t.foreign, object);
    if !old.is_null() {
        crate::rc::release_object(old);
    }
}

/// Writes element `index` of a new tuple object (`pycc_rt_tuple_new`),
/// which adopts the caller's reference to `word`.
///
/// # Safety
/// `t` must be a tuple object `pycc_rt_tuple_new` made and nothing has read
/// yet, `index` in range.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_tuple_set_word(t: *mut PyListObj, index: i64, word: i64) {
    let t = unsafe { &mut *t };
    debug_assert!(index >= 0 && index < t.len());
    unsafe { *t.data.cast::<i64>().add(index as usize) = word };
}

/// Room for kinds bytes after `len` words: [`tuple_from`] stores a tuple
/// object's kinds in its data allocation, so a tuple costs two allocations
/// (the object and its data) rather than three.
fn kinds_words(len: usize) -> usize {
    len.div_ceil(std::mem::size_of::<i64>())
}

/// An empty word vector with capacity for `len` words and their kinds
/// ([`tuple_from`] then needs no reallocation).
pub(crate) fn tuple_words(len: usize) -> Vec<i64> {
    Vec::with_capacity(len + kinds_words(len))
}

/// A tuple object owning `words` (one reference each) of the given kinds.
///
/// The kinds are copied into the spare capacity after the words (reserved
/// here unless [`tuple_words`] made `words`): a tuple object's words never
/// change after it is built, so its data is never reallocated and the
/// kinds stay put until the data is freed with the tuple.
pub(crate) fn tuple_from(mut words: Vec<i64>, kinds: &[u8]) -> *mut PyListObj {
    debug_assert_eq!(words.len(), kinds.len());
    let len = words.len();
    words.reserve_exact(kinds_words(len));
    let spare = words.spare_capacity_mut();
    let at = spare.as_mut_ptr().cast::<u8>();
    // SAFETY: the spare capacity holds at least `kinds_words(len)` words,
    // `kinds.len()` bytes or more, and nothing else uses it.
    unsafe { std::ptr::copy_nonoverlapping(kinds.as_ptr(), at, kinds.len()) };
    let tuple = crate::list::new_raw(TUPLE_OBJECT, words);
    // Non-null even when empty: a null `kinds` is what marks a list.
    unsafe { (*tuple).kinds = at };
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
            &[LIST_KIND_INT as u8, LIST_KIND_STR as u8],
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
        let one = tuple_from(vec![int(7)], &[LIST_KIND_INT as u8]) as i64;
        assert_eq!(render(LIST_KIND_TUPLE, one), "(7,)");
        // hash((1, 2)) on CPython 3.13.
        let ints = tuple_from(
            vec![int(1), int(2)],
            &[LIST_KIND_INT as u8, LIST_KIND_INT as u8],
        ) as i64;
        assert_eq!(
            value::hash(LIST_KIND_TUPLE, ints),
            Some(-3_550_055_125_485_641_917)
        );
        for word in [a, b, c, one, ints] {
            value::release(LIST_KIND_TUPLE, word);
        }
    }

    #[test]
    fn slices_concatenation_repetition_and_membership_keep_kinds() {
        let a = pair(1, "x");
        let t = unsafe { &*(a as *mut PyListObj) };
        let tail = slice(t, 1, 0, 1, 1);
        assert_eq!(render(LIST_KIND_TUPLE, tail as i64), "('x',)");
        let both = concat(t, unsafe { &*tail });
        assert_eq!(render(LIST_KIND_TUPLE, both as i64), "(1, 'x', 'x')");
        let twice = repeat(unsafe { &*tail }, 2);
        assert_eq!(render(LIST_KIND_TUPLE, twice as i64), "('x', 'x')");
        let none = repeat(t, -1);
        assert_eq!(render(LIST_KIND_TUPLE, none as i64), "()");
        let probe = str_word("x");
        assert!(contains(unsafe { &*twice }, probe));
        assert!(!contains(unsafe { &*none }, probe));
        value::release(LIST_KIND_STR, probe);
        for word in [a, tail as i64, both as i64, twice as i64, none as i64] {
            value::release(LIST_KIND_TUPLE, word);
        }
    }
}
