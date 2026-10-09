//! #1475 (Part 2 of #1387): a native `int`, `float`, `bool`, `str`, `None`
//! or compiled-class value moves into an `object` slot by boxing it into
//! the `PyObject *` CPython would hold -- a call argument of every call
//! shape, an annotated binding and its later rebinding, a returned value,
//! an attribute store and a property setter's parameter. Before this change
//! each of those positions was a `T0021`/`T0022`/`T0023`/`T0025` mismatch,
//! because `object` admitted only `object`.
//!
//! [`DRIVER`] runs against the `--ext` artifact of [`MODULE`] and against
//! the same source imported as plain Python, so CPython is the oracle for
//! every line except the one deliberate deviation pinned separately:
//! a native `int` beyond 64 bits boxes through the scalar packer, which
//! raises `OverflowError` (#1040) where CPython returns the value.
//!
//! [`EMBEDDED`] is a foreign-class annotation, which is `object` too
//! (Part 1 of #1367), receiving a native `int` in an embedded executable,
//! compared byte for byte with CPython 3.14.7.
//!
//! The hosted tests are `#[ignore]`d for the reason every `ext` and embed
//! test is; the refusals need no interpreter and run everywhere.

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

/// Every boxing seam, each reached from a public function the driver calls.
const MODULE: &str = r#"import sys


class C:
    def __init__(self, v: int) -> None:
        self.v = v


class Holder:
    def __init__(self, x: object) -> None:
        self.x = x

    def put(self, x: object) -> object:
        self.x = x
        return self.x

    @staticmethod
    def st(x: object) -> object:
        return x


class Child(Holder):
    def __init__(self) -> None:
        super().__init__(2.5)

    @classmethod
    def make(cls, x: object) -> object:
        return x


class Prop:
    seen: object

    def __init__(self) -> None:
        self._v = 0
        self.seen = 0

    @property
    def v(self) -> int:
        return self._v

    @v.setter
    def v(self, value: object) -> None:
        self.seen = value


class Q:
    def __init__(self, v: int) -> None:
        self.v = v

    def __eq__(self, other) -> bool:
        if not isinstance(other, Q):
            return NotImplemented
        return self.v > 0


class E(Exception):
    def __init__(self, x: object) -> None:
        self.x = x


def f(x: object) -> object:
    return x


def kw(a: int, x: object) -> object:
    return x


def _helper(x: object) -> object:
    return x


def _branch(flag: bool) -> object:
    if flag:
        return 1
    return "s"


def _rebound() -> object:
    y: object = 4
    y = 2.5
    return y


def nothing() -> None:
    pass


def call_int() -> object:
    return f(3)


def call_str() -> object:
    return f("s")


def call_bool() -> object:
    return f(True)


def call_inst() -> object:
    return f(C(1))


def call_none() -> object:
    return f(None)


def call_kw() -> object:
    return kw(1, x="k")


def call_private() -> object:
    return _helper(2.5)


def branch_int() -> object:
    return _branch(True)


def branch_str() -> object:
    return _branch(False)


def rebound() -> object:
    return _rebound()


def bind() -> object:
    y: object = 4
    y = "s"
    return y


def ret_float() -> object:
    return 2.5


def ret_bool() -> object:
    return True


def ret_none_call() -> object:
    return nothing()


def ctor() -> object:
    return Holder(7).x


def method() -> object:
    return Holder(1).put(False)


def static() -> object:
    return Holder.st("t")


def super_init() -> object:
    return Child().x


def classm() -> object:
    return Child.make(9)


def setter() -> object:
    p = Prop()
    p.v = "set"
    return p.seen


def attr_store() -> object:
    h = Holder(1)
    h.x = "z"
    return h.x


def exc_ctor() -> object:
    return E(5).x


def eq_explicit() -> object:
    return Q(1).__eq__(Q(2))


def eq_explicit_native() -> object:
    return Q(1).__eq__(3)


class Pair:
    def __init__(self, a: object, b: object) -> None:
        self.a = a
        self.b = b


def twice() -> Pair:
    c = C(4)
    return Pair(c, c)


