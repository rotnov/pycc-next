//! Encoded ints: tagging, bigint refcounts and promotion, arithmetic, comparison and `range` stepping.

use super::*;

/// #146 Part 1: a heap bigint round-trips through retain/release and is
/// freed exactly once, at the release that retires its last reference.
/// `BIGINT_DROPS` is the only way a test can observe the free at all --
/// nothing about a freed word is inspectable afterwards.
#[test]
fn a_bigint_is_freed_only_when_its_last_reference_goes_away() {
    let before = int_encoding::BIGINT_DROPS.with(|c| c.get());
    let word = tag_bigint(bigint_from_i128(1i128 << 62));
    // Birth reference plus two more.
    bigint_retain(word);
    bigint_retain(word);
    bigint_release(word);
    bigint_release(word);
    assert_eq!(
        int_encoding::BIGINT_DROPS.with(|c| c.get()),
        before,
        "a bigint with a live reference left must not be freed"
    );
    // The value is still intact and readable at this point.
    assert_eq!(to_sign_and_magnitude(word), (false, vec![0, 0x4000_0000]));
    bigint_release(word);
    assert_eq!(
        int_encoding::BIGINT_DROPS.with(|c| c.get()),
        before + 1,
        "the release retiring the last reference must free the object"
    );
}

/// #146 Part 2 (D-181): the no-aliasing invariant `pycc_codegen`'s
/// operand releases depend on. No `int` operation may hand back an
/// operand's own encoded word -- every bigint result is a freshly
/// allocated object. If an identity fast path were ever added (say
/// `a + 0 -> a`), the emitter's release of both operands right after
/// the call would free the value just returned, and this test is what
/// stands between that change and a use-after-free in every compiled
/// program.
///
/// Only `int_add`/`int_sub` are exercised: `int_mul`, `int_floordiv`,
/// `int_floormod`, `int_pow` and `int_cmp` all route through
/// `decode_inline_or_raise`, which raises `OverflowError` and returns a
/// sentinel on a bigint operand (Part C of #1038), so they have no
/// bigint result to alias in the first place (`docs/ROADMAP.md`'s own
/// bigint capability gap).
#[test]
fn an_int_operation_never_returns_an_operand_s_own_word() {
    let big = tag_bigint(bigint_from_i128(1i128 << 62));
    let zero = int_encoding::tag_smallint(0);
    for (name, result) in [
        ("add-zero-right", pycc_rt_int_add(big, zero)),
        ("add-zero-left", pycc_rt_int_add(zero, big)),
        ("sub-zero-right", pycc_rt_int_sub(big, zero)),
        ("add-self", pycc_rt_int_add(big, big)),
        ("sub-self", pycc_rt_int_sub(big, big)),
    ] {
        assert_ne!(
            result, big,
            "`{name}` returned the operand's own encoded word; \
             pycc_codegen releases both operands after the call, so an \
             aliased result is a use-after-free"
        );
        bigint_release(result);
    }
    bigint_release(big);
}

/// The word `0` is what an `int` storage slot holds before its first
/// store (`storage_slot_at_entry`'s zero-initialization). It is *not* a
/// valid encoded int -- `classify_encoded_int` fails closed on it -- so
/// both operations must return before classifying rather than panic.
#[test]
fn the_empty_slot_word_is_a_no_op_for_both_refcount_operations() {
    let before = int_encoding::BIGINT_DROPS.with(|c| c.get());
    bigint_retain(0);
    bigint_release(0);
    assert_eq!(int_encoding::BIGINT_DROPS.with(|c| c.get()), before);
}

/// Smallints and the two D-141 bool-identity markers own no allocation,
/// so both operations are no-ops on them. `pycc_codegen` guards its call
/// sites inline so these never actually reach the runtime, but the
/// runtime contract holds independently of that guard.
#[test]
fn inline_int_kinds_are_no_ops_for_both_refcount_operations() {
    let before = int_encoding::BIGINT_DROPS.with(|c| c.get());
    for word in [tag_smallint(0), tag_smallint(-7), 0b0010, 0b0110] {
        bigint_retain(word);
        bigint_release(word);
    }
    assert_eq!(int_encoding::BIGINT_DROPS.with(|c| c.get()), before);
}

/// A word ending in binary `10` other than the bool markers is an inline
/// `float` of an `int | float` word (`crate::num`), which holds no
/// reference: both operations leave it alone rather than classifying it.
#[test]
fn an_inline_float_word_is_a_no_op_for_both_refcount_operations() {
    let before = int_encoding::BIGINT_DROPS.with(|c| c.get());
    bigint_retain(0x2a);
    bigint_release(0x2a);
    assert_eq!(int_encoding::BIGINT_DROPS.with(|c| c.get()), before);
}

