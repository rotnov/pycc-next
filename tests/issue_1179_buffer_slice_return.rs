//! Part 2 of #1175 ([#1179](https://github.com/rotnov/pycc/issues/1179)):
//! returning a **sub-range** of a `memoryview` parameter from a public
//! `pycc build --ext` export.
//!
//! Part 1 (#1180) admitted the bare parameter name. Its wrapper decides
//! which argument slot a call returned by comparing the compiled function's
//! result against `&a{index}` -- the `PyccExtBufferView` it filled from
//! `args[index]` -- and then acquires a second, independent buffer export on
//! that argument object. A sub-range breaks that test the naive way round:
//! narrow the returned view inside the callee and the pointer no longer
//! equals any argument slot, so the wrapper can no longer name an owner.
//!
//! This part therefore does not narrow the returned pointer at all. The
//! compiled body still returns `&a{index}`, unchanged, and hands the bounds
//! out of band through three trailing `long long *` out-pointers --
//! `has_slice`, `start`, `stop`. The wrapper runs Part 1's identity test and
//! its PEP 688 second-export check over the **whole** window exactly as
//! before, and only then derives the sub-range host-side, with `PySlice_New`
//! plus `PyObject_GetItem` on the checked `memoryview`. So every safety
//! property Part 1 established holds here verbatim, and CPython -- not
//! pycc -- is what interprets negative, absent, inverted and out-of-range
//! bounds.
//!
//! Two host-side assertions carry the substance and no front-end test can
//! make either:
//!
//! * the derived sub-view still pins the *host's* storage, so
//!   `array.array('d').extend(...)` raises `BufferError` while it is alive
//!   and succeeds once it is released, and `pycc_rt_buffer_live_views` is
//!   zero throughout -- the artifact allocates nothing on this path either;
//! * the bounds CPython applies are Python's own, which is the whole reason
//!   they are derived host-side. `b[-2:]`, `b[3:1]`, `b[0:99]` and
//!   `b[-99:99]` are asserted against what CPython does to a plain
//!   `memoryview`, not against a rule pycc reimplemented.
//!
//! The scope boundary is a `step`-free slice of a **parameter**. A `step`,
//! a slice of artifact-owned storage, and a slice in any position other
//! than the returned expression each stay refused for their own stated
//! reason, and each is pinned below.

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
    std::fs::write(dir.join("slice_probe.py"), source).expect("write the subject");
    dir
}

fn build_ext(dir: &Path) -> Output {
    pycc()
        .arg("build")
        .arg(dir.join("slice_probe.py"))
        .arg("-o")
        .arg(dir.join("slice_probe"))
        .arg("--ext")
        .output()
        .expect("pycc should spawn")
}

/// `pycc check` reports on stdout and knows nothing about artifact mode, so
/// it sees exactly the `pycc_types` half of this change.
fn check(dir: &Path) -> Output {
    pycc()
        .arg("check")
        .arg(dir.join("slice_probe.py"))
        .output()
        .expect("pycc should spawn")
}

/// The admitted subject: every bound spelling, both provenances in one
/// function, a whole-view export beside the sliced ones, a public method,
/// and two companion readers.
const SUBJECT: &str = "\
def head(b: memoryview) -> memoryview:
    return b[1:3]


def copy_all(b: memoryview) -> memoryview:
    return b[:]


def tail(b: memoryview) -> memoryview:
    return b[1:]


def front(b: memoryview) -> memoryview:
    return b[:3]


def last_two(b: memoryview) -> memoryview:
    return b[-2:]


def drop_last(b: memoryview) -> memoryview:
    return b[0:-1]


def past_the_end(b: memoryview) -> memoryview:
    return b[0:99]


def inverted(b: memoryview) -> memoryview:
    return b[3:1]


def wide_open(b: memoryview) -> memoryview:
    return b[-99:99]


def dynamic(b: memoryview, i: int, j: int) -> memoryview:
    return b[i:j]


def touched(b: memoryview) -> memoryview:
    b[0] = 7.0
    return b[0:2]


def branchy(b: memoryview, which: int) -> memoryview:
    if which == 0:
        return b
    return b[1:3]


