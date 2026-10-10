//! `object` values (`PyObject *`) follow the native ownership model: every
//! `object` slot -- a local, a parameter, a module global, an instance
//! attribute -- owns one reference, a produced value is owned by whoever
//! receives it, and a frame releases its slots on every exit.
//!
//! The probes run a compiled `--ext --non-strict` module and the same
//! source under CPython through one driver and compare the reports byte
//! for byte: each function's result, the change in its arguments'
//! reference counts, traced memory over repeated runs, and the resident
//! set over a million calls through the boundary. A native embedded build
//! loops a million times over object arithmetic in constant memory. The
//! probes also run under CPython's development mode and debug allocator,
//! which catch a release of freed memory and a refcount underflow.
#![cfg(unix)]

use pycc_scratch::ScratchDir;
use std::path::Path;
use std::process::{Command, Output};

const MODULE: &str = include_str!("fixtures/object_ownership.py");
const DRIVER: &str = include_str!("fixtures/object_ownership_driver.py");

fn python() -> Command {
    Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
}

fn checked(output: Output, what: &str) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n");
    assert!(
        output.status.success(),
        "{what} failed: {:?}\nstdout: {stdout}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
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

/// Runs the driver against `module` with `extra` interpreter flags and env.
fn drive(dir: &Path, module: &str, flags: &[&str], env: &[(&str, &str)]) -> String {
    let mut command = python();
    command
        .args(flags)
        .arg(dir.join("driver.py"))
        .arg(module)
        .arg("200")
        .current_dir(dir);
    for (key, value) in env {
        command.env(key, value);
    }
    checked(command.output().expect("python3 should spawn"), module)
}

fn ext_dir(tag: &str) -> ScratchDir {
    let dir = ScratchDir::new(tag).expect("scratch dir");
    let source = dir.join("src").join("pycc_oo.py");
    std::fs::create_dir_all(source.parent().expect("a parent")).expect("mkdir");
    std::fs::write(&source, MODULE).expect("write the module");
    std::fs::write(dir.join("oo_py.py"), MODULE).expect("write the oracle module");
    std::fs::write(dir.join("driver.py"), DRIVER).expect("write the driver");
    build(&dir, &source, "pycc_oo", true);
    dir
}

/// A compiled module's reference counts, traced memory and resident set
/// match CPython's own across the `--ext` boundary in both directions.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn ext_object_arguments_and_results_keep_exact_counts() {
    let dir = ext_dir("object_ownership_ext");
    let compiled = drive(&dir, "pycc_oo", &[], &[]);
    let oracle = drive(&dir, "oo_py", &[], &[]);
    assert_eq!(compiled, oracle);
    assert!(compiled.contains("traced memory flat True\n"), "{compiled}");
    assert!(compiled.contains("rss flat True\n"), "{compiled}");
    assert!(compiled.contains("counts flat True\n"), "{compiled}");
    assert!(compiled.contains("ident Thing [0]\n"), "{compiled}");
}

/// The same probe under the debug allocator, which detects a release of
/// freed memory and a refcount underflow.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn ext_object_probe_is_clean_under_the_debug_allocator() {
    let dir = ext_dir("object_ownership_dev");
    let compiled = drive(
        &dir,
        "pycc_oo",
        &["-X", "dev"],
        &[("PYTHONMALLOC", "debug")],
    );
    let oracle = drive(&dir, "oo_py", &[], &[]);
    assert_eq!(compiled, oracle);
}

const EMBEDDED: &str = "\
import resource


def step(acc, x):
    y = acc + x * 2
    z = y - x
    return z


def run(n):
    acc = 10 ** 25
    x = 3
    i = 0
    while i < n:
        acc = step(acc, x)
        i = i + 1
    return acc


before = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss
total = run(1000000)
after = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss
print(total)
print(after - before < 4096)
";

/// A native embedded build doing object arithmetic a million times runs in
/// constant memory and prints what CPython prints.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn embedded_object_arithmetic_runs_in_constant_memory() {
    let dir = ScratchDir::new("object_ownership_embedded").expect("scratch dir");
    let source = dir.join("loop.py");
    std::fs::write(&source, EMBEDDED).expect("write the program");
    build(&dir, &source, "loop", false);
    let compiled = checked(
        Command::new(dir.join("loop")).output().expect("spawn"),
        "embedded program",
    );
    let oracle = checked(
        python()
            .arg(&source)
            .output()
            .expect("python3 should spawn"),
        "CPython",
    );
    assert_eq!(compiled, oracle);
    assert!(compiled.ends_with("True\n"), "{compiled}");
}
