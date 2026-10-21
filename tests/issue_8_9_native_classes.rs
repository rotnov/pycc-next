//! Issues #8 and #9: a `--non-strict` native build publishes its compiled
//! classes in `__main__`, as an `--ext` module publishes them.
//!
//! #8: a module-level statement running in CPython no longer sends every
//! compiled class of the module to CPython; a class stays native when Python
//! code can use every member of it (`tests/fixtures/issue_8_native_classes.py`),
//! and one with a member it could not (an unannotated method) still runs in
//! CPython. #9: with no fallback code at all, CPython code reading
//! `type(obj).method` of a compiled instance finds the method
//! (`tests/fixtures/issue_9_type_self.py`).
//!
//! The oracle is `python3` (or `PYCC_PYTHON`) running each fixture: the
//! native build, and the fixture built with `--ext --non-strict` and
//! imported, print the same and fail with the same exception.

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
    assert_eq!(text(&ours.stdout), text(&oracle.stdout), "{}", text(&ours.stderr));
    assert_eq!(ours.status.code(), oracle.status.code(), "{}", text(&ours.stderr));
    assert_eq!(last_line(&ours.stderr), last_line(&oracle.stderr));
}

/// Builds `source` with `flags` and returns the `--report-fallbacks` report.
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

/// The fixture as a `--non-strict` native program against `python3`;
/// returns the report.
fn native(name: &str) -> String {
    let dir = ScratchDir::new(&format!("{name}_native")).expect("scratch");
    let source = fixture(name, &dir, "m.py");
    let report = build(&source, &dir.join("app"), &["--non-strict"]);
    let ours = Command::new(dir.join("app")).output().expect("run the build");
    let oracle = python().arg(&source).output().expect("python should spawn");
    assert_matches(&ours, &oracle);
    report
}

/// The fixture as a `--ext --non-strict` module imported by `python3`,
/// against importing its source.
fn ext(name: &str) {
    let compiled = ScratchDir::new(&format!("{name}_ext")).expect("scratch");
    let source = fixture(name, &compiled, "m.py");
    build(&source, &compiled.join("m"), &["--ext", "--non-strict"]);
    std::fs::remove_file(&source).expect("import the artifact, not the source");
    let plain = ScratchDir::new(&format!("{name}_ext_src")).expect("scratch");
    fixture(name, &plain, "m.py");
    let run = |dir: &Path| {
        python()
            .args(["-c", "import m"])
            .current_dir(dir)
            .output()
            .expect("python should spawn")
    };
    assert_matches(&run(&compiled), &run(&plain));
}

#[test]
fn issue_8_a_fallback_statement_keeps_published_classes_native() {
    let report = native("issue_8_native_classes");
    assert!(report.contains("m.py:68-70 runs in CPython"), "{report}");
    assert!(!report.contains("class `Point`"), "Point stays native:\n{report}");
    assert!(
        report.contains("fallback: class `Loose` at ")
            && report.contains("m.py:42 runs in CPython, its methods compiled: fallback code may \
                                hold an instance of `Loose`"),
        "{report}"
    );
}

#[test]
fn issue_8_ext() {
    ext("issue_8_native_classes");
}

#[test]
fn issue_9_type_of_a_compiled_instance_has_its_methods() {
    let report = native("issue_9_type_self");
    assert!(!report.contains("fallback:"), "nothing runs in CPython:\n{report}");
}

#[test]
fn issue_9_ext() {
    ext("issue_9_type_self");
}

/// A class with a member Python code could not use compiled stays native
/// while the module has no fallback code: only a fallback statement sends
/// it to CPython.
#[test]
fn issue_8_without_fallback_code_every_class_stays_native() {
    let dir = ScratchDir::new("issue_8_no_fallback").expect("scratch");
    let source = dir.join("m.py");
    std::fs::write(
        &source,
        "class Loose:\n    def __init__(self, v):\n        self.v = v\n\n    def get(self):\n        \
         return self.v\n\n\nprint(Loose(3).get())\n",
    )
    .unwrap();
    let report = build(&source, &dir.join("app"), &["--non-strict"]);
    assert!(!report.contains("fallback:"), "{report}");
    let ours = Command::new(dir.join("app")).output().expect("run the build");
    assert_eq!(text(&ours.stdout), "3\n", "{}", text(&ours.stderr));
}