def second_only(b: memoryview, c: memoryview) -> memoryview:
    return c[1:2]


def mixed(b: memoryview, which: int) -> memoryview:
    a = ndarray(2)
    a[0] = 5.0
    if which == 0:
        return b[1:3]
    return a


def whole(b: memoryview) -> memoryview:
    return b


def peek(b: memoryview) -> float:
    return b[0]


def span(b: memoryview) -> int:
    return len(b)


class Grid:
    def __init__(self, scale: float) -> None:
        self.scale = scale

    def middle(self, b: memoryview) -> memoryview:
        return b[1:3]
";

/// The admission arm, asserted as the absence of any diagnostic rather than
/// as a successful build -- exactly as Part 1's own admission arm is, and
/// for its reason: the build reaches `clang` and fails there on a host with
/// no CPython development headers, and that failure is not what this arm
/// owns.
#[test]
fn returning_a_slice_of_a_buffer_parameter_is_admitted_by_an_ext_build() {
    let dir = fixture("1179_admit", SUBJECT);
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
fn returning_a_slice_of_a_buffer_parameter_is_admitted_by_pycc_check_too() {
    let dir = fixture("1179_admit_check", SUBJECT);
    let checked = check(&dir);
    assert!(checked.status.success(), "{}", stdout_of(&checked));
}

/// A native build still refuses the same signature with `I0405`. The
/// admission is `--ext`-only for this provenance too: natively there is no
/// host object to borrow an export from and no `memoryview` to take a
/// sub-range of, and `refuse_in_native_mode` answers the signature before
/// any provenance question is asked.
#[test]
fn a_native_build_still_refuses_a_slice_returning_signature() {
    let dir = fixture("1179_native", SUBJECT);
    let build = pycc()
        .arg("build")
        .arg(dir.join("slice_probe.py"))
        .arg("-o")
        .arg(dir.join("slice_probe"))
        .output()
        .expect("pycc should spawn");
    assert!(!build.status.success(), "{}", stdout_of(&build));
    let err = stderr_of(&build);
    assert!(err.contains("error[I0405]"), "{err}");
    assert!(!err.contains("panicked"), "{err}");
}

/// A `step` is refused at compile time, in every spelling including the
/// identity `1`.
///
/// `PyccExtBufferView` is `{ void *ptr; long long len; }` -- it carries no
/// stride -- and the out-of-band bounds are two `long long`s for the same
/// reason. A strided sub-view is representable in CPython but not in the
/// artifact's own view type, so admitting `b[::2]` would mean the compiled
/// body and the host disagreed about what the returned window is. The
/// literal `1` is refused with the rest rather than folded away, so the
/// rule stays "no `step` token" and does not become "no *interesting*
/// step", which a reader could not check by looking at the source.
const STEP_REFUSALS: [(&str, &str); 3] = [
    (
        "1179_step_two",
        "def head(b: memoryview) -> memoryview:
    return b[1:3:2]
",
    ),
    (
        "1179_step_dynamic",
        "def head(b: memoryview, k: int) -> memoryview:
    return b[::k]
",
    ),
    (
        "1179_step_identity",
        "def head(b: memoryview) -> memoryview:
    return b[1:3:1]
",
    ),
];

#[test]
fn a_step_on_the_returned_slice_is_refused() {
    for (category, source) in STEP_REFUSALS {
        let dir = fixture(category, source);
        let build = build_ext(&dir);
        assert!(!build.status.success(), "{category}: {}", stdout_of(&build));
        let err = stderr_of(&build);
        assert!(!err.contains("panicked"), "{category}: {err}");
        assert!(err.contains("error[C0001]"), "{category}: {err}");
        // The dedicated message, not `reject_memoryview_read`'s. A reader
        // who is told to "read one element at a time" would be told to do
        // the one thing that cannot express a returned window at all.
        assert!(err.contains("`step`"), "{category}: {err}");
        assert!(
            !err.contains("read one element at a time"),
            "{category}: {err}"
        );
        assert!(err.contains('b'), "{category}: {err}");
    }
}

/// The shapes this part still leaves refused, one arm per reason.
///
/// * a slice of **artifact-owned** storage -- the `!owned` half of the
///   admission. The wrapper's artifact-owned path hands the host an
///   exporter over storage the artifact allocated, and there is no second
///   host object to derive a sub-range from; widening it is a separate
///   question with a separate owner.
/// * a slice **bound to a name** -- the value would then be an ordinary
///   `memoryview` local, which every other rule still refuses to read.
/// * a slice **passed to a call**, **measured**, or **indexed** -- the same,
///   in the three positions a reader is most likely to try next.
/// * a buffer-returning **call result** -- #1175's own scope boundary,
///   unmoved: a value produced inside a callee frame need not equal any of
///   this wrapper's `args[i]` slots, and that holds whether the callee
///   returned a whole window or a sub-range of one.
const SCOPE_BOUNDARY_REFUSALS: [(&str, &str); 6] = [
    (
        "1179_scope_owned",
        "def head(n: int) -> memoryview:
    a = ndarray(4)
    return a[1:3]
",
    ),
    (
        "1179_scope_bound",
        "def head(b: memoryview) -> memoryview:
    x = b[1:3]
    return x
",
    ),
    (
        "1179_scope_argument",
        "def span(b: memoryview) -> int:
    return len(b)


def head(b: memoryview) -> int:
    return span(b[1:3])
",
    ),
    (
        "1179_scope_len",
        "def head(b: memoryview) -> int:
    return len(b[1:3])
",
    ),
    (
        "1179_scope_index",
        "def head(b: memoryview) -> float:
    return b[1:3][0]
",
    ),
    (
        "1179_scope_call_result",
        "def inner(b: memoryview) -> memoryview:
    return b[1:3]


def outer(b: memoryview) -> memoryview:
    return inner(b)
",
    ),
];

#[test]
fn the_shapes_outside_part_twos_boundary_stay_refused() {
    for (category, source) in SCOPE_BOUNDARY_REFUSALS {
        let dir = fixture(category, source);
        let build = build_ext(&dir);
        assert!(!build.status.success(), "{category}: {}", stdout_of(&build));
        let err = stderr_of(&build);
        assert!(!err.contains("panicked"), "{category}: {err}");
        assert!(err.contains("error[C0001]"), "{category}: {err}");
    }
}

/// ...and `pycc check` reports every one of them too, so the interception is
/// a frontend property rather than an artifact-mode gate.
#[test]
fn every_refused_shape_is_reported_by_check_too() {
    for (category, source) in STEP_REFUSALS.iter().chain(SCOPE_BOUNDARY_REFUSALS.iter()) {
        let dir = fixture(category, source);
        let checked = check(&dir);
        assert!(!checked.status.success(), "{category}");
        let out = stdout_of(&checked);
        assert!(!out.contains("panicked"), "{category}: {out}");
        assert!(out.contains("error[C0001]"), "{category}: {out}");
    }
}

/// A slice return from a **private** function is refused too, and by a
/// different gate than everything above: the admission is gated on
/// export-set membership exactly as the two provenances before it are, and
/// asks nothing about the slice. `src/memoryview_mode.rs` owns that gate and
/// it is an artifact-mode one, so -- unlike every other refusal in this file
/// -- `pycc check` says nothing about it. That asymmetry is pinned here
/// rather than left to be rediscovered as a puzzling green `check`.
#[test]
fn a_slice_return_from_a_private_function_is_refused_by_the_ext_build_only() {
    let source = "def _head(b: memoryview) -> memoryview:
    return b[1:3]


def total() -> int:
    return 0
";
    let dir = fixture("1179_scope_private", source);
    let build = build_ext(&dir);
    assert!(!build.status.success(), "{}", stdout_of(&build));
    let err = stderr_of(&build);
    assert!(!err.contains("panicked"), "{err}");
    assert!(err.contains("error[C0001]"), "{err}");

    let checked = check(&fixture("1179_scope_private_check", source));
    assert!(checked.status.success(), "{}", stdout_of(&checked));
}

/// The bounds are ordinary expressions and are type-checked as such. The
/// admitted branch exits before the ordinary `HirExpr::Slice` arm runs, so
/// without this the early exit would silently skip the checks that arm
/// performs -- and a `str` bound would reach codegen's untagging.
///
/// A **bigint-valued** bound is deliberately absent from this table and from
/// the hosted driver. It is a run-time condition rather than a diagnostic:
/// the bound decodes through `pycc_rt_int_untag_checked`, exactly as a
/// `list` slice bound and a `b[i]` element index already do, and that
/// boundary is one of the four positions `docs/RUNTIME.md` records as
/// *still aborting* rather than raising after #1040's Part C. Reaching it
/// from a hosted artifact therefore aborts the interpreter, which a test
/// process cannot survive and observe. This part adds no new abort class --
/// it routes a new expression position through the same, already-documented
/// boundary -- so the residual stays where `docs/RUNTIME.md` records it.
const BOUND_REFUSALS: [(&str, &str, &str); 3] = [
    (
        "1179_bound_str",
        "def head(b: memoryview, s: str) -> memoryview:
    return b[s:3]
",
        "T0021",
    ),
    (
        "1179_bound_str_stop",
        "def head(b: memoryview, s: str) -> memoryview:
    return b[1:s]
",
        "T0021",
    ),
    (
        "1179_bound_unbound",
        "def head(b: memoryview, flag: int) -> memoryview:
    if flag == 0:
        k = 1
    return b[k:3]
",
        "T0041",
    ),
];

#[test]
fn a_slice_bound_is_type_checked_like_any_other_expression() {
    for (category, source, code) in BOUND_REFUSALS {
        let dir = fixture(category, source);
        let checked = check(&dir);
        assert!(!checked.status.success(), "{category}");
        let out = stdout_of(&checked);
        assert!(!out.contains("panicked"), "{category}: {out}");
        assert!(out.contains(&format!("error[{code}]")), "{category}: {out}");
    }
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
import slice_probe as m

live = ctypes.CDLL(m.__file__).pycc_rt_buffer_live_views
live.restype = ctypes.c_longlong
live.argtypes = []

# The artifact allocates nothing on this path, so the counter is zero before
# the call, *while the returned sub-view is live*, and after it is dropped.
assert live() == 0, live()

a = array.array('d', [1.0, 2.0, 3.0, 4.0])

# Every bound spelling, each checked against what CPython itself does to a
# plain memoryview over the same storage. The point of deriving the
# sub-range host-side is that these are Python's rules rather than pycc's,
# so the expected values are computed rather than written out.
#
# The reference views are read into plain lists and released immediately:
# a live memoryview of `a` would itself keep the array exported and would
# make the pinning assertion further down pass for the wrong reason.
def expect(sl):
    ref = memoryview(a)[sl]
    answer = (list(ref), ref.shape)
    ref.release()
    return answer

cases = [
    (m.head, expect(slice(1, 3))),
    (m.copy_all, expect(slice(None, None))),
    (m.tail, expect(slice(1, None))),
    (m.front, expect(slice(None, 3))),
    (m.last_two, expect(slice(-2, None))),
    (m.drop_last, expect(slice(0, -1))),
    (m.past_the_end, expect(slice(0, 99))),
    (m.inverted, expect(slice(3, 1))),
    (m.wide_open, expect(slice(-99, 99))),
]
for fn, (values, shape) in cases:
    v = fn(a)
    assert isinstance(v, memoryview), (fn.__name__, type(v))
    assert v.format == 'd' and v.itemsize == 8 and v.ndim == 1, (fn.__name__, v.format)
    assert v.shape == shape, (fn.__name__, v.shape, shape)
    assert list(v) == values, (fn.__name__, list(v), values)
    assert v.c_contiguous, fn.__name__
    v.release()
    assert live() == 0, (fn.__name__, live())

# The empty sub-view in particular is a real, well-formed memoryview rather
# than a null window: an implementation that clamped the bounds itself and
# handed back a negative length would show up here and nowhere else.
v = m.inverted(a)
assert v.shape == (0,), v.shape
assert len(v) == 0, len(v)
assert list(v) == [], list(v)
assert v.c_contiguous, v.c_contiguous
v.release()

# Bounds computed at run time reach the same derivation, including the
# negative and out-of-range ones -- they are `long long` out-slots, not
# folded literals.
for start, stop in [(1, 3), (0, 4), (2, 2), (-2, 99), (3, 1), (-99, -1), (0, -3)]:
    values, _shape = expect(slice(start, stop))
    v = m.dynamic(a, start, stop)
    assert list(v) == values, (start, stop, list(v), values)
    v.release()
    assert live() == 0, (start, stop, live())

# The sub-view pins the *host's* storage, exactly as Part 1's whole window
# does. `array.array` relocates its block under `extend` and refuses to
# while it is exported, so this raises while the sub-view is alive and
# succeeds once it is released.
v = m.head(a)
try:
    a.extend([5.0])
except BufferError:
    pass
else:
    raise AssertionError('extend should have been refused while the sub-view was live')
v.release()
a.extend([5.0])
assert len(a) == 5, len(a)
del a[4:]
assert live() == 0, live()

# Writeback through the returned sub-view reaches the host object, at the
# sub-range's own offset rather than at the window's.
v = m.head(a)
v[0] = 20.0
v.release()
assert a[1] == 20.0, a[1]
a[1] = 2.0

# ...and a body that writes through the parameter before slicing it hands
# back a view over the same, already-written storage.
v = m.touched(a)
assert list(v) == [7.0, 2.0], list(v)
v.release()
assert a[0] == 7.0, a[0]
a[0] = 1.0
assert live() == 0, live()

# The returned sub-view's writability is the host exporter's.
ro = memoryview(bytes(32)).cast('d')
v = m.head(ro)
assert v.readonly is True, v.readonly
assert list(v) == [0.0, 0.0], list(v)
v.release()
ro.release()
assert live() == 0, live()

# One export, two provenances on two branches: the whole window and a
# sub-range of it. The `has_slice` out-slot is what distinguishes them, and
# it is initialized to zero by the wrapper, so the bare-return branch must
# come back as the whole window rather than as whatever the previous call
# left behind.
for _ in range(3):
    v = m.branchy(a, 1)
    assert list(v) == [2.0, 3.0], list(v)
    v.release()
    w = m.branchy(a, 0)
    assert list(w) == [1.0, 2.0, 3.0, 4.0], list(w)
    w.release()
assert live() == 0, live()

# Two buffer parameters, only the second sliced: the wrapper's
# pointer-identity test still names the owner, because the compiled body
# returns the *unnarrowed* `&a1`.
b = array.array('d', [10.0, 11.0, 12.0])
v = m.second_only(a, b)
assert list(v) == [11.0], list(v)
v.release()
v = m.second_only(b, b)
assert list(v) == [11.0], list(v)
v.release()
assert live() == 0, live()

# A whole-view export in the same artifact is untouched by this part: its
# generated wrapper is byte-identical to the one Part 1 emitted, and it
# carries no out-slots at all.
v = m.whole(a)
assert list(v) == [1.0, 2.0, 3.0, 4.0], list(v)
v.release()
assert live() == 0, live()

# A function mixing the sub-range and the artifact-owned provenance. The
# sliced branch allocates nothing; the owned branch does, and its view is
# the one the host then owns and frees.
v = m.mixed(a, 0)
assert list(v) == [2.0, 3.0], list(v)
assert live() == 0, live()
v.release()
w = m.mixed(a, 1)
assert list(w) == [5.0, 0.0], list(w)
assert live() == 1, live()
del w
assert live() == 0, live()

# A public method, which shares the same generated wrapper.
g = m.Grid(2.0)
v = g.middle(a)
assert list(v) == [2.0, 3.0], list(v)
v.release()
assert live() == 0, live()

# The PEP 688 checks run against the *whole* window, before any sub-range is
# derived -- so they behave exactly as Part 1 established, and the
# sub-range is never reached when they refuse.
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
v = m.head(owner)
assert owner.log == ['acquire', 'acquire', 'release'], owner.log
assert list(v) == [2.5, 3.5], list(v)
v.release()
assert owner.log == ['acquire', 'acquire', 'release', 'release'], owner.log
assert live() == 0, live()


class MovesSecond:
    def __init__(self):
        self.first = array.array('d', [4.5, 5.5, 6.5])
        self.second = array.array('d', [9.5, 8.5, 7.5])
        self.calls = 0

    def __buffer__(self, flags):
        self.calls += 1
        return memoryview(self.first if self.calls == 1 else self.second)

    def __release_buffer__(self, view):
        view.release()


moving = MovesSecond()
try:
    m.head(moving)
except BufferError as exc:
    assert 'second export' in str(exc), str(exc)
else:
    raise AssertionError('a relocated second export should have been refused')
assert moving.calls == 2, moving.calls
assert live() == 0, live()


class FailsSecond:
    def __init__(self):
        self.store = array.array('d', [4.5, 5.5, 6.5])
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
    m.head(failing)
except ValueError:
    pass
else:
    raise AssertionError('the failed second acquire should have propagated')
assert failing.calls == 2, failing.calls
assert live() == 0, live()

# The window a sub-range is taken of is the window the compiled body saw.
# The argument is itself a *slice* of the host's own buffer, so an
# implementation that re-derived the view from the underlying object instead
# of from the argument would disagree here and nowhere else.
window = memoryview(a)[1:4]
assert m.span(window) == 3, m.span(window)
assert m.peek(window) == 2.0, m.peek(window)
v = m.head(window)
assert list(v) == [3.0, 4.0], list(v)
v.release()
window.release()
assert live() == 0, live()

# Repetition, because a leak of one export per call -- or one leaked slice
# object -- is invisible in a single round trip.
for _ in range(64):
    v = m.head(a)
    assert live() == 0, live()
    v.release()
assert live() == 0, live()

print('ok')
"#;

/// The hosted arm. Everything above proves what the compiler accepts and
/// what C it emits; this is the only arm that proves the emitted C is
/// *correct* against a real CPython.
///
/// Not compiled on Windows, matching Part 1's own counter-reading arm and
/// for its reason: the counter is linked into the `.pyd` from a static
/// archive but is absent from its export table.
#[test]
#[cfg(not(target_os = "windows"))]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_returned_buffer_sub_range_reaches_the_host_over_storage_the_caller_owns() {
    let dir = fixture("1179_hosted", SUBJECT);
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

/// Review round 2's subject: the orderings in which one frame executes more
/// than one `return`, and the two redefinition orderings.
///
/// Both classes attack the same assumption -- that the wrapper's zero-init
/// of `has_slice` is enough because at most one return ever stores.
///
/// *Within one frame*, `pycc_types::buffer::buffer_return_inside_finally`
/// removes only the case of two returns in flight *simultaneously*. The
/// sequential case survives: a return abandoned by a raising finalizer that
/// an enclosing handler then swallows, followed by a second return. The
/// out-slots must describe the return that actually reached the wrapper,
/// not the last one that executed a store, so every return in a widened
/// frame writes `has_slice` itself.
///
/// *Across definitions*, the two `def`s of one name share one `fnptr_`
/// slot and one signature, so the out-slot fact is resolved per name over
/// every definition rather than per definition. The last definition is
/// still the one bound -- `redefined` slices and `redefined_to_whole` does
/// not -- but both are widened, which is why the whole-view definition's
/// own `has_slice = 0` store is what makes last-wins come out right.
///
/// Consumed by the admission arm below on every target, so no `cfg` gate.
const RESEQUENCED: &str = "\
def abandoned_then_whole(b: memoryview) -> memoryview:
    try:
        try:
            return b[1:3]
        finally:
            raise ValueError('x')
    except ValueError:
        pass
    return b


def abandoned_then_sliced(b: memoryview) -> memoryview:
    try:
        try:
            return b
        finally:
            raise ValueError('x')
    except ValueError:
        pass
    return b[1:3]


def abandoned_slice_then_slice(b: memoryview) -> memoryview:
    try:
        try:
            return b[1:3]
        finally:
            raise ValueError('x')
    except ValueError:
        pass
    return b[2:5]


def redefined(b: memoryview) -> memoryview:
    return b


def redefined(b: memoryview) -> memoryview:
    return b[1:3]


def redefined_to_whole(b: memoryview) -> memoryview:
    return b[1:3]


def redefined_to_whole(b: memoryview) -> memoryview:
    return b
";

/// The compile-time half, in the same "absence of a diagnostic" form the
/// admission arm above uses and for its reason.
///
/// `redefined` is the regression that matters here: widening only the
/// slicing definition left `UserFunction::fn_type` describing the *first*
/// definition, so the thunk forwarded four arguments through a
/// one-argument function type and `verify()` aborted the compiler on input
/// the front end had accepted.
#[test]
fn a_redefined_or_resequenced_buffer_slice_export_compiles() {
    let dir = fixture("1179_resequenced", RESEQUENCED);
    let build = build_ext(&dir);
    let err = stderr_of(&build);
    assert!(!err.contains("panicked"), "{err}");
    assert!(!err.contains("Incorrect number of arguments"), "{err}");
    assert!(!err.contains("error[C0001]"), "{err}");
    assert!(!err.contains("error[T0"), "{err}");

    let checked = check(&fixture("1179_resequenced_check", RESEQUENCED));
    assert!(checked.status.success(), "{}", stdout_of(&checked));
}

/// The host-side driver for [`RESEQUENCED`]. Only the Unix-only hosted arm
/// below consumes it, so Windows would otherwise see an unused constant and
/// fail `-D warnings`.
#[cfg(not(target_os = "windows"))]
const RESEQUENCED_DRIVER: &str = r#"
import array
import ctypes
import slice_probe as m

live = ctypes.CDLL(m.__file__).pycc_rt_buffer_live_views
live.restype = ctypes.c_longlong
live.argtypes = []

a = array.array('d', [0.0, 1.0, 2.0, 3.0, 4.0, 5.0])

# Each expectation is CPython's own answer for the bounds the *surviving*
# return names, so the abandoned return's bounds cannot pass for them.
def expect(sl):
    ref = memoryview(a)[sl]
    answer = list(ref)
    ref.release()
    return answer

whole = list(a)
cases = [
    # The abandoned sub-range must not colour the bare return that
    # superseded it: this is the whole buffer, not `b[1:3]`.
    (m.abandoned_then_whole, whole),
    # ...and the reverse ordering still derives the sub-range.
    (m.abandoned_then_sliced, expect(slice(1, 3))),
    # Two sub-ranges in one frame: the surviving one's bounds win.
    (m.abandoned_slice_then_slice, expect(slice(2, 5))),
    # Last definition wins, in both directions.
    (m.redefined, expect(slice(1, 3))),
    (m.redefined_to_whole, whole),
]
for fn, values in cases:
    # Repeated, because a stale out-slot from a previous call is exactly the
    # failure shape here and a single round trip can hide it.
    for _ in range(3):
        v = fn(a)
        assert isinstance(v, memoryview), (fn.__name__, type(v))
        assert list(v) == values, (fn.__name__, list(v), values)
        assert len(v) == len(values), (fn.__name__, len(v), len(values))
        v.release()
        assert live() == 0, (fn.__name__, live())

# Interleaved, so one export's out-slots cannot be read as another's.
for _ in range(8):
    for fn, values in cases:
        v = fn(a)
        assert list(v) == values, (fn.__name__, list(v), values)
        v.release()
assert live() == 0, live()

print('ok')
"#;

/// The hosted arm for [`RESEQUENCED`]. Only a real CPython can say what the
/// wrapper actually handed back, which is the whole substance of both
/// findings.
///
/// Not compiled on Windows, for the same reason the arm above is not: the
/// live-view counter is linked into the `.pyd` but absent from its export
/// table.
#[test]
#[cfg(not(target_os = "windows"))]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_superseded_sub_range_never_colours_the_return_that_reached_the_host() {
    let dir = fixture("1179_resequenced_hosted", RESEQUENCED);
    let build = build_ext(&dir);
    assert!(build.status.success(), "{}", stderr_of(&build));
    std::fs::write(dir.join("driver.py"), RESEQUENCED_DRIVER).expect("write the driver");

    let run = Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
        .arg("driver.py")
        .current_dir(&*dir)
        .output()
        .expect("python3 should spawn");
    assert!(run.status.success(), "{}", stderr_of(&run));
    assert_eq!(stdout_of(&run), "ok\n");
}
