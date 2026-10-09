//! Part 1 of #1443: a host-side `obj.x = v` and `del obj.x` on a compiled
//! instance of a constructible published class, through the slot
//! descriptor's setter (`src/ext_build/getset/setter.rs`). lark's
//! `ParserState` and `InteractiveParser` assign their fields from host code
//! once the extension owns the class; before this change every such store
//! raised `attribute ... is not writable`.
//!
//! A store converts the value by the parameter row of the slot's declared
//! type and replaces the slot word as a compiled `self.x = v` does,
//! releasing the replaced word by its kind
//! (`pycc_rt_ext_instance_store_slot`); a `del` un-assigns the slot
//! (`pycc_rt_ext_instance_delete_slot`), so the next read -- host or
//! compiled -- raises CPython's `AttributeError`.
//!
//! The hosted test drives the extension from a host script and runs the
//! very same script against the source imported as plain Python, so CPython
//! is the oracle for every line of [`DRIVER`], including the stored
//! object's reference-count delta, a subclass instance in a base-typed
//! slot, a `bool` in an `int` slot read back by the getter and by compiled
//! arithmetic, a host-stored `str` a compiled method then replaces, a store
//! of a value a compiled method then promotes past the inline range, `del` and re-store, and a copy's
//! independent slots. [`EXT_ONLY_DRIVER`] pins the documented differences
//! (D-244's #1443 amendment): a value outside the parameter row is refused
//! with that row's `TypeError` or `OverflowError` (a `float` slot refuses an
//! `int`, as a `float` parameter does); a name with no
//! descriptor has nowhere to go; a carrier whose `__init__` never ran
//! cannot be stored into; and the object
//! a store replaces or a `del` removes keeps the reference the slot held
//! (the compiled object store's own #1092 leak).
//!
//! The hosted test is `#[ignore]`d and contributes no line coverage; the
//! Tier-1 `native-build-test` leg runs it with
//! `cargo test --workspace -- --include-ignored`. The changed lines are
//! covered by `src/ext_build_tests/getset.rs` and the runtime tests.

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

/// The module under test. `Conf` has one slot of each carried type
/// (`int`, `float`, `bool`, `str`, an opaque object, an instance of a
/// published class) plus a list slot, which has no descriptor, and one
/// compiled reader per slot, so a host store is checked through compiled
/// code as well as through the getter. `Twig` subclasses the declared
/// `Leaf`; `Other` does not.
const MODULE: &str = r#"from typing import Any, List


class Leaf:
    def __init__(self, k: int) -> None:
        self.k = k


class Twig(Leaf):
    def __init__(self, k: int, j: int) -> None:
        self.k = k
        self.j = j


class Other:
    def __init__(self, k: int) -> None:
        self.k = k


class Conf:
    def __init__(self, n: int, f: float, b: bool, s: str, o: Any, leaf: Leaf) -> None:
        self.n = n
        self.f = f
        self.b = b
        self.s = s
        self.o = o
        self.leaf = leaf
        self.xs: List[int] = []

    def n_c(self) -> int:
        return self.n

    def f_c(self) -> float:
        return self.f

    def b_c(self) -> bool:
        return self.b

    def s_c(self) -> str:
        return self.s

    def o_c(self) -> Any:
        return self.o

    def leaf_k(self) -> int:
        return self.leaf.k

    def plus(self) -> int:
        return self.n + 100

    def restr(self) -> str:
        self.s = self.s + "!"
        return self.s

    def grow(self) -> None:
        self.n = self.n * self.n

    def text(self) -> str:
        return f"{self.n}"
"#;

/// The host script both sides run. Every line is compared with CPython.
const DRIVER: &str = r#"import copy
import sys
import pycc_field_set_mod as m
o = object()
c = m.Conf(3, 1.5, True, "ab", o, m.Leaf(1))
c.n = 5
c.f = 2.5
c.b = False
c.s = "xy"
print(c.n, c.n_c(), c.f, c.f_c(), c.b, c.b_c(), c.s, c.s_c())
o2 = object()
before = sys.getrefcount(o2)
c.o = o2
print(sys.getrefcount(o2) - before, c.o is o2, c.o_c() is o2)
t = m.Twig(7, 8)
c.leaf = t
print(c.leaf is t, c.leaf_k(), type(c.leaf).__name__)
c.n = True
print(c.n, c.plus())
c.s = "q" * 3
print(c.restr(), c.s, c.s_c())
c.n = 2**31
c.grow()
print(c.text())
c.n = 1
print(c.n, c.text())
del c.n
print(hasattr(c, 'n'))
try:
    c.n_c()
except AttributeError as e:
    print('AttributeError', e)
try:
    del c.n
except AttributeError as e:
    print('AttributeError', e)
c.n = 4
print(c.n, c.plus())
del c.s
c.s = "back"
print(c.s_c())
blank = m.Conf.__new__(m.Conf)
try:
    del blank.n
