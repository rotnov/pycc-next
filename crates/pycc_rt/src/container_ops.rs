//! Builtins over containers that `pycc_hir::builtins` lowers to runtime
//! calls: `enumerate`, `zip`, the `set`/`list`/`dict`/`sorted`/`min`/`max`
//! conversions between container types, and the dict and set methods that
//! take or return element words.
//!
//! The ABI is `pycc_hir::builtins`': every argument is borrowed, and a
//! returned container or element word is a new reference. Kinds a result
//! needs that its arguments do not carry are passed as trailing `i64` kind
//! arguments (`BuiltinSig::kind_args`).

use crate::dict::{
    PyDictObj, pycc_rt_dict_copy, pycc_rt_dict_from_pairs, pycc_rt_dict_keys, pycc_rt_dict_pop,
    pycc_rt_dict_setdefault,
};
use crate::list::{LIST_KIND_INT, LIST_KIND_TUPLE, PyListObj, new_raw_words};
use crate::set::{PySetObj, pycc_rt_set_from_list, pycc_rt_set_to_list};
use crate::value;

fn list<'a>(list: *mut PyListObj) -> &'a PyListObj {
    unsafe { &*list }
}

/// `list(enumerate(xs, start))`: `(index, element)` tuples, the index
/// starting at the encoded int `start`.
///
/// # Safety
/// `xs` must be a live list.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_list_enumerate(xs: *mut PyListObj, start: i64) -> *mut PyListObj {
    let items = list(xs).kinded_items();
    let start = crate::inline_int_value(start).unwrap_or(0);
    let tuples: Vec<i64> = items
        .into_iter()
        .enumerate()
        .map(|(i, (kind, word))| {
            value::retain(kind, word);
            let index = crate::tag_smallint(start + i as i64);
            crate::tuple::tuple_from(
                vec![index, word],
                vec![LIST_KIND_INT as u8, kind as u8].into(),
            ) as i64
        })
        .collect();
    new_raw_words(LIST_KIND_TUPLE, tuples)
}

/// `list(zip(*lists))` for the `count` lists at `lists`: tuples of the
/// items at each position, as long as the shortest list.
fn zip(lists: &[*mut PyListObj]) -> *mut PyListObj {
    let columns: Vec<Vec<(i64, i64)>> = lists.iter().map(|xs| list(*xs).kinded_items()).collect();
    let rows = columns.iter().map(Vec::len).min().unwrap_or(0);
    let tuples: Vec<i64> = (0..rows)
        .map(|row| {
            let mut words = Vec::with_capacity(columns.len());
            let mut kinds = Vec::with_capacity(columns.len());
            for column in &columns {
                let (kind, word) = column[row];
                value::retain(kind, word);
                words.push(word);
                kinds.push(kind as u8);
            }
            crate::tuple::tuple_from(words, kinds.into()) as i64
        })
        .collect();
    new_raw_words(LIST_KIND_TUPLE, tuples)
}

/// `list(zip(a, b))`.
///
/// # Safety
/// `a` and `b` must be live lists.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_list_zip2(a: *mut PyListObj, b: *mut PyListObj) -> *mut PyListObj {
    zip(&[a, b])
}

/// `list(zip(a, b, c))`.
///
/// # Safety
/// `a`, `b` and `c` must be live lists.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_list_zip3(
    a: *mut PyListObj,
    b: *mut PyListObj,
    c: *mut PyListObj,
) -> *mut PyListObj {
    zip(&[a, b, c])
}

/// `set(xs)` of a list, its element kind the list's.
///
/// # Safety
/// `xs` must be a live list.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_set_of_list(xs: *mut PyListObj) -> *mut PySetObj {
    let kind = list(xs).kind();
    unsafe { pycc_rt_set_from_list(xs, kind) }
}

/// `sorted(s)` of a set as a new list.
///
/// # Safety
/// `s` must be a live set.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_set_sorted(s: *mut PySetObj) -> *mut PyListObj {
    let items = unsafe { pycc_rt_set_to_list(s) };
    unsafe { crate::list::ops::pycc_rt_list_sort(items) };
    items
}

/// `sorted(d)` of a dict: its keys, sorted, as a new list.
///
/// # Safety
/// `d` must be a live dict.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_dict_sorted(d: *mut PyDictObj) -> *mut PyListObj {
    let keys = unsafe { pycc_rt_dict_keys(d) };
    unsafe { crate::list::ops::pycc_rt_list_sort(keys) };
    keys
}

