//! #1458 (Part 3 of #1443): a host-side `obj.p = v` on a compiled instance
//! of a constructible published class runs the property's compiled setter
//! (`src/ext_build/getset/setter.rs`'s `property_setter_c`), and a store or
//! `del` the property cannot take raises CPython's own wording. Before this
//! change every property descriptor was read-only, so each of these raised
//! `attribute ... is not writable`.
//!
//! The setter is reached through its `METH_FASTCALL` wrapper, so the value
//! is converted by the setter parameter's row of the boundary table, as an
//! argument of that type is.
//!
//! The hosted test drives the extension from a host script and runs the
//! very same script against the source imported as plain Python, so CPython
//! is the oracle for every line of [`DRIVER`]: a setter of each carried
//! value type (`int`, `float`, `bool`, `str`, an opaque object, an instance
//! of a published class) that transforms the value so the run is
//! observable, read back through the getter and a compiled method; a
//! subclass instance in an instance-typed setter; a `bool` into an `int`
//! setter; a raising setter; a subclass whose inherited setter calls an
//! overridden method; a compiled store through an `Any` name (#1457's
//! `PyObject_SetAttr`) reaching the setter; and CPython's `has no setter`
//! and `has no deleter` refusals, including on a carrier whose `__init__`
//! never ran. [`EXT_ONLY_DRIVER`] pins the documented differences (D-244's
//! #1458 amendment): a value outside the parameter row is refused with that
//! row's error; a setter whose value type the boundary does not carry
//! (`list[int]`) leaves the property read-only; and a carrier whose
//! `__init__` never ran cannot be stored into.
//!
//! The hosted test is `#[ignore]`d and contributes no line coverage; the
//! Tier-1 `native-build-test` leg runs it with
//! `cargo test --workspace -- --include-ignored`. The changed lines are
//! covered by `src/ext_build_tests/getset.rs`.

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

/// The module under test. `Box` has one property with a setter per carried
/// value type, a getter-only `ro`, and a `count` whose setter takes a
/// `list[int]`, which the boundary does not carry. `Wide` inherits `n`'s
/// setter, whose body calls the overridden `scale`. `poke` stores through
/// an `Any` name.
const MODULE: &str = r#"from typing import Any, List


class Leaf:
    def __init__(self, k: int) -> None:
        self.k = k


class Twig(Leaf):
    def __init__(self, k: int, j: int) -> None:
        self.k = k
        self.j = j


class Box:
    def __init__(self, n: int, o: Any, leaf: Leaf) -> None:
        self._n = n
        self._f = 0.5
        self._b = False
        self._s = "a"
        self._o = o
        self._leaf = leaf
        self._xs: List[int] = []
        self.sets = 0

    def scale(self) -> int:
        return 2

    @property
    def n(self) -> int:
        return self._n

    @n.setter
    def n(self, v: int) -> None:
        if v < 0:
            raise ValueError("negative")
        self._n = v * self.scale()
        self.sets = self.sets + 1

    @property
    def f(self) -> float:
        return self._f

    @f.setter
    def f(self, v: float) -> None:
        self._f = v + 0.25

    @property
    def b(self) -> bool:
        return self._b

    @b.setter
    def b(self, v: bool) -> None:
        self._b = not v

    @property
    def s(self) -> str:
        return self._s

    @s.setter
    def s(self, v: str) -> None:
        self._s = v + "!"

    @property
    def o(self) -> Any:
        return self._o

    @o.setter
    def o(self, v: Any) -> None:
        self._o = v

    @property
    def leaf(self) -> Leaf:
        return self._leaf

    @leaf.setter
    def leaf(self, v: Leaf) -> None:
        self._leaf = v

    @property
    def ro(self) -> int:
        return self._n + 1

    @property
    def count(self) -> int:
        return len(self._xs)

    @count.setter
    def count(self, v: List[int]) -> None:
        self._xs = v

    def n_c(self) -> int:
        return self._n

    def leaf_k(self) -> int:
        return self._leaf.k


class Wide(Box):
    def scale(self) -> int:
        return 10


def poke(target: Any, v: int) -> None:
    target.n = v
"#;

/// The host script both sides run. Every line is compared with CPython.
const DRIVER: &str = r#"import pycc_prop_set_mod as m
def attempt(f):
    try:
        f()
        print('ok')
    except (TypeError, AttributeError, OverflowError, ValueError) as e:
        print(type(e).__name__, e)
