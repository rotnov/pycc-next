//! #1457 (Part 2 of #1443): compiled code stores and deletes an attribute
//! through an object-typed name in an `--ext` module (`other.x = v` and
//! `del other.x` with `other: Any`, `object` or an unannotated private
//! helper's parameter), lowered to CPython's `PyObject_SetAttr` and
//! `PyObject_DelAttr` through `pycc_ext_obj_setattr` and
//! `pycc_ext_obj_delattr`.
//!
//! The hosted test drives the extension from a host script and runs the
//! very same script against the source imported as plain Python, so CPython
//! is the oracle for every line of [`DRIVER`]: a store of each admitted
//! value type and a `del` on a `types.SimpleNamespace`, CPython's
//! value-before-base order, the stored and removed object's reference
//! counts measured inside compiled code, the raising paths caught by a
//! compiled `try` and propagated to the host, and a store into each carried
//! slot of a compiled instance (Part 1's setters) read back through the
//! getter and through compiled methods. [`EXT_ONLY_DRIVER`] pins the
//! documented differences. No compiled function prints: the extension's
//! output and the host's are buffered separately, so every value is
//! returned to the host and printed there.
//!
//! The hosted tests are `#[ignore]`d and contribute no line coverage; the
//! Tier-1 `native-build-test` leg runs them with
//! `cargo test --workspace -- --include-ignored`. The changed lines are
//! covered by the unit tests in `crates/pycc_hir/src/stmt/del/tests.rs`,
//! `crates/pycc_types/src/foreign/attr_store/tests.rs`,
//! `crates/pycc_mir/src/obj_compare/tests.rs` and
//! `crates/pycc_codegen/src/tests/object_attr_store.rs`.

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

fn check_with(dir: &Path, body: &str) -> Output {
    pycc()
        .arg("check")
        .arg(write(dir, "m.py", body))
        .output()
        .expect("pycc should spawn")
}

/// `pycc check` of `body` fails with exactly one `code` diagnostic whose
/// text contains `needle`; returns the rendered diagnostics.
fn assert_one_error(tag: &str, body: &str, code: &str, needle: &str) -> String {
    let dir = ScratchDir::new(tag).expect("scratch");
    let output = check_with(&dir, body);
    assert_eq!(output.status.code(), Some(1), "{}", stderr_of(&output));
    let rendered = stdout_of(&output);
    assert_eq!(rendered.matches("error[").count(), 1, "{rendered}");
    assert!(rendered.contains(&format!("error[{code}]")), "{rendered}");
    assert!(rendered.contains(needle), "{rendered}");
    rendered
}

/// A native-mode `pycc check` admits a store and a `del` on an object a
/// foreign import produced, in a module body and in a function body, and
/// through a parameter annotated with a class a foreign import binds.
#[test]
fn check_accepts_an_object_store_and_delete() {
    let dir = ScratchDir::new("obj_attr_store_check").expect("scratch");
    let output = check_with(
        &dir,
        "import types\nfrom types import SimpleNamespace\n\nns = types.SimpleNamespace()\nns.a = 1\nns.b = None\nns.c = ns\n\
         del ns.a, ns.c\n\n\ndef f(v: int) -> None:\n    ns.d = v\n    del ns.d\n\n\n\
         def g(o: SimpleNamespace) -> None:\n    o.e = 1.5\n    del o.e\n",
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}{}",
        stdout_of(&output),
        stderr_of(&output)
    );
}

