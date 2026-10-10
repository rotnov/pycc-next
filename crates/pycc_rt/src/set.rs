//! `set[T]`'s and `frozenset[T]`'s one runtime object, `PySetObj`, for
//! every element type the compiler admits but a user-class instance (which
//! keeps `int_set.rs`'s object, driven by the class's own `__hash__` and
//! `__eq__`).
//!
//! **CPython's table, probe for probe.** A set's iteration order is its
//! hash table's slot order, which programs print. This is
//! `Objects/setobject.c`'s open-addressing table reproduced exactly -- the
//! same minimum size, linear probes, perturbation, resize thresholds and
//! growth factors, the same dummy handling on deletion, and the same
//! algorithms for copy, update, `pop`, `|`, `&`, `-` and `^` -- with
//! CPython's hashes (`value.rs`), so a set of ints, floats or tuples of
//! them iterates in CPython's order. A `str` hash is CPython's keyed
//! SipHash, randomized per process unless `PYTHONHASHSEED` fixes it, so a
//! set of `str` iterates in CPython's order under the same seed.
//!
//! **Ownership.** The set owns one reference to every element: adding
//! adopts the caller's reference (dropping it when the element is already
//! present), and removing releases it. [`pycc_rt_set_get`] borrows;
//! [`pycc_rt_set_pop`] hands its element's reference to the caller. The
//! set itself is reference counted ([`pycc_rt_set_incref`]/
//! [`pycc_rt_set_decref`]).

use crate::list::PyListObj;
use crate::value;
use crate::{PyStrObj, new_pystr};
use std::cell::Cell;

const MINSIZE: usize = 8;
const LINEAR_PROBES: usize = 9;
const PERTURB_SHIFT: u32 = 5;

const EMPTY: u8 = 0;
const ACTIVE: u8 = 1;
const DUMMY: u8 = 2;

#[derive(Clone, Copy, Default)]
struct Entry {
    hash: i64,
    key: i64,
    state: u8,
}

/// A set: see the module doc comment.
#[repr(C)]
pub struct PySetObj {
    /// Active elements (CPython's `used`).
    len: i64,
    /// Nonzero for a frozenset: only `repr` tells the two apart.
    frozen: i64,
    _reserved: i64,
    /// [`SET_MARKER`], telling this object from an instance set
    /// (`int_set.rs`), which shares the header.
    marker: i64,
    rc: i64,
    kind: i64,
    /// Slots that are active or dummy.
    fill: usize,
    mask: usize,
    /// Where `pop` resumes its search.
    finger: usize,
    table: Vec<Entry>,
    /// `(element index, slot)` of the last [`pycc_rt_set_get`], so an
    /// in-order walk is linear.
    cursor: Cell<(i64, usize)>,
}

const _: () = assert!(std::mem::offset_of!(PySetObj, rc) == crate::rc::RC_OFFSET);

/// The header marker of a `PySetObj`.
const SET_MARKER: i64 = 0x5e7;

/// Whether `s` is an instance set (`int_set.rs`) rather than a `PySetObj`.
fn is_instance_set(s: *mut PySetObj) -> bool {
    (unsafe { (*s).marker }) == crate::int_set::INSTANCE_SET_MARKER
}

impl PySetObj {
    fn new(kind: i64) -> Self {
        PySetObj {
            len: 0,
            frozen: 0,
            _reserved: 0,
            marker: SET_MARKER,
            rc: 1,
            kind,
            fill: 0,
            mask: MINSIZE - 1,
            finger: 0,
            table: vec![Entry::default(); MINSIZE],
            cursor: Cell::new((-1, 0)),
        }
    }

    fn invalidate(&self) {
        self.cursor.set((-1, 0));
    }

    /// `set_lookkey`: the slot holding `key`, or the empty slot ending the
    /// probe sequence.
    fn lookkey(&self, key: i64, hash: i64) -> usize {
        let mask = self.mask;
        let mut perturb = hash as u64;
        let mut i = (hash as u64 as usize) & mask;
        loop {
            let mut j = i;
            let mut probes = if i + LINEAR_PROBES <= mask {
                LINEAR_PROBES
            } else {
                0
            };
            loop {
                let entry = &self.table[j];
                if entry.state == EMPTY {
                    return j;
                }
                if entry.state == ACTIVE
                    && entry.hash == hash
                    && (entry.key == key || value::eq(self.kind, entry.key, key))
                {
                    return j;
                }
                if probes == 0 {
                    break;
                }
                probes -= 1;
                j += 1;
            }
            perturb >>= PERTURB_SHIFT;
            i = (i
                .wrapping_mul(5)
                .wrapping_add(1)
                .wrapping_add(perturb as usize))
                & mask;
        }
    }

