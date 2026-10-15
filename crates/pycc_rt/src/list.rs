//! `list[T]`'s one runtime object, `PyListObj`, for every element type the
//! compiler admits (`int`, `float`, `bool`, `str`, a user-class instance and
//! a nested `list`).
//!
//! **Storage.** Every element crosses the API as one 8-byte word holding the
//! element's own native representation, the same word an instance slot holds
//! (`instance.rs`): an `int` is its D-141 encoded word (bool markers kept), a
//! `float` its `f64` bit pattern, a `bool` `0`/`1`, and a `str`, instance or
//! nested list its pointer. In memory each element is that word, except in a
//! `list[bool]`, which stores one byte (`0`/`1`) per element -- an eighth of
//! the memory traffic for a sieve-style flag array. The element kind
//! (`LIST_KIND_*`) is recorded in the object, so the generic operations here
//! -- equality, `repr`, release -- know how to treat a word without the
//! compiler passing a type.
//!
//! **Layout.** `#[repr(C)]` with `len` at field 0 and `data` at field 1:
//! `pycc_codegen` inlines the hot paths (`len(xs)`, `xs[i]`, `xs[i] = v`,
//! `for x in xs`) as loads from those two fields, indexing `data` by 8-byte
//! words or, for `list[bool]`, by bytes, so the layout is an ABI contract
//! pinned by `list_layout_matches_the_codegen_contract`.
//!
//! **Ownership.** A list owns one reference to each `str`, list or instance
//! element: the storing operations (`append`, `insert`, `set`) adopt the
//! reference the caller passes, `set` releases the element it overwrites,
//! `pop` hands its element's reference back, and the copying operations
//! (`+`, `*`, slicing) take a new reference per copy. Reads (`xs[i]`,
//! iteration) are borrowed. A list is itself reference counted (`crate::rc`):
//! the count sits at `rc::RC_OFFSET`, and freeing the list releases every
//! element it owns.

use super::*;
use crate::rc::{LIST_LIVE, Worklist, release_word_into, retain_word};

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
/// Element kind of a list of exception-class instances (counted
/// `PyExceptionObj` pointers, `exception/lifetime.rs`).
pub const LIST_KIND_EXCEPTION: i64 = 6;
/// Element kind of a list of tuples (tuple-object `PyListObj` pointers,
/// `tuple.rs`).
pub const LIST_KIND_TUPLE: i64 = 7;
/// Element kind of a list of dicts (`PyDictObj` pointers, `dict.rs`).
pub const LIST_KIND_DICT: i64 = 8;
/// Element kind of a list of sets (`PySetObj` pointers, `set.rs`).
pub const LIST_KIND_SET: i64 = 9;
/// Element kind of a list of function values (`PyClosureObj` pointers,
/// `closure.rs`).
pub const LIST_KIND_CLOSURE: i64 = 10;

/// See the module doc comment for the layout and ownership contract.
#[repr(C)]
pub struct PyListObj {
    len: i64,
    /// `len` elements of `i64`, or of `u8` for `LIST_KIND_BOOL`.
    data: *mut u8,
    cap: i64,
    kind: i64,
    /// The reference count (`crate::rc`).
    rc: i64,
    /// A tuple object's per-element kinds (`len` bytes); null for a list,
    /// whose elements all have `kind`.
    pub(crate) kinds: *mut u8,
    /// A homogeneous tuple held as a list of `kind` (a function's `*args`,
    /// [`pycc_rt_list_mark_tuple`]): it renders as a tuple, and a slice or
    /// repeat of it is one too.
    pub(crate) frozen: bool,
    /// [`deque::DEQUE`] for a `collections.deque` (`list/deque.rs`).
    pub(crate) flavor: u8,
    /// A deque's `maxlen`, or `-1` for none.
    pub(crate) maxlen: i64,
    /// Unused element slots in front of `data` (a deque that popped from
    /// the left): the allocation starts `head` elements before `data`, and
    /// `cap` counts from there. Every operation other than the deque's own
    /// end operations first moves the elements back to the start.
    pub(crate) head: i64,
}

const _: () = assert!(std::mem::offset_of!(PyListObj, rc) == crate::rc::RC_OFFSET);

/// One stored element: an 8-byte word, or a `list[bool]`'s byte.
trait Elem: Copy {
    fn from_word(word: i64) -> Self;
    fn word(self) -> i64;
}

impl Elem for i64 {
    fn from_word(word: i64) -> Self {
        word
    }

    fn word(self) -> i64 {
        self
    }
}

impl Elem for u8 {
    fn from_word(word: i64) -> Self {
        u8::from(word != 0)
    }

    fn word(self) -> i64 {
        i64::from(self)
    }
}

