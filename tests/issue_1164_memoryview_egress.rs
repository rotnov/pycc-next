//! Part 2b of #1142 (#1164): handing a real `memoryview` back to the CPython
//! host over storage the artifact owns.
//!
//! What this file owns is the *end-to-end* half of the statement -- a program
//! that allocates a buffer and returns it is admitted by `pycc build --ext`,
//! and the host receives a working `memoryview` over the artifact's own
//! storage whose lifetime is the exporter's rather than the call's. The
//! front-end refusals that bound the admission live with the issues that
//! introduced them (`tests/issue_1112_ext_memoryview.rs`,
//! `tests/issue_1129_ndarray_buffer_carrier.rs`), the generated C is pinned by
//! `src/ext_build_tests/generated_c.rs`, and the emitted IR by
//! `crates/pycc_codegen/src/tests.rs`.
//!
//! Only the arm that builds and loads an artifact is `#[ignore]`d, for the
//! reason every `ext` test is. The admission arm is not: it asserts that no
//! *refusal* is reported, which the frontend decides before the toolchain is
//! ever probed, so it runs on a host without CPython development headers.

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
    std::fs::write(dir.join("egress_probe.py"), source).expect("write the subject");
    dir
}

fn build_ext(dir: &Path) -> Output {
    pycc()
        .arg("build")
        .arg(dir.join("egress_probe.py"))
        .arg("-o")
        .arg(dir.join("egress_probe"))
        .arg("--ext")
        .output()
        .expect("pycc should spawn")
}

/// The subject: every admitted shape at once. `make` allocates and returns;
/// `fill` allocates, writes through the buffer and returns it; `mixed` takes
/// a host buffer *and* returns an artifact-owned one, which is the pair that
/// proves the two provenances do not collide in one wrapper.
const SUBJECT: &str = "\
def make(n: int) -> memoryview:
    a = ndarray(n)
    return a


def fill(n: int) -> ndarray:
    a = ndarray(n)
    i = 0
    while i < len(a):
        a[i] = float(i) * 2.0
        i = i + 1
    return a


def mixed(v: memoryview) -> NDArray:
    a = ndarray(len(v))
    i = 0
    while i < len(v):
        a[i] = v[i] + 1.0
        i = i + 1
    return a
";

/// The admission arm. Every one of the three spellings is accepted at the
/// return position of a public module-level export, and nothing in the
/// frontend refuses the program -- which is what the `C0003` capability gap
/// and the `I0405` signature refusal each used to do.
///
/// Asserted as the absence of any diagnostic rather than as a successful
/// build: the build reaches `clang` and fails there on a host with no CPython
/// development headers, and that failure is not what this arm owns.
#[test]
fn a_buffer_returning_export_is_admitted_by_an_ext_build() {
    let dir = fixture("1164_admit", SUBJECT);
    let build = build_ext(&dir);
    let err = stderr_of(&build);
    assert!(!err.contains("error[C0003]"), "{err}");
    assert!(!err.contains("error[C0001]"), "{err}");
    assert!(!err.contains("error[I0405]"), "{err}");
    assert!(!err.contains("error[T0"), "{err}");
    assert!(!err.contains("panicked"), "{err}");
}

/// ...and a native build still refuses it, unchanged. Egress is a property
/// of the `pycc build --ext` boundary: a native executable has no CPython
/// host to hand the buffer to, so the `I0405` signature refusal is exactly as
/// it was. Two-directional so a regression that admitted the signature
/// natively -- where nothing would own the storage -- cannot pass.
#[test]
fn a_native_build_still_refuses_the_same_signature() {
    let dir = fixture("1164_native", SUBJECT);
    let build = pycc()
        .arg("build")
        .arg(dir.join("egress_probe.py"))
        .arg("-o")
        .arg(dir.join("egress_probe"))
        .output()
        .expect("pycc should spawn");
    assert!(!build.status.success(), "{}", stdout_of(&build));
    let err = stderr_of(&build);
    assert!(err.contains("error[I0405]"), "{err}");
    assert!(err.contains("pycc build --ext"), "{err}");
}

/// A `tuple` of the carrier is still refused, at the return position as at
/// every other. The egress arm lives in `src/ext_build/carrier.rs`'s
/// `return_c_type` and deliberately *not* in `BoundaryCarrier::into_scalar`,
/// whose admissible set is what a tuple element is drawn from: loosening the
/// latter would have admitted `tuple[memoryview]` for free, with no shim to
/// carry an element.
#[test]
fn a_tuple_of_the_carrier_is_still_refused_at_a_return_position() {
    let dir = fixture(
        "1164_tuple_return",
        "def make(n: int) -> tuple[memoryview, int]:\n    a = ndarray(n)\n    return (a, n)\n",
    );
    let build = build_ext(&dir);
    assert!(!build.status.success(), "{}", stdout_of(&build));
    let err = stderr_of(&build);
    assert!(err.contains("error[T0039]"), "{err}");
    assert!(!err.contains("panicked"), "{err}");
}

