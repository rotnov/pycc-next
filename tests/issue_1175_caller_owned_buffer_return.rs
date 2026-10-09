//! Part 1 of #1175 (#1178): returning the name a `memoryview` **parameter**
//! binds, from a public `pycc build --ext` export.
//!
//! Part 2b of #1142 ([#1164](https://github.com/rotnov/pycc/issues/1164))
//! admitted exactly one provenance at a buffer return -- storage the
//! artifact itself allocated with `a = ndarray(n)`, whose exporter's
//! `tp_dealloc` frees it -- and #1174 extended that to a public method. A
//! *parameter*-bound name was refused, and the stated reason was that the
//! wrapper releases the host's `Py_buffer` on the way out, so handing the
//! view back would be a use-after-free.
//!
//! That reason is false, and this file is where the correction is proven
//! rather than argued. The wrapper acquires a **second, independent buffer
//! export** on the host's own argument object with
//! `PyMemoryView_FromObject` *before* it releases its own `Py_buffer`. An
//! export -- not a reference -- is what pins an exporter's storage, so the
//! returned view spans storage the caller's object still owns and pycc
//! frees nothing: `pycc_rt_buffer_live_views` is zero for the whole call.
//!
//! Two host-side assertions carry the substance and no front-end test can
//! make either:
//!
//! * the host's own `array.array('d').extend(...)` raises `BufferError`
//!   while the returned view is alive and succeeds once it is released --
//!   the storage really is pinned, and a later "optimization" that replaced
//!   the packer with a `Py_INCREF` would fail here and nowhere else;
//! * a PEP 688 Python-level exporter logs `__buffer__` / `__release_buffer__`
//!   in the order the wrapper actually calls them. That is the *only*
//!   subject that discriminates acquire-before-release from
//!   release-before-acquire: an `array.array('d')` is a pure C exporter and
//!   its acquire/release are unobservable from Python.
//!
//! #1178's scope boundary is the **bare** parameter name. `return b[1:3]`
//! is Part 2 (#1179), which admits it and owns its evidence in
//! `tests/issue_1179_buffer_slice_return.rs`. `return g(b)` for a
//! buffer-returning `g` stays refused by #1175's own scope boundary and is
//! pinned below, so a later widening of *that* has to delete an assertion
//! deliberately.

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
    std::fs::write(dir.join("caller_probe.py"), source).expect("write the subject");
    dir
}

fn build_ext(dir: &Path) -> Output {
    pycc()
        .arg("build")
        .arg(dir.join("caller_probe.py"))
        .arg("-o")
        .arg(dir.join("caller_probe"))
        .arg("--ext")
        .output()
        .expect("pycc should spawn")
}

/// `pycc check` reports on stdout and knows nothing about artifact mode, so
/// it sees exactly the `pycc_types` half of this change.
fn check(dir: &Path) -> Output {
    pycc()
        .arg("check")
        .arg(dir.join("caller_probe.py"))
        .output()
        .expect("pycc should spawn")
}

/// The admitted subject. Every shape §5 of the plan's inventory names is
/// here: a bare parameter return, a second parameter, a branch returning
/// either, a body that writes through the parameter before returning it, a
/// function mixing both provenances, a public method, and two companion
/// readers used by the window-agreement assertion.
const SUBJECT: &str = "\
def first(b: memoryview) -> memoryview:
    return b


def second(b: memoryview, c: memoryview) -> memoryview:
    return c


def pick(b: memoryview, c: memoryview, which: int) -> memoryview:
    if which == 0:
        return b
    return c


def touched(b: memoryview) -> memoryview:
    b[0] = 7.0
    return b


def mixed(b: memoryview, which: int) -> memoryview:
    a = ndarray(2)
    a[0] = 5.0
    if which == 0:
        return b
    return a


def deferred(b: memoryview) -> memoryview:
    try:
        return b
    finally:
        a = ndarray(2)
        a[0] = 3.0


def peek(b: memoryview) -> float:
    return b[0]


def span(b: memoryview) -> int:
    return len(b)


