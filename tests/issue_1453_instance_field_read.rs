//! #1453: a field declared as another class compiled in the same `--ext`
//! module is readable from the host. lark's `ParserState.parse_conf:
//! ParseConf` is the shape: `InteractiveParser.accepts()` reads
//! `parser_state.parse_conf` from host code. Each constructible published
//! class's `Py_tp_getset` table (#1442) now describes such a slot
//! or `@property` too, packing the stored instance through #1449's egress
//! (`pycc_ext_pack_instance`), so the instance's live carrier is returned
//! again and two reads are the same object (`src/ext_build/getset.rs`).
//!
//! The hosted test drives the extension from a host script and runs the
//! very same script against the source imported as plain Python, so CPython
//! is the oracle for every line of [`DRIVER`], including the reference-count
//! deltas of the host reads. [`EXT_ONLY_DRIVER`] pins the documented
//! differences: the compiled `o.parse_conf` read in `conf_of` keeps one
//! reference per call -- the attribute-read temporary #1092 tracks, which
//! #1442's `other.state_stack` read keeps too, not the descriptor -- an
//! enum-typed slot gets no descriptor, and a host-side store of an object
//! that is not a carrier of the declared class is refused with the
//! parameter row's `TypeError` (Part 1 of #1443).
//!
//! The hosted test is `#[ignore]`d and contributes no line coverage; the
//! Tier-1 `native-build-test` leg runs it with
//! `cargo test --workspace -- --include-ignored`. The changed lines are
//! covered by the unit tests in `src/ext_build_tests/getset.rs`.

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

/// The module under test. `Conf` and `St` are lark's generic
/// `ParseConf`/`ParserState` pair: `St.parse_conf` is a `Conf[T]` slot and
/// `conf` a property returning it. `fresh` stores an instance compiled code
/// made, which has no carrier until the first host read. `B`/`SubB`/`Holder`
/// store a subclass instance in a base-typed slot. `Cfg` is lark's
/// `ParseConf` shape, a class whose only member is `__init__` (published
/// since #1450), held in `Wrap.c`. `color` is an enum slot.
/// `conf_of` reads the field off an `Any` operand, which compiled code does
/// through `PyObject_GetAttr` (D-258).
const MODULE: &str = r#"from enum import Enum
from typing import Any, Generic, TypeVar

T = TypeVar("T")


class Color(Enum):
    RED = 1


class Conf(Generic[T]):
    def __init__(self, n: int) -> None:
        self.n = n

    def get(self) -> int:
        return self.n


class St(Generic[T]):
    parse_conf: Conf[T]
    color: Color

    def __init__(self, parse_conf: Conf[T], k: int) -> None:
        self.parse_conf = parse_conf
        self.color = Color.RED
        self.k = k

    @property
    def conf(self) -> Conf[T]:
        return self.parse_conf

    def fresh(self) -> None:
        self.parse_conf = Conf(self.k)


class B:
    def __init__(self, n: int) -> None:
        self.n = n

    def get(self) -> int:
        return self.n


class SubB(B):
    def tag(self) -> int:
        return 2


class Holder:
    def __init__(self, b: B) -> None:
        self.b = b

    def get(self) -> int:
        return self.b.n


class Cfg:
    def __init__(self, n: int) -> None:
        self.n = n


class Wrap:
    def __init__(self, c: Cfg) -> None:
        self.c = c

    def renew(self) -> None:
        self.c = Cfg(9)


def conf_of(o: Any) -> Any:
    return o.parse_conf
"#;

/// The host script both sides run. Every line is compared with CPython.
const DRIVER: &str = r#"import sys
import pycc_instance_field_mod as m
c = m.Conf(3)
s = m.St(c, 7)
print(s.parse_conf is c, s.conf is c, m.conf_of(s) is c, s.parse_conf.get())
print(type(s.parse_conf).__name__, s.k)
before = sys.getrefcount(c)
for _ in range(100):
    s.parse_conf
    s.conf
print(sys.getrefcount(c) - before)
s.fresh()
x = s.parse_conf
print(x is c, x.get(), type(x).__name__, x is s.parse_conf, s.conf is x, m.conf_of(s) is x)
before = sys.getrefcount(x)
for _ in range(100):
    s.parse_conf
    s.conf
print(sys.getrefcount(x) - before)
del x
print(s.parse_conf is s.parse_conf, s.parse_conf.get())
h = m.Holder(m.SubB(5))
y = h.b
print(type(y).__name__, y is h.b, y.tag(), y.get(), h.get())
g = m.Cfg(4)
w = m.Wrap(g)
print(w.c is g, w.c.n, type(w.c).__name__)
w.renew()
print(w.c is g, w.c.n, w.c is w.c)
blank = m.St.__new__(m.St)
try:
    blank.parse_conf
except AttributeError as e:
    print('AttributeError', e)
print(hasattr(blank, 'conf'), getattr(blank, 'parse_conf', 'dflt'))
"#;

const DRIVER_OUT: &str = "True True True 3\n\
    Conf 7\n\
    0\n\
    False 7 Conf True True True\n\
    0\n\
    True 7\n\
    SubB True 2 5 5\n\
    True 4 Cfg\n\
    False 9 True\n\
    AttributeError 'St' object has no attribute 'parse_conf'\n\
    False dflt\n";

/// The documented differences from CPython, run against the extension only.
const EXT_ONLY_DRIVER: &str = r#"import sys
import pycc_instance_field_mod as m
c = m.Conf(3)
s = m.St(c, 7)
before = sys.getrefcount(c)
for _ in range(100):
    m.conf_of(s)
print(sys.getrefcount(c) - before)
print(hasattr(s, 'color'))
try:
    s.parse_conf = object()
except TypeError as e:
    print('TypeError', e)
print(s.parse_conf is c)
"#;

const EXT_ONLY_OUT: &str = "100\n\
    False\n\
    TypeError St.parse_conf() argument 1 must be pycc_instance_field_mod.Conf, not object\n\
    True\n";

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
fn an_instance_typed_field_reads_back_the_same_carrier_like_cpython() {
    let dir = ScratchDir::new("instance_field_hosted").expect("scratch");
    let ext = dir.join("ext");
    let oracle = dir.join("oracle");
    std::fs::create_dir_all(&ext).expect("ext dir");
    std::fs::create_dir_all(&oracle).expect("oracle dir");
    let source = write(&oracle, "pycc_instance_field_mod.py", MODULE);
    let build = pycc()
        .arg("build")
        .arg(&source)
        .arg("-o")
        .arg(ext.join("pycc_instance_field_mod"))
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
