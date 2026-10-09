//! #1174: a buffer return from a **public method of a published class**.
//!
//! Part 2b of #1142 ([#1164](https://github.com/rotnov/pycc/issues/1164))
//! admitted a `memoryview` return from a public *module-level* `def` only.
//! The narrowing was not about the carrier -- `src/ext_build/carrier.rs`'s
//! `return_c_type` is keyed on the type alone and the generated wrapper is
//! already shared by methods -- but about the *intra-artifact call*: a
//! method's return type is resolved through `pycc_types`' three
//! `class::resolve_*` routes, which the `HirExpr::Call`-keyed
//! buffer-returning-call refusal never reaches, so `g.make()` inside the
//! artifact would have walked into `crates/pycc_codegen/src/call_result.rs`'s
//! `Ty::MemoryView` panic -- a compiler crash on valid Python.
//!
//! This file owns both halves of that change: the admission (a method of
//! each of the three kinds hands the host a real `memoryview`) and the
//! interception that makes it safe (every intra-artifact route to a
//! buffer-returning method is a `C0001`, never a panic). The shapes that
//! stay refused keep their assertions here too, each naming *which* filter
//! refuses it, so a later change to that filter fails a test rather than
//! reopening the panic.

use pycc_scratch::ScratchDir;
use std::path::Path;
use std::process::{Command, Output};

fn pycc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
}

fn stdout_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n")
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).replace("\r\n", "\n")
}

fn fixture(category: &str, source: &str) -> ScratchDir {
    let dir = ScratchDir::new(category).expect("scratch");
    std::fs::write(dir.join("method_probe.py"), source).expect("write the subject");
    dir
}

fn build_ext(dir: &Path) -> Output {
    pycc()
        .arg("build")
        .arg(dir.join("method_probe.py"))
        .arg("-o")
        .arg(dir.join("method_probe"))
        .arg("--ext")
        .output()
        .expect("pycc should spawn")
}

/// `pycc check` reports on stdout and knows nothing about artifact mode, so
/// it sees exactly the `pycc_types` half of this change.
fn check(dir: &Path) -> Output {
    pycc()
        .arg("check")
        .arg(dir.join("method_probe.py"))
        .output()
        .expect("pycc should spawn")
}

/// The admitted subject: all three method kinds, inheritance in both
/// directions, and the buffer-in/buffer-out shape #1039's reference workload
/// actually has.
const SUBJECT: &str = "\
class Base:
    def rows(self) -> memoryview:
        a = ndarray(3)
        a[0] = 1.0
        return a

    def shared(self) -> memoryview:
        a = ndarray(2)
        a[0] = 9.0
        return a


class Grid(Base):
    def __init__(self, scale: float) -> None:
        self.scale = scale

    @staticmethod
    def make(n: int) -> memoryview:
        a = ndarray(n)
        a[0] = 2.0
        return a

    @classmethod
    def build(cls, n: int) -> memoryview:
        a = ndarray(n)
        a[0] = 3.0
        return a

    def shared(self) -> memoryview:
        a = ndarray(2)
        a[0] = 4.0
        return a

    def apply(self, xs: memoryview) -> memoryview:
        out = ndarray(len(xs))
        i = 0
        while i < len(xs):
            out[i] = xs[i] * self.scale
            i = i + 1
        return out
";

/// The admission arm. Nothing in the frontend refuses the program: not the
/// `C0003` carrier gap, not `src/memoryview_mode.rs`'s `ext` return gap, and
/// not the new intra-artifact call interception, which this program never
/// reaches because no method here calls another.
///
/// Asserted as the absence of any diagnostic rather than as a successful
/// build, exactly as `tests/issue_1164_memoryview_egress.rs`'s own admission
/// arm is: the build reaches `clang` and fails there on a host with no
/// CPython development headers, and that failure is not what this arm owns.
#[test]
fn a_buffer_returning_method_of_every_kind_is_admitted_by_an_ext_build() {
    let dir = fixture("1174_admit", SUBJECT);
    let build = build_ext(&dir);
    let err = stderr_of(&build);
    assert!(!err.contains("error[C0003]"), "{err}");
    assert!(!err.contains("error[C0001]"), "{err}");
    assert!(!err.contains("error[I0405]"), "{err}");
    assert!(!err.contains("error[T0"), "{err}");
    assert!(!err.contains("panicked"), "{err}");
}

