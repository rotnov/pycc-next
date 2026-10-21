#![cfg_attr(not(windows), allow(dead_code))]
//! CPython's text-mode standard streams on Windows, for a native program.
//!
//! CPython 3.13 on Windows wraps fds 0-2 in text-mode `TextIOWrapper`s
//! (`Python/pylifecycle.c` `create_stdio`, `newline=None`): a write
//! translates `\n` to `\r\n` and then encodes the translated text, and a
//! read translates `\r\n` and `\r` to `\n`. The encoding
//! (`Python/initconfig.c` `config_init_stdio_encoding`):
//!
//! - a stream attached to a console is `_WindowsConsoleIO`, always UTF-8
//!   (written with `WriteConsoleW`), unless `PYTHONLEGACYWINDOWSSTDIO` is set;
//! - otherwise the encoding part of `PYTHONIOENCODING` (`encoding[:errors]`),
//!   else UTF-8 in UTF-8 mode (`PYTHONUTF8=1`), else with
//!   `PYTHONLEGACYWINDOWSSTDIO` the console's output code page, else the
//!   ANSI code page (`GetACP()`, `cp1252` on an English system);
//! - errors: those of `PYTHONIOENCODING` (`strict` when it names only an
//!   encoding), else `surrogateescape`; stderr always `backslashreplace`.
//!
//! The codecs and the stream rules here are pure, so every host tests them;
//! [`crate::io`] applies them on Windows only. A failed `strict` encode is
//! CPython's `UnicodeEncodeError`, with its message, raised by the write.

/// An output codec: the encodings CPython's standard streams can get.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Codec {
    Utf8,
    Ascii,
    Latin1,
    /// `cp1252` with CPython's own table (`Lib/encodings/cp1252.py`).
    Cp1252,
    /// Any other Windows code page, through `WideCharToMultiByte`.
    CodePage(u32),
}

/// An error handler (`codecs.register_error` names CPython ships).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Errors {
    Strict,
    Ignore,
    Replace,
    BackslashReplace,
    XmlCharRefReplace,
    /// `surrogateescape`/`surrogatepass`: a pycc `str` holds no lone
    /// surrogate, so on its text they fail exactly like `strict`.
    Surrogate,
}

/// How one standard stream turns text into bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct StreamCodec {
    pub(crate) codec: Codec,
    pub(crate) errors: Errors,
    /// Write `\n` as `\r\n`.
    pub(crate) crlf: bool,
}

/// The facts of a `UnicodeEncodeError`: `str(e)` is [`EncodeError::message`].
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct EncodeError {
    codec_name: &'static str,
    /// The unencodable run `[start, end)` in characters of the written text,
    /// and its first character.
    start: usize,
    end: usize,
    first: char,
    reason: &'static str,
}

impl EncodeError {
    /// `str(UnicodeEncodeError)` (`Objects/exceptions.c`).
    pub(crate) fn message(&self) -> String {
        if self.end - self.start == 1 {
            let c = self.first as u32;
            let escaped = if c < 0x100 {
                format!("\\x{c:02x}")
            } else if c < 0x10000 {
                format!("\\u{c:04x}")
            } else {
                format!("\\U{c:08x}")
            };
            format!(
                "'{}' codec can't encode character '{escaped}' in position {}: {}",
                self.codec_name, self.start, self.reason
            )
        } else {
            format!(
                "'{}' codec can't encode characters in position {}-{}: {}",
                self.codec_name,
                self.start,
                self.end - 1,
                self.reason
            )
        }
    }
}

/// CPython's `cp1252` decoding table for 0x80..=0x9F; `None` is undefined
/// (every other byte is its own code point).
const CP1252_HIGH: [Option<u16>; 32] = [
    Some(0x20AC),
    None,
    Some(0x201A),
    Some(0x0192),
    Some(0x201E),
    Some(0x2026),
    Some(0x2020),
    Some(0x2021),
    Some(0x02C6),
    Some(0x2030),
    Some(0x0160),
    Some(0x2039),
    Some(0x0152),
    None,
    Some(0x017D),
    None,
    None,
    Some(0x2018),
    Some(0x2019),
    Some(0x201C),
    Some(0x201D),
    Some(0x2022),
    Some(0x2013),
    Some(0x2014),
    Some(0x02DC),
    Some(0x2122),
    Some(0x0161),
    Some(0x203A),
    Some(0x0153),
    None,
    Some(0x017E),
    Some(0x0178),
];

