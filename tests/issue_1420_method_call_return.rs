//! #1420: an unannotated private or dunder method whose return is a method
//! call on a user-class instance infers the resolved method's return type
//! (`docs/TYPE_SYSTEM.md`, "v0.1 local inference"), so lark's
//! `def __copy__(self): return self.copy()` compiles.
//!
//! The native tests run the compiled program through `pycc run` and pin the
//! stdout CPython prints for the same source; the refusal tests pin the
//! diagnostic for a recursive helper and for an inherited body whose
//! inferred return would differ per subclass. The hosted test is
//! `#[ignore]`d and contributes no line coverage; the Tier-1
//! `native-build-test` leg runs it with `cargo test --workspace --
//! --include-ignored`, comparing the extension artifact against CPython's
//! own run of the same source.

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

/// `pycc check --error-format json` on `body` fails with exit `1`; returns
/// the rendered diagnostics.
fn check_fails(tag: &str, body: &str) -> String {
    let dir = ScratchDir::new(tag).expect("scratch");
    let output = pycc()
        .arg("check")
        .arg("--error-format")
        .arg("json")
        .arg(write(&dir, "m.py", body))
        .output()
        .expect("pycc should spawn");
    let rendered = format!("{}{}", stdout_of(&output), stderr_of(&output));
    assert_eq!(output.status.code(), Some(1), "{tag}: {rendered}");
    rendered
}

/// The issue's shape: `__copy__` returns `copy()`'s annotated instance, a
/// private helper returns an annotated `int` method, and a helper chain
/// declared before the helper it calls still resolves.
#[test]
fn a_method_returning_a_method_call_infers_the_callees_return() {
    assert_prints(
        "1420_shape",
        "from __future__ import annotations\n\
         class P:\n\
         \x20   def __init__(self, n: int) -> None:\n        self.n = n\n\
         \x20   def __copy__(self):\n        return self.copy()\n\
         \x20   def copy(self) -> P:\n        return P(self.n + 1)\n\
         \x20   def value(self) -> int:\n        return self.n * 10\n\
         \x20   def _outer(self):\n        return self._inner() + 1\n\
         \x20   def _inner(self):\n        return self.value()\n\
         def _of(p: P):\n    return p.copy().value()\n\
         p = P(1)\n\
         print(p.__copy__().n, p.__copy__().__copy__().n)\n\
         print(p._outer(), _of(p))\n",
        "2 3\n11 20\n",
    );
}

/// A method found through the MRO resolves for the subclass, and an
/// inherited body constructing `type(self)(...)` still narrows to each
/// receiver's own class.
#[test]
fn inherited_methods_resolve_through_the_mro() {
    assert_prints(
        "1420_mro",
        "class Base:\n\
         \x20   def __init__(self, n: int) -> None:\n        self.n = n\n\
         \x20   def tag(self) -> str:\n        return \"base\"\n\
         \x20   def _mk(self):\n        return type(self)(self.n + 1)\n\
         \x20   def _label(self):\n        return self.tag()\n\
         class Derived(Base):\n\
         \x20   def tag(self) -> str:\n        return \"derived\"\n\
         \x20   def _twice_label(self):\n        return self._label() + self._label()\n\
         d = Derived(2)\n\
         print(d._mk().n, d._mk().tag(), d._label(), d._twice_label())\n\
         print(Base(5)._mk().tag(), Base(5)._label())\n",
        "3 derived derived derivedderived\nbase base\n",
    );
}

/// Recursion and mutual recursion terminate with the usual `T0021`.
#[test]
fn recursion_is_refused_with_the_usual_diagnostic() {
    let rendered = check_fails(
        "1420_recursion",
        "class C:\n\
         \x20   def _a(self):\n        return self._b()\n\
         \x20   def _b(self):\n        return self._a()\n\
         print(C()._a())\n",
    );
    assert!(rendered.contains("T0021"), "{rendered}");
    assert!(
        rendered.contains("cannot infer return type of private helper `C._"),
        "{rendered}"
    );
}

