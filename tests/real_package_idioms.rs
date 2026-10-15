//! Idioms of typed real-world packages (v0.4 accept 4), each against
//! CPython 3.13:
//!
//! - `typed_pkg`: a small parser package written as typed packages are
//!   (`from __future__ import annotations`, aliases imported only under
//!   `if TYPE_CHECKING:`, `Final` constants, frozenset tables, keyword-only
//!   parameters with computed defaults, `str | None` locals, `tuple[str,
//!   ...]` keys, an exception class of another module raised `from` a
//!   caught one), built module by module with `--ext --non-strict
//!   --foreign-relative-imports` as `bench/package_check.py` builds a real
//!   package; its driver prints the same as against the sources.
//! - `frozenset(str)`, the `str`-argument set predicates and membership in
//!   a set display, in a strict native build.
//! - Non-strict rebinding: a name a `try` body and its handler bind with
//!   two types, an object rebinding an `int` parameter, `type()` of a
//!   native `str`.
//! - A method name an import shares (`m.sqrt(...)` beside `Box.sqrt` with
//!   a keyword-only parameter): the import's call is not bound to the
//!   method's signature.
//! - Implicit concatenation of plain string literals and f-strings
//!   (`"allowed" f" {n} levels"`) keeps every part.
//! - `ext_scalar_guard`: a non-strict `--ext` function a Python caller
//!   passes an argument of another type than its scalar parameter's runs
//!   as CPython runs it, its own error handling included.

use pycc_scratch::ScratchDir;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

fn python() -> std::ffi::OsString {
    std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into())
}

fn pycc_build(args: &[&str], source: &Path, out: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
        .arg("build")
        .arg(source)
        .arg("-o")
        .arg(out)
        .args(args)
        .output()
        .expect("pycc should spawn")
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("dir");
    for entry in std::fs::read_dir(from).expect("read_dir") {
        let entry = entry.expect("entry");
        let target = to.join(entry.file_name());
        if entry.file_type().expect("type").is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).expect("copy");
        }
    }
}

/// Builds the oracle native program `name` with `args` and compares its
/// stdout and exit status with CPython running the source.
fn assert_native_matches(name: &str, args: &[&str]) {
    let dir = ScratchDir::new(&format!("real_package_idioms_{name}")).expect("scratch");
    let source = fixture(&format!("{name}.py"));
    let out = dir.join("prog");
    let build = pycc_build(args, &source, &out);
    assert!(build.status.success(), "{}", text(&build.stderr));
    let ours = Command::new(&out).output().expect("run");
    let oracle = Command::new(python()).arg(&source).output().expect("python");
    assert!(oracle.status.success(), "{}", text(&oracle.stderr));
    assert_eq!(text(&ours.stdout), text(&oracle.stdout), "{}", text(&ours.stderr));
    assert_eq!(ours.status.code(), oracle.status.code());
}

#[test]
#[ignore = "requires a CPython 3.13+ oracle (PYCC_PYTHON, default python3)"]
fn a_method_name_an_import_shares_matches_cpython() {
    assert_native_matches("method_name_of_import", &["--release"]);
}

#[test]
#[ignore = "requires a CPython 3.13+ oracle (PYCC_PYTHON, default python3)"]
fn fstring_implicit_concat_matches_cpython() {
    assert_native_matches("fstring_implicit_concat", &["--release"]);
}

#[test]
#[ignore = "requires a CPython 3.13+ oracle (PYCC_PYTHON, default python3)"]
fn frozenset_of_str_matches_cpython() {
    assert_native_matches("frozenset_of_str", &["--release"]);
}

#[test]
#[ignore = "requires a CPython 3.13+ oracle (PYCC_PYTHON, default python3)"]
fn non_strict_rebinding_matches_cpython() {
    assert_native_matches("non_strict_rebinding", &["--non-strict"]);
}

/// Builds `typed_pkg`'s `_core`, `_types` and `__init__` in place under
/// `<dir>/ext`, next to an untouched copy under `<dir>/src`; returns the
/// build reports.
fn build_typed_pkg(dir: &Path) -> String {
    let (ext, src) = (dir.join("ext"), dir.join("src"));
    copy_tree(&fixture("typed_pkg"), &ext);
    copy_tree(&fixture("typed_pkg"), &src);
    let mut report = String::new();
    for module in ["_core", "_types", "__init__"] {
        let source = ext.join("pkg").join(format!("{module}.py"));
        let build = pycc_build(
            &["--ext", "--non-strict", "--foreign-relative-imports", "--report-fallbacks"],
            &source,
            &ext.join("pkg").join(module),
        );
        report.push_str(&text(&build.stderr));
        assert!(build.status.success(), "{module}: {report}");
    }
    for module in ["_core", "_types", "__init__"] {
        std::fs::remove_file(ext.join("pkg").join(format!("{module}.py"))).expect("rm");
    }
    report
}

#[test]
#[ignore = "requires a CPython 3.13+ oracle and Python.h (PYCC_PYTHON, default python3)"]
fn a_typed_package_builds_unchanged_and_matches_cpython() {
    let dir = ScratchDir::new("real_package_idioms_typed_pkg").expect("scratch");
    let report = build_typed_pkg(&dir);
    // The parser's functions all compile: none falls back.
    for function in [
        "skip_chars",
        "parse_key",
        "parse_string",
        "parse_value",
        "loads",
        "describe",
        "Table",
    ] {
        // (An `__init__` re-export that names one runs in CPython anyway.)
        assert!(
            !report.lines().any(|line| {
                (line.starts_with("fallback: function") || line.starts_with("fallback: class"))
                    && line.contains(&format!("`{function}`"))
            }),
            "{function}\n{report}"
        );
    }
    // A function whose signature the boundary cannot carry is compiled for
    // the module's own callers and runs in CPython for Python's.
    assert!(report.contains("twin: function `skip_until`"), "{report}");
    let run = |side: &str| {
        Command::new(python())
            .arg("driver.py")
            .current_dir(dir.join(side))
            .output()
            .expect("python")
    };
    let (ours, oracle) = (run("ext"), run("src"));
    assert!(oracle.status.success(), "{}", text(&oracle.stderr));
    assert_eq!(text(&ours.stdout), text(&oracle.stdout), "{}", text(&ours.stderr));
    assert!(ours.status.success(), "{}", text(&ours.stderr));
}

#[test]
#[ignore = "requires a CPython 3.13+ oracle and Python.h (PYCC_PYTHON, default python3)"]
fn a_mistyped_scalar_argument_runs_as_cpython_runs_it() {
    let dir = ScratchDir::new("real_package_idioms_ext_scalar_guard").expect("scratch");
    let fixture_dir = fixture("ext_scalar_guard");
    let build = pycc_build(
        &["--ext", "--non-strict", "--report-fallbacks"],
        &fixture_dir.join("m.py"),
        &dir.join("m"),
    );
    let report = text(&build.stderr);
    assert!(build.status.success(), "{report}");
    // Every function compiles: a mistyped call is the only CPython run.
    assert!(!report.contains("fallback:"), "{report}");
    let run = |module_dir: &Path| {
        Command::new(python())
            .arg("-B")
            .arg(fixture_dir.join("driver.py"))
            .arg(module_dir)
            .output()
            .expect("python")
    };
    let (ours, oracle) = (run(&dir), run(&fixture_dir));
    assert!(oracle.status.success(), "{}", text(&oracle.stderr));
    assert_eq!(text(&ours.stdout), text(&oracle.stdout), "{}", text(&ours.stderr));
    assert!(ours.status.success(), "{}", text(&ours.stderr));
}