/// `min(s)`/`max(s)` of a set (`max != 0` for `max`): CPython's first
/// extreme in iteration order, a new reference; `ValueError` when empty.
///
/// # Safety
/// `s` must be a live set.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_set_extreme(s: *mut PySetObj, max: i64) -> i64 {
    let items = unsafe { pycc_rt_set_to_list(s) };
    let word = if crate::inline_int_value(max) == Some(0) {
        unsafe { crate::list::ops::pycc_rt_list_min(items) }
    } else {
        unsafe { crate::list::ops::pycc_rt_list_max(items) }
    };
    unsafe { crate::pycc_rt_list_decref(items) };
    word
}

/// `dict(pairs)` of a list of `(key, value)` tuples, keys of `key_kind`
/// and values of `value_kind`.
///
/// # Safety
/// `pairs` must be a live list of two-element tuples.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_dict_of_pairs(
    pairs: *mut PyListObj,
    key_kind: i64,
    value_kind: i64,
) -> *mut PyDictObj {
    unsafe { pycc_rt_dict_from_pairs(pairs, key_kind, value_kind) }
}

/// `dict(d)` / `d.copy()`.
///
/// # Safety
/// `d` must be a live dict.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_dict_copy_of(d: *mut PyDictObj) -> *mut PyDictObj {
    unsafe { pycc_rt_dict_copy(d) }
}

/// `d.pop(key)`, `key` borrowed: the removed value's reference.
///
/// # Safety
/// `d` must be a live dict.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_dict_pop_key(d: *mut PyDictObj, key: i64) -> i64 {
    unsafe { pycc_rt_dict_pop(d, key) }
}

/// `d.pop(key, default)`, both borrowed: the removed value, or a new
/// reference to `default`.
///
/// # Safety
/// `d` must be a live dict whose value kind is `value_kind`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_dict_pop_or(
    d: *mut PyDictObj,
    key: i64,
    default: i64,
    value_kind: i64,
) -> i64 {
    value::retain(value_kind, default);
    unsafe { crate::dict::pycc_rt_dict_pop_default(d, key, default) }
}

/// `d.setdefault(key, default)`, both borrowed: a new reference to the
/// stored value, inserting `default` first when `key` is absent.
///
/// # Safety
/// `d` must be a live dict with keys of `key_kind` and values of
/// `value_kind`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_dict_setdefault_of(
    d: *mut PyDictObj,
    key: i64,
    default: i64,
    key_kind: i64,
    value_kind: i64,
) -> i64 {
    value::retain(key_kind, key);
    value::retain(value_kind, default);
    unsafe { pycc_rt_dict_setdefault(d, key, default) }
}

/// `list(d)` / `list(s)` style conversions keep their own names in
/// `dict.rs` and `set.rs`; this one is `list(t)` of a tuple whose
/// elements share one kind.
///
/// # Safety
/// `t` must be a live tuple object.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_tuple_to_list(t: *mut PyListObj, kind: i64) -> *mut PyListObj {
    let words: Vec<i64> = list(t)
        .kinded_items()
        .into_iter()
        .map(|(k, word)| {
            value::retain(k, word);
            word
        })
        .collect();
    new_raw_words(kind, words)
}

/// `list(s)` of a set (a new list in iteration order).
///
/// # Safety
/// `s` must be a live set.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_set_list(s: *mut PySetObj) -> *mut PyListObj {
    unsafe { pycc_rt_set_to_list(s) }
}

/// `list(enumerate(xs))`.
///
/// # Safety
/// `xs` must be a live list.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_list_enumerate0(xs: *mut PyListObj) -> *mut PyListObj {
    unsafe { pycc_rt_list_enumerate(xs, crate::tag_smallint(0)) }
}

/// `min(s)` of a set.
///
/// # Safety
/// `s` must be a live set.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_set_extreme_min(s: *mut PySetObj) -> i64 {
    unsafe { pycc_rt_set_extreme(s, crate::tag_smallint(0)) }
}

/// `max(s)` of a set.
///
/// # Safety
/// `s` must be a live set.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_set_extreme_max(s: *mut PySetObj) -> i64 {
    unsafe { pycc_rt_set_extreme(s, crate::tag_smallint(1)) }
}