except AttributeError as e:
    print('AttributeError', e)
y = copy.copy(c)
y.n = 50
y.s = "copy"
print(c.n, c.s, y.n, y.s)
"#;

const DRIVER_OUT: &str = "5 5 2.5 2.5 False False xy xy\n\
    1 True True\n\
    True 7 Twig\n\
    True 101\n\
    qqq! qqq! qqq!\n\
    4611686018427387904\n\
    1 1\n\
    False\n\
    AttributeError 'Conf' object has no attribute 'n'\n\
    AttributeError 'Conf' object has no attribute 'n'\n\
    4 104\n\
    back\n\
    AttributeError 'Conf' object has no attribute 'n'\n\
    4 back 50 copy\n";

/// The documented differences from CPython, run against the extension only.
const EXT_ONLY_DRIVER: &str = r#"import sys
import pycc_field_set_mod as m
def attempt(f):
    try:
        f()
        print('ok')
    except (TypeError, AttributeError, OverflowError) as e:
        print(type(e).__name__, e)
def store(obj, name, value):
    return lambda: setattr(obj, name, value)
c = m.Conf(3, 1.5, True, "ab", object(), m.Leaf(1))
attempt(store(c, 'n', 'x'))
attempt(store(c, 'n', 2**70))
attempt(store(c, 'f', 'x'))
attempt(store(c, 'f', 1))
attempt(store(c, 'b', 1))
attempt(store(c, 's', 3))
attempt(store(c, 'leaf', object()))
attempt(store(c, 'leaf', m.Leaf.__new__(m.Leaf)))
attempt(store(c, 'leaf', m.Other(1)))
attempt(store(c, 'xs', [1]))
attempt(store(c, 'zzz', 1))
print(c.n, c.f, c.b, c.s, c.leaf.k)
attempt(store(m.Conf.__new__(m.Conf), 'n', 1))
o = object()
c = m.Conf(3, 1.5, True, "ab", o, m.Leaf(1))
before = sys.getrefcount(o)
c.o = object()
print(sys.getrefcount(o) - before)
c.o = o
before = sys.getrefcount(o)
del c.o
print(sys.getrefcount(o) - before)
"#;

const EXT_ONLY_OUT: &str = "TypeError Conf.n() argument 1: 'str' object cannot be interpreted as an integer\n\
    OverflowError Conf.n() argument 1: int is outside the inline-integer range [-2**62, 2**62-1] this pycc version's `ext` boundary supports (see #1040)\n\
    TypeError Conf.f() argument 1: 'str' object cannot be interpreted as a float\n\
    TypeError Conf.f() argument 1: 'int' object cannot be interpreted as a float\n\
    TypeError Conf.b() argument 1: 'int' object cannot be interpreted as a bool\n\
    TypeError Conf.s() argument 1: 'int' object cannot be interpreted as a str\n\
    TypeError Conf.leaf() argument 1 must be pycc_field_set_mod.Leaf, not object\n\
    TypeError Conf.leaf() argument 1: the pycc_field_set_mod.Leaf object is uninitialized (its __init__ never ran)\n\
    TypeError Conf.leaf() argument 1 must be pycc_field_set_mod.Leaf, not pycc_field_set_mod.Other\n\
    AttributeError 'pycc_field_set_mod.Conf' object has no attribute 'xs' and no __dict__ for setting new attributes\n\
    AttributeError 'pycc_field_set_mod.Conf' object has no attribute 'zzz' and no __dict__ for setting new attributes\n\
    3 1.5 True ab 1\n\
    AttributeError cannot set 'n' on a 'Conf' object whose __init__ never ran\n\
    0\n\
    0\n";

/// What CPython answers for the lines [`EXT_ONLY_DRIVER`] pins, so the
/// divergence is stated, not inferred: a dynamically typed attribute takes
/// any value, and the replaced or
/// deleted object's reference is dropped.
const EXT_ONLY_CPYTHON_OUT: &str = "ok\n\
    ok\n\
    ok\n\
    ok\n\
    ok\n\
    ok\n\
    ok\n\
    ok\n\
    ok\n\
    ok\n\
    ok\n\
    1180591620717411303424 1 1 3 1\n\
    ok\n\
    -1\n\
    -1\n";

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
fn a_host_store_into_a_compiled_field_matches_cpython() {
    let dir = ScratchDir::new("field_setter_hosted").expect("scratch");
    let ext = dir.join("ext");
    let oracle = dir.join("oracle");
    std::fs::create_dir_all(&ext).expect("ext dir");
    std::fs::create_dir_all(&oracle).expect("oracle dir");
    let source = write(&oracle, "pycc_field_set_mod.py", MODULE);
    let build = pycc()
        .arg("build")
        .arg(&source)
        .arg("-o")
        .arg(ext.join("pycc_field_set_mod"))
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