/// The shapes outside #1457 keep a refusal: a `del` of a native value's
/// attribute is a `C0001` located at the target, a store of an unpackable
/// value is the argument row's `I0404`, a store on a builtin value is
/// `T0043` with the store's own wording, and a walrus base keeps the HIR's
/// refusal.
#[test]
fn the_shapes_outside_1457_are_refused() {
    const HEAD: &str = "import types\n\nns = types.SimpleNamespace()\nxs = [1, 2]\n";
    for (tag, tail, code, needle, location) in [
        (
            "obj_attr_del_native",
            "del xs.n\n",
            "C0001",
            "a `del` of an attribute (`del obj.attr`) is supported only on a CPython object, \
             not on `list[int]`",
            Some(".py:5:5"),
        ),
        (
            "obj_attr_store_list_value",
            "ns.xs = xs\n",
            "I0404",
            "passing a `list[int]` argument to a CPython object's attribute store",
            None,
        ),
        (
            "obj_attr_store_int_base",
            "n: int = 1\nn.x = 2\n",
            "T0043",
            "cannot assign an attribute on `int`: it is not a class instance",
            None,
        ),
        (
            "obj_attr_del_walrus",
            "del (m := ns).x\n",
            "C0001",
            "a walrus assignment",
            Some(".py:5:5"),
        ),
    ] {
        let rendered = assert_one_error(tag, &format!("{HEAD}{tail}"), code, needle);
        if let Some(location) = location {
            assert!(rendered.contains(location), "{tag}: {rendered}");
        }
    }
}

/// The module under test. `Conf` has one slot of each carried type, each
/// with a compiled reader, so a store through an `Any` name is checked
/// through Part 1's getter and through compiled code. `poke_object` takes
/// an `object` parameter and `_poke` is an unannotated private helper whose
/// parameter the constraint solver types as the object. `order` logs into
/// a host list to show the value is evaluated before the base. `pin` and
/// `unpin` measure the value's reference count inside compiled code, so the
/// argument boundary's own references do not enter the count. The module
/// body stores and deletes on a `SimpleNamespace` of its own.
const MODULE: &str = r#"from typing import Any
import sys
import types


class Leaf:
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


def poke(other: Any, n: int, f: float, b: bool, s: str) -> None:
    other.n = n
    other.f = f
    other.b = b
    other.s = s


def poke_obj(other: Any, value: Any) -> None:
    other.o = value


def poke_leaf(other: Any, leaf: Leaf) -> None:
    other.leaf = leaf


def poke_none(other: Any) -> None:
    other.z = None


def poke_computed(other: Any, v: int) -> None:
    other.n = v + 1


def poke_object(other: object, v: int) -> None:
    other.n = v


def _poke(other, v: int) -> None:
    other.n = v
    del other.f


def via_helper(other: Any) -> None:
    _poke(other, 33)


def poke_big(other: Any, v: int) -> None:
    other.n = v * v


def store_text(other: Any, s: str) -> str:
    try:
        other.n = s
    except TypeError:
        return "TypeError"
    return "stored"


def zap_o(other: Any) -> None:
    del other.o


def zap(other: Any) -> None:
    del other.n


def zap_all(other: Any) -> None:
    del other.f, other.s


def zap_rest(other: Any) -> None:
    del other.b
    del other.leaf


def tag(other: Any, log: Any, label: str) -> Any:
    log.append(label)
    return other


def val(log: Any, label: str) -> int:
    log.append(label)
    return 9


def order(other: Any, log: Any) -> None:
    tag(other, log, "base").n = val(log, "value")
    del tag(other, log, "del base").n


def pin(other: Any, value: Any) -> int:
    before = int(sys.getrefcount(value))
    i = 0
    while i < 200:
        other.o = value
        del other.o
        other.o = value
        i += 1
    return int(sys.getrefcount(value)) - before


def unpin(other: Any, value: Any) -> int:
    before = int(sys.getrefcount(value))
    del other.o
    return int(sys.getrefcount(value)) - before


def catch(other: Any) -> str:
    out = ""
    try:
        del other.nope
    except AttributeError:
        out = out + "AttributeError del, "
    try:
        other.n = 1
    except AttributeError:
        out = out + "AttributeError store"
    return out


ns = types.SimpleNamespace()
ns.a = 1
ns.b = "two"
ns.c = ns
ns.d = None
del ns.c


def module_ns() -> Any:
    return ns
"#;

/// The host script both sides run. Every line is compared with CPython.
const DRIVER: &str = r#"import sys
import types
import pycc_obj_store_mod as m
print(m.module_ns())
ns = types.SimpleNamespace()
m.poke(ns, 5, 2.5, True, "xy")
print(ns.n, ns.f, ns.b, ns.s)
m.poke_none(ns)
print(ns.z)
m.poke_computed(ns, 4)
print(ns.n)
m.poke_object(ns, 6)
print(ns.n)
m.via_helper(ns)
print(ns.n, hasattr(ns, "f"))
leaf = m.Leaf(3)
m.poke_leaf(ns, leaf)
print(ns.leaf is leaf, ns.leaf.k)
m.zap(ns)
print(hasattr(ns, "n"))
try:
    m.zap(ns)
