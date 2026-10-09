//! Part 3 of #1227 (#1273): `pycc build --static-libpython` against a
//! real `libpython3.14.a`, end to end (the static-libpython decision entry
//! under `docs/decisions/`, D-251; `docs/CLI_SPEC.md`'s `pycc build`
//! options). `tests/issue_1271_static_libpython.rs` and
//! `tests/issue_1272_static_lock.rs` stop at a fixture archive; these tests
//! link the embed interpreter's own `LIBPL/LIBRARY`, run the executable,
//! and compare it with that interpreter.
//!
//! Every test is `#[ignore]`d: it needs CPython 3.14.7 as `PYCC_PYTHON`
//! (default `python3.14`), and runs under `cargo test --workspace --
//! --include-ignored` on every non-Windows Tier-1 leg. Whether the
//! interpreter's `LIBPL` holds a genuine archive decides what a test proves,
//! judged as pycc's own archive check judges it (symlinks resolved, a
//! regular file, the ar magic):
//!
//! - a genuine archive: the full build, run, relocation and `lib-dynload`
//!   evidence below;
//! - no genuine archive on a Linux GitHub Actions leg: a failure. The
//!   `actions/setup-python` Linux builds are configured `--enable-shared`
//!   with CPython's default `--with-static-libpython`, so their `LIBPL`
//!   carries the archive, and those legs are where this evidence comes
//!   from; a leg that lost it must not pass silently;
//! - no genuine archive anywhere else (a macOS framework build's `LIBPL`
//!   entry is a symlink to its dylib; uv's builds ship none): the build is
//!   refused at exit 2 with the static request named, and the test ends.
//!
//! Every spawn uses `Command::output()`, whose stdin is null.

#![cfg_attr(windows, allow(dead_code, unused_imports))]

#[allow(dead_code)]
#[path = "../src/embed/sha256.rs"]
mod sha256;

use pycc_scratch::ScratchDir;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).replace("\r\n", "\n")
}

fn stdout_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n")
}

fn write(path: &Path, bytes: &[u8]) {
    std::fs::create_dir_all(path.parent().expect("a parent")).expect("create the parent");
    std::fs::write(path, bytes).expect("write a fixture file");
}

/// The embed interpreter a default build probes.
fn base_interpreter() -> OsString {
    std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3.14".into())
}

/// Whether this run is a Linux GitHub Actions leg, where the archive must
/// be genuine.
fn archive_required() -> bool {
    cfg!(target_os = "linux") && std::env::var("GITHUB_ACTIONS").as_deref() == Ok("true")
}

/// `python`'s version and `LIBPL/LIBRARY`, as its `sysconfig` reports them.
fn probe(python: &Path) -> (String, PathBuf) {
    let output = Command::new(python)
        .args([
            "-c",
            "import platform, sysconfig\n\
             print(platform.python_version())\n\
             print(sysconfig.get_config_var('LIBPL'))\n\
             print(sysconfig.get_config_var('LIBRARY'))\n",
        ])
        .output()
        .unwrap_or_else(|e| panic!("the embed interpreter {} runs: {e}", python.display()));
    assert!(output.status.success(), "{}", stderr_of(&output));
    let stdout = stdout_of(&output);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 3, "{stdout}");
    (lines[0].to_string(), Path::new(lines[1]).join(lines[2]))
}

/// The archive `archive` resolves to, when it is what pycc links: a
/// regular file, after symlinks, starting with the ar magic.
fn genuine(archive: &Path) -> Option<PathBuf> {
    use std::io::Read;
    let resolved = std::fs::canonicalize(archive).ok()?;
    if !resolved.is_file() {
        return None;
    }
    let mut head = [0u8; 8];
    std::fs::File::open(&resolved)
        .ok()?
        .read_exact(&mut head)
        .ok()?;
    (&head == b"!<arch>\n").then_some(resolved)
}

fn pycc_in(dir: &Path, python: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
        .args(args)
        .current_dir(dir)
        .env("PYCC_PYTHON", python)
        .output()
        .expect("pycc should spawn")
}

const STATIC_BUILD: [&str; 5] = ["build", "m.py", "-o", "app", "--static-libpython"];