/// The hosted arm, and the property no front-end assertion can make: the
/// returned `memoryview` outlives the call.
///
/// Every `memoryview` this artifact hands back is backed by an exporter
/// object the artifact owns, so the storage is released when the host drops
/// its last view and not when the exported call returns. The probe is
/// therefore a *use after the call returned*: reading and writing elements,
/// and `bytes()` over the whole view, all happen with the compiled frame long
/// gone. A wrapper that handed back a view over the frame's own slot would
/// pass a same-call assertion and fail these.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_returned_buffer_outlives_the_call_and_is_readable_and_writable() {
    let dir = fixture("1164_hosted", SUBJECT);
    let build = build_ext(&dir);
    assert!(build.status.success(), "{}", stderr_of(&build));

    let run = Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
        .arg("-c")
        .arg(
            "import egress_probe\n\
             v = egress_probe.make(4)\n\
             assert isinstance(v, memoryview), type(v)\n\
             assert v.format == 'd' and v.itemsize == 8, (v.format, v.itemsize)\n\
             assert v.ndim == 1 and v.shape == (4,) and v.strides == (8,)\n\
             assert not v.readonly and v.c_contiguous\n\
             v[1] = 7.5\n\
             assert v[1] == 7.5\n\
             assert len(bytes(v)) == 32\n\
             f = egress_probe.fill(3)\n\
             assert list(f) == [0.0, 2.0, 4.0], list(f)\n\
             store = bytearray(24)\n\
             host = memoryview(store).cast('d')\n\
             host[0], host[1], host[2] = 1.0, 2.0, 3.0\n\
             m = egress_probe.mixed(host)\n\
             assert list(m) == [2.0, 3.0, 4.0], list(m)\n\
             host.release()\n\
             store.append(0)\n\
             del v, f, m\n\
             for _ in range(200):\n\
             \x20   egress_probe.make(8)\n\
             print('ok')\n",
        )
        .current_dir(&*dir)
        .output()
        .expect("python3 should spawn");
    assert!(run.status.success(), "{}", stderr_of(&run));
    assert_eq!(stdout_of(&run), "ok\n");
}

/// The ownership-transfer arm, and the one property the functional probe
/// above cannot make: the returned storage is owned by the exporter object
/// the host now holds, and by nothing else.
///
/// `pycc_rt_buffer_live_views` is the allocator pair's own balance counter,
/// linked into the module from the `pycc_rt` staticlib and read back through
/// `ctypes`. Egress moves an allocation *out* of the frame, so the balance is
/// no longer zero-after-every-call: it is one while the host holds the
/// returned `memoryview`, and back to zero once the host drops it. Both
/// directions are asserted, and both are **looped**: a transfer that leaked
/// one view per call and a transfer that double-freed would each show the
/// right balance on a single iteration and diverge over sixty-four.
///
/// Not compiled on Windows, matching `issue_1165_ext_buffer_alloc.rs`'s own
/// gate and for its reason: the counter is linked into the `.pyd` from a
/// static archive but is absent from its export table. The property is
/// codegen plus runtime bookkeeping and is not platform-specific.
#[test]
#[cfg(not(target_os = "windows"))]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_returned_buffer_transfers_ownership_rather_than_leaking_or_double_freeing() {
    let dir = fixture("1164_hosted_live_views", SUBJECT);
    let build = build_ext(&dir);
    assert!(build.status.success(), "{}", stderr_of(&build));

    let run = Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
        .arg("-c")
        .arg(
            "import ctypes, egress_probe\n\
             live = ctypes.CDLL(egress_probe.__file__).pycc_rt_buffer_live_views\n\
             live.restype = ctypes.c_longlong\n\
             live.argtypes = []\n\
             assert live() == 0, live()\n\
             for _ in range(64):\n\
             \x20   v = egress_probe.make(8)\n\
             \x20   assert live() == 1, live()\n\
             \x20   del v\n\
             \x20   assert live() == 0, live()\n\
             held = [egress_probe.make(4) for _ in range(64)]\n\
             assert live() == 64, live()\n\
             del held\n\
             assert live() == 0, live()\n\
             print('ok')\n",
        )
        .current_dir(&*dir)
        .output()
        .expect("python3 should spawn");
    assert!(run.status.success(), "{}", stderr_of(&run));
    assert_eq!(stdout_of(&run), "ok\n");
}

