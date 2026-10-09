//! Issue #604 (Part 3 of #573): public-CLI differential coverage for `not x`
//! and `~x`.
//!
//! Unlike #603's `-x`/`+x`, neither operator has a literal-folding arm: `not
//! 5` and `~5` are not part of Python's numeric-literal grammar the way a
//! source-level `-5` is, so both always lower into the same `HirExpr::UnaryOp`
//! node regardless of the operand's own shape.
//!
//! `not x` is defined by truthiness, not numeric promotion, so its coverage
//! spans every operand type this compiler can compute a truth value for
//! (`bool`, `int`, `float`, `str`, `Optional`), including each type's own
//! falsy/zero/empty case -- the exact case CPython's truthiness rule exists
//! to get right. `~x` is `int -> int` only (`bool` included, as a numeric
//! subtype), so its own coverage instead spans the smallint/bigint boundary
//! (`pycc_rt`'s tagged-fixnum representation switches to a heap bigint past
//! `i64::MAX >> 1`).
//!
//! Every expectation is CPython 3.14's own output for the same source (checked
//! against the pinned oracle by `tests/conformance/numeric.rs`'s
//! `unary_not_invert_matches_cpython_3_14_7_byte_for_byte` over
//! `tests/fixtures/unary_not_invert.py`); these tests restate it without the
//! oracle so the behavior is gated on every CI run, not only on the
//! oracle-bearing job.

use pycc_scratch::ScratchDir;
use std::io::Write;
use std::process::Command;

fn pycc_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_pycc"))
}

fn case_dir(case: &str) -> ScratchDir {
    ScratchDir::new(&format!("issue604_{case}")).expect("failed to create scratch dir")
}

fn write_fixture(dir: &std::path::Path, source: &str) -> std::path::PathBuf {
    let path = dir.join("unary.py");
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(source.as_bytes()).unwrap();
    path
}

/// Build `source` with the public CLI and return the compiled program's stdout.
fn build_and_run(case: &str, source: &str) -> String {
    let dir = case_dir(case);
    let src = write_fixture(&dir, source);
    let out = dir.join("unary");

    let output = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "pycc build should succeed for {case}, got {:?}: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );

    let run = Command::new(&out).output().unwrap();
    assert!(run.status.success(), "compiled program failed for {case}");
    String::from_utf8(run.stdout).unwrap()
}