for loop_item in sys.path:
    pass
loop_item = 5


def mirror() -> object:
    return loop_item


shadowed: object = "module"


def shadow() -> int:
    shadowed = 1
    return shadowed + 1


def big() -> object:
    n = 2 ** 70
    return n
"#;

const DRIVER: &str = r#"import boxing as m

for name in [
    "call_int", "call_str", "call_bool", "call_none", "call_kw", "call_private",
    "branch_int", "branch_str", "rebound", "bind", "ret_float", "ret_bool",
    "ret_none_call", "ctor", "method", "static", "super_init", "classm",
    "setter", "attr_store", "exc_ctor", "eq_explicit", "eq_explicit_native", "mirror", "shadow",
]:
    r = getattr(m, name)()
    print(name, type(r).__name__, r)
r = m.call_inst()
print("call_inst", type(r).__name__, r.v)
c = m.C(5)
print(m.f(c) is c, m.f(None) is None)
print(type(m.Prop().seen).__name__)
print(m.Q(1) == m.Q(2), m.Q(0) == m.Q(0), m.Q(1) == 3)
p = m.twice()
print(p.a is p.b, type(p.a).__name__, p.a.v)
"#;

/// What CPython prints for [`DRIVER`] over [`MODULE`]; the extension must
/// print the same.
const EXPECTED: &str = "call_int int 3\ncall_str str s\ncall_bool bool True\n\
                        call_none NoneType None\ncall_kw str k\ncall_private float 2.5\n\
                        branch_int int 1\nbranch_str str s\nrebound float 2.5\nbind str s\n\
                        ret_float float 2.5\nret_bool bool True\nret_none_call NoneType None\n\
                        ctor int 7\nmethod bool False\nstatic str t\nsuper_init float 2.5\n\
                        classm int 9\nsetter str set\nattr_store str z\nexc_ctor int 5\n\
                        eq_explicit bool True\neq_explicit_native NotImplementedType NotImplemented\nmirror int 5\nshadow int 2\n\
                        call_inst C 1\nTrue True\nint\nTrue False False\nTrue C 4\n";

/// Builds [`MODULE`] as `boxing` in `out`, its source in `src`.
fn build_module(dir: &Path) -> (PathBuf, PathBuf) {
    let src_dir = dir.join("src");
    let out_dir = dir.join("out");
    std::fs::create_dir_all(&src_dir).expect("create the source directory");
    std::fs::create_dir_all(&out_dir).expect("create the output directory");
    let source = write(&src_dir, "boxing.py", MODULE);
    let build = pycc()
        .arg("build")
        .arg(&source)
        .arg("-o")
        .arg(out_dir.join("boxing"))
        .arg("--ext")
        .output()
        .expect("pycc should spawn");
    assert_ok(&build);
    (src_dir, out_dir)
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn every_boxing_seam_matches_cpython_in_an_ext_module() {
    let dir = ScratchDir::new("ext_1475_boxing").expect("scratch");
    let (src_dir, out_dir) = build_module(&dir);
    let oracle = run(DRIVER, &src_dir, &dir);
    assert_ok(&oracle);
    assert_eq!(stdout_of(&oracle), EXPECTED, "CPython's own answer");
    let compiled = run(DRIVER, &out_dir, &dir);
    assert_ok(&compiled);
    assert_eq!(stdout_of(&compiled), EXPECTED);
}

/// The one deliberate deviation: a native `int` past 64 bits boxes through
/// the scalar packer, which raises `OverflowError` (#1040) instead of
/// producing the value CPython returns.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_bigint_boxed_into_object_raises_overflow_error() {
    let dir = ScratchDir::new("ext_1475_bigint").expect("scratch");
    let (src_dir, out_dir) = build_module(&dir);
    let script = "import boxing as m\ntry:\n    print(m.big())\nexcept OverflowError:\n    \
                  print('OverflowError')\n";
    let oracle = run(script, &src_dir, &dir);
    assert_ok(&oracle);
    assert_eq!(stdout_of(&oracle), "1180591620717411303424\n");
    let compiled = run(script, &out_dir, &dir);
    assert_ok(&compiled);
    assert_eq!(stdout_of(&compiled), "OverflowError\n");
}

