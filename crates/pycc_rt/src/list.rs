//! `list[T]`'s one runtime object, `PyListObj`, for every element type the
//! compiler admits (`int`, `float`, `bool`, `str`, a user-class instance and
//! a nested `list`).
//!
//! **Storage.** Each element occupies one 8-byte word holding the element's
//! own native representation, the same word an instance slot holds
//! (`instance.rs`): an `int` is its D-141 encoded word (bool markers kept), a
//! `float` its `f64` bit pattern, a `bool` `0`/`1`, and a `str`, instance or
//! nested list its pointer. The element kind (`LIST_KIND_*`) is recorded in
//! the object, so the generic operations here -- equality, `repr`, release
//! -- know how to treat a word without the compiler passing a type.
//!
//! **Layout.** `#[repr(C)]` with `len` at field 0 and `data` at field 1:
//! `pycc_codegen` inlines the hot paths (`len(xs)`, `xs[i]`, `xs[i] = v`,
//! `for x in xs`) as loads from those two fields, so the layout is an ABI
//! contract pinned by `list_layout_matches_the_codegen_contract`.
//!
//! **Ownership.** A list owns one reference to each `str` element: the
//! storing operations (`append`, `insert`, `set`) adopt the reference the
//! caller passes, `set` releases the element it overwrites, `pop` hands its
//! element's reference back, and the copying operations (`+`, `*`, slicing)
//! take a new reference per copy. Reads (`xs[i]`, iteration) are borrowed.
//! Instances and lists are never freed (D-107, D-154), so their words carry
//! no reference. A list itself is never freed either.

use super::*;
use crate::instance::PyInstanceObj;

/// Element kind of a `list[int]` (D-141 encoded words).
pub const LIST_KIND_INT: i64 = 0;
/// Element kind of a `list[float]` (`f64` bit patterns).
pub const LIST_KIND_FLOAT: i64 = 1;
/// Element kind of a `list[bool]` (`0`/`1`).
pub const LIST_KIND_BOOL: i64 = 2;
/// Element kind of a `list[str]` (owned `PyStrObj` pointers).
pub const LIST_KIND_STR: i64 = 3;
/// Element kind of a list of user-class instances (`PyInstanceObj` pointers).
pub const LIST_KIND_INSTANCE: i64 = 4;
/// Element kind of a list of lists (`PyListObj` pointers).
pub const LIST_KIND_LIST: i64 = 5;

/// See the module doc comment for the layout and ownership contract.
#[repr(C)]
pub struct PyListObj {
    len: i64,
    data: *mut i64,
    cap: i64,
    kind: i64,
}

impl PyListObj {
    fn items(&self) -> &[i64] {
        if self.len == 0 {
            return &[];
        }
        unsafe { std::slice::from_raw_parts(self.data, self.len as usize) }
    }

    /// Runs `f` on the backing storage as a `Vec`, writing its (possibly
    /// reallocated) parts back afterwards.
    fn with_vec<R>(&mut self, f: impl FnOnce(&mut Vec<i64>) -> R) -> R {
        let mut vec =
            unsafe { Vec::from_raw_parts(self.data, self.len as usize, self.cap as usize) };
        let result = f(&mut vec);
        let mut vec = std::mem::ManuallyDrop::new(vec);
        self.data = vec.as_mut_ptr();
        self.len = vec.len() as i64;
        self.cap = vec.capacity() as i64;
        result
    }

    fn retain(&self, word: i64) {
        if self.kind == LIST_KIND_STR {
            unsafe { pycc_rt_str_incref(word as *mut PyStrObj) };
        }
    }

    fn release(&self, word: i64) {
        if self.kind == LIST_KIND_STR {
            unsafe { pycc_rt_str_decref(word as *mut PyStrObj) };
        }
    }

    /// The type-valid word returned beside a raised exception.
    fn sentinel(&self) -> i64 {
        match self.kind {
            LIST_KIND_INT => tag_smallint(0),
            LIST_KIND_STR => new_pystr(b"") as i64,
            _ => 0,
        }
    }
}

fn new_list(kind: i64, items: Vec<i64>) -> *mut PyListObj {
    let mut items = std::mem::ManuallyDrop::new(items);
    Box::into_raw(Box::new(PyListObj {
        len: items.len() as i64,
        data: items.as_mut_ptr(),
        cap: items.capacity() as i64,
        kind,
    }))
}

