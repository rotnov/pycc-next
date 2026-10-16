//! `collections.deque`: a list object with the deque flavor.
//!
//! A deque is a `PyListObj` whose [`PyListObj::flavor`] is [`DEQUE`], so
//! every list operation (indexing, iteration, `len`, `in`, `count`,
//! `index`, `remove`, `reverse`, `clear`, ...) serves it unchanged, inlined
//! by the compiler as for a list. Its own end operations are O(1) amortized:
//! `popleft` advances `data` past the first element (leaving a gap in front,
//! [`PyListObj::head`]), `appendleft` fills that gap or regrows the
//! allocation with room in front, `append` writes into spare room at the
//! back. A `maxlen` deque drops from the opposite end when full, as CPython
//! does. It renders as `deque([...])` (`, maxlen=n` when bounded).

use super::*;

/// [`PyListObj::flavor`] of a `collections.deque`.
pub(crate) const DEQUE: u8 = 1;

fn raise(type_tag: u8, name: &'static str, message: &str) {
    raise_builtin(type_tag, name, message);
}

/// Raises CPython's `AttributeError` for a deque method called on a list.
fn require_deque(list: &PyListObj, method: &str) -> bool {
    if list.flavor == DEQUE {
        return true;
    }
    let owner = if list.kind == crate::value::TUPLE_OBJECT || list.frozen {
        "tuple"
    } else {
        "list"
    };
    raise(
        crate::exception::EXCEPTION_TYPE_ATTRIBUTE_ERROR,
        "AttributeError",
        &format!("'{owner}' object has no attribute '{method}'"),
    );
    false
}

impl PyListObj {
    fn elem_size(&self) -> usize {
        if self.kind == LIST_KIND_BOOL { 1 } else { 8 }
    }

    /// The word of slot `i` (`0 <= i < len`).
    fn read(&self, i: i64) -> i64 {
        self.word_at(i as usize)
    }

    /// Writes slot `i`, which must lie inside the allocation.
    fn write(&mut self, i: i64, word: i64) {
        // SAFETY (caller): slot `i` is inside the allocation.
        unsafe {
            if self.kind == LIST_KIND_BOOL {
                self.data.offset(i as isize).write(u8::from(word != 0));
            } else {
                self.data.cast::<i64>().offset(i as isize).write(word);
            }
        }
    }

    /// Removes and returns the first element (`len > 0`).
    fn take_front(&mut self) -> i64 {
        let word = self.read(0);
        self.len -= 1;
        if self.len == 0 {
            // Back to the start of the allocation.
            // SAFETY: `data` is `head` elements past the allocation's start.
            self.data = unsafe { self.data.sub(self.head as usize * self.elem_size()) };
            self.head = 0;
        } else {
            // SAFETY: the next element is inside the allocation.
            self.data = unsafe { self.data.add(self.elem_size()) };
            self.head += 1;
        }
        word
    }

    /// Removes and returns the last element (`len > 0`).
    fn take_back(&mut self) -> i64 {
        let word = self.read(self.len - 1);
        self.len -= 1;
        word
    }

    /// Makes room for one element in front of `data`.
    fn grow_front<T: Elem>(&mut self) {
        let len = self.len as usize;
        let room = len.max(4);
        let cap = (self.cap as usize).max(len) + room;
        let mut fresh: Vec<T> = Vec::with_capacity(cap);
        let base = fresh.as_mut_ptr();
        // SAFETY: `fresh` holds `room + len` elements; the old elements are
        // moved, then the old allocation (whose start is `head` elements
        // before `data`) is freed as an empty vector.
        unsafe {
            std::ptr::copy_nonoverlapping(self.data.cast::<T>(), base.add(room), len);
            let old = self.data.cast::<T>().sub(self.head as usize);
            drop(Vec::from_raw_parts(old, 0, self.cap as usize));
            self.data = base.add(room).cast::<u8>();
        }
        self.cap = fresh.capacity() as i64;
        std::mem::forget(fresh);
        self.head = room as i64;
    }
}

