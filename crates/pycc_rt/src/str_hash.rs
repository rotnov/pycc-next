//! `hash(s)` of a `str`: CPython 3.13's keyed SipHash-1-3 (`Python/pyhash.c`)
//! over the string's PEP 393 representation, with a per-process key.
//!
//! An unkeyed hash lets anyone who controls the strings (`input()`, a file)
//! pick colliding keys and drive every dict and set operation to linear
//! time. Like CPython, the key comes from OS randomness unless
//! `PYTHONHASHSEED` is set: `0` disables randomization (an all-zero key), and
//! an integer `1..=4294967295` derives the key with CPython's `lcg_urandom`.
//! The same seed therefore gives the same hashes -- and the same set
//! iteration order -- as CPython itself.

use std::sync::OnceLock;

/// The process's `(k0, k1)` SipHash key.
fn key() -> (u64, u64) {
    static KEY: OnceLock<(u64, u64)> = OnceLock::new();
    *KEY.get_or_init(|| key_for(std::env::var("PYTHONHASHSEED").ok().as_deref()))
}

/// The key for a `PYTHONHASHSEED` value (`None` or `"random"`: random).
/// A value CPython would reject falls back to a random key.
fn key_for(seed: Option<&str>) -> (u64, u64) {
    match seed.map(str::parse::<u32>) {
        Some(Ok(0)) => (0, 0),
        Some(Ok(seed)) => lcg_key(seed),
        _ => random_key(),
    }
}

/// CPython's `lcg_urandom`: the first 16 of the 24 secret bytes.
fn lcg_key(seed: u32) -> (u64, u64) {
    let mut x = seed;
    let mut bytes = [0u8; 16];
    for byte in &mut bytes {
        x = x.wrapping_mul(214_013).wrapping_add(2_531_011);
        *byte = ((x >> 16) & 0xff) as u8;
    }
    let (k0, k1) = bytes.split_at(8);
    (
        u64::from_le_bytes(k0.try_into().unwrap()),
        u64::from_le_bytes(k1.try_into().unwrap()),
    )
}

/// A key from the OS: `std`'s `RandomState` is seeded from the platform's
/// randomness source (`getrandom` on Linux).
fn random_key() -> (u64, u64) {
    use std::hash::BuildHasher;
    let state = std::collections::hash_map::RandomState::new();
    (state.hash_one(0x6b30u64), state.hash_one(0x6b31u64))
}

fn round(v: &mut [u64; 4]) {
    v[0] = v[0].wrapping_add(v[1]);
    v[2] = v[2].wrapping_add(v[3]);
    v[1] = v[1].rotate_left(13) ^ v[0];
    v[3] = v[3].rotate_left(16) ^ v[2];
    v[0] = v[0].rotate_left(32);
    v[2] = v[2].wrapping_add(v[1]);
    v[0] = v[0].wrapping_add(v[3]);
    v[1] = v[1].rotate_left(17) ^ v[2];
    v[3] = v[3].rotate_left(21) ^ v[0];
    v[2] = v[2].rotate_left(32);
}

/// CPython's `siphash13`.
fn siphash13(k0: u64, k1: u64, data: &[u8]) -> u64 {
    let mut v = [
        k0 ^ 0x736f_6d65_7073_6575,
        k1 ^ 0x646f_7261_6e64_6f6d,
        k0 ^ 0x6c79_6765_6e65_7261,
        k1 ^ 0x7465_6462_7974_6573,
    ];
    let mut chunks = data.chunks_exact(8);
    for chunk in &mut chunks {
        let m = u64::from_le_bytes(chunk.try_into().unwrap());
        v[3] ^= m;
        round(&mut v);
        v[0] ^= m;
    }
    let mut tail = [0u8; 8];
    tail[..chunks.remainder().len()].copy_from_slice(chunks.remainder());
    let b = ((data.len() as u64) << 56) | u64::from_le_bytes(tail);
    v[3] ^= b;
    round(&mut v);
    v[0] ^= b;
    v[2] ^= 0xff;
    round(&mut v);
    round(&mut v);
    round(&mut v);
    (v[0] ^ v[1]) ^ (v[2] ^ v[3])
}

/// The bytes CPython hashes for `text`: its PEP 393 representation
/// (Latin-1, UCS-2 or UCS-4 little-endian, by its widest code point).
fn pep393_bytes(text: &str) -> Vec<u8> {
    let widest = text.chars().map(u32::from).max().unwrap_or(0);
    match widest {
        0..=0xff => text.chars().map(|c| u32::from(c) as u8).collect(),
        0x100..=0xffff => text
            .chars()
            .flat_map(|c| (u32::from(c) as u16).to_le_bytes())
            .collect(),
        _ => text.chars().flat_map(|c| u32::from(c).to_le_bytes()).collect(),
    }
}

/// `hash(s)` of a `str` whose UTF-8 bytes are `bytes`.
pub(crate) fn hash_str(bytes: &[u8]) -> i64 {
    hash_str_with(key(), bytes)
}

fn hash_str_with((k0, k1): (u64, u64), bytes: &[u8]) -> i64 {
    if bytes.is_empty() {
        return 0;
    }
    let hash = match std::str::from_utf8(bytes) {
        Ok(text) => siphash13(k0, k1, &pep393_bytes(text)),
        Err(_) => siphash13(k0, k1, bytes),
    } as i64;
    if hash == -1 { -2 } else { hash }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `[hash(w) for w in WORDS]` from CPython 3.13 under each
    /// `PYTHONHASHSEED`.
    #[test]
    fn matches_cpython_for_fixed_seeds() {
        let words = [
            "",
            "a",
            "abc",
            "hello world!",
            "abcdefgh",
            "abcdefghi",
            "\u{e9}t\u{e9}",
            "\u{4e2d}\u{6587}",
            "\u{1f600}x",
        ];
        let expected: [(&str, [i64; 9]); 3] = [
            (
                "0",
                [
                    0,
                    4_644_417_185_603_328_019,
                    -4_594_863_902_769_663_758,
                    839_851_713_330_019_024,
                    4_574_395_652_268_504_554,
                    -532_774_252_720_507_163,
                    -1_033_370_291_238_291_217,
                    501_833_946_989_431_548,
                    -8_926_728_262_118_538_918,
                ],
            ),
            (
                "42",
                [
                    0,
                    -123_207_753_977_932_514,
                    3_869_580_338_025_362_921,
                    -8_695_723_632_380_824_742,
                    -5_457_871_895_989_710_762,
                    -5_970_266_004_662_334_337,
                    4_053_562_854_040_562_755,
                    4_786_665_480_535_555_245,
                    -7_412_435_959_675_129_227,
                ],
            ),
            (
                "4294967295",
                [
                    0,
                    -5_989_683_813_037_840_249,
                    -6_122_489_556_238_538_401,
                    3_716_697_847_625_381_019,
                    3_424_456_984_880_118_976,
                    6_340_115_119_033_867_570,
                    -8_233_957_282_333_719_077,
                    3_226_049_666_939_785_657,
                    461_681_878_319_395_138,
                ],
            ),
        ];
        for (seed, hashes) in expected {
            let key = key_for(Some(seed));
            for (word, hash) in words.iter().zip(hashes) {
                assert_eq!(hash_str_with(key, word.as_bytes()), hash, "{seed} {word:?}");
            }
        }
    }

    #[test]
    fn unset_or_random_seed_draws_a_fresh_key() {
        assert_ne!(key_for(None), key_for(None));
        assert_ne!(key_for(Some("random")), (0, 0));
    }
}