/// The subject for the `finally`-interposed set. Every shape here puts user
/// code *between* the `return` statement and the point control actually
/// leaves the frame, which is the distinction ownership transfer turns on.
///
/// `overrides` -- a `finally` that returns a buffer of its own -- is absent
/// deliberately, but only its *bare* form is caught upstream: a `return`
/// written directly in a `finally` body is refused by the parser with
/// `L0001`. That check is syntactic and does not survive loop entry, so
/// `finally: while True: return a` reaches codegen; since review round 5 it
/// is refused with `C0001` by `body_returns_inside_finally` instead. See
/// `REFUSED_MULTI_RETURN_SUBJECT` below for those shapes.
/// `with` is absent for the same reason, refused with `C0001` ("statement
/// kind not supported yet: a `with` statement").
const FINALLY_SUBJECT: &str = "\
def reads(n: int) -> memoryview:
    a = ndarray(n)
    try:
        a[0] = 1.0
        return a
    finally:
        a[0] = 42.0


def raises(n: int) -> memoryview:
    a = ndarray(n)
    try:
        return a
    finally:
        raise ValueError(\"boom\")


def nested(n: int) -> memoryview:
    a = ndarray(n)
    try:
        try:
            return a
        finally:
            a[0] = 1.0
    finally:
        a[1] = 2.0


def in_loop(n: int) -> memoryview:
    a = ndarray(n)
    try:
        i = 0
        while i < n:
            a[i] = 3.0
            return a
        return a
    finally:
        a[0] = 4.0


def rebinds(n: int) -> memoryview:
    a = ndarray(n)
    try:
        a[0] = 1.0
        return a
    finally:
        a = ndarray(n)
        a[0] = 7.0


def rebinds_and_raises(n: int) -> memoryview:
    a = ndarray(n)
    try:
        return a
    finally:
        a = ndarray(n)
        raise ValueError(\"boom\")


def double_rebind_and_raises(n: int) -> memoryview:
    a = ndarray(n)
    try:
        return a
    finally:
        a = ndarray(n)
        a = ndarray(n)
        a[0] = 6.0
        raise ValueError(\"boom\")


def plain(n: int) -> memoryview:
    a = ndarray(n)
    a[0] = 5.0
    return a
";

/// The abort arm. A `finally` that *reads* the returned name must still see
/// live storage, and the value the host receives must be what CPython would
/// hand back.
///
/// Ownership does not transfer at the `return` statement -- that statement
/// only makes the value pending -- so the name stays bound until control
/// actually leaves the frame. Clearing it at the statement made
/// `finally: a[0] = 42.0` dereference null inside a plain `extern "C" fn`,
/// where a Rust panic cannot unwind and becomes a process abort: the
/// **hosting interpreter** dies, not just the call.
///
/// The observable is therefore the host's own exit status, which is why this
/// probe asserts `run.status.success()` before it asserts any value: a
/// harness that only compared stdout would report a confusing empty-output
/// mismatch for what is really a killed interpreter. The per-function
/// expectations are CPython's own semantics for each shape -- `reads` sees
/// the finalizer's write, `rebinds` does not (the finalizer rebound the
/// *name*, not the object already pending), and `plain` is the
/// no-`try`-at-all regression direction.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_returned_buffer_survives_every_finalizer_interposed_before_the_frame_exits() {
    let dir = fixture("1164_finally_hosted", FINALLY_SUBJECT);
    let build = build_ext(&dir);
    assert!(build.status.success(), "{}", stderr_of(&build));

    let run = Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
        .arg("-c")
        .arg(
            "import egress_probe as m\n\
             assert list(m.reads(4)) == [42.0, 0.0, 0.0, 0.0], list(m.reads(4))\n\
             assert list(m.nested(4)) == [1.0, 2.0, 0.0, 0.0], list(m.nested(4))\n\
             assert list(m.in_loop(4)) == [4.0, 0.0, 0.0, 0.0], list(m.in_loop(4))\n\
             assert list(m.rebinds(4)) == [1.0, 0.0, 0.0, 0.0], list(m.rebinds(4))\n\
             assert list(m.plain(4)) == [5.0, 0.0, 0.0, 0.0], list(m.plain(4))\n\
             for name in ('raises', 'rebinds_and_raises', 'double_rebind_and_raises'):\n\
             \x20   try:\n\
             \x20       getattr(m, name)(4)\n\
             \x20   except ValueError:\n\
             \x20       pass\n\
             \x20   else:\n\
             \x20       raise AssertionError(name)\n\
             print('ok')\n",
        )
        .current_dir(&*dir)
        .output()
        .expect("python3 should spawn");
    assert!(run.status.success(), "{}", stderr_of(&run));
    assert_eq!(stdout_of(&run), "ok\n");
}

/// The leak arm, and a **different observable** from the abort arm above on
/// purpose. A finalizer that raises abandons the pending return: the value
/// never reaches the host, so the frame still owns it and must release it.
/// Nothing about that shape crashes or returns a wrong value -- the only
/// witness is the allocator's own balance, so a probe that asserted "does
/// not abort" would pass while every raising call leaked one buffer.
///
/// Looped for the same reason the neighbouring transfer arm is looped: a
/// single iteration shows the right balance whether the frame leaks one view
/// per call or releases one too many. `rebinds_and_raises` is the arm that
/// needs the loop most -- there the pending pointer is the *only* thing that
/// still names the storage, because the finalizer rebound the local slot
/// away from it before raising.
///
/// `double_rebind_and_raises` is the **carrier** arm. The orphan flag is
/// raised by the first rebind, then a *second*, non-matching rebind of the
/// same slot runs before anything reads the flag back. Only `raised =
/// select(is_pending, 1, previously)`'s `previously` operand carries it
/// across that store, and only the raising exit makes the carry observable:
/// on a transferred return the epilogue's own `transferred` test suppresses
/// the release whatever the flag says, so replacing `previously` with a zero
/// is invisible there and leaks one buffer per call here. Every other arm has
/// at most one intervening rebind, so none of them reaches that operand.
///
/// Not compiled on Windows, matching this file's other counter-reading arm
/// and for its reason: the counter is linked into the `.pyd` from a static
/// archive but is absent from its export table.
#[test]
#[cfg(not(target_os = "windows"))]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_finalizer_that_raises_releases_the_buffer_the_abandoned_return_left_behind() {
    let dir = fixture("1164_finally_balance", FINALLY_SUBJECT);
    let build = build_ext(&dir);
    assert!(build.status.success(), "{}", stderr_of(&build));

    let run = Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
        .arg("-c")
        .arg(
            "import ctypes, egress_probe as m\n\
             live = ctypes.CDLL(m.__file__).pycc_rt_buffer_live_views\n\
             live.restype = ctypes.c_longlong\n\
             live.argtypes = []\n\
             assert live() == 0, live()\n\
             for name in ('raises', 'rebinds_and_raises', 'double_rebind_and_raises'):\n\
             \x20   for _ in range(64):\n\
             \x20       try:\n\
             \x20           getattr(m, name)(8)\n\
             \x20       except ValueError:\n\
             \x20           pass\n\
             \x20       else:\n\
             \x20           raise AssertionError(name)\n\
             \x20   assert live() == 0, (name, live())\n\
             for name in ('reads', 'nested', 'in_loop', 'rebinds', 'plain'):\n\
             \x20   for _ in range(64):\n\
             \x20       v = getattr(m, name)(8)\n\
             \x20       assert live() == 1, (name, live())\n\
             \x20       del v\n\
             \x20       assert live() == 0, (name, live())\n\
             print('ok')\n",
        )
        .current_dir(&*dir)
        .output()
        .expect("python3 should spawn");
    assert!(run.status.success(), "{}", stderr_of(&run));
    assert_eq!(stdout_of(&run), "ok\n");
}

/// The subject for the **multi-write** set: a frame that reaches a second
/// `return` while an earlier one is still pending.
///
/// Review round 5 of #1164 narrowed the buffer-egress admission to refuse
/// any function containing a `return` inside a `finally`
/// (`REFUSED_MULTI_RETURN_SUBJECT` below is what that removed, and
/// `pycc_types::buffer::buffer_return_inside_finally` carries the host crash
/// that forced it). That narrowing removes the *simultaneous* case only --
/// two returns in flight at once, which requires the inner `return` to sit
/// lexically inside a `finally` the outer one's exit path runs. A second
/// `return` reached **sequentially** survives, and is what this subject
/// exercises: a `return` abandoned by a raising finalizer is caught by an
/// enclosing handler, the name is rebound (which orphans the abandoned
/// pointer), and a second `return` then supersedes the pending record.
///
/// What it reaches is not a control-flow shape but the pending record's own
/// state machine: `(pending, orphaned)` is one two-field state whose
/// invariant is "`orphaned != 0` means the record is the pointer's sole
/// owner", and a superseding `return` is its second mutator. Each arm kills
/// one half of that mutator's transition, verified by mutation:
///
/// * `cancelled_then_rebind` kills the **release**. Neutering the
///   superseding store's release leaves the orphaned predecessor with no
///   owner at all and leaks one buffer per call -- 64 live views after 64
///   calls, where the balance arm below asserts 0.
/// * `cancelled_then_rebind_then_raises` kills the **flag clear**. Leaving
///   the inherited orphan flag raised makes the epilogue's trailing release
///   free the new pointer the owned-slot loop had already released, and
///   `pycc build --ext` links that epilogue into a plain `extern "C" fn`
///   where the allocator's abort kills the hosting interpreter (`SIGABRT`,
///   exit 134). No balance reading can substitute for that observable,
///   because the process never reaches the reading.
///
/// `two_sequential_returns` is the regression direction for both: two
/// `return` statements of two different owned buffers with nothing pending
/// between them, so the superseding store runs twice with an *unorphaned*
/// predecessor and must not release anything. Without it a naive "always
/// free the predecessor" would pass both arms above and double-free here.
///
/// Every expectation below is CPython's own answer for the same program with
/// `ndarray(n)` read as `[0.0] * n`, checked against `python3` directly.
const MULTI_RETURN_SUBJECT: &str = "\
def cancelled_then_rebind(n: int) -> memoryview:
    a = ndarray(n)
    try:
        try:
            a[0] = 1.0
            return a
        finally:
            raise ValueError(\"x\")
    except ValueError:
        pass
    a = ndarray(n)
    a[1] = 2.0
    return a


def cancelled_then_rebind_then_raises(n: int) -> memoryview:
    a = ndarray(n)
    try:
        try:
            try:
                return a
            finally:
                raise ValueError(\"x\")
        except ValueError:
            pass
        a = ndarray(n)
        return a
    finally:
        raise TypeError(\"y\")


def two_sequential_returns(c: bool, n: int) -> memoryview:
    a = ndarray(n)
    b = ndarray(n)
    b[0] = 3.0
    if c:
        return a
    return b
";

/// The leak arm of the multi-write set, and the reason it is a *counter*
/// assertion rather than a value assertion: a superseding `return` that
/// overwrote an orphaned predecessor's record returned exactly the right
/// `memoryview` and did not crash. The only witness is the allocator's
/// balance, and only over a loop -- one call leaking one buffer and one call
/// releasing one too many show the same single-iteration reading.
///
/// `cancelled_then_rebind_then_raises` is deliberately absent: it is the
/// other arm's subject, and keeping the two disjoint is what makes each
/// mutation kill exactly one test. Reverting the release at the superseding
/// store fails this arm and leaves the other green; reverting the flag clear
/// does the reverse.
///
/// Not compiled on Windows, matching this file's other counter-reading arms
/// and for their reason: the counter is linked into the `.pyd` from a static
/// archive but is absent from its export table.
#[test]
#[cfg(not(target_os = "windows"))]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_superseded_pending_return_is_released_rather_than_leaked() {
    let dir = fixture("1164_multi_return_balance", MULTI_RETURN_SUBJECT);
    let build = build_ext(&dir);
    assert!(build.status.success(), "{}", stderr_of(&build));

    let run = Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
        .arg("-c")
        .arg(
            "import ctypes, egress_probe as m\n\
             live = ctypes.CDLL(m.__file__).pycc_rt_buffer_live_views\n\
             live.restype = ctypes.c_longlong\n\
             live.argtypes = []\n\
             assert live() == 0, live()\n\
             for _ in range(64):\n\
             \x20   v = m.cancelled_then_rebind(8)\n\
             \x20   assert live() == 1, live()\n\
             \x20   del v\n\
             \x20   assert live() == 0, live()\n\
             for c in (True, False):\n\
             \x20   for _ in range(64):\n\
             \x20       v = m.two_sequential_returns(c, 8)\n\
             \x20       assert live() == 1, (c, live())\n\
             \x20       del v\n\
             \x20       assert live() == 0, (c, live())\n\
             print('ok')\n",
        )
        .current_dir(&*dir)
        .output()
        .expect("python3 should spawn");
    assert!(run.status.success(), "{}", stderr_of(&run));
    assert_eq!(stdout_of(&run), "ok\n");
}

/// The double-free arm of the multi-write set, and a **different observable**
/// again. A superseding `return` that left the inherited orphan flag raised
/// made the epilogue's trailing release free the new pointer that the
/// owned-slot loop had just released. `pycc build --ext` links that epilogue
/// into a plain `extern "C" fn`, where the allocator's abort is not a failed
/// call but a dead interpreter, so the observable is the host's own exit
/// status -- which no balance reading can substitute for, because the process
/// never reaches the reading.
///
/// The value expectations ride along for the shapes that return normally:
/// each is CPython's own answer for the same program, so an artifact that
/// stopped aborting by returning the wrong buffer would still fail here.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_superseding_return_does_not_free_the_pointer_the_slot_loop_already_released() {
    let dir = fixture("1164_multi_return_hosted", MULTI_RETURN_SUBJECT);
    let build = build_ext(&dir);
    assert!(build.status.success(), "{}", stderr_of(&build));

    let run = Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
        .arg("-c")
        .arg(
            "import egress_probe as m\n\
             for _ in range(64):\n\
             \x20   try:\n\
             \x20       m.cancelled_then_rebind_then_raises(8)\n\
             \x20   except TypeError:\n\
             \x20       pass\n\
             \x20   else:\n\
             \x20       raise AssertionError('cancelled_then_rebind_then_raises')\n\
             assert list(m.cancelled_then_rebind(4)) == [0.0, 2.0, 0.0, 0.0], \
             list(m.cancelled_then_rebind(4))\n\
             assert list(m.two_sequential_returns(True, 4)) == [0.0, 0.0, 0.0, 0.0], \
             list(m.two_sequential_returns(True, 4))\n\
             assert list(m.two_sequential_returns(False, 4)) == [3.0, 0.0, 0.0, 0.0], \
             list(m.two_sequential_returns(False, 4))\n\
             print('ok')\n",
        )
        .current_dir(&*dir)
        .output()
        .expect("python3 should spawn");
    assert!(run.status.success(), "{}", stderr_of(&run));
    assert_eq!(stdout_of(&run), "ok\n");
}

/// What review round 5 of #1164 removed: every shape that puts two `return`
/// statements in flight at once.
///
/// These six functions were admitted before that round, and the first of
/// them -- with a raising innermost finalizer -- segfaulted the hosting
/// CPython interpreter. The outer `return a` makes `a` pending; the
/// finalizer rebinds `a`, orphaning it; the nested `return b` overwrites the
/// single pending record and releases the orphaned predecessor; the inner
/// `finally` raises, cancelling the nested return; the surrounding handler
/// catches it, the outer finalizer completes, and the **outer return
/// resumes** -- handing the host a `memoryview` over freed storage.
///
/// The refusal is a property of the *admission*, not of the named shape:
/// every function here contains a `return` lexically inside a `finally`, and
/// that is the whole test. Each is checked separately, so a refusal that
/// fired for only one of them cannot pass for the rest.
///
/// `cancel` is the reproduction itself and is the transitivity pin: its
/// inner `return` sits four blocks below the `finally` that encloses it
/// (`try`/`finally` inside `while` inside a `try`, inside the `finally`
/// body), so a check of the `finally` clause's direct children answers
/// "no return here" and re-admits the segfault.
const REFUSED_MULTI_RETURN_SUBJECT: [(&str, &str); 7] = [
    (
        "cancel",
        "def cancel(n: int) -> memoryview:\n\
         \x20   a = ndarray(n)\n\
         \x20   try:\n\
         \x20       a[0] = 1.0\n\
         \x20       return a\n\
         \x20   finally:\n\
         \x20       a = ndarray(n)\n\
         \x20       a[0] = 2.0\n\
         \x20       try:\n\
         \x20           while True:\n\
         \x20               b = ndarray(n)\n\
         \x20               try:\n\
         \x20                   b[0] = 3.0\n\
         \x20                   return b\n\
         \x20               finally:\n\
         \x20                   raise ValueError(\"x\")\n\
         \x20       except ValueError:\n\
         \x20           pass\n",
    ),
    (
        "supersedes",
        "def supersedes(n: int) -> memoryview:\n\
         \x20   a = ndarray(n)\n\
         \x20   try:\n\
         \x20       a[0] = 1.0\n\
         \x20       return a\n\
         \x20   finally:\n\
         \x20       a = ndarray(n)\n\
         \x20       a[0] = 7.0\n\
         \x20       while True:\n\
         \x20           return a\n",
    ),
    (
        "supersedes_then_raises",
        "def supersedes_then_raises(n: int) -> memoryview:\n\
         \x20   a = ndarray(n)\n\
         \x20   try:\n\
         \x20       a[0] = 1.0\n\
         \x20       return a\n\
         \x20   finally:\n\
         \x20       try:\n\
         \x20           a = ndarray(n)\n\
         \x20           while True:\n\
         \x20               return a\n\
         \x20       finally:\n\
         \x20           raise ValueError(\"boom\")\n",
    ),
    (
        "supersedes_then_rebinds",
        "def supersedes_then_rebinds(n: int) -> memoryview:\n\
         \x20   a = ndarray(n)\n\
         \x20   try:\n\
         \x20       try:\n\
         \x20           return a\n\
         \x20       finally:\n\
         \x20           a = ndarray(n)\n\
         \x20           a[1] = 5.0\n\
         \x20           while True:\n\
         \x20               return a\n\
         \x20   finally:\n\
         \x20       a = ndarray(n)\n\
         \x20       a[0] = 9.0\n",
    ),
    (
        "unorphaned_predecessor",
        "def unorphaned_predecessor(n: int) -> memoryview:\n\
         \x20   a = ndarray(n)\n\
         \x20   b = ndarray(n)\n\
         \x20   try:\n\
         \x20       b[0] = 2.0\n\
         \x20       return a\n\
         \x20   finally:\n\
         \x20       while True:\n\
         \x20           return b\n",
    ),
    (
        "repeats",
        "def repeats(n: int) -> memoryview:\n\
         \x20   a = ndarray(n)\n\
         \x20   try:\n\
         \x20       a[0] = 3.0\n\
         \x20       return a\n\
         \x20   finally:\n\
         \x20       while True:\n\
         \x20           return a\n",
    ),
    (
        "double_rebind",
        "def double_rebind(n: int) -> memoryview:\n\
         \x20   a = ndarray(n)\n\
         \x20   try:\n\
         \x20       a[0] = 4.0\n\
         \x20       return a\n\
         \x20   finally:\n\
         \x20       a = ndarray(n)\n\
         \x20       a = ndarray(n)\n\
         \x20       a[0] = 6.0\n\
         \x20       while True:\n\
         \x20           return a\n",
    ),
];

/// The public-CLI half of review round 5's narrowing: every shape in
/// [`REFUSED_MULTI_RETURN_SUBJECT`] is refused, by `pycc check` and by the
/// `--ext` build path that would otherwise have emitted the segfaulting
/// artifact.
///
/// The message is pinned, not just the code: `C0001` alone is satisfied by
/// the #1165 owned-buffer refusal falling through, which would be a
/// diagnostic that lies about why the program is rejected.
#[test]
fn a_return_inside_a_finally_is_refused_at_the_buffer_egress() {
    for (label, source) in REFUSED_MULTI_RETURN_SUBJECT {
        let dir = fixture(&format!("1164_refused_{label}"), source);
        let check = pycc()
            .arg("check")
            .arg(dir.join("egress_probe.py"))
            .output()
            .expect("pycc should spawn");
        let report = format!("{}{}", stdout_of(&check), stderr_of(&check));
        assert!(!check.status.success(), "{label}: {report}");
        assert!(report.contains("error[C0001]"), "{label}: {report}");
        assert!(
            report.contains("a `return` inside a `finally` clause"),
            "{label}: {report}"
        );
        assert!(!report.contains("panicked"), "{label}: {report}");

        let build = build_ext(&dir);
        let build_report = stderr_of(&build);
        assert!(!build.status.success(), "{label}: {build_report}");
        assert!(
            build_report.contains("a `return` inside a `finally` clause"),
            "{label}: {build_report}"
        );
    }
}

/// The regression direction of the same narrowing: a `return` inside a
/// `finally` in a function whose return type is *not* the buffer type keeps
/// exactly the behavior it had before this round.
///
/// The refusal is gated at the egress admission, so this program still
/// compiles and still prints CPython's own answer. Moving the check up to
/// the function level would break every program of this shape while every
/// other test in this file stayed green.
#[test]
fn a_return_inside_a_finally_still_compiles_for_a_non_buffer_return() {
    let source = "\
def pick() -> int:
    try:
        return 1
    finally:
        while True:
            return 3


def main() -> None:
    print(pick())


main()
";
    let dir = fixture("1164_non_buffer_return_in_finally", source);
    let out = dir.join("non_buffer");
    let build = pycc()
        .arg("build")
        .arg(dir.join("egress_probe.py"))
        .arg("-o")
        .arg(&out)
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", stderr_of(&build));

    let run = Command::new(&out).output().expect("artifact should spawn");
    assert!(run.status.success(), "{}", stderr_of(&run));
    assert_eq!(stdout_of(&run), "3\n");
}

/// The public-CLI half of the definite-assignment narrowing (review round 3
/// of #1164), in Python source rather than in hand-built HIR.
///
/// The admission consulted only `owned_buffers`, which joins as a *union*,
/// while the binding joins on the `Definitely`/`Maybe`/unbound lattice. A
/// buffer allocated on one path and read back after the join was therefore
/// admitted: `pycc check` printed nothing and exited 0, and the compiled
/// artifact loaded the null-initialized slot on the other path, handing the
/// host an internal `SystemError` where `docs/TYPE_SYSTEM.md`'s
/// definite-assignment contract owes a `T0041`.
///
/// Every join form is a row, `for`-over-a-list included -- the one form the
/// unit-level table in `crates/pycc_types/src/tests/buffer_producer.rs`
/// leaves to this file, since a list iterable has no one-line HIR spelling.
/// Both directions are asserted per row: the possibly-unbound program is
/// refused with `T0041`, and the definitely-assigned twin of the same shape
/// is still admitted, so the narrowing cannot be satisfied by refusing the
/// egress outright.
#[test]
fn a_possibly_unbound_buffer_is_refused_at_the_egress_return() {
    let refused = [
        ("if", "    if c:\n        a = ndarray(4)\n"),
        (
            "if_else_one_arm",
            "    if c:\n        a = ndarray(4)\n    else:\n        other = 1\n",
        ),
        (
            "while",
            "    i = 0\n    while i < 2:\n        a = ndarray(4)\n        i = i + 1\n",
        ),
        (
            "for_range",
            "    for i in range(2):\n        a = ndarray(4)\n",
        ),
        (
            "for_list",
            "    xs = [1, 2]\n    for x in xs:\n        a = ndarray(4)\n",
        ),
        (
            "match",
            "    match c:\n        case True:\n            a = ndarray(4)\n        case _:\n            other = 1\n",
        ),
        (
            "try",
            "    try:\n        a = ndarray(4)\n    except ValueError:\n        other = 1\n",
        ),
    ];
    for (label, body) in refused {
        let source = format!("def make(c: bool) -> memoryview:\n{body}    return a\n");
        let dir = fixture(&format!("1164_unbound_{label}"), &source);
        let check = pycc()
            .arg("check")
            .arg(dir.join("egress_probe.py"))
            .output()
            .expect("pycc should spawn");
        let report = format!("{}{}", stdout_of(&check), stderr_of(&check));
        assert!(!check.status.success(), "{label}: {report}");
        assert!(report.contains("error[T0041]"), "{label}: {report}");
        assert!(!report.contains("panicked"), "{label}: {report}");

        // ...and the `--ext` build path, which is the one that would have
        // emitted the null load, reports the same diagnostic rather than
        // reaching codegen.
        let build = build_ext(&dir);
        let build_report = stderr_of(&build);
        assert!(!build.status.success(), "{label}: {build_report}");
        assert!(
            build_report.contains("error[T0041]"),
            "{label}: {build_report}"
        );
    }

    // The regression direction: the definitely-assigned twin of each join
    // shape is still admitted. `try` has no twin -- its join reports every
    // body binding back as `Maybe` whatever the handlers do, which is the
    // language's own contract and not this admission's, and the unit-level
    // table pins that the equivalent `int` program is refused identically.
    let admitted = [
        (
            "if_else_both_arms",
            "    if c:\n        a = ndarray(4)\n    else:\n        a = ndarray(8)\n",
        ),
        (
            "while_over_pre_bound",
            "    a = ndarray(4)\n    i = 0\n    while i < 2:\n        a = ndarray(8)\n        i = i + 1\n",
        ),
        (
            "for_range_over_pre_bound",
            "    a = ndarray(4)\n    for i in range(2):\n        a = ndarray(8)\n",
        ),
        (
            "for_list_over_pre_bound",
            "    a = ndarray(4)\n    xs = [1, 2]\n    for x in xs:\n        a = ndarray(8)\n",
        ),
        (
            "match_every_case",
            "    match c:\n        case True:\n            a = ndarray(4)\n        case _:\n            a = ndarray(8)\n",
        ),
        (
            "rebound_after_a_one_armed_join",
            "    if c:\n        a = ndarray(4)\n    a = ndarray(8)\n",
        ),
    ];
    for (label, body) in admitted {
        let source = format!("def make(c: bool) -> memoryview:\n{body}    return a\n");
        let dir = fixture(&format!("1164_bound_{label}"), &source);
        let check = pycc()
            .arg("check")
            .arg(dir.join("egress_probe.py"))
            .output()
            .expect("pycc should spawn");
        let report = format!("{}{}", stdout_of(&check), stderr_of(&check));
        assert!(check.status.success(), "{label}: {report}");
        assert!(!report.contains("error["), "{label}: {report}");
    }
}
