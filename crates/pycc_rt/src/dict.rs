//! `dict[K, V]`'s one runtime object, `PyDictObj`, for every key and value
//! type the compiler admits.
//!
//! Keys and values are container words (`value.rs`) of the dict's
//! `key_kind` and `value_kind`. Lookup is a hash table over an
//! insertion-ordered entry array, as in CPython, so iteration and `repr`
//! follow insertion order and a re-inserted key moves to the end.
//!
//! **Ownership.** The dict owns one reference to every stored key and
//! value: storing adopts the caller's references, overwriting a value
//! releases the old one (the old key is kept and the incoming one released,
//! as CPython keeps the first key object), and deleting releases both. A
//! read through [`pycc_rt_dict_get`] and the other value-returning lookups
//! returns a new reference; [`pycc_rt_dict_key_at`] and
//! [`pycc_rt_dict_value_at`] borrow. The dict itself is reference counted
//! (its count at `rc::RC_OFFSET`, like a list's) and freed through
//! `crate::rc`'s worklist.

pub(crate) mod flavor;
mod num_key;

use crate::exception::raise_key_error;
use crate::list::{LIST_KIND_TUPLE, PyListObj};
use crate::value;
use crate::{PyStrObj, new_pystr};

/// A dict: see the module doc comment.
#[repr(C)]
pub struct PyDictObj {
    /// Live entries.
    len: i64,
    /// A `Counter`'s or `defaultdict`'s flavor (`flavor`); null for a
    /// plain dict.
    extra: *mut flavor::Extra,
    _reserved: [i64; 2],
    rc: i64,
    key_kind: i64,
    value_kind: i64,
    table: Table,
}

const _: () = assert!(std::mem::offset_of!(PyDictObj, rc) == crate::rc::RC_OFFSET);

/// An index slot: an entry position, or one of these markers.
const EMPTY: i64 = -1;
const DUMMY: i64 = -2;

#[derive(Default)]
struct Table {
    /// `(hash, key, value)` in insertion order; `None` once deleted.
    entries: Vec<Option<(i64, i64, i64)>>,
    /// Open-addressing index (a power of two long), or empty before the
    /// first insert.
    indices: Vec<i64>,
    /// Slots of `indices` that are not `EMPTY`.
    fill: usize,
    /// Deleted entries still in `entries`.
    dead: usize,
}

impl Table {
    /// The slot holding `key`, or `Err(slot)` of the first free one.
    fn find(&self, kind: i64, key: i64, hash: i64) -> Result<usize, usize> {
        let mask = self.indices.len() - 1;
        let mut perturb = hash as u64;
        let mut i = (hash as u64 as usize) & mask;
        let mut free: Option<usize> = None;
        loop {
            match self.indices[i] {
                EMPTY => return Err(free.unwrap_or(i)),
                DUMMY => {
                    free.get_or_insert(i);
                }
                ix => {
                    let (h, k, _) = self.entries[ix as usize].expect("live entry");
                    if h == hash && (k == key || value::eq(kind, k, key)) {
                        return Ok(i);
                    }
                }
            }
            perturb >>= 5;
            i = (i
                .wrapping_mul(5)
                .wrapping_add(1)
                .wrapping_add(perturb as usize))
                & mask;
        }
    }

    /// Rebuilds the index (and compacts `entries`) for `live` entries.
    fn rebuild(&mut self, live: usize) {
        let mut size = 8;
        while size * 2 <= live * 3 {
            size *= 2;
        }
        self.entries.retain(Option::is_some);
        self.dead = 0;
        self.indices = vec![EMPTY; size];
        self.fill = self.entries.len();
        let mask = size - 1;
        for (ix, entry) in self.entries.iter().enumerate() {
            let hash = entry.expect("compacted").0;
            let mut perturb = hash as u64;
            let mut i = (hash as u64 as usize) & mask;
            while self.indices[i] != EMPTY {
                perturb >>= 5;
                i = (i
                    .wrapping_mul(5)
                    .wrapping_add(1)
                    .wrapping_add(perturb as usize))
                    & mask;
            }
            self.indices[i] = ix as i64;
        }
    }
}

