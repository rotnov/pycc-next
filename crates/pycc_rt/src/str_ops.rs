//! `str` builtins and methods (`pycc_hir::builtins` lowers the calls), and
//! `input()`.
//!
//! A `PyStrObj` holds UTF-8. Every operation that counts or indexes takes
//! the all-ASCII fast path on bytes and otherwise works on `char`s, so
//! indices and lengths are CPython's code-point indices. Character classes
//! (`isdigit`, whitespace for `split()`/`strip()`, case mapping) follow
//! CPython for ASCII and Rust's Unicode tables beyond it.

use super::*;
use crate::exception::{EXCEPTION_TYPE_EOF_ERROR, EXCEPTION_TYPE_STOP_ITERATION};
use crate::list::{PyListObj, str_list};

fn bytes_of<'a>(s: *mut PyStrObj) -> &'a [u8] {
    unsafe { &*s }.bytes()
}

fn text_of<'a>(s: *mut PyStrObj) -> &'a str {
    // Every `PyStrObj` is built from UTF-8 (source literals, stdin decoded
    // lossily, and the operations below, which only split at char
    // boundaries).
    unsafe { std::str::from_utf8_unchecked(bytes_of(s)) }
}

fn new_str(text: &str) -> *mut PyStrObj {
    new_pystr(text.as_bytes())
}

fn value_error(msg: &str) {
    raise_builtin(EXCEPTION_TYPE_VALUE_ERROR, "ValueError", msg);
}

/// Python's `str.isspace()` for one character (`str.split()`/`strip()`
/// whitespace).
fn is_py_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0b' | '\x0c' | '\x1c'..='\x1f' | '\u{85}')
        || (!c.is_ascii() && c.is_whitespace())
}

/// `repr()` of a `str`, for error messages.
fn repr(text: &str) -> String {
    let mut out = String::new();
    crate::list::str_repr(text, &mut out);
    out
}

fn str_list_of<'a>(parts: impl IntoIterator<Item = &'a str>) -> *mut PyListObj {
    str_list(parts.into_iter().map(|p| new_str(p) as i64).collect())
}

/// Number of code points in `text`.
fn char_len(text: &str) -> usize {
    if text.is_ascii() {
        text.len()
    } else {
        text.chars().count()
    }
}

/// Code-point index of byte offset `byte` in `text`.
fn char_index(text: &str, byte: usize) -> i64 {
    if text.is_ascii() {
        byte as i64
    } else {
        text[..byte].chars().count() as i64
    }
}

// ---------------------------------------------------------------- input
//
// Every read and write goes through `crate::io`, which shares CPython's
// `sys.stdin`/`sys.stdout` when an interpreter is present.

fn input_line() -> *mut PyStrObj {
    // CPython's `input()` flushes stdout before it reads.
    crate::io::flush();
    let Some(line) = crate::io::read_line() else {
        return new_str("");
    };
    let text = text_of(line);
    if text.is_empty() {
        unsafe { pycc_rt_str_decref(line) };
        raise_builtin(EXCEPTION_TYPE_EOF_ERROR, "EOFError", "EOF when reading a line");
        return new_str("");
    }
    let Some(stripped) = text.strip_suffix('\n') else {
        return line;
    };
    let result = new_str(stripped);
    unsafe { pycc_rt_str_decref(line) };
    result
}

/// `input()`: one line of stdin without its trailing newline.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_input() -> *mut PyStrObj {
    input_line()
}

/// `input(prompt)`: writes `prompt` (no newline) first.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_input_prompt(prompt: *mut PyStrObj) -> *mut PyStrObj {
    crate::io::out(bytes_of(prompt));
    input_line()
}

/// `sys.stdin.readline()`: one line including its newline, `''` at EOF.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_stdin_readline() -> *mut PyStrObj {
    crate::io::read_line().unwrap_or_else(|| new_str(""))
}

/// `next(sys.stdin)`: one line including its newline; `StopIteration` at
/// EOF, as the exhausted file iterator raises.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_stdin_next() -> *mut PyStrObj {
    let line = pycc_rt_stdin_readline();
    if text_of(line).is_empty() {
        unsafe { pycc_rt_str_decref(line) };
        raise_builtin(EXCEPTION_TYPE_STOP_ITERATION, "StopIteration", "");
        return new_str("");
    }
    line
}

