use std::fmt::Write;

pub mod explain;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

impl Span {
    pub const fn new(start: u32, end: u32) -> Self {
        Self { start, end }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Diagnostic {
    pub code: &'static str,
    pub severity: Severity,
    pub message: String,
    pub span: Option<Span>,
    pub label: Option<String>,
    pub help: Option<String>,
}

impl Diagnostic {
    pub fn error(code: &'static str, message: impl Into<String>, span: Span) -> Self {
        let message = message.into();
        Self {
            code,
            severity: Severity::Error,
            label: Some(message.clone()),
            message,
            span: Some(span),
            help: None,
        }
    }

    pub fn warning(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            severity: Severity::Warning,
            message: message.into(),
            span: None,
            label: None,
            help: None,
        }
    }

    /// Attaches a structured `help` suggestion -- a determinate, safe fix
    /// derived from the diagnostic's own message (see D-152). Rendered as a
    /// single-element JSON `help[]` array by `render_json`; `render_human`
    /// has no `help:` line codepath yet (D-083, D-043's still-open gap).
    #[must_use]
    pub fn with_help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }
}

/// 1-indexed line and column, computed from a byte offset into `source` --
/// CLI_SPEC.md's human format shows `src/main.py:5:15` (1-indexed line:col),
/// and the JSON format's `spans[{line,col,...}]` needs the same. Hand-rolled
/// rather than pulling in `ruff_source_file`'s `LineIndex` (a separate crate
/// this workspace doesn't otherwise depend on) for one small function.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineCol {
    pub line: u32,
    pub column: u32,
}

pub fn byte_offset_to_line_col(source: &str, offset: u32) -> LineCol {
    let offset = offset as usize;
    let mut line = 1u32;
    let mut last_newline_end = 0usize;
    for (i, b) in source.bytes().enumerate() {
        if i >= offset {
            break;
        }
        if b == b'\n' {
            line += 1;
            last_newline_end = i + 1;
        }
    }
    let column = source[last_newline_end..offset].chars().count() as u32 + 1;
    LineCol { line, column }
}

/// CLI_SPEC.md's human diagnostic format, reproduced byte-for-byte for the
/// primary error + location block: `error[CODE]: message` / ` --> file:
/// line:col` / a blank gutter line / the source line prefixed with its line
/// number / a caret-underline beneath the span and its primary label.
/// `help:` lines are not rendered yet -- no code path currently populates a
/// safe suggestion (see D-043).
pub fn render_human(diag: &Diagnostic, file_path: &str, source: &str) -> String {
    let mut out = String::new();
    let severity_word = match diag.severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
    };
    let message = escape_terminal_controls(&diag.message, false);
    out.push_str(&format!("{severity_word}[{}]: {}\n", diag.code, message));
    let Some(span) = diag.span else {
        return out;
    };
    let start = byte_offset_to_line_col(source, span.start);
    let end = byte_offset_to_line_col(source, span.end);
    let file_path = display_path(file_path);
    out.push_str(&format!(
        " --> {file_path}:{}:{}\n",
        start.line, start.column
    ));
    let gutter_pad = " ".repeat(start.line.to_string().len());
    out.push_str(&gutter_pad);
    out.push_str(" |\n");
    let start_offset = span.start as usize;
    let end_offset = span.end as usize;
    let line_start = source[..start_offset]
        .rfind('\n')
        .map_or(0, |newline| newline + 1);
    let line_end = source[start_offset..]
        .find('\n')
        .map_or(source.len(), |newline| start_offset + newline);
    let source_line = escape_terminal_controls(&source[line_start..line_end], true);
    let source_prefix = escape_terminal_controls(&source[line_start..start_offset], true);
    out.push_str(&format!("{} | {source_line}\n", start.line));
    out.push_str(&gutter_pad);
    out.push_str(" | ");
    out.push_str(&display_padding(&source_prefix));
    let caret_len = if end.line == start.line {
        let highlight_end = end_offset.max(start_offset).min(line_end);
        let highlight = escape_terminal_controls(&source[start_offset..highlight_end], true);
        display_width(&highlight).max(1)
    } else {
        1
    };
    out.push_str(&"^".repeat(caret_len));
    if let Some(label) = &diag.label {
        out.push(' ');
        out.push_str(&escape_terminal_controls(label, false));
    }
    out.push('\n');
    out
}

