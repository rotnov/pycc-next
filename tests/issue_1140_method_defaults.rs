//! The method part of #1140: a method -- `__init__`, a regular method, a
//! `@staticmethod` or a `@classmethod` -- may declare default parameter
//! values (`docs/TYPE_SYSTEM.md`, "Call surface"), and a host calling the
//! `--ext` artifact may omit those arguments (`docs/RUNTIME.md`, the
//! generated wrappers).
//!
//! The native tests pin what an in-module program sees: the defaulted
//! method compiles and runs when every argument is passed, and a short
//! in-module constructor call keeps its arity `T0021` (Part 4 of #884,
//! #1191). A short regular-method call is filled from its defaults since
//! Part 1 of #1191 (#1438, `tests/issue_1438_method_call_defaults.rs`). The
//! hosted tests are `#[ignore]`d and contribute no line coverage; the
//! Tier-1 `native-build-test` leg runs them with
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

/// `pycc <command>` on `body` written as `m.py` in a fresh scratch directory.
fn pycc_on(command: &str, tag: &str, body: &str) -> Output {
    let dir = ScratchDir::new(tag).expect("scratch");
    pycc()
        .arg(command)
        .arg(write(&dir, "m.py", body))
        .output()
        .expect("pycc should spawn")
}

const CLASS: &str = "class S:\n\
    \x20   def __init__(self, n: int, label: str = 'x') -> None:\n\
    \x20       self.n = n\n        self.label = label\n\
    \x20   def feed(self, token: int, is_end: bool = False) -> int:\n\
    \x20       if is_end:\n            return token + self.n\n        return token\n\
    \x20   @staticmethod\n    def make(x: int = 3) -> int:\n        return x * 2\n\
    \x20   @classmethod\n    def scaled(cls, f: float = -1.5) -> float:\n        return f * 2.0\n";

#[test]
fn a_defaulted_method_runs_natively_when_every_argument_is_passed() {
    let output = pycc_on(
        "run",
        "1140_native_full",
        &format!(
            "{CLASS}s = S(5, 'y')\nprint(s.label, s.feed(3, True), s.feed(3, False))\n\
             print(S.make(4), S.scaled(1.0))\n"
        ),
    );
    assert_eq!(output.status.code(), Some(0), "{}", stderr_of(&output));
    assert_eq!(stdout_of(&output), "y 8 3\n8 2.0\n");
}

#[test]
fn a_short_constructor_call_keeps_its_arity_error_and_a_method_call_reports_the_range() {
    for (tail, message) in [
        ("S()\n", "`S` expects 2 argument(s), got 0"),
        ("S(1)\n", "`S` expects 2 argument(s), got 1"),
        // Part 1 of #1191 fills omitted method defaults, not a missing
        // required argument.
        (
            "S(1, 'a').feed()\n",
            "`feed` expects from 1 to 2 argument(s), got 0",
        ),
    ] {
        let output = pycc_on("check", "1140_native_short", &format!("{CLASS}{tail}"));
        let rendered = stdout_of(&output);
        assert_eq!(output.status.code(), Some(1), "{rendered}");
        assert!(rendered.contains("error[T0021]"), "{rendered}");
        assert!(rendered.contains(message), "{rendered}");
    }
}

