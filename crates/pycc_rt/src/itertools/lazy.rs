//! `count`, `islice`, `chain`, `chain.from_iterable`, `accumulate`,
//! `pairwise` and `groupby`: iterators over other iterators, each value
//! computed when it is asked for, as in CPython.

use super::{Desc, Input, Item, Native, flag, release, retain, small_int, spawn, spawn_empty};
use crate::generator::PyGenObj;
use crate::list::{LIST_KIND_FLOAT, LIST_KIND_INT, LIST_KIND_STR, PyListObj};

// ------------------------------------------------------------------ count

struct CountInt {
    next: i64,
    step: i64,
}

impl Native for CountInt {
    fn next(&mut self) -> Option<Item> {
        let value = self.next;
        self.next = crate::int_arith::int_add(value, self.step);
        Some(Item::Word(value))
    }
}

impl Drop for CountInt {
    fn drop(&mut self) {
        release(LIST_KIND_INT, self.next);
        release(LIST_KIND_INT, self.step);
    }
}

/// `itertools.count(start, step)` of `int`s.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_it_count_int(start: i64, step: i64, out: i64) -> *mut PyGenObj {
    retain(LIST_KIND_INT, start);
    retain(LIST_KIND_INT, step);
    spawn(Box::new(CountInt { next: start, step }), out, b"count\0")
}

struct CountFloat {
    next: f64,
    step: f64,
}

impl Native for CountFloat {
    fn next(&mut self) -> Option<Item> {
        let value = self.next;
        // CPython adds the step to the running value each time.
        self.next += self.step;
        Some(Item::Word(value.to_bits() as i64))
    }
}

/// `itertools.count(start, step)` of `float`s.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_it_count_float(start: f64, step: f64, out: i64) -> *mut PyGenObj {
    spawn(Box::new(CountFloat { next: start, step }), out, b"count\0")
}

// ------------------------------------------------------------------ range

struct Range {
    next: i64,
    stop: i64,
    step: i64,
}

impl Native for Range {
    fn next(&mut self) -> Option<Item> {
        let more = if self.step > 0 {
            self.next < self.stop
        } else {
            self.next > self.stop
        };
        if !more {
            return None;
        }
        let value = self.next;
        self.next = value.saturating_add(self.step);
        if self.next == value {
            // Saturated: this was the last value.
            self.stop = value;
        }
        Some(Item::Word(crate::int_encoding::tag_smallint(value)))
    }
}

/// `range(start, stop, step)` as an iterable argument of a native iterator
/// (CPython iterates it lazily, so the rewrite does not build a list).
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_it_range(start: i64, stop: i64, step: i64, out: i64) -> *mut PyGenObj {
    const NAME: &[u8] = b"range_iterator\0";
    const LIMIT: &str = "Python int too large to convert to C ssize_t";
    let bound = |word: i64| match crate::int_encoding::inline_int_value(word) {
        Some(v) => Some(v),
        None => {
            crate::raise_builtin(crate::EXCEPTION_TYPE_OVERFLOW_ERROR, "OverflowError", LIMIT);
            None
        }
    };
    let (Some(start), Some(stop), Some(step)) = (bound(start), bound(stop), bound(step)) else {
        return spawn_empty(out, NAME);
    };
    if step == 0 {
        crate::raise_builtin(
            crate::EXCEPTION_TYPE_VALUE_ERROR,
            "ValueError",
            "range() arg 3 must not be zero",
        );
        return spawn_empty(out, NAME);
    }
    spawn(
        Box::new(Range {
            next: start,
            stop,
            step,
        }),
        out,
        NAME,
    )
}

// ----------------------------------------------------------------- islice

struct Islice {
    input: Input,
    /// Index of the next value to yield, and the stop (`None`: no stop).
    next: usize,
    stop: Option<usize>,
    step: usize,
    /// Values taken from the input so far.
    count: usize,
}

impl Native for Islice {
    fn next(&mut self) -> Option<Item> {
        if self.stop.is_some_and(|stop| self.next >= stop) {
            return None;
        }
        while self.count < self.next {
            let skipped = self.input.next()?;
            skipped.release(self.input.desc());
            self.count += 1;
        }
        let item = self.input.next()?;
        self.count += 1;
        self.next = self.next.saturating_add(self.step);
        if let Some(stop) = self.stop {
            self.next = self.next.min(stop);
        }
        Some(item)
    }
}

const ISLICE: &[u8] = b"itertools.islice\0";