    fn contains_entry(&self, key: i64, hash: i64) -> bool {
        self.table[self.lookkey(key, hash)].state == ACTIVE
    }

    /// `set_add_entry`, adopting `key`'s reference.
    fn add_entry(&mut self, key: i64, hash: i64) {
        self.invalidate();
        let mask = self.mask;
        let mut perturb = hash as u64;
        let mut i = (hash as u64 as usize) & mask;
        let mut freeslot: Option<usize> = None;
        let slot = 'search: loop {
            let mut j = i;
            let mut probes = if i + LINEAR_PROBES <= mask {
                LINEAR_PROBES
            } else {
                0
            };
            loop {
                let entry = self.table[j];
                if entry.state == EMPTY {
                    break 'search j;
                }
                if entry.state == ACTIVE {
                    if entry.hash == hash
                        && (entry.key == key || value::eq(self.kind, entry.key, key))
                    {
                        value::release(self.kind, key);
                        return;
                    }
                } else {
                    freeslot = Some(j);
                }
                if probes == 0 {
                    break;
                }
                probes -= 1;
                j += 1;
            }
            perturb >>= PERTURB_SHIFT;
            i = (i
                .wrapping_mul(5)
                .wrapping_add(1)
                .wrapping_add(perturb as usize))
                & mask;
        };
        if let Some(free) = freeslot {
            self.len += 1;
            self.table[free] = Entry {
                hash,
                key,
                state: ACTIVE,
            };
            return;
        }
        self.fill += 1;
        self.len += 1;
        self.table[slot] = Entry {
            hash,
            key,
            state: ACTIVE,
        };
        if self.fill * 5 < mask * 3 {
            return;
        }
        let used = self.len as usize;
        self.resize(if used > 50000 { used * 2 } else { used * 4 });
    }

    /// `set_insert_clean` into a table with no dummies.
    fn insert_clean(table: &mut [Entry], mask: usize, key: i64, hash: i64) {
        let mut perturb = hash as u64;
        let mut i = (hash as u64 as usize) & mask;
        loop {
            if table[i].state == EMPTY {
                table[i] = Entry {
                    hash,
                    key,
                    state: ACTIVE,
                };
                return;
            }
            if i + LINEAR_PROBES <= mask {
                for j in i + 1..=i + LINEAR_PROBES {
                    if table[j].state == EMPTY {
                        table[j] = Entry {
                            hash,
                            key,
                            state: ACTIVE,
                        };
                        return;
                    }
                }
            }
            perturb >>= PERTURB_SHIFT;
            i = (i
                .wrapping_mul(5)
                .wrapping_add(1)
                .wrapping_add(perturb as usize))
                & mask;
        }
    }

    /// `set_table_resize`.
    fn resize(&mut self, minused: usize) {
        self.invalidate();
        let mut newsize = MINSIZE;
        while newsize <= minused {
            newsize <<= 1;
        }
        if newsize == MINSIZE && self.mask == MINSIZE - 1 && self.fill == self.len as usize {
            return;
        }
        let old = std::mem::replace(&mut self.table, vec![Entry::default(); newsize]);
        self.mask = newsize - 1;
        self.fill = self.len as usize;
        for entry in old.iter().filter(|e| e.state == ACTIVE) {
            Self::insert_clean(&mut self.table, self.mask, entry.key, entry.hash);
        }
    }

    /// `set_discard_entry`: whether `key` was present (its reference
    /// released).
    fn discard_entry(&mut self, key: i64, hash: i64) -> bool {
        let slot = self.lookkey(key, hash);
        if self.table[slot].state != ACTIVE {
            return false;
        }
        self.invalidate();
        let old = self.table[slot].key;
        self.table[slot] = Entry {
            hash: -1,
            key: 0,
            state: DUMMY,
        };
        self.len -= 1;
        value::release(self.kind, old);
        true
    }

    /// The active entries in slot order.
    fn active(&self) -> Vec<(i64, i64)> {
        self.table
            .iter()
            .filter(|e| e.state == ACTIVE)
            .map(|e| (e.key, e.hash))
            .collect()
    }

    /// `set_merge`: adds every element of `other` (taking references).
    fn merge(&mut self, other: &PySetObj) {
        if std::ptr::eq(self, other) || other.len == 0 {
            return;
        }
        self.invalidate();
        if (self.fill + other.len as usize) * 5 >= self.mask * 3 {
            self.resize((self.len + other.len) as usize * 2);
        }
        if self.fill == 0 && self.mask == other.mask && other.fill == other.len as usize {
            for (i, entry) in other.table.iter().enumerate() {
                if entry.state == ACTIVE {
                    value::retain(self.kind, entry.key);
                    self.table[i] = *entry;
                }
            }
            self.fill = other.fill;
            self.len = other.len;
            return;
        }
        if self.fill == 0 {
            self.fill = other.len as usize;
            self.len = other.len;
            for entry in other.table.iter().filter(|e| e.state == ACTIVE) {
                value::retain(self.kind, entry.key);
                Self::insert_clean(&mut self.table, self.mask, entry.key, entry.hash);
            }
            return;
        }
        for (key, hash) in other.active() {
            value::retain(self.kind, key);
            self.add_entry(key, hash);
        }
    }

    /// Adds `key` (adopting its reference), hashing it first.
    fn add_key(&mut self, key: i64) {
        match value::hash_or_raise(self.kind, key) {
            Some(hash) => self.add_entry(key, hash),
            None => value::release(self.kind, key),
        }
    }

    /// `set_clear_internal`.
    fn clear(&mut self) {
        self.invalidate();
        let old = std::mem::replace(&mut self.table, vec![Entry::default(); MINSIZE]);
        self.mask = MINSIZE - 1;
        self.fill = 0;
        self.len = 0;
        for entry in old.iter().filter(|e| e.state == ACTIVE) {
            value::release(self.kind, entry.key);
        }
    }

    /// `set_difference_update_internal` with a set operand.
    fn difference_update(&mut self, other: &PySetObj) {
        if std::ptr::eq(self, other) {
            self.clear();
            return;
        }
        let other_items = if (other.len >> 3) > self.len {
            let inter = intersection(self, other);
            let items = unsafe { &*inter }.active();
            for (key, _) in &items {
                value::retain(self.kind, *key);
            }
            free_set(inter);
            items
        } else {
            let items = other.active();
            for (key, _) in &items {
                value::retain(self.kind, *key);
            }
            items
        };
        for (key, hash) in other_items {
            self.discard_entry(key, hash);
            value::release(self.kind, key);
        }
        if self.fill - self.len as usize <= self.mask / 4 {
            return;
        }
        let used = self.len as usize;
        self.resize(if used > 50000 { used * 2 } else { used * 4 });
    }

    /// `set_symmetric_difference_update_set`.
    fn symmetric_difference_update(&mut self, other: &PySetObj) {
        for (key, hash) in other.active() {
            if !self.discard_entry(key, hash) {
                value::retain(self.kind, key);
                self.add_entry(key, hash);
            }
        }
    }
}

