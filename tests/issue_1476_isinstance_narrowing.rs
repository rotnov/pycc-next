//! #1476 (Part 3 of #1387): an `isinstance(o, C)` guard narrows an
//! `object` name back to `int`, `float`, `bool`, `str` or a class compiled
//! in the module, for the reads the guard dominates. A narrowed read unboxes
//! the `PyObject *` once (`pycc_ext_obj_unbox_*`); a read that packs the
//! value straight back into an object (a return to `object`, an identity
//! test, a list element) hands over the object itself.
//!
//! [`DRIVER`] runs against the `--ext` artifact of [`MODULE`] and against
//! the same source imported as plain Python, so CPython is the oracle for
//! every line. The deliberate deviations are pinned separately: an `int`
//! past 64 bits raises `OverflowError` at a narrowed read, a compiled-class
//! carrier whose `__init__` never ran raises `TypeError` where CPython
//! raises `AttributeError`, an object whose `__class__` property answers
//! `int` or a compiled class passes the guard but raises `TypeError` at the
//! narrowed read, a `str` subclass handed to a native `str`
//! parameter or flowing through a native expression (a conditional
//! expression's arm, an annotated binding) comes back a plain `str`, and a
//! native use of an `int`,
//! `float` or `str` subclass instance runs the base type's operation, not
//! an override.
//!
//! Two guards do not narrow: one on a class that has a compiled subclass
//! keeps the object, so an override runs through CPython's own lookup
//! ([`a_class_with_a_compiled_subclass_keeps_the_object`]), and one whose
//! class name a module function shadows is refused. An `else` that
//! rebinds the name ends the narrowing for the rest of the block
//! ([`an_else_that_rebinds_the_name_leaves_the_continuation_an_object`]).
//!
//! The hosted tests are `#[ignore]`d for the reason every `ext` test is;
//! the refusals need no interpreter and run everywhere.

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

fn run(script: &str, path_entry: &Path, cwd: &Path) -> Output {
    host_python()
        .arg("-c")
        .arg(script)
        .env("PYTHONPATH", path_entry)
        .current_dir(cwd)
        .output()
        .expect("python3 should spawn")
}

/// Every narrowing shape, each reached from a public function the driver
/// calls.
const MODULE: &str = r#"class Token:
    def __init__(self, kind: str, value: int) -> None:
        self.kind = kind
        self.value = value

    def doubled(self) -> int:
        return self.value * 2


def as_int(o: object) -> int:
    if isinstance(o, int):
        return o + 1
    return -1


def as_float(o: object) -> float:
    if isinstance(o, float):
        return o * 2.0
    return -1.0


def as_bool(o: object) -> str:
    if isinstance(o, bool):
        if o:
            return "yes"
        return "no"
    return "not bool"


def as_str(o: object) -> str:
    if not isinstance(o, str):
        return "?"
    return o + "!"


def as_token(o: object) -> int:
    if isinstance(o, Token):
        return o.value + o.doubled()
    return 0


def kind_of(o: object) -> str:
    if isinstance(o, Token):
        return o.kind
    return "none"


def int_then_bool(o: object) -> int:
    if isinstance(o, int):
        if isinstance(o, bool):
            return -2
        return o + 10
    return 0


def same(o: object) -> object:
    if isinstance(o, str):
        return o
    return None


def is_same(o: object, p: object) -> bool:
    if isinstance(o, str):
        return o is p
    return False


def wrap(o: object) -> object:
    if isinstance(o, str):
        r: object = [o]
        return r
    return None


def shifted(o: object, n: int) -> int:
    xs = [i for i in range(n)]
    if isinstance(o, int):
        ys = [x + o for x in xs]
        total = 0
        for y in ys:
            total = total + y
        return total
    return -1


def keep(o: object) -> int:
    if isinstance(o, Token):
        t: Token = o
        o = None
        return t.value
    return -1


def _echo(s: str) -> str:
    return s


def relay(o: object) -> object:
    if isinstance(o, str):
        return _echo(o)
    return None


def pick(o: object, flag: bool) -> object:
    if isinstance(o, str):
        return o if flag else "x"
    return None


def annotated(o: object) -> object:
    if isinstance(o, str):
        y: str = o
        return y
    return None


def back(o: object) -> int:
    if isinstance(o, int):
        return o
    return -1


