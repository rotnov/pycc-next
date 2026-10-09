//! #1418: `return NotImplemented` in a comparison method of an `--ext`
//! module.
//!
//! The hosted test builds the source with `pycc build --ext`, imports the
//! artifact into the host CPython, and compares what a driver script prints
//! against CPython's own run of the same source (`runpy.run_path`). It is
//! `#[ignore]`d and contributes no line coverage; the Tier-1
//! `native-build-test` leg runs it with `cargo test --workspace --
//! --include-ignored`. The changed lines are covered by the unit tests in
//! `crates/pycc_hir/src/not_implemented_tests.rs`,
//! `crates/pycc_types/src/tests/not_implemented.rs` and
//! `crates/pycc_codegen/src/tests/not_implemented.rs`; the two refusal pins
//! below need no CPython, because the front end refuses before the build
//! probes for one.

use pycc_scratch::ScratchDir;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn pycc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
}

fn host_python() -> Command {
    Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
}

fn stdout_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n")
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).replace("\r\n", "\n")
}

fn write(dir: &Path, file: &str, body: &str) -> PathBuf {
    let path = dir.join(file);
    std::fs::write(&path, body).expect("write the fixture source");
    path
}

fn build(dir: &Path, module: &str, body: &str) -> Output {
    pycc()
        .arg("build")
        .arg(write(dir, "m.py", body))
        .arg("-o")
        .arg(dir.join(module))
        .arg("--ext")
        .output()
        .expect("pycc should spawn")
}

fn python(dir: &Path, script: &str) -> Output {
    host_python()
        .arg("-c")
        .arg(script)
        .current_dir(dir)
        .output()
        .expect("python3 should spawn")
}

fn assert_ok(run: &Output) {
    assert!(
        run.status.success(),
        "stdout: {}\nstderr: {}",
        stdout_of(run),
        stderr_of(run)
    );
}

/// The lark `lalr_parser_state.py` shape: an `__eq__` annotated `-> bool`
/// that returns `NotImplemented` for an operand it does not handle, and an
/// object comparison otherwise.
const SOURCE: &str = "from typing import Any\n\
    \n\
    \n\
    class C:\n\
    \x20   def __init__(self, v: Any) -> None:\n\
    \x20       self.v = v\n\
    \n\
    \x20   def __eq__(self, other: Any) -> bool:\n\
    \x20       if not isinstance(other, int):\n\
    \x20           return NotImplemented\n\
    \x20       return self.v == other\n\
    \n\
    \x20   def eq(self, o: Any) -> Any:\n\
    \x20       return self.__eq__(o)\n";

/// Prints what `C(3).eq(...)` returns, by type and by identity with
/// `NotImplemented`, for two handled operands and an unhandled one. `ns` is
/// the module's namespace.
///
/// It calls `__eq__` through `eq`, which pins the method's own return; a
/// host `==` runs the same body through the carrier type's
/// `tp_richcompare` since #1427 (`tests/issue_1427_ext_richcompare.rs`).
fn report(load: &str) -> String {
    format!(
        "{load}\n\
         c = ns['C'](3)\n\
         for arg in (3, 4, 'x'):\n\
         \x20   r = c.eq(arg)\n\
         \x20   print(type(r).__name__, r is NotImplemented)\n"
    )
}

const REPORT_OUT: &str = "bool False\nbool False\nNotImplementedType True\n";

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn not_implemented_from_a_comparison_method_behaves_like_cpython_in_the_host() {
    let dir = ScratchDir::new("not_implemented_hosted").expect("scratch");
    let built = build(&dir, "pycc_not_implemented_mod", SOURCE);
    assert!(built.status.success(), "{}", stderr_of(&built));
    let compiled = python(
        &dir,
        &report("import pycc_not_implemented_mod\nns = vars(pycc_not_implemented_mod)"),
    );
    assert_ok(&compiled);
    let oracle = python(&dir, &report("import runpy\nns = runpy.run_path('m.py')"));
    assert_ok(&oracle);
    assert_eq!(stdout_of(&compiled), stdout_of(&oracle));
    assert_eq!(stdout_of(&compiled), REPORT_OUT);
}

