//! `str(KeyError(x))` is `repr(x)`.
//!
//! `KeyError.__str__` renders its single argument's `repr`, so `print(e)`,
//! `f"{e}"` and the uncaught-exception line show `'k'` where every other
//! exception shows `k`; a missing dict key raises `KeyError(key)`.
//!
//! `tests/fixtures/key_error_str.py` is compared with the live CPython
//! oracle (`PYCC_PYTHON`, default `python3`): stdout byte-for-byte, then the
//! uncaught exception's last stderr line and the exit status.

use pycc_scratch::ScratchDir;
use std::path::Path;
use std::process::{Command, Output};

const FIXTURE: &str = "tests/fixtures/key_error_str.py";

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

fn last_stderr_line(output: &Output) -> String {
    text(&output.stderr)
        .lines()
        .last()
        .unwrap_or_default()
        .to_string()
}

#[test]
fn key_error_renders_its_argument_as_a_repr_like_cpython() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join(FIXTURE);
    let dir = ScratchDir::new("key_error_str").expect("scratch");
    let binary = dir.join("app");
    let build = Command::new(env!("CARGO_BIN_EXE_pycc"))
        .arg("build")
        .arg(&fixture)
        .arg("-o")
        .arg(&binary)
        .arg("--release")
        .output()
        .expect("pycc should spawn");
    assert!(
        build.status.success(),
        "pycc build failed:\n{}{}",
        text(&build.stdout),
        text(&build.stderr)
    );
    let run = Command::new(&binary).output().expect("the binary runs");
    let oracle = std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into());
    let cpython = Command::new(oracle)
        .arg(&fixture)
        .output()
        .expect("python3 should spawn");
    assert_eq!(text(&run.stdout), text(&cpython.stdout));
    assert_eq!(last_stderr_line(&run), last_stderr_line(&cpython));
    assert_eq!(last_stderr_line(&run), "KeyError: 'boom'");
    assert_eq!(run.status.code(), Some(1));
    assert_eq!(cpython.status.code(), Some(1));
}
