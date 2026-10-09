//! #1380 (Part 2 of #1138): a buffer-carrier from-import.
//!
//! `from numpy import ndarray` and `from numpy.typing import NDArray` -- the
//! exact pairs, absolute, unaliased -- are admitted as CPython-backed
//! imports that run in the host but bind a hidden name, so the spelling
//! keeps the buffer-carrier meaning it has without any import (#1129,
//! #1134). Every other use of the spelling in such a module is refused with
//! `C0001` at its own span: it would mean the CPython object in CPython and
//! the buffer carrier in pycc (D-244's #1380 amendment). Every other
//! resolved-spelling from-import keeps its refusal.
//!
//! The `pycc check` and lock tests resolve on the program before any host
//! toolchain probe, so they run in the coverage job. The hosted tests are
//! `#[ignore]`d: they build an extension against a stub `numpy` package on
//! a host-only `PYTHONPATH` (no real numpy is needed; the stdlib
//! `array.array('d')` is the buffer exporter) and compare the extension's
//! import with CPython's own import of the same source.

use pycc_scratch::ScratchDir;
use std::path::Path;
use std::process::{Command, Output};

fn pycc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
}

fn oracle() -> Command {
    Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
}

fn stdout_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n")
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).replace("\r\n", "\n")
}

/// Writes each `(relative path, text)` file under `dir`, creating parents.
fn write_files(dir: &Path, files: &[(&str, &str)]) {
    for (path, text) in files {
        let path = dir.join(path);
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("create the parent");
        std::fs::write(&path, text).expect("write a fixture file");
    }
}

/// `pycc check m.py` in a fresh directory holding `m.py` = `body` and
/// `files`. Returns the exit code and the rendered output, and asserts the
/// hidden binding name never reaches it.
fn check(tag: &str, body: &str, files: &[(&str, &str)], extra: &[&str]) -> (Option<i32>, String) {
    let dir = ScratchDir::new(tag).expect("scratch");
    write_files(&dir, files);
    write_files(&dir, &[("m.py", body)]);
    let output = pycc()
        .arg("check")
        .args(extra)
        .arg("m.py")
        .current_dir(&*dir)
        .output()
        .expect("pycc should spawn");
    let rendered = format!("{}{}", stdout_of(&output), stderr_of(&output));
    assert!(!rendered.contains("$carrier"), "{rendered}");
    (output.status.code(), rendered)
}

fn assert_clean(tag: &str, body: &str) {
    let (code, rendered) = check(tag, body, &[], &[]);
    assert_eq!(code, Some(0), "{rendered}");
}

/// `pycc check` of `body` fails with exactly one `code` diagnostic whose
/// text contains `needle`, located at `location` (`m.py:L:C`).
fn assert_one_error(
    tag: &str,
    body: &str,
    extra: &[&str],
    code: &str,
    needle: &str,
    location: &str,
) {
    let (status, rendered) = check(tag, body, &[], extra);
    assert_eq!(status, Some(1), "{rendered}");
    assert_eq!(rendered.matches("error[").count(), 1, "{rendered}");
    assert!(rendered.contains(&format!("error[{code}]: ")), "{rendered}");
    assert!(rendered.contains(needle), "{rendered}");
    assert!(
        rendered.contains(&format!(" --> {location}\n")),
        "{rendered}"
    );
}

const READ_REFUSAL: &str = "outside a type annotation is not supported yet: `from numpy";

#[test]
fn both_carrier_pairs_check_cleanly_with_annotated_defs() {
    assert_clean(
        "carrier_both_pairs",
        "from numpy.typing import NDArray\nfrom numpy import ndarray\n\n\
         def total(a: NDArray) -> float:\n    s: float = 0.0\n    i: int = 0\n\
         \x20   for i in range(len(a)):\n        s = s + a[i]\n    return s\n\n\
         def first(b: ndarray[float]) -> float:\n    return b[0]\n",
    );
}

#[test]
fn a_mixed_statement_checks_cleanly() {
    assert_clean(
        "carrier_mixed",
        "from numpy import ndarray, float64\n\n\
         def first(b: ndarray) -> float:\n    return b[0]\n",
    );
}

#[test]
fn a_type_checking_guarded_carrier_import_checks_cleanly() {
    assert_clean(
        "carrier_type_checking",
        "from typing import TYPE_CHECKING\n\nif TYPE_CHECKING:\n\
         \x20   from numpy.typing import NDArray\n\n\
         def first(a: NDArray) -> float:\n    return a[0]\n",
    );
}

#[test]
fn a_read_of_the_spelling_is_refused_at_the_read() {
    assert_one_error(
        "carrier_read",
        "from numpy.typing import NDArray\n\nx = NDArray\n",
        &[],
        "C0001",
        &format!("reading `NDArray` {READ_REFUSAL}.typing import NDArray`"),
        "m.py:3:5",
    );
}