/// ...and a native build still refuses it, unchanged. #1174 widens the
/// `pycc build --ext` admission and nothing else: natively there is no host
/// to hand the buffer to, so `refuse_in_native_mode`'s `I0405` answers a
/// method's signature exactly as it answers a module-level `def`'s.
#[test]
fn a_native_build_still_refuses_a_buffer_returning_method() {
    let dir = fixture("1174_native", SUBJECT);
    let build = pycc()
        .arg("build")
        .arg(dir.join("method_probe.py"))
        .arg("-o")
        .arg(dir.join("method_probe"))
        .output()
        .expect("pycc should spawn");
    assert!(!build.status.success(), "{}", stdout_of(&build));
    let err = stderr_of(&build);
    assert!(err.contains("error[I0405]"), "{err}");
    assert!(!err.contains("panicked"), "{err}");
}

/// The hosted arm: each of the three method kinds, an inherited method, an
/// overriding method and the buffer-in/buffer-out pair hand the host a real
/// `memoryview`, and `pycc_rt_buffer_live_views` returns to zero once the
/// host drops it.
///
/// The counter reading is what no front-end assertion can make: a wrapper
/// that handed back a view over the compiled frame's own slot would satisfy
/// every value assertion above it and fail the balance.
///
/// Not compiled on Windows, matching `tests/issue_1164_memoryview_egress.rs`'s
/// own counter-reading arms and for their reason: the counter is linked into
/// the `.pyd` from a static archive but is absent from its export table.
#[test]
#[cfg(not(target_os = "windows"))]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_returned_buffer_from_each_method_kind_reaches_the_host_and_transfers_ownership() {
    let dir = fixture("1174_hosted", SUBJECT);
    let build = build_ext(&dir);
    assert!(build.status.success(), "{}", stderr_of(&build));

    let run = Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
        .arg("-c")
        .arg(
            "import ctypes, method_probe as m\n\
             live = ctypes.CDLL(m.__file__).pycc_rt_buffer_live_views\n\
             live.restype = ctypes.c_longlong\n\
             live.argtypes = []\n\
             assert live() == 0, live()\n\
             g = m.Grid(3.0)\n\
             v = m.Grid.make(4)\n\
             assert isinstance(v, memoryview), type(v)\n\
             assert v.format == 'd' and v.itemsize == 8 and v.ndim == 1\n\
             assert not v.readonly and v.shape == (4,)\n\
             assert list(v) == [2.0, 0.0, 0.0, 0.0], list(v)\n\
             assert live() == 1, live()\n\
             del v\n\
             assert live() == 0, live()\n\
             assert list(m.Grid.build(2)) == [3.0, 0.0]\n\
             assert list(g.rows()) == [1.0, 0.0, 0.0]\n\
             assert list(g.shared()) == [4.0, 0.0]\n\
             assert list(m.Base().shared()) == [9.0, 0.0]\n\
             store = bytearray(24)\n\
             host = memoryview(store).cast('d')\n\
             host[0], host[1], host[2] = 1.0, 2.0, 3.0\n\
             out = g.apply(host)\n\
             assert list(out) == [3.0, 6.0, 9.0], list(out)\n\
             host.release()\n\
             del out\n\
             assert live() == 0, live()\n\
             for _ in range(64):\n\
             \x20   w = m.Grid.make(8)\n\
             \x20   assert live() == 1, live()\n\
             \x20   del w\n\
             \x20   assert live() == 0, live()\n\
             print('ok')\n",
        )
        .current_dir(&*dir)
        .output()
        .expect("python3 should spawn");
    assert!(run.status.success(), "{}", stderr_of(&run));
    assert_eq!(stdout_of(&run), "ok\n");
}

