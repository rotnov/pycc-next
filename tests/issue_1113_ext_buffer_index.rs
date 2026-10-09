//! Part 2 of #1027 (#1113): a bounds-checked native element load from a
//! buffer, exercised end to end.
//!
//! Part 1 (#1112) admitted `memoryview` as an annotation and refused every
//! *use* of such a name. What this part adds is exactly one use -- `b[i]`,
//! reading one `f64` element -- so this file owns the source-level halves
//! that no in-crate unit test can reach: the diagnostics a real program
//! gets for the index position, the refusal that still covers every other
//! use, and the one hosted arm that proves the compiled load agrees with
//! CPython on both the value and the out-of-bounds exception.
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

/// Writes `source` as the entry module of a fresh scratch directory.
fn fixture(category: &str, source: &str) -> ScratchDir {
    let dir = ScratchDir::new(category).expect("scratch");
    std::fs::write(dir.join("buf_probe.py"), source).expect("write the subject");
    dir
}

/// Builds the entry module as a CPython extension module directly into
/// `dir`, so a CPython run with `dir` as its working directory imports it.
///
/// The output path carries no extension suffix, for the reason
/// `tests/issue_1112_ext_memoryview.rs`'s own helper records: `pycc build
/// --ext` appends the one its target triple calls for and derives the
/// exported `PyInit_<mod>` name from the path's own spelling.
fn build_ext(dir: &Path) -> Output {
    pycc()
        .arg("build")
        .arg(dir.join("buf_probe.py"))
        .arg("-o")
        .arg(dir.join("buf_probe"))
        .arg("--ext")
        .output()
        .expect("pycc should spawn")
}

/// The subject: the whole read surface Part 2 admits, at the one position
/// #1027 admits a `memoryview` at all.
///
/// `at` takes the index as a parameter and `first` supplies a literal, so
/// the artifact exercises both index shapes through one build. Neither
/// wraps a negative index (D-108): `pycc_rt_buffer_f64_get` refuses one
/// with the same `IndexError` an index past the end gets.
const SUBJECT: &str = "\
def at(b: memoryview, i: int) -> float:
    return b[i]


def first(b: memoryview) -> float:
    return b[0]
";

