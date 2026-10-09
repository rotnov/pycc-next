//! #1389: an instance attribute holds an instance of a class of this
//! program, established by a class-body declaration (`a: _A`) or by a bare
//! `__init__` parameter of that class's type (`docs/TYPE_SYSTEM.md`,
//! "Classes"; `docs/RUNTIME.md`, the instance contract).
//!
//! The slot word is the instance pointer. `pycc_rt` never frees an instance,
//! so the store and the read carry no refcount traffic and every read aliases
//! the stored instance, as in CPython.
//!
//! The native tests run the compiled program through `pycc run` and pin the
//! stdout CPython 3.14 prints for the same source. The hosted tests are
//! `#[ignore]`d and contribute no line coverage; the Tier-1
//! `native-build-test` leg runs them with
//! `cargo test --workspace -- --include-ignored`, comparing the extension
//! artifact against CPython's own run of the same source. Every fixture
//! defines a class before any annotation names it, since CPython 3.13 (CI's
//! hosted floor) evaluates annotations eagerly.

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

/// Writes `body` to `dir/<file>`, creating its parent directories.
fn write(dir: &Path, file: &str, body: &str) -> PathBuf {
    let path = dir.join(file);
    std::fs::create_dir_all(path.parent().expect("a parent")).expect("mkdir");
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

/// `pycc check` on `body` fails with exit `1`; returns the rendered
/// diagnostics.
fn check_fails(tag: &str, body: &str) -> String {
    let dir = ScratchDir::new(tag).expect("scratch");
    let output = pycc()
        .arg("check")
        .arg(write(&dir, "m.py", body))
        .output()
        .expect("pycc should spawn");
    let rendered = stdout_of(&output);
    assert_eq!(output.status.code(), Some(1), "{tag}: {rendered}");
    rendered
}

/// The issue's own example: a declared slot established from an `__init__`
/// parameter, with attribute reads and method calls through it, from module
/// level and from a method of the holder; a store through the slot is
/// visible through the original reference and back (one object, two
/// names); a reassignment replaces only the slot.
const ISSUE_EXAMPLE: &str = "class _A:\n\
    \x20   def __init__(self, n: int):\n        self.n = n\n\
    \x20   def double(self) -> int:\n        return self.n * 2\n\
    class _B:\n    a: _A\n\
    \x20   def __init__(self, a: _A):\n        self.a = a\n\
    \x20   def get(self) -> int:\n        return self.a.double() + self.a.n\n\
    \x20   def replace(self, a: _A) -> None:\n        self.a = a\n\
    def run() -> None:\n\
    \x20   x = _A(21)\n\
    \x20   b = _B(x)\n\
    \x20   print(b.a.n, b.a.double(), b.get())\n\
    \x20   b.a.n = 7\n\
    \x20   print(x.n)\n\
    \x20   x.n = 8\n\
    \x20   print(b.a.n)\n\
    \x20   b.replace(_A(5))\n\
    \x20   print(b.a.n, x.n, b.get())\n\
    \x20   b.a = x\n\
    \x20   print(b.a.n)\n";

const ISSUE_EXAMPLE_STDOUT: &str = "21 42 63\n7\n8\n5 8 15\n8\n";

#[test]
fn a_declared_instance_slot_reads_calls_and_aliases_as_cpython() {
    assert_prints(
        "1389_issue_example",
        &format!("{ISSUE_EXAMPLE}run()\n"),
        ISSUE_EXAMPLE_STDOUT,
    );
}

/// Without a declaration the parameter's own type establishes the slot, and
/// the receiver itself is such a parameter: `self.link = self` is a cycle,
/// reached through any number of hops.
#[test]
fn an_undeclared_instance_slot_and_a_self_link_are_established() {
    assert_prints(
        "1389_undeclared",
        "class Leaf:\n\
         \x20   def __init__(self, v: int) -> None:\n        self.v = v\n\
         class Node:\n\
         \x20   def __init__(self, leaf: Leaf) -> None:\n\
         \x20       self.leaf = leaf\n        self.link = self\n\
         n = Node(Leaf(3))\n\
         print(n.link.link.leaf.v)\n\
         n.link.leaf.v = 5\n\
         print(n.leaf.v)\n",
        "3\n5\n",
    );
}

/// The declaration may name a subscripted class whose `Generic[...]` base
/// is erased (the lark `parse_conf: ParseConf[StateT]` shape) or an enum,
/// and the slot may be established from any expression (#1388), here a
/// constructor call.
#[test]
fn a_generic_or_enum_class_slot_is_established_from_any_expression() {
    assert_prints(
        "1389_generic_enum",
        "from enum import Enum\n\
         from typing import Generic, TypeVar\n\
         StateT = TypeVar('StateT')\n\
         class Color(Enum):\n    RED = 1\n    BLUE = 2\n\
         class ParseConf(Generic[StateT]):\n\
         \x20   def __init__(self, start: str) -> None:\n        self.start = start\n\
         class ParserState(Generic[StateT]):\n\
         \x20   parse_conf: ParseConf[StateT]\n    color: Color\n\
         \x20   def __init__(self, parse_conf: ParseConf[StateT]) -> None:\n\
         \x20       self.parse_conf = parse_conf\n        self.color = Color.BLUE\n\
         s = ParserState(ParseConf('start'))\n\
         print(s.parse_conf.start, s.color.name)\n",
        "start BLUE\n",
    );
}

/// An instance slot read before its assignment raises CPython's
/// `AttributeError` (#1388), like every other slot type.
#[test]
fn an_instance_slot_read_before_its_assignment_raises() {
    assert_prints(
        "1389_read_before_assign",
        "class Base:\n\
         \x20   def __init__(self, n: int) -> None:\n        self.n = n\n\
         class Early:\n    other: Base\n    n: int\n\
         \x20   def __init__(self) -> None:\n\
         \x20       self.n = self.other.n\n        self.other = Base(1)\n\
         try:\n    Early()\nexcept AttributeError as e:\n    print('AttributeError', e)\n",
        "AttributeError 'Early' object has no attribute 'other'\n",
    );
}

/// A value of another class is refused against the declared class, and a
/// protocol-typed declaration still has no slot representation.
#[test]
fn a_wrong_class_or_a_protocol_slot_is_refused() {
    let text = check_fails(
        "1389_wrong_class",
        "class A:\n    def __init__(self) -> None:\n        self.n = 1\n\
         class B:\n    def __init__(self) -> None:\n        self.m = 1\n\
         class H:\n    a: A\n\
         \x20   def __init__(self, b: B) -> None:\n        self.a = b\n",
    );
    assert!(
        text.contains("error[T0021]: cannot assign `B` to attribute `a` of type `A`"),
        "{text}"
    );
    let text = check_fails(
        "1389_protocol",
        "from typing import Protocol\n\
         class P(Protocol):\n    def f(self) -> int: ...\n\
         class H:\n    p: P\n\
         \x20   def __init__(self, p: P) -> None:\n        self.p = p\n",
    );
    assert!(
        text.contains("has type `P`, which has no instance-slot representation"),
        "{text}"
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

/// The issue's example, driven by the host through an exported function.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn the_issue_example_matches_cpython_in_an_extension() {
    let stdout =
        assert_matches_cpython("1389_hosted_example", ISSUE_EXAMPLE, "import m\nm.run()\n");
    assert_eq!(stdout, ISSUE_EXAMPLE_STDOUT);
}

/// The lark shape (`lark/parsers/lalr_parser_state.py`, MIT): a
/// `ParserState(Generic[StateT])` whose `parse_conf: ParseConf[StateT]`
/// slot holds the program's own `ParseConf`, read through by a method.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn the_lark_parse_conf_slot_matches_cpython_in_an_extension() {
    let stdout = assert_matches_cpython(
        "1389_hosted_lark",
        "from typing import Generic, TypeVar\n\
         StateT = TypeVar('StateT')\n\
         class ParseConf(Generic[StateT]):\n\
         \x20   start: str\n    start_state: int\n\
         \x20   def __init__(self, start: str, start_state: int):\n\
         \x20       self.start = start\n        self.start_state = start_state\n\
         class ParserState(Generic[StateT]):\n\
         \x20   __slots__ = 'parse_conf', 'depth'\n\
         \x20   parse_conf: ParseConf[StateT]\n    depth: int\n\
         \x20   def __init__(self, parse_conf: ParseConf[StateT], depth: int):\n\
         \x20       self.parse_conf = parse_conf\n        self.depth = depth\n\
         \x20   def position(self) -> int:\n\
         \x20       return self.parse_conf.start_state + self.depth\n\
         def build() -> None:\n\
         \x20   s = ParserState(ParseConf('start', 4), 2)\n\
         \x20   print(s.parse_conf.start, s.position())\n",
        "import m\nm.build()\n",
    );
    assert_eq!(stdout, "start 6\n");
}
