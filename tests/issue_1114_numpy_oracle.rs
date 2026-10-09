//! Part 3 of #1027 (#1114): the numpy oracle -- a real third-party array
//! reaching the compiled buffer path, checked against CPython.
//!
//! Part 1 (#1112) admitted `memoryview` in an `--ext` signature and Part 2
//! (#1113) added the one bounds-checked element load. Both proved the
//! seam against `array.array`, a buffer this repository creates itself.
//! What this part adds is the first subject whose data comes from a
//! library pycc knows nothing about: `numpy.ndarray`, reaching the
//! admitted slot as `memoryview(a.reshape(-1))` -- a zero-copy, 1-D,
//! C-contiguous, format `'d'` view.
//!
//! #1129 widened the boundary's first refusal arm from `PyMemoryView_Check`
//! to `PyObject_CheckBuffer`, so the same array now also reaches the slot
//! **bare**, with no `memoryview(...)` around it. This file carries both
//! halves of that: the bare array agrees with the view element for element
//! in the oracle below, and the refusal arms it moved off are re-pinned
//! where they land now.
//!
//! D-244 rule 7 closes the type boundary at the thunk export seam, so the
//! interesting half is symmetric: the conforming view must agree with
//! CPython element for element, and every *non*-conforming numpy argument
//! must be refused by pycc's own authored text rather than read wrong.
//!
//! The subject takes the row count `n` as a parameter because `len(b)` was
//! not a capability when this file was written (#1116, then a `C0001` gap).
//! It has been one since #1116 landed, and the prediction this paragraph
//! made held: these functions kept compiling and the file needed no change,
//! so the parameter stays as the record of that rather than being rewritten
//! now.
//!
//! The hosted arms below need numpy importable by the same interpreter
//! that `pycc build --ext` links against. On CI that is supplied by the
//! `native-build-test` job's "Install numpy into the hosted ext floor
//! interpreter" step in `.github/workflows/ci.yml`; without it these
//! tests skip and this file asserts nothing. `build-test-coverage` has no
//! such step and is expected to skip.
//!
//! None of these contributes line coverage: CI's coverage job runs
//! `llvm-cov` without `--include-ignored` and `scripts/check_diff_coverage.py`
//! excludes `tests/` from its denominator either way.

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

/// The hosted interpreter every arm here spawns: the same name
/// `pycc build --ext` probes, overridable the same way.
fn python_bin() -> std::ffi::OsString {
    std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into())
}

/// Writes the subject as the entry module of a fresh scratch directory.
fn fixture(category: &str) -> ScratchDir {
    let dir = ScratchDir::new(category).expect("scratch");
    std::fs::write(dir.join("buf_oracle.py"), SUBJECT).expect("write the subject");
    dir
}

/// Builds the subject as a CPython extension module directly into `dir`,
/// so a CPython run with `dir` as its working directory imports it.
///
/// The output path carries no extension suffix, for the reason
/// `tests/issue_1113_ext_buffer_index.rs`'s own helper records: `pycc
/// build --ext` appends the one its target triple calls for and derives
/// the exported `PyInit_<mod>` name from the path's own spelling.
fn build_ext(dir: &Path) -> Output {
    pycc()
        .arg("build")
        .arg(dir.join("buf_oracle.py"))
        .arg("-o")
        .arg(dir.join("buf_oracle"))
        .arg("--ext")
        .output()
        .expect("pycc should spawn")
}

/// Runs `script` under the hosted interpreter with `dir` as its working
/// directory, so `import buf_oracle` finds the artifact just built there.
fn python(dir: &Path, script: &str) -> Output {
    Command::new(python_bin())
        .arg("-c")
        .arg(script)
        .current_dir(dir)
        .output()
        .expect("python3 should spawn")
}