impl PyDictObj {
    fn lookup(&self, key: i64, hash: i64) -> Option<usize> {
        if self.table.indices.is_empty() {
            return None;
        }
        self.table
            .find(self.key_kind, key, hash)
            .ok()
            .map(|slot| self.table.indices[slot] as usize)
    }

    /// The entry index of `key`, raising `TypeError` for an unhashable key
    /// (then `Err`).
    fn locate(&self, key: i64) -> Result<(Option<usize>, i64), ()> {
        let hash = value::hash_or_raise(self.key_kind, key).ok_or(())?;
        Ok((self.lookup(key, hash), hash))
    }

    /// Stores `key: value`, adopting both references.
    fn insert(&mut self, key: i64, val: i64) {
        let Ok((found, hash)) = self.locate(key) else {
            value::release(self.key_kind, key);
            value::release(self.value_kind, val);
            return;
        };
        if let Some(ix) = found {
            let entry = self.table.entries[ix].as_mut().expect("live entry");
            let old = std::mem::replace(&mut entry.2, val);
            value::release(self.key_kind, key);
            value::release(self.value_kind, old);
            return;
        }
        if self.table.indices.is_empty()
            || (self.table.fill + 1) * 3 >= self.table.indices.len() * 2
        {
            self.table.rebuild(self.len as usize + 1);
        }
        let slot = self
            .table
            .find(self.key_kind, key, hash)
            .expect_err("absent key");
        if self.table.indices[slot] == EMPTY {
            self.table.fill += 1;
        }
        self.table.indices[slot] = self.table.entries.len() as i64;
        self.table.entries.push(Some((hash, key, val)));
        self.len += 1;
    }

    /// Removes `key`'s entry, handing back its key and value references.
    fn remove(&mut self, key: i64) -> Option<(i64, i64)> {
        let (found, hash) = self.locate(key).ok()?;
        found?;
        let slot = self.table.find(self.key_kind, key, hash).ok()?;
        let ix = self.table.indices[slot] as usize;
        self.table.indices[slot] = DUMMY;
        let (_, k, v) = self.table.entries[ix].take().expect("live entry");
        self.table.dead += 1;
        self.len -= 1;
        if self.table.dead > 8 && self.table.dead > self.len as usize {
            self.table.rebuild(self.len as usize);
        }
        Some((k, v))
    }

    fn compact(&mut self) {
        if self.table.dead > 0 {
            self.table.rebuild(self.len as usize);
        }
    }

    /// The live entries in order.
    fn live(&self) -> impl Iterator<Item = (i64, i64)> + '_ {
        self.table
            .entries
            .iter()
            .flatten()
            .map(|(_, k, v)| (*k, *v))
    }

    /// The type-valid word returned beside a raised exception.
    fn sentinel(&self) -> i64 {
        sentinel(self.value_kind)
    }
}

/// A placeholder word of `kind` for a result that comes with a pending
/// exception.
pub(crate) fn sentinel(kind: i64) -> i64 {
    match kind {
        crate::list::LIST_KIND_INT => crate::tag_smallint(0),
        crate::list::LIST_KIND_STR => new_pystr(b"") as i64,
        _ => 0,
    }
}

fn dict<'a>(d: *mut PyDictObj) -> &'a mut PyDictObj {
    unsafe { &mut *d }
}

pub(crate) fn dict_len(d: *mut PyDictObj) -> i64 {
    dict(d).len
}

/// A fresh empty dict with keys of `key_kind` and values of `value_kind`.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_dict_new(key_kind: i64, value_kind: i64) -> *mut PyDictObj {
    crate::rc::DICT_LIVE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    Box::into_raw(Box::new(PyDictObj {
        len: 0,
        extra: std::ptr::null_mut(),
        _reserved: [0; 2],
        rc: 1,
        key_kind,
        value_kind,
        table: Table::default(),
    }))
}

/// `d[key] = value`, adopting both references.
///
/// # Safety
/// `d` must be a live dict.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_dict_set(d: *mut PyDictObj, key: i64, val: i64) {
    dict(d).insert(key, val);
}