/// `isinstance` reports the carrier rule, not the buffer parameter's
/// whole-value refusal.
#[test]
fn an_isinstance_check_gets_the_carrier_diagnostic() {
    assert_one_error(
        "carrier_isinstance",
        "from numpy import ndarray\n\n\
         def f(a: int) -> bool:\n    return isinstance(a, ndarray)\n",
        &[],
        "C0001",
        &format!("reading `ndarray` {READ_REFUSAL} import ndarray`"),
        "m.py:4:26",
    );
}

#[test]
fn a_producer_call_above_the_import_is_refused() {
    assert_one_error(
        "carrier_producer_above",
        "def make(n: int) -> int:\n    a = ndarray(n)\n    return n\n\n\
         from numpy import ndarray\n",
        &[],
        "C0001",
        &format!("reading `ndarray` {READ_REFUSAL} import ndarray`"),
        "m.py:2:9",
    );
}

#[test]
fn a_binding_of_the_spelling_is_refused_at_the_binding() {
    assert_one_error(
        "carrier_binding",
        "from numpy import ndarray\n\nclass ndarray:\n    pass\n",
        &[],
        "C0001",
        "binding `ndarray` in a module that imports it with `from numpy import ndarray`",
        "m.py:3:7",
    );
}

/// Only the exact pairs are carriers: a builtin spelling and the other
/// module's spelling keep the resolved-spelling refusal.
#[test]
fn every_other_resolved_spelling_stays_refused() {
    assert_one_error(
        "carrier_builtin",
        "from builtins import range\n",
        &[],
        "C0001",
        "binding the CPython object `builtins.range` to `range`",
        "m.py:1:1",
    );
    assert_one_error(
        "carrier_other_module",
        "from numpy import NDArray\n",
        &[],
        "C0001",
        "binding the CPython object `numpy.NDArray` to `NDArray`",
        "m.py:1:1",
    );
}

/// The interop policy quotes the statement as written, never the hidden
/// binding name.
#[test]
fn a_denied_carrier_import_quotes_the_statement() {
    assert_one_error(
        "carrier_deny",
        "from numpy.typing import NDArray\n",
        &["--interop-policy", "deny"],
        "I0402",
        "`from numpy.typing import NDArray` is a CPython-backed import",
        "m.py:1:1",
    );
}

/// The hidden name is never a top-level name of the module, so a sibling
/// that imports the spelling from it gets the ordinary missing-name
/// diagnostic.
#[test]
fn a_sibling_cannot_import_the_spelling_from_a_carrier_module() {
    let (code, rendered) = check(
        "carrier_sibling",
        "from a import ndarray\n",
        &[("a.py", "from numpy import ndarray\n")],
        &[],
    );
    assert_eq!(code, Some(1), "{rendered}");
    assert!(
        rendered.contains("error[T0021]: module `a` (`a.py`) has no top-level name `ndarray`"),
        "{rendered}"
    );
    assert!(rendered.contains(" --> m.py:1:1\n"), "{rendered}");
}

/// An embedded build of a carrier import needs the lock, like any
/// non-standard CPython import (shape of `tests/issue_1138_*`'s numpy
/// case; no buffer parameter, so `I0405` cannot fire first).
#[test]
fn an_embedded_build_of_a_carrier_import_needs_the_lock() {
    let dir = ScratchDir::new("carrier_lock").expect("scratch");
    write_files(
        &dir,
        &[("m.py", "from numpy.typing import NDArray\nprint(1)\n")],
    );
    let output = pycc()
        .arg("build")
        .arg(dir.join("m.py"))
        .arg("-o")
        .arg(dir.join("m"))
        .output()
        .expect("pycc should spawn");
    assert_eq!(output.status.code(), Some(2), "{}", stderr_of(&output));
    let rendered = stderr_of(&output);
    assert!(
        rendered.contains("imports `numpy` from outside the standard library"),
        "{rendered}"
    );
    assert!(rendered.contains("run `pycc lock"), "{rendered}");
    assert!(!rendered.contains("$carrier"), "{rendered}");
}

// ---- hosted -------------------------------------------------------------

/// A stub `numpy` package: `ndarray` (subscriptable, since CPython 3.13
/// evaluates a `def`'s annotations eagerly) and `float64` in the package,
/// `NDArray` in `numpy.typing`.
const STUB_INIT: (&str, &str) = (
    "hostlib/numpy/__init__.py",
    "class ndarray:\n    def __class_getitem__(cls, item):\n        return cls\n\n\
     float64 = float\n",
);
const STUB_TYPING: (&str, &str) = ("hostlib/numpy/typing.py", "NDArray = object\n");

/// Builds `body` as the extension module `module` inside `dir`.
fn build_ext(dir: &Path, module: &str, body: &str) {
    write_files(dir, &[("m.py", body)]);
    let build = pycc()
        .arg("build")
        .arg(dir.join("m.py"))
        .arg("-o")
        .arg(dir.join(module))
        .arg("--ext")
        .env("PYTHONPATH", dir.join("hostlib"))
        .output()
        .expect("pycc should spawn");
    assert!(
        build.status.success(),
        "{}{}",
        stdout_of(&build),
        stderr_of(&build)
    );
}

