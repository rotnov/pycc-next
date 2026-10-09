//! #1442: a compiled instance's fields are readable through an object-typed
//! name. lark `lalr_parser_state.py`'s `ParserState.__eq__` reads
//! `other.state_stack` and `other.position` from an unannotated `other`,
//! which D-258 makes the opaque CPython object; compiled code reads such an
//! attribute with `PyObject_GetAttr`, so the published type must answer it.
//! Each constructible published class carries one `Py_tp_getset`
//! descriptor per carriable slot and per carriable `@property`
//! (`src/ext_build/getset.rs`); a slot's descriptor is writable since
//! Part 1 of #1443.
//!
//! The hosted tests drive the extension from a host script and run the very
//! same script against the source imported as plain Python, so CPython is
//! the oracle for every line of [`DRIVER`]. [`EXT_ONLY_DRIVER`] pins the
//! documented differences from CPython: a slot holding an int outside the
//! inline range raises `OverflowError` (the D-244 `int` boundary, #1040), a
//! host-side store of a value the slot's parameter row refuses raises
//! `TypeError` (Part 1 of #1443), and the `self.items[-1]` subscript inside
//! the `top` getter keeps one reference per call, an object-operation
//! temporary #1092 tracks, not the descriptor: the host read of a slot or of
//! the `whole` property, which creates no such temporary, is
//! reference-neutral in [`DRIVER`]. The compiled `other.state_stack` read in
//! `ParserState.__eq__` kept one reference per call too until #1476: the
//! `isinstance(other, ParserState)` guard before it now narrows `other`, so
//! the read is a native field read and the pinned delta is `0`.
//!
//! The hosted tests are `#[ignore]`d and contribute no line coverage; the
//! Tier-1 `native-build-test` leg runs them with
//! `cargo test --workspace -- --include-ignored`. The changed lines are
//! covered by the unit tests in `src/ext_build_tests/getset.rs`.

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

fn assert_ok(run: &Output) {
    assert!(
        run.status.success(),
        "stdout: {}\nstderr: {}",
        stdout_of(run),
        stderr_of(run)
    );
}

/// The module under test. `ParserState` is the lark shape: `__slots__`, an
/// `Any` configuration, a `List[Any]` and a bare `list` stack, a `position`
/// property and an unannotated `__eq__` that reads both off its object-typed
/// operand -- lark's own `__eq__` verbatim, whose `return NotImplemented`
/// widens its return to the object (D-258's #1418 amendment). The host
/// reaches it through `eq`; a host `==` runs the same body through the
/// carrier type's `tp_richcompare` since #1427.
/// `Fields` has one slot of every carried type plus a bigint, `Base` and
/// `Derived` share an inherited property, and `Partial`'s `__init__` never
/// calls `Base`'s, so `Base`'s slots stay unassigned (#1148). Every class
/// has a method, which publishes it whether or not it is constructible
/// (a method-less class is published only when constructible, #1450).
const MODULE: &str = r#"from typing import Any, List


class ParserState:
    __slots__ = 'parse_conf', 'state_stack', 'value_stack'

    state_stack: List[Any]
    value_stack: list

    def __init__(self, parse_conf: Any, state_stack: Any, value_stack: Any) -> None:
        self.parse_conf = parse_conf
        self.state_stack = state_stack
        self.value_stack = value_stack

    @property
    def position(self) -> Any:
        return self.state_stack[-1]

    def __eq__(self, other) -> bool:
        if not isinstance(other, ParserState):
            return NotImplemented
        return len(self.state_stack) == len(other.state_stack) and self.position == other.position

    def eq(self, other: Any) -> Any:
        return self.__eq__(other)


class Fields:
    big: int

    def __init__(self, n: int, r: float, flag: bool, s: str, o: Any) -> None:
        self.n = n
        self.r = r
        self.flag = flag
        self.s = s
        self.o = o
        self.big = 9223372036854775807 + 2

    def get(self) -> int:
        return self.n


class Base:
    items: List[Any]

    def __init__(self, items: Any) -> None:
        self.items = items
        self.label = 'base'

    @property
    def top(self) -> Any:
        return self.items[-1]

    @property
    def whole(self) -> Any:
        return self.items

    def size(self) -> int:
        return len(self.items)


class Derived(Base):
    def __init__(self, items: Any, extra: int) -> None:
        self.items = items
        self.label = 'derived'
        self.extra = extra


class Partial(Base):
    def __init__(self, extra: int) -> None:
        self.extra = extra

    def get(self) -> int:
        return self.extra


def label_of(o: Any) -> Any:
    return o.label


def top_of(o: Any) -> Any:
    return o.top
"#;