/// A foreign-class annotation is `object` (Part 1 of #1367), so a native
/// `int`, `float`, `str`, `bool` or `None` boxes into it, as CPython (which
/// does not enforce annotations) runs it. A name bound to a foreign value
/// is `object` too, so rebinding it to a native value, at module level or
/// in a function, boxes that value. A native program cannot spell `object`
/// itself (`C0001`); these are the embedded half's seams.
#[cfg(not(windows))]
const EMBEDDED: &str = r#"import json
from fractions import Fraction


def show(x: Fraction) -> None:
    print(x)


def same(x: Fraction) -> Fraction:
    return x


def rebind() -> None:
    y = json.loads("1")
    y = "s"
    print(y)
    y = None
    print(y)


show(3)
show(Fraction(1, 2))
show(None)
print(same(2.5), same("s"), same(True))
x = json.loads("[1]")
x = 2
print(x)
rebind()
"#;

/// Builds `body` as an embedded executable and answers its bundle's
/// interpreter, asserted to be CPython 3.14.7 (as
/// `tests/issue_1223_embedded_executable.rs` does).
#[cfg(not(windows))]
fn build_embedded(dir: &Path, body: &str) -> PathBuf {
    let output = pycc()
        .arg("build")
        .arg(write(dir, "m.py", body))
        .arg("-o")
        .arg(dir.join("app"))
        .output()
        .expect("pycc should spawn");
    assert!(output.status.success(), "{}", stderr_of(&output));
    let marker = std::fs::read_to_string(dir.join("app.pycc").join("PYCC-BUNDLE"))
        .expect("an embedded build writes its marker");
    let mut lines = marker.lines();
    assert_eq!(lines.next(), Some("pycc-bundle 1"));
    assert_eq!(lines.next(), Some("python 3.14.7"), "{marker}");
    let executable = lines
        .next()
        .and_then(|line| line.strip_prefix("executable "))
        .expect("the marker names its interpreter");
    PathBuf::from(executable)
}

#[cfg(not(windows))]
#[test]
#[ignore = "needs CPython 3.14.7 as python3.14 or PYCC_PYTHON; run with --include-ignored"]
fn a_native_value_boxes_into_a_foreign_annotation_in_an_embedded_executable() {
    let dir = ScratchDir::new("embed_1475_boxing").expect("scratch");
    let python = build_embedded(&dir, EMBEDDED);
    let embedded = Command::new(dir.join("app"))
        .output()
        .expect("the embedded binary runs");
    assert_eq!(embedded.status.code(), Some(0), "{}", stderr_of(&embedded));
    let oracle = Command::new(python)
        .arg(dir.join("m.py"))
        .output()
        .expect("CPython runs the oracle program");
    assert_eq!(stdout_of(&oracle), "3\n1/2\nNone\n2.5 s True\n2\ns\nNone\n");
    assert_eq!(embedded.stdout, oracle.stdout);
}

/// Builds `body` as an `--ext` module with no interpreter reachable, so the
/// refusal is decided by the type checker alone, and answers stderr.
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

/// A native container keeps the strict rule: a boxed copy would break the
/// aliasing a CPython list shares (D-258 rule 4).
#[test]
fn a_native_list_argument_is_still_refused() {
    let rendered = refused(
        "ext_1475_list",
        "def f(x: object) -> object:\n    return x\n\n\ndef g() -> object:\n    return f([1, 2])\n",
    );
    assert!(
        rendered.contains("error[T0021]: argument 1 of `f` expects `object`, got `list[int]`"),
        "{rendered}"
    );
}

/// An `Optional` value has no single packer for its `None` and its payload.
#[test]
fn an_optional_argument_is_still_refused() {
    let rendered = refused(
        "ext_1475_optional",
        "def f(x: object) -> object:\n    return x\n\n\ndef g(n: int | None) -> object:\n    \
         return f(n)\n",
    );
    assert!(
        rendered.contains("error[T0021]: argument 1 of `f` expects `object`, got `int | None`"),
        "{rendered}"
    );
}

