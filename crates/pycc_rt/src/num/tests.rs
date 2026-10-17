use super::*;
use crate::exception::{pycc_rt_exception_active, pycc_rt_exception_clear};

fn int(value: i64) -> i64 {
    crate::int_arith::encode_big(num_bigint::BigInt::from(value))
}

fn big(text: &str) -> i64 {
    encode_big(text.parse::<num_bigint::BigInt>().unwrap())
}

fn float(value: f64) -> i64 {
    pycc_rt_num_from_float(value)
}

fn text(word: i64) -> String {
    let mut out = String::new();
    assert!(repr(word, &mut out));
    out
}

#[test]
fn every_value_round_trips_through_its_word() {
    for value in [0, 1, -1, (1 << 49) - 1, -(1 << 49), 1 << 49, -(1 << 49) - 1, i64::MAX >> 1, i64::MIN >> 1] {
        let word = int(value);
        assert!(!is_float(word));
        assert_eq!(text(word), value.to_string(), "{value}");
        pycc_rt_num_release(word);
    }
    for value in [0.0, -0.0, 1.5, f64::INFINITY, f64::NEG_INFINITY, f64::MIN_POSITIVE, -1e308] {
        let word = float(value);
        assert!(is_float(word));
        assert_eq!(to_f64(word).to_bits(), value.to_bits());
    }
    let word = big("123456789012345678901234567890");
    assert_eq!(text(word), "123456789012345678901234567890");
    pycc_rt_num_release(word);
}

#[test]
fn a_nan_keeps_only_its_sign() {
    let positive = float(f64::NAN);
    let negative = float(-f64::NAN);
    let odd = float(f64::from_bits(0xFFFF_0000_1234_0000));
    assert!(to_f64(positive).is_nan() && to_f64(positive).is_sign_positive());
    assert!(to_f64(negative).is_nan() && to_f64(negative).is_sign_negative());
    assert_eq!(odd, negative);
    assert!(is_float(odd));
}

#[test]
fn bools_keep_their_identity() {
    let t = BOOL_TRUE_MARKER;
    assert_eq!(text(t), "True");
    assert_eq!(type_name(t), "bool");
    assert_eq!(text(binop(NUM_ADD, t, t)), "2");
    assert_eq!(inline_int_value(pycc_rt_num_to_int(t)), Some(1));
    assert_eq!(pycc_rt_num_to_int(t), tag_smallint(1));
}

#[test]
fn arithmetic_follows_cpython() {
    let cases: [(i32, i64, i64, &str); 12] = [
        (NUM_ADD, int(2), int(3), "5"),
        (NUM_ADD, int(2), float(0.5), "2.5"),
        (NUM_SUB, float(1.0), int(1), "0.0"),
        (NUM_MUL, int(1 << 40), int(1 << 40), "1208925819614629174706176"),
        (NUM_FLOORDIV, int(-7), int(2), "-4"),
        (NUM_FLOORDIV, float(-7.0), int(2), "-4.0"),
        (NUM_MOD, int(-7), int(3), "2"),
        (NUM_MOD, float(7.5), int(-2), "-0.5"),
        (NUM_POW, int(2), int(10), "1024"),
        (NUM_POW, int(2), int(-1), "0.5"),
        (NUM_POW, float(2.0), int(3), "8.0"),
        (NUM_POW, int(4), float(0.5), "2.0"),
    ];
    for (op, a, b, want) in cases {
        let result = binop(op, a, b);
        assert_eq!(pycc_rt_exception_active(), 0);
        assert_eq!(text(result), want, "{op} {} {}", text(a), text(b));
        pycc_rt_num_release(result);
    }
    assert_eq!(truediv(int(1), int(3)), 1.0 / 3.0);
    assert_eq!(truediv(float(1.0), int(4)), 0.25);
}

#[test]
fn errors_carry_cpython_messages() {
    fn raises(f: impl FnOnce()) -> String {
        pycc_rt_exception_clear();
        f();
        assert_eq!(pycc_rt_exception_active(), 1);
        let obj = crate::exception::pycc_rt_exception_value();
        let message = String::from_utf8(unsafe { (*crate::exception::pycc_rt_exception_message(obj)).bytes() }.to_vec()).unwrap();
        pycc_rt_exception_clear();
        message
    }
    assert_eq!(raises(|| { binop(NUM_FLOORDIV, int(1), int(0)); }), "integer division or modulo by zero");
    assert_eq!(raises(|| { binop(NUM_FLOORDIV, int(1), float(0.0)); }), "float floor division by zero");
    assert_eq!(raises(|| { binop(NUM_MOD, float(1.0), int(0)); }), "float modulo by zero");
    assert_eq!(raises(|| { truediv(int(1), int(0)); }), "division by zero");
    assert_eq!(raises(|| { truediv(float(1.0), int(0)); }), "float division by zero");
    assert_eq!(raises(|| { binop(NUM_POW, int(0), int(-1)); }), "0.0 cannot be raised to a negative power");
    let huge = big(&format!("1{}", "0".repeat(400)));
    assert_eq!(raises(|| { binop(NUM_ADD, huge, float(1.5)); }), "int too large to convert to float");
    assert_eq!(raises(|| { pycc_rt_num_to_int(float(f64::NAN)); }), "cannot convert float NaN to integer");
    assert_eq!(raises(|| { pycc_rt_num_to_int(float(f64::INFINITY)); }), "cannot convert float infinity to integer");
}

