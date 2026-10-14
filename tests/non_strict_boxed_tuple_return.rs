//! Non-strict mode: an unannotated function whose return the checker types
//! from its body as a *boxed* tuple (`return text[:n], len(text) - n`, a
//! `str` element) compiles and runs as CPython runs it, in a native build
//! with fallback code and as an `--ext` module.
//!
//! A build with fallback code (and every `--ext` build) emits an export
//! thunk for each exportable function with a tuple in its signature, and
//! the thunk flattened every tuple into its fields -- but a boxed tuple is
//! one pointer to its runtime list, so codegen panicked on it
//! (`pycc_codegen::ext::is_flat_tuple`). The non-strict solver had kept such
//! returns untyped to dodge the panic; the fixture pins both halves.

use pycc_scratch::ScratchDir;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const FIXTURE: &str = "non_strict_boxed_tuple_return.py";

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(FIXTURE)
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

fn python() -> std::ffi::OsString {
    std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into())
}

fn pycc(args: &[&str], source: &Path, out: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
        .arg("build")
        .args(args)
        .arg(source)
        .arg("-o")
        .arg(out)
        .output()
        .expect("pycc should spawn")
}

/// Same stdout, exit status and exception line.
fn assert_same(ours: &Output, oracle: &Output) {
    assert_eq!(text(&ours.stdout), text(&oracle.stdout));
    assert_eq!(
        ours.status.code(),
        oracle.status.code(),
        "{}",
        text(&ours.stderr)
    );
    assert_eq!(
        text(&ours.stderr).lines().last(),
        text(&oracle.stderr).lines().last()
    );
    assert_eq!(
        text(&oracle.stderr).lines().last(),
        Some("IndexError: list index out of range")
    );
}

#[test]
#[ignore = "requires a CPython 3.13+ oracle and embed interpreter (PYCC_PYTHON, default python3)"]
fn boxed_tuple_returns_build_natively_next_to_fallback_code() {
    let dir = ScratchDir::new("non_strict_boxed_tuple_native").expect("scratch");
    let source = dir.join("m.py");
    std::fs::copy(fixture(), &source).expect("copy the fixture");
    let build = pycc(&["--non-strict", "--report-fallbacks"], &source, &dir.join("app"));
    let report = text(&build.stderr);
    assert!(build.status.success(), "{report}");
    // Only the `eval` statement runs in CPython: every function compiles.
    assert_eq!(report.matches("fallback:").count(), 1, "{report}");
    assert!(report.contains("`eval`"), "{report}");
    let ours = Command::new(dir.join("app")).output().expect("run");
    let oracle = Command::new(python()).arg(&source).output().expect("python");
    assert_same(&ours, &oracle);
}

#[test]
#[ignore = "requires a CPython 3.13+ oracle and Python.h (PYCC_PYTHON, default python3)"]
fn boxed_tuple_returns_build_as_an_ext_module() {
    let dir = ScratchDir::new("non_strict_boxed_tuple_ext").expect("scratch");
    let ext_dir = dir.join("ext");
    let src_dir = dir.join("src");
    std::fs::create_dir_all(&ext_dir).expect("ext dir");
    std::fs::create_dir_all(&src_dir).expect("src dir");
    let source = src_dir.join("m.py");
    std::fs::copy(fixture(), &source).expect("copy the fixture");
    let build = pycc(&["--non-strict", "--ext"], &source, &ext_dir.join("m"));
    assert!(build.status.success(), "{}", text(&build.stderr));
    let import = |dir: &Path| {
        Command::new(python())
            .args(["-c", "import m"])
            .current_dir(dir)
            .output()
            .expect("python")
    };
    assert_same(&import(&ext_dir), &import(&src_dir));
}
