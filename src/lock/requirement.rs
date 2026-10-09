//! A `Requires-Dist` value: the distribution it names, the extras it
//! requests and its environment marker. Version specifiers and `@ url`
//! references are skipped: the lock pins what is installed, and
//! specifier consistency is the installer's contract (the pycc.lock
//! decision entry, rule 2).

use super::marker::{Marker, normalize_name, parse_marker};
use std::collections::BTreeSet;

/// One parsed `Requires-Dist` requirement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Requirement {
    /// The PEP 503-normalized distribution name.
    pub(crate) name: String,
    /// The PEP 685-normalized extras requested of it.
    pub(crate) extras: BTreeSet<String>,
    /// The marker and its source text, when present.
    pub(crate) marker: Option<(Marker, String)>,
}

fn is_name_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.')
}

/// Parses one `Requires-Dist` value.
pub(crate) fn parse_requirement(text: &str) -> Result<Requirement, String> {
    let refuse = |what: &str| format!("{what} in requirement `{text}`");
    let trimmed = text.trim_start();
    let name_len = trimmed
        .find(|ch| !is_name_char(ch))
        .unwrap_or(trimmed.len());
    let name = &trimmed[..name_len];
    if name.is_empty()
        || !name.starts_with(|ch: char| ch.is_ascii_alphanumeric())
        || !name.ends_with(|ch: char| ch.is_ascii_alphanumeric())
    {
        return Err(refuse("a missing or malformed distribution name"));
    }
    let mut rest = trimmed[name_len..].trim_start();
    let mut extras = BTreeSet::new();
    if let Some(after) = rest.strip_prefix('[') {
        let close = after.find(']').ok_or_else(|| refuse("an unclosed `[`"))?;
        for extra in after[..close].split(',') {
            let extra = extra.trim();
            if extra.is_empty() || !extra.chars().all(is_name_char) {
                return Err(refuse(&format!("a malformed extra `{extra}`")));
            }
            extras.insert(normalize_name(extra));
        }
        rest = after[close + 1..].trim_start();
    }
    let marker_text = if let Some(url) = rest.strip_prefix('@') {
        // A URL runs to whitespace; the marker, if any, follows a `;`
        // after that whitespace (PEP 508).
        let url = url.trim_start();
        let end = url.find(char::is_whitespace).unwrap_or(url.len());
        let after = url[end..].trim_start();
        match after.strip_prefix(';') {
            Some(marker) => Some(marker),
            None if after.is_empty() => None,
            None => return Err(refuse("text after a URL that is not a marker")),
        }
    } else {
        rest.split_once(';').map(|(_, marker)| marker)
    };
    let marker = match marker_text {
        Some(marker) => {
            let marker = marker.trim();
            Some((parse_marker(marker)?, marker.to_string()))
        }
        None => None,
    };
    Ok(Requirement {
        name: normalize_name(name),
        extras,
        marker,
    })
}

#[cfg(test)]
#[path = "requirement_tests.rs"]
mod tests;