o = object()
c = m.Box(3, o, m.Leaf(1))
c.n = 5
c.f = 1.5
c.b = True
c.s = "xy"
print(c.n, c.n_c(), c.sets, c.f, c.b, c.s)
o2 = object()
c.o = o2
print(c.o is o2)
t = m.Twig(7, 8)
c.leaf = t
print(c.leaf is t, c.leaf_k(), type(c.leaf).__name__)
c.n = True
print(c.n, c.sets)
attempt(lambda: setattr(c, 'n', -1))
print(c.n, c.sets)
w = m.Wide(1, o, m.Leaf(2))
w.n = 3
print(w.n, w.n_c())
m.poke(c, 4)
m.poke(w, 4)
print(c.n, w.n)
attempt(lambda: setattr(c, 'ro', 9))
attempt(lambda: delattr(c, 'ro'))
attempt(lambda: delattr(c, 'n'))
attempt(lambda: setattr(m.Box.__new__(m.Box), 'ro', 1))
attempt(lambda: delattr(m.Box.__new__(m.Box), 'n'))
print(c.ro, c.n)
"#;

const DRIVER_OUT: &str = "10 10 1 1.75 False xy!\n\
    True\n\
    True 7 Twig\n\
    2 2\n\
    ValueError negative\n\
    2 2\n\
    30 30\n\
    8 40\n\
    AttributeError property 'ro' of 'Box' object has no setter\n\
    AttributeError property 'ro' of 'Box' object has no deleter\n\
    AttributeError property 'n' of 'Box' object has no deleter\n\
    AttributeError property 'ro' of 'Box' object has no setter\n\
    AttributeError property 'n' of 'Box' object has no deleter\n\
    9 8\n";

/// The documented differences from CPython, run against the extension only.
const EXT_ONLY_DRIVER: &str = r#"import pycc_prop_set_mod as m
def attempt(f):
    try:
        f()
        print('ok')
    except (TypeError, AttributeError, OverflowError, ValueError) as e:
        print(type(e).__name__, e)
def store(obj, name, value):
    return lambda: setattr(obj, name, value)
c = m.Box(3, object(), m.Leaf(1))
attempt(store(c, 'n', 'x'))
attempt(store(c, 'n', 2**70))
attempt(store(c, 'f', 1))
attempt(store(c, 'b', 1))
attempt(store(c, 's', 3))
attempt(store(c, 'leaf', object()))
attempt(store(c, 'count', [1, 2]))
print(c.n, c.f, c.b, c.s, type(c.leaf).__name__, c.count)
attempt(store(m.Box.__new__(m.Box), 'n', 1))
"#;

const EXT_ONLY_OUT: &str = "TypeError Box.n() argument 1: 'str' object cannot be interpreted as an integer\n\
    OverflowError Box.n() argument 1: int is outside the inline-integer range [-2**62, 2**62-1] this pycc version's `ext` boundary supports (see #1040)\n\
    TypeError Box.f() argument 1: 'int' object cannot be interpreted as a float\n\
    TypeError Box.b() argument 1: 'int' object cannot be interpreted as a bool\n\
    TypeError Box.s() argument 1: 'int' object cannot be interpreted as a str\n\
    TypeError Box.leaf() argument 1 must be pycc_prop_set_mod.Leaf, not object\n\
    AttributeError attribute 'count' of 'pycc_prop_set_mod.Box' objects is not writable\n\
    3 0.5 False a Leaf 0\n\
    AttributeError cannot set 'n' on a 'Box' object whose __init__ never ran\n";

/// What CPython answers for the lines [`EXT_ONLY_DRIVER`] pins, so the
/// divergence is stated, not inferred: the setter runs on any value (and
/// raises from its own body where the value does not fit it), the
/// `list[int]` setter stores, the setter runs on a never-initialized
/// object (and fails on the field `__init__` would have set).
const EXT_ONLY_CPYTHON_OUT: &str = "TypeError '<' not supported between instances of 'str' and 'int'\n\
    ok\n\
    ok\n\
    ok\n\
    TypeError unsupported operand type(s) for +: 'int' and 'str'\n\
    ok\n\
    ok\n\
    2361183241434822606848 1.25 False a object 2\n\
    AttributeError 'Box' object has no attribute 'sets'\n";

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
fn a_host_store_through_a_compiled_property_setter_matches_cpython() {
    let dir = ScratchDir::new("property_setter_hosted").expect("scratch");
    let ext = dir.join("ext");
    let oracle = dir.join("oracle");
    std::fs::create_dir_all(&ext).expect("ext dir");
    std::fs::create_dir_all(&oracle).expect("oracle dir");
    let source = write(&oracle, "pycc_prop_set_mod.py", MODULE);
    let build = pycc()
        .arg("build")
        .arg(&source)
        .arg("-o")
        .arg(ext.join("pycc_prop_set_mod"))
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
