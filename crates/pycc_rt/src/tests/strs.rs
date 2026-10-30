//! `str` objects, `str()` of scalars and the print primitives.

use super::*;

#[test]
fn a_short_literal_round_trips_through_the_inline_representation() {
    // Every `pycc_rt_str_*` function is `unsafe extern "C"` (see their
    // own doc comments' `# Safety` sections) -- a genuinely public,
    // pointer-taking FFI entry point, unlike every plain-`i64`
    // `pycc_rt_int_*` function above, so calling one directly (even
    // from this crate's own same-binary Rust tests) needs an explicit
    // `unsafe` block, upholding each call's own documented precondition.
    unsafe {
        let bytes = b"hi";
        let s = pycc_rt_str_from_literal(bytes.as_ptr(), bytes.len() as i64);
        assert_eq!((*s).bytes(), b"hi");
        pycc_rt_str_decref(s);
    }
}

#[test]
fn a_long_literal_round_trips_through_the_heap_representation() {
    unsafe {
        let long = "x".repeat(23); // one byte past the 22-byte inline cap (D-059)
        let s = pycc_rt_str_from_literal(long.as_ptr(), long.len() as i64);
        assert_eq!((*s).bytes(), long.as_bytes());
        pycc_rt_str_decref(s);
    }
}

#[test]
fn the_ext_str_accessor_exposes_the_bytes_of_both_payload_representations() {
    // The D-244 hosted `ext` shim reads a returned `str` only through
    // this accessor, so it has to see through both arms of the D-059
    // inline/heap split -- the shim has no way to tell them apart and
    // must not care. An embedded NUL and the empty string are covered
    // here too, because the accessor's length-out contract is what makes
    // both round-trip on the boundary instead of truncating at `strlen`.
    unsafe {
        let mut len = usize::MAX;

        let inline = pycc_rt_str_from_literal(b"hi\0there".as_ptr(), 8);
        let ptr = pycc_rt_ext_str_bytes(inline, &raw mut len);
        assert_eq!(len, 8);
        assert_eq!(std::slice::from_raw_parts(ptr, len), b"hi\0there");
        pycc_rt_str_decref(inline);

        let long = "x".repeat(23); // one byte past the 22-byte inline cap (D-059)
        let heap = pycc_rt_str_from_literal(long.as_ptr(), long.len() as i64);
        let ptr = pycc_rt_ext_str_bytes(heap, &raw mut len);
        assert_eq!(len, 23);
        assert_eq!(std::slice::from_raw_parts(ptr, len), long.as_bytes());
        pycc_rt_str_decref(heap);

        let empty = pycc_rt_str_from_literal(b"".as_ptr(), 0);
        let ptr = pycc_rt_ext_str_bytes(empty, &raw mut len);
        assert_eq!(len, 0);
        assert!(!ptr.is_null());
        pycc_rt_str_decref(empty);
    }
}

#[test]
fn concat_joins_bytes_from_both_operands() {
    unsafe {
        let a = pycc_rt_str_from_literal(b"foo".as_ptr(), 3);
        let b = pycc_rt_str_from_literal(b"bar".as_ptr(), 3);
        let joined = pycc_rt_str_concat(a, b);
        assert_eq!((*joined).bytes(), b"foobar");
        pycc_rt_str_decref(a);
        pycc_rt_str_decref(b);
        pycc_rt_str_decref(joined);
    }
}

#[test]
fn repeat_concatenates_the_operand_count_times() {
    unsafe {
        let s = pycc_rt_str_from_literal(b"ab".as_ptr(), 2);
        let thrice = pycc_rt_str_repeat(s, 3);
        assert_eq!((*thrice).bytes(), b"ababab");
        let once = pycc_rt_str_repeat(s, 1);
        assert_eq!((*once).bytes(), b"ab");
        // Past D-059's 22-byte inline cap, so the heap payload branch of
        // `new_pystr` is exercised through repetition too, not only
        // through a long literal.
        let long = pycc_rt_str_repeat(s, 12);
        assert_eq!((*long).bytes(), "ab".repeat(12).as_bytes());
        pycc_rt_str_decref(s);
        pycc_rt_str_decref(thrice);
        pycc_rt_str_decref(once);
        pycc_rt_str_decref(long);
    }
}

