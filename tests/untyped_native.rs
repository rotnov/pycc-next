//! Untyped code runs native (v0.4 accept 1).
//!
//! - An `--ext --non-strict` module's unannotated functions and methods get
//!   guarded native specializations (`pycc_ast::specialize_untyped`,
//!   `src/specialize.rs`): called from CPython with the types the guard
//!   expects, with others (a `str` subclass, `bytes`, a `list`, `None`,
//!   `bool`, an `int` subclass, a float), the module prints what CPython
//!   prints for its source, and repeated calls leak nothing.
//! - Non-strict widening is per function: a name one function rebinds with
//!   another type stays native in every other function binding it once.
//! - Native `str` indexing and slicing by code point, ASCII and not
//!   (`pycc_rt::str_index`), match CPython in a strict native build.

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

fn pycc(args: &[&str], source: &Path, out: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
        .arg("build")
        .args(args)
        .arg(source)
        .arg("-o")
        .arg(out)
        .output()
        .expect("pycc should spawn")
}

/// Same stdout, exit status and last stderr line (the exception).
fn assert_same(ours: &Output, oracle: &Output) {
    assert_eq!(text(&pycc_scratch::lf(&ours.stdout)), text(&pycc_scratch::lf(&oracle.stdout)));
    assert_eq!(ours.status.code(), oracle.status.code(), "{}", text(&pycc_scratch::lf(&ours.stderr)));
    assert_eq!(
        text(&pycc_scratch::lf(&ours.stderr)).lines().last(),
        text(&pycc_scratch::lf(&oracle.stderr)).lines().last()
    );
}

fn run_python(dir: &Path, script: &str) -> Output {
    Command::new(python())
        .arg(script)
        .current_dir(dir)
        .output()
        .expect("python")
}

/// Builds `untyped_specialize.py` as the extension `m` in `<dir>/ext`, next
/// to its source as `m.py` in `<dir>/src`, each with the driver files;
/// returns the build's report.
fn build_specialized(dir: &Path) -> String {
    let (ext, src) = (dir.join("ext"), dir.join("src"));
    for d in [&ext, &src] {
        std::fs::create_dir_all(d).expect("dir");
        std::fs::copy(fixture("untyped_specialize_driver.py"), d.join("d.py")).expect("driver");
        std::fs::copy(fixture("untyped_specialize_leaks.py"), d.join("l.py")).expect("probe");
    }
    std::fs::copy(fixture("untyped_specialize.py"), src.join("m.py")).expect("source");
    let build_src = dir.join("m.py");
    std::fs::copy(fixture("untyped_specialize.py"), &build_src).expect("source");
    let build = pycc(
        &["--ext", "--non-strict", "--report-fallbacks"],
        &build_src,
        &ext.join("m"),
    );
    let report = text(&pycc_scratch::lf(&build.stderr));
    assert!(build.status.success(), "{report}{}", text(&pycc_scratch::lf(&build.stdout)));
    report
}

