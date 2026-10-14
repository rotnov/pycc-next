//! Native `itertools` iterators.
//!
//! Each `itertools` function the compiler admits natively (`pycc_ast`'s
//! `desugar::itertools` rewrites the call, `pycc_hir::builtins` types it)
//! returns a generator object (`crate::generator::PyGenObj`) whose frame is
//! not an LLVM coroutine but a [`Frame`] defined here: its resume function
//! computes the next value in Rust and stores it in the object's value area
//! exactly as a compiled `yield` of the same type would, so every consumer
//! of a generator (`for`, `list()`, `sum()`, `next()`, a comprehension, ...)
//! consumes these unchanged, lazily, as CPython does.
//!
//! **Values.** A value crosses as an [`Item`]: one container word (`list.rs`)
//! of the value's kind, owned, a tuple as a tuple object -- or, for a tuple
//! not boxed yet, its element words. A [`Desc`] (built by
//! `pycc_codegen::containers::value_desc` from the static type) says what a
//! value is and how it sits in a generator's value area: one word (a `bool`
//! one byte), or, for a tuple of only `int`/`float`/`bool`, the LLVM struct
//! compiled code keeps such a tuple in.
//!
//! **Sources.** An iterable argument is a list (borrowed while iterated, as
//! CPython iterates a list) or a generator (resumed for each value); the
//! compiler converts a `str` to the list of its characters first.

mod combinatoric;
mod lazy;

use crate::generator::{PyGenObj, native_area, native_finish, native_resume_mode};
use crate::list::{LIST_KIND_BOOL, LIST_KIND_INT, LIST_KIND_TUPLE, PyListObj};

/// Bits of a descriptor word: the value's kind (`LIST_KIND_*`).
const KIND_MASK: i64 = 0xf;
/// A tuple held by value (an LLVM struct of `int`/`float`/`bool`).
const UNBOXED: i64 = 1 << 4;
/// A tuple's arity, four bits from here.
const ARITY_SHIFT: i64 = 5;
/// Each tuple element's kind, four bits apiece from here.
const ELEMS_SHIFT: i64 = 9;
/// Source descriptors: the argument is a list.
const SOURCE_LIST: i64 = 1 << 60;
/// Source descriptors: the argument is a generator.
const SOURCE_GEN: i64 = 1 << 61;

/// What a value is and how it is stored (see the module documentation).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Desc {
    kind: i64,
    unboxed: bool,
    arity: usize,
    elems: [u8; 13],
}

impl Desc {
    pub(crate) fn decode(raw: i64) -> Self {
        let mut elems = [0u8; 13];
        let arity = ((raw >> ARITY_SHIFT) & 0xf) as usize;
        for (i, slot) in elems.iter_mut().enumerate().take(arity.min(12)) {
            *slot = ((raw >> (ELEMS_SHIFT + 4 * i as i64)) & 0xf) as u8;
        }
        Desc {
            kind: raw & KIND_MASK,
            unboxed: raw & UNBOXED != 0,
            arity,
            elems,
        }
    }

    pub(crate) fn kind(&self) -> i64 {
        self.kind
    }

    /// The kind of tuple element `i`.
    fn elem(&self, i: usize) -> i64 {
        i64::from(self.elems[i])
    }

    fn elem_kinds(&self) -> Box<[u8]> {
        self.elems[..self.arity].into()
    }

    /// The byte offset of each field of an unboxed tuple, and its size.
    fn layout(&self) -> (Vec<usize>, usize) {
        let mut offsets = Vec::with_capacity(self.arity);
        let mut at = 0usize;
        let mut align = 1usize;
        for i in 0..self.arity {
            let (size, a) = if self.elem(i) == LIST_KIND_BOOL {
                (1, 1)
            } else {
                (8, 8)
            };
            at = at.div_ceil(a) * a;
            offsets.push(at);
            at += size;
            align = align.max(a);
        }
        (offsets, at.div_ceil(align) * align)
    }

    /// The size of a value in a generator's value area.
    fn area_size(&self) -> usize {
        if self.unboxed { self.layout().1 } else { 8 }
    }
}

/// Takes one more reference to `word` of `kind` (an `int` may be a heap
/// bigint here, unlike a list's `int`).
pub(crate) fn retain(kind: i64, word: i64) {
    if kind == LIST_KIND_INT {
        crate::pycc_rt_bigint_retain(word);
    } else {
        crate::value::retain(kind, word);
    }
}

/// Drops one reference to `word` of `kind`.
pub(crate) fn release(kind: i64, word: i64) {
    if kind == LIST_KIND_INT {
        crate::pycc_rt_bigint_release(word);
    } else {
        crate::value::release(kind, word);
    }
}

/// An owned value (see the module documentation).
pub(crate) enum Item {
    Word(i64),
    Fields(Vec<i64>),
}

