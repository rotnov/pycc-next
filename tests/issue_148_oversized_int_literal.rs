//! Issue #148 (D-178): public-CLI coverage for `int` literals outside D-061's
//! tagged 63-bit range.
//!
//! Two families live here, both deliberately kept out of
//! `tests/fixtures/oversized_int_literal.py`'s differential fixture:
//!
//! * **D-141 runtime `int`-boundary positions.** Making an out-of-range
//!   literal compile at all (D-178) meant it could flow into the D-141
//!   runtime `int` boundaries that the retired codegen panic used to shadow.
//!   [#618](https://github.com/rotnov/pycc/issues/618) (`T0051`, D-207) later
//!   closed that gap for the literal case specifically at every position
//!   that resolves syntactically during HIR lowering: a literal at one of
//!   them is now rejected by `pycc check` itself, not merely by a run-time
//!   abort, and the cases below were rewritten from "accepted, aborts at run
//!   time" to "rejected at check time" to match. `list.append()`, a
//!   container-literal element, `set.add()`, a comprehension element, and a
//!   list index are all `T0051` now; `str * int` repeat count is `T0051`
//!   only when the string side is itself a literal (`"ab" * <huge int>`), a
//!   `str`-typed *variable* operand still hits the runtime abort pinned
//!   below (D-207's documented, narrower scope). The one member of this
//!   family that stays an accepted run-time success rather than a boundary
//!   failure at all, the `range` operand, left the inventory in #147
//!   (D-179) and was never a `T0051` candidate; see
//!   `tests/issue_147_bigint_range.rs` for its own success-path coverage.
//! * **Still-open bigint operations.** `*`, `//`, `%`, `**`, `/`, `int`->`float`
//!   conversion, and comparison remain accepted failure boundaries; before
//!   #148 they were unreachable from a literal, so their current behavior is
//!   pinned here too. None of these are D-141 boundary *positions* (they are
//!   ordinary arithmetic/comparison operators), so #618 does not touch them.
//!   Part C of #1038
//!   ([#1065](https://github.com/rotnov/pycc/issues/1065)) changed *how* they
//!   fail, not *that* they fail: each now raises a catchable `OverflowError`
//!   (D-173) instead of aborting the process, so the seven cases below moved
//!   from `assert_runtime_abort` to `assert_uncaught_overflow_error`.

use pycc_scratch::ScratchDir;
use std::io::Write;
use std::process::Command;

/// The one out-of-range literal every case below uses: `2^62`, the smallest
/// magnitude that does not round-trip through the tagged 63-bit encoding.
const OVERSIZED: &str = "4611686018427387904";

const BOUNDARY_MESSAGE: &str = "pycc_rt: int boundary does not support bigint-valued values yet";

fn pycc_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_pycc"))
}

