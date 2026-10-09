//! Part 2 of #1227 (#1272): `pycc lock` accepts an interpreter with no
//! shared libpython and records the digest of its `LIBPL` archive as
//! `libpython-sha256`, and `pycc build --static-libpython` consumes that
//! section (D-249's and D-251's 2026-09-24 amendments under
//! `docs/decisions/`; `docs/CLI_SPEC.md`'s `pycc lock` and `pycc build`).
//!
//! No test needs a Python: a `sh` script answers the lock probe
//! (`# pycc-lock-probe`), the static probe (`# pycc-static-probe`) and the
//! embed probe for a static-only interpreter (`Py_ENABLE_SHARED=0`, no
//! shared library on disk). The archive is a fixture, not a real static
//! libpython: a build that gets past the lock check fails in the compiler
//! on the fake prefix's one-line `Python.h` (exit 1). A real archive's
//! end-to-end evidence belongs to #1273. `cfg(not(windows))`: the fixture
//! interpreter is a `sh` script, and a static libpython is not available
//! on a Windows host (D-251); `pycc lock` on Windows is
//! `tests/issue_1296_windows_locked_closure.rs` (#1296).

#![cfg_attr(windows, allow(dead_code, unused_imports))]

#[path = "../src/embed/sha256.rs"]
mod sha256;

use pycc_scratch::ScratchDir;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).replace("\r\n", "\n")
}

fn write(path: &Path, bytes: &[u8]) {
    std::fs::create_dir_all(path.parent().expect("a parent")).expect("create the parent");
    std::fs::write(path, bytes).expect("write a fixture file");
}

/// A static-only fake interpreter under `root`: its script, and the
/// fixture archive at its `LIBPL`.
#[cfg(unix)]
fn fake_python(root: &Path) -> (PathBuf, PathBuf) {
    use std::os::unix::fs::PermissionsExt;
    let prefix = root.join("python");
    let include = prefix.join("include").join("python3.14");
    let lib = prefix.join("lib");
    let stdlib = lib.join("python3.14");
    let config = stdlib.join("config-3.14");
    let site = root.join("site-packages");
    std::fs::create_dir_all(&site).expect("create site-packages");
    write(&include.join("Python.h"), b"#error not a real Python.h\n");
    write(&stdlib.join("os.py"), b"# os\n");
    write(&stdlib.join("json").join("__init__.py"), b"# json\n");
    let archive = config.join("libpython3.14.a");
    write(&archive, b"!<arch>\nfixture members, first version");
    let embed = format!(
        "3.14.7\n{}\n{}\n{}\n{}\n0\n\nlibpython3.14.a\n{}\n\n0\n",
        prefix.join("bin").join("python3.14").display(),
        include.display(),
        stdlib.display(),
        prefix.display(),
        lib.display()
    );
    let lock = format!(
        "pycc-lock-probe 1\n{0}\n{0}\ncpython-314\nmacosx-11.0-arm64\ncpython\n3.14.7\nposix\n\
         arm64\nCPython\n25.0.0\nDarwin\nDarwin Kernel Version 25.0.0\n3.14.7\n3.14\ndarwin\n",
        site.display()
    );
    let script = root.join("fake-python");
    let body = format!(
        "#!/bin/sh\ncase \"$3\" in\n*pycc-lock-probe*) cat <<'PYCC'\n{lock}PYCC\n;;\n\
         *pycc-static-probe*) printf '%s\\n' '{}' libpython3.14.a '-ldl' '' ;;\n\
         *) cat <<'PYCC'\n{embed}PYCC\n;;\nesac\n",
        config.display()
    );
    std::fs::write(&script, body).expect("write the fake interpreter");
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    (script, archive)
}

fn pycc_in(dir: &Path, python: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
        .args(args)
        .current_dir(dir)
        .env("PYCC_PYTHON", python)
        .output()
        .expect("pycc should spawn")
}

fn locked_digest(dir: &Path) -> String {
    let text = std::fs::read_to_string(dir.join("pycc.lock")).expect("read pycc.lock");
    let line = text
        .lines()
        .find_map(|line| line.strip_prefix("libpython-sha256 = \""))
        .unwrap_or_else(|| panic!("no libpython-sha256 in {text}"));
    line.trim_end_matches('"').to_string()
}

/// `pycc lock` locks a static-only interpreter by its archive's digest,
/// `--check` accepts it, and a static build consumes the section; once the
/// archive changes, `--check` fails and the build is refused as stale
/// before anything is written.
#[cfg(unix)]
#[test]
fn a_static_only_interpreter_is_locked_by_its_archive_and_the_build_consumes_it() {
    let dir = ScratchDir::new("static_lock").expect("scratch");
    let root = std::fs::canonicalize(&*dir).expect("canonicalize");
    let (python, archive) = fake_python(&root);
    std::fs::write(
        root.join("m.py"),
        "import json\nprint(str(json.dumps(1)))\n",
    )
    .expect("write");

    let output = pycc_in(&root, &python, &["lock", "m.py"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr_of(&output));
    let bytes = std::fs::read(&archive).expect("read the archive");
    assert_eq!(locked_digest(&root), sha256::sha256_hex(&bytes));
    let output = pycc_in(&root, &python, &["lock", "--check", "m.py"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr_of(&output));

    let build = ["build", "m.py", "-o", "app", "--static-libpython"];
    let output = pycc_in(&root, &python, &build);
    let rendered = stderr_of(&output);
    assert_eq!(output.status.code(), Some(1), "{rendered}");
    assert!(!rendered.contains("pycc.lock"), "{rendered}");
    let marker =
        std::fs::read_to_string(root.join("app.pycc").join("PYCC-BUNDLE")).expect("the marker");
    assert!(marker.ends_with("libpython-link static\n"), "{marker}");
    std::fs::remove_dir_all(root.join("app.pycc")).expect("remove the sidecar");

    write(&archive, b"!<arch>\nfixture members, second version");
    let output = pycc_in(&root, &python, &["lock", "--check", "m.py"]);
    assert_ne!(output.status.code(), Some(0), "{}", stderr_of(&output));
    let output = pycc_in(&root, &python, &build);
    let rendered = stderr_of(&output);
    assert_eq!(output.status.code(), Some(2), "{rendered}");
    assert!(rendered.contains("does not match this build"), "{rendered}");
    assert!(rendered.contains("libpython-sha256"), "{rendered}");
    assert!(rendered.contains("run `pycc lock m.py`"), "{rendered}");
    assert!(!root.join("app.pycc").exists());
    assert!(!root.join("app").exists());
}

/// With no archive at `LIBPL` either, `pycc lock` refuses the interpreter
/// in the lock's own words and writes no lock.
#[cfg(unix)]
#[test]
fn a_static_only_interpreter_without_its_archive_is_refused_by_lock() {
    let dir = ScratchDir::new("static_lock_none").expect("scratch");
    let root = std::fs::canonicalize(&*dir).expect("canonicalize");
    let (python, archive) = fake_python(&root);
    std::fs::remove_file(&archive).expect("remove the archive");
    std::fs::write(root.join("m.py"), "import json\n").expect("write");
    let output = pycc_in(&root, &python, &["lock", "m.py"]);
    let rendered = stderr_of(&output);
    assert_eq!(output.status.code(), Some(2), "{rendered}");
    assert!(rendered.contains("has no shared libpython"), "{rendered}");
    assert!(
        rendered.contains("identifies it by the digest of its `LIBPL` archive"),
        "{rendered}"
    );
    assert!(!root.join("pycc.lock").exists());
}