fn set<'a>(s: *mut PySetObj) -> &'a mut PySetObj {
    unsafe { &mut *s }
}

pub(crate) fn set_len(s: *mut PySetObj) -> i64 {
    set(s).len
}

fn boxed(s: PySetObj) -> *mut PySetObj {
    crate::rc::SET_LIVE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    Box::into_raw(Box::new(s))
}

/// Releases a set's elements and frees it.
fn free_set(s: *mut PySetObj) {
    let mut owned = unsafe { Box::from_raw(s) };
    owned.clear();
    crate::rc::SET_LIVE.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
}

/// Takes one more reference to a set or frozenset of either kind.
///
/// # Safety
/// `s` must be null or a live set.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_set_incref(s: *mut PySetObj) {
    if !s.is_null() {
        unsafe { *s.cast::<u8>().add(crate::rc::RC_OFFSET).cast::<i64>() += 1 };
    }
}

/// Drops one reference to a set or frozenset of either kind, freeing it
/// (and releasing the elements it owns) at zero.
///
/// # Safety
/// `s` must be null or a live set not used again after its last release.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_set_decref(s: *mut PySetObj) {
    crate::rc::release_word(b'e', s as i64);
}

/// Frees a set of either kind whose count reached zero, its elements'
/// releases going onto `dead` (`crate::rc`'s worklist) rather than
/// recursing.
pub(crate) fn free_set_into(s: *mut PySetObj, dead: &mut crate::rc::Worklist) {
    if is_instance_set(s) {
        crate::int_set::free(s.cast(), dead);
        return;
    }
    let owned = unsafe { Box::from_raw(s) };
    crate::rc::SET_LIVE.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
    let kind = value::rc_kind(owned.kind);
    for entry in owned.table.iter().filter(|e| e.state == ACTIVE) {
        crate::rc::release_word_into(kind, entry.key, dead);
    }
}