#[test]
fn repeat_yields_the_empty_string_for_a_non_positive_count() {
    // CPython: `"ab" * 0` and `"ab" * -3` are both `""`. The negative
    // case is reachable from real source (`n = 0 - 3`) even though a
    // `-3` literal is not yet lowered (#573).
    unsafe {
        let s = pycc_rt_str_from_literal(b"ab".as_ptr(), 2);
        let zero = pycc_rt_str_repeat(s, 0);
        let negative = pycc_rt_str_repeat(s, -3);
        assert_eq!((*zero).bytes(), b"");
        assert_eq!((*negative).bytes(), b"");
        pycc_rt_str_decref(s);
        pycc_rt_str_decref(zero);
        pycc_rt_str_decref(negative);
    }
}

#[test]
fn cmp_orders_strings_lexicographically() {
    unsafe {
        let a = pycc_rt_str_from_literal(b"apple".as_ptr(), 5);
        let b = pycc_rt_str_from_literal(b"banana".as_ptr(), 6);
        assert_eq!(pycc_rt_str_cmp(a, a), 0);
        assert_eq!(pycc_rt_str_cmp(a, b), -1);
        assert_eq!(pycc_rt_str_cmp(b, a), 1);
        pycc_rt_str_decref(a);
        pycc_rt_str_decref(b);
    }
}

#[test]
fn truthy_is_false_only_for_the_empty_string() {
    unsafe {
        let empty = pycc_rt_str_from_literal(b"".as_ptr(), 0);
        let non_empty = pycc_rt_str_from_literal(b"x".as_ptr(), 1);
        assert_eq!(pycc_rt_str_truthy(empty), 0);
        assert_eq!(pycc_rt_str_truthy(non_empty), 1);
        pycc_rt_str_decref(empty);
        pycc_rt_str_decref(non_empty);
    }
}

#[test]
fn incref_then_decref_survives_until_the_final_decref() {
    unsafe {
        let s = pycc_rt_str_from_literal(b"hi".as_ptr(), 2);
        pycc_rt_str_incref(s); // rc 1 -> 2
        pycc_rt_str_decref(s); // rc 2 -> 1, must NOT free yet
        assert_eq!(pycc_rt_str_cmp(s, s), 0); // still safe to read
        pycc_rt_str_decref(s); // rc 1 -> 0, frees
    }
}

#[test]
fn incref_and_decref_on_a_null_pointer_are_safe_no_ops() {
    unsafe {
        pycc_rt_str_incref(std::ptr::null_mut());
        pycc_rt_str_decref(std::ptr::null_mut());
    }
}

#[test]
fn pycc_rt_int_to_str_formats_the_untagged_decimal_value() {
    // Deviation from the task brief: the brief's own version of this
    // (and every other new test below) called `pycc_rt_str_decref`
    // directly, unwrapped -- but that function is `pub unsafe extern
    // "C" fn` (see its own doc comment/safety section above), so an
    // unwrapped call doesn't compile (`E0133`, "call to unsafe function
    // ... is unsafe and requires unsafe block or unsafe function").
    // Every `pycc_rt_str_decref` call below is wrapped in its own
    // `unsafe { ... }` block, matching every other test in this file
    // that already calls an `unsafe fn` (e.g. `cmp_orders_strings_
    // lexicographically` above).
    let s = pycc_rt_int_to_str(tag_smallint(42));
    assert_eq!(unsafe { &*s }.bytes(), b"42");
    unsafe { pycc_rt_str_decref(s) };
    let s = pycc_rt_int_to_str(tag_smallint(-7));
    assert_eq!(unsafe { &*s }.bytes(), b"-7");
    unsafe { pycc_rt_str_decref(s) };
}

#[test]
fn pycc_rt_bool_to_str_matches_python_s_capitalized_spelling() {
    let s = pycc_rt_bool_to_str(1);
    assert_eq!(unsafe { &*s }.bytes(), b"True");
    unsafe { pycc_rt_str_decref(s) };
    let s = pycc_rt_bool_to_str(0);
    assert_eq!(unsafe { &*s }.bytes(), b"False");
    unsafe { pycc_rt_str_decref(s) };
}

