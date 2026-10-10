//! `list` builtins and methods `pycc_hir::builtins` lowers to runtime calls:
//! ordering (`sorted`, `sort`, `min`, `max`), `sum`, `any`/`all`, and the
//! element-searching and copying methods.
//!
//! Ordering is CPython's: only `<` is consulted, and sorting is stable, so a
//! list holding a `nan` sorts exactly as CPython's merge-based sort leaves
//! it whenever `<` is a consistent order -- the merge sort here never
//! assumes totality.

use super::*;

/// CPython's `a < b` between two element words of `kind` (`TypeError` for
/// an unordered kind, which then compares `false`).
pub(crate) fn word_lt(kind: i64, a: i64, b: i64) -> bool {
    crate::value::lt_or_raise(kind, a, b)
}

fn word_truthy(kind: i64, word: i64) -> bool {
    crate::value::truthy(kind, word)
}

/// Runs `f` on the list's elements as words, writing them back (a
/// `list[bool]` stores one byte per element).
pub(crate) fn edit_words<R>(list: &mut PyListObj, f: impl FnOnce(&mut Vec<i64>) -> R) -> R {
    if list.kind == LIST_KIND_BOOL {
        let mut words: Vec<i64> = list.items().into_owned();
        let result = f(&mut words);
        list.with_typed_vec::<u8, _>(|bytes| {
            bytes.clear();
            bytes.extend(words.iter().map(|w| u8::from_word(*w)));
        });
        result
    } else {
        list.with_typed_vec::<i64, _>(f)
    }
}

/// Stable merge sort consulting only `lt`.
pub(crate) fn merge_sort(items: &mut Vec<i64>, lt: &dyn Fn(i64, i64) -> bool) {
    let n = items.len();
    if n < 2 {
        return;
    }
    // Insertion sort for short runs keeps small sorts cheap.
    if n <= 16 {
        for i in 1..n {
            let mut j = i;
            while j > 0 && lt(items[j], items[j - 1]) {
                items.swap(j, j - 1);
                j -= 1;
            }
        }
        return;
    }
    let mut right = items.split_off(n / 2);
    merge_sort(items, lt);
    merge_sort(&mut right, lt);
    let left = std::mem::take(items);
    items.reserve(n);
    let (mut i, mut j) = (0, 0);
    while i < left.len() && j < right.len() {
        // Take from the right only when strictly smaller: stability.
        if lt(right[j], left[i]) {
            items.push(right[j]);
            j += 1;
        } else {
            items.push(left[i]);
            i += 1;
        }
    }
    items.extend_from_slice(&left[i..]);
    items.extend_from_slice(&right[j..]);
}

fn sort_words(kind: i64, items: &mut Vec<i64>) {
    match kind {
        // Fast paths with a genuine total order on the words' values.
        LIST_KIND_INT if items.iter().all(|w| inline_int_value(*w).is_some()) => {
            items.sort_by_key(|w| inline_int_value(*w));
        }
        LIST_KIND_BOOL => items.sort(),
        LIST_KIND_STR => items.sort_by(|a, b| unsafe {
            (*(*a as *mut PyStrObj))
                .bytes()
                .cmp((*(*b as *mut PyStrObj)).bytes())
        }),
        _ => merge_sort(items, &|a, b| word_lt(kind, a, b)),
    }
}

/// `xs.sort()`.
///
/// # Safety
/// `list` must be a live `PyListObj` pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_list_sort(list: *mut PyListObj) {
    let list = unsafe { &mut *list };
    let kind = list.kind;
    edit_words(list, |items| sort_words(kind, items));
}

/// `sorted(xs)` as a new list.
///
/// # Safety
/// `list` must be a live `PyListObj` pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_list_sorted(list: *mut PyListObj) -> *mut PyListObj {
    let copy = unsafe { pycc_rt_list_copy(list) };
    unsafe { pycc_rt_list_sort(copy) };
    copy
}

/// `xs.copy()` / `list(xs)`.
///
/// # Safety
/// `list` must be a live `PyListObj` pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_list_copy(list: *mut PyListObj) -> *mut PyListObj {
    let list = unsafe { &*list };
    let items = list.items().to_vec();
    for word in &items {
        list.retain(*word);
    }
    new_list(list.kind, items)
}

/// `reversed(xs)`, materialized as a new list.
///
/// # Safety
/// `list` must be a live `PyListObj` pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_list_reversed(list: *mut PyListObj) -> *mut PyListObj {
    let copy = unsafe { pycc_rt_list_copy(list) };
    unsafe { pycc_rt_list_reverse(copy) };
    copy
}

/// `xs.reverse()`.
///
/// # Safety
/// `list` must be a live `PyListObj` pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_list_reverse(list: *mut PyListObj) {
    edit_words(unsafe { &mut *list }, |items| items.reverse());
}

/// `xs.clear()`.
///
/// # Safety
/// `list` must be a live `PyListObj` pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_list_clear(list: *mut PyListObj) {
    let list = unsafe { &mut *list };
    let old = edit_words(list, std::mem::take);
    for word in old {
        list.release(word);
    }
}