except AttributeError as e:
    print("AttributeError", e)
m.poke(ns, 5, 2.5, True, "xy")
m.zap_all(ns)
print(sorted(vars(ns)))
log = []
m.order(ns, log)
print(log, hasattr(ns, "n"))
v = object()
print(m.pin(ns, v), ns.o is v)
print(m.unpin(ns, v), hasattr(ns, "o"))
print(m.catch(len))
print(m.catch(object()))
try:
    m.poke(1, 1, 1.0, True, "s")
except AttributeError as e:
    print("AttributeError", e)
c = m.Conf(3, 1.5, True, "ab", object(), m.Leaf(1))
m.poke(c, 5, 2.5, False, "xy")
print(c.n, c.n_c(), c.f, c.f_c(), c.b, c.b_c(), c.s, c.s_c())
m.poke_computed(c, 40)
print(c.n, c.n_c())
m.poke_object(c, 41)
print(c.n_c())
m.poke_leaf(c, m.Leaf(7))
print(c.leaf.k, c.leaf_k())
o2 = object()
m.poke_obj(c, o2)
print(c.o is o2, c.o_c() is o2)
m.zap(c)
print(hasattr(c, "n"))
try:
    c.n_c()
except AttributeError as e:
    print("AttributeError", e)
try:
    m.zap(c)
except AttributeError as e:
    print("AttributeError", e)
m.poke(c, 8, 0.5, True, "back")
print(c.n_c(), c.s_c())
m.zap_all(c)
m.zap_o(c)
m.zap_rest(c)
print([hasattr(c, k) for k in ("n", "f", "b", "s", "o", "leaf")])
for reader in (c.f_c, c.b_c, c.s_c, c.o_c, c.leaf_k):
    try:
        reader()
    except AttributeError as e:
        print("AttributeError", e)
try:
    m.zap_o(c)
except AttributeError as e:
    print("AttributeError", e)
m.poke(c, 9, 4.5, False, "again")
m.poke_obj(c, o2)
m.poke_leaf(c, m.Leaf(2))
print(c.f_c(), c.b_c(), c.s_c(), c.o_c() is o2, c.leaf_k())
"#;

/// What [`DRIVER`] prints under CPython.
const DRIVER_OUT: &str = "\
namespace(a=1, b='two', d=None)\n\
5 2.5 True xy\n\
None\n\
5\n\
6\n\
33 False\n\
True 3\n\
False\n\
AttributeError 'types.SimpleNamespace' object has no attribute 'n'\n\
['b', 'leaf', 'n', 'z']\n\
['value', 'base', 'del base'] False\n\
1 True\n\
-1 False\n\
AttributeError del, AttributeError store\n\
AttributeError del, AttributeError store\n\
AttributeError 'int' object has no attribute 'n' and no __dict__ for setting new attributes\n\
5 5 2.5 2.5 False False xy xy\n\
41 41\n\
41\n\
7 7\n\
True True\n\
False\n\
AttributeError 'Conf' object has no attribute 'n'\n\
AttributeError 'Conf' object has no attribute 'n'\n\
8 back\n\
[True, False, False, False, False, False]\n\
AttributeError 'Conf' object has no attribute 'f'\n\
AttributeError 'Conf' object has no attribute 'b'\n\
AttributeError 'Conf' object has no attribute 's'\n\
AttributeError 'Conf' object has no attribute 'o'\n\
AttributeError 'Conf' object has no attribute 'leaf'\n\
AttributeError 'Conf' object has no attribute 'o'\n\
4.5 False again True 2\n";

