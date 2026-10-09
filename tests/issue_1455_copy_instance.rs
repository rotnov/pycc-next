//! #1455: `copy.copy` of a compiled class instance held by the host. lark's
//! `InteractiveParser.accepts()` does `copy(self.parser_state.parse_conf)`
//! on a compiled `ParseConf`, which raised `cannot pickle` before this
//! change. Every carrier type now carries one shared `__copy__`
//! (`pycc_ext_instance_copy` in `src/ext/pycc_ext_module.c`): a new carrier
//! of the same type wrapping a clone of the compiled instance, driven by
//! the per-class kind table `src/ext_build/instance_copy.rs` generates.
//!
//! The hosted test drives the extension from a host script and runs the
//! very same script against the source imported as plain Python, so CPython
//! is the oracle for every line of [`DRIVER`], including the reference-count
//! deltas. [`EXT_ONLY_DRIVER`] pins the documented differences (D-244's
//! #1455 amendment): a class whose namespace takes part in the copy
//! protocol, and a PEP 695 template, are refused rather than copied
//! slot-wise; `deepcopy` and pickle stay refused; every carrier answers
//! `hasattr(x, '__copy__')`; and a copy's object-slot reference outlives
//! the copy, because the compiled clone is never freed (D-107, D-154).
//!
//! The hosted test is `#[ignore]`d and contributes no line coverage; the
//! Tier-1 `native-build-test` leg runs it with
//! `cargo test --workspace -- --include-ignored`. The changed lines are
//! covered by `src/ext_build_tests/instance_copy.rs` and the runtime tests.

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

/// The module under test. `Conf` is lark's generic `ParseConf` shape with
/// one slot of each kind the clone owns differently (`int`, `str`, an
/// opaque object, a leak-only list). `Sub` adds a slot to a base's. `Holder`
/// holds another compiled instance. `St` is copied never initialized.
/// `_Plain` and `_PrivCopy` publish nothing (private names), so they reach
/// the host only through a callback, on an on-demand carrier type.
/// `WithCopy`/`_PrivCopy` define `__copy__`, `DeepOnly` only `__deepcopy__`,
/// and `G` is a PEP 695 template.
const MODULE: &str = r#"from typing import Any, Generic, List, TypeVar

T = TypeVar("T")


class Conf(Generic[T]):
    def __init__(self, n: int, s: str, o: Any) -> None:
        self.n = n
        self.s = s
        self.o = o
        self.xs: List[int] = []

    def push(self, v: int) -> None:
        self.xs.append(v)

    def size(self) -> int:
        return len(self.xs)

    def bump(self) -> None:
        self.n = self.n + 100
        self.s = self.s + "!"

    def grow(self) -> None:
        self.n = self.n * self.n

    def text(self) -> str:
        return f"{self.n}"


def read(c: Conf[Any]) -> int:
    return c.n * 10


class Base:
    def __init__(self, a: int) -> None:
        self.a = a


class Sub(Base):
    def __init__(self, a: int, b: int) -> None:
        self.a = a
        self.b = b


class Inner:
    def __init__(self, k: int) -> None:
        self.k = k


class Holder:
    def __init__(self, b: Inner) -> None:
        self.b = b


class St:
    def __init__(self, parse_conf: Inner) -> None:
        self.parse_conf = parse_conf


class _Plain:
    def __init__(self, n: int) -> None:
        self.n = n


def plain(cb: object) -> object:
    return cb(_Plain(5))


def plain_n(p: _Plain) -> int:
    return p.n


class DeepOnly:
    def __init__(self, n: int) -> None:
        self.n = n

    def __deepcopy__(self, memo: Any) -> "DeepOnly":
        return DeepOnly(self.n + 1)


class WithCopy:
    def __init__(self, n: int) -> None:
        self.n = n

    def __copy__(self) -> "WithCopy":
        return WithCopy(self.n + 1)


class _PrivCopy:
    def __init__(self, n: int) -> None:
        self.n = n

    def __copy__(self) -> "_PrivCopy":
        return _PrivCopy(self.n + 1)


def priv(cb: object) -> object:
    return cb(_PrivCopy(1))


class G[U]:
    def __init__(self, x: U) -> None:
        self.x = x


def gen(cb: object) -> object:
    return cb(G[int](1))
"#;

/// The host script both sides run. Every line is compared with CPython.
const DRIVER: &str = r#"import copy
import sys
import pycc_copy_mod as m
o = object()
c = m.Conf(3, "ab", o)
c.push(1)
c.push(2)
y = copy.copy(c)
print(type(y) is m.Conf, y is not c, y.n, y.s, y.o is c.o)
print(m.read(y), m.read(c))
y.push(9)
print(c.size(), y.size())
y.bump()
print(c.n, c.s, y.n, y.s)
before = sys.getrefcount(c)
for _ in range(100):
    copy.copy(c)