/// Run `pycc check` on `source` and return its combined diagnostic output,
/// asserting the check failed.
fn check_err(case: &str, source: &str) -> String {
    let dir = case_dir(case);
    let src = write_fixture(&dir, source);
    let output = Command::new(pycc_bin())
        .args(["check", src.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "pycc check should have rejected {case}"
    );
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

// ---- `not` over `bool` ----

#[test]
fn not_true_is_false() {
    assert_eq!(build_and_run("not_true", "print(not True)\n"), "False\n");
}

#[test]
fn not_false_is_true() {
    assert_eq!(build_and_run("not_false", "print(not False)\n"), "True\n");
}

// ---- `not` over `int` ----

#[test]
fn not_zero_int_is_true() {
    assert_eq!(
        build_and_run("not_zero_int", "x = 0\nprint(not x)\n"),
        "True\n"
    );
}

#[test]
fn not_nonzero_int_is_false() {
    assert_eq!(
        build_and_run("not_nonzero_int", "x = 5\nprint(not x)\n"),
        "False\n"
    );
}

#[test]
fn not_negative_int_is_false() {
    assert_eq!(
        build_and_run("not_negative_int", "x = -3\nprint(not x)\n"),
        "False\n"
    );
}

#[test]
fn not_a_bigint_is_false() {
    // Past the tagged-fixnum boundary (`i64::MAX >> 1`): `truthy` must call
    // `pycc_rt_int_truthy` rather than compare the raw tagged word to zero,
    // or a nonzero bigint whose *tag bits* happen to be zero would be
    // misreported as falsy.
    assert_eq!(
        build_and_run("not_bigint", "x = 9000000000000000000\nprint(not x)\n"),
        "False\n"
    );
}

// ---- `not` over `float` ----

#[test]
fn not_positive_zero_float_is_true() {
    assert_eq!(
        build_and_run("not_pos_zero_float", "x = 0.0\nprint(not x)\n"),
        "True\n"
    );
}

#[test]
fn not_negative_zero_float_is_true() {
    // `bool(-0.0)` is `False` in CPython, so `not -0.0` is `True` -- this is
    // exactly why `truthy`'s float arm uses the *unordered*-not-equal
    // predicate against `0.0` rather than a sign check.
    assert_eq!(
        build_and_run("not_neg_zero_float", "x = -0.0\nprint(not x)\n"),
        "True\n"
    );
}

#[test]
fn not_nonzero_float_is_false() {
    assert_eq!(
        build_and_run("not_nonzero_float", "x = 2.5\nprint(not x)\n"),
        "False\n"
    );
}

// ---- `not` over `str` ----

#[test]
fn not_empty_str_is_true() {
    assert_eq!(
        build_and_run("not_empty_str", "x = \"\"\nprint(not x)\n"),
        "True\n"
    );
}

#[test]
fn not_nonempty_str_is_false() {
    assert_eq!(
        build_and_run("not_nonempty_str", "x = \"ab\"\nprint(not x)\n"),
        "False\n"
    );
}

// ---- `not` over `Optional` ----

#[test]
fn not_a_none_optional_is_true() {
    assert_eq!(
        build_and_run("not_none_optional", "x: int | None = None\nprint(not x)\n",),
        "True\n"
    );
}

#[test]
fn not_a_present_nonzero_optional_is_false() {
    assert_eq!(
        build_and_run("not_present_optional", "x: int | None = 5\nprint(not x)\n",),
        "False\n"
    );
}

#[test]
fn not_a_present_zero_optional_is_true() {
    // Present *and* falsy: the payload's own truthiness still applies once
    // the value is known present, exactly like a plain `int` `0`.
    assert_eq!(
        build_and_run(
            "not_present_zero_optional",
            "x: int | None = 0\nprint(not x)\n",
        ),
        "True\n"
    );
}

// ---- `not` composition ----

#[test]
fn double_not_round_trips_to_the_original_truthiness() {
    assert_eq!(
        build_and_run("double_not", "x = 5\nprint(not not x)\n"),
        "True\n"
    );
}

#[test]
fn not_composes_with_if() {
    assert_eq!(
        build_and_run(
            "not_if",
            "x = 0\nif not x:\n    print(\"empty\")\nelse:\n    print(\"full\")\n",
        ),
        "empty\n"
    );
}

// ---- `~` over `int` ----

#[test]
fn invert_zero_is_minus_one() {
    assert_eq!(build_and_run("invert_zero", "x = 0\nprint(~x)\n"), "-1\n");
}

#[test]
fn invert_a_positive_int() {
    assert_eq!(build_and_run("invert_pos", "x = 5\nprint(~x)\n"), "-6\n");
}

#[test]
fn invert_a_negative_int() {
    assert_eq!(build_and_run("invert_neg", "x = -5\nprint(~x)\n"), "4\n");
}

#[test]
fn invert_double_round_trips() {
    assert_eq!(
        build_and_run("invert_double", "x = 7\nprint(~(~x))\n"),
        "7\n"
    );
}

#[test]
fn invert_at_the_largest_smallint_crosses_into_bigint() {
    // `i64::MAX >> 1` is the largest tagged smallint this runtime's
    // representation holds inline (D-061); `~x == -x - 1` on it must still
    // inherit `int_sub`'s arbitrary-precision promotion, the same guarantee
    // #603's own negation test exercises for `-x` alone.
    assert_eq!(
        build_and_run(
            "invert_smallint_boundary",
            "x = 4611686018427387903\nprint(~x)\n",
        ),
        "-4611686018427387904\n"
    );
}

#[test]
fn invert_a_bigint() {
    assert_eq!(
        build_and_run("invert_bigint", "x = 9000000000000000000\nprint(~x)\n"),
        "-9000000000000000001\n"
    );
}

// ---- `~` over `bool` ----

#[test]
fn invert_true_promotes_to_int() {
    assert_eq!(
        build_and_run("invert_true", "t = True\nprint(~t)\n"),
        "-2\n"
    );
}

#[test]
fn invert_false_promotes_to_int() {
    assert_eq!(
        build_and_run("invert_false", "f = False\nprint(~f)\n"),
        "-1\n"
    );
}

// ---- rejections (T0021) ----

#[test]
fn not_over_a_list_is_rejected() {
    // `truthy`'s own `Scalar::List` arm panics rather than modeling a truth
    // value for containers (v0.2 has no `bool(list)` semantics) --
    // `unary_result_type` rejects it before that panic could ever be
    // reached from a type-checked program.
    let message = check_err("not_list", "xs = [1, 2]\nprint(not xs)\n");
    assert!(
        message.contains("T0021"),
        "unexpected diagnostic: {message}"
    );
}

#[test]
fn invert_over_a_float_is_rejected() {
    let message = check_err("invert_float", "x = 2.5\nprint(~x)\n");
    assert!(
        message.contains("T0021"),
        "unexpected diagnostic: {message}"
    );
}

#[test]
fn invert_over_a_str_is_rejected() {
    let message = check_err("invert_str", "x = \"ab\"\nprint(~x)\n");
    assert!(
        message.contains("T0021"),
        "unexpected diagnostic: {message}"
    );
}
