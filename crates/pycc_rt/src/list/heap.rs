//! `heapq` and `bisect` over a native list, with the exact algorithms of
//! CPython's `heapq.py`/`_heapqmodule.c` and `_bisectmodule.c` (only `<` is
//! consulted), so ties land where CPython puts them.
//!
//! An element argument is a borrowed word; the list takes its own
//! reference. A returned element is a new reference.

use super::ops::{edit_words, word_lt};
use super::*;

/// `heapq._siftdown`: moves the item at `pos` up towards `start`.
fn sift_down(kind: i64, heap: &mut [i64], start: usize, mut pos: usize) {
    let item = heap[pos];
    while pos > start {
        let parent = (pos - 1) >> 1;
        if word_lt(kind, item, heap[parent]) {
            heap[pos] = heap[parent];
            pos = parent;
            continue;
        }
        break;
    }
    heap[pos] = item;
}

/// `heapq._siftup`: moves the smaller child up until a leaf, then sifts
/// the item at `pos` down from there.
fn sift_up(kind: i64, heap: &mut [i64], mut pos: usize) {
    let end = heap.len();
    let start = pos;
    let item = heap[pos];
    let mut child = 2 * pos + 1;
    while child < end {
        let right = child + 1;
        if right < end && !word_lt(kind, heap[child], heap[right]) {
            child = right;
        }
        heap[pos] = heap[child];
        pos = child;
        child = 2 * pos + 1;
    }
    heap[pos] = item;
    sift_down(kind, heap, start, pos);
}

fn index_error() {
    raise_builtin(EXCEPTION_TYPE_INDEX_ERROR, "IndexError", "index out of range");
}

/// `heapq.heappush(heap, item)`.
///
/// # Safety
/// `list` must be a live `PyListObj` pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_heappush(list: *mut PyListObj, word: i64) {
    let list = unsafe { &mut *list };
    list.retain(word);
    let kind = list.kind;
    edit_words(list, |items| {
        items.push(word);
        let last = items.len() - 1;
        sift_down(kind, items, 0, last);
    });
}

/// `heapq.heappop(heap)`: the smallest item (`IndexError` when empty).
///
/// # Safety
/// `list` must be a live `PyListObj` pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_heappop(list: *mut PyListObj) -> i64 {
    let list = unsafe { &mut *list };
    if list.len == 0 {
        index_error();
        return list.sentinel();
    }
    let kind = list.kind;
    // The list's reference to the returned item moves to the caller.
    edit_words(list, |items| {
        let last = items.pop().expect("non-empty");
        if items.is_empty() {
            return last;
        }
        let first = std::mem::replace(&mut items[0], last);
        sift_up(kind, items, 0);
        first
    })
}

/// `heapq.heapreplace(heap, item)`: pops the smallest, then pushes `item`.
///
/// # Safety
/// `list` must be a live `PyListObj` pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_heapreplace(list: *mut PyListObj, word: i64) -> i64 {
    let list = unsafe { &mut *list };
    if list.len == 0 {
        index_error();
        return list.sentinel();
    }
    list.retain(word);
    let kind = list.kind;
    edit_words(list, |items| {
        let first = std::mem::replace(&mut items[0], word);
        sift_up(kind, items, 0);
        first
    })
}

/// `heapq.heappushpop(heap, item)`: pushes `item`, then pops the smallest.
///
/// # Safety
/// `list` must be a live `PyListObj` pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_heappushpop(list: *mut PyListObj, word: i64) -> i64 {
    let list = unsafe { &mut *list };
    list.retain(word);
    let kind = list.kind;
    edit_words(list, |items| {
        if items.is_empty() || !word_lt(kind, items[0], word) {
            return word;
        }
        let first = std::mem::replace(&mut items[0], word);
        sift_up(kind, items, 0);
        first
    })
}

