//! Part 1 of #1227 (#1271): `pycc build --static-libpython`, or `[build]
//! static = true` in a neighboring `pycc.toml`, links the embed
//! interpreter's static libpython into the executable instead of bundling
//! its shared library (the static-libpython decision entry under
//! `docs/decisions/`, D-251; `docs/CLI_SPEC.md`'s `pycc build` options).
//!
//! No test needs a Python: a `sh` script answers the embed probe for a
//! static-only interpreter (`Py_ENABLE_SHARED=0`, no shared library on
//! disk) and the static probe (`# pycc-static-probe`) with a `LIBPL` the
//! test controls. A build that gets past every check fails in the compiler
//! on the fake prefix's one-line `Python.h`, after the sidecar is written.
//! `cfg(not(windows))`: a static libpython is not available on a Windows host
//! (D-251; D-253 refuses it before the probe).

#![cfg_attr(windows, allow(dead_code, unused_imports))]

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

/// A static-only fake interpreter under `root`: its script, and where the
/// static probe says the archive is (not written yet).
#[cfg(unix)]
fn fake_python(root: &Path) -> (PathBuf, PathBuf) {
    use std::os::unix::fs::PermissionsExt;
    let prefix = root.join("python");
    let include = prefix.join("include").join("python3.14");
    let lib = prefix.join("lib");
    let stdlib = lib.join("python3.14");
    let config = stdlib.join("config-3.14");
    write(&include.join("Python.h"), b"#error not a real Python.h\n");
    write(&stdlib.join("os.py"), b"# os\n");
    write(&stdlib.join("json").join("__init__.py"), b"# json\n");
    let embed = format!(
        "3.14.7\n{}\n{}\n{}\n{}\n0\n\nlibpython3.14.a\n{}\n\n0\n",
        prefix.join("bin").join("python3.14").display(),
        include.display(),
        stdlib.display(),
        prefix.display(),
        lib.display()
    );
    let script = root.join("fake-python");
    let body = format!(
        "#!/bin/sh\ncase \"$3\" in\n*pycc-static-probe*) printf '%s\\n' '{}' libpython3.14.a \
         '-ldl' '' ;;\n*) cat <<'PYCC'\n{embed}PYCC\n;;\nesac\n",
        config.display()
    );
    std::fs::write(&script, body).expect("write the fake interpreter");
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    (script, config.join("libpython3.14.a"))
}

fn pycc_in(dir: &Path, python: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
        .args(args)
        .current_dir(dir)
        .env("PYCC_PYTHON", python)
        .output()
        .expect("pycc should spawn")
}

/// A scratch project whose entry imports the standard library, so the
/// build embeds an interpreter; with `manifest`, a `pycc.toml` beside it
/// sets `[build] static = true`.
#[cfg(unix)]
fn project(tag: &str, manifest: bool) -> (ScratchDir, PathBuf, PathBuf, PathBuf) {
    let dir = ScratchDir::new(tag).expect("scratch");
    let root = std::fs::canonicalize(&*dir).expect("canonicalize");
    std::fs::write(
        root.join("m.py"),
        "import json\nprint(str(json.dumps(1)))\n",
    )
    .expect("write");
    if manifest {
        std::fs::write(
            root.join("pycc.toml"),
            "[project]\nname = \"m\"\nentry = \"m.py\"\npython = \"3.14\"\n\n\
             [build]\nstatic = true\n",
        )
        .expect("write the manifest");
    }
    let (python, archive) = fake_python(&root);
    (dir, root, python, archive)
}

/// The flag and the manifest key each select the static link: with no
/// archive at `LIBPL`, both are refused at exit 2 naming the request, and
/// nothing is written.
#[cfg(unix)]
#[test]
fn the_flag_and_the_manifest_key_both_require_the_archive() {
    for (tag, manifest, args) in [
        (
            "static_flag",
            false,
            &["build", "m.py", "-o", "app", "--static-libpython"][..],
        ),
        ("static_manifest", true, &["build", "m.py", "-o", "app"][..]),
    ] {
        let (_dir, root, python, archive) = project(tag, manifest);
        let output = pycc_in(&root, &python, args);
        let rendered = stderr_of(&output);
        assert_eq!(output.status.code(), Some(2), "{tag}: {rendered}");
        assert!(
            rendered.contains("a static libpython was requested (`--static-libpython` or"),
            "{tag}: {rendered}"
        );
        assert!(
            rendered.contains(&format!("`{}` does not exist", archive.display())),
            "{tag}: {rendered}"
        );
        assert!(!root.join("app.pycc").exists(), "{tag}");
        assert!(!root.join("app").exists(), "{tag}");
    }
}

/// Without the request, the same static-only interpreter is refused by the
/// shared path, and `pycc run` ignores the manifest key.
#[cfg(unix)]
#[test]
fn without_the_request_and_under_run_the_shared_path_applies() {
    let (_dir, root, python, _archive) = project("static_run", true);
    let output = pycc_in(&root, &python, &["run", "m.py"]);
    let rendered = stderr_of(&output);
    assert_eq!(output.status.code(), Some(2), "{rendered}");
    assert!(rendered.contains("no shared libpython"), "{rendered}");
    std::fs::remove_file(root.join("pycc.toml")).expect("remove the manifest");
    let output = pycc_in(&root, &python, &["build", "m.py", "-o", "app"]);
    let rendered = stderr_of(&output);
    assert_eq!(output.status.code(), Some(2), "{rendered}");
    assert!(rendered.contains("no shared libpython"), "{rendered}");
}

/// A valid archive gets the build past every check: the sidecar holds no
/// libpython, and its marker records the archive's digest and the link
/// mode. The compile then fails on the stub `Python.h` (exit 1).
#[cfg(unix)]
#[test]
fn a_valid_archive_writes_a_sidecar_without_libpython() {
    let (_dir, root, python, archive) = project("static_valid", false);
    write(&archive, b"!<arch>\nnot real members");
    let output = pycc_in(
        &root,
        &python,
        &["build", "m.py", "-o", "app", "--static-libpython"],
    );
    let rendered = stderr_of(&output);
    assert_eq!(output.status.code(), Some(1), "{rendered}");
    let sidecar = root.join("app.pycc");
    let marker = std::fs::read_to_string(sidecar.join("PYCC-BUNDLE")).expect("the marker");
    assert!(marker.ends_with("libpython-link static\n"), "{marker}");
    let libs: Vec<String> = std::fs::read_dir(sidecar.join("lib"))
        .expect("lib")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    assert_eq!(libs, ["python3.14"]);
}

/// The flag is a no-op for a build that embeds no interpreter, and `--ext`
/// rejects it as a usage error.
#[cfg(unix)]
#[test]
fn a_native_build_ignores_the_flag_and_ext_rejects_it() {
    let (_dir, root, python, _archive) = project("static_native", false);
    std::fs::write(root.join("n.py"), "print(1)\n").expect("write");
    let output = pycc_in(
        &root,
        &python,
        &["build", "n.py", "-o", "native", "--static-libpython"],
    );
    assert_eq!(output.status.code(), Some(0), "{}", stderr_of(&output));
    assert!(root.join("native").is_file());
    assert!(!root.join("native.pycc").exists());
    let output = pycc_in(
        &root,
        &python,
        &["build", "n.py", "-o", "n.so", "--ext", "--static-libpython"],
    );
    let rendered = stderr_of(&output);
    assert_eq!(output.status.code(), Some(2), "{rendered}");
    assert!(rendered.contains("cannot be used with"), "{rendered}");
}
