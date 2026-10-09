//! Part 1 of #1226 (#1286): on a Windows host a plain `pycc build` of a
//! program whose CPython imports are all standard-library roots produces an
//! embedded executable -- a stub `OUT` that imports only the system DLLs
//! `KERNEL32` and `ntdll` and loads `OUT.pycc\pycc_program.dll`, which links `python314.dll` (D-253).
//!
//! The file compiles on every host; every test returns at once off Windows.
//! Two refusal tests need no interpreter. The `#[ignore]`d tests build and
//! run a real embedded executable against CPython 3.14.7 (`PYCC_PYTHON`,
//! default `python3.14.exe`); on a Windows GitHub Actions leg a missing
//! precondition fails the test instead of skipping it, the pattern of
//! `tests/issue_1273_real_static_archive.rs`. They are the Windows
//! counterpart of the byte-for-byte tests in
//! `tests/issue_1223_embedded_executable.rs`, which read the POSIX sidecar
//! layout.
//!
//! Output comparisons apply D-082's `\r\n` -> `\n` normalization to both
//! sides: a Python-side write is newline-translated on Windows, a
//! `pycc_rt` write is not. Every spawn uses `Command::output()`, whose
//! stdin is null.

use pycc_scratch::ScratchDir;
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// pycc's own PE import reader, checked against `llvm-readobj` over the
/// real images a Windows sidecar bundles (#1305).
#[allow(dead_code)]
#[path = "../src/embed/pe.rs"]
mod pe;

fn pycc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
}

fn stdout_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// Drops the `\r` of every `\r\n` pair (D-082).
fn normalized(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut iter = bytes.iter().copied().peekable();
    while let Some(byte) = iter.next() {
        if byte == b'\r' && iter.peek() == Some(&b'\n') {
            continue;
        }
        out.push(byte);
    }
    out
}

/// Whether this run is a Windows GitHub Actions leg, where every
/// precondition must hold.
fn windows_required() -> bool {
    cfg!(windows) && std::env::var("GITHUB_ACTIONS").as_deref() == Ok("true")
}

/// The embed interpreter a default Windows build probes.
fn interpreter() -> OsString {
    std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3.14.exe".into())
}

/// The hosted tests' precondition: the embed interpreter is CPython 3.14.7.
fn precondition() -> Result<(), String> {
    let python = interpreter();
    let output = Command::new(&python)
        .arg("--version")
        .output()
        .map_err(|e| format!("the embed interpreter {python:?} does not run: {e}"))?;
    let version = stdout_of(&output);
    if version.trim() == "Python 3.14.7" {
        Ok(())
    } else {
        Err(format!(
            "the embed interpreter {python:?} is `{}`, not Python 3.14.7",
            version.trim()
        ))
    }
}

/// Whether a hosted test runs here: never off Windows; on Windows only
/// when the precondition holds, and a CI leg panics when it does not.
fn hosted() -> bool {
    if !cfg!(windows) {
        return false;
    }
    match precondition() {
        Ok(()) => true,
        Err(why) if windows_required() => panic!("{why}"),
        Err(_) => false,
    }
}

fn source(dir: &Path, body: &str) -> PathBuf {
    let src = dir.join("main.py");
    std::fs::write(&src, body).expect("write the fixture source");
    src
}

/// Builds `body` as `dir\app` and returns the sidecar.
fn build_embedded(dir: &Path, body: &str) -> PathBuf {
    let output = pycc()
        .arg("build")
        .arg(source(dir, body))
        .arg("-o")
        .arg(dir.join("app"))
        .output()
        .expect("pycc should spawn");
    assert!(output.status.success(), "{}", stderr_of(&output));
    let sidecar = dir.join("app.pycc");
    assert!(sidecar.join("PYCC-BUNDLE").is_file());
    sidecar
}

/// Runs the interpreter on `dir\main.py` from `dir`.
fn cpython(dir: &Path) -> Output {
    Command::new(interpreter())
        .arg("main.py")
        .current_dir(dir)
        .output()
        .expect("the oracle runs")
}