class Grid:
    def __init__(self, scale: float) -> None:
        self.scale = scale

    def pass_through(self, b: memoryview) -> memoryview:
        return b
";

/// The admission arm. Nothing in the frontend refuses the subject -- not
/// `reject_memoryview_read`, which the return interception now precedes for
/// this provenance, and not `src/memoryview_mode.rs`'s `ext` return gap,
/// whose predicate is export-set membership rather than provenance.
///
/// Asserted as the absence of any diagnostic rather than as a successful
/// build, exactly as `tests/issue_1174_method_buffer_return.rs`'s own
/// admission arm is: the build reaches `clang` and fails there on a host
/// with no CPython development headers, and that failure is not what this
/// arm owns.
#[test]
fn returning_a_buffer_parameter_is_admitted_by_an_ext_build() {
    let dir = fixture("1175_admit", SUBJECT);
    let build = build_ext(&dir);
    let err = stderr_of(&build);
    assert!(!err.contains("error[C0001]"), "{err}");
    assert!(!err.contains("error[C0003]"), "{err}");
    assert!(!err.contains("error[I0405]"), "{err}");
    assert!(!err.contains("error[T0"), "{err}");
    assert!(!err.contains("panicked"), "{err}");
}

/// ...and `pycc check` agrees, which is the surface that actually moved:
/// the admission lives in `crates/pycc_types`, so it is visible without
/// artifact mode.
#[test]
fn returning_a_buffer_parameter_is_admitted_by_pycc_check_too() {
    let dir = fixture("1175_admit_check", SUBJECT);
    let checked = check(&dir);
    assert!(checked.status.success(), "{}", stdout_of(&checked));
}

/// A native build still refuses the same signature with `I0405`. The new
/// admission is `--ext`-only: natively there is no host object to borrow an
/// export from, and `refuse_in_native_mode` answers the signature before any
/// provenance question is asked.
#[test]
fn a_native_build_still_refuses_a_buffer_returning_signature() {
    let dir = fixture("1175_native", SUBJECT);
    let build = pycc()
        .arg("build")
        .arg(dir.join("caller_probe.py"))
        .arg("-o")
        .arg(dir.join("caller_probe"))
        .output()
        .expect("pycc should spawn");
    assert!(!build.status.success(), "{}", stdout_of(&build));
    let err = stderr_of(&build);
    assert!(err.contains("error[I0405]"), "{err}");
    assert!(!err.contains("panicked"), "{err}");
}