/// `d[key]`: a new reference to the value; a missing key raises
/// `KeyError(key)` and returns a placeholder. `key` is borrowed.
///
/// # Safety
/// `d` must be a live dict.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_dict_get(d: *mut PyDictObj, key: i64) -> i64 {
    let d = dict(d);
    match d.locate(key) {
        Ok((Some(ix), _)) => {
            let v = d.table.entries[ix].expect("live entry").2;
            value::retain(d.value_kind, v);
            v
        }
        Ok((None, _)) => {
            // A `defaultdict` factory is user code that may drop the last
            // reference to `d`: keep it alive until this call is done.
            let dict_ptr: *mut PyDictObj = d;
            unsafe { pycc_rt_dict_incref(dict_ptr) };
            let out = match flavor::missing(d, key) {
                Some(Ok(value)) => value,
                Some(Err(())) => d.sentinel(),
                None => {
                    raise_key_error(d.key_kind, key);
                    d.sentinel()
                }
            };
            // SAFETY: balances the incref above; `d` is not used after it.
            unsafe { pycc_rt_dict_decref(dict_ptr) };
            out
        }
        Err(()) => d.sentinel(),
    }
}

/// `d[key] += n` on a dict of `int` values with one lookup (`pycc_mir`'s
/// `dict_add`): a missing key raises `KeyError`, or reads a `Counter`'s `0`
/// or a `defaultdict`'s new value, as `d[key]` would. `key` and `n` are
/// borrowed.
///
/// # Safety
/// `d` must be a live dict with `int` values.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_dict_add_int(d: *mut PyDictObj, key: i64, n: i64) {
    // A `defaultdict` factory run for a missing key is user code that may
    // drop the last reference to `d` or mutate it: keep `d` alive for the
    // whole operation (`insert` re-locates the key, so a mutation is fine).
    unsafe { pycc_rt_dict_incref(d) };
    unsafe { dict_add_int(dict(d), key, n) };
    // SAFETY: balances the incref above.
    unsafe { pycc_rt_dict_decref(d) };
}

/// [`pycc_rt_dict_add_int`] on a dict its caller keeps alive.
unsafe fn dict_add_int(d: &mut PyDictObj, key: i64, n: i64) {
    let found = match d.locate(key) {
        Ok((found, _)) => found,
        Err(()) => return,
    };
    let old = match found {
        Some(ix) => d.table.entries[ix].expect("live entry").2,
        None => match flavor::missing(d, key) {
            Some(Ok(value)) => {
                // `missing` handed back a new reference: an `int` word,
                // which needs no release.
                value
            }
            Some(Err(())) => return,
            None => {
                raise_key_error(d.key_kind, key);
                return;
            }
        },
    };
    let sum = crate::int_arith::int_add(old, n);
    if crate::int_encoding::inline_int_value(sum).is_none() {
        crate::int_arith::int_untag_checked(sum, 0);
        crate::pycc_rt_bigint_release(sum);
        return;
    }
    match found {
        Some(ix) => d.table.entries[ix].as_mut().expect("live entry").2 = sum,
        None => {
            value::retain(d.key_kind, key);
            d.insert(key, sum);
        }
    }
}

/// `d.get(key, default)`: a new reference to the stored value, or `default`
/// (whose reference this call adopts). `key` is borrowed.
///
/// # Safety
/// `d` must be a live dict.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_dict_get_or_default(
    d: *mut PyDictObj,
    key: i64,
    default: i64,
) -> i64 {
    let d = dict(d);
    match d.locate(key) {
        Ok((Some(ix), _)) => {
            value::release(d.value_kind, default);
            let v = d.table.entries[ix].expect("live entry").2;
            value::retain(d.value_kind, v);
            v
        }
        _ => default,
    }
}

/// `d.setdefault(key, default)`: the stored value, inserting `default`
/// first when `key` is absent. Adopts `key` and `default`; returns a new
/// reference.
///
/// # Safety
/// `d` must be a live dict.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_dict_setdefault(d: *mut PyDictObj, key: i64, default: i64) -> i64 {
    let d = dict(d);
    match d.locate(key) {
        Ok((Some(ix), _)) => {
            value::release(d.key_kind, key);
            value::release(d.value_kind, default);
            let v = d.table.entries[ix].expect("live entry").2;
            value::retain(d.value_kind, v);
            v
        }
        Ok((None, _)) => {
            value::retain(d.value_kind, default);
            d.insert(key, default);
            default
        }
        Err(()) => default,
    }
}