/// The `extern "C"` wrappers themselves, on the non-panicking paths that
/// generated code actually reaches.
#[test]
fn the_extern_c_bigint_refcount_wrappers_forward_to_the_private_pair() {
    let before = int_encoding::BIGINT_DROPS.with(|c| c.get());
    let word = tag_bigint(bigint_from_i128(1i128 << 62));
    pycc_rt_bigint_retain(word);
    pycc_rt_bigint_release(word);
    assert_eq!(int_encoding::BIGINT_DROPS.with(|c| c.get()), before);
    pycc_rt_bigint_retain(0);
    pycc_rt_bigint_release(0);
    assert_eq!(int_encoding::BIGINT_DROPS.with(|c| c.get()), before);
    pycc_rt_bigint_release(word);
    assert_eq!(int_encoding::BIGINT_DROPS.with(|c| c.get()), before + 1);
}

#[test]
fn tagging_a_small_value_and_untagging_it_round_trips() {
    for n in [0i64, 1, -1, 42, -42, i64::MAX >> 1, -(i64::MAX >> 1)] {
        let tagged = tag_smallint(n);
        assert!(is_smallint(tagged), "expected {n} to be tagged as small");
        assert_eq!(untag_smallint(tagged), n);
    }
}

#[test]
fn a_value_needing_the_full_64_bits_does_not_fit_the_tagged_range() {
    assert_eq!(fits_smallint(i64::MAX), None);
    assert_eq!(fits_smallint(i64::MIN), None);
}

#[test]
fn pycc_rt_int_add_computes_the_correct_tagged_sum() {
    let a = tag_smallint(2);
    let b = tag_smallint(3);
    assert_eq!(untag_smallint(pycc_rt_int_add(a, b)), 5);
}

#[test]
fn pycc_rt_int_sub_computes_the_correct_tagged_difference() {
    let a = tag_smallint(5);
    let b = tag_smallint(3);
    assert_eq!(untag_smallint(pycc_rt_int_sub(a, b)), 2);
}

#[test]
fn pycc_rt_int_mul_computes_the_correct_tagged_product() {
    let a = tag_smallint(6);
    let b = tag_smallint(7);
    assert_eq!(untag_smallint(pycc_rt_int_mul(a, b)), 42);
}

#[test]
fn pycc_rt_int_mul_promotes_a_product_outside_the_smallint_range() {
    let product = pycc_rt_int_mul(tag_smallint(i64::MAX >> 1), tag_smallint(2));
    assert!(!is_smallint(product));
    let text = pycc_rt_int_to_str(product);
    assert_eq!(unsafe { &*text }.bytes(), b"9223372036854775806");
    unsafe { pycc_rt_str_decref(text) };
}

#[test]
fn pycc_rt_int_floordiv_matches_python_floor_semantics() {
    // Python: -7 // 2 == -4 (floors toward negative infinity), not -3
    // (truncation toward zero, which is what a raw LLVM/Rust `/` gives).
    assert_eq!(
        untag_smallint(pycc_rt_int_floordiv(tag_smallint(-7), tag_smallint(2))),
        -4
    );
    assert_eq!(
        untag_smallint(pycc_rt_int_floordiv(tag_smallint(7), tag_smallint(2))),
        3
    );
    assert_eq!(
        untag_smallint(pycc_rt_int_floordiv(tag_smallint(7), tag_smallint(-2))),
        -4
    );
}

#[test]
fn pycc_rt_int_floormod_matches_python_floor_semantics() {
    // Python: -7 % 2 == 1 (result takes the divisor's sign), not -1.
    assert_eq!(
        untag_smallint(pycc_rt_int_floormod(tag_smallint(-7), tag_smallint(2))),
        1
    );
    assert_eq!(
        untag_smallint(pycc_rt_int_floormod(tag_smallint(7), tag_smallint(2))),
        1
    );
    assert_eq!(
        untag_smallint(pycc_rt_int_floormod(tag_smallint(7), tag_smallint(-2))),
        -1
    );
}

#[test]
fn pycc_rt_int_floordiv_by_zero_sets_exception_flag() {
    // D-173: zero division now sets the pending exception flag instead
    // of panicking. Clear the flag first for test isolation.
    pycc_rt_exception_clear();
    let result = int_floordiv(tag_smallint(1), tag_smallint(0));
    assert_eq!(pycc_rt_exception_active(), 1);
    assert_eq!(untag_smallint(result), 0); // sentinel value
    pycc_rt_exception_clear();
}

