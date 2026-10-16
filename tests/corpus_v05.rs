//! v0.5 accept 3 (the real-world corpus): what the corpus needed natively.
//!
//! * A tuple assignment of inert values (`a, b = [], set()`) binds each
//!   name on its own, so its empty containers are typed by their own uses.
//! * The solver reads a `dict` subscript's value type, so `f(d[k])` types
//!   an unannotated parameter.
//! * An unannotated `__init__` parameter is typed from a constructor
//!   argument read from an unannotated function's return (`N, K = MAP()`)
//!   or a comprehension's loop variable (`[Node(i) for i in range(n)]`).
//! * `"literal".format(args)` with `{}`/`{0}` fields, `!r`/`!s` and a
//!   constant spec compiles natively.
//! * `float('inf')` in a variable an `int` later replaces is an `int`
//!   infinity (`pycc_types::int_inf`, `pycc_rt::int_inf`); `min`/`max` of a
//!   `str` are of its characters.
//! * Non-strict mode: `itertools.accumulate`, `min` and `max` over a
//!   CPython object run CPython's own; a generator yielding a native value
//!   beside CPython objects yields objects; a function returning `None` on
//!   one path and a value of another type on another returns an object;
//!   a tuple display with a `None` element is an object to the solver too.
//! * Non-strict mode: `range(...)` as a value, `tuple(xs)`, `iter` of a list
//!   display, `next`, `list`, `sorted`, `sum`, `min`/`max` of two,
//!   `enumerate` and `itertools.chain` over CPython objects are CPython's
//!   own; a fresh set on the right of a set operator on an object crosses
//!   as a copy; a store of another type into a list of `None`s makes only
//!   that list CPython's, not the stored value's other containers.
//!
//! The oracle is CPython: each fixture is built, run and compared with
//! `python3` running the file -- stdout, the uncaught exception's last line
//! and the exit status.

use pycc_scratch::ScratchDir;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

fn pycc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
}

