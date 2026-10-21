//! `while s[i] in chars: i += 1` (or `not in`) runs as one scan of the
//! text (`pycc_mir::str_scan`): where it stops, the index it leaves and the
//! `IndexError` of reading past either end -- a bigint index included --
//! are CPython's. The fixture runs natively (strict and non-strict) and as
//! an `--ext` module imported by CPython, against CPython running the
//! source; the leak probe repeats the scan, with a bigint index, from a
//! CPython host and checks that CPython's allocated blocks do not grow.

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
    assert!(expected.status.success(), "{}", text(&pycc_scratch::lf(&expected.stderr)));
    text(&pycc_scratch::lf(&expected.stdout))
}

fn native_build_matches_cpython(name: &str, flags: &[&str]) {
    let dir = ScratchDir::new(&format!("str_scan_native_{name}")).expect("scratch");
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
    assert!(build.status.success(), "{}", text(&pycc_scratch::lf(&build.stderr)));
    assert!(!text(&pycc_scratch::lf(&build.stderr)).contains("fallback:"), "{}", text(&pycc_scratch::lf(&build.stderr)));
    let run = Command::new(&binary).output().expect("the binary runs");
    assert!(run.status.success(), "{}", text(&pycc_scratch::lf(&run.stderr)));
    assert_eq!(text(&pycc_scratch::lf(&run.stdout)), oracle(name));
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
    assert!(build.status.success(), "{}", text(&pycc_scratch::lf(&build.stderr)));
    assert!(!text(&pycc_scratch::lf(&build.stderr)).contains("fallback:"), "{}", text(&pycc_scratch::lf(&build.stderr)));
}

fn ext_module_matches_cpython(name: &str) {
    let dir = ScratchDir::new(&format!("str_scan_ext_{name}")).expect("scratch");
    std::fs::create_dir_all(dir.join("src")).expect("source directory");
    let source = dir.join("src").join(format!("{name}.py"));
    std::fs::copy(fixture_named(name), &source).expect("copy the fixture");
    build_ext(&dir, name, &source);
    let run = Command::new(python())
        .args(["-c", &format!("import {name}")])
        .current_dir(&*dir)
        .output()
        .expect("python3 should spawn");
    assert!(run.status.success(), "{}", text(&pycc_scratch::lf(&run.stderr)));
    assert_eq!(text(&pycc_scratch::lf(&run.stdout)), oracle(name));
}

#[test]
fn strict_matches_cpython() {
    native_build_matches_cpython("str_scan_loop", &[]);
}

#[test]
fn non_strict_matches_cpython() {
    native_build_matches_cpython("str_scan_loop", &["--non-strict"]);
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn ext_matches_cpython() {
    ext_module_matches_cpython("str_scan_loop");
}

const PROBE: &str = r#"WS = frozenset(" \t")


def skip(src: str, pos: int) -> int:
    try:
        while src[pos] in WS:
            pos += 1
    except IndexError:
        pass
    return pos


def run(src: str, far: int) -> int:
    total = 0
    for start in range(-len(src), len(src) + 1):
        total += skip(src, start)
    return total + skip(src, far) - far
"#;

const DRIVER: &str = r#"import sys
import probe_scan as m
src = "  a \t b  " * 3
far = 10**30
assert m.run(src, far) == m.run(src, far)
before = sys.getallocatedblocks()
for _ in range(3000):
    r = m.run(src, far)
after = sys.getallocatedblocks()
assert after - before < 100, (before, after)
print(r)
"#;

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn scans_leak_nothing_from_a_cpython_host() {
    let dir = ScratchDir::new("str_scan_leaks").expect("scratch");
    std::fs::create_dir_all(dir.join("src")).expect("source directory");
    let source = dir.join("src").join("probe_scan.py");
    std::fs::write(&source, PROBE).expect("write the probe");
    build_ext(&dir, "probe_scan", &source);
    let run = Command::new(python())
        .args(["-c", DRIVER])
        .current_dir(&*dir)
        .output()
        .expect("python3 should spawn");
    assert!(run.status.success(), "{}{}", text(&pycc_scratch::lf(&run.stdout)), text(&pycc_scratch::lf(&run.stderr)));
    // The same driver against the source gives the same output.
    let plain = Command::new(python())
        .args(["-c", DRIVER])
        .current_dir(dir.join("src"))
        .output()
        .expect("python3 should spawn");
    assert!(plain.status.success(), "{}", text(&pycc_scratch::lf(&plain.stderr)));
    assert_eq!(text(&pycc_scratch::lf(&run.stdout)), text(&pycc_scratch::lf(&plain.stdout)));
}