/// `set_intersection` of two sets, as a new set.
fn intersection(a: &PySetObj, b: &PySetObj) -> *mut PySetObj {
    if std::ptr::eq(a, b) {
        let mut copy = PySetObj::new(a.kind);
        copy.merge(a);
        return boxed(copy);
    }
    let mut result = PySetObj::new(a.kind);
    let (so, other) = if b.len > a.len { (b, a) } else { (a, b) };
    for (key, hash) in other.active() {
        if so.contains_entry(key, hash) {
            value::retain(a.kind, key);
            result.add_entry(key, hash);
        }
    }
    boxed(result)
}

/// A fresh empty set of element kind `kind`.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_set_new(kind: i64) -> *mut PySetObj {
    boxed(PySetObj::new(kind))
}

/// `s.add(word)`, adopting the reference.
///
/// # Safety
/// `s` must be a live set.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_set_add(s: *mut PySetObj, word: i64) {
    set(s).add_key(word);
}

/// `word in s` (`word` borrowed).
///
/// # Safety
/// `s` must be a live set.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_set_contains(s: *mut PySetObj, word: i64) -> i8 {
    let s = set(s);
    match value::hash_or_raise(s.kind, word) {
        Some(hash) => i8::from(s.contains_entry(word, hash)),
        None => 0,
    }
}

/// `len(s)` of a set or an instance set.
///
/// # Safety
/// `s` must be a live set.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_set_len(s: *mut PySetObj) -> i64 {
    unsafe { (*s).len }
}

/// The element at position `index` of the iteration (slot) order,
/// borrowed. `index` must be in range.
///
/// # Safety
/// `s` must be a live set.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_set_get(s: *mut PySetObj, index: i64) -> i64 {
    if is_instance_set(s) {
        return crate::int_set::element_at(s.cast(), index);
    }
    let s = set(s);
    let (last, slot) = s.cursor.get();
    let (mut at, mut i) = if last >= 0 && last < index {
        (last, slot + 1)
    } else {
        (-1, 0)
    };
    loop {
        if s.table[i].state == ACTIVE {
            at += 1;
            if at == index {
                s.cursor.set((index, i));
                return s.table[i].key;
            }
        }
        i += 1;
    }
}

/// `s.discard(word)` (`word` borrowed).
///
/// # Safety
/// `s` must be a live set.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_set_discard(s: *mut PySetObj, word: i64) {
    let s = set(s);
    if let Some(hash) = value::hash_or_raise(s.kind, word) {
        s.discard_entry(word, hash);
    }
}

/// `s.remove(word)`: `KeyError(word)` when absent (`word` borrowed).
///
/// # Safety
/// `s` must be a live set.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_set_remove(s: *mut PySetObj, word: i64) {
    let s = set(s);
    if let Some(hash) = value::hash_or_raise(s.kind, word)
        && !s.discard_entry(word, hash)
    {
        crate::exception::raise_key_error(s.kind, word);
    }
}

/// `s.pop()`: CPython's `set_pop`, handing the element's reference to the
/// caller; `KeyError` when empty.
///
/// # Safety
/// `s` must be a live set.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_set_pop(s: *mut PySetObj) -> i64 {
    let s = set(s);
    if s.len == 0 {
        crate::raise_builtin(
            crate::exception::EXCEPTION_TYPE_KEY_ERROR,
            "KeyError",
            "pop from an empty set",
        );
        return crate::dict::sentinel(s.kind);
    }
    s.invalidate();
    let mut i = s.finger & s.mask;
    while s.table[i].state != ACTIVE {
        i += 1;
        if i > s.mask {
            i = 0;
        }
    }
    let key = s.table[i].key;
    s.table[i] = Entry {
        hash: -1,
        key: 0,
        state: DUMMY,
    };
    s.len -= 1;
    s.finger = i + 1;
    key
}