/// Every intra-artifact route to a buffer-returning callee, re-run for the
/// provenance this change creates.
///
/// `crates/pycc_codegen/src/call_result.rs`'s `Ty::MemoryView` panic fires
/// on a buffer-typed *call result*, and under D-141 a panic unwinding past a
/// plain `extern "C" fn` aborts the hosting CPython interpreter. #1174
/// closed every route for the artifact-owned provenance. Part 1 of #1175
/// makes `-> memoryview` callees exist in shapes that previously could not
/// type-check at all -- a callee whose body is a bare `return b` -- so each
/// route is re-asserted here against a *parameter*-returning callee rather
/// than assumed to carry over.
///
/// Each route keys on the callee's declared return type, never on the
/// provenance of the value it returns, which is why they all still fire.
/// `!err.contains("panicked")` is the load-bearing assertion in every arm.
///
/// The routes, and which exit each reaches:
///
/// * `first(b)` at module level -- `expr.rs`'s buffer-returning-call arm.
/// * the same inside an unannotated helper -- `constraints.rs`'s mirror,
///   which the solver reaches first.
/// * `g.pass(b)` -- `class::method_call::resolve_method_call`'s MRO arm.
/// * `Buf.make(b)` -- `resolve_static_or_class_method_call`'s
///   `static_methods` exit.
/// * `Buf.build(b)` -- the same function's separate `class_methods` exit.
/// * `super().pass(b)` -- `resolve_super_method_call`.
/// * `Buf[b]` -- PEP 560's `__class_getitem__`, routed through
///   `resolve_static_or_class_method_call` too.
/// * a generic specialization -- refused by the producer/position rules
///   before any call route, since a specialization is not an export.
const CALLER_OWNED_CALL_ROUTES: [(&str, &str); 8] = [
    (
        "1175_route_module",
        "def inner(b: memoryview) -> memoryview:
    return b


def outer(b: memoryview) -> memoryview:
    return inner(b)
",
    ),
    (
        "1175_route_solver",
        "def inner(b: memoryview) -> memoryview:
    return b


def _helper(n):
    return n


def outer(b: memoryview) -> memoryview:
    v = inner(b)
    return v


def total() -> int:
    return _helper(1)
",
    ),
    (
        "1175_route_instance",
        "class Buf:
    def pass_through(self, b: memoryview) -> memoryview:
        return b


def outer(b: memoryview) -> int:
    g = Buf()
    v = g.pass_through(b)
    return 0
",
    ),
    (
        "1175_route_static",
        "class Buf:
    @staticmethod
    def make(b: memoryview) -> memoryview:
        return b


def outer(b: memoryview) -> int:
    v = Buf.make(b)
    return 0
",
    ),
    (
        "1175_route_classmethod",
        "class Buf:
    @classmethod
    def build(cls, b: memoryview) -> memoryview:
        return b


def outer(b: memoryview) -> int:
    v = Buf.build(b)
    return 0
",
    ),
    (
        "1175_route_super",
        "class Base:
    def pass_through(self, b: memoryview) -> memoryview:
        return b


class Sub(Base):
    def use(self, b: memoryview) -> int:
        v = super().pass_through(b)
        return 0
",
    ),
    (
        "1175_route_class_getitem",
        "class Buf:
    @classmethod
    def __class_getitem__(cls, b: memoryview) -> memoryview:
        return b


def outer(b: memoryview) -> int:
    v = Buf[b]
    return 0
",
    ),
    (
        "1175_route_generic",
        "class Buf[T]:
    def __init__(self, x: T) -> None:
        self.x = x

    def view(self, b: memoryview) -> memoryview:
        return b


def total(b: memoryview) -> int:
    g = Buf(1)
    v = g.view(b)
    return 0
",
    ),
];

/// The `pycc build --ext` half: every route is a `C0001` rather than a
/// compiler panic, for the caller-owned provenance too.
#[test]
fn every_intra_artifact_route_to_a_caller_owned_return_is_refused_under_ext() {
    for (category, source) in CALLER_OWNED_CALL_ROUTES {
        let dir = fixture(category, source);
        let build = build_ext(&dir);
        assert!(!build.status.success(), "{category}: {}", stdout_of(&build));
        let err = stderr_of(&build);
        assert!(!err.contains("panicked"), "{category}: {err}");
        assert!(err.contains("error[C0001]"), "{category}: {err}");
    }
}

/// ...and `pycc check` reports each one too, so the interception is a
/// frontend property rather than an artifact-mode gate.
#[test]
fn every_intra_artifact_route_to_a_caller_owned_return_is_reported_by_check() {
    for (category, source) in CALLER_OWNED_CALL_ROUTES {
        let dir = fixture(category, source);
        let checked = check(&dir);
        assert!(!checked.status.success(), "{category}");
        let out = stdout_of(&checked);
        assert!(!out.contains("panicked"), "{category}: {out}");
        assert!(out.contains("error[C0001]"), "{category}: {out}");
    }
}

/// A protocol member's `-> memoryview` stays refused at the declaration.
/// Its ground is untouched by this change and survives it: a protocol method
/// is dispatched dynamically and has no generated wrapper at all, so there
/// is nothing to acquire a borrowed export in -- which holds for both
/// provenances. Its rendered message enumerates them, and this arm pins that
/// the enumeration reaches the reader.
#[test]
fn a_protocol_members_buffer_return_is_still_refused() {
    let dir = fixture(
        "1175_protocol",
        "from typing import Protocol


class Viewer(Protocol):
    def view(self, b: memoryview) -> memoryview: ...


def total() -> int:
    return 0
",
    );
    let build = build_ext(&dir);
    assert!(!build.status.success(), "{}", stdout_of(&build));
    let err = stderr_of(&build);
    assert!(!err.contains("panicked"), "{err}");
    assert!(err.contains("error[C0001]"), "{err}");
    assert!(err.contains("protocol"), "{err}");
}

