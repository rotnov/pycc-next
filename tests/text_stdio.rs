//! A native program's standard streams are CPython's text-mode streams
//! (v0.8 accept 5): stdout and stderr byte for byte, read raw -- on Windows
//! `\r\n` line endings and the encoding CPython 3.13 picks (UTF-8 in UTF-8
//! mode, else `PYTHONIOENCODING`, else the ANSI code page), a strict
//! encoding failure raising CPython's `UnicodeEncodeError`; everywhere else
//! `\n` and UTF-8, as CPython writes there. The oracle is CPython
//! (`PYCC_PYTHON`, default `python3`) running the same source with the same
//! environment and stdin.

#[path = "stdlib_support/mod.rs"]
mod support;

use pycc_scratch::ScratchDir;
use std::io::Write;
use std::process::{Command, Output, Stdio};
use support::{pycc, python, rendered};

/// Runs `cmd` with `stdin` and the environment edits: `Some(v)` sets a
/// variable, `None` removes it.
fn run(mut cmd: Command, stdin: &[u8], env: &[(&str, Option<&str>)]) -> Output {
    for (key, value) in env {
        match value {
            Some(value) => cmd.env(key, value),
            None => cmd.env_remove(key),
        };
    }
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the program spawns");
    child
        .stdin
        .take()
        .expect("piped stdin")
        .write_all(stdin)
        .expect("stdin is written");
    child.wait_with_output().expect("the program runs")
}

/// The raw stdout, the raw last stderr line and the exit status.
fn observed(output: &Output) -> (Vec<u8>, Vec<u8>, Option<i32>) {
    let stderr = &output.stderr;
    let body = stderr.strip_suffix(b"\n").unwrap_or(stderr);
    let last = match body.iter().rposition(|&b| b == b'\n') {
        Some(at) => body[at + 1..].to_vec(),
        None => body.to_vec(),
    };
    (output.stdout.clone(), last, output.status.code())
}

/// Builds `source` natively in strict and non-strict mode and checks each
/// run against CPython's, with `stdin` and `env` for both.
fn assert_streams_match(name: &str, source: &str, stdin: &[u8], env: &[(&str, Option<&str>)]) {
    let dir = ScratchDir::new(&format!("text_stdio_{name}")).expect("scratch");
    let path = dir.join(format!("{name}.py"));
    std::fs::write(&path, source).expect("the source is written");
    let mut oracle = Command::new(python());
    oracle.arg(&path);
    let expected = observed(&run(oracle, stdin, env));
    for (mode, flags) in [("strict", &[][..]), ("non-strict", &["--non-strict"][..])] {
        let binary = dir.join(format!("{name}-{mode}"));
        let build = pycc()
            .arg("build")
            .arg(&path)
            .arg("-o")
            .arg(&binary)
            .args(flags)
            .output()
            .expect("pycc spawns");
        assert!(
            build.status.success(),
            "{name} ({mode}): {}",
            rendered(&build)
        );
        let got = observed(&run(Command::new(&binary), stdin, env));
        assert_eq!(
            got,
            expected,
            "{name} ({mode}): stdout {:?} / {:?}, stderr {:?} / {:?}",
            String::from_utf8_lossy(&got.0),
            String::from_utf8_lossy(&expected.0),
            String::from_utf8_lossy(&got.1),
            String::from_utf8_lossy(&expected.1),
        );
    }
}

const LINES: &str = "\
def main() -> None:
    print('one')
    print('two', 3, sep='-', end='\\n\\n')
    print('a\\nb')
    print('caf\\u00e9 \\u20ac')
    name = input()
    print('read', len(name), name)


main()
";

const RAISES: &str = "\
def main() -> None:
    print('before')
    raise ValueError('bad \\u2192 value')


main()
";

#[test]
#[ignore = "requires CPython 3.13 on PATH"]
fn lines_match_cpythons_bytes_in_utf8_mode() {
    let utf8 = [("PYTHONUTF8", Some("1")), ("PYTHONIOENCODING", None)];
    assert_streams_match("lines", LINES, "caf\u{e9}\r\n".as_bytes(), &utf8);
    assert_streams_match("raises", RAISES, b"", &utf8);
}

/// The encoding CPython picks without UTF-8 mode: the ANSI code page on
/// Windows, the locale's elsewhere (pycc's programs write UTF-8 there, so
/// only Windows runs these).
#[cfg(windows)]
mod windows {
    use super::*;

    const ANSI: [(&str, Option<&str>); 2] = [("PYTHONUTF8", None), ("PYTHONIOENCODING", None)];

    #[test]
    #[ignore = "requires CPython 3.13 on PATH"]
    fn the_ansi_code_page_encodes_stdout_and_decodes_stdin() {
        assert_streams_match("ansi_lines", LINES, b"caf\xe9\r\n", &ANSI);
        // stderr is `backslashreplace`.
        assert_streams_match("ansi_raises", RAISES, b"", &ANSI);
    }

    const UNENCODABLE: &str = "\
def main() -> None:
    try:
        print('a', '\\u2192', 'b')
    except ValueError:
        print('caught')
    print('arrow \\u2192')
    print('not reached')


main()
";

    #[test]
    #[ignore = "requires CPython 3.13 on PATH"]
    fn an_unencodable_character_raises_unicode_encode_error() {
        assert_streams_match("unencodable", UNENCODABLE, b"", &ANSI);
    }

    #[test]
    #[ignore = "requires CPython 3.13 on PATH"]
    fn pythonioencoding_sets_the_encoding_and_errors() {
        for (name, value) in [
            ("io_latin1", "latin-1"),
            ("io_ascii_bs", "ascii:backslashreplace"),
            ("io_replace", ":replace"),
            ("io_xml", "cp1252:xmlcharrefreplace"),
            ("io_utf8", "utf-8"),
        ] {
            let env = [("PYTHONUTF8", None), ("PYTHONIOENCODING", Some(value))];
            assert_streams_match(name, UNENCODABLE, b"", &env);
            assert_streams_match(&format!("{name}_lines"), LINES, b"cafe\r\n", &env);
        }
    }
}