/// A fresh list of element kind `kind` with room for `capacity` elements.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_list_new(kind: i64, capacity: i64) -> *mut PyListObj {
    new_list(kind, Vec::with_capacity(capacity.max(0) as usize))
}

/// `xs.append(word)`, adopting the caller's reference to a `str` word.
///
/// # Safety
/// `list` must be a live `PyListObj` pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_list_append(list: *mut PyListObj, word: i64) {
    unsafe { &mut *list }.with_vec(|items| items.push(word));
}

/// Resolves a Python index (negative counts from the end) against `len`.
fn resolve_index(index: i64, len: i64) -> Option<usize> {
    let index = if index < 0 { index + len } else { index };
    (0..len).contains(&index).then_some(index as usize)
}

/// `xs[index]`, borrowed. Raises CPython's `IndexError` on an index outside
/// `[-len, len)` and returns a type-valid sentinel. Generated code inlines
/// the in-range path and calls [`pycc_rt_list_index_error`] itself; this
/// out-of-line form serves the runtime's own callers and tests.
///
/// # Safety
/// `list` must be a live `PyListObj` pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_list_get(list: *mut PyListObj, index: i64) -> i64 {
    let list = unsafe { &*list };
    match resolve_index(index, list.len) {
        Some(i) => list.items()[i],
        None => {
            pycc_rt_list_index_error(0);
            list.sentinel()
        }
    }
}

/// Raises the `IndexError` of an out-of-range list index: `which == 0` is a
/// read (`list index out of range`), anything else a store (`list assignment
/// index out of range`). Generated code calls it on the cold path of an
/// inlined bounds check.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_list_index_error(which: i64) {
    let message = if which == 0 {
        "list index out of range"
    } else {
        "list assignment index out of range"
    };
    raise_builtin(EXCEPTION_TYPE_INDEX_ERROR, "IndexError", message);
}

/// `xs[index] = word`, adopting the caller's reference and releasing the
/// overwritten element. On an out-of-range index raises `IndexError` and
/// releases `word` instead.
///
/// # Safety
/// `list` must be a live `PyListObj` pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_list_set(list: *mut PyListObj, index: i64, word: i64) {
    let list = unsafe { &mut *list };
    match resolve_index(index, list.len) {
        Some(i) => {
            let slot = unsafe { &mut *list.data.add(i) };
            let old = std::mem::replace(slot, word);
            list.release(old);
        }
        None => {
            list.release(word);
            pycc_rt_list_index_error(1);
        }
    }
}

/// `len(xs)` as a raw count.
///
/// # Safety
/// `list` must be a live `PyListObj` pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_list_len(list: *mut PyListObj) -> i64 {
    unsafe { &*list }.len
}

/// `xs.pop()` (`has_index == 0`) or `xs.pop(index)`, handing the element's
/// reference to the caller. Raises CPython's `IndexError` for an empty list
/// or an out-of-range index and returns a type-valid sentinel.
///
/// # Safety
/// `list` must be a live `PyListObj` pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_list_pop(list: *mut PyListObj, has_index: i64, index: i64) -> i64 {
    let list = unsafe { &mut *list };
    if list.len == 0 {
        raise_builtin(
            EXCEPTION_TYPE_INDEX_ERROR,
            "IndexError",
            "pop from empty list",
        );
        return list.sentinel();
    }
    let index = if has_index == 0 { -1 } else { index };
    match resolve_index(index, list.len) {
        Some(i) => list.with_vec(|items| items.remove(i)),
        None => {
            raise_builtin(
                EXCEPTION_TYPE_INDEX_ERROR,
                "IndexError",
                "pop index out of range",
            );
            list.sentinel()
        }
    }
}

/// `xs.insert(index, word)` with CPython's clamping: a negative index counts
/// from the end and every out-of-range index inserts at the nearer end.
/// Adopts the caller's reference.
///
/// # Safety
/// `list` must be a live `PyListObj` pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_list_insert(list: *mut PyListObj, index: i64, word: i64) {
    let list = unsafe { &mut *list };
    let len = list.len;
    let at = if index < 0 {
        (index + len).max(0)
    } else {
        index.min(len)
    };
    list.with_vec(|items| items.insert(at as usize, word));
}