#[test]
fn a_mutable_method_default_is_still_refused() {
    let output = pycc_on(
        "check",
        "1140_native_mutable",
        "class S:\n    def m(self, a: list[int] = []) -> None:\n        return\n",
    );
    let rendered = stdout_of(&output);
    assert_eq!(output.status.code(), Some(1), "{rendered}");
    assert!(rendered.contains("error[C0001]"), "{rendered}");
    assert!(
        rendered.contains("only a literal `int`, `float`, `bool`, `str`, or `None` default"),
        "{rendered}"
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

/// Builds `body` as the extension `m`, runs `script` against it and against
/// the same source imported as `m.py` by CPython, asserts both succeed with
/// the same stdout, and returns it.
fn assert_matches_cpython(tag: &str, body: &str, script: &str) -> String {
    let compiled_dir = ScratchDir::new(tag).expect("scratch");
    let source_dir = ScratchDir::new(&format!("{tag}_src")).expect("scratch");
    let source = write(&source_dir, "m.py", body);
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

/// A host omits each defaulted argument of every method kind, passes it
/// explicitly, and gets CPython's `TypeError` class for a count outside the
/// range (the message text differs from CPython's own and is not compared).
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn the_host_may_omit_a_defaulted_method_argument() {
    let stdout = assert_matches_cpython(
        "1140_hosted_kinds",
        CLASS,
        "import m\n\
         s = m.S(5)\n\
         print(s.feed(3), s.feed(3, True), s.feed(3, False))\n\
         print(m.S.make(), m.S.make(4), m.S.scaled(), m.S.scaled(1.0))\n\
         print(m.S(5, 'y').feed(1, True))\n\
         for f, args in [(m.S, ()), (m.S, (1, 'a', 2)), (s.feed, ()), (s.feed, (1, 2, 3)),\n\
         \x20               (m.S.make, (1, 2))]:\n\
         \x20   try:\n        f(*args)\n\
         \x20   except TypeError:\n        print('TypeError')\n",
    );
    assert_eq!(
        stdout,
        "3 8 3\n6 8 -3.0 2.0\n6\nTypeError\nTypeError\nTypeError\nTypeError\nTypeError\n"
    );
}

/// The #1207 subject's constructor shape (lark
/// `lark/parsers/lalr_parser_state.py`, `ParserState.__init__`): two
/// `Any = None` parameters a host may omit, read back as the host's own
/// `None` or the object it passed.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn an_any_parameter_defaults_to_the_host_s_none() {
    let stdout = assert_matches_cpython(
        "1140_hosted_any_none",
        "from typing import Any\n\
         class ParserState:\n\
         \x20   def __init__(self, parse_conf: int, debug: bool, state_stack: Any = None, \
         value_stack: Any = None) -> None:\n\
         \x20       self.parse_conf = parse_conf\n        self.debug = debug\n\
         \x20       self.state_stack = state_stack\n        self.value_stack = value_stack\n\
         \x20   def states(self) -> Any:\n        return self.state_stack\n\
         \x20   def values(self) -> Any:\n        return self.value_stack\n",
        "import m\n\
         a = m.ParserState(1, False)\n\
         print(a.states() is None, a.values() is None)\n\
         stack = [0]\n\
         b = m.ParserState(1, True, stack)\n\
         print(b.states() is stack, b.values() is None)\n\
         c = m.ParserState(1, True, stack, 'v')\n\
         print(c.states(), c.values())\n",
    );
    assert_eq!(stdout, "True True\nTrue True\n[0] v\n");
}

/// A `str` default carrying a quote, a backslash, a newline and a non-ASCII
/// character, and an `int` default at the bottom of the boundary's inline
/// range (#1040), read back exactly. A default outside that range is
/// refused with the same `OverflowError` as the literal passed explicitly
/// (`docs/RUNTIME.md`).
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn an_escaped_str_and_an_extreme_int_default_read_back_exactly() {
    let stdout = assert_matches_cpython(
        "1140_hosted_literals",
        "class K:\n\
         \x20   @staticmethod\n\
         \x20   def text(s: str = 'a\"b\\\\c\\n\\u00e9') -> str:\n        return s\n\
         \x20   @staticmethod\n\
         \x20   def low(n: int = -4611686018427387904) -> int:\n        return n\n",
        "import m\nprint(repr(m.K.text()), m.K.low())\n",
    );
    assert_eq!(stdout, "'a\"b\\\\c\\né' -4611686018427387904\n");
}

/// A `Generic[T]` base is erased (#1394), and the class's defaulted
/// `__init__` and method keep their defaults at the boundary -- the #1207
/// subject's `ParserState` is generic. A `bool` default at an `int`
/// parameter reads back as the `bool` itself, as in CPython.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_generic_class_and_a_bool_at_int_default_match_cpython() {
    let stdout = assert_matches_cpython(
        "1140_hosted_generic",
        "from typing import Any, Generic, TypeVar\n\
         T = TypeVar('T')\n\
         class P(Generic[T]):\n\
         \x20   def __init__(self, n: int, stack: Any = None, flag: bool = True) -> None:\n\
         \x20       self.n = n\n        self.stack = stack\n        self.flag = flag\n\
         \x20   def get(self) -> Any:\n        return self.stack\n\
         \x20   def on(self, k: int = 7) -> int:\n\
         \x20       return self.n + k if self.flag else self.n\n\
         \x20   @staticmethod\n    def one(n: int = True) -> int:\n        return n\n",
        "import m\n\
         p = m.P(1)\n\
         print(p.get(), p.on(), m.P(2, [3], False).get(), m.P(2, [3], False).on(1))\n\
         print(repr(m.P.one()), repr(m.P.one(5)))\n",
    );
    assert_eq!(stdout, "None 8 [3] 2\nTrue 5\n");
}

/// An `int` default one past the top of the boundary's inline range
/// (#1040): omitting the argument raises the same `OverflowError` as
/// passing that literal explicitly (`docs/RUNTIME.md`). pycc-only, since
/// CPython has no such bound; an in-range argument still runs.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn an_out_of_range_int_default_raises_overflow_error_when_omitted() {
    let compiled_dir = ScratchDir::new("1140_hosted_overflow").expect("scratch");
    let source_dir = ScratchDir::new("1140_hosted_overflow_src").expect("scratch");
    let source = write(
        &source_dir,
        "m.py",
        "class K:\n\
         \x20   @staticmethod\n\
         \x20   def big(n: int = 4611686018427387904) -> int:\n        return n\n",
    );
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
    let run = host_run(
        &compiled_dir,
        "import m\n\
         for args in [(), (4611686018427387904,)]:\n\
         \x20   try:\n        m.K.big(*args)\n\
         \x20   except OverflowError:\n        print('OverflowError')\n\
         print(m.K.big(5))\n",
    );
    assert!(
        run.status.success(),
        "{}{}",
        stdout_of(&run),
        stderr_of(&run)
    );
    assert_eq!(stdout_of(&run), "OverflowError\nOverflowError\n5\n");
}