fn cp1252_encode(c: char) -> Option<u8> {
    let u = c as u32;
    if u < 0x80 || (0xA0..0x100).contains(&u) {
        return Some(u as u8);
    }
    CP1252_HIGH
        .iter()
        .position(|&m| m == Some(u as u16) && u < 0x10000)
        .map(|i| 0x80 + i as u8)
}

fn cp1252_decode(b: u8) -> char {
    match b {
        0x80..=0x9F => CP1252_HIGH[(b - 0x80) as usize]
            .and_then(|u| char::from_u32(u32::from(u)))
            .unwrap_or('\u{FFFD}'),
        _ => b as char,
    }
}

impl Codec {
    /// The codec's name in a `UnicodeEncodeError` and its reason text.
    fn error_names(self) -> (&'static str, &'static str) {
        match self {
            Codec::Utf8 => ("utf-8", "surrogates not allowed"),
            Codec::Ascii => ("ascii", "ordinal not in range(128)"),
            Codec::Latin1 => ("latin-1", "ordinal not in range(256)"),
            Codec::Cp1252 => ("charmap", "character maps to <undefined>"),
            Codec::CodePage(cp) if code_page_is_single_byte(cp) => {
                ("charmap", "character maps to <undefined>")
            }
            Codec::CodePage(_) => ("mbcs", "illegal multibyte sequence"),
        }
    }

    /// One character's bytes, appended to `out`; false when unencodable.
    fn encode_char(self, c: char, out: &mut Vec<u8>) -> bool {
        match self {
            Codec::Utf8 => {
                let mut buf = [0u8; 4];
                out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
                true
            }
            Codec::Ascii if c.is_ascii() => {
                out.push(c as u8);
                true
            }
            Codec::Latin1 if (c as u32) < 0x100 => {
                out.push(c as u32 as u8);
                true
            }
            Codec::Cp1252 => match cp1252_encode(c) {
                Some(b) => {
                    out.push(b);
                    true
                }
                None => false,
            },
            Codec::CodePage(cp) => code_page_encode_char(cp, c, out),
            Codec::Ascii | Codec::Latin1 => false,
        }
    }

    /// Bytes read from a redirected stdin as text (undecodable bytes become
    /// U+FFFD, where CPython's `surrogateescape` keeps them as lone
    /// surrogates a pycc `str` cannot hold).
    pub(crate) fn decode(self, bytes: &[u8]) -> String {
        match self {
            Codec::Utf8 => String::from_utf8_lossy(bytes).into_owned(),
            Codec::Ascii => bytes
                .iter()
                .map(|&b| if b < 0x80 { b as char } else { '\u{FFFD}' })
                .collect(),
            Codec::Latin1 => bytes.iter().map(|&b| b as char).collect(),
            Codec::Cp1252 => bytes.iter().map(|&b| cp1252_decode(b)).collect(),
            Codec::CodePage(cp) => code_page_decode(cp, bytes),
        }
    }
}

impl StreamCodec {
    /// The bytes one `write(text)` puts on the stream: `\n` translated
    /// first, then encoded, as `TextIOWrapper.write` does -- so an error's
    /// position counts the translated text.
    pub(crate) fn encode(&self, text: &str) -> Result<Vec<u8>, EncodeError> {
        let mut out = Vec::with_capacity(text.len() + 8);
        let translated;
        let text = if self.crlf && text.contains('\n') {
            translated = text.replace('\n', "\r\n");
            translated.as_str()
        } else {
            text
        };
        if self.codec == Codec::Utf8 {
            out.extend_from_slice(text.as_bytes());
            return Ok(out);
        }
        let chars: Vec<char> = text.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            if self.codec.encode_char(chars[i], &mut out) {
                i += 1;
                continue;
            }
            // The unencodable run, as the codecs hand it to the handler.
            let mut end = i + 1;
            let mut scratch = Vec::new();
            while end < chars.len() && !self.codec.encode_char(chars[end], &mut scratch) {
                end += 1;
            }
            match self.errors {
                Errors::Strict | Errors::Surrogate => {
                    let (codec_name, reason) = self.codec.error_names();
                    return Err(EncodeError {
                        codec_name,
                        start: i,
                        end,
                        first: chars[i],
                        reason,
                    });
                }
                Errors::Ignore => {}
                Errors::Replace => out.extend(std::iter::repeat_n(b'?', end - i)),
                Errors::BackslashReplace => {
                    for &c in &chars[i..end] {
                        let u = c as u32;
                        let esc = if u < 0x100 {
                            format!("\\x{u:02x}")
                        } else if u < 0x10000 {
                            format!("\\u{u:04x}")
                        } else {
                            format!("\\U{u:08x}")
                        };
                        out.extend_from_slice(esc.as_bytes());
                    }
                }
                Errors::XmlCharRefReplace => {
                    for &c in &chars[i..end] {
                        out.extend_from_slice(format!("&#{};", c as u32).as_bytes());
                    }
                }
            }
            i = end;
        }
        Ok(out)
    }
}

