//! Untyped code compiled natively by `--non-strict` (ROADMAP v0.6 accept 2).
//!
//! `tests/fixtures/untyped_classes.py` holds the constructs: `bytes`
//! literals, calls unpacking `*args`/`**kwargs` into a CPython callee,
//! `except` clauses naming a CPython object, an object tested against a
//! tuple of scalars, a standard-library function called with an object, a
//! builtin exception class as a value and builtin calls inside generators.
//! Built `--ext --non-strict`, the functions holding them compile
//! (`--report-fallbacks` lists none of them) and, called from CPython with
//! the arguments they expect and with others, print what CPython prints
//! for the source; repeated calls leak nothing. Built as a non-strict
//! program, the fixture prints what CPython prints; strict mode keeps
//! refusing a `bytes` literal. `tests/fixtures/untyped_readers.py` does the
//! same for the shapes of a binary format reader (pypng's).

use pycc_scratch::ScratchDir;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

fn python() -> std::ffi::OsString {
    std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into())
}

fn run_python(dir: &Path, script: &str) -> Output {
    Command::new(python())
        .arg(script)
        .current_dir(dir)
        .output()
        .expect("python")
}

/// The functions of the fixture that must compile (not fall back).
const COMPILED: &[&str] = &[
    "tag",
    "literals",
    "group",
    "chain",
    "pack",
    "call_mixed",
    "unpack_word",
    "classify",
    "unmatched",
    "filter_kind",
    "row_count",
    "warn_if",
    "decompress",
    "pick",
    "apply",
];

/// Builds the fixture `untyped_classes.py` as the extension `m` in
/// `<dir>/ext`, next to its source as `m.py` in `<dir>/src`, each with
/// `script` as `d.py`; returns the build's report.
fn build(dir: &Path, script: &str, flags: &[&str]) -> String {
    build_module(dir, "untyped_classes.py", script, flags)
}

/// [`build`] for the fixture module `module`.
fn build_module(dir: &Path, module: &str, script: &str, flags: &[&str]) -> String {
    let (ext, src) = (dir.join("ext"), dir.join("src"));
    for d in [&ext, &src] {
        std::fs::create_dir_all(d).expect("dir");
        std::fs::copy(fixture(script), d.join("d.py")).expect("driver");
    }
    std::fs::copy(fixture(module), src.join("m.py")).expect("source");
    let build_src = dir.join("m.py");
    std::fs::copy(fixture(module), &build_src).expect("source");
    let out = Command::new(env!("CARGO_BIN_EXE_pycc"))
        .arg("build")
        .args(["--ext", "--non-strict", "--report-fallbacks"])
        .args(flags)
        .arg(&build_src)
        .arg("-o")
        .arg(ext.join("m"))
        .output()
        .expect("pycc should spawn");
    let report = text(&out.stderr);
    assert!(out.status.success(), "{report}{}", text(&out.stdout));
    report
}

