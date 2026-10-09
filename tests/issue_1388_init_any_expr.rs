//! #1388: a declared instance attribute is established in `__init__` from
//! any expression the body can type, and a slot read before its assignment
//! raises CPython's own `AttributeError` (`docs/TYPE_SYSTEM.md`, "Classes";
//! `docs/RUNTIME.md`, the instance contract).
//!
//! The native tests run the compiled program through `pycc run` and pin the
//! stdout CPython 3.14 prints for the same source (no attribute names here
//! have a near neighbour, so CPython's traceback adds no "Did you mean"
//! suffix). The hosted tests are `#[ignore]`d and contribute no line
//! coverage; the Tier-1 `native-build-test` leg runs them with
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

/// Writes `body` to `dir/<file>`, creating its parent directories.
fn write(dir: &Path, file: &str, body: &str) -> PathBuf {
    let path = dir.join(file);
    std::fs::create_dir_all(path.parent().expect("a parent")).expect("mkdir");
    std::fs::write(&path, body).expect("write the fixture source");
    path
}

/// `pycc run` on `body` written as `m.py` in a fresh scratch directory.
fn run(tag: &str, body: &str) -> Output {
    let dir = ScratchDir::new(tag).expect("scratch");
    pycc()
        .arg("run")
        .arg(write(&dir, "m.py", body))
        .output()
        .expect("pycc should spawn")
}

/// `body` compiles, runs to completion and prints exactly `expected`.
fn assert_prints(tag: &str, body: &str, expected: &str) {
    let output = run(tag, body);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{tag}: {}{}",
        stdout_of(&output),
        stderr_of(&output)
    );
    assert_eq!(stdout_of(&output), expected, "{tag}");
}

/// The issue's own example and its neighbours: a subscript, a builtin call,
/// arithmetic, string concatenation, and reads of slots assigned earlier in
/// `__init__` -- an `int` big enough to be heap-allocated and a `str` among
/// them, so the copied word is the aliased value CPython shares.
#[test]
fn a_declared_attribute_is_established_from_any_expression() {
    assert_prints(
        "1388_any_expr",
        "class ParseConf:\n\
         \x20   table: dict[str, int]\n    start_state: int\n    total: int\n\
         \x20   label: str\n    again: str\n    big: int\n    bigger: int\n\
         \x20   def __init__(self, table: dict[str, int], start: str) -> None:\n\
         \x20       self.table = table\n\
         \x20       self.start_state = table[start]\n\
         \x20       self.total = len(self.table) * 10 + self.start_state\n\
         \x20       self.label = start + '!'\n\
         \x20       self.again = self.label\n\
         \x20       self.big = 9223372036854775807 + len(table)\n\
         \x20       self.bigger = self.big + self.total\n\
         c = ParseConf({'a': 3, 'b': 4}, 'b')\n\
         print(c.start_state, c.total, c.label, c.again, c.big, c.bigger)\n\
         c.label = 'x'\nprint(c.again)\n",
        "4 24 b! b! 9223372036854775809 9223372036854775833\nb!\n",
    );
}

/// A slot read before its assignment raises `AttributeError` naming the
/// instance's own class -- the subclass for an inherited `__init__` -- both
/// in an `__init__` right-hand side and through a helper method it calls,
/// and with or without `__slots__`.
#[test]
fn a_slot_read_before_its_assignment_raises_attribute_error() {
    assert_prints(
        "1388_read_before_assign",
        "class C:\n    x: int\n    y: int\n\
         \x20   def __init__(self) -> None:\n        self.y = self.x\n        self.x = 1\n\
         class S:\n    __slots__ = ('x', 'y')\n    x: int\n    y: int\n\
         \x20   def __init__(self) -> None:\n        self.y = self.x + 1\n        self.x = 1\n\
         class D(C):\n    pass\n\
         class H:\n    w: str\n\
         \x20   def __init__(self) -> None:\n        self.show()\n        self.w = 'w'\n\
         \x20   def show(self) -> None:\n        print(self.w)\n\
         def make(k: int) -> None:\n\
         \x20   try:\n\
         \x20       if k == 0:\n            C()\n\
         \x20       elif k == 1:\n            S()\n\
         \x20       elif k == 2:\n            D()\n\
         \x20       else:\n            H()\n\
         \x20   except AttributeError as e:\n        print('AttributeError', e)\n\
         for k in range(4):\n    make(k)\n",
        "AttributeError 'C' object has no attribute 'x'\n\
         AttributeError 'S' object has no attribute 'x'\n\
         AttributeError 'D' object has no attribute 'x'\n\
         AttributeError 'H' object has no attribute 'w'\n",
    );
}