/// The documented differences from CPython, run against the extension
/// only. A store into a compiled instance's typed slot is converted by the
/// slot's parameter row (Part 1 of #1443), so a `str` stored into an `int`
/// slot raises `TypeError`, which a compiled `except` catches. An object
/// slot of a compiled instance keeps the reference a replaced or deleted
/// object held (the #1092 leak Part 1 documents), so 400 stores and 200
/// deletions leave 400 references and a `del` drops none. A compiled `int`
/// past the inline range cannot be packed (#1040): the store raises
/// `OverflowError` and the attribute keeps its old value -- a failed pack
/// never becomes a deletion.
const EXT_ONLY_DRIVER: &str = r#"import types
import pycc_obj_store_mod as m
c = m.Conf(3, 1.5, True, "ab", object(), m.Leaf(1))
print(m.store_text(c, "x"), c.n)
o = object()
m.poke_obj(c, o)
print(m.pin(c, o))
print(m.unpin(c, o))
ns = types.SimpleNamespace(n=1)
try:
    m.poke_big(ns, 2**40)
except OverflowError as e:
    print("OverflowError", e)
print(ns.n)
"#;

/// What the extension prints for [`EXT_ONLY_DRIVER`].
const EXT_ONLY_OUT: &str = "\
TypeError 3\n\
400\n\
0\n\
OverflowError an int argument to a CPython object's method is outside the inline-integer range [-2**62, 2**62-1] this pycc version's `ext` boundary supports (see #1040)\n\
1\n";

/// What CPython answers for [`EXT_ONLY_DRIVER`]: a plain attribute takes
/// any value, and the replaced or deleted object's reference is dropped.
const EXT_ONLY_CPYTHON_OUT: &str = "\
stored x\n\
0\n\
-1\n\
1208925819614629174706176\n";

fn run(script: &str, path_entry: &Path, cwd: &Path) -> Output {
    host_python()
        .arg("-c")
        .arg(script)
        .env("PYTHONPATH", path_entry)
        .current_dir(cwd)
        .output()
        .expect("python3 should spawn")
}

fn build_ext(source: &Path, out: &Path) {
    let build = pycc()
        .arg("build")
        .arg(source)
        .arg("-o")
        .arg(out)
        .arg("--ext")
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", stderr_of(&build));
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_compiled_store_through_an_object_name_matches_cpython() {
    let dir = ScratchDir::new("obj_attr_store_hosted").expect("scratch");
    let ext = dir.join("ext");
    let oracle = dir.join("oracle");
    std::fs::create_dir_all(&ext).expect("ext dir");
    std::fs::create_dir_all(&oracle).expect("oracle dir");
    let source = write(&oracle, "pycc_obj_store_mod.py", MODULE);
    build_ext(&source, &ext.join("pycc_obj_store_mod"));
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

/// A store or `del` raising in the module body fails the import with
/// CPython's exception and message, and the statement after it never runs.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_raising_module_body_store_or_delete_fails_the_import() {
    for (tag, module, tail) in [
        (
            "obj_attr_store_raise",
            "pycc_obj_attr_store_raise",
            "builtins.len.x = 1\nprint(\"unreached\")\n",
        ),
        (
            "obj_attr_del_raise",
            "pycc_obj_attr_del_raise",
            "del builtins.len.x\nprint(\"unreached\")\n",
        ),
    ] {
        let dir = ScratchDir::new(tag).expect("scratch");
        let ext = dir.join("ext");
        let oracle = dir.join("oracle");
        std::fs::create_dir_all(&ext).expect("ext dir");
        std::fs::create_dir_all(&oracle).expect("oracle dir");
        let body = format!("import builtins\n\n{tail}");
        let source = write(&oracle, &format!("{module}.py"), &body);
        build_ext(&source, &ext.join(module));
        let script = format!(
            "try:\n    import {module}\nexcept Exception as e:\n    \
             print(type(e).__name__, e)\nelse:\n    print('no error')\n"
        );
        let compiled = run(&script, &ext, &dir);
        assert_ok(&compiled);
        let cpython = run(&script, &oracle, &dir);
        assert_ok(&cpython);
        assert_eq!(stdout_of(&compiled), stdout_of(&cpython), "{tag}");
        assert!(stdout_of(&compiled).starts_with("AttributeError "), "{tag}");
    }
}
