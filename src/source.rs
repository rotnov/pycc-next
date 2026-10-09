const UTF8_BOM: &[u8] = b"\xef\xbb\xbf";

#[derive(Clone, Copy)]
enum SourceEncoding {
    Utf8,
    Latin1,
    Ascii,
}

impl SourceEncoding {
    fn decode(self, bytes: &[u8], label: &str) -> Result<String, String> {
        match self {
            Self::Utf8 => std::str::from_utf8(bytes)
                .map(str::to_owned)
                .map_err(|_| format!("source is not valid {label}")),
            Self::Latin1 => Ok(bytes.iter().map(|byte| char::from(*byte)).collect()),
            Self::Ascii if bytes.is_ascii() => Ok(std::str::from_utf8(bytes)
                .expect("ASCII is valid UTF-8")
                .to_owned()),
            Self::Ascii => Err(format!("source is not valid {label}")),
        }
    }
}

pub fn decode_python_source(bytes: &[u8]) -> Result<String, String> {
    let (bytes, has_utf8_bom) = match bytes.strip_prefix(UTF8_BOM) {
        Some(without_bom) => (without_bom, true),
        None => (bytes, false),
    };
    let declared = detect_encoding_cookie(bytes);
    if has_utf8_bom
        && declared
            .as_deref()
            .is_some_and(|label| !bom_allows_encoding_cookie(label))
    {
        return Err("UTF-8 BOM conflicts with the declared source encoding".to_string());
    }
    let label = declared.as_deref().unwrap_or("utf-8");
    let encoding =
        resolve_encoding(label).ok_or_else(|| format!("unknown source encoding `{label}`"))?;

    encoding
        .decode(bytes, label)
        .map(|source| source.replace("\r\n", "\n").replace('\r', "\n"))
}

fn detect_encoding_cookie(bytes: &[u8]) -> Option<String> {
    let (first_line, rest) = take_line(bytes);
    if let Some(encoding) = find_cookie(first_line) {
        return Some(encoding);
    }
    if !is_comment_or_blank(first_line) {
        return None;
    }

    let (second_line, _) = take_line(rest);
    find_cookie(second_line)
}

fn take_line(bytes: &[u8]) -> (&[u8], &[u8]) {
    match bytes.iter().position(|byte| matches!(byte, b'\r' | b'\n')) {
        Some(newline) => {
            let mut end = newline + 1;
            if bytes[newline] == b'\r' && bytes.get(end) == Some(&b'\n') {
                end += 1;
            }
            bytes.split_at(end)
        }
        None => (bytes, &[]),
    }
}

fn find_cookie(line: &[u8]) -> Option<String> {
    let leading_whitespace = line
        .iter()
        .take_while(|byte| matches!(byte, b' ' | b'\t' | b'\x0c'))
        .count();
    let comment = line[leading_whitespace..].strip_prefix(b"#")?;
    let mut search_start = 0;

    while let Some(relative_start) = comment[search_start..]
        .windows(b"coding".len())
        .position(|window| window == b"coding")
    {
        let coding_start = search_start + relative_start;
        let after_coding = &comment[coding_start + b"coding".len()..];
        if let Some(after_marker) = after_coding
            .strip_prefix(b":")
            .or_else(|| after_coding.strip_prefix(b"="))
        {
            let label_start = after_marker
                .iter()
                .take_while(|byte| matches!(byte, b' ' | b'\t'))
                .count();
            let label_len = after_marker[label_start..]
                .iter()
                .take_while(|byte| {
                    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')
                })
                .count();
            if label_len != 0 {
                return Some(
                    std::str::from_utf8(&after_marker[label_start..label_start + label_len])
                        .expect("the encoding-cookie label only permits ASCII")
                        .to_string(),
                );
            }
        }
        search_start = coding_start + 1;
    }
    None
}

fn is_comment_or_blank(line: &[u8]) -> bool {
    match line
        .iter()
        .copied()
        .find(|byte| !matches!(byte, b' ' | b'\t' | b'\x0c'))
    {
        None => true,
        Some(byte) => matches!(byte, b'#' | b'\r' | b'\n'),
    }
}