/// The subject's whole `ParserState.__eq__` (lines 51-54): an unannotated
/// operand, an `isinstance` guard against the pycc class itself, and a final
/// `return` that is an `and` of a native `bool` with an object comparison.
/// That last `return` stayed `T0022` (an object into the `-> bool` slot)
/// until the `return NotImplemented` widened the method's return to the
/// object.
const SUBJECT_SOURCE: &str = "from typing import Any\n\
    \n\
    \n\
    class ParserState:\n\
    \x20   def __init__(self, stack: Any) -> None:\n\
    \x20       self.state_stack = stack\n\
    \n\
    \x20   @property\n\
    \x20   def position(self) -> Any:\n\
    \x20       return self.state_stack[-1]\n\
    \n\
    \x20   def __eq__(self, other) -> bool:\n\
    \x20       if not isinstance(other, ParserState):\n\
    \x20           return NotImplemented\n\
    \x20       return len(self.state_stack) == len(other.state_stack) \
    and self.position == other.position\n\
    \n\
    \x20   def eq(self, o: Any) -> Any:\n\
    \x20       return self.__eq__(o)\n";

/// Prints what the subject's `__eq__` returns for operands it does not
/// handle. A handled operand (another `ParserState`), whose fields the
/// method reads through the object, is covered by
/// `tests/issue_1442_compiled_field_read.rs`.
fn subject_report(load: &str) -> String {
    format!(
        "{load}\n\
         s = ns['ParserState']([1, 2])\n\
         for arg in (3, 'x', None):\n\
         \x20   r = s.eq(arg)\n\
         \x20   print(type(r).__name__, r is NotImplemented)\n"
    )
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn the_subjects_whole_eq_builds_and_refuses_an_unhandled_operand_like_cpython() {
    let dir = ScratchDir::new("not_implemented_subject").expect("scratch");
    let built = build(&dir, "pycc_not_implemented_subject", SUBJECT_SOURCE);
    assert!(built.status.success(), "{}", stderr_of(&built));
    let compiled = python(
        &dir,
        &subject_report(
            "import pycc_not_implemented_subject\nns = vars(pycc_not_implemented_subject)",
        ),
    );
    assert_ok(&compiled);
    let oracle = python(
        &dir,
        &subject_report("import runpy\nns = runpy.run_path('m.py')"),
    );
    assert_ok(&oracle);
    assert_eq!(stdout_of(&compiled), stdout_of(&oracle));
    assert_eq!(
        stdout_of(&compiled),
        "NotImplementedType True\nNotImplementedType True\nNotImplementedType True\n"
    );
}

#[test]
fn not_implemented_elsewhere_in_an_ext_module_is_refused() {
    let dir = ScratchDir::new("not_implemented_refused").expect("scratch");
    let output = build(
        &dir,
        "pycc_not_implemented_refused",
        "def f() -> object:\n    return NotImplemented\n",
    );
    assert_eq!(output.status.code(), Some(1), "{}", stderr_of(&output));
    let rendered = format!("{}{}", stdout_of(&output), stderr_of(&output));
    assert!(
        rendered.contains(
            "error[C0001]: `NotImplemented` is not supported here yet: an `--ext` module \
             admits it only as `return NotImplemented` in a class's `__eq__`, `__ne__`, \
             `__lt__`, `__le__`, `__gt__` or `__ge__` method (#1418)"
        ),
        "{rendered}"
    );
}

#[test]
fn a_native_build_keeps_its_unbound_name_error() {
    let dir = ScratchDir::new("not_implemented_native").expect("scratch");
    let output = pycc()
        .arg("check")
        .arg(write(
            &dir,
            "m.py",
            "class C:\n    def __init__(self, v: int) -> None:\n        self.v = v\n\n    \
             def __eq__(self, other: C) -> bool:\n        return NotImplemented\n",
        ))
        .output()
        .expect("pycc should spawn");
    assert_eq!(output.status.code(), Some(1), "{}", stderr_of(&output));
    let rendered = stdout_of(&output);
    assert_eq!(rendered.matches("error[").count(), 1, "{rendered}");
    assert!(
        rendered.contains("error[T0021]: name `NotImplemented` is not defined"),
        "{rendered}"
    );
}