fn assert_same(embedded: &Output, oracle: &Output) {
    assert_eq!(
        String::from_utf8_lossy(&normalized(&embedded.stdout)),
        String::from_utf8_lossy(&normalized(&oracle.stdout)),
        "stdout differs; embedded stderr: {}",
        stderr_of(embedded)
    );
    assert_eq!(
        String::from_utf8_lossy(&normalized(&embedded.stderr)),
        String::from_utf8_lossy(&normalized(&oracle.stderr)),
        "stderr differs"
    );
    assert_eq!(embedded.status.code(), oracle.status.code());
}

/// A scrubbed environment: only `SystemRoot`, and a `PATH` of the system
/// directories, so no other interpreter's `python3.dll` is reachable.
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

fn names(dir: &Path) -> BTreeSet<String> {
    std::fs::read_dir(dir)
        .expect("read_dir")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect()
}

/// The interpreter's `sys.base_prefix`.
fn base_prefix() -> PathBuf {
    let output = Command::new(interpreter())
        .args(["-c", "import sys; print(sys.base_prefix)"])
        .output()
        .expect("the interpreter runs");
    assert!(output.status.success(), "{}", stderr_of(&output));
    PathBuf::from(stdout_of(&output).trim())
}

/// `math` is a pycc-native module, so the third root is `textwrap`.
const ORACLE: &str = "\
import json
import sys
import textwrap

