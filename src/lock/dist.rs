//! Reading installed distributions: the `*.dist-info` directories of a
//! site directory, their METADATA headers and their RECORD rows (the
//! pycc.lock decision entry, rules 1-4).

use super::marker::normalize_name;
use crate::embed::layout::EmbedPlatform;
use std::path::{Path, PathBuf};

/// Which scanned site directory holds a distribution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum SiteKind {
    Purelib,
    Platlib,
}

impl SiteKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            SiteKind::Purelib => "purelib",
            SiteKind::Platlib => "platlib",
        }
    }
}

/// One scanned site directory, canonicalized.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Site {
    pub(crate) path: PathBuf,
    pub(crate) kind: SiteKind,
}

/// One indexed `*.dist-info` directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DistInfo {
    /// The PEP 503-normalized `{name}` part of the directory name.
    pub(crate) name: String,
    /// The directory's own name, `{name}-{version}.dist-info`.
    pub(crate) dir_name: String,
    /// The site directory holding it.
    pub(crate) site: Site,
}

impl DistInfo {
    pub(crate) fn path(&self) -> PathBuf {
        self.site.path.join(&self.dir_name)
    }

    /// A human-readable location for messages.
    pub(crate) fn display(&self) -> String {
        self.path().display().to_string()
    }
}

/// Lists the indexable `*.dist-info` directories of `site`, sorted by
/// directory name. A directory name without `-` is not indexed.
pub(crate) fn scan_site(site: &Site) -> Result<Vec<DistInfo>, String> {
    let entries = std::fs::read_dir(&site.path).map_err(|e| {
        format!(
            "cannot read the {} site directory `{}`: {e}",
            site.kind.as_str(),
            site.path.display()
        )
    })?;
    let mut found = Vec::new();
    for entry in entries.flatten() {
        // A non-UTF-8 name cannot be a dist-info directory either.
        let file_name = entry.file_name();
        let Some(stem) = file_name
            .to_str()
            .and_then(|n| n.strip_suffix(".dist-info"))
        else {
            continue;
        };
        let Some((name, _version)) = stem.rsplit_once('-') else {
            continue;
        };
        if !entry.path().is_dir() {
            continue;
        }
        found.push(DistInfo {
            name: normalize_name(name),
            dir_name: format!("{stem}.dist-info"),
            site: site.clone(),
        });
    }
    found.sort_by(|a, b| a.dir_name.cmp(&b.dir_name));
    Ok(found)
}

/// The METADATA headers the resolver reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Metadata {
    pub(crate) name: String,
    pub(crate) version: String,
    pub(crate) requires_dist: Vec<String>,
}

/// Parses METADATA's header block (RFC 822 style: continuation lines start
/// with whitespace; the headers end at the first blank line).
pub(crate) fn parse_metadata(text: &str) -> Result<Metadata, String> {
    let mut headers: Vec<(String, String)> = Vec::new();
    for line in text.lines() {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.is_empty() {
            break;
        }
        if line.starts_with([' ', '\t']) {
            if let Some((_, value)) = headers.last_mut() {
                value.push('\n');
                value.push_str(line.trim());
            }
            continue;
        }
        if let Some((key, value)) = line.split_once(':') {
            headers.push((key.trim().to_ascii_lowercase(), value.trim().to_string()));
        }
    }
    let first = |key: &str| {
        headers
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
            .filter(|v| !v.is_empty())
    };
    let name = first("name").ok_or("METADATA has no `Name` header")?;
    let version = first("version").ok_or("METADATA has no `Version` header")?;
    let requires_dist = headers
        .iter()
        .filter(|(k, _)| k == "requires-dist")
        .map(|(_, v)| v.replace('\n', " "))
        .collect();
    Ok(Metadata {
        name,
        version,
        requires_dist,
    })
}

/// One RECORD row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RecordEntry {
    pub(crate) path: String,
    pub(crate) hash: String,
}

