//! Part 2 of #1225 (#1242): an embedded build of a program with a
//! CPython-backed import outside the standard library consumes `pycc.lock`
//! and bundles the locked closure into `OUT.pycc/closure/` (the pycc.lock
//! decision entry, D-249 rule 7; `docs/CLI_SPEC.md`'s `pycc.lock` section).
//!
//! The non-ignored tests need no Python. A missing lock is refused before
//! any interpreter is probed, so a nonexistent `PYCC_PYTHON` pins that
//! order. The lock round trip uses a `sh` script that answers the embed
//! probe and the lock probe with canned lines over a fake prefix and a fake
//! `site-packages` the test writes (as `tests/issue_1241_pycc_lock.rs`
//! does); every distribution is written by the test itself, so nothing
//! downloads anything and no test calls `pip`, `uv` or an index. They are
//! `cfg(not(windows))` because their fixture interpreter is a `sh` script;
//! the Windows closure is `tests/issue_1296_windows_locked_closure.rs`
//! (#1296).
//!
//! The `#[ignore]`d oracle test builds a real embedded executable from a
//! `python3.14 -m venv --without-pip` environment (`PYCC_PYTHON`, default
//! `python3.14`, must be CPython 3.14.7 with a shared libpython); `venv
//! --without-pip` is offline, and `tinypkg`/`tinydep` are test-authored.

#![cfg_attr(windows, allow(dead_code, unused_imports))]

use pycc_scratch::ScratchDir;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// pycc's own SHA-256, shared so the fixtures' RECORD hashes need no
/// hashing crate.
#[allow(dead_code)]
#[path = "../src/embed/sha256.rs"]
mod sha256;

fn pycc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).replace("\r\n", "\n")
}

fn write(path: &Path, bytes: &[u8]) {
    std::fs::create_dir_all(path.parent().expect("a parent")).expect("create the parent");
    std::fs::write(path, bytes).expect("write a fixture file");
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

/// `tinypkg` 1.0, requiring `tinydep` 2.0: `run()` prints through both,
/// `where()` prints the file `tinypkg` was loaded from.
fn tiny_closure(site: &Path) {
    write_dist(
        site,
        "tinypkg",
        "1.0",
        &[(
            "tinypkg/__init__.py",
            b"import tinydep\n\n\ndef run():\n    print('tinypkg', tinydep.X * 2)\n\n\n\
              def where():\n    print(__file__)\n",
        )],
        &["tinydep>=2"],
    );
    write_dist(site, "tinydep", "2.0", &[("tinydep.py", b"X = 21\n")], &[]);
}

/// A fake interpreter: `script` answers both probes over a fake prefix,
/// `site` is its purelib and platlib, and every run appends a line to
/// `sentinel`.
#[cfg(unix)]
struct FakePython {
    script: PathBuf,
    site: PathBuf,
    sentinel: PathBuf,
}

#[cfg(unix)]
fn fake_python(root: &Path) -> FakePython {
    use std::os::unix::fs::PermissionsExt;
    let root = std::fs::canonicalize(root).expect("canonicalize the scratch root");
    let site = root.join("site-packages");
    std::fs::create_dir_all(&site).expect("create site-packages");
    let prefix = root.join("python");
    let include = prefix.join("include").join("python3.14");
    let lib = prefix.join("lib");
    let stdlib = lib.join("python3.14");
    write(&include.join("Python.h"), b"#error not a real Python.h\n");
    write(&lib.join("libpython3.14.dylib"), b"not a real library");
    write(&lib.join("libpython3.14.so.1.0"), b"not a real library");
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
        site.display()
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
        site,
        sentinel,
    }
}

fn run_in(dir: &Path, python: &Path, args: &[&str]) -> Output {
    pycc()
        .args(args)
        .current_dir(dir)
        .env("PYCC_PYTHON", python)
        .output()
        .expect("pycc should spawn")
}

fn build(dir: &Path, python: &Path) -> Output {
    run_in(dir, python, &["build", "m.py", "-o", "app"])
}

