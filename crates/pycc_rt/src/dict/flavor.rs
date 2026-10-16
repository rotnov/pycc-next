//! `collections.Counter` and `collections.defaultdict`: dict objects with a
//! flavor.
//!
//! A flavored dict is a `PyDictObj` whose [`PyDictObj::extra`] points at an
//! [`Extra`], so every dict operation serves it unchanged. What differs is
//! here: a missing key of a `Counter` reads as `0` (without inserting it),
//! of a `defaultdict` as a new value that is stored first -- a fresh copy of
//! a prototype value when the factory is a type or a lambda returning a
//! literal (`pycc_ast::desugar::collections`), else the result of calling
//! the factory function; the `repr` (`Counter({...})` in `most_common`
//! order, `defaultdict(<class 'int'>, {...})`); a `Counter`'s own methods and
//! operators; and `Counter` equality, which treats a missing key as zero.

use super::{PyDictObj, dict};
use crate::closure::PyClosureObj;
use crate::list::{LIST_KIND_BOOL, LIST_KIND_DICT, LIST_KIND_FLOAT, LIST_KIND_INT, LIST_KIND_LIST};
use crate::list::{LIST_KIND_SET, LIST_KIND_TUPLE, PyListObj};
use crate::value;
use crate::{PyStrObj, raise_builtin};

/// What a flavored dict is.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Flavor {
    Counter,
    DefaultDict,
}

/// How a `defaultdict` makes the value of a missing key.
pub(crate) enum Factory {
    /// `defaultdict()`: none; a missing key raises `KeyError`.
    None,
    /// A fresh copy of this owned word of the dict's value kind.
    Proto(i64),
    /// Calls this function value (one reference held).
    Call(*mut PyClosureObj),
}

/// The flavor of a dict and what it needs.
pub(crate) struct Extra {
    pub(crate) flavor: Flavor,
    pub(crate) factory: Factory,
    /// A prototype factory's `repr` (`<class 'int'>`).
    pub(crate) name: String,
}

/// The flavor of `d`, if any.
pub(crate) fn flavor(d: &PyDictObj) -> Option<Flavor> {
    // SAFETY: a non-null `extra` is owned by the dict.
    (!d.extra.is_null()).then(|| unsafe { (*d.extra).flavor })
}

fn set_extra(d: &mut PyDictObj, extra: Extra) {
    debug_assert!(d.extra.is_null());
    d.extra = Box::into_raw(Box::new(extra));
}

/// Releases a dying dict's [`Extra`].
pub(crate) fn free_extra(d: &mut PyDictObj) {
    if d.extra.is_null() {
        return;
    }
    // SAFETY: owned by the dict, freed once.
    let extra = unsafe { Box::from_raw(std::mem::replace(&mut d.extra, std::ptr::null_mut())) };
    match extra.factory {
        Factory::Proto(word) => value::release(d.value_kind, word),
        Factory::Call(obj) => crate::closure::pycc_rt_closure_decref(obj),
        Factory::None => {}
    }
}

/// A copy of `source`'s [`Extra`] for a copy of the dict (taking new
/// references).
pub(crate) fn copy_extra(source: &PyDictObj, copy: &mut PyDictObj) {
    if source.extra.is_null() {
        return;
    }
    // SAFETY: owned by `source`.
    let extra = unsafe { &*source.extra };
    let factory = match extra.factory {
        Factory::Proto(word) => {
            value::retain(source.value_kind, word);
            Factory::Proto(word)
        }
        Factory::Call(obj) => {
            crate::closure::pycc_rt_closure_incref(obj);
            Factory::Call(obj)
        }
        Factory::None => Factory::None,
    };
    set_extra(
        copy,
        Extra {
            flavor: extra.flavor,
            factory,
            name: extra.name.clone(),
        },
    );
}

