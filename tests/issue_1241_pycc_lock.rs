//! Part 1 of #1225 (#1241): `pycc lock PATH [--check]` records the CPython
//! dependency closure an embedded build will carry into `pycc.lock` (the
//! pycc.lock decision entry, D-249; `docs/CLI_SPEC.md`'s `pycc.lock`
//! section).
//!
//! The non-ignored tests need no Python: `PYCC_PYTHON` names a `sh` script
//! that answers the embed probe and the lock probe with canned lines, over
//! a fake interpreter prefix and a fake `site-packages` the test writes.
//! Every distribution is written by the test itself, METADATA and RECORD
//! with hashes computed here, so nothing downloads anything and no test
//! calls `pip`, `uv` or an index. They are `cfg(not(windows))` one by one
//! because their fixture interpreter is a `sh` script; `pycc lock` on a
//! Windows host is `tests/issue_1296_windows_locked_closure.rs` (#1296).
//!
//! The `#[ignore]`d test locks a real `python3.14 -m venv --without-pip`
//! environment (`PYCC_PYTHON`, default `python3.14`, must be CPython
//! 3.14.7 with a shared libpython); `venv --without-pip` is offline.

#![cfg_attr(windows, allow(dead_code, unused_imports))]

use pycc_scratch::ScratchDir;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// pycc's own SHA-256, shared so the fixtures' RECORD hashes need no
/// hashing crate.
#[allow(dead_code)]
#[path = "../src/embed/sha256.rs"]
mod sha256;

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// A fake interpreter: `script` answers both probes, `site` is its
/// purelib and platlib, and every run appends a line to `sentinel`.
struct FakePython {
    script: PathBuf,
    site: PathBuf,
    sentinel: PathBuf,
}

impl FakePython {
    fn ran(&self) -> bool {
        self.sentinel.exists()
    }
}

fn write(path: &Path, bytes: &[u8]) {
    std::fs::create_dir_all(path.parent().expect("a parent")).expect("create the parent");
    std::fs::write(path, bytes).expect("write a fixture file");
}

/// Writes a fake prefix under `root/python` and the script that reports
/// it, with `purelib` as both site directories.
#[cfg(unix)]
fn fake_python_with_site(root: &Path, purelib: &Path) -> FakePython {
    use std::os::unix::fs::PermissionsExt;
    let root = std::fs::canonicalize(root).expect("canonicalize the scratch root");
    let prefix = root.join("python");
    let include = prefix.join("include").join("python3.14");
    let lib = prefix.join("lib");
    let stdlib = lib.join("python3.14");
    write(&include.join("Python.h"), b"#error not a real Python.h\n");
    write(&lib.join("libpython3.14.dylib"), b"not a real library");
    write(&stdlib.join("os.py"), b"# os\n");
    let embed = format!(
        "3.14.7\n{}\n{}\n{}\n{}\n1\n\nlibpython3.14.dylib\n{}\nlibpython3.14.so.1.0\n0\n",
        prefix.join("bin").join("python3.14").display(),
        include.display(),
        stdlib.display(),
        prefix.display(),
        lib.display()
    );
    let lock = format!(
        "pycc-lock-probe 1\n{0}\n{0}\ncpython-314\nmacosx-11.0-arm64\ncpython\n3.14.7\nposix\n\
         arm64\nCPython\n25.0.0\nDarwin\nDarwin Kernel Version 25.0.0\n3.14.7\n3.14\ndarwin\n",
        purelib.display()
    );
    let sentinel = root.join("python-ran");
    let script = root.join("fake-python");
    let body = format!(
        "#!/bin/sh\necho ran >> '{}'\ncase \"$3\" in\n*pycc-lock-probe*) cat <<'PYCC'\n{lock}PYCC\n;;\n\
         *) cat <<'PYCC'\n{embed}PYCC\n;;\nesac\n",
        sentinel.display()
    );
    std::fs::write(&script, body).expect("write the fake interpreter");
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    FakePython {
        script,
        site: purelib.to_path_buf(),
        sentinel,
    }
}

#[cfg(unix)]
fn fake_python(root: &Path) -> FakePython {
    let site = std::fs::canonicalize(root)
        .expect("canonicalize")
        .join("site-packages");
    std::fs::create_dir_all(&site).expect("create site-packages");
    fake_python_with_site(root, &site)
}

