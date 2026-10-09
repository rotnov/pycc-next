//! Part 1 of #1447 (#1449): an instance of a regular class compiled in the
//! same `--ext` module, passed *into* a published function or method as an
//! argument and handed *back* as a result.
//!
//! Ingress hands the compiled body the instance the host's carrier holds,
//! after `pycc_ext_unpack_instance` (`src/ext/pycc_ext_module.c`) checks the
//! instance's run-time class against the declared class's MRO; egress is
//! #1435's carrier packer, so a returned instance that already has a live
//! carrier -- `self`, or an argument the host still holds -- is that very
//! object. The module mirrors lark's `ParserState`/`ParseConf` shape
//! (#1207): generic classes, a constructor taking another class of the
//! module, and a `copy` returning the class itself.
//!
//! The hosted tests build the source with `pycc build --ext`, import the
//! artifact into the host CPython, and compare a driver script's output
//! with the same script run against CPython's own import of the source.
//! They are `#[ignore]`d and contribute no line coverage; the Tier-1
//! `native-build-test` leg runs them with `cargo test --workspace --
//! --include-ignored`. The changed lines are covered by
//! `src/ext_build_tests/instance_boundary.rs`,
//! `src/ext_build_tests/refusal_completeness.rs` and
//! `src/ext_build_tests/exports.rs`.

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

/// The module both sides import. `Conf` and its subclass `SubConf` are
/// the argument classes; `St` takes a `Conf` in its constructor (lark's
/// `ParserState(parse_conf, ...)`), hands out `self` (`me`), a fresh
/// instance of itself (`copy`, lark's `ParserState.copy`) and the instance
/// it stored (`conf_of`), and takes an instance argument in an instance
/// method, a static method and a class method. `make_conf` and `conf_n`
/// are the module-level forms. `Other` is a class of the same module that
/// is not on `Conf`'s MRO.
const MODULE: &str = "from typing import Any, Generic, TypeVar\n\
    \n\
    T = TypeVar(\"T\")\n\
    \n\
    \n\
    class Conf(Generic[T]):\n    def __init__(self, n: int) -> None:\n        self.n = n\n\n    \
    def get(self) -> int:\n        return self.n\n\
    \n\
    \n\
    class SubConf(Conf):\n    def extra(self) -> int:\n        return self.n + 100\n\
    \n\
    \n\
    class Other:\n    def __init__(self, k: int) -> None:\n        self.k = k\n\n    \
    def get(self) -> int:\n        return self.k\n\
    \n\
    \n\
    class St(Generic[T]):\n    def __init__(self, conf: Conf[T], k: int) -> None:\n        \
    self.conf = conf\n        self.k = k\n\n    \
    def kk(self) -> int:\n        return self.k + self.conf.n\n\n    \
    def me(self) -> 'St[T]':\n        return self\n\n    \
    def copy(self) -> 'St[T]':\n        return St(self.conf, self.k + 1)\n\n    \
    def conf_of(self) -> Conf[T]:\n        return self.conf\n\n    \
    def take(self, c: Conf[T]) -> int:\n        return c.n * 10\n\n    \
    @staticmethod\n    def peek(c: Conf[Any]) -> int:\n        return c.n + 1\n\n    \
    @classmethod\n    def build(cls, c: Conf[Any]) -> 'St[Any]':\n        return St(c, 0)\n\
    \n\
    \n\
    def make_conf(n: int) -> Conf[Any]:\n    return Conf(n)\n\
    \n\
    \n\
    def conf_n(c: Conf[Any]) -> int:\n    return c.n\n";

/// The host-side driver; `{module}` is the module it imports as `mod`.
/// Reference counts are compared as deltas over many calls, never as
/// absolute values, which differ between CPython versions.
const DRIVER: &str = "import gc\n\
    import sys\n\
    import {module} as mod\n\
    c = mod.Conf(3)\n\
    s = mod.St(c, 4)\n\
    print(s.kk())\n\
    print(s.me() is s)\n\
    d = s.copy()\n\
    print(type(d).__name__, type(d) is mod.St, d.kk(), d is s)\n\
    print(s.conf_of() is c, d.conf_of() is c)\n\
    print(s.take(mod.Conf(7)), s.take(mod.SubConf(8)))\n\
    print(mod.St(mod.SubConf(1), 2).kk())\n\
    print(mod.St.peek(c), s.peek(c))\n\
    b = mod.St.build(c)\n\
    print(type(b).__name__, b.kk(), b.conf_of() is c)\n\
    m = mod.make_conf(5)\n\
    print(type(m).__name__, type(m) is mod.Conf, m.get())\n\
    print(mod.conf_n(c), mod.conf_n(mod.SubConf(9)))\n\
    before = sys.getrefcount(c)\n\
    for _ in range(200):\n\
    \x20   s.take(c)\n\
    \x20   mod.conf_n(c)\n\
    \x20   mod.St.peek(c)\n\
    gc.collect()\n\
    print(sys.getrefcount(c) - before)\n\
    before = sys.getrefcount(s)\n\
    for _ in range(200):\n\
    \x20   s.me()\n\
    gc.collect()\n\
    print(sys.getrefcount(s) - before)\n\
    held = s.conf_of()\n\
    before = sys.getrefcount(held)\n\
    for _ in range(200):\n\
    \x20   s.conf_of()\n\
    gc.collect()\n\
    print(sys.getrefcount(held) - before, held is c)\n";