/// A fresh value like the prototype `word` of `kind`: an immutable value is
/// shared, a container is copied (shallowly, with its own flavor), as
/// calling the factory again would build it.
fn fresh_copy(kind: i64, word: i64) -> i64 {
    match kind {
        LIST_KIND_LIST => {
            // SAFETY: a list prototype is a live list.
            let copy =
                unsafe { crate::list::deque::pycc_rt_list_copy_method(word as *mut PyListObj) };
            copy as i64
        }
        LIST_KIND_DICT => {
            // SAFETY: a dict prototype is a live dict.
            unsafe { pycc_rt_dict_copy_method(word as *mut PyDictObj) as i64 }
        }
        LIST_KIND_SET => {
            // SAFETY: a set prototype is a live set.
            unsafe { crate::set::pycc_rt_set_copy(word as *mut crate::set::PySetObj) as i64 }
        }
        _ => {
            crate::itertools::retain(kind, word);
            word
        }
    }
}

/// Calls the zero-argument function value `obj`, whose result is a value of
/// container kind `kind`, and returns it as a word; `None` with an
/// exception pending.
fn call_factory(obj: *mut PyClosureObj, kind: i64) -> Option<i64> {
    let call = crate::closure::call_ptr(obj);
    // SAFETY: a function value's `call` takes the object first, then its
    // arguments (none here), and returns its declared type: a `float` in a
    // floating-point register, a `bool` as a byte, everything else as one
    // word (an encoded `int` or a pointer).
    let word = unsafe {
        match kind {
            LIST_KIND_FLOAT => {
                let f: unsafe extern "C" fn(*mut PyClosureObj) -> f64 = std::mem::transmute(call);
                f(obj).to_bits() as i64
            }
            LIST_KIND_BOOL => {
                let f: unsafe extern "C" fn(*mut PyClosureObj) -> u8 = std::mem::transmute(call);
                i64::from(f(obj) & 1)
            }
            _ => {
                let f: unsafe extern "C" fn(*mut PyClosureObj) -> i64 = std::mem::transmute(call);
                f(obj)
            }
        }
    };
    if crate::pycc_rt_exception_active() != 0 {
        return None;
    }
    Some(word)
}

/// The value `d[key]` reads for a missing `key` of a flavored dict (a new
/// reference), storing it first for a `defaultdict`; `None` when `d` is not
/// flavored or has no factory (the caller raises `KeyError`), or the factory
/// raised.
pub(crate) fn missing(d: &mut PyDictObj, key: i64) -> Option<Result<i64, ()>> {
    if d.extra.is_null() {
        return None;
    }
    // SAFETY: owned by the dict.
    let extra = unsafe { &*d.extra };
    let value = match (extra.flavor, &extra.factory) {
        (Flavor::Counter, _) => return Some(Ok(crate::tag_smallint(0))),
        (Flavor::DefaultDict, Factory::None) => return None,
        (Flavor::DefaultDict, Factory::Proto(proto)) => fresh_copy(d.value_kind, *proto),
        (Flavor::DefaultDict, Factory::Call(obj)) => {
            // The factory is user code: it may drop the last reference to
            // this dict (rebinding the global that held it) or rebind
            // nothing but mutate it. Keep the dict and the factory alive
            // across the call; `extra` is not used after it.
            let obj = *obj;
            let dict_ptr: *mut PyDictObj = d;
            // SAFETY: `d` is live here; the matching decref is below.
            unsafe { super::pycc_rt_dict_incref(dict_ptr) };
            crate::closure::pycc_rt_closure_incref(obj);
            let result = call_factory(obj, d.value_kind);
            crate::closure::pycc_rt_closure_decref(obj);
            let outcome = match result {
                Some(value) => {
                    value::retain(d.key_kind, key);
                    value::retain(d.value_kind, value);
                    // A factory that stored `key` itself is overwritten, as
                    // CPython's `__missing__` does.
                    d.insert(key, value);
                    Some(Ok(value))
                }
                None => Some(Err(())),
            };
            // SAFETY: balances the incref above; `d` is not used after it.
            unsafe { super::pycc_rt_dict_decref(dict_ptr) };
            return outcome;
        }
    };
    value::retain(d.key_kind, key);
    value::retain(d.value_kind, value);
    d.insert(key, value);
    Some(Ok(value))
}