/// `key in d` (`key` borrowed).
///
/// # Safety
/// `d` must be a live dict.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_dict_contains(d: *mut PyDictObj, key: i64) -> i8 {
    i8::from(matches!(dict(d).locate(key), Ok((Some(_), _))))
}

/// `len(d)`.
///
/// # Safety
/// `d` must be a live dict.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_dict_len(d: *mut PyDictObj) -> i64 {
    dict(d).len
}

/// The key at position `index` of the iteration order, borrowed (`for k in
/// d`). `index` must be in range.
///
/// # Safety
/// `d` must be a live dict.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_dict_key_at(d: *mut PyDictObj, index: i64) -> i64 {
    let d = dict(d);
    d.compact();
    d.table.entries[index as usize].expect("compacted").1
}

/// `del d[key]` (`KeyError` when absent; `key` borrowed).
///
/// # Safety
/// `d` must be a live dict.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_dict_del(d: *mut PyDictObj, key: i64) {
    let d = dict(d);
    match d.remove(key) {
        Some((k, v)) => {
            value::release(d.key_kind, k);
            value::release(d.value_kind, v);
        }
        None if crate::pycc_rt_exception_active() == 0 => raise_key_error(d.key_kind, key),
        None => {}
    }
}

/// `d.pop(key)`: the removed value (its reference handed to the caller);
/// `KeyError` when absent. `key` is borrowed.
///
/// # Safety
/// `d` must be a live dict.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_dict_pop(d: *mut PyDictObj, key: i64) -> i64 {
    let d = dict(d);
    match d.remove(key) {
        Some((k, v)) => {
            value::release(d.key_kind, k);
            v
        }
        None => {
            if crate::pycc_rt_exception_active() == 0 {
                raise_key_error(d.key_kind, key);
            }
            d.sentinel()
        }
    }
}

/// `d.pop(key, default)`: the removed value, or `default` (adopted).
///
/// # Safety
/// `d` must be a live dict.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_dict_pop_default(
    d: *mut PyDictObj,
    key: i64,
    default: i64,
) -> i64 {
    let d = dict(d);
    match d.remove(key) {
        Some((k, v)) => {
            value::release(d.key_kind, k);
            value::release(d.value_kind, default);
            v
        }
        None => default,
    }
}

/// `d.clear()`.
///
/// # Safety
/// `d` must be a live dict.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_dict_clear(d: *mut PyDictObj) {
    let d = dict(d);
    let table = std::mem::take(&mut d.table);
    d.len = 0;
    for (_, k, v) in table.entries.into_iter().flatten() {
        value::release(d.key_kind, k);
        value::release(d.value_kind, v);
    }
}

/// A shallow copy of `d` (`d.copy()`, `dict(d)`).
///
/// # Safety
/// `d` must be a live dict.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_dict_copy(d: *mut PyDictObj) -> *mut PyDictObj {
    let d = dict(d);
    let copy = pycc_rt_dict_new(d.key_kind, d.value_kind);
    for (k, v) in d.live() {
        value::retain(d.key_kind, k);
        value::retain(d.value_kind, v);
        dict(copy).insert(k, v);
    }
    copy
}

/// `d.update(other)`.
///
/// # Safety
/// `d` and `other` must be live dicts of the same kinds.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_dict_update(d: *mut PyDictObj, other: *mut PyDictObj) {
    if flavor::flavor(dict(d)) == Some(flavor::Flavor::Counter) {
        // `Counter.update(mapping)` adds the counts.
        unsafe { flavor::pycc_rt_counter_add_dict(d, other, crate::tag_smallint(1)) };
        return;
    }
    unsafe { update_plain(d, other) };
}

/// `dict.update(other)`: `other`'s values replace `d`'s.
///
/// # Safety
/// `d` and `other` must be live dicts of the same kinds.
pub(crate) unsafe fn update_plain(d: *mut PyDictObj, other: *mut PyDictObj) {
    let pairs: Vec<(i64, i64)> = dict(other).live().collect();
    let d = dict(d);
    for (k, v) in pairs {
        value::retain(d.key_kind, k);
        value::retain(d.value_kind, v);
        d.insert(k, v);
    }
}

fn list_of(kind: i64, words: Vec<i64>) -> *mut PyListObj {
    for word in &words {
        value::retain(kind, *word);
    }
    crate::list::new_raw_words(kind, words)
}