/// A private helper's return goes through the solver too, which defers a
/// concrete value to the check phase; a refused one is still a `T0022`.
#[test]
fn a_private_helper_returning_a_list_into_object_is_refused() {
    let rendered = refused(
        "ext_1475_private_list",
        "def _l() -> object:\n    return [1]\n\n\ndef pub() -> object:\n    return _l()\n",
    );
    assert!(
        rendered.contains("error[T0022]: expected return type `object`, got `list[int]`"),
        "{rendered}"
    );
}

/// A rebinding of an `object` name to a container stays a mismatch.
#[test]
fn rebinding_an_object_name_to_a_list_is_refused() {
    let rendered = refused(
        "ext_1475_rebind_list",
        "def g() -> object:\n    y: object = 1\n    y = [1]\n    return y\n",
    );
    assert!(rendered.contains("error[T0023]"), "{rendered}");
}

/// A default is validated in HIR before boxing exists, so an `object`
/// parameter's native default is still refused (deferred).
#[test]
fn a_native_default_for_an_object_parameter_is_still_refused() {
    let rendered = refused(
        "ext_1475_default",
        "def f(x: object = 3) -> object:\n    return x\n",
    );
    assert!(
        rendered.contains(
            "error[T0021]: default value of parameter `x` of `f` expects `object`, got `int`"
        ),
        "{rendered}"
    );
}

/// A class method's own `cls` is typed as an instance but holds none, so
/// boxing it is refused rather than crossing as a NULL.
#[test]
fn boxing_a_class_methods_cls_is_refused() {
    let rendered = refused(
        "ext_1475_cls",
        "def f(x: object) -> object:\n    return x\n\n\nclass K:\n    @classmethod\n    \
         def m(cls) -> object:\n        return f(cls)\n",
    );
    assert!(
        rendered.contains(
            "error[I0404]: boxing a class method's `cls` into an `object` slot is not supported yet"
        ),
        "{rendered}"
    );
}

/// Only a plain `name = value` rebinding boxes; a `for` loop over a `range` stores its value
/// unboxed, so rebinding an `object` name there stays a mismatch.
#[test]
fn a_native_value_through_a_for_range_binder_is_refused() {
    let rendered = refused(
        "ext_1475_binder_for_range",
        "def g() -> None:\n    y: object = 1\n    for y in range(3):\n        pass\n",
    );
    assert!(rendered.contains("error[T0023]"), "{rendered}");
}

/// Only a plain `name = value` rebinding boxes; a `for` loop over a list stores its value
/// unboxed, so rebinding an `object` name there stays a mismatch.
#[test]
fn a_native_value_through_a_for_list_binder_is_refused() {
    let rendered = refused(
        "ext_1475_binder_for_list",
        "def g(xs: list[int]) -> None:\n    y: object = 1\n    for y in xs:\n        pass\n",
    );
    assert!(rendered.contains("error[T0023]"), "{rendered}");
}

/// Only a plain `name = value` rebinding boxes; a walrus stores its value
/// unboxed, so rebinding an `object` name there stays a mismatch.
#[test]
fn a_native_value_through_a_walrus_binder_is_refused() {
    let rendered = refused(
        "ext_1475_binder_walrus",
        "def g() -> object:\n    y: object = 1\n    if (y := 5) > 0:\n        pass\n    return y\n",
    );
    assert!(rendered.contains("error[T0023]"), "{rendered}");
}

/// Only a plain `name = value` rebinding boxes; a `match` capture pattern stores its value
/// unboxed, so rebinding an `object` name there stays a mismatch.
#[test]
fn a_native_value_through_a_match_capture_binder_is_refused() {
    let rendered = refused(
        "ext_1475_binder_match_capture",
        "def g() -> object:\n    y: object = 1\n    match 3:\n        case y:\n            pass\n    return y\n",
    );
    assert!(rendered.contains("error[T0023]"), "{rendered}");
}