/// Every intra-artifact route to a buffer-returning method, one arm per
/// resolver exit the interception sits on.
///
/// `!err.contains("panicked")` is the load-bearing assertion in every arm:
/// before #1174 each of these programs was refused at the *declaration* by
/// `src/memoryview_mode.rs`, and admitting the declaration is exactly what
/// makes `crates/pycc_codegen/src/call_result.rs`'s `Ty::MemoryView` panic
/// reachable if a route is missed. A sixth route this inventory does not
/// know about would show up here as a panic rather than as a diagnostic.
///
/// The routes, and which exit each reaches:
///
/// * `g.make()` and `self.make()` -- `class::method_call::resolve_method_call`'s
///   MRO arm.
/// * `Buf.make()` -- `resolve_static_or_class_method_call`'s `static_methods`
///   exit.
/// * `Buf.build()` -- the same function's *separate* `class_methods` exit,
///   which is why a `@classmethod` arm is required and not a duplicate of
///   the `@staticmethod` one.
/// * `super().make()` -- `resolve_super_method_call`.
/// * `Buf[0]` -- PEP 560's `__class_getitem__`, which `expr.rs` routes
///   through `resolve_static_or_class_method_call` too. Written as a
///   `@classmethod` taking `cls`: a plain `def __class_getitem__(i: int)`
///   is rejected earlier -- since #1181 by `class::receiver`'s
///   implicitly-rebound-dunder guard, which keeps requiring `self` for
///   `__new__`/`__init_subclass__`/`__class_getitem__` -- and would test
///   nothing.
const CALL_ROUTES: [(&str, &str, &str); 6] = [
    (
        "1174_call_instance",
        "class Buf:
    def make(self, n: int) -> memoryview:
        a = ndarray(n)
        return a


def total() -> int:
    g = Buf()
    v = g.make(4)
    return 0
",
        "calling `Buf.make`, whose return type is a buffer",
    ),
    (
        "1174_call_self",
        "class Buf:
    def make(self, n: int) -> memoryview:
        a = ndarray(n)
        return a

    def use(self) -> int:
        v = self.make(4)
        return 0
",
        "calling `Buf.make`, whose return type is a buffer",
    ),
    (
        "1174_call_static",
        "class Buf:
    @staticmethod
    def make(n: int) -> memoryview:
        a = ndarray(n)
        return a


def total() -> int:
    v = Buf.make(4)
    return 0
",
        "calling `Buf.make`, whose return type is a buffer",
    ),
    (
        "1174_call_classmethod",
        "class Buf:
    @classmethod
    def build(cls, n: int) -> memoryview:
        a = ndarray(n)
        return a


def total() -> int:
    v = Buf.build(4)
    return 0
",
        "calling `Buf.build`, whose return type is a buffer",
    ),
    (
        "1174_call_super",
        "class Base:
    def make(self, n: int) -> memoryview:
        a = ndarray(n)
        return a


class Sub(Base):
    def use(self) -> int:
        v = super().make(4)
        return 0
",
        "calling `Sub.make`, whose return type is a buffer",
    ),
    (
        "1174_call_class_getitem",
        "class Buf:
    @classmethod
    def __class_getitem__(cls, i: int) -> memoryview:
        a = ndarray(i)
        return a


def total() -> int:
    v = Buf[0]
    return 0
",
        "calling `Buf.__class_getitem__`, whose return type is a buffer",
    ),
];

