//! `float` division, floor division/modulo, power and `str()` formatting.

use super::*;

/// Python true division rejects both positive and negative zero divisors.
/// D-173 (#382): a zero divisor now sets the pending exception state and
/// returns 0.0 instead of panicking. Generated code checks
/// `pycc_rt_exception_active()` after this call.
pub(crate) fn float_div(a: f64, b: f64) -> f64 {
    if b == 0.0 {
        // D-173: set the pending exception flag instead of panicking.
        raise_builtin(
            EXCEPTION_TYPE_ZERO_DIV_ERROR,
            "ZeroDivisionError",
            "float division by zero",
        );
        return 0.0;
    }
    a / b
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_float_div(a: f64, b: f64) -> f64 {
    float_div(a, b)
}

/// CPython-compatible float floor-division and modulo. Computing both from
/// the same adjusted remainder avoids the off-by-one quotient produced by a
/// naive `(a / b).floor()` for values such as `1.0 // 0.1`.
pub(crate) fn float_divmod(a: f64, b: f64) -> (f64, f64) {
    if b == 0.0 {
        // D-173: set the pending exception flag instead of panicking.
        raise_builtin(
            EXCEPTION_TYPE_ZERO_DIV_ERROR,
            "ZeroDivisionError",
            "float floor division or modulo by zero",
        );
        return (0.0, 0.0);
    }

    let mut modulo = a % b;
    let mut div = (a - modulo) / b;
    if modulo != 0.0 {
        if (b < 0.0) != (modulo < 0.0) {
            modulo += b;
            div -= 1.0;
        }
    } else {
        modulo = 0.0_f64.copysign(b);
    }

    let floordiv = if div != 0.0 {
        let mut floored = div.floor();
        if div - floored > 0.5 {
            floored += 1.0;
        }
        floored
    } else {
        0.0_f64.copysign(a / b)
    };
    (floordiv, modulo)
}

/// Python's `//` on `float`: floors toward negative infinity and snaps the
/// quotient the same way CPython does after floating-point remainder error.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_float_floordiv(a: f64, b: f64) -> f64 {
    float_divmod(a, b).0
}

/// Python's `%` on `float`: result takes the divisor's sign, including
/// signed zero.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_float_floormod(a: f64, b: f64) -> f64 {
    float_divmod(a, b).1
}

/// Python's `**` on `float`: unlike `int_pow`, a negative exponent is
/// perfectly ordinary here (`2.0 ** -1 == 0.5`). But three domains where
/// `f64::powf` silently returns an IEEE-754 special value diverge from
/// Python, which raises or produces a complex result instead (verified against
/// `python3.13`):
/// zero raised to a negative power (`ZeroDivisionError`), a negative base
/// raised to a non-integer power (a complex result -- `pycc` has no
/// complex type), and a finite base/exponent pair whose true result
/// overflows `float` range (`OverflowError`). Part A of #1038 (#1063) turns
/// each into a D-173 raise instead of a panic, matching this crate's
/// division-by-zero convention above; every raising arm returns `0.0` as its
/// sentinel. Generated code *can* observe that sentinel: `Pow` stays in
/// `pycc_codegen::exception::expression_can_set_exception`'s infallible arm,
/// so no check is emitted after the `**` itself and the pending exception is
/// seen only at the next enclosing checkpoint -- a statement or more later, or
/// at program exit. D-244's 2026-09-13 native-mode amendment records that
/// residual and why it is deferred. The sentinel `return` is load-bearing
/// rather than cosmetic: without it the zero-base arm would fall through to
/// `powf`, produce `inf`, and have its `ZeroDivisionError` immediately
/// relabelled `OverflowError` by the third arm in the same call.
pub(crate) fn float_pow(a: f64, b: f64) -> f64 {
    // CPython delegates non-finite exponent/base domains to libm: for
    // example, `(-1.0) ** inf == 1.0`, `0.0 ** -inf == inf`, and
    // `(-inf) ** 0.5 == inf`. The explicit exception/complex guards apply
    // only to finite operands; `fract()` is NaN for an infinite or NaN
    // exponent and would otherwise misclassify those ordinary real results.
    if b.is_finite() {
        if a == 0.0 && b < 0.0 {
            // Conformant: CPython raises `ZeroDivisionError` here, with this
            // exact sentence.
            raise_builtin(
                EXCEPTION_TYPE_ZERO_DIV_ERROR,
                "ZeroDivisionError",
                "0.0 cannot be raised to a negative power",
            );
            return 0.0;
        }
        if a.is_finite() && a < 0.0 && b.fract() != 0.0 {
            // A deliberate deviation: CPython returns a `complex` here, and
            // pycc has no complex type, so there is no conformant class to
            // name. See D-244's 2026-09-13 amendment.
            raise_builtin(
                EXCEPTION_TYPE_RUNTIME_ERROR,
                "RuntimeError",
                "a negative float raised to a non-integer power is not supported yet (would require a complex result)",
            );
            return 0.0;
        }
    }
    let result = a.powf(b);
    if result.is_infinite() && a.is_finite() && b.is_finite() {
        // Conformant: CPython raises `OverflowError` from `errno` here, so
        // its text is `(34, strerror(ERANGE))`.
        raise_builtin(
            EXCEPTION_TYPE_OVERFLOW_ERROR,
            "OverflowError",
            &range_error_text(),
        );
        return 0.0;
    }
    result
}

