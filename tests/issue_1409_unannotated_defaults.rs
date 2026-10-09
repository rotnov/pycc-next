//! #1409: in an `--ext` build an unannotated parameter with a literal or
//! `None` default takes the type its default implies -- `None` the opaque
//! CPython object, a literal its scalar type (`docs/TYPE_SYSTEM.md`, "Call
//! surface"; D-258's #1409 amendment) -- and a host calling the artifact may
//! omit a defaulted method argument (`docs/RUNTIME.md`).
//!
//! The diagnostic tests pin what the rule leaves alone. The hosted tests are
//! `#[ignore]`d and contribute no line coverage; the Tier-1
//! `native-build-test` leg runs them with
//! `cargo test --workspace -- --include-ignored`, comparing the extension
//! artifact against CPython's own run of the same source.

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

/// Writes `body` to `dir/<file>`.
fn write(dir: &Path, file: &str, body: &str) -> PathBuf {
    let path = dir.join(file);
    std::fs::write(&path, body).expect("write the fixture source");
    path
}

/// The artifact's file name for `-o <dir>/m`: `.pyd` on Windows and
/// `.abi3.so` elsewhere (`docs/CLI_SPEC.md`).
fn artifact_name() -> &'static str {
    if cfg!(windows) { "m.pyd" } else { "m.abi3.so" }
}

/// `pycc build --ext` (when `ext`) or `pycc check` of `body` as `m.py`.
fn compile(tag: &str, body: &str, ext: bool) -> Output {
    let dir = ScratchDir::new(tag).expect("scratch");
    let source = write(&dir, "m.py", body);
    let mut command = pycc();
    if ext {
        command
            .arg("build")
            .arg(&source)
            .arg("-o")
            .arg(dir.join("m"))
            .arg("--ext");
    } else {
        command.arg("check").arg(&source);
    }
    command.output().expect("pycc should spawn")
}

/// Asserts `body` is refused with `T0001` naming parameter `param` of `func`.
fn assert_t0001(tag: &str, body: &str, ext: bool, param: &str, func: &str) {
    let output = compile(tag, body, ext);
    let rendered = format!("{}{}", stdout_of(&output), stderr_of(&output));
    assert_eq!(output.status.code(), Some(1), "{tag}: {rendered}");
    assert!(
        rendered.contains(&format!(
            "error[T0001]: parameter `{param}` of public function `{func}` needs a type \
             annotation"
        )),
        "{tag}: {rendered}"
    );
}

#[test]
fn an_unannotated_parameter_without_a_default_keeps_t0001_under_ext() {
    assert_t0001(
        "1409_no_default",
        "class S:\n    def m(self, a) -> None:\n        return\n",
        true,
        "a",
        "m",
    );
}

#[test]
fn a_non_literal_default_keeps_t0001_under_ext() {
    assert_t0001(
        "1409_non_literal",
        "class S:\n    def m(self, a=[]) -> None:\n        return\n",
        true,
        "a",
        "m",
    );
}

#[test]
fn a_module_level_none_default_keeps_t0001_under_ext() {
    assert_t0001(
        "1409_function_none",
        "def f(a=None) -> None:\n    return\n",
        true,
        "a",
        "f",
    );
}

#[test]
fn a_native_build_keeps_t0001_for_a_defaulted_parameter() {
    for (tag, body, param, func) in [
        (
            "1409_native_method",
            "class S:\n    def copy(self, deepcopy_values=True) -> None:\n        return\n",
            "deepcopy_values",
            "copy",
        ),
        (
            "1409_native_init",
            "class S:\n    def __init__(self, state_stack=None) -> None:\n        return\n",
            "state_stack",
            "__init__",
        ),
    ] {
        assert_t0001(tag, body, false, param, func);
    }
}

// ---------------------------------------------------------------------
// Hosted: the artifact against CPython's own run of the same source.
// ---------------------------------------------------------------------

/// Runs `script` with `dir` as the working directory. Stdout is forced to
/// UTF-8 so a non-ASCII default reads back byte-exactly on a host whose
/// console encoding is not UTF-8 (Windows).
fn host_run(dir: &Path, script: &str) -> Output {
    host_python()
        .args(["-B", "-c", script])
        .current_dir(dir)
        .env("PYTHONUNBUFFERED", "1")
        .env("PYTHONIOENCODING", "utf-8")
        .output()
        .expect("python3 should spawn")
}

/// Builds `body` as the extension `m` into `compiled_dir`, asserting success.
fn build_ext(compiled_dir: &Path, source_dir: &Path, body: &str) {
    let source = write(source_dir, "m.py", body);
    let build = pycc()
        .arg("build")
        .arg(&source)
        .arg("-o")
        .arg(compiled_dir.join("m"))
        .arg("--ext")
        .output()
        .expect("pycc should spawn");
    assert!(
        build.status.success(),
        "{}{}",
        stdout_of(&build),
        stderr_of(&build)
    );
    assert!(compiled_dir.join(artifact_name()).is_file());
}

