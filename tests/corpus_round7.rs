//! v0.2 accept 4 (the real-world corpus): what the corpus needed natively.
//!
//! * Strict mode infers the signature of an unannotated public function or
//!   method of a native program from its call sites, like a private
//!   helper's: one signature all call sites agree on, otherwise the
//!   `T0001` a missing annotation always had. An `--ext` module keeps
//!   `T0001`: Python calls it with anything.
//! * A native program does not compile an unannotated definition nothing in
//!   the entry module mentions (it can never run), so it cannot fall back
//!   (non-strict mode) or need annotations (strict mode, since v0.3).
//! * `reversed(range(...))`, `map(f, range(...))` and `filter(f,
//!   range(...))` iterate the range as a list.
//! * Non-strict mode copies a fresh native list (nothing else holds it
//!   yet) into a CPython one where CPython code takes it.
//! * `sys.stdin.buffer` reads `bytes`: natively only through an alias
//!   whose every call is converted to numbers at once.
//! * `name = iter(lines).__next__`, `.pop()` of a subscripted list, list
//!   displays as unpacking targets, nested `for` targets, and a tuple
//!   display only iterated or searched compile natively.
//! * Non-strict mode: a list that comes to hold values of two types is
//!   CPython's own list; an object used as a list or `str` index or as a
//!   `range()` argument is read through `__index__`; a display of `None`
//!   elements is CPython's own list or tuple; a comprehension collecting
//!   CPython objects builds CPython's own list or set; a mixed-type store
//!   into an item of a nested list widens the outer list; `str % args`
//!   formats through CPython; `int ** <negative literal>` is a float; a dict
//!   given a key or value of another type is CPython's own dict; keyed
//!   sorting and `heapq`/`bisect` over an object run CPython's own; a name
//!   bound on some paths only is checked when read (`NameError`,
//!   `UnboundLocalError`); `open(0)` is standard input; `x is True` is
//!   identity with a `bool` singleton.
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
        let dir = ScratchDir::new("corpus_round7").expect("scratch");
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
fn strict_infers_unannotated_public_functions_and_methods() {
    matches_cpython("strict_public_inference.py", &[]);
}

#[test]
fn non_strict_runs_the_same_program() {
    matches_cpython("strict_public_inference.py", &["--non-strict"]);
}

#[test]
fn an_uninferable_public_signature_keeps_t0001() {
    let param = "needs a type annotation";
    // No call site types the parameter of a function the program names.
    refused(
        "t0001_uncalled",
        "def f(x):\n    return x\n\nif False:\n    f\nprint(1)\n",
        &[],
        "T0001",
        "parameter `x` of public function `f` needs a type annotation",
    );
    // Call sites disagree: one signature cannot serve both.
    refused(
        "t0001_conflict",
        "def f(x):\n    return x + 1\n\nprint(f(1))\nprint(f(2.5))\n",
        &[],
        "T0001",
        param,
    );
    // The body never pins the return type (only its own recursion does).
    refused(
        "t0001_return",
        "def g(n):\n    return g(n - 1)\n\nif False:\n    print(g(3))\n",
        &[],
        "T0001",
        "public function `g` needs a return type annotation",
    );
    // A method's call sites disagree.
    refused(
        "t0001_method_conflict",
        "class C:\n    def m(self, x):\n        return x\n\nc = C()\nprint(c.m(1), c.m(\"a\"))\n",
        &[],
        "T0001",
        "parameter `x` of public function `m` needs a type annotation",
    );
    // Constructor calls disagree on an `__init__` parameter.
    refused(
        "t0001_init",
        "class C:\n    def __init__(self, n):\n        self.n = n\n\nprint(C(1).n, C(\"a\").n)\n",
        &[],
        "T0001",
        "parameter `n` of public function `__init__` needs a type annotation",
    );
}

/// Strict mode, like non-strict mode, does not compile an unannotated
/// definition nothing in the entry module mentions: no call site could type
/// it, and it can never run.
#[test]
fn strict_mode_does_not_compile_dead_unannotated_definitions() {
    matches_cpython("non_strict_dead_defs.py", &[]);
}

#[test]
fn an_ext_module_keeps_t0001_for_an_unannotated_public_function() {
    let dir = ScratchDir::new("corpus_round7_ext").expect("scratch");
    let path = dir.join("m.py");
    std::fs::write(&path, "def f(x):\n    return x + 1\n\nprint(f(1))\n").expect("write");
    let output = pycc()
        .arg("build")
        .arg(&path)
        .arg("-o")
        .arg(dir.join("m"))
        .arg("--ext")
        .output()
        .expect("pycc should spawn");
    let rendered = format!("{}{}", text(&output.stdout), text(&output.stderr));
    assert_eq!(output.status.code(), Some(1), "{rendered}");
    assert!(
        rendered.contains("parameter `x` of public function `f` needs a type annotation"),
        "{rendered}"
    );
}

#[test]
fn non_strict_does_not_compile_dead_unannotated_definitions() {
    let report = matches_cpython(
        "non_strict_dead_defs.py",
        &["--non-strict", "--report-fallbacks"],
    );
    assert!(!report.contains("fallback:"), "{report}");
}