/// `str()` of the `OverflowError` CPython raises from `errno == ERANGE`
/// (`PyErr_SetFromErrno`): `(34, 'Numerical result out of range')` with
/// glibc, `(34, 'Result too large')` on macOS and Windows.
pub(crate) fn range_error_text() -> String {
    const ERANGE: i32 = 34;
    let text = if cfg!(windows) {
        "Result too large".to_string()
    } else {
        let full = std::io::Error::from_raw_os_error(ERANGE).to_string();
        let suffix = format!(" (os error {ERANGE})");
        full.strip_suffix(&suffix).unwrap_or(&full).to_string()
    };
    format!("({ERANGE}, '{text}')")
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_float_pow(a: f64, b: f64) -> f64 {
    float_pow(a, b)
}

/// Formats a `float` the way CPython's own `str(f)` would (Task 8, reused
/// unchanged by f-string interpolation and Task 10's `print`).
///
/// Verified against `python3.13`'s actual `repr(float)`/`str(float)`
/// (identical since Python 3.1): CPython switches to scientific
/// notation once the value's magnitude is `>= 1e16` or (nonzero and)
/// `< 1e-4`; within that range it always shows at least one digit after
/// the decimal point (`3.0`, never bare `3`, unlike Rust's own `{}`
/// `Display` for `f64`), and `inf`/`-inf`/`nan` are lowercase (Rust's
/// own `Display` capitalizes `NaN`). Outside that range it prints
/// CPython's scientific form (`1e+16`, `1.5e-05`).
pub(crate) fn float_to_str(value: f64) -> *mut PyStrObj {
    new_pystr(float_repr(value).as_bytes())
}

/// `repr(value)`: CPython's shortest round-tripping text.
pub(crate) fn float_repr(value: f64) -> String {
    if value.is_nan() {
        return "nan".to_string();
    }
    if value.is_infinite() {
        return if value > 0.0 { "inf" } else { "-inf" }.to_string();
    }
    if value == 0.0 {
        return if value.is_sign_negative() { "-0.0" } else { "0.0" }.to_string();
    }
    let (digits, exponent) = shortest_digits(value);
    let sign = if value < 0.0 { "-" } else { "" };
    // CPython's `repr` switches to scientific notation below 1e-4 and from
    // 1e16 on, with a signed exponent of at least two digits (`1e+16`,
    // `1.5e-05`); between, it always shows a digit after the point.
    if !(-4..16).contains(&exponent) {
        let (first, rest) = digits.split_at(1);
        let mantissa = if rest.is_empty() { first.to_string() } else { format!("{first}.{rest}") };
        let exp_sign = if exponent < 0 { '-' } else { '+' };
        return format!("{sign}{mantissa}e{exp_sign}{:0>2}", exponent.unsigned_abs());
    }
    if exponent < 0 {
        let zeros = "0".repeat((-exponent - 1) as usize);
        return format!("{sign}0.{zeros}{digits}");
    }
    let point = exponent as usize + 1;
    if digits.len() <= point {
        let zeros = "0".repeat(point - digits.len());
        format!("{sign}{digits}{zeros}.0")
    } else {
        format!("{sign}{}.{}", &digits[..point], &digits[point..])
    }
}

/// The shortest round-tripping significant digits of finite, nonzero
/// `value` and the decimal exponent of the first one, as CPython's
/// `repr` (David Gay's mode 0) chooses them: when two strings of that
/// length round-trip, the one nearest the exact binary value, ties to even
/// (`137931034482758.625` is `...758.62`, where Rust's `{}` says `.63`).
fn shortest_digits(value: f64) -> (String, i32) {
    let shortest = format!("{:e}", value.abs());
    let count = shortest
        .split_once('e')
        .map_or(0, |(mantissa, _)| mantissa.bytes().filter(u8::is_ascii_digit).count());
    // Rust's fixed-precision formatting rounds the exact value, ties to even.
    let exact = format!("{:.*e}", count.saturating_sub(1), value.abs());
    let (mantissa, exponent) = exact.split_once('e').expect("`{:e}` always has an exponent");
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    let digits = digits.trim_end_matches('0');
    let digits = if digits.is_empty() { "0" } else { digits };
    let exponent: i32 = exponent.parse().expect("an integer exponent");
    // Next to a power of two the interval below a float is half the one
    // above it: should the nearest string fall outside, keep the shortest.
    if format!("{digits}e{}", exponent - digits.len() as i32 + 1).parse::<f64>() != Ok(value.abs()) {
        let (mantissa, exponent) = shortest.split_once('e').expect("`{:e}` always has an exponent");
        let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
        return (digits, exponent.parse().expect("an integer exponent"));
    }
    (digits.to_string(), exponent)
}

/// `str(value)` as a new `str` (see `float_to_str`).
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_float_to_str(value: f64) -> *mut PyStrObj {
    float_to_str(value)
}
