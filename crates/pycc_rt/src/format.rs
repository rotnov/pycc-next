//! `format(value, spec)` and f-string format specs (`f"{x:>8.3f}"`) over
//! `int`, `bool`, `float` and `str`: CPython's format-specification
//! mini-language (`Python/formatter_unicode.c`), and printf-style `%`
//! formatting of the same values ([`printf`]).
//!
//! Every entry point takes the spec as a `str` and returns a new `str`; an
//! invalid spec raises CPython's own `ValueError` (or `OverflowError` for
//! `%c` out of range) and returns an empty placeholder.

use super::*;
use crate::exception::EXCEPTION_TYPE_TYPE_ERROR;
use num_bigint::BigInt;
use num_traits::{Signed, ToPrimitive};

mod printf;

/// A raised error: the exception type tag, its name and its message.
pub(crate) struct Fail(u8, &'static str, String);

impl Fail {
    fn value(msg: impl Into<String>) -> Self {
        Fail(EXCEPTION_TYPE_VALUE_ERROR, "ValueError", msg.into())
    }
    fn overflow(msg: impl Into<String>) -> Self {
        Fail(EXCEPTION_TYPE_OVERFLOW_ERROR, "OverflowError", msg.into())
    }
    fn type_error(msg: impl Into<String>) -> Self {
        Fail(EXCEPTION_TYPE_TYPE_ERROR, "TypeError", msg.into())
    }
}

/// Raises `fail` or wraps the text, as a new `str`.
fn finish(result: Result<String, Fail>) -> *mut PyStrObj {
    match result {
        Ok(text) => new_pystr(text.as_bytes()),
        Err(Fail(tag, name, msg)) => {
            raise_builtin(tag, name, &msg);
            new_pystr(b"")
        }
    }
}

fn text_of<'a>(s: *mut PyStrObj) -> &'a str {
    // Every `PyStrObj` holds UTF-8.
    unsafe { std::str::from_utf8_unchecked((*s).bytes()) }
}

/// A parsed format spec: `[[fill]align][sign][z][#][0][width][grouping][.precision][type]`.
#[derive(Clone, Copy)]
struct Spec {
    fill: char,
    align: Option<char>,
    sign: Option<char>,
    no_neg_0: bool,
    alternate: bool,
    width: usize,
    grouping: Option<char>,
    precision: Option<usize>,
    ty: Option<char>,
}

/// Parses `spec` for an object of type `type_name`, whose default type
/// code is `default_type` and default alignment `default_align`.
fn parse(
    spec: &str,
    type_name: &str,
    default_type: Option<char>,
    default_align: char,
) -> Result<Spec, Fail> {
    let c: Vec<char> = spec.chars().collect();
    let end = c.len();
    let mut pos = 0;
    let is_align = |ch: char| matches!(ch, '<' | '>' | '=' | '^');
    let mut out = Spec {
        fill: ' ',
        align: None,
        sign: None,
        no_neg_0: false,
        alternate: false,
        width: 0,
        grouping: None,
        precision: None,
        ty: default_type,
    };
    let mut fill_specified = false;
    if end - pos >= 2 && is_align(c[pos + 1]) {
        out.fill = c[pos];
        out.align = Some(c[pos + 1]);
        fill_specified = true;
        pos += 2;
    } else if end - pos >= 1 && is_align(c[pos]) {
        out.align = Some(c[pos]);
        pos += 1;
    }
    if pos < end && matches!(c[pos], '+' | '-' | ' ') {
        out.sign = Some(c[pos]);
        pos += 1;
    }
    if pos < end && c[pos] == 'z' {
        out.no_neg_0 = true;
        pos += 1;
    }
    if pos < end && c[pos] == '#' {
        out.alternate = true;
        pos += 1;
    }
    // The `0` flag: zero fill, sign-aware for numbers.
    if !fill_specified && pos < end && c[pos] == '0' {
        out.fill = '0';
        if out.align.is_none() && default_align == '>' {
            out.align = Some('=');
        }
        pos += 1;
    }
    let digits = |pos: &mut usize| -> Result<Option<usize>, Fail> {
        let start = *pos;
        let mut value: usize = 0;
        while *pos < end && c[*pos].is_ascii_digit() {
            value = value
                .checked_mul(10)
                .and_then(|v| v.checked_add(c[*pos] as usize - '0' as usize))
                .ok_or_else(|| Fail::value("Too many decimal digits in format string"))?;
            *pos += 1;
        }
        Ok((*pos > start).then_some(value))
    };
    out.width = digits(&mut pos)?.unwrap_or(0);
    let both = || Fail::value("Cannot specify both ',' and '_'.");
    if pos < end && c[pos] == ',' {
        out.grouping = Some(',');
        pos += 1;
    }
    if pos < end && c[pos] == '_' {
        if out.grouping.is_some() {
            return Err(both());
        }
        out.grouping = Some('_');
        pos += 1;
    }
    if pos < end && c[pos] == ',' && out.grouping == Some('_') {
        return Err(both());
    }
    if pos < end && c[pos] == '.' {
        pos += 1;
        match digits(&mut pos)? {
            Some(p) => out.precision = Some(p),
            None => return Err(Fail::value("Format specifier missing precision")),
        }
    }
    if end - pos > 1 {
        return Err(Fail::value(format!(
            "Invalid format specifier '{spec}' for object of type '{type_name}'"
        )));
    }
    if end - pos == 1 {
        out.ty = Some(c[pos]);
    }
    if let Some(sep) = out.grouping {
        let allowed = match out.ty {
            None | Some('d' | 'e' | 'f' | 'g' | 'E' | 'G' | '%' | 'F') => true,
            Some('b' | 'o' | 'x' | 'X') => sep == '_',
            _ => false,
        };
        if !allowed {
            let ty = out.ty.unwrap_or('\0');
            return Err(Fail::value(if (' '..'\u{7f}').contains(&ty) {
                format!("Cannot specify '{sep}' with '{ty}'.")
            } else {
                format!("Cannot specify '{sep}' with '\\x{:x}'.", ty as u32)
            }));
        }
    }
    Ok(out)
}

fn unknown_code(ty: char, type_name: &str) -> Fail {
    if (' '..'\u{7f}').contains(&ty) {
        Fail::value(format!("Unknown format code '{ty}' for object of type '{type_name}'"))
    } else {
        Fail::value(format!(
            "Unknown format code '\\x{:x}' for object of type '{type_name}'",
            ty as u32
        ))
    }
}

/// `digits` with `sep` every `size` digits from the right, zero-padded to
/// at least `min_width` characters (CPython's
/// `_PyUnicode_InsertThousandsGrouping`).
fn group(digits: &str, min_width: isize, sep: char, size: isize) -> String {
    let chars: Vec<char> = digits.chars().collect();
    let mut remaining = chars.len() as isize;
    let mut min_width = min_width;
    let mut out: Vec<char> = Vec::new();
    let mut use_sep = false;
    loop {
        let l = size.min(remaining.max(min_width).max(1));
        let zeros = (l - remaining).max(0);
        let n = remaining.min(l).max(0);
        if use_sep {
            out.push(sep);
        }
        for i in 0..n {
            out.push(chars[(remaining - 1 - i) as usize]);
        }
        out.extend(std::iter::repeat_n('0', zeros as usize));
        remaining -= n;
        min_width -= l;
        if remaining <= 0 && min_width <= 0 {
            break;
        }
        min_width -= 1;
        use_sep = true;
    }
    out.iter().rev().collect()
}

/// Lays out a number: `sign` and `prefix`, the integer `digits` (grouped
/// per the spec) and the `rest` (fraction, exponent, `%`), padded to the
/// spec's width.
fn layout_number(sign: &str, prefix: &str, digits: &str, rest: &str, spec: &Spec, group_size: isize) -> String {
    let digits = match spec.grouping {
        Some(sep) => {
            let min_width = if spec.fill == '0' && spec.align == Some('=') {
                spec.width as isize
                    - (sign.chars().count() + prefix.chars().count() + rest.chars().count()) as isize
            } else {
                0
            };
            group(digits, min_width, sep, group_size)
        }
        None => digits.to_string(),
    };
    let body_len = sign.chars().count() + prefix.chars().count() + digits.chars().count() + rest.chars().count();
    let pad = spec.width.saturating_sub(body_len);
    let fill = |n: usize| spec.fill.to_string().repeat(n);
    match spec.align.unwrap_or('>') {
        '<' => format!("{sign}{prefix}{digits}{rest}{}", fill(pad)),
        '^' => format!("{}{sign}{prefix}{digits}{rest}{}", fill(pad / 2), fill(pad - pad / 2)),
        '=' => format!("{sign}{prefix}{}{digits}{rest}", fill(pad)),
        _ => format!("{}{sign}{prefix}{digits}{rest}", fill(pad)),
    }
}

/// The sign text for a value, per the spec's sign option.
fn sign_text(negative: bool, sign: Option<char>) -> &'static str {
    match (negative, sign) {
        (true, _) => "-",
        (false, Some('+')) => "+",
        (false, Some(' ')) => " ",
        _ => "",
    }
}

