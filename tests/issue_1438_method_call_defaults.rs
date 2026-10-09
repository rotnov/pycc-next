//! Part 1 of #1191 (issue #1438): an in-module call of a regular instance
//! method may omit the method's defaulted trailing parameters, and pycc
//! fills them from the `def` (`docs/TYPE_SYSTEM.md`, "Call surface").
//!
//! The native tests run the program and compare its stdout with CPython's
//! own output for the same source, recorded here. They cover an omitted
//! and a supplied default, an inherited method, an override that declares
//! a different default, a user method named like a builtin container
//! method (`pop`), and the #1207 lark subject's `Generic` class whose
//! `__copy__` calls `self.copy()`. The diagnostic tests pin the arity
//! `T0021` a short call still gets, and that the call shapes Part 1 leaves
//! alone (a static or class method, `super()`) keep their message byte-identical.
//!
//! The hosted test is `#[ignore]`d and contributes no line coverage; the
//! Tier-1 `native-build-test` leg runs it with
//! `cargo test --workspace -- --include-ignored`.

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

/// `pycc <command>` on `body` written as `m.py` in a fresh scratch directory.
fn pycc_on(command: &str, tag: &str, body: &str) -> Output {
    let dir = ScratchDir::new(tag).expect("scratch");
    pycc()
        .arg(command)
        .arg(write(&dir, "m.py", body))
        .output()
        .expect("pycc should spawn")
}

/// `pycc check` on `body` must fail with exactly one `T0021` carrying
/// `message`.
#[track_caller]
fn assert_arity_error(tag: &str, body: &str, message: &str) {
    let output = pycc_on("check", tag, body);
    let rendered = stdout_of(&output);
    assert_eq!(output.status.code(), Some(1), "{rendered}");
    assert!(rendered.contains("error[T0021]"), "{rendered}");
    assert!(rendered.contains(message), "{message:?} in {rendered}");
}

const BASE: &str = "class Base:\n\
    \x20   def __init__(self, n: int) -> None:\n        self.n = n\n\
    \x20   def feed(self, token: int, is_end: bool = False) -> int:\n\
    \x20       if is_end:\n            return token + self.n\n        return token\n\
    \x20   def label(self, prefix: str = 'p', suffix: str = '!') -> str:\n\
    \x20       return prefix + suffix\n\
    \x20   def scaled(self, width: float = 2.0) -> float:\n        return width * self.n\n\
    \x20   def m(self, k: int = 1) -> int:\n        return k\n\
    \x20   def f(self) -> int:\n        return self.m()\n\
    \x20   def pop(self, index: int = -1) -> int:\n        return self.n * index\n\
    class Derived(Base):\n\
    \x20   def m(self, k: int = 7) -> int:\n        return k * 10\n\
    class Leaf(Base):\n    pass\n";

/// The lark `lalr_parser_state.py` shape: a `Generic` class whose
/// `__copy__` calls `self.copy()`, omitting `deepcopy_values`.
const PARSER_STATE: &str = "from typing import Generic, TypeVar\n\
    StateT = TypeVar('StateT')\n\
    class ParserState(Generic[StateT]):\n\
    \x20   def __init__(self, depth: int) -> None:\n        self.depth = depth\n\
    \x20   def __copy__(self) -> 'ParserState[StateT]':\n        return self.copy()\n\
    \x20   def copy(self, deepcopy_values: bool = True) -> 'ParserState[StateT]':\n\
    \x20       if deepcopy_values:\n            return ParserState(self.depth + 1)\n\
    \x20       return ParserState(self.depth)\n";

#[test]
fn an_omitted_method_default_is_filled_like_cpython() {
    let output = pycc_on(
        "run",
        "1438_native_fill",
        &format!(
            "{BASE}b = Base(5)\n\
             print(b.feed(3), b.feed(3, True), b.feed(3, False))\n\
             print(b.label(), b.label('q'), b.label('r', '?'), b.scaled(), b.scaled(0.5))\n\
             print(b.m(), b.m(4), b.f())\n\
             print(Derived(1).m(), Derived(1).f(), Leaf(2).m(), Leaf(2).f())\n\
             print(b.pop(), b.pop(2))\n"
        ),
    );
    assert_eq!(output.status.code(), Some(0), "{}", stderr_of(&output));
    // CPython 3.14 prints exactly this for the same source.
    assert_eq!(
        stdout_of(&output),
        "3 8 3\np! q! r? 10.0 2.5\n1 4 1\n70 70 1 1\n-5 10\n"
    );
}