fn resolve_encoding(label: &str) -> Option<SourceEncoding> {
    let normalized = normalize_python_encoding(label);
    resolve_normalized_encoding(&normalized).or_else(|| {
        normalized
            .contains('.')
            .then(|| normalized.replace('.', "-"))
            .and_then(|fallback| resolve_normalized_encoding(&fallback))
    })
}

fn resolve_normalized_encoding(normalized: &str) -> Option<SourceEncoding> {
    match normalized {
        "utf-8" | "cp65001" | "u8" | "utf" | "utf8" | "utf8-ucs2" | "utf8-ucs4" => {
            Some(SourceEncoding::Utf8)
        }
        "iso-8859-1" | "8859" | "cp819" | "csisolatin1" | "ibm819" | "iso8859" | "iso8859-1"
        | "iso-8859-1-1987" | "iso-ir-100" | "l1" | "latin" | "latin1" => {
            Some(SourceEncoding::Latin1)
        }
        "ascii" | "us-ascii" | "646" | "ansi-x3.4-1968" | "ansi-x3.4-1986" | "ansi-x3-4-1968"
        | "cp367" | "csascii" | "ibm367" | "iso646-us" | "iso-646.irv-1991" | "iso-ir-6" | "us" => {
            Some(SourceEncoding::Ascii)
        }
        _ => None,
    }
}

fn bom_allows_encoding_cookie(label: &str) -> bool {
    let normalized = label
        .chars()
        .take(12)
        .map(|character| {
            if character == '_' {
                '-'
            } else {
                character.to_ascii_lowercase()
            }
        })
        .collect::<String>();
    normalized == "utf-8" || normalized.starts_with("utf-8-")
}