/// A root outside the standard library with no `pycc.lock` is an
/// environment failure naming `pycc lock`, reported before the interpreter
/// is probed (the interpreter here does not exist), and nothing is
/// written.
#[cfg(not(windows))]
#[test]
fn a_third_party_import_without_a_lock_is_refused_naming_pycc_lock() {
    let dir = ScratchDir::new("closure_no_lock").expect("scratch");
    std::fs::write(dir.join("m.py"), "import tinypkg\n").expect("write");
    let output = build(&dir, Path::new("/nonexistent/pycc-no-python"));
    assert_eq!(output.status.code(), Some(2), "{}", stderr_of(&output));
    let rendered = stderr_of(&output);
    assert!(rendered.contains("imports `tinypkg`"), "{rendered}");
    assert!(rendered.contains("run `pycc lock m.py`"), "{rendered}");
    assert!(!rendered.contains("PYCC_PYTHON"), "{rendered}");
    assert!(!rendered.contains("I0403"), "{rendered}");
    assert!(!dir.join("app").exists());
    assert!(!dir.join("app.pycc").exists());
}

/// `pycc check` has no artifact, so it needs no lock.
#[cfg(not(windows))]
#[test]
fn check_accepts_a_third_party_import_without_a_lock() {
    let dir = ScratchDir::new("closure_check").expect("scratch");
    std::fs::write(dir.join("m.py"), "import tinypkg\n").expect("write");
    let output = run_in(
        &dir,
        Path::new("/nonexistent/pycc-no-python"),
        &["check", "m.py"],
    );
    assert_eq!(output.status.code(), Some(0), "{}", stderr_of(&output));
}