/// `itertools.islice(it, start, stop, step)`; `has_stop == 0` is a `None`
/// stop. Raises CPython's `ValueError`s for a negative bound or a step that
/// is not positive.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_it_islice(
    it: *mut u8,
    start: i64,
    stop: i64,
    step: i64,
    has_stop: i64,
    source: i64,
    out: i64,
) -> *mut PyGenObj {
    const STOP: &str =
        "Stop argument for islice() must be None or an integer: 0 <= x <= sys.maxsize.";
    const INDICES: &str = "Indices for islice() must be None or an integer: 0 <= x <= sys.maxsize.";
    const STEP: &str = "Step for islice() must be a positive integer or None.";
    let stop = if flag(has_stop) != 0 {
        match small_int(stop, STOP) {
            Some(v) if v >= 0 => Some(v as usize),
            // CPython's `-1` (also its conversion-error sentinel) names the
            // stop; any other negative stop has the indices' wording.
            Some(-1) => {
                crate::raise_builtin(crate::EXCEPTION_TYPE_VALUE_ERROR, "ValueError", STOP);
                return spawn_empty(out, ISLICE);
            }
            Some(_) => {
                crate::raise_builtin(crate::EXCEPTION_TYPE_VALUE_ERROR, "ValueError", INDICES);
                return spawn_empty(out, ISLICE);
            }
            None => return spawn_empty(out, ISLICE),
        }
    } else {
        None
    };
    let start = match small_int(start, INDICES) {
        Some(v) if v >= 0 => v as usize,
        Some(_) => {
            crate::raise_builtin(crate::EXCEPTION_TYPE_VALUE_ERROR, "ValueError", INDICES);
            return spawn_empty(out, ISLICE);
        }
        None => return spawn_empty(out, ISLICE),
    };
    let step = match small_int(step, STEP) {
        Some(v) if v >= 1 => v as usize,
        Some(_) => {
            crate::raise_builtin(crate::EXCEPTION_TYPE_VALUE_ERROR, "ValueError", STEP);
            return spawn_empty(out, ISLICE);
        }
        None => return spawn_empty(out, ISLICE),
    };
    let input = Input::new(it, source);
    spawn(
        Box::new(Islice {
            input,
            next: start,
            stop,
            step,
            count: 0,
        }),
        out,
        ISLICE,
    )
}

// ------------------------------------------------------------------ chain

struct Chain {
    inputs: Vec<Input>,
    at: usize,
}

impl Native for Chain {
    fn next(&mut self) -> Option<Item> {
        while self.at < self.inputs.len() {
            let input = &mut self.inputs[self.at];
            match input.next() {
                Some(item) => {
                    // A tuple from one input may be boxed and from another
                    // not: the value area takes either.
                    return Some(item);
                }
                None if crate::exception::pycc_rt_exception_active() != 0 => return None,
                None => self.at += 1,
            }
        }
        None
    }
}

const CHAIN: &[u8] = b"itertools.chain\0";

/// `itertools.chain(a)`.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_it_chain1(a: *mut u8, da: i64, out: i64) -> *mut PyGenObj {
    let inputs = vec![Input::new(a, da)];
    spawn(Box::new(Chain { inputs, at: 0 }), out, CHAIN)
}

/// `itertools.chain(a, b)` (`chain(a, b, c)` nests).
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_it_chain2(
    a: *mut u8,
    b: *mut u8,
    da: i64,
    db: i64,
    out: i64,
) -> *mut PyGenObj {
    let inputs = vec![Input::new(a, da), Input::new(b, db)];
    spawn(Box::new(Chain { inputs, at: 0 }), out, CHAIN)
}

struct ChainFrom {
    outer: Input,
    /// The inner iterable being drained: its own source descriptor.
    inner: Option<Input>,
    inner_source: i64,
}

impl Native for ChainFrom {
    fn next(&mut self) -> Option<Item> {
        loop {
            if let Some(inner) = &mut self.inner {
                match inner.next() {
                    Some(item) => return Some(item),
                    None if crate::exception::pycc_rt_exception_active() != 0 => return None,
                    None => self.inner = None,
                }
            }
            let next = self.outer.next()?;
            let word = next.into_word(self.outer.desc());
            let object = if self.outer.desc().kind() == LIST_KIND_STR {
                // A `str` iterable yields its characters.
                let chars = crate::str_ops::pycc_rt_str_to_list(word as *mut crate::PyStrObj);
                release(LIST_KIND_STR, word);
                chars
            } else {
                word as *mut PyListObj
            };
            self.inner = Some(Input::new(object.cast(), self.inner_source));
            // `Input::new` took its own reference.
            release(crate::list::LIST_KIND_LIST, object as i64);
        }
    }
}

