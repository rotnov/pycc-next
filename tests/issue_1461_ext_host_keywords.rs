//! #1461 (part of #884): a host call into a compiled `--ext` function,
//! method or constructor may name its parameters as keywords, bound exactly
//! as CPython binds the same call to the uncompiled source
//! (`src/ext_build/keywords.rs`, the shim's `pycc_ext_kw_bind_*`). lark's
//! `InteractiveParser.copy` calls the compiled `ParserState.copy` as
//! `parser_state.copy(deepcopy_values=...)`; before this change every such
//! call raised `takes no keyword arguments` (D-244 rule 7).
//!
//! [`DRIVER`] runs against the extension and against the same source
//! imported as plain Python, so CPython is the oracle for every line: a
//! keyword in any position and order, a defaulted parameter left out or
//! named, `**` unpacking, a keyword-free call to the same method, a static
//! and a class method, a module-level function without defaults, a
//! constructor with and without defaults, a `Generic[T]` class (lark's
//! `ParserState(Generic[StateT])`), and CPython's own wording for an
//! unexpected keyword, a parameter given twice (including after too many
//! positional arguments) and one, two, three or four missing parameters. Its last
//! line pins that a keyword argument costs no reference a positional one
//! does not.
//!
//! [`EXT_ONLY_DRIVER`] pins the documented differences (D-244's #1461
//! amendment): no `Did you mean` suggestion; the receiver's own name is an
//! unexpected keyword rather than a duplicate; a module-level function
//! with a default (#1194) and a function with a positional-only parameter
//! keep the `takes no keyword arguments` refusal; a keyword argument is
//! converted by the parameter's declared type exactly as a positional one,
//! a `memoryview` passed by keyword included, whose buffer is released on
//! the success path (shared driver) and on a later argument's refusal (the
//! `bytearray` resizes afterwards); and a keyword-free call keeps pycc's
//! own arity wording (#1025).
//!
//! The hosted test is `#[ignore]`d and contributes no line coverage; the
//! Tier-1 `native-build-test` leg runs it with
//! `cargo test --workspace -- --include-ignored`. The changed lines are
//! covered by `src/ext_build/keywords_tests.rs`.

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

fn assert_ok(run: &Output) {
    assert!(
        run.status.success(),
        "stdout: {}\nstderr: {}",
        stdout_of(run),
        stderr_of(run)
    );
}

/// The module under test.
const MODULE: &str = r#"from typing import Any, Generic, TypeVar

T = TypeVar("T")


class P:
    def __init__(self, n: int, step: int = 1) -> None:
        self.n = n
        self.step = step

    def copy(self, deepcopy_values: bool = True) -> int:
        if deepcopy_values:
            return self.n
        return -self.n

    def add(self, a: int, b: int = 10, c: int = 100, label: str = "s") -> str:
        return f"{label}{self.n + a + b + c}"

    def three(self, a: int, b: int, c: int) -> int:
        return a * 100 + b * 10 + c

    def four(self, a: int, b: int, c: int, d: int) -> int:
        return a + b + c + d

    def five(self, a: int, b: int, c: int, d: int, e: int) -> int:
        return a + b + c + d + e

    def size(self) -> int:
        return self.n

    def hold(self, o: Any) -> int:
        return 0

    @staticmethod
    def mul(x: int, y: int = 2) -> int:
        return x * y

    @classmethod
    def tag(cls, t: str) -> str:
        return "<" + t + ">"


class Exact:
    def __init__(self, k: int, j: int) -> None:
        self.k = k
        self.j = j

    def total(self) -> int:
        return self.k * 10 + self.j


class State(Generic[T]):
    def __init__(self, k: int) -> None:
        self.k = k

    def copy(self, deepcopy_values: bool = True) -> int:
        if deepcopy_values:
            return self.k
        return -self.k


def rep(a: int, b: str) -> str:
    return b * a


def pad(a: int, b: int = 2) -> int:
    return a + b


def only(a: int, /, b: int) -> int:
    return a - b


def view_n(v: memoryview, n: int) -> int:
    return n
"#;

/// The host script both sides run. Every line is compared with CPython.
const DRIVER: &str = r#"import sys
import pycc_kw_mod as m


def attempt(thunk):
    try:
        print(thunk())
    except TypeError as e:
        print("TypeError", e)


