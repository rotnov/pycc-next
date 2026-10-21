//! `int` locals proven to hold only smallints (`pycc_codegen`'s
//! `int_range`) compute on tagged words without tag tests, overflow checks
//! or bigint releases. The programs mix them with locals that leave the
//! smallint range; each is built strict, `--non-strict` and as an `--ext`
//! module, and compared with CPython 3.13 (`PYCC_PYTHON`, default `python3`):
//! stdout byte-for-byte, the uncaught exception's message line and the exit
//! status. An `--ext` module of such functions is also called from CPython
//! with `bool`s, bigints, floats, `str` and `None`, and leaks nothing.

#[path = "stdlib_support/mod.rs"]
mod support;

use pycc_scratch::ScratchDir;
use std::process::{Command, Output};
use support::{fixture, pycc, python, rendered};

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

/// stdout, the last stderr line and the status.
fn observed(output: &Output) -> (String, String, Option<i32>) {
    let last = text(&pycc_scratch::lf(&output.stderr))
        .lines()
        .last()
        .unwrap_or_default()
        .to_string();
    (text(&pycc_scratch::lf(&output.stdout)), last, output.status.code())
}

#[test]
#[ignore = "requires a CPython 3.13+ oracle and Python.h (PYCC_PYTHON, default python3)"]
fn proven_smallint_locals_match_cpython_in_every_mode() {
    let source = fixture("int_ranges.py");
    let expected = observed(
        &Command::new(python())
            .arg(&source)
            .output()
            .expect("the oracle runs"),
    );
    assert_eq!(expected.2, Some(1), "the fixture ends in ZeroDivisionError");
    let dir = ScratchDir::new("int_ranges").expect("scratch");
    for (mode, flags) in [
        ("debug", &[][..]),
        ("strict", &["--release"][..]),
        ("non-strict", &["--release", "--non-strict"][..]),
    ] {
        let binary = dir.join(format!("app-{mode}"));
        let build = pycc()
            .arg("build")
            .arg(&source)
            .arg("-o")
            .arg(&binary)
            .args(flags)
            .output()
            .expect("pycc spawns");
        assert!(build.status.success(), "{mode}: {}", rendered(&build));
        let got = observed(&Command::new(&binary).output().expect("the program runs"));
        assert_eq!(got, expected, "{mode}");
    }
    let ext = dir.join("ext");
    std::fs::create_dir_all(&ext).expect("ext dir");
    let build = pycc()
        .arg("build")
        .arg(&source)
        .arg("-o")
        .arg(ext.join("int_ranges"))
        .args(["--ext", "--release"])
        .output()
        .expect("pycc spawns");
    assert!(build.status.success(), "--ext: {}", rendered(&build));
    let got = observed(
        &Command::new(python())
            .current_dir(&ext)
            .args(["-c", "import int_ranges"])
            .output()
            .expect("python3 runs"),
    );
    assert_eq!(got, expected, "--ext");
}

/// The proof's facts reach the code: `paeth_like`'s loop over bytes calls
/// no runtime `int` operation but `total`'s addition.
#[test]
fn a_byte_valued_loop_calls_no_int_runtime_function() {
    let dir = ScratchDir::new("int_ranges_ir").expect("scratch");
    let ll = dir.join("m.ll");
    let build = pycc()
        .arg("build")
        .arg(fixture("int_ranges.py"))
        .arg("-o")
        .arg(dir.join("app"))
        .arg("--release")
        .env("PYCC_DUMP_LLVM", &ll)
        .output()
        .expect("pycc spawns");
    assert!(build.status.success(), "{}", rendered(&build));
    let ir = std::fs::read_to_string(&ll).expect("ir");
    let body = ir
        .split("\ndefine ")
        .find(|f| {
            f.lines()
                .next()
                .is_some_and(|l| l.contains("@pyfn_paeth_like("))
        })
        .expect("paeth_like");
    // `total` alone is not proven: its `+=` keeps its `add` fast path, and
    // masking the parameter once calls `pycc_rt_int_and` for a bigint.
    for call in [
        "@pycc_rt_int_sub",
        "@pycc_rt_int_mul",
        "@pycc_rt_int_abs",
        "@pycc_rt_int_cmp",
        "@llvm.ssub.with.overflow",
        "@llvm.smul.with.overflow",
    ] {
        assert!(!body.contains(call), "{call} in\n{body}");
    }
}

/// Builds `int_ranges_api.py` as an `--ext` module (with `flags`) beside the
/// driver, and its source as a plain module beside another copy.
fn ext_and_source(dir: &std::path::Path, flags: &[&str]) {
    let (ext, src) = (dir.join("ext"), dir.join("src"));
    for d in [&ext, &src] {
        std::fs::create_dir_all(d).expect("dir");
        std::fs::copy(fixture("int_ranges_driver.py"), d.join("d.py")).expect("driver");
    }
    std::fs::copy(fixture("int_ranges_api.py"), src.join("int_ranges_api.py")).expect("source");
    let build = pycc()
        .arg("build")
        .arg(fixture("int_ranges_api.py"))
        .arg("-o")
        .arg(ext.join("int_ranges_api"))
        .arg("--ext")
        .args(flags)
        .output()
        .expect("pycc spawns");
    assert!(build.status.success(), "{}", rendered(&build));
}

#[test]
#[ignore = "requires a CPython 3.13+ oracle and Python.h (PYCC_PYTHON, default python3)"]
fn an_ext_module_called_with_other_types_matches_cpython() {
    for flags in [&["--release"][..], &["--release", "--non-strict"][..]] {
        let dir = ScratchDir::new("int_ranges_ext").expect("scratch");
        ext_and_source(&dir, flags);
        let run = |d: &str| {
            Command::new(python())
                .arg("d.py")
                .current_dir(dir.join(d))
                .output()
                .expect("python3 runs")
        };
        let (ours, oracle) = (run("ext"), run("src"));
        assert!(oracle.status.success(), "{}", text(&pycc_scratch::lf(&oracle.stderr)));
        assert!(ours.status.success(), "{}", text(&pycc_scratch::lf(&ours.stderr)));
        assert_eq!(text(&pycc_scratch::lf(&ours.stdout)), text(&pycc_scratch::lf(&oracle.stdout)), "{flags:?}");
    }
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn proven_smallint_functions_leak_nothing() {
    let dir = ScratchDir::new("int_ranges_leaks").expect("scratch");
    ext_and_source(&dir, &["--release"]);
    std::fs::copy(fixture("int_ranges_leaks.py"), dir.join("ext").join("l.py")).expect("probe");
    let out = Command::new(python())
        .arg("l.py")
        .current_dir(dir.join("ext"))
        .output()
        .expect("python3 runs");
    assert!(out.status.success(), "{}", text(&pycc_scratch::lf(&out.stderr)));
    assert_eq!(text(&pycc_scratch::lf(&out.stdout)), "ok\n");
}