/// `encodings.normalize_encoding` plus the aliases of the codecs above
/// (`Lib/encodings/aliases.py`); `None` for an encoding pycc does not
/// write (the stream then stays UTF-8, docs/DEVIATIONS.md).
pub(crate) fn codec_named(name: &str) -> Option<Codec> {
    let norm: String = name
        .trim()
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c == '-' || c == ' ' { '_' } else { c })
        .collect();
    Some(match norm.as_str() {
        "utf_8" | "utf8" | "u8" | "utf" | "utf8_ucs2" | "utf8_ucs4" | "cp65001" => Codec::Utf8,
        "ascii" | "us_ascii" | "646" | "us" | "ansi_x3.4_1968" | "ansi_x3_4_1968"
        | "ansi_x3.4_1986" | "cp367" | "csascii" | "ibm367" | "iso646_us" | "iso_646.irv_1991"
        | "iso_ir_6" => Codec::Ascii,
        "latin_1" | "latin1" | "latin" | "l1" | "iso8859_1" | "iso_8859_1" | "8859" | "cp819"
        | "ibm819" | "iso_ir_100" | "csisolatin1" | "iso_8859_1_1987" => Codec::Latin1,
        "cp1252" | "windows_1252" | "1252" => Codec::Cp1252,
        other => {
            let digits = other
                .strip_prefix("cp")
                .or_else(|| other.strip_prefix("windows_"))?;
            let cp: u32 = digits.parse().ok()?;
            if !code_page_available(cp) {
                return None;
            }
            Codec::CodePage(cp)
        }
    })
}

/// CPython's encoding for the ANSI code page `cp` (`_Py_GetLocaleEncoding`
/// spells it `cp<N>`).
pub(crate) fn code_page_codec(cp: u32) -> Codec {
    codec_named(&format!("cp{cp}")).unwrap_or(Codec::Utf8)
}

/// What the stream needs to know about the process environment.
#[derive(Default, Debug)]
pub(crate) struct StdioEnv {
    pub(crate) pythonioencoding: Option<String>,
    pub(crate) pythonutf8: Option<String>,
    pub(crate) legacy_windows_stdio: bool,
    /// `GetACP()`.
    pub(crate) ansi_code_page: u32,
    /// `GetConsoleOutputCP()`: 0 when the process has no console.
    pub(crate) console_output_code_page: u32,
}

/// Which standard stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Stream {
    Stdin,
    Stdout,
    Stderr,
}

