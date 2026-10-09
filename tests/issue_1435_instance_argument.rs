//! #1435: an instance of a pycc class passed as a positional or keyword
//! argument to a call on a CPython object, in an `--ext` build.
//!
//! The instance crosses as a `PyccExtInstance` carrier of its run-time
//! class (`pycc_ext_obj_pack_instance` in `src/ext/pycc_ext_module.c`): the
//! class's published type when it has one, otherwise a carrier
//! type named `<module>.<Class>`. Identity follows CPython while the
//! carrier lives -- `self` handed out by a method of a host-constructed
//! object is that object, and two crossings of one instance are `is`-equal.
//!
//! The hosted tests build the source with `pycc build --ext`, import the
//! artifact into the host CPython, and compare a driver script's output
//! with the same script run against CPython's own import of the source.
//! They are `#[ignore]`d and contribute no line coverage; the Tier-1
//! `native-build-test` leg runs them with `cargo test --workspace --
//! --include-ignored`. The changed lines are covered by
//! `crates/pycc_types/src/foreign/instance_arg_tests.rs`,
//! `crates/pycc_codegen/src/foreign_call/tests/call_tests.rs`,
//! `crates/pycc_rt/src/instance/carrier.rs`,
//! `src/ext_build_tests/generated_c.rs` and
//! `src/ext_build_tests/compiled_isinstance.rs`.

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

/// `pycc check` of `body` fails with exactly one `code` diagnostic whose
/// text contains `needle`.
fn assert_one_error(tag: &str, body: &str, code: &str, needle: &str) {
    let dir = ScratchDir::new(tag).expect("scratch");
    let output = pycc()
        .arg("check")
        .arg(write(&dir, "m.py", body))
        .output()
        .expect("pycc should spawn");
    assert_eq!(output.status.code(), Some(1), "{}", stderr_of(&output));
    let rendered = stdout_of(&output);
    assert_eq!(rendered.matches("error[").count(), 1, "{rendered}");
    assert!(rendered.contains(&format!("error[{code}]")), "{rendered}");
    assert!(rendered.contains(needle), "{rendered}");
}

/// The module both sides import. `Q` is published and constructible, `R`
/// inherits `Q`'s methods (so an inherited body hands out a derived
/// `self`), `P` takes another pycc class in its constructor (lark's
/// `ParserState` shape) and `Hidden` publishes nothing at all -- it has no
/// method, and its `tuple` constructor parameter is one no generated
/// constructor carries, so #1450's constructible-class publication does not
/// reach it and it crosses on a type the artifact never publishes (since
/// #1448 a hidden carrier type carrying its `n` descriptor; the embedded
/// test below still exercises the shim's on-demand type); `_D` is a
/// private, so unpublished, subclass of the published `Q`. Compiled
/// `isinstance` on a carrier that comes back from the host answers from its
/// run-time class (`back_hidden`, `back_private`), and `kw`/`kwself` pass an
/// instance as a keyword value (lark's `state=self`). `G` is a
/// module-level instance that outlives its first carrier: the driver's
/// first crossing keeps no reference (a call result `g` returns is leaked,
/// so that crossing returns an `int`, not the carrier), the carrier dies
/// with the call's argument array, and crossing `G` again proves the dying
/// carrier unlinked itself (`pycc_ext_instance_dealloc`) rather than leaving
/// a dangling back-pointer the packer would reuse -- the `junk` carriers
/// refill the freed block, so a reused dangling pointer would answer some
/// other `Q`.
const MODULE: &str = "class Conf:\n    def __init__(self, n: int) -> None:\n        self.n = n\n\
    \n\
    \n\
    class P:\n    def __init__(self, c: Conf) -> None:\n        self.c = c\n\n    \
    def go(self, cb: object) -> object:\n        return cb(self)\n\
    \n\
    \n\
    class Q:\n    def __init__(self, n: int) -> None:\n        self.n = n\n\n    \
    def go(self, cb: object) -> object:\n        return cb(self)\n\n    \
    def twice(self, cb: object) -> object:\n        return cb(self, self)\n\n    \
    def mixed(self, cb: object) -> object:\n        return cb(1, self, \"s\", cb)\n\n    \
    def size(self) -> int:\n        return self.n\n\n    \
    def kwself(self, cb: object) -> object:\n        return cb(state=self)\n\
    \n\
    \n\
    class R(Q):\n    def size(self) -> int:\n        return self.n * 10\n\
    \n\
    \n\
    class Hidden:\n    def __init__(self, n: int, ab: tuple[int, int]) -> None:\n        \
    self.n = n\n\
    \n\
    \n\
    def make_p(cb: object) -> object:\n    return P(Conf(4)).go(cb)\n\
    \n\
    \n\
    def hidden(cb: object) -> object:\n    return cb(Hidden(5, (0, 0)))\n\
    \n\
    \n\
    def sub(cb: object) -> object:\n    return R(2).go(cb)\n\
    \n\
    \n\
    def keep(store: object) -> object:\n    q = Q(7)\n    store.append(q)\n    \
    store.append(q)\n    return store\n\
    \n\
    \n\
    def spin(cb: object) -> int:\n    i = 0\n    while i < 200:\n        cb(Q(i))\n        \
    i += 1\n    return i\n\
    \n\
    \n\
    G = Q(9)\n\
    \n\
    \n\
    def g(cb: object) -> object:\n    return cb(G)\n\
    \n\
    \n\
    class _D(Q):\n    pass\n\
    \n\
    \n\
    def kw(cb: object) -> object:\n    return cb(1, state=Hidden(6, (0, 0)))\n\
    \n\
    \n\
    def back_hidden(cb: object) -> bool:\n    r = cb(Hidden(5, (0, 0)))\n    return isinstance(r, Hidden)\n\
    \n\
    \n\
    def back_private(cb: object) -> bool:\n    r = cb(_D(3))\n    \
    return isinstance(r, Q) and isinstance(r, _D) and not isinstance(r, Hidden)\n";