/// Bit of `pycc_rt_list_slice`'s `present` mask: `start` was written.
pub const SLICE_HAS_START: i64 = 1;
/// Bit of `pycc_rt_list_slice`'s `present` mask: `stop` was written.
pub const SLICE_HAS_STOP: i64 = 2;
/// Bit of `pycc_rt_list_slice`'s `present` mask: `step` was written.
pub const SLICE_HAS_STEP: i64 = 4;

/// CPython's `PySlice_AdjustIndices`: the start and element count a slice
/// with these (possibly omitted) bounds selects from a sequence of `len`.
fn adjust_slice(len: i64, start: Option<i64>, stop: Option<i64>, step: i64) -> (i64, i64) {
    let clamp = |bound: i64, low: i64, high: i64| {
        if bound < 0 {
            (bound + len).max(low)
        } else {
            bound.min(high)
        }
    };
    let (start, stop) = if step > 0 {
        (
            start.map_or(0, |s| clamp(s, 0, len)),
            stop.map_or(len, |s| clamp(s, 0, len)),
        )
    } else {
        (
            start.map_or(len - 1, |s| clamp(s, -1, len - 1)),
            stop.map_or(-1, |s| clamp(s, -1, len - 1)),
        )
    };
    let count = if step > 0 {
        if stop > start {
            (stop - start - 1) / step + 1
        } else {
            0
        }
    } else if start > stop {
        (start - stop - 1) / (-step) + 1
    } else {
        0
    };
    (start, count)
}

/// `xs[start:stop:step]` as a new list, CPython's semantics exactly
/// (negative bounds and steps included). `present` says which bounds were
/// written (`SLICE_HAS_*`). A zero step raises `ValueError` and returns an
/// empty list.
///
/// # Safety
/// `list` must be a live `PyListObj` pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_list_slice(
    list: *mut PyListObj,
    start: i64,
    stop: i64,
    step: i64,
    present: i64,
) -> *mut PyListObj {
    let list = unsafe { &*list };
    let step = if present & SLICE_HAS_STEP != 0 {
        step
    } else {
        1
    };
    if step == 0 {
        raise_builtin(
            EXCEPTION_TYPE_VALUE_ERROR,
            "ValueError",
            "slice step cannot be zero",
        );
        return new_list(list.kind, Vec::new());
    }
    let start = (present & SLICE_HAS_START != 0).then_some(start);
    let stop = (present & SLICE_HAS_STOP != 0).then_some(stop);
    let (first, count) = adjust_slice(list.len, start, stop, step);
    let items = list.items();
    let picked: Vec<i64> = (0..count)
        .map(|k| items[(first + k * step) as usize])
        .collect();
    for word in &picked {
        list.retain(*word);
    }
    new_list(list.kind, picked)
}

/// `a + b` as a new list.
///
/// # Safety
/// `a` and `b` must be live `PyListObj` pointers of the same kind.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_list_concat(
    a: *mut PyListObj,
    b: *mut PyListObj,
) -> *mut PyListObj {
    let (a, b) = unsafe { (&*a, &*b) };
    let mut items = Vec::with_capacity(a.items().len() + b.items().len());
    items.extend_from_slice(a.items());
    items.extend_from_slice(b.items());
    for word in &items {
        a.retain(*word);
    }
    new_list(a.kind, items)
}

/// `xs * count` (and `count * xs`) as a new list; a non-positive count gives
/// an empty list, as in CPython. Elements are shared, not copied, so a list
/// of lists repeats the same inner object, exactly as CPython does.
///
/// # Safety
/// `list` must be a live `PyListObj` pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_list_repeat(list: *mut PyListObj, count: i64) -> *mut PyListObj {
    let list = unsafe { &*list };
    let items = if count <= 0 {
        Vec::new()
    } else {
        list.items().repeat(count as usize)
    };
    for word in &items {
        list.retain(*word);
    }
    new_list(list.kind, items)
}