/// The `pycc build --ext` half: every route is a `C0001` rather than a
/// compiler panic.
#[test]
fn every_intra_artifact_route_to_a_buffer_returning_method_is_refused_under_ext() {
    for (category, source, expected) in CALL_ROUTES {
        let dir = fixture(category, source);
        let build = build_ext(&dir);
        assert!(!build.status.success(), "{category}: {}", stdout_of(&build));
        let err = stderr_of(&build);
        assert!(!err.contains("panicked"), "{category}: {err}");
        assert!(err.contains("error[C0001]"), "{category}: {err}");
        assert!(err.contains(expected), "{category}: {err}");
        // The mangled spelling never reaches the reader: the resolvers hold
        // the source-level method name and the class name, not the export
        // table's `Buf.make.static`.
        assert!(!err.contains(".static"), "{category}: {err}");
        assert!(!err.contains(".classmethod"), "{category}: {err}");
    }
}

/// The `pycc check` half, and a surface this change genuinely moves: the
/// refusal lives in `crates/pycc_types` rather than in an artifact-mode
/// gate, so `check` -- which knows nothing about `--ext` -- reports it too,
/// where before #1174 it was silent on an intra-artifact `g.make()`.
#[test]
fn every_intra_artifact_route_is_reported_by_pycc_check_too() {
    for (category, source, expected) in CALL_ROUTES {
        let dir = fixture(category, source);
        let checked = check(&dir);
        assert!(!checked.status.success(), "{category}");
        let out = stdout_of(&checked);
        assert!(!out.contains("panicked"), "{category}: {out}");
        assert!(out.contains("error[C0001]"), "{category}: {out}");
        assert!(out.contains(expected), "{category}: {out}");
    }
}

/// The shapes that stay refused at the *declaration*, one arm per filter,
/// so a later change to that filter fails a test rather than opening the
/// `call_result.rs` panic.
///
/// Every arm's third field names the filter that refuses it:
///
/// * `_make` on a public class, and `view` / `make` / `build` on a private
///   class -- the `collect_exports` privacy verdict
///   (`classify_export_name`). The `@staticmethod` and `@classmethod` arms
///   are not redundant with the instance-method one: each is the only
///   subject in this file that gives the `.static` and `.classmethod`
///   mangled-suffix assertions below something real to refute, since
///   `ext_return_gap` renders a demangled name for all three kinds.
/// * a private module-level `def` -- the same verdict, module-level arm,
///   unchanged since #1164.
/// * a method of an exception class -- `collect_exports`' exception-class
///   filter.
/// * a method of a non-constructible class -- `instance_method_reachable`.
/// * a `@property` getter -- `collect_exports`' property filter, which
///   excludes getters from the export set; admitting them later needs its
///   own interception in the attribute route, which this change does not
///   add.
const DECLARATION_REFUSALS: [(&str, &str, &str); 8] = [
    (
        "1174_private_method",
        "class Buf:
    def _make(self) -> memoryview:
        a = ndarray(4)
        return a


def total() -> int:
    return 0
",
        "`Buf._make`'s return type is a buffer",
    ),
    (
        "1174_private_class_method",
        "class _Buf:
    def view(self) -> memoryview:
        a = ndarray(4)
        return a


def total() -> int:
    return 0
",
        "`_Buf.view`'s return type is a buffer",
    ),
    (
        "1174_private_class_staticmethod",
        "class _Buf:
    @staticmethod
    def make() -> memoryview:
        a = ndarray(4)
        return a


def total() -> int:
    return 0
",
        "`_Buf.make`'s return type is a buffer",
    ),
    (
        "1174_private_class_classmethod",
        "class _Buf:
    @classmethod
    def build(cls) -> memoryview:
        a = ndarray(4)
        return a


def total() -> int:
    return 0
",
        "`_Buf.build`'s return type is a buffer",
    ),
    (
        "1174_private_module_level",
        "def _make() -> memoryview:
    a = ndarray(4)
    return a


def total() -> int:
    return 0
",
        "`_make`'s return type is a buffer",
    ),
    (
        "1174_exception_class_method",
        "class Boom(Exception):
    def view(self) -> memoryview:
        a = ndarray(4)
        return a


def total() -> int:
    return 0
",
        "`Boom.view`'s return type is a buffer",
    ),
    (
        "1174_abstract_class_method",
        "from abc import ABC, abstractmethod


class Buf(ABC):
    @abstractmethod
    def size(self) -> int: ...

    def view(self) -> memoryview:
        a = ndarray(4)
        return a


def total() -> int:
    return 0
",
        "`Buf.view`'s return type is a buffer",
    ),
    (
        "1174_property_getter",
        "class Buf:
    @property
    def view(self) -> memoryview:
        a = ndarray(4)
        return a


def total() -> int:
    return 0
",
        "`Buf.view`'s return type is a buffer",
    ),
];

