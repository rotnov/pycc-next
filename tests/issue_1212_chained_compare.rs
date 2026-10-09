//! End-to-end proof for chained comparisons `a < b < c` ([#1212], Part 4 of
//! [#1018]).
//!
//! `docs/TYPE_SYSTEM.md`'s "Chained comparisons" section is the contract.
//! The byte-exact oracle fixture is `tests/fixtures/chained_compare.py`
//! (`tests/conformance/numeric.rs`, oracle-gated). This file owns what runs
//! without the oracle: the short circuit and single evaluation, a raising
//! operand and a raising link, the refusals, and peak-RSS gates for the
//! release of every heap-bigint operand on the exception edge.
//!
//! [#1212]: https://github.com/rotnov/pycc/issues/1212
//! [#1018]: https://github.com/rotnov/pycc/issues/1018

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
    let dir = ScratchDir::new("e2e_1212_check").expect("scratch");
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

/// Builds `source` in `--debug` and `--release`, runs each, and asserts it
/// exits 0 printing exactly `expected`.
fn assert_prints(source: &str, expected: &str) {
    for release in [false, true] {
        let dir = ScratchDir::new("e2e_1212_run").expect("scratch");
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
fn operands_are_evaluated_once_and_later_operands_short_circuit() {
    // The plan's CPython 3.14 probe, byte for byte.
    assert_prints(
        "def say(label: str, n: int) -> int:\n    print(\"eval\", label)\n    return n\n\n\
         print(say(\"a\", 1) < say(\"b\", 2) < say(\"c\", 3))\n\
         print(say(\"a\", 1) < say(\"b\", 0) < say(\"c\", 3))\n\
         print(say(\"a\", 1) < say(\"b\", 2) > say(\"c\", 3) < say(\"d\", 9))\n",
        "eval a\neval b\neval c\nTrue\neval a\neval b\nFalse\neval a\neval b\neval c\nFalse\n",
    );
}

#[test]
fn chains_over_inline_ints_and_names_in_every_context() {
    assert_prints(
        "def f(a: int, b: int, c: int) -> bool:\n    return a < b <= c\n\n\
         def g(n: int) -> None:\n\
         \x20   ok = 0 <= n < 10\n    print(ok, 1 < 2 < 3, 3 > 2 > 2, 1 == 1 == 1 != 2)\n\
         \x20   if 0 < n < 5:\n        print(\"small\")\n    elif 5 <= n < 10 >= n:\n        print(\"medium\")\n\
         \x20   count = 0\n    while 0 <= count < n < 100:\n        count = count + 1\n    print(count)\n\n\
         print(f(1, 2, 2), f(1, 3, 2), f(2, 1, 3))\ng(3)\ng(7)\n",
        "True False False\nTrue True False True\nsmall\n3\nTrue True False True\nmedium\n7\n",
    );
}

#[test]
fn dataclass_links_call_the_synthesized_eq_once_per_operand() {
    assert_prints(
        "from dataclasses import dataclass\n\n@dataclass\nclass P:\n    x: int\n\n\
         def make(label: str, x: int) -> P:\n    print(\"make\", label)\n    return P(x)\n\n\
         p = P(1)\nq = P(1)\nr = P(2)\n\
         print(p == q == P(1), p == q == r, p == q != r, p != q == r, p == r == q)\n\
         print(make(\"a\", 1) == make(\"b\", 2) == make(\"c\", 2))\n\
         print(make(\"d\", 1) != make(\"e\", 2) == make(\"f\", 2))\n",
        "True False True False False\nmake a\nmake b\nFalse\nmake d\nmake e\nmake f\nTrue\n",
    );
}

#[test]
fn a_walrus_in_operand_one_binds_and_is_usable_afterwards() {
    assert_prints(
        "def f(a: int, b: int) -> int:\n    n = 0\n\
         \x20   if 0 < (n := a + 1) < b:\n        return n * 10\n    return n\n\n\
         print(f(2, 5), f(6, 5), f(-1, 5))\n",
        "30 7 0\n",
    );
}

#[test]
fn a_raising_operand_propagates_while_an_earlier_bigint_operand_is_pending() {
    assert_prints(
        &format!(
            "def big(n: int) -> int:\n    return n + 1\n\n\
             def f(zero: int) -> None:\n    base = {PROMOTED}\n\
             \x20   try:\n        print(big(base) < (1 // zero) < 3)\n    except ZeroDivisionError:\n        print(\"caught\")\n\
             \x20   print(base)\n\nf(0)\n"
        ),
        "caught\n4611686018427387904\n",
    );
}

#[test]
fn no_later_operand_runs_after_a_raising_link() {
    // `small() <= big()` raises `OverflowError` and returns the Equal
    // sentinel; without the link guard `<=` would take its true edge and
    // evaluate `say("c", 1)` while the exception is pending.
    assert_prints(
        &format!(
            "def say(label: str, n: int) -> int:\n    print(\"eval\", label)\n    return n\n\n\
             def small(n: int) -> int:\n    return n\n\n\
             def big(n: int) -> int:\n    return n + 1\n\n\
             def f() -> None:\n    base = {PROMOTED}\n\
             \x20   try:\n        print(small(1) <= big(base) < say(\"c\", 1))\n    except OverflowError:\n        print(\"caught\")\n\
             \x20   try:\n        print(say(\"a\", 1) < 2 < base < say(\"d\", 1))\n    except OverflowError:\n        print(\"caught again\")\n\n\
             f()\n"
        ),
        "caught\neval a\ncaught again\n",
    );
}

#[test]
fn a_bad_link_is_refused_with_the_single_comparison_diagnostic() {
    let text = check_fails("def f(a: int) -> bool:\n    return 0 < a < \"z\"\n");
    assert!(
        text.contains("error[T0021]: cannot compare `int` and `str`"),
        "{text}"
    );
}

#[test]
fn a_walrus_in_a_later_operand_is_refused_at_its_location() {
    let text =
        check_fails("def f(a: int, b: int) -> None:\n    if 0 < a < (n := b):\n        print(n)\n");
    assert!(
        text.contains(
            "error[C0001]: a walrus assignment (`:=`) in a short-circuited chained-comparison operand is not supported"
        ),
        "{text}"
    );
    assert!(text.contains("subject.py:2:17"), "{text}");
}

#[test]
fn in_and_general_identity_links_keep_their_c0001() {
    let text = check_fails("def f(a: int, b: list[int]) -> bool:\n    return 0 < a not in b\n");
    assert!(
        text.contains("error[C0001]: comparison operator not supported yet: NotIn"),
        "{text}"
    );
    let text = check_fails("def f(a: int, b: int) -> bool:\n    return a < b is a\n");
    assert!(
        text.contains("error[C0001]: comparison operator not supported yet: Is"),
        "{text}"
    );
}

// ---------------------------------------------------------------------------
// Peak-RSS gates on the exception edge, the only edge a heap bigint can reach
// today (comparing one raises). They follow
// `tests/issue_638_bigint_exception_release.rs`: a caught-exception loop
// grows by itself on this tree, so each gate compares the leak shape's
// *marginal* growth between 250k and 500k trips with a control that raises
// the same exception at the same rate without the operand under test. The
// control reads its bigint by name, allocating nothing per trip; a call-
// based control (`lt(small(), big())`) would itself leak `big()` on this
// edge and mask a leaking chain. Measured on this change: a chain that
// skips the release of the middle operand across its link guard grows
// ~1.25x the control's marginal; a correct chain tracks it at ~1.00x.
// ---------------------------------------------------------------------------

#[cfg(unix)]
mod peak_rss {
    use super::{PROMOTED, ScratchDir, pycc};
    use std::process::Command;

    /// The child's `ru_maxrss`, only ever compared as a difference against
    /// a control measured the same way. Follows
    /// `tests/issue_146_bigint_release.rs`.
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
        assert!(libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 0);
        usage.ru_maxrss
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
        let mut run = Command::new(&bin);
        run.stdout(std::process::Stdio::null());
        peak_rss(run)
    }

    /// Peak-RSS growth of `repro` between `iterations` and `2 * iterations`
    /// trips.
    fn marginal_rss(case_prefix: &str, repro: impl Fn(u32) -> String, iterations: u32) -> i64 {
        let single = built_program_peak_rss(&format!("{case_prefix}_1x"), &repro(iterations));
        let double = built_program_peak_rss(&format!("{case_prefix}_2x"), &repro(iterations * 2));
        double - single
    }

    /// A loop whose body is `ok = <expression>` inside a `try` that catches
    /// `exception` on every trip.
    fn repro(expression: &str, exception: &str, iterations: u32) -> String {
        format!(
            "def small(n: int) -> int:\n    return n\n\n\n\
             def big(n: int) -> int:\n    return n + 1\n\n\n\
             x: int = {PROMOTED}\nz: int = 0\nok: bool = False\n\
             for i in range({iterations}):\n    try:\n        ok = {expression}\n\
             \x20   except {exception}:\n        z = 0\nprint(ok)\n"
        )
    }

    fn assert_tracks_control(case: &str, leak: &str, control: &str, exception: &str) {
        let leak_marginal = marginal_rss(
            &format!("rss_1212_{case}"),
            |n| repro(leak, exception, n),
            250_000,
        );
        let control_marginal = marginal_rss(
            &format!("rss_1212_{case}_ctl"),
            |n| repro(control, exception, n),
            250_000,
        );
        assert!(
            (leak_marginal as f64) < (control_marginal as f64) * 1.15,
            "`{leak}`'s marginal RSS growth must track the same-exception-rate \
             control `{control}`, not add a `BigIntObj` per trip: \
             leak_marginal={leak_marginal} control_marginal={control_marginal}"
        );
    }

    #[test]
    fn a_heap_middle_operand_is_released_on_a_raising_links_unwind_edge() {
        assert_tracks_control(
            "mid",
            "small(1) < big(x) < 3",
            "small(1) < x",
            "OverflowError",
        );
    }

    #[test]
    fn a_heap_first_operand_is_released_before_its_links_guard() {
        assert_tracks_control(
            "first",
            "big(x) < small(1) < 3",
            "small(1) < x",
            "OverflowError",
        );
    }

    #[test]
    fn a_pending_heap_operand_is_released_when_the_next_operand_raises() {
        assert_tracks_control(
            "zd",
            "big(x) < (1 // z) < 3",
            "small(1) < (1 // z) < 3",
            "ZeroDivisionError",
        );
    }
}