#[test]
fn pycc_rt_int_floormod_by_zero_sets_exception_flag() {
    pycc_rt_exception_clear();
    let result = int_floormod(tag_smallint(1), tag_smallint(0));
    assert_eq!(pycc_rt_exception_active(), 1);
    assert_eq!(untag_smallint(result), 0); // sentinel value
    pycc_rt_exception_clear();
}

#[test]
fn pycc_rt_int_floordiv_handles_the_minimum_taggable_value_divided_by_one() {
    // The task brief's own version of this test's comment claimed this
    // exercised the classic `i64::MIN / -1` hardware-trap pair -- it
    // does not (it divides by `1`, not `-1`); see `int_floordiv`'s own
    // comment for why that trap is actually unreachable here at all.
    // What this test does prove: the boundary value itself (the most
    // negative value that still tags) round-trips correctly through
    // floor-division by `1` without spuriously reporting overflow.
    let min = i64::MIN >> 1; // most negative value that still tags
    assert_eq!(
        untag_smallint(pycc_rt_int_floordiv(tag_smallint(min), tag_smallint(1))),
        min
    );
}

#[test]
fn pycc_rt_int_floordiv_promotes_the_negated_minimum_taggable_value() {
    // The actual reachable promotion case for `int_floordiv` (see its
    // comment): floor-dividing the minimum taggable value by `-1`
    // negates it, producing exactly one more than the maximum taggable
    // magnitude -- a real overflow of the tagged result, distinct from (and
    // the reason the brief's original i64::MIN/-1 *input* guard turned
    // out to be dead code).
    let min = i64::MIN >> 1;
    let quotient = int_floordiv(tag_smallint(min), tag_smallint(-1));
    assert!(!is_smallint(quotient));
    let text = pycc_rt_int_to_str(quotient);
    assert_eq!(unsafe { &*text }.bytes(), b"4611686018427387904");
    unsafe { pycc_rt_str_decref(text) };
}

#[test]
fn pycc_rt_int_pow_computes_the_correct_tagged_power() {
    assert_eq!(
        untag_smallint(pycc_rt_int_pow(tag_smallint(2), tag_smallint(10))),
        1024
    );
    assert_eq!(
        untag_smallint(pycc_rt_int_pow(tag_smallint(5), tag_smallint(0))),
        1
    );
}

#[test]
fn pycc_rt_int_pow_with_a_negative_exponent_raises_runtime_error() {
    // Part A of #1038 (#1063): was `#[should_panic]`. `RuntimeError`
    // rather than a conformant class because CPython raises nothing at
    // all here -- it computes `2 ** -1` as `0.5`. Calls the private
    // `int_pow`, not the aborting `extern "C"` wrapper, following this
    // module's established convention.
    pycc_rt_exception_clear();
    let result = int_pow(tag_smallint(2), tag_smallint(-1));
    let (tag, message) = pending_tag_and_message();
    assert_eq!(tag, EXCEPTION_TYPE_RUNTIME_ERROR);
    assert!(message.contains("negative exponent"), "{message}");
    assert!(!message.contains("pycc_rt: "), "{message}");
    assert_eq!(result, tag_smallint(0)); // sentinel value
    pycc_rt_exception_clear();
}

#[test]
fn pycc_rt_int_cmp_reports_less_equal_and_greater() {
    assert_eq!(pycc_rt_int_cmp(tag_smallint(1), tag_smallint(2)), -1);
    assert_eq!(pycc_rt_int_cmp(tag_smallint(2), tag_smallint(2)), 0);
    assert_eq!(pycc_rt_int_cmp(tag_smallint(3), tag_smallint(2)), 1);
}

#[test]
#[should_panic(expected = "invalid encoded int word 0x0")]
fn encoded_int_classification_rejects_null_before_any_pointer_cast() {
    classify_encoded_int(0);
}

#[test]
#[should_panic(expected = "invalid encoded int word 0xa")]
fn encoded_int_classification_rejects_unrecognized_low_tag_10_words() {
    classify_encoded_int(0b1010);
}

#[test]
#[should_panic(expected = "attempted bigint dereference of a non-pointer int word")]
fn bigint_dereference_rechecks_the_encoded_word_before_casting() {
    // Even an unsafe internal caller cannot turn a valid bool marker
    // into a pointer without passing the central classifier again.
    unsafe {
        let _ = bigint_ref(BOOL_TRUE_MARKER);
    }
}