print(sys.getrefcount(c) - before)
before = sys.getrefcount(o)
z = copy.copy(c)
print(sys.getrefcount(o) - before)
print(c.n, c.s, c.o is o)
s = copy.copy(m.Sub(20, 2))
print(type(s) is m.Sub, s.a, s.b)
h = m.Holder(m.Inner(4))
print(copy.copy(h).b is h.b)
blank = m.St.__new__(m.St)
w = copy.copy(blank)
print(type(w) is m.St, w is not blank)
try:
    w.parse_conf
except AttributeError as e:
    print('AttributeError', e)
print(m.plain(lambda p: m.plain_n(copy.copy(p))))
print(m.plain(lambda p: type(copy.copy(p)).__name__), m.plain(lambda p: copy.copy(p) is not p))
d = copy.copy(m.DeepOnly(7))
print(type(d) is m.DeepOnly, d.n)
big = m.Conf(2**31, "ab", o)
big.grow()
yb = copy.copy(big)
yb.bump()
big.bump()
print(big.text(), yb.text())
"#;

const DRIVER_OUT: &str = "True True 3 ab True\n\
    30 30\n\
    3 3\n\
    3 ab 103 ab!\n\
    0\n\
    1\n\
    3 ab True\n\
    True 20 2\n\
    True\n\
    True True\n\
    AttributeError 'St' object has no attribute 'parse_conf'\n\
    5\n\
    _Plain True\n\
    True 7\n\
    4611686018427388004 4611686018427388004\n";

/// The documented differences from CPython, run against the extension only.
const EXT_ONLY_DRIVER: &str = r#"import copy
import pickle
import sys
import pycc_copy_mod as m
def attempt(f):
    try:
        r = f()
        print('ok', type(r).__name__)
    except (TypeError, AttributeError) as e:
        print(type(e).__name__, e)
attempt(lambda: copy.copy(m.WithCopy(1)))
print(m.priv(lambda p: attempt(lambda: copy.copy(p))))
print(m.gen(lambda g: attempt(lambda: copy.copy(g))))
attempt(lambda: copy.copy(m.WithCopy.__new__(m.WithCopy)))
c = m.Conf(3, "ab", object())
attempt(lambda: copy.deepcopy(c))
attempt(lambda: pickle.dumps(c))
print(hasattr(c, '__copy__'), hasattr(m.Conf, '__deepcopy__'))
o = object()
c = m.Conf(3, "ab", o)
before = sys.getrefcount(o)
y = copy.copy(c)
del y
print(sys.getrefcount(o) - before)
"#;

const EXT_ONLY_OUT: &str = "TypeError cannot copy 'pycc_copy_mod.WithCopy' object: its compiled __copy__ is not published\n\
    TypeError cannot copy 'pycc_copy_mod._PrivCopy' object: its compiled __copy__ is not published\n\
    None\n\
    TypeError cannot copy 'pycc_copy_mod.G' object: its generic specializations share one instance layout\n\
    None\n\
    TypeError cannot copy 'pycc_copy_mod.WithCopy' object: its compiled __copy__ is not published\n\
    TypeError cannot pickle 'pycc_copy_mod.Conf' object\n\
    TypeError cannot pickle 'pycc_copy_mod.Conf' object\n\
    True False\n\
    1\n";

/// What CPython answers for the lines [`EXT_ONLY_DRIVER`] pins, so the
/// divergence is stated, not inferred: the user's `__copy__` runs (and,
/// on a never-initialized instance, raises its own `AttributeError`), a
/// generic copies, deepcopy and pickle succeed, a Python class has no
/// `__copy__` attribute, and the copy's reference dies with it.
const EXT_ONLY_CPYTHON_OUT: &str = "ok WithCopy\n\
    ok _PrivCopy\n\
    None\n\
    ok G\n\
    None\n\
    AttributeError 'WithCopy' object has no attribute 'n'\n\
    ok Conf\n\
    ok bytes\n\
    False False\n\
    0\n";

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
fn copy_copy_of_a_compiled_instance_matches_cpython() {
    let dir = ScratchDir::new("copy_instance_hosted").expect("scratch");
    let ext = dir.join("ext");
    let oracle = dir.join("oracle");
    std::fs::create_dir_all(&ext).expect("ext dir");
    std::fs::create_dir_all(&oracle).expect("oracle dir");
    let source = write(&oracle, "pycc_copy_mod.py", MODULE);
    let build = pycc()
        .arg("build")
        .arg(&source)
        .arg("-o")
        .arg(ext.join("pycc_copy_mod"))
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
