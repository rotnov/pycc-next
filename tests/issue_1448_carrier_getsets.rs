//! #1448: every carrier type carries its field descriptors. #1442 gave only
//! a *constructible* published class's type object a `Py_tp_getset` table,
//! so a field read through any other carrier -- a non-constructible
//! published type, or the carrier type of a class that publishes nothing
//! -- raised `AttributeError` where CPython returns the field. The table is
//! now built per carrier class (`collect_carrier_getsets` in
//! `src/ext_build/getset.rs`): a published type installs it on its own
//! slot array, and any other class with a table gets a hidden carrier type
//! registered at module exec (`richcompare::hidden_carrier_types_c`, #1427's
//! mechanism), so its instances never cross on the descriptor-less
//! on-demand type.
//!
//! The hosted test drives the extension from a host script and runs the
//! very same script against the source imported as plain Python, so CPython
//! is the oracle for every line of [`DRIVER`], including the
//! reference-count deltas of the host reads. [`EXT_ONLY_DRIVER`] pins the
//! documented differences, none of them new: the compiled reads through an
//! `Any` operand keep one reference per call -- the `object`-parameter and
//! attribute-read temporaries #1092 tracks, which #1442's and #1453's
//! compiled reads keep too -- and a host store of a value the slot's type
//! does not admit is refused with the parameter row's `TypeError` (Part 1
//! of #1443). It also pins the residual #1448 leaves: a class with nothing
//! to describe still crosses on the on-demand type, whose field read raises
//! `AttributeError`.
//!
//! The hosted test is `#[ignore]`d and contributes no line coverage; the
//! Tier-1 `native-build-test` leg runs it with
//! `cargo test --workspace -- --include-ignored`. The changed lines are
//! covered by the unit tests in `src/ext_build_tests/carrier_getsets.rs`,
//! `src/ext_build_tests/getset.rs` and `src/ext_build/richcompare_tests.rs`.

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

/// The module under test. `Base` is published -- its method is reachable
/// through the constructible `Sub` -- but its `dict` constructor parameter
/// keeps it from being constructible, so before #1448 its type carried no
/// table. `Hidden` publishes nothing (no public method, and a `tuple`
/// constructor parameter no generated constructor carries) and `_P` is
/// private, so both crossed on the shim's on-demand type. Each has an
/// `int`, a `str` and a same-module-instance (`Leaf`) slot and `float` and
/// `str` properties. `f`, `leaf_of` and `half_of` read a field off an `Any`
/// operand, which compiled code does through `PyObject_GetAttr` (D-258).
/// `G` is a constructible published PEP 695 template: it keeps the table
/// #1442 built for it, which #1448's generic exclusion must not remove.
/// `Bare` is the residual: its only field is a `List[int]`, which no
/// descriptor carries (D-258's silent partiality), so it has nothing to
/// describe, gets no hidden type and still crosses on the shim's
/// descriptor-less on-demand type -- [`EXT_ONLY_DRIVER`] pins that its field
/// read raises `AttributeError` where CPython returns the list.
const MODULE: &str = r#"from typing import Any, Dict, List, Tuple


class Leaf:
    def __init__(self, v: int) -> None:
        self.v = v


class Base:
    leaf: Leaf

    def __init__(self, d: Dict[str, int], x: int) -> None:
        self.x = x
        self.s = 'base'
        self.leaf = Leaf(x + d['k'])

    def get(self) -> int:
        return self.x

    @property
    def half(self) -> float:
        return self.x / 2

    @property
    def tag(self) -> str:
        return self.s + '!'


class Sub(Base):
    leaf: Leaf

    def __init__(self, x: int) -> None:
        self.x = x
        self.s = 'sub'
        self.leaf = Leaf(x)


class Hidden:
    x: int
    leaf: Leaf

    def __init__(self, t: Tuple[int, int]) -> None:
        self.x = t[0]
        self.s = 'hidden'
        self.leaf = Leaf(t[1])

    @property
    def half(self) -> float:
        return self.x / 2

    @property
    def tag(self) -> str:
        return self.s + '!'


class _P:
    leaf: Leaf

    def __init__(self, x: int) -> None:
        self.x = x
        self.s = 'p'
        self.leaf = Leaf(x * 2)

    @property
    def half(self) -> float:
        return self.x / 2

    @property
    def tag(self) -> str:
        return self.s + '!'


class G[T]:
    def __init__(self, n: int) -> None:
        self.n = n

    def get(self) -> int:
        return self.n


class Bare:
    items: List[int]

    def __init__(self, x: int) -> None:
        self.items = [x]


def make_base(x: int) -> Base:
    return Base({'k': 1}, x)


def make_hidden(a: int, b: int) -> Hidden:
    return Hidden((a, b))


def make_p(x: int) -> _P:
    return _P(x)


def f(o: Any) -> Any:
    return o.x


def leaf_of(o: Any) -> Any:
    return o.leaf


def half_of(o: Any) -> Any:
    return o.half


def make_bare(x: int) -> Bare:
    return Bare(x)


def items_of(o: Any) -> Any:
    return o.items
"#;