def text(o: object) -> str:
    if isinstance(o, int):
        return f"/{o}"
    return "?"


def show(o: object) -> None:
    if isinstance(o, int):
        print(o, o + 0)


def in_range(o: object) -> bool:
    if isinstance(o, int):
        return 0 < o < 10
    return False


def guard_in_try(o: object) -> int:
    try:
        if not isinstance(o, int):
            return 0
    except Exception:
        return 1
    return o + 1


def rebind_then_finally(o: object, p: object) -> str:
    seen = "-"
    if isinstance(o, int):
        try:
            o = p
        finally:
            seen = f"{o}"
    return seen


def rebind_then_else(o: object, p: object) -> str:
    if isinstance(o, int):
        try:
            o = p
        except Exception:
            return "e"
        else:
            return f"{o}"
    return "?"


def handler_rebinds(o: object) -> int:
    if not isinstance(o, int):
        return 0
    try:
        pass
    except Exception:
        o = None
        return -1
    return o + 1
"#;

/// A second module compiling a class of the same name: its instances are
/// not [`MODULE`]'s `Token`.
const OTHER: &str = r#"class Token:
    def __init__(self, kind: str, value: int) -> None:
        self.kind = kind
        self.value = value

    def get(self) -> int:
        return self.value
"#;

/// A `str` subclass instance keeps its identity through every use that
/// packs it back; a host-side class named `Token` is not the compiled one;
/// host `int` and `float` subclasses narrow like their bases; a `bool`
/// read under an `int` guard stays `True` wherever it flows back out (D-141);
/// a same-named class compiled in [`OTHER`] does not pass the guard.
const DRIVER: &str = r#"import narrowing as m
import other


class S(str):
    pass


class Token:
    value = 99


class I(int):
    pass


class F(float):
    pass


# First: the compiled `print` bypasses the host's buffered `sys.stdout`.
m.show(True)
t = m.Token("name", 4)
s = S("a")
print(m.as_int(41), m.as_int(True), m.as_int("x"), m.as_int(2.0))
print(m.as_float(1.25), m.as_float(3))
print(m.as_bool(True), m.as_bool(False), m.as_bool(1))
print(m.as_str("hi"), m.as_str(s), type(m.as_str(s)).__name__, m.as_str(3))
print(m.as_token(t), m.as_token(Token()), m.kind_of(t), m.kind_of("t"))
print(m.int_then_bool(True), m.int_then_bool(5), m.int_then_bool("5"))
print(m.same(s) is s, type(m.same(s)).__name__, m.is_same(s, s), m.is_same(s, S("a")))
w = m.wrap(s)
print(type(w).__name__, w[0] is s, type(w[0]).__name__)
print(m.shifted(2, 4), m.shifted("2", 4))
print(m.keep(t), m.keep(3))
print(m.as_int(I(5)), m.as_float(F(1.5)), m.int_then_bool(I(1)))
r = m.back(True)
print(r, type(r).__name__, m.back(7), m.text(True), m.text(3))
print(m.in_range(5), m.in_range(10), m.in_range("5"))
print(m.as_token(other.Token("x", 4)), m.kind_of(other.Token("x", 4)))
print(m.guard_in_try(4), m.guard_in_try("x"), m.rebind_then_finally(1, "zz"), m.rebind_then_finally("a", 2))
print(m.rebind_then_else(1, "yy"), m.rebind_then_else("a", 1), m.handler_rebinds(4), m.handler_rebinds("q"))
"#;

/// CPython 3.14.7's own output for [`DRIVER`] over [`MODULE`].
const EXPECTED: &str = "True 1
42 2 -1 -1
2.5 -1.0
yes no not bool
hi! a! str ?
12 0 name none
-2 15 0
True S True False
list True S
14 -1
4 -1
6 3.0 11
True bool 7 /True /3
True False False
0 none
5 0 zz -
yy ? 5 0
";

