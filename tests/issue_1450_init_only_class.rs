//! Part 2 of #1447 (#1450): a class compiled in an `--ext` module whose only
//! member is `__init__` -- lark's `ParseConf` (#1207) -- is published, so
//! the host can name it, construct it, read its fields and pass the
//! instance to a compiled function, exactly as with CPython's own import of
//! the source. `class Empty: pass`, constructible through the implicit
//! `object.__init__`, is published the same way.
//!
//! The hosted tests build the source with `pycc build --ext`, import the
//! artifact into the host CPython, and compare a driver script's output
//! with the same script run against CPython's own import of the source.
//! They are `#[ignore]`d and contribute no line coverage; the Tier-1
//! `native-build-test` leg runs them with `cargo test --workspace --
//! --include-ignored`. The changed lines are covered by
//! `src/ext_build_tests/init_only_publication.rs` and
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

/// The module both sides import. `ParseConf` has nothing but `__init__`
/// (lark's `ParseConf`, generic as there); `St` takes one in its
/// constructor, as lark's `ParserState` does, and one in a method
/// (`merge`). `Empty` has no member at all.
/// `Pair`'s only member is an `__init__` taking a `tuple`, which no
/// generated constructor can carry, so it stays unpublished (the pinned
/// deviation below). `Cell[int]` is a monomorphized specialization that
/// resolves no method: it must not be published, or the artifact references
/// an `__init__` function pointer no specialization has and fails to load.
const MODULE: &str = "from typing import Any, Generic, TypeVar\n\
    \n\
    T = TypeVar(\"T\")\n\
    \n\
    \n\
    class ParseConf(Generic[T]):\n    def __init__(self, n: int, name: str) -> None:\n        \
    self.n = n\n        self.name = name\n\
    \n\
    \n\
    class Empty:\n    pass\n\
    \n\
    \n\
    class Pair:\n    def __init__(self, ab: tuple[int, int]) -> None:\n        \
    self.a = 0\n\
    \n\
    \n\
    class St(Generic[T]):\n    def __init__(self, conf: ParseConf[T], k: int) -> None:\n        \
    self.conf = conf\n        self.k = k\n\n    \
    def total(self) -> int:\n        return self.k + self.conf.n\n\n    \
    def conf_of(self) -> ParseConf[T]:\n        return self.conf\n\n    \
    def merge(self, other: ParseConf[T]) -> int:\n        return self.k * other.n\n\
    \n\
    \n\
    def conf_n(c: ParseConf[Any]) -> int:\n    return c.n\n\
    \n\
    \n\
    def make_conf(n: int) -> ParseConf[Any]:\n    return ParseConf(n, \"made\")\n\
    \n\
    \n\
    def make_empty() -> Empty:\n    return Empty()\n\
    \n\
    \n\
    class Cell[V]:\n    def __init__(self, v: V) -> None:\n        self.v = v\n\
    \n\
    \n\
    def cell_v() -> int:\n    return Cell[int](6).v\n";

/// The host-side driver; `{module}` is the module it imports as `mod`.
/// Reference counts are compared as deltas over many calls, never as
/// absolute values, which differ between CPython versions.
const DRIVER: &str = "import gc\n\
    import sys\n\
    import {module} as mod\n\
    c = mod.ParseConf(3, 'x')\n\
    print(type(c).__name__, type(c) is mod.ParseConf)\n\
    print(c.n, c.name)\n\
    s = mod.St(c, 4)\n\
    print(s.total(), s.conf_of() is c, s.merge(mod.ParseConf(5, 'y')))\n\
    print(mod.conf_n(c), mod.conf_n(mod.make_conf(9)))\n\
    m = mod.make_conf(5)\n\
    print(type(m) is mod.ParseConf, m.n, m.name)\n\
    print(isinstance(c, mod.ParseConf), isinstance(s, mod.ParseConf))\n\
    e = mod.Empty()\n\
    print(type(e).__name__, type(mod.make_empty()) is mod.Empty)\n\
    print(mod.cell_v())\n\
    for call in (lambda: mod.ParseConf(), lambda: mod.ParseConf(1, 'a', 3), \
    lambda: mod.Empty(1)):\n\
    \x20   try:\n\
    \x20       call()\n\
    \x20       print('admitted')\n\
    \x20   except TypeError:\n\
    \x20       print('TypeError')\n\
    before = sys.getrefcount(c)\n\
    for _ in range(200):\n\
    \x20   mod.conf_n(c)\n\
    \x20   s.conf_of()\n\
    gc.collect()\n\
    print(sys.getrefcount(c) - before)\n";

/// What `DRIVER` prints under CPython.
const EXPECTED: &str = "ParseConf True\n3 x\n7 True 20\n3 9\nTrue 5 made\nTrue False\n\
    Empty True\n6\nTypeError\nTypeError\nTypeError\n0\n";

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
fn an_init_only_class_is_constructed_and_passed_like_cpython() {
    let dir = ScratchDir::new("init_only_hosted").expect("scratch");
    let module = "pycc_init_only";
    build(&dir, module);
    let compiled = python(&dir, &DRIVER.replace("{module}", module));
    assert_ok(&compiled);
    let oracle = python(&dir, &DRIVER.replace("{module}", "m"));
    assert_ok(&oracle);
    assert_eq!(stdout_of(&compiled), stdout_of(&oracle));
    assert_eq!(stdout_of(&compiled), EXPECTED);
}

/// Where the artifact still differs from CPython, pinned so a change is
/// deliberate: a class with no method and no constructor the boundary can
/// generate (`Pair`'s `tuple` parameter) gets no type object. CPython
/// answers `True`. The keyword call to the generated constructor was the
/// second pinned deviation until #1461 bound host keywords; it is kept as
/// the check that it now matches CPython's `admitted`.
const DEVIATION_DRIVER: &str = "import {module} as mod\n\
    print(hasattr(mod, 'Pair'))\n\
    try:\n\
    \x20   mod.ParseConf(1, name='y')\n\
    \x20   print('admitted')\n\
    except TypeError:\n\
    \x20   print('TypeError')\n";

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_class_with_neither_method_nor_carriable_constructor_stays_unpublished() {
    let dir = ScratchDir::new("init_only_deviation").expect("scratch");
    let module = "pycc_init_only_dev";
    build(&dir, module);
    let compiled = python(&dir, &DEVIATION_DRIVER.replace("{module}", module));
    assert_ok(&compiled);
    assert_eq!(stdout_of(&compiled), "False\nadmitted\n");
    let oracle = python(&dir, &DEVIATION_DRIVER.replace("{module}", "m"));
    assert_ok(&oracle);
    assert_eq!(stdout_of(&oracle), "True\nadmitted\n");
}
