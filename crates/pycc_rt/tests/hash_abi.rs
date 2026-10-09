//! #1331 (Part 1 of #1327) and #1335 (Part 1 of #1332): the
//! `pycc_rt_hash_*` entry points, called
//! through the `rlib` exactly as generated code links them from the
//! `staticlib`.
//!
//! The unit tests in `crates/pycc_rt/src/hash.rs` own the semantics; this
//! file pins the exported ABI from outside the crate. It is also where the
//! diff-coverage gate sees these functions: the `--workspace` export keeps
//! an integration-test binary's copy of a `pycc_rt` symbol and discards the
//! unit-test binary's counters for it (`docs/TESTING.md`, "A runtime
//! function an integration test links is measured from that binary").
//! Every expected value is CPython 3.14's own on a 64-bit build.

use pycc_rt::{
    pycc_rt_bigint_release, pycc_rt_ext_pending_type, pycc_rt_hash_int, pycc_rt_hash_pointer,
    pycc_rt_hash_slot_int, pycc_rt_hash_tuple, pycc_rt_int_lshift, pycc_rt_int_sub,
};

/// D-061's inline smallint word for `value`.
fn small(value: i64) -> i64 {
    (value << 1) | 1
}

/// `2**shift` as an encoded word, a heap bigint from `2**62` up.
fn pow2(shift: i64) -> i64 {
    let word = pycc_rt_int_lshift(small(1), small(shift));
    assert_eq!(pycc_rt_ext_pending_type(), -1, "the shift raises nothing");
    word
}

/// Hashes `word`, then releases it when it is a heap bigint.
fn hash_owned(word: i64) -> i64 {
    let hash = pycc_rt_hash_int(word);
    pycc_rt_bigint_release(word);
    hash
}

fn tuple(lanes: &[i64]) -> i64 {
    unsafe { pycc_rt_hash_tuple(lanes.as_ptr(), lanes.len()) }
}

#[test]
fn int_and_bool_words_hash_like_cpython() {
    assert_eq!(pycc_rt_hash_int(small(0)), 0);
    assert_eq!(pycc_rt_hash_int(small(-1)), -2);
    assert_eq!(pycc_rt_hash_int(small(-2)), -2);
    assert_eq!(pycc_rt_hash_int(small((1 << 61) - 1)), 0);
    assert_eq!(pycc_rt_hash_int(small(1 << 61)), 1);
    assert_eq!(pycc_rt_hash_int(small(-(1 << 61))), -2);
    assert_eq!(pycc_rt_hash_int(2), 0, "D-141's False marker");
    assert_eq!(pycc_rt_hash_int(6), 1, "D-141's True marker");
}

#[test]
fn heap_bigints_hash_like_cpython_without_releasing_the_argument() {
    assert_eq!(hash_owned(pow2(62)), 2);
    assert_eq!(hash_owned(pow2(64)), 8);
    assert_eq!(hash_owned(pow2(70)), 512);
    assert_eq!(hash_owned(pow2(100)), 549_755_813_888);
    let negative = pycc_rt_int_sub(small(0), pow2(100));
    assert_eq!(pycc_rt_hash_int(negative), -549_755_813_888);
    // Borrowed, not consumed: hashing twice sees the same live bigint.
    assert_eq!(pycc_rt_hash_int(negative), -549_755_813_888);
    pycc_rt_bigint_release(negative);
    let minus_2_62 = pycc_rt_int_sub(small(0), pow2(62));
    assert_eq!(hash_owned(minus_2_62), -2);
}

#[test]
fn tuple_lanes_fold_like_cpython() {
    assert_eq!(tuple(&[]), 5_740_354_900_026_072_187);
    assert_eq!(tuple(&[-2]), 8_078_679_518_589_016_365);
    assert_eq!(tuple(&[1, 2]), -3_550_055_125_485_641_917);
    assert_eq!(tuple(&[1, 0]), -5_164_621_852_614_943_976);
    assert_eq!(tuple(&[1, -2]), -6_779_188_579_744_246_035);
    assert_eq!(
        tuple(&[tuple(&[1, 2]), 3]),
        -333_907_151_259_015_829,
        "a nested tuple's hash is an ordinary lane"
    );
}