/// Only a plain `name = value` rebinding boxes; an annotation on a name already bound stores its value
/// unboxed, so rebinding an `object` name there stays a mismatch.
#[test]
fn a_native_value_through_a_reannotation_binder_is_refused() {
    let rendered = refused(
        "ext_1475_binder_reannotation",
        "def g() -> object:\n    y: object = 1\n    y: int = 5\n    return y\n",
    );
    assert!(rendered.contains("error[T0023]"), "{rendered}");
}

/// `cls` is refused as a constructor's boxed value too, not only as a plain
/// function argument.
#[test]
fn boxing_cls_into_ctor_is_refused() {
    let rendered = refused(
        "ext_1475_cls_ctor",
        "class H:\n    def __init__(self, x: object) -> None:\n        self.x = x\n\n\nclass K:\n    @classmethod\n    def m(cls) -> H:\n        return H(cls)\n",
    );
    assert!(
        rendered.contains(
            "error[I0404]: boxing a class method's `cls` into an `object` slot is not supported yet"
        ),
        "{rendered}"
    );
}

/// `cls` is refused as a method's boxed value too, not only as a plain
/// function argument.
#[test]
fn boxing_cls_into_method_is_refused() {
    let rendered = refused(
        "ext_1475_cls_method",
        "class H:\n    def put(self, x: object) -> None:\n        pass\n\n\nclass K:\n    @classmethod\n    def m(cls, h: H) -> None:\n        h.put(cls)\n",
    );
    assert!(
        rendered.contains(
            "error[I0404]: boxing a class method's `cls` into an `object` slot is not supported yet"
        ),
        "{rendered}"
    );
}

/// `cls` is refused as a static method's boxed value too, not only as a plain
/// function argument.
#[test]
fn boxing_cls_into_static_is_refused() {
    let rendered = refused(
        "ext_1475_cls_static",
        "class H:\n    @staticmethod\n    def st(x: object) -> None:\n        pass\n\n\nclass K:\n    @classmethod\n    def m(cls) -> None:\n        H.st(cls)\n",
    );
    assert!(
        rendered.contains(
            "error[I0404]: boxing a class method's `cls` into an `object` slot is not supported yet"
        ),
        "{rendered}"
    );
}

/// `cls` is refused as another class method's boxed value too, not only as a plain
/// function argument.
#[test]
fn boxing_cls_into_classmethod_is_refused() {
    let rendered = refused(
        "ext_1475_cls_classmethod",
        "class K:\n    @classmethod\n    def take(cls, x: object) -> None:\n        pass\n\n    @classmethod\n    def m(cls) -> None:\n        K.take(cls)\n",
    );
    assert!(
        rendered.contains(
            "error[I0404]: boxing a class method's `cls` into an `object` slot is not supported yet"
        ),
        "{rendered}"
    );
}

/// `cls` is refused as a rebinding's boxed value too, not only as a plain
/// function argument.
#[test]
fn boxing_cls_into_rebinding_is_refused() {
    let rendered = refused(
        "ext_1475_cls_rebinding",
        "class K:\n    @classmethod\n    def m(cls) -> object:\n        y: object = 1\n        y = cls\n        return y\n",
    );
    assert!(
        rendered.contains(
            "error[I0404]: boxing a class method's `cls` into an `object` slot is not supported yet"
        ),
        "{rendered}"
    );
}

/// A native program still cannot spell `object` (D-258): the boxing is
/// reachable only where an annotation is the opaque top type.
#[test]
fn a_native_program_still_refuses_an_object_annotation() {
    let dir = ScratchDir::new("native_1475_object").expect("scratch");
    let output = pycc()
        .arg("build")
        .arg(write(
            &dir,
            "m.py",
            "def f(x: object) -> int:\n    return 1\n\n\nprint(f(3))\n",
        ))
        .arg("-o")
        .arg(dir.join("app"))
        .env("PYCC_PYTHON", "/nonexistent/pycc-no-python")
        .output()
        .expect("pycc should spawn");
    let rendered = stderr_of(&output);
    assert_eq!(output.status.code(), Some(1), "{rendered}");
    assert!(
        rendered.contains("error[C0001]: type annotation `object` is not supported yet"),
        "{rendered}"
    );
}