/// Python `==` between two element words of `kind`.
fn words_equal(kind: i64, a: i64, b: i64) -> bool {
    match kind {
        LIST_KIND_INT => encoded_int_cmp(a, b) == std::cmp::Ordering::Equal,
        LIST_KIND_FLOAT => f64::from_bits(a as u64) == f64::from_bits(b as u64),
        LIST_KIND_STR => unsafe { pycc_rt_str_cmp(a as *mut PyStrObj, b as *mut PyStrObj) == 0 },
        LIST_KIND_LIST => a == b || lists_equal(a as *mut PyListObj, b as *mut PyListObj),
        // `bool` values and instances (identity: `pycc_types` admits `in`
        // only on a class without `__eq__`).
        _ => a == b,
    }
}

fn lists_equal(a: *mut PyListObj, b: *mut PyListObj) -> bool {
    let (a, b) = unsafe { (&*a, &*b) };
    a.len == b.len
        && a.items()
            .iter()
            .zip(b.items())
            .all(|(x, y)| words_equal(a.kind, *x, *y))
}

/// `word in xs`: `1` when an element equals `word`, else `0`. `word` is
/// borrowed.
///
/// # Safety
/// `list` must be a live `PyListObj` pointer and `word` a valid word of its
/// element kind.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_list_contains(list: *mut PyListObj, word: i64) -> i8 {
    let list = unsafe { &*list };
    i8::from(
        list.items()
            .iter()
            .any(|item| words_equal(list.kind, *item, word)),
    )
}

/// Python's `repr()` of a `str`: single quotes unless the text contains a
/// single quote and no double quote, with CPython's escapes.
fn str_repr(text: &str, out: &mut String) {
    let quote = if text.contains('\'') && !text.contains('"') {
        '"'
    } else {
        '\''
    };
    out.push(quote);
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            c if (c as u32) < 0x20
                || c as u32 == 0x7f
                || (0x80..0xa0).contains(&(c as u32))
                || c as u32 == 0xad =>
            {
                out.push_str(&format!("\\x{:02x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push(quote);
}

/// Appends `repr()` of one element word to `out`; `false` when rendering
/// raised (a `float` needing scientific notation).
fn word_repr(kind: i64, word: i64, out: &mut String) -> bool {
    let rendered = match kind {
        LIST_KIND_INT => int_to_str(word),
        LIST_KIND_FLOAT => pycc_rt_float_to_str(f64::from_bits(word as u64)),
        LIST_KIND_BOOL => pycc_rt_bool_to_str(word as i8),
        LIST_KIND_STR => {
            let text = String::from_utf8_lossy(unsafe { &*(word as *mut PyStrObj) }.bytes());
            str_repr(&text, out);
            return true;
        }
        LIST_KIND_LIST => return list_repr(unsafe { &*(word as *mut PyListObj) }, out),
        _ => {
            let instance = word as *mut PyInstanceObj;
            let mut len = 0usize;
            let name = unsafe { pycc_rt_ext_instance_class(instance, &mut len) };
            let name = unsafe { std::slice::from_raw_parts(name, len) };
            out.push_str(&format!(
                "<__main__.{} object at {:#x}>",
                String::from_utf8_lossy(name),
                word
            ));
            return true;
        }
    };
    out.push_str(&String::from_utf8_lossy(unsafe { &*rendered }.bytes()));
    unsafe { pycc_rt_str_decref(rendered) };
    pycc_rt_exception_active() == 0
}

fn list_repr(list: &PyListObj, out: &mut String) -> bool {
    out.push('[');
    for (i, word) in list.items().iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        if !word_repr(list.kind, *word, out) {
            return false;
        }
    }
    out.push(']');
    true
}

/// `str(xs)` / `repr(xs)` as a new `str`. A `float` element needing
/// scientific notation leaves `float_to_str`'s `RuntimeError` pending and
/// yields an empty string.
///
/// # Safety
/// `list` must be a live `PyListObj` pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_list_repr(list: *mut PyListObj) -> *mut PyStrObj {
    let mut out = String::new();
    if !list_repr(unsafe { &*list }, &mut out) {
        return new_pystr(b"");
    }
    new_pystr(out.as_bytes())
}

/// The words of a `list[int]`, for `frozenset(xs)`.
pub(crate) fn list_words(list: *mut PyListObj) -> Vec<i64> {
    unsafe { &*list }.items().to_vec()
}

#[cfg(test)]
mod tests;