/// The host-side driver; `{module}` is the module it imports as `mod`.
const DRIVER: &str = "import gc\n\
    import sys\n\
    import {module} as mod\n\
    q = mod.Q(3)\n\
    print(q.go(lambda x: x) is q)\n\
    print(type(q.go(lambda x: x)).__name__)\n\
    a, b = q.twice(lambda x, y: (x, y))\n\
    print(a is b, a is q)\n\
    print(q.go(lambda x: x.size()))\n\
    print(q.mixed(lambda *args: (args[0], args[1] is q, args[2])))\n\
    s = mod.sub(lambda x: x)\n\
    print(type(s).__name__, type(s) is mod.R, s.size())\n\
    p = mod.make_p(lambda x: x)\n\
    print(type(p).__name__, type(p).__module__ == mod.__name__)\n\
    h = mod.hidden(lambda x: x)\n\
    print(type(h).__name__, type(h).__qualname__)\n\
    try:\n\
    \x20   type(h)()\n\
    except TypeError:\n\
    \x20   print('TypeError')\n\
    store = mod.keep([])\n\
    print(store[0] is store[1], store[0].size(), type(store[0]) is mod.Q)\n\
    del store\n\
    gc.collect()\n\
    store = mod.keep([])\n\
    print(store[0] is store[1])\n\
    seen = []\n\
    print(mod.spin(lambda x: seen.append(type(x).__name__)), set(seen))\n\
    before = sys.getrefcount(mod.Q)\n\
    mod.spin(lambda x: None)\n\
    gc.collect()\n\
    print(sys.getrefcount(mod.Q) == before)\n\
    held = q.go(lambda x: [x])\n\
    del held\n\
    gc.collect()\n\
    print(q.go(lambda x: x) is q)\n\
    try:\n\
    \x20   q.go(lambda x: 1 / 0)\n\
    except ZeroDivisionError:\n\
    \x20   print('ZeroDivisionError')\n\
    print(mod.g(lambda x: x.size()))\n\
    gc.collect()\n\
    junk = [mod.Q(i) for i in range(1000)]\n\
    y = mod.g(lambda x: x)\n\
    print(y.size(), mod.g(lambda x: x) is y)\n\
    print(q.kwself(lambda state: state is q))\n\
    print(mod.kw(lambda n, state: (n, type(state).__name__)))\n\
    print(mod.back_hidden(lambda x: x), mod.back_hidden(lambda x: 1))\n\
    print(mod.back_private(lambda x: x))\n";