#[test]
fn bool_identity_markers_decode_numerically_but_format_as_bools() {
    assert_eq!(int_untag_checked(BOOL_FALSE_MARKER, UNTAG_INDEX), 0);
    assert_eq!(int_untag_checked(BOOL_TRUE_MARKER, UNTAG_INDEX), 1);
    assert_eq!(pycc_rt_int_truthy(BOOL_FALSE_MARKER), 0);
    assert_eq!(pycc_rt_int_truthy(BOOL_TRUE_MARKER), 1);
    assert_eq!(pycc_rt_int_to_float(BOOL_FALSE_MARKER), 0.0);
    assert_eq!(pycc_rt_int_to_float(BOOL_TRUE_MARKER), 1.0);

    unsafe {
        let false_text = int_to_str(BOOL_FALSE_MARKER);
        let true_text = int_to_str(BOOL_TRUE_MARKER);
        assert_eq!((&*false_text).bytes(), b"False");
        assert_eq!((&*true_text).bytes(), b"True");
        pycc_rt_str_decref(false_text);
        pycc_rt_str_decref(true_text);
    }
}

#[test]
fn arithmetic_consumes_bool_markers_and_produces_ordinary_ints() {
    let cases = [
        (int_add(BOOL_TRUE_MARKER, tag_smallint(1)), 2),
        (int_add(tag_smallint(1), BOOL_TRUE_MARKER), 2),
        (int_add(BOOL_FALSE_MARKER, tag_smallint(2)), 2),
        (int_add(tag_smallint(2), BOOL_FALSE_MARKER), 2),
        (int_sub(BOOL_TRUE_MARKER, tag_smallint(1)), 0),
        (int_sub(tag_smallint(1), BOOL_TRUE_MARKER), 0),
        (int_mul(BOOL_TRUE_MARKER, tag_smallint(2)), 2),
        (int_mul(tag_smallint(2), BOOL_TRUE_MARKER), 2),
        (int_mul(BOOL_FALSE_MARKER, tag_smallint(2)), 0),
        (int_mul(tag_smallint(2), BOOL_FALSE_MARKER), 0),
        (int_floordiv(BOOL_FALSE_MARKER, tag_smallint(2)), 0),
        (int_floordiv(tag_smallint(2), BOOL_TRUE_MARKER), 2),
        (int_floormod(BOOL_FALSE_MARKER, tag_smallint(2)), 0),
        (int_floormod(tag_smallint(2), BOOL_TRUE_MARKER), 0),
        (int_pow(BOOL_FALSE_MARKER, BOOL_TRUE_MARKER), 0),
        (int_pow(tag_smallint(2), BOOL_TRUE_MARKER), 2),
        (int_pow(BOOL_TRUE_MARKER, tag_smallint(2)), 1),
        (int_pow(tag_smallint(2), BOOL_FALSE_MARKER), 1),
    ];
    for (actual, expected) in cases {
        assert_eq!(actual, tag_smallint(expected));
        assert!(is_smallint(actual));
    }

    assert_eq!(int_cmp(BOOL_FALSE_MARKER, tag_smallint(0)), 0);
    assert_eq!(int_cmp(tag_smallint(1), BOOL_TRUE_MARKER), 0);
    assert_eq!(int_cmp(BOOL_FALSE_MARKER, BOOL_TRUE_MARKER), -1);
    assert_eq!(
        range_continue(BOOL_FALSE_MARKER, BOOL_TRUE_MARKER, BOOL_TRUE_MARKER),
        1
    );
    assert_eq!(
        range_continue(BOOL_TRUE_MARKER, BOOL_TRUE_MARKER, BOOL_TRUE_MARKER),
        0
    );
}

#[test]
fn false_identity_marker_is_a_zero_range_step() {
    // D-173/#150: `False` decodes to the smallint `0`, so this hits the
    // same zero-step guard as any other literal zero and now sets a
    // pending `ValueError` instead of panicking.
    pycc_rt_exception_clear();
    let result = range_continue(tag_smallint(0), tag_smallint(1), BOOL_FALSE_MARKER);
    assert_eq!(pycc_rt_exception_active(), 1);
    assert_eq!(result, 0); // sentinel value
    let obj = exception::pycc_rt_exception_value();
    assert_eq!(unsafe { (*obj).type_tag }, EXCEPTION_TYPE_VALUE_ERROR);
    pycc_rt_exception_clear();
}

#[test]
fn add_and_sub_accept_bool_markers_next_to_bigints_in_both_positions() {
    fn assert_value(encoded: i64, expected: i128) {
        let expected = bigint_from_i128(expected);
        let (actual_negative, actual_limbs) = to_sign_and_magnitude(encoded);
        assert_eq!(actual_negative, expected.negative);
        assert_eq!(actual_limbs, expected.limbs);
    }

    let magnitude = 1i128 << 80;
    let bigint = tag_bigint(bigint_from_i128(magnitude));
    assert_value(int_add(bigint, BOOL_TRUE_MARKER), magnitude + 1);
    assert_value(int_add(BOOL_TRUE_MARKER, bigint), magnitude + 1);
    assert_value(int_sub(bigint, BOOL_TRUE_MARKER), magnitude - 1);
    assert_value(int_sub(BOOL_TRUE_MARKER, bigint), 1 - magnitude);
}

