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

/// `except` naming an imported exception class (`from json import
/// JSONDecodeError`) tests the caught exception against the CPython class at
/// run time (`pycc_ast::object_except_handlers`), as an attribute
/// (`except json.JSONDecodeError`) already did.
#[test]
fn imported_exception_classes_in_except_native() {
    native("v09_imported_except", &[]);
}

#[test]
fn imported_exception_classes_in_except_ext() {
    ext("v09_imported_except", &[]);
}

/// Builds fixture `name` as `m` with `--ext --non-strict` and runs fixture
/// `host` (Python code importing `m`) against the artifact and against the
/// source: the same output and failure.
fn ext_hosted(name: &str, host: &str, flags: &[&str]) {
    let compiled = ScratchDir::new(&format!("{name}_hosted")).expect("scratch");
    let source = fixture(name, &compiled, "m.py");
    let mut all = vec!["--ext", "--non-strict"];
    all.extend_from_slice(flags);
    build(&source, &compiled.join("m"), &all);
    std::fs::remove_file(&source).expect("import the artifact, not the source");
    fixture(host, &compiled, "host.py");
    let plain = ScratchDir::new(&format!("{name}_hosted_src")).expect("scratch");
    fixture(name, &plain, "m.py");
    fixture(host, &plain, "host.py");
    let run = |dir: &Path| {
        python()
            .arg("host.py")
            .current_dir(dir)
            .output()
            .expect("python should spawn")
    };
    assert_matches(&run(&compiled), &run(&plain));
}

/// `*args` / `**kwargs` of CPython objects: a constructor's `*items`
/// (`Py_tp_init` binds the call as CPython does, keywords included), a
/// static method's and a module function's (published through a wrapper
/// of the declared signature, `pycc_ast::ext_signatures`).
#[test]
fn object_varargs_native() {
    native("v09_varargs", &[]);
}

#[test]
fn object_varargs_from_python_ext() {
    ext_hosted("v09_varargs", "v09_varargs_host", &[]);
}

/// A nested function with unannotated parameters used as a value -- passed
/// to `map`, returned, called by CPython's `sorted` -- is a native closure
/// over its stable captures, its parameters CPython objects
/// (`pycc_ast::desugar::closures::values`).
#[test]
fn closure_values_native() {
    native("v09_closure_values", &[]);
}

#[test]
fn closure_values_ext() {
    ext("v09_closure_values", &[]);
}

#[test]
fn closure_values_ext_release() {
    ext("v09_closure_values", &["--release"]);
}

/// Raising a compiled exception class with a CPython object argument
/// (`raise MissingError(name)`) is built by CPython, which keeps the object
/// in `args`: the function raising it runs in CPython (it failed codegen).
#[test]
fn raise_of_a_compiled_exception_with_an_object_argument_ext() {
    let compiled = ScratchDir::new("v09_raise_object_ext").expect("scratch");
    let source = fixture("v09_raise_object", &compiled, "m.py");
    let build = pycc()
        .args(["build", "--ext", "--non-strict", "--report-fallbacks"])
        .arg(&source)
        .arg("-o")
        .arg(compiled.join("m"))
        .output()
        .expect("pycc should spawn");
    let report = text(&pycc_scratch::lf(&build.stderr));
    assert!(build.status.success(), "{report}");
    assert!(
        report.contains("raising `MissingError` with a CPython object argument"),
        "{report}"
    );
    std::fs::remove_file(&source).expect("import the artifact, not the source");
    let plain = ScratchDir::new("v09_raise_object_src").expect("scratch");
    fixture("v09_raise_object", &plain, "m.py");
    let host = "import m\ntry:\n    m.lookup({}, 5)\nexcept m.MissingError as e:\n    print(repr(e), e.args)\n";
    let run = |dir: &Path| {
        python()
            .args(["-c", host])
            .current_dir(dir)
            .output()
            .expect("python should spawn")
    };
    assert_matches(&run(&compiled), &run(&plain));
}

#[test]
fn object_varargs_from_python_ext_release() {
    ext_hosted("v09_varargs", "v09_varargs_host", &["--release"]);
}
