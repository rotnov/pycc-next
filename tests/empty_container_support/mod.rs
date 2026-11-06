//! The `pycc build` / `pycc check` helpers the issue #1021 empty-container
//! inference tests share (`tests/issue_1021_empty_container_inference*.rs`).

// Each test file uses its own share of the helpers.
#![allow(dead_code)]

use pycc_scratch::ScratchDir;
use std::io::Write;
use std::process::Command;

pub fn pycc_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_pycc"))
}

pub fn write_source(label: &str, source: &str) -> (ScratchDir, std::path::PathBuf) {
    let dir =
        ScratchDir::new(&format!("issue_1021_{label}")).expect("failed to create scratch dir");
    let path = dir.join(format!("{label}.py"));
    let mut file = std::fs::File::create(&path).unwrap();
    file.write_all(source.as_bytes()).unwrap();
    (dir, path)
}

/// Builds and runs `source`, and additionally asserts that `pycc check`
/// accepts it. The two entry points (`check_all_keyed` and
/// `check_and_resolve_all_keyed`) each run the resolution pass themselves, so
/// every positive case pins that they agree: `pycc check` must never accept
/// what `pycc build` rejects, nor the reverse.
pub fn check_build_and_run(label: &str, source: &str) -> std::process::Output {
    let (dir, path) = write_source(label, source);
    let checked = Command::new(pycc_bin())
        .args(["check", path.to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(
        checked.status.code(),
        Some(0),
        "`pycc check` rejected {label}: {}",
        pycc_scratch::lf(&*String::from_utf8_lossy(&checked.stdout))
    );
    let out = dir.join(label);
    let status = Command::new(pycc_bin())
        .args(["build", path.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(status.success(), "`pycc build` failed for {label}");
    Command::new(&out).output().unwrap()
}

/// Runs `pycc check` on `source` and returns its rendered diagnostic output,
/// asserting the exit status really is the compile-error one. Also asserts
/// that `pycc build` agrees it is an error, which is the check-phase /
/// resolve-phase agreement property in its negative direction.
pub fn check_error(label: &str, source: &str) -> String {
    let (dir, path) = write_source(label, source);
    let output = Command::new(pycc_bin())
        .args(["check", path.to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(1),
        "{label} should be a compile error"
    );
    let out = dir.join(label);
    let built = Command::new(pycc_bin())
        .args(["build", path.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(
        built.status.code(),
        Some(1),
        "`pycc build` disagreed with `pycc check` on {label}"
    );
    String::from_utf8(pycc_scratch::lf(&output.stdout)).expect("diagnostics are UTF-8")
}

/// The JSON rendering of `pycc check`'s diagnostics for `source`. The human
/// renderer prints code, message and label but not `help`, so a test about
/// help text has to read this surface rather than [`check_error`]'s.
pub fn check_error_json(label: &str, source: &str) -> String {
    let (_dir, path) = write_source(label, source);
    let output = Command::new(pycc_bin())
        .args(["check", "--error-format", "json", path.to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(1),
        "{label} should be a compile error"
    );
    String::from_utf8(pycc_scratch::lf(&output.stdout)).expect("diagnostics are UTF-8")
}