/// The host script both sides run. Every line is compared with CPython.
const DRIVER: &str = r#"import sys
import pycc_carrier_getset_mod as m
for name, o in [("base", m.make_base(4)), ("hidden", m.make_hidden(5, 6)), ("private", m.make_p(7))]:
    print(name, type(o).__name__, o.x, m.f(o), o.s, o.half, m.half_of(o), o.tag)
    print(name, o.leaf.v, m.leaf_of(o).v, o.leaf is m.leaf_of(o), type(o.leaf).__name__)
    print(name, hasattr(o, "x"), hasattr(o, "leaf"), hasattr(o, "half"), hasattr(o, "nope"))
    before = sys.getrefcount(o)
    for _ in range(1000):
        o.x
        o.half
        o.tag
    print(name, sys.getrefcount(o) - before)
    leaf = o.leaf
    before = sys.getrefcount(leaf)
    for _ in range(1000):
        o.leaf
    print(name, sys.getrefcount(leaf) - before)
    del leaf
    o.x = 10
    print(name, o.x, m.f(o), o.half, m.half_of(o))
    fresh = m.Leaf(3)
    o.leaf = fresh
    print(name, o.leaf is fresh, m.leaf_of(o) is fresh)
    try:
        o.half = 1.0
    except AttributeError as e:
        print(name, "AttributeError", e)
    del o.s
    print(name, hasattr(o, "s"), hasattr(o, "x"))
    try:
        o.s
    except AttributeError as e:
        print(name, "AttributeError", e)
g = m.G(3)
print("generic", g.n, hasattr(g, "n"), g.get())
"#;

const DRIVER_OUT: &str = "base Base 4 4 base 2.0 2.0 base!\n\
    base 5 5 True Leaf\n\
    base True True True False\n\
    base 0\n\
    base 0\n\
    base 10 10 5.0 5.0\n\
    base True True\n\
    base AttributeError property 'half' of 'Base' object has no setter\n\
    base False True\n\
    base AttributeError 'Base' object has no attribute 's'\n\
    hidden Hidden 5 5 hidden 2.5 2.5 hidden!\n\
    hidden 6 6 True Leaf\n\
    hidden True True True False\n\
    hidden 0\n\
    hidden 0\n\
    hidden 10 10 5.0 5.0\n\
    hidden True True\n\
    hidden AttributeError property 'half' of 'Hidden' object has no setter\n\
    hidden False True\n\
    hidden AttributeError 'Hidden' object has no attribute 's'\n\
    private _P 7 7 p 3.5 3.5 p!\n\
    private 14 14 True Leaf\n\
    private True True True False\n\
    private 0\n\
    private 0\n\
    private 10 10 5.0 5.0\n\
    private True True\n\
    private AttributeError property 'half' of '_P' object has no setter\n\
    private False True\n\
    private AttributeError '_P' object has no attribute 's'\n\
    generic 3 True 3\n";

/// The documented differences from CPython, run against the extension only.
const EXT_ONLY_DRIVER: &str = r#"import sys
import pycc_carrier_getset_mod as m
for name, o in [("base", m.make_base(4)), ("hidden", m.make_hidden(5, 6)), ("private", m.make_p(7))]:
    before = sys.getrefcount(o)
    for _ in range(1000):
        m.f(o)
    print(name, sys.getrefcount(o) - before)
    leaf = o.leaf
    before = sys.getrefcount(leaf)
    for _ in range(1000):
        m.leaf_of(o)
    print(name, sys.getrefcount(leaf) - before)
    try:
        o.x = "a"
    except TypeError as e:
        print(name, "TypeError", e)
    print(name, o.x)
bare = m.make_bare(3)
print("bare", type(bare).__name__, hasattr(bare, "items"))
try:
    m.items_of(bare)
except AttributeError as e:
    print("bare AttributeError", e)
"#;

const EXT_ONLY_OUT: &str = "base 1000\n\
    base 1000\n\
    base TypeError Base.x() argument 1: 'str' object cannot be interpreted as an integer\n\
    base 4\n\
    hidden 1000\n\
    hidden 1000\n\
    hidden TypeError Hidden.x() argument 1: 'str' object cannot be interpreted as an integer\n\
    hidden 5\n\
    private 1000\n\
    private 1000\n\
    private TypeError _P.x() argument 1: 'str' object cannot be interpreted as an integer\n\
    private 7\n\
    bare Bare False\n\
    bare AttributeError 'pycc_carrier_getset_mod.Bare' object has no attribute 'items'\n";

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
fn every_carrier_type_reads_and_stores_its_fields_like_cpython() {
    let dir = ScratchDir::new("carrier_getset_hosted").expect("scratch");
    let ext = dir.join("ext");
    let oracle = dir.join("oracle");
    std::fs::create_dir_all(&ext).expect("ext dir");
    std::fs::create_dir_all(&oracle).expect("oracle dir");
    let source = write(&oracle, "pycc_carrier_getset_mod.py", MODULE);
    let build = pycc()
        .arg("build")
        .arg(&source)
        .arg("-o")
        .arg(ext.join("pycc_carrier_getset_mod"))
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
