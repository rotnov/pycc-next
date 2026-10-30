//! v0.10 accept 2: `str`-keyed dicts in `--non-strict`. Each fixture under
//! `tests/fixtures/v10_*.py` prints what it computes and the oracle is
//! `python3` (or `PYCC_PYTHON`) running it: the module built with `--ext
//! --non-strict` (debug and `--release`) and driven by Python code (its
//! `*_driver.py` fixture, else imported alone) must print the same and fail
//! the same way, and no function, method or class of it may run in CPython.
//!
//! * `v10_exact_dicts`: a CPython dict compiled code reads, tests or stores
//!   into goes through the dict API when it is an exact `dict` (its
//!   `KeyError`s, tuple keys and unhashable keys as `dict`'s own) and through
//!   its own methods when it is a subclass (`__missing__`, overridden
//!   `__getitem__`/`__contains__`/`__setitem__`, `OrderedDict`,
//!   `defaultdict`); a `str` literal crosses as CPython's interned string.
//! * `v10_native_dicts`: a function's `str`-keyed display of objects that
//!   never escapes is a native dict (insertion order, overwrites, `in`,
//!   `len`, truth, `KeyError`, the iteration's `RuntimeError`, no leaked
//!   reference or dict); one that may escape (returned, passed, stored,
//!   compared, pickled, viewed) is CPython's, identity and all.
//! * `v10_native_dicts_refused`: a function the checker does not admit with
//!   a native dict keeps its CPython dict, the module's others native.

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

fn ext(name: &str, flags: &[&str]) {
    let compiled = ScratchDir::new(&format!("{name}_ext")).expect("scratch");
    let source = fixture(name, &compiled, "m.py");
    let mut all = vec!["--ext", "--non-strict"];
    all.extend_from_slice(flags);
    build(&source, &compiled.join("m"), &all);
    std::fs::remove_file(&source).expect("import the artifact, not the source");
    let plain = ScratchDir::new(&format!("{name}_ext_src")).expect("scratch");
    fixture(name, &plain, "m.py");
    let driver = format!("{name}_driver");
    let has_driver = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(format!("tests/fixtures/{driver}.py"))
        .exists();
    if has_driver {
        fixture(&driver, &compiled, "driver.py");
        fixture(&driver, &plain, "driver.py");
    }
    let run = |dir: &Path| {
        python()
            .args(["-c", if has_driver { "import driver" } else { "import m" }])
            .current_dir(dir)
            .output()
            .expect("python should spawn")
    };
    assert_matches(&run(&compiled), &run(&plain));
}

#[test]
fn exact_dicts_ext() {
    ext("v10_exact_dicts", &[]);
}

#[test]
fn exact_dicts_ext_release() {
    ext("v10_exact_dicts", &["--release"]);
}

/// The functions of `name`'s `--ext --non-strict` build whose own code
/// calls `callee` (from the LLVM IR `PYCC_DUMP_LLVM` writes).
fn functions_calling(name: &str, callee: &str) -> Vec<String> {
    let dir = ScratchDir::new(&format!("{name}_ir")).expect("scratch");
    let source = fixture(name, &dir, "m.py");
    let ir = dir.join("m.ll");
    let build = pycc()
        .args(["build", "--ext", "--non-strict"])
        .arg(&source)
        .arg("-o")
        .arg(dir.join("m"))
        .env("PYCC_DUMP_LLVM", &ir)
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", text(&build.stderr));
    let dump = std::fs::read_to_string(dir.join("m.ll.pre.ll")).expect("the IR dump");
    let mut out: Vec<String> = Vec::new();
    let mut current: Option<String> = None;
    for line in dump.lines() {
        if let Some(rest) = line.strip_prefix("define ") {
            current = rest
                .split_once("@pyfn_")
                .and_then(|(_, name)| name.split_once('('))
                .map(|(name, _)| name.to_string());
        } else if line.starts_with('}') {
            current = None;
        } else if let Some(function) = &current
            && line.contains(&format!("@{callee}("))
            && !out.contains(function)
        {
            out.push(function.clone());
        }
    }
    out
}

#[test]
fn native_dicts_ext() {
    ext("v10_native_dicts", &[]);
}

#[test]
fn native_dicts_ext_release() {
    ext("v10_native_dicts", &["--release"]);
}

/// The dicts that never escape are native; every one that may is CPython's.
#[test]
fn native_dicts_are_exactly_the_non_escaping_ones() {
    let native = functions_calling("v10_native_dicts", "pycc_rt_dict_new");
    for function in ["tally", "order", "tests", "missing", "grow", "nested", "rebound", "narrowed", "bad_key"] {
        assert!(
            native.iter().any(|f| f == function),
            "{function} should build a native dict: {native:?}"
        );
    }
    for function in ["returned", "passed", "stored", "compared", "pickled", "viewed", "duplicated", "raising_value", "subclass_key"] {
        assert!(
            !native.iter().any(|f| f == function),
            "{function} must keep CPython's dict: {native:?}"
        );
    }
}

/// A function the checker does not admit with a native dict (a CPython
/// object as a key) keeps its CPython dict and runs as CPython does; the
/// module's other functions keep theirs native.
#[test]
fn a_function_that_does_not_check_with_native_dicts_keeps_cpython_dicts() {
    ext("v10_native_dicts_refused", &[]);
    let native = functions_calling("v10_native_dicts_refused", "pycc_rt_dict_new");
    assert_eq!(native, vec!["local".to_string()]);
}