/// `d.append(word)` on a deque, adopting the reference.
pub(crate) fn append(list: &mut PyListObj, word: i64) {
    if list.maxlen == 0 {
        list.release(word);
        return;
    }
    if list.maxlen > 0 && list.len >= list.maxlen {
        let dropped = list.take_front();
        list.release(dropped);
    }
    if list.head + list.len < list.cap {
        list.write(list.len, word);
        list.len += 1;
        return;
    }
    // Compacted by `with_vec!`; regrown when more than half full, so
    // compacting stays amortized O(1) for a queue of steady length.
    with_vec!(list, |items| {
        if items.len() * 2 > items.capacity() {
            items.reserve(items.len().max(4));
        }
        items.push(Elem::from_word(word));
    });
}

/// Whether a bounded deque is at its `maxlen`, raising CPython's
/// `IndexError` for `d.insert(...)`, which cannot drop an element.
pub(crate) fn is_full(list: &PyListObj) -> bool {
    if list.maxlen >= 0 && list.len >= list.maxlen {
        raise(
            EXCEPTION_TYPE_INDEX_ERROR,
            "IndexError",
            "deque already at its maximum size",
        );
        return true;
    }
    false
}

/// `d.pop()` on a deque: the element's reference to the caller.
pub(crate) fn pop_right(list: &mut PyListObj) -> i64 {
    if list.len == 0 {
        raise(
            EXCEPTION_TYPE_INDEX_ERROR,
            "IndexError",
            "pop from an empty deque",
        );
        return list.sentinel();
    }
    list.take_back()
}

/// `d.appendleft(word)` (`word` borrowed, as every builtin argument is).
///
/// # Safety
/// `list` must be a live `PyListObj` pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_deque_appendleft(list: *mut PyListObj, word: i64) {
    let list = unsafe { &mut *list };
    if !require_deque(list, "appendleft") {
        return;
    }
    list.retain(word);
    appendleft(list, word);
}

/// `d.appendleft(word)` on a deque, adopting the reference.
fn appendleft(list: &mut PyListObj, word: i64) {
    if list.maxlen == 0 {
        list.release(word);
        return;
    }
    if list.maxlen > 0 && list.len >= list.maxlen {
        let dropped = list.take_back();
        list.release(dropped);
    }
    if list.head == 0 {
        if list.kind == LIST_KIND_BOOL {
            list.grow_front::<u8>();
        } else {
            list.grow_front::<i64>();
        }
    }
    // SAFETY: `head > 0` slots are free in front of `data`.
    list.data = unsafe { list.data.sub(list.elem_size()) };
    list.head -= 1;
    list.len += 1;
    list.write(0, word);
}

/// `d.popleft()`: the first element's reference to the caller.
///
/// # Safety
/// `list` must be a live `PyListObj` pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_deque_popleft(list: *mut PyListObj) -> i64 {
    let list = unsafe { &mut *list };
    if !require_deque(list, "popleft") {
        return list.sentinel();
    }
    if list.len == 0 {
        raise(
            EXCEPTION_TYPE_INDEX_ERROR,
            "IndexError",
            "pop from an empty deque",
        );
        return list.sentinel();
    }
    list.take_front()
}

/// `d.extendleft(other)`: each element of `other` appended on the left, in
/// order (so they end up reversed).
///
/// # Safety
/// Both must be live `PyListObj` pointers of one element kind.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_deque_extendleft(list: *mut PyListObj, other: *mut PyListObj) {
    let words = unsafe { &*other }.items().to_vec();
    let list = unsafe { &mut *list };
    if !require_deque(list, "extendleft") {
        return;
    }
    for word in words {
        list.retain(word);
        appendleft(list, word);
    }
}

/// `d.rotate(n)`: rotates right by `n` (left for a negative `n`).
///
/// # Safety
/// `list` must be a live `PyListObj` pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_deque_rotate(list: *mut PyListObj, n: i64) {
    let list = unsafe { &mut *list };
    if !require_deque(list, "rotate") {
        return;
    }
    let Some(n) = crate::int_encoding::inline_int_value(n) else {
        raise(
            crate::exception::EXCEPTION_TYPE_OVERFLOW_ERROR,
            "OverflowError",
            "Python int too large to convert to C ssize_t",
        );
        return;
    };
    let len = list.len;
    if len <= 1 {
        return;
    }
    let k = n.rem_euclid(len) as usize;
    with_vec!(list, |items| items.rotate_right(k));
}

/// `d.rotate()`: rotates right by one.
///
/// # Safety
/// `list` must be a live `PyListObj` pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_deque_rotate1(list: *mut PyListObj) {
    unsafe { pycc_rt_deque_rotate(list, crate::int_encoding::tag_smallint(1)) };
}

