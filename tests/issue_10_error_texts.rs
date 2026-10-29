//! Issue #10: a published compiled function, method or constructor called
//! with too few or too many positional arguments, or with an unknown,
//! repeated or misspelled keyword, raises CPython 3.13's `TypeError` word
//! for word (`missing 1 required positional argument: 'x'`, `takes from 2
//! to 3 positional arguments but 4 were given`, `got an unexpected keyword
//! argument 'b'. Did you mean 'by'?`), and a missing attribute of a compiled
//! class or instance CPython's `AttributeError`.
//!
//! The oracle is `python3` (or `PYCC_PYTHON`): `issue_10_driver.py` makes
//! the wrong calls against `issue_10_api.py` imported as source, and against
//! the same file built `--ext` and `--ext --non-strict`;
//! `issue_10_native.py` makes them from fallback code of a native
//! `--non-strict` build. Strict native code cannot make these calls: they
//! are compile-time `T0021` diagnostics there.

use pycc_scratch::ScratchDir;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn pycc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
}

fn python() -> Command {
    Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(&pycc_scratch::lf(bytes)).replace("\r\n", "\n")
}

fn last_line(bytes: &[u8]) -> String {
    text(bytes).lines().last().unwrap_or_default().to_string()
}

fn fixture(name: &str, dir: &Path, as_name: &str) -> PathBuf {
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(format!("{name}.py"));
    let path = dir.join(as_name);
    std::fs::copy(source, &path).expect("copy the fixture");
    path
}

fn assert_matches(ours: &Output, oracle: &Output) {
    assert_eq!(
        text(&ours.stdout),
        text(&oracle.stdout),
        "{}",
        text(&ours.stderr)
    );
    assert_eq!(
        ours.status.code(),
        oracle.status.code(),
        "{}",
        text(&ours.stderr)
    );
    assert_eq!(last_line(&ours.stderr), last_line(&oracle.stderr));
}

fn build(source: &Path, out: &Path, flags: &[&str]) -> String {
    let build = pycc()
        .args(["build", "--report-fallbacks"])
        .args(flags)
        .arg(source)
        .arg("-o")
        .arg(out)
        .output()
        .expect("pycc should spawn");
    let report = text(&build.stderr);
    assert!(build.status.success(), "{report}");
    report
}

fn run_driver(dir: &Path) -> Output {
    python()
        .arg("driver.py")
        .current_dir(dir)
        .output()
        .expect("python should spawn")
}

/// The driver against `api` built with `flags`, and against its source.
fn ext(flags: &[&str], tag: &str) {
    let compiled = ScratchDir::new(&format!("issue_10_{tag}")).expect("scratch");
    let source = fixture("issue_10_api", &compiled, "api.py");
    fixture("issue_10_driver", &compiled, "driver.py");
    build(&source, &compiled.join("api"), flags);
    std::fs::remove_file(&source).expect("import the artifact, not the source");
    let plain = ScratchDir::new(&format!("issue_10_{tag}_src")).expect("scratch");
    fixture("issue_10_api", &plain, "api.py");
    fixture("issue_10_driver", &plain, "driver.py");
    let oracle = run_driver(&plain);
    assert!(
        text(&oracle.stdout).contains("Did you mean 'by'?"),
        "the oracle is CPython 3.13: {}",
        text(&oracle.stdout)
    );
    assert_matches(&run_driver(&compiled), &oracle);
}

#[test]
fn ext_strict_raises_cpython_argument_and_attribute_errors() {
    ext(&["--ext"], "ext");
}

#[test]
fn ext_non_strict_raises_cpython_argument_and_attribute_errors() {
    ext(&["--ext", "--non-strict"], "ext_ns");
}

#[test]
fn native_non_strict_fallback_code_sees_cpython_errors() {
    let dir = ScratchDir::new("issue_10_native").expect("scratch");
    let source = fixture("issue_10_native", &dir, "m.py");
    let report = build(&source, &dir.join("app"), &["--non-strict"]);
    // The classes stay native: fallback code calls their published members
    // (a missing attribute is read through `getattr`, since code reading an
    // attribute a class does not declare keeps that class in CPython).
    assert!(!report.contains("class `Grid`"), "{report}");
    assert!(!report.contains("class `Box`"), "{report}");
    let ours = Command::new(dir.join("app"))
        .output()
        .expect("run the build");
    let oracle = python().arg(&source).output().expect("python should spawn");
    assert_matches(&ours, &oracle);
}

/// Strict native code refuses each wrong call at compile time instead.
#[test]
fn strict_native_refuses_a_wrong_call() {
    let dir = ScratchDir::new("issue_10_strict").expect("scratch");
    let source = dir.join("m.py");
    std::fs::write(
        &source,
        "def two(x: int, y: int) -> int:\n    return x + y\n\n\nprint(two(1))\n",
    )
    .expect("write");
    let out = pycc()
        .args(["check"])
        .arg(&source)
        .output()
        .expect("pycc should spawn");
    assert!(!out.status.success());
    let all = text(&out.stdout) + &text(&out.stderr);
    assert!(all.contains("T0021"), "{all}");
}
