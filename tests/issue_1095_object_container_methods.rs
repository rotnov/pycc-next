//! Issue #1095: `append`, `pop`, `get` and `add` called on a CPython object
//! are the foreign method call (`pycc_ext_obj_call`), whatever their arity,
//! in any module that can hold an object -- an `ext` module (D-258), one
//! whose body binds a foreign import (D-244 rule 3; anywhere in the body
//! since #1482, `tests/issue_1482_own_import_admission.rs`), or (#1425) one whose
//! direct project dependency can hold one
//! (`tests/issue_1425_sibling_object_receivers.rs`). A native `list`,
//! `dict` or `set` receiver keeps its native container path.
//!
//! The hosted tests build `MODULE` with `pycc build --ext`, drive it from the
//! host CPython with a `list`, `dict` and `set` made there, and compare the
//! output with CPython's own run of the same source. They are `#[ignore]`d
//! and contribute no line coverage; the Tier-1 `native-build-test` leg runs
//! them with `cargo test --workspace -- --include-ignored`. The changed lines
//! are covered by the unit tests in
//! `crates/pycc_hir/src/import/tests/object_receivers.rs` and
//! `crates/pycc_types/src/foreign/container_names_tests.rs`.

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

fn check(tag: &str, body: &str) -> Output {
    let dir = ScratchDir::new(tag).expect("scratch");
    pycc()
        .arg("check")
        .arg(write(&dir, "m.py", body))
        .output()
        .expect("pycc should spawn")
}

/// The issue's measured matrix: each call on a module object was refused
/// with `I0404`, or with a container diagnostic for the arities the
/// container fast path does not take. Each now passes `pycc check`.
#[test]
fn every_container_name_on_a_foreign_module_passes_check() {
    for call in [
        "gc.get(1, 2)",
        "gc.pop()",
        "gc.append(1)",
        "gc.add(1)",
        "gc.get(1)",
        "gc.pop(1)",
    ] {
        let output = check("obj_container_check", &format!("import gc\n\n{call}\n"));
        assert_eq!(
            output.status.code(),
            Some(0),
            "{call}: {}{}",
            stdout_of(&output),
            stderr_of(&output)
        );
    }
}

/// A native list in a module with a foreign import keeps its container
/// reading, so a wrong element type is still `T0021`.
#[test]
fn a_native_list_beside_a_foreign_import_keeps_its_diagnostic() {
    let output = check(
        "obj_container_native",
        "import gc\n\nxs = [1]\nxs.append(\"s\")\n",
    );
    assert_eq!(output.status.code(), Some(1), "{}", stderr_of(&output));
    let rendered = stdout_of(&output);
    assert_eq!(rendered.matches("error[").count(), 1, "{rendered}");
    assert!(rendered.contains("error[T0021]"), "{rendered}");
    assert!(
        rendered.contains("cannot append `str` to a list of `int`"),
        "{rendered}"
    );
}

/// The extension module. `Stack` is lark's parser-state shape: bare
/// `list`/`dict`/`set` slots are objects in an `ext` module (D-258), while
/// `native` stays a native `list[int]`. `stacks` is lark
/// `lalr_parser_state.py` lines 87/88 and 105/106 (`state_stack.append`,
/// `value_stack.append`). `pin` compares the reference counts `append` and
/// `add` leave behind; it calls no result-producing method, whose discarded
/// result is the separate #1092 leak.
const MODULE: &str = "import sys\n\
    from os import environ\n\
    from typing import Any, List\n\
    \n\
    \n\
    class Stack:\n    \
    def __init__(self, items: list, table: dict, seen: set) -> None:\n        \
    self.items = items\n        self.table = table\n        self.seen = seen\n        \
    self.native: list[int] = []\n\
    \n    \
    def push(self, x: Any) -> None:\n        self.items.append(x)\n        \
    self.native.append(len(self.items))\n\
    \n    \
    def top(self) -> Any:\n        return self.items.pop()\n\
    \n    \
    def look(self, k: str) -> Any:\n        return self.table.get(k, -1)\n\
    \n    \
    def mark(self, k: Any) -> None:\n        self.seen.add(k)\n\
    \n    \
    def depth(self) -> int:\n        return self.native.pop()\n\
    \n\
    \n\
    def natives() -> int:\n    xs = []\n    xs.append(4)\n    ys: list[int] = [1, 2]\n    \
    ys.append(3)\n    d: dict[str, int] = {\"a\": 5}\n    s: set[int] = {1}\n    s.add(9)\n    \
    return xs.pop() + ys.pop() + d.get(\"a\", 0) + d.get(\"b\", 7) + len(s)\n\
    \n\
    \n\
    def stacks(state_stack: List[Any], value_stack: list, callbacks: dict, token: Any) -> Any:\n    \
    state_stack.append(1)\n    \
    value_stack.append(token if token not in callbacks else callbacks[token](token))\n    \
    return value_stack[-1]\n\
    \n\
    \n\
    def from_host(o: Any, x: Any, d: Any, s: Any) -> None:\n    o.append(x)\n    o.append(1)\n    \
    o.extend(x)\n    print(o.pop())\n    print(d.get(\"a\"))\n    print(d.get(\"zz\", 5))\n    \
    print(d.keys())\n    print(d.items())\n    s.add(3)\n    print(o, s)\n\
    \n\
    \n\
    def pin(o: Any, x: Any, s: Any) -> None:\n    bo = int(sys.getrefcount(o))\n    \
    bx = int(sys.getrefcount(x))\n    bs = int(sys.getrefcount(s))\n    i = 0\n    \
    while i < 200:\n        o.append(x)\n        s.add(x)\n        i += 1\n    \
    print(int(sys.getrefcount(o)) - bo, int(sys.getrefcount(x)) - bx, \
    int(sys.getrefcount(s)) - bs)\n    o.clear()\n    s.clear()\n    \
    print(int(sys.getrefcount(x)) - bx)\n\
    \n\
    \n\
    sys.path.append(\"pycc-1095\")\n\
    print(sys.path.pop())\n\
    print(environ.get(\"PYCC_NOPE_1095\", \"dflt\"))\n\
    print(environ.get(\"PYCC_NOPE_1095\"))\n\
    found = sys.modules.get(\"sys\")\n\
    print(found is sys)\n\
    print(natives())\n";