/// `format(value, spec)` of an `int` (`type_name` is `int` or `bool`).
fn format_int(value: &BigInt, spec_text: &str, type_name: &str) -> Result<String, Fail> {
    let spec = parse(spec_text, type_name, Some('d'), '>')?;
    let ty = spec.ty.unwrap_or('d');
    match ty {
        'b' | 'c' | 'd' | 'o' | 'x' | 'X' | 'n' => {}
        'e' | 'E' | 'f' | 'F' | 'g' | 'G' | '%' => {
            let as_float = value.to_f64().filter(|f| f.is_finite()).ok_or_else(|| {
                Fail::overflow("int too large to convert to float")
            })?;
            return format_float_spec(as_float, &spec);
        }
        other => return Err(unknown_code(other, type_name)),
    }
    if spec.precision.is_some() {
        return Err(Fail::value("Precision not allowed in integer format specifier"));
    }
    if spec.no_neg_0 {
        return Err(Fail::value(
            "Negative zero coercion (z) not allowed in integer format specifier",
        ));
    }
    if ty == 'c' {
        if spec.sign.is_some() {
            return Err(Fail::value("Sign not allowed with integer format specifier 'c'"));
        }
        if spec.alternate {
            return Err(Fail::value(
                "Alternate form (#) not allowed with integer format specifier 'c'",
            ));
        }
        let ch = value
            .to_u32()
            .filter(|v| *v < 0x110000)
            .and_then(char::from_u32)
            .ok_or_else(|| Fail::overflow("%c arg not in range(0x110000)"))?;
        return Ok(layout_number("", "", &ch.to_string(), "", &spec, 3));
    }
    let (radix, prefix) = match ty {
        'b' => (2, "0b"),
        'o' => (8, "0o"),
        'x' => (16, "0x"),
        'X' => (16, "0X"),
        _ => (10, ""),
    };
    let mut digits = value.abs().to_str_radix(radix);
    if ty == 'X' {
        digits = digits.to_uppercase();
    }
    let prefix = if spec.alternate { prefix } else { "" };
    let sign = sign_text(value.is_negative(), spec.sign);
    let group_size = if radix == 10 { 3 } else { 4 };
    Ok(layout_number(sign, prefix, &digits, "", &spec, group_size))
}