/// `itertools.chain.from_iterable(it)`: `it` yields lists (or `str`s, which
/// yield their characters); `inner` is the source descriptor of each.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_it_chain_from(
    it: *mut u8,
    source: i64,
    inner: i64,
    out: i64,
) -> *mut PyGenObj {
    let outer = Input::new(it, source);
    spawn(
        Box::new(ChainFrom {
            outer,
            inner: None,
            inner_source: inner,
        }),
        out,
        b"itertools.chain\0",
    )
}

// ------------------------------------------------------------- accumulate

/// `accumulate`'s binary functions (`pycc_hir::builtins` mirrors these).
const OP_ADD: i64 = 0;
const OP_MUL: i64 = 1;
const OP_SUB: i64 = 2;
const OP_MAX: i64 = 3;
const OP_MIN: i64 = 4;
const OP_AND: i64 = 5;
const OP_OR: i64 = 6;
const OP_XOR: i64 = 7;

/// `a <op> b` for two owned words of `kind`, consuming both; `None` with an
/// exception pending.
fn combine(op: i64, kind: i64, a: i64, b: i64) -> Option<i64> {
    let result = match kind {
        LIST_KIND_INT => match op {
            OP_ADD => crate::int_arith::int_add(a, b),
            OP_MUL => crate::int_arith::int_mul(a, b),
            OP_SUB => crate::int_arith::int_sub(a, b),
            OP_MAX | OP_MIN => {
                let take_b = if op == OP_MAX {
                    crate::int_arith::int_cmp(b, a) > 0
                } else {
                    crate::int_arith::int_cmp(b, a) < 0
                };
                return Some(if take_b {
                    release(kind, a);
                    b
                } else {
                    release(kind, b);
                    a
                });
            }
            OP_AND => crate::pycc_rt_int_and(a, b),
            OP_OR => crate::pycc_rt_int_or(a, b),
            _ => {
                debug_assert_eq!(op, OP_XOR);
                crate::pycc_rt_int_xor(a, b)
            }
        },
        LIST_KIND_FLOAT => {
            let (x, y) = (f64::from_bits(a as u64), f64::from_bits(b as u64));
            let r = match op {
                OP_ADD => x + y,
                OP_MUL => x * y,
                OP_SUB => x - y,
                // CPython's `max(a, b)` keeps `a` unless `b > a`.
                OP_MAX => {
                    if y > x {
                        y
                    } else {
                        x
                    }
                }
                _ => {
                    if y < x {
                        y
                    } else {
                        x
                    }
                }
            };
            return Some(r.to_bits() as i64);
        }
        _ => match op {
            OP_ADD => {
                // SAFETY: two live `str` words.
                unsafe {
                    crate::pycc_rt_str_concat(a as *mut crate::PyStrObj, b as *mut crate::PyStrObj)
                        as i64
                }
            }
            _ => {
                let take_b = if op == OP_MAX {
                    crate::value::lt(kind, a, b) == Some(true)
                } else {
                    crate::value::lt(kind, b, a) == Some(true)
                };
                return Some(if take_b {
                    release(kind, a);
                    b
                } else {
                    release(kind, b);
                    a
                });
            }
        },
    };
    release(kind, a);
    release(kind, b);
    if crate::exception::pycc_rt_exception_active() != 0 {
        release(kind, result);
        return None;
    }
    Some(result)
}

struct Accumulate {
    input: Input,
    op: i64,
    /// The running total (owned), once there is one.
    total: Option<i64>,
    /// `initial`, yielded first.
    initial: Option<i64>,
}

impl Native for Accumulate {
    fn next(&mut self) -> Option<Item> {
        let kind = self.input.desc().kind();
        if let Some(initial) = self.initial.take() {
            retain(kind, initial);
            self.total = Some(initial);
            return Some(Item::Word(initial));
        }
        let item = self.input.next()?;
        let word = item.into_word(self.input.desc());
        let total = match self.total.take() {
            None => word,
            Some(total) => combine(self.op, kind, total, word)?,
        };
        retain(kind, total);
        self.total = Some(total);
        Some(Item::Word(total))
    }
}

impl Drop for Accumulate {
    fn drop(&mut self) {
        let kind = self.input.desc().kind();
        for word in self.total.take().into_iter().chain(self.initial.take()) {
            release(kind, word);
        }
    }
}

/// `itertools.accumulate(it, op)`, `op` one of `OP_*`.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_it_accumulate(
    it: *mut u8,
    op: i64,
    source: i64,
    out: i64,
) -> *mut PyGenObj {
    let input = Input::new(it, source);
    spawn(
        Box::new(Accumulate {
            input,
            op: flag(op),
            total: None,
            initial: None,
        }),
        out,
        b"itertools.accumulate\0",
    )
}