/// `s.clear()`.
///
/// # Safety
/// `s` must be a live set.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_set_clear(s: *mut PySetObj) {
    set(s).clear();
}

/// `s.copy()` / `set(s)` / `frozenset(s)`: CPython's `set_copy`.
///
/// # Safety
/// `s` must be a live set.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_set_copy(s: *mut PySetObj) -> *mut PySetObj {
    let s = set(s);
    let mut copy = PySetObj::new(s.kind);
    copy.merge(s);
    boxed(copy)
}

/// `set(xs)` of a list (or a tuple object): its elements added in order.
///
/// # Safety
/// `list` must be a live list.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_set_from_list(list: *mut PyListObj, kind: i64) -> *mut PySetObj {
    let mut s = PySetObj::new(kind);
    for (k, word) in unsafe { &*list }.kinded_items() {
        value::retain(k, word);
        s.add_key(word);
    }
    boxed(s)
}

/// `set(s)` of a `str`: its characters added in order.
///
/// # Safety
/// `text` must be a live `str`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_set_from_str(text: *mut PyStrObj) -> *mut PySetObj {
    let mut s = PySetObj::new(crate::list::LIST_KIND_STR);
    let text = String::from_utf8_lossy(unsafe { &*text }.bytes()).into_owned();
    let mut buf = [0u8; 4];
    for c in text.chars() {
        s.add_key(new_pystr(c.encode_utf8(&mut buf).as_bytes()) as i64);
    }
    boxed(s)
}

/// `set(d)` of a dict: CPython's `set_update_dict`, keys in insertion
/// order after one up-front resize.
///
/// # Safety
/// `d` must be a live dict.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_set_from_dict(d: *mut crate::dict::PyDictObj) -> *mut PySetObj {
    let keys = unsafe { crate::dict::pycc_rt_dict_keys(d) };
    let keys_ref = unsafe { &*keys };
    let mut s = PySetObj::new(keys_ref.kind());
    let n = keys_ref.len() as usize;
    if n * 5 >= s.mask * 3 {
        s.resize(n * 2);
    }
    for (kind, word) in keys_ref.kinded_items() {
        value::retain(kind, word);
        s.add_key(word);
    }
    unsafe { crate::pycc_rt_list_decref(keys) };
    boxed(s)
}

/// `s.update(t)` / `s |= t` with a set operand.
///
/// # Safety
/// `s` and `t` must be live sets of one kind.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_set_update(s: *mut PySetObj, t: *mut PySetObj) {
    let other = set(t);
    set(s).merge(other);
}

/// `s.update(xs)` with a list operand.
///
/// # Safety
/// `s` must be a live set and `list` a live list.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_set_update_list(s: *mut PySetObj, list: *mut PyListObj) {
    let s = set(s);
    for (k, word) in unsafe { &*list }.kinded_items() {
        value::retain(k, word);
        s.add_key(word);
    }
}

/// `a | b` as a new set.
///
/// # Safety
/// `a` and `b` must be live sets of one kind.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_set_union(a: *mut PySetObj, b: *mut PySetObj) -> *mut PySetObj {
    let result = unsafe { pycc_rt_set_copy(a) };
    if a != b {
        set(result).merge(set(b));
    }
    inherit_frozen(result, a)
}

/// `result`, a frozenset when `a` is one: CPython's set operators return
/// the left operand's type.
fn inherit_frozen(result: *mut PySetObj, a: *mut PySetObj) -> *mut PySetObj {
    set(result).frozen = set(a).frozen;
    result
}

/// Marks a new set as a frozenset (`frozenset(...)`); returns it.
///
/// # Safety
/// `s` must be a live set of either kind.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_set_freeze(s: *mut PySetObj) -> *mut PySetObj {
    // Both kinds of set keep the flag at the same header offset.
    unsafe { (*s).frozen = 1 };
    s
}

/// `s.copy()`: a frozenset's copy is a frozenset.
///
/// # Safety
/// `s` must be a live set.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_set_copy_method(s: *mut PySetObj) -> *mut PySetObj {
    inherit_frozen(unsafe { pycc_rt_set_copy(s) }, s)
}

