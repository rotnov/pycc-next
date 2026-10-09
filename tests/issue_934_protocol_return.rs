//! #934 (PEP 544): a protocol class written as a return type annotation
//! (`def make() -> P:`) is rejected with `C0001` by
//! `pycc_hir::func::lower_return_annotation`. Before this fix every shape of
//! such a function passed `pycc check` and aborted the compiler: the issue's
//! `p: P = make(); p.foo()` panicked in `pycc_mir` ("method `foo` not
//! declared on class `P`"), passing the result to a protocol-typed parameter
//! panicked with "has no recorded type", and an unused or never-called
//! `-> P` function panicked in `pycc_codegen` ("P has no LLVM representation
//! yet").
//!
//! The unit tests beside the gate pin the code, message, and span for each
//! definition shape; these prove the binary reports the diagnostic (exit 1,
//! never 101) on the issue's own program and its sibling shapes, and that
//! the protocol positions the message names as supported still compile and
//! run.

use pycc_scratch::ScratchDir;
use std::path::{Path, PathBuf};
use std::process::Command;

fn pycc_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_pycc"))
}

fn write(root: &Path, relative: &str, contents: &str) -> PathBuf {
    let path = root.join(relative);
    std::fs::write(&path, contents).unwrap();
    path
}

const PRELUDE: &str = "from typing import Protocol\n\nclass P(Protocol):\n    def foo(self) -> int: ...\n\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n\n    def foo(self) -> int:\n        return self.x\n\n";

const MESSAGE: &str = "a protocol class (`P`) as a return type annotation is not supported yet -- a protocol type is currently supported in parameter and variable positions only";

/// Runs `pycc <subcommand>` on `entry`, asserting the compiler exits with
/// status 1 (a diagnostic, not the 101 a panic exits with), reports the
/// protocol-return `C0001`, renders the span at `line:column` of the entry
/// on the source line `header`, and never prints a panic or internal-error
/// line. `pycc check` renders its diagnostic on stdout and `pycc build` on
/// stderr, so both streams are searched together.
fn assert_protocol_return_rejected(
    dir: &ScratchDir,
    subcommand: &str,
    entry: &Path,
    line: u32,
    column: u32,
    header: &str,
) {
    let out = dir.join("prog");
    let mut cmd = Command::new(pycc_bin());
    cmd.arg(subcommand).arg(entry.to_str().unwrap());
    if subcommand == "build" {
        cmd.args(["-o", out.to_str().unwrap()]);
    }
    let result = cmd.output().unwrap();
    let stderr = String::from_utf8_lossy(&result.stderr);
    let stdout = String::from_utf8_lossy(&result.stdout);
    let combined = format!("{stdout}{stderr}");
    assert_eq!(
        result.status.code(),
        Some(1),
        "pycc {subcommand} should fail with a diagnostic, got {:?}\nstdout: {stdout}\nstderr: {stderr}",
        result.status
    );
    assert!(
        combined.contains("error[C0001]"),
        "the output should carry a C0001 diagnostic, got: {combined}"
    );
    assert!(
        combined.contains(MESSAGE),
        "the output should carry {MESSAGE:?}, got: {combined}"
    );
    // The renderer prints paths with forward slashes on every platform
    // (see `tests/slice0.rs`), so normalize the expected location the same way.
    let location = format!(
        "{}:{line}:{column}",
        entry.to_string_lossy().replace('\\', "/")
    );
    assert!(
        combined.contains(&location),
        "the diagnostic should point at the return annotation {location}, got: {combined}"
    );
    assert!(
        combined.contains(&format!("{line} | {header}")),
        "the rendered source line should be the definition header {header:?}, got: {combined}"
    );
    for forbidden in ["panicked", "internal error", "pycc_rt:"] {
        assert!(
            !combined.contains(forbidden),
            "the compiler must diagnose, not abort; found {forbidden:?} in: {combined}"
        );
    }
    assert!(!out.exists(), "a failing build must not leave a binary");
}

/// Runs `pycc run` on `entry` and asserts the program compiles and prints
/// exactly `expected_stdout`.
fn assert_runs_and_prints(entry: &Path, expected_stdout: &str) {
    let result = Command::new(pycc_bin())
        .arg("run")
        .arg(entry.to_str().unwrap())
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&result.stderr);
    let stdout = String::from_utf8_lossy(&result.stdout);
    assert!(
        result.status.success(),
        "pycc run should succeed, got {:?}\nstdout: {stdout}\nstderr: {stderr}",
        result.status
    );
    assert_eq!(stdout, expected_stdout, "stderr: {stderr}");
}

