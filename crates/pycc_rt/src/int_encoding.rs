//! `int`'s one-word encoding and its heap bigint representation.
//!
//! Split out of `lib.rs` under this repository's "keep source files
//! decomposable" rule while #147 (D-179) was adding `magnitude_sign` and
//! `encoded_int_cmp` here. The cut is by cohesion: everything that decides
//! *how an `int` value is represented* lives in this module, while the
//! arithmetic, comparison, and formatting operations *over* that
//! representation stay in `lib.rs` beside their `extern "C"` wrappers. No
//! `#[unsafe(no_mangle)] pub extern "C"` symbol moved -- the ABI surface is
//! unchanged.
//!
//! Everything here is `pub(crate)`: `pycc_rt` owns this representation, and
//! nothing outside the crate may interpret an encoded word.

/// See D-061/D-141: every int-compatible value is one LLVM `i64`. Odd words
/// are ordinary smallints; exact words `2` and `6` preserve `False` and
/// `True` identity after a bool-to-int boundary; non-zero words aligned to
/// four bytes are heap `BigInt` pointers. `classify_encoded_int` is the one
/// fail-closed classifier used before interpreting any even word.
pub(crate) const TAG_BIT: i64 = 1;
pub(crate) const LOW_TAG_MASK: i64 = 0b11;
pub(crate) const BOOL_FALSE_MARKER: i64 = 0b0010;
pub(crate) const BOOL_TRUE_MARKER: i64 = 0b0110;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EncodedIntKind {
    SmallInt,
    BoolFalse,
    BoolTrue,
    BigInt,
}

pub(crate) fn tag_smallint(value: i64) -> i64 {
    (value << 1) | TAG_BIT
}

pub(crate) fn untag_smallint(tagged: i64) -> i64 {
    tagged >> 1 // arithmetic (sign-extending) shift for `i64`
}

pub(crate) fn is_smallint(tagged: i64) -> bool {
    tagged & TAG_BIT == TAG_BIT
}

/// Classifies every word in the int-compatible ABI before any pointer cast.
/// Odd words are ordinary smallints; two exact low-tag-`10` words preserve
/// bool identity; non-zero aligned words are bigint pointers. Every other
/// pattern fails closed instead of being dereferenced as an attacker-chosen
/// pointer.
pub(crate) fn classify_encoded_int(encoded: i64) -> EncodedIntKind {
    if is_smallint(encoded) {
        EncodedIntKind::SmallInt
    } else if encoded == BOOL_FALSE_MARKER {
        EncodedIntKind::BoolFalse
    } else if encoded == BOOL_TRUE_MARKER {
        EncodedIntKind::BoolTrue
    } else if encoded != 0 && encoded & LOW_TAG_MASK == 0 {
        EncodedIntKind::BigInt
    } else {
        panic!("pycc_rt: invalid encoded int word {encoded:#x}")
    }
}

pub(crate) fn inline_int_value(encoded: i64) -> Option<i64> {
    match classify_encoded_int(encoded) {
        EncodedIntKind::SmallInt => Some(untag_smallint(encoded)),
        EncodedIntKind::BoolFalse => Some(0),
        EncodedIntKind::BoolTrue => Some(1),
        EncodedIntKind::BigInt => None,
    }
}

/// `None` when `value` needs the full 64 bits (including sign) to
/// represent -- i.e. tagging then untagging would not round-trip.
pub(crate) fn fits_smallint(value: i64) -> Option<i64> {
    let tagged = tag_smallint(value);
    (untag_smallint(tagged) == value).then_some(tagged)
}

/// D-058: hand-rolled sign-magnitude limbs, base 2^32, little-endian,
/// no trailing zero limbs except a single `[0]` representing zero itself.
///
/// Reference-counted since #146 Part 1, which narrows D-058's original
/// "never freed" concession: `rc` is an ordinary non-atomic `Cell` because
/// pycc emits single-threaded programs, exactly like `PyStrObj`'s own
/// D-060/D-074 counter. `tag_bigint` hands out the birth reference at
/// `rc == 1`; `bigint_retain`/`bigint_release` are the only two operations
/// that move it, and the object is freed when the last reference goes away.
///
/// The counter is *not* a general ownership model: only the site families
/// #146 Part 1 enumerates (named storage slots and loop-induction
/// variables) participate. Unbound arithmetic temporaries still leak --
/// that is Part 2 (#625) -- and the accepted residual concessions are
/// listed in the ADR that narrows D-058.
pub(crate) struct BigIntObj {
    /// Live references. `1` at birth; the object is freed at the release
    /// that would take it to `0`.
    pub(crate) rc: std::cell::Cell<u32>,
    pub(crate) negative: bool,
    pub(crate) limbs: Vec<u32>,
}