/// Whether the hosted interpreter can import numpy.
///
/// A missing numpy is an environment fact, not a defect in the diff, so
/// the hosted tests below print why they stopped and return rather than
/// failing -- exactly as `tests/slice0.rs` does for an unavailable
/// cross-compilation target.
fn numpy_is_importable() -> bool {
    Command::new(python_bin())
        .arg("-c")
        .arg("import numpy")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

/// The subject: two loops over one flattened `(n, 9)` float64 array.
///
/// `dot9` reads two elements per row and `total9` reads all nine through
/// an inner loop, so one build covers both the single-load row shape and
/// the nested one. `n` is the *row* count throughout; the element count
/// is `n * 9`, which is what the caller asserts `len(view)` to be.
const SUBJECT: &str = "\
def dot9(b: memoryview, n: int) -> float:
    s: float = 0.0
    i: int = 0
    for i in range(n):
        s = s + b[i * 9 + 0] * b[i * 9 + 1]
    return s


def total9(b: memoryview, n: int) -> float:
    s: float = 0.0
    i: int = 0
    j: int = 0
    for i in range(n):
        for j in range(9):
            s = s + b[i * 9 + j]
    return s
";

/// The subject type-checks with no interpreter and no numpy at all.
///
/// Not `#[ignore]`d, so every leg that runs the suite proves the shape
/// the hosted arms build is still an accepted program even where they
/// skip -- the one assertion in this file that never depends on the
/// environment.
#[test]
fn the_numpy_oracle_type_checks() {
    let dir = fixture("1114_check");
    let check = pycc()
        .arg("check")
        .arg(dir.join("buf_oracle.py"))
        .output()
        .expect("pycc should spawn");
    assert!(
        check.status.success(),
        "{}{}",
        stdout_of(&check),
        stderr_of(&check)
    );
}

/// The oracle itself: the compiled loops and the same source `exec`'d by
/// CPython, over the same numpy array, in the same interpreter, produce
/// bit-identical `repr` output.
///
/// Comparing `repr` rather than a tolerance is deliberate: both arms sum
/// the same `f64` values in the same order, so anything but an exact
/// match is a real divergence rather than rounding.
///
/// The small `arange(27)` case is checked against literals as well, so a
/// failure that moved *both* arms the same way is still caught.
///
/// The bare-`ndarray` arm (#1129) is asserted against the *view* arm's own
/// answers rather than printed: the interpreted arm cannot mirror it --
/// interpreted `b[i]` over a bare array is ordinary numpy indexing and
/// would agree for a reason that proves nothing about the boundary -- so
/// what it states is the property this issue owns, that the two spellings
/// of the same data reach the compiled loop identically.
///
/// Hosted, for the reason every `--ext` build-and-load test is: `--ext`
/// requires a CPython 3.13+ with development headers, and CI's coverage
/// interpreter is a different one.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_compiled_numpy_buffer_loop_matches_cpython() {
    if !numpy_is_importable() {
        eprintln!("skipping: numpy is not importable by the hosted ext interpreter");
        return;
    }
    let dir = fixture("1114_oracle");
    let build = build_ext(&dir);
    assert!(build.status.success(), "{}", stderr_of(&build));

    // The shared preamble: a float64, C-contiguous `(64, 9)` array,
    // flattened zero-copy into the 1-D format `'d'` view the signature
    // admits. `len(v) == n * 9` is asserted before any call, so a
    // reshape that silently copied or re-ranked the array fails here
    // rather than as a mismatch neither arm explains.
    const PREAMBLE: &str = "import numpy\n\
         a = numpy.random.default_rng(20260917).random((64, 9))\n\
         n = a.shape[0]\n\
         v = memoryview(a.reshape(-1))\n\
         assert v.ndim == 1 and v.format == 'd', (v.ndim, v.format)\n\
         assert len(v) == n * 9, (len(v), n)\n\
         small = numpy.arange(27, dtype=numpy.float64).reshape(3, 9)\n\
         sv = memoryview(small.reshape(-1))\n\
         bare = a.reshape(-1)\n\
         assert type(bare).__name__ == 'ndarray', type(bare)\n";

    // `__file__` is the load-bearing guard: without it an import that
    // resolved to `buf_oracle.py` instead of the built artifact would
    // make this arm a copy of the interpreted one and the oracle would
    // pass while proving nothing.
    let compiled = python(
        &dir,
        &format!(
            "{PREAMBLE}\
             import buf_oracle\n\
             assert not buf_oracle.__file__.endswith('.py'), buf_oracle.__file__\n\
             print(repr(buf_oracle.total9(v, n)), repr(buf_oracle.dot9(v, n)))\n\
             print(repr(buf_oracle.total9(sv, 3)), repr(buf_oracle.dot9(sv, 3)))\n\
             assert buf_oracle.total9(bare, n) == buf_oracle.total9(v, n)\n\
             assert buf_oracle.dot9(bare, n) == buf_oracle.dot9(v, n)\n"
        ),
    );
    assert!(compiled.status.success(), "{}", stderr_of(&compiled));

    let interpreted = python(
        &dir,
        &format!(
            "{PREAMBLE}\
             ns = {{}}\n\
             exec(open('buf_oracle.py').read(), ns)\n\
             print(repr(ns['total9'](v, n)), repr(ns['dot9'](v, n)))\n\
             print(repr(ns['total9'](sv, 3)), repr(ns['dot9'](sv, 3)))\n"
        ),
    );
    assert!(interpreted.status.success(), "{}", stderr_of(&interpreted));

    assert_eq!(
        stdout_of(&compiled),
        stdout_of(&interpreted),
        "the compiled and interpreted arms disagree"
    );
    // The exact case: `arange(27)` sums to 351.0 over all nine columns,
    // and `dot9` sums `row[0] * row[1]` over the three rows.
    assert_eq!(
        stdout_of(&compiled).lines().nth(1),
        Some("351.0 432.0"),
        "{}",
        stdout_of(&compiled)
    );
}