/// CPython 3.13's codec for `stream` (`config_init_stdio_encoding`,
/// `create_stdio`); `is_console` is whether the stream's handle is a
/// console.
pub(crate) fn stream_codec(env: &StdioEnv, stream: Stream, is_console: bool) -> StreamCodec {
    fn nonempty(v: &Option<String>) -> Option<&str> {
        v.as_deref().filter(|s| !s.is_empty())
    }
    let utf8_mode = nonempty(&env.pythonutf8) == Some("1");
    let (mut encoding, mut errors) = (None, None);
    if let Some(opt) = nonempty(&env.pythonioencoding) {
        let (enc, errs) = match opt.split_once(':') {
            Some((enc, errs)) => (enc, Some(errs).filter(|e| !e.is_empty())),
            None => (opt, None),
        };
        if !enc.is_empty() {
            encoding = Some(codec_named(enc).unwrap_or(Codec::Utf8));
            errors = Some(errs.unwrap_or("strict"));
        } else {
            errors = errs;
        }
    }
    let codec = encoding.unwrap_or_else(|| {
        if env.legacy_windows_stdio && env.console_output_code_page != 0 {
            code_page_codec(env.console_output_code_page)
        } else if utf8_mode {
            Codec::Utf8
        } else {
            code_page_codec(env.ansi_code_page)
        }
    });
    // `_WindowsConsoleIO` is always UTF-8, whatever the configured encoding.
    let codec = if is_console && !env.legacy_windows_stdio {
        Codec::Utf8
    } else {
        codec
    };
    let errors = if stream == Stream::Stderr {
        Errors::BackslashReplace
    } else {
        match errors.unwrap_or("surrogateescape") {
            "ignore" => Errors::Ignore,
            "replace" => Errors::Replace,
            "backslashreplace" => Errors::BackslashReplace,
            "xmlcharrefreplace" => Errors::XmlCharRefReplace,
            "surrogateescape" | "surrogatepass" => Errors::Surrogate,
            _ => Errors::Strict,
        }
    };
    StreamCodec {
        codec,
        errors,
        crlf: true,
    }
}

#[cfg(windows)]
mod win {
    #[repr(C)]
    pub(super) struct CpInfo {
        pub(super) max_char_size: u32,
        pub(super) default_char: [u8; 2],
        pub(super) lead_byte: [u8; 12],
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        pub(super) fn GetStdHandle(which: u32) -> *mut core::ffi::c_void;
        pub(super) fn GetConsoleMode(handle: *mut core::ffi::c_void, mode: *mut u32) -> i32;
        pub(super) fn GetACP() -> u32;
        pub(super) fn GetConsoleOutputCP() -> u32;
        pub(super) fn GetCPInfo(cp: u32, info: *mut CpInfo) -> i32;
        pub(super) fn WideCharToMultiByte(
            cp: u32,
            flags: u32,
            wide: *const u16,
            wide_len: i32,
            out: *mut u8,
            out_len: i32,
            default_char: *const u8,
            used_default: *mut i32,
        ) -> i32;
        pub(super) fn MultiByteToWideChar(
            cp: u32,
            flags: u32,
            bytes: *const u8,
            len: i32,
            out: *mut u16,
            out_len: i32,
        ) -> i32;
    }
}

#[cfg(windows)]
fn code_page_info(cp: u32) -> Option<win::CpInfo> {
    let mut info = win::CpInfo {
        max_char_size: 0,
        default_char: [0; 2],
        lead_byte: [0; 12],
    };
    (unsafe { win::GetCPInfo(cp, &mut info) } != 0).then_some(info)
}

#[cfg(windows)]
fn code_page_available(cp: u32) -> bool {
    code_page_info(cp).is_some()
}

#[cfg(not(windows))]
fn code_page_available(_cp: u32) -> bool {
    false
}

#[cfg(windows)]
fn code_page_is_single_byte(cp: u32) -> bool {
    code_page_info(cp).is_some_and(|i| i.max_char_size == 1)
}

#[cfg(not(windows))]
fn code_page_is_single_byte(_cp: u32) -> bool {
    true
}

#[cfg(windows)]
fn code_page_encode_char(cp: u32, c: char, out: &mut Vec<u8>) -> bool {
    const WC_NO_BEST_FIT_CHARS: u32 = 0x400;
    let mut wide = [0u16; 2];
    let wide = c.encode_utf16(&mut wide);
    let mut buf = [0u8; 8];
    let mut used_default = 0i32;
    let n = unsafe {
        win::WideCharToMultiByte(
            cp,
            WC_NO_BEST_FIT_CHARS,
            wide.as_ptr(),
            wide.len() as i32,
            buf.as_mut_ptr(),
            buf.len() as i32,
            std::ptr::null(),
            &mut used_default,
        )
    };
    if n <= 0 || used_default != 0 {
        return false;
    }
    out.extend_from_slice(&buf[..n as usize]);
    true
}