/// An inherited body whose inferred return follows an override to an
/// unrelated type is refused at the origin, naming the subclass.
#[test]
fn an_inherited_return_that_drifts_per_subclass_is_refused() {
    let rendered = check_fails(
        "1420_drift",
        "class Base:\n\
         \x20   def __init__(self, n: int) -> None:\n        self.n = n\n\
         \x20   def val(self) -> int:\n        return self.n\n\
         \x20   def _twice(self):\n        return self.val()\n\
         class Derived(Base):\n\
         \x20   def val(self) -> str:\n        return \"d\"\n\
         y: int = Derived(2)._twice()\n\
         print(y + 1)\n",
    );
    assert!(rendered.contains("T0022"), "{rendered}");
    assert!(
        rendered.contains(
            "expected `int`, found `str` (inherited `Base._twice` compiled for subclass `Derived`)"
        ),
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

/// The lark shape (`lark/parsers/lalr_parser_state.py`, MIT, the #1207
/// subject's lines 56-57): `__copy__` returns `self.copy()`, which
/// constructs `type(self)(...)`. The classes are generic like lark's, which
/// keeps them out of the `--ext` export set (an instance cannot cross the
/// CPython boundary yet), so `run` drives the copy inside the module and
/// reports identity and equality of the copy's slots. `copy` is called with
/// its argument spelled out: an omitted defaulted method argument is
/// #1191's.
const LARK: &str = "from __future__ import annotations\n\
    from copy import copy, deepcopy\n\
    from typing import Any, Generic, TypeVar\n\
    StateT = TypeVar('StateT')\n\
    class ParseConf(Generic[StateT]):\n\
    \x20   def __init__(self, start: str) -> None:\n        self.start = start\n\
    class ParserState(Generic[StateT]):\n\
    \x20   __slots__ = 'parse_conf', 'lexer', 'state_stack', 'value_stack'\n\
    \x20   parse_conf: ParseConf[StateT]\n    lexer: Any\n    state_stack: list\n    value_stack: list\n\
    \x20   def __init__(self, parse_conf: ParseConf[StateT], lexer: Any, state_stack: list, \
    value_stack: list) -> None:\n\
    \x20       self.parse_conf = parse_conf\n        self.lexer = lexer\n\
    \x20       self.state_stack = state_stack\n        self.value_stack = value_stack\n\
    \x20   def __copy__(self):\n        return self.copy(True)\n\
    \x20   def copy(self, deepcopy_values: bool = True) -> ParserState:\n\
    \x20       return type(self)(\n\
    \x20           self.parse_conf,\n            self.lexer,\n\
    \x20           copy(self.state_stack),\n\
    \x20           deepcopy(self.value_stack) if deepcopy_values else copy(self.value_stack),\n\
    \x20       )\n\
    def run(lexer: Any, ss: list, vs: list) -> None:\n\
    \x20   s = ParserState(ParseConf('start'), lexer, ss, vs)\n\
    \x20   c = s.__copy__()\n\
    \x20   print(c.parse_conf.start, c.lexer is lexer, c.state_stack is ss, c.state_stack == ss)\n\
    \x20   print(c.value_stack is vs, c.value_stack == vs, c.value_stack[0] is vs[0])\n\
    \x20   print(c.__copy__().value_stack, c.state_stack)\n";

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn the_lark_dunder_copy_matches_cpython_in_an_extension() {
    let compiled_dir = ScratchDir::new("1420_hosted_lark").expect("scratch");
    let source_dir = ScratchDir::new("1420_hosted_lark_src").expect("scratch");
    let source = write(&source_dir, "m.py", LARK);
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
    let script = "import m\nm.run(object(), [1, 2], [[3], 4])\n";
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
    // `__copy__` deep-copies the value stack (a fresh inner list); the
    // lexer is shared and the outer lists are fresh.
    assert_eq!(
        stdout_of(&compiled),
        "start True False True\nFalse True False\n[[3], 4] [1, 2]\n"
    );
}
