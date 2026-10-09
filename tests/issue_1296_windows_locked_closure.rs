//! Part 1 of #1287 (#1296): on a Windows host `pycc lock` writes the
//! `x86_64-pc-windows-msvc` section, and `pycc build` bundles the locked
//! pure-Python closure into `OUT.pycc\closure\`, the third module search
//! path after `Lib` and `DLLs` (D-249's and D-253's 2026-09-24
//! amendments under `docs/decisions/`). A closure's PE images are
//! classified strictly and the natives beside them copied into
//! `OUT.pycc\natives\` (#1306); an import no rule places is refused by
//! the lock and the build alike.
//!
//! Windows-only: every other host runs the same code on the fake Windows
//! layout in `src/embed/windows_lock_tests.rs`. The tests lock a real
//! `python -m venv --without-pip` environment of CPython 3.14.7
//! (`PYCC_PYTHON`, default `python3.14.exe`) holding a test-authored
//! `tinypkg` and the `tinyext` it requires, whose test-written C images
//! are compiled by the CI leg's LLVM `clang.exe` (`LLVM_SYS_221_PREFIX`),
//! so nothing downloads anything and no test calls `pip`, `uv` or an
//! index. On a Windows GitHub Actions leg a missing precondition fails the
//! test instead of skipping it, the pattern of
//! `tests/issue_1286_windows_embedded_executable.rs`.

#![cfg(windows)]

use pycc_scratch::ScratchDir;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// pycc's own SHA-256, shared so the fixtures' RECORD hashes need no
/// hashing crate.
#[allow(dead_code)]
#[path = "../src/embed/sha256.rs"]
mod sha256;

/// pycc's own PE reader, for the forwarder check on the real
/// `python3.dll`.
#[allow(dead_code)]
#[path = "../src/embed/pe.rs"]
mod pe;

fn stdout_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n")
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).replace("\r\n", "\n")
}

/// Whether this run is a Windows GitHub Actions leg, where every
/// precondition must hold.
fn windows_required() -> bool {
    std::env::var("GITHUB_ACTIONS").as_deref() == Ok("true")
}

/// The interpreter the venv is created from.
fn interpreter() -> OsString {
    std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3.14.exe".into())
}

/// Whether the tests run here: only when the interpreter is CPython
/// 3.14.7, and a CI leg panics when it is not.
fn hosted() -> bool {
    let python = interpreter();
    let version = Command::new(&python)
        .arg("--version")
        .output()
        .map(|output| stdout_of(&output).trim().to_string());
    match version {
        Ok(version) if version == "Python 3.14.7" => true,
        other if windows_required() => {
            panic!("the embed interpreter {python:?} is not CPython 3.14.7: {other:?}")
        }
        _ => false,
    }
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

/// One RECORD line for `path` holding `bytes`.
fn record_line(path: &str, bytes: &[u8]) -> String {
    let hex = sha256::sha256_hex(bytes);
    let raw: Vec<u8> = (0..32)
        .map(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).expect("hex"))
        .collect();
    format!("{path},sha256={},{}\n", urlsafe_b64(&raw), bytes.len())
}

const PROGRAM: &str = "import tinypkg\n\nprint(str(tinypkg.f()))\n";

/// `tinyext`'s pure-Python default: `tinypkg.f()` returns its `value()`.
const PURE_EXT: &[u8] = b"def value():\n    return 42\n";

/// A venv with `tinypkg` and `tinyext` installed the way `pip` does, and
/// `main.py`.
struct Fixture {
    dir: PathBuf,
    python: PathBuf,
    site: PathBuf,
    _scratch: ScratchDir,
}

