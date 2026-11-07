//! Untyped byte-buffer loops run native (v0.5 accept 2).
//!
//! An `--ext --non-strict` module's unannotated functions whose parameters'
//! items are used as numbers compile (`--select all`). Called from CPython
//! with `bytearray`s and with anything else (`bytes`, a `list`, an `array`,
//! a `memoryview`, a `bytearray` subclass, `str`, `None`, `bool`, a float),
//! through out-of-range indices and bytes, a buffer the loop itself grows or
//! shrinks, and a result aliasing an argument, the module prints what
//! CPython prints for its source; repeated calls leak nothing.

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

/// Builds `untyped_bytearray.py` as the extension `m` in `<dir>/ext` (with
/// `flags`), next to its source as `m.py` in `<dir>/src`, each with the
/// driver and the leak probe; returns the build's report.
fn build(dir: &Path, flags: &[&str]) -> String {
    let (ext, src) = (dir.join("ext"), dir.join("src"));
    for d in [&ext, &src] {
        std::fs::create_dir_all(d).expect("dir");
        std::fs::copy(fixture("untyped_bytearray_driver.py"), d.join("d.py")).expect("driver");
        std::fs::copy(fixture("untyped_bytearray_leaks.py"), d.join("l.py")).expect("probe");
    }
    std::fs::copy(fixture("untyped_bytearray.py"), src.join("m.py")).expect("source");
    let build_src = dir.join("m.py");
    std::fs::copy(fixture("untyped_bytearray.py"), &build_src).expect("source");
    let out = Command::new(env!("CARGO_BIN_EXE_pycc"))
        .arg("build")
        .args(["--ext", "--non-strict", "--select", "all", "--report-fallbacks"])
        .args(flags)
        .arg(&build_src)
        .arg("-o")
        .arg(ext.join("m"))
        .output()
        .expect("pycc should spawn");
    let report = text(&pycc_scratch::lf(&out.stderr));
    assert!(out.status.success(), "{report}{}", text(&pycc_scratch::lf(&out.stdout)));
    report
}

#[test]
#[ignore = "requires a CPython 3.13+ oracle and Python.h (PYCC_PYTHON, default python3)"]
fn bytearray_loops_run_native_with_cpython_results() {
    for flags in [&[][..], &["--release"][..]] {
        let dir = ScratchDir::new("untyped_bytearray").expect("scratch");
        let report = build(&dir, flags);
        assert!(!report.contains("not compiled:"), "{report}");
        for name in ["undo_sub", "paeth", "checksum", "poke", "peek", "grow", "shrink", "unpack"] {
            assert!(report.contains(&format!("compiled: m:{name}\n")), "{name}\n{report}");
        }
        let ours = run_python(&dir.join("ext"), "d.py");
        let oracle = run_python(&dir.join("src"), "d.py");
        assert!(oracle.status.success(), "{}", text(&pycc_scratch::lf(&oracle.stderr)));
        assert!(ours.status.success(), "{}", text(&pycc_scratch::lf(&ours.stderr)));
        assert_eq!(text(&pycc_scratch::lf(&ours.stdout)), text(&pycc_scratch::lf(&oracle.stdout)));
    }
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn bytearray_specializations_leak_nothing() {
    let dir = ScratchDir::new("untyped_bytearray_leaks").expect("scratch");
    build(&dir, &["--release"]);
    let probe = run_python(&dir.join("ext"), "l.py");
    assert!(probe.status.success(), "{}", text(&pycc_scratch::lf(&probe.stderr)));
    assert_eq!(text(&pycc_scratch::lf(&probe.stdout)), "ok\n");
}