/// `deque(items, maxlen)`: marks the fresh list `items` a deque (`has_maxlen
/// == 0`: unbounded) and returns it as a new reference. A bounded deque keeps
/// the last `maxlen` elements.
///
/// # Safety
/// `list` must be a live `PyListObj` pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_deque_new(
    list: *mut PyListObj,
    maxlen: i64,
    has_maxlen: i64,
) -> *mut PyListObj {
    unsafe { crate::rc::pycc_rt_list_incref(list) };
    let list_ref = unsafe { &mut *list };
    list_ref.flavor = DEQUE;
    if crate::int_encoding::inline_int_value(has_maxlen) != Some(0) {
        match crate::int_encoding::inline_int_value(maxlen) {
            Some(m) if m >= 0 => {
                list_ref.maxlen = m;
                while list_ref.len > m {
                    let dropped = list_ref.take_front();
                    list_ref.release(dropped);
                }
            }
            Some(_) => raise(
                EXCEPTION_TYPE_VALUE_ERROR,
                "ValueError",
                "maxlen must be non-negative",
            ),
            None => raise(
                crate::exception::EXCEPTION_TYPE_OVERFLOW_ERROR,
                "OverflowError",
                "Python int too large to convert to C ssize_t",
            ),
        }
    }
    list
}

/// `d.maxlen`: the bound, or `-1` for none (the compiler maps it to
/// `None`).
///
/// # Safety
/// `list` must be a live `PyListObj` pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_deque_maxlen(list: *mut PyListObj) -> i64 {
    let list = unsafe { &*list };
    if !require_deque(list, "maxlen") {
        return -1;
    }
    list.maxlen
}

/// `xs.copy()`: a shallow copy of the same flavor (a deque's copy is a
/// deque with its `maxlen`).
///
/// # Safety
/// `list` must be a live `PyListObj` pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_list_copy_method(list: *mut PyListObj) -> *mut PyListObj {
    let copy = unsafe { super::ops::pycc_rt_list_copy(list) };
    let source = unsafe { &*list };
    // SAFETY: freshly built.
    unsafe {
        (*copy).flavor = source.flavor;
        (*copy).maxlen = source.maxlen;
    }
    copy
}

/// `result`, a fresh concatenation or repetition of `source`, given
/// `source`'s flavor (a deque's `maxlen` keeps the last elements).
pub(crate) fn flavored_like(source: &PyListObj, result: *mut PyListObj) -> *mut PyListObj {
    if source.flavor == DEQUE {
        // SAFETY: freshly built.
        let result = unsafe { &mut *result };
        result.flavor = DEQUE;
        result.maxlen = source.maxlen;
        if source.maxlen >= 0 {
            while result.len > source.maxlen {
                let dropped = result.take_front();
                result.release(dropped);
            }
        }
    }
    result
}

/// Raises CPython's `TypeError` for `a + b` over a deque and a list (in
/// either order); `false` then.
pub(crate) fn concat_compatible(a: &PyListObj, b: &PyListObj) -> bool {
    if a.flavor == b.flavor {
        return true;
    }
    let message = if a.flavor == DEQUE {
        format!(
            "can only concatenate deque (not \"{}\") to deque",
            type_name(b)
        )
    } else {
        format!(
            "can only concatenate {} (not \"{}\") to {}",
            type_name(a),
            type_name(b),
            type_name(a)
        )
    };
    raise(
        crate::exception::EXCEPTION_TYPE_TYPE_ERROR,
        "TypeError",
        &message,
    );
    false
}

/// The type name CPython's messages give a list object of this flavor.
pub(crate) fn type_name(list: &PyListObj) -> &'static str {
    if list.flavor == DEQUE {
        "collections.deque"
    } else if list.kind == crate::value::TUPLE_OBJECT || list.frozen {
        "tuple"
    } else {
        "list"
    }
}

/// `repr()` of a deque: `deque([1, 2])`, `deque([1], maxlen=3)`.
pub(crate) fn repr(list: &PyListObj, out: &mut String) -> bool {
    out.push_str("deque([");
    for i in 0..list.len {
        if i > 0 {
            out.push_str(", ");
        }
        if !word_repr(list.kind, list.read(i), out) {
            return false;
        }
    }
    out.push(']');
    if list.maxlen >= 0 {
        out.push_str(&format!(", maxlen={}", list.maxlen));
    }
    out.push(')');
    true
}