print(str(json.dumps(2.5)))
print(str(json.dumps(\"h\u{e9}\")))
print(str(textwrap.shorten(\"the quick brown fox jumps over\", 15)))
print(str(sys.version_info[0]))
print(str(sys.version_info[1]))
";

#[test]
#[ignore = "needs CPython 3.14.7 as python3.14.exe or PYCC_PYTHON on Windows; run with --include-ignored"]
fn a_windows_embedded_executable_matches_cpython() {
    if !hosted() {
        return;
    }
    let dir = ScratchDir::new("win_embed_oracle").expect("scratch");
    build_embedded(&dir, ORACLE);
    let embedded = Command::new(dir.join("app"))
        .output()
        .expect("the embedded executable runs");
    assert_same(&embedded, &cpython(&dir));
}

#[test]
#[ignore = "needs CPython 3.14.7 as python3.14.exe or PYCC_PYTHON on Windows; run with --include-ignored"]
fn a_relocated_windows_embedded_executable_runs_with_a_scrubbed_path() {
    if !hosted() {
        return;
    }
    let dir = ScratchDir::new("win_embed_build").expect("scratch");
    build_embedded(
        &dir,
        "import sys\n\
         \n\
         print(str(sys.executable))\n\
         print(str(sys.argv[0]))\n\
         print(str(sys.flags.isolated))\n\
         for p in sys.path:\n\
         \x20   print(str(p))\n",
    );
    let moved = ScratchDir::new("win_embed_moved").expect("scratch");
    std::fs::copy(dir.join("app"), moved.join("app")).expect("copy the stub");
    copy_tree(&dir.join("app.pycc"), &moved.join("app.pycc"));
    std::fs::remove_file(dir.join("app")).expect("remove the original stub");
    std::fs::remove_dir_all(dir.join("app.pycc")).expect("remove the original sidecar");

    let app = moved.join("app");
    let run = scrubbed(&mut Command::new(&app))
        .output()
        .expect("the relocated executable runs");
    assert_eq!(run.status.code(), Some(0), "{}", stderr_of(&run));
    let stdout = String::from_utf8_lossy(&normalized(&run.stdout)).into_owned();
    let lines: Vec<&str> = stdout.lines().collect();
    assert!(lines.len() > 3, "{stdout}");
    let spawned = app.to_string_lossy().to_lowercase();
    assert_eq!(lines[0].to_lowercase(), spawned, "sys.executable: {stdout}");
    assert_eq!(lines[1].to_lowercase(), spawned, "sys.argv[0]: {stdout}");
    assert_eq!(lines[2], "1", "sys.flags.isolated: {stdout}");
    let sidecar = moved.join("app.pycc").to_string_lossy().to_lowercase();
    for entry in &lines[3..] {
        assert!(
            entry.to_lowercase().starts_with(&sidecar),
            "a sys.path entry outside {sidecar}: {entry}"
        );
    }
}

#[test]
#[ignore = "needs CPython 3.14.7 as python3.14.exe or PYCC_PYTHON on Windows; run with --include-ignored"]
fn a_windows_embedded_executable_loads_extension_dlls() {
    if !hosted() {
        return;
    }
    let dir = ScratchDir::new("win_embed_pyd").expect("scratch");
    build_embedded(
        &dir,
        "import _decimal\n\
         import ctypes\n\
         import sqlite3\n\
         import ssl\n\
         \n\
         print(str(ssl.OPENSSL_VERSION))\n\
         print(str(_decimal.__libmpdec_version__))\n\
         print(str(sqlite3.sqlite_version))\n\
         print(str(ctypes.c_int(7).value))\n",
    );
    let embedded = scrubbed(&mut Command::new(dir.join("app")))
        .output()
        .expect("the embedded executable runs");
    assert_same(&embedded, &cpython(&dir));
}

#[test]
#[ignore = "needs CPython 3.14.7 as python3.14.exe or PYCC_PYTHON on Windows; run with --include-ignored"]
fn pycc_run_of_a_windows_embedded_program_matches_cpython() {
    if !hosted() {
        return;
    }
    let dir = ScratchDir::new("win_embed_run").expect("scratch");
    let src = source(&dir, ORACLE);
    let run = pycc()
        .arg("run")
        .arg(&src)
        .output()
        .expect("pycc run spawns");
    assert_same(&run, &cpython(&dir));
}

#[test]
#[ignore = "needs CPython 3.14.7 as python3.14.exe or PYCC_PYTHON on Windows; run with --include-ignored"]
fn sys_exit_from_a_windows_embedded_executable_propagates() {
    if !hosted() {
        return;
    }
    let dir = ScratchDir::new("win_embed_exit").expect("scratch");
    build_embedded(&dir, "import sys\n\nprint(\"before\")\nsys.exit(3)\n");
    let embedded = Command::new(dir.join("app"))
        .output()
        .expect("the embedded executable runs");
    assert_eq!(embedded.status.code(), Some(3), "{}", stderr_of(&embedded));
    assert_same(&embedded, &cpython(&dir));
}

#[test]
#[ignore = "needs CPython 3.14.7 as python3.14.exe or PYCC_PYTHON on Windows; run with --include-ignored"]
fn a_windows_embedded_executable_without_its_interpreter_dll_fails_loudly() {
    if !hosted() {
        return;
    }
    let dir = ScratchDir::new("win_embed_no_dll").expect("scratch");
    let sidecar = build_embedded(&dir, "import json\n\nprint(str(json.dumps(1)))\n");
    std::fs::remove_file(sidecar.join("python314.dll")).expect("remove python314.dll");
    let run = scrubbed(&mut Command::new(dir.join("app")))
        .output()
        .expect("the stub runs");
    assert_eq!(run.status.code(), Some(121), "{}", stderr_of(&run));
    assert!(
        stderr_of(&run).contains("pycc: cannot load"),
        "{}",
        stderr_of(&run)
    );
}

#[test]
#[ignore = "needs CPython 3.14.7 as python3.14.exe or PYCC_PYTHON on Windows; run with --include-ignored"]
fn a_windows_embedded_build_bundles_the_interpreter_dlls() {
    if !hosted() {
        return;
    }
    let dir = ScratchDir::new("win_embed_layout").expect("scratch");
    let sidecar = build_embedded(&dir, "import json\n\nprint(str(json.dumps(1)))\n");
    let base = base_prefix();
    let runtime = ["vcruntime140.dll", "vcruntime140_1.dll"];
    let mut expected: BTreeSet<String> = [
        "PYCC-BUNDLE",
        "pycc_program.dll",
        "python314.dll",
        "python3.dll",
        "Lib",
        "DLLs",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    for dll in runtime {
        if windows_required() {
            assert!(
                base.join(dll).is_file(),
                "{dll} is not in {}",
                base.display()
            );
        }
        if base.join(dll).is_file() {
            expected.insert(dll.to_owned());
            assert_eq!(
                std::fs::read(base.join(dll)).expect("read the source DLL"),
                std::fs::read(sidecar.join(dll)).expect("read the bundled DLL"),
                "{dll} is not byte-identical"
            );
        }
    }
    assert_eq!(names(&sidecar), expected);
    let lib = names(&sidecar.join("Lib"));
    assert!(lib.contains("os.py"), "{lib:?}");
    assert!(!lib.contains("tkinter"), "{lib:?}");
    assert!(!lib.contains("site-packages"), "{lib:?}");
    let dlls = names(&sidecar.join("DLLs"));
    assert!(dlls.contains("_ssl.pyd"), "{dlls:?}");
    assert!(!dlls.contains("_tkinter.pyd"), "{dlls:?}");
    for name in &dlls {
        let lower = name.to_lowercase();
        assert!(
            !lower.starts_with("tcl") && !lower.starts_with("tk"),
            "a Tcl/Tk file was bundled: {name}"
        );
    }
}

/// `llvm-readobj --coff-imports`'s output for `path`.
fn readobj_imports(path: &Path) -> String {
    let prefix = std::env::var_os("LLVM_SYS_221_PREFIX")
        .map(PathBuf::from)
        .expect("LLVM_SYS_221_PREFIX names the LLVM 22 install");
    let tool = prefix.join("bin").join("llvm-readobj.exe");
    let output = Command::new(&tool)
        .arg("--coff-imports")
        .arg(path)
        .output()
        .unwrap_or_else(|e| panic!("{} runs: {e}", tool.display()));
    assert!(output.status.success(), "{}", stderr_of(&output));
    stdout_of(&output)
}

/// The COFF import DLL names `llvm-readobj --coff-imports` lists for `path`:
/// one `Name:` per import descriptor, then one per delay-import descriptor
/// (a per-symbol entry carries no `Name:`).
fn coff_imports(path: &Path) -> Vec<String> {
    readobj_imports(path)
        .lines()
        .filter_map(|line| line.trim().strip_prefix("Name: "))
        .map(str::to_owned)
        .collect()
}

/// Every file under `dir`, recursively.
fn files_under(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).expect("read the directory") {
        let path = entry.expect("an entry").path();
        if path.is_dir() {
            out.extend(files_under(&path));
        } else {
            out.push(path);
        }
    }
    out
}

/// Whether `path` is named like a PE image a Windows build loads.
fn image_named(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("pyd") || ext.eq_ignore_ascii_case("dll"))
}

#[test]
#[ignore = "needs CPython 3.14.7 as python3.14.exe or PYCC_PYTHON on Windows; run with --include-ignored"]
fn the_windows_stub_imports_only_system_dlls() {
    if !hosted() {
        return;
    }
    let dir = ScratchDir::new("win_embed_imports").expect("scratch");
    let sidecar = build_embedded(&dir, "import json\n\nprint(str(json.dumps(1)))\n");
    let stub = coff_imports(&dir.join("app"));
    // The static CRT also references `ntdll.dll`; both are system DLLs the
    // loader always finds in System32, so nothing the stub imports can come
    // from `PATH` or the sidecar.
    assert!(
        stub.iter()
            .any(|name| name.eq_ignore_ascii_case("kernel32.dll"))
            && stub.iter().all(|name| {
                name.eq_ignore_ascii_case("kernel32.dll") || name.eq_ignore_ascii_case("ntdll.dll")
            }),
        "{stub:?}"
    );
    // The program DLL imports `python314.dll` statically, so it is loaded
    // before any extension: the premise of the interpreter scan's rule that
    // an extension may delay-load the interpreter's DLL (#1305).
    let program = coff_imports(&sidecar.join("pycc_program.dll"));
    println!("pycc_program.dll imports: {program:?}");
    assert!(
        program
            .iter()
            .any(|name| name.eq_ignore_ascii_case("python314.dll")),
        "{program:?}"
    );
}

/// pycc's PE reader (#1305) against `llvm-readobj` over every image a real
/// Windows sidecar bundles at its root and in `DLLs\`, recursively: the
/// import names then the delay-import names, in order, and one delay
/// descriptor per `DelayImport {` block (LLVM sizes that list by the
/// directory size, the reader by its terminator). `Lib\` holds no image by
/// name, the real counterpart of the scan's `Lib\` refusal.
#[test]
#[ignore = "needs CPython 3.14.7 as python3.14.exe or PYCC_PYTHON on Windows; run with --include-ignored"]
fn the_pe_scanner_reads_the_bundled_images_as_llvm_readobj_does() {
    if !hosted() {
        return;
    }
    let dir = ScratchDir::new("win_embed_pe_oracle").expect("scratch");
    let sidecar = build_embedded(&dir, "import json\n\nprint(str(json.dumps(1)))\n");
    let mut images: Vec<PathBuf> = std::fs::read_dir(&sidecar)
        .expect("read the sidecar")
        .map(|entry| entry.expect("an entry").path())
        .filter(|path| path.is_file() && image_named(path))
        .collect();
    images.extend(
        files_under(&sidecar.join("DLLs"))
            .into_iter()
            .filter(|path| {
                image_named(path) || std::fs::read(path).is_ok_and(|bytes| bytes.starts_with(b"MZ"))
            }),
    );
    assert!(images.len() > 3, "{images:?}");
    for image in &images {
        let bytes = std::fs::read(image).expect("read the image");
        let parsed = pe::parse_pe(&bytes)
            .unwrap_or_else(|e| panic!("{}: {e}", image.display()))
            .unwrap_or_else(|| panic!("{} is not a PE image", image.display()));
        let mut ours = parsed.imports.clone();
        ours.extend(parsed.delay_imports.iter().cloned());
        let output = readobj_imports(image);
        assert_eq!(ours, coff_imports(image), "{}", image.display());
        let delay_blocks = output
            .lines()
            .filter(|line| line.trim() == "DelayImport {")
            .count();
        assert_eq!(
            delay_blocks,
            parsed.delay_imports.len(),
            "{}",
            image.display()
        );
    }
    let lib_images: Vec<PathBuf> = files_under(&sidecar.join("Lib"))
        .into_iter()
        .filter(|path| image_named(path))
        .collect();
    assert!(lib_images.is_empty(), "{lib_images:?}");
}

/// A root outside the standard library with no `pycc.lock` is refused on a
/// Windows host as on every host (#1296): an environment failure naming
/// `pycc lock`, before any interpreter is probed, with nothing written.
#[test]
fn a_windows_host_refuses_a_root_outside_the_standard_library_without_a_lock() {
    if !cfg!(windows) {
        return;
    }
    let dir = ScratchDir::new("win_embed_numpy").expect("scratch");
    let output = pycc()
        .arg("build")
        .arg(source(&dir, "import numpy\n"))
        .arg("-o")
        .arg(dir.join("app"))
        .env("PYCC_PYTHON", r"C:\nonexistent\pycc-no-python.exe")
        .output()
        .expect("pycc should spawn");
    assert_eq!(output.status.code(), Some(2), "{}", stderr_of(&output));
    let rendered = stderr_of(&output);
    assert!(rendered.contains("pycc lock"), "{rendered}");
    assert!(!rendered.contains("I0403"), "{rendered}");
    assert!(!dir.join("app").exists());
    assert!(!dir.join("app.pycc").exists());
}

/// `--static-libpython` is refused before the probe (D-251, D-253): no
/// interpreter is needed and nothing is written.
#[test]
fn a_windows_static_libpython_request_is_refused() {
    if !cfg!(windows) {
        return;
    }
    let dir = ScratchDir::new("win_embed_static").expect("scratch");
    let output = pycc()
        .arg("build")
        .arg(source(&dir, "import json\n"))
        .arg("-o")
        .arg(dir.join("app"))
        .arg("--static-libpython")
        .env("PYCC_PYTHON", r"C:\nonexistent\pycc-no-python.exe")
        .output()
        .expect("pycc should spawn");
    assert_eq!(output.status.code(), Some(2), "{}", stderr_of(&output));
    let rendered = stderr_of(&output);
    assert!(
        rendered.contains("CPython for Windows ships no static library"),
        "{rendered}"
    );
    assert!(!dir.join("app").exists());
    assert!(!dir.join("app.pycc").exists());
}