/// Runs `$body` with `$vec` bound to `$list`'s storage as a `&mut Vec` of
/// its element type (`u8` or `i64`); `Elem::from_word`/`Elem::word` in the
/// body convert by inference.
macro_rules! with_vec {
    ($list:expr, |$vec:ident| $body:expr) => {
        if $list.kind == LIST_KIND_BOOL {
            $list.with_typed_vec::<u8, _>(|$vec| $body)
        } else {
            $list.with_typed_vec::<i64, _>(|$vec| $body)
        }
    };
}

impl PyListObj {
    /// The element count.
    pub(crate) fn len(&self) -> i64 {
        self.len
    }

    /// The element kind (`LIST_KIND_*`, or `value::TUPLE_OBJECT`).
    pub(crate) fn kind(&self) -> i64 {
        self.kind
    }

    fn typed<T: Elem>(&self) -> &[T] {
        if self.len == 0 {
            return &[];
        }
        unsafe { std::slice::from_raw_parts(self.data.cast::<T>(), self.len as usize) }
    }

    /// The elements as words: borrowed, or widened from a `list[bool]`'s
    /// bytes.
    fn items(&self) -> std::borrow::Cow<'_, [i64]> {
        if self.kind == LIST_KIND_BOOL {
            std::borrow::Cow::Owned(self.typed::<u8>().iter().map(|b| b.word()).collect())
        } else {
            std::borrow::Cow::Borrowed(self.typed::<i64>())
        }
    }

    /// Every element with its kind: a tuple object's per-element kinds, or
    /// the list's one element kind.
    pub(crate) fn kinded_items(&self) -> Vec<(i64, i64)> {
        if self.kind == crate::value::TUPLE_OBJECT {
            let kinds = crate::tuple::kinds_of(self);
            return self
                .typed::<i64>()
                .iter()
                .zip(kinds)
                .map(|(word, kind)| (i64::from(*kind), *word))
                .collect();
        }
        self.items().iter().map(|word| (self.kind, *word)).collect()
    }

    /// The word of element `i`, which must be in range.
    pub(crate) fn word_at(&self, i: usize) -> i64 {
        if self.kind == LIST_KIND_BOOL {
            self.typed::<u8>()[i].word()
        } else {
            self.typed::<i64>()[i]
        }
    }

    /// Runs `f` on the backing storage as a `Vec<T>`, writing its (possibly
    /// reallocated) parts back afterwards. `T` must be the kind's element
    /// type; [`with_vec!`] picks it.
    fn with_typed_vec<T: Elem, R>(&mut self, f: impl FnOnce(&mut Vec<T>) -> R) -> R {
        self.compact_front::<T>();
        let mut vec = unsafe {
            Vec::from_raw_parts(self.data.cast::<T>(), self.len as usize, self.cap as usize)
        };
        let result = f(&mut vec);
        let mut vec = std::mem::ManuallyDrop::new(vec);
        self.data = vec.as_mut_ptr().cast::<u8>();
        self.len = vec.len() as i64;
        self.cap = vec.capacity() as i64;
        result
    }

    /// Moves the elements back to the start of the allocation (see
    /// [`PyListObj::head`]).
    fn compact_front<T: Elem>(&mut self) {
        if self.head == 0 {
            return;
        }
        // SAFETY: the allocation holds `head + len` elements from `base`.
        unsafe {
            let base = self.data.cast::<T>().sub(self.head as usize);
            std::ptr::copy(self.data.cast::<T>(), base, self.len as usize);
            self.data = base.cast::<u8>();
        }
        self.head = 0;
    }

    /// The `crate::rc` kind byte of this list's elements.
    fn element_rc_kind(&self) -> u8 {
        element_rc_kind(self.kind)
    }

    fn retain(&self, word: i64) {
        retain_word(self.element_rc_kind(), word);
    }

    fn release(&self, word: i64) {
        crate::rc::release_word(self.element_rc_kind(), word);
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

/// The `crate::rc` kind byte of a list of kind `kind`'s elements.
fn element_rc_kind(kind: i64) -> u8 {
    crate::value::rc_kind(kind)
}

/// A list of kind `kind` owning `items`, whose element type must be the
/// kind's (`u8` exactly for `LIST_KIND_BOOL`), with a count of one.
fn new_typed_list<T: Elem>(kind: i64, items: Vec<T>) -> *mut PyListObj {
    let mut items = std::mem::ManuallyDrop::new(items);
    LIST_LIVE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    Box::into_raw(Box::new(PyListObj {
        len: items.len() as i64,
        data: items.as_mut_ptr().cast::<u8>(),
        cap: items.capacity() as i64,
        kind,
        rc: 1,
        kinds: std::ptr::null_mut(),
        frozen: false,
        flavor: 0,
        maxlen: -1,
        head: 0,
    }))
}

/// Marks `list` -- a function's `*args`, built for the call -- a tuple
/// (`frozen`). A `list[bool]` keeps its byte storage and stays a list
/// (`docs/DEVIATIONS.md`).
///
/// # Safety
/// `list` must be a live `PyListObj` pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_list_mark_tuple(list: *mut PyListObj) {
    // SAFETY (caller): live.
    let list = unsafe { &mut *list };
    if list.kind != LIST_KIND_BOOL && list.kind != crate::value::TUPLE_OBJECT {
        list.frozen = true;
    }
}