/// `sys.stdin.read()`: the rest of stdin.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_stdin_read() -> *mut PyStrObj {
    crate::io::read_all().unwrap_or_else(|| new_str(""))
}

/// `sys.stdin.readlines()`: the remaining lines, each with its newline.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_stdin_readlines() -> *mut PyListObj {
    let rest = pycc_rt_stdin_read();
    let text = text_of(rest);
    let list = str_list_of(text.split_inclusive('\n'));
    unsafe { pycc_rt_str_decref(rest) };
    list
}

/// `sys.stdout.write(s)`: writes `s`, returns its length.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_stdout_write(s: *mut PyStrObj) -> i64 {
    let text = text_of(s);
    crate::io::out_str(text);
    tag_smallint(char_len(text) as i64)
}

/// `sys.setrecursionlimit(n)`: compiled code recurses on the native stack,
/// so there is no limit to set.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_setrecursionlimit(_limit: i64) {}

// ----------------------------------------------------------- conversions

fn digit_value(c: u8) -> Option<u32> {
    match c {
        b'0'..=b'9' => Some((c - b'0') as u32),
        b'a'..=b'z' => Some((c - b'a') as u32 + 10),
        b'A'..=b'Z' => Some((c - b'A') as u32 + 10),
        _ => None,
    }
}

/// CPython's default `sys.get_int_max_str_digits()`: a string of more
/// digits than this in a base that is not a power of two is refused before
/// the (quadratic) conversion runs.
const INT_MAX_STR_DIGITS: usize = 4300;

/// Why [`parse_int`] produced no value.
enum IntParseError {
    Invalid,
    TooManyDigits(usize),
}

/// Parses CPython's `int(text, base)` grammar (surrounding whitespace, a
/// sign, `_` between digits, a `0x`/`0o`/`0b` prefix matching `base`, or
/// any of them for base 0) into an encoded int.
fn parse_int(text: &str, base: i64) -> Result<i64, IntParseError> {
    parse_int_checked(text, base).unwrap_or(Err(IntParseError::Invalid))
}

fn parse_int_checked(text: &str, base: i64) -> Option<Result<i64, IntParseError>> {
    let s = text.trim_matches(is_py_space).as_bytes();
    let (negative, mut s) = match s.first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let mut base = base as u32;
    let prefix_base = |p: u8| match p.to_ascii_lowercase() {
        b'x' => Some(16),
        b'o' => Some(8),
        b'b' => Some(2),
        _ => None,
    };
    let mut prefixed = false;
    if s.len() >= 2 && s[0] == b'0'
        && let Some(pb) = prefix_base(s[1])
        && (base == 0 || base == pb)
    {
        base = pb;
        s = &s[2..];
        prefixed = true;
    }
    if base == 0 {
        // Base 0 without a prefix is decimal, and forbids leading zeros
        // on a nonzero value.
        base = 10;
        if s.len() > 1 && s[0] == b'0' && s.iter().any(|&c| c != b'0' && c != b'_') {
            return None;
        }
    }
    // `_` may separate digits (and follow a base prefix), never lead,
    // trail or double.
    if s.is_empty() || s[s.len() - 1] == b'_' || (s[0] == b'_' && !prefixed) {
        return None;
    }
    // Validate and count first, so an over-long string is refused (after
    // an invalid one, as in CPython) without any quadratic work.
    let mut digits = 0usize;
    let mut prev_underscore = false;
    for &c in s {
        if c == b'_' {
            if prev_underscore {
                return None;
            }
            prev_underscore = true;
            continue;
        }
        prev_underscore = false;
        digit_value(c).filter(|d| *d < base)?;
        digits += 1;
    }
    if !base.is_power_of_two() && digits > INT_MAX_STR_DIGITS {
        return Some(Err(IntParseError::TooManyDigits(digits)));
    }
    let mut limbs: Vec<u32> = vec![0];
    let mut small: i128 = 0;
    let mut use_small = true;
    let mut prev_underscore = false;
    for &c in s {
        if c == b'_' {
            if prev_underscore {
                return None;
            }
            prev_underscore = true;
            continue;
        }
        prev_underscore = false;
        let d = digit_value(c).filter(|d| *d < base)?;
        if use_small {
            small = small * base as i128 + d as i128;
            if small > i64::MAX as i128 {
                use_small = false;
                limbs = bigint_from_i128(small).limbs.clone();
            }
            continue;
        }
        let mut carry = d as u64;
        for limb in limbs.iter_mut() {
            let v = *limb as u64 * base as u64 + carry;
            *limb = v as u32;
            carry = v >> 32;
        }
        if carry > 0 {
            limbs.push(carry as u32);
        }
    }
    if use_small {
        let value = if negative { -small } else { small } as i64;
        return Some(Ok(match fits_smallint(value) {
            Some(tagged) => tagged,
            None => tag_bigint(bigint_from_i128(value as i128)),
        }));
    }
    Some(Ok(tag_bigint(BigIntObj::new(negative, trim(&limbs)))))
}

