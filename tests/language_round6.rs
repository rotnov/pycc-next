//! The generator protocol and full call signatures.
//!
//! * `send()`, `throw()`, `close()`, a `yield` used as a value, `return
//!   value` (`StopIteration.value`), `yield from` delegating all three and
//!   returning the subgenerator's value, PEP 479, and generators held in
//!   module globals closed by `del` and at exit;
//! * `*args`, keyword-only parameters, `**kwargs`, positional-only
//!   parameters and defaults of every kind, for functions, methods,
//!   constructors and nested functions, called with keywords, `*seq` and
//!   `**mapping`, with CPython's `TypeError` messages;
//! * function values stored in lists, dicts and instance attributes and
//!   called from there, and functions and generators printed as CPython
//!   prints them.
//!
//! The oracle is CPython itself: each fixture is built, run, and compared
//! byte for byte with `python3` running the file -- stdout, the uncaught
//! exception's last line and the exit status -- except that an object's
//! address (`<function f at 0x7f...>`) is compared as `0x...`: no compiled
//! program reproduces CPython's heap addresses.

use pycc_scratch::ScratchDir;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn pycc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
}

fn python() -> Command {
    Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

/// `text` with every ` at 0x<hex>` address written ` at 0x...`.
fn without_addresses(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(" at 0x") {
        let (head, tail) = rest.split_at(at + " at 0x".len());
        out.push_str(head);
        out.push_str("...");
        rest = tail.trim_start_matches(|c: char| c.is_ascii_hexdigit());
    }
    out.push_str(rest);
    out
}

fn last_line(bytes: &[u8]) -> String {
    text(bytes).lines().last().unwrap_or_default().to_string()
}

fn same_run(ours: &Output, theirs: &Output, name: &str) {
    assert_eq!(
        without_addresses(&text(&ours.stdout)),
        without_addresses(&text(&theirs.stdout)),
        "stdout differs ({name})"
    );
    assert_eq!(last_line(&ours.stderr), last_line(&theirs.stderr), "{name}");
    assert_eq!(ours.status.code(), theirs.status.code(), "{name}");
}

/// Builds fixture `name` (plus `flags`) in both profiles, runs it and
/// compares with CPython. Returns the build's stderr.
fn matches_cpython(name: &str, flags: &[&str]) -> String {
    let mut report = String::new();
    for release in [false, true] {
        let dir = ScratchDir::new("language_round6").expect("scratch");
        let out = dir.join("prog");
        let mut build = pycc();
        build
            .arg("build")
            .arg(fixture(name))
            .arg("-o")
            .arg(&out)
            .args(flags);
        if release {
            build.arg("--release");
        }
        let build = build.output().expect("pycc should spawn");
        assert!(
            build.status.success(),
            "{}{}",
            text(&build.stdout),
            text(&build.stderr)
        );
        report = text(&build.stderr);
        let ours = Command::new(&out).output().expect("the program runs");
        let theirs = python().arg(fixture(name)).output().expect("python3 runs");
        same_run(&ours, &theirs, name);
    }
    report
}

/// `pycc check` of `source`, its whole output.
fn check(source: &str) -> String {
    let dir = ScratchDir::new("language_round6_check").expect("scratch");
    let path = dir.join("subject.py");
    std::fs::write(&path, source).expect("write");
    let out = pycc()
        .arg("check")
        .arg(&path)
        .output()
        .expect("pycc should spawn");
    format!("{}{}", text(&out.stdout), text(&out.stderr))
}

/// `send()`/`throw()`/`close()`, `yield` as a value, `return value`,
/// `yield from` delegation, PEP 479, `GeneratorExit` handling, and
/// generators closed by `del`, by the last reference and at exit.
#[test]
fn the_generator_protocol_matches_cpython() {
    matches_cpython("generator_protocol.py", &[]);
}

/// The same program compiles natively in non-strict mode.
#[test]
fn the_generator_protocol_compiles_natively_in_non_strict_mode() {
    let report = matches_cpython(
        "generator_protocol.py",
        &["--non-strict", "--report-fallbacks"],
    );
    assert!(!report.contains("fallback:"), "{report}");
}

/// `*args`, keyword-only parameters, `**kwargs`, positional-only
/// parameters and defaults of every kind, called with keywords, `*seq` and
/// `**mapping`, and the `TypeError` of every call that cannot bind.
#[test]
fn call_signatures_match_cpython() {
    matches_cpython("call_signatures.py", &[]);
}

#[test]
fn call_signatures_compile_natively_in_non_strict_mode() {
    let report = matches_cpython(
        "call_signatures.py",
        &["--non-strict", "--report-fallbacks"],
    );
    assert!(!report.contains("fallback:"), "{report}");
}

/// Function values in lists, dicts and attributes, called from there, and
/// functions and generators printed as CPython prints them.
#[test]
fn function_values_in_containers_and_attributes_match_cpython() {
    matches_cpython("function_values.py", &[]);
}

#[test]
fn function_values_compile_natively_in_non_strict_mode() {
    let report = matches_cpython(
        "function_values.py",
        &["--non-strict", "--report-fallbacks"],
    );
    assert!(!report.contains("fallback:"), "{report}");
}

#[test]
fn an_address_is_compared_as_an_ellipsis() {
    assert_eq!(
        without_addresses("<function f at 0x7f1a2b> and <generator object g at 0xAB0>"),
        "<function f at 0x...> and <generator object g at 0x...>"
    );
}

/// What stays refused is refused with a spanned `C0001`, never miscompiled.
#[test]
fn unsupported_generator_shapes_are_refused() {
    for (source, message) in [
        (
            "from typing import Iterator\n\n\ndef f() -> Iterator[int]:\n    x = yield from [1, 2]\n    print(x)\n",
            "`yield from` a `list[int]` is not supported yet",
        ),
        (
            "from typing import Iterator\n\n\ndef f() -> Iterator[int]:\n    xs = [(yield i) for i in range(3)]\n",
            "expression kind not supported yet: a `yield` expression",
        ),
        (
            "try:\n    print(1)\nexcept StopIteration as e:\n    print(e.value)\n",
            "`.value` of a caught `StopIteration` is supported only when",
        ),
    ] {
        let output = check(source);
        assert!(output.contains("error[C0001]"), "{source}: {output}");
        assert!(output.contains(message), "{source}: {output}");
    }
}

/// A value of the wrong type sent into or returned from a generator is a
/// type error.
#[test]
fn generator_send_and_return_types_are_checked() {
    for (source, message) in [
        (
            "from typing import Generator\n\n\ndef g() -> Generator[int, int, None]:\n    x = yield 1\n    print(x)\n\n\nit = g()\nnext(it)\nit.send(\"s\")\n",
            "`send()` to a generator taking `int` values, got `str`",
        ),
        (
            "from typing import Generator\n\n\ndef g() -> Generator[int, None, str]:\n    yield 1\n    return 2\n",
            "expected a generator return value of type `str`, got `int`",
        ),
    ] {
        let output = check(source);
        assert!(output.contains(message), "{source}: {output}");
    }
}