/// `result`, frozen when `source` is (a slice or repeat of a tuple is a
/// tuple).
fn frozen_like(source: &PyListObj, result: *mut PyListObj) -> *mut PyListObj {
    if source.frozen {
        // SAFETY: freshly built.
        unsafe { (*result).frozen = true };
    }
    result
}

/// Frees `list` (count zero), pushing every element whose count its
/// release brings to zero onto `dead`.
pub(crate) fn free_list(list: *mut PyListObj, dead: &mut Worklist) {
    LIST_LIVE.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
    // SAFETY: the count reached zero, so nothing else names the list.
    let mut list = unsafe { Box::from_raw(list) };
    if list.kind == crate::value::TUPLE_OBJECT {
        let kinds = crate::tuple::kinds_of(&list);
        for (word, kind) in list.typed::<i64>().iter().zip(kinds) {
            release_word_into(crate::value::rc_kind(i64::from(*kind)), *word, dead);
        }
        crate::tuple::free_kinds(&mut list);
    } else {
        let kind = list.element_rc_kind();
        if kind != b'w' {
            for word in list.typed::<i64>() {
                release_word_into(kind, *word, dead);
            }
        }
    }
    with_vec!(list, |items| drop(std::mem::take(items)));
}

/// A new object of kind `kind` (not `LIST_KIND_BOOL`) owning `words`.
pub(crate) fn new_raw(kind: i64, words: Vec<i64>) -> *mut PyListObj {
    new_typed_list(kind, words)
}

/// A new list of kind `kind` owning the elements `words` denote.
pub(crate) fn new_raw_words(kind: i64, words: Vec<i64>) -> *mut PyListObj {
    new_list(kind, words)
}

/// A list of kind `kind` holding the elements `items` denote.
fn new_list(kind: i64, items: Vec<i64>) -> *mut PyListObj {
    if kind == LIST_KIND_BOOL {
        new_typed_list(
            kind,
            items.into_iter().map(u8::from_word).collect::<Vec<u8>>(),
        )
    } else {
        new_typed_list(kind, items)
    }
}

/// A fresh list of element kind `kind` with room for `capacity` elements.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_list_new(kind: i64, capacity: i64) -> *mut PyListObj {
    let capacity = capacity.max(0) as usize;
    if kind == LIST_KIND_BOOL {
        new_typed_list(kind, Vec::<u8>::with_capacity(capacity))
    } else {
        new_typed_list(kind, Vec::<i64>::with_capacity(capacity))
    }
}

/// `xs.append(word)`, adopting the caller's reference to a `str` word.
///
/// # Safety
/// `list` must be a live `PyListObj` pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_list_append(list: *mut PyListObj, word: i64) {
    let list = unsafe { &mut *list };
    if list.flavor == deque::DEQUE {
        deque::append(list, word);
        return;
    }
    with_vec!(list, |items| items.push(Elem::from_word(word)));
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
        Some(i) => list.word_at(i),
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

/// `IndexError: tuple index out of range` for a computed index into a
/// struct tuple (`pycc_codegen::tuple_index`), which has no object to name.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_tuple_index_error() {
    raise_builtin(
        EXCEPTION_TYPE_INDEX_ERROR,
        "IndexError",
        "tuple index out of range",
    );
}

/// [`pycc_rt_list_index_error`] naming `list`'s own type: `deque index out
/// of range`, `tuple index out of range`. Generated code calls it on the
/// cold path of an inlined bounds check.
///
/// # Safety
/// `list` must be a live `PyListObj` pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_list_index_error_of(list: *mut PyListObj, which: i64) {
    let list = unsafe { &*list };
    let name = deque::type_name(list).trim_start_matches("collections.");
    let message = if which == 0 || name == "tuple" {
        format!("{name} index out of range")
    } else {
        format!("{name} assignment index out of range")
    };
    raise_builtin(EXCEPTION_TYPE_INDEX_ERROR, "IndexError", &message);
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
            let old = with_vec!(list, |items| {
                Elem::word(std::mem::replace(&mut items[i], Elem::from_word(word)))
            });
            list.release(old);
        }
        None => {
            list.release(word);
            pycc_rt_list_index_error(1);
        }
    }
}