#[test]
fn pycc_rt_float_to_str_always_shows_a_decimal_point() {
    // CPython: `str(3.0) == "3.0"`, not `"3"` -- unlike Rust's own `{}`
    // `Display` for `f64`, which omits the fractional part entirely for
    // a whole-number value.
    let s = pycc_rt_float_to_str(3.0);
    assert_eq!(unsafe { &*s }.bytes(), b"3.0");
    unsafe { pycc_rt_str_decref(s) };
    let s = pycc_rt_float_to_str(2.5);
    assert_eq!(unsafe { &*s }.bytes(), b"2.5");
    unsafe { pycc_rt_str_decref(s) };
    let s = pycc_rt_float_to_str(-0.5);
    assert_eq!(unsafe { &*s }.bytes(), b"-0.5");
    unsafe { pycc_rt_str_decref(s) };
}

#[test]
fn pycc_rt_float_to_str_handles_infinity_and_nan_like_cpython() {
    // CPython: `str(float('inf')) == "inf"`, `str(float('nan')) == "nan"`
    // -- lowercase, unlike Rust's own `{}` (`"inf"`/`"NaN"`, capitalized
    // for `NaN`).
    let s = pycc_rt_float_to_str(f64::INFINITY);
    assert_eq!(unsafe { &*s }.bytes(), b"inf");
    unsafe { pycc_rt_str_decref(s) };
    let s = pycc_rt_float_to_str(f64::NEG_INFINITY);
    assert_eq!(unsafe { &*s }.bytes(), b"-inf");
    unsafe { pycc_rt_str_decref(s) };
    let s = pycc_rt_float_to_str(f64::NAN);
    assert_eq!(unsafe { &*s }.bytes(), b"nan");
    unsafe { pycc_rt_str_decref(s) };
}

#[test]
fn pycc_rt_float_to_str_uses_cpython_scientific_notation_outside_its_range() {
    // CPython's `repr(float)` is scientific at magnitudes `>= 1e16` or
    // nonzero `< 1e-4`, with a signed exponent of at least two digits.
    for (value, expected) in [
        (1e17, "1e+17"),
        (1e-5, "1e-05"),
        (-2.5e-10, "-2.5e-10"),
        (123456789012345678.0, "1.2345678901234568e+17"),
        (1e300, "1e+300"),
    ] {
        let result = pycc_rt_float_to_str(value);
        assert_eq!(unsafe { &*result }.bytes(), expected.as_bytes());
        unsafe { pycc_rt_str_decref(result) };
    }
    assert_eq!(pycc_rt_exception_active(), 0);
}

#[test]
fn pycc_rt_float_to_str_accepts_the_boundary_just_inside_the_supported_range() {
    // Deviation from the task brief: the brief's own version of this
    // test called `pycc_rt_float_to_str(1e16)`, expecting
    // `b"10000000000000000.0"` -- but `1e16` is exactly the boundary
    // this function's own `magnitude >= 1e16` check *rejects* (verified
    // against `python3.13`: `repr(1e16)` is itself `'1e+16'`, scientific
    // notation, contradicting the brief test's expectation that it
    // stays positional). That test as written would either panic
    // (contradicting its own non-`#[should_panic]` assertion) or --
    // had the boundary check instead been written as `>` -- silently
    // accept a value CPython itself always renders in scientific
    // notation. Fixed to use the actual double just inside the
    // supported range: `9999999999999998.0`, the IEEE-754 `f64`
    // immediately below `1e16` (`ulp` there is `2.0`), which
    // `python3.13`'s own `repr`/`str` renders positionally as
    // `'9999999999999998.0'`.
    let s = pycc_rt_float_to_str(9999999999999998.0);
    assert_eq!(unsafe { &*s }.bytes(), b"9999999999999998.0");
    unsafe { pycc_rt_str_decref(s) };
}

