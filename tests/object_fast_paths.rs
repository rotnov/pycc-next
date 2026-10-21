//! Object operations compiled code takes a fast path for in non-strict mode
//! match CPython: `o[i]` and `o[a:b]` with native `int` keys (an exact
//! `tuple`/`list` read directly, anything else through CPython), tuple
//! displays and unpacking of objects, method calls whose arguments are
//! inert (one `PyObject_VectorcallMethod`, no bound method), a rich
//! comparison in an `if`/`while` (no result object), and `s[i] in chars`
//! for a set-of-`str` parameter. The fixture runs natively and as an
//! `--ext` module imported by CPython, against CPython running the source;
//! the leak probe repeats every path from a CPython host and checks that
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

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/object_fast_paths.py")
}

fn oracle() -> String {
    let expected = Command::new(python()).arg(fixture()).output().expect("python3 should spawn");
    assert!(expected.status.success(), "{}", text(&pycc_scratch::lf(&expected.stderr)));
    text(&pycc_scratch::lf(&expected.stdout))
}

#[test]
fn native_non_strict_build_matches_cpython() {
    let dir = ScratchDir::new("object_fast_paths_native").expect("scratch");
    let binary = dir.join("app");
    let build = Command::new(env!("CARGO_BIN_EXE_pycc"))
        .arg("build")
        .arg(fixture())
        .arg("-o")
        .arg(&binary)
        .args(["--release", "--non-strict", "--report-fallbacks"])
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", text(&pycc_scratch::lf(&build.stderr)));
    assert!(!text(&pycc_scratch::lf(&build.stderr)).contains("fallback:"), "{}", text(&pycc_scratch::lf(&build.stderr)));
    let run = Command::new(&binary).output().expect("the binary runs");
    assert!(run.status.success(), "{}", text(&pycc_scratch::lf(&run.stderr)));
    assert_eq!(text(&pycc_scratch::lf(&run.stdout)), oracle());
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

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn ext_module_matches_cpython() {
    let dir = ScratchDir::new("object_fast_paths_ext").expect("scratch");
    std::fs::create_dir_all(dir.join("src")).expect("source directory");
    let source = dir.join("src").join("object_fast_paths.py");
    std::fs::copy(fixture(), &source).expect("copy the fixture");
    build_ext(&dir, "object_fast_paths", &source);
    let run = Command::new(python())
        .args(["-c", "import object_fast_paths"])
        .current_dir(&*dir)
        .output()
        .expect("python3 should spawn");
    assert!(run.status.success(), "{}", text(&pycc_scratch::lf(&run.stderr)));
    assert_eq!(text(&pycc_scratch::lf(&run.stdout)), oracle());
}

const PROBE: &str = r#"from typing import Any


def run(t: Any, items: Any, d: Any, s: Any, n: int, chars: frozenset[str]) -> int:
    total = 0
    for i in range(-4, 4):
        try:
            if t[i] == "two":
                total += 1
        except IndexError:
            total += 10
    a, b, c = t
    pair = (a, items)
    x, y = pair
    if x == 1:
        total += 1
    if t[1:] == t[:-1]:
        total += 100
    if items[0:n] is items:
        total += 100
    try:
        t[10**30]
    except IndexError:
        total += 1
    if d.get(2) != "two":
        total += 100
    if s.upper() < s:
        total += 1
    try:
        d.missing(1)
    except AttributeError:
        total += 1
    try:
        if None < s:
            total += 100
    except TypeError:
        total += 1
    if isinstance(items, list) and not isinstance(d, (list, tuple)):
        total += 1
    return total + scan(s, 0, chars) + scan(s, 7, chars)


def scan(src: str, pos: int, chars: frozenset[str]) -> int:
    try:
        while src[pos] in chars:
            pos += 1
    except IndexError:
        pass
    return pos
"#;

const DRIVER: &str = r#"import sys
import probe_fast_paths as m
t = (1, "two", 3.0)
items = [4, 5, 6]
d = {2: "two"}
s = "  ab"
chars = frozenset(" ab")
def refs():
    return [sys.getrefcount(o) for o in (t, items, d, s, chars)]
assert m.run(t, items, d, s, 3, chars) == m.run(t, items, d, s, 3, chars)
before_refs = refs()
before = sys.getallocatedblocks()
for _ in range(3000):
    r = m.run(t, items, d, s, 3, chars)
after = sys.getallocatedblocks()
assert refs() == before_refs, (refs(), before_refs)
assert after - before < 100, (before, after)
print(r)
"#;

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn fast_paths_leak_nothing_from_a_cpython_host() {
    let dir = ScratchDir::new("object_fast_paths_leaks").expect("scratch");
    std::fs::create_dir_all(dir.join("src")).expect("source directory");
    let source = dir.join("src").join("probe_fast_paths.py");
    std::fs::write(&source, PROBE).expect("write the probe");
    build_ext(&dir, "probe_fast_paths", &source);
    let run = Command::new(python())
        .args(["-c", DRIVER])
        .current_dir(&*dir)
        .output()
        .expect("python3 should spawn");
    assert!(run.status.success(), "{}{}", text(&pycc_scratch::lf(&run.stdout)), text(&pycc_scratch::lf(&run.stderr)));
    // The same driver against the source gives the same total.
    let plain = Command::new(python())
        .args(["-c", DRIVER])
        .current_dir(dir.join("src"))
        .output()
        .expect("python3 should spawn");
    assert!(plain.status.success(), "{}", text(&pycc_scratch::lf(&plain.stderr)));
    assert_eq!(text(&pycc_scratch::lf(&run.stdout)), text(&pycc_scratch::lf(&plain.stdout)));
}