/// The shapes #1175 leaves refused, one arm per reason. Each is refused for
/// a stated cause rather than by accident, so a later widening has to delete
/// an assertion here.
///
/// A **slice** of a parameter was one of these arms and is no longer: Part 2
/// of #1175 (#1179) admits it, carrying the sub-range out of the callee
/// frame in three out-pointers so the wrapper's pointer-identity test
/// against `args[i]` still names the owner exactly. Its evidence moved to
/// `tests/issue_1179_buffer_slice_return.rs` rather than being deleted.
///
/// * a buffer-returning **call result** -- #1175's own scope boundary. A
///   value produced inside a callee frame need not equal any of this
///   wrapper's `args[i]` slots, so the identity test cannot name an owner
///   for it.
/// * `return ndarray(n)` as a whole return expression -- the producer's own
///   position rule, in neither part of #1175.
/// * a `return` inside a `finally` -- #1164 review round 5's narrowing,
///   which Part 1 deliberately keeps for the caller-owned provenance too
///   (widening it is #1173's question).
const SCOPE_BOUNDARY_REFUSALS: [(&str, &str); 3] = [
    (
        "1175_scope_call_result",
        "def inner(b: memoryview) -> memoryview:
    return b


def outer(b: memoryview) -> memoryview:
    return inner(b)
",
    ),
    (
        "1175_scope_producer",
        "def first(n: int) -> memoryview:
    return ndarray(n)
",
    ),
    (
        "1175_scope_finally",
        "def first(b: memoryview) -> memoryview:
    try:
        return b
    finally:
        while True:
            return b
",
    ),
];

#[test]
fn the_shapes_outside_part_ones_boundary_stay_refused() {
    for (category, source) in SCOPE_BOUNDARY_REFUSALS {
        let dir = fixture(category, source);
        let build = build_ext(&dir);
        assert!(!build.status.success(), "{category}: {}", stdout_of(&build));
        let err = stderr_of(&build);
        assert!(!err.contains("panicked"), "{category}: {err}");
        assert!(err.contains("error[C0001]"), "{category}: {err}");
    }
}

/// Rebinding a buffer parameter stays refused, which is what keeps the flat
/// provenance model sound: without it a name could be parameter-bound in one
/// half of a function and artifact-owned in the other, and the wrapper's
/// pointer-identity test would then have to answer a question the type
/// checker could not.
#[test]
fn rebinding_a_buffer_parameter_is_still_refused() {
    let dir = fixture(
        "1175_rebind",
        "def first(b: memoryview) -> memoryview:
    b = ndarray(4)
    return b
",
    );
    let build = build_ext(&dir);
    assert!(!build.status.success(), "{}", stdout_of(&build));
    let err = stderr_of(&build);
    assert!(!err.contains("panicked"), "{err}");
    assert!(err.contains("error[C0001]"), "{err}");
    assert!(
        err.contains("a buffer parameter of a `pycc build --ext` export"),
        "{err}"
    );
}

/// The host-side driver. Written to a file rather than passed to `python3
/// -c` so it reads as ordinary Python; every assertion carries its own
/// message so a failure names itself.
///
/// Only the hosted arm below consumes it, and that arm is Unix-only, so
/// Windows would otherwise see an unused constant and fail `-D warnings`.
#[cfg(not(target_os = "windows"))]
const DRIVER: &str = r#"
import array
import ctypes
import caller_probe as m

live = ctypes.CDLL(m.__file__).pycc_rt_buffer_live_views
live.restype = ctypes.c_longlong
live.argtypes = []

