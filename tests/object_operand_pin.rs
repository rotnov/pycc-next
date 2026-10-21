//! An `object` operation keeps the module globals it operates on alive
//! while it runs CPython code (`operand_pin`): CPython's C API borrows its
//! operands for the call and relies on the caller's reference, so a hook
//! that rebinds the global holding an operand must not free it under the
//! operation.
//!
//! The `--ext` probe (`fixtures/object_operand_pin.py`) runs each
//! operation on module globals and on an attribute of a compiled instance,
//! in the module body and in a function; every hook of the stub re-execs
//! the module beneath the operation, which rebinds every global, and then
//! keeps using its operands -- as does CPython after the hook (the
//! reflected `__radd__`/`__eq__`, the dict probe after `__hash__`). Without
//! the pins, the comparison, arithmetic, subscript, membership and item
//! store read freed objects. The native embedded probe
//! (`fixtures/object_operand_pin_fallback.py`) rebinds the globals from
//! non-strict fallback code instead. Both run under CPython's development
//! mode and debug allocator and must print exactly what CPython prints;
//! repeated calls must leave live objects and reference counts unchanged.
//!
//! The tests are `#[ignore]`d (they need a CPython 3.13 with development
//! headers); CI runs them with `-- --include-ignored`.
#![cfg(unix)]

use pycc_scratch::ScratchDir;
use std::path::Path;
use std::process::{Command, Output};

const STUB: &str = include_str!("fixtures/object_operand_pin_stub.py");
const MODULE: &str = include_str!("fixtures/object_operand_pin.py");
const HOST: &str = include_str!("fixtures/object_operand_pin_host.py");
const FALLBACK: &str = include_str!("fixtures/object_operand_pin_fallback.py");

const DEBUG_ENV: [(&str, &str); 1] = [("PYTHONMALLOC", "debug")];

fn python() -> Command {
    Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
}

fn checked(output: Output, what: &str) -> String {
    let stdout = pycc_scratch::lf(&*String::from_utf8_lossy(&output.stdout)).replace("\r\n", "\n");
    assert!(
        output.status.success(),
        "{what} failed: {:?}\nstdout: {stdout}\nstderr: {}",
        output.status,
        pycc_scratch::lf(&*String::from_utf8_lossy(&output.stderr))
    );
    stdout
}

fn build(dir: &Path, source: &Path, out: &str, ext: bool) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_pycc"));
    command
        .arg("build")
        .arg(source)
        .arg("-o")
        .arg(dir.join(out))
        .arg("--non-strict")
        .arg("--release");
    if ext {
        command.arg("--ext");
    }
    checked(command.output().expect("pycc should spawn"), "pycc build");
}

fn host(dir: &Path, module: &str) -> String {
    let output = python()
        .args(["-X", "dev"])
        .arg(dir.join("host.py"))
        .arg(module)
        .envs(DEBUG_ENV)
        .current_dir(dir)
        .output()
        .expect("python3 should spawn");
    checked(output, &format!("the host importing {module}"))
}

/// Every operation that runs CPython code on a module global, or on an
/// attribute a rebind releases, holds the operand across the code: the
/// output matches CPython's line for line under the debug allocator, and
/// repeated calls leak nothing.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn an_operand_rebound_by_the_code_its_operation_runs_stays_alive() {
    let dir = ScratchDir::new("object_operand_pin_ext").expect("scratch");
    std::fs::write(dir.join("object_operand_pin_stub.py"), STUB).expect("write the stub");
    std::fs::write(dir.join("host.py"), HOST).expect("write the host");
    std::fs::create_dir_all(dir.join("src")).expect("create the source directory");
    let source = dir.join("src").join("pycc_operand_pin.py");
    std::fs::write(&source, MODULE).expect("write the module");
    std::fs::write(dir.join("pycc_operand_pin_py.py"), MODULE).expect("write the oracle");
    build(&dir, &source, "pycc_operand_pin", true);
    let compiled = host(&dir, "pycc_operand_pin");
    let oracle = host(&dir, "pycc_operand_pin_py");
    assert_eq!(compiled, oracle);
    assert!(
        oracle.contains("Item.radd Item(boxed2)\n"),
        "the reflected operand of `g + box.o` ran: {oracle}"
    );
    assert!(
        oracle.ends_with("-- live 0 refs 0 0\n"),
        "nothing leaks: {oracle}"
    );
}

/// The embedded executable: fallback (CPython) code rebinds the globals
/// compiled code is operating on, and the program prints what CPython
/// prints, finalizers included.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_fallback_rebind_of_an_operand_matches_cpython_in_an_embedded_build() {
    let dir = ScratchDir::new("object_operand_pin_native").expect("scratch");
    let source = dir.join("rebind.py");
    std::fs::write(&source, FALLBACK).expect("write the program");
    build(&dir, &source, "app", false);
    let ours = Command::new(dir.join("app"))
        .envs(DEBUG_ENV)
        .output()
        .expect("run the build");
    let oracle = python()
        .args(["-X", "dev"])
        .arg(&source)
        .output()
        .expect("python3 should spawn");
    assert_eq!(
        checked(ours, "the embedded build"),
        checked(oracle, "CPython")
    );
}