/// What `DRIVER` prints under CPython.
const EXPECTED: &str = "True\nQ\nTrue True\n3\n(1, True, 's')\nR True 20\nP True\n\
    Hidden Hidden\nTypeError\nTrue 7 True\nTrue\n200 {'Q'}\nTrue\nTrue\nZeroDivisionError\n9\n9 True\n\
    True\n(1, 'Hidden')\nTrue False\nTrue\n";

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

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn an_instance_argument_crosses_like_cpython() {
    let dir = ScratchDir::new("inst_arg_hosted").expect("scratch");
    let module = "pycc_inst_arg_mod";
    let build = pycc()
        .arg("build")
        .arg(write(&dir, "m.py", MODULE))
        .arg("-o")
        .arg(dir.join(module))
        .arg("--ext")
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", stderr_of(&build));
    let compiled = python(&dir, &DRIVER.replace("{module}", module));
    assert_ok(&compiled);
    let oracle = python(&dir, &DRIVER.replace("{module}", "m"));
    assert_ok(&oracle);
    assert_eq!(stdout_of(&compiled), stdout_of(&oracle));
    assert_eq!(stdout_of(&compiled), EXPECTED);
}

/// The carrier's documented deviations from CPython (`docs/RUNTIME.md`,
/// "A pycc instance argument crosses as a carrier of its run-time class"), pinned at their current values so a later fix
/// flips them deliberately. Each line's CPython value is in its comment.
const DEVIATION_MODULE: &str = "class Q:\n    def __init__(self, n: int) -> None:\n        \
    self.n = n\n\n    def go(self, cb: object) -> object:\n        return cb(self)\n\n\n\
    class R(Q):\n    def size(self) -> int:\n        return self.n * 10\n\n\n\
    class Same:\n    def __init__(self, n: int, ab: tuple[int, int]) -> None:\n        \
    self.n = n\n\n    \
    def __eq__(self, other: object) -> bool:\n        return True\n\n    \
    def __repr__(self) -> str:\n        return \"Same!\"\n\n\n\
    def sub(cb: object) -> object:\n    return R(2).go(cb)\n\n\n\
    def same(cb: object) -> object:\n    return cb(Same(1, (0, 0)), Same(2, (0, 0)))\n\n\n\
    class Esc:\n    def __init__(self, cb: object) -> None:\n        self.n = 1\n        \
    cb(self)\n\n    def size(self) -> int:\n        return self.n\n";

const DEVIATION_DRIVER: &str = "import pycc_inst_dev_mod as mod\n\
    s = mod.sub(lambda x: x)\n\
    print(isinstance(s, mod.Q))\n\
    print(hasattr(s, 'n'))\n\
    a, b = mod.same(lambda a, b: (a, b))\n\
    print(hasattr(a, 'n'))\n\
    print(a == b)\n\
    print(repr(a).startswith('<pycc_inst_dev_mod.Same object at '))\n\
    seen = []\n\
    e = mod.Esc(seen.append)\n\
    print(seen[0] is e, type(seen[0]) is mod.Esc)\n";

/// `isinstance` against a base: CPython `True` (published carrier types are
/// flat); attribute read through `R`'s carrier: CPython `True`, and since
/// #1442 pycc's too (a constructible class's type carries a descriptor
/// per field, writable for a slot since Part 1 of #1443); attribute read
/// through `Same`'s carrier: CPython `True`, and since #1448 pycc's too --
/// `Same`'s `tuple` constructor parameter keeps it unpublished, and its
/// hidden carrier type now carries its field descriptors beside #1427's
/// comparison slot;
/// `__eq__` override: CPython `True`, and since #1427 pycc's too (the
/// carrier type's `tp_richcompare` runs the compiled `__eq__`); `__repr__`
/// override: CPython `Same!` (`__repr__` is not wired to a type slot); a
/// `self` escaping during `__init__`:
/// CPython `True True` (the escape is packed before `tp_init` links the
/// host's object, so it gets its own carrier of the same type).
const DEVIATION_PINNED: &str = "False\nTrue\nTrue\nTrue\nTrue\nFalse True\n";

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn the_carrier_deviations_from_cpython_are_pinned() {
    let dir = ScratchDir::new("inst_arg_deviation").expect("scratch");
    let build = pycc()
        .arg("build")
        .arg(write(&dir, "m.py", DEVIATION_MODULE))
        .arg("-o")
        .arg(dir.join("pycc_inst_dev_mod"))
        .arg("--ext")
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", stderr_of(&build));
    let compiled = python(&dir, DEVIATION_DRIVER);
    assert_ok(&compiled);
    assert_eq!(stdout_of(&compiled), DEVIATION_PINNED);
}

