//! Non-strict mode keeps code compiled where containers meet CPython
//! objects: a name rebound from a list to a set becomes an object, a
//! comprehension over an object collects native lists into a native list,
//! `set(o)` and an empty `set()`/`dict()` of objects are CPython's own, and
//! the empty-container pass reads a dict filled through `.get(k, 0)` when
//! the call is also readable as a user method. The fixture runs with no
//! statement falling back to CPython and prints what CPython prints.

use pycc_scratch::ScratchDir;
use std::path::Path;
use std::process::Command;

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

#[test]
fn object_containers_stay_compiled_and_match_cpython() {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/non_strict_object_containers.py");
    let dir = ScratchDir::new("non_strict_object_containers").expect("scratch");
    let binary = dir.join("app");
    let build = Command::new(env!("CARGO_BIN_EXE_pycc"))
        .arg("build")
        .arg(&fixture)
        .arg("-o")
        .arg(&binary)
        .args(["--release", "--non-strict", "--report-fallbacks"])
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", text(&build.stderr));
    assert!(!text(&build.stderr).contains("fallback:"), "{}", text(&build.stderr));
    let run = Command::new(&binary).output().expect("the binary runs");
    let oracle = std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into());
    let expected = Command::new(oracle).arg(&fixture).output().expect("python3 should spawn");
    assert!(run.status.success(), "{}", text(&run.stderr));
    assert_eq!(text(&run.stdout), text(&expected.stdout));
}
