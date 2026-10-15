//! `a, b = x, y` with CPython-object values (non-strict mode) binds its
//! targets from the elements without a tuple, and matches CPython: every
//! element is evaluated before any target is bound, so a raising element
//! leaves every target as it was. The fixture runs natively and as an
//! `--ext` module imported by CPython, against CPython running the source;
//! the leak probe repeats the paths from a CPython host and checks that
//! neither CPython's allocated blocks nor the operands' reference counts
//! grow, and that no reference outlives the statement.

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
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/object_swap_unpack.py")
}

fn oracle() -> String {
    let expected = Command::new(python()).arg(fixture()).output().expect("python3 should spawn");
    assert!(expected.status.success(), "{}", text(&expected.stderr));
    text(&expected.stdout)
}

#[test]
fn native_non_strict_build_matches_cpython() {
    let dir = ScratchDir::new("object_swap_unpack_native").expect("scratch");
    let binary = dir.join("app");
    let build = Command::new(env!("CARGO_BIN_EXE_pycc"))
        .arg("build")
        .arg(fixture())
        .arg("-o")
        .arg(&binary)
        .args(["--release", "--non-strict", "--report-fallbacks"])
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", text(&build.stderr));
    assert!(!text(&build.stderr).contains("fallback:"), "{}", text(&build.stderr));
    let run = Command::new(&binary).output().expect("the binary runs");
    assert!(run.status.success(), "{}", text(&run.stderr));
    assert_eq!(text(&run.stdout), oracle());
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

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn ext_module_matches_cpython() {
    let dir = ScratchDir::new("object_swap_unpack_ext").expect("scratch");
    std::fs::create_dir_all(dir.join("src")).expect("source directory");
    let source = dir.join("src").join("object_swap_unpack.py");
    std::fs::copy(fixture(), &source).expect("copy the fixture");
    build_ext(&dir, "object_swap_unpack", &source);
    let run = Command::new(python())
        .args(["-c", "import object_swap_unpack"])
        .current_dir(&*dir)
        .output()
        .expect("python3 should spawn");
    assert!(run.status.success(), "{}", text(&run.stderr));
    assert_eq!(text(&run.stdout), oracle());
}

const PROBE: &str = r#"from typing import Any


def run(key: Any, a: Any, b: Any) -> int:
    total = 0
    for _ in range(3):
        parent, stem = key[:-1], key[-1]
        a, b = b, a
        total += len(parent) + len(stem)
    try:
        x, y = key[0], key[99]
    except IndexError:
        total += 1
    return total + len(a)
"#;

const DRIVER: &str = r#"import gc
import sys
import weakref
import probe_swap as m
class Box:
    def __len__(self):
        return 1
key = ("p", "q", "stem")
a = "aa"
b = "bbb"
def refs():
    return [sys.getrefcount(o) for o in (key, a, b)]
assert m.run(key, a, b) == m.run(key, a, b)
before_refs = refs()
before = sys.getallocatedblocks()
for _ in range(3000):
    r = m.run(key, a, b)
after = sys.getallocatedblocks()
assert refs() == before_refs, (refs(), before_refs)
assert after - before < 100, (before, after)
# Nothing keeps a swapped value alive once `run` returns.
box = Box()
seen = weakref.ref(box)
m.run(("x", box), "z", "zz")
del box
gc.collect()
print(r, seen() is None)
"#;

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn swaps_leak_nothing_from_a_cpython_host() {
    let dir = ScratchDir::new("object_swap_unpack_leaks").expect("scratch");
    std::fs::create_dir_all(dir.join("src")).expect("source directory");
    let source = dir.join("src").join("probe_swap.py");
    std::fs::write(&source, PROBE).expect("write the probe");
    build_ext(&dir, "probe_swap", &source);
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