impl Fixture {
    fn new(tag: &str) -> Self {
        let scratch = ScratchDir::new(tag).expect("scratch");
        let dir = std::fs::canonicalize(&*scratch).expect("canonicalize");
        let venv = dir.join("venv");
        let status = Command::new(interpreter())
            .args(["-m", "venv", "--without-pip"])
            .arg(&venv)
            .status()
            .expect("spawn the base interpreter");
        assert!(status.success());
        let python = venv.join("Scripts").join("python.exe");
        let site = Command::new(&python)
            .args([
                "-c",
                "import sysconfig; print(sysconfig.get_path('purelib'))",
            ])
            .output()
            .expect("spawn the venv interpreter");
        let site = PathBuf::from(stdout_of(&site).trim());
        let fixture = Self {
            dir,
            python,
            site,
            _scratch: scratch,
        };
        fixture.install(&[]);
        fixture.install_ext(&[("tinyext/__init__.py", PURE_EXT)], &[]);
        std::fs::write(fixture.dir.join("main.py"), PROGRAM).expect("write the program");
        fixture
    }

    /// Writes `tinypkg` 1.0, requiring `tinyext`, plus `extra` payload
    /// files, with a RECORD that also lists two console scripts outside the
    /// site directory (one spelled with `\`, as some installers write
    /// them) and an `MZ` launcher inside the package.
    fn install(&self, extra: &[(&str, &[u8])]) {
        let mut files: Vec<(&str, &[u8])> = vec![
            (
                "tinypkg/__init__.py",
                b"from tinyext import value\n\n\ndef f():\n    return value()\n",
            ),
            ("tinypkg/sub/__init__.py", b""),
            ("tinypkg/sub/mod.py", b"def g():\n    return 21\n"),
            ("tinypkg/cli.exe", b"MZ\x90\x00 a test-written launcher"),
            ("../../Scripts/tinypkg.exe", b"MZ a console script"),
            ("..\\..\\Scripts\\tinypkg-gui.exe", b"MZ a gui script"),
            (
                "tinypkg-1.0.dist-info/METADATA",
                b"Metadata-Version: 2.1\nName: tinypkg\nVersion: 1.0\nRequires-Dist: tinyext\n",
            ),
            ("tinypkg-1.0.dist-info/INSTALLER", b"pip\n"),
        ];
        files.extend_from_slice(extra);
        self.record("tinypkg-1.0.dist-info", &files);
    }

    /// Writes `tinyext` 1.0 (a transitive root, never a direct one) with
    /// `files` in its RECORD, then `unrecorded` beside them: the natives.
    fn install_ext(&self, files: &[(&str, &[u8])], unrecorded: &[(&str, &[u8])]) {
        let mut all: Vec<(&str, &[u8])> = files.to_vec();
        all.push((
            "tinyext-1.0.dist-info/METADATA",
            b"Metadata-Version: 2.1\nName: tinyext\nVersion: 1.0\n",
        ));
        all.push(("tinyext-1.0.dist-info/INSTALLER", b"pip\n"));
        self.record("tinyext-1.0.dist-info", &all);
        for (path, bytes) in unrecorded {
            write(&self.site.join(path), bytes);
        }
    }

    /// Writes `files` under the site and `dist_info`'s RECORD listing them.
    fn record(&self, dist_info: &str, files: &[(&str, &[u8])]) {
        let mut record = String::new();
        for (path, bytes) in files {
            write(&self.site.join(path), bytes);
            record.push_str(&record_line(path, bytes));
        }
        record.push_str(&format!("{dist_info}/RECORD,,\n"));
        write(&self.site.join(dist_info).join("RECORD"), record.as_bytes());
    }

    fn pycc(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_pycc"))
            .args(args)
            .current_dir(&self.dir)
            .env("PYCC_PYTHON", &self.python)
            .output()
            .expect("pycc should spawn")
    }

    fn lock(&self) {
        let output = self.pycc(&["lock", "main.py"]);
        assert_eq!(output.status.code(), Some(0), "{}", stderr_of(&output));
    }
}