#[test]
fn pycc_rt_float_to_str_handles_zero_and_negative_zero() {
    // CPython: `str(0.0) == "0.0"`, `str(-0.0) == "-0.0"` (verified
    // against `python3.13`) -- neither the brief's own given tests nor
    // any test above ever passes a zero magnitude, so the `magnitude !=
    // 0.0` short-circuit guard's `false` outcome (skipping the
    // large/small-magnitude check entirely) was otherwise never
    // exercised.
    let s = pycc_rt_float_to_str(0.0);
    assert_eq!(unsafe { &*s }.bytes(), b"0.0");
    unsafe { pycc_rt_str_decref(s) };
    let s = pycc_rt_float_to_str(-0.0);
    assert_eq!(unsafe { &*s }.bytes(), b"-0.0");
    unsafe { pycc_rt_str_decref(s) };
}

#[test]
fn print_write_str_writes_bytes_with_no_trailing_newline() {
    // stdout is captured by the test harness; this only proves the call
    // itself doesn't panic/crash (same rationale as this file's other
    // direct extern-fn exercises). `pycc_rt_str_from_literal`/`pycc_rt_
    // str_decref`/`pycc_rt_print_write_str` (see that function's own
    // doc comment for why it's `unsafe` too, a fix over the task
    // brief's own version) are all `unsafe extern "C" fn`s, so every
    // call below needs an `unsafe` block -- the task brief's own test
    // listing omitted it entirely, a genuine compile-error bug, fixed
    // here the same way this file's other direct `pycc_rt_str_from_
    // literal` call sites already do it.
    unsafe {
        let s = pycc_rt_str_from_literal(b"hi".as_ptr(), 2);
        pycc_rt_print_write_str(s);
        pycc_rt_str_decref(s);
    }
}

#[test]
fn print_space_and_newline_and_none_do_not_panic() {
    pycc_rt_print_space();
    pycc_rt_print_newline();
    pycc_rt_print_none();
}

/// #1054, coverage-bearing companion to
/// `crates/pycc_rt/tests/str_live_objects.rs`. `STR_LIVE` is a single
/// process-wide atomic and dozens of tests in *this* binary construct
/// and free `PyStrObj` values concurrently, so no absolute reading of
/// the counter is assertable here -- the construct/free pairing is
/// proved in that integration test, which owns its own process. What
/// is true under concurrency, and is the property asserted here, is
/// that the net count never goes negative: a decrement only ever
/// retires an object some increment already counted, so an unbalanced
/// free anywhere in this binary's `str` tests would drive the reading
/// below zero.
#[test]
fn str_live_objects_never_reads_back_a_negative_count() {
    let live = pycc_rt_str_live_objects();
    assert!(
        live >= 0,
        "the live-str count must never go negative, read {live}"
    );
}

/// Two literal sites of a name-like text share one object, as CPython's
/// interned constants do, whichever copy of the literal's bytes each site
/// passes; any other text gets an object per site; and a site keeps the
/// object it built.
#[test]
fn literal_sites_of_a_name_share_one_object() {
    static FIRST: &[u8] = b"interned_literal_test_key";
    static SECOND: [u8; 25] = *b"interned_literal_test_key";
    static SPACED: &[u8] = b"not a name";
    unsafe {
        let mut a = std::ptr::null_mut();
        let mut b = std::ptr::null_mut();
        let x = pycc_rt_str_from_literal_cached(FIRST.as_ptr(), FIRST.len() as i64, &mut a);
        let y = pycc_rt_str_from_literal_cached(SECOND.as_ptr(), SECOND.len() as i64, &mut b);
        assert_eq!(x, y);
        assert_eq!(a, x);
        assert_eq!((*x).bytes(), FIRST);
        let again = pycc_rt_str_from_literal_cached(FIRST.as_ptr(), FIRST.len() as i64, &mut a);
        assert_eq!(again, x);
        let mut c = std::ptr::null_mut();
        let mut d = std::ptr::null_mut();
        let p = pycc_rt_str_from_literal_cached(SPACED.as_ptr(), SPACED.len() as i64, &mut c);
        let q = pycc_rt_str_from_literal_cached(SPACED.as_ptr(), SPACED.len() as i64, &mut d);
        assert_ne!(p, q);
    }
}