/// The digits of `magnitude` (finite or not, non-negative) rendered like
/// `PyOS_double_to_string` with `ty` in `e f g r %` (lower case) and
/// `precision`: `alternate` keeps a trailing point and zeros, `add_dot_0`
/// gives an integral fixed-point result a `.0`.
pub(crate) fn float_digits(magnitude: f64, ty: char, precision: usize, alternate: bool, add_dot_0: bool) -> String {
    if magnitude.is_nan() {
        return "nan".to_string();
    }
    if magnitude.is_infinite() {
        return "inf".to_string();
    }
    let exponential = |p: usize| {
        let text = format!("{magnitude:.p$e}");
        let (mantissa, exponent) = text.split_once('e').expect("`{:e}` always has an exponent");
        let (sign, digits) = match exponent.strip_prefix('-') {
            Some(digits) => ('-', digits),
            None => ('+', exponent),
        };
        let point = if alternate && !mantissa.contains('.') { "." } else { "" };
        format!("{mantissa}{point}e{sign}{digits:0>2}")
    };
    match ty {
        'f' => {
            let text = format!("{magnitude:.precision$}");
            if alternate && precision == 0 { format!("{text}.") } else { text }
        }
        'e' => exponential(precision),
        'g' => {
            let p = precision.max(1);
            let probe = format!("{magnitude:.prec$e}", prec = p - 1);
            let exponent: i64 = probe
                .split_once('e')
                .and_then(|(_, e)| e.parse().ok())
                .expect("`{:e}` always has an integer exponent");
            // With `add_dot_0` (no type code, a precision) the exponent form
            // starts one digit earlier (`PyOS_double_to_string`'s
            // `Py_DTSF_ADD_DOT_0`): `format(123.0, ".3")` is `1.23e+02`.
            let fixed_below = if add_dot_0 { p as i64 - 1 } else { p as i64 };
            let mut text = if (-4..fixed_below).contains(&exponent) {
                let decimals = (p as i64 - 1 - exponent) as usize;
                let fixed = format!("{magnitude:.decimals$}");
                if alternate && decimals == 0 { format!("{fixed}.") } else { fixed }
            } else {
                exponential(p - 1)
            };
            if !alternate {
                text = strip_zeros(&text);
            }
            if add_dot_0 && !text.contains(['.', 'e']) {
                text.push_str(".0");
            }
            text
        }
        // `repr`, sign removed by the caller's magnitude; the alternate form
        // keeps a point in the mantissa (`format(1e16, "#")` is `1.e+16`).
        _ => {
            let text = crate::float_arith::float_repr(magnitude);
            match text.find('e') {
                Some(at) if alternate && !text[..at].contains('.') => {
                    format!("{}.{}", &text[..at], &text[at..])
                }
                _ => text,
            }
        }
    }
}

