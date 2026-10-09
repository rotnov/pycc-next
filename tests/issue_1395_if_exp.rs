//! End-to-end proof for the conditional expression `body if test else
//! orelse` ([#1395]).
//!
//! `docs/TYPE_SYSTEM.md`'s "Conditional expressions" section is the
//! contract. The byte-exact oracle fixture is `tests/fixtures/if_exp.py`
//! (`tests/conformance/numeric.rs`, oracle-gated). This file owns what runs
//! without the oracle: the join of every branch type, single evaluation of
//! the condition and of exactly one branch, a raising branch or condition,
//! the bigint ownership shapes and a peak-RSS gate for a missed release or a
//! double free, plus the refusals.
//!
//! [#1395]: https://github.com/rotnov/pycc/issues/1395

use pycc_scratch::ScratchDir;
use std::process::{Command, Output};

/// `2^62`, the smallest magnitude that always allocates a `BigIntObj`.
const PROMOTED: &str = "4611686018427387904";

fn pycc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
}

fn rendered(output: &Output) -> String {
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    text.replace("\r\n", "\n")
}

/// Runs `pycc check` on `source` and returns the rendered diagnostics,
/// asserting that the check failed.
fn check_fails(source: &str) -> String {
    let dir = ScratchDir::new("e2e_1395_check").expect("scratch");
    let path = dir.join("subject.py");
    std::fs::write(&path, source).expect("write the subject");
    let output = pycc()
        .arg("check")
        .arg(&path)
        .output()
        .expect("pycc should spawn");
    assert!(!output.status.success(), "{source} was accepted");
    rendered(&output)
}

/// Builds `source` in debug and `--release`, runs it, and asserts it exits 0
/// printing exactly `expected`.
fn assert_prints(source: &str, expected: &str) {
    for release in [false, true] {
        let dir = ScratchDir::new("e2e_1395_run").expect("scratch");
        let path = dir.join("subject.py");
        std::fs::write(&path, source).expect("write the subject");
        let mut build = pycc();
        build
            .arg("build")
            .arg(&path)
            .arg("-o")
            .arg(dir.join("subject"));
        if release {
            build.arg("--release");
        }
        let build = build.output().expect("pycc should spawn");
        assert!(build.status.success(), "{source}: {}", rendered(&build));
        let run = Command::new(dir.join("subject"))
            .output()
            .expect("the built program should spawn");
        assert_eq!(
            run.status.code(),
            Some(0),
            "{source} (release={release}): {}",
            rendered(&run)
        );
        assert_eq!(
            String::from_utf8_lossy(&run.stdout).replace("\r\n", "\n"),
            expected,
            "{source} (release={release}) printed the wrong output"
        );
    }
}

#[test]
fn every_branch_type_joins_and_the_right_branch_is_selected() {
    assert_prints(
        "class Box:\n    def __init__(self, n: int) -> None:\n        self.n = n\n\n\
         def f(t: bool, n: int, x: float, s: str) -> None:\n\
         \x20   print(n if t else -n, x if t else -x, s if t else s + s, t if n else not t)\n\
         \x20   xs = [1, 2] if t else [3]\n    print(len(xs), xs[0])\n\
         \x20   p = (1, 2.5) if t else (3, 4.5)\n    print(p[0], p[1])\n\
         \x20   d = {\"k\": 1} if t else {\"k\": 2}\n    print(d[\"k\"])\n\
         \x20   print((Box(1) if t else Box(2)).n)\n\
         \x20   o = n if t else None\n    if o is not None:\n        print(\"o\", o)\n\
         \x20   q = None if t else x\n    if q is not None:\n        print(\"q\", q)\n\
         \x20   r: int | None = None\n    m = r if t else n\n    print(m is None)\n\n\
         f(True, 3, 1.5, \"s\")\nf(False, 3, 1.5, \"s\")\n",
        "3 1.5 s True\n2 1\n1 2.5\n1\n1\no 3\nTrue\n\
         -3 -1.5 ss False\n1 3\n3 4.5\n2\n2\nq 1.5\nFalse\n",
    );
}

#[test]
fn a_generic_function_selects_between_two_values_of_its_type_parameter() {
    assert_prints(
        "def pick[T](a: T, b: T, t: bool) -> T:\n    return a if t else b\n\n\
         print(pick(1, 2, True), pick(\"a\", \"b\", False), pick(1.5, 2.5, False))\n",
        "1 b 2.5\n",
    );
}

#[test]
fn the_condition_runs_once_and_exactly_one_branch_runs() {
    assert_prints(
        "def say(label: str, n: int) -> int:\n    print(\"eval\", label)\n    return n\n\n\
         print(say(\"a\", 1) if say(\"t\", 1) else say(\"b\", 2))\n\
         print(say(\"c\", 1) if say(\"u\", 0) else say(\"d\", 2))\n\
         print(say(\"e\", 5) if say(\"v\", 0) else say(\"f\", 6) if say(\"w\", 1) else say(\"g\", 7))\n",
        "eval t\neval a\n1\neval u\neval d\n2\neval v\neval w\neval f\n6\n",
    );
}