/// What `DRIVER` prints under CPython.
const EXPECTED: &str = "7\nTrue\nSt True 8 False\nTrue True\n70 80\n3\n4 4\n\
    St 3 True\nConf True 5\n3 9\n0\n0\n0 True\n";

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

fn build(dir: &Path, module: &str) {
    let build = pycc()
        .arg("build")
        .arg(write(dir, "m.py", MODULE))
        .arg("-o")
        .arg(dir.join(module))
        .arg("--ext")
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", stderr_of(&build));
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_same_module_instance_crosses_in_and_out_like_cpython() {
    let dir = ScratchDir::new("same_mod_inst_hosted").expect("scratch");
    let module = "pycc_same_mod_inst";
    build(&dir, module);
    let compiled = python(&dir, &DRIVER.replace("{module}", module));
    assert_ok(&compiled);
    let oracle = python(&dir, &DRIVER.replace("{module}", "m"));
    assert_ok(&oracle);
    assert_eq!(stdout_of(&compiled), stdout_of(&oracle));
    assert_eq!(stdout_of(&compiled), EXPECTED);
}

/// The ingress refusals, which CPython does not perform at all -- an
/// annotation is not checked at run time, so CPython would run each body
/// and fail (or not) on its own terms. pycc's compiled body dereferences the
/// instance, so every argument that is not an initialized instance of the
/// declared class from this very module is a `TypeError` before the body
/// runs. The second module is a separate build of the same source, so its
/// `Conf` has the same name and is still refused: its carrier deallocates
/// through that artifact's own copy of the shim.
const REFUSAL_DRIVER: &str = "import pycc_same_mod_inst_a as mod\n\
    import pycc_same_mod_inst_b as other\n\
    s = mod.St(mod.Conf(1), 2)\n\
    for bad in (mod.Other(1), 5, None, object(), mod.Conf.__new__(mod.Conf), other.Conf(1)):\n\
    \x20   try:\n\
    \x20       s.take(bad)\n\
    \x20       print('admitted')\n\
    \x20   except TypeError as e:\n\
    \x20       print(e)\n\
    for call in (lambda: mod.St(mod.Other(1), 2), lambda: mod.conf_n(3), \
    lambda: mod.St.peek(other.Conf(1))):\n\
    \x20   try:\n\
    \x20       call()\n\
    \x20       print('admitted')\n\
    \x20   except TypeError as e:\n\
    \x20       print(e)\n\
    print(s.take(mod.Conf(4)))\n";

const REFUSAL_PINNED: &str = "St.take() argument 1 must be pycc_same_mod_inst_a.Conf, not \
    pycc_same_mod_inst_a.Other\n\
    St.take() argument 1 must be pycc_same_mod_inst_a.Conf, not int\n\
    St.take() argument 1 must be pycc_same_mod_inst_a.Conf, not NoneType\n\
    St.take() argument 1 must be pycc_same_mod_inst_a.Conf, not object\n\
    St.take() argument 1: the pycc_same_mod_inst_a.Conf object is uninitialized (its __init__ \
    never ran)\n\
    St.take() argument 1 must be pycc_same_mod_inst_a.Conf, not pycc_same_mod_inst_b.Conf\n\
    St.__init__() argument 1 must be pycc_same_mod_inst_a.Conf, not pycc_same_mod_inst_a.Other\n\
    conf_n() argument 1 must be pycc_same_mod_inst_a.Conf, not int\n\
    St.peek() argument 1 must be pycc_same_mod_inst_a.Conf, not pycc_same_mod_inst_b.Conf\n\
    40\n";

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn an_argument_that_is_not_an_instance_of_the_declared_class_is_refused() {
    let dir = ScratchDir::new("same_mod_inst_refusal").expect("scratch");
    build(&dir, "pycc_same_mod_inst_a");
    build(&dir, "pycc_same_mod_inst_b");
    let compiled = python(&dir, REFUSAL_DRIVER);
    assert_ok(&compiled);
    assert_eq!(stdout_of(&compiled), REFUSAL_PINNED);
}

/// A class the boundary cannot carry -- an enum, an exception class --
/// stays the `C0003` capability gap at both positions, naming the class.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn an_enum_or_exception_instance_is_still_a_capability_gap() {
    let dir = ScratchDir::new("same_mod_inst_gap").expect("scratch");
    let source = "from enum import Enum\n\n\nclass Color(Enum):\n    RED = 1\n\n\n\
        class Boom(Exception):\n    pass\n\n\n\
        class Box:\n    def __init__(self, n: int) -> None:\n        self.n = n\n\n    \
        def paint(self, c: Color) -> int:\n        return 1\n\n    \
        def err(self) -> Boom:\n        raise Boom(\"x\")\n";
    let build = pycc()
        .arg("build")
        .arg(write(&dir, "m.py", source))
        .arg("-o")
        .arg(dir.join("pycc_same_mod_inst_gap"))
        .arg("--ext")
        .output()
        .expect("pycc should spawn");
    assert_eq!(build.status.code(), Some(1), "{}", stderr_of(&build));
    let rendered = stderr_of(&build);
    assert_eq!(rendered.matches("error[C0003]").count(), 2, "{rendered}");
    assert!(
        rendered.contains("`Box.paint`: its parameter `c: Color`"),
        "{rendered}"
    );
    assert!(
        rendered.contains("`Box.err`: its return type `-> Boom`"),
        "{rendered}"
    );
}