/// `a & b` as a new set.
///
/// # Safety
/// `a` and `b` must be live sets of one kind.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_set_intersection(
    a: *mut PySetObj,
    b: *mut PySetObj,
) -> *mut PySetObj {
    inherit_frozen(intersection(set(a), set(b)), a)
}

/// `a - b` as a new set: CPython's `set_difference`.
///
/// # Safety
/// `a` and `b` must be live sets of one kind.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_set_difference(
    a: *mut PySetObj,
    b: *mut PySetObj,
) -> *mut PySetObj {
    let (so, other) = (set(a), set(b));
    if (so.len >> 2) > other.len {
        let result = unsafe { pycc_rt_set_copy(a) };
        set(result).difference_update(other);
        return inherit_frozen(result, a);
    }
    let mut result = PySetObj::new(so.kind);
    for (key, hash) in so.active() {
        if !other.contains_entry(key, hash) {
            value::retain(so.kind, key);
            result.add_entry(key, hash);
        }
    }
    inherit_frozen(boxed(result), a)
}

/// `a ^ b` as a new set: CPython's `set_symmetric_difference`.
///
/// # Safety
/// `a` and `b` must be live sets of one kind.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_set_symmetric_difference(
    a: *mut PySetObj,
    b: *mut PySetObj,
) -> *mut PySetObj {
    let mut result = PySetObj::new(set(a).kind);
    result.merge(set(b));
    result.symmetric_difference_update(set(a));
    inherit_frozen(boxed(result), a)
}

/// `a &= b`: CPython computes the intersection and swaps it in.
///
/// # Safety
/// `a` and `b` must be live sets of one kind.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_set_intersection_update(a: *mut PySetObj, b: *mut PySetObj) {
    let tmp = intersection(set(a), set(b));
    swap_bodies(set(a), set(tmp));
    free_set(tmp);
}

/// CPython's `set_swap_bodies`: exchanges only the table state (`len`,
/// `fill`, `mask`, `table`). Each object keeps its own identity -- its
/// reference count, marker, element kind, frozen flag and `pop` finger --
/// so aliases of `a` keep a valid count and `a` stays the object they see.
fn swap_bodies(a: &mut PySetObj, b: &mut PySetObj) {
    debug_assert_eq!(a.kind, b.kind);
    std::mem::swap(&mut a.len, &mut b.len);
    std::mem::swap(&mut a.fill, &mut b.fill);
    std::mem::swap(&mut a.mask, &mut b.mask);
    std::mem::swap(&mut a.table, &mut b.table);
    a.invalidate();
    b.invalidate();
}

/// `a -= b`.
///
/// # Safety
/// `a` and `b` must be live sets of one kind.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_set_difference_update(a: *mut PySetObj, b: *mut PySetObj) {
    if a == b {
        set(a).clear();
        return;
    }
    let other = set(b);
    set(a).difference_update(other);
}

/// `a ^= b`.
///
/// # Safety
/// `a` and `b` must be live sets of one kind.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_set_symmetric_difference_update(
    a: *mut PySetObj,
    b: *mut PySetObj,
) {
    if a == b {
        set(a).clear();
        return;
    }
    let other = set(b);
    set(a).symmetric_difference_update(other);
}

/// `a |= b`, `a &= b`, `a -= b`, `a ^= b` on a `set` (`op` the encoded int `0`..`3`):
/// `a` updated in place and returned as a new reference, as CPython's
/// in-place operators return the left operand itself.
///
/// # Safety
/// `a` and `b` must be live sets of one kind.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_set_inplace(
    a: *mut PySetObj,
    b: *mut PySetObj,
    op: i64,
) -> *mut PySetObj {
    unsafe {
        match crate::int_encoding::inline_int_value(op).unwrap_or(3) {
            0 => pycc_rt_set_update(a, b),
            1 => pycc_rt_set_intersection_update(a, b),
            2 => pycc_rt_set_difference_update(a, b),
            _ => pycc_rt_set_symmetric_difference_update(a, b),
        }
        (*a).rc += 1;
    }
    a
}

/// `a <= b` between two sets.
///
/// # Safety
/// `a` and `b` must be live sets of one kind.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_set_issubset(a: *mut PySetObj, b: *mut PySetObj) -> i8 {
    let (a, b) = (set(a), set(b));
    i8::from(a.len <= b.len && a.active().iter().all(|(k, h)| b.contains_entry(*k, *h)))
}