/// CLI_SPEC.md's versioned JSON diagnostic format: `format_version: 1`,
/// `spans[{file,line,col,len,label}]`. Columns are 1-indexed Unicode scalar
/// positions and lengths count Unicode scalar values. `help` is a
/// single-element array holding `Diagnostic.help` when it carries a
/// determinate, safe suggestion (D-152), and an empty array otherwise --
/// `render_human` still has no equivalent `help:` line (see its own doc
/// comment and D-043).
pub fn render_json(diag: &Diagnostic, file_path: &str, source: &str) -> String {
    let severity_word = match diag.severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
    };
    let spans = if let Some(span) = diag.span {
        let start = byte_offset_to_line_col(source, span.start);
        let scalar_len = source[span.start as usize..span.end.max(span.start) as usize]
            .chars()
            .count() as u32;
        let file_path = display_path(file_path);
        serde_json::json!([{
            "file": file_path,
            "line": start.line,
            "col": start.column,
            "len": scalar_len,
            "label": diag.label,
        }])
    } else {
        serde_json::json!([])
    };
    let help = diag
        .help
        .as_ref()
        .map_or_else(Vec::new, |h| vec![h.as_str()]);
    let value = serde_json::json!({
        "format_version": 1,
        "code": diag.code,
        "severity": severity_word,
        "message": diag.message,
        "spans": spans,
        "help": help,
    });
    value.to_string()
}

/// Produces the terminal-safe, lexically normalized path used by diagnostics.
pub fn display_path(path: &str) -> String {
    escape_terminal_controls(&normalize_diagnostic_path(path), false)
}

fn normalize_diagnostic_path(path: &str) -> String {
    #[cfg(windows)]
    let path = path.replace('\\', "/");
    #[cfg(not(windows))]
    let path = path.to_string();
    let root = if path.starts_with("//") {
        "//"
    } else if path.starts_with('/') {
        "/"
    } else {
        ""
    };
    let joined = path
        .trim_start_matches('/')
        .split('/')
        .filter(|component| !component.is_empty() && *component != ".")
        .collect::<Vec<_>>()
        .join("/");

    if joined.is_empty() {
        if root.is_empty() {
            ".".to_string()
        } else {
            root.to_string()
        }
    } else {
        format!("{root}{joined}")
    }
}

fn escape_terminal_controls(text: &str, preserve_tabs: bool) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '\t' if preserve_tabs => escaped.push('\t'),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            '\0' => escaped.push_str("\\0"),
            character if character.is_control() || is_bidi_format_control(character) => {
                write!(escaped, "\\u{{{:x}}}", u32::from(character))
                    .expect("writing to a string must succeed");
            }
            character => escaped.push(character),
        }
    }
    escaped
}

fn is_bidi_format_control(character: char) -> bool {
    matches!(
        character,
        '\u{061c}'
            | '\u{200e}'
            | '\u{200f}'
            | '\u{202a}'..='\u{202e}'
            | '\u{2066}'..='\u{2069}'
    )
}

const ZERO_WIDTH_RANGES: &[(u32, u32)] = &[
    (0x0300, 0x036f),
    (0x1ab0, 0x1aff),
    (0x1dc0, 0x1dff),
    (0x20d0, 0x20ff),
    (0xfe00, 0xfe0f),
    (0xfe20, 0xfe2f),
    (0x1f3fb, 0x1f3ff),
    (0xe0100, 0xe01ef),
];

const WIDE_RANGES: &[(u32, u32)] = &[
    (0x1100, 0x115f),
    (0x2329, 0x232a),
    (0x2e80, 0xa4cf),
    (0xac00, 0xd7a3),
    (0xf900, 0xfaff),
    (0xfe10, 0xfe19),
    (0xfe30, 0xfe6f),
    (0xff00, 0xff60),
    (0xffe0, 0xffe6),
    (0x1f300, 0x1faff),
    (0x20000, 0x3fffd),
];

