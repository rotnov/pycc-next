//! Part 1 of #1283 (#1318): a user class whose base is a CPython builtin
//! type -- lark 1.3.1's `class fzset(frozenset)` -- is valid Python this
//! version cannot compile yet, and `pycc` now says so instead of calling the
//! base an unknown class.
//!
//! The fixtures under `tests/diagnostics/` are driven from here rather than
//! from `tests/diagnostics_test.rs` (no fixture auto-discovery there; its
//! directory walk only scans every `.txt` for AST `Debug` dumps). The
//! comparison matches that file's `assert_diagnostic_matches_fixture` and
//! `assert_json_diagnostic_matches_fixture`: `pycc check` with a
//! repo-relative path from the repository root, expected bytes normalized
//! from `\r\n`, exit code 1.
//!
//! The last test is the CPython comparison, `#[ignore]`d like every test
//! that needs an installed interpreter (`PYCC_PYTHON`, default `python3`);
//! CI runs it under `--include-ignored`. It is asymmetric by design: CPython
//! accepts the class, and `pycc build --ext` refuses it with exactly the new
//! message. It also checks the enumeration of subclassable builtin types in
//! `crates/pycc_hir/src/class/mro.rs` against the interpreter, so that list
//! is not asserted from memory.

use pycc_scratch::ScratchDir;
use std::path::Path;
use std::process::Command;

const FROZENSET_MESSAGE: &str = "class `fzset` inherits from builtin type `frozenset` -- \
                                 subclassing a builtin type is not supported yet";

fn pycc_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_pycc"))
}

/// Runs `pycc check tests/diagnostics/<stem>.py <extra>` from the repository
/// root and compares its stdout with `<stem>.<extension>`.
fn assert_check_matches(stem: &str, extra: &[&str], extension: &str) {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let expected_path = repo_root
        .join("tests/diagnostics")
        .join(format!("{stem}.{extension}"));
    let expected = std::fs::read_to_string(&expected_path)
        .unwrap_or_else(|e| panic!("could not read {}: {e}", expected_path.display()))
        .replace("\r\n", "\n");
    let relative_py_path = format!("tests/diagnostics/{stem}.py");
    let output = Command::new(pycc_bin())
        .arg("check")
        .arg(&relative_py_path)
        .args(extra)
        .current_dir(repo_root)
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        expected,
        "{stem}: output did not match its .{extension} fixture"
    );
    assert_eq!(
        output.status.code(),
        Some(1),
        "{stem} should be a compile error"
    );
}

#[test]
fn a_frozenset_subclass_reports_one_builtin_base_diagnostic() {
    // One diagnostic: the later `size(items: fzset)` annotation is a D-219
    // cascade of the failed class and stays silent.
    assert_check_matches("c0001_builtin_frozenset_base", &[], "expected.txt");
}

#[test]
fn a_frozenset_subclass_reports_one_builtin_base_diagnostic_as_json() {
    assert_check_matches(
        "c0001_builtin_frozenset_base",
        &["--error-format", "json"],
        "expected.json",
    );
}

#[test]
fn a_list_subclass_reports_the_builtin_base_diagnostic() {
    assert_check_matches("c0001_builtin_list_base", &[], "expected.txt");
}

/// Driver run after the fixture's own source: CPython defines `fzset`, and
/// it behaves as a `frozenset` with its own `__repr__`. Then the
/// enumeration: every name `mro.rs` reports as a subclassable builtin type
/// is one, and the four it excludes as CPython-refused are refused.
const ORACLE_DRIVER: &str = r#"
s = fzset({1})
assert repr(s) == "{1}", repr(s)
assert isinstance(s, frozenset)
assert s == frozenset({1}) and hash(s) == hash(frozenset({1}))
assert 1 in s and size(s) == 1
for base in (int, float, complex, str, bytes, bytearray, list, tuple, dict, set, frozenset):
    type("X", (base,), {})
for base in (bool, range, slice, memoryview):
    try:
        type("X", (base,), {})
    except TypeError as error:
        assert "is not an acceptable base type" in str(error), error
    else:
        raise AssertionError(f"{base.__name__} was subclassable")
print("ok")
"#;

#[test]
#[ignore = "requires a CPython 3.13+ on PATH (PYCC_PYTHON)"]
fn cpython_accepts_the_frozenset_subclass_and_pycc_refuses_it_honestly() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let source = std::fs::read_to_string(
        repo_root.join("tests/diagnostics/c0001_builtin_frozenset_base.py"),
    )
    .expect("read the fixture");
    let run = Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
        .arg("-c")
        .arg(format!("{source}\n{ORACLE_DRIVER}"))
        .output()
        .expect("python3 should spawn");
    // CPython's text-mode stdout writes `\r\n` on Windows.
    assert_eq!(
        String::from_utf8_lossy(&run.stdout).replace("\r\n", "\n"),
        "ok\n",
        "stderr: {}",
        String::from_utf8_lossy(&run.stderr)
    );

    let dir = ScratchDir::new("1283_ext").expect("scratch");
    let src = dir.join("m.py");
    std::fs::write(&src, &source).expect("write the fixture");
    let build = Command::new(pycc_bin())
        .arg("build")
        .arg(&src)
        .arg("-o")
        // A bare name gains the host's own suffix (`.abi3.so` or `.pyd`).
        .arg(dir.join("m"))
        .arg("--ext")
        .output()
        .expect("pycc should spawn");
    let output = format!(
        "{}{}",
        String::from_utf8_lossy(&build.stdout),
        String::from_utf8_lossy(&build.stderr)
    );
    assert_eq!(build.status.code(), Some(1), "{output}");
    assert_eq!(
        output.matches("error[").count(),
        1,
        "exactly one diagnostic expected:\n{output}"
    );
    assert!(
        output.contains(&format!("error[C0001]: {FROZENSET_MESSAGE}")),
        "{output}"
    );
}
