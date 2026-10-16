//! Issue #1: a module-level function name bound by its `def` and by other
//! bindings (an assignment, a `def` or lambda in CPython fallback, a
//! `global` store, a loop), and a redefined function used as a value.
//! Compiled callers see every rebinding in program order, natively and in
//! an `--ext` module; strict mode refuses a value binding of a `def`'s name
//! with a spanned `C0001` rather than miscompiling it.
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
    let dir = ScratchDir::new(&format!("rebound_function_{name}")).expect("scratch");
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
    let dir = ScratchDir::new(&format!("rebound_function_ext_{name}")).expect("scratch");
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
fn compiled_callers_see_every_rebinding_of_a_function_name() {
    native_matches_cpython("rebound_function", &["--non-strict"]);
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn compiled_callers_see_every_rebinding_of_a_function_name_in_an_extension() {
    ext_matches_cpython("rebound_function", &["--non-strict"]);
}

#[test]
#[ignore = "requires a CPython 3.13+ oracle (PYCC_PYTHON, default python3)"]
fn a_redefined_function_used_as_a_value_is_the_definition_of_that_moment() {
    native_matches_cpython("rebound_function_values", &[]);
    native_matches_cpython("rebound_function_values", &["--non-strict"]);
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_redefined_function_used_as_a_value_is_the_definition_of_that_moment_in_an_extension() {
    ext_matches_cpython("rebound_function_values", &["--non-strict"]);
}

#[test]
fn strict_mode_refuses_a_value_binding_of_a_def_name() {
    let dir = ScratchDir::new("rebound_function_check").expect("scratch");
    let source = dir.join("m.py");
    std::fs::write(
        &source,
        "def a(x: int) -> int:\n    return x + 1\n\n\ndef f(x: int) -> int:\n    return x * 2\n\n\nf = a\n",
    )
    .expect("write the program");
    let check = pycc()
        .arg("check")
        .arg(&source)
        .output()
        .expect("pycc should spawn");
    let out = rendered(&check);
    assert_eq!(check.status.code(), Some(1), "{out}");
    assert!(
        out.contains("error[C0001]: rebinding `f`, which a module-level `def` also binds, is not supported yet"),
        "{out}"
    );
    assert!(out.contains("m.py:9:1"), "{out}");
}