/// A lock whose roots no longer match the program is refused before the
/// interpreter runs; a current one, with roots imported from a project
/// module too, passes every lock check and the build goes on to bundle the
/// (fake, so unbundleable) interpreter.
#[cfg(not(windows))]
#[test]
fn a_stale_lock_is_refused_and_a_current_multi_module_lock_is_accepted() {
    let dir = ScratchDir::new("closure_roundtrip").expect("scratch");
    let python = fake_python(&dir);
    tiny_closure(&python.site);
    let helper = "def f() -> int:\n    return 1\n";
    std::fs::write(dir.join("helper.py"), format!("import tinydep\n{helper}")).expect("write");
    std::fs::write(
        dir.join("m.py"),
        "import tinypkg\nfrom helper import f\nx = f()\n",
    )
    .expect("write");
    let output = run_in(&dir, &python.script, &["lock", "m.py"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr_of(&output));
    let text = std::fs::read_to_string(dir.join("pycc.lock")).expect("read the lock");
    assert!(
        text.contains("roots = [\"tinydep\", \"tinypkg\"]\n"),
        "{text}"
    );

    std::fs::write(dir.join("helper.py"), helper).expect("write");
    std::fs::remove_file(&python.sentinel).expect("reset the sentinel");
    let output = build(&dir, &python.script);
    assert_eq!(output.status.code(), Some(2), "{}", stderr_of(&output));
    let rendered = stderr_of(&output);
    assert!(
        rendered.contains("`roots` lists `tinydep`, `tinypkg` but the program requires `tinypkg`"),
        "{rendered}"
    );
    assert!(rendered.contains("run `pycc lock m.py`"), "{rendered}");
    assert!(!python.sentinel.exists(), "the interpreter was not probed");
    assert!(!dir.join("app.pycc").exists());

    std::fs::write(dir.join("helper.py"), format!("import tinydep\n{helper}")).expect("write");
    let output = build(&dir, &python.script);
    let rendered = stderr_of(&output);
    assert!(!output.status.success(), "the fake library cannot link");
    assert!(python.sentinel.exists(), "the interpreter was probed");
    assert!(!rendered.contains("pycc.lock"), "{rendered}");
    assert!(!rendered.contains("pycc lock"), "{rendered}");
}

/// The locked closure is bundled: the embedded executable imports
/// `tinypkg` and its dependency from `app.pycc/closure/` and matches
/// CPython 3.14.7, still after the environment it was locked from is moved
/// away.
#[cfg(not(windows))]
#[test]
#[ignore = "needs CPython 3.14.7 with a shared libpython (PYCC_PYTHON, default python3.14)"]
fn a_locked_closure_runs_from_the_sidecar_and_matches_cpython_3_14_7() {
    let dir = ScratchDir::new("closure_oracle").expect("scratch");
    let dir = std::fs::canonicalize(&*dir).expect("canonicalize");
    let base = std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3.14".into());
    let venv = dir.join("venv");
    let status = Command::new(&base)
        .args(["-m", "venv", "--without-pip"])
        .arg(&venv)
        .status()
        .expect("spawn the base interpreter");
    assert!(status.success());
    let python = venv.join("bin").join("python");
    let site = Command::new(&python)
        .args([
            "-c",
            "import sysconfig; print(sysconfig.get_path('purelib'))",
        ])
        .output()
        .expect("spawn the venv interpreter");
    let site = PathBuf::from(String::from_utf8_lossy(&site.stdout).trim());
    tiny_closure(&site);
    std::fs::write(
        dir.join("m.py"),
        "import tinypkg\n\ntinypkg.run()\ntinypkg.where()\n",
    )
    .expect("write");
    let output = run_in(&dir, &python, &["lock", "m.py"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr_of(&output));
    let output = build(&dir, &python);
    assert_eq!(output.status.code(), Some(0), "{}", stderr_of(&output));
    assert!(
        dir.join("app.pycc/closure/tinypkg/__init__.py").is_file()
            && dir.join("app.pycc/closure/tinydep.py").is_file()
    );
    let oracle = Command::new(&python)
        .arg(dir.join("m.py"))
        .output()
        .expect("CPython runs the program");
    assert_eq!(oracle.status.code(), Some(0), "{}", stderr_of(&oracle));
    std::fs::rename(&venv, dir.join("venv-moved")).expect("move the environment away");
    let embedded = Command::new(dir.join("app"))
        .output()
        .expect("the embedded binary runs");
    assert_eq!(embedded.status.code(), Some(0), "{}", stderr_of(&embedded));
    let stdout = String::from_utf8_lossy(&embedded.stdout).replace("\r\n", "\n");
    let oracle_stdout = String::from_utf8_lossy(&oracle.stdout).replace("\r\n", "\n");
    let mut lines = stdout.lines();
    let mut oracle_lines = oracle_stdout.lines();
    assert_eq!(lines.next(), Some("tinypkg 42"), "{stdout}");
    assert_eq!(lines.next().is_some(), oracle_lines.nth(1).is_some());
    assert_eq!(oracle_stdout.lines().next(), Some("tinypkg 42"));
    let file = stdout
        .lines()
        .nth(1)
        .expect("where() printed")
        .replace('\\', "/");
    assert!(
        file.ends_with("/app.pycc/closure/tinypkg/__init__.py"),
        "{file}"
    );
}

/// The #1290 program: `tinypkg` is optional, and the fallback prints.
#[cfg(not(windows))]
const OPTIONAL_PROGRAM: &str = "try:\n    import tinypkg\nexcept ImportError:\n    print('fallback')\nelse:\n    tinypkg.run()\n";

/// #1290: a root imported only under an `except ImportError` guard is an
/// optional root. It is locked under `optional-roots`, with its closure when
/// it is installed and none when it is absent. The built program's roots
/// (post-monomorphize) agree with the lock in both cases, so the build
/// passes every lock check and goes on to bundle the (fake, so
/// unbundleable) interpreter. `pycc lock --check` still needs the lock.
#[cfg(not(windows))]
#[test]
fn an_optional_root_is_locked_and_accepted_whether_installed_or_absent() {
    let dir = ScratchDir::new("closure_optional").expect("scratch");
    let python = fake_python(&dir);
    std::fs::write(dir.join("m.py"), OPTIONAL_PROGRAM).expect("write");

    let output = run_in(&dir, &python.script, &["lock", "--check", "m.py"]);
    assert_eq!(output.status.code(), Some(1), "{}", stderr_of(&output));
    assert!(stderr_of(&output).contains("it does not exist"));

    for installed in [false, true] {
        if installed {
            tiny_closure(&python.site);
        }
        let output = run_in(&dir, &python.script, &["lock", "m.py"]);
        assert_eq!(output.status.code(), Some(0), "{}", stderr_of(&output));
        let text = std::fs::read_to_string(dir.join("pycc.lock")).expect("read the lock");
        assert!(
            text.contains("roots = []\noptional-roots = [\"tinypkg\"]\n"),
            "{text}"
        );
        assert_eq!(text.contains("name = \"tinypkg\""), installed, "{text}");
        std::fs::remove_file(&python.sentinel).expect("reset the sentinel");
        let output = build(&dir, &python.script);
        let rendered = stderr_of(&output);
        assert!(!output.status.success(), "the fake library cannot link");
        assert!(python.sentinel.exists(), "the interpreter was probed");
        assert!(!rendered.contains("pycc lock"), "{rendered}");
    }

    // Installing the package after the lock makes it stale.
    std::fs::remove_dir_all(&python.site).expect("remove the site");
    std::fs::create_dir_all(&python.site).expect("recreate the site");
    let output = run_in(&dir, &python.script, &["lock", "m.py"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr_of(&output));
    tiny_closure(&python.site);
    let output = run_in(&dir, &python.script, &["lock", "--check", "m.py"]);
    assert_eq!(output.status.code(), Some(1), "{}", stderr_of(&output));
    assert!(
        stderr_of(&output).contains("package `tinydep` is in the closure but not locked"),
        "{}",
        stderr_of(&output)
    );
}

/// #1290: the optional-import program runs its fallback when `tinypkg` is
/// absent and uses the bundled package when it is installed, in both cases
/// exactly as CPython 3.14.7 does in the same environment.
#[cfg(not(windows))]
#[test]
#[ignore = "needs CPython 3.14.7 with a shared libpython (PYCC_PYTHON, default python3.14)"]
fn an_optional_import_matches_cpython_3_14_7_absent_and_installed() {
    let dir = ScratchDir::new("closure_optional_oracle").expect("scratch");
    let dir = std::fs::canonicalize(&*dir).expect("canonicalize");
    let base = std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3.14".into());
    let venv = dir.join("venv");
    let status = Command::new(&base)
        .args(["-m", "venv", "--without-pip"])
        .arg(&venv)
        .status()
        .expect("spawn the base interpreter");
    assert!(status.success());
    let python = venv.join("bin").join("python");
    let site = Command::new(&python)
        .args([
            "-c",
            "import sysconfig; print(sysconfig.get_path('purelib'))",
        ])
        .output()
        .expect("spawn the venv interpreter");
    let site = PathBuf::from(String::from_utf8_lossy(&site.stdout).trim());
    std::fs::write(dir.join("m.py"), OPTIONAL_PROGRAM).expect("write");
    for (installed, expected) in [(false, "fallback\n"), (true, "tinypkg 42\n")] {
        if installed {
            tiny_closure(&site);
        }
        let output = run_in(&dir, &python, &["lock", "m.py"]);
        assert_eq!(output.status.code(), Some(0), "{}", stderr_of(&output));
        let output = build(&dir, &python);
        assert_eq!(output.status.code(), Some(0), "{}", stderr_of(&output));
        assert_eq!(
            dir.join("app.pycc/closure").exists(),
            installed,
            "a closure is bundled only for an installed optional root"
        );
        let oracle = Command::new(&python)
            .arg(dir.join("m.py"))
            .output()
            .expect("CPython runs the program");
        let embedded = Command::new(dir.join("app"))
            .output()
            .expect("the embedded binary runs");
        assert_eq!(embedded.status.code(), Some(0), "{}", stderr_of(&embedded));
        assert_eq!(oracle.status.code(), Some(0), "{}", stderr_of(&oracle));
        let stdout = String::from_utf8_lossy(&embedded.stdout).replace("\r\n", "\n");
        assert_eq!(stdout, expected);
        assert_eq!(
            String::from_utf8_lossy(&oracle.stdout).replace("\r\n", "\n"),
            expected
        );
    }
}