/// A compiled class instance that reaches CPython code as an object in an
/// embedded executable keeps its attributes and properties there.
#[test]
#[ignore = "needs CPython 3.13+ with a shared libpython (PYCC_PYTHON, default python3)"]
fn an_embedded_instance_keeps_its_attributes_in_cpython_code() {
    matches_cpython("embedded_instance_attrs.py", &["--non-strict"]);
}

/// A definition fallback code reads by name is not dead: the fallback
/// rewrite's `exec`uted source is scanned too.
#[test]
fn a_definition_only_fallback_code_reads_is_kept() {
    let dir = ScratchDir::new("corpus_round7_fallback_reads").expect("scratch");
    let path = dir.join("m.py");
    std::fs::write(
        &path,
        "def helper(x):\n    return x * 2\n\nf = lambda: [helper(3), None]\nprint(f())\n",
    )
    .expect("write");
    let out = dir.join("m");
    let build = pycc()
        .args(["build", "--non-strict", "--report-fallbacks"])
        .arg(&path)
        .arg("-o")
        .arg(&out)
        .output()
        .expect("pycc should spawn");
    assert!(
        build.status.success(),
        "{}{}",
        text(&build.stdout),
        text(&build.stderr)
    );
    let ours = Command::new(&out).output().expect("the program runs");
    let theirs = python().arg(&path).output().expect("python3 runs");
    assert_eq!(text(&ours.stdout), text(&theirs.stdout));
    assert_eq!(ours.status.code(), theirs.status.code());
}

#[test]
fn non_strict_copies_fresh_lists_into_cpython_code() {
    matches_cpython("non_strict_fresh_lists.py", &["--non-strict"]);
}

#[test]
fn a_numbers_only_stdin_buffer_alias_reads_natively() {
    matches_cpython("stdin_buffer_numbers.py", &[]);
}

/// Anything else of `sys.stdin.buffer` is `bytes`, CPython's own binary
/// stream read through the `sys` module (it used to read as `str`).
#[test]
fn stdin_buffer_reads_bytes() {
    matches_cpython("stdin_buffer_bytes.py", &["--non-strict"]);
    matches_cpython("stdin_buffer_strict.py", &[]);
}

#[test]
fn corpus_idioms_compile_natively() {
    matches_cpython("round7_idioms.py", &[]);
}

#[test]
fn non_strict_lists_of_two_types_are_cpython_lists() {
    let report = matches_cpython(
        "non_strict_mixed_lists.py",
        &["--non-strict", "--report-fallbacks"],
    );
    assert!(!report.contains("fallback:"), "{report}");
}

#[test]
fn non_strict_object_indexes_read_through_index() {
    let report = matches_cpython(
        "non_strict_object_index.py",
        &["--non-strict", "--report-fallbacks"],
    );
    assert!(!report.contains("fallback:"), "{report}");
}

#[test]
fn non_strict_none_displays_are_cpython_containers() {
    let report = matches_cpython(
        "non_strict_none_lists.py",
        &["--non-strict", "--report-fallbacks"],
    );
    assert!(!report.contains("fallback:"), "{report}");
}

#[test]
fn non_strict_object_comprehensions_are_cpython_containers() {
    let report = matches_cpython(
        "non_strict_object_comprehensions.py",
        &["--non-strict", "--report-fallbacks"],
    );
    assert!(!report.contains("fallback:"), "{report}");
}

#[test]
fn non_strict_nested_item_stores_widen_the_outer_list() {
    let report = matches_cpython(
        "non_strict_nested_stores.py",
        &["--non-strict", "--report-fallbacks"],
    );
    assert!(!report.contains("fallback:"), "{report}");
}

#[test]
fn non_strict_percent_formatting_is_cpythons() {
    let report = matches_cpython(
        "non_strict_percent_format.py",
        &["--non-strict", "--report-fallbacks"],
    );
    assert!(!report.contains("fallback:"), "{report}");
}

#[test]
fn a_negative_literal_exponent_is_a_float() {
    let report = matches_cpython("negative_literal_exponent.py", &[]);
    assert!(!report.contains("fallback:"), "{report}");
}

#[test]
fn non_strict_mixed_dicts_are_cpython_dicts() {
    let report = matches_cpython(
        "non_strict_dict_widening.py",
        &["--non-strict", "--report-fallbacks"],
    );
    assert!(!report.contains("fallback:"), "{report}");
}

#[test]
fn non_strict_keyed_sorting_and_heaps_over_objects_are_cpythons() {
    let report = matches_cpython(
        "non_strict_object_stdlib.py",
        &["--non-strict", "--report-fallbacks"],
    );
    assert!(!report.contains("fallback:"), "{report}");
}

#[test]
fn non_strict_maybe_bound_reads_are_checked_at_run_time() {
    let report = matches_cpython(
        "non_strict_maybe_bound.py",
        &["--non-strict", "--report-fallbacks"],
    );
    assert!(!report.contains("fallback:"), "{report}");
}

#[test]
fn open_zero_reads_standard_input() {
    for fixture in ["open_stdin.py", "open_stdin_lines.py"] {
        let report = matches_cpython(fixture, &[]);
        assert!(!report.contains("fallback:"), "{report}");
    }
}

#[test]
fn non_strict_identity_with_bool_and_none_literals() {
    let report = matches_cpython(
        "non_strict_identity_literals.py",
        &["--non-strict", "--report-fallbacks"],
    );
    assert!(!report.contains("fallback:"), "{report}");
}
