//! Untyped code using module functions as values and testing objects
//! against constant displays (`tests/fixtures/object_function_values.py`),
//! as pypng's `undo_filter` does: built `--non-strict` as a native program
//! and as an `--ext` module imported by CPython, each compared with CPython
//! 3.13 (`PYCC_PYTHON`, default `python3`). A function value is one
//! callable object called through CPython's vectorcall protocol (a wrong
//! argument count is CPython's `TypeError`); a constant tuple or list a
//! membership test reads is built once per site, and an item's `__eq__`
//! still runs against every element in order.

#[path = "stdlib_support/mod.rs"]
mod support;

use pycc_scratch::ScratchDir;
use std::process::Command;
use support::{fixture, outcome, pycc, python, rendered, run_native};

#[test]
#[ignore = "requires a CPython 3.13+ oracle and Python.h (PYCC_PYTHON, default python3)"]
fn function_values_and_constant_displays_match_cpython() {
    let source = fixture("object_function_values.py");
    let expected = outcome(
        &Command::new(python())
            .arg(&source)
            .output()
            .expect("the oracle runs"),
    );
    assert_eq!(expected.status, Some(0), "{expected:?}");
    let native = run_native(
        "object_function_values",
        &source,
        &["--release", "--non-strict"],
    );
    assert_eq!(native, expected, "--non-strict");
    let dir = ScratchDir::new("object_function_values_ext").expect("scratch");
    let build = pycc()
        .arg("build")
        .arg(&source)
        .arg("-o")
        .arg(dir.join("object_function_values"))
        .args(["--ext", "--non-strict", "--release"])
        .output()
        .expect("pycc spawns");
    assert!(build.status.success(), "{}", rendered(&build));
    let ext = outcome(
        &Command::new(python())
            .current_dir(&*dir)
            .args(["-c", "import object_function_values"])
            .output()
            .expect("python3 runs"),
    );
    assert_eq!(ext, expected, "--ext --non-strict");
}
