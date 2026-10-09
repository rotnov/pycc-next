//! #1342: an unannotated private helper or dunder method whose return is a
//! constructor call `C(...)` on a non-generic user class infers `C`
//! (`docs/TYPE_SYSTEM.md`, "v0.1 local inference").
//!
//! The native tests run the compiled program through `pycc run` and pin the
//! stdout CPython prints for the same source; the refusal test pins the
//! check phase's argument diagnostic, which the solver's new term does not
//! displace. The hosted test is `#[ignore]`d and contributes no line
//! coverage; the Tier-1 `native-build-test` leg runs it with `cargo test
//! --workspace -- --include-ignored`, comparing the extension artifact
//! against CPython's own run of the same source.

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

/// `body` compiles, runs to completion and prints exactly `expected`.
fn assert_prints(tag: &str, body: &str, expected: &str) {
    let dir = ScratchDir::new(tag).expect("scratch");
    let output = pycc()
        .arg("run")
        .arg(write(&dir, "m.py", body))
        .output()
        .expect("pycc should spawn");
    assert_eq!(
        output.status.code(),
        Some(0),
        "{tag}: {}{}",
        stdout_of(&output),
        stderr_of(&output)
    );
    assert_eq!(stdout_of(&output), expected, "{tag}");
}

/// The issue's own program: CPython prints `1`.
#[test]
fn the_issue_program_prints_what_cpython_prints() {
    assert_prints(
        "1342_issue",
        "class R:\n\
         \x20   def __init__(self, v: int) -> None:\n        self.v = v\n\
         def _mk():\n    return R(1)\n\
         def main() -> None:\n    print(_mk().v)\n\
         main()\n",
        "1\n",
    );
}

/// A local bound from the call, an inherited `__init__`, a method called on
/// the constructed instance (#1420's term), and a dunder method returning a
/// fresh instance of its own class.
#[test]
fn constructor_calls_type_through_locals_subclasses_and_methods() {
    assert_prints(
        "1342_shapes",
        "class B:\n\
         \x20   def __init__(self, v: int) -> None:\n        self.v = v\n\
         \x20   def get(self) -> int:\n        return self.v * 10\n\
         \x20   def __copy__(self):\n        return B(self.v + 1)\n\
         class D(B):\n    pass\n\
         def _local():\n    r = D(4)\n    return r\n\
         def _chained():\n    return B(5).get()\n\
         print(_local().v, _local().get(), _chained(), B(7).__copy__().v)\n",
        "4 40 50 8\n",
    );
}

/// The check phase still validates the arguments against `__init__`.
#[test]
fn a_mistyped_constructor_argument_is_still_refused() {
    let dir = ScratchDir::new("1342_mistyped").expect("scratch");
    let output = pycc()
        .arg("check")
        .arg(write(
            &dir,
            "m.py",
            "class R:\n\
             \x20   def __init__(self, v: int) -> None:\n        self.v = v\n\
             def _mk():\n    return R(\"x\")\n\
             print(_mk().v)\n",
        ))
        .output()
        .expect("pycc should spawn");
    let rendered = format!("{}{}", stdout_of(&output), stderr_of(&output));
    assert_eq!(output.status.code(), Some(1), "{rendered}");
    assert!(
        rendered.contains("error[T0021]: argument 1 of `R` expects `int`, got `str`"),
        "{rendered}"
    );
    assert!(!rendered.contains("cannot infer return type"), "{rendered}");
}

/// An enum class call never reaches the solver: lowering refuses it with
/// its own `C0001`, unchanged by the constructor term.
#[test]
fn an_enum_class_call_keeps_the_lowering_refusal() {
    let dir = ScratchDir::new("1342_enum").expect("scratch");
    let output = pycc()
        .arg("check")
        .arg(write(
            &dir,
            "m.py",
            "from enum import Enum\n\n\nclass E(Enum):\n    A = 1\n\n\n\
             def _mk():\n    return E(1)\n\n\nprint(_mk())\n",
        ))
        .output()
        .expect("pycc should spawn");
    let rendered = format!("{}{}", stdout_of(&output), stderr_of(&output));
    assert_eq!(output.status.code(), Some(1), "{rendered}");
    assert!(
        rendered.contains("error[C0001]: cannot call enum class `E`"),
        "{rendered}"
    );
}

// ---------------------------------------------------------------------
// Hosted: the extension artifact against CPython's own run.
// ---------------------------------------------------------------------

/// The artifact's file name for `-o <dir>/m`: `.pyd` on Windows and
/// `.abi3.so` elsewhere (`docs/CLI_SPEC.md`).
fn artifact_name() -> &'static str {
    if cfg!(windows) { "m.pyd" } else { "m.abi3.so" }
}

/// Runs `script` with `dir` as the working directory.
fn host_run(dir: &Path, script: &str) -> Output {
    host_python()
        .args(["-B", "-c", script])
        .current_dir(dir)
        .env("PYTHONUNBUFFERED", "1")
        .output()
        .expect("python3 should spawn")
}

/// An unannotated `__copy__` constructing its own class by name, in a
/// `Generic[StateT]` class like the #1207 subject's (its erased base keeps
/// the class non-generic to pycc). The instance stays inside the module, so
/// `run` drives the copy and prints its slots.
const HOSTED: &str = "from typing import Any, Generic, TypeVar\n\
    StateT = TypeVar('StateT')\n\
    class State(Generic[StateT]):\n\
    \x20   def __init__(self, lexer: Any, depth: int) -> None:\n\
    \x20       self.lexer = lexer\n        self.depth = depth\n\
    \x20   def __copy__(self):\n        return State(self.lexer, self.depth + 1)\n\
    def _fresh(lexer: Any):\n    return State(lexer, 0)\n\
    def run(lexer: Any) -> None:\n\
    \x20   c = _fresh(lexer).__copy__()\n\
    \x20   print(c.depth, c.lexer is lexer, c.__copy__().depth)\n";

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_dunder_copy_constructing_its_class_matches_cpython_in_an_extension() {
    let compiled_dir = ScratchDir::new("1342_hosted").expect("scratch");
    let source_dir = ScratchDir::new("1342_hosted_src").expect("scratch");
    let source = write(&source_dir, "m.py", HOSTED);
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
    let script = "import m\nm.run(object())\n";
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
    assert_eq!(stdout_of(&compiled), "1 True 2\n");
}