/// `Counter()` / the dict `d` marked a `Counter`, returned as a new
/// reference.
///
/// # Safety
/// `d` must be a live dict.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_counter_new(d: *mut PyDictObj) -> *mut PyDictObj {
    unsafe { super::pycc_rt_dict_incref(d) };
    set_extra(
        dict(d),
        Extra {
            flavor: Flavor::Counter,
            factory: Factory::None,
            name: String::new(),
        },
    );
    d
}

fn defaultdict(d: *mut PyDictObj, factory: Factory, name: String) -> *mut PyDictObj {
    unsafe { super::pycc_rt_dict_incref(d) };
    set_extra(
        dict(d),
        Extra {
            flavor: Flavor::DefaultDict,
            factory,
            name,
        },
    );
    d
}

fn text(s: *mut PyStrObj) -> String {
    // SAFETY: a live `str`.
    String::from_utf8_lossy(unsafe { &*s }.bytes()).into_owned()
}

/// `defaultdict(factory)` whose missing keys get copies of the borrowed
/// prototype word `proto` (`name` is the factory's `repr`); marks `d` and
/// returns it as a new reference.
///
/// # Safety
/// `d` must be a live dict, `name` a live `str`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_defaultdict_new(
    d: *mut PyDictObj,
    proto: i64,
    name: *mut PyStrObj,
) -> *mut PyDictObj {
    value::retain(dict(d).value_kind, proto);
    defaultdict(d, Factory::Proto(proto), text(name))
}

/// `defaultdict(f)` calling the function value `f` for each missing key.
///
/// # Safety
/// `d` must be a live dict, `f` a live function value.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_defaultdict_call_new(
    d: *mut PyDictObj,
    f: *mut PyClosureObj,
    _name: *mut PyStrObj,
) -> *mut PyDictObj {
    crate::closure::pycc_rt_closure_incref(f);
    defaultdict(d, Factory::Call(f), String::new())
}

/// `defaultdict()`: no factory.
///
/// # Safety
/// `d` must be a live dict.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_defaultdict_none_new(d: *mut PyDictObj) -> *mut PyDictObj {
    defaultdict(d, Factory::None, "None".to_string())
}

/// `d.copy()`: a shallow copy of the same flavor (`dict(d)` is a plain
/// dict).
///
/// # Safety
/// `d` must be a live dict.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_dict_copy_method(d: *mut PyDictObj) -> *mut PyDictObj {
    let copy = unsafe { super::pycc_rt_dict_copy(d) };
    copy_extra(dict(d), dict(copy));
    copy
}

/// The live `(key, count)` pairs of a counter in `most_common` order:
/// descending count, ties in insertion order.
pub(crate) fn most_common_pairs(d: &PyDictObj) -> Vec<(i64, i64)> {
    let mut pairs: Vec<(i64, i64)> = d.live().collect();
    pairs.sort_by(|a, b| crate::int_arith::int_cmp(b.1, a.1).cmp(&0));
    pairs
}

/// Appends `repr(d)` of a flavored dict to `out`; `None` for a plain dict.
pub(crate) fn repr_into(d: *mut PyDictObj, out: &mut String) -> Option<bool> {
    let d_ref = dict(d);
    let flavor = flavor(d_ref)?;
    if flavor == Flavor::Counter {
        if d_ref.len == 0 {
            out.push_str("Counter()");
            return Some(true);
        }
        out.push_str("Counter({");
        for (i, (k, v)) in most_common_pairs(d_ref).into_iter().enumerate() {
            if i > 0 {
                out.push_str(", ");
            }
            if !value::repr(d_ref.key_kind, k, out) {
                return Some(false);
            }
            out.push_str(": ");
            if !value::repr(d_ref.value_kind, v, out) {
                return Some(false);
            }
        }
        out.push_str("})");
        return Some(true);
    }
    // SAFETY: flavored.
    let extra = unsafe { &*d_ref.extra };
    out.push_str("defaultdict(");
    match extra.factory {
        Factory::Call(obj) => out.push_str(&crate::closure::closure_repr(obj)),
        _ => out.push_str(&extra.name),
    }
    out.push_str(", ");
    if !super::plain_repr_into(d, out) {
        return Some(false);
    }
    out.push(')');
    Some(true)
}