/// `a.isdisjoint(b)`.
///
/// # Safety
/// `a` and `b` must be live sets of one kind.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_set_isdisjoint(a: *mut PySetObj, b: *mut PySetObj) -> i8 {
    let (a, b) = (set(a), set(b));
    i8::from(!a.active().iter().any(|(k, h)| b.contains_entry(*k, *h)))
}

/// `a == b` for two sets of one kind.
///
/// The type checker never routes an instance set here (its elements need
/// their class's `__eq__`); should one arrive anyway it is compared by
/// element identity rather than read as a hash table.
pub(crate) fn sets_equal(a: *mut PySetObj, b: *mut PySetObj) -> bool {
    if is_instance_set(a) || is_instance_set(b) {
        if !(is_instance_set(a) && is_instance_set(b)) {
            return false;
        }
        let (xs, ys) = (
            crate::int_set::items(a.cast()),
            crate::int_set::items(b.cast()),
        );
        return xs.len() == ys.len() && xs.iter().all(|x| ys.contains(x));
    }
    let (a, b) = (set(a), set(b));
    a.len == b.len && a.active().iter().all(|(k, h)| b.contains_entry(*k, *h))
}

/// The elements in iteration order as a new list (`list(s)`,
/// `sorted(s)`'s input).
///
/// # Safety
/// `s` must be a live set.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_set_to_list(s: *mut PySetObj) -> *mut PyListObj {
    let s = set(s);
    let words: Vec<i64> = s.active().into_iter().map(|(k, _)| k).collect();
    for word in &words {
        value::retain(s.kind, *word);
    }
    crate::list::new_raw_words(s.kind, words)
}

/// Appends `repr(s)` to `out` (`{1, 2}`, `set()` when empty).
pub(crate) fn set_repr_into(s: *mut PySetObj, out: &mut String) -> bool {
    if is_instance_set(s) {
        // Rendering an instance needs its class's `__repr__`, which the
        // runtime cannot call; the type checker never routes one here.
        crate::raise_builtin(
            crate::exception::EXCEPTION_TYPE_TYPE_ERROR,
            "TypeError",
            "repr of a set of class instances is not compiled",
        );
        return false;
    }
    let s = set(s);
    if s.frozen != 0 {
        if s.len == 0 {
            out.push_str("frozenset()");
            return true;
        }
        out.push_str("frozenset(");
    } else if s.len == 0 {
        out.push_str("set()");
        return true;
    }
    out.push('{');
    for (i, (key, _)) in s.active().iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        if !value::repr(s.kind, *key, out) {
            return false;
        }
    }
    out.push('}');
    if s.frozen != 0 {
        out.push(')');
    }
    true
}

/// `str(s)` / `repr(s)` as a new `str`; `frozen` renders
/// `frozenset({...})`.
///
/// # Safety
/// `s` must be a live set.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_set_repr(s: *mut PySetObj, frozen: i8) -> *mut PyStrObj {
    let mut out = String::new();
    // `frozen` asks for the frozenset spelling of a set not flagged as one.
    let wrap = frozen != 0 && set(s).frozen == 0;
    if wrap {
        if set(s).len == 0 {
            return new_pystr(b"frozenset()");
        }
        out.push_str("frozenset(");
    }
    if !set_repr_into(s, &mut out) {
        return new_pystr(b"");
    }
    if wrap {
        out.push(')');
    }
    new_pystr(out.as_bytes())
}