/// `a >= b` (`a.issuperset(b)`).
///
/// # Safety
/// `a` and `b` must be live sets of one kind.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_set_issuperset(a: *mut PySetObj, b: *mut PySetObj) -> i8 {
    unsafe { crate::set::pycc_rt_set_issubset(b, a) }
}

/// The `(key, element)` pairs of a `[(f(v), v) for v in xs]` list (the
/// `key=` rewrite of `pycc_ast::desugar::keys`), borrowed.
fn keyed_pairs(pairs: *mut PyListObj) -> Vec<((i64, i64), (i64, i64))> {
    list(pairs)
        .kinded_items()
        .into_iter()
        .map(|(_, tuple)| {
            let items = list(tuple as *mut PyListObj).kinded_items();
            (items[0], items[1])
        })
        .collect()
}

/// The positions of `pairs` in CPython's `sorted(..., key=...,
/// reverse=...)` order: a stable sort consulting only `<` on the keys, and
/// for `reverse` CPython's own reverse-sort-reverse, which keeps equal keys
/// in their original order.
fn order_by_keys(pairs: &[((i64, i64), (i64, i64))], reverse: bool) -> Vec<i64> {
    let mut positions: Vec<i64> = (0..pairs.len() as i64).collect();
    if reverse {
        positions.reverse();
    }
    crate::list::ops::merge_sort(&mut positions, &|a, b| {
        let ((kind, x), (_, y)) = (pairs[a as usize].0, pairs[b as usize].0);
        value::lt_or_raise(kind, x, y)
    });
    if reverse {
        positions.reverse();
    }
    positions
}

/// `sorted(xs, key=..., reverse=...)` as a new list of `kind` words, from
/// the `(key, element)` pairs of `xs`.
///
/// # Safety
/// `pairs` must be a live list of 2-tuples.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_list_sorted_keyed(
    pairs: *mut PyListObj,
    reverse: i8,
    kind: i64,
) -> *mut PyListObj {
    let items = keyed_pairs(pairs);
    let words: Vec<i64> = order_by_keys(&items, reverse != 0)
        .iter()
        .map(|i| {
            let (kind, word) = items[*i as usize].1;
            value::retain(kind, word);
            word
        })
        .collect();
    new_raw_words(kind, words)
}

/// `xs.sort(key=..., reverse=...)`: `xs` reordered in place from its
/// `(key, element)` pairs.
///
/// # Safety
/// `xs` must be a live list and `pairs` a live list of 2-tuples of it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_list_sort_keyed(
    xs: *mut PyListObj,
    pairs: *mut PyListObj,
    reverse: i8,
) {
    let sorted = unsafe { pycc_rt_list_sorted_keyed(pairs, reverse, list(xs).kind()) };
    unsafe {
        crate::list::ops::pycc_rt_list_clear(xs);
        crate::list::ops::pycc_rt_list_extend(xs, sorted);
        crate::rc::pycc_rt_list_decref(sorted);
    }
}

/// `min(xs, key=...)` / `max(xs, key=...)` from the `(key, element)` pairs:
/// the first element with the smallest (largest) key, a new reference;
/// `ValueError` when empty.
fn extreme_keyed(pairs: *mut PyListObj, kind: i64, max: bool) -> i64 {
    let items = keyed_pairs(pairs);
    if items.is_empty() {
        let name = if max { "max" } else { "min" };
        crate::exception::raise_builtin(
            crate::exception::EXCEPTION_TYPE_VALUE_ERROR,
            "ValueError",
            &format!("{name}() iterable argument is empty"),
        );
        return crate::dict::sentinel(kind);
    }
    let mut best = 0;
    for i in 1..items.len() {
        let ((kind, candidate), (_, current)) = (items[i].0, items[best].0);
        let better = if max {
            value::lt_or_raise(kind, current, candidate)
        } else {
            value::lt_or_raise(kind, candidate, current)
        };
        if better {
            best = i;
        }
    }
    let (kind, word) = items[best].1;
    value::retain(kind, word);
    word
}

/// `min(xs, key=...)`.
///
/// # Safety
/// `pairs` must be a live list of 2-tuples.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_list_min_keyed(pairs: *mut PyListObj, kind: i64) -> i64 {
    extreme_keyed(pairs, kind, false)
}

/// `max(xs, key=...)`.
///
/// # Safety
/// `pairs` must be a live list of 2-tuples.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_list_max_keyed(pairs: *mut PyListObj, kind: i64) -> i64 {
    extreme_keyed(pairs, kind, true)
}
