//! A set of `str` and one-character text: membership of a character with
//! no `str` allocated for it (an ASCII bitmap for a frozenset), the
//! `str`-argument predicates (`isdisjoint`, `issuperset`, `issubset`), and
//! the CPython `frozenset` a frozenset shares across the `--ext` boundary.

use super::*;

impl PySetObj {
    /// Whether a set of `str` holds the one-character string `ch`, with
    /// no `str` allocated for it.
    pub(super) fn contains_char(&self, ch: char) -> bool {
        if ch.is_ascii() && self.frozen != 0 {
            return self.ascii_members() & (1u128 << (ch as u32)) != 0;
        }
        self.lookup_char(ch)
    }

    /// [`PySetObj::ascii`], built on first use or when the table changed.
    pub(super) fn ascii_members(&self) -> u128 {
        let table = self.table.as_ptr();
        if let Some(cached) = self.ascii.get()
            && cached.table == table
            && cached.len == self.len
        {
            return cached.bits;
        }
        let bits = (0u8..128)
            .filter(|&byte| self.lookup_char(char::from(byte)))
            .fold(0u128, |bits, byte| bits | (1u128 << byte));
        self.ascii.set(Some(AsciiMembers {
            table,
            len: self.len,
            bits,
        }));
        bits
    }

    /// [`PySetObj::contains_char`] by a hash table lookup.
    fn lookup_char(&self, ch: char) -> bool {
        let mut buf = [0u8; 4];
        let bytes = ch.encode_utf8(&mut buf).as_bytes();
        let hash = crate::str_hash::hash_str(bytes);
        let slot = self.lookkey_by(hash, |entry| {
            // SAFETY: a set of `str` holds live `PyStrObj` keys.
            unsafe { &*(entry as *mut PyStrObj) }.bytes() == bytes
        });
        self.table[slot].state == ACTIVE
    }
}

/// The CPython `frozenset` the frozenset `s` shares (borrowed), or null
/// (`PySetObj::foreign`).
///
/// # Safety
/// `s` must be a live set.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_set_foreign(s: *mut PySetObj) -> *mut std::ffi::c_void {
    if is_instance_set(s) || set(s).frozen == 0 {
        return std::ptr::null_mut();
    }
    set(s).foreign.get()
}

/// Makes `object` (a CPython `frozenset` of `s`'s elements, whose reference
/// this adopts) the one the frozenset `s` shares; released at once when `s`
/// is not a frozenset.
///
/// # Safety
/// `s` must be a live set, `object` a `PyObject *` of its elements.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_set_set_foreign(s: *mut PySetObj, object: *mut std::ffi::c_void) {
    if is_instance_set(s) || set(s).frozen == 0 {
        crate::rc::release_object(object);
        return;
    }
    let old = set(s).foreign.replace(object);
    if !old.is_null() {
        crate::rc::release_object(old);
    }
}

/// `a.isdisjoint(text)`, `a.issuperset(text)` and `a.issubset(text)` for a
/// set of `str` and a `str` argument (an iterable of its characters):
/// the predicate against the set of those characters.
///
/// # Safety
/// `a` must be a live set of `str`, `text` a live `str`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_set_isdisjoint_str(a: *mut PySetObj, text: *mut PyStrObj) -> i8 {
    match unsafe { chars_of(a, text) } {
        Some(mut chars) => i8::from(!chars.any(|ch| set(a).contains_char(ch))),
        None => unsafe { with_char_set(text, |chars| pycc_rt_set_isdisjoint(a, chars)) },
    }
}

/// See [`pycc_rt_set_isdisjoint_str`].
///
/// # Safety
/// `a` must be a live set of `str`, `text` a live `str`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_set_issuperset_str(a: *mut PySetObj, text: *mut PyStrObj) -> i8 {
    match unsafe { chars_of(a, text) } {
        Some(mut chars) => i8::from(chars.all(|ch| set(a).contains_char(ch))),
        None => unsafe { with_char_set(text, |chars| pycc_rt_set_issubset(chars, a)) },
    }
}