impl BigIntObj {
    /// The only constructor: every `BigIntObj` starts at one live
    /// reference, held by whoever receives `tag_bigint`'s word.
    pub(crate) fn new(negative: bool, limbs: Vec<u32>) -> Self {
        BigIntObj {
            rc: std::cell::Cell::new(1),
            negative,
            limbs,
        }
    }
}

// Counts `BigIntObj` frees so unit tests can observe a release actually
// running the destructor. Nothing outside `#[cfg(test)]` can see a free
// otherwise -- the freed word is simply gone. A line comment rather than a
// doc comment: rustdoc does not document items produced by a macro
// invocation, and `-D warnings` rejects the unused doc comment.
#[cfg(test)]
thread_local! {
    pub(crate) static BIGINT_DROPS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
impl Drop for BigIntObj {
    fn drop(&mut self) {
        BIGINT_DROPS.with(|c| c.set(c.get() + 1));
    }
}

const _: () = assert!(std::mem::align_of::<BigIntObj>() >= 4);
const _: () = assert!(std::mem::size_of::<*const BigIntObj>() <= std::mem::size_of::<i64>());

pub(crate) fn trim(limbs: &[u32]) -> Vec<u32> {
    let mut end = limbs.len();
    while end > 1 && limbs[end - 1] == 0 {
        end -= 1;
    }
    limbs[..end].to_vec()
}

pub(crate) fn magnitude_cmp(a: &[u32], b: &[u32]) -> std::cmp::Ordering {
    let (a, b) = (trim(a), trim(b));
    if a.len() != b.len() {
        return a.len().cmp(&b.len());
    }
    for i in (0..a.len()).rev() {
        if a[i] != b[i] {
            return a[i].cmp(&b[i]);
        }
    }
    std::cmp::Ordering::Equal
}

pub(crate) fn magnitude_add(a: &[u32], b: &[u32]) -> Vec<u32> {
    let mut result = Vec::with_capacity(a.len().max(b.len()) + 1);
    let mut carry: u64 = 0;
    for i in 0..a.len().max(b.len()) {
        let av = *a.get(i).unwrap_or(&0) as u64;
        let bv = *b.get(i).unwrap_or(&0) as u64;
        let sum = av + bv + carry;
        result.push((sum & 0xFFFF_FFFF) as u32);
        carry = sum >> 32;
    }
    if carry > 0 {
        result.push(carry as u32);
    }
    result
}

/// Requires `a >= b` (checked by every caller via `magnitude_cmp` first).
pub(crate) fn magnitude_sub(a: &[u32], b: &[u32]) -> Vec<u32> {
    let mut result = Vec::with_capacity(a.len());
    let mut borrow: i64 = 0;
    for (i, &av) in a.iter().enumerate() {
        let av = av as i64;
        let bv = *b.get(i).unwrap_or(&0) as i64;
        let mut diff = av - bv - borrow;
        if diff < 0 {
            diff += 1i64 << 32;
            borrow = 1;
        } else {
            borrow = 0;
        }
        result.push(diff as u32);
    }
    result
}

pub(crate) fn bigint_add_signed(
    a_neg: bool,
    a_mag: &[u32],
    b_neg: bool,
    b_mag: &[u32],
) -> BigIntObj {
    if a_neg == b_neg {
        BigIntObj::new(a_neg, trim(&magnitude_add(a_mag, b_mag)))
    } else {
        match magnitude_cmp(a_mag, b_mag) {
            std::cmp::Ordering::Equal => BigIntObj::new(false, vec![0]),
            std::cmp::Ordering::Greater => {
                BigIntObj::new(a_neg, trim(&magnitude_sub(a_mag, b_mag)))
            }
            std::cmp::Ordering::Less => BigIntObj::new(b_neg, trim(&magnitude_sub(b_mag, a_mag))),
        }
    }
}

pub(crate) fn bigint_from_i128(v: i128) -> BigIntObj {
    let negative = v < 0;
    let mut mag = v.unsigned_abs();
    let mut limbs = Vec::new();
    while mag > 0 {
        limbs.push((mag & 0xFFFF_FFFF) as u32);
        mag >>= 32;
    }
    if limbs.is_empty() {
        limbs.push(0);
    }
    BigIntObj::new(negative, limbs)
}

/// Moves `b` to the heap and hands back its encoded word carrying the
/// single birth reference (`rc == 1`). Whoever receives the word owns that
/// reference and is responsible for the matching `bigint_release`.
pub(crate) fn tag_bigint(b: BigIntObj) -> i64 {
    Box::into_raw(Box::new(b)) as i64
}

/// Adds one live reference to `word` when it names a heap bigint.
///
/// `word == 0` returns immediately *before* classification: `0` is not a
/// valid encoded int (it is `classify_encoded_int`'s fail-closed case), but
/// it is exactly what an `int` storage slot holds between
/// `storage_slot_at_entry`'s zero-initialization and the slot's first
/// store. Generated code releases a slot's previous value before every
/// store, so the very first store on any path necessarily passes `0` here.
/// Smallints and the two bool-identity markers are no-ops; `pycc_codegen`
/// additionally guards the call site inline so those never reach this
/// function at all (see the `(w & 3) == 0 && w != 0` test it emits).
pub(crate) fn bigint_retain(word: i64) {
    if word == 0 {
        return;
    }
    if classify_encoded_int(word) == EncodedIntKind::BigInt {
        // SAFETY: `classify_encoded_int` just proved `word` is a live
        // `BigIntObj` pointer, and pycc programs are single-threaded, so no
        // other reference can be mutating `rc` concurrently.
        let obj = unsafe { &*(word as *const BigIntObj) };
        obj.rc.set(obj.rc.get() + 1);
    }
}

/// Drops one live reference from `word`, freeing the object when the last
/// one goes away. `word == 0` and the inline kinds behave exactly as in
/// `bigint_retain`.
pub(crate) fn bigint_release(word: i64) {
    if word == 0 {
        return;
    }
    if classify_encoded_int(word) == EncodedIntKind::BigInt {
        // SAFETY: as in `bigint_retain`. The `Box::from_raw` reclaims the
        // very allocation `tag_bigint` leaked, and only on the release that
        // retires the final reference, so no other name can still hold it.
        let obj = unsafe { &*(word as *const BigIntObj) };
        let live = obj.rc.get();
        if live == 1 {
            drop(unsafe { Box::from_raw(word as *mut BigIntObj) });
        } else {
            obj.rc.set(live - 1);
        }
    }
}

/// # Safety
/// `tagged` must classify as a `BigIntObj` pointer. Classification is
/// repeated here so no dereference can bypass the fail-closed tag check.
pub(crate) unsafe fn bigint_ref<'a>(tagged: i64) -> &'a BigIntObj {
    if classify_encoded_int(tagged) != EncodedIntKind::BigInt {
        panic!("pycc_rt: internal error: attempted bigint dereference of a non-pointer int word")
    }
    unsafe { &*(tagged as *const BigIntObj) }
}

pub(crate) fn to_sign_and_magnitude(tagged: i64) -> (bool, Vec<u32>) {
    if let Some(v) = inline_int_value(tagged) {
        let negative = v < 0;
        let mag = v.unsigned_abs();
        (
            negative,
            trim(&[(mag & 0xFFFF_FFFF) as u32, (mag >> 32) as u32]),
        )
    } else {
        let b = unsafe { bigint_ref(tagged) };
        (b.negative, b.limbs.clone())
    }
}

/// The sign of a sign-magnitude pair as `-1`/`0`/`1`.
///
/// **Zero is decided by the magnitude, never by the `negative` flag.**
/// `bigint_add_signed` normalizes an equal-magnitude, opposite-sign result
/// to `BigIntObj::new(false, vec![0])`, so `!negative` does
/// *not* imply "positive" -- exactly the trap `pycc_rt_int_truthy` already
/// documents for its own magnitude inspection.
pub(crate) fn magnitude_sign(negative: bool, magnitude: &[u32]) -> i8 {
    if trim(magnitude) == [0] {
        0
    } else if negative {
        -1
    } else {
        1
    }
}

/// Orders two encoded int-compatible words (D-061/D-141) across the whole
/// representation, including heap bigints and the bool-identity markers.
///
/// Two inline operands take the cheap `i64` path so an ordinary smallint
/// `range` loop allocates nothing per iteration. As soon as either side is a
/// bigint the comparison decodes both to sign-magnitude and orders them
/// sign-first, then by magnitude (reversed for two negatives).
///
/// This is deliberately *not* `pycc_rt_int_cmp`'s general comparison
/// operator: `int_cmp` keeps its D-141 bigint boundary (see #618). This
/// helper exists for the loop-control comparison `range_continue` performs,
/// which #147 makes bigint-capable.
pub(crate) fn encoded_int_cmp(a: i64, b: i64) -> std::cmp::Ordering {
    if let (Some(a), Some(b)) = (inline_int_value(a), inline_int_value(b)) {
        return a.cmp(&b);
    }
    let (a_negative, a_magnitude) = to_sign_and_magnitude(a);
    let (b_negative, b_magnitude) = to_sign_and_magnitude(b);
    let a_sign = magnitude_sign(a_negative, &a_magnitude);
    let b_sign = magnitude_sign(b_negative, &b_magnitude);
    if a_sign != b_sign {
        return a_sign.cmp(&b_sign);
    }
    match a_sign {
        0 => std::cmp::Ordering::Equal,
        1 => magnitude_cmp(&a_magnitude, &b_magnitude),
        // Both negative: the larger magnitude is the smaller value.
        _ => magnitude_cmp(&b_magnitude, &a_magnitude),
    }
}