/// Drives `MODULE` from the host: the compiled extension when `argv[1]` is
/// `compiled`, CPython's own run of `m.py` otherwise. The driver reports on
/// stderr: compiled `print` writes through its own stdout buffer, separate
/// from CPython's `sys.stdout` (`docs/RUNTIME.md`), so the two writers'
/// relative order on one stream is not CPython's.
const DRIVER: &str = "import runpy\n\
    import sys\n\
    import types\n\
    \n\
    if sys.argv[1] == 'compiled':\n    import pycc_obj_containers as m\n\
    else:\n    m = types.SimpleNamespace(**runpy.run_path('m.py'))\n\
    \n\
    m.from_host([1, 2], [3], {'a': 1}, {1})\n\
    items, table, seen = [], {'k': 'v'}, set()\n\
    st = m.Stack(items, table, seen)\n\
    probe = object()\n\
    st.push(probe)\n\
    st.push('b')\n\
    print(len(items), st.depth(), st.top(), st.look('k'), st.look('z'), file=sys.stderr)\n\
    st.mark(probe)\n\
    st.mark(3)\n\
    print(len(seen), items[0] is probe, file=sys.stderr)\n\
    ss, vs = [], []\n\
    print(m.stacks(ss, vs, {'t': str.upper}, 't'), m.stacks(ss, vs, {}, 'u'), ss, vs, file=sys.stderr)\n\
    m.pin([], object(), set())\n\
    for bad in (\n    lambda: m.Stack([], {}, set()).top(),\n    \
    lambda: m.Stack(5, {}, set()).push(1),\n    \
    lambda: m.Stack([], {}, frozenset()).mark(1),\n):\n    \
    try:\n        bad()\n    except (IndexError, AttributeError) as e:\n        \
    print(type(e).__name__, e, file=sys.stderr)\n";

/// What `MODULE` prints (stdout) under CPython: its body, then `from_host`
/// and `pin`.
const EXPECTED_OUT: &str = "pycc-1095\ndflt\nNone\nTrue\n21\n\
    3\n1\n5\ndict_keys(['a'])\ndict_items([('a', 1)])\n[1, 2, [3], 1] {1, 3}\n0 201 0\n0\n";

/// What `DRIVER` reports (stderr) under CPython.
const EXPECTED_ERR: &str = "2 2 b v -1\n2 True\nT u [1, 1] ['T', 'u']\n\
    IndexError pop from empty list\n\
    AttributeError 'int' object has no attribute 'append'\n\
    AttributeError 'frozenset' object has no attribute 'add'\n";

fn drive(dir: &Path, mode: &str) -> Output {
    host_python()
        .arg("-c")
        .arg(DRIVER)
        .arg(mode)
        .current_dir(dir)
        .env("PYTHONIOENCODING", "utf-8")
        .output()
        .expect("python3 should spawn")
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn container_names_on_host_objects_behave_like_cpython() {
    let dir = ScratchDir::new("obj_container_hosted").expect("scratch");
    let build = pycc()
        .arg("build")
        .arg(write(&dir, "m.py", MODULE))
        .arg("-o")
        .arg(dir.join("pycc_obj_containers"))
        .arg("--ext")
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", stderr_of(&build));
    let compiled = drive(&dir, "compiled");
    let oracle = drive(&dir, "oracle");
    for run in [&compiled, &oracle] {
        assert!(
            run.status.success(),
            "stdout: {}\nstderr: {}",
            stdout_of(run),
            stderr_of(run)
        );
    }
    assert_eq!(stdout_of(&compiled), stdout_of(&oracle));
    assert_eq!(stderr_of(&compiled), stderr_of(&oracle));
    assert_eq!(stdout_of(&compiled), EXPECTED_OUT);
    assert_eq!(stderr_of(&compiled), EXPECTED_ERR);
}