#[test]
fn a_buffer_return_outside_the_export_set_is_still_refused_in_ext_mode() {
    for (category, source, expected) in DECLARATION_REFUSALS {
        let dir = fixture(category, source);
        let build = build_ext(&dir);
        assert!(!build.status.success(), "{category}: {}", stdout_of(&build));
        let err = stderr_of(&build);
        assert!(!err.contains("panicked"), "{category}: {err}");
        assert!(err.contains("error[C0001]"), "{category}: {err}");
        assert!(err.contains(expected), "{category}: {err}");
        assert!(!err.contains("error[C0003]"), "{category}: {err}");
        // Carried forward from `tests/issue_1112_ext_memoryview.rs`'s own
        // CASES loop, whose method-bearing arms became admissions here:
        // `ext_return_gap` renders the *source-level* name, never the
        // mangled `_Buf.make.static` the export table carries.
        assert!(!err.contains(".static"), "{category}: {err}");
        assert!(!err.contains(".classmethod"), "{category}: {err}");
    }
}

/// A buffer producer inside a *generic* method body is refused earlier
/// still, by the producer's own position rule rather than by anything this
/// change touches: a specialization is not an export, so it has no admitted
/// producer position at all.
///
/// The class is *instantiated*, which is load-bearing and was measured:
/// without `b = Buf(1)` no specialization is emitted, the method body is
/// never lowered, and `pycc build --ext` exits 0 on a program that looks
/// refused. The arm would then assert nothing.
#[test]
fn a_buffer_producer_in_a_generic_body_is_refused_by_the_producer_rule() {
    let dir = fixture(
        "1174_generic_producer",
        "class Buf[T]:
    def __init__(self, x: T) -> None:
        self.x = x

    def view(self) -> memoryview:
        a = ndarray(4)
        return a


def total() -> int:
    b = Buf(1)
    return 0
",
    );
    let build = build_ext(&dir);
    assert!(!build.status.success(), "{}", stdout_of(&build));
    let err = stderr_of(&build);
    assert!(!err.contains("panicked"), "{err}");
    assert!(err.contains("error[C0001]"), "{err}");
    assert!(
        err.contains("a buffer producer is admitted only as the whole right-hand side"),
        "{err}"
    );
}

/// #1173's `return`-inside-`finally` narrowing carries over to a method
/// unchanged: `env.returns_inside_finally` is computed per function in
/// `check_function_in`, which asks nothing about where the function lives.
///
/// `while True: return a` rather than a bare `return a` in the `finally`
/// body, because the bare form is refused syntactically by the parser's own
/// `L0001` (PEP 765) and would not reach the egress admission at all.
#[test]
fn a_method_with_a_return_inside_a_finally_is_still_refused() {
    let dir = fixture(
        "1174_finally_method",
        "class Buf:
    def view(self) -> memoryview:
        a = ndarray(4)
        try:
            return a
        finally:
            while True:
                return a
",
    );
    let build = build_ext(&dir);
    assert!(!build.status.success(), "{}", stdout_of(&build));
    let err = stderr_of(&build);
    assert!(!err.contains("panicked"), "{err}");
    assert!(err.contains("error[C0001]"), "{err}");
    assert!(err.contains("`return` inside a `finally`"), "{err}");
}
