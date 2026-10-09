//! End-to-end coverage for issue #150 (D-173's pending-exception mechanism
//! applied to `range()`'s zero-step guard): a zero-step `range()` must fail
//! through a stable, intentional `ValueError`, not an undocumented Rust
//! panic/abort crossing the `pycc_rt_range_continue` `extern "C"` boundary.
//!
//! Mirrors the helper functions and style of `tests/issue_382_exceptions.rs`,
//! the D-173 precedent this change follows (float division by zero ->
//! `ZeroDivisionError`).

use pycc_scratch::ScratchDir;
use std::io::Write;
use std::process::Command;

fn pycc_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_pycc"))
}

fn write_fixture(dir: &std::path::Path, name: &str, source: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(source.as_bytes()).unwrap();
    path
}

fn build_and_run(dir: &std::path::Path, src_name: &str, source: &str) -> (bool, Vec<u8>, String) {
    let src = write_fixture(dir, src_name, source);
    let out = dir.join(src_name.replace(".py", ""));
    let output = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .output()
        .unwrap();
    if !output.status.success() {
        return (
            false,
            Vec::new(),
            String::from_utf8_lossy(&output.stderr).to_string(),
        );
    }
    let run = Command::new(&out).output().unwrap();
    (
        run.status.success(),
        run.stdout.clone(),
        String::from_utf8_lossy(&run.stderr).to_string(),
    )
}

fn check_only(dir: &std::path::Path, src_name: &str, source: &str) -> (bool, String) {
    let src = write_fixture(dir, src_name, source);
    let output = Command::new(pycc_bin())
        .args(["check", src.to_str().unwrap()])
        .output()
        .unwrap();
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (output.status.success(), combined)
}

fn scratch_dir(tag: &str) -> ScratchDir {
    ScratchDir::new(&format!("150_{tag}")).expect("failed to create scratch dir")
}

// -- `check` accepts a zero-step range: no static literal-zero-divisor --
// -- diagnostic exists in this compiler (division by zero isn't caught  --
// -- at check time either), so this deliberately documents non-rejection --
// -- rather than a new compile-time check, per issue #150's "or" clause. --

#[test]
fn check_accepts_a_literal_zero_step_range() {
    let dir = scratch_dir("check_literal");
    let (ok, out) = check_only(
        &dir,
        "literal.py",
        "for i in range(0, 3, 0):\n    print(i)\n",
    );
    assert!(ok, "check should accept a literal zero step: {out}");
}

#[test]
fn check_accepts_a_computed_zero_step_range() {
    let dir = scratch_dir("check_computed");
    let (ok, out) = check_only(
        &dir,
        "computed.py",
        "n: int = 0\nfor i in range(0, 3, n):\n    print(i)\n",
    );
    assert!(ok, "check should accept a computed zero step: {out}");
}

// -- An uncaught top-level zero-step range fails cleanly, not with a --
// -- panic/backtrace crossing the `pycc_rt_range_continue` extern "C" --
// -- boundary. --

#[test]
fn uncaught_literal_zero_step_range_exits_cleanly_with_value_error() {
    let dir = scratch_dir("uncaught_literal");
    let (ok, _out, err) = build_and_run(
        &dir,
        "uncaught_literal.py",
        "for i in range(0, 3, 0):\n    print(i)\n",
    );
    assert!(
        !ok,
        "expected non-zero exit for an uncaught zero-step range"
    );
    assert!(
        err.contains("ValueError: range() arg 3 must not be zero"),
        "stderr should report a clean ValueError, not a panic: {err}"
    );
    assert!(
        !err.contains("panicked") && !err.contains("stack backtrace"),
        "stderr must not leak a Rust panic/backtrace: {err}"
    );
}

#[test]
fn uncaught_computed_zero_step_range_exits_cleanly_with_value_error() {
    // A computed (non-literal) step exercises the runtime path even if a
    // future static check ever special-cased a literal `0`.
    let dir = scratch_dir("uncaught_computed");
    let (ok, _out, err) = build_and_run(
        &dir,
        "uncaught_computed.py",
        "n: int = 0\nfor i in range(0, 3, n):\n    print(i)\n",
    );
    assert!(
        !ok,
        "expected non-zero exit for an uncaught zero-step range"
    );
    assert!(
        err.contains("ValueError: range() arg 3 must not be zero"),
        "stderr should report a clean ValueError, not a panic: {err}"
    );
    assert!(
        !err.contains("panicked") && !err.contains("stack backtrace"),
        "stderr must not leak a Rust panic/backtrace: {err}"
    );
}

// -- A zero-step range is catchable as an ordinary `ValueError`. --

