//! One printf-style conversion of `str % args` (`PyUnicode_Format`):
//! `conv` is the conversion's text after the `%` -- flags, width,
//! precision and the conversion character, e.g. `-08.3f`. The compiler
//! splits the template into literal text and conversions
//! (`pycc_hir::printf`), so each argument arrives here with its own
//! conversion, already of a type that conversion accepts.

use super::*;

/// The parsed flags, width, precision and conversion character.
struct Conv {
    left: bool,
    plus: bool,
    blank: bool,
    alternate: bool,
    zero: bool,
    width: usize,
    precision: Option<usize>,
    ch: char,
}

fn parse_conv(text: &str) -> Conv {
    let mut conv = Conv {
        left: false,
        plus: false,
        blank: false,
        alternate: false,
        zero: false,
        width: 0,
        precision: None,
        ch: 's',
    };
    let mut chars = text.chars().peekable();
    while let Some(&c) = chars.peek() {
        match c {
            '-' => conv.left = true,
            '+' => conv.plus = true,
            ' ' => conv.blank = true,
            '#' => conv.alternate = true,
            '0' => conv.zero = true,
            _ => break,
        }
        chars.next();
    }
    let number = |chars: &mut std::iter::Peekable<std::str::Chars<'_>>| {
        let mut value = 0usize;
        while let Some(d) = chars.peek().and_then(|c| c.to_digit(10)) {
            value = value.saturating_mul(10).saturating_add(d as usize);
            chars.next();
        }
        value
    };
    conv.width = number(&mut chars);
    if chars.peek() == Some(&'.') {
        chars.next();
        conv.precision = Some(number(&mut chars));
    }
    // A length modifier (`%ld`) is accepted and ignored, as in CPython.
    while matches!(chars.peek(), Some('h' | 'l' | 'L')) {
        chars.next();
    }
    conv.ch = chars.next().unwrap_or('s');
    conv
}

/// Pads `body` (whose sign and numeric prefix, if any, lead it) to the
/// conversion's width (`unicode_format_arg_output`).
fn output(body: &str, conv: &Conv, numeric: bool) -> String {
    let mut chars: Vec<char> = body.chars().collect();
    let fill = if numeric && conv.zero { '0' } else { ' ' };
    let mut sign: Option<char> = None;
    if numeric {
        match chars.first() {
            Some(&c @ ('-' | '+' | ' ')) => {
                sign = Some(c);
                chars.remove(0);
            }
            _ if conv.plus => sign = Some('+'),
            _ if conv.blank => sign = Some(' '),
            _ => {}
        }
    }
    let prefix_len = if numeric && conv.alternate && matches!(conv.ch, 'x' | 'X' | 'o') { 2 } else { 0 };
    let prefix: String = chars.drain(..prefix_len.min(chars.len())).collect();
    let digits: String = chars.into_iter().collect();
    let len = sign.map_or(0, |_| 1) + prefix.chars().count() + digits.chars().count();
    let pad = conv.width.saturating_sub(len);
    let sign = sign.map(String::from).unwrap_or_default();
    if conv.left {
        format!("{sign}{prefix}{digits}{}", " ".repeat(pad))
    } else if fill == '0' {
        format!("{sign}{prefix}{}{digits}", "0".repeat(pad))
    } else {
        format!("{}{sign}{prefix}{digits}", " ".repeat(pad))
    }
}

/// `%d`/`%i`/`%u`/`%o`/`%x`/`%X` of an integer value
/// (`_PyUnicode_FormatLong`).
fn int_body(value: &BigInt, conv: &Conv) -> String {
    let (radix, prefix) = match conv.ch {
        'o' => (8, "0o"),
        'x' => (16, "0x"),
        'X' => (16, "0X"),
        _ => (10, ""),
    };
    let mut digits = value.abs().to_str_radix(radix);
    if conv.ch == 'X' {
        digits = digits.to_uppercase();
    }
    if let Some(precision) = conv.precision
        && precision > digits.len()
    {
        digits = format!("{}{digits}", "0".repeat(precision - digits.len()));
    }
    let prefix = if conv.alternate { prefix } else { "" };
    let sign = if value.is_negative() { "-" } else { "" };
    format!("{sign}{prefix}{digits}")
}