/// Builds `body` as the extension `m`, runs `script` against it and against
/// the same source imported as `m.py` by CPython, asserts both succeed with
/// the same stdout, and returns it.
fn assert_matches_cpython(tag: &str, body: &str, script: &str) -> String {
    let compiled_dir = ScratchDir::new(tag).expect("scratch");
    let source_dir = ScratchDir::new(&format!("{tag}_src")).expect("scratch");
    build_ext(&compiled_dir, &source_dir, body);
    let compiled = host_run(&compiled_dir, script);
    let oracle = host_run(&source_dir, script);
    for (what, run) in [("pycc", &compiled), ("cpython", &oracle)] {
        assert!(
            run.status.success(),
            "{what}: {}{}",
            stdout_of(run),
            stderr_of(run)
        );
    }
    assert_eq!(stdout_of(&compiled), stdout_of(&oracle));
    stdout_of(&compiled)
}

/// The #1207 subject's two signatures, unannotated as in lark
/// `lark/parsers/lalr_parser_state.py` (`ParserState.__init__`, line 40, and
/// `ParserState.copy`, line 59, whose body branches on its `bool` default
/// to choose between `deepcopy` and `copy` of a CPython list). The host
/// omits and passes each defaulted argument.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn the_subject_s_unannotated_signatures_match_cpython() {
    let stdout = assert_matches_cpython(
        "1409_hosted_subject",
        "from copy import deepcopy, copy\n\
         from typing import Any\n\
         class ParserState:\n\
         \x20   def __init__(self, parse_conf: int, debug: bool, state_stack=None, \
         value_stack=None) -> None:\n\
         \x20       self.parse_conf = parse_conf\n        self.debug = debug\n\
         \x20       self.state_stack = state_stack\n        self.value_stack = value_stack\n\
         \x20   def states(self) -> Any:\n        return self.state_stack\n\
         \x20   def values(self) -> Any:\n        return self.value_stack\n\
         \x20   def copy(self, deepcopy_values=True) -> Any:\n\
         \x20       return deepcopy(self.value_stack) if deepcopy_values else \
         copy(self.value_stack)\n",
        "import m\n\
         a = m.ParserState(1, False)\n\
         print(a.states() is None, a.values() is None)\n\
         stack = [0]\n\
         values = [[1], [2]]\n\
         b = m.ParserState(1, True, stack, values)\n\
         print(b.states() is stack, b.values() is values)\n\
         deep, shallow, default = b.copy(True), b.copy(False), b.copy()\n\
         print(deep == values, deep is values, deep[0] is values[0])\n\
         print(shallow == values, shallow is values, shallow[0] is values[0])\n\
         print(default[0] is values[0])\n",
    );
    assert_eq!(
        stdout,
        "True True\nTrue True\nTrue False False\nTrue False True\nFalse\n"
    );
}

/// Every literal kind a host may omit or pass: an `int` (negative), a
/// `float`, a non-ASCII `str` and a `bool`, on a `@staticmethod` and a
/// `@classmethod`; and a module-level `def`'s literal default filled by an
/// in-module call (a host still passes every argument of a module-level
/// export, #1194).
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn each_literal_default_kind_matches_cpython() {
    let stdout = assert_matches_cpython(
        "1409_hosted_literals",
        "class K:\n\
         \x20   @staticmethod\n\
         \x20   def s(n=-4, x=0.25, flag=False) -> float:\n\
         \x20       if flag:\n            return x\n        return x * n\n\
         \x20   @staticmethod\n    def t(t='\\u00e9!') -> str:\n        return t + t\n\
         \x20   @classmethod\n    def c(cls, n=7) -> int:\n        return n + 1\n\
         def f(a=3, b='z') -> str:\n    return b * a\n\
         def g() -> str:\n    return f() + f(1)\n",
        "import m\n\
         print(m.K.s(), m.K.s(2), m.K.s(2, 1.5), m.K.s(2, 1.5, True))\n\
         print(m.K.t(), m.K.t('ab'))\n\
         print(m.K.c(), m.K.c(1))\n\
         print(m.g(), m.f(2, 'y'))\n",
    );
    assert_eq!(stdout, "-1.0 0.5 3.0 1.5\né!é! abab\n8 2\nzzzz yy\n");
}

/// The recorded deviation (D-258's #1409 amendment): a scalar-typed
/// unannotated parameter is checked at the boundary exactly as its
/// annotated twin, so a host value of another type raises `TypeError` where
/// CPython, which checks nothing, would run. pycc-only by construction.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_non_conforming_host_value_raises_type_error() {
    let compiled_dir = ScratchDir::new("1409_hosted_deviation").expect("scratch");
    let source_dir = ScratchDir::new("1409_hosted_deviation_src").expect("scratch");
    build_ext(
        &compiled_dir,
        &source_dir,
        "class S:\n\
         \x20   def copy(self, deepcopy_values=True) -> bool:\n\
         \x20       return deepcopy_values\n",
    );
    let run = host_run(
        &compiled_dir,
        "import m\n\
         s = m.S()\n\
         print(s.copy(), s.copy(False))\n\
         for value in [0, None, 'yes']:\n\
         \x20   try:\n        s.copy(value)\n\
         \x20   except TypeError:\n        print('TypeError')\n",
    );
    assert!(
        run.status.success(),
        "{}{}",
        stdout_of(&run),
        stderr_of(&run)
    );
    assert_eq!(
        stdout_of(&run),
        "True False\nTypeError\nTypeError\nTypeError\n"
    );
}
