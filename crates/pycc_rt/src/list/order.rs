//! `sorted(xs)`, `xs.sort()`, `min(xs)` and `max(xs)` -- with or without
//! `key=`/`reverse=` -- over elements (or keys) that compare through a user
//! method (class instances).
//!
//! The runtime cannot call user code, so compiled code drives the algorithm
//! (`pycc_codegen::instance_order`): [`pycc_rt_order_begin`] snapshots the
//! elements, then each [`pycc_rt_order_step`] either asks for one comparison
//! -- `left < right` for a sort or `min`, `left > right` for `max`, read
//! with [`pycc_rt_order_left`]/[`pycc_rt_order_right`] -- or reports that
//! the result is ready for [`pycc_rt_order_finish`]. A comparison that
//! raises ends the loop with [`pycc_rt_order_abort`].
//!
//! The sort is a stable bottom-up merge sort consulting only `<`, as
//! CPython's does (the number and order of the comparisons differ from
//! CPython's timsort, which only a comparison with side effects can tell);
//! `reverse` is CPython's reverse-sort-reverse, which keeps equal elements
//! in their original order. `min`/`max` keep the first extreme element,
//! comparing each later element against the current one (`item < best`,
//! `item > best`).
//!
//! A keyed ordering (`pycc_ast::desugar::keys`) is given the list of
//! `(key, element)` pairs: it compares the keys and yields the elements.
//!
//! `sorted`/`min`/`max` take a reference to every element (to the pairs
//! list, for a keyed one), so a comparison that changes the list cannot
//! free one. `xs.sort()` does what CPython's `list.sort` does: the list's
//! storage is detached when the sort starts (the list reads as empty to the
//! comparisons, and the snapshot owns its references), and moved back at
//! the end; a list that is no longer empty then was modified during the
//! sort, so what it holds is released and CPython's `ValueError: list
//! modified during sort` raised. A comparison that raises leaves the list
//! holding its elements in the order of the last completed merge pass (the
//! original order when the first comparison raises). The list itself is
//! kept alive across the comparisons.

use super::*;

/// The `mode` flag of a keyed ordering (the list holds `(key, element)`
/// pairs).
const KEYED: i64 = 4;
/// The `mode` flag of `reverse=True`.
const REVERSE: i64 = 8;

/// What the ordering computes.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// `sorted(xs)`: a new list.
    Sorted,
    /// `xs.sort()`: `xs` reordered.
    Sort,
    Min,
    Max,
}

/// One ordering in progress.
pub struct OrderState {
    mode: Mode,
    reverse: bool,
    /// The words compared (the keys of a keyed ordering).
    keys: Vec<i64>,
    /// The elements yielded, parallel to `keys` (the keys themselves when
    /// not keyed).
    elems: Vec<i64>,
    /// The element kind (of the elements yielded).
    kind: i64,
    /// The pairs list a keyed ordering keeps alive, or null.
    pairs: *mut PyListObj,
    /// The list sorted in place (`sort`), its storage detached, or null.
    target: *mut PyListObj,
    /// The detached words of `target`, whose references the snapshot owns.
    detached: Vec<i64>,
    /// The positions in order so far, and the merge's destination.
    perm: Vec<usize>,
    buf: Vec<usize>,
    width: usize,
    /// The current merge: `perm[a..mid]` with `perm[b..hi]` into `buf[k..]`.
    a: usize,
    mid: usize,
    b: usize,
    hi: usize,
    k: usize,
    /// The extreme element's position (`min`/`max`), and the next one.
    best: usize,
    next: usize,
    /// Whether the last step asked for a comparison.
    pending: bool,
    done: bool,
}

impl OrderState {
    fn setup_merge(&mut self, start: usize) {
        let n = self.perm.len();
        self.a = start;
        self.k = start;
        self.mid = (start + self.width).min(n);
        self.b = self.mid;
        self.hi = (start + 2 * self.width).min(n);
    }

