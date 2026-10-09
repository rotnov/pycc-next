//! #1421: a list display in a value position of an object slot -- stored
//! into an object-declared attribute (`self.s = [o]`), or an operand of
//! `and`/`or` or a branch of a conditional expression bound to one (lark
//! `lalr_parser_state.py` lines 43-44, `self.state_stack = state_stack or
//! [self.parse_conf.start_state]` and `self.value_stack = value_stack or
//! []`) -- builds a fresh CPython `list` with every element boxed.
//!
//! Every hosted test builds the source with `pycc build --ext`, drives the
//! artifact from the host CPython, and compares the driver's output with
//! the same driver run against CPython's own execution of the source. The
//! hosted tests are `#[ignore]`d and contribute no line coverage; the
//! Tier-1 `native-build-test` leg runs them with
//! `cargo test --workspace -- --include-ignored`. The refusal test needs
//! no host interpreter, since it only runs `pycc build`, so it is not
//! ignored and runs on every leg. The changed lines are
//! covered by the unit tests in
//! `crates/pycc_types/src/empty_container/object_slot_tests.rs`.

use pycc_scratch::ScratchDir;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn pycc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
}

fn host_python() -> Command {
    let mut command =
        Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()));
    command.env("PYTHONIOENCODING", "utf-8");
    command
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

fn build_ext_output(dir: &Path, module: &str, body: &str) -> Output {
    pycc()
        .arg("build")
        .arg(write(dir, "m.py", body))
        .arg("-o")
        .arg(dir.join(module))
        .arg("--ext")
        .output()
        .expect("pycc should spawn")
}

fn build_ext(dir: &Path, module: &str, body: &str) {
    let build = build_ext_output(dir, module, body);
    assert!(
        build.status.success(),
        "{}{}",
        stdout_of(&build),
        stderr_of(&build)
    );
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

/// Builds `body` as `module`, runs `driver` with `G` bound to the
/// extension's namespace and then to CPython's own run of `m.py`, asserts
/// the two outputs agree, and returns the compiled run's output.
fn assert_matches_cpython(tag: &str, module: &str, body: &str, driver: &str) -> String {
    let dir = ScratchDir::new(tag).expect("scratch");
    build_ext(&dir, module, body);
    let compiled = python(
        &dir,
        &format!("import {module}\nG = vars({module})\n{driver}"),
    );
    assert_ok(&compiled);
    let oracle = python(
        &dir,
        &format!("import runpy\nG = runpy.run_path('m.py')\n{driver}"),
    );
    assert_ok(&oracle);
    assert_eq!(stdout_of(&compiled), stdout_of(&oracle));
    stdout_of(&compiled)
}

/// The lark `ParserState` shape: the `or` keeps a passed stack by identity
/// and otherwise builds a fresh list per instance, empty or holding the
/// start state; a later method rebinds the slot through either branch of a
/// conditional expression; and a display operand on the right of `or` is
/// never evaluated when the left one is truthy (`1 // z` with `z == 0`).
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn the_lark_parser_state_shape_matches_cpython() {
    let body = "from typing import Any, List\n\
        \n\
        \n\
        class ParserState:\n    \
        __slots__ = 'start', 'state_stack', 'value_stack'\n\
        \n    \
        start: Any\n    \
        state_stack: List[Any]\n    \
        value_stack: list\n\
        \n    \
        def __init__(self, start: Any, state_stack=None, value_stack=None):\n        \
        self.start = start\n        \
        self.state_stack = state_stack or [self.start]\n        \
        self.value_stack = value_stack or []\n\
        \n    \
        def push(self, v: Any) -> None:\n        \
        self.value_stack.append(v)\n\
        \n    \
        def pick(self, c: bool, n: int) -> None:\n        \
        self.state_stack = [n, 'a', 2.5, c] if c else []\n\
        \n    \
        def lazy(self, z: int) -> None:\n        \
        self.state_stack = self.state_stack or [1 // z]\n\
        \n    \
        def stacks(self) -> Any:\n        \
        return self.state_stack\n\
        \n    \
        def values(self) -> Any:\n        \
        return self.value_stack\n";
    let driver = "P = G['ParserState']\n\
        a = P('s0')\n\
        b = P('s0')\n\
        a.push(7)\n\
        print(a.stacks(), a.values(), b.values(), a.values() is b.values())\n\
        given = ['g']\n\
        c = P('s1', given, given)\n\
        print(c.stacks() is given, c.values() is given)\n\
        c.pick(True, 3)\n\
        print(c.stacks())\n\
        d = P('s2')\n\
        d.pick(False, 3)\n\
        print(d.stacks())\n\
        try:\n    d.lazy(0)\n\
        except Exception as e:\n    print(type(e).__name__)\n\
        d.lazy(1)\n\
        a.lazy(0)\n\
        print(d.stacks(), a.stacks())\n\
        e = P('s3', [], [])\n\
        print(e.stacks(), e.values())\n";
    let out = assert_matches_cpython("obj_slot_lark", "pycc_obj_slot_lark", body, driver);
    assert_eq!(
        out,
        "['s0'] [7] [] False\nTrue True\n[3, 'a', 2.5, True]\n[]\nZeroDivisionError\n\
         [1] ['s0']\n['s3'] []\n"
    );
}

/// A slot whose object type comes from a base class or from an object
/// parameter rather than its own declaration, and a store through a
/// receiver not named `self`.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn inherited_established_and_renamed_slots_match_cpython() {
    let body = "from typing import Any\n\
        \n\
        \n\
        class Base:\n    \
        s: list\n\
        \n    \
        def __init__(self, o: list) -> None:\n        \
        self.s = o\n\
        \n\
        \n\
        class Child(Base):\n    \
        def fill(this, n: int) -> None:\n        \
        this.s = [n, n + 1]\n\
        \n    \
        def get(self) -> Any:\n        \
        return self.s\n\
        \n\
        \n\
        class Loose:\n    \
        def __init__(self, o: Any) -> None:\n        \
        self.t = o\n\
        \n    \
        def fill(self, n: int) -> None:\n        \
        self.t = self.t and [n]\n\
        \n    \
        def get(self) -> Any:\n        \
        return self.t\n";
    let driver = "c = G['Child']([])\n\
        c.fill(4)\n\
        print(c.get())\n\
        for start in (0, 'x'):\n    \
        l = G['Loose'](start)\n    \
        l.fill(9)\n    \
        print(l.get())\n";
    let out = assert_matches_cpython("obj_slot_inherit", "pycc_obj_slot_inherit", body, driver);
    assert_eq!(out, "[4, 5]\n0\n[9]\n");
}