/// Removes trailing fractional zeros (and a then-trailing point) from the
/// mantissa of `text`.
fn strip_zeros(text: &str) -> String {
    let (mantissa, exponent) = match text.find('e') {
        Some(at) => text.split_at(at),
        None => (text, ""),
    };
    let mantissa = if mantissa.contains('.') {
        mantissa.trim_end_matches('0').trim_end_matches('.')
    } else {
        mantissa
    };
    format!("{mantissa}{exponent}")
}

fn format_float_spec(value: f64, spec: &Spec) -> Result<String, Fail> {
    let (ty, precision, add_dot_0) = match spec.ty {
        None => match spec.precision {
            None => ('r', 0, true),
            Some(p) => ('g', p, true),
        },
        Some(t @ ('e' | 'E' | 'f' | 'F' | 'g' | 'G' | '%')) => (t, spec.precision.unwrap_or(6), false),
        Some('n') => ('g', spec.precision.unwrap_or(6), false),
        Some(other) => return Err(unknown_code(other, "float")),
    };
    let percent = ty == '%';
    let shown = if percent { value * 100.0 } else { value };
    let lower = match ty {
        '%' => 'f',
        other => other.to_ascii_lowercase(),
    };
    let mut text = float_digits(shown.abs(), lower, precision, spec.alternate, add_dot_0);
    if ty.is_ascii_uppercase() {
        text = text.to_uppercase();
    }
    let mut negative = shown.is_sign_negative() && !shown.is_nan();
    if negative && spec.no_neg_0 && !text.chars().any(|c| c.is_ascii_digit() && c != '0') && !shown.is_infinite() {
        negative = false;
    }
    if percent {
        text.push('%');
    }
    let split = text.find(|c: char| !c.is_ascii_digit()).unwrap_or(text.len());
    let (digits, rest) = text.split_at(split);
    let sign = sign_text(negative, spec.sign);
    if !shown.is_finite() {
        // `inf`/`nan` have no digits to group.
        let ungrouped = Spec { grouping: None, ..*spec };
        return Ok(layout_number(sign, "", digits, rest, &ungrouped, 3));
    }
    Ok(layout_number(sign, "", digits, rest, spec, 3))
}