/// See [`pycc_rt_set_isdisjoint_str`].
///
/// # Safety
/// `a` must be a live set of `str`, `text` a live `str`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_set_issubset_str(a: *mut PySetObj, text: *mut PyStrObj) -> i8 {
    unsafe { with_char_set(text, |chars| pycc_rt_set_issubset(a, chars)) }
}

/// `text`'s characters, to look up one by one in `a` with no `str`
/// allocated per character: `None` when `a` is not a set of `str` (an
/// empty set built from a list may have another kind) or `text` is not
/// UTF-8, which [`with_char_set`] answers instead.
unsafe fn chars_of<'t>(a: *mut PySetObj, text: *mut PyStrObj) -> Option<std::str::Chars<'t>> {
    if is_instance_set(a) || set(a).kind != crate::list::LIST_KIND_STR {
        return None;
    }
    // SAFETY: `text` is a live `str`, which outlives the predicate's call.
    let bytes: &'t [u8] = unsafe { &*text }.bytes();
    std::str::from_utf8(bytes).ok().map(str::chars)
}

/// Runs `f` on a temporary set of `text`'s characters.
unsafe fn with_char_set(text: *mut PyStrObj, f: impl FnOnce(*mut PySetObj) -> i8) -> i8 {
    let chars = unsafe { pycc_rt_set_from_str(text) };
    let answer = f(chars);
    unsafe { pycc_rt_set_decref(chars) };
    answer
}

/// `text[index] in s` for a set of `str`, without making the character
/// (`index` an encoded `int`, `text` and `s` borrowed): what
/// `pycc_rt_str_getitem` followed by [`pycc_rt_set_contains`] answers,
/// `IndexError` included (made pending, answering `0`). An ASCII character
/// of ASCII text in a frozenset is a bit test.
///
/// # Safety
/// `text` must be a live `str` and `s` a live set of `str`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_str_index_in_set(
    text: *mut PyStrObj,
    index: i64,
    s: *mut PySetObj,
) -> i8 {
    if index & 1 == 1 {
        // SAFETY: the caller passes a live `str`.
        let obj = unsafe { &*text };
        let bytes = obj.bytes();
        let count = obj.chars.get();
        if count == bytes.len() as i64 {
            let i = index >> 1;
            let i = if i < 0 { i + count } else { i };
            if (0..count).contains(&i) {
                let byte = bytes[i as usize];
                let st = set(s);
                if st.frozen != 0 && st.kind == crate::list::LIST_KIND_STR {
                    return i8::from(st.ascii_members() & (1u128 << byte) != 0);
                }
            }
        }
    }
    let ch = crate::str_ops::pycc_rt_str_getitem(text, index);
    if crate::exception::pycc_rt_exception_active() != 0 {
        unsafe { crate::pycc_rt_str_decref(ch) };
        return 0;
    }
    // SAFETY: `s` is live; the character is a live `str`.
    let found = unsafe { super::pycc_rt_set_contains(s, ch as i64) };
    unsafe { crate::pycc_rt_str_decref(ch) };
    found
}