/// The index must be an `int`, and the refusal says so at the index rather
/// than blaming the base.
///
/// `T0033` ("base does not support indexing") is what a `memoryview` base
/// got before this part, and `T0021` is what every other indexable base
/// already gives a mistyped index -- so the interception has to produce the
/// latter, in the checker and in the constraint solver alike. A `bool`
/// index is admitted for the same reason every other `int` position admits
/// one (D-086), which is why the assertion below names `str`.
#[test]
fn a_non_int_buffer_index_is_refused_as_an_index_type_error() {
    let dir = fixture(
        "1113_index_type",
        "\
def total(b: memoryview) -> float:
    return b[\"k\"]
",
    );
    let build = build_ext(&dir);
    assert!(!build.status.success(), "{}", stdout_of(&build));
    let err = stderr_of(&build);
    assert!(err.contains("error[T0021]"), "{err}");
    assert!(
        err.contains("`memoryview` index must be `int`, found `str`"),
        "{err}"
    );
    // Not the "base does not support indexing" refusal Part 1 gave: the
    // base is now indexable, and only the index is wrong.
    assert!(!err.contains("error[T0033]"), "{err}");
}

/// The index position is an ordinary `int` position, so D-141's
/// int-boundary literal rule reaches it unchanged.
///
/// `T0051` is emitted during `pycc_hir` lowering, before any type is known,
/// which is precisely why this is a source-level test and not a
/// `pycc_types` unit test: hand-built HIR bypasses the pass that emits it.
/// The literal below is the first value past D-061's 63-bit tagged
/// smallint range.
#[test]
fn an_out_of_range_literal_buffer_index_is_the_d141_boundary_error() {
    let dir = fixture(
        "1113_index_boundary",
        "\
def total(b: memoryview) -> float:
    return b[4611686018427387904]
",
    );
    let build = build_ext(&dir);
    assert!(!build.status.success(), "{}", stdout_of(&build));
    let err = stderr_of(&build);
    assert!(err.contains("error[T0051]"), "{err}");
    assert!(err.contains("4611686018427387904"), "{err}");
}

/// Every *other* use of the name is still the Part 1 capability gap, and
/// the reworded message now points at the one use that works.
///
/// Aliasing is the case worth pinning: `x = b` is the shortest way a
/// buffer could escape the slot its own parameter name binds, and D-244's
/// 2026-09-17 Part-1-of-#1027 amendment statement (e) makes the wholly-
/// wrapper-owned lifetime true *by construction* only as long as that
/// escape is refused. The subject was the element store `b[0] = 1.0` until
/// Part 1 of #1142 admitted it; the arm's subject has always been the
/// shared read seam rather than whichever operation happens to reach it.
#[test]
fn aliasing_a_buffer_into_a_local_is_still_a_capability_gap() {
    let dir = fixture(
        "1113_alias_local",
        "\
def total(b: memoryview) -> float:
    x = b
    return 0.0
",
    );
    let build = build_ext(&dir);
    assert!(!build.status.success(), "{}", stdout_of(&build));
    let err = stderr_of(&build);
    assert!(err.contains("error[C0001]"), "{err}");
    assert!(
        err.contains("using `b`, which is bound to a buffer parameter"),
        "{err}"
    );
    // The reworded gap names the admitted read, so a user who hits it
    // learns what Part 2 does support rather than only what it does not.
    assert!(
        err.contains("read one element at a time with `b[i]`"),
        "{err}"
    );
}

/// The element load is admitted in native mode's *type check* -- what a
/// native build refuses is the signature, with `I0405`, exactly as Part 1
/// left it. Pinned because Part 2 adds the first `memoryview` expression
/// the type checker accepts, and an interception that answered `Ty::Float`
/// before the artifact gate ran would have replaced the documented `I0405`
/// with something else.
#[test]
fn a_native_build_still_refuses_the_signature_carrying_an_admitted_load() {
    let dir = fixture("1113_native", SUBJECT);
    let build = pycc()
        .arg("build")
        .arg(dir.join("buf_probe.py"))
        .arg("-o")
        .arg(dir.join("buf_probe"))
        .output()
        .expect("pycc should spawn");
    assert!(!build.status.success(), "{}", stdout_of(&build));
    let err = stderr_of(&build);
    assert!(err.contains("error[I0405]"), "{err}");
    assert!(err.contains("`at`'s parameter `b: memoryview`"), "{err}");
    assert!(!err.contains("error[C0001]"), "{err}");
}

/// The whole point of the part, against a real interpreter: a compiled
/// `b[i]` reads the same `f64` CPython's own `memoryview` indexing does,
/// and every index outside `0..len` raises `IndexError` instead of reading
/// out of bounds.
///
/// Negative indices are refused rather than wrapped (D-108), which is the
/// one place this deliberately differs from CPython's `memoryview` and the
/// reason `-1` is asserted explicitly rather than left to the generic
/// out-of-range case.
///
/// Hosted, for the reason every `--ext` build-and-load test is: `--ext`
/// requires a CPython 3.13+ with development headers, and CI's interpreter
/// is older.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_compiled_buffer_load_matches_cpython_and_raises_past_the_end() {
    let dir = fixture("1113_hosted_load", SUBJECT);
    let build = build_ext(&dir);
    assert!(build.status.success(), "{}", stderr_of(&build));

    let run = Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
        .arg("-c")
        .arg(
            "import array, buf_probe\n\
             data = array.array('d', [1.5, -2.25, 3.0])\n\
             view = memoryview(data)\n\
             for i in range(len(data)):\n\
             \x20   assert buf_probe.at(view, i) == view[i], (i, buf_probe.at(view, i))\n\
             assert buf_probe.first(view) == 1.5, buf_probe.first(view)\n\
             for bad in (3, 4, -1, -4):\n\
             \x20   try:\n\
             \x20       buf_probe.at(view, bad)\n\
             \x20   except IndexError as error:\n\
             \x20       assert 'out of bounds' in str(error), str(error)\n\
             \x20   else:\n\
             \x20       raise AssertionError('the load accepted %r' % (bad,))\n\
             empty = memoryview(array.array('d', []))\n\
             try:\n\
             \x20   buf_probe.first(empty)\n\
             except IndexError:\n\
             \x20   pass\n\
             else:\n\
             \x20   raise AssertionError('an empty view accepted index 0')\n\
             print('ok')\n",
        )
        .current_dir(&*dir)
        .output()
        .expect("python3 should spawn");
    assert!(run.status.success(), "{}", stderr_of(&run));
    assert_eq!(stdout_of(&run), "ok\n");
}
