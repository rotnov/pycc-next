//! `len(b)` on a `pycc build --ext` export's `memoryview` parameter (#1116),
//! exercised end to end.
//!
//! Part 2 of #1027 (#1113) admitted exactly one read of such a name, `b[i]`,
//! and left `len(b)` a `C0001` capability gap. This part admits the second,
//! which is what makes a *sweep* -- one call consuming the whole input --
//! expressible; `tests/issue_1113_ext_buffer_index.rs` owns the element
//! load's own source-level halves and is not duplicated here.
//!
//! The arm that matters is the composition one: `len(b)` has to work as a
//! `range` bound, not merely type-check in `return len(b)`. A count that
//! reached the user untagged (D-141) would satisfy a return-value test and
//! then drive the wrong number of loop iterations, which is the whole
//! failure this file exists to exclude.
//!
//! None of these contributes line coverage, for the reason
//! `tests/issue_1113_ext_buffer_index.rs`'s own header records: CI's
//! coverage job runs `llvm-cov` without `--include-ignored` and
//! `scripts/check_diff_coverage.py` excludes `tests/` from its denominator
//! either way.

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
    std::fs::write(dir.join("len_probe.py"), source).expect("write the subject");
    dir
}

/// Builds the entry module as a CPython extension module directly into
/// `dir`, so a CPython run with `dir` as its working directory imports it.
///
/// The output path carries no extension suffix, for the reason the #1113
/// file's own helper records: `pycc build --ext` appends the one its target
/// triple calls for and derives the exported `PyInit_<mod>` name from the
/// path's own spelling.
fn build_ext(dir: &Path) -> Output {
    pycc()
        .arg("build")
        .arg(dir.join("len_probe.py"))
        .arg("-o")
        .arg(dir.join("len_probe"))
        .arg("--ext")
        .output()
        .expect("pycc should spawn")
}

/// The subject: the bare count, and the sweep the count exists to enable.
///
/// `total` is the composition arm -- `for i in range(len(b)):` accumulating
/// every element -- so its result is wrong, not merely absent, if the count
/// reaches `range` with the wrong value or the wrong representation.
const SUBJECT: &str = "\
def count(b: memoryview) -> int:
    return len(b)


def total(b: memoryview) -> float:
    s = 0.0
    for i in range(len(b)):
        s = s + b[i]
    return s
";

/// Every *other* read of the name is still the Part 1 capability gap, and
/// the reworded message now names both admitted reads.
///
/// Iteration over the name itself is the case worth pinning here: `for x in
/// b` is what a user reaches for once `len(b)` works, it is *not* what this
/// part admits, and the refusal it gets has to be the read refusal on the
/// name rather than a silent acceptance.
#[test]
fn iterating_a_buffer_directly_is_still_a_capability_gap() {
    let dir = fixture(
        "1116_direct_iteration",
        "\
def total(b: memoryview) -> float:
    s = 0.0
    for x in b:
        s = s + x
    return s
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
    // The reworded gap names the sweep that does work, so a user who hits
    // it learns the shape to write instead.
    assert!(err.contains("over `range(len(b))`"), "{err}");
}

/// `len(b)` is admitted in native mode's *type check* too -- what a native
/// build refuses is the signature, with `I0405`, exactly as Part 1 left it.
/// Pinned for the reason the #1113 file pins the same property for `b[i]`:
/// an interception that answered `Ty::Int` before the artifact gate ran
/// would have replaced the documented `I0405` with something else.
#[test]
fn a_native_build_still_refuses_the_signature_carrying_an_admitted_length() {
    let dir = fixture("1116_native", SUBJECT);
    let build = pycc()
        .arg("build")
        .arg(dir.join("len_probe.py"))
        .arg("-o")
        .arg(dir.join("len_probe"))
        .output()
        .expect("pycc should spawn");
    assert!(!build.status.success(), "{}", stdout_of(&build));
    let err = stderr_of(&build);
    assert!(err.contains("error[I0405]"), "{err}");
    assert!(err.contains("`count`'s parameter `b: memoryview`"), "{err}");
    assert!(!err.contains("error[C0001]"), "{err}");
}

/// The whole point of the issue, against a real interpreter: a compiled
/// `len(b)` equals CPython's own `len(view)`, in elements and never in
/// bytes -- an `array.array('d')` of three items is eight times as many
/// bytes, so a byte count would be visibly wrong -- and the count composes
/// as a `range` bound, driving exactly `len(view)` iterations.
///
/// The empty view is asserted separately: it is the one input where a
/// wrong-by-a-constant count still produces a plausible-looking `0.0`, and
/// the one where an off-by-one sweep would index out of bounds and raise
/// instead of returning.
///
/// Hosted, for the reason every `--ext` build-and-load test is: `--ext`
/// requires a CPython 3.13+ with development headers, and CI's interpreter
/// is older.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_compiled_buffer_length_matches_cpython_and_drives_a_full_sweep() {
    let dir = fixture("1116_hosted_len", SUBJECT);
    let build = build_ext(&dir);
    assert!(build.status.success(), "{}", stderr_of(&build));

    let run = Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
        .arg("-c")
        .arg(
            "import array, len_probe\n\
             for values in ([1.5, -2.25, 3.0], [7.0], []):\n\
             \x20   data = array.array('d', values)\n\
             \x20   view = memoryview(data)\n\
             \x20   got = len_probe.count(view)\n\
             \x20   assert got == len(view), (values, got, len(view))\n\
             \x20   assert isinstance(got, int), (values, type(got))\n\
             \x20   # Elements, not bytes: `nbytes` is 8x `len` for 'd'.\n\
             \x20   assert got != view.nbytes or len(view) == 0, (values, got)\n\
             \x20   swept = len_probe.total(view)\n\
             \x20   assert swept == sum(values), (values, swept, sum(values))\n\
             print('ok')\n",
        )
        .current_dir(&*dir)
        .output()
        .expect("python3 should spawn");
    assert!(run.status.success(), "{}", stderr_of(&run));
    assert_eq!(stdout_of(&run), "ok\n");
}