/// The genuine archive of `python` (whose `m.py` is already in `dir`), or
/// `None` once the build has been shown to refuse a missing one. Panics on
/// a Linux GitHub Actions leg without one.
fn archive_or_refusal(dir: &Path, python: &Path) -> Option<PathBuf> {
    let (version, archive) = probe(python);
    assert_eq!(version, "3.14.7", "the embed interpreter must be 3.14.7");
    if let Some(resolved) = genuine(&archive) {
        return Some(resolved);
    }
    assert!(
        !archive_required(),
        "the embed interpreter {}'s `LIBPL/LIBRARY` `{}` is not a genuine ar archive on a Linux \
         GitHub Actions leg, where #1273's end-to-end evidence comes from",
        python.display(),
        archive.display()
    );
    let output = pycc_in(dir, python, &STATIC_BUILD);
    let rendered = stderr_of(&output);
    assert_eq!(output.status.code(), Some(2), "{rendered}");
    assert!(
        rendered.contains("a static libpython was requested (`--static-libpython` or"),
        "{rendered}"
    );
    assert!(!dir.join("app").exists());
    assert!(!dir.join("app.pycc").exists());
    None
}

/// Runs an embedded executable with no loader or interpreter variables
/// that could find a libpython or a standard library outside its sidecar.
/// `actions/setup-python` exports `LD_LIBRARY_PATH` at its install's `lib`.
fn run_app(app: &Path) -> Output {
    Command::new(app)
        .env_remove("LD_LIBRARY_PATH")
        .env_remove("DYLD_LIBRARY_PATH")
        .env_remove("PYTHONHOME")
        .env_remove("PYTHONPATH")
        .output()
        .expect("the embedded binary runs")
}

/// Every dynamic dependency `app` records, one per line.
fn dependencies(app: &Path) -> String {
    let output = if cfg!(target_os = "macos") {
        Command::new("otool").arg("-L").arg(app).output()
    } else {
        Command::new("readelf").arg("-d").arg(app).output()
    }
    .expect("the dependency lister runs");
    assert!(output.status.success(), "{}", stderr_of(&output));
    stdout_of(&output)
}

/// What a static build's output holds: the marker records the static link
/// and the archive's digest, the sidecar carries no libpython, and the
/// executable depends on none.
fn assert_static_output(dir: &Path, archive: &Path) {
    let sidecar = dir.join("app.pycc");
    let marker = std::fs::read_to_string(sidecar.join("PYCC-BUNDLE")).expect("the marker");
    assert!(marker.ends_with("libpython-link static\n"), "{marker}");
    let digest = sha256::sha256_hex(&std::fs::read(archive).expect("read the archive"));
    assert!(marker.contains(&digest), "{marker} lacks {digest}");
    for entry in std::fs::read_dir(sidecar.join("lib")).expect("the sidecar's lib") {
        let name = entry.expect("entry").file_name();
        let name = name.to_string_lossy();
        assert!(!name.starts_with("libpython"), "the sidecar holds {name}");
    }
    let deps = dependencies(&dir.join("app"));
    assert!(!deps.contains("libpython"), "{deps}");
    assert!(!deps.contains("Python.framework"), "{deps}");
}

/// The bundled `lib-dynload` file of the extension `root`.
fn dynload_file(dir: &Path, root: &str) -> PathBuf {
    let dynload = dir.join("app.pycc/lib/python3.14/lib-dynload");
    let prefix = format!("{root}.");
    std::fs::read_dir(&dynload)
        .expect("the sidecar has a lib-dynload directory")
        .map(|entry| entry.expect("entry").path())
        .find(|path| {
            path.file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with(&prefix))
        })
        .unwrap_or_else(|| panic!("no `{root}` extension in {}", dynload.display()))
}

/// Exactly what CPython prints for `m.py` in `dir`.
fn oracle(python: &Path, dir: &Path) -> Output {
    let output = Command::new(python)
        .arg(dir.join("m.py"))
        .output()
        .expect("CPython runs the program");
    assert_eq!(output.status.code(), Some(0), "{}", stderr_of(&output));
    output
}

fn assert_same(app: &Output, oracle: &Output) {
    assert_eq!(app.status.code(), Some(0), "{}", stderr_of(app));
    assert_eq!(stdout_of(app), stdout_of(oracle), "{}", stderr_of(app));
    assert_eq!(app.stdout, oracle.stdout);
}