fn format_float(value: f64, spec_text: &str) -> Result<String, Fail> {
    if spec_text.is_empty() {
        return Ok(crate::float_arith::float_repr(value));
    }
    let spec = parse(spec_text, "float", None, '>')?;
    format_float_spec(value, &spec)
}

fn format_str(text: &str, spec_text: &str) -> Result<String, Fail> {
    let spec = parse(spec_text, "str", Some('s'), '<')?;
    match spec.ty {
        Some('s') | None => {}
        Some(other) => return Err(unknown_code(other, "str")),
    }
    match spec.sign {
        Some(' ') => return Err(Fail::value("Space not allowed in string format specifier")),
        Some(_) => return Err(Fail::value("Sign not allowed in string format specifier")),
        None => {}
    }
    if spec.no_neg_0 {
        return Err(Fail::value(
            "Negative zero coercion (z) not allowed in string format specifier",
        ));
    }
    if spec.alternate {
        return Err(Fail::value("Alternate form (#) not allowed in string format specifier"));
    }
    if spec.align == Some('=') {
        return Err(Fail::value("'=' alignment not allowed in string format specifier"));
    }
    let body: String = match spec.precision {
        Some(p) => text.chars().take(p).collect(),
        None => text.to_string(),
    };
    let len = body.chars().count();
    let pad = spec.width.saturating_sub(len);
    let fill = |n: usize| spec.fill.to_string().repeat(n);
    Ok(match spec.align.unwrap_or('<') {
        '>' => format!("{}{body}", fill(pad)),
        '^' => format!("{}{body}{}", fill(pad / 2), fill(pad - pad / 2)),
        _ => format!("{body}{}", fill(pad)),
    })
}

/// `format(value, spec)` of an `int`, as a new `str`.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_format_int(value: i64, spec: *mut PyStrObj) -> *mut PyStrObj {
    let spec = text_of(spec);
    if spec.is_empty() {
        return int_to_str(value);
    }
    finish(format_int(&decode_big(value), spec, "int"))
}

/// `format(value, spec)` of a `bool`, as a new `str`: `str(value)` for an
/// empty spec, else the `int` formatting of `0`/`1`.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_format_bool(value: i8, spec: *mut PyStrObj) -> *mut PyStrObj {
    let spec = text_of(spec);
    if spec.is_empty() {
        return pycc_rt_bool_to_str(value);
    }
    finish(format_int(&BigInt::from(i32::from(value != 0)), spec, "bool"))
}

/// `format(value, spec)` of a `float`, as a new `str`.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_format_float(value: f64, spec: *mut PyStrObj) -> *mut PyStrObj {
    finish(format_float(value, text_of(spec)))
}

/// `format(value, spec)` of a `str`, as a new `str`.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_format_str(value: *mut PyStrObj, spec: *mut PyStrObj) -> *mut PyStrObj {
    finish(format_str(text_of(value), text_of(spec)))
}

/// `str()` of an `Optional[int]`: `None` when absent.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_optional_int_to_str(value: i64, present: i8) -> *mut PyStrObj {
    if present == 0 { new_pystr(b"None") } else { int_to_str(value) }
}

