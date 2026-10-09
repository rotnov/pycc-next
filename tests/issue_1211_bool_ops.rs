//! End-to-end proof for `and`/`or` ([#1211], Part 3 of [#1018]).
//!
//! `docs/TYPE_SYSTEM.md`'s "`and` and `or`" section is the
//! contract. The byte-exact oracle fixture is `tests/fixtures/bool_ops.py`
//! (`tests/conformance/numeric.rs`, oracle-gated). This file owns what runs
//! without the oracle: the join of every value type, the short circuit, a
//! raising right operand, the bigint ownership shapes (the plan's S4) and a
//! peak-RSS gate for a missed release, plus the refusals.
//!
//! [#1211]: https://github.com/rotnov/pycc/issues/1211
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
    let dir = ScratchDir::new("e2e_1211_check").expect("scratch");
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

/// Builds `source` (optionally with `--release`), runs it, and asserts it
/// exits 0 printing exactly `expected`.
fn assert_prints(source: &str, expected: &str) {
    for release in [false, true] {
        let dir = ScratchDir::new("e2e_1211_run").expect("scratch");
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
fn every_value_type_joins_and_selects_the_right_operand() {
    assert_prints(
        "class Box:\n    def __init__(self, n: int) -> None:\n        self.n = n\n\n\
         def maybe(n: int) -> int | None:\n    r: int | None = None\n    if n >= 0:\n        r = n\n    return r\n\n\
         def maybe_float(x: float, present: bool) -> float | None:\n    r: float | None = None\n    if present:\n        r = x\n    return r\n\n\
         def maybe_bool(b: bool, present: bool) -> bool | None:\n    r: bool | None = None\n    if present:\n        r = b\n    return r\n\n\
         def f(n: int, z: int, s: str, e: str, x: float, t: bool) -> None:\n\
         \x20   print(n or z, z or n, n and z, z and n)\n\
         \x20   print(s or e, e or s, s and e, e and s)\n\
         \x20   print(x or 2.5, 0.0 or x, x and 0.0)\n\
         \x20   print(t or False, False or t, t and False)\n\
         \x20   print(True or 0, 0 or True, t and n)\n\
         \x20   b = Box(1)\n    c = Box(2)\n    print((b or c).n, (b and c).n)\n\
         \x20   print(maybe(-1) or 11, maybe(0) or 12, maybe(5) or 13)\n\
         \x20   print(maybe_float(0.5, True) or 1.5, maybe_float(0.5, False) or 1.5)\n\
         \x20   print(maybe_bool(True, True) or False, maybe_bool(True, False) or False)\n\
         \x20   r = maybe(0) and 1\n    if r is not None:\n        print(\"kept\", r)\n\
         \x20   q = 7 and maybe(4)\n    if q is not None:\n        print(\"opt\", q)\n\n\
         f(3, 0, \"s\", \"\", 1.5, True)\n",
        "3 3 0 0\ns s  \n1.5 1.5 0.0\nTrue True False\nTrue True 3\n1 2\n11 12 5\n0.5 1.5\nTrue False\nkept 0\nopt 4\n",
    );
}

#[test]
fn operands_short_circuit_and_are_evaluated_once() {
    assert_prints(
        "def say(label: str, n: int) -> int:\n    print(\"eval\", label)\n    return n\n\n\
         print(say(\"a\", 0) or say(\"b\", 5))\n\
         print(say(\"c\", 3) or say(\"d\", 5))\n\
         print(say(\"e\", 0) and say(\"f\", 5))\n\
         print(say(\"g\", 1) and say(\"h\", 2) and say(\"i\", 0))\n\
         if say(\"j\", 0) or say(\"k\", 0):\n    print(\"no\")\n\
         else:\n    print(\"falsy\")\n",
        "eval a\neval b\n5\neval c\n3\neval e\n0\neval g\neval h\neval i\n0\neval j\neval k\nfalsy\n",
    );
}

#[test]
fn truth_context_tests_mixed_operand_types() {
    assert_prints(
        "def f(n: int, s: str, x: float, o: int | None) -> None:\n\
         \x20   if n and s:\n        print(\"both\")\n\
         \x20   elif x or o:\n        print(\"either\")\n\
         \x20   else:\n        print(\"neither\")\n\
         \x20   print(not (n or s), not (n and s))\n\
         \x20   count = 2\n    while count and s:\n        count = count - 1\n    print(count)\n\n\
         f(0, \"s\", 0.0, None)\nf(5, \"\", 0.0, 3)\nf(5, \"s\", 1.0, None)\n",
        "neither\nFalse True\n0\neither\nFalse True\n2\nboth\nFalse False\n0\n",
    );
}

#[test]
fn a_raising_right_operand_propagates_to_its_handler() {
    assert_prints(
        "def divide(a: int, b: int) -> int:\n    return a // b\n\n\
         def f(n: int, z: int) -> None:\n\
         \x20   try:\n        print(z or divide(n, z))\n    except ZeroDivisionError:\n        print(\"caught call\")\n\
         \x20   try:\n        print(n and n // z)\n    except ZeroDivisionError:\n        print(\"caught floor\")\n\
         \x20   print(n or n // z)\n\n\
         f(8, 0)\n",
        "caught call\ncaught floor\n8\n",
    );
}

#[test]
fn bigint_operands_keep_their_values_under_release() {
    assert_prints(
        &format!(
            "def f() -> None:\n\
             \x20   big = {PROMOTED}\n    small = 3\n    zero = 0\n\
             \x20   print(big or small, zero or big, small and big, big and small)\n\
             \x20   x = big or small\n    y = zero or big\n    big = 5\n    print(x, y, big)\n\
             \x20   other = {PROMOTED} + 1\n    z = other or other\n    w = other and other\n    other = 1\n    print(z, w, other)\n\n\
             class Holder:\n    def __init__(self, n: int) -> None:\n        self.n = n\n\n\
             def g() -> None:\n    h = Holder({PROMOTED} + 1)\n    v = h.n or 0\n    h.n = 2\n    print(v, h.n)\n\n\
             def loop(n: int) -> None:\n    base = {PROMOTED}\n    total = 0\n\
             \x20   for i in range(n):\n        v = (base + i) or 1\n        total = total + (v - base)\n    print(total)\n\n\
             def raising(n: int, z: int) -> None:\n    big = {PROMOTED}\n\
             \x20   try:\n        print(big and n // z)\n    except ZeroDivisionError:\n        print(\"caught\")\n    print(big)\n\n\
             f()\ng()\nloop(10)\nraising(1, 0)\n"
        ),
        "4611686018427387904 4611686018427387904 4611686018427387904 3\n\
         4611686018427387904 4611686018427387904 5\n\
         4611686018427387905 4611686018427387905 1\n\
         4611686018427387905 2\n45\ncaught\n4611686018427387904\n",
    );
}

#[test]
fn an_optional_bigint_payload_selected_by_or_is_retained() {
    assert_prints(
        &format!(
            "def f() -> None:\n    big = {PROMOTED}\n    o: int | None = big + 7\n\
             \x20   r = o or 1\n    o = None\n    print(r)\n    s = o or big\n    print(s)\n\nf()\n"
        ),
        "4611686018427387911\n4611686018427387904\n",
    );
}

#[test]
fn value_joins_with_no_common_type_are_refused() {
    let text = check_fails("def f(a: int, b: str) -> None:\n    print(a or b)\n");
    assert!(
        text.contains("error[T0021]: `or` operands have no common type: int and str"),
        "{text}"
    );
}

#[test]
fn a_walrus_in_a_short_circuited_operand_is_refused_at_its_location() {
    let text =
        check_fails("def f(a: int, b: int) -> None:\n    if a or (n := b):\n        print(n)\n");
    assert!(
        text.contains(
            "error[C0001]: a walrus assignment (`:=`) in a short-circuited `and`/`or` operand is not supported"
        ),
        "{text}"
    );
    assert!(text.contains("subject.py:2:14"), "{text}");
}

#[cfg(unix)]
mod peak_rss {
    use super::{PROMOTED, ScratchDir, pycc};
    use std::process::Command;

    /// The child's `ru_maxrss` (bytes on macOS, kilobytes on Linux, so only
    /// ever compared as a ratio). Follows `tests/issue_146_bigint_release.rs`.
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

    /// Every `int`-typed operand shape a loop can strand a word through: a
    /// discarded truthy left under `and`, a selected owned left under `or`
    /// that a later iteration overwrites, and a truth-context operand.
    fn repro(iterations: u32) -> String {
        format!(
            "base = {PROMOTED}\ny = 0\nz = 0\nhits = 0\n\
             for i in range({iterations}):\n\
             \x20   y = (base + i) and 1\n\
             \x20   z = (base + i) or 1\n\
             \x20   if (base + i) and y:\n        hits = hits + 1\n\
             print(y, hits)\n"
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
    fn discarded_and_selected_bigint_operands_do_not_grow_with_the_trip_count() {
        let single = built_program_peak_rss("rss_1211_1x", &repro(500_000));
        let double = built_program_peak_rss("rss_1211_2x", &repro(1_000_000));
        let ratio = double as f64 / single as f64;
        assert!(
            ratio < 1.35,
            "peak RSS must not scale with the loop trip count: \
             500k iterations={single} 1M iterations={double} ratio={ratio:.4} \
             (a per-iteration `BigIntObj` leak reads as ratio ~2.0)"
        );
    }
}
