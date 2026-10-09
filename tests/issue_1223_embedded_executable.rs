//! Part 1 of #1028 (#1223): a plain `pycc build` of a program whose only
//! CPython imports are standard-library modules produces an embedded
//! executable -- the native binary plus an `OUT.pycc/` sidecar holding the
//! build host's CPython 3.14 shared library and filtered standard library.
//!
//! The non-ignored tests pin every refusal (`I0403` with its reason, the
//! unmarked-sidecar and missing-interpreter environment failures) and that
//! a program with no CPython import stays fully native (#1045). They need
//! no interpreter, so they run on every leg and on the coverage host.
//!
//! The `#[ignore]`d `*_matches_cpython_3_14_7_byte_for_byte` tests build and
//! run a real embedded executable. They run on every non-Windows Tier-1 leg
//! (they read the POSIX sidecar layout, such as `lib/python3.14`; the
//! Windows counterpart is `tests/issue_1286_windows_embedded_executable.rs`)
//! under `cargo test --workspace -- --include-ignored`, where `python3.14`
//! on `PATH` is CPython 3.14.7 (the build's default embed interpreter;
//! `PYCC_PYTHON` overrides it). Each takes its CPython oracle from the
//! bundle's own `PYCC-BUNDLE` marker, so the oracle is exactly the
//! interpreter that was embedded, and asserts it is 3.14.7.
//!
//! Every spawn uses `Command::output()`, whose stdin is null: an embedded
//! launcher that blocked on its interpreter would read EOF, not hang.

use pycc_scratch::ScratchDir;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn pycc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn source(dir: &Path, body: &str) -> PathBuf {
    let src = dir.join("m.py");
    std::fs::write(&src, body).expect("write the fixture source");
    src
}

fn build(dir: &Path, body: &str, extra: &[&str]) -> Output {
    pycc()
        .arg("build")
        .arg(source(dir, body))
        .arg("-o")
        .arg(dir.join("app"))
        .args(extra)
        .output()
        .expect("pycc should spawn")
}

/// `--target` wins over every other reason: an embedded executable bundles
/// the build host's own interpreter, which cannot serve another target. Its
/// precedence makes this one assertion hold on every leg, Windows included.
#[test]
fn a_standard_library_import_in_a_target_build_is_refused_with_the_cross_target_reason() {
    let dir = ScratchDir::new("embed_cross_target").expect("scratch");
    let output = build(&dir, "import json\n", &["--target", "x86_64-apple-darwin"]);
    assert_eq!(output.status.code(), Some(1), "{}", stderr_of(&output));
    let rendered = stderr_of(&output);
    assert!(rendered.contains("error[I0403]"), "{rendered}");
    assert!(rendered.contains("in a `--target` build"), "{rendered}");
    assert!(!dir.join("app.pycc").exists());
}

/// A Tcl/Tk-backed standard-library root is excluded from the bundle and
/// keeps its own reason.
#[test]
fn an_excluded_standard_library_root_is_refused_with_its_own_reason() {
    let dir = ScratchDir::new("embed_tkinter").expect("scratch");
    let output = build(&dir, "import tkinter\n", &[]);
    assert_eq!(output.status.code(), Some(1), "{}", stderr_of(&output));
    let rendered = stderr_of(&output);
    assert!(rendered.contains("error[I0403]"), "{rendered}");
    assert!(rendered.contains("needs Tcl/Tk libraries"), "{rendered}");
}

/// A directory at `OUT.pycc` that pycc did not write is never replaced:
/// the build stops with an environment failure and leaves it intact,
/// before it looks for an interpreter.
#[test]
fn an_unmarked_sidecar_directory_is_refused_and_left_intact() {
    let dir = ScratchDir::new("embed_unmarked").expect("scratch");
    let sidecar = dir.join("app.pycc");
    std::fs::create_dir(&sidecar).expect("create the foreign directory");
    std::fs::write(sidecar.join("keep.txt"), "mine").expect("write a user file");
    let output = pycc()
        .arg("build")
        .arg(source(&dir, "import json\n"))
        .arg("-o")
        .arg(dir.join("app"))
        .env("PYCC_PYTHON", "/nonexistent/pycc-no-python")
        .output()
        .expect("pycc should spawn");
    assert_eq!(output.status.code(), Some(2), "{}", stderr_of(&output));
    let rendered = stderr_of(&output);
    assert!(
        rendered.contains("is not a bundle this pycc wrote"),
        "{rendered}"
    );
    assert_eq!(
        std::fs::read_to_string(sidecar.join("keep.txt")).unwrap(),
        "mine"
    );
    assert!(!dir.join("app").exists());
}