fn int_from_str(s: *mut PyStrObj, base: i64) -> i64 {
    if base != 0 && !(2..=36).contains(&base) {
        value_error("int() base must be >= 2 and <= 36, or 0");
        return tag_smallint(0);
    }
    let text = text_of(s);
    match parse_int(text, base) {
        Ok(value) => value,
        Err(IntParseError::Invalid) => {
            // CPython formats the literal with `%.200R`: its repr, cut to
            // 200 characters.
            let shown: String = repr(text).chars().take(200).collect();
            value_error(&format!("invalid literal for int() with base {base}: {shown}"));
            tag_smallint(0)
        }
        Err(IntParseError::TooManyDigits(digits)) => {
            value_error(&format!(
                "Exceeds the limit ({INT_MAX_STR_DIGITS} digits) for integer string conversion: \
                 value has {digits} digits; use sys.set_int_max_str_digits() to increase the limit"
            ));
            tag_smallint(0)
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_int_from_str(s: *mut PyStrObj) -> i64 {
    int_from_str(s, 10)
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_int_from_str_base(s: *mut PyStrObj, base: i64) -> i64 {
    match inline_int_value(base) {
        Some(base) => int_from_str(s, base),
        None => {
            value_error("int() base must be >= 2 and <= 36, or 0");
            tag_smallint(0)
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_float_from_str(s: *mut PyStrObj) -> f64 {
    let text = text_of(s);
    let t = text.trim_matches(is_py_space);
    let lower = t.to_ascii_lowercase();
    let unsigned = lower.trim_start_matches(['+', '-']);
    let negative = lower.starts_with('-');
    let special = match unsigned {
        "inf" | "infinity" => Some(f64::INFINITY),
        "nan" => Some(f64::NAN),
        _ => None,
    };
    if let Some(v) = special
        && lower.len() - unsigned.len() <= 1
    {
        return if negative { -v } else { v };
    }
    let valid = !t.is_empty()
        && !t.contains("__")
        && !t.starts_with('_')
        && !t.ends_with('_')
        && t.bytes().all(|c| c.is_ascii_digit() || b"+-.eE_".contains(&c))
        && !lower.contains("inf")
        && !lower.contains("nan");
    match t.replace('_', "").parse::<f64>() {
        Ok(v) if valid => v,
        _ => {
            value_error(&format!(
                "could not convert string to float: {}",
                repr(text)
            ));
            0.0
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_str_copy(s: *mut PyStrObj) -> *mut PyStrObj {
    unsafe { pycc_rt_str_incref(s) };
    s
}

/// `repr(s)` of a `str`: CPython's quoting and escapes (`'a'`, `"it's"`).
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_str_repr(s: *mut PyStrObj) -> *mut PyStrObj {
    new_str(&repr(text_of(s)))
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_str_empty() -> *mut PyStrObj {
    new_str("")
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_str_len(s: *mut PyStrObj) -> i64 {
    tag_smallint(unsafe { &*s }.char_count() as i64)
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_str_ord(s: *mut PyStrObj) -> i64 {
    let text = text_of(s);
    let mut chars = text.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) => tag_smallint(c as i64),
        _ => {
            raise_builtin(
                EXCEPTION_TYPE_TYPE_ERROR,
                "TypeError",
                &format!(
                    "ord() expected a character, but string of length {} found",
                    char_len(text)
                ),
            );
            tag_smallint(0)
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_str_chr(code: i64) -> *mut PyStrObj {
    match inline_int_value(code)
        .and_then(|v| u32::try_from(v).ok())
        .and_then(char::from_u32)
    {
        Some(c) => new_str(c.encode_utf8(&mut [0; 4])),
        None => {
            value_error("chr() arg not in range(0x110000)");
            new_str("")
        }
    }
}

/// `limbs / divisor` and the remainder, for little-endian `u32` limbs.
fn divmod_limbs(limbs: &[u32], divisor: u32) -> (Vec<u32>, u32) {
    let mut quotient = vec![0u32; limbs.len()];
    let mut rem: u64 = 0;
    for i in (0..limbs.len()).rev() {
        let cur = (rem << 32) | limbs[i] as u64;
        quotient[i] = (cur / divisor as u64) as u32;
        rem = cur % divisor as u64;
    }
    (quotient, rem as u32)
}

fn radix_str(x: i64, radix: u32, prefix: &str) -> *mut PyStrObj {
    let (negative, limbs) = to_sign_and_magnitude(x);
    let mut digits = Vec::new();
    let mut mag = trim(&limbs);
    while !(mag.len() == 1 && mag[0] == 0) {
        let (q, r) = divmod_limbs(&mag, radix);
        digits.push(std::char::from_digit(r, radix).expect("digit below radix"));
        mag = trim(&q);
    }
    if digits.is_empty() {
        digits.push('0');
    }
    let body: String = digits.iter().rev().collect();
    new_str(&format!("{}{prefix}{body}", if negative { "-" } else { "" }))
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_int_bin(x: i64) -> *mut PyStrObj {
    radix_str(x, 2, "0b")
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_int_hex(x: i64) -> *mut PyStrObj {
    radix_str(x, 16, "0x")
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_int_oct(x: i64) -> *mut PyStrObj {
    radix_str(x, 8, "0o")
}

// ------------------------------------------------------------- min / max

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_str_min2(a: *mut PyStrObj, b: *mut PyStrObj) -> *mut PyStrObj {
    let pick = if bytes_of(b) < bytes_of(a) { b } else { a };
    pycc_rt_str_copy(pick)
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_str_max2(a: *mut PyStrObj, b: *mut PyStrObj) -> *mut PyStrObj {
    let pick = if bytes_of(b) > bytes_of(a) { b } else { a };
    pycc_rt_str_copy(pick)
}

// --------------------------------------------------------------- indexing

/// `s[i]`, raising CPython's `IndexError` out of range.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_str_getitem(s: *mut PyStrObj, index: i64) -> *mut PyStrObj {
    let obj = unsafe { &*s };
    let len = obj.char_count() as i64;
    // An index beyond `isize` is CPython's own `IndexError`; any other
    // bigint is out of range of every string.
    let i = match inline_int_value(index) {
        Some(i) => i,
        None => {
            let (_, magnitude) = to_sign_and_magnitude(index);
            if magnitude.len() > 2 || (magnitude.len() == 2 && magnitude[1] >= 1 << 31) {
                raise_builtin(
                    EXCEPTION_TYPE_INDEX_ERROR,
                    "IndexError",
                    "cannot fit 'int' into an index-sized integer",
                );
                return new_str("");
            }
            i64::MAX
        }
    };
    let i = if i < 0 { i + len } else { i };
    if !(0..len).contains(&i) {
        raise_builtin(
            EXCEPTION_TYPE_INDEX_ERROR,
            "IndexError",
            "string index out of range",
        );
        return new_str("");
    }
    crate::str_index::char_at(obj, i as usize)
}

/// Decodes an encoded slice bound, saturating a bigint-valued one.
fn slice_bound(encoded: i64) -> i64 {
    inline_int_value(encoded).unwrap_or_else(|| {
        if to_sign_and_magnitude(encoded).0 {
            i64::MIN / 2
        } else {
            i64::MAX / 2
        }
    })
}

/// `s[start:stop:step]`; every argument after `s` is an encoded int and
/// `present` is `pycc_rt_list_slice`'s bound mask.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_str_slice(
    s: *mut PyStrObj,
    start: i64,
    stop: i64,
    step: i64,
    present: i64,
) -> *mut PyStrObj {
    let (start, stop, step) = (slice_bound(start), slice_bound(stop), slice_bound(step));
    let present = slice_bound(present);
    let obj = unsafe { &*s };
    let len = obj.char_count() as i64;
    let Some(indices) = crate::list::slice_indices(len, start, stop, step, present) else {
        return new_str("");
    };
    let bytes = obj.bytes();
    if step == 1 || present & crate::list::SLICE_HAS_STEP == 0 {
        // A contiguous run of code points: one copy of its bytes.
        let mut indices = indices;
        let Some(first) = indices.next() else {
            return new_str("");
        };
        let count = 1 + indices.count();
        if obj.is_ascii_text() {
            let out = new_pystr(&bytes[first..first + count]);
            unsafe { &*out }.chars.set(count as i64);
            return out;
        }
        let (from, to) = (obj.byte_offset(first), obj.byte_offset(first + count));
        let out = new_pystr(&bytes[from..to]);
        unsafe { &*out }.chars.set(count as i64);
        return out;
    }
    let mut out: Vec<u8> = Vec::new();
    for i in indices {
        out.extend_from_slice(obj.char_bytes(i));
    }
    new_pystr(&out)
}

// ---------------------------------------------------------- split / strip

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_str_split_ws(s: *mut PyStrObj) -> *mut PyListObj {
    str_list_of(text_of(s).split(is_py_space).filter(|p| !p.is_empty()))
}

fn split_by(text: &str, sep: &str, max: Option<usize>) -> Option<*mut PyListObj> {
    if sep.is_empty() {
        value_error("empty separator");
        return None;
    }
    Some(match max {
        Some(n) => str_list_of(text.splitn(n + 1, sep)),
        None => str_list_of(text.split(sep)),
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_str_split(s: *mut PyStrObj, sep: *mut PyStrObj) -> *mut PyListObj {
    split_by(text_of(s), text_of(sep), None).unwrap_or_else(|| str_list(Vec::new()))
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_str_split_max(
    s: *mut PyStrObj,
    sep: *mut PyStrObj,
    maxsplit: i64,
) -> *mut PyListObj {
    let max = inline_int_value(maxsplit)
        .and_then(|m| usize::try_from(m).ok());
    split_by(text_of(s), text_of(sep), max).unwrap_or_else(|| str_list(Vec::new()))
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_str_splitlines(s: *mut PyStrObj) -> *mut PyListObj {
    let text = text_of(s);
    let mut parts = Vec::new();
    let mut start = 0;
    let mut iter = text.char_indices().peekable();
    while let Some((i, c)) = iter.next() {
        let is_break = matches!(c, '\n' | '\r' | '\x0b' | '\x0c' | '\x1c' | '\x1d' | '\x1e' | '\u{85}' | '\u{2028}' | '\u{2029}');
        if is_break {
            parts.push(&text[start..i]);
            let mut end = i + c.len_utf8();
            if c == '\r' && iter.peek().map(|(_, n)| *n) == Some('\n') {
                iter.next();
                end += 1;
            }
            start = end;
        }
    }
    if start < text.len() {
        parts.push(&text[start..]);
    }
    str_list_of(parts)
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_str_strip(s: *mut PyStrObj) -> *mut PyStrObj {
    new_str(text_of(s).trim_matches(is_py_space))
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_str_lstrip(s: *mut PyStrObj) -> *mut PyStrObj {
    new_str(text_of(s).trim_start_matches(is_py_space))
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_str_rstrip(s: *mut PyStrObj) -> *mut PyStrObj {
    new_str(text_of(s).trim_end_matches(is_py_space))
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_str_strip_chars(s: *mut PyStrObj, chars: *mut PyStrObj) -> *mut PyStrObj {
    let set: Vec<char> = text_of(chars).chars().collect();
    new_str(text_of(s).trim_matches(|c| set.contains(&c)))
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_str_lstrip_chars(s: *mut PyStrObj, chars: *mut PyStrObj) -> *mut PyStrObj {
    let set: Vec<char> = text_of(chars).chars().collect();
    new_str(text_of(s).trim_start_matches(|c| set.contains(&c)))
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_str_rstrip_chars(s: *mut PyStrObj, chars: *mut PyStrObj) -> *mut PyStrObj {
    let set: Vec<char> = text_of(chars).chars().collect();
    new_str(text_of(s).trim_end_matches(|c| set.contains(&c)))
}

// ------------------------------------------------------------------ join

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_str_join(sep: *mut PyStrObj, items: *mut PyListObj) -> *mut PyStrObj {
    let sep = bytes_of(sep);
    let words = crate::list::list_words(items);
    let mut out = Vec::new();
    for (i, w) in words.iter().enumerate() {
        if i > 0 {
            out.extend_from_slice(sep);
        }
        out.extend_from_slice(bytes_of(*w as *mut PyStrObj));
    }
    new_pystr(&out)
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_str_join_chars(sep: *mut PyStrObj, s: *mut PyStrObj) -> *mut PyStrObj {
    let sep = text_of(sep);
    let mut out = String::new();
    for (i, c) in text_of(s).chars().enumerate() {
        if i > 0 {
            out.push_str(sep);
        }
        out.push(c);
    }
    new_str(&out)
}

// ---------------------------------------------------------------- search

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_str_count(s: *mut PyStrObj, sub: *mut PyStrObj) -> i64 {
    let (text, sub) = (text_of(s), text_of(sub));
    if sub.is_empty() {
        return tag_smallint(char_len(text) as i64 + 1);
    }
    tag_smallint(text.matches(sub).count() as i64)
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_str_find(s: *mut PyStrObj, sub: *mut PyStrObj) -> i64 {
    let text = text_of(s);
    tag_smallint(text.find(text_of(sub)).map_or(-1, |b| unsafe { &*s }.char_index_of(b) as i64))
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_str_rfind(s: *mut PyStrObj, sub: *mut PyStrObj) -> i64 {
    let text = text_of(s);
    tag_smallint(text.rfind(text_of(sub)).map_or(-1, |b| unsafe { &*s }.char_index_of(b) as i64))
}

fn index_or_raise(found: i64) -> i64 {
    if found == tag_smallint(-1) {
        value_error("substring not found");
    }
    found
}

/// The byte offset a `start` argument (`s.find(sub, start)`) selects, as
/// a slice start: a negative one counts from the end, clamped to 0; `None`
/// past the end. A bigint `start` is past the end when positive.
fn start_offset(s: *mut PyStrObj, start: i64) -> Option<usize> {
    let obj = unsafe { &*s };
    let len = obj.char_count() as i64;
    let index = match crate::int_encoding::inline_int_value(start) {
        Some(index) if index < 0 => (index + len).max(0),
        Some(index) => index,
        None => i64::MAX,
    };
    (index <= len).then(|| obj.byte_offset(index as usize))
}

/// `s.find(sub, start)`.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_str_find_from(s: *mut PyStrObj, sub: *mut PyStrObj, start: i64) -> i64 {
    let Some(from) = start_offset(s, start) else {
        return tag_smallint(-1);
    };
    let text = text_of(s);
    tag_smallint(
        text[from..]
            .find(text_of(sub))
            .map_or(-1, |b| unsafe { &*s }.char_index_of(from + b) as i64),
    )
}

/// `s.index(sub, start)`.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_str_index_from(s: *mut PyStrObj, sub: *mut PyStrObj, start: i64) -> i64 {
    index_or_raise(pycc_rt_str_find_from(s, sub, start))
}

/// `s.startswith(prefix, start)`.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_str_startswith_from(
    s: *mut PyStrObj,
    prefix: *mut PyStrObj,
    start: i64,
) -> i8 {
    match start_offset(s, start) {
        Some(from) => i8::from(bytes_of(s)[from..].starts_with(bytes_of(prefix))),
        None => 0,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_str_index(s: *mut PyStrObj, sub: *mut PyStrObj) -> i64 {
    index_or_raise(pycc_rt_str_find(s, sub))
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_str_rindex(s: *mut PyStrObj, sub: *mut PyStrObj) -> i64 {
    index_or_raise(pycc_rt_str_rfind(s, sub))
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_str_replace(
    s: *mut PyStrObj,
    old: *mut PyStrObj,
    new: *mut PyStrObj,
) -> *mut PyStrObj {
    let (text, old, new) = (text_of(s), text_of(old), text_of(new));
    if old.is_empty() {
        let mut out = String::from(new);
        for c in text.chars() {
            out.push(c);
            out.push_str(new);
        }
        return new_str(&out);
    }
    new_str(&text.replace(old, new))
}

/// `needle in haystack`.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_str_contains(needle: *mut PyStrObj, haystack: *mut PyStrObj) -> i8 {
    i8::from(text_of(haystack).contains(text_of(needle)))
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_str_startswith(s: *mut PyStrObj, prefix: *mut PyStrObj) -> i8 {
    i8::from(bytes_of(s).starts_with(bytes_of(prefix)))
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_str_endswith(s: *mut PyStrObj, suffix: *mut PyStrObj) -> i8 {
    i8::from(bytes_of(s).ends_with(bytes_of(suffix)))
}

// ------------------------------------------------------------ case/class

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_str_upper(s: *mut PyStrObj) -> *mut PyStrObj {
    new_str(&text_of(s).to_uppercase())
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_str_lower(s: *mut PyStrObj) -> *mut PyStrObj {
    new_str(&text_of(s).to_lowercase())
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_str_swapcase(s: *mut PyStrObj) -> *mut PyStrObj {
    let out: String = text_of(s)
        .chars()
        .flat_map(|c| -> Vec<char> {
            if c.is_uppercase() {
                c.to_lowercase().collect()
            } else if c.is_lowercase() {
                c.to_uppercase().collect()
            } else {
                vec![c]
            }
        })
        .collect();
    new_str(&out)
}

fn all_chars(s: *mut PyStrObj, pred: impl Fn(char) -> bool) -> i8 {
    let text = text_of(s);
    i8::from(!text.is_empty() && text.chars().all(pred))
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_str_isdigit(s: *mut PyStrObj) -> i8 {
    all_chars(s, |c| c.is_ascii_digit() || (!c.is_ascii() && c.is_numeric()))
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_str_isalpha(s: *mut PyStrObj) -> i8 {
    all_chars(s, char::is_alphabetic)
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_str_isalnum(s: *mut PyStrObj) -> i8 {
    all_chars(s, char::is_alphanumeric)
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_str_isspace(s: *mut PyStrObj) -> i8 {
    all_chars(s, is_py_space)
}

fn cased(s: *mut PyStrObj, want_upper: bool) -> i8 {
    let text = text_of(s);
    let mut any_cased = false;
    for c in text.chars() {
        if c.is_uppercase() || c.is_lowercase() {
            any_cased = true;
            if c.is_uppercase() != want_upper {
                return 0;
            }
        }
    }
    i8::from(any_cased)
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_str_isupper(s: *mut PyStrObj) -> i8 {
    cased(s, true)
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_str_islower(s: *mut PyStrObj) -> i8 {
    cased(s, false)
}

// --------------------------------------------------------------- padding

fn pad(s: *mut PyStrObj, width: i64, left: impl Fn(usize, usize) -> usize, fill: char) -> *mut PyStrObj {
    let text = text_of(s);
    let len = char_len(text);
    let width = inline_int_value(width).unwrap_or(0).max(0) as usize;
    if width <= len {
        return pycc_rt_str_copy(s);
    }
    let marg = width - len;
    let l = left(marg, width);
    let mut out = String::with_capacity(width);
    out.extend(std::iter::repeat_n(fill, l));
    out.push_str(text);
    out.extend(std::iter::repeat_n(fill, marg - l));
    new_str(&out)
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_str_ljust(s: *mut PyStrObj, width: i64) -> *mut PyStrObj {
    pad(s, width, |_, _| 0, ' ')
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_str_rjust(s: *mut PyStrObj, width: i64) -> *mut PyStrObj {
    pad(s, width, |marg, _| marg, ' ')
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_str_center(s: *mut PyStrObj, width: i64) -> *mut PyStrObj {
    // CPython's `pad()`: `left = marg / 2 + (marg & width & 1)`.
    pad(s, width, |marg, width| marg / 2 + (marg & width & 1), ' ')
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_str_zfill(s: *mut PyStrObj, width: i64) -> *mut PyStrObj {
    let text = text_of(s);
    let padded = pad(s, width, |marg, _| marg, '0');
    let sign = match text.as_bytes().first() {
        Some(b'+' | b'-') => &text[..1],
        _ => return padded,
    };
    // The sign moves in front of the zeros.
    let out = text_of(padded).replacen(sign, "", 1);
    unsafe { pycc_rt_str_decref(padded) };
    new_str(&format!("{sign}{out}"))
}

// -------------------------------------------------------- str -> list

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_str_to_list(s: *mut PyStrObj) -> *mut PyListObj {
    let text = text_of(s);
    let mut buf = [0u8; 4];
    str_list(text.chars().map(|c| new_str(c.encode_utf8(&mut buf)) as i64).collect())
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_str_sorted(s: *mut PyStrObj) -> *mut PyListObj {
    let mut chars: Vec<char> = text_of(s).chars().collect();
    chars.sort_unstable();
    let mut buf = [0u8; 4];
    str_list(chars.into_iter().map(|c| new_str(c.encode_utf8(&mut buf)) as i64).collect())
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_str_reversed(s: *mut PyStrObj) -> *mut PyListObj {
    let mut buf = [0u8; 4];
    str_list(text_of(s).chars().rev().map(|c| new_str(c.encode_utf8(&mut buf)) as i64).collect())
}

// ---------------------------------------------------------------- scalars

fn retained(word: i64) -> i64 {
    bigint_retain(word);
    word
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_int_min2(a: i64, b: i64) -> i64 {
    retained(if encoded_int_cmp(b, a).is_lt() { b } else { a })
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_int_max2(a: i64, b: i64) -> i64 {
    retained(if encoded_int_cmp(b, a).is_gt() { b } else { a })
}

/// `min`/`max` of two `bool`s: the arguments arrive as encoded words (the
/// bool markers) and the result is the `i8` a `bool` is.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_bool_min2(a: i64, b: i64) -> i8 {
    i8::from(inline_int_value(a).unwrap_or(0).min(inline_int_value(b).unwrap_or(0)) != 0)
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_bool_max2(a: i64, b: i64) -> i8 {
    i8::from(inline_int_value(a).unwrap_or(0).max(inline_int_value(b).unwrap_or(0)) != 0)
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_float_min2(a: f64, b: f64) -> f64 {
    if b < a { b } else { a }
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_float_max2(a: f64, b: f64) -> f64 {
    if b > a { b } else { a }
}

/// `pow(base, exp, mod)` over machine-sized ints.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_int_powmod(base: i64, exp: i64, modulus: i64) -> i64 {
    let (Some(b), Some(e), Some(m)) = (
        inline_int_value(base),
        inline_int_value(exp),
        inline_int_value(modulus),
    ) else {
        raise_builtin(
            EXCEPTION_TYPE_OVERFLOW_ERROR,
            "OverflowError",
            "three-argument pow() of a bigint-valued int is not supported yet",
        );
        return tag_smallint(0);
    };
    if m == 0 {
        value_error("pow() 3rd argument cannot be 0");
        return tag_smallint(0);
    }
    let m128 = (m as i128).abs();
    let mut b128 = (b as i128).rem_euclid(m128);
    let mut e = e;
    if e < 0 {
        // Modular inverse by the extended Euclidean algorithm.
        let (mut r0, mut r1, mut s0, mut s1) = (m128, b128, 0i128, 1i128);
        while r1 != 0 {
            let q = r0 / r1;
            (r0, r1) = (r1, r0 - q * r1);
            (s0, s1) = (s1, s0 - q * s1);
        }
        if r0 != 1 {
            value_error("base is not invertible for the given modulus");
            return tag_smallint(0);
        }
        b128 = s0.rem_euclid(m128);
        e = -e;
    }
    let mut result: i128 = 1 % m128;
    while e > 0 {
        if e & 1 == 1 {
            result = result * b128 % m128;
        }
        b128 = b128 * b128 % m128;
        e >>= 1;
    }
    // CPython's result takes the modulus's sign.
    if m < 0 && result != 0 {
        result -= m128;
    }
    let result = result as i64;
    fits_smallint(result).unwrap_or_else(|| tag_bigint(bigint_from_i128(result as i128)))
}