#[test]
fn the_lark_copy_shape_fills_its_default() {
    let output = pycc_on(
        "run",
        "1438_native_copy",
        &format!(
            "{PARSER_STATE}s: ParserState[int] = ParserState(1)\n\
             print(s.copy().depth, s.copy(False).depth, s.__copy__().depth)\n"
        ),
    );
    assert_eq!(output.status.code(), Some(0), "{}", stderr_of(&output));
    assert_eq!(stdout_of(&output), "2 1 2\n");
}

#[test]
fn a_short_call_missing_a_required_argument_reports_the_range() {
    assert_arity_error(
        "1438_too_few",
        &format!("{BASE}Base(1).feed()\n"),
        "`feed` expects from 1 to 2 argument(s), got 0",
    );
}

#[test]
fn a_call_with_too_many_arguments_reports_the_range() {
    assert_arity_error(
        "1438_too_many",
        &format!("{BASE}Base(1).feed(1, True, 3)\n"),
        "`feed` expects from 1 to 2 argument(s), got 3",
    );
}

#[test]
fn a_method_without_defaults_keeps_the_exact_arity_message() {
    assert_arity_error(
        "1438_plain",
        &format!("{BASE}Base(1).f(1)\n"),
        "`f` expects 0 argument(s), got 1",
    );
}

#[test]
fn a_short_static_class_method_or_super_call_keeps_its_arity_error() {
    const S: &str = "class S:\n    @staticmethod\n    def make(x: int = 3) -> int:\n        return x\n\
        \x20   @classmethod\n    def scaled(cls, f: float = -1.5) -> float:\n        return f\n";
    for (tail, message) in [
        ("print(S.make())\n", "`make` expects 1 argument(s), got 0"),
        ("print(S().make())\n", "`make` expects 1 argument(s), got 0"),
        (
            "print(S.scaled())\n",
            "`scaled` expects 1 argument(s), got 0",
        ),
        (
            "print(S().scaled())\n",
            "`scaled` expects 1 argument(s), got 0",
        ),
    ] {
        assert_arity_error("1438_static", &format!("{S}{tail}"), message);
    }
    assert_arity_error(
        "1438_super",
        "class B:\n    def m(self, k: int = 1) -> int:\n        return k\n\
         class D(B):\n    def m(self, k: int = 2) -> int:\n        return super().m()\n\
         print(D().m())\n",
        "`m` expects 1 argument(s), got 0",
    );
}

// ---------------------------------------------------------------------
// Hosted: the artifact against CPython's own run of the same source.
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
        .env("PYTHONIOENCODING", "utf-8")
        .output()
        .expect("python3 should spawn")
}

/// The hosted variant of the lark shape. A public method returning its
/// own class cannot cross the `--ext` boundary yet (`C0003`), so the
/// self-returning `copy` is private here and a public `int` method reaches
/// it with the defaulted argument omitted.
const HOSTED_PARSER_STATE: &str = "from typing import Generic, TypeVar\n\
    StateT = TypeVar('StateT')\n\
    class ParserState(Generic[StateT]):\n\
    \x20   def __init__(self, depth: int) -> None:\n        self.depth = depth\n\
    \x20   def copied_depth(self) -> int:\n        return self._copy().depth\n\
    \x20   def kept_depth(self) -> int:\n        return self._copy(False).depth\n\
    \x20   def _copy(self, deepcopy_values: bool = True) -> 'ParserState[StateT]':\n\
    \x20       if deepcopy_values:\n            return ParserState(self.depth + 1)\n\
    \x20       return ParserState(self.depth)\n";

/// A host call reaches a compiled method whose in-module `self._copy()`
/// omits `deepcopy_values`.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_host_copy_reaches_the_filled_in_module_call() {
    let compiled_dir = ScratchDir::new("1438_hosted_copy").expect("scratch");
    let source_dir = ScratchDir::new("1438_hosted_copy_src").expect("scratch");
    let source = write(&source_dir, "m.py", HOSTED_PARSER_STATE);
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
    let script = "import m\n\
                  s = m.ParserState(1)\n\
                  print(s.copied_depth(), s.kept_depth())\n";
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
    assert_eq!(stdout_of(&compiled), "2 1\n");
}