/// `a == b` when either is a `Counter` (CPython 3.10+): every key's count,
/// a missing key counting zero.
pub(crate) fn counters_equal(a: &PyDictObj, b: &PyDictObj) -> Option<bool> {
    if flavor(a) != Some(Flavor::Counter) || flavor(b) != Some(Flavor::Counter) {
        return None;
    }
    let count = |d: &PyDictObj, k: i64| match d.locate(k) {
        Ok((Some(ix), _)) => d.table.entries[ix].expect("live").2,
        _ => crate::tag_smallint(0),
    };
    let same = |x: &PyDictObj, y: &PyDictObj| {
        x.live()
            .all(|(k, v)| crate::int_arith::int_cmp(v, count(y, k)) == 0)
    };
    Some(same(a, b) && same(b, a))
}

/// Raises `AttributeError` unless `d` is a `Counter`.
fn require_counter(d: &PyDictObj, method: &str) -> bool {
    if flavor(d) == Some(Flavor::Counter) {
        return true;
    }
    let owner = if flavor(d) == Some(Flavor::DefaultDict) {
        "collections.defaultdict"
    } else {
        "dict"
    };
    raise_builtin(
        crate::EXCEPTION_TYPE_ATTRIBUTE_ERROR,
        "AttributeError",
        &format!("'{owner}' object has no attribute '{method}'"),
    );
    false
}

/// Adds `n` (an owned encoded `int`) to `key`'s count (`key` borrowed).
fn add_count(d: &mut PyDictObj, key: i64, n: i64) {
    let current = match d.locate(key) {
        Ok((Some(ix), _)) => d.table.entries[ix].expect("live").2,
        Ok((None, _)) => crate::tag_smallint(0),
        Err(()) => return,
    };
    let total = crate::int_arith::int_add(current, n);
    crate::itertools::release(LIST_KIND_INT, n);
    value::retain(d.key_kind, key);
    d.insert(key, total);
}

/// `c.most_common()` (`n < 0`) or `c.most_common(n)`: a new list of
/// `(key, count)` tuples.
///
/// # Safety
/// `d` must be a live dict.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_counter_most_common(d: *mut PyDictObj, n: i64) -> *mut PyListObj {
    let d = dict(d);
    let kinds: [u8; 2] = [d.key_kind as u8, d.value_kind as u8];
    if !require_counter(d, "most_common") {
        return crate::list::new_raw_words(LIST_KIND_TUPLE, Vec::new());
    }
    let limit = match crate::int_encoding::inline_int_value(n) {
        Some(n) if n < 0 => 0,
        Some(n) => n as usize,
        None => usize::MAX,
    };
    let tuples: Vec<i64> = most_common_pairs(d)
        .into_iter()
        .take(limit)
        .map(|(k, v)| {
            value::retain(d.key_kind, k);
            crate::itertools::retain(LIST_KIND_INT, v);
            crate::tuple::tuple_from(vec![k, v], &kinds) as i64
        })
        .collect();
    crate::list::new_raw_words(LIST_KIND_TUPLE, tuples)
}

/// `c.most_common()` with no argument.
///
/// # Safety
/// `d` must be a live dict.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_counter_most_common_all(d: *mut PyDictObj) -> *mut PyListObj {
    unsafe { pycc_rt_counter_most_common(d, crate::tag_smallint(i64::MAX >> 2)) }
}

/// `c.total()`: the sum of the counts.
///
/// # Safety
/// `d` must be a live dict.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_counter_total(d: *mut PyDictObj) -> i64 {
    let d = dict(d);
    if !require_counter(d, "total") {
        return crate::tag_smallint(0);
    }
    let mut total = crate::tag_smallint(0);
    for (_, v) in d.live() {
        let next = crate::int_arith::int_add(total, v);
        crate::itertools::release(LIST_KIND_INT, total);
        total = next;
    }
    total
}