/// Unpadded urlsafe base64.
fn urlsafe_b64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let mut acc = 0u32;
        for (i, byte) in chunk.iter().enumerate() {
            acc |= u32::from(*byte) << (16 - 8 * i);
        }
        for i in 0..=chunk.len() {
            out.push(ALPHABET[((acc >> (18 - 6 * i)) & 63) as usize] as char);
        }
    }
    out
}

fn record_hash(bytes: &[u8]) -> String {
    let hex = sha256::sha256_hex(bytes);
    let raw: Vec<u8> = (0..32)
        .map(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).expect("hex"))
        .collect();
    format!("sha256={}", urlsafe_b64(&raw))
}

/// Writes an installed distribution the way `pip` does: payload `files`,
/// METADATA with `requires`, INSTALLER, and a RECORD listing them all.
fn write_dist(site: &Path, name: &str, version: &str, files: &[(&str, &[u8])], requires: &[&str]) {
    let dist_info = format!("{name}-{version}.dist-info");
    let mut metadata = format!("Metadata-Version: 2.1\nName: {name}\nVersion: {version}\n");
    for requirement in requires {
        metadata.push_str(&format!("Requires-Dist: {requirement}\n"));
    }
    let mut record = String::new();
    let mut add = |path: &str, bytes: &[u8]| {
        write(&site.join(path), bytes);
        record.push_str(&format!("{path},{},{}\n", record_hash(bytes), bytes.len()));
    };
    for (path, bytes) in files {
        add(path, bytes);
    }
    add(&format!("{dist_info}/METADATA"), metadata.as_bytes());
    add(&format!("{dist_info}/INSTALLER"), b"pip\n");
    record.push_str(&format!("{dist_info}/RECORD,,\n"));
    write(&site.join(&dist_info).join("RECORD"), record.as_bytes());
}

/// `tinypkg` 1.0, requiring `tinydep` 2.0.
fn tiny_closure(site: &Path) {
    write_dist(
        site,
        "tinypkg",
        "1.0",
        &[
            ("tinypkg/__init__.py", b"VALUE = 1\n"),
            ("tinypkg/core.py", b"def f():\n    return 2\n"),
        ],
        &["tinydep>=2", "unused ; sys_platform == \"win32\""],
    );
    write_dist(site, "tinydep", "2.0", &[("tinydep.py", b"X = 3\n")], &[]);
}

fn lock(python: &FakePython, dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
        .arg("lock")
        .args(args)
        .current_dir(dir)
        .env("PYCC_PYTHON", &python.script)
        .output()
        .expect("pycc should spawn")
}

fn read_lock(dir: &Path) -> String {
    std::fs::read_to_string(dir.join("pycc.lock")).expect("read pycc.lock")
}

fn host_triple() -> &'static str {
    match (std::env::consts::ARCH, std::env::consts::OS) {
        ("aarch64", "macos") => "aarch64-apple-darwin",
        ("x86_64", "macos") => "x86_64-apple-darwin",
        ("aarch64", _) => "aarch64-unknown-linux-gnu",
        _ => "x86_64-unknown-linux-gnu",
    }
}

/// A foreign section for `entry` on the Windows triple, which no test host
/// ever derives.
fn windows_section(entry: &str) -> String {
    format!(
        "\n[[target]]\nentry = \"{entry}\"\ntriple = \"x86_64-pc-windows-msvc\"\npython = \
         \"3.14.7\"\ncache-tag = \"cpython-314\"\nplatform = \"win-amd64\"\nlibpython-sha256 = \
         \"00\"\nroots = []\n"
    )
}

const HEADER: &str = "# Generated by `pycc lock`. Do not edit.\nversion = 1\n";

