//! Issue #20: fallback code of a non-strict build reaching compiled module
//! globals by a string -- `eval("c")`, `exec(src)`, `globals()["c"]`, and
//! `locals()`, `vars()` and `dir()` at module level -- sees and rebinds them
//! as CPython does, compiled class instances included, and compiled code
//! reads back what it binds.
//!
//! The oracle is `python3` (or `PYCC_PYTHON`): `issue_20_native.py` runs as
//! a native `--non-strict` build and as source; `issue_20_driver.py`
//! imports `issue_20_api.py` built `--ext --non-strict` and as source.
//! Strict mode never falls back: these builtins are a spanned `C0001`
//! there.

use pycc_scratch::ScratchDir;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn pycc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
}

fn python() -> Command {
    Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(&pycc_scratch::lf(bytes)).replace("\r\n", "\n")
}

fn last_line(bytes: &[u8]) -> String {
    text(bytes).lines().last().unwrap_or_default().to_string()
}

fn fixture(name: &str, dir: &Path, as_name: &str) -> PathBuf {
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(format!("{name}.py"));
    let path = dir.join(as_name);
    std::fs::copy(source, &path).expect("copy the fixture");
    path
}

fn assert_matches(ours: &Output, oracle: &Output) {
    assert_eq!(
        text(&ours.stdout),
        text(&oracle.stdout),
        "{}",
        text(&ours.stderr)
    );
    assert_eq!(
        ours.status.code(),
        oracle.status.code(),
        "{}",
        text(&ours.stderr)
    );
    assert_eq!(last_line(&ours.stderr), last_line(&oracle.stderr));
}

fn build(source: &Path, out: &Path, flags: &[&str]) -> String {
    let build = pycc()
        .args(["build", "--report-fallbacks"])
        .args(flags)
        .arg(source)
        .arg("-o")
        .arg(out)
        .output()
        .expect("pycc should spawn");
    let report = text(&build.stderr);
    assert!(build.status.success(), "{report}");
    report
}

#[test]
fn native_fallback_code_reaches_compiled_globals_by_a_string() {
    let dir = ScratchDir::new("issue_20_native").expect("scratch");
    let source = fixture("issue_20_native", &dir, "m.py");
    let report = build(&source, &dir.join("app"), &["--non-strict"]);
    // Only the functions calling the builtins run in CPython: the class and
    // the functions reading the globals stay compiled.
    assert!(!report.contains("class `Point`"), "{report}");
    assert!(!report.contains("`read_point`"), "{report}");
    let ours = Command::new(dir.join("app"))
        .output()
        .expect("run the build");
    let oracle = python().arg(&source).output().expect("python should spawn");
    assert!(
        text(&oracle.stdout).contains("exec sees -4 5 hi [1, 2]"),
        "the oracle runs the fixture: {}",
        text(&oracle.stdout)
    );
    assert_matches(&ours, &oracle);
}

/// Code reaching globals by a string does not reach a name a project import
/// binds (compiled into the program, it cannot fall back): the import stays
/// compiled and only the statement calling `eval` runs in CPython.
#[test]
fn reaching_globals_by_a_string_keeps_a_project_import_compiled() {
    let dir = ScratchDir::new("issue_20_project").expect("scratch");
    let source = fixture("issue_20_project_main", &dir, "main.py");
    fixture("issue_20_project_helper", &dir, "helper.py");
    let report = build(&source, &dir.join("app"), &["--non-strict"]);
    // Only the report's own lines: the build may also print linker
    // warnings (macOS `ld`).
    let fallbacks = report
        .lines()
        .filter(|line| line.starts_with("fallback:") || line.starts_with("lifted:"))
        .count();
    assert_eq!(fallbacks, 1, "{report}");
    assert!(report.contains("main.py:8"), "{report}");
    let ours = Command::new(dir.join("app"))
        .output()
        .expect("run the build");
    let oracle = python()
        .arg(&source)
        .current_dir(&*dir)
        .output()
        .expect("python should spawn");
    assert_eq!(text(&oracle.stdout), "42\n1 43\n", "the oracle runs the fixture");
    assert_matches(&ours, &oracle);
}

#[test]
fn ext_fallback_code_reaches_compiled_globals_by_a_string() {
    let compiled = ScratchDir::new("issue_20_ext").expect("scratch");
    let source = fixture("issue_20_api", &compiled, "api.py");
    fixture("issue_20_driver", &compiled, "driver.py");
    build(&source, &compiled.join("api"), &["--ext", "--non-strict"]);
    std::fs::remove_file(&source).expect("import the artifact, not the source");
    let plain = ScratchDir::new("issue_20_ext_src").expect("scratch");
    fixture("issue_20_api", &plain, "api.py");
    fixture("issue_20_driver", &plain, "driver.py");
    let run = |dir: &Path| {
        python()
            .arg("driver.py")
            .current_dir(dir)
            .output()
            .expect("python should spawn")
    };
    let oracle = run(&plain);
    assert!(
        text(&oracle.stdout).contains("(7, 30, 3)"),
        "the oracle runs the driver: {}",
        text(&oracle.stdout)
    );
    assert_matches(&run(&compiled), &oracle);
}

/// Strict mode refuses each builtin with a spanned `C0001`.
#[test]
fn strict_refuses_reaching_globals_by_a_string() {
    let dir = ScratchDir::new("issue_20_strict").expect("scratch");
    for (index, call) in ["eval('c')", "exec('c = 2')", "globals()", "locals()"]
        .iter()
        .enumerate()
    {
        let source = dir.join(format!("m{index}.py"));
        std::fs::write(&source, format!("c = 1\nprint({call})\n")).expect("write");
        let out = pycc()
            .args(["check"])
            .arg(&source)
            .output()
            .expect("pycc should spawn");
        assert!(!out.status.success(), "{call}");
        let all = text(&out.stdout) + &text(&out.stderr);
        assert!(all.contains("C0001"), "{call}: {all}");
        assert!(all.contains(&format!("m{index}.py:2:")), "{call}: {all}");
    }
}