fn python() -> Command {
    Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

fn last_line(bytes: &[u8]) -> String {
    text(bytes).lines().last().unwrap_or_default().to_string()
}

/// Builds fixture `name` (plus `flags`) in both profiles, runs it and
/// compares with CPython. Returns the build's stderr.
fn matches_cpython(name: &str, flags: &[&str]) -> String {
    let mut report = String::new();
    for release in [false, true] {
        let dir = ScratchDir::new("corpus_v05").expect("scratch");
        let out = dir.join("prog");
        let mut build = pycc();
        build
            .arg("build")
            .arg(fixture(name))
            .arg("-o")
            .arg(&out)
            .args(flags);
        if release {
            build.arg("--release");
        }
        let build = build.output().expect("pycc should spawn");
        assert!(
            build.status.success(),
            "{}{}",
            text(&build.stdout),
            text(&build.stderr)
        );
        report = text(&build.stderr);
        let ours = run_with_stdin(Command::new(&out), name);
        let mut oracle = python();
        oracle.arg(fixture(name));
        let theirs = run_with_stdin(oracle, name);
        assert_eq!(
            text(&ours.stdout),
            text(&theirs.stdout),
            "stdout differs ({name})"
        );
        assert_eq!(last_line(&ours.stderr), last_line(&theirs.stderr), "{name}");
        assert_eq!(ours.status.code(), theirs.status.code(), "{name}");
    }
    report
}

/// Runs `command` with fixture `name`'s `.stdin` file (if any) as stdin.
fn run_with_stdin(mut command: Command, name: &str) -> Output {
    let stdin = fixture(&name.replace(".py", ".stdin"));
    let input = std::fs::File::open(&stdin).map_or_else(|_| Stdio::null(), Stdio::from);
    command.stdin(input).output().expect("the program runs")
}

/// `pycc check` (plus `flags`) of `source` fails with exactly one
/// diagnostic, `code`, whose text contains `needle`.
fn refused(tag: &str, source: &str, flags: &[&str], code: &str, needle: &str) {
    let dir = ScratchDir::new(tag).expect("scratch");
    let path = dir.join("m.py");
    std::fs::write(&path, source).expect("write");
    let output = pycc()
        .arg("check")
        .args(flags)
        .arg(&path)
        .output()
        .expect("pycc should spawn");
    let rendered = text(&output.stdout);
    assert_eq!(output.status.code(), Some(1), "{tag}: {rendered}");
    assert_eq!(rendered.matches("error[").count(), 1, "{tag}: {rendered}");
    assert!(
        rendered.contains(&format!("error[{code}]")),
        "{tag}: {rendered}"
    );
    assert!(rendered.contains(needle), "{tag}: {rendered}");
}

#[test]
fn strict_infers_the_corpus_signatures() {
    matches_cpython("corpus_v05_inference.py", &[]);
}

#[test]
fn non_strict_runs_the_inference_fixture_natively() {
    let report = matches_cpython("corpus_v05_inference.py", &["--non-strict", "--report-fallbacks"]);
    assert!(!report.contains("fallback:"), "{report}");
}

#[test]
fn a_format_field_the_arguments_do_not_cover_is_not_rewritten() {
    // CPython raises `IndexError` here; the rewrite leaves it alone, so
    // strict mode refuses the call instead of miscompiling it.
    refused(
        "format_uncovered",
        "print('{} {}'.format(1))\n",
        &[],
        "T0043",
        "cannot call a method on `str`",
    );
}

#[test]
fn non_strict_runs_object_paths_without_fallback() {
    let report = matches_cpython("corpus_v05_objects.py", &["--non-strict", "--report-fallbacks"]);
    assert!(!report.contains("fallback:"), "{report}");
}

#[test]
fn non_strict_runs_cpython_builtins_over_objects_without_fallback() {
    let report = matches_cpython("corpus_v05_builtins.py", &["--non-strict", "--report-fallbacks"]);
    assert!(!report.contains("fallback:"), "{report}");
}

#[test]
fn strict_mode_keeps_refusing_the_object_builtins() {
    refused(
        "strict_range_value",
        "r = range(3)\nprint(r)\n",
        &[],
        "C0001",
        "call to builtin `range`",
    );
    refused(
        "strict_tuple_of_list",
        "t = tuple([1, 2])\nprint(t)\n",
        &[],
        "C0001",
        "call to builtin `tuple`",
    );
}

#[test]
fn an_infinity_an_int_replaces_matches_cpython() {
    matches_cpython("corpus_v05_int_inf.py", &[]);
    let report = matches_cpython("corpus_v05_int_inf.py", &["--non-strict", "--report-fallbacks"]);
    assert!(!report.contains("fallback:"), "{report}");
}

/// Builds `source` (strict) and returns the last line its run prints to
/// stderr, which must exit with status 1.
fn raised(tag: &str, source: &str) -> String {
    let dir = ScratchDir::new(tag).expect("scratch");
    let path = dir.join("m.py");
    std::fs::write(&path, source).expect("write");
    let out = dir.join("prog");
    let build = pycc()
        .arg("build")
        .arg(&path)
        .arg("-o")
        .arg(&out)
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{tag}: {}", text(&build.stdout));
    let run = Command::new(&out).output().expect("the program runs");
    assert_eq!(run.status.code(), Some(1), "{tag}");
    last_line(&run.stderr)
}

/// `docs/DEVIATIONS.md`: an operation on an `int` infinity whose CPython
/// result is a float other than an infinity (`nan`, a floor division)
/// raises CPython's conversion error, and a container does not hold one.
#[test]
fn an_int_infinity_outside_its_exact_operations_raises() {
    let prelude = "x = float('inf')\nif len('ab') > 5:\n    x = 1\n";
    assert_eq!(
        raised("int_inf_floordiv", &format!("{prelude}print(x // 2)\n")),
        "OverflowError: cannot convert float infinity to integer"
    );
    assert_eq!(
        raised("int_inf_cancel", &format!("{prelude}print(x - x)\n")),
        "OverflowError: cannot convert float infinity to integer"
    );
    assert!(
        raised("int_inf_list", &format!("{prelude}print([x])\n")).starts_with("OverflowError:")
    );
}
