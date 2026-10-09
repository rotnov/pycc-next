//! Issue #147 (D-179): public-CLI coverage for `range()` over values outside
//! D-061's tagged 63-bit range.
//!
//! Two families live here, both deliberately kept out of
//! `tests/fixtures/bigint_range.py`'s differential fixture:
//!
//! * **Divergences.** A bigint-valued *zero* step is a `ValueError` in
//!   CPython, and -- since issue #150 extended D-173's pending-exception
//!   mechanism to `range` -- a catchable `ValueError` in pycc too. It still
//!   does not belong in the differential fixture (the fixture's own harness
//!   does not model catching a Python exception), but it is no longer an
//!   abort; see `tests/issue_150_zero_step_range.rs` for the broader D-173
//!   coverage of that mechanism.
//! * **Positions that still abort.** #147 moves `range` operands off D-141's
//!   runtime `int` boundary; it does not move container ingress. A
//!   comprehension whose *element* is the bigint loop variable still aborts,
//!   and that boundary is pinned here so a later change cannot silently
//!   widen the fix's scope without updating this file.

use pycc_scratch::ScratchDir;
use std::io::Write;
use std::process::Command;

/// `2^62`, the smallest magnitude that does not round-trip through the
/// tagged 63-bit encoding.
const PROMOTED: &str = "4611686018427387904";

fn pycc_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_pycc"))
}

/// Writes `source` into a fresh scratch directory and returns the directory
/// handle together with the source path. The caller must keep the handle
/// alive while the path is in use -- dropping it removes the directory.
fn write_case(case: &str, source: &str) -> (ScratchDir, std::path::PathBuf) {
    let dir = ScratchDir::new(&format!("issue147_{case}")).expect("failed to create scratch dir");
    let src = dir.join("case.py");
    std::fs::File::create(&src)
        .unwrap()
        .write_all(source.as_bytes())
        .unwrap();
    (dir, src)
}

