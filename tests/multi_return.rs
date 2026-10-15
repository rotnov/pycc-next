//! A function whose every `return` is a tuple display (or forwards another
//! such function) and whose result is unpacked by its caller returns its
//! values without a tuple, through a multi-value twin
//! (`pycc_mir::multi_return`). The fixtures run natively (strict where
//! they can, and non-strict) and as `--ext` modules imported by CPython,
//! against CPython running the source; the leak probe repeats the paths --
//! a raising result included -- from a CPython host and checks that
//! neither CPython's allocated blocks nor the operands' reference counts
//! grow.

use pycc_scratch::ScratchDir;
use std::path::{Path, PathBuf};
use std::process::Command;

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

fn python() -> std::ffi::OsString {
    std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into())
}

fn fixture_named(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/fixtures/{name}.py"))
}

fn oracle(name: &str) -> String {
    let expected = Command::new(python()).arg(fixture_named(name)).output().expect("python3 should spawn");
    assert!(expected.status.success(), "{}", text(&expected.stderr));
    text(&expected.stdout)
}

fn native_build_matches_cpython(name: &str, flags: &[&str]) {
    let dir = ScratchDir::new(&format!("multi_return_native_{name}")).expect("scratch");
    let binary = dir.join("app");
    let build = Command::new(env!("CARGO_BIN_EXE_pycc"))
        .arg("build")
        .arg(fixture_named(name))
        .arg("-o")
        .arg(&binary)
        .args(["--release", "--report-fallbacks"])
        .args(flags)
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", text(&build.stderr));
    assert!(!text(&build.stderr).contains("fallback:"), "{}", text(&build.stderr));
    let run = Command::new(&binary).output().expect("the binary runs");
    assert!(run.status.success(), "{}", text(&run.stderr));
    assert_eq!(text(&run.stdout), oracle(name));
}

#[test]
fn object_tuples_non_strict_match_cpython() {
    native_build_matches_cpython("multi_return_object", &["--non-strict"]);
}

#[test]
fn native_tuples_strict_match_cpython() {
    native_build_matches_cpython("multi_return_native", &[]);
}

#[test]
fn native_tuples_non_strict_match_cpython() {
    native_build_matches_cpython("multi_return_native", &["--non-strict"]);
}

fn build_ext(dir: &Path, module: &str, source: &Path) {
    let build = Command::new(env!("CARGO_BIN_EXE_pycc"))
        .arg("build")
        .arg(source)
        .arg("-o")
        .arg(dir.join(module))
        .args(["--ext", "--non-strict", "--release", "--report-fallbacks"])
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", text(&build.stderr));
    assert!(!text(&build.stderr).contains("fallback:"), "{}", text(&build.stderr));
}

fn ext_module_matches_cpython(name: &str) {
    let dir = ScratchDir::new(&format!("multi_return_ext_{name}")).expect("scratch");
    std::fs::create_dir_all(dir.join("src")).expect("source directory");
    let source = dir.join("src").join(format!("{name}.py"));
    std::fs::copy(fixture_named(name), &source).expect("copy the fixture");
    build_ext(&dir, name, &source);
    let run = Command::new(python())
        .args(["-c", &format!("import {name}")])
        .current_dir(&*dir)
        .output()
        .expect("python3 should spawn");
    assert!(run.status.success(), "{}", text(&run.stderr));
    assert_eq!(text(&run.stdout), oracle(name));
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn object_tuples_ext_match_cpython() {
    ext_module_matches_cpython("multi_return_object");
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn native_tuples_ext_match_cpython() {
    ext_module_matches_cpython("multi_return_native");
}

const PROBE: &str = r#"from typing import Any


def leaf(src: str, pos: int, d: Any) -> tuple[int, Any]:
    if pos >= len(src):
        return pos, None
    if src[pos] == "k":
        return pos + 1, d[src[pos]]
    return pos + 1, src[pos]


def mid(src: str, pos: int, d: Any) -> tuple[int, Any]:
    if src.startswith("x", pos):
        return leaf(src, pos + 1, d)
    return leaf(src, pos, d)


def text(src: str, pos: int) -> tuple[int, str]:
    return pos + 1, src[pos] * 2


def run(src: str, d: Any) -> int:
    pos = 0
    total = 0
    while pos < len(src):
        pos, value = mid(src, pos, d)
        if value is not None:
            total += 1
        at, doubled = text(src, pos - 1)
        total += len(doubled) + at - pos
    return total
"#;

const DRIVER: &str = r#"import sys
import probe_multi as m
src = "abxkkz" * 3
d = {"k": object()}
def refs():
    return [sys.getrefcount(o) for o in (src, d, d["k"])]
assert m.run(src, d) == m.run(src, d)
before_refs = refs()
before = sys.getallocatedblocks()
for _ in range(3000):
    r = m.run(src, d)
    try:
        m.run("ak", {})
    except KeyError:
        pass
after = sys.getallocatedblocks()
assert refs() == before_refs, (refs(), before_refs)
assert after - before < 100, (before, after)
print(r)
try:
    m.run("ak", {})
except KeyError as e:
    print("KeyError", e)
"#;

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn twins_leak_nothing_from_a_cpython_host() {
    let dir = ScratchDir::new("multi_return_leaks").expect("scratch");
    std::fs::create_dir_all(dir.join("src")).expect("source directory");
    let source = dir.join("src").join("probe_multi.py");
    std::fs::write(&source, PROBE).expect("write the probe");
    build_ext(&dir, "probe_multi", &source);
    let run = Command::new(python())
        .args(["-c", DRIVER])
        .current_dir(&*dir)
        .output()
        .expect("python3 should spawn");
    assert!(run.status.success(), "{}{}", text(&run.stdout), text(&run.stderr));
    // The same driver against the source gives the same output.
    let plain = Command::new(python())
        .args(["-c", DRIVER])
        .current_dir(dir.join("src"))
        .output()
        .expect("python3 should spawn");
    assert!(plain.status.success(), "{}", text(&plain.stderr));
    assert_eq!(text(&run.stdout), text(&plain.stdout));
}
