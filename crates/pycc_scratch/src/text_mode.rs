//! A process's captured output as Unix text, for tests that compare it.
//!
//! On Windows a native program writes CPython's text-mode streams: `\n` goes
//! out as `\r\n`, as CPython's own `print` writes it (docs/DEVIATIONS.md no
//! longer lists a difference). Tests compare line content, so every capture
//! they read goes through [`lf`], which turns each `\r\n` back into `\n` on
//! Windows and returns the bytes unchanged everywhere else, where the
//! programs write `\n` as CPython does. The tests that check the Windows
//! bytes themselves read the raw capture.

/// Captured output [`lf`] accepts: bytes or text.
pub trait LineText {
    /// What [`lf`] returns: the owned form of the same kind.
    type Owned;
    /// See [`lf`].
    fn lf(&self) -> Self::Owned;
}

/// `\r\n` as `\n` (Windows only).
fn crlf_to_lf(bytes: &[u8]) -> Vec<u8> {
    if !cfg!(windows) || !bytes.contains(&b'\r') {
        return bytes.to_vec();
    }
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\r' && bytes.get(i + 1) == Some(&b'\n') {
            i += 1;
        }
        out.push(bytes[i]);
        i += 1;
    }
    out
}

impl LineText for [u8] {
    type Owned = Vec<u8>;
    fn lf(&self) -> Vec<u8> {
        crlf_to_lf(self)
    }
}

impl LineText for Vec<u8> {
    type Owned = Vec<u8>;
    fn lf(&self) -> Vec<u8> {
        crlf_to_lf(self)
    }
}

impl LineText for str {
    type Owned = String;
    fn lf(&self) -> String {
        String::from_utf8(crlf_to_lf(self.as_bytes())).expect("removing `\\r` keeps UTF-8")
    }
}

impl LineText for String {
    type Owned = String;
    fn lf(&self) -> String {
        self.as_str().lf()
    }
}

/// `output` with Windows' `\r\n` line endings read as `\n`; unchanged on
/// other platforms.
pub fn lf<T: LineText + ?Sized>(output: &T) -> T::Owned {
    output.lf()
}

#[cfg(test)]
mod tests {
    use super::lf;

    #[test]
    fn crlf_becomes_lf_on_windows_only() {
        let expected: &[u8] = if cfg!(windows) {
            b"a\nb\r"
        } else {
            b"a\r\nb\r"
        };
        assert_eq!(lf(&b"a\r\nb\r".to_vec()), expected);
        assert_eq!(lf("x\r\n"), if cfg!(windows) { "x\n" } else { "x\r\n" });
        assert_eq!(lf(&String::from("y\n")), "y\n");
    }
}
