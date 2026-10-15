//! A call of a module-level function before its `def` statement ran (issue
//! #22's call-before-def path) raises CPython's catchable
//! `NameError: name '<f>' is not defined` instead of aborting the process:
//! natively (strict and `--non-strict`) and in an `--ext` module, from
//! module-level code, from functions called early, through methods, nested
//! functions and function values, and uncaught (traceback, exit status 1).
//! The cross-module case is in `tests/multi_module.rs` (`cycle_call`).
//!
//! Every expected output is CPython's, produced in the same test.

use pycc_scratch::ScratchDir;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn pycc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
}

fn python() -> Command {
    let mut command =
        Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()));
    command.env("PYTHONDONTWRITEBYTECODE", "1");
    command
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(format!("{name}.py"))
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

fn rendered(output: &Output) -> String {
    format!("{}{}", text(&output.stdout), text(&output.stderr))
}

/// The last line of a traceback: the exception CPython reports.
fn exception_line(stderr: &[u8]) -> String {
    text(stderr).lines().last().unwrap_or_default().to_string()
}

/// Compares stdout, the exit status and (on failure) the reported
/// exception of `compiled` with `oracle`.
fn assert_same(label: &str, compiled: &Output, oracle: &Output) {
    assert_eq!(text(&compiled.stdout), text(&oracle.stdout), "{label}");
    assert_eq!(
        compiled.status.code(),
        oracle.status.code(),
        "{label}: {}",
        rendered(compiled)
    );
    if !oracle.status.success() {
        assert_eq!(
            exception_line(&compiled.stderr),
            exception_line(&oracle.stderr),
            "{label}"
        );
    }
}

fn native_matches_cpython(name: &str, extra: &[&str]) {
    let dir = ScratchDir::new(&format!("call_before_def_{name}")).expect("scratch");
    let binary = dir.join("prog");
    let build = pycc()
        .arg("build")
        .arg(fixture(name))
        .arg("-o")
        .arg(&binary)
        .arg("--release")
        .args(extra)
        .output()
        .expect("pycc should spawn");
    assert!(
        build.status.success(),
        "{name} {extra:?}: {}",
        rendered(&build)
    );
    let compiled = Command::new(&binary).output().expect("the program runs");
    let oracle = python()
        .arg(fixture(name))
        .output()
        .expect("python3 should spawn");
    assert_same(&format!("{name} {extra:?}"), &compiled, &oracle);
}

fn ext_matches_cpython(name: &str, extra: &[&str]) {
    let dir = ScratchDir::new(&format!("call_before_def_ext_{name}")).expect("scratch");
    let build = pycc()
        .arg("build")
        .arg(fixture(name))
        .arg("-o")
        .arg(dir.join(name))
        .arg("--ext")
        .args(extra)
        .output()
        .expect("pycc should spawn");
    assert!(
        build.status.success(),
        "{name} {extra:?}: {}",
        rendered(&build)
    );
    let import = format!("import {name}");
    let compiled = python()
        .args(["-c", &import])
        .current_dir(&*dir)
        .output()
        .expect("python3 should spawn");
    let oracle = python()
        .args(["-c", &import])
        .current_dir(fixture(name).parent().expect("fixtures directory"))
        .output()
        .expect("python3 should spawn");
    assert_same(&format!("{name} --ext {extra:?}"), &compiled, &oracle);
}

#[test]
#[ignore = "requires a CPython 3.13+ oracle (PYCC_PYTHON, default python3)"]
fn a_call_before_def_raises_a_catchable_name_error() {
    native_matches_cpython("call_before_def", &[]);
    native_matches_cpython("call_before_def", &["--non-strict"]);
}

#[test]
#[ignore = "requires a CPython 3.13+ oracle (PYCC_PYTHON, default python3)"]
fn a_call_before_def_from_methods_closures_and_values_raises_name_error() {
    native_matches_cpython("call_before_def_callers", &[]);
    native_matches_cpython("call_before_def_callers", &["--non-strict"]);
}

#[test]
#[ignore = "requires a CPython 3.13+ oracle (PYCC_PYTHON, default python3)"]
fn an_uncaught_call_before_def_ends_the_program_as_cpython_does() {
    native_matches_cpython("call_before_def_uncaught", &[]);
    native_matches_cpython("call_before_def_uncaught", &["--non-strict"]);
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_call_before_def_raises_a_catchable_name_error_in_an_extension() {
    for name in [
        "call_before_def",
        "call_before_def_callers",
        "call_before_def_uncaught",
    ] {
        ext_matches_cpython(name, &[]);
        ext_matches_cpython(name, &["--non-strict"]);
    }
}