#[test]
#[ignore = "requires a CPython 3.13+ oracle and Python.h (PYCC_PYTHON, default python3)"]
fn untyped_constructs_compile_with_cpython_results() {
    for flags in [&[][..], &["--release"][..]] {
        let dir = ScratchDir::new("untyped_classes").expect("scratch");
        let report = build(&dir, "untyped_classes_driver.py", flags);
        for name in COMPILED {
            assert!(
                !report.contains(&format!("fallback: function `{name}`")),
                "{name}\n{report}"
            );
        }
        assert!(!report.contains("fallback: class `Decoder`"), "{report}");
        let ours = run_python(&dir.join("ext"), "d.py");
        let oracle = run_python(&dir.join("src"), "d.py");
        assert!(oracle.status.success(), "{}", text(&oracle.stderr));
        assert!(ours.status.success(), "{}", text(&ours.stderr));
        assert_eq!(text(&ours.stdout), text(&oracle.stdout));
    }
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn untyped_constructs_leak_nothing() {
    let dir = ScratchDir::new("untyped_classes_leaks").expect("scratch");
    build(&dir, "untyped_classes_leaks.py", &["--release"]);
    let probe = run_python(&dir.join("ext"), "d.py");
    assert!(probe.status.success(), "{}", text(&probe.stderr));
    assert_eq!(text(&probe.stdout), "ok\n");
}

/// `tests/fixtures/untyped_readers.py`: a binary format reader's shapes
/// (handlers looked up by name, generator methods, byte buffers, `None`
/// defaults, an attribute snapshot in a specialization) compile with no
/// fallback, and compare with CPython from a driver passing other types.
#[test]
#[ignore = "requires a CPython 3.13+ oracle and Python.h (PYCC_PYTHON, default python3)"]
fn a_reader_compiles_with_cpython_results() {
    for flags in [&[][..], &["--release"][..]] {
        let dir = ScratchDir::new("untyped_readers").expect("scratch");
        let report = build_module(&dir, "untyped_readers.py", "untyped_readers_driver.py", flags);
        assert!(!report.contains("fallback:"), "{report}");
        assert!(
            report.contains("`Reader._values` runs natively when called with bs: bytearray, self.depth: int"),
            "{report}"
        );
        let ours = run_python(&dir.join("ext"), "d.py");
        let oracle = run_python(&dir.join("src"), "d.py");
        assert!(oracle.status.success(), "{}", text(&oracle.stderr));
        assert!(ours.status.success(), "{}", text(&ours.stderr));
        assert_eq!(text(&ours.stdout), text(&oracle.stdout));
    }
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_reader_leaks_nothing() {
    let dir = ScratchDir::new("untyped_readers_leaks").expect("scratch");
    build_module(&dir, "untyped_readers.py", "untyped_readers_leaks.py", &["--release"]);
    let probe = run_python(&dir.join("ext"), "d.py");
    assert!(probe.status.success(), "{}", text(&probe.stderr));
    assert_eq!(text(&probe.stdout), "ok\n");
}

#[test]
#[ignore = "requires a CPython 3.13+ oracle and Python.h (PYCC_PYTHON, default python3)"]
fn untyped_constructs_run_as_a_non_strict_program() {
    for module in ["untyped_classes.py", "untyped_readers.py"] {
        run_as_program(module);
    }
}

fn run_as_program(module: &str) {
    let dir = ScratchDir::new("untyped_classes_native").expect("scratch");
    let source = dir.join("prog.py");
    std::fs::copy(fixture(module), &source).expect("source");
    let out = Command::new(env!("CARGO_BIN_EXE_pycc"))
        .args(["build", "--non-strict", "--release"])
        .arg(&source)
        .arg("-o")
        .arg(dir.join("prog"))
        .output()
        .expect("pycc should spawn");
    assert!(out.status.success(), "{}", text(&out.stderr));
    let ours = Command::new(dir.join("prog")).output().expect("program");
    let oracle = run_python(&dir, "prog.py");
    assert!(oracle.status.success(), "{}", text(&oracle.stderr));
    assert!(ours.status.success(), "{}", text(&ours.stderr));
    assert_eq!(text(&ours.stdout), text(&oracle.stdout));
}

/// Strict mode has no CPython object to hold a `bytes` value in: the
/// literal stays refused, at its span.
#[test]
fn strict_mode_refuses_a_bytes_literal() {
    let dir = ScratchDir::new("untyped_classes_strict").expect("scratch");
    let source = dir.join("prog.py");
    std::fs::write(&source, "def f() -> int:\n    x = b\"ab\"\n    return len(x)\n\n\nprint(f())\n")
        .expect("write");
    let out = Command::new(env!("CARGO_BIN_EXE_pycc"))
        .arg("check")
        .arg(&source)
        .output()
        .expect("pycc should spawn");
    assert!(!out.status.success());
    let stderr = text(&out.stderr) + &text(&out.stdout);
    assert!(stderr.contains("C0001"), "{stderr}");
    assert!(stderr.contains("a bytes literal"), "{stderr}");
    assert!(stderr.contains("prog.py:2:9"), "{stderr}");
}