#[cfg(unix)]
#[test]
fn a_lock_is_written_checked_and_rewritten_byte_identically() {
    let dir = ScratchDir::new("lock_roundtrip").expect("scratch");
    let python = fake_python(&dir);
    tiny_closure(&python.site);
    std::fs::write(dir.join("m.py"), "import tinypkg\nimport json\n").expect("write");
    let output = lock(&python, &dir, &["m.py"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr_of(&output));
    let text = read_lock(&dir);
    assert!(text.starts_with(HEADER), "{text}");
    assert!(text.contains("entry = \"m.py\"\n"), "{text}");
    assert!(
        text.contains(&format!("triple = \"{}\"\n", host_triple())),
        "{text}"
    );
    assert!(text.contains("roots = [\"tinypkg\"]\n"), "{text}");
    // `files` counts each payload, METADATA included (rule 4).
    assert!(
        text.contains("name = \"tinydep\"\nversion = \"2.0\"\nsite = \"purelib\"\nfiles = 2\n"),
        "{text}"
    );
    assert!(
        text.contains("name = \"tinypkg\"\nversion = \"1.0\"\nsite = \"purelib\"\nfiles = 3\n"),
        "{text}"
    );
    assert!(text.contains("requires = [\"tinydep\"]\n"), "{text}");
    assert!(!text.contains("unused"), "{text}");
    let output = lock(&python, &dir, &["--check", "m.py"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr_of(&output));
    let output = lock(&python, &dir, &["m.py"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr_of(&output));
    assert_eq!(read_lock(&dir), text);
    let leftovers: Vec<_> = std::fs::read_dir(&*dir)
        .expect("list")
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_name().to_string_lossy().contains(".tmp-"))
        .collect();
    assert!(leftovers.is_empty());
}

#[cfg(unix)]
#[test]
fn a_reinstalled_package_makes_check_fail_and_lock_update() {
    let dir = ScratchDir::new("lock_reinstalled").expect("scratch");
    let python = fake_python(&dir);
    tiny_closure(&python.site);
    // `import math` is a pycc_std binding, not a CPython-backed root.
    std::fs::write(dir.join("m.py"), "import math\nimport tinypkg\n").expect("write");
    assert_eq!(lock(&python, &dir, &["m.py"]).status.code(), Some(0));
    let before = read_lock(&dir);
    assert!(!before.contains("\"math\""), "{before}");
    // A reinstalled build: a payload byte and its RECORD hash change together.
    std::fs::remove_dir_all(python.site.join("tinypkg-1.0.dist-info")).expect("remove");
    write_dist(
        &python.site,
        "tinypkg",
        "1.0",
        &[
            ("tinypkg/__init__.py", b"VALUE = 9\n"),
            ("tinypkg/core.py", b"def f():\n    return 2\n"),
        ],
        &["tinydep>=2"],
    );
    let output = lock(&python, &dir, &["--check", "m.py"]);
    assert_eq!(output.status.code(), Some(1), "{}", stderr_of(&output));
    let stderr = stderr_of(&output);
    assert!(stderr.contains("package `tinypkg` differs"), "{stderr}");
    assert!(stderr.contains("run `pycc lock m.py`"), "{stderr}");
    assert_eq!(read_lock(&dir), before, "--check writes nothing");
    assert_eq!(lock(&python, &dir, &["m.py"]).status.code(), Some(0));
    assert_ne!(read_lock(&dir), before);
    assert_eq!(
        lock(&python, &dir, &["--check", "m.py"]).status.code(),
        Some(0)
    );
}

#[cfg(unix)]
#[test]
fn a_payload_edited_after_install_is_refused_naming_the_file() {
    let dir = ScratchDir::new("lock_tampered").expect("scratch");
    let python = fake_python(&dir);
    tiny_closure(&python.site);
    std::fs::write(dir.join("m.py"), "import tinypkg\n").expect("write");
    assert_eq!(lock(&python, &dir, &["m.py"]).status.code(), Some(0));
    let before = read_lock(&dir);
    std::fs::write(python.site.join("tinydep.py"), b"X = 4\n").expect("tamper");
    for args in [&["m.py"][..], &["--check", "m.py"][..]] {
        let output = lock(&python, &dir, args);
        assert_eq!(output.status.code(), Some(2), "{}", stderr_of(&output));
        assert!(
            stderr_of(&output).contains("tinydep.py"),
            "{}",
            stderr_of(&output)
        );
    }
    assert_eq!(read_lock(&dir), before);
}

#[cfg(unix)]
#[test]
fn sections_for_another_triple_and_another_entry_survive() {
    let dir = ScratchDir::new("lock_other_sections").expect("scratch");
    let python = fake_python(&dir);
    std::fs::write(dir.join("m.py"), "import json\n").expect("write");
    std::fs::write(dir.join("other.py"), "print(1)\n").expect("write");
    let seeded = format!(
        "{HEADER}{}{}",
        windows_section("m.py"),
        windows_section("other.py")
    );
    std::fs::write(dir.join("pycc.lock"), &seeded).expect("seed");
    let output = lock(&python, &dir, &["m.py"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr_of(&output));
    let text = read_lock(&dir);
    assert!(text.contains(&windows_section("m.py")), "{text}");
    assert!(text.contains(&windows_section("other.py")), "{text}");
    assert!(
        text.contains(&format!("entry = \"m.py\"\ntriple = \"{}\"", host_triple())),
        "{text}"
    );
    assert!(text.contains("roots = []\n"), "{text}");
}

#[cfg(unix)]
#[test]
fn a_policy_rejected_root_is_i0402() {
    let dir = ScratchDir::new("lock_pure").expect("scratch");
    let python = fake_python(&dir);
    std::fs::write(dir.join("m.py"), "import json\n").expect("write");
    let output = lock(&python, &dir, &["--pure", "m.py"]);
    assert_eq!(output.status.code(), Some(1), "{}", stderr_of(&output));
    assert!(
        stderr_of(&output).contains("error[I0402]"),
        "{}",
        stderr_of(&output)
    );
    assert!(!python.ran());
    assert!(!dir.join("pycc.lock").exists());
}

#[cfg(unix)]
#[test]
fn an_unowned_root_and_an_editable_install_are_refused() {
    let dir = ScratchDir::new("lock_refusals").expect("scratch");
    let python = fake_python(&dir);
    std::fs::write(dir.join("m.py"), "import missingpkg\n").expect("write");
    let output = lock(&python, &dir, &["m.py"]);
    assert_eq!(output.status.code(), Some(2), "{}", stderr_of(&output));
    assert!(
        stderr_of(&output).contains("`missingpkg`"),
        "{}",
        stderr_of(&output)
    );
    write_dist(
        &python.site,
        "editpkg",
        "0.1",
        &[
            ("editpkg/__init__.py", b"\n"),
            (
                "editpkg-0.1.dist-info/direct_url.json",
                b"{\"url\": \"file:///src\", \"dir_info\": {\"editable\": true}}",
            ),
        ],
        &[],
    );
    std::fs::write(dir.join("m.py"), "import editpkg\n").expect("write");
    let output = lock(&python, &dir, &["m.py"]);
    assert_eq!(output.status.code(), Some(2), "{}", stderr_of(&output));
    assert!(
        stderr_of(&output).contains("editable"),
        "{}",
        stderr_of(&output)
    );
    assert!(!dir.join("pycc.lock").exists());
}

#[cfg(unix)]
#[test]
fn an_unparsable_or_future_lock_is_refused_and_left_unchanged() {
    let dir = ScratchDir::new("lock_bad_existing").expect("scratch");
    let python = fake_python(&dir);
    std::fs::write(dir.join("m.py"), "import json\n").expect("write");
    for (text, needle) in [
        ("this is not toml [", "not a valid lock file"),
        ("version = 2\n", "schema version 2"),
    ] {
        std::fs::write(dir.join("pycc.lock"), text).expect("seed");
        for args in [&["m.py"][..], &["--check", "m.py"][..]] {
            let output = lock(&python, &dir, args);
            assert_eq!(output.status.code(), Some(2), "{}", stderr_of(&output));
            assert!(
                stderr_of(&output).contains(needle),
                "{}",
                stderr_of(&output)
            );
            assert_eq!(read_lock(&dir), text);
        }
    }
}

#[cfg(unix)]
#[test]
fn a_standard_library_only_program_is_locked_without_a_site_scan() {
    let dir = ScratchDir::new("lock_stdlib_only").expect("scratch");
    let root = std::fs::canonicalize(&*dir).expect("canonicalize");
    // The purelib the probe reports is a regular file: scanning it fails.
    let not_a_dir = root.join("site-is-a-file");
    std::fs::write(&not_a_dir, "").expect("write");
    let python = fake_python_with_site(&root, &not_a_dir);
    std::fs::write(dir.join("m.py"), "import json\nimport os\n").expect("write");
    let output = lock(&python, &dir, &["m.py"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr_of(&output));
    let text = read_lock(&dir);
    assert!(text.contains("roots = []\n"), "{text}");
    assert!(!text.contains("[[target.package]]"), "{text}");
    std::fs::write(dir.join("t.py"), "import json\nimport tinypkg\n").expect("write");
    let output = lock(&python, &dir, &["t.py"]);
    assert_eq!(output.status.code(), Some(2), "{}", stderr_of(&output));
    assert!(
        stderr_of(&output).contains("site-is-a-file"),
        "{}",
        stderr_of(&output)
    );
}

#[cfg(unix)]
#[test]
fn check_accepts_a_standard_library_only_program_with_no_lock() {
    let dir = ScratchDir::new("lock_stdlib_check").expect("scratch");
    let python = fake_python(&dir);
    std::fs::write(dir.join("m.py"), "import json\n").expect("write");
    let output = lock(&python, &dir, &["--check", "m.py"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr_of(&output));
    assert!(!python.ran(), "the lock is optional here: nothing to probe");
    assert!(!dir.join("pycc.lock").exists());
    // A third-party root has no such exemption.
    std::fs::write(dir.join("t.py"), "import tinypkg\n").expect("write");
    tiny_closure(&python.site);
    let output = lock(&python, &dir, &["--check", "t.py"]);
    assert_eq!(output.status.code(), Some(1), "{}", stderr_of(&output));
    assert!(
        stderr_of(&output).contains("it does not exist"),
        "{}",
        stderr_of(&output)
    );
    assert!(!dir.join("pycc.lock").exists());
}

#[cfg(unix)]
#[test]
fn every_spelling_of_one_entry_updates_one_section() {
    let dir = ScratchDir::new("lock_spellings").expect("scratch");
    let python = fake_python(&dir);
    let app = dir.join("app");
    std::fs::create_dir_all(&app).expect("mkdir");
    std::fs::write(app.join("main.py"), "import json\n").expect("write");
    std::os::unix::fs::symlink(&app, dir.join("link")).expect("symlink");
    let absolute = app.join("main.py");
    for spelling in [
        "./app/../app/main.py",
        absolute.to_str().expect("utf-8"),
        "link/main.py",
    ] {
        let output = lock(&python, &dir, &[spelling]);
        assert_eq!(output.status.code(), Some(0), "{}", stderr_of(&output));
    }
    let text = read_lock(&app);
    assert_eq!(text.matches("[[target]]").count(), 1, "{text}");
    assert!(text.contains("entry = \"main.py\"\n"), "{text}");
    assert!(!dir.join("pycc.lock").exists());
}

#[cfg(unix)]
#[test]
fn a_deleted_entry_is_pruned_and_fails_check_until_then() {
    let dir = ScratchDir::new("lock_prune").expect("scratch");
    let python = fake_python(&dir);
    std::fs::write(dir.join("a.py"), "import json\n").expect("write");
    std::fs::write(dir.join("b.py"), "import os\n").expect("write");
    assert_eq!(lock(&python, &dir, &["a.py"]).status.code(), Some(0));
    assert_eq!(lock(&python, &dir, &["b.py"]).status.code(), Some(0));
    assert_eq!(read_lock(&dir).matches("[[target]]").count(), 2);
    std::fs::remove_file(dir.join("a.py")).expect("remove");
    let output = lock(&python, &dir, &["--check", "b.py"]);
    assert_eq!(output.status.code(), Some(1), "{}", stderr_of(&output));
    assert!(
        stderr_of(&output).contains("no longer exists"),
        "{}",
        stderr_of(&output)
    );
    assert_eq!(lock(&python, &dir, &["b.py"]).status.code(), Some(0));
    let text = read_lock(&dir);
    assert_eq!(text.matches("[[target]]").count(), 1, "{text}");
    assert!(!text.contains("a.py"), "{text}");
    assert_eq!(
        lock(&python, &dir, &["--check", "b.py"]).status.code(),
        Some(0)
    );
}

#[cfg(unix)]
#[test]
fn a_program_with_no_cpython_import_removes_its_section_and_an_empty_lock() {
    let dir = ScratchDir::new("lock_native").expect("scratch");
    let python = fake_python(&dir);
    std::fs::write(dir.join("m.py"), "print(1)\n").expect("write");
    let output = lock(&python, &dir, &["m.py"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr_of(&output));
    assert!(!dir.join("pycc.lock").exists(), "no section, no file");
    assert!(!python.ran(), "the interpreter never starts");
    // A stale host section is removed, and the lock left empty is deleted.
    let stale = format!(
        "{HEADER}\n[[target]]\nentry = \"m.py\"\ntriple = \"{}\"\npython = \"3.14.7\"\n\
         cache-tag = \"cpython-314\"\nplatform = \"p\"\nlibpython-sha256 = \"00\"\nroots = []\n",
        host_triple()
    );
    std::fs::write(dir.join("pycc.lock"), &stale).expect("seed");
    let output = lock(&python, &dir, &["--check", "m.py"]);
    assert_eq!(output.status.code(), Some(1), "{}", stderr_of(&output));
    assert!(
        stderr_of(&output).contains("not needed"),
        "{}",
        stderr_of(&output)
    );
    assert_eq!(lock(&python, &dir, &["m.py"]).status.code(), Some(0));
    assert!(!dir.join("pycc.lock").exists());
    // With another section beside it, only the stale one goes.
    std::fs::write(
        dir.join("pycc.lock"),
        format!("{stale}{}", windows_section("m.py")),
    )
    .expect("seed");
    assert_eq!(lock(&python, &dir, &["m.py"]).status.code(), Some(0));
    assert_eq!(
        read_lock(&dir),
        format!("{HEADER}{}", windows_section("m.py"))
    );
    assert!(!python.ran());
}

#[cfg(unix)]
#[test]
fn a_read_only_directory_fails_without_leaving_a_temporary_file() {
    use std::os::unix::fs::PermissionsExt;
    let dir = ScratchDir::new("lock_read_only").expect("scratch");
    let python = fake_python(&dir);
    let app = dir.join("app");
    std::fs::create_dir_all(&app).expect("mkdir");
    std::fs::write(app.join("m.py"), "import json\n").expect("write");
    std::fs::set_permissions(&app, std::fs::Permissions::from_mode(0o555)).expect("chmod");
    let output = lock(&python, &dir, &["app/m.py"]);
    std::fs::set_permissions(&app, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    assert_eq!(output.status.code(), Some(2), "{}", stderr_of(&output));
    assert!(
        stderr_of(&output).contains("cannot write"),
        "{}",
        stderr_of(&output)
    );
    let names: Vec<String> = std::fs::read_dir(&app)
        .expect("list")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    assert_eq!(names, ["m.py"]);
}

#[cfg(unix)]
#[test]
fn the_lock_sits_beside_the_nearest_pycc_toml() {
    let dir = ScratchDir::new("lock_manifest").expect("scratch");
    let python = fake_python(&dir);
    tiny_closure(&python.site);
    let project = dir.join("project");
    let src = project.join("src");
    std::fs::create_dir_all(&src).expect("mkdir");
    std::fs::write(
        project.join("pycc.toml"),
        "[project]\nname = \"p\"\nentry = \"src/main.py\"\npython = \"3.14\"\n",
    )
    .expect("write");
    // (a) An entry that imports only a third-party root.
    std::fs::write(src.join("main.py"), "import tinypkg\n").expect("write");
    let output = lock(&python, &src, &["main.py"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr_of(&output));
    assert!(!src.join("pycc.lock").exists());
    let text = read_lock(&project);
    assert!(text.contains("entry = \"src/main.py\"\n"), "{text}");
    // (b) An entry with no import (whose load never runs discovery) removes
    // its stale section from the parent's lock.
    std::fs::write(src.join("main.py"), "print(1)\n").expect("write");
    let output = lock(&python, &src, &["main.py"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr_of(&output));
    assert!(!project.join("pycc.lock").exists());
    assert!(!src.join("pycc.lock").exists());
}

/// A real `venv --without-pip` of the build host's CPython 3.14.7 passes
/// the embed checks and locks a closure written into its site-packages.
#[cfg(unix)]
#[test]
#[ignore = "needs CPython 3.14.7 with a shared libpython (PYCC_PYTHON, default python3.14)"]
fn a_real_venv_is_locked_and_checked() {
    let dir = ScratchDir::new("lock_real_venv").expect("scratch");
    let base = std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3.14".into());
    let venv = dir.join("venv");
    let status = Command::new(&base)
        .args(["-m", "venv", "--without-pip"])
        .arg(&venv)
        .status()
        .expect("spawn the base interpreter");
    assert!(status.success());
    let site = Command::new(venv.join("bin").join("python"))
        .args([
            "-c",
            "import sysconfig; print(sysconfig.get_path('purelib'))",
        ])
        .output()
        .expect("spawn the venv interpreter");
    let site = PathBuf::from(String::from_utf8_lossy(&site.stdout).trim());
    tiny_closure(&site);
    std::fs::write(dir.join("m.py"), "import tinypkg\n").expect("write");
    let python = FakePython {
        script: venv.join("bin").join("python"),
        site,
        sentinel: dir.join("unused"),
    };
    let output = lock(&python, &dir, &["m.py"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr_of(&output));
    let text = read_lock(&dir);
    assert!(text.contains("python = \"3.14.7\"\n"), "{text}");
    assert!(text.contains("cache-tag = \"cpython-314\"\n"), "{text}");
    assert!(text.contains("roots = [\"tinypkg\"]\n"), "{text}");
    assert!(text.contains("name = \"tinydep\""), "{text}");
    let output = lock(&python, &dir, &["--check", "m.py"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr_of(&output));
}