/// `itertools.accumulate(it, op, initial=initial)`; `initial` is a borrowed
/// word of the element kind.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_it_accumulate_initial(
    it: *mut u8,
    op: i64,
    initial: i64,
    source: i64,
    out: i64,
) -> *mut PyGenObj {
    let input = Input::new(it, source);
    retain(input.desc().kind(), initial);
    spawn(
        Box::new(Accumulate {
            input,
            op: flag(op),
            total: None,
            initial: Some(initial),
        }),
        out,
        b"itertools.accumulate\0",
    )
}

// --------------------------------------------------------------- pairwise

struct Pairwise {
    input: Input,
    previous: Option<i64>,
}

impl Native for Pairwise {
    fn next(&mut self) -> Option<Item> {
        let desc = *self.input.desc();
        let previous = match self.previous.take() {
            Some(previous) => previous,
            None => self.input.next()?.into_word(&desc),
        };
        let Some(item) = self.input.next() else {
            release(desc.kind(), previous);
            return None;
        };
        let current = item.into_word(&desc);
        retain(desc.kind(), current);
        self.previous = Some(current);
        Some(Item::Fields(vec![previous, current]))
    }
}

impl Drop for Pairwise {
    fn drop(&mut self) {
        if let Some(previous) = self.previous.take() {
            release(self.input.desc().kind(), previous);
        }
    }
}

/// `itertools.pairwise(it)`.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_it_pairwise(it: *mut u8, source: i64, out: i64) -> *mut PyGenObj {
    let input = Input::new(it, source);
    spawn(
        Box::new(Pairwise {
            input,
            previous: None,
        }),
        out,
        b"itertools.pairwise\0",
    )
}

// ---------------------------------------------------------------- groupby

/// `groupby`'s state: each input value is split into a key word and an
/// element word (`keyed`: the input yields `(key, element)` pairs).
struct Groupby {
    input: Input,
    keyed: bool,
    key_kind: i64,
    elem_kind: i64,
    /// The first value of the next group, read ahead.
    pending: Option<(i64, i64)>,
}

impl Groupby {
    fn take(&mut self) -> Option<(i64, i64)> {
        let desc: Desc = *self.input.desc();
        let item = self.input.next()?;
        if self.keyed {
            let fields = item.into_fields(&desc);
            return Some((fields[0], fields[1]));
        }
        let word = item.into_word(&desc);
        retain(desc.kind(), word);
        Some((word, word))
    }
}

impl Native for Groupby {
    fn next(&mut self) -> Option<Item> {
        let (key, first) = match self.pending.take() {
            Some(pair) => pair,
            None => self.take()?,
        };
        let mut group = vec![first];
        loop {
            match self.take() {
                Some((k, e)) if crate::value::eq(self.key_kind, k, key) => {
                    release(self.key_kind, k);
                    group.push(e);
                }
                Some(pair) => {
                    self.pending = Some(pair);
                    break;
                }
                None if crate::exception::pycc_rt_exception_active() != 0 => {
                    release(self.key_kind, key);
                    for e in group {
                        release(self.elem_kind, e);
                    }
                    return None;
                }
                None => break,
            }
        }
        let list = crate::list::new_raw_words(self.elem_kind, group);
        Some(Item::Fields(vec![key, list as i64]))
    }
}

impl Drop for Groupby {
    fn drop(&mut self) {
        if let Some((k, e)) = self.pending.take() {
            release(self.key_kind, k);
            release(self.elem_kind, e);
        }
    }
}

/// `itertools.groupby(it)` (`keyed == 0`), or `groupby` over `(key,
/// element)` pairs (`keyed == 1`, `groupby(it, key=f)` rewritten): yields
/// `(key, list of the group)`. `key_kind`/`elem_kind` are the word kinds.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_it_groupby(
    it: *mut u8,
    keyed: i64,
    source: i64,
    out: i64,
) -> *mut PyGenObj {
    let keyed = flag(keyed);
    let input = Input::new(it, source);
    let out_desc = Desc::decode(out);
    let key_kind = out_desc.elem(0);
    let elem_kind = if keyed != 0 {
        input.desc().elem(1)
    } else {
        input.desc().kind()
    };
    spawn(
        Box::new(Groupby {
            input,
            keyed: keyed != 0,
            key_kind,
            elem_kind,
            pending: None,
        }),
        out,
        b"itertools.groupby\0",
    )
}