/// With no usable interpreter the build is an environment failure that
/// names the variable to set.
#[test]
fn a_missing_embed_interpreter_is_an_environment_failure_naming_pycc_python() {
    let dir = ScratchDir::new("embed_no_python").expect("scratch");
    let output = pycc()
        .arg("build")
        .arg(source(&dir, "import json\n"))
        .arg("-o")
        .arg(dir.join("app"))
        .env("PYCC_PYTHON", "/nonexistent/pycc-no-python")
        .output()
        .expect("pycc should spawn");
    assert_eq!(output.status.code(), Some(2), "{}", stderr_of(&output));
    let rendered = stderr_of(&output);
    assert!(rendered.contains("PYCC_PYTHON"), "{rendered}");
    assert!(
        rendered.contains("/nonexistent/pycc-no-python"),
        "{rendered}"
    );
    assert!(!dir.join("app").exists());
    assert!(!dir.join("app.pycc").exists());
}

/// A program with no CPython import stays a fully native executable: no
/// sidecar, and (#1045) no reference to any CPython symbol.
#[test]
fn a_program_without_a_cpython_import_stays_native_with_no_sidecar() {
    let dir = ScratchDir::new("embed_native").expect("scratch");
    let output = build(&dir, "print(42)\n", &[]);
    assert!(output.status.success(), "{}", stderr_of(&output));
    assert!(!dir.join("app.pycc").exists());
    // `pycc build -o app` writes exactly `app` on every host, Windows
    // included (no `.exe` suffix is appended).
    let exe = dir.join("app");
    let run = Command::new(&exe).output().expect("the native binary runs");
    assert_eq!(run.stdout, b"42\n");
    #[cfg(not(windows))]
    {
        let nm = Command::new("nm")
            .arg("-u")
            .arg(&exe)
            .output()
            .expect("nm runs");
        assert!(nm.status.success(), "{}", stderr_of(&nm));
        let undefined = String::from_utf8_lossy(&nm.stdout);
        for symbol in undefined.split_whitespace() {
            let name = symbol.trim_start_matches('_');
            assert!(
                !name.starts_with("Py"),
                "a native build must not reference CPython: {symbol}"
            );
        }
    }
}

// ---------------------------------------------------------------------
// Hosted end-to-end tests (need CPython 3.14.7 as the embed interpreter).
// ---------------------------------------------------------------------

/// The synthetic oracle program: it interleaves Python-side writes
/// (`pprint`) with `pycc_rt` writes and reads `__name__`. Whether `json`
/// finds its `_json` accelerator varies by build (uv links it in), so
/// `lib-dynload` has its own test below.
#[cfg(not(windows))]
const ORACLE: &str = "\
import json
import pprint
import textwrap

