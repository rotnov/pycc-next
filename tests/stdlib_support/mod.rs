//! Shared oracle helpers for the stdlib end-to-end tests
//! (`tests/stdlib_*.rs`): build a fixture natively (strict and
//! `--non-strict`) and as an `--ext` module, and compare with CPython 3.13
//! (`PYCC_PYTHON`, default `python3`).

#![allow(dead_code)]

use pycc_scratch::ScratchDir;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

pub fn pycc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
}

pub fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

pub fn python() -> PathBuf {
    let default = if cfg!(windows) { "python.exe" } else { "python3" };
    PathBuf::from(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| default.into()))
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

pub fn rendered(output: &Output) -> String {
    format!("{}{}", text(&output.stdout), text(&output.stderr))
}

/// The observable result of a run: stdout, the last stderr line (the
/// exception's `Type: message`; the traceback above it differs on
/// purpose) and the exit status.
#[derive(Debug, PartialEq, Eq)]
pub struct Outcome {
    pub stdout: String,
    pub error: String,
    pub status: Option<i32>,
}

pub fn outcome(output: &Output) -> Outcome {
    Outcome {
        stdout: text(&output.stdout),
        error: text(&output.stderr)
            .lines()
            .last()
            .unwrap_or_default()
            .to_string(),
        status: output.status.code(),
    }
}

/// Builds `source` with `flags` and runs it, panicking when the build fails.
pub fn run_native(label: &str, source: &Path, flags: &[&str]) -> Outcome {
    let dir = ScratchDir::new(label).expect("scratch");
    let out = dir.join("prog");
    let build = pycc()
        .arg("build")
        .arg(source)
        .arg("-o")
        .arg(&out)
        .args(flags)
        .output()
        .expect("pycc spawns");
    assert!(
        build.status.success(),
        "{} {flags:?}: {}",
        source.display(),
        rendered(&build)
    );
    outcome(&Command::new(&out).output().expect("the program spawns"))
}

/// Asserts that fixture `name` behaves as under CPython when built natively
/// in strict mode (release) and with `--non-strict`.
pub fn assert_native_matches(name: &str) {
    let source = fixture(name);
    let expected = outcome(
        &Command::new(python())
            .arg(&source)
            .output()
            .expect("the oracle runs"),
    );
    for flags in [&["--release"][..], &["--release", "--non-strict"][..]] {
        let label = format!("stdlib_{}", name.trim_end_matches(".py"));
        assert_eq!(
            run_native(&label, &source, flags),
            expected,
            "{name} {flags:?}"
        );
    }
}

/// `pycc check` of `source`, asserting it fails; the rendered diagnostics.
pub fn check_fails(source: &str) -> String {
    let dir = ScratchDir::new("stdlib_check").expect("scratch");
    let path = dir.join("subject.py");
    std::fs::write(&path, source).expect("write the subject");
    let output = pycc()
        .arg("check")
        .arg(&path)
        .output()
        .expect("pycc spawns");
    assert!(!output.status.success(), "{source} was accepted");
    rendered(&output)
}

/// Builds fixture module `name` with `--ext`, then runs the `driver` script
/// (which imports the module by its stem) against the compiled module and
/// against the source, asserting the two outcomes agree.
pub fn assert_ext_matches(name: &str, driver: &str) {
    let source = fixture(name);
    let stem = name.trim_end_matches(".py");
    let dir = ScratchDir::new(&format!("stdlib_ext_{stem}")).expect("scratch");
    let compiled = dir.join("compiled");
    std::fs::create_dir_all(&compiled).expect("mkdir");
    let build = pycc()
        .arg("build")
        .arg(&source)
        .arg("-o")
        .arg(compiled.join(stem))
        .arg("--ext")
        .output()
        .expect("pycc spawns");
    assert!(build.status.success(), "{name} --ext: {}", rendered(&build));
    let run = |dir: &Path| {
        outcome(
            &Command::new(python())
                .args(["-B", "-c"])
                .arg(format!(
                    "import sys\nsys.path.insert(0, {:?})\n{driver}",
                    dir.to_str().expect("utf-8 path")
                ))
                .output()
                .expect("the oracle runs"),
        )
    };
    let expected = run(source.parent().expect("fixture dir"));
    assert_eq!(expected.status, Some(0), "{expected:?}");
    assert_eq!(run(&compiled), expected, "{name} --ext");
}
