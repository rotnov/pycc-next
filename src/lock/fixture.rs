//! Test fixtures: installed distributions written the way `pip` writes
//! them, with real RECORD hashes, so resolver tests need no network.

use crate::embed::sha256::sha256_hex;
use std::path::{Path, PathBuf};

/// Unpadded urlsafe base64, as RECORD hashes use.
pub(crate) fn urlsafe_b64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let mut acc = 0u32;
        for (i, byte) in chunk.iter().enumerate() {
            acc |= u32::from(*byte) << (16 - 8 * i);
        }
        for i in 0..=chunk.len() {
            out.push(ALPHABET[((acc >> (18 - 6 * i)) & 63) as usize] as char);
        }
    }
    out
}

/// The RECORD hash field for `bytes`.
pub(crate) fn record_hash(bytes: &[u8]) -> String {
    let hex = sha256_hex(bytes);
    let raw: Vec<u8> = (0..32)
        .map(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).unwrap())
        .collect();
    format!("sha256={}", urlsafe_b64(&raw))
}

/// Writes distribution `name` `version` into `site`: its payload `files`,
/// METADATA with `requires` as `Requires-Dist`, INSTALLER, and a RECORD
/// listing all of them. Returns the dist-info directory.
pub(crate) fn write_dist(
    site: &Path,
    name: &str,
    version: &str,
    files: &[(&str, &[u8])],
    requires: &[&str],
) -> PathBuf {
    let dist_info = format!("{name}-{version}.dist-info");
    let dir = site.join(&dist_info);
    std::fs::create_dir_all(&dir).unwrap();
    let mut metadata = format!("Metadata-Version: 2.1\nName: {name}\nVersion: {version}\n");
    for requirement in requires {
        metadata.push_str(&format!("Requires-Dist: {requirement}\n"));
    }
    let mut record = String::new();
    let mut add = |path: &str, bytes: &[u8]| {
        let full = site.join(path);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(&full, bytes).unwrap();
        record.push_str(&format!("{path},{},{}\n", record_hash(bytes), bytes.len()));
    };
    for (path, bytes) in files {
        add(path, bytes);
    }
    add(&format!("{dist_info}/METADATA"), metadata.as_bytes());
    add(&format!("{dist_info}/INSTALLER"), b"pip\n");
    record.push_str(&format!("{dist_info}/RECORD,,\n"));
    std::fs::write(dir.join("RECORD"), record).unwrap();
    dir
}

/// Appends one raw line to a dist-info's RECORD.
pub(crate) fn append_record(dist_info: &Path, line: &str) {
    let path = dist_info.join("RECORD");
    let mut text = std::fs::read_to_string(&path).unwrap();
    text.push_str(line);
    text.push('\n');
    std::fs::write(path, text).unwrap();
}