/// The multiplicative inverse of the odd `a` modulo `2**64`, by Newton's
/// iteration (each step doubles the correct low bits).
fn inverse(a: u64) -> u64 {
    let mut inv: u64 = a;
    for _ in 0..6 {
        inv = inv.wrapping_mul(2u64.wrapping_sub(a.wrapping_mul(inv)));
    }
    assert_eq!(a.wrapping_mul(inv), 1);
    inv
}

#[test]
fn an_accumulator_of_all_ones_maps_to_cpythons_sentinel() {
    // CPython's `tuplehash` never returns `-1`: an accumulator of
    // `u64::MAX` becomes `1546275796`. Solve one lane backwards from it.
    const XXPRIME_1: u64 = 11_400_714_785_074_694_791;
    const XXPRIME_2: u64 = 14_029_467_366_897_019_727;
    const XXPRIME_5: u64 = 2_870_177_450_012_600_261;
    let before_finish = u64::MAX.wrapping_sub(1 ^ (XXPRIME_5 ^ 3_527_539));
    let rotated = before_finish.wrapping_mul(inverse(XXPRIME_1));
    let summed = rotated.rotate_right(31);
    let lane = summed
        .wrapping_sub(XXPRIME_5)
        .wrapping_mul(inverse(XXPRIME_2));
    assert_eq!(tuple(&[lane as i64]), 1_546_275_796);
}

#[test]
fn an_instance_pointer_hashes_like_py_hash_pointer() {
    assert_eq!(pycc_rt_hash_pointer(0x10 as *const _), 1);
    assert_eq!(pycc_rt_hash_pointer(0x1234_5670 as *const _), 0x0123_4567);
    assert_eq!(
        pycc_rt_hash_pointer(usize::MAX as *const _),
        -2,
        "-1 maps to -2"
    );
}

/// `-word`, for an encoded `word`.
fn negate(word: i64) -> i64 {
    pycc_rt_int_sub(small(0), word)
}

/// `slot_tp_hash` of `word`, then releases it when it is a heap bigint.
fn slot_owned(word: i64) -> i64 {
    let hash = pycc_rt_hash_slot_int(word);
    pycc_rt_bigint_release(word);
    hash
}

#[test]
fn a_hash_method_result_passes_through_slot_tp_hash_like_cpython() {
    assert_eq!(pycc_rt_hash_slot_int(small(1 << 61)), 1 << 61);
    assert_eq!(pycc_rt_hash_slot_int(small(-1)), -2);
    assert_eq!(pycc_rt_hash_slot_int(small(-2)), -2);
    assert_eq!(pycc_rt_hash_slot_int(6), 1, "D-141's True marker");
    assert_eq!(pycc_rt_hash_slot_int(2), 0, "D-141's False marker");
    // Heap bigints that still fit an i64 are used unreduced.
    assert_eq!(slot_owned(pow2(62)), 1 << 62);
    assert_eq!(slot_owned(pycc_rt_int_sub(pow2(63), small(1))), i64::MAX);
    assert_eq!(slot_owned(negate(pow2(63))), i64::MIN);
    assert_eq!(
        slot_owned(pycc_rt_int_sub(negate(pow2(62)), small(1))),
        -(1 << 62) - 1
    );
    // Wider values are reduced like `long_hash`.
    assert_eq!(slot_owned(pow2(63)), 4);
    assert_eq!(slot_owned(pycc_rt_int_sub(negate(pow2(63)), small(1))), -5);
    assert_eq!(slot_owned(negate(pow2(70))), -512);
    // Borrowed, not consumed.
    let word = pow2(64);
    assert_eq!(pycc_rt_hash_slot_int(word), 8);
    assert_eq!(pycc_rt_hash_slot_int(word), 8);
    pycc_rt_bigint_release(word);
}
