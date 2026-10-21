//! An object compared with a string literal (`o == "x"`, `"x" != o`) and
//! tested against a native `frozenset[str]` (`o in fs`) in a condition
//! (non-strict mode) matches CPython: an exact `str` is answered from its
//! UTF-8 bytes, `None` and every other object -- a `str` subclass, a
//! `UserString`, a `str` with a lone surrogate -- through CPython's own
//! operation. The fixture runs natively and as an `--ext` module imported
//! by CPython, against CPython running the source; the leak probe repeats
//! every path from a CPython host and checks that neither CPython's
//! allocated blocks nor the operands' reference counts grow.

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
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/object_text_compare.py")
}

fn oracle() -> String {
    let expected = Command::new(python()).arg(fixture()).output().expect("python3 should spawn");
    assert!(expected.status.success(), "{}", text(&pycc_scratch::lf(&expected.stderr)));
    text(&pycc_scratch::lf(&expected.stdout))
}

#[test]
fn native_non_strict_build_matches_cpython() {
    let dir = ScratchDir::new("object_text_compare_native").expect("scratch");
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
    let dir = ScratchDir::new("object_text_compare_ext").expect("scratch");
    std::fs::create_dir_all(dir.join("src")).expect("source directory");
    let source = dir.join("src").join("object_text_compare.py");
    std::fs::copy(fixture(), &source).expect("copy the fixture");
    build_ext(&dir, "object_text_compare", &source);
    let run = Command::new(python())
        .args(["-c", "import object_text_compare"])
        .current_dir(&*dir)
        .output()
        .expect("python3 should spawn");
    assert!(run.status.success(), "{}", text(&pycc_scratch::lf(&run.stderr)));
    assert_eq!(text(&pycc_scratch::lf(&run.stdout)), oracle());
}

const PROBE: &str = r#"from typing import Any

CHARS = frozenset(["a", "bc", "é"])


def run(values: Any) -> int:
    total = 0
    for o in values:
        if o == "a":
            total += 1
        if "bc" != o:
            total += 10
        if o == "é":
            total += 100
        if o in CHARS:
            total += 1000
        if o not in CHARS:
            total += 10000
    return total
"#;

const DRIVER: &str = r#"import collections
import sys
import probe_text as m
class Eq:
    def __eq__(self, other):
        return other == "a"
    def __hash__(self):
        return hash("a")
class Raises:
    def __eq__(self, other):
        raise ValueError("no")
    def __hash__(self):
        return 1
values = ["a", "bc", "é", "zz", chr(0xD800), None, 3,
          collections.UserString("bc"), Eq()]
def refs():
    return [sys.getrefcount(o) for o in values]
assert m.run(values) == m.run(values)
for bad in ([Raises()],):
    try:
        m.run(bad)
    except ValueError:
        pass
before_refs = refs()
before = sys.getallocatedblocks()
for _ in range(3000):
    r = m.run(values)
    try:
        m.run([Raises()])
    except ValueError:
        pass
after = sys.getallocatedblocks()
assert refs() == before_refs, (refs(), before_refs)
assert after - before < 100, (before, after)
print(r)
try:
    m.run([Raises()])
except ValueError as e:
    print("ValueError", e)
"#;

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn text_compares_leak_nothing_from_a_cpython_host() {
    let dir = ScratchDir::new("object_text_compare_leaks").expect("scratch");
    std::fs::create_dir_all(dir.join("src")).expect("source directory");
    let source = dir.join("src").join("probe_text.py");
    std::fs::write(&source, PROBE).expect("write the probe");
    build_ext(&dir, "probe_text", &source);
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