    /// Advances the merge sort past `less` (the answer to the pending
    /// comparison `key[perm[b]] < key[perm[a]]`), stopping at the next
    /// comparison or the end.
    fn sort_step(&mut self, less: bool) -> bool {
        if self.pending {
            self.pending = false;
            if less {
                self.buf[self.k] = self.perm[self.b];
                self.b += 1;
            } else {
                self.buf[self.k] = self.perm[self.a];
                self.a += 1;
            }
            self.k += 1;
        }
        let n = self.perm.len();
        loop {
            if self.done {
                return false;
            }
            if self.a < self.mid && self.b < self.hi {
                self.pending = true;
                return true;
            }
            // One run is exhausted: copy the rest of the other.
            while self.a < self.mid {
                self.buf[self.k] = self.perm[self.a];
                self.a += 1;
                self.k += 1;
            }
            while self.b < self.hi {
                self.buf[self.k] = self.perm[self.b];
                self.b += 1;
                self.k += 1;
            }
            let start = self.hi;
            if start < n {
                self.setup_merge(start);
                continue;
            }
            std::mem::swap(&mut self.perm, &mut self.buf);
            self.width *= 2;
            if self.width >= n {
                self.done = true;
                return false;
            }
            self.setup_merge(0);
        }
    }

    /// Advances `min`/`max` past `better` (whether element `next` beats
    /// element `best`).
    fn extreme_step(&mut self, better: bool) -> bool {
        if self.pending {
            self.pending = false;
            if better {
                self.best = self.next;
            }
            self.next += 1;
        }
        if self.next < self.keys.len() {
            self.pending = true;
            true
        } else {
            self.done = true;
            false
        }
    }

    /// The words the pending comparison reads: `(left, right)`.
    fn operands(&self) -> (i64, i64) {
        let (l, r) = match self.mode {
            Mode::Sorted | Mode::Sort => (self.perm[self.b], self.perm[self.a]),
            Mode::Min | Mode::Max => (self.next, self.best),
        };
        (self.keys[l], self.keys[r])
    }

    /// The elements in order.
    fn ordered(&self) -> Vec<i64> {
        let mut out: Vec<i64> = self.perm.iter().map(|i| self.elems[*i]).collect();
        if self.reverse {
            out.reverse();
        }
        out
    }

    /// Drops the snapshot's references (for `sort`, the target's elements
    /// are handed back by [`Self::restore`] instead).
    fn release(&self) {
        if self.pairs.is_null() {
            if self.target.is_null() {
                for word in &self.elems {
                    crate::value::release(self.kind, *word);
                }
            }
        } else {
            unsafe { crate::rc::pycc_rt_list_decref(self.pairs) };
        }
    }

    /// Moves `words` (the detached elements, reordered) back into the
    /// sorted list, releasing whatever a comparison stored there meanwhile;
    /// `true` when the list had been modified. Drops the list's reference.
    fn restore(&self, words: Vec<i64>) -> bool {
        let target = unsafe { &mut *self.target };
        let modified = target.len() != 0;
        if modified {
            let stored = target.items().into_owned();
            super::ops::edit_words(target, Vec::clear);
            for word in stored {
                target.release(word);
            }
        }
        super::ops::edit_words(target, |items| *items = words);
        unsafe { crate::rc::pycc_rt_list_decref(self.target) };
        modified
    }

    /// The detached elements in `perm` order (the keyed elements are the
    /// same objects as the detached ones, which own the references).
    fn reordered(&self) -> Vec<i64> {
        let ordered = self.ordered();
        debug_assert_eq!(ordered.len(), self.detached.len());
        ordered
    }
}