fn build_module(dir: &Path) -> (PathBuf, PathBuf) {
    let src_dir = dir.join("src");
    let out_dir = dir.join("out");
    std::fs::create_dir_all(&src_dir).expect("create the source directory");
    std::fs::create_dir_all(&out_dir).expect("create the output directory");
    for (name, body) in [("narrowing", MODULE), ("other", OTHER)] {
        let source = write(&src_dir, &format!("{name}.py"), body);
        let build = pycc()
            .arg("build")
            .arg(&source)
            .arg("-o")
            .arg(out_dir.join(name))
            .arg("--ext")
            .output()
            .expect("pycc should spawn");
        assert_ok(&build);
    }
    (src_dir, out_dir)
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn every_narrowing_shape_matches_cpython_in_an_ext_module() {
    let dir = ScratchDir::new("ext_1476_narrowing").expect("scratch");
    let (src_dir, out_dir) = build_module(&dir);
    let oracle = run(DRIVER, &src_dir, &dir);
    assert_ok(&oracle);
    assert_eq!(stdout_of(&oracle), EXPECTED, "CPython's own answer");
    let compiled = run(DRIVER, &out_dir, &dir);
    assert_ok(&compiled);
    assert_eq!(stdout_of(&compiled), EXPECTED);
}

/// [`the_narrowed_read_deviations_are_the_documented_ones`]'s driver. The
/// two `__class__` spoofs pass CPython's `isinstance` (which consults a
/// `__class__` attribute) without being an `int` or a compiled `Token`.
const DEVIATIONS: &str = r#"import narrowing as m


class S(str):
    def __add__(self, other):
        return 'over'


class FakeInt:
    __class__ = property(lambda self: int)

    def __add__(self, other):
        return 7


class FakeToken:
    __class__ = property(lambda self: m.Token)
    value = 1

    def doubled(self):
        return 2


for f in (lambda: m.as_int(2**70), lambda: m.as_token(m.Token.__new__(m.Token)),
          lambda: m.as_int(FakeInt()), lambda: m.as_token(FakeToken())):
    try:
        print(f())
    except Exception as e:
        print(type(e).__name__, e)
print(type(m.relay(S('a'))).__name__, m.as_str(S('a')))
s = S('b')
print(type(m.pick(s, True)).__name__, m.pick(s, True) is s, type(m.annotated(s)).__name__)
"#;

/// The deliberate deviations, each against CPython's own answer:
/// - an `int` past 64 bits does not fit the native `int` a narrowed read
///   produces (#1040);
/// - a carrier whose `__init__` never ran has no native instance to hand
///   over;
/// - an object whose `__class__` property answers `int` or a compiled class
///   passes the guard but has no native representation, so the narrowed
///   read raises `TypeError` where CPython runs the guarded body on it;
/// - a `str` subclass handed to a native `str` parameter is copied as a
///   plain `str`, and a native use runs `str`'s own `+`, not a subclass's
///   overriding `__add__`;
/// - a narrowed read that flows through a native expression -- a
///   conditional expression's arm, an annotated binding -- is a native
///   value, so boxing it back copies a `str` subclass into a new plain
///   `str`: only a narrowed read that is itself the boxed operand keeps
///   the object (the identity peephole, `pycc_codegen::object_unbox`).
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn the_narrowed_read_deviations_are_the_documented_ones() {
    let dir = ScratchDir::new("ext_1476_deviations").expect("scratch");
    let (src_dir, out_dir) = build_module(&dir);
    let oracle = run(DEVIATIONS, &src_dir, &dir);
    assert_ok(&oracle);
    assert_eq!(
        stdout_of(&oracle),
        "1180591620717411303425\n\
         AttributeError 'Token' object has no attribute 'value'\n\
         7\n3\nS over\nS True S\n"
    );
    let compiled = run(DEVIATIONS, &out_dir, &dir);
    assert_ok(&compiled);
    assert_eq!(
        stdout_of(&compiled),
        "OverflowError an int narrowed by isinstance() is outside the inline-integer range \
         [-2**62, 2**62-1] this pycc version's `ext` boundary supports (see #1040)\n\
         TypeError the narrowed narrowing.Token object is uninitialized (its __init__ never ran)\n\
         TypeError isinstance() held for a 'FakeInt' object, but only an int has the native \
         representation the narrowed read needs\n\
         TypeError narrowed object must be a compiled narrowing.Token instance, not FakeToken\n\
         str a!\nstr False str\n"
    );
}

/// A module-level class spelled like a builtin shadows it, in CPython and
/// in the guard: `isinstance(o, int)` tests the compiled `int`, and the
/// guarded body reads `o` as that class's instance, not as a native `int`.
const SHADOWING: &str = r#"class int:
    def __init__(self, v: float) -> None:
        self.v = v


def kind(o: object) -> str:
    if isinstance(o, int):
        return "compiled"
    return "other"


def field(o: object) -> float:
    if isinstance(o, int):
        return o.v
    return 0.0
"#;

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_module_class_named_int_is_the_class_the_guard_narrows_to() {
    let dir = ScratchDir::new("ext_1476_shadowing").expect("scratch");
    let src_dir = dir.join("src");
    let out_dir = dir.join("out");
    std::fs::create_dir_all(&src_dir).expect("create the source directory");
    std::fs::create_dir_all(&out_dir).expect("create the output directory");
    let build = pycc()
        .arg("build")
        .arg(write(&src_dir, "shadowing.py", SHADOWING))
        .arg("-o")
        .arg(out_dir.join("shadowing"))
        .arg("--ext")
        .output()
        .expect("pycc should spawn");
    assert_ok(&build);
    let script = "import shadowing as m\n\
                  print(m.kind(m.int(1.5)), m.kind(3), m.field(m.int(1.5)), m.field(3))\n";
    for path in [&src_dir, &out_dir] {
        let output = run(script, path, &dir);
        assert_ok(&output);
        assert_eq!(stdout_of(&output), "compiled other 1.5 0.0\n", "{path:?}");
    }
}

fn refused(name: &str, body: &str) -> String {
    let dir = ScratchDir::new(name).expect("scratch");
    let output = pycc()
        .arg("build")
        .arg(write(&dir, "m.py", body))
        .arg("-o")
        .arg(dir.join("m"))
        .arg("--ext")
        .env("PYCC_PYTHON", "/nonexistent/pycc-no-python")
        .output()
        .expect("pycc should spawn");
    let rendered = stderr_of(&output);
    assert_eq!(output.status.code(), Some(1), "{rendered}");
    rendered
}

/// Past the guard the name is an `object` again.
#[test]
fn a_read_outside_the_guard_is_still_refused() {
    let rendered = refused(
        "ext_1476_outside",
        "def f(o: object) -> int:\n    if isinstance(o, int):\n        pass\n    return o + 1\n",
    );
    assert!(
        rendered.contains("error[T0021]: operator Add is not defined for `object` and `int`"),
        "{rendered}"
    );
}

/// A tuple of classes names no single native type to narrow to.
#[test]
fn a_tuple_of_classes_does_not_narrow() {
    let rendered = refused(
        "ext_1476_tuple",
        "def f(o: object) -> int:\n    if isinstance(o, (int, str)):\n        return o + 1\n    \
         return 0\n",
    );
    assert!(
        rendered.contains("error[T0021]: operator Add is not defined for `object` and `int`"),
        "{rendered}"
    );
}

/// A guard on `Base`, which `Derived` subclasses and overrides, keeps `o`
/// an object: `o.who()` and `o.total()` go through CPython's own method
/// lookup, so `Derived`'s overrides run. `Derived` and `Leaf` have no
/// subclass and narrow as before.
const SUBCLASSED: &str = r#"class Base:
    def __init__(self, x: int) -> None:
        self.x = x

    def who(self) -> str:
        return "base"

    def total(self) -> int:
        return self.x


class Derived(Base):
    def __init__(self, x: int, y: int) -> None:
        super().__init__(x)
        self.y = y

    def who(self) -> str:
        return "derived"

    def total(self) -> int:
        return self.x + self.y


class Leaf:
    def __init__(self, v: int) -> None:
        self.v = v

    def who(self) -> str:
        return "leaf"


def name(o: object) -> object:
    if isinstance(o, Base):
        return o.who()
    return "other"


def total(o: object) -> object:
    if isinstance(o, Base):
        return o.total()
    return -1


def field(o: object) -> object:
    if isinstance(o, Derived):
        return o.x + o.y
    if isinstance(o, Base):
        return o.x
    return -1


def leaf(o: object) -> str:
    if isinstance(o, Leaf):
        return o.who() + "!"
    return "other"
"#;

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_class_with_a_compiled_subclass_keeps_the_object() {
    let dir = ScratchDir::new("ext_1476_subclassed").expect("scratch");
    let src_dir = dir.join("src");
    let out_dir = dir.join("out");
    std::fs::create_dir_all(&src_dir).expect("create the source directory");
    std::fs::create_dir_all(&out_dir).expect("create the output directory");
    let build = pycc()
        .arg("build")
        .arg(write(&src_dir, "subclassed.py", SUBCLASSED))
        .arg("-o")
        .arg(out_dir.join("subclassed"))
        .arg("--ext")
        .output()
        .expect("pycc should spawn");
    assert_ok(&build);
    let script = "import subclassed as m\n\
                  for o in (m.Derived(1, 2), m.Base(1), 3):\n    \
                  print(m.name(o), m.total(o), m.field(o))\n\
                  print(m.leaf(m.Leaf(4)), m.leaf(m.Base(1)))\n";
    for path in [&src_dir, &out_dir] {
        let output = run(script, path, &dir);
        assert_ok(&output);
        assert_eq!(
            stdout_of(&output),
            "derived 3 3\nbase 1 1\nother -1 -1\nleaf! other\n",
            "{path:?}"
        );
    }
}

/// The same guard does not license a native use: the read stays an object.
#[test]
fn a_native_use_under_a_subclassed_class_guard_is_refused() {
    let rendered = refused(
        "ext_1476_subclassed_native",
        "class Base:\n    def who(self) -> str:\n        return \"base\"\n\n\n\
         class Derived(Base):\n    def who(self) -> str:\n        return \"derived\"\n\n\n\
         def name(o: object) -> str:\n    if isinstance(o, Base):\n        return o.who()\n    \
         return \"other\"\n",
    );
    assert!(
        rendered.contains("error[T0022]: return type mismatch: expected `str`, found `object`"),
        "{rendered}"
    );
}

/// A module function spelled like `int` shadows the builtin; CPython's
/// guard raises `TypeError` (argument 2 is not a class), pycc refuses it.
#[test]
fn a_function_spelled_like_a_guarded_class_is_refused() {
    let rendered = refused(
        "ext_1476_function_shadow",
        "def int(x: str) -> str:\n    return x\n\n\n\
         def f(o: object) -> str:\n    if isinstance(o, int):\n        return \"yes\"\n    \
         return \"no\"\n",
    );
    assert!(
        rendered.contains(
            "error[I0404]: testing a CPython object with `isinstance` against the function `int`"
        ),
        "{rendered}"
    );
}

/// `if not isinstance(o, int): return ...` narrows the rest of the block
/// only when the surviving `else` leaves `o` alone. Here it rebinds `o` to
/// a `float`, directly or in an `elif` arm, and the comparison after the
/// `if` must see that `float`, not unbox it as an `int`.
const REBOUND: &str = r#"def rebound_else(o: object) -> object:
    if not isinstance(o, int):
        return "no"
    else:
        o = 1.5
    return o == 1.5


def rebound_elif(o: object, flag: bool) -> object:
    if not isinstance(o, int):
        return "no"
    elif flag:
        o = 2.5
    return o == 2.5
"#;

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn an_else_that_rebinds_the_name_leaves_the_continuation_an_object() {
    let dir = ScratchDir::new("ext_1476_rebound").expect("scratch");
    let src_dir = dir.join("src");
    let out_dir = dir.join("out");
    std::fs::create_dir_all(&src_dir).expect("create the source directory");
    std::fs::create_dir_all(&out_dir).expect("create the output directory");
    let build = pycc()
        .arg("build")
        .arg(write(&src_dir, "rebound.py", REBOUND))
        .arg("-o")
        .arg(out_dir.join("rebound"))
        .arg("--ext")
        .output()
        .expect("pycc should spawn");
    assert_ok(&build);
    let script = "import rebound as m\n\
                  for v in (3, 'a'):\n    \
                  print(m.rebound_else(v), m.rebound_elif(v, True), m.rebound_elif(v, False))\n";
    for path in [&src_dir, &out_dir] {
        let output = run(script, path, &dir);
        assert_ok(&output);
        assert_eq!(
            stdout_of(&output),
            "True True False\nno no no\n",
            "{path:?}"
        );
    }
}

/// A native use after the `if` is refused: the continuation reads the
/// object.
#[test]
fn a_native_use_after_an_else_that_rebinds_the_name_is_refused() {
    let rendered = refused(
        "ext_1476_rebound_native",
        "def f(o: object) -> int:\n    if not isinstance(o, int):\n        return 0\n    \
         else:\n        o = 1.5\n    return o + 1\n",
    );
    assert!(
        rendered.contains("error[T0021]: operator Add is not defined for `object` and `int`"),
        "{rendered}"
    );
}