/// A float conversion's body, sign included (`formatfloat`).
fn float_body(value: f64, conv: &Conv) -> String {
    let lower = conv.ch.to_ascii_lowercase();
    let mut text = super::float_digits(value.abs(), lower, conv.precision.unwrap_or(6), conv.alternate, false);
    if conv.ch.is_ascii_uppercase() {
        text = text.to_uppercase();
    }
    if value.is_sign_negative() && !value.is_nan() {
        text.insert(0, '-');
    }
    text
}

fn int_conv(value: &BigInt, conv: &Conv) -> Result<String, Fail> {
    match conv.ch {
        'd' | 'i' | 'u' | 'o' | 'x' | 'X' => Ok(output(&int_body(value, conv), conv, true)),
        'e' | 'E' | 'f' | 'F' | 'g' | 'G' => {
            let as_float = value
                .to_f64()
                .filter(|f| f.is_finite())
                .ok_or_else(|| Fail::overflow("int too large to convert to float"))?;
            Ok(output(&float_body(as_float, conv), conv, true))
        }
        'c' => {
            let ch = value
                .to_u32()
                .filter(|v| *v < 0x110000)
                .and_then(char::from_u32)
                .ok_or_else(|| Fail::overflow("%c arg not in range(0x110000)"))?;
            Ok(output(&ch.to_string(), conv, false))
        }
        // `%s`/`%r`/`%a`: the decimal text.
        _ => Ok(output(&truncate(&value.to_string(), conv), conv, false)),
    }
}

fn truncate(text: &str, conv: &Conv) -> String {
    match conv.precision {
        Some(p) => text.chars().take(p).collect(),
        None => text.to_string(),
    }
}

/// `%`-conversion of an `int`, as a new `str`.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_printf_int(value: i64, conv: *mut PyStrObj) -> *mut PyStrObj {
    let conv = parse_conv(text_of(conv));
    // A bare `%d`/`%i`/`%u` of an inline value is its decimal text.
    if let Some(v) = crate::int_encoding::inline_int_value(value)
        && matches!(conv.ch, 'd' | 'i' | 'u')
        && !(conv.left || conv.plus || conv.blank || conv.alternate || conv.zero)
        && conv.width == 0
        && conv.precision.is_none()
    {
        return finish(Ok(v.to_string()));
    }
    finish(int_conv(&decode_big(value), &conv))
}

/// `%`-conversion of a `bool`: its `int` value, or `True`/`False` for
/// `%s`/`%r`.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_printf_bool(value: i8, conv: *mut PyStrObj) -> *mut PyStrObj {
    let conv = parse_conv(text_of(conv));
    if matches!(conv.ch, 's' | 'r' | 'a') {
        let text = if value != 0 { "True" } else { "False" };
        return finish(Ok(output(&truncate(text, &conv), &conv, false)));
    }
    finish(int_conv(&BigInt::from(i32::from(value != 0)), &conv))
}

/// `%`-conversion of a `float`: `%d` truncates it to an `int` as CPython
/// does, `%x` and `%o` refuse it.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_printf_float(value: f64, conv: *mut PyStrObj) -> *mut PyStrObj {
    let conv = parse_conv(text_of(conv));
    let result = match conv.ch {
        'e' | 'E' | 'f' | 'F' | 'g' | 'G' => Ok(output(&float_body(value, &conv), &conv, true)),
        'd' | 'i' | 'u' => {
            if value.is_nan() {
                Err(Fail::value("cannot convert float NaN to integer"))
            } else if value.is_infinite() {
                Err(Fail::overflow("cannot convert float infinity to integer"))
            } else {
                let truncated = num_bigint::BigInt::from_f64_trunc(value);
                Ok(output(&int_body(&truncated, &conv), &conv, true))
            }
        }
        'o' | 'x' | 'X' => Err(Fail::type_error(format!(
            "%{} format: an integer is required, not float",
            conv.ch
        ))),
        'c' => Err(Fail::type_error("%c requires int or char")),
        _ => Ok(output(&truncate(&crate::float_arith::float_repr(value), &conv), &conv, false)),
    };
    finish(result)
}

