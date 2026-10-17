//! `float` division, floor division/modulo and power.

use super::*;

#[test]
fn pycc_rt_int_to_float_converts_the_untagged_value() {
    assert_eq!(pycc_rt_int_to_float(tag_smallint(5)), 5.0);
    assert_eq!(pycc_rt_int_to_float(tag_smallint(-3)), -3.0);
}

#[test]
fn int_untag_checked_recovers_the_original_value() {
    // Calls the public `extern "C"` wrapper (not the private
    // `int_untag_checked`) on purpose: this is the non-panicking path,
    // so no unwind ever crosses that boundary, and it is what covers
    // the wrapper's own line -- the same split this file's `int_add`
    // implementation note establishes for every panicking
    // `pycc_rt_int_*` symbol.
    for n in [0i64, 1, -1, 42, -42, i64::MAX >> 1, -(i64::MAX >> 1)] {
        assert_eq!(pycc_rt_int_untag_checked(tag_smallint(n), UNTAG_SIZE), n);
    }
}

#[test]
fn pycc_rt_float_floordiv_matches_python_floor_semantics() {
    assert_eq!(pycc_rt_float_floordiv(7.0, 2.0), 3.0);
    assert_eq!(pycc_rt_float_floordiv(-7.0, 2.0), -4.0);
    assert_eq!(pycc_rt_float_floordiv(1.0, 0.1), 9.0);

    // Exercises CPython's quotient snap-up correction: the intermediate
    // quotient is -5603572390.000001, whose `floor()` alone is one low.
    assert_eq!(
        pycc_rt_float_floordiv(
            f64::from_bits(0x8ec2_3615_82f2_e770),
            f64::from_bits(0x0cbb_eab0_bc9a_0e0c),
        ),
        -5_603_572_390.0
    );

    let negative_zero = pycc_rt_float_floordiv(-0.0, 2.0);
    assert_eq!(negative_zero, 0.0);
    assert!(negative_zero.is_sign_negative());
}

#[test]
fn pycc_rt_float_div_computes_a_nonzero_division() {
    assert_eq!(pycc_rt_float_div(7.0, 2.0), 3.5);
}

#[test]
fn float_div_rejects_a_zero_divisor() {
    // D-173: float division by zero now sets the exception flag
    // instead of panicking.
    pycc_rt_exception_clear();
    let result = float_div(1.0, -0.0);
    assert_eq!(pycc_rt_exception_active(), 1);
    assert_eq!(result, 0.0);
    pycc_rt_exception_clear();
}

#[test]
fn float_divmod_rejects_a_zero_divisor() {
    // D-173: float floor division/modulo by zero now sets the
    // exception flag instead of panicking.
    pycc_rt_exception_clear();
    let (q, r) = float_divmod(1.0, 0.0, "float modulo by zero");
    assert_eq!(pycc_rt_exception_active(), 1);
    assert_eq!(q, 0.0);
    assert_eq!(r, 0.0);
    pycc_rt_exception_clear();
}

#[test]
fn pycc_rt_float_floormod_matches_python_floor_semantics() {
    assert_eq!(pycc_rt_float_floormod(-7.0, 2.0), 1.0);
    assert_eq!(pycc_rt_float_floormod(7.0, 2.0), 1.0);
    let negative_zero = pycc_rt_float_floormod(8.0, -2.0);
    assert_eq!(negative_zero, 0.0);
    assert!(negative_zero.is_sign_negative());
}

#[test]
fn pycc_rt_float_pow_computes_the_correct_power() {
    assert_eq!(pycc_rt_float_pow(2.0, 10.0), 1024.0);
    assert_eq!(pycc_rt_float_pow(9.0, 0.5), 3.0);
    assert_eq!(pycc_rt_float_pow(2.0, -1.0), 0.5);
    assert_eq!(pycc_rt_float_pow(-1.0, f64::INFINITY), 1.0);
    assert_eq!(pycc_rt_float_pow(-2.0, f64::NEG_INFINITY), 0.0);
    assert_eq!(pycc_rt_float_pow(0.0, f64::NEG_INFINITY), f64::INFINITY);
    assert_eq!(pycc_rt_float_pow(f64::NEG_INFINITY, 0.5), f64::INFINITY);
    assert!(pycc_rt_float_pow(-1.0, f64::NAN).is_nan());
}

#[test]
fn pycc_rt_float_pow_accepts_a_negative_base_with_an_integer_exponent() {
    // An integer exponent (positive or negative) always yields a real
    // result, matching `python3.13`: `(-2.0) ** 3 == -8.0`,
    // `(-2.0) ** -3 == -0.125`, `(-2.0) ** 2 == 4.0`.
    assert_eq!(pycc_rt_float_pow(-2.0, 3.0), -8.0);
    assert_eq!(pycc_rt_float_pow(-2.0, -3.0), -0.125);
    assert_eq!(pycc_rt_float_pow(-2.0, 2.0), 4.0);
}

#[test]
fn pycc_rt_float_pow_accepts_a_tiny_finite_result() {
    // Verified against `python3.13`: `2.0 ** -1024.0` produces a tiny
    // finite positive value, no exception -- only *overflow* to infinity
    // is a Python `OverflowError`, not a representable subnormal result.
    let result = pycc_rt_float_pow(2.0, -1024.0);
    assert!(result > 0.0 && result < 1e-300);
}