impl Item {
    /// The value as one word of `desc`'s kind (a tuple boxed).
    pub(crate) fn into_word(self, desc: &Desc) -> i64 {
        match self {
            Item::Word(word) => word,
            Item::Fields(words) => crate::tuple::tuple_from(words, desc.elem_kinds()) as i64,
        }
    }

    /// A tuple value's owned element words.
    pub(crate) fn into_fields(self, desc: &Desc) -> Vec<i64> {
        match self {
            Item::Fields(words) => words,
            Item::Word(word) => {
                // SAFETY: a tuple word is a live tuple object.
                let tuple = unsafe { &*(word as *mut PyListObj) };
                let words: Vec<i64> = (0..tuple.len() as usize)
                    .map(|i| {
                        let w = tuple.word_at(i);
                        retain(desc.elem(i), w);
                        w
                    })
                    .collect();
                release(LIST_KIND_TUPLE, word);
                words
            }
        }
    }

    pub(crate) fn release(self, desc: &Desc) {
        match self {
            Item::Word(word) => release(desc.kind, word),
            Item::Fields(words) => {
                for (i, word) in words.into_iter().enumerate() {
                    release(desc.elem(i), word);
                }
            }
        }
    }
}

/// Stores `item` in the value area `area` as a value of `desc`.
fn write_area(desc: &Desc, item: Item, area: *mut u8) {
    if desc.unboxed {
        let words = item.into_fields(desc);
        let (offsets, _) = desc.layout();
        for (i, (word, at)) in words.into_iter().zip(offsets).enumerate() {
            // SAFETY: the area is `area_size` bytes long.
            unsafe {
                if desc.elem(i) == LIST_KIND_BOOL {
                    area.add(at).write(u8::from(word != 0));
                } else {
                    area.add(at).cast::<i64>().write_unaligned(word);
                }
            }
        }
        return;
    }
    let word = item.into_word(desc);
    // SAFETY: the area holds at least eight bytes.
    unsafe { area.cast::<i64>().write_unaligned(word) };
}

/// Takes the value a generator left in its area as a value of `desc`.
fn read_area(desc: &Desc, area: *mut u8) -> Item {
    if desc.unboxed {
        let (offsets, _) = desc.layout();
        let words = offsets
            .iter()
            .enumerate()
            .map(|(i, at)| {
                // SAFETY: the area holds a value of this layout.
                unsafe {
                    if desc.elem(i) == LIST_KIND_BOOL {
                        i64::from(area.add(*at).read() != 0)
                    } else {
                        area.add(*at).cast::<i64>().read_unaligned()
                    }
                }
            })
            .collect();
        return Item::Fields(words);
    }
    // SAFETY: as above.
    let word = unsafe {
        if desc.kind == LIST_KIND_BOOL {
            i64::from(area.read() != 0)
        } else {
            area.cast::<i64>().read_unaligned()
        }
    };
    Item::Word(word)
}

/// An iterable argument being iterated (see the module documentation).
pub(crate) enum Source {
    List { list: *mut PyListObj, pos: usize },
    Gen { generator: *mut PyGenObj },
}

/// A [`Source`] with the descriptor of the values it produces.
pub(crate) struct Input {
    source: Source,
    desc: Desc,
}

impl Input {
    /// Iterates the argument `object` described by the source descriptor
    /// `raw`, holding a reference to it.
    pub(crate) fn new(object: *mut u8, raw: i64) -> Self {
        let desc = Desc::decode(raw);
        let source = if raw & SOURCE_GEN != 0 {
            let generator = object.cast::<PyGenObj>();
            crate::generator::pycc_rt_gen_incref(generator);
            Source::Gen { generator }
        } else {
            debug_assert!(raw & SOURCE_LIST != 0, "a list or generator source");
            let list = object.cast::<PyListObj>();
            // SAFETY: a list argument is live.
            unsafe { crate::rc::pycc_rt_list_incref(list) };
            Source::List { list, pos: 0 }
        };
        Input { source, desc }
    }

    pub(crate) fn desc(&self) -> &Desc {
        &self.desc
    }

    /// The next value, owned; `None` once exhausted or when iterating
    /// raised (the exception is pending).
    pub(crate) fn next(&mut self) -> Option<Item> {
        match &mut self.source {
            Source::List { list, pos } => {
                // SAFETY: the list is held live.
                let list = unsafe { &**list };
                if *pos >= list.len() as usize {
                    return None;
                }
                let word = list.word_at(*pos);
                *pos += 1;
                retain(list.kind(), word);
                Some(Item::Word(word))
            }
            Source::Gen { generator } => {
                // SAFETY: the generator is held live.
                if unsafe { crate::generator::pycc_rt_gen_resume(*generator) } == 0 {
                    return None;
                }
                Some(read_area(&self.desc, native_area(*generator)))
            }
        }
    }