/// Raises `RuntimeError: dictionary changed size during iteration` when
/// `len` differs from `initial` (and nothing is pending yet).
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_dict_check_not_resized(len: i64, initial: i64) {
    if len != initial && crate::pycc_rt_exception_active() == 0 {
        crate::raise_builtin(
            crate::exception::EXCEPTION_TYPE_RUNTIME_ERROR,
            "RuntimeError",
            "dictionary changed size during iteration",
        );
    }
}

/// `list(d)` / `list(d.keys())` as a new list.
///
/// # Safety
/// `d` must be a live dict.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_dict_keys(d: *mut PyDictObj) -> *mut PyListObj {
    let d = dict(d);
    list_of(d.key_kind, d.live().map(|(k, _)| k).collect())
}

/// `list(d.values())` as a new list.
///
/// # Safety
/// `d` must be a live dict.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_dict_values(d: *mut PyDictObj) -> *mut PyListObj {
    let d = dict(d);
    list_of(d.value_kind, d.live().map(|(_, v)| v).collect())
}

/// `list(d.items())` as a new list of `(key, value)` tuples.
///
/// # Safety
/// `d` must be a live dict.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_dict_items(d: *mut PyDictObj) -> *mut PyListObj {
    let d = dict(d);
    let kinds: [u8; 2] = [d.key_kind as u8, d.value_kind as u8];
    let tuples: Vec<i64> = d
        .live()
        .map(|(k, v)| {
            value::retain(d.key_kind, k);
            value::retain(d.value_kind, v);
            crate::tuple::tuple_from(vec![k, v], &kinds) as i64
        })
        .collect();
    crate::list::new_raw_words(LIST_KIND_TUPLE, tuples)
}

/// `d.popitem()`: the last inserted `(key, value)` as a new tuple;
/// `KeyError` when empty.
///
/// # Safety
/// `d` must be a live dict.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_dict_popitem(d: *mut PyDictObj) -> *mut PyListObj {
    let d = dict(d);
    d.compact();
    let kinds: [u8; 2] = [d.key_kind as u8, d.value_kind as u8];
    let Some(Some((_, k, _))) = d.table.entries.last().copied() else {
        crate::raise_builtin(
            crate::exception::EXCEPTION_TYPE_KEY_ERROR,
            "KeyError",
            "popitem(): dictionary is empty",
        );
        return crate::tuple::tuple_from(
            vec![sentinel(d.key_kind), sentinel(d.value_kind)],
            &kinds,
        );
    };
    let (k, v) = d.remove(k).expect("present");
    crate::tuple::tuple_from(vec![k, v], &kinds)
}

/// A dict built from a list of `(key, value)` tuples (`dict(pairs)`).
///
/// # Safety
/// `pairs` must be a live list of two-element tuple objects.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_dict_from_pairs(
    pairs: *mut PyListObj,
    key_kind: i64,
    value_kind: i64,
) -> *mut PyDictObj {
    let d = pycc_rt_dict_new(key_kind, value_kind);
    for (_, pair) in unsafe { &*pairs }.kinded_items() {
        let items = value::sequence_items(pair);
        let (k, v) = (items[0].1, items[1].1);
        value::retain(key_kind, k);
        value::retain(value_kind, v);
        dict(d).insert(k, v);
    }
    d
}

/// `a == b` for two dicts of the same kinds.
pub(crate) fn dicts_equal(a: *mut PyDictObj, b: *mut PyDictObj) -> bool {
    let (a, b) = (dict(a), dict(b));
    if let Some(equal) = flavor::counters_equal(a, b) {
        return equal;
    }
    a.len == b.len
        && a.live().all(|(k, v)| match b.locate(k) {
            Ok((Some(ix), _)) => value::eq(a.value_kind, v, b.table.entries[ix].expect("live").2),
            _ => false,
        })
}

/// Appends `repr(d)` to `out`.
pub(crate) fn dict_repr_into(d: *mut PyDictObj, out: &mut String) -> bool {
    if let Some(rendered) = flavor::repr_into(d, out) {
        return rendered;
    }
    plain_repr_into(d, out)
}

