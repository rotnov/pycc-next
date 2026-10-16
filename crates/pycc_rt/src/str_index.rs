//! Code-point counting and indexing of a UTF-8 `PyStrObj`.
//!
//! CPython indexes a `str` by code point in O(1). pycc's `str` is UTF-8, so
//! a string caches its code-point count on first use (`PyStrObj::chars`;
//! an all-ASCII string is one whose count equals its byte length) and a
//! non-ASCII string builds, on its first index or slice, a table of the
//! byte offset of every [`STRIDE`]-th code point (`PyStrObj::offsets`).
//! `s[i]`, `s[i:j]` and `len(s)` are then O(1) in the length of `s`: at most
//! `STRIDE - 1` characters are stepped over after the table lookup.
//!
//! One-character ASCII strings (what `s[i]` of ASCII text yields) are shared
//! immortal objects ([`ascii_char`]), as CPython shares its one-character
//! Latin-1 strings, so indexing allocates nothing.

use super::*;
use std::sync::atomic::AtomicPtr;

/// Code points between two entries of the offset table.
pub(crate) const STRIDE: usize = 32;

/// Whether `byte` starts a UTF-8 sequence (is not a continuation byte).
fn starts_char(byte: u8) -> bool {
    (byte as i8) >= -0x40
}

/// The number of code points in UTF-8 `bytes`.
pub(crate) fn count_chars(bytes: &[u8]) -> usize {
    if bytes.is_ascii() {
        bytes.len()
    } else {
        bytes.iter().filter(|b| starts_char(**b)).count()
    }
}

impl PyStrObj {
    /// The number of code points (`len(s)`), counted once.
    pub(crate) fn char_count(&self) -> usize {
        let cached = self.chars.get();
        if cached >= 0 {
            return cached as usize;
        }
        let count = count_chars(self.bytes());
        self.chars.set(count as i64);
        count
    }

    /// Whether every code point is ASCII (one byte each).
    pub(crate) fn is_ascii_text(&self) -> bool {
        self.char_count() == self.bytes().len()
    }

    fn offset_table(&self) -> &[usize] {
        self.offsets.get_or_init(|| {
            let bytes = self.bytes();
            let mut table = Vec::with_capacity(self.char_count() / STRIDE + 1);
            let mut seen = 0usize;
            for (at, byte) in bytes.iter().enumerate() {
                if starts_char(*byte) {
                    if seen % STRIDE == 0 {
                        table.push(at);
                    }
                    seen += 1;
                }
            }
            table.into_boxed_slice()
        })
    }

    /// The byte offset of code point `index`, for `0 <= index <=
    /// char_count()` (`char_count()` is the end of the string).
    pub(crate) fn byte_offset(&self, index: usize) -> usize {
        if self.is_ascii_text() {
            return index;
        }
        let bytes = self.bytes();
        if index >= self.char_count() {
            return bytes.len();
        }
        let mut at = self.offset_table()[index / STRIDE];
        for _ in 0..index % STRIDE {
            at += 1;
            while at < bytes.len() && !starts_char(bytes[at]) {
                at += 1;
            }
        }
        at
    }

    /// The code-point index of byte offset `byte` (a character boundary).
    pub(crate) fn char_index_of(&self, byte: usize) -> usize {
        if self.is_ascii_text() {
            return byte;
        }
        let table = self.offset_table();
        let block = table.partition_point(|start| *start <= byte).saturating_sub(1);
        let start = table.get(block).copied().unwrap_or(0);
        block * STRIDE + count_chars(&self.bytes()[start..byte])
    }

    /// The UTF-8 bytes of code point `index` (in range).
    pub(crate) fn char_bytes(&self, index: usize) -> &[u8] {
        let bytes = self.bytes();
        if self.is_ascii_text() {
            return &bytes[index..index + 1];
        }
        let start = self.byte_offset(index);
        let mut end = start + 1;
        while end < bytes.len() && !starts_char(bytes[end]) {
            end += 1;
        }
        &bytes[start..end]
    }
}

/// A reference count no program reaches: a shared one-character string is
/// never freed.
pub(crate) const IMMORTAL: u32 = 1 << 30;

static ASCII_CHARS: [AtomicPtr<PyStrObj>; 128] = [const { AtomicPtr::new(std::ptr::null_mut()) }; 128];