/// An embedded executable (D-248) passes an instance to a CPython call the
/// same way: no class is published, so every instance crosses as the lazy
/// carrier type (only the shared `__copy__`, #1455), named after
/// `__main__` as CPython names it.
const EMBEDDED: &str = "import builtins\n\n\n\
    class Q:\n    def __init__(self, n: int) -> None:\n        self.n = n\n\n\n\
    q = Q(1)\n\
    print(builtins.type(q).__name__)\n\
    print(builtins.repr(q)[:12])\n\
    h = builtins.list()\n\
    h.append(q)\n\
    h.append(q)\n\
    print(h[0] is h[1])\n\
    print(builtins.type(q) is builtins.type(Q(2)))\n\
    r = h[0]\n\
    print(isinstance(r, Q))\n";

#[test]
#[ignore = "needs a relocatable CPython 3.14 as python3.14 or PYCC_PYTHON; run with --include-ignored"]
fn an_embedded_executable_passes_an_instance_like_cpython() {
    let dir = ScratchDir::new("inst_arg_embedded").expect("scratch");
    let source = write(&dir, "m.py", EMBEDDED);
    let build = pycc()
        .arg("build")
        .arg(&source)
        .arg("-o")
        .arg(dir.join("app"))
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", stderr_of(&build));
    let embedded = Command::new(dir.join("app"))
        .output()
        .expect("the embedded binary runs");
    assert_ok(&embedded);
    let oracle = host_python()
        .arg(&source)
        .output()
        .expect("python3 should spawn");
    assert_ok(&oracle);
    assert_eq!(stdout_of(&embedded), stdout_of(&oracle));
    assert_eq!(stdout_of(&embedded), "Q\n<__main__.Q \nTrue\nTrue\nTrue\n");
}

/// The instance is admitted as a positional or keyword call argument of a
/// regular class only; every other shape keeps a diagnostic, never a panic.
#[test]
fn the_shapes_outside_issue_1435_are_refused() {
    const HEAD: &str = "import builtins\n\ncb = builtins.len\n\n\n\
        class Q:\n    def __init__(self, n: int) -> None:\n        self.n = n\n\n\n";
    for (tag, tail, code, needle) in [
        (
            "inst_arg_enum",
            "from enum import Enum\n\n\nclass Color(Enum):\n    RED = 1\n\n\ncb(Color.RED)\n",
            "I0404",
            "passing a `Color` argument to a CPython object's call",
        ),
        (
            "inst_arg_exception",
            "class E(Exception):\n    def __init__(self, n: int) -> None:\n        \
             self.n = n\n\n\ncb(E(1))\n",
            "I0404",
            "passing a `E` argument to a CPython object's call",
        ),
        (
            "inst_arg_cls",
            "class K:\n    def __init__(self, n: int) -> None:\n        self.n = n\n\n    \
             @classmethod\n    def make(cls) -> None:\n        cb(cls)\n",
            "I0404",
            "passing a class method's `cls` argument to a CPython object's call",
        ),
        // #1470 admits a regular instance as a key and a comparison
        // operand; an enum member and a class method's `cls` stay refused.
        (
            "inst_key_enum",
            "from enum import Enum\n\n\nclass Color(Enum):\n    RED = 1\n\n\n\
             x = builtins.__dict__[Color.RED]\n",
            "I0404",
            "indexing a CPython object with a `Color` key",
        ),
        (
            "inst_compare_cls",
            "class K:\n    def __init__(self, n: int) -> None:\n        self.n = n\n\n    \
             @classmethod\n    def make(cls) -> None:\n        print(cb == cls)\n",
            "I0404",
            "comparing a CPython object with a class method's `cls`",
        ),
        (
            "inst_arg_keyword_cls",
            "class K:\n    def __init__(self, n: int) -> None:\n        self.n = n\n\n    \
             @classmethod\n    def make(cls) -> None:\n        cb(state=cls)\n",
            "I0404",
            "passing a class method's `cls` argument to a CPython object's call",
        ),
    ] {
        assert_one_error(tag, &format!("{HEAD}{tail}"), code, needle);
    }
}