/// The host script both sides run. Every line is compared with CPython.
const DRIVER: &str = r#"import sys
import pycc_field_read_mod as m
a = m.ParserState('conf', [1, 2, 3], [])
b = m.ParserState('conf', [1, 2, 3], [])
c = m.ParserState('conf', [1, 2], [])
d = m.ParserState('conf', [1, 2, 4], [])
print(a.eq(b), a.eq(c), a.eq(d), a.eq(3), a.eq(None), a.eq(a))
print(a.state_stack, a.position, a.parse_conf, a.value_stack, a.state_stack is a.state_stack)
f = m.Fields(7, 2.5, True, 'héllo', [1])
g = m.Fields(-3, -0.0, False, '', None)
for _ in range(3):
    print(f.n, f.r, f.flag, f.s, f.o, g.n, g.r, g.flag, repr(g.s), g.o)
print(type(f.flag).__name__, f.flag is True, g.flag is False, type(f.r).__name__)
s = f.s
for _ in range(1000):
    s = f.s
print(s, f.s == 'héllo')
top = object()
base = m.Base([1, top])
derived = m.Derived(['x', top], 5)
print(base.top is top, derived.top is top, derived.items[0], derived.extra, base.label, derived.label)
print(m.label_of(base), m.label_of(derived), m.top_of(derived) is top)
stack = derived.items
before = sys.getrefcount(stack)
for _ in range(100):
    derived.whole
    derived.items
print(sys.getrefcount(stack) - before, derived.whole is stack, base.whole is stack)
o = f.o
before = sys.getrefcount(o)
for _ in range(100):
    f.o
print(sys.getrefcount(o) - before)
p = m.Partial(4)
print(p.extra, hasattr(p, 'items'), getattr(p, 'label', 'dflt'))
for name in ('items', 'top'):
    try:
        getattr(p, name)
    except AttributeError as e:
        print('AttributeError', e)
print(p.get(), p.extra)
blank = m.Fields.__new__(m.Fields)
try:
    blank.n
except AttributeError as e:
    print('AttributeError', e)
blank_base = m.Base.__new__(m.Base)
try:
    blank_base.top
except AttributeError:
    print('AttributeError')
print(hasattr(blank_base, 'top'), getattr(blank_base, 'items', 'dflt'))
"#;

const DRIVER_OUT: &str = "True False False NotImplemented NotImplemented True\n\
    [1, 2, 3] 3 conf [] True\n\
    7 2.5 True h\u{e9}llo [1] -3 -0.0 False '' None\n\
    7 2.5 True h\u{e9}llo [1] -3 -0.0 False '' None\n\
    7 2.5 True h\u{e9}llo [1] -3 -0.0 False '' None\n\
    bool True True float\n\
    h\u{e9}llo True\n\
    True True x 5 base derived\n\
    base derived True\n\
    0 True False\n\
    0\n\
    4 False dflt\n\
    AttributeError 'Partial' object has no attribute 'items'\n\
    AttributeError 'Partial' object has no attribute 'items'\n\
    4 4\n\
    AttributeError 'Fields' object has no attribute 'n'\n\
    AttributeError\n\
    False dflt\n";

/// The documented differences from CPython, run against the extension only.
const EXT_ONLY_DRIVER: &str = r#"import sys
import pycc_field_read_mod as m
f = m.Fields(7, 2.5, True, 's', None)
for _ in range(2):
    try:
        f.big
    except OverflowError:
        print('OverflowError')
try:
    f.n = 'x'
except TypeError as e:
    print('TypeError', e)
print(f.n)
a = m.ParserState('c', [1], [])
b = m.ParserState('c', [1], [])
stack = b.state_stack
before = sys.getrefcount(stack)
for _ in range(100):
    a.eq(b)
print(sys.getrefcount(stack) - before)
top = object()
base = m.Base([top])
before = sys.getrefcount(top)
for _ in range(100):
    base.top
print(sys.getrefcount(top) - before)
"#;

const EXT_ONLY_OUT: &str = "OverflowError\n\
    OverflowError\n\
    TypeError Fields.n() argument 1: 'str' object cannot be interpreted as an integer\n\
    7\n\
    0\n\
    100\n";

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
fn compiled_fields_read_through_an_object_name_like_cpython() {
    let dir = ScratchDir::new("field_read_hosted").expect("scratch");
    let ext = dir.join("ext");
    let oracle = dir.join("oracle");
    std::fs::create_dir_all(&ext).expect("ext dir");
    std::fs::create_dir_all(&oracle).expect("oracle dir");
    let source = write(&oracle, "pycc_field_read_mod.py", MODULE);
    let build = pycc()
        .arg("build")
        .arg(&source)
        .arg("-o")
        .arg(ext.join("pycc_field_read_mod"))
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
}