#[test]
fn comparisons_are_exact_between_int_and_float() {
    let two53 = int(1 << 53);
    let two53_plus = int((1 << 53) + 1);
    let f = float(9007199254740992.0);
    assert!(compare_op(NUM_EQ, two53, f));
    assert!(!compare_op(NUM_EQ, two53_plus, f));
    assert!(compare_op(NUM_GT, two53_plus, f));
    assert!(compare_op(NUM_LT, f, two53_plus));
    let nan = float(f64::NAN);
    for op in [NUM_EQ, NUM_LT, NUM_LE, NUM_GT, NUM_GE] {
        assert!(!compare_op(op, nan, int(1)));
        assert!(!compare_op(op, int(1), nan));
    }
    assert!(compare_op(NUM_NE, nan, nan));
    let inf = float(f64::INFINITY);
    assert!(compare_op(NUM_LT, big("1000000000000000000000000000000000"), inf));
}

#[test]
fn min_and_max_keep_the_first_of_equal_values() {
    let (one, one_f) = (int(1), float(1.0));
    assert_eq!(pycc_rt_num_min2(one, one_f), one);
    assert_eq!(pycc_rt_num_min2(one_f, one), one_f);
    assert_eq!(pycc_rt_num_max2(one, one_f), one);
    assert_eq!(text(pycc_rt_num_min2(float(f64::INFINITY), int(7))), "7");
}

#[test]
fn hashes_agree_across_equal_values() {
    assert_eq!(hash(int(5)), hash(float(5.0)));
    assert_eq!(hash(int(-1)), -2);
    assert_eq!(hash(float(0.5)), crate::value::hash_float(0.5));
    let big_word = big("1267650600228229401496703205376");
    assert_eq!(hash(big_word), hash(float(1267650600228229401496703205376.0)));
}

#[test]
fn rounding_matches_cpython() {
    assert_eq!(round_int(tag_smallint(1234), -2), tag_smallint(1200));
    assert_eq!(round_int(tag_smallint(15), -1), tag_smallint(20));
    assert_eq!(round_int(tag_smallint(25), -1), tag_smallint(20));
    assert_eq!(round_int(tag_smallint(-25), -1), tag_smallint(-20));
    assert_eq!(round_int(tag_smallint(5), -1_000_000_000), tag_smallint(0));
    assert_eq!(round_int(BOOL_TRUE_MARKER, 3), tag_smallint(1));
    assert_eq!(pycc_rt_num_round(float(2.5)), tag_smallint(2));
    assert_eq!(text(pycc_rt_num_round_digits(float(2.675), 2)), "2.67");
}

#[test]
fn every_float_round_trips_inline_or_boxed() {
    let mut rng = 0x1234_5678_9abc_def0_u64;
    for _ in 0..200_000 {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        let value = f64::from_bits(rng);
        let word = from_f64(value);
        let back = to_f64(word);
        if value.is_nan() {
            assert!(back.is_nan() && back.is_sign_negative() == value.is_sign_negative());
        } else {
            assert_eq!(back.to_bits(), value.to_bits(), "{value:e}");
        }
        assert!(is_float(word));
        pycc_rt_num_release(word);
    }
    for value in [0.0, -0.0, 2.0, -2.0, 1.0, 0.1, 1e77, f64::INFINITY, f64::NEG_INFINITY, f64::NAN] {
        let word = inline_word(value).expect("a common float is inline");
        assert_ne!(word & 0b11, 0);
        assert_ne!(word, BOOL_FALSE_MARKER);
        assert_ne!(word, BOOL_TRUE_MARKER);
        assert_eq!(to_f64(word).to_bits(), value.to_bits(), "{value}");
    }
    for value in [1e300, 1e-300, f64::from_bits(0x3000_0000_0000_0001), 5e-324] {
        assert_eq!(inline_word(value), None, "{value:e}");
    }
}

#[test]
fn a_boxed_float_is_counted_like_a_bigint() {
    let word = from_f64(1e300);
    assert_eq!(word & 0b11, 0);
    pycc_rt_num_retain(word);
    pycc_rt_num_release(word);
    assert_eq!(text(word), "1e+300");
    pycc_rt_num_release(word);
}