#[test]
fn pycc_rt_int_truthy_is_false_only_for_zero() {
    assert_eq!(pycc_rt_int_truthy(tag_smallint(0)), 0);
    assert_eq!(pycc_rt_int_truthy(tag_smallint(1)), 1);
    assert_eq!(pycc_rt_int_truthy(tag_smallint(-1)), 1);
}

#[test]
fn pycc_rt_range_continue_handles_positive_step() {
    assert_eq!(
        pycc_rt_range_continue(tag_smallint(0), tag_smallint(3), tag_smallint(1)),
        1
    );
    assert_eq!(
        pycc_rt_range_continue(tag_smallint(3), tag_smallint(3), tag_smallint(1)),
        0
    );
}

#[test]
fn pycc_rt_range_continue_handles_negative_step() {
    assert_eq!(
        pycc_rt_range_continue(tag_smallint(3), tag_smallint(0), tag_smallint(-1)),
        1
    );
    assert_eq!(
        pycc_rt_range_continue(tag_smallint(0), tag_smallint(0), tag_smallint(-1)),
        0
    );
}

#[test]
fn pycc_rt_range_continue_with_a_zero_step_sets_exception_flag() {
    // D-173/#150: a zero step (inline fast path) now sets a pending
    // `ValueError` instead of panicking, and returns the ordinary
    // "stop" sentinel. Clear the flag first for test isolation, mirroring
    // `pycc_rt_int_floordiv_by_zero_sets_exception_flag` above.
    pycc_rt_exception_clear();
    let result = range_continue(tag_smallint(0), tag_smallint(3), tag_smallint(0));
    assert_eq!(pycc_rt_exception_active(), 1);
    assert_eq!(result, 0); // sentinel value
    let obj = exception::pycc_rt_exception_value();
    assert_eq!(unsafe { (*obj).type_tag }, EXCEPTION_TYPE_VALUE_ERROR);
    pycc_rt_exception_clear();
}

/// A bigint holding exactly `2^62` -- the smallest magnitude that does
/// not round-trip through the tagged 63-bit encoding, i.e. the first
/// value `fits_smallint` rejects.
fn bigint_two_pow_62() -> i64 {
    tag_bigint(bigint_from_i128(1i128 << 62))
}

/// A *bigint-tagged zero*. Arithmetic never produces one (its results
/// are canonical), but the runtime must not read sign before magnitude
/// if a heap word ever holds zero.
fn bigint_zero() -> i64 {
    tag_bigint(BigIntObj::new(false, vec![0]))
}

#[test]
fn magnitude_sign_reads_zero_from_the_magnitude_not_the_negative_flag() {
    assert_eq!(magnitude_sign(false, &[0]), 0);
    // The trap this helper exists for: a normalized bigint zero carries
    // `negative: false`, so "not negative" alone would read as positive.
    assert_eq!(magnitude_sign(true, &[0, 0]), 0);
    assert_eq!(magnitude_sign(false, &[1]), 1);
    assert_eq!(magnitude_sign(true, &[1]), -1);
}

#[test]
fn encoded_int_cmp_orders_inline_operands_without_decoding() {
    assert_eq!(encoded_int_cmp(tag_smallint(1), tag_smallint(2)), Less);
    assert_eq!(encoded_int_cmp(tag_smallint(2), tag_smallint(1)), Greater);
    assert_eq!(encoded_int_cmp(tag_smallint(2), tag_smallint(2)), Equal);
    assert_eq!(encoded_int_cmp(BOOL_FALSE_MARKER, tag_smallint(0)), Equal);
    assert_eq!(encoded_int_cmp(BOOL_TRUE_MARKER, tag_smallint(1)), Equal);
    assert_eq!(encoded_int_cmp(BOOL_FALSE_MARKER, BOOL_TRUE_MARKER), Less);
}

#[test]
fn encoded_int_cmp_orders_bigints_against_inline_operands_in_both_positions() {
    let big = bigint_two_pow_62();
    let negative_big = int_sub(tag_smallint(0), big);
    assert_eq!(encoded_int_cmp(big, tag_smallint(i64::MAX >> 1)), Greater);
    assert_eq!(encoded_int_cmp(tag_smallint(i64::MAX >> 1), big), Less);
    assert_eq!(encoded_int_cmp(negative_big, tag_smallint(0)), Less);
    assert_eq!(encoded_int_cmp(tag_smallint(0), negative_big), Greater);
    // Opposite signs on both sides of the comparison.
    assert_eq!(encoded_int_cmp(big, negative_big), Greater);
    assert_eq!(encoded_int_cmp(negative_big, big), Less);
}