/// Runs `source` through `pycc run` and asserts it exits `0` with exactly
/// `expected` on stdout.
fn assert_runs_and_prints(case: &str, source: &str, expected: &str) {
    let (_dir, src) = write_case(case, source);
    let run = Command::new(pycc_bin())
        .args(["run", src.to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(
        run.status.code(),
        Some(0),
        "{case} should run to completion, got {:?}: {}",
        run.status.code(),
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&run.stdout),
        expected,
        "{case} printed the wrong output"
    );
}

/// Asserts an uncaught, top-level `ValueError` (D-173's pending-exception
/// mechanism, since #150 also covering `range`'s zero-step guard).
///
/// `pycc run` maps *every* unsuccessful child termination to CLI_SPEC.md's
/// stable exit `101` (see `run()` in `src/main.rs`), so this still expects
/// `101` -- unlike `tests/issue_150_zero_step_range.rs`'s own
/// `build_and_run` helper, which builds and execs the binary directly and so
/// observes the underlying process's real exit `1`. What distinguishes this
/// case from `assert_runtime_abort` below is the *message*, not the exit
/// code: no panic/backtrace text may leak across the
/// `pycc_rt_range_continue` `extern "C"` boundary.
fn assert_clean_value_error(case: &str, source: &str, message: &str) {
    let (_dir, src) = write_case(case, source);
    let run = Command::new(pycc_bin())
        .args(["run", src.to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(
        run.status.code(),
        Some(101),
        "{case} should hit the driver's mapped exit-101 boundary, got {:?}: {}",
        run.status.code(),
        String::from_utf8_lossy(&run.stderr)
    );
    let stderr = String::from_utf8_lossy(&run.stderr);
    assert!(
        stderr.contains(message),
        "{case} should report {message:?}, got: {stderr}"
    );
    assert!(
        !stderr.contains("panicked") && !stderr.contains("stack backtrace"),
        "{case} must not leak a Rust panic/backtrace: {stderr}"
    );
}

/// Asserts the documented D-072 exit-`101` boundary carrying `message`.
/// `pycc run` (rather than `pycc build` plus a direct exec) is deliberate:
/// `pycc_rt`'s panic unwinds across an `extern "C"` boundary and becomes a
/// non-unwinding process abort, so the raw child reports no exit code at
/// all. The `101` is the driver's own mapping of that abort.
fn assert_runtime_abort(case: &str, source: &str, message: &str) {
    let (_dir, src) = write_case(case, source);
    let run = Command::new(pycc_bin())
        .args(["run", src.to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(
        run.status.code(),
        Some(101),
        "{case} should hit the documented exit-101 boundary, got {:?}: {}",
        run.status.code(),
        String::from_utf8_lossy(&run.stderr)
    );
    let stderr = String::from_utf8_lossy(&run.stderr);
    assert!(
        stderr.contains(message),
        "{case} should report {message:?}, got: {stderr}"
    );
}

// ---------------------------------------------------------------------------
// Completion criteria 1-2: bigint bounds, steps, and induction variables.
// ---------------------------------------------------------------------------

#[test]
fn a_range_whose_bounds_are_bigints_iterates_instead_of_aborting() {
    assert_runs_and_prints(
        "bigint_bounds",
        &format!("for i in range({PROMOTED}, 4611686018427387906):\n    print(i)\n"),
        "4611686018427387904\n4611686018427387905\n",
    );
}

#[test]
fn a_range_whose_step_is_a_bigint_iterates_instead_of_aborting() {
    assert_runs_and_prints(
        "bigint_step",
        &format!("for i in range(0, 9223372036854775807, {PROMOTED}):\n    print(i)\n"),
        "0\n4611686018427387904\n",
    );
}

#[test]
fn an_induction_variable_that_promotes_mid_loop_still_stops_at_stop() {
    // The shape completion criterion 1 names: `start`, `stop`, and `step`
    // all stay smallints, but the third `int_add` produces 2^62, which no
    // longer fits the tagged encoding. `range_continue` therefore compares a
    // *bigint* `i` against a *smallint* `stop` exactly once, and must
    // report "stop" rather than aborting.
    assert_runs_and_prints(
        "promote_mid_loop",
        "for i in range(4611686018427387900, 4611686018427387903, 2):\n    print(i)\n",
        "4611686018427387900\n4611686018427387902\n",
    );
}

#[test]
fn an_empty_bigint_range_runs_its_body_zero_times() {
    assert_runs_and_prints(
        "empty_bigint_range",
        &format!(
            "for i in range({PROMOTED}, {PROMOTED}):\n    print(\"unreachable\")\nprint(\"done\")\n"
        ),
        "done\n",
    );
}

// ---------------------------------------------------------------------------
// Divergence: a bigint-valued zero step.
// ---------------------------------------------------------------------------

#[test]
fn a_bigint_valued_zero_step_still_reports_the_zero_step_boundary() {
    // `z` is a *bigint-tagged* zero: `bigint_add_signed`'s equal-magnitude,
    // opposite-sign case normalizes to `negative: false, limbs: [0]`, so its
    // encoded word is a non-zero pointer whose numeric value is zero. The
    // guard must therefore compare values, not words -- reading the sign
    // flag first would treat it as a positive step and loop forever.
    //
    // CPython raises `ValueError: range() arg 3 must not be zero`; since
    // #150, pycc reports the same message the same way -- a catchable
    // `ValueError`, not D-072's exit-`101` abort boundary -- which is why
    // this case is pinned here (an *uncaught* top-level occurrence) rather
    // than in the differential fixture (which cannot model catching).
    assert_clean_value_error(
        "bigint_zero_step",
        &format!("z = {PROMOTED} - {PROMOTED}\nfor i in range(0, 10, z):\n    print(i)\n"),
        "ValueError: range() arg 3 must not be zero",
    );
}

// ---------------------------------------------------------------------------
// Still-aborting position: container ingress is deliberately out of scope.
// ---------------------------------------------------------------------------

#[test]
fn a_comprehension_collecting_a_bigint_loop_variable_still_hits_the_container_boundary() {
    // `listcomp_validate_elt`, not `range_untag_operand`: #147 removed the
    // *operand* guard, and D-141's container-value guard (issue #618) stays.
    assert_runtime_abort(
        "listcomp_bigint_element",
        &format!("x = [i for i in range({PROMOTED}, 4611686018427387906)]\nprint(len(x))\n"),
        "pycc_rt: int boundary does not support bigint-valued values yet",
    );
}

#[test]
fn a_comprehension_over_a_bigint_range_with_a_smallint_element_now_succeeds() {
    // The same three emitters, proving the normalizer reached all of them.
    assert_runs_and_prints(
        "comprehensions_over_bigint_range",
        &format!(
            "a = [0 for i in range({PROMOTED}, 4611686018427387906)]\n\
             b = {{0 for i in range({PROMOTED}, 4611686018427387906)}}\n\
             c = {{\"k\": 0 for i in range({PROMOTED}, 4611686018427387906)}}\n\
             print(len(a))\nprint(len(b))\nprint(len(c))\n"
        ),
        "2\n1\n1\n",
    );
}