/// The object-slot display leaks on the terms `docs/RUNTIME.md` records
/// for every object producer: the list and the one reference it holds per
/// element. Over 100 calls a mortal module-global `probe` gains `+200` for
/// `[probe, probe]`, `+100` for `self.s or [probe]` with an empty slot (the
/// display is built), and nothing when the slot is truthy (it is not).
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn the_slot_display_leaks_only_what_every_producer_leaks() {
    let body = "import builtins\n\
        import sys\n\
        from typing import Any\n\
        \n\
        probe = builtins.object()\n\
        \n\
        \n\
        def rc() -> int:\n    return int(sys.getrefcount(probe))\n\
        \n\
        \n\
        class H:\n    \
        s: Any\n\
        \n    \
        def __init__(self, o: Any) -> None:\n        \
        self.s = o\n\
        \n    \
        def keep(self) -> None:\n        \
        self.s = [probe, probe]\n\
        \n    \
        def reset(self) -> None:\n        \
        self.s = []\n\
        \n    \
        def either(self) -> None:\n        \
        self.s = self.s or [probe]\n";
    let dir = ScratchDir::new("obj_slot_refcount").expect("scratch");
    build_ext(&dir, "pycc_obj_slot_rc", body);
    let driver = "import pycc_obj_slot_rc as m\n\
        h = m.H(0)\n\
        before = m.rc()\n\
        for _ in range(100):\n    h.keep()\n\
        print('keep', m.rc() - before)\n\
        h = m.H(0)\n\
        before = m.rc()\n\
        for _ in range(100):\n    h.reset()\n    h.either()\n\
        print('either-empty', m.rc() - before)\n\
        h = m.H([1])\n\
        before = m.rc()\n\
        for _ in range(100):\n    h.either()\n\
        print('either-kept', m.rc() - before)\n";
    let run = python(&dir, driver);
    assert_ok(&run);
    assert_eq!(
        stdout_of(&run),
        "keep 200\neither-empty 100\neither-kept 0\n"
    );
}

/// What stays outside the object slot keeps one diagnostic, never a panic:
/// an element with no boxing helper, a dict display, and a store through a
/// name that is not the method's receiver.
#[test]
fn the_shapes_outside_the_object_slot_are_refused() {
    const HEAD: &str = "from typing import Any\n\n\nclass A:\n    s: Any\n\n    \
        def __init__(self, o: Any) -> None:\n        self.s = o\n\n    ";
    for (tag, tail, code, needle) in [
        (
            "obj_slot_nested",
            "def m(self) -> None:\n        self.s = self.s or [[1]]\n",
            "I0404",
            "a `list[int]` element in a list display bound to a CPython object",
        ),
        (
            "obj_slot_dict",
            "def m(self) -> None:\n        self.s = {}\n",
            "T0003",
            "an empty dict literal",
        ),
        (
            "obj_slot_foreign_receiver",
            "def m(self) -> None:\n        pass\n\n\n\
             def f(a: A, o: Any) -> None:\n    a.s = [o]\n",
            "T0034",
            "list[object]",
        ),
    ] {
        let dir = ScratchDir::new(tag).expect("scratch");
        let build = build_ext_output(&dir, "pycc_obj_slot_refused", &format!("{HEAD}{tail}"));
        assert!(!build.status.success(), "{tag}");
        let rendered = format!("{}{}", stdout_of(&build), stderr_of(&build));
        assert_eq!(rendered.matches("error[").count(), 1, "{tag}: {rendered}");
        assert!(
            rendered.contains(&format!("error[{code}]")),
            "{tag}: {rendered}"
        );
        assert!(rendered.contains(needle), "{tag}: {rendered}");
    }
}