#[test]
fn encoded_int_cmp_orders_two_bigints_of_the_same_sign_by_magnitude() {
    let big = bigint_two_pow_62();
    let bigger = int_add(big, tag_smallint(1));
    assert_eq!(encoded_int_cmp(big, bigger), Less);
    assert_eq!(encoded_int_cmp(bigger, big), Greater);
    assert_eq!(encoded_int_cmp(big, int_add(big, tag_smallint(0))), Equal);

    // Two negatives: the larger magnitude is the *smaller* value.
    let negative = int_sub(tag_smallint(0), big);
    let more_negative = int_sub(tag_smallint(0), bigger);
    assert_eq!(encoded_int_cmp(more_negative, negative), Less);
    assert_eq!(encoded_int_cmp(negative, more_negative), Greater);
}

#[test]
fn encoded_int_cmp_treats_a_bigint_zero_as_zero() {
    let zero = bigint_zero();
    assert_eq!(encoded_int_cmp(zero, tag_smallint(0)), Equal);
    assert_eq!(encoded_int_cmp(zero, bigint_zero()), Equal);
    assert_eq!(encoded_int_cmp(zero, tag_smallint(1)), Less);
    assert_eq!(encoded_int_cmp(tag_smallint(-1), zero), Less);
}

#[test]
fn range_normalize_operand_maps_bool_markers_to_ordinary_smallints() {
    // D-141's contract, restated by D-179: `range` consumes the numeric
    // value and produces ordinary integer objects, so the identity
    // markers must not survive into the induction variable.
    assert_eq!(range_normalize_operand(BOOL_FALSE_MARKER), tag_smallint(0));
    assert_eq!(range_normalize_operand(BOOL_TRUE_MARKER), tag_smallint(1));
}

#[test]
fn range_normalize_operand_passes_smallints_and_bigints_through_unchanged() {
    assert_eq!(range_normalize_operand(tag_smallint(7)), tag_smallint(7));
    assert_eq!(range_normalize_operand(tag_smallint(-7)), tag_smallint(-7));
    let big = bigint_two_pow_62();
    // The #147 defect in one assertion: this used to abort.
    assert_eq!(range_normalize_operand(big), big);
    // The public wrapper's own line, on a non-panicking input.
    assert_eq!(pycc_rt_range_normalize_operand(big), big);
}

#[test]
#[should_panic(expected = "pycc_rt: invalid encoded int word")]
fn range_normalize_operand_rejects_a_malformed_word() {
    // Calls the private fn for the usual `extern "C"` abort reason.
    range_normalize_operand(0);
}

#[test]
fn range_continue_drives_a_loop_whose_operands_are_bigints() {
    let start = bigint_two_pow_62();
    let stop = int_add(start, tag_smallint(2));
    assert_eq!(range_continue(start, stop, tag_smallint(1)), 1);
    assert_eq!(range_continue(stop, stop, tag_smallint(1)), 0);
    // A bigint step, ascending and descending.
    assert_eq!(range_continue(tag_smallint(0), start, start), 1);
    assert_eq!(
        range_continue(
            tag_smallint(0),
            tag_smallint(-1),
            int_sub(tag_smallint(0), start)
        ),
        1
    );
}

#[test]
fn range_continue_stops_when_the_induction_variable_promotes_mid_loop() {
    // The exact shape `range(4611686018427387900, 4611686018427387903, 2)`
    // reaches: `start`/`stop`/`step` all stay smallints, but the third
    // `int_add` produces 2^62, which `fits_smallint` rejects. The loop
    // guard then sees a *bigint* `i` against a *smallint* `stop`.
    let stop = tag_smallint(4_611_686_018_427_387_903);
    let promoted = int_add(tag_smallint(4_611_686_018_427_387_902), tag_smallint(2));
    assert_eq!(classify_encoded_int(promoted), EncodedIntKind::BigInt);
    assert_eq!(range_continue(promoted, stop, tag_smallint(2)), 0);
}

