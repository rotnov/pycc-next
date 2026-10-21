//! Untyped byte-buffer loops run native (v0.5 accept 2).
//!
//! An `--ext --non-strict` module's unannotated function whose parameters'
//! items are used as numbers gets a guarded specialization taking those
//! parameters as `bytearray` (`pycc_ast::specialize_untyped`), which reads
//! and writes the caller's buffer in place (`pycc_hir::ndarray`'s byte
//! buffers). Called from CPython with `bytearray`s and with anything else
//! (`bytes`, a `list`, an `array`, a `memoryview`, a `bytearray` subclass,
//! `str`, `None`, `bool`, a float), through out-of-range indices and bytes,
//! a buffer the loop itself grows or shrinks, and a result aliasing an
//! argument, the module prints what CPython prints for its source; repeated
//! calls leak nothing.

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
        .args(["--ext", "--non-strict", "--report-fallbacks"])
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
        assert!(!report.contains("fallback:"), "{report}");
        assert!(!report.contains("not specialized"), "{report}");
        for spec in [
            "`undo_sub` runs natively when called with fu: int, scanline: bytearray, result: bytearray",
            "`paeth` runs natively when called with fu: int, scanline: bytearray, previous: bytearray, result: bytearray",
            "`checksum` runs natively when called with data: bytearray",
            "`poke` runs natively when called with buf: bytearray, i: int, v: int",
            "`peek` runs natively when called with buf: bytearray, i: int",
            "`grow` runs natively when called with buf: bytearray, n: int",
            "`shrink` runs natively when called with buf: bytearray",
            "`unpack` runs natively when called with bs: bytearray, depth: int, k: int",
        ] {
            assert!(report.contains(&format!("specialized: {spec}")), "{spec}\n{report}");
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

/// A specialization whose body calls a builtin the original calls through
/// CPython (`abs` of an object) calls it natively, and one that cannot is
/// still specialized, calling it as the original does.
#[test]
#[ignore = "requires a CPython 3.13+ oracle and Python.h (PYCC_PYTHON, default python3)"]
fn a_specialization_calls_builtins_natively_when_it_can() {
    let dir = ScratchDir::new("untyped_bytearray_builtins").expect("scratch");
    // `abs` of an `int` compiles; `format` and `ascii` of a `str` do not.
    let source = "def f(s, k):\n    t = 0\n    for i in range(k):\n        t += abs(s[i] - 9)\n    return t\n\n\
                  def g(s, k):\n    t = []\n    for i in range(k):\n        t.append(format(s[i:i + 1], \">3\") + ascii(s[i]))\n    return t\n";
    let (ext, src) = (dir.join("ext"), dir.join("src"));
    std::fs::create_dir_all(&ext).expect("dir");
    std::fs::create_dir_all(&src).expect("dir");
    std::fs::write(dir.join("m.py"), source).expect("write");
    std::fs::write(src.join("m.py"), source).expect("write");
    let ll = dir.join("m.ll");
    let out = Command::new(env!("CARGO_BIN_EXE_pycc"))
        .args(["build", "--ext", "--non-strict", "--report-fallbacks"])
        .arg(dir.join("m.py"))
        .arg("-o")
        .arg(ext.join("m"))
        .env("PYCC_DUMP_LLVM", &ll)
        .output()
        .expect("pycc should spawn");
    let report = text(&pycc_scratch::lf(&out.stderr));
    assert!(out.status.success(), "{report}");
    assert!(report.contains("specialized: `f` runs natively when called with s: bytearray, k: int"), "{report}");
    assert!(report.contains("specialized: `g` runs natively when called with s: str, k: int"), "{report}");
    // `f`'s specialization computes `abs` natively, not with CPython: inline,
    // since `s[i] - 9` is a smallint (`int_range`).
    let ir = std::fs::read_to_string(&ll).expect("ir");
    let body = ir
        .split("\ndefine ")
        .find(|f| f.lines().next().is_some_and(|l| l.contains("@pyfn__pycc_native_f(")))
        .expect("the specialization");
    assert!(!body.contains("@pycc_rt_int_abs"), "{body}");
    assert!(!body.contains("@pycc_ext_obj_call"), "{body}");
    let script = "import m\nprint(m.f(bytearray(b'\\x01\\x10\\xff'), 3), m.f([1, 2], 2), m.g('abc', 3))\n\
                  try:\n    m.f(bytearray(b'a'), 2)\nexcept IndexError as e:\n    print(e)\n";
    let run = |d: &Path| {
        Command::new(python())
            .args(["-c", script])
            .current_dir(d)
            .output()
            .expect("python")
    };
    let (ours, oracle) = (run(&ext), run(&src));
    assert!(oracle.status.success(), "{}", text(&pycc_scratch::lf(&oracle.stderr)));
    assert_eq!(text(&pycc_scratch::lf(&ours.stdout)), text(&pycc_scratch::lf(&oracle.stdout)), "{}", text(&pycc_scratch::lf(&ours.stderr)));
}