/// Raises `RuntimeError: Set changed size during iteration` when `len`
/// differs from `initial` (and nothing is pending yet).
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_set_check_not_resized(len: i64, initial: i64) {
    if len != initial && crate::pycc_rt_exception_active() == 0 {
        crate::raise_builtin(
            crate::exception::EXCEPTION_TYPE_RUNTIME_ERROR,
            "RuntimeError",
            "Set changed size during iteration",
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::list::{LIST_KIND_FLOAT, LIST_KIND_INT};

    fn int(v: i64) -> i64 {
        crate::tag_smallint(v)
    }

    fn ints(values: &[i64]) -> *mut PySetObj {
        let s = pycc_rt_set_new(LIST_KIND_INT);
        for v in values {
            unsafe { pycc_rt_set_add(s, int(*v)) };
        }
        s
    }

    fn render(s: *mut PySetObj) -> String {
        let mut out = String::new();
        assert!(set_repr_into(s, &mut out));
        out
    }

    /// Orders checked against CPython 3.13.
    #[test]
    fn iteration_order_matches_cpython() {
        assert_eq!(render(ints(&[3, 1, 2, 100, 33])), "{1, 2, 3, 100, 33}");
        let many: Vec<i64> = (0..40).map(|i| i * 37 % 101).collect();
        assert_eq!(
            render(ints(&many)),
            "{0, 3, 6, 9, 10, 13, 16, 19, 20, 23, 26, 29, 30, 33, 36, 37, 40, 43, 46, 47, 50, 53, \
             56, 57, 60, 63, 67, 70, 73, 74, 77, 80, 83, 84, 87, 90, 93, 94, 97, 100}"
        );
        let neg = ints(&[-5, 7, -1, 1000, 64, 8]);
        assert_eq!(render(neg), "{64, 7, 1000, 8, -5, -1}");
        let floats = pycc_rt_set_new(LIST_KIND_FLOAT);
        for v in [0.5f64, 2.25, -1.0, 3.0] {
            unsafe { pycc_rt_set_add(floats, v.to_bits() as i64) };
        }
        assert_eq!(render(floats), "{0.5, 2.25, 3.0, -1.0}");
    }

    /// `a &= b` swaps only the table in: `a` keeps its count (an alias
    /// still owns a reference), kind and marker, and the temporary is freed
    /// with the old table, not with `a`'s identity.
    #[test]
    fn intersection_update_keeps_the_left_operand_identity() {
        let a = ints(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
        let b = ints(&[5, 6, 70]);
        unsafe {
            pycc_rt_set_incref(a);
            pycc_rt_set_intersection_update(a, b);
            assert_eq!(set(a).rc, 2);
            assert_eq!(set(a).marker, SET_MARKER);
            assert_eq!(set(a).kind, LIST_KIND_INT);
            assert_eq!(render(a), "{5, 6}");
            pycc_rt_set_add(a, int(11));
            assert_eq!(render(a), "{11, 5, 6}");
            pycc_rt_set_decref(a);
            pycc_rt_set_decref(a);
            pycc_rt_set_decref(b);
        }
    }

    /// An instance set reaching the value routines is never read as a hash
    /// table.
    #[test]
    fn instance_sets_are_not_read_as_hash_tables() {
        let a = crate::int_set::pycc_rt_int_set_new().cast::<PySetObj>();
        let b = crate::int_set::pycc_rt_int_set_new().cast::<PySetObj>();
        unsafe {
            crate::int_set::pycc_rt_obj_set_push(a.cast(), 100, 1);
            crate::int_set::pycc_rt_obj_set_push(b.cast(), 100, 1);
        }
        assert!(sets_equal(a, b));
        unsafe { crate::int_set::pycc_rt_obj_set_push(b.cast(), 200, 2) };
        assert!(!sets_equal(a, b));
        assert!(!sets_equal(a, ints(&[1])));
        let mut out = String::new();
        assert!(!set_repr_into(a, &mut out));
        assert_eq!(crate::pycc_rt_exception_active(), 1);
        crate::pycc_rt_exception_clear();
    }

    #[test]
    fn pop_and_operators_match_cpython() {
        let a = ints(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
        let b = ints(&[5, 6, 7, 20, 30]);
        unsafe {
            assert_eq!(
                render(pycc_rt_set_union(a, b)),
                "{1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 20, 30}"
            );
            assert_eq!(render(pycc_rt_set_intersection(a, b)), "{5, 6, 7}");
            assert_eq!(
                render(pycc_rt_set_difference(a, b)),
                "{1, 2, 3, 4, 8, 9, 10}"
            );
            assert_eq!(
                render(pycc_rt_set_symmetric_difference(a, b)),
                "{1, 2, 3, 4, 8, 9, 10, 20, 30}"
            );
            let p = pycc_rt_set_pop(a);
            assert_eq!(crate::inline_int_value(p), Some(1));
            let p = pycc_rt_set_pop(a);
            assert_eq!(crate::inline_int_value(p), Some(2));
            assert_eq!(pycc_rt_set_len(a), 8);
            assert_eq!(crate::inline_int_value(pycc_rt_set_get(a, 0)), Some(3));
            assert_eq!(crate::inline_int_value(pycc_rt_set_get(a, 1)), Some(4));
        }
    }
}