/// A new reference to the shared one-character `str` of ASCII `byte`.
///
/// The objects are created on first use and never freed, nor counted by
/// `STR_LIVE` (a leak probe compares counts around a computation, which
/// must not see the first use of a character as a leak).
pub(crate) fn ascii_char(byte: u8) -> *mut PyStrObj {
    let slot = &ASCII_CHARS[byte as usize & 0x7f];
    let mut obj = slot.load(Ordering::Acquire);
    if obj.is_null() {
        let fresh = new_pystr(&[byte]);
        STR_LIVE.fetch_sub(1, Ordering::Relaxed);
        unsafe { &*fresh }.rc.set(IMMORTAL);
        unsafe { &*fresh }.chars.set(1);
        match slot.compare_exchange(std::ptr::null_mut(), fresh, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => obj = fresh,
            Err(existing) => {
                drop(unsafe { Box::from_raw(fresh) });
                obj = existing;
            }
        }
    }
    retain_immortal(obj)
}

static EMPTY: AtomicPtr<PyStrObj> = AtomicPtr::new(std::ptr::null_mut());

/// A reference to the shared immortal empty `str`.
///
/// Returned by a runtime function that raised instead of producing its
/// `str`: the caller's exception path may skip releasing the placeholder,
/// so it must not be a fresh allocation (it would leak). Like the shared
/// characters it is never freed nor counted by `STR_LIVE`.
pub(crate) fn empty_str() -> *mut PyStrObj {
    let mut obj = EMPTY.load(Ordering::Acquire);
    if obj.is_null() {
        let fresh = new_pystr(&[]);
        STR_LIVE.fetch_sub(1, Ordering::Relaxed);
        unsafe { &*fresh }.rc.set(IMMORTAL);
        match EMPTY.compare_exchange(std::ptr::null_mut(), fresh, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => obj = fresh,
            Err(existing) => {
                drop(unsafe { Box::from_raw(fresh) });
                obj = existing;
            }
        }
    }
    retain_immortal(obj)
}

/// A reference to `obj`, made immortal: from then on its count never
/// changes (`pycc_rt_str_incref`/`pycc_rt_str_decref` leave a count at or
/// above `IMMORTAL / 2` alone) and it is never freed.
pub(crate) fn retain_immortal(obj: *mut PyStrObj) -> *mut PyStrObj {
    let rc = &unsafe { &*obj }.rc;
    if rc.get() < IMMORTAL / 2 {
        rc.set(IMMORTAL);
    }
    obj
}

/// A new `str` holding code point `index` of `s` (in range).
pub(crate) fn char_at(s: &PyStrObj, index: usize) -> *mut PyStrObj {
    let bytes = s.char_bytes(index);
    if bytes.len() == 1 {
        return ascii_char(bytes[0]);
    }
    let out = new_pystr(bytes);
    unsafe { &*out }.chars.set(1);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_of(s: *mut PyStrObj) -> String {
        String::from_utf8(unsafe { &*s }.bytes().to_vec()).expect("UTF-8")
    }

    #[test]
    fn counts_and_offsets_follow_code_points() {
        let text: String = (0..200)
            .map(|i| match i % 4 {
                0 => 'a',
                1 => '\u{e9}',
                2 => '\u{65e5}',
                _ => '\u{1f600}',
            })
            .collect();
        let s = unsafe { &*new_pystr(text.as_bytes()) };
        assert_eq!(s.char_count(), 200);
        assert!(!s.is_ascii_text());
        let starts: Vec<usize> = text.char_indices().map(|(at, _)| at).collect();
        for (index, start) in starts.iter().enumerate() {
            assert_eq!(s.byte_offset(index), *start, "{index}");
            assert_eq!(s.char_index_of(*start), index, "{index}");
            let c = text[*start..].chars().next().expect("char");
            assert_eq!(s.char_bytes(index), c.to_string().as_bytes());
        }
        assert_eq!(s.byte_offset(200), text.len());
        assert_eq!(s.char_index_of(text.len()), 200);
        let ascii = unsafe { &*new_pystr(b"hello") };
        assert!(ascii.is_ascii_text());
        assert_eq!((ascii.byte_offset(3), ascii.char_index_of(3)), (3, 3));
        assert_eq!(ascii.char_bytes(4), b"o");
    }

    #[test]
    fn one_ascii_character_is_shared_and_never_freed() {
        let s = new_pystr("xy\u{e9}".as_bytes());
        let a = char_at(unsafe { &*s }, 0);
        let b = char_at(unsafe { &*s }, 0);
        assert_eq!(a, b);
        unsafe {
            pycc_rt_str_decref(a);
            pycc_rt_str_decref(b);
        }
        assert_eq!(text_of(a), "x");
        let e = char_at(unsafe { &*s }, 2);
        assert_eq!(text_of(e), "\u{e9}");
        assert_eq!(unsafe { &*e }.char_count(), 1);
    }
}