#[test]
fn a_raising_branch_or_condition_propagates_to_its_handler() {
    assert_prints(
        "def f(t: bool, xs: list[int], z: int) -> None:\n\
         \x20   try:\n        print(xs[5] if t else xs[0])\n    except IndexError:\n        print(\"caught branch\")\n\
         \x20   try:\n        print(1 if xs[9] else 2)\n    except IndexError:\n        print(\"caught condition\")\n\
         \x20   print(8 // z if z else -1)\n\n\
         f(True, [1], 0)\nf(False, [7], 2)\n",
        "caught branch\ncaught condition\n-1\n7\ncaught condition\n4\n",
    );
}

#[test]
fn bigint_branches_keep_their_values_after_the_source_is_rebound() {
    assert_prints(
        &format!(
            "class Holder:\n    def __init__(self, n: int) -> None:\n        self.n = n\n\n\
             def f(t: bool) -> None:\n\
             \x20   big = {PROMOTED}\n    small = 3\n\
             \x20   x = big if t else small\n    y = small if t else big\n    big = 5\n    print(x, y, big)\n\
             \x20   other = {PROMOTED} + 1\n    z = other if t else other\n    other = 1\n    print(z, other)\n\
             \x20   h = Holder({PROMOTED} + 2)\n    v = h.n if t else 0\n    h.n = 2\n    print(v, h.n)\n\
             \x20   w = ({PROMOTED} + 3) if t else ({PROMOTED} + 4)\n    print(w)\n\
             \x20   c = {PROMOTED} + 5\n    print(1 if c + 1 else 2, c)\n\n\
             f(True)\nf(False)\n"
        ),
        "4611686018427387904 3 5\n4611686018427387905 1\n4611686018427387906 2\n4611686018427387907\n1 4611686018427387909\n\
         3 4611686018427387904 5\n4611686018427387905 1\n0 2\n4611686018427387908\n1 4611686018427387909\n",
    );
}

#[test]
fn a_str_branch_keeps_its_value_after_the_source_is_rebound() {
    assert_prints(
        "def f(t: bool, one: str, two: str) -> None:\n    a = \"left\" + one\n    b = \"right\" + two\n\
         \x20   c = a if t else b\n    a = \"x\"\n    b = \"y\"\n    print(c, a, b)\n\n\
         f(True, \"1\", \"2\")\nf(False, \"1\", \"2\")\n",
        "left1 x y\nright2 x y\n",
    );
}

#[test]
fn branches_with_no_common_type_are_refused() {
    let text = check_fails("def f(t: bool) -> None:\n    print(1 if t else True)\n");
    assert!(
        text.contains(
            "error[T0021]: conditional expression branches have no common type: int and bool (pycc has no union types)"
        ),
        "{text}"
    );
    let text = check_fails("def f(t: bool) -> None:\n    print(\"s\" if t else None)\n");
    assert!(
        text.contains("conditional expression branches have no common type: str and None"),
        "{text}"
    );
}

#[test]
fn a_condition_pycc_cannot_test_is_refused() {
    let text = check_fails("def f(xs: list[int]) -> int:\n    return 1 if xs else 2\n");
    assert!(
        text.contains(
            "error[T0021]: conditional expression condition of type `list[int]` has no truth value pycc can test"
        ),
        "{text}"
    );
    let text = check_fails(
        "class C:\n    def __init__(self) -> None:\n        self.n = 0\n    def __len__(self) -> int:\n        return 0\n\n\
         def f(c: C) -> int:\n    return 1 if c else 2\n",
    );
    assert!(
        text.contains(
            "class `C` defines `__len__`, and pycc does not call it for a truth test yet"
        ),
        "{text}"
    );
}

#[test]
fn a_walrus_in_a_branch_is_refused_at_its_location() {
    let text = check_fails("def f(t: bool) -> int:\n    return 1 if t else (n := 2)\n");
    assert!(
        text.contains(
            "error[C0001]: a walrus assignment (`:=`) in a conditional expression branch is not supported"
        ),
        "{text}"
    );
    assert!(text.contains("subject.py:2:25"), "{text}");
}

#[test]
fn an_empty_list_branch_is_refused_with_t0003() {
    let text = check_fails(
        "def f(t: bool) -> int:\n    xs: list[int] = [] if t else [1]\n    return len(xs)\n",
    );
    assert!(
        text.contains("error[T0003]: an empty list literal has no inferable element type"),
        "{text}"
    );
}

