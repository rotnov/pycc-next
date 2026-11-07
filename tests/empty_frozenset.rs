//! An empty `frozenset()` is typed by its uses (an equality, a set
//! operator or a membership test with a typed frozenset), as an empty
//! `set()` is, rather than always being a `frozenset[int]`: so
//! `frozenset("") == frozenset()` is `True`, as in CPython. Strict and
//! non-strict builds both match CPython's output.

use pycc_scratch::ScratchDir;
use std::path::Path;
use std::process::Command;

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

fn check(label: &str, flags: &[&str]) {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/empty_frozenset.py");
    let dir = ScratchDir::new(label).expect("scratch");
    let binary = dir.join("app");
    let build = Command::new(env!("CARGO_BIN_EXE_pycc"))
        .arg("build")
        .arg(&fixture)
        .arg("-o")
        .arg(&binary)
        .arg("--release")
        .args(flags)
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", text(&pycc_scratch::lf(&build.stderr)));
    assert!(!text(&pycc_scratch::lf(&build.stderr)).contains("fallback:"), "{}", text(&pycc_scratch::lf(&build.stderr)));
    let run = Command::new(&binary).output().expect("the binary runs");
    let oracle = std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into());
    let expected = Command::new(oracle).arg(&fixture).output().expect("python3 should spawn");
    assert!(run.status.success(), "{}", text(&pycc_scratch::lf(&run.stderr)));
    assert_eq!(text(&pycc_scratch::lf(&run.stdout)), text(&pycc_scratch::lf(&expected.stdout)));
}

#[test]
fn an_empty_frozenset_is_typed_by_its_uses() {
    check("empty_frozenset_strict", &[]);
}

#[test]
fn an_empty_frozenset_is_typed_by_its_uses_in_non_strict_mode() {
    check("empty_frozenset_non_strict", &["--non-strict", "--report-fallbacks"]);
}