print(\"start\")
print(str(json.dumps(2.5)))
print(str(json.dumps(\"h\u{e9}\")))
pprint.pprint(5)
print(str(textwrap.shorten(\"the quick brown fox jumps over\", 15)))
if __name__ == \"__main__\":
    print(\"main\")
print(\"end\")
";

/// Builds `body` as an embedded executable at `dir/app` and returns the
/// interpreter its marker records, after checking that interpreter is the
/// pinned CPython 3.14.7.
#[cfg(not(windows))]
fn build_embedded(dir: &Path, body: &str) -> PathBuf {
    let output = build(dir, body, &[]);
    assert!(output.status.success(), "{}", stderr_of(&output));
    let marker = std::fs::read_to_string(dir.join("app.pycc").join("PYCC-BUNDLE"))
        .expect("an embedded build writes its marker");
    let mut lines = marker.lines();
    assert_eq!(lines.next(), Some("pycc-bundle 1"));
    assert_eq!(lines.next(), Some("python 3.14.7"), "{marker}");
    let executable = lines
        .next()
        .and_then(|line| line.strip_prefix("executable "))
        .expect("the marker names its interpreter");
    let executable = PathBuf::from(executable);
    let version = Command::new(&executable)
        .arg("--version")
        .output()
        .expect("the oracle runs");
    assert_eq!(
        String::from_utf8_lossy(&version.stdout).trim(),
        "Python 3.14.7"
    );
    executable
}

#[cfg(not(windows))]
fn cpython(python: &Path, script: &Path, envs: &[(&str, &Path)]) -> Output {
    let mut command = Command::new(python);
    command.arg(script);
    for (key, value) in envs {
        command.env(key, value);
    }
    command.output().expect("CPython runs the oracle program")
}

#[cfg(not(windows))]
fn assert_same(pycc_run: &Output, cpython_run: &Output) {
    assert_eq!(
        String::from_utf8_lossy(&pycc_run.stdout),
        String::from_utf8_lossy(&cpython_run.stdout),
        "stderr: {}",
        stderr_of(pycc_run)
    );
    assert_eq!(pycc_run.stdout, cpython_run.stdout);
    assert_eq!(pycc_run.status.code(), cpython_run.status.code());
}

#[cfg(not(windows))]
#[test]
#[ignore = "needs CPython 3.14.7 as python3.14 or PYCC_PYTHON; run with --include-ignored"]
fn the_oracle_program_matches_cpython_3_14_7_byte_for_byte() {
    let dir = ScratchDir::new("embed_oracle").expect("scratch");
    let python = build_embedded(&dir, ORACLE);
    let embedded = Command::new(dir.join("app"))
        .output()
        .expect("the embedded binary runs");
    assert_eq!(embedded.status.code(), Some(0), "{}", stderr_of(&embedded));
    assert_same(&embedded, &cpython(&python, &dir.join("m.py"), &[]));
}

/// Standard-library extensions some `lib-dynload` directory commonly
/// holds, in preference order. uv's CPython links most accelerators in and
/// keeps only `_dbm` and `_tkinter` (excluded) there.
#[cfg(not(windows))]
const DYNLOAD_CANDIDATES: [&str; 6] = ["_json", "_bisect", "_heapq", "_random", "_struct", "_dbm"];

/// A `lib-dynload` extension loads from the sidecar: the import succeeds
/// and matches CPython, and fails once the bundled file is removed, so the
/// isolated interpreter found it nowhere else.
#[cfg(not(windows))]
#[test]
#[ignore = "needs CPython 3.14.7 as python3.14 or PYCC_PYTHON; run with --include-ignored"]
fn a_lib_dynload_extension_loads_from_the_sidecar_and_matches_cpython_3_14_7_byte_for_byte() {
    let dir = ScratchDir::new("embed_dynload").expect("scratch");
    let dynload = dir
        .join("app.pycc")
        .join("lib")
        .join("python3.14")
        .join("lib-dynload");
    // A first build populates the sidecar so the test can see which
    // candidate this interpreter ships as a separate file.
    build_embedded(&dir, "import json\n\nprint(\"probe\")\n");
    let files: Vec<String> = std::fs::read_dir(&dynload)
        .expect("the sidecar has a lib-dynload directory")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    let (root, file) = DYNLOAD_CANDIDATES
        .iter()
        .find_map(|root| {
            let prefix = format!("{root}.");
            files
                .iter()
                .find(|file| file.starts_with(&prefix))
                .map(|file| (*root, file.clone()))
        })
        .unwrap_or_else(|| panic!("no candidate extension in lib-dynload: {files:?}"));
    let python = build_embedded(&dir, &format!("import {root}\n\nprint(\"loaded\")\n"));
    let embedded = Command::new(dir.join("app"))
        .output()
        .expect("the embedded binary runs");
    assert_eq!(embedded.status.code(), Some(0), "{}", stderr_of(&embedded));
    assert_same(&embedded, &cpython(&python, &dir.join("m.py"), &[]));
    std::fs::remove_file(dynload.join(&file)).expect("remove the bundled extension");
    let missing = Command::new(dir.join("app"))
        .output()
        .expect("the embedded binary runs");
    assert_eq!(missing.status.code(), Some(1), "{}", stderr_of(&missing));
    assert!(
        stderr_of(&missing).contains("ModuleNotFoundError"),
        "{}",
        stderr_of(&missing)
    );
}

/// The executable and its sidecar move together; nothing points back at
/// the build directory or the build host's interpreter.
#[cfg(not(windows))]
#[test]
#[ignore = "needs CPython 3.14.7 as python3.14 or PYCC_PYTHON; run with --include-ignored"]
fn a_relocated_embedded_executable_matches_cpython_3_14_7_byte_for_byte() {
    let dir = ScratchDir::new("embed_relocate").expect("scratch");
    let python = build_embedded(&dir, ORACLE);
    let moved = dir.join("moved");
    std::fs::create_dir(&moved).expect("create the new home");
    std::fs::rename(dir.join("app"), moved.join("app")).expect("move the binary");
    std::fs::rename(dir.join("app.pycc"), moved.join("app.pycc")).expect("move the sidecar");
    let embedded = Command::new(moved.join("app"))
        .output()
        .expect("the moved binary runs");
    assert_same(&embedded, &cpython(&python, &dir.join("m.py"), &[]));
}

/// The launcher runs an isolated interpreter: a `PYTHONPATH` entry holding
/// a shadowing `json.py` is ignored.
#[cfg(not(windows))]
#[test]
#[ignore = "needs CPython 3.14.7 as python3.14 or PYCC_PYTHON; run with --include-ignored"]
fn an_embedded_executable_ignoring_pythonpath_matches_cpython_3_14_7_byte_for_byte() {
    let dir = ScratchDir::new("embed_pythonpath").expect("scratch");
    let python = build_embedded(&dir, ORACLE);
    let shadow = dir.join("shadow");
    std::fs::create_dir(&shadow).expect("create the shadow directory");
    std::fs::write(shadow.join("json.py"), "raise SystemExit('shadowed')\n").expect("write shadow");
    let embedded = Command::new(dir.join("app"))
        .env("PYTHONPATH", &shadow)
        .output()
        .expect("the embedded binary runs");
    assert_same(&embedded, &cpython(&python, &dir.join("m.py"), &[]));
}

/// `pycc run` builds the same embedded executable into its scratch
/// directory and prints the same bytes.
#[cfg(not(windows))]
#[test]
#[ignore = "needs CPython 3.14.7 as python3.14 or PYCC_PYTHON; run with --include-ignored"]
fn pycc_run_of_the_oracle_program_matches_cpython_3_14_7_byte_for_byte() {
    let dir = ScratchDir::new("embed_run").expect("scratch");
    let python = build_embedded(&dir, ORACLE);
    let run = pycc()
        .arg("run")
        .arg(dir.join("m.py"))
        .output()
        .expect("pycc run spawns");
    assert_same(&run, &cpython(&python, &dir.join("m.py"), &[]));
}

/// `sys.exit(3)` from pycc code reaches the process exit status.
#[cfg(not(windows))]
#[test]
#[ignore = "needs CPython 3.14.7 as python3.14 or PYCC_PYTHON; run with --include-ignored"]
fn sys_exit_from_an_embedded_executable_matches_cpython_3_14_7_byte_for_byte() {
    let dir = ScratchDir::new("embed_exit").expect("scratch");
    let python = build_embedded(&dir, "import sys\n\nprint(\"before\")\nsys.exit(3)\n");
    let embedded = Command::new(dir.join("app"))
        .output()
        .expect("the embedded binary runs");
    assert_eq!(embedded.status.code(), Some(3), "{}", stderr_of(&embedded));
    assert_same(&embedded, &cpython(&python, &dir.join("m.py"), &[]));
}

/// `src/embed/stdlib_roots.rs`' two static lists, together, are exactly
/// the pinned interpreter's `sys.stdlib_module_names`.
#[cfg(not(windows))]
#[test]
#[ignore = "needs CPython 3.14.7 as python3.14 or PYCC_PYTHON; run with --include-ignored"]
fn the_embeddable_root_list_matches_cpython_3_14_7_byte_for_byte() {
    let dir = ScratchDir::new("embed_roots").expect("scratch");
    let python = build_embedded(&dir, "import json\n");
    let listed = Command::new(&python)
        .args([
            "-c",
            "import sys; print('\\n'.join(sorted(sys.stdlib_module_names)))",
        ])
        .output()
        .expect("the oracle lists its roots");
    let expected: Vec<String> = String::from_utf8_lossy(&listed.stdout)
        .lines()
        .map(str::to_owned)
        .collect();

    let file = include_str!("../src/embed/stdlib_roots.rs");
    let code = file
        .split("#[cfg(test)]")
        .next()
        .expect("the list precedes the tests");
    let mut actual: Vec<String> = code
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .flat_map(|line| {
            line.split('"')
                .skip(1)
                .step_by(2)
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .collect();
    actual.sort();
    assert_eq!(actual, expected);
}
