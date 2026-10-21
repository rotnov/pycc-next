//! v0.9 accept 1: native hot state in `--non-strict`. Each fixture under
//! `tests/fixtures/v09_*.py` prints what it computes and the oracle is
//! `python3` (or `PYCC_PYTHON`) running it: the module built with `--ext
//! --non-strict` (debug and `--release`) and imported (and, where the
//! fixture runs no statement in CPython, a `--non-strict` native build) must
//! print the same and fail the same way, and no function, method or class
//! of it may run in CPython.
//!
//! * `v09_object_slots`: a list or dict slot holding CPython objects is a
//!   CPython list or dict (`pycc_types::empty_container`'s class phase), so
//!   the class stays native and Python code storing through the slot is
//!   seen by its methods; a private class is published in non-strict mode,
//!   and its methods -- with instance parameters too -- answer Python code
//!   holding one of its instances.

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
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
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
    assert_eq!(text(&pycc_scratch::lf(&ours.stdout)), text(&pycc_scratch::lf(&oracle.stdout)), "{}", text(&pycc_scratch::lf(&ours.stderr)));
    assert_eq!(ours.status.code(), oracle.status.code(), "{}", text(&pycc_scratch::lf(&ours.stderr)));
    assert_eq!(last_line(&pycc_scratch::lf(&ours.stderr)), last_line(&pycc_scratch::lf(&oracle.stderr)));
}

/// Builds `source` with `flags`, asserting it builds and that no function,
/// method or class runs in CPython.
fn build(source: &Path, out: &Path, flags: &[&str]) -> String {
    let build = pycc()
        .args(["build", "--report-fallbacks"])
        .args(flags)
        .arg(source)
        .arg("-o")
        .arg(out)
        .output()
        .expect("pycc should spawn");
    let report = text(&pycc_scratch::lf(&build.stderr));
    assert!(build.status.success(), "{report}");
    for unit in ["fallback: function", "fallback: method", "fallback: class"] {
        assert!(!report.contains(unit), "nothing should run in CPython:\n{report}");
    }
    report
}

#[allow(dead_code)]
fn native(name: &str, flags: &[&str]) {
    let dir = ScratchDir::new(&format!("{name}_native")).expect("scratch");
    let source = fixture(name, &dir, "m.py");
    let mut all = vec!["--non-strict"];
    all.extend_from_slice(flags);
    build(&source, &dir.join("app"), &all);
    let ours = Command::new(dir.join("app")).output().expect("run the build");
    let oracle = python().arg(&source).output().expect("python should spawn");
    assert_matches(&ours, &oracle);
}

fn ext(name: &str, flags: &[&str]) {
    let compiled = ScratchDir::new(&format!("{name}_ext")).expect("scratch");
    let source = fixture(name, &compiled, "m.py");
    let mut all = vec!["--ext", "--non-strict"];
    all.extend_from_slice(flags);
    build(&source, &compiled.join("m"), &all);
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
fn object_slots_and_private_classes_ext() {
    ext("v09_object_slots", &[]);
}

#[test]
fn object_slots_and_private_classes_ext_release() {
    ext("v09_object_slots", &["--release"]);
}

#[test]
fn callable_slots_sentinels_and_mixed_helpers_ext() {
    ext("v09_callable_slots", &[]);
}

#[test]
fn callable_slots_sentinels_and_mixed_helpers_ext_release() {
    ext("v09_callable_slots", &["--release"]);
}