/// `heapq.heapify(x)`.
///
/// # Safety
/// `list` must be a live `PyListObj` pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_heapify(list: *mut PyListObj) {
    let list = unsafe { &mut *list };
    let kind = list.kind;
    edit_words(list, |items| {
        for i in (0..items.len() / 2).rev() {
            sift_up(kind, items, i);
        }
    });
}

/// `bisect_left`/`bisect_right` over the whole list.
fn bisect(list: &PyListObj, word: i64, right: bool) -> usize {
    let items = list.items();
    let (mut lo, mut hi) = (0usize, items.len());
    while lo < hi {
        let mid = (lo + hi) / 2;
        let go_right = if right {
            !word_lt(list.kind, word, items[mid])
        } else {
            word_lt(list.kind, items[mid], word)
        };
        if go_right {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    lo
}

/// `bisect.bisect_left(a, x)`.
///
/// # Safety
/// `list` must be a live `PyListObj` pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_bisect_left(list: *mut PyListObj, word: i64) -> i64 {
    tag_smallint(bisect(unsafe { &*list }, word, false) as i64)
}

/// `bisect.bisect_right(a, x)` (and `bisect.bisect`).
///
/// # Safety
/// `list` must be a live `PyListObj` pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_bisect_right(list: *mut PyListObj, word: i64) -> i64 {
    tag_smallint(bisect(unsafe { &*list }, word, true) as i64)
}

fn insort(list: *mut PyListObj, word: i64, right: bool) {
    let list = unsafe { &mut *list };
    let at = bisect(list, word, right);
    list.retain(word);
    edit_words(list, |items| items.insert(at, word));
}

/// `bisect.insort_left(a, x)`.
///
/// # Safety
/// `list` must be a live `PyListObj` pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_insort_left(list: *mut PyListObj, word: i64) {
    insort(list, word, false);
}

/// `bisect.insort_right(a, x)` (and `bisect.insort`).
///
/// # Safety
/// `list` must be a live `PyListObj` pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_insort_right(list: *mut PyListObj, word: i64) {
    insort(list, word, true);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ints(values: &[i64]) -> *mut PyListObj {
        let list = pycc_rt_list_new(LIST_KIND_INT, 0);
        for v in values {
            unsafe { pycc_rt_list_append(list, tag_smallint(*v)) };
        }
        list
    }

    fn values(list: *mut PyListObj) -> Vec<i64> {
        unsafe { &*list }
            .items()
            .iter()
            .map(|w| crate::int_encoding::untag_smallint(*w))
            .collect()
    }

    #[test]
    fn heap_operations_follow_heapq() {
        let heap = ints(&[]);
        for v in [5, 1, 4, 1, 3] {
            unsafe { pycc_rt_heappush(heap, tag_smallint(v)) };
        }
        assert_eq!(values(heap), [1, 1, 4, 5, 3]);
        let popped: Vec<i64> = (0..5)
            .map(|_| crate::int_encoding::untag_smallint(unsafe { pycc_rt_heappop(heap) }))
            .collect();
        assert_eq!(popped, [1, 1, 3, 4, 5]);
        let xs = ints(&[9, 8, 7, 6, 5, 4]);
        unsafe { pycc_rt_heapify(xs) };
        assert_eq!(values(xs), [4, 5, 7, 6, 8, 9]);
    }

    #[test]
    fn bisect_and_insort_follow_the_bisect_module() {
        let xs = ints(&[1, 2, 2, 4]);
        let at = |f: unsafe extern "C" fn(*mut PyListObj, i64) -> i64, v| {
            crate::int_encoding::untag_smallint(unsafe { f(xs, tag_smallint(v)) })
        };
        assert_eq!(at(pycc_rt_bisect_left, 2), 1);
        assert_eq!(at(pycc_rt_bisect_right, 2), 3);
        unsafe { pycc_rt_insort_right(xs, tag_smallint(3)) };
        assert_eq!(values(xs), [1, 2, 2, 3, 4]);
    }
}