/// `c.update(xs)` / `c.subtract(xs)` (`sign` `1`/`-1`) over the elements of
/// the list `xs`.
///
/// # Safety
/// `d` must be a live dict, `xs` a live list of its key kind.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_counter_add_list(
    d: *mut PyDictObj,
    xs: *mut PyListObj,
    sign: i64,
) {
    let d = dict(d);
    let method = if crate::int_encoding::inline_int_value(sign) == Some(1) {
        "update"
    } else {
        "subtract"
    };
    if method == "subtract" && !require_counter(d, method) {
        return;
    }
    if flavor(d) != Some(Flavor::Counter) {
        raise_builtin(
            crate::EXCEPTION_TYPE_TYPE_ERROR,
            "TypeError",
            "cannot convert dictionary update sequence element #0 to a sequence",
        );
        return;
    }
    let xs = unsafe { &*xs };
    for i in 0..xs.len() as usize {
        add_count(d, xs.word_at(i), sign);
    }
}

/// `c.update(other)` / `c.subtract(other)` (`sign` `1`/`-1`) with a mapping:
/// a counter adds (or subtracts) the counts, a plain dict's `update`
/// replaces values.
///
/// # Safety
/// Both must be live dicts of the same kinds.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_counter_add_dict(
    d: *mut PyDictObj,
    other: *mut PyDictObj,
    sign: i64,
) {
    let negate = crate::int_encoding::inline_int_value(sign) != Some(1);
    if flavor(dict(d)) != Some(Flavor::Counter) {
        if negate {
            require_counter(dict(d), "subtract");
        } else {
            unsafe { super::update_plain(d, other) };
        }
        return;
    }
    let pairs: Vec<(i64, i64)> = dict(other).live().collect();
    let d = dict(d);
    for (k, v) in pairs {
        let n = if negate {
            crate::int_arith::int_sub(crate::tag_smallint(0), v)
        } else {
            crate::itertools::retain(LIST_KIND_INT, v);
            v
        };
        add_count(d, k, n);
    }
}

/// `c.update(xs)` over a list.
///
/// # Safety
/// As [`pycc_rt_counter_add_list`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_counter_update_list(d: *mut PyDictObj, xs: *mut PyListObj) {
    unsafe { pycc_rt_counter_add_list(d, xs, crate::tag_smallint(1)) };
}

/// `c.subtract(xs)` over a list.
///
/// # Safety
/// As [`pycc_rt_counter_add_list`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_counter_subtract_list(d: *mut PyDictObj, xs: *mut PyListObj) {
    unsafe { pycc_rt_counter_add_list(d, xs, crate::tag_smallint(-1)) };
}

/// `c.update(s)` over the characters of a `str`.
///
/// # Safety
/// `d` must be a live dict with `str` keys, `s` a live `str`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_counter_update_str(d: *mut PyDictObj, s: *mut PyStrObj) {
    let chars = crate::str_ops::pycc_rt_str_to_list(s);
    unsafe { pycc_rt_counter_add_list(d, chars, crate::tag_smallint(1)) };
    value::release(LIST_KIND_LIST, chars as i64);
}

/// `c.subtract(mapping)`.
///
/// # Safety
/// As [`pycc_rt_counter_add_dict`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_counter_subtract_dict(d: *mut PyDictObj, other: *mut PyDictObj) {
    unsafe { pycc_rt_counter_add_dict(d, other, crate::tag_smallint(-1)) };
}

/// `Counter(xs)` over the list `xs`: a new counter of its elements.
///
/// # Safety
/// `xs` must be a live list.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_counter_of_list(xs: *mut PyListObj) -> *mut PyDictObj {
    let kind = unsafe { &*xs }.kind();
    let d = super::pycc_rt_dict_new(kind, LIST_KIND_INT);
    set_extra(
        dict(d),
        Extra {
            flavor: Flavor::Counter,
            factory: Factory::None,
            name: String::new(),
        },
    );
    unsafe { pycc_rt_counter_add_list(d, xs, crate::tag_smallint(1)) };
    d
}

/// `Counter(mapping)`: a new counter with `mapping`'s counts.
///
/// # Safety
/// `m` must be a live dict with `int` values.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_counter_of_dict(m: *mut PyDictObj) -> *mut PyDictObj {
    let copy = unsafe { super::pycc_rt_dict_copy(m) };
    set_extra(
        dict(copy),
        Extra {
            flavor: Flavor::Counter,
            factory: Factory::None,
            name: String::new(),
        },
    );
    copy
}