p = m.P(3)
attempt(lambda: p.copy())
attempt(lambda: p.copy(deepcopy_values=False))
attempt(lambda: p.copy(deepcopy_values=True))
attempt(lambda: p.add(1))
attempt(lambda: p.add(1, c=5))
attempt(lambda: p.add(c=5, a=1))
attempt(lambda: p.add(1, label="k", b=0))
attempt(lambda: p.add(**{"a": 2, "label": "d"}))
attempt(lambda: p.add(b=5))
attempt(lambda: p.add(c=5, b=5))
attempt(lambda: p.add(1, a=2))
attempt(lambda: p.add(1, zz=2))
attempt(lambda: p.add(1, 2, 3, "x", c=1))
attempt(lambda: p.add(1, 2, 3, "x", 5, zz=1))
attempt(lambda: p.three(c=3, b=2, a=1))
attempt(lambda: p.three(1, c=3))
attempt(lambda: p.three(c=3))
attempt(lambda: p.three(b=2, c=3))
attempt(lambda: p.four(d=1))
attempt(lambda: p.five(e=1))
attempt(lambda: p.four(1, d=4, c=3, b=2))
attempt(lambda: p.size(zz=1))
attempt(lambda: m.P.mul(y=3, x=2))
attempt(lambda: m.P.mul(x=4))
attempt(lambda: m.P.mul(y=3))
attempt(lambda: m.P.tag(t="a"))
attempt(lambda: p.tag(t="b"))
attempt(lambda: m.rep(b="x", a=2))
attempt(lambda: m.rep(2, b="y"))
attempt(lambda: m.rep(b="x"))
attempt(lambda: m.P(n=4).n)
attempt(lambda: m.P(4, step=2).step)
attempt(lambda: m.P(step=5, n=6).step)
attempt(lambda: m.P(zz=1))
attempt(lambda: m.P(1, n=2))
attempt(lambda: m.P(step=2))
attempt(lambda: m.Exact(j=2, k=1).total())
attempt(lambda: m.Exact(1, j=2).total())
attempt(lambda: m.Exact(j=2))
attempt(lambda: m.State(5).copy(deepcopy_values=False))
attempt(lambda: m.State(k=6).copy(deepcopy_values=True))
ba = bytearray(16)
mv = memoryview(ba).cast("d")
attempt(lambda: m.view_n(n=4, v=mv))
mv.release()
ba.append(0)
print(len(ba))
o = object()
before = sys.getrefcount(o)
for _ in range(10):
    p.hold(o)
positional = sys.getrefcount(o) - before
before = sys.getrefcount(o)
for _ in range(10):
    p.hold(o=o)
print(sys.getrefcount(o) - before - positional)
"#;

const DRIVER_OUT: &str = "3\n\
    -3\n\
    3\n\
    s114\n\
    s19\n\
    s19\n\
    k104\n\
    d115\n\
    TypeError P.add() missing 1 required positional argument: 'a'\n\
    TypeError P.add() missing 1 required positional argument: 'a'\n\
    TypeError P.add() got multiple values for argument 'a'\n\
    TypeError P.add() got an unexpected keyword argument 'zz'\n\
    TypeError P.add() got multiple values for argument 'c'\n\
    TypeError P.add() got an unexpected keyword argument 'zz'\n\
    123\n\
    TypeError P.three() missing 1 required positional argument: 'b'\n\
    TypeError P.three() missing 2 required positional arguments: 'a' and 'b'\n\
    TypeError P.three() missing 1 required positional argument: 'a'\n\
    TypeError P.four() missing 3 required positional arguments: 'a', 'b', and 'c'\n\
    TypeError P.five() missing 4 required positional arguments: 'a', 'b', 'c', and 'd'\n\
    10\n\
    TypeError P.size() got an unexpected keyword argument 'zz'\n\
    6\n\
    8\n\
    TypeError P.mul() missing 1 required positional argument: 'x'\n\
    <a>\n\
    <b>\n\
    xx\n\
    yy\n\
    TypeError rep() missing 1 required positional argument: 'a'\n\
    4\n\
    2\n\
    5\n\
    TypeError P.__init__() got an unexpected keyword argument 'zz'\n\
    TypeError P.__init__() got multiple values for argument 'n'\n\
    TypeError P.__init__() missing 1 required positional argument: 'n'\n\
    12\n\
    12\n\
    TypeError Exact.__init__() missing 1 required positional argument: 'k'\n\
    -5\n\
    6\n\
    4\n\
    17\n\
    0\n";

/// The host script whose output differs from CPython's, by design.
const EXT_ONLY_DRIVER: &str = r#"import pycc_kw_mod as m