#[test]
fn range_continue_rejects_a_bigint_zero_step() {
    // A bigint-tagged zero is *numerically* zero even though its word is
    // a non-zero pointer, so the guard must compare values, not words.
    // D-173/#150: the general (`encoded_int_cmp`) path also sets a
    // pending `ValueError` instead of panicking now.
    pycc_rt_exception_clear();
    let result = range_continue(tag_smallint(0), tag_smallint(10), bigint_zero());
    assert_eq!(pycc_rt_exception_active(), 1);
    assert_eq!(result, 0); // sentinel value
    let obj = exception::pycc_rt_exception_value();
    assert_eq!(unsafe { (*obj).type_tag }, EXCEPTION_TYPE_VALUE_ERROR);
    pycc_rt_exception_clear();
}

#[test]
fn adding_past_i64_range_now_promotes_instead_of_panicking() {
    // This is the exact fixture Task 3's
    // `pycc_rt_int_add_panics_on_overflow_before_bigint_promotion_exists`
    // used to require a panic for -- it must now succeed and print the
    // exact mathematical sum.
    let huge = pycc_rt_int_add(tag_smallint(i64::MAX >> 1), tag_smallint(1));
    let s = pycc_rt_int_to_str(huge);
    assert_eq!(unsafe { &*s }.bytes(), b"4611686018427387904");
    unsafe { pycc_rt_str_decref(s) };
}

#[test]
fn int_from_i64_keeps_an_in_range_value_inline() {
    let encoded = pycc_rt_int_from_i64(42);
    assert_eq!(classify_encoded_int(encoded), EncodedIntKind::SmallInt);
    assert_eq!(untag_smallint(encoded), 42);
}

#[test]
fn int_from_i64_promotes_values_outside_the_tagged_range() {
    // The four boundary literals `pycc_codegen` can no longer fold:
    // `2^62`, `i64::MAX`, `-(2^62) - 1`, and `i64::MIN`.
    for (value, expected) in [
        (4_611_686_018_427_387_904_i64, "4611686018427387904"),
        (i64::MAX, "9223372036854775807"),
        (-4_611_686_018_427_387_905_i64, "-4611686018427387905"),
        (i64::MIN, "-9223372036854775808"),
    ] {
        let encoded = pycc_rt_int_from_i64(value);
        assert_eq!(classify_encoded_int(encoded), EncodedIntKind::BigInt);
        let s = pycc_rt_int_to_str(encoded);
        assert_eq!(unsafe { &*s }.bytes(), expected.as_bytes());
        unsafe { pycc_rt_str_decref(s) };
    }
}

#[test]
fn subtracting_past_the_negative_range_promotes_correctly() {
    let huge = pycc_rt_int_sub(tag_smallint(i64::MIN >> 1), tag_smallint(1));
    let s = pycc_rt_int_to_str(huge);
    assert_eq!(unsafe { &*s }.bytes(), b"-4611686018427387905");
    unsafe { pycc_rt_str_decref(s) };
}

#[test]
fn repeated_addition_exercises_the_general_bigint_plus_smallint_path() {
    // Simulates unbounded `fib`-style growth. The first `pycc_rt_int_add`
    // call overflows the tagged range (`i64::MAX >> 1` is exactly the
    // largest value that still round-trips through tagging, so adding
    // it to itself does not fit) and promotes via the one-time
    // `i128`-widening path. Every one of the 20 subsequent additions
    // then has a `BigIntObj` as its left operand and a tagged smallint
    // as its right operand -- `is_smallint(a) && is_smallint(b)` is
    // false for all of them, so they all go through
    // `to_sign_and_magnitude`/`bigint_add_signed`'s general limb
    // arithmetic, not the fast path or the one-time promotion shortcut.
    // (The running total here stays well under 128 bits -- about 67 at
    // the end -- this test is not about exceeding `i128`'s own range,
    // only about exercising the general bigint-arithmetic code path
    // repeatedly rather than just once.)
    let mut acc = pycc_rt_int_add(tag_smallint(i64::MAX >> 1), tag_smallint(i64::MAX >> 1));
    for _ in 0..20 {
        acc = pycc_rt_int_add(acc, tag_smallint(i64::MAX >> 1));
    }
    assert!(!is_smallint(acc));
    let s = pycc_rt_int_to_str(acc);
    let text = String::from_utf8(unsafe { &*s }.bytes().to_vec()).unwrap();
    assert_eq!(text, (bigint_reference_sum()).to_string());
    unsafe { pycc_rt_str_decref(s) };
}

/// Independent reference computation for the test above (22 additions
/// of `i64::MAX >> 1`, well within `i128`'s own range) -- this doesn't
/// exercise `pycc_rt`'s bigint code at all, so it's a trustworthy oracle
/// for what the *correct* sum is, independent of any bug the code under
/// test might have.
fn bigint_reference_sum() -> i128 {
    let step = (i64::MAX >> 1) as i128;
    let mut acc = step + step;
    for _ in 0..20 {
        acc += step;
    }
    acc
}