/// Compiles and runs `source` through `pycc run` and asserts the documented
/// D-072 exit `101` boundary carrying `message` on stderr.
///
/// `pycc run` (rather than `pycc build` plus a direct exec) is deliberate:
/// `pycc_rt`'s panic unwinds across an `extern "C"` boundary and becomes a
/// non-unwinding process abort, so the raw child is killed by a signal and
/// reports no exit code at all. The `101` in `docs/CLI_SPEC.md`'s boundary
/// list is the driver's own mapping of that abort -- and, as
/// `assert_uncaught_overflow_error` below relies on, of any other non-zero
/// child exit.
fn assert_runtime_abort(case: &str, source: &str, message: &str) {
    let dir = ScratchDir::new(&format!("issue148_{case}")).expect("failed to create scratch dir");
    let src = dir.join("case.py");
    std::fs::File::create(&src)
        .unwrap()
        .write_all(source.as_bytes())
        .unwrap();

    // The literal itself must still *compile* -- that is what #148 changed.
    let check = Command::new(pycc_bin())
        .args(["check", src.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        check.status.success(),
        "pycc check should accept {case}: {}",
        String::from_utf8_lossy(&check.stderr)
    );

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

/// Part C of #1038 (#1065): the same `pycc run` exit `101`, but reached by an
/// uncaught D-173 `OverflowError` rather than by a process abort. The class
/// name is asserted alongside the message precisely because the exit code no
/// longer distinguishes the two -- `pycc run` reports `101` for *any*
/// non-zero child exit, and an uncaught raise exits `1`.
///
/// The message must not carry the retired `pycc_rt: ` panic prefix: a raised
/// exception's message is user-facing Python text.
fn assert_uncaught_overflow_error(case: &str, source: &str, message: &str) {
    assert_runtime_abort(case, source, &format!("OverflowError: {message}"));
}

// ---------------------------------------------------------------------------
// D-141 runtime `int` boundaries -- now caught at compile time by #618
// (T0051, D-207) for every position that resolves syntactically in
// `pycc_hir`.
// ---------------------------------------------------------------------------

/// Compiles `source` through `pycc check` and asserts it is rejected with a
/// spanned `T0051` diagnostic and a non-zero exit -- the #618 (D-207)
/// compile-time catch point that replaced the run-time
/// `pycc_rt_int_untag_checked` abort these same fixtures used to hit before
/// #618. `tests/int_literal_boundary_check.rs` covers the complementary
/// end-to-end `check`+`build` shape and the still-unaffected arithmetic
/// case; this helper stays scoped to `check`'s own exit code and message,
/// mirroring `assert_runtime_abort` above.
fn assert_compile_time_boundary_rejection(case: &str, source: &str) {
    let dir = ScratchDir::new(&format!("issue148_{case}")).expect("failed to create scratch dir");
    let src = dir.join("case.py");
    std::fs::File::create(&src)
        .unwrap()
        .write_all(source.as_bytes())
        .unwrap();

    let check = Command::new(pycc_bin())
        .args(["check", src.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        !check.status.success(),
        "pycc check should reject {case} at compile time (#618/T0051)"
    );
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&check.stdout),
        String::from_utf8_lossy(&check.stderr)
    );
    assert!(
        text.contains("T0051"),
        "{case} should report T0051, got: {text}"
    );
}

#[test]
fn an_oversized_literal_as_a_container_value_is_rejected_at_compile_time() {
    // `list_validate_element` -- the container-value guard family.
    assert_compile_time_boundary_rejection(
        "list_element",
        &format!("x = [{OVERSIZED}]\nprint(len(x))\n"),
    );
}

#[test]
fn an_oversized_literal_appended_to_a_list_is_rejected_at_compile_time() {
    assert_compile_time_boundary_rejection(
        "list_append",
        &format!("x = [1]\nx.append({OVERSIZED})\nprint(len(x))\n"),
    );
}

#[test]
fn an_oversized_literal_added_to_a_set_is_rejected_at_compile_time() {
    assert_compile_time_boundary_rejection(
        "set_add",
        &format!("x = {{1}}\nx.add({OVERSIZED})\nprint(len(x))\n"),
    );
}

#[test]
fn an_oversized_literal_as_a_comprehension_element_is_rejected_at_compile_time() {
    assert_compile_time_boundary_rejection(
        "listcomp_element",
        &format!("x = [{OVERSIZED} for i in range(2)]\nprint(len(x))\n"),
    );
}

#[test]
fn an_oversized_literal_as_a_list_index_is_rejected_at_compile_time() {
    // `list_untag_index` -- the index/count guard family. CPython raises
    // `IndexError` here, so this remains a divergence in kind from CPython,
    // just caught one phase earlier than before #618.
    assert_compile_time_boundary_rejection(
        "list_index",
        &format!("x = [1, 2, 3]\nprint(x[{OVERSIZED}])\n"),
    );
}

#[test]
fn an_oversized_literal_as_a_str_literal_repeat_count_is_rejected_at_compile_time() {
    assert_compile_time_boundary_rejection(
        "str_repeat_count",
        &format!("print(\"ab\" * {OVERSIZED})\n"),
    );
}

#[test]
fn an_oversized_literal_as_a_str_variable_repeat_count_still_hits_the_runtime_int_boundary() {
    // D-207's deliberately narrowed 13th position: `pycc_hir` has no type
    // information at lowering time, so it cannot tell a `str`-typed
    // *variable* operand from any other operand here. Unlike the string
    // *literal* case just above, this one still compiles and hits the same
    // run-time abort D-178 always described -- an accepted, documented gap.
    assert_runtime_abort(
        "str_repeat_count_variable",
        &format!("s = \"ab\"\nprint(s * {OVERSIZED})\n"),
        BOUNDARY_MESSAGE,
    );
}

#[test]
fn an_oversized_literal_as_a_range_argument_no_longer_hits_the_runtime_int_boundary() {
    // #147 (D-179) removed `range_untag_operand`: `range` operands are now
    // normalized rather than decoded, so a bigint bound drives the loop.
    //
    // A bounded stop is essential. The naive spelling this test used to
    // carry -- `range(4611686018427387904)` -- now *succeeds*, and would
    // run ~4.6e18 iterations. The pair below crosses exactly the same
    // representation boundary in two.
    //
    // The success paths themselves live in
    // `tests/issue_147_bigint_range.rs`; this case stays here so the #148
    // boundary inventory records which position left the list and why.
    let dir = ScratchDir::new("issue148_range").expect("failed to create scratch dir");
    let src = dir.join("range.py");
    std::fs::File::create(&src)
        .unwrap()
        .write_all(
            format!("for i in range({OVERSIZED}, 4611686018427387906):\n    print(i)\n").as_bytes(),
        )
        .unwrap();

    let run = Command::new(pycc_bin())
        .args(["run", src.to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(
        run.status.code(),
        Some(0),
        "a bigint range should run to completion since #147, got {:?}: {}",
        run.status.code(),
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&run.stdout),
        "4611686018427387904\n4611686018427387905\n"
    );
}

// ---------------------------------------------------------------------------
// Still-open bigint operations (out of scope for #148, pinned here).
// ---------------------------------------------------------------------------

#[test]
fn bigint_operations_unreachable_before_issue_148_keep_their_accepted_boundaries() {
    for (case, expr, message) in [
        (
            "compare",
            format!("{OVERSIZED} > 0"),
            "comparing a bigint-valued `int` is not supported yet",
        ),
        (
            "multiply",
            format!("{OVERSIZED} * 2"),
            "multiplying a bigint-valued `int` is not supported yet",
        ),
        (
            "floordiv",
            format!("{OVERSIZED} // 2"),
            "dividing a bigint-valued `int` is not supported yet",
        ),
        (
            "modulo",
            format!("{OVERSIZED} % 2"),
            "computing the modulo of a bigint-valued `int` is not supported yet",
        ),
        (
            "power",
            format!("{OVERSIZED} ** 2"),
            "exponentiating a bigint-valued `int` is not supported yet",
        ),
        (
            "truediv",
            format!("{OVERSIZED} / 2"),
            "converting a bigint-valued `int` is not supported yet",
        ),
        (
            "mixed_float",
            format!("{OVERSIZED} + 1.5"),
            "converting a bigint-valued `int` is not supported yet",
        ),
    ] {
        assert_uncaught_overflow_error(case, &format!("print({expr})\n"), message);
    }
}

// ---------------------------------------------------------------------------
// Completion criterion 3: a literal outside `i64` range stays a compile-time
// capability diagnostic, not a runtime abort.
// ---------------------------------------------------------------------------

#[test]
fn a_literal_outside_i64_range_is_still_a_spanned_capability_diagnostic() {
    let dir = ScratchDir::new("issue148_beyond_i64").expect("failed to create scratch dir");
    let src = dir.join("beyond.py");
    std::fs::File::create(&src)
        .unwrap()
        .write_all(b"x: int = 99999999999999999999999999\nprint(x)\n")
        .unwrap();

    let check = Command::new(pycc_bin())
        .args(["check", src.to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(
        check.status.code(),
        Some(1),
        "a literal beyond i64 range should still be rejected at check time"
    );
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&check.stdout),
        String::from_utf8_lossy(&check.stderr)
    );
    assert!(
        text.contains("error[C0001]") && text.contains("integer literal does not fit in i64"),
        "expected the spanned C0001 capability diagnostic, got: {text}"
    );
}