def attempt(thunk):
    try:
        print(thunk())
    except TypeError as e:
        print("TypeError", e)


p = m.P(3)
attempt(lambda: p.copy(deepcopy_value=False))
attempt(lambda: p.add(self=p, a=1))
attempt(lambda: m.pad(a=1))
attempt(lambda: m.only(1, b=2))
attempt(lambda: m.only(a=1, b=2))
attempt(lambda: p.add(1, label=7))
attempt(lambda: p.add(1, label="ok"))
ba = bytearray(16)
mv = memoryview(ba).cast("d")
attempt(lambda: m.view_n(v=mv, n="x"))
mv.release()
ba.append(0)
print(len(ba))
attempt(lambda: m.P())
attempt(lambda: m.P(1, 2, 3))
attempt(lambda: p.add(1, 2, 3, "x", 5))
attempt(lambda: m.rep(2))
"#;

const EXT_ONLY_OUT: &str = "TypeError P.copy() got an unexpected keyword argument 'deepcopy_value'\n\
    TypeError P.add() got an unexpected keyword argument 'self'\n\
    TypeError pycc_kw_mod.pad() takes no keyword arguments\n\
    TypeError pycc_kw_mod.only() takes no keyword arguments\n\
    TypeError pycc_kw_mod.only() takes no keyword arguments\n\
    TypeError P.add() argument 4: 'int' object cannot be interpreted as a str\n\
    ok114\n\
    TypeError view_n() argument 2: 'str' object cannot be interpreted as an integer\n\
    17\n\
    TypeError P.__init__() takes from 1 to 2 arguments (0 given)\n\
    TypeError P.__init__() takes from 1 to 2 arguments (3 given)\n\
    TypeError P.add() takes from 1 to 4 arguments (5 given)\n\
    TypeError rep() takes exactly 2 arguments (1 given)\n";

/// What CPython answers for [`EXT_ONLY_DRIVER`], so the divergence is
/// stated, not inferred.
const EXT_ONLY_CPYTHON_OUT: &str = "TypeError P.copy() got an unexpected keyword argument 'deepcopy_value'. Did you mean 'deepcopy_values'?\n\
    TypeError P.add() got multiple values for argument 'self'\n\
    3\n\
    -1\n\
    TypeError only() got some positional-only arguments passed as keyword arguments: 'a'\n\
    7114\n\
    ok114\n\
    x\n\
    17\n\
    TypeError P.__init__() missing 1 required positional argument: 'n'\n\
    TypeError P.__init__() takes from 2 to 3 positional arguments but 4 were given\n\
    TypeError P.add() takes from 2 to 5 positional arguments but 6 were given\n\
    TypeError rep() missing 1 required positional argument: 'b'\n";

fn run(script: &str, path_entry: &Path, cwd: &Path) -> Output {
    host_python()
        .arg("-c")
        .arg(script)
        .env("PYTHONPATH", path_entry)
        .current_dir(cwd)
        .output()
        .expect("python3 should spawn")
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_host_keyword_call_into_a_compiled_export_matches_cpython() {
    let dir = ScratchDir::new("ext_host_keywords").expect("scratch");
    let ext = dir.join("ext");
    let oracle = dir.join("oracle");
    std::fs::create_dir_all(&ext).expect("ext dir");
    std::fs::create_dir_all(&oracle).expect("oracle dir");
    let source = write(&oracle, "pycc_kw_mod.py", MODULE);
    let build = pycc()
        .arg("build")
        .arg(&source)
        .arg("-o")
        .arg(ext.join("pycc_kw_mod"))
        .arg("--ext")
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", stderr_of(&build));
    // Each run's working directory is the scratch root, so neither side can
    // import the module from where it runs.
    let compiled = run(DRIVER, &ext, &dir);
    assert_ok(&compiled);
    let cpython = run(DRIVER, &oracle, &dir);
    assert_ok(&cpython);
    assert_eq!(stdout_of(&compiled), stdout_of(&cpython));
    assert_eq!(stdout_of(&compiled), DRIVER_OUT);
    let divergent = run(EXT_ONLY_DRIVER, &ext, &dir);
    assert_ok(&divergent);
    assert_eq!(stdout_of(&divergent), EXT_ONLY_OUT);
    let cpython_divergent = run(EXT_ONLY_DRIVER, &oracle, &dir);
    assert_ok(&cpython_divergent);
    assert_eq!(stdout_of(&cpython_divergent), EXT_ONLY_CPYTHON_OUT);
}