#[cfg(not(windows))]
fn code_page_encode_char(_cp: u32, _c: char, _out: &mut Vec<u8>) -> bool {
    false
}

#[cfg(windows)]
fn code_page_decode(cp: u32, bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return String::new();
    }
    let len = unsafe {
        win::MultiByteToWideChar(
            cp,
            0,
            bytes.as_ptr(),
            bytes.len() as i32,
            std::ptr::null_mut(),
            0,
        )
    };
    if len <= 0 {
        return String::from_utf8_lossy(bytes).into_owned();
    }
    let mut wide = vec![0u16; len as usize];
    unsafe {
        win::MultiByteToWideChar(
            cp,
            0,
            bytes.as_ptr(),
            bytes.len() as i32,
            wide.as_mut_ptr(),
            len,
        )
    };
    String::from_utf16_lossy(&wide)
}

#[cfg(not(windows))]
fn code_page_decode(_cp: u32, bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// The process's facts for [`stream_codec`], read once.
#[cfg(windows)]
pub(crate) fn process_env() -> StdioEnv {
    let var = |name: &str| std::env::var(name).ok();
    StdioEnv {
        pythonioencoding: var("PYTHONIOENCODING"),
        pythonutf8: var("PYTHONUTF8"),
        legacy_windows_stdio: var("PYTHONLEGACYWINDOWSSTDIO").is_some_and(|v| !v.is_empty()),
        ansi_code_page: unsafe { win::GetACP() },
        console_output_code_page: unsafe { win::GetConsoleOutputCP() },
    }
}

/// Whether `stream`'s handle is a console (what makes CPython pick
/// `_WindowsConsoleIO`).
#[cfg(windows)]
pub(crate) fn is_console(stream: Stream) -> bool {
    let which = match stream {
        Stream::Stdin => -10i32,
        Stream::Stdout => -11,
        Stream::Stderr => -12,
    } as u32;
    let handle = unsafe { win::GetStdHandle(which) };
    let mut mode = 0u32;
    !handle.is_null()
        && handle as isize != -1
        && unsafe { win::GetConsoleMode(handle, &mut mode) } != 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stream(codec: Codec, errors: Errors) -> StreamCodec {
        StreamCodec {
            codec,
            errors,
            crlf: true,
        }
    }

    #[test]
    fn newlines_are_translated_before_encoding() {
        let s = stream(Codec::Utf8, Errors::Strict);
        assert_eq!(s.encode("a\nb\n").unwrap(), b"a\r\nb\r\n");
        assert_eq!(s.encode("\u{e9}\n").unwrap(), "\u{e9}\r\n".as_bytes());
        let lf = StreamCodec { crlf: false, ..s };
        assert_eq!(lf.encode("a\n").unwrap(), b"a\n");
    }

    #[test]
    fn cp1252_matches_cpythons_table() {
        let s = stream(Codec::Cp1252, Errors::Strict);
        assert_eq!(s.encode("\u{20ac}\u{e9}\u{178}").unwrap(), b"\x80\xe9\x9f");
        // The position counts the translated text (`a\r\n` is 3 characters).
        let err = s.encode("a\n\u{2192}").unwrap_err();
        assert_eq!(
            err.message(),
            "'charmap' codec can't encode character '\\u2192' in position 3: \
             character maps to <undefined>"
        );
        // 0x81 is undefined in CPython's cp1252.
        let err = s.encode("x\u{81}").unwrap_err();
        assert_eq!(
            err.message(),
            "'charmap' codec can't encode character '\\x81' in position 1: \
             character maps to <undefined>"
        );
        let err = s.encode("\u{2192}\u{2190}x").unwrap_err();
        assert_eq!(
            err.message(),
            "'charmap' codec can't encode characters in position 0-1: \
             character maps to <undefined>"
        );
        assert_eq!(
            Codec::Cp1252.decode(b"\x80\xe9\x81"),
            "\u{20ac}\u{e9}\u{fffd}"
        );
    }

    #[test]
    fn ascii_and_latin1_report_their_ranges() {
        let err = stream(Codec::Ascii, Errors::Strict)
            .encode("\u{1F600}")
            .unwrap_err();
        assert_eq!(
            err.message(),
            "'ascii' codec can't encode character '\\U0001f600' in position 0: \
             ordinal not in range(128)"
        );
        let err = stream(Codec::Latin1, Errors::Surrogate)
            .encode("\u{e9}\u{2192}")
            .unwrap_err();
        assert_eq!(
            err.message(),
            "'latin-1' codec can't encode character '\\u2192' in position 1: \
             ordinal not in range(256)"
        );
        assert_eq!(Codec::Latin1.decode(b"\xe9"), "\u{e9}");
        assert_eq!(Codec::Ascii.decode(b"a\xe9"), "a\u{fffd}");
    }

    #[test]
    fn error_handlers_replace_the_unencodable_run() {
        let text = "a\u{2192}\u{1F600}\u{e9}";
        let enc = |errors| stream(Codec::Ascii, errors).encode(text).unwrap();
        assert_eq!(enc(Errors::Ignore), b"a");
        assert_eq!(enc(Errors::Replace), b"a???");
        assert_eq!(enc(Errors::BackslashReplace), b"a\\u2192\\U0001f600\\xe9");
        assert_eq!(enc(Errors::XmlCharRefReplace), b"a&#8594;&#128512;&#233;");
    }

    #[test]
    fn stream_rules_follow_cpython_3_13() {
        let env = StdioEnv {
            ansi_code_page: 1252,
            ..StdioEnv::default()
        };
        let out = stream_codec(&env, Stream::Stdout, false);
        assert_eq!((out.codec, out.errors), (Codec::Cp1252, Errors::Surrogate));
        let err = stream_codec(&env, Stream::Stderr, false);
        assert_eq!(
            (err.codec, err.errors),
            (Codec::Cp1252, Errors::BackslashReplace)
        );
        // A console is UTF-8.
        assert_eq!(stream_codec(&env, Stream::Stdout, true).codec, Codec::Utf8);
        // UTF-8 mode.
        let utf8 = StdioEnv {
            pythonutf8: Some("1".into()),
            ..StdioEnv::default()
        };
        assert_eq!(
            stream_codec(&utf8, Stream::Stdout, false).codec,
            Codec::Utf8
        );
        // PYTHONIOENCODING wins over UTF-8 mode; an encoding alone is strict.
        let io = StdioEnv {
            pythonioencoding: Some("latin-1".into()),
            pythonutf8: Some("1".into()),
            ..StdioEnv::default()
        };
        let out = stream_codec(&io, Stream::Stdout, false);
        assert_eq!((out.codec, out.errors), (Codec::Latin1, Errors::Strict));
        let io = StdioEnv {
            pythonioencoding: Some(":replace".into()),
            ansi_code_page: 1252,
            ..StdioEnv::default()
        };
        let out = stream_codec(&io, Stream::Stdout, false);
        assert_eq!((out.codec, out.errors), (Codec::Cp1252, Errors::Replace));
        let io = StdioEnv {
            pythonioencoding: Some("utf8:backslashreplace".into()),
            ..StdioEnv::default()
        };
        let out = stream_codec(&io, Stream::Stdout, false);
        assert_eq!(
            (out.codec, out.errors),
            (Codec::Utf8, Errors::BackslashReplace)
        );
        // The ANSI code page 65001 is UTF-8.
        let acp = StdioEnv {
            ansi_code_page: 65001,
            ..StdioEnv::default()
        };
        assert_eq!(stream_codec(&acp, Stream::Stdout, false).codec, Codec::Utf8);
        // Legacy stdio uses the console's output code page.
        let legacy = StdioEnv {
            legacy_windows_stdio: true,
            console_output_code_page: 1252,
            ansi_code_page: 65001,
            ..StdioEnv::default()
        };
        assert_eq!(
            stream_codec(&legacy, Stream::Stdout, true).codec,
            Codec::Cp1252
        );
    }

    #[test]
    fn codec_names_are_normalized() {
        assert_eq!(codec_named("UTF-8"), Some(Codec::Utf8));
        assert_eq!(codec_named("Windows-1252"), Some(Codec::Cp1252));
        assert_eq!(codec_named("ISO-8859-1"), Some(Codec::Latin1));
        assert_eq!(codec_named("US-ASCII"), Some(Codec::Ascii));
        assert_eq!(codec_named("no-such-codec"), None);
    }
}