/// `except Exception` catches it in a module that never spells
/// `AttributeError`, so the builtin class needs no seeding.
#[test]
fn except_exception_catches_it_without_naming_attribute_error() {
    assert_prints(
        "1388_except_exception",
        "class C:\n    x: int\n    y: int\n\
         \x20   def __init__(self) -> None:\n        self.y = self.x\n        self.x = 1\n\
         def f() -> None:\n\
         \x20   try:\n        C()\n    except Exception as e:\n        print('caught', e)\n\
         f()\n",
        "caught 'C' object has no attribute 'x'\n",
    );
}

/// Uncaught, it ends the program through the uncaught-exception boundary
/// (exit `101`, `docs/CLI_SPEC.md`) with CPython's last traceback line.
#[test]
fn an_uncaught_read_before_assignment_ends_the_program() {
    let output = run(
        "1388_uncaught",
        "class C:\n    x: int\n    y: int\n\
         \x20   def __init__(self) -> None:\n        self.y = self.x\n        self.x = 1\n\
         print('before')\nC()\nprint('after')\n",
    );
    assert_eq!(output.status.code(), Some(101), "{}", stderr_of(&output));
    assert_eq!(stdout_of(&output), "before\n");
    assert!(
        stderr_of(&output).contains("AttributeError: 'C' object has no attribute 'x'"),
        "{}",
        stderr_of(&output)
    );
}

/// A monomorphised generic class reports its source name, never its
/// mangling.
#[test]
fn a_generic_class_reports_its_source_name() {
    assert_prints(
        "1388_generic_name",
        "class Box[T]:\n    n: int\n    v: T\n\
         \x20   def __init__(self, v: T) -> None:\n\
         \x20       self.show()\n        self.n = 1\n        self.v = v\n\
         \x20   def show(self) -> None:\n        print(self.n)\n\
         try:\n    b = Box[int](3)\nexcept AttributeError as e:\n    print('AttributeError', e)\n",
        "AttributeError 'Box' object has no attribute 'n'\n",
    );
}

/// #1148: a derived `__init__` that never calls the base's leaves the base's
/// slot unassigned, and reading it raises instead of reading a zero word.
#[test]
fn a_base_slot_a_derived_init_never_assigns_raises() {
    assert_prints(
        "1388_issue_1148",
        "class Base:\n\
         \x20   def __init__(self, n: int) -> None:\n        self.n = n\n\
         class Derived(Base):\n\
         \x20   def __init__(self) -> None:\n        self.m = 2\n\
         d = Derived()\nprint(d.m)\n\
         try:\n    print(d.n)\nexcept AttributeError as e:\n    print('AttributeError', e)\n",
        "2\nAttributeError 'Derived' object has no attribute 'n'\n",
    );
}