/// `while text[index] in s: index += 1` (`not in` when `negate`) for a set
/// of `str`, with no character made (`index` an encoded `int`, `text` and
/// `s` borrowed): the index the loop stops at, encoded. A loop that would
/// stop on an `IndexError` stops at the index that raises it, without
/// raising: the caller reads `text[index]` once more to raise it. A
/// bigint index (borrowed) is returned unchanged for the same reason, with
/// the reference the caller owns in the result.
///
/// # Safety
/// `text` must be a live `str` and `s` a live set of `str`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_str_skip_in_set(
    text: *mut PyStrObj,
    index: i64,
    s: *mut PySetObj,
    negate: i8,
) -> i64 {
    if index & 1 == 0 {
        crate::pycc_rt_bigint_retain(index);
        return index;
    }
    // SAFETY: the caller passes a live `str`.
    let obj = unsafe { &*text };
    let bytes = obj.bytes();
    let ascii_set = !is_instance_set(s)
        && set(s).frozen != 0
        && set(s).kind == crate::list::LIST_KIND_STR;
    let bits = if ascii_set { set(s).ascii_members() } else { 0 };
    let stop_on = negate == 0;
    let mut i = index >> 1;
    // Text already known to be ASCII (its counted code points are its
    // bytes), from a non-negative index: a run of bit tests.
    if ascii_set && i >= 0 && obj.chars.get() == bytes.len() as i64 {
        let (low, high) = (bits as u64, (bits >> 64) as u64);
        let mut at = i as usize;
        while let Some(&byte) = bytes.get(at) {
            let word = if byte < 64 { low >> byte } else { high >> (byte - 64) };
            if (word & 1 == 0) == stop_on {
                break;
            }
            at += 1;
        }
        return ((at as i64) << 1) | 1;
    }
    let count = obj.char_count() as i64;
    let ascii = ascii_set && count == bytes.len() as i64;
    loop {
        let at = if i < 0 { i + count } else { i };
        if !(0..count).contains(&at) {
            break;
        }
        let member = if ascii {
            bits & (1u128 << bytes[at as usize]) != 0
        } else {
            let ch = crate::str_index::char_at(obj, at as usize);
            // SAFETY: `s` is live; the character is a live `str`.
            let found = unsafe { super::pycc_rt_set_contains(s, ch as i64) } != 0;
            unsafe { crate::pycc_rt_str_decref(ch) };
            found
        };
        if member != stop_on {
            break;
        }
        i += 1;
    }
    (i << 1) | 1
}

/// Raises the `IndexError` `text[index]` raises, if any, without making the
/// character (`index` an encoded `int`, borrowed): what follows a
/// [`pycc_rt_str_skip_in_set`] scan. Answers whether `index` is in range.
///
/// # Safety
/// `text` must be a live `str`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_str_check_index(text: *mut PyStrObj, index: i64) -> i8 {
    if index & 1 == 1 {
        // SAFETY: the caller passes a live `str`.
        let count = unsafe { &*text }.char_count() as i64;
        let i = index >> 1;
        let at = if i < 0 { i + count } else { i };
        if (0..count).contains(&at) {
            return 1;
        }
    }
    let ch = crate::str_ops::pycc_rt_str_getitem(text, index);
    unsafe { crate::pycc_rt_str_decref(ch) };
    0
}

/// `text in s` for a set of `str` and a text given by its UTF-8 bytes
/// (`bytes`, `len`), with no `str` made for it: `1` or `0`, or `-1` when
/// `s` is not known to hold `str` keys (an instance set, or a non-empty set
/// of another kind), which the caller then answers another way. A
/// one-character ASCII text in a frozenset is a bit test.
///
/// # Safety
/// `s` must be a live set and `bytes` point at `len` readable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_set_contains_utf8(
    s: *mut PySetObj,
    bytes: *const u8,
    len: usize,
) -> i8 {
    if is_instance_set(s) {
        return -1;
    }
    let st = set(s);
    if st.len == 0 {
        return 0;
    }
    if st.kind != crate::list::LIST_KIND_STR {
        return -1;
    }
    // SAFETY: the caller passes `len` readable bytes.
    let text = unsafe { std::slice::from_raw_parts(bytes, len) };
    if let [byte] = text
        && byte.is_ascii()
        && st.frozen != 0
    {
        return i8::from(st.ascii_members() & (1u128 << byte) != 0);
    }
    let hash = crate::str_hash::hash_str(text);
    let slot = st.lookkey_by(hash, |entry| {
        // SAFETY: a set of `str` holds live `PyStrObj` keys.
        unsafe { &*(entry as *mut PyStrObj) }.bytes() == text
    });
    i8::from(st.table[slot].state == ACTIVE)
}