/// `xs.extend(ys)`.
///
/// # Safety
/// Both must be live `PyListObj` pointers of the same kind.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_list_extend(list: *mut PyListObj, other: *mut PyListObj) {
    let added = unsafe { &*other }.items().to_vec();
    let list = unsafe { &mut *list };
    for word in &added {
        list.retain(*word);
    }
    edit_words(list, |items| items.extend_from_slice(&added));
}

/// `xs += ys` on a list: extends `xs` in place and returns it (a new
/// reference, which the augmented assignment stores back).
///
/// # Safety
/// `list` and `other` must be live `PyListObj` pointers of one kind.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_list_iadd(
    list: *mut PyListObj,
    other: *mut PyListObj,
) -> *mut PyListObj {
    unsafe { pycc_rt_list_extend(list, other) };
    unsafe { crate::rc::pycc_rt_list_incref(list) };
    list
}

/// A repeat count as CPython's `PyNumber_AsSsize_t` reads it: an encoded
/// `int` outside the `Py_ssize_t` range raises `OverflowError`.
fn repeat_count(n: i64) -> Option<i64> {
    if let Some(v) = inline_int_value(n) {
        return Some(v);
    }
    let big = crate::int_arith::decode_big(n);
    let fits = num_traits::ToPrimitive::to_i64(&big);
    if fits.is_none() {
        raise_builtin(
            crate::exception::EXCEPTION_TYPE_OVERFLOW_ERROR,
            "OverflowError",
            "cannot fit 'int' into an index-sized integer",
        );
    }
    fits
}

/// `xs *= n` on a list: repeats `xs` in place (empties it for `n <= 0`)
/// and returns it (a new reference). `n` is an encoded `int`. A size that
/// cannot be allocated raises `MemoryError` before any element reference
/// changes, as CPython's `list_inplace_repeat` does.
///
/// # Safety
/// `list` must be a live `PyListObj` pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_list_imul(list: *mut PyListObj, n: i64) -> *mut PyListObj {
    unsafe { crate::rc::pycc_rt_list_incref(list) };
    let Some(count) = repeat_count(n) else {
        return list;
    };
    let list_ref = unsafe { &mut *list };
    let items = list_ref.items().to_vec();
    if items.is_empty() || count == 1 {
        return list;
    }
    if count <= 0 {
        // Detach first, then release: a released element may run code.
        edit_words(list_ref, |all| all.clear());
        for word in &items {
            list_ref.release(*word);
        }
        return list;
    }
    let count = count as usize;
    let fits = items
        .len()
        .checked_mul(count)
        .filter(|total| *total <= isize::MAX as usize / std::mem::size_of::<i64>());
    let reserved = fits.is_some_and(|total| {
        edit_words(list_ref, |all| all.try_reserve_exact(total - all.len()).is_ok())
    });
    if !reserved {
        raise_builtin(
            crate::exception::EXCEPTION_TYPE_MEMORY_ERROR,
            "MemoryError",
            "",
        );
        return list;
    }
    for _ in 1..count {
        for word in &items {
            list_ref.retain(*word);
        }
    }
    edit_words(list_ref, |all| {
        for _ in 1..count {
            all.extend_from_slice(&items);
        }
    });
    list
}

/// `xs.count(x)`.
///
/// # Safety
/// `list` must be a live `PyListObj` pointer and `word` a valid word of its
/// kind (borrowed).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_list_count(list: *mut PyListObj, word: i64) -> i64 {
    let list = unsafe { &*list };
    let n = list
        .items()
        .iter()
        .filter(|item| words_equal(list.kind, **item, word))
        .count();
    tag_smallint(n as i64)
}

fn word_repr_string(kind: i64, word: i64) -> String {
    let mut out = String::new();
    word_repr(kind, word, &mut out);
    out
}

/// `xs.index(x)`.
///
/// # Safety
/// As [`pycc_rt_list_count`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_list_index(list: *mut PyListObj, word: i64) -> i64 {
    let list = unsafe { &*list };
    match list
        .items()
        .iter()
        .position(|item| words_equal(list.kind, *item, word))
    {
        Some(i) => tag_smallint(i as i64),
        None => {
            raise_builtin(
                EXCEPTION_TYPE_VALUE_ERROR,
                "ValueError",
                &format!("{} is not in list", word_repr_string(list.kind, word)),
            );
            tag_smallint(0)
        }
    }
}

/// `xs.remove(x)`.
///
/// # Safety
/// As [`pycc_rt_list_count`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_list_remove(list: *mut PyListObj, word: i64) {
    let list = unsafe { &mut *list };
    let kind = list.kind;
    match list
        .items()
        .iter()
        .position(|item| words_equal(kind, *item, word))
    {
        Some(i) => {
            let removed = edit_words(list, |items| items.remove(i));
            list.release(removed);
        }
        None => raise_builtin(
            EXCEPTION_TYPE_VALUE_ERROR,
            "ValueError",
            "list.remove(x): x not in list",
        ),
    }
}