# (c) of the D-244 amendment: the artifact allocates nothing on this path,
# so the counter is zero before the call, *while the returned view is live*,
# and after it is dropped. "back to zero" would not state the middle one,
# and the middle one is the assertion that a wrongly-taken artifact-owned
# packer would fail.
assert live() == 0, live()

a = array.array('d', [1.0, 2.0, 3.0])
v = m.first(a)
assert isinstance(v, memoryview), type(v)
assert v.format == 'd' and v.itemsize == 8 and v.ndim == 1, v.format
assert v.shape == (3,), v.shape
assert list(v) == [1.0, 2.0, 3.0], list(v)
assert live() == 0, live()

# The storage is pinned by a real export the returned view holds, not by a
# reference. `array.array` relocates its block under `extend`, and refuses
# to while it is exported -- so this raises while the view is alive and
# succeeds once it is released. A packer that merely incremented a
# refcount would let the extend through and silently relocate the storage
# the host is still reading.
try:
    a.extend([4.0])
except BufferError:
    pass
else:
    raise AssertionError('extend should have been refused while the view was live')
v.release()
a.extend([4.0])
assert len(a) == 4, len(a)
del a[3:]
assert live() == 0, live()

# Writeback through the returned view reaches the host object.
v = m.first(a)
v[1] = 20.0
v.release()
assert a[1] == 20.0, a[1]
a[1] = 2.0

# ...and a body that writes through the parameter before returning it hands
# back a view over the same, already-written storage.
v = m.touched(a)
assert list(v) == [7.0, 2.0, 3.0], list(v)
v.release()
assert a[0] == 7.0, a[0]
a[0] = 1.0
assert live() == 0, live()

# (d) of the amendment: the returned view's writability is the host
# exporter's, not the artifact-owned path's hard-coded `readonly = 0`.
ro = memoryview(bytes(24)).cast('d')
v = m.first(ro)
assert v.readonly is True, v.readonly
assert list(v) == [0.0, 0.0, 0.0], list(v)
v.release()
ro.release()

# The second parameter, a branch returning either, and the same object at
# two parameters -- the wrapper decides which slot a call returned by
# pointer identity, so each has to be exercised in turn.
b = array.array('d', [10.0, 11.0])
v = m.second(a, b)
assert list(v) == [10.0, 11.0], list(v)
v.release()
v = m.pick(a, b, 0)
assert list(v) == [1.0, 2.0, 3.0], list(v)
v.release()
v = m.pick(a, b, 1)
assert list(v) == [10.0, 11.0], list(v)
v.release()
v = m.second(a, a)
assert list(v) == [1.0, 2.0, 3.0], list(v)
v.release()
assert live() == 0, live()

# A function mixing both provenances. The parameter branch allocates
# nothing; the owned branch does, and its view is the one the host then
# owns and frees.
v = m.mixed(a, 0)
assert list(v) == [1.0, 2.0, 3.0], list(v)
assert live() == 0, live()
v.release()
w = m.mixed(a, 1)
assert list(w) == [5.0, 0.0], list(w)
assert live() == 1, live()
del w
assert live() == 0, live()

# A caller-owned return that is already pending while an *owned* buffer
# slot is allocated and then freed by the function's own epilogue. The
# epilogue must free the owned slot and leave the parameter alone, so the
# host's counter is zero throughout and the returned window still reads
# the caller's storage.
v = m.deferred(a)
assert list(v) == [1.0, 2.0, 3.0], list(v)
assert live() == 0, live()
v.release()
assert live() == 0, live()

# A public method, which shares the same generated wrapper.
g = m.Grid(2.0)
v = g.pass_through(a)
assert list(v) == [1.0, 2.0, 3.0], list(v)
v.release()
assert live() == 0, live()

# The ordering assertion, and the only subject that can make it. A PEP 688
# Python-level exporter observes its own `__buffer__` / `__release_buffer__`
# calls, so the log states the order the wrapper actually used. Under
# acquire-before-release the host object is never at zero outstanding
# exports across the boundary; under the opposite order it is, and an
# exporter that recycles storage on its last release would hand the caller
# a view over storage it had already reclaimed.
class Logged:
    def __init__(self):
        self.store = array.array('d', [1.5, 2.5, 3.5])
        self.log = []

    def __buffer__(self, flags):
        self.log.append('acquire')
        return memoryview(self.store)

    def __release_buffer__(self, view):
        self.log.append('release')
        view.release()