/// `%`-conversion of a `str`: `%s`, `%r`/`%a` (its `repr`), or `%c` of a
/// one-character string.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_printf_str(value: *mut PyStrObj, conv: *mut PyStrObj) -> *mut PyStrObj {
    let conv = parse_conv(text_of(conv));
    let text = text_of(value);
    let result = match conv.ch {
        's' => Ok(output(&truncate(text, &conv), &conv, false)),
        'r' | 'a' => {
            let mut repr = String::new();
            crate::list::str_repr(text, &mut repr);
            Ok(output(&truncate(&repr, &conv), &conv, false))
        }
        'c' if text.chars().count() == 1 => Ok(output(text, &conv, false)),
        'c' => Err(Fail::type_error("%c requires int or char")),
        'd' | 'i' | 'u' => Err(Fail::type_error(format!(
            "%{} format: a real number is required, not str",
            conv.ch
        ))),
        'o' | 'x' | 'X' => Err(Fail::type_error(format!(
            "%{} format: an integer is required, not str",
            conv.ch
        ))),
        _ => Err(Fail::type_error("must be real number, not str")),
    };
    finish(result)
}

trait FromF64Trunc {
    fn from_f64_trunc(value: f64) -> Self;
}

impl FromF64Trunc for BigInt {
    fn from_f64_trunc(value: f64) -> Self {
        num_traits::FromPrimitive::from_f64(value.trunc()).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversions_match_cpython() {
        let i = |v: i64, c: &str| int_conv(&BigInt::from(v), &parse_conv(c)).unwrap_or_default();
        assert_eq!(i(-42, "-05d"), "-42  ");
        assert_eq!(i(-42, "08.3d"), "-0000042");
        assert_eq!(i(255, "#05x"), "0x0ff");
        assert_eq!(i(8, "-#8o"), "0o10    ");
        assert_eq!(i(-42, " 05d"), "-0042");
        assert_eq!(i(66, "5c"), "    B");
        let f = |v: f64, c: &str| output(&float_body(v, &parse_conv(c)), &parse_conv(c), true);
        assert_eq!(f(-3.25, "05f"), "-3.250000");
        assert_eq!(f(12345.678, "+e"), "+1.234568e+04");
        assert_eq!(f(-3.25, "#.0f"), "-3.");
        assert_eq!(f(12345.678, "#.0e"), "1.e+04");
        assert_eq!(f(0.000012345, "#g"), "1.23450e-05");
        assert_eq!(f(f64::INFINITY, "05f"), "00inf");
    }

    #[test]
    fn a_bare_decimal_conversion_of_an_inline_int_is_its_text() {
        let text = |word: i64, c: &str| {
            let conv = unsafe { crate::pycc_rt_str_from_literal(c.as_ptr(), c.len() as i64) };
            let s = pycc_rt_printf_int(word, conv);
            String::from_utf8(unsafe { &*s }.bytes().to_vec()).unwrap()
        };
        let small = crate::int_encoding::tag_smallint;
        assert_eq!(text(small(-42), "d"), "-42");
        assert_eq!(text(small(7), "i"), "7");
        assert_eq!(text(small(7), "u"), "7");
        // Flags, width and precision still go through the full conversion.
        assert_eq!(text(small(7), "03d"), "007");
        assert_eq!(text(small(7), "x"), "7");
        let big = crate::int_arith::pycc_rt_int_from_i64(i64::MAX);
        assert_eq!(text(big, "d"), "9223372036854775807");
    }
}