/// Parses RECORD as RFC 4180 CSV (quoted fields, doubled quotes). Each
/// row has three fields: path, hash, size.
pub(crate) fn parse_record(text: &str) -> Result<Vec<RecordEntry>, String> {
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut chars = text.chars().peekable();
    let mut quoted = false;
    let mut row_started = false;
    while let Some(ch) = chars.next() {
        if quoted {
            if ch == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    field.push('"');
                } else {
                    quoted = false;
                }
            } else {
                field.push(ch);
            }
            continue;
        }
        match ch {
            '"' => {
                quoted = true;
                row_started = true;
            }
            ',' => {
                row.push(std::mem::take(&mut field));
                row_started = true;
            }
            '\r' => {}
            '\n' => {
                if row_started || !field.is_empty() {
                    row.push(std::mem::take(&mut field));
                    rows.push(std::mem::take(&mut row));
                }
                row_started = false;
            }
            _ => {
                field.push(ch);
                row_started = true;
            }
        }
    }
    if quoted {
        return Err("RECORD has an unterminated quoted field".to_string());
    }
    if row_started || !field.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows.into_iter()
        .map(|row| match <[String; 3]>::try_from(row) {
            Ok([path, hash, _size]) if !path.is_empty() => Ok(RecordEntry { path, hash }),
            Ok(_) => Err("RECORD has a row with an empty path".to_string()),
            Err(row) => Err(format!(
                "RECORD has a row with {} fields instead of 3: `{}`",
                row.len(),
                row.join(",")
            )),
        })
        .collect()
}

/// Decodes unpadded urlsafe base64 (RFC 4648 section 5), as RECORD
/// hashes use; `None` for an invalid character or length.
pub(crate) fn decode_urlsafe_b64(text: &str) -> Option<Vec<u8>> {
    let value = |byte: u8| -> Option<u32> {
        Some(match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'-' => 62,
            b'_' => 63,
            _ => return None,
        } as u32)
    };
    let bytes = text.as_bytes();
    if bytes.len() % 4 == 1 {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len() * 3 / 4);
    for chunk in bytes.chunks(4) {
        let mut acc = 0u32;
        for (i, &byte) in chunk.iter().enumerate() {
            acc |= value(byte)? << (18 - 6 * i);
        }
        let produced = chunk.len() - 1;
        out.extend_from_slice(&acc.to_be_bytes()[1..1 + produced]);
    }
    Some(out)
}

/// Whether a `/`-separated RECORD path names import root `root` on
/// `platform`: a path under `root/`, or exactly `root` plus one of the
/// platform's import suffixes. POSIX: `root.py`, `root.pyc`, `root.so` or
/// `root.<anything>.so`. Windows (#1296), whose import system also loads
/// `.pyw` sources and `.pyd` extensions: `root.py`, `root.pyw`,
/// `root.pyc`, `root.pyd` or `root.<anything>.pyd`.
pub(crate) fn names_root(path: &str, root: &str, platform: EmbedPlatform) -> bool {
    let (sources, extension): (&[&str], &str) = match platform {
        EmbedPlatform::Windows => (&[".py", ".pyw", ".pyc", ".pyd"], ".pyd"),
        EmbedPlatform::MacOs | EmbedPlatform::Linux => (&[".py", ".pyc", ".so"], ".so"),
    };
    if let Some(rest) = path.strip_prefix(root) {
        if rest.starts_with('/') {
            return rest.len() > 1;
        }
        if sources.contains(&rest) {
            return true;
        }
        if let Some(middle) = rest
            .strip_prefix('.')
            .and_then(|r| r.strip_suffix(extension))
        {
            return !middle.is_empty() && !middle.contains('/');
        }
    }
    false
}

/// Lexically normalizes `path` (`.` dropped, `..` pops a component)
/// without touching the filesystem.
pub(crate) fn normalize_lexically(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
#[path = "dist_tests.rs"]
mod tests;