/// `str()` of an `Optional[float]`: `None` when absent.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_optional_float_to_str(value: f64, present: i8) -> *mut PyStrObj {
    if present == 0 {
        new_pystr(b"None")
    } else {
        new_pystr(crate::float_arith::float_repr(value).as_bytes())
    }
}

/// `str()` of an `Optional[bool]`: `None` when absent.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_optional_bool_to_str(value: i8, present: i8) -> *mut PyStrObj {
    if present == 0 { new_pystr(b"None") } else { pycc_rt_bool_to_str(value) }
}

/// `str()` of a pointer-represented `T | None` (a `str`, a container or a
/// tuple object, whose word is null for `None`): a new `str`.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_optional_word_to_str(word: i64, kind: i64) -> *mut PyStrObj {
    if word == 0 {
        return new_pystr(b"None");
    }
    if kind == crate::list::LIST_KIND_STR {
        let s = word as *mut PyStrObj;
        unsafe { crate::pycc_rt_str_incref(s) };
        return s;
    }
    let mut out = String::new();
    crate::value::repr(kind, word, &mut out);
    new_pystr(out.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pointer_optional_renders_none_or_its_value() {
        let text = |s: *mut PyStrObj| String::from_utf8_lossy(unsafe { &*s }.bytes()).into_owned();
        assert_eq!(text(pycc_rt_optional_word_to_str(0, crate::list::LIST_KIND_STR)), "None");
        let abc = new_pystr(b"abc");
        assert_eq!(text(pycc_rt_optional_word_to_str(abc as i64, crate::list::LIST_KIND_STR)), "abc");
        let xs = crate::list::pycc_rt_list_new(crate::list::LIST_KIND_INT, 0);
        unsafe { crate::list::pycc_rt_list_append(xs, crate::tag_smallint(4)) };
        assert_eq!(text(pycc_rt_optional_word_to_str(xs as i64, crate::list::LIST_KIND_LIST)), "[4]");
    }

    fn int(v: i64, spec: &str) -> String {
        format_int(&BigInt::from(v), spec, "int").unwrap_or_else(|Fail(_, _, m)| m)
    }

    fn float(v: f64, spec: &str) -> String {
        format_float(v, spec).unwrap_or_else(|Fail(_, _, m)| m)
    }

    #[test]
    fn ints_follow_the_mini_language() {
        assert_eq!(int(1234, "012,"), "0,000,001,234");
        assert_eq!(int(1234, "0>12,"), "00000001,234");
        assert_eq!(int(-5, "#010b"), "-0b0000101");
        assert_eq!(int(255, "_x"), "ff");
        assert_eq!(int(65, "c"), "A");
        assert_eq!(int(7, ".2"), "Precision not allowed in integer format specifier");
        assert_eq!(int(7, "q"), "Unknown format code 'q' for object of type 'int'");
        assert_eq!(int(7, ",x"), "Cannot specify ',' with 'x'.");
    }

    #[test]
    fn floats_follow_the_mini_language() {
        assert_eq!(float(2.5, ".9f"), "2.500000000");
        assert_eq!(float(f64::INFINITY, "010"), "0000000inf");
        assert_eq!(float(2.0, ".3"), "2.0");
        assert_eq!(float(1e16, ""), "1e+16");
        assert_eq!(float(12345.678, ","), "12,345.678");
        assert_eq!(float(2.5, ".0e"), "2e+00");
        assert_eq!(float(1e-5, "g"), "1e-05");
        assert_eq!(float(-0.0, "z.1f"), "0.0");
        assert_eq!(float(0.125, ".1%"), "12.5%");
    }

    #[test]
    fn strs_pad_and_truncate() {
        let s = |t: &str, spec: &str| format_str(t, spec).unwrap_or_else(|Fail(_, _, m)| m);
        assert_eq!(s("ab", "05"), "ab000");
        assert_eq!(s("abcdef", ">5.3"), "  abc");
        assert_eq!(s("ab", "+"), "Sign not allowed in string format specifier");
    }
}