/// The CI leg's LLVM `clang.exe`, which compiles the test-written images;
/// `None` (skip) off CI when it is missing, a panic on CI.
fn clang() -> Option<PathBuf> {
    let found = std::env::var_os("LLVM_SYS_221_PREFIX")
        .map(|prefix| PathBuf::from(prefix).join("bin").join("clang.exe"))
        .filter(|clang| clang.is_file());
    if found.is_none() && windows_required() {
        panic!("LLVM_SYS_221_PREFIX must name the LLVM install holding bin\\clang.exe");
    }
    found
}

/// The base interpreter's prefix, which holds `include\` and `libs\`.
fn base_prefix() -> PathBuf {
    let output = Command::new(interpreter())
        .args(["-c", "import sys; print(sys.base_prefix)"])
        .output()
        .expect("spawn the base interpreter");
    PathBuf::from(stdout_of(&output).trim())
}

/// Compiles `source` into `<out>\<name>` with `-shared` for the MSVC
/// target, the driver and target of pycc's own program-DLL link; the
/// driver also writes the import library `-l` names. `<out>` is outside
/// the venv.
fn link(clang: &Path, out: &Path, name: &str, source: &str, args: &[OsString]) -> PathBuf {
    let stem = name.rsplit_once('.').expect("a suffix").0;
    let c = out.join(format!("{stem}.c"));
    write(&c, source.as_bytes());
    let target = out.join(name);
    let output = Command::new(clang)
        .args(["-target", "x86_64-pc-windows-msvc", "-shared", "-o"])
        .arg(&target)
        .arg(&c)
        .args(args)
        .output()
        .expect("clang runs");
    assert!(output.status.success(), "{}", stderr_of(&output));
    target
}

/// A DLL exporting `int <function>(void)`, which returns `body`.
fn exporting(function: &str, body: &str) -> String {
    format!("__declspec(dllexport) int {function}(void) {{ return {body}; }}\n")
}

/// A single-phase extension module `module` whose `value()` returns
/// `body`, after declaring the imported functions `imports`.
fn extension(module: &str, imports: &[&str], body: &str) -> String {
    let mut source = String::from("#define PY_SSIZE_T_CLEAN\n#include <Python.h>\n");
    for function in imports {
        source.push_str(&format!("__declspec(dllimport) int {function}(void);\n"));
    }
    source.push_str(&format!(
        "static PyObject *value(PyObject *self, PyObject *args) {{\n\
         \x20   return PyLong_FromLong({body});\n}}\n\
         static PyMethodDef methods[] = {{\n\
         \x20   {{\"value\", value, METH_NOARGS, NULL}},\n\
         \x20   {{NULL, NULL, 0, NULL}}}};\n\
         static struct PyModuleDef module = {{PyModuleDef_HEAD_INIT, \"{module}\", NULL, -1, methods}};\n\
         PyMODINIT_FUNC PyInit_{module}(void) {{ return PyModule_Create(&module); }}\n"
    ));
    source
}

/// The flags linking an extension against the base interpreter and the
/// import libraries in `out` named by `libs`.
fn extension_args(out: &Path, libs: &[&str]) -> Vec<OsString> {
    let base = base_prefix();
    let mut args: Vec<OsString> = vec!["-I".into(), base.join("include").into()];
    args.extend(["-L".into(), base.join("libs").into(), "-lpython314".into()]);
    args.extend(["-L".into(), out.as_os_str().to_owned()]);
    args.extend(libs.iter().map(|lib| OsString::from(format!("-l{lib}"))));
    args
}

fn read(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// A scrubbed environment: only `SystemRoot`, and a `PATH` of the system
/// directories, so no other interpreter is reachable.
fn scrubbed(command: &mut Command) -> &mut Command {
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into());
    let root = PathBuf::from(root);
    let path = std::env::join_paths([root.join("System32"), root.clone()]).expect("join PATH");
    command
        .env_clear()
        .env("SystemRoot", &root)
        .env("PATH", path)
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir(to).expect("create the copy");
    for entry in std::fs::read_dir(from).expect("read_dir") {
        let entry = entry.expect("entry");
        let target = to.join(entry.file_name());
        if entry.file_type().expect("file type").is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).expect("copy");
        }
    }
}