/// An undeclared attribute keeps the shape gate, whose message now names
/// the declaration that lifts it.
#[test]
fn an_undeclared_attribute_keeps_the_shape_gate() {
    let dir = ScratchDir::new("1388_undeclared").expect("scratch");
    let output = pycc()
        .arg("check")
        .arg(write(
            &dir,
            "m.py",
            "class C:\n    def __init__(self, n: int) -> None:\n        self.x = n + 1\n",
        ))
        .output()
        .expect("pycc should spawn");
    let rendered = stdout_of(&output);
    assert_eq!(output.status.code(), Some(1), "{rendered}");
    assert!(rendered.contains("error[C0001]"), "{rendered}");
    assert!(
        rendered.contains("or declare it in the class body (`x: <type>`)"),
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

/// The host catches the compiled module's `AttributeError` as CPython's own
/// class, with CPython's message: raised by an exported function, and by the
/// compiled `__init__` a host-side construction runs (`get` dates from
/// when a class with no method published no type object; since #1450 a
/// constructible one is published without it).
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn the_host_catches_a_read_before_assignment_as_attribute_error() {
    let stdout = assert_matches_cpython(
        "1388_hosted_host_catch",
        "class Late:\n    x: int\n    y: int\n\
         \x20   def __init__(self) -> None:\n        self.y = self.x\n        self.x = 1\n\
         \x20   def get(self) -> int:\n        return self.y\n\
         def make() -> None:\n    Late()\n",
        "import m\n\
         for f in (m.make, m.Late):\n\
         \x20   try:\n        f()\n\
         \x20   except AttributeError as e:\n        print(type(e) is AttributeError, str(e))\n",
    );
    assert_eq!(
        stdout,
        "True 'Late' object has no attribute 'x'\nTrue 'Late' object has no attribute 'x'\n"
    );
}

/// A foreign attribute CPython does not find raises `AttributeError`, which
/// a compiled `except AttributeError` now catches by its own class.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_compiled_handler_catches_a_foreign_attribute_error() {
    let stdout = assert_matches_cpython(
        "1388_hosted_foreign_catch",
        "from fractions import Fraction\n\
         def probe(f: Fraction) -> None:\n\
         \x20   try:\n        print(f.nowhere_to_be_found)\n\
         \x20   except AttributeError:\n        print('caught')\n\
         probe(Fraction(1, 2))\n",
        "import m\n",
    );
    assert_eq!(stdout, "caught\n");
}

/// The #1207 subject's `ParseConf` (lark `lark/parsers/lalr_parser_state.py`
/// lines 10-29, MIT), verbatim but for the elided `###{standalone` marker
/// and its sibling imports: every slot is established from a read through
/// one assigned earlier, on CPython objects (D-258), under
/// `--foreign-relative-imports`. The sibling `ParseTableBase` is generic, as
/// in lark: CPython 3.13 (CI's hosted floor) evaluates the
/// `ParseTableBase[StateT]` annotations eagerly, where 3.14 defers them.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn the_lark_parse_conf_establishes_its_slots_from_earlier_ones() {
    let dir = ScratchDir::new("1388_hosted_lark").expect("scratch");
    for (file, text) in [
        ("top/__init__.py", ""),
        ("top/pkg/__init__.py", ""),
        (
            "top/pkg/lalr_analysis.py",
            "from typing import Generic, TypeVar\nStateT = TypeVar('StateT')\n\
             class ParseTableBase(Generic[StateT]):\n\
             \x20   def __init__(self):\n\
             \x20       self.start_states = {'start': 0}\n\
             \x20       self.end_states = {'start': 9}\n\
             \x20       self.states = {0: {'A': ('shift', 1)}}\n",
        ),
        ("top/pkg/common.py", "class ParserCallbacks:\n    pass\n"),
    ] {
        write(&dir, file, text);
    }
    let body = "from typing import Dict, Any, Generic, List\n\
        from .common import ParserCallbacks\n\
        from .lalr_analysis import ParseTableBase, StateT\n\
        class ParseConf(Generic[StateT]):\n\
        \x20   __slots__ = 'parse_table', 'callbacks', 'start', 'start_state', 'end_state', 'states'\n\
        \x20   parse_table: ParseTableBase[StateT]\n\
        \x20   callbacks: ParserCallbacks\n\
        \x20   start: str\n\
        \x20   start_state: StateT\n\
        \x20   end_state: StateT\n\
        \x20   states: Dict[StateT, Dict[str, tuple]]\n\
        \x20   def __init__(self, parse_table: ParseTableBase[StateT], callbacks: ParserCallbacks, start: str):\n\
        \x20       self.parse_table = parse_table\n\
        \x20       self.start_state = self.parse_table.start_states[start]\n\
        \x20       self.end_state = self.parse_table.end_states[start]\n\
        \x20       self.states = self.parse_table.states\n\
        \x20       self.callbacks = callbacks\n\
        \x20       self.start = start\n\
        \x20   def show(self) -> None:\n\
        \x20       print(self.start_state, self.end_state, self.states, self.start)\n\
        def build() -> None:\n\
        \x20   ParseConf(ParseTableBase(), ParserCallbacks(), 'start').show()\n";
    let source = write(&dir, "src/m.py", body);
    std::fs::create_dir_all(dir.join("stage")).expect("mkdir stage");
    // Built outside the package tree: CPython's finder tries extension
    // loaders before source loaders, so an artifact already in `top/pkg`
    // would shadow the `.py` oracle.
    let build = pycc()
        .arg("build")
        .arg(&source)
        .arg("-o")
        .arg(dir.join("stage").join("m"))
        .args(["--ext", "--foreign-relative-imports"])
        .output()
        .expect("pycc should spawn");
    assert!(
        build.status.success(),
        "{}{}",
        stdout_of(&build),
        stderr_of(&build)
    );
    let script = "import top.pkg.m as m\nm.build()\n";
    let installed_source = dir.join("top/pkg/m.py");
    std::fs::copy(&source, &installed_source).expect("install m.py");
    let oracle = host_run(&dir, script);
    std::fs::remove_file(&installed_source).expect("remove m.py");
    std::fs::copy(
        dir.join("stage").join(artifact_name()),
        dir.join("top/pkg").join(artifact_name()),
    )
    .expect("install the artifact");
    let compiled = host_run(&dir, script);
    for (what, output) in [("cpython", &oracle), ("pycc", &compiled)] {
        assert!(
            output.status.success(),
            "{what}: {}{}",
            stdout_of(output),
            stderr_of(output)
        );
        assert_eq!(
            stdout_of(output),
            "0 9 {0: {'A': ('shift', 1)}} start\n",
            "{what}"
        );
    }
}