fn in_scalar_ranges(character: char, ranges: &[(u32, u32)]) -> bool {
    let scalar = u32::from(character);
    ranges
        .iter()
        .any(|&(start, end)| start <= scalar && scalar <= end)
}

fn scalar_display_width(character: char) -> usize {
    if in_scalar_ranges(character, ZERO_WIDTH_RANGES) {
        0
    } else if in_scalar_ranges(character, WIDE_RANGES) {
        2
    } else {
        1
    }
}

fn is_emoji_presentation_extension(character: char) -> bool {
    matches!(character, '\u{fe0f}' | '\u{1f3fb}'..='\u{1f3ff}')
}

fn display_width(text: &str) -> usize {
    let mut width = 0;
    let mut current_cluster_width = 0;
    let mut joins_next = false;
    for character in text.chars() {
        if character == '\u{200d}' {
            joins_next = true;
            continue;
        }
        if is_emoji_presentation_extension(character) {
            if current_cluster_width == 1 {
                width += 1;
                current_cluster_width = 2;
            }
            continue;
        }
        let scalar_width = scalar_display_width(character);
        if scalar_width == 0 {
            continue;
        }
        if joins_next {
            let joined_width = current_cluster_width.max(scalar_width);
            width += joined_width - current_cluster_width;
            current_cluster_width = joined_width;
            joins_next = false;
        } else {
            width += scalar_width;
            current_cluster_width = scalar_width;
        }
    }
    width
}