#[test]
fn computed_zero_step_range_is_caught_in_a_try_except() {
    let dir = scratch_dir("caught_computed");
    let (ok, out, err) = build_and_run(
        &dir,
        "caught_computed.py",
        "n: int = 0\ntry:\n    for i in range(0, 3, n):\n        print(i)\nexcept ValueError:\n    print(\"caught zero step\")\n",
    );
    assert!(ok, "build/run failed: {err}");
    assert_eq!(out, b"caught zero step\n");
}

#[test]
fn literal_zero_step_range_is_caught_in_a_try_except() {
    let dir = scratch_dir("caught_literal");
    let (ok, out, err) = build_and_run(
        &dir,
        "caught_literal.py",
        "try:\n    for i in range(0, 3, 0):\n        print(i)\nexcept ValueError:\n    print(\"caught zero step\")\n",
    );
    assert!(ok, "build/run failed: {err}");
    assert_eq!(out, b"caught zero step\n");
}

// -- A promoted (bigint) zero step is caught the same way -- the general --
// -- `encoded_int_cmp` path in `range_continue`, not only the inline --
// -- fast path. --

#[test]
fn bigint_zero_step_range_is_caught_in_a_try_except() {
    let dir = scratch_dir("caught_bigint");
    // `PROMOTED - PROMOTED` on a value outside the tagged smallint range
    // forces the subtraction result through the bigint path, and its
    // numeric value is zero even though its encoded word is a non-zero
    // heap pointer -- the exact shape `range_continue`'s general-path guard
    // must reject on value, not on word (see RUNTIME.md).
    const PROMOTED: &str = "4611686018427387904"; // 2^62, outside the tagged range (issue #147/#146's own constant)
    let (ok, out, err) = build_and_run(
        &dir,
        "caught_bigint.py",
        &format!(
            "b: int = {PROMOTED}\nn: int = b - b\ntry:\n    for i in range(0, 3, n):\n        print(i)\nexcept ValueError:\n    print(\"caught zero step\")\n"
        ),
    );
    assert!(ok, "build/run failed: {err}");
    assert_eq!(out, b"caught zero step\n");
}

// -- A zero step raised from inside a plain function body (no enclosing --
// -- `try`) is observed only at the next enclosing checkpoint, not --
// -- immediately -- the same D-173/`float_div` scope characteristic --
// -- documented in `crates/pycc_rt/src/lib.rs` above `range_continue`. This --
// -- pins the exact observed shape so a later change cannot silently widen --
// -- or narrow that scope without updating this test. --

#[test]
fn uncaught_zero_step_range_inside_a_function_body_is_observed_at_the_next_checkpoint() {
    let dir = scratch_dir("function_body_boundary");
    let (ok, out, err) = build_and_run(
        &dir,
        "function_body_boundary.py",
        "def f(n: int) -> None:\n    for i in range(0, 3, n):\n        print(i)\n    print(\"after loop\")\n\nf(0)\nprint(\"after call\")\n",
    );
    assert!(
        !ok,
        "expected non-zero exit for an uncaught zero-step range"
    );
    assert_eq!(
        out, b"after loop\n",
        "the statement after the loop, still inside f's own body with no \
         enclosing try, must run to completion before the pending exception \
         is observed at the next checkpoint"
    );
    assert!(
        err.contains("ValueError: range() arg 3 must not be zero"),
        "stderr should report a clean ValueError, not a panic: {err}"
    );
    assert!(
        !err.contains("panicked") && !err.contains("stack backtrace"),
        "stderr must not leak a Rust panic/backtrace: {err}"
    );
}

// -- Existing positive- and negative-step iteration behavior is preserved. --

#[test]
fn positive_and_negative_step_iteration_still_work() {
    let dir = scratch_dir("regression");
    let (ok, out, err) = build_and_run(
        &dir,
        "regression.py",
        "for i in range(0, 3, 1):\n    print(i)\nfor i in range(3, 0, -1):\n    print(i)\n",
    );
    assert!(ok, "build/run failed: {err}");
    assert_eq!(out, b"0\n1\n2\n3\n2\n1\n");
}

// -- The runtime path is also reachable through a comprehension, one of --
// -- the three `CompLoopTail::Range` call sites distinct from the plain --
// -- `for` statement's own `MirStmt::ForRange` call site. --

#[test]
fn computed_zero_step_range_in_a_list_comprehension_is_caught() {
    let dir = scratch_dir("caught_listcomp");
    let (ok, out, err) = build_and_run(
        &dir,
        "caught_listcomp.py",
        "n: int = 0\ntry:\n    xs = [i for i in range(0, 3, n)]\n    print(len(xs))\nexcept ValueError:\n    print(\"caught zero step\")\n",
    );
    assert!(ok, "build/run failed: {err}");
    assert_eq!(out, b"caught zero step\n");
}