    /// Every remaining value, as words (CPython's `tuple(iterable)` of a
    /// pool); `None` when iterating raised.
    pub(crate) fn drain_words(&mut self) -> Option<Vec<i64>> {
        let mut words = Vec::new();
        while let Some(item) = self.next() {
            words.push(item.into_word(&self.desc));
        }
        if crate::exception::pycc_rt_exception_active() != 0 {
            for word in words {
                release(self.desc.kind, word);
            }
            return None;
        }
        Some(words)
    }
}

impl Drop for Input {
    fn drop(&mut self) {
        match &mut self.source {
            Source::List { list, .. } => release(crate::list::LIST_KIND_LIST, *list as i64),
            Source::Gen { generator } => crate::generator::pycc_rt_gen_decref(*generator),
        }
    }
}

/// A pool of owned words of one kind (`permutations`, `product`, ...).
pub(crate) struct Pool {
    words: Vec<i64>,
    kind: i64,
}

impl Pool {
    /// Takes every value of `input`; `None` when iterating raised.
    pub(crate) fn take(mut input: Input) -> Option<Self> {
        let kind = input.desc().kind();
        input.drain_words().map(|words| Pool { words, kind })
    }

    pub(crate) fn len(&self) -> usize {
        self.words.len()
    }

    /// A new reference to element `i`.
    pub(crate) fn get(&self, i: usize) -> i64 {
        let word = self.words[i];
        retain(self.kind, word);
        word
    }
}

impl Drop for Pool {
    fn drop(&mut self) {
        for word in self.words.drain(..) {
            release(self.kind, word);
        }
    }
}

/// One native iterator's state.
pub(crate) trait Native {
    /// The next value; `None` once exhausted or with an exception pending.
    fn next(&mut self) -> Option<Item>;
}

/// The frame of a native iterator: the first two words are its resume and
/// destroy functions, as `crate::generator` expects of a coroutine frame.
#[repr(C)]
struct Frame {
    resume: unsafe extern "C" fn(*mut u8),
    destroy: unsafe extern "C" fn(*mut u8),
    generator: *mut PyGenObj,
    out: Desc,
    iter: Box<dyn Native>,
}

unsafe extern "C" fn resume(frame: *mut u8) {
    // SAFETY: `frame` is a live `Frame` (the generator's).
    let frame = unsafe { &mut *frame.cast::<Frame>() };
    let g = frame.generator;
    if native_resume_mode(g) != 0 {
        native_finish(g);
        return;
    }
    match frame.iter.next() {
        Some(item) => write_area(&frame.out, item, native_area(g)),
        None => native_finish(g),
    }
}

unsafe extern "C" fn destroy(frame: *mut u8) {
    // SAFETY: the generator destroys its frame exactly once.
    drop(unsafe { Box::from_raw(frame.cast::<Frame>()) });
}

/// A generator object running `iter`, yielding values of `out` (a
/// descriptor word), whose `repr` names `name` (NUL-terminated).
pub(crate) fn spawn(iter: Box<dyn Native>, out: i64, name: &'static [u8]) -> *mut PyGenObj {
    let out = Desc::decode(out);
    let frame = Box::into_raw(Box::new(Frame {
        resume,
        destroy,
        generator: std::ptr::null_mut(),
        out,
        iter,
    }));
    let g = crate::generator::pycc_rt_gen_new(
        frame.cast::<u8>(),
        out.area_size() as i64,
        None,
        None,
        name.as_ptr(),
    );
    crate::generator::mark_native(g);
    // SAFETY: freshly allocated.
    unsafe { (*frame).generator = g };
    g
}

/// An iterator that yields nothing: what a call that raised at
/// construction returns beside its pending exception.
struct Empty;

impl Native for Empty {
    fn next(&mut self) -> Option<Item> {
        None
    }
}

/// [`spawn`] of an empty iterator.
pub(crate) fn spawn_empty(out: i64, name: &'static [u8]) -> *mut PyGenObj {
    spawn(Box::new(Empty), out, name)
}

/// A small non-negative `int` argument the compiler wrote itself (an arity,
/// a flag), decoded from its encoded word.
pub(crate) fn flag(word: i64) -> i64 {
    crate::int_encoding::inline_int_value(word).unwrap_or(0)
}

/// Decodes an `int` argument that must fit a machine word, raising
/// `ValueError: {message}` otherwise (CPython's wording for each caller).
pub(crate) fn small_int(word: i64, message: &str) -> Option<i64> {
    match crate::int_encoding::inline_int_value(word) {
        Some(value) => Some(value),
        None => {
            crate::raise_builtin(crate::EXCEPTION_TYPE_VALUE_ERROR, "ValueError", message);
            None
        }
    }
}