/// Every file under `root`, by relative path, with its bytes.
fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("read_dir") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let rel = path.strip_prefix(root).expect("under root").to_path_buf();
                out.insert(rel, std::fs::read(&path).expect("read"));
            }
        }
    }
    out
}

/// (a) the Windows section is written with no native library; (b) the
/// build bundles the closure and matches the venv's CPython; (c) the
/// relocated executable still runs it with a scrubbed `PATH`.
#[test]
#[ignore = "needs CPython 3.14.7 as python3.14.exe or PYCC_PYTHON on Windows; run with --include-ignored"]
fn a_windows_locked_closure_runs_from_the_sidecar_and_matches_cpython() {
    if !hosted() {
        return;
    }
    let fixture = Fixture::new("win_lock_closure");
    let dir = &fixture.dir;
    fixture.lock();
    let text = std::fs::read_to_string(dir.join("pycc.lock")).expect("the lock");
    assert!(text.contains("x86_64-pc-windows-msvc"), "{text}");
    assert!(!text.contains("[[target.native]]"), "{text}");
    // Plan risk R6: a Windows venv keeps purelib and platlib in one
    // `Lib\site-packages`, and the section must still record purelib.
    assert!(text.contains("site = \"purelib\""), "{text}");

    let output = fixture.pycc(&["build", "main.py", "-o", "app"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr_of(&output));
    let closure = dir.join("app.pycc").join("closure");
    assert!(closure.join("tinypkg").join("sub").join("mod.py").is_file());
    assert!(closure.join("tinypkg").join("cli.exe").is_file());
    let oracle = Command::new(&fixture.python)
        .arg("main.py")
        .current_dir(dir)
        .output()
        .expect("CPython runs the program");
    assert_eq!(oracle.status.code(), Some(0), "{}", stderr_of(&oracle));
    assert_eq!(stdout_of(&oracle), "42\n");
    let embedded = Command::new(dir.join("app"))
        .output()
        .expect("the embedded executable runs");
    assert_eq!(embedded.status.code(), Some(0), "{}", stderr_of(&embedded));
    assert_eq!(stdout_of(&embedded), stdout_of(&oracle));

    let moved = ScratchDir::new("win_lock_moved").expect("scratch");
    std::fs::copy(dir.join("app"), moved.join("app")).expect("copy the stub");
    copy_tree(&dir.join("app.pycc"), &moved.join("app.pycc"));
    std::fs::rename(dir.join("venv"), dir.join("venv-moved")).expect("move the venv away");
    let run = scrubbed(&mut Command::new(moved.join("app")))
        .output()
        .expect("the relocated executable runs");
    assert_eq!(run.status.code(), Some(0), "{}", stderr_of(&run));
    assert_eq!(stdout_of(&run), stdout_of(&oracle));
}

/// (d) a native with an unresolvable import: after a clean lock and build
/// of a pure `tinyext`, `tinyext` alone is reinstalled at the same version
/// with `_bad.pyd` in its RECORD, linked against `pycc1306helper.dll`
/// beside it (unrecorded, so a native), which is linked against
/// `pycc1306missing.dll` (deleted after the link). `pycc lock` refuses it,
/// leaving the earlier lock byte-identical; `pycc build` refuses it by the
/// same scan (which precedes the payload copy's digest check), writing
/// nothing for a fresh output and leaving the earlier sidecar
/// byte-identical.
#[test]
#[ignore = "needs CPython 3.14.7 as python3.14.exe or PYCC_PYTHON and LLVM clang on Windows; run with --include-ignored"]
fn a_windows_native_with_an_unresolvable_import_is_refused_by_lock_and_build() {
    if !hosted() {
        return;
    }
    let Some(clang) = clang() else {
        return;
    };
    let fixture = Fixture::new("win_lock_native_bad");
    let dir = &fixture.dir;
    fixture.lock();
    let output = fixture.pycc(&["build", "main.py", "-o", "app"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr_of(&output));
    let before = snapshot(&dir.join("app.pycc"));
    let lock_before = read(&dir.join("pycc.lock"));

    let out = dir.join("cbuild");
    std::fs::create_dir(&out).expect("create the build directory");
    let source = exporting("pycc1306_missing", "1");
    let missing = link(&clang, &out, "pycc1306missing.dll", &source, &[]);
    let source = format!(
        "__declspec(dllimport) int pycc1306_missing(void);\n{}",
        exporting("pycc1306_helper", "pycc1306_missing() + 1")
    );
    let args = ["-L".into(), out.clone().into(), "-lpycc1306missing".into()];
    let helper = link(&clang, &out, "pycc1306helper.dll", &source, &args);
    let source = extension("_bad", &["pycc1306_helper"], "pycc1306_helper()");
    let args = extension_args(&out, &["pycc1306helper"]);
    let bad = link(&clang, &out, "_bad.pyd", &source, &args);
    std::fs::remove_file(&missing).expect("remove the missing DLL");
    std::fs::remove_file(out.join("pycc1306missing.lib")).expect("remove its import library");
    fixture.install_ext(
        &[
            ("tinyext/__init__.py", b"from tinyext._bad import value\n"),
            ("tinyext/_bad.pyd", &read(&bad)),
        ],
        &[("tinyext/pycc1306helper.dll", &read(&helper))],
    );

    let names = ["pycc1306helper.dll", "`tinyext`", "pycc1306missing.dll"];
    let output = fixture.pycc(&["lock", "main.py"]);
    assert_eq!(output.status.code(), Some(2), "{}", stderr_of(&output));
    let rendered = stderr_of(&output);
    for name in names {
        assert!(rendered.contains(name), "{name}: {rendered}");
    }
    assert_eq!(read(&dir.join("pycc.lock")), lock_before);

    let output = fixture.pycc(&["build", "main.py", "-o", "fresh"]);
    assert_eq!(output.status.code(), Some(2), "{}", stderr_of(&output));
    let rendered = stderr_of(&output);
    for name in names {
        assert!(rendered.contains(name), "{name}: {rendered}");
    }
    assert!(!dir.join("fresh.pycc").exists());
    assert!(!dir.join("fresh").exists() && !dir.join("fresh.exe").exists());

    let output = fixture.pycc(&["build", "main.py", "-o", "app"]);
    assert_eq!(output.status.code(), Some(2), "{}", stderr_of(&output));
    assert_eq!(snapshot(&dir.join("app.pycc")), before);
}

/// (e) a closure `.pyd` with a payload DLL beside it in RECORD and a
/// native beside it outside RECORD: the lock records the native, the build
/// copies it into `natives\` (never `closure\`), the embedded run matches
/// the venv's CPython, and still does relocated with a scrubbed `PATH`,
/// the real loader reaching the native through the launcher's
/// `AddDllDirectory(<sidecar>\natives)`. Without the native the import
/// fails.
#[test]
#[ignore = "needs CPython 3.14.7 as python3.14.exe or PYCC_PYTHON and LLVM clang on Windows; run with --include-ignored"]
fn a_windows_closure_pyd_with_a_native_runs_relocated_and_matches_cpython() {
    if !hosted() {
        return;
    }
    let Some(clang) = clang() else {
        return;
    };
    let fixture = Fixture::new("win_lock_native");
    let dir = &fixture.dir;
    let out = dir.join("cbuild");
    std::fs::create_dir(&out).expect("create the build directory");
    let source = exporting("pycc1306_native", "20");
    let native = link(&clang, &out, "pycc1306native.dll", &source, &[]);
    let source = exporting("pycc1306_payload", "22");
    let payload = link(&clang, &out, "pycc1306payload.dll", &source, &[]);
    let imports = ["pycc1306_native", "pycc1306_payload"];
    let body = "pycc1306_native() + pycc1306_payload()";
    let source = extension("_speed", &imports, body);
    let args = extension_args(&out, &["pycc1306native", "pycc1306payload"]);
    let speed = link(&clang, &out, "_speed.pyd", &source, &args);
    fixture.install_ext(
        &[
            ("tinyext/__init__.py", b"from tinyext._speed import value\n"),
            ("tinyext/_speed.pyd", &read(&speed)),
            ("tinyext/pycc1306payload.dll", &read(&payload)),
        ],
        &[("tinyext/pycc1306native.dll", &read(&native))],
    );

    fixture.lock();
    let text = std::fs::read_to_string(dir.join("pycc.lock")).expect("the lock");
    assert!(text.contains("name = \"pycc1306native.dll\""), "{text}");
    assert!(text.contains("required-by = [\"tinyext\"]"), "{text}");
    let output = fixture.pycc(&["build", "main.py", "-o", "app"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr_of(&output));
    let sidecar = dir.join("app.pycc");
    assert_eq!(
        read(&sidecar.join("natives").join("pycc1306native.dll")),
        read(&fixture.site.join("tinyext/pycc1306native.dll"))
    );
    let ext = sidecar.join("closure").join("tinyext");
    assert!(!ext.join("pycc1306native.dll").exists());
    assert!(ext.join("pycc1306payload.dll").is_file());

    let oracle = Command::new(&fixture.python)
        .arg("main.py")
        .current_dir(dir)
        .output()
        .expect("CPython runs the program");
    assert_eq!(oracle.status.code(), Some(0), "{}", stderr_of(&oracle));
    assert_eq!(stdout_of(&oracle), "42\n");
    let embedded = Command::new(dir.join("app"))
        .output()
        .expect("the embedded executable runs");
    assert_eq!(embedded.status.code(), Some(0), "{}", stderr_of(&embedded));
    assert_eq!(stdout_of(&embedded), stdout_of(&oracle));

    let moved = ScratchDir::new("win_lock_native_moved").expect("scratch");
    std::fs::copy(dir.join("app"), moved.join("app")).expect("copy the stub");
    copy_tree(&sidecar, &moved.join("app.pycc"));
    std::fs::rename(dir.join("venv"), dir.join("venv-moved")).expect("move the venv away");
    let run = scrubbed(&mut Command::new(moved.join("app")))
        .output()
        .expect("the relocated executable runs");
    assert_eq!(run.status.code(), Some(0), "{}", stderr_of(&run));
    assert_eq!(stdout_of(&run), stdout_of(&oracle));

    // Negative control: the native is what the loader resolved.
    std::fs::remove_file(
        moved
            .join("app.pycc")
            .join("natives")
            .join("pycc1306native.dll"),
    )
    .expect("remove the native");
    let run = scrubbed(&mut Command::new(moved.join("app")))
        .output()
        .expect("the relocated executable runs");
    assert_ne!(run.status.code(), Some(0));
    let rendered = stderr_of(&run);
    assert!(rendered.contains("_speed"), "{rendered}");
    assert!(rendered.contains("DLL load failed"), "{rendered}");
}

/// (f) the forwarder reader over the base interpreter's real `python3.dll`,
/// whose exports all forward to `python314.dll`.
#[test]
#[ignore = "needs CPython 3.14.7 as python3.14.exe or PYCC_PYTHON on Windows; run with --include-ignored"]
fn the_forwarder_reader_reads_the_real_python3_dll() {
    if !hosted() {
        return;
    }
    let bytes = read(&base_prefix().join("python3.dll"));
    let modules = pe::parse_forwarders(&bytes).expect("python3.dll's exports read");
    let folded: Vec<String> = modules.iter().map(|m| m.to_ascii_lowercase()).collect();
    assert_eq!(folded, ["python314.dll"]);
}