#[test]
#[ignore = "requires a CPython 3.13+ oracle and Python.h (PYCC_PYTHON, default python3)"]
fn untyped_ext_functions_run_native_with_cpython_results() {
    let dir = ScratchDir::new("untyped_native_ext").expect("scratch");
    let report = build_specialized(&dir);
    assert!(!report.contains("fallback:"), "{report}");
    for spec in [
        "`Text.common_prefix` runs natively when called with a: str, b: str",
        "`Text.common_suffix` runs natively when called with a: str, b: str",
        "`Text.scan` runs natively when called with text: str, start: int",
        "`windows` runs natively when called with s: str, k: int",
        "`stats` runs natively when called with s: str",
        "`char_at` runs natively when called with s: str, i: int",
        "`total` runs natively when called with n: int",
    ] {
        assert!(report.contains(&format!("specialized: {spec}")), "{spec}\n{report}");
    }
    let ours = run_python(&dir.join("ext"), "d.py");
    let oracle = run_python(&dir.join("src"), "d.py");
    assert!(oracle.status.success(), "{}", text(&pycc_scratch::lf(&oracle.stderr)));
    assert_same(&ours, &oracle);
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn specialized_calls_leak_nothing() {
    let dir = ScratchDir::new("untyped_native_leaks").expect("scratch");
    build_specialized(&dir);
    let probe = run_python(&dir.join("ext"), "l.py");
    assert!(probe.status.success(), "{}", text(&pycc_scratch::lf(&probe.stderr)));
    assert_eq!(text(&pycc_scratch::lf(&probe.stdout)), "ok\n");
}

/// The specialization of a function that does not type-check with the
/// guessed types is dropped, and the function still compiles: nothing
/// falls back, and the module behaves as its source.
#[test]
#[ignore = "requires a CPython 3.13+ oracle and Python.h (PYCC_PYTHON, default python3)"]
fn a_refused_specialization_is_dropped_not_fallen_back() {
    let dir = ScratchDir::new("untyped_native_dropped").expect("scratch");
    let source = "def f(s, k):\n    out = []\n    for i in range(k):\n        out.append(s[i])\n    print(s + 1)\n    return out\n\n\
                  def g(s):\n    n = 0\n    for c in s:\n        n += len(s[0:1])\n    return n\n";
    let (ext, src) = (dir.join("ext"), dir.join("src"));
    std::fs::create_dir_all(&ext).expect("dir");
    std::fs::create_dir_all(&src).expect("dir");
    std::fs::write(dir.join("m.py"), source).expect("write");
    std::fs::write(src.join("m.py"), source).expect("write");
    let build = pycc(&["--ext", "--non-strict", "--report-fallbacks"], &dir.join("m.py"), &ext.join("m"));
    let report = text(&pycc_scratch::lf(&build.stderr));
    assert!(build.status.success(), "{report}");
    assert!(!report.contains("fallback:"), "{report}");
    assert!(report.contains("not specialized: `f`"), "{report}");
    assert!(report.contains("specialized: `g` runs natively when called with s: str"), "{report}");
    let script = "import m\nprint(m.g('abc'), m.g([[1]]))\ntry:\n    m.f('ab', 2)\nexcept TypeError as e:\n    print(e)\n";
    let run = |d: &Path| {
        Command::new(python())
            .args(["-c", script])
            .current_dir(d)
            .output()
            .expect("python")
    };
    assert_same(&run(&ext), &run(&src));
}

#[test]
#[ignore = "requires a CPython 3.13+ oracle and embed interpreter (PYCC_PYTHON, default python3)"]
fn widening_a_rebound_name_is_per_function() {
    let dir = ScratchDir::new("untyped_native_widening").expect("scratch");
    let source = dir.join("w.py");
    std::fs::copy(fixture("non_strict_local_widening.py"), &source).expect("copy");
    let ir = dir.join("w.ll");
    let build = Command::new(env!("CARGO_BIN_EXE_pycc"))
        .env("PYCC_DUMP_LLVM", &ir)
        .args(["build", "--non-strict", "--report-fallbacks"])
        .arg(&source)
        .arg("-o")
        .arg(dir.join("w"))
        .output()
        .expect("pycc");
    let report = text(&pycc_scratch::lf(&build.stderr));
    assert!(build.status.success(), "{report}");
    assert!(!report.contains("fallback:"), "{report}");
    let ours = Command::new(dir.join("w")).output().expect("run");
    let oracle = Command::new(python()).arg(&source).output().expect("python");
    assert_same(&ours, &oracle);
    let ir = std::fs::read_to_string(dir.join("w.ll.pre.ll"))
        .expect("IR")
        .replace("\r\n", "\n");
    let body = |name: &str| -> String {
        let start = ir
            .find(&format!("@pyfn_{name}("))
            .unwrap_or_else(|| panic!("no {name}"));
        let end = ir[start..].find("\n}\n").expect("end") + start;
        ir[start..end].to_string()
    };
    // The functions that rebind keep the object; the others stay native.
    assert!(body("rebinds").contains("pycc_ext_obj"));
    assert!(body("Walker.label").contains("pycc_ext_obj"));
    assert!(!body("counts").contains("pycc_ext_obj"), "{}", body("counts"));
    assert!(!body("Walker.walk").contains("pycc_ext_obj"), "{}", body("Walker.walk"));
}

#[test]
#[ignore = "requires a CPython 3.13+ oracle (PYCC_PYTHON, default python3)"]
fn native_str_indexing_by_code_point_matches_cpython() {
    let dir = ScratchDir::new("untyped_native_str_index").expect("scratch");
    let source = dir.join("s.py");
    std::fs::copy(fixture("str_unicode_index.py"), &source).expect("copy");
    let build = pycc(&["--release"], &source, &dir.join("s"));
    assert!(build.status.success(), "{}", text(&pycc_scratch::lf(&build.stderr)));
    let ours = Command::new(dir.join("s")).output().expect("run");
    let oracle = Command::new(python()).arg(&source).output().expect("python");
    assert_eq!(oracle.status.code(), Some(1));
    assert_same(&ours, &oracle);
}
