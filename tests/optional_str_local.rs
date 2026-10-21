//! A `str | None` local bound only to native `str` values and `None`
//! (non-strict mode, `pycc_mir::optional_str`) stays native: its literal
//! comparisons, `frozenset[str]` membership and `None` tests read null as
//! `None`, and every other read boxes it back into the object it stands
//! for. The fixture runs natively and as an `--ext` module imported by
//! CPython, against CPython running the source; the leak probe repeats the
//! paths from a CPython host and checks that neither CPython's allocated
//! blocks nor the operands' reference counts grow.

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
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/optional_str_local.py")
}

fn oracle() -> String {
    let expected = Command::new(python()).arg(fixture()).output().expect("python3 should spawn");
    assert!(expected.status.success(), "{}", text(&pycc_scratch::lf(&expected.stderr)));
    text(&pycc_scratch::lf(&expected.stdout))
}

#[test]
fn native_non_strict_build_matches_cpython() {
    let dir = ScratchDir::new("optional_str_local_native").expect("scratch");
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
    let dir = ScratchDir::new("optional_str_local_ext").expect("scratch");
    std::fs::create_dir_all(dir.join("src")).expect("source directory");
    let source = dir.join("src").join("optional_str_local.py");
    std::fs::copy(fixture(), &source).expect("copy the fixture");
    build_ext(&dir, "optional_str_local", &source);
    let run = Command::new(python())
        .args(["-c", "import optional_str_local"])
        .current_dir(&*dir)
        .output()
        .expect("python3 should spawn");
    assert!(run.status.success(), "{}", text(&pycc_scratch::lf(&run.stderr)));
    assert_eq!(text(&pycc_scratch::lf(&run.stdout)), oracle());
}

const PROBE: &str = r#"from __future__ import annotations

from typing import Any

CHARS = frozenset(" ab")


def kind(src: str, pos: int, keep: Any) -> int:
    try:
        char: str | None = src[pos]
    except IndexError:
        char = None
    total = 0
    if char == "a":
        total += 1
    if char in CHARS:
        total += 10
    if char is None:
        total += 100
    keep.append(char)
    keep.pop()
    return total


def run(src: str) -> int:
    keep: list[Any] = []
    total = 0
    for pos in range(len(src) + 2):
        total += kind(src, pos, keep)
    return total
"#;

const DRIVER: &str = r#"import sys
import probe_optstr as m
src = "ab c\u00e9" * 4
def refs():
    return [sys.getrefcount(o) for o in (src, None)]
assert m.run(src) == m.run(src)
before_refs = refs()
before = sys.getallocatedblocks()
for _ in range(3000):
    r = m.run(src)
after = sys.getallocatedblocks()
assert refs()[0] == before_refs[0], (refs(), before_refs)
assert after - before < 100, (before, after)
print(r)
"#;

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn native_str_locals_leak_nothing_from_a_cpython_host() {
    let dir = ScratchDir::new("optional_str_local_leaks").expect("scratch");
    std::fs::create_dir_all(dir.join("src")).expect("source directory");
    let source = dir.join("src").join("probe_optstr.py");
    std::fs::write(&source, PROBE).expect("write the probe");
    build_ext(&dir, "probe_optstr", &source);
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
