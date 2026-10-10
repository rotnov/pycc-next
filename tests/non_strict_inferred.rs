//! Non-strict mode (#3) in a native build: an unannotated public function's
//! signature is inferred from its call sites like a private helper's, an
//! import or `sys` stream alias that fallback code reads runs in CPython too
//! instead of dragging the statement into the fallback, and the CPython
//! output stays the oracle.

use pycc_scratch::ScratchDir;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

fn embed_interpreter_available() -> bool {
    let interpreter = std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into());
    Command::new(interpreter)
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
}

fn run_with_stdin(mut command: Command, stdin: &[u8]) -> Output {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(stdin)
        .expect("write stdin");
    child.wait_with_output().expect("wait")
}

/// Builds `name` non-strict, runs it and CPython on the shared stdin (a
/// program that reads none ignores it), and returns the fallback report.
fn matches_cpython(name: &str) -> String {
    let dir = ScratchDir::new("non_strict_inferred").expect("scratch");
    let source = dir.join("m.py");
    std::fs::copy(fixture(name), &source).expect("copy the fixture");
    let stdin = std::fs::read(fixture("non_strict_inferred.stdin")).expect("stdin fixture");
    let build = Command::new(env!("CARGO_BIN_EXE_pycc"))
        .args(["build", "--non-strict", "--report-fallbacks"])
        .arg(&source)
        .arg("-o")
        .arg(dir.join("app"))
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", text(&build.stderr));
    let ours = run_with_stdin(Command::new(dir.join("app")), &stdin);
    let mut python = Command::new("python3");
    python.arg(&source);
    let oracle = run_with_stdin(python, &stdin);
    assert_eq!(text(&ours.stdout), text(&oracle.stdout));
    assert_eq!(
        ours.status.code(),
        oracle.status.code(),
        "{}",
        text(&ours.stderr)
    );
    assert_eq!(
        text(&ours.stderr).lines().last(),
        text(&oracle.stderr).lines().last()
    );
    text(&build.stderr)
}

#[test]
fn unannotated_functions_and_stdin_alias_compile_without_fallback() {
    if !embed_interpreter_available() {
        eprintln!("skipped: no CPython to embed (set PYCC_PYTHON)");
        return;
    }
    let report = matches_cpython("non_strict_inferred.py");
    assert!(!report.contains("fallback:"), "{report}");
}

#[test]
fn fallback_calls_into_inferred_functions_keep_cpython_semantics() {
    if !embed_interpreter_available() {
        eprintln!("skipped: no CPython to embed (set PYCC_PYTHON)");
        return;
    }
    let report = matches_cpython("non_strict_inferred_mixed.py");
    // `show` is called with an `int` and a `str`: fallback code calls it, so
    // it is compiled with an object parameter (and published) rather than
    // an inferred `int` one, and returns `None` off its end.
    assert!(!report.contains("function `show`"), "{report}");
    // The `sys` import and the `input` alias stay compiled.
    assert!(!report.contains(":1 runs"), "{report}");
    assert!(!report.contains(":3 runs"), "{report}");
}

#[test]
fn fallback_calls_omitting_a_default_match_cpython() {
    if !embed_interpreter_available() {
        eprintln!("skipped: no CPython to embed (set PYCC_PYTHON)");
        return;
    }
    let report = matches_cpython("non_strict_fallback_defaults.py");
    assert!(report.contains("function `scale`"), "{report}");
    assert!(report.contains("function `scale2`"), "{report}");
}

#[test]
fn builtin_calls_pycc_does_not_compile_run_through_cpython_builtins() {
    if !embed_interpreter_available() {
        eprintln!("skipped: no CPython to embed (set PYCC_PYTHON)");
        return;
    }
    // Ends in CPython's `TypeError` from `min(1, "a")`, raised in compiled
    // code through `builtins.min`.
    let report = matches_cpython("non_strict_builtins.py");
    // Only the keyword call falls back; `min`, `chr`, `set()`, `divmod`,
    // three-argument `pow`, `bin`, `hex` and a mixed `max` stay compiled.
    assert_eq!(report.matches("fallback:").count(), 1, "{report}");
    assert!(report.contains(":18 runs"), "{report}");
}