/// Starts ordering `list` (`mode`: 0 `sorted`, 1 `sort`, 2 `min`, 3
/// `max`, plus [`KEYED`] when `list` holds `(key, element)` pairs and
/// [`REVERSE`]); `kind` is the kind of the elements yielded; `target` is
/// the list a `sort` reorders (`list` itself unless keyed), null otherwise.
/// For `min`/`max` of an empty list, raises CPython's `ValueError` and
/// returns null.
///
/// # Safety
/// `list` must be a live list (of 2-tuples when keyed), `target` a live
/// list or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_order_begin(
    list: *mut PyListObj,
    mode: i64,
    kind: i64,
    target: *mut PyListObj,
) -> *mut OrderState {
    let keyed = mode & KEYED != 0;
    let reverse = mode & REVERSE != 0;
    let mode = match mode & 3 {
        0 => Mode::Sorted,
        1 => Mode::Sort,
        2 => Mode::Min,
        _ => Mode::Max,
    };
    let source = unsafe { &*list };
    if source.len() == 0 && matches!(mode, Mode::Min | Mode::Max) {
        let name = if mode == Mode::Max { "max" } else { "min" };
        crate::exception::raise_builtin(
            crate::exception::EXCEPTION_TYPE_VALUE_ERROR,
            "ValueError",
            &format!("{name}() iterable argument is empty"),
        );
        return std::ptr::null_mut();
    }
    // `xs.sort()`: the list is kept alive and its storage detached.
    let target = if mode == Mode::Sort { target } else { std::ptr::null_mut() };
    let detached = if target.is_null() {
        Vec::new()
    } else {
        unsafe { crate::rc::pycc_rt_list_incref(target) };
        let list = unsafe { &mut *target };
        let words = list.items().into_owned();
        super::ops::edit_words(list, Vec::clear);
        words
    };
    let (keys, elems, pairs) = if keyed {
        let mut keys = Vec::new();
        let mut elems = Vec::new();
        for (_, tuple) in source.kinded_items() {
            let items = unsafe { &*(tuple as *mut PyListObj) }.kinded_items();
            keys.push(items[0].1);
            elems.push(items[1].1);
        }
        unsafe { crate::rc::pycc_rt_list_incref(list) };
        (keys, elems, list)
    } else if !target.is_null() {
        (detached.clone(), detached.clone(), std::ptr::null_mut())
    } else {
        let items = source.items().into_owned();
        for word in &items {
            source.retain(*word);
        }
        (items.clone(), items, std::ptr::null_mut())
    };
    let n = keys.len();
    let mut perm: Vec<usize> = (0..n).collect();
    if reverse {
        perm.reverse();
    }
    let mut state = OrderState {
        mode,
        reverse,
        keys,
        elems,
        kind,
        pairs,
        target,
        detached,
        buf: perm.clone(),
        perm,
        width: 1,
        a: 0,
        mid: 0,
        b: 0,
        hi: 0,
        k: 0,
        best: 0,
        next: 1,
        pending: false,
        done: false,
    };
    if matches!(mode, Mode::Sorted | Mode::Sort) {
        if n < 2 {
            state.done = true;
        } else {
            state.setup_merge(0);
        }
    }
    Box::into_raw(Box::new(state))
}

/// Takes the answer to the pending comparison (`less`, ignored before the
/// first one) and returns `1` when another comparison is needed, `0` when
/// the result is ready.
///
/// # Safety
/// `state` must come from [`pycc_rt_order_begin`] and not be finished.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_order_step(state: *mut OrderState, less: i8) -> i8 {
    let state = unsafe { &mut *state };
    let more = match state.mode {
        Mode::Sorted | Mode::Sort => state.sort_step(less != 0),
        Mode::Min | Mode::Max => state.extreme_step(less != 0),
    };
    i8::from(more)
}

/// The left operand of the pending comparison (a borrowed word).
///
/// # Safety
/// `state` must have a pending comparison.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_order_left(state: *mut OrderState) -> i64 {
    unsafe { &*state }.operands().0
}

/// The right operand of the pending comparison (a borrowed word).
///
/// # Safety
/// `state` must have a pending comparison.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_order_right(state: *mut OrderState) -> i64 {
    unsafe { &*state }.operands().1
}