/// Runs `script` in `dir` with the stub on the host-only `PYTHONPATH`.
fn python(dir: &Path, script: &str) -> Output {
    oracle()
        .arg("-c")
        .arg(script)
        .current_dir(dir)
        .env("PYTHONPATH", dir.join("hostlib"))
        .env("PYTHONUNBUFFERED", "1")
        .output()
        .expect("python3 should spawn")
}

fn assert_ok(run: &Output, what: &str) {
    assert!(
        run.status.success(),
        "{what} -- stdout: {}\nstderr: {}",
        stdout_of(run),
        stderr_of(run)
    );
}

/// Runs `script` against `body` built by pycc as `module`, and against
/// `body` imported as `module` by CPython, with the same `stub` files, and
/// asserts both print `expected`.
fn assert_matches_cpython(
    tag: &str,
    module: &str,
    body: &str,
    stub: &[(&str, &str)],
    script: &str,
    expected: &str,
) {
    let hosted = ScratchDir::new(tag).expect("scratch");
    write_files(&hosted, stub);
    build_ext(&hosted, module, body);
    let run = python(&hosted, script);
    assert_ok(&run, "pycc");
    assert_eq!(stdout_of(&run), expected, "pycc on {body}");

    let reference = ScratchDir::new(&format!("{tag}_cpython")).expect("scratch");
    write_files(&reference, stub);
    write_files(&reference, &[(&format!("{module}.py"), body)]);
    let cpython = python(&reference, script);
    assert_ok(&cpython, "CPython");
    assert_eq!(stdout_of(&cpython), expected, "CPython on {body}");
}

/// Imports `module` and prints what it raised: the type, the message
/// without its trailing ` (<path>)`, and `.name`.
fn import_report(module: &str) -> String {
    format!(
        "try:\n\
         \x20   import {module}\n\
         except ImportError as e:\n\
         \x20   print(type(e).__name__, str(e).split(' (')[0], e.name)\n\
         else:\n\
         \x20   print('no error')\n"
    )
}

/// Both pairs, the mixed statement and a block-site duplicate import run in
/// the host, keep the module's print order, bind no hidden name in the
/// module dict, and take an `array.array('d')` at each annotated parameter.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_carrier_import_runs_in_the_host_and_keeps_the_annotation() {
    let module = "pycc_carrier_ok_mod";
    assert_matches_cpython(
        "carrier_hosted_ok",
        module,
        "print('before')\nfrom numpy.typing import NDArray\nfrom numpy import ndarray, float64\n\
         if True:\n    from numpy import ndarray\nprint('after')\n\n\
         def total(a: NDArray) -> float:\n    s: float = 0.0\n    i: int = 0\n\
         \x20   for i in range(len(a)):\n        s = s + a[i]\n    return s\n\n\
         def first(b: ndarray[float]) -> float:\n    return b[0]\n",
        &[STUB_INIT, STUB_TYPING],
        &format!(
            "import array\nimport {module} as m\n\
             d = array.array('d', [1.5, 2.5, 2.0])\n\
             print(m.total(d))\nprint(m.first(d))\n\
             print([k for k in vars(m) if k.startswith('$')])\n"
        ),
        "before\nafter\n6.0\n1.5\n[]\n",
    );
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_missing_typing_module_raises_cpythons_error_after_the_earlier_print() {
    let module = "pycc_carrier_no_typing_mod";
    assert_matches_cpython(
        "carrier_hosted_no_typing",
        module,
        "print('before')\nfrom numpy.typing import NDArray\n\n\
         def first(a: NDArray) -> float:\n    return a[0]\n",
        &[STUB_INIT],
        &import_report(module),
        "before\nModuleNotFoundError No module named 'numpy.typing' numpy.typing\n",
    );
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_missing_carrier_name_raises_cpythons_error() {
    let module = "pycc_carrier_no_ndarray_mod";
    assert_matches_cpython(
        "carrier_hosted_no_ndarray",
        module,
        "from numpy import ndarray\n\ndef first(b: ndarray) -> float:\n    return b[0]\n",
        &[("hostlib/numpy/__init__.py", "float64 = float\n")],
        &import_report(module),
        "ImportError cannot import name 'ndarray' from 'numpy' numpy\n",
    );
}

/// Each name of the mixed statement gets its own `IMPORT_FROM`: the carrier
/// binds, then the missing `float64` raises.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_mixed_statement_fails_on_its_missing_name() {
    let module = "pycc_carrier_no_float64_mod";
    assert_matches_cpython(
        "carrier_hosted_no_float64",
        module,
        "from numpy import ndarray, float64\n\ndef first(b: ndarray) -> float:\n    return b[0]\n",
        &[("hostlib/numpy/__init__.py", "class ndarray:\n    pass\n")],
        &import_report(module),
        "ImportError cannot import name 'float64' from 'numpy' numpy\n",
    );
}