/// Appends `repr(d)` of the dict's contents (`{k: v, ...}`) to `out`.
pub(crate) fn plain_repr_into(d: *mut PyDictObj, out: &mut String) -> bool {
    let d = dict(d);
    out.push('{');
    for (i, (k, v)) in d.live().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        if !value::repr(d.key_kind, k, out) {
            return false;
        }
        out.push_str(": ");
        if !value::repr(d.value_kind, v, out) {
            return false;
        }
    }
    out.push('}');
    true
}

/// `str(d)` / `repr(d)` as a new `str`.
///
/// # Safety
/// `d` must be a live dict.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_dict_repr(d: *mut PyDictObj) -> *mut PyStrObj {
    let mut out = String::new();
    if !dict_repr_into(d, &mut out) {
        return new_pystr(b"");
    }
    new_pystr(out.as_bytes())
}

/// Takes one more reference to `d`.
///
/// # Safety
/// `d` must be null or a live dict.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_dict_incref(d: *mut PyDictObj) {
    if !d.is_null() {
        dict(d).rc += 1;
    }
}

/// Drops one reference to `d`, freeing it at zero: the dict owns one
/// reference to each key and value, released here.
///
/// # Safety
/// `d` must be null or a live dict not used again after its last release.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_dict_decref(d: *mut PyDictObj) {
    crate::rc::release_word(b'd', d as i64);
}

/// Frees a dict whose count reached zero, its keys' and values' releases
/// going onto `dead` (`crate::rc`'s worklist) rather than recursing, so a
/// long chain of nested dicts is freed in constant stack.
pub(crate) fn free_dict(d: *mut PyDictObj, dead: &mut crate::rc::Worklist) {
    let mut owned = unsafe { Box::from_raw(d) };
    flavor::free_extra(&mut owned);
    crate::rc::DICT_LIVE.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
    let (key_kind, value_kind) = (
        value::rc_kind(owned.key_kind),
        value::rc_kind(owned.value_kind),
    );
    for (_, k, v) in owned.table.entries.into_iter().flatten() {
        crate::rc::release_word_into(key_kind, k, dead);
        crate::rc::release_word_into(value_kind, v, dead);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::list::{LIST_KIND_INT, LIST_KIND_STR};

    fn int(v: i64) -> i64 {
        crate::tag_smallint(v)
    }

    fn render(d: *mut PyDictObj) -> String {
        let mut out = String::new();
        assert!(dict_repr_into(d, &mut out));
        out
    }

    #[test]
    fn int_keys_insert_update_delete_in_cpython_order() {
        let d = pycc_rt_dict_new(LIST_KIND_INT, LIST_KIND_INT);
        unsafe {
            for i in 0..100 {
                pycc_rt_dict_set(d, int(i * 7 % 13), int(i));
            }
            assert_eq!(pycc_rt_dict_len(d), 13);
            assert_eq!(
                crate::inline_int_value(pycc_rt_dict_get(d, int(0))),
                Some(91)
            );
            for i in 0..10 {
                pycc_rt_dict_del(d, int(i));
            }
            assert_eq!(render(d), "{10: 98, 11: 87, 12: 89}");
            pycc_rt_dict_set(d, int(3), int(-1));
            assert_eq!(render(d), "{10: 98, 11: 87, 12: 89, 3: -1}");
            assert_eq!(pycc_rt_dict_contains(d, int(11)), 1);
            assert_eq!(pycc_rt_dict_contains(d, int(1)), 0);
            assert_eq!(crate::inline_int_value(pycc_rt_dict_key_at(d, 3)), Some(3));
        }
    }

    #[test]
    fn missing_key_raises_key_error_and_large_tables_work() {
        let d = pycc_rt_dict_new(LIST_KIND_STR, LIST_KIND_INT);
        unsafe {
            for i in 0..5000 {
                pycc_rt_dict_set(d, new_pystr(format!("k{i}").as_bytes()) as i64, int(i));
            }
            let probe = new_pystr(b"k4999") as i64;
            assert_eq!(
                crate::inline_int_value(pycc_rt_dict_get(d, probe)),
                Some(4999)
            );
            let missing = new_pystr(b"zz") as i64;
            let _ = pycc_rt_dict_get(d, missing);
            assert_eq!(crate::pycc_rt_exception_active(), 1);
            crate::pycc_rt_exception_clear();
        }
    }
}