#[test]
fn a_bigint_that_would_fit_back_in_smallint_range_still_formats_correctly() {
    // Two already-promoted values that sum back to something small
    // (mathematically representable as a smallint) are not required to
    // shrink back down (D-061/this task's own "simplest correct" choice
    // -- once a value touches the bigint path, it stays represented as
    // one) -- but the printed *value* must still be exactly right.
    let a = pycc_rt_int_add(tag_smallint(i64::MAX >> 1), tag_smallint(1)); // a bigint
    let b = pycc_rt_int_sub(tag_smallint(0), a); // -a, also a bigint (sub promotes too)
    let zero = pycc_rt_int_add(a, b);
    let s = pycc_rt_int_to_str(zero);
    assert_eq!(unsafe { &*s }.bytes(), b"0");
    unsafe { pycc_rt_str_decref(s) };
}

#[test]
fn a_bigint_zero_is_falsy() {
    let a = pycc_rt_int_add(tag_smallint(i64::MAX >> 1), tag_smallint(1));
    let b = pycc_rt_int_sub(tag_smallint(0), a);
    let zero = pycc_rt_int_add(a, b);
    assert_eq!(pycc_rt_int_truthy(zero), 0);
}

#[test]
fn a_nonzero_bigint_is_truthy() {
    // Companion to `a_bigint_zero_is_falsy`: pins the *other* outcome of
    // `pycc_rt_int_truthy`'s bigint branch (a magnitude that is not a
    // single zero limb). `i64::MAX >> 1` is the largest tagged smallint,
    // so `+ 1` overflows the fixnum range and promotes to a real bigint.
    let huge = pycc_rt_int_add(tag_smallint(i64::MAX >> 1), tag_smallint(1));
    assert!(
        !is_smallint(huge),
        "value must actually be a bigint, not a tagged smallint"
    );
    assert_eq!(pycc_rt_int_truthy(huge), 1);
}

#[test]
fn adding_two_opposite_sign_bigints_with_the_same_limb_count_but_different_magnitude() {
    // Every other opposite-sign addition/subtraction test above either
    // has exactly equal magnitudes (immediately `Ordering::Equal`,
    // never reaching `magnitude_cmp`'s per-limb loop body) or magnitudes
    // needing a different number of limbs (resolved by `magnitude_cmp`'s
    // own length check before the loop runs at all) -- so this is the
    // only test exercising `magnitude_cmp`'s `a[i] != b[i]` true branch,
    // `bigint_add_signed`'s `Ordering::Greater` arm, and
    // `magnitude_sub`'s borrow path (`diff < 0`), all at once.
    //
    // `step = i64::MAX >> 1 = 0x3FFF_FFFF_FFFF_FFFF` (low limb
    // `0xFFFF_FFFF`, high limb `0x3FFF_FFFF`). `big = 2 * step`,
    // `bigger = 3 * step` -- both promote to real 2-limb bigints (their
    // magnitude exceeds a single `u32`'s range), and `bigger`'s low limb
    // (`0xFFFF_FFFD`) is smaller than `big`'s low limb (`0xFFFF_FFFE`),
    // so subtracting them genuinely borrows from the high limb.
    // `bigger + (-big)` must equal `step` exactly.
    let step = i64::MAX >> 1;
    let big = pycc_rt_int_add(tag_smallint(step), tag_smallint(step));
    let bigger = pycc_rt_int_add(big, tag_smallint(step));
    let neg_big = pycc_rt_int_sub(tag_smallint(0), big);
    assert!(!is_smallint(big) && !is_smallint(bigger) && !is_smallint(neg_big));
    let result = pycc_rt_int_add(bigger, neg_big);
    let s = pycc_rt_int_to_str(result);
    assert_eq!(unsafe { &*s }.bytes(), step.to_string().as_bytes());
    unsafe { pycc_rt_str_decref(s) };
}

#[test]
fn bigint_from_i128_of_zero_still_has_a_single_zero_limb() {
    // Not reachable through `int_add`/`int_sub`'s own overflow-promotion
    // call sites -- a mathematically-zero sum/difference always fits
    // the tagged smallint range, so `checked_add`/`checked_sub` +
    // `fits_smallint` succeed first and this fallback is never invoked
    // with `0`. Tested directly against the private helper instead,
    // matching this file's own convention of testing a general-purpose
    // private function's own contract rather than only the narrower
    // paths its current callers happen to exercise.
    let b = bigint_from_i128(0);
    assert!(!b.negative);
    assert_eq!(b.limbs, vec![0]);
}