/// The element kind of `xs[index]` (`LIST_KIND_*`): a tuple object's own
/// per-element kind, a list's one kind. The `--ext` shim reads it to copy a
/// fresh native container into a CPython one (`pycc_ext_obj_pack_list`).
///
/// # Safety
/// `list` must be a live `PyListObj` pointer and `index` in `[0, len)`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_list_elem_kind(list: *mut PyListObj, index: i64) -> i64 {
    let list = unsafe { &*list };
    if list.kinds.is_null() {
        list.kind
    } else {
        i64::from(unsafe { *list.kinds.add(index as usize) })
    }
}

/// Whether `xs` is a tuple object (it has per-element kinds, or is a
/// homogeneous tuple held as a list).
///
/// # Safety
/// `list` must be a live `PyListObj` pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_list_is_tuple(list: *mut PyListObj) -> i64 {
    let list = unsafe { &*list };
    i64::from(!list.kinds.is_null() || list.frozen)
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
    if list.flavor == deque::DEQUE && has_index == 0 {
        return deque::pop_right(list);
    }
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
        Some(i) => with_vec!(list, |items| Elem::word(items.remove(i))),
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
    with_vec!(list, |items| items
        .insert(at as usize, Elem::from_word(word)));
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
    frozen_like(list, new_list(list.kind, picked))
}

/// The positions `seq[start:stop:step]` selects from a sequence of `len`
/// (`pycc_rt_list_slice`'s arguments), or `None` after raising
/// `ValueError` for a zero step.
pub(crate) fn slice_indices(
    len: i64,
    start: i64,
    stop: i64,
    step: i64,
    present: i64,
) -> Option<impl Iterator<Item = usize>> {
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
        return None;
    }
    let start = (present & SLICE_HAS_START != 0).then_some(start);
    let stop = (present & SLICE_HAS_STOP != 0).then_some(stop);
    let (first, count) = adjust_slice(len, start, stop, step);
    Some((0..count).map(move |k| (first + k * step) as usize))
}

/// A new `list[str]` adopting one reference to each `str` word.
pub(crate) fn str_list(words: Vec<i64>) -> *mut PyListObj {
    new_list(LIST_KIND_STR, words)
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
    if !deque::concat_compatible(a, b) {
        return new_list(a.kind, Vec::new());
    }
    let (a_items, b_items) = (a.items(), b.items());
    let mut items = Vec::with_capacity(a_items.len() + b_items.len());
    items.extend_from_slice(&a_items);
    items.extend_from_slice(&b_items);
    for word in &items {
        a.retain(*word);
    }
    deque::flavored_like(a, new_list(a.kind, items))
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
    let count = count.max(0) as usize;
    if list.kind == LIST_KIND_BOOL {
        // Bytes repeat directly: `[True] * n` is the flag-array idiom.
        return new_typed_list(list.kind, list.typed::<u8>().repeat(count));
    }
    let items = list.items().repeat(count);
    for word in &items {
        list.retain(*word);
    }
    deque::flavored_like(list, frozen_like(list, new_list(list.kind, items)))
}

/// Python `==` between two element words of `kind`.
pub(crate) fn words_equal(kind: i64, a: i64, b: i64) -> bool {
    crate::value::eq(kind, a, b)
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
pub(crate) fn str_repr(text: &str, out: &mut String) {
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
pub(crate) fn word_repr(kind: i64, word: i64, out: &mut String) -> bool {
    crate::value::repr(kind, word, out)
}

/// Appends `repr()` of a list (`[a, b]`) or a tuple object (`(a, b)`,
/// `(a,)`) to `out`.
pub(crate) fn sequence_repr(list: &PyListObj, out: &mut String) -> bool {
    if list.flavor == deque::DEQUE {
        return deque::repr(list, out);
    }
    let tuple = list.kind == crate::value::TUPLE_OBJECT || list.frozen;
    out.push(if tuple { '(' } else { '[' });
    let items = list.kinded_items();
    for (i, (kind, word)) in items.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        if !word_repr(*kind, *word, out) {
            return false;
        }
    }
    if tuple && items.len() == 1 {
        out.push(',');
    }
    out.push(if tuple { ')' } else { ']' });
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
    if !sequence_repr(unsafe { &*list }, &mut out) {
        return new_pystr(b"");
    }
    new_pystr(out.as_bytes())
}

/// The words of a `list[int]`, for `frozenset(xs)`.
pub(crate) fn list_words(list: *mut PyListObj) -> Vec<i64> {
    unsafe { &*list }.items().to_vec()
}

pub(crate) mod deque;
pub(crate) mod heap;
pub(crate) mod ops;
pub(crate) mod order;

#[cfg(test)]
mod tests;