fn display_padding(text: &str) -> String {
    let mut padding = String::new();
    for (index, segment) in text.split('\t').enumerate() {
        if index > 0 {
            padding.push('\t');
        }
        padding.extend(std::iter::repeat_n(' ', display_width(segment)));
    }
    padding
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostic_carries_code_severity_and_span() {
        let d = Diagnostic::error("T0001", "argument missing annotation", Span::new(10, 14));
        assert_eq!(d.code, "T0001");
        assert_eq!(d.severity, Severity::Error);
        assert_eq!(d.span, Some(Span::new(10, 14)));
        assert_eq!(d.label.as_deref(), Some("argument missing annotation"));
    }

    #[test]
    fn diagnostic_without_span_is_allowed() {
        let d = Diagnostic::warning("W1001", "unreachable code");
        assert_eq!(d.severity, Severity::Warning);
        assert_eq!(d.span, None);
        assert_eq!(d.label, None);
    }

    #[test]
    fn diagnostic_help_defaults_to_none_and_with_help_sets_it() {
        let without_help = Diagnostic::error("T0021", "expects `int`, got `str`", Span::new(0, 1));
        assert_eq!(without_help.help, None);

        let with_help = Diagnostic::error("T0021", "expects `int`, got `str`", Span::new(0, 1))
            .with_help("pass an `int` value");
        assert_eq!(with_help.help.as_deref(), Some("pass an `int` value"));

        let warning_with_help =
            Diagnostic::warning("W1001", "unreachable code").with_help("remove the dead code");
        assert_eq!(
            warning_with_help.help.as_deref(),
            Some("remove the dead code")
        );
    }

    #[test]
    fn byte_offset_to_line_col_finds_first_line_first_column() {
        assert_eq!(
            byte_offset_to_line_col("print(42)\n", 0),
            LineCol { line: 1, column: 1 }
        );
    }

    #[test]
    fn byte_offset_to_line_col_finds_a_later_line() {
        let source = "def f():\n    print(42)\n";
        // offset 13 is the 'p' in "print", on line 2, column 5 (1-indexed, after 4 spaces)
        assert_eq!(
            byte_offset_to_line_col(source, 13),
            LineCol { line: 2, column: 5 }
        );
    }

    #[test]
    fn byte_offset_to_line_col_at_a_newline_byte_stays_on_the_line_it_ends() {
        let source = "ab\ncd";
        // offset 2 is the '\n' itself -- still counted as the end of line 1, column 3
        assert_eq!(
            byte_offset_to_line_col(source, 2),
            LineCol { line: 1, column: 3 }
        );
    }

    #[test]
    fn render_human_matches_cli_spec_format() {
        let source = "def fib(n):\n    print(fib(\"35\"))\n";
        // Byte offsets 26..30 slice out exactly `"35"` (with quotes) from
        // this source -- verified with a standalone script before trusting
        // this test, not counted by hand: the token starts at column 15 of
        // line 2 ("    print(fib(" is 14 chars, so the opening quote is the
        // 15th).
        assert_eq!(&source[26..30], "\"35\"");
        let diag = Diagnostic::error(
            "T0021",
            "argument 1 of `fib` expects `int`, got `str`".to_string(),
            Span::new(26, 30),
        );
        let rendered = render_human(&diag, "src/main.py", source);
        let expected = "\
error[T0021]: argument 1 of `fib` expects `int`, got `str`
 --> src/main.py:2:15
  |
2 |     print(fib(\"35\"))
  |               ^^^^ argument 1 of `fib` expects `int`, got `str`
";
        assert_eq!(rendered, expected);
    }

    #[test]
    fn render_human_with_a_multi_line_span_underlines_a_single_caret() {
        let source = "x\ny\n";
        // start = offset 0 = line 1, column 1; end = offset 2 = line 2,
        // column 1 -- lines differ, so only the start line's single caret
        // is meaningful for a one-line gutter display, rather than
        // attempting a (not-yet-supported) multi-line highlight.
        let diag = Diagnostic::error("T0001", "multi-line span".to_string(), Span::new(0, 2));
        let rendered = render_human(&diag, "src/main.py", source);
        let expected = "\
error[T0001]: multi-line span
 --> src/main.py:1:1
  |
1 | x
  | ^ multi-line span
";
        assert_eq!(rendered, expected);
    }

    #[test]
    fn render_human_with_no_span_omits_the_location_block() {
        let diag = Diagnostic::warning("W1001", "unreachable code");
        let rendered = render_human(&diag, "src/main.py", "x = 1\n");
        assert_eq!(rendered, "warning[W1001]: unreachable code\n");
    }

    #[test]
    fn render_human_with_an_unlabelled_span_omits_the_label() {
        let diag = Diagnostic {
            code: "W1001",
            severity: Severity::Warning,
            message: "unreachable code".to_string(),
            span: Some(Span::new(0, 1)),
            label: None,
            help: None,
        };
        let rendered = render_human(&diag, "src/main.py", "x = 1\n");
        let expected = "\
warning[W1001]: unreachable code
 --> src/main.py:1:1
  |
1 | x = 1
  | ^
";
        assert_eq!(rendered, expected);
    }

    #[test]
    fn render_json_matches_the_versioned_schema() {
        let source = "print(1)\n";
        let diag = Diagnostic::error("T0001", "missing annotation".to_string(), Span::new(0, 5));
        let rendered = render_json(&diag, "src/main.py", source);
        let parsed: serde_json::Value = serde_json::from_str(&rendered).unwrap();
        assert_eq!(parsed["format_version"], 1);
        assert_eq!(parsed["code"], "T0001");
        assert_eq!(parsed["severity"], "error");
        assert_eq!(parsed["message"], "missing annotation");
        assert_eq!(parsed["spans"][0]["file"], "src/main.py");
        assert_eq!(parsed["spans"][0]["line"], 1);
        assert_eq!(parsed["spans"][0]["col"], 1);
        assert_eq!(parsed["spans"][0]["len"], 5);
        assert_eq!(parsed["spans"][0]["label"], "missing annotation");
    }

    #[test]
    fn render_json_populates_help_when_the_diagnostic_carries_one() {
        let source = "for item in range(\"three\"):\n    print(item)\n";
        let diag = Diagnostic::error(
            "T0021",
            "range stop expects `int`, got `str`".to_string(),
            Span::new(0, 5),
        )
        .with_help("pass an `int` value");
        let rendered = render_json(&diag, "src/main.py", source);
        let parsed: serde_json::Value = serde_json::from_str(&rendered).unwrap();
        assert_eq!(parsed["help"], serde_json::json!(["pass an `int` value"]));
    }

    #[test]
    fn render_json_with_no_span_has_an_empty_spans_array() {
        let diag = Diagnostic::warning("W1001", "unreachable code");
        let rendered = render_json(&diag, "src/main.py", "x = 1\n");
        let parsed: serde_json::Value = serde_json::from_str(&rendered).unwrap();
        assert_eq!(parsed["spans"], serde_json::json!([]));
    }

    #[test]
    fn render_json_uses_unicode_scalar_columns_and_lengths() {
        let source = "éx\n";
        let first = Diagnostic::error("T0001", "first".to_string(), Span::new(0, 2));
        let first: serde_json::Value =
            serde_json::from_str(&render_json(&first, "unicode.py", source)).unwrap();
        assert_eq!(first["spans"][0]["col"], 1);
        assert_eq!(first["spans"][0]["len"], 1);

        let second = Diagnostic::error("T0001", "second".to_string(), Span::new(2, 3));
        let second: serde_json::Value =
            serde_json::from_str(&render_json(&second, "unicode.py", source)).unwrap();
        assert_eq!(second["spans"][0]["col"], 2);
        assert_eq!(second["spans"][0]["len"], 1);
    }

    #[test]
    fn diagnostic_paths_are_lexically_normalized() {
        #[cfg(windows)]
        assert_eq!(
            normalize_diagnostic_path(r".\src\.\package\\module.py"),
            "src/package/module.py"
        );
        #[cfg(not(windows))]
        assert_eq!(normalize_diagnostic_path(r"bad\name.py"), r"bad\name.py");
        assert_eq!(
            normalize_diagnostic_path("/tmp//./package/module.py"),
            "/tmp/package/module.py"
        );
        assert_eq!(
            normalize_diagnostic_path("//server//share/./module.py"),
            "//server/share/module.py"
        );
        assert_eq!(normalize_diagnostic_path("."), ".");
        assert_eq!(normalize_diagnostic_path("/"), "/");
    }

    #[test]
    fn terminal_controls_are_escaped_without_losing_source_tabs() {
        assert_eq!(
            escape_terminal_controls(
                "a\n\r\t\0\u{1b}\u{85}\u{061c}\u{200e}\u{200f}\u{202a}\u{202e}\u{2066}\u{2069}z",
                false,
            ),
            r"a\n\r\t\0\u{1b}\u{85}\u{61c}\u{200e}\u{200f}\u{202a}\u{202e}\u{2066}\u{2069}z"
        );
        assert_eq!(escape_terminal_controls("a\t\u{1b}z", true), "a\t\\u{1b}z");
        assert_eq!(escape_terminal_controls("a\u{200d}z", false), "a\u{200d}z");
    }

    #[test]
    fn diagnostic_padding_uses_whole_unicode_sequences_and_retains_tabs() {
        assert_eq!(display_padding("👩‍💻👍🏽"), "    ");
        assert_eq!(display_padding("☝🏽❤️‍🔥"), "    ");
        assert_eq!(display_padding("e\u{301}"), " ");
        assert_eq!(display_padding("\t👩‍💻"), "\t  ");
    }

    #[test]
    fn source_rendering_escapes_controls_and_aligns_the_caret_to_rendered_text() {
        let source = "\u{1b}\u{202e}👩‍💻👍🏽$\n";
        let start = source.find('$').unwrap();
        let diagnostic = Diagnostic::error(
            "L0001",
            "invalid syntax",
            Span::new(start as u32, (start + 1) as u32),
        );
        let rendered = render_human(&diagnostic, "bad.py", source);

        assert!(rendered.contains("1 | \\u{1b}\\u{202e}👩‍💻👍🏽$"));
        assert!(rendered.contains(&format!("  | {}^ invalid syntax", " ".repeat(18))));
        assert!(!rendered.contains('\u{1b}'));
        assert!(!rendered.contains('\u{202e}'));
    }
}