/// The `lark` subject's shape (lines 64, 88 and 101 of
/// `lalr_parser_state.py`): both branches and the condition are CPython
/// objects. Built with `--ext` and imported by a real interpreter, which
/// observes the selected value, and that a loop selecting an object and
/// testing its truth never over-releases it (an object selection aliases
/// with no refcount traffic, #1092's leak-only rule).
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_conditional_expression_over_cpython_objects_runs_in_the_host() {
    let dir = ScratchDir::new("e2e_1395_hosted").expect("scratch");
    let src = dir.join("pycc_ifexp_obj.py");
    std::fs::write(
        &src,
        "from fractions import Fraction\n\n\
         def pick(flag: bool, n: int) -> str:\n\
         \x20   a = Fraction(1, 2)\n    b = Fraction(n, 3)\n\
         \x20   v = a if flag else b\n    w = a if b else v\n\
         \x20   return str(v) + \" \" + str(w)\n\n\
         def churn(n: int) -> int:\n\
         \x20   hits = 0\n    a = Fraction(1, 2)\n    z = Fraction(0, 1)\n\
         \x20   for i in range(n):\n        v = a if i % 2 == 0 else z\n\
         \x20       if v:\n            hits = hits + 1\n    return hits\n",
    )
    .expect("write the module");
    let build = pycc()
        .arg("build")
        .arg(&src)
        .arg("-o")
        .arg(dir.join("pycc_ifexp_obj"))
        .arg("--ext")
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", rendered(&build));
    let run = Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
        .arg("-c")
        .arg(
            "import pycc_ifexp_obj as m\n\
             assert m.__file__.endswith(('.so', '.pyd')), m.__file__\n\
             print(m.pick(True, 2), '|', m.pick(False, 2), '|', m.pick(False, 0))\n\
             print(m.churn(200001))\n",
        )
        .current_dir(&*dir)
        .output()
        .expect("python3 should spawn");
    assert!(run.status.success(), "{}", rendered(&run));
    assert_eq!(
        String::from_utf8_lossy(&run.stdout).replace("\r\n", "\n"),
        "1/2 1/2 | 2/3 1/2 | 0 0\n100001\n"
    );
}

#[cfg(unix)]
mod peak_rss {
    use super::{PROMOTED, ScratchDir, pycc};
    use std::process::Command;

    /// The child's `ru_maxrss` (bytes on macOS, kilobytes on Linux, so only
    /// ever compared as a ratio). Follows `tests/issue_1211_bool_ops.rs`.
    #[allow(clippy::zombie_processes)]
    fn peak_rss(mut command: Command) -> libc::c_long {
        let child = command.spawn().expect("command must spawn");
        let pid = child.id() as libc::pid_t;
        let mut status = 0;
        let mut usage = std::mem::MaybeUninit::<libc::rusage>::zeroed();
        let waited = loop {
            // SAFETY: `pid` is the live child spawned above, and both output
            // pointers are valid for writes for the duration of the call.
            let result = unsafe { libc::wait4(pid, &mut status, 0, usage.as_mut_ptr()) };
            if result != -1 {
                break result;
            }
            let error = std::io::Error::last_os_error();
            assert_eq!(error.kind(), std::io::ErrorKind::Interrupted, "{error}");
        };
        assert_eq!(waited, pid);
        // SAFETY: the successful `wait4` above initialized `usage`.
        let usage = unsafe { usage.assume_init() };
        assert!(
            libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 0,
            "the program must exit 0 (a double free aborts it)"
        );
        usage.ru_maxrss
    }

    /// Every `int`-typed shape a loop can strand or double-release a word
    /// through: a selected borrowed name a later iteration overwrites, a
    /// selected owned temporary, and a bigint condition temporary.
    fn repro(iterations: u32) -> String {
        format!(
            "base = {PROMOTED}\nx = 0\ny = 0\nhits = 0\n\
             for i in range({iterations}):\n\
             \x20   x = base if i % 2 == 0 else (base + i)\n\
             \x20   y = (base + i) if i % 3 == 0 else x\n\
             \x20   if (1 if base + i else 0):\n        hits = hits + 1\n\
             print(x - base, y - base, hits)\n"
        )
    }

    fn built_program_peak_rss(case: &str, source: &str) -> libc::c_long {
        let dir = ScratchDir::new(case).expect("scratch");
        let src = dir.join("case.py");
        std::fs::write(&src, source).expect("write the case");
        let bin = dir.join("case");
        let build = pycc()
            .arg("build")
            .arg(&src)
            .arg("-o")
            .arg(&bin)
            .status()
            .expect("pycc build must spawn");
        assert!(build.success(), "pycc build of {case} failed");
        peak_rss(Command::new(&bin))
    }

    #[test]
    fn selected_bigint_branches_do_not_grow_with_the_trip_count() {
        let single = built_program_peak_rss("rss_1395_1x", &repro(500_000));
        let double = built_program_peak_rss("rss_1395_2x", &repro(1_000_000));
        let ratio = double as f64 / single as f64;
        assert!(
            ratio < 1.35,
            "peak RSS must not scale with the loop trip count: \
             500k iterations={single} 1M iterations={double} ratio={ratio:.4} \
             (a per-iteration `BigIntObj` leak reads as ratio ~2.0)"
        );
    }
}