#[test]
fn float_pow_raises_zero_division_error_for_zero_to_a_negative_power() {
    // Verified against `python3.13`: `0.0 ** -1.0` raises
    // `ZeroDivisionError: 0.0 cannot be raised to a negative power`,
    // while `f64::powf` alone silently returns `inf`. Part A of #1038
    // (#1063) turned the former `panic!` into this D-173 raise. Calls the
    // private `float_pow` directly, not the public `extern "C"` wrapper,
    // keeping this module's convention for the passing arms; the raise
    // itself would now cross the wrapper safely.
    pycc_rt_exception_clear();
    let result = float_pow(0.0, -1.0);
    let (tag, message) = pending_tag_and_message();
    assert_eq!(tag, EXCEPTION_TYPE_ZERO_DIV_ERROR);
    // CPython's own sentence, verbatim -- the `pycc_rt: ` prefix the
    // panic carried is gone, since a raised message is user-visible.
    assert_eq!(message, "0.0 cannot be raised to a negative power");
    assert_eq!(result, 0.0); // sentinel value
    pycc_rt_exception_clear();
}

#[test]
fn float_pow_raises_zero_division_error_for_negative_zero_to_a_negative_power() {
    // `-0.0 == 0.0` under IEEE-754, and `python3.13` raises the same
    // `ZeroDivisionError` for `(-0.0) ** -1.0` as for `0.0 ** -1.0`.
    pycc_rt_exception_clear();
    let result = float_pow(-0.0, -1.0);
    let (tag, message) = pending_tag_and_message();
    assert_eq!(tag, EXCEPTION_TYPE_ZERO_DIV_ERROR);
    assert_eq!(message, "0.0 cannot be raised to a negative power");
    assert_eq!(result, 0.0);
    pycc_rt_exception_clear();
}

#[test]
fn float_pow_raises_runtime_error_for_a_negative_base_and_a_non_integer_power() {
    // Verified against `python3.13`: `(-2.0) ** 3.5` returns a complex
    // number (`pycc` has no complex type), while `f64::powf` alone
    // silently returns `NaN`. `RuntimeError` is the deliberate deviation
    // -- there is no conformant class, because CPython raises nothing.
    pycc_rt_exception_clear();
    let result = float_pow(-2.0, 3.5);
    let (tag, message) = pending_tag_and_message();
    assert_eq!(tag, EXCEPTION_TYPE_RUNTIME_ERROR);
    assert_eq!(
        message,
        "a negative float raised to a non-integer power is not supported yet (would require a complex result)"
    );
    assert_eq!(result, 0.0);
    pycc_rt_exception_clear();
}

#[test]
fn float_pow_raises_overflow_error_for_a_finite_result_outside_float_range() {
    // Verified against `python3.13`: `2.0 ** 1024.0` raises
    // `OverflowError: (34, 'Numerical result out of range')` (glibc's
    // `strerror(ERANGE)`; `'Result too large'` on macOS), while `f64::powf`
    // alone silently returns `inf`.
    pycc_rt_exception_clear();
    let result = float_pow(2.0, 1024.0);
    let (tag, message) = pending_tag_and_message();
    assert_eq!(tag, EXCEPTION_TYPE_OVERFLOW_ERROR);
    let expected = if cfg!(target_os = "linux") {
        "(34, 'Numerical result out of range')"
    } else {
        "(34, 'Result too large')"
    };
    assert_eq!(message, expected);
    assert_eq!(result, 0.0);
    pycc_rt_exception_clear();
}

/// Part A of #1038 (#1063), §4b-bis: `raise_builtin` installs pending
/// state unconditionally, so a later raise on the same thread relabels an
/// earlier one rather than being suppressed by it. The zero-base arm's
/// `return` is what keeps that from happening *inside* one `float_pow`
/// call -- without it, `0.0 ** -1.0` would fall through to `powf`,
/// produce `inf`, and have its `ZeroDivisionError` overwritten by the
/// overflow arm. Across *statements* the relabelling is reachable in
/// generated code too, precisely because `Pow` emits no checkpoint of its
/// own: two consecutive `**` assignments with no fallible expression
/// between them leave only the second raise pending. That is the
/// native-mode residual D-244's 2026-09-13 amendment defers, not a
/// property this test claims away; what the `return` guarantees is the
/// narrower thing pinned here -- that no single `float_pow` call can
/// relabel its own raise.
#[test]
fn a_later_pow_raise_relabels_an_earlier_one_but_never_within_one_call() {
    pycc_rt_exception_clear();
    assert_eq!(float_pow(0.0, -1.0), 0.0);
    assert_eq!(pending_tag_and_message().0, EXCEPTION_TYPE_ZERO_DIV_ERROR);
    assert_eq!(float_pow(2.0, 1024.0), 0.0);
    assert_eq!(pending_tag_and_message().0, EXCEPTION_TYPE_OVERFLOW_ERROR);
    pycc_rt_exception_clear();
}