owner = Logged()
v = m.first(owner)
assert owner.log == ['acquire', 'acquire', 'release'], owner.log
assert list(v) == [1.5, 2.5, 3.5], list(v)
v.release()
assert owner.log == ['acquire', 'acquire', 'release', 'release'], owner.log
assert live() == 0, live()

# The acquire can fail, and the flag rather than a `borrowed != NULL` test
# is what makes that case answerable: an exporter whose *second*
# `__buffer__` raises drives `PyMemoryView_FromObject` to NULL with the
# exception set, and the wrapper must propagate that exception rather than
# fall through to the artifact-owned packer, which would free the host's
# own storage.
class MovesSecond:
    """A conforming PEP 688 exporter whose second export is a *different*
    window. The wrapper must refuse rather than hand the host storage the
    compiled body never operated on."""

    def __init__(self):
        self.first = array.array('d', [4.5, 5.5])
        self.second = array.array('d', [9.5, 8.5])
        self.calls = 0

    def __buffer__(self, flags):
        self.calls += 1
        return memoryview(self.first if self.calls == 1 else self.second)

    def __release_buffer__(self, view):
        view.release()


moving = MovesSecond()
try:
    m.first(moving)
except BufferError as exc:
    assert 'second export' in str(exc), str(exc)
else:
    raise AssertionError('a relocated second export should have been refused')
assert moving.calls == 2, moving.calls
assert live() == 0, live()


class FailsSecond:
    def __init__(self):
        self.store = array.array('d', [4.5, 5.5])
        self.calls = 0

    def __buffer__(self, flags):
        self.calls += 1
        if self.calls > 1:
            raise ValueError('no second export')
        return memoryview(self.store)

    def __release_buffer__(self, view):
        view.release()

failing = FailsSecond()
try:
    m.first(failing)
except ValueError:
    pass
else:
    raise AssertionError('the failed second acquire should have propagated')
assert failing.calls == 2, failing.calls
assert live() == 0, live()

# The window the returned view spans is the window the compiled body saw.
# The argument is a *slice* of the host's own buffer, so an implementation
# that re-derived the view from the underlying object instead of from the
# argument would disagree here and nowhere else.
window = memoryview(a)[1:3]
assert m.span(window) == 2, m.span(window)
assert m.peek(window) == 2.0, m.peek(window)
v = m.first(window)
assert len(v) == 2, len(v)
assert list(v) == [2.0, 3.0], list(v)
v.release()
window.release()
assert live() == 0, live()

# Repetition, because a leak of one export per call is invisible in a single
# round trip.
for _ in range(64):
    v = m.first(a)
    assert live() == 0, live()
    v.release()
assert live() == 0, live()

print('ok')
"#;

/// The hosted arm. Everything above proves what the compiler accepts and
/// what C it emits; this is the only arm that proves the emitted C is
/// *correct* against a real CPython.
///
/// Not compiled on Windows, matching
/// `tests/issue_1174_method_buffer_return.rs`'s own counter-reading arm and
/// for its reason: the counter is linked into the `.pyd` from a static
/// archive but is absent from its export table.
#[test]
#[cfg(not(target_os = "windows"))]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_returned_buffer_parameter_reaches_the_host_over_storage_the_caller_owns() {
    let dir = fixture("1175_hosted", SUBJECT);
    let build = build_ext(&dir);
    assert!(build.status.success(), "{}", stderr_of(&build));
    std::fs::write(dir.join("driver.py"), DRIVER).expect("write the driver");

    let run = Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
        .arg("driver.py")
        .current_dir(&*dir)
        .output()
        .expect("python3 should spawn");
    assert!(run.status.success(), "{}", stderr_of(&run));
    assert_eq!(stdout_of(&run), "ok\n");
}