fn normalize_python_encoding(label: &str) -> String {
    let mut normalized = String::with_capacity(label.len());
    let mut separator = false;
    for character in label.chars() {
        if character.is_ascii_alphanumeric() || character == '.' {
            if separator && !normalized.is_empty() {
                normalized.push('-');
            }
            normalized.push(character.to_ascii_lowercase());
            separator = false;
        } else {
            separator = true;
        }
    }
    if normalized == "utf-8" || normalized.starts_with("utf-8-") {
        "utf-8".to_string()
    } else if normalized == "latin-1"
        || normalized == "iso-8859-1"
        || normalized == "iso-latin-1"
        || normalized.starts_with("latin-1-")
        || normalized.starts_with("iso-8859-1-")
        || normalized.starts_with("iso-latin-1-")
    {
        "iso-8859-1".to_string()
    } else {
        normalized
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolved_kind(label: &str) -> &'static str {
        match resolve_encoding(label).unwrap() {
            SourceEncoding::Utf8 => "utf8",
            SourceEncoding::Latin1 => "latin1",
            SourceEncoding::Ascii => "ascii",
        }
    }

    #[test]
    fn defaults_to_utf8() {
        assert_eq!(
            decode_python_source(b"# caf\xc3\xa9\nprint(42)\n").unwrap(),
            "# caf\u{e9}\nprint(42)\n"
        );
    }

    #[test]
    fn strips_a_utf8_bom_and_accepts_a_matching_cookie() {
        assert_eq!(
            decode_python_source(b"\xef\xbb\xbfprint(42)\n").unwrap(),
            "print(42)\n"
        );
        assert_eq!(
            decode_python_source(b"\xef\xbb\xbf# coding: utf-8-sig\nprint(42)\n").unwrap(),
            "# coding: utf-8-sig\nprint(42)\n"
        );
    }

    #[test]
    fn decodes_a_first_line_latin1_cookie_without_windows_1252_substitution() {
        assert_eq!(
            decode_python_source(b"# coding: latin-1 # Andr\xe9\n# \x80\xe9\nprint(42)\n").unwrap(),
            "# coding: latin-1 # Andr\u{e9}\n# \u{80}\u{e9}\nprint(42)\n"
        );
    }

    #[test]
    fn recognizes_the_full_pep_263_cookie_shape_without_a_regex_dependency() {
        assert_eq!(
            find_cookie(b" \t\x0c# coding= latin_1\n").as_deref(),
            Some("latin_1")
        );
        assert_eq!(
            find_cookie(b"# coding utf-8; coding: ascii\n").as_deref(),
            Some("ascii")
        );
        assert_eq!(find_cookie(b"# coding: !utf-8\n"), None);
        assert_eq!(find_cookie(b"print('# coding: latin-1')\n"), None);
    }

    #[test]
    fn decodes_a_second_line_cookie_after_a_shebang() {
        assert_eq!(
            decode_python_source(
                b"#!/usr/bin/env python Andr\xe9\r# coding: iso-latin-1-unix\r# caf\xe9\rprint(42)\r"
            )
            .unwrap(),
            "#!/usr/bin/env python Andr\u{e9}\n# coding: iso-latin-1-unix\n# caf\u{e9}\nprint(42)\n"
        );
    }

    #[test]
    fn rejects_a_codec_without_a_python_compatible_decoder() {
        assert!(decode_python_source(b"# coding: gbk\n# \x80\nprint(42)\n").is_err());
    }

    #[test]
    fn supports_strict_ascii_and_its_aliases() {
        assert_eq!(
            decode_python_source(b"# coding: us_ascii\nprint(42)\n").unwrap(),
            "# coding: us_ascii\nprint(42)\n"
        );
        assert!(decode_python_source(b"# coding: ascii\n# \xff\n").is_err());
    }

    #[test]
    fn rejects_invalid_default_utf8() {
        assert!(decode_python_source(b"print(42)\n# \xff\n").is_err());
    }

    #[test]
    fn rejects_unknown_encodings_and_bom_conflicts() {
        assert!(decode_python_source(b"# coding: utf-42\nprint(42)\n").is_err());
        assert!(decode_python_source(b"\xef\xbb\xbf# coding: latin-1\nprint(42)\n").is_err());
        for alias in ["utf8", "cp65001", "utf8_ucs4", "utf--8", "utf__8"] {
            let source = format!("\u{feff}# coding: {alias}\nprint(42)\n");
            assert!(decode_python_source(source.as_bytes()).is_err());
        }
    }

    #[test]
    fn bom_cookie_agreement_uses_cpython_tokenizer_normalization() {
        for alias in ["utf-8", "UTF_8", "utf-8-sig", "utf_8_sig"] {
            assert!(bom_allows_encoding_cookie(alias), "alias: {alias}");
        }
        for alias in ["utf8", "utf--8", "utf__8", "cp65001"] {
            assert!(!bom_allows_encoding_cookie(alias), "alias: {alias}");
        }
    }

    #[test]
    fn ignores_a_second_line_cookie_after_python_code() {
        assert_eq!(
            decode_python_source(b"print(42)\n# coding: latin-1\n").unwrap(),
            "print(42)\n# coding: latin-1\n"
        );
    }

    #[test]
    fn accepts_empty_and_single_line_sources() {
        assert_eq!(decode_python_source(b"").unwrap(), "");
        assert_eq!(decode_python_source(b"\n").unwrap(), "\n");
        assert_eq!(decode_python_source(b"\r").unwrap(), "\n");
        assert_eq!(decode_python_source(b"\r\n").unwrap(), "\n");
    }

    #[test]
    fn python_encoding_alias_normalization_matches_the_cookie_rules() {
        assert_eq!(resolved_kind("UTF_8"), "utf8");
        assert_eq!(resolved_kind("utf--8"), "utf8");
        assert_eq!(resolved_kind("utf8_ucs4"), "utf8");
        assert_eq!(resolved_kind("cp65001"), "utf8");
        assert_eq!(resolved_kind("latin-1-unix"), "latin1");
        assert_eq!(resolved_kind("latin__1"), "latin1");
        assert_eq!(resolved_kind("cp819"), "latin1");
        assert_eq!(resolved_kind("iso_8859_1"), "latin1");
        assert_eq!(resolved_kind("iso-latin-1"), "latin1");
        assert_eq!(resolved_kind("iso.8859.1"), "latin1");
        assert_eq!(resolved_kind("646"), "ascii");
        assert_eq!(resolved_kind("iso_646.irv_1991"), "ascii");
        assert_eq!(resolved_kind("us.ascii"), "ascii");
    }
}