/// Ends the ordering and frees `state`: the new list (`sorted`), `0` after
/// reordering the list in place (`sort`; `ValueError` when it was modified
/// meanwhile), or the extreme element, a new reference (`min`, `max`).
///
/// # Safety
/// `state` must come from [`pycc_rt_order_begin`] and not be finished.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_order_finish(state: *mut OrderState) -> i64 {
    let state = unsafe { Box::from_raw(state) };
    match state.mode {
        Mode::Sorted => {
            let ordered = state.ordered();
            if !state.pairs.is_null() {
                for word in &ordered {
                    crate::value::retain(state.kind, *word);
                }
                state.release();
            }
            // The snapshot's references move into the new list.
            new_list(state.kind, ordered) as i64
        }
        Mode::Sort => {
            let modified = state.restore(state.reordered());
            state.release();
            if modified {
                crate::exception::raise_builtin(
                    crate::exception::EXCEPTION_TYPE_VALUE_ERROR,
                    "ValueError",
                    "list modified during sort",
                );
            }
            0
        }
        Mode::Min | Mode::Max => {
            let best = state.elems[state.best];
            crate::value::retain(state.kind, best);
            state.release();
            best
        }
    }
}

/// Frees `state` after a comparison raised.
///
/// # Safety
/// `state` must come from [`pycc_rt_order_begin`] and not be finished.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_order_abort(state: *mut OrderState) {
    let state = unsafe { Box::from_raw(state) };
    if !state.target.is_null() {
        // The order of the last completed merge pass.
        let mut words: Vec<i64> = state.perm.iter().map(|i| state.elems[*i]).collect();
        if state.reverse {
            words.reverse();
        }
        state.restore(words);
    }
    state.release();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Drives an ordering of `words` (uncounted float-kind words, compared
    /// as integers) and returns the result and the comparison count.
    fn drive(words: &[i64], mode: i64) -> (Vec<i64>, usize) {
        let list = new_list(LIST_KIND_FLOAT, words.to_vec());
        let mut compares = 0;
        unsafe {
            let target = if mode & 3 == 1 { list } else { std::ptr::null_mut() };
            let state = pycc_rt_order_begin(list, mode, LIST_KIND_FLOAT, target);
            let mut less = 0;
            while pycc_rt_order_step(state, less) == 1 {
                compares += 1;
                let (l, r) = (pycc_rt_order_left(state), pycc_rt_order_right(state));
                less = i8::from(if mode & 3 == 3 { l > r } else { l < r });
            }
            let result = pycc_rt_order_finish(state);
            let out = match mode & 3 {
                0 => crate::list::list_words(result as *mut PyListObj),
                1 => crate::list::list_words(list),
                _ => vec![result],
            };
            (out, compares)
        }
    }

    #[test]
    fn sorts_stably_in_n_log_n_comparisons() {
        let words: Vec<i64> = (0..100).map(|i| (i * 37) % 101).collect();
        let mut expected = words.clone();
        expected.sort_unstable();
        let (sorted, compares) = drive(&words, 0);
        assert_eq!(sorted, expected);
        assert!(compares <= 100 * 7, "{compares}");
        let (in_place, _) = drive(&words, 1);
        assert_eq!(in_place, expected);
        expected.reverse();
        assert_eq!(drive(&words, REVERSE).0, expected);
        assert_eq!(drive(&[], 0).0, Vec::<i64>::new());
        assert_eq!(drive(&[5], 1).0, vec![5]);
    }

    #[test]
    fn min_and_max_keep_the_first_extreme() {
        assert_eq!(drive(&[3, 1, 4, 1, 5], 2).0, vec![1]);
        assert_eq!(drive(&[3, 1, 4, 1, 5, 2], 3).0, vec![5]);
        assert_eq!(drive(&[7], 3), (vec![7], 0));
    }
}