/// `c.elements()`: each key repeated by its count (non-positive counts
/// skipped), as a new list in insertion order.
///
/// # Safety
/// `d` must be a live dict.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_counter_elements(d: *mut PyDictObj) -> *mut PyListObj {
    let d = dict(d);
    if !require_counter(d, "elements") {
        return crate::list::new_raw_words(d.key_kind, Vec::new());
    }
    let mut words = Vec::new();
    for (k, v) in d.live() {
        let n = crate::int_encoding::inline_int_value(v).unwrap_or(0);
        for _ in 0..n.max(0) {
            value::retain(d.key_kind, k);
            words.push(k);
        }
    }
    crate::list::new_raw_words(d.key_kind, words)
}

/// `a | b`, `a & b`, `a - b`, `a + b` (`op` `0`..`3`) between two dicts of
/// one key type and `int` values: a counter's multiset operation when both
/// are counters (only positive counts kept), the dict union for `|` of two
/// plain dicts, else CPython's `TypeError`.
///
/// # Safety
/// Both must be live dicts of the same kinds.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_counter_binop(
    a: *mut PyDictObj,
    b: *mut PyDictObj,
    op: i64,
) -> *mut PyDictObj {
    let op = crate::int_encoding::inline_int_value(op).unwrap_or(0);
    let (fa, fb) = (flavor(dict(a)), flavor(dict(b)));
    let counters = fa == Some(Flavor::Counter) && fb == Some(Flavor::Counter);
    if !counters {
        if op == 0 {
            // `dict | dict`: a copy of `a` updated with `b`, `a`'s flavor.
            let copy = unsafe { pycc_rt_dict_copy_method(a) };
            unsafe { super::update_plain(copy, b) };
            return copy;
        }
        let symbol = ["|", "&", "-", "+"][op as usize];
        let name = |f: Option<Flavor>| match f {
            Some(Flavor::Counter) => "Counter",
            Some(Flavor::DefaultDict) => "collections.defaultdict",
            None => "dict",
        };
        raise_builtin(
            crate::EXCEPTION_TYPE_TYPE_ERROR,
            "TypeError",
            &format!(
                "unsupported operand type(s) for {symbol}: '{}' and '{}'",
                name(fa),
                name(fb)
            ),
        );
        return super::pycc_rt_dict_new(dict(a).key_kind, dict(a).value_kind);
    }
    let (a, b) = (dict(a), dict(b));
    let result = super::pycc_rt_dict_new(a.key_kind, a.value_kind);
    set_extra(
        dict(result),
        Extra {
            flavor: Flavor::Counter,
            factory: Factory::None,
            name: String::new(),
        },
    );
    let count = |d: &PyDictObj, k: i64| match d.locate(k) {
        Ok((Some(ix), _)) => Some(d.table.entries[ix].expect("live").2),
        _ => None,
    };
    let zero = crate::tag_smallint(0);
    let positive = |v: i64| crate::int_arith::int_cmp(v, zero) > 0;
    let mut put = |k: i64, v: i64| {
        if positive(v) {
            value::retain(a.key_kind, k);
            dict(result).insert(k, v);
        } else {
            crate::itertools::release(LIST_KIND_INT, v);
        }
    };
    for (k, va) in a.live() {
        let vb = count(b, k).unwrap_or(zero);
        let v = match op {
            0 => {
                let take = if crate::int_arith::int_cmp(vb, va) > 0 {
                    vb
                } else {
                    va
                };
                crate::itertools::retain(LIST_KIND_INT, take);
                take
            }
            1 => {
                let take = if crate::int_arith::int_cmp(vb, va) < 0 {
                    vb
                } else {
                    va
                };
                crate::itertools::retain(LIST_KIND_INT, take);
                take
            }
            2 => crate::int_arith::int_sub(va, vb),
            _ => crate::int_arith::int_add(va, vb),
        };
        put(k, v);
    }
    for (k, vb) in b.live() {
        if count(a, k).is_some() {
            continue;
        }
        let v = match op {
            0 | 3 => {
                crate::itertools::retain(LIST_KIND_INT, vb);
                vb
            }
            2 => crate::int_arith::int_sub(zero, vb),
            _ => continue,
        };
        put(k, v);
    }
    result
}