/// The other half of D-244 rule 7: a numpy argument that is not the
/// admitted carrier is refused at the thunk, in pycc's own words.
///
/// Four of the six arms are pycc-authored `TypeError`s naming the
/// function, the 1-based argument position, and what was wrong. The
/// remaining two are not pycc's to author: the exporter itself refuses to
/// build a C-contiguous `Py_buffer` over a strided operand, and whatever
/// it raises is propagated verbatim, so only the exception *type* is
/// asserted -- pinning wording this repository does not own would pin the
/// exporter's.
///
/// Since #1129 those two are not even one exception type. A strided
/// `memoryview` gets CPython's `BufferError`; a strided bare `ndarray`
/// gets numpy's own `ValueError`, because once the boundary admits any
/// buffer exporter the refusal at that arm is the *exporter's* and its
/// type is the exporter's choice (D-244's #1129 amendment statement (g)).
/// Both are accepted here rather than one being pinned across numpy
/// versions.
///
/// The negative-index `IndexError` is deliberately *not* re-asserted
/// here; `tests/issue_1113_ext_buffer_index.rs` owns it (D-108).
///
/// A read-only view would also be admitted rather than refused -- Part 3
/// reads only -- so there is no writability arm to assert.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_non_conforming_numpy_argument_is_refused_with_pycc_authored_text() {
    if !numpy_is_importable() {
        eprintln!("skipping: numpy is not importable by the hosted ext interpreter");
        return;
    }
    let dir = fixture("1114_refusal");
    let build = build_ext(&dir);
    assert!(build.status.success(), "{}", stderr_of(&build));

    let run = python(
        &dir,
        "import numpy, buf_oracle\n\
         a = numpy.arange(27, dtype=numpy.float64).reshape(3, 9)\n\
         def refused(arg):\n\
         \x20   try:\n\
         \x20       buf_oracle.total9(arg, 3)\n\
         \x20   except BaseException as error:\n\
         \x20       return type(error).__name__, str(error)\n\
         \x20   raise AssertionError('the thunk accepted %r' % (type(arg),))\n\
         # A bare two-dimensional ndarray. Before #1129 this was refused\n\
         # by the first arm, for not being a `memoryview` at all; the\n\
         # boundary now admits any buffer exporter, so the same array is\n\
         # answered by the rank arm instead -- which is the proof that the\n\
         # widening took effect, since only its *shape* is wrong now.\n\
         kind, text = refused(a)\n\
         assert kind == 'TypeError', (kind, text)\n\
         assert 'total9() argument 1' in text, text\n\
         assert 'ndim 2' in text, text\n\
         # A two-dimensional view of the same array.\n\
         kind, text = refused(memoryview(a))\n\
         assert kind == 'TypeError', (kind, text)\n\
         assert 'total9() argument 1' in text, text\n\
         assert 'ndim 2' in text, text\n\
         # The right rank, the wrong element type.\n\
         kind, text = refused(memoryview(numpy.arange(27, dtype=numpy.float32)))\n\
         assert kind == 'TypeError', (kind, text)\n\
         assert 'total9() argument 1' in text, text\n\
         assert \"format 'f'\" in text, text\n\
         # Not C-contiguous: CPython's own refusal, propagated verbatim.\n\
         kind, text = refused(memoryview(a.reshape(-1)[::2]))\n\
         assert kind == 'BufferError', (kind, text)\n\
         # The same condition on a bare ndarray is numpy's refusal, and\n\
         # numpy spells it `ValueError`. Both types are accepted: the arm\n\
         # propagates whatever the exporter raised, and which exception\n\
         # that is belongs to the exporter, not to this repository.\n\
         kind, text = refused(a.reshape(-1)[::2])\n\
         assert kind in ('BufferError', 'ValueError'), (kind, text)\n\
         assert 'contiguous' in text, text\n\
         # A list of the same floats exports no buffer at all, so it is\n\
         # the one arm the widening did not move. (A numpy *scalar* would\n\
         # not do: `numpy.float64` does export a buffer, of rank 0, and\n\
         # lands on the rank arm.)\n\
         kind, text = refused([1.0, 2.0, 3.0])\n\
         assert kind == 'TypeError', (kind, text)\n\
         assert 'does not export a buffer' in text, text\n\
         print('ok')\n",
    );
    assert!(run.status.success(), "{}", stderr_of(&run));
    assert_eq!(stdout_of(&run), "ok\n");
}