/// `del xs[index]` for an encoded int `index`: CPython's `IndexError`s for
/// an out-of-range or too-large index.
///
/// # Safety
/// `list` must be a live `PyListObj` pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_list_delitem(list: *mut PyListObj, index: i64) {
    let list = unsafe { &mut *list };
    let Some(index) = crate::int_encoding::inline_int_value(index) else {
        raise_builtin(
            EXCEPTION_TYPE_INDEX_ERROR,
            "IndexError",
            "cannot fit 'int' into an index-sized integer",
        );
        return;
    };
    match resolve_index(index, list.len) {
        Some(i) => {
            let removed = edit_words(list, |items| items.remove(i));
            list.release(removed);
        }
        None => raise_builtin(
            EXCEPTION_TYPE_INDEX_ERROR,
            "IndexError",
            "list assignment index out of range",
        ),
    }
}

fn extreme(list: *mut PyListObj, name: &str, better: impl Fn(i64, i64, i64) -> bool) -> i64 {
    let list = unsafe { &*list };
    let items = list.items();
    let Some(mut best) = items.first().copied() else {
        raise_builtin(
            EXCEPTION_TYPE_VALUE_ERROR,
            "ValueError",
            &format!("{name}() iterable argument is empty"),
        );
        return list.sentinel();
    };
    for &word in &items[1..] {
        if better(list.kind, word, best) {
            best = word;
        }
    }
    list.retain(best);
    best
}

/// `min(xs)`: the first smallest element's word (a new reference).
///
/// # Safety
/// `list` must be a live `PyListObj` pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_list_min(list: *mut PyListObj) -> i64 {
    extreme(list, "min", word_lt)
}

/// `max(xs)`: the first largest element's word (a new reference).
///
/// # Safety
/// `list` must be a live `PyListObj` pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_list_max(list: *mut PyListObj) -> i64 {
    extreme(list, "max", |kind, word, best| word_lt(kind, best, word))
}

/// `sum(xs)` of a `list[int]` or `list[bool]`.
///
/// # Safety
/// `list` must be a live `PyListObj` pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_list_sum_int(list: *mut PyListObj) -> i64 {
    let list = unsafe { &*list };
    let mut small: i64 = 0;
    let mut acc: Option<i64> = None;
    for &word in list.items().iter() {
        let value = if list.kind == LIST_KIND_BOOL {
            tag_smallint(word)
        } else {
            word
        };
        if acc.is_none()
            && let Some(v) = inline_int_value(value)
            && let Some(next) = small.checked_add(v)
        {
            small = next;
            continue;
        }
        let current = acc.unwrap_or_else(|| {
            fits_smallint(small).unwrap_or_else(|| tag_bigint(bigint_from_i128(small as i128)))
        });
        let next = pycc_rt_int_add(current, value);
        if acc.is_some() {
            bigint_release(current);
        }
        acc = Some(next);
    }
    acc.unwrap_or_else(|| {
        fits_smallint(small).unwrap_or_else(|| tag_bigint(bigint_from_i128(small as i128)))
    })
}

/// `sum(xs)` of a `list[float]`, with CPython 3.12+'s Neumaier-compensated
/// summation.
///
/// # Safety
/// `list` must be a live `PyListObj` pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_list_sum_float(list: *mut PyListObj) -> f64 {
    let list = unsafe { &*list };
    let (mut sum, mut c) = (0.0f64, 0.0f64);
    for &word in list.items().iter() {
        let x = f64::from_bits(word as u64);
        let t = sum + x;
        if sum.abs() >= x.abs() {
            c += (sum - t) + x;
        } else {
            c += (x - t) + sum;
        }
        sum = t;
    }
    if c != 0.0 && c.is_finite() {
        sum += c;
    }
    sum
}

/// `any(xs)`.
///
/// # Safety
/// `list` must be a live `PyListObj` pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_list_any(list: *mut PyListObj) -> i8 {
    let list = unsafe { &*list };
    i8::from(list.items().iter().any(|w| word_truthy(list.kind, *w)))
}

/// `all(xs)`.
///
/// # Safety
/// `list` must be a live `PyListObj` pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_list_all(list: *mut PyListObj) -> i8 {
    let list = unsafe { &*list };
    i8::from(list.items().iter().all(|w| word_truthy(list.kind, *w)))
}

/// `a, b = xs`: checks `xs` has exactly `arity` (an encoded int) elements,
/// raising CPython's `ValueError` otherwise, and returns `xs`.
///
/// # Safety
/// `list` must be a live `PyListObj` pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_list_unpack(list: *mut PyListObj, arity: i64) -> *mut PyListObj {
    let len = unsafe { &*list }.len;
    let expected = inline_int_value(arity).unwrap_or(0);
    if len != expected {
        let message = if len > expected {
            format!("too many values to unpack (expected {expected})")
        } else {
            format!("not enough values to unpack (expected {expected}, got {len})")
        };
        raise_builtin(EXCEPTION_TYPE_VALUE_ERROR, "ValueError", &message);
    }
    list
}
