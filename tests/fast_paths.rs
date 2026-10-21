//! Oracle test for the inline fast paths (`tests/fixtures/fast_paths.py`):
//! instance slots read and written in place, float `/` and `**` with their
//! exceptions, the exact `x ** 0.5` shortcut on inputs where glibc's `pow`
//! and `sqrt` disagree, the smallint version of `for ... in range(...)` next
//! to the generic loop it falls back to for bigint bounds, and every
//! operation on a byte-stored `list[bool]`. The compiled
//! program must print exactly what CPython prints, in both build profiles.

use pycc_scratch::ScratchDir;
use std::path::Path;
use std::process::{Command, Output};

fn stdout(output: &Output) -> String {
    pycc_scratch::lf(&*String::from_utf8_lossy(&output.stdout)).replace("\r\n", "\n")
}

fn rendered(output: &Output) -> String {
    format!(
        "status: {}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        stdout(output),
        pycc_scratch::lf(&*String::from_utf8_lossy(&output.stderr))
    )
}

fn check(release: bool) {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fast_paths.py");
    let dir = ScratchDir::new(&format!("fast_paths_{release}")).expect("scratch");
    let binary = dir.join("prog");
    let mut command = Command::new(env!("CARGO_BIN_EXE_pycc"));
    command.arg("build").arg(&source).arg("-o").arg(&binary);
    if release {
        command.arg("--release");
    }
    let build = command.output().expect("pycc should spawn");
    assert!(build.status.success(), "{}", rendered(&build));
    let compiled = Command::new(&binary).output().expect("the program should spawn");
    let oracle = Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
        .arg(&source)
        .output()
        .expect("python3 should spawn");
    assert!(oracle.status.success(), "{}", rendered(&oracle));
    assert!(compiled.status.success(), "{}", rendered(&compiled));
    assert_eq!(stdout(&compiled), stdout(&oracle));
}

#[test]
fn fast_paths_match_cpython_in_release() {
    check(true);
}

#[test]
fn fast_paths_match_cpython_in_debug() {
    check(false);
}