/// `_json` is imported directly, because `json` falls back to pure Python
/// when its accelerator fails to load; `random` imports `math` and
/// `_random` unconditionally. All three come from `lib-dynload` in a
/// shared-configured CPython. pycc compiles `import math` natively, so
/// `math` is reached through `random`.
const STDLIB_PROGRAM: &str = "\
import _json
import json
import random

print(\"start\")
print(str(_json.encode_basestring_ascii(\"h\u{e9}\")))
print(str(json.dumps(2.5)))
print(str(json.dumps(\"h\u{e9}\")))
random.seed(7)
print(str(random.random()))
if __name__ == \"__main__\":
    print(\"main\")
print(\"end\")
";

/// The executable links the real archive whole and exports its symbols:
/// it starts CPython with no libpython beside it, and the `lib-dynload`
/// extensions `_json`, `math` and `_random` resolve their `Py*` symbols
/// against the executable itself. It matches CPython byte for byte, still
/// after it moves with its sidecar, and fails once the bundled `math`
/// extension is removed, so that is the file `random` loaded.
#[cfg(not(windows))]
#[test]
#[ignore = "needs CPython 3.14.7 as python3.14 or PYCC_PYTHON; run with --include-ignored"]
fn a_real_static_libpython_runs_lib_dynload_extensions_and_matches_cpython_3_14_7() {
    let dir = ScratchDir::new("real_static").expect("scratch");
    let dir = std::fs::canonicalize(&*dir).expect("canonicalize");
    std::fs::write(dir.join("m.py"), STDLIB_PROGRAM).expect("write");
    let python = PathBuf::from(base_interpreter());
    let Some(archive) = archive_or_refusal(&dir, &python) else {
        return;
    };
    let members = Command::new("ar")
        .arg("t")
        .arg(&archive)
        .output()
        .expect("ar runs");
    assert!(members.status.success(), "{}", stderr_of(&members));
    let members = stdout_of(&members);
    assert!(
        members.lines().any(|member| member == "pylifecycle.o"),
        "{} lists no pylifecycle.o: {members}",
        archive.display()
    );

    let output = pycc_in(&dir, &python, &STATIC_BUILD);
    assert_eq!(output.status.code(), Some(0), "{}", stderr_of(&output));
    assert_static_output(&dir, &archive);
    for root in ["_json", "math", "_random"] {
        assert!(dynload_file(&dir, root).is_file(), "{root}");
    }
    let expected = oracle(&python, &dir);
    assert_same(&run_app(&dir.join("app")), &expected);

    let moved = dir.join("moved");
    std::fs::create_dir(&moved).expect("create the new home");
    std::fs::rename(dir.join("app"), moved.join("app")).expect("move the binary");
    std::fs::rename(dir.join("app.pycc"), moved.join("app.pycc")).expect("move the sidecar");
    assert_same(&run_app(&moved.join("app")), &expected);

    std::fs::remove_file(dynload_file(&moved, "math")).expect("remove the bundled math");
    let missing = run_app(&moved.join("app"));
    assert_eq!(missing.status.code(), Some(1), "{}", stderr_of(&missing));
    assert!(
        stderr_of(&missing).contains("ModuleNotFoundError: No module named 'math'"),
        "{}",
        stderr_of(&missing)
    );
}

/// `ssl` loads `_ssl` from `lib-dynload`, which needs the system OpenSSL:
/// a separate test, so a failure there is not read as the archive's.
#[cfg(not(windows))]
#[test]
#[ignore = "needs CPython 3.14.7 as python3.14 or PYCC_PYTHON; run with --include-ignored"]
fn a_real_static_libpython_loads_ssl_and_matches_cpython_3_14_7() {
    let dir = ScratchDir::new("real_static_ssl").expect("scratch");
    let dir = std::fs::canonicalize(&*dir).expect("canonicalize");
    std::fs::write(
        dir.join("m.py"),
        "import ssl\n\nprint(str(ssl.OPENSSL_VERSION))\nprint(str(ssl.HAS_TLSv1_3))\n",
    )
    .expect("write");
    let python = PathBuf::from(base_interpreter());
    let Some(archive) = archive_or_refusal(&dir, &python) else {
        return;
    };
    let output = pycc_in(&dir, &python, &STATIC_BUILD);
    assert_eq!(output.status.code(), Some(0), "{}", stderr_of(&output));
    assert_static_output(&dir, &archive);
    assert!(dynload_file(&dir, "_ssl").is_file());
    assert_same(&run_app(&dir.join("app")), &oracle(&python, &dir));
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

/// A `pycc.lock` closure in a static build: `pycc lock` over a
/// `venv --without-pip` environment holding the test-authored `tinypkg`
/// (requiring `tinydep`) records the interpreter, `lock --check` accepts
/// it, and the static build bundles the closure. With the environment moved
/// away, the executable imports both from `app.pycc/closure/` and prints
/// CPython 3.14.7's first line; the second, the imported file, differs by
/// design and is checked to lie in the closure.
#[cfg(not(windows))]
#[test]
#[ignore = "needs CPython 3.14.7 as python3.14 or PYCC_PYTHON; run with --include-ignored"]
fn a_real_static_libpython_bundles_a_locked_closure_and_matches_cpython_3_14_7() {
    let dir = ScratchDir::new("real_static_lock").expect("scratch");
    let dir = std::fs::canonicalize(&*dir).expect("canonicalize");
    let venv = dir.join("venv");
    let created = Command::new(base_interpreter())
        .args(["-m", "venv", "--without-pip"])
        .arg(&venv)
        .output()
        .expect("spawn the base interpreter");
    assert!(created.status.success(), "{}", stderr_of(&created));
    let python = venv.join("bin").join("python");
    let site = Command::new(&python)
        .args([
            "-c",
            "import sysconfig; print(sysconfig.get_path('purelib'))",
        ])
        .output()
        .expect("spawn the venv interpreter");
    assert!(site.status.success(), "{}", stderr_of(&site));
    let site = PathBuf::from(stdout_of(&site).trim());
    write_dist(
        &site,
        "tinypkg",
        "1.0",
        &[(
            "tinypkg/__init__.py",
            b"import json\nimport tinydep\n\n\ndef run():\n    \
              print('tinypkg', json.dumps(tinydep.X * 2))\n\n\n\
              def where():\n    print(__file__)\n",
        )],
        &["tinydep>=2"],
    );
    write_dist(&site, "tinydep", "2.0", &[("tinydep.py", b"X = 21\n")], &[]);
    std::fs::write(
        dir.join("m.py"),
        "import tinypkg\n\ntinypkg.run()\ntinypkg.where()\n",
    )
    .expect("write");
    // The lock comes first: a build refuses a missing lock before it looks
    // for the archive.
    let output = pycc_in(&dir, &python, &["lock", "m.py"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr_of(&output));
    let lock = std::fs::read_to_string(dir.join("pycc.lock")).expect("the lock");
    assert!(
        lock.contains("tinypkg") && lock.contains("tinydep"),
        "{lock}"
    );
    let output = pycc_in(&dir, &python, &["lock", "--check", "m.py"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr_of(&output));
    let Some(archive) = archive_or_refusal(&dir, &python) else {
        return;
    };
    let output = pycc_in(&dir, &python, &STATIC_BUILD);
    assert_eq!(output.status.code(), Some(0), "{}", stderr_of(&output));
    assert_static_output(&dir, &archive);
    assert!(
        dir.join("app.pycc/closure/tinypkg/__init__.py").is_file()
            && dir.join("app.pycc/closure/tinydep.py").is_file()
    );
    let expected = oracle(&python, &dir);
    std::fs::rename(&venv, dir.join("venv-moved")).expect("move the environment away");
    let app = run_app(&dir.join("app"));
    assert_eq!(app.status.code(), Some(0), "{}", stderr_of(&app));
    let stdout = stdout_of(&app);
    let mut lines = stdout.lines();
    assert_eq!(lines.next(), Some("tinypkg 42"), "{stdout}");
    assert_eq!(stdout_of(&expected).lines().next(), Some("tinypkg 42"));
    let file = lines.next().expect("where() printed");
    assert!(
        file.ends_with("/app.pycc/closure/tinypkg/__init__.py"),
        "{file}"
    );
    assert_eq!(lines.next(), None, "{stdout}");
}
