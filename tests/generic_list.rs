//! One generic `list[T]`: `int`, `float`, `bool`, `str`, user-class instance
//! and nested-list elements, every list operation the compiler admits, and
//! the `round()` builtin the float benchmarks print with.
//!
//! The oracle is CPython itself: `tests/fixtures/generic_list.py` is built
//! with `--release`, run, and compared byte for byte with `python3` running
//! the same file -- stdout, the uncaught exception's last line and the exit
//! status. The `--ext` arm imports the same module into the host interpreter
//! and is `#[ignore]`d like every hosted test. The refusals at the end pin
//! the shapes deliberately left unsupported.

use pycc_scratch::ScratchDir;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn pycc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
}

fn python() -> Command {
    Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
}

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/generic_list.py")
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

fn last_line(bytes: &[u8]) -> String {
    text(bytes).lines().last().unwrap_or_default().to_string()
}

fn assert_same_as_cpython(ours: &Output, theirs: &Output) {
    assert_eq!(text(&ours.stdout), text(&theirs.stdout), "stdout differs");
    assert_eq!(last_line(&ours.stderr), last_line(&theirs.stderr));
    assert_eq!(ours.status.code(), theirs.status.code());
}

#[test]
fn every_list_element_type_matches_cpython() {
    let dir = ScratchDir::new("generic_list_native").expect("scratch");
    let out = dir.join("generic_list");
    let build = pycc()
        .args([
            "build",
            fixture().to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
            "--release",
        ])
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", text(&build.stderr));
    let ours = Command::new(&out).output().expect("the program should run");
    let theirs = python()
        .arg(fixture())
        .output()
        .expect("python3 should spawn");
    assert_eq!(
        theirs.status.code(),
        Some(1),
        "the fixture ends on an IndexError"
    );
    assert_same_as_cpython(&ours, &theirs);
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn every_list_element_type_matches_cpython_inside_an_ext_module() {
    let dir = ScratchDir::new("generic_list_ext").expect("scratch");
    std::fs::copy(fixture(), dir.join("generic_list_src.py")).expect("copy the fixture");
    let build = pycc()
        .arg("build")
        .arg(fixture())
        .arg("-o")
        .arg(dir.join("generic_list"))
        .arg("--ext")
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", text(&build.stderr));
    let ours = python()
        .args(["-c", "import generic_list"])
        .current_dir(&*dir)
        .output()
        .expect("python3 should spawn");
    let theirs = python()
        .arg("generic_list_src.py")
        .current_dir(&*dir)
        .output()
        .expect("python3 should spawn");
    assert_same_as_cpython(&ours, &theirs);
}

/// Runs `pycc check` on `source` and returns its diagnostics, asserting failure.
fn check_fails(label: &str, source: &str) -> String {
    let dir = ScratchDir::new(label).expect("scratch");
    let path = dir.join("subject.py");
    std::fs::write(&path, source).expect("write the subject");
    let output = pycc()
        .arg("check")
        .arg(&path)
        .output()
        .expect("pycc should spawn");
    assert!(!output.status.success(), "{source} was accepted");
    text(&output.stdout) + &text(&output.stderr)
}

#[test]
fn unsupported_list_shapes_are_refused_not_miscompiled() {
    for (source, message) in [
        (
            "class P:\n    def __init__(self, x: int) -> None:\n        self.x = x\n\n\nprint([P(1)])\n",
            "error[C0001]: rendering `list[P]` (a list of `P` instances) as a string",
        ),
        (
            "from dataclasses import dataclass\n\n\n@dataclass\nclass P:\n    x: int\n\n\nps = [P(1)]\nprint(P(1) in ps)\n",
            "error[C0001]: `in` over a list of `P`, which defines `__eq__`",
        ),
        (
            "xs = [1]\nys = xs + [1.5]\n",
            "error[T0021]: operator Add is not defined for `list[int]` and `list[float]`",
        ),
        (
            "xs = [None]\n",
            "error[T0034]: list[None] is not compiled yet",
        ),
        (
            "xs = [1]\nprint(round(1, 2))\n",
            "error[C0001]: `round()` of (int, int)",
        ),
    ] {
        let stderr = check_fails("generic_list_refusals", source);
        assert!(stderr.contains(message), "{source}: {stderr}");
    }
}

#[test]
fn a_float_list_benchmark_shape_matches_cpython() {
    let source = "def mul(v: list[float], n: int) -> list[float]:\n    out: list[float] = []\n    for i in range(n):\n        s = 0.0\n        for j in range(n):\n            s += v[j] / (i + j + 1)\n        out.append(s)\n    return out\n\n\nu = [1.0] * 5\nfor _ in range(3):\n    u = mul(u, 5)\nprint(round(u[0], 9), round(u[-1], 3), round(2.5), round(-1.5))\n";
    let dir = ScratchDir::new("generic_list_bench_shape").expect("scratch");
    let path = dir.join("subject.py");
    std::fs::write(&path, source).expect("write the subject");
    let out = dir.join("subject");
    let build = pycc()
        .arg("build")
        .arg(&path)
        .arg("-o")
        .arg(&out)
        .arg("--release")
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", text(&build.stderr));
    let ours = Command::new(&out).output().expect("the program should run");
    let theirs = python().arg(&path).output().expect("python3 should spawn");
    assert_same_as_cpython(&ours, &theirs);
}