/// The issue's own reproduction: `pycc check` used to exit 0 and `pycc run`
/// used to panic in `pycc_mir`.
#[test]
fn the_issue_program_is_c0001_on_the_return_annotation() {
    let dir = ScratchDir::new("934_check").expect("failed to create scratch dir");
    let entry = write(
        &dir,
        "prog.py",
        &format!(
            "{PRELUDE}def make() -> P:\n    return C()\n\ndef main() -> None:\n    p: P = make()\n    print(p.foo())\n\nmain()\n"
        ),
    );
    assert_protocol_return_rejected(&dir, "check", &entry, 13, 15, "def make() -> P:");
}

/// `pycc build` stops at HIR lowering too, so neither the `pycc_mir` panic
/// nor the codegen one is reachable and no artifact is produced.
#[test]
fn building_the_issue_program_is_c0001_and_leaves_no_binary() {
    let dir = ScratchDir::new("934_build").expect("failed to create scratch dir");
    let entry = write(
        &dir,
        "prog.py",
        &format!(
            "{PRELUDE}def make() -> P:\n    return C()\n\ndef main() -> None:\n    p: P = make()\n    print(p.foo())\n\nmain()\n"
        ),
    );
    assert_protocol_return_rejected(&dir, "build", &entry, 13, 15, "def make() -> P:");
}

/// A `-> P` function that is never called used to reach `pycc_codegen`'s
/// "P has no LLVM representation yet" panic instead of `pycc_mir`'s.
#[test]
fn an_uncalled_protocol_returning_function_is_c0001() {
    let dir = ScratchDir::new("934_uncalled").expect("failed to create scratch dir");
    let entry = write(
        &dir,
        "prog.py",
        &format!("{PRELUDE}def make() -> P:\n    return C()\n\nprint(C().foo())\n"),
    );
    assert_protocol_return_rejected(&dir, "check", &entry, 13, 15, "def make() -> P:");
}

/// Passing the result to a protocol-typed parameter used to panic with
/// "`$fn:use` has no recorded type" -- monomorphization could not specialize
/// on a protocol-typed argument.
#[test]
fn a_protocol_returning_result_passed_to_a_protocol_parameter_is_c0001() {
    let dir = ScratchDir::new("934_param").expect("failed to create scratch dir");
    let entry = write(
        &dir,
        "prog.py",
        &format!(
            "{PRELUDE}def use(p: P) -> int:\n    return p.foo()\n\ndef make() -> P:\n    return C()\n\nprint(use(make()))\n"
        ),
    );
    assert_protocol_return_rejected(&dir, "check", &entry, 16, 15, "def make() -> P:");
}

/// A concrete class's method annotated `-> P` goes through the same seam.
#[test]
fn a_method_returning_a_protocol_is_c0001() {
    let dir = ScratchDir::new("934_method").expect("failed to create scratch dir");
    let entry = write(
        &dir,
        "prog.py",
        &format!(
            "{PRELUDE}class D:\n    def __init__(self) -> None:\n        self.y = 0\n\n    def clone(self) -> P:\n        return C()\n\nprint(D().clone().foo())\n"
        ),
    );
    assert_protocol_return_rejected(&dir, "check", &entry, 17, 24, "    def clone(self) -> P:");
}

/// The positions the message names as supported -- a protocol-typed
/// parameter, a protocol-typed local, and a protocol-typed module-level
/// variable -- still compile and run: each binds the value's concrete class
/// (D-166), so there is a real method table to dispatch through.
#[test]
fn protocol_typed_parameters_and_variables_still_run() {
    let dir = ScratchDir::new("934_negative").expect("failed to create scratch dir");
    let entry = write(
        &dir,
        "prog.py",
        &format!(
            "{PRELUDE}def use(p: P) -> int:\n    return p.foo()\n\ndef local() -> int:\n    x: P = C()\n    return x.foo()\n\nq: P = C()\nprint(use(C()))\nprint(local())\nprint(q.foo())\n"
        ),
    );
    assert_runs_and_prints(&entry, "0\n0\n0\n");
}

/// D-146's private-helper solver: an *unannotated* helper that returns its
/// protocol-typed parameter is not caught by the gate (there is no
/// annotation), and it must not need to be -- the helper has a protocol
/// parameter, so monomorphization specializes it and each copy returns the
/// concrete class. Pinned end to end so the gate is proven not to
/// over-reject the one way a protocol-typed value can still flow out of a
/// function.
#[test]
fn an_unannotated_helper_returning_its_protocol_parameter_still_runs() {
    let dir = ScratchDir::new("934_helper").expect("failed to create scratch dir");
    let entry = write(
        &dir,
        "prog.py",
        &format!("{PRELUDE}def _pass(p: P):\n    return p\n\nq = _pass(C())\nprint(q.foo())\n"),
    );
    assert_runs_and_prints(&entry, "0\n");
}
