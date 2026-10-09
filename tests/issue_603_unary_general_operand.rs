//! Issue #603 (Part 2 of #573): public-CLI differential coverage for `-x` and
//! `+x` over an operand that is *not* a numeric literal.
//!
//! #602 (Part 1) folds a sign that sits directly on a literal, so `-1` never
//! produces a node at all. Everything here is a case that fold cannot reach: a
//! name, a call result, a parenthesized expression, an attribute, a subscript,
//! and a nested unary.
//!
//! Every expectation is CPython 3.14's own output for the same source (checked
//! against the pinned oracle by `tests/conformance/numeric.rs`'s
//! `unary_general_operand_matches_cpython_3_14_7_byte_for_byte` over
//! `tests/fixtures/unary_general_operand.py`); these tests restate it without
//! the oracle so the behavior is gated on every CI run, not only on the
//! oracle-bearing job.
//!
//! The `int`/`bool` and `float` cases are separated deliberately, because
//! `pycc_mir` rewrites them into different binary shapes -- `0 - x` / `0 + x`
//! for the tagged-fixnum representation (so bigint promotion is inherited from
//! `pycc_rt`'s own `int_sub`/`int_add`, which `x * -1` would not give: `int_mul`
//! rejects an already-promoted bigint), and `x * -1.0` / `x * 1.0` for `float`
//! (so `-0.0` and the infinities negate exactly, which `0.0 - x` would not do).

use pycc_scratch::ScratchDir;
use std::io::Write;
use std::process::Command;

fn pycc_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_pycc"))
}

fn case_dir(case: &str) -> ScratchDir {
    ScratchDir::new(&format!("issue603_{case}")).expect("failed to create scratch dir")
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

// ---- int ----

#[test]
fn negating_an_int_name_prints_the_negated_value() {
    assert_eq!(build_and_run("int_name", "x = 5\nprint(-x)\n"), "-5\n");
}

#[test]
fn unary_plus_on_an_int_name_prints_the_value_unchanged() {
    assert_eq!(build_and_run("int_plus", "x = 5\nprint(+x)\n"), "5\n");
}

#[test]
fn a_nested_negation_cancels_out() {
    assert_eq!(build_and_run("int_nested", "x = 5\nprint(-(-x))\n"), "5\n");
}

#[test]
fn negating_a_parenthesized_sum_negates_the_whole_operand() {
    // `-(x + y)` must not be read as `(-x) + y`.
    assert_eq!(
        build_and_run(
            "int_paren",
            "x = 5\ny = 3\nprint(-(x + y))\nprint(-x + y)\n"
        ),
        "-8\n-2\n"
    );
}

#[test]
fn negating_a_call_result_negates_what_the_call_returned() {
    assert_eq!(
        build_and_run(
            "int_call",
            "def twice(n: int) -> int:\n    return n * 2\n\n\nprint(-twice(4))\nprint(+twice(4))\n",
        ),
        "-8\n8\n"
    );
}

#[test]
fn negating_a_parameter_inside_a_function_body_works() {
    assert_eq!(
        build_and_run(
            "int_param",
            "def negated(n: int) -> int:\n    return -n\n\n\nprint(negated(7))\nprint(negated(-7))\n",
        ),
        "-7\n7\n"
    );
}

#[test]
fn negating_a_subscript_negates_the_element() {
    assert_eq!(
        build_and_run("int_subscript", "xs = [1, 2, 3]\nprint(-xs[2])\n"),
        "-3\n"
    );
}

#[test]
fn negating_an_attribute_negates_the_attribute_value() {
    assert_eq!(
        build_and_run(
            "int_attr",
            concat!(
                "class Point:\n",
                "    def __init__(self, dx: int) -> None:\n",
                "        self.dx = dx\n",
                "\n",
                "    def flipped(self) -> int:\n",
                "        return -self.dx\n",
                "\n",
                "\n",
                "pt = Point(9)\n",
                "print(-pt.dx)\n",
                "print(pt.flipped())\n",
            ),
        ),
        "-9\n-9\n"
    );
}

/// The `0 - x` rewrite (rather than `x * -1`) is what makes this work:
/// `pycc_rt`'s `int_mul` raises `OverflowError` (Part C of #1038; formerly an
/// abort) on an operand that has already been promoted to a bigint, while
/// `int_sub` handles it.
#[test]
fn negating_a_bigint_keeps_arbitrary_precision() {
    assert_eq!(
        build_and_run(
            "int_bigint",
            concat!(
                "base = 2000000000000000000\n",
                "big = base * 4\n",
                "print(-big)\n",
                "print(-(big + 1))\n",
                "print(-big + big)\n",
                "print(-(-big))\n",
            ),
        ),
        "-8000000000000000000\n-8000000000000000001\n0\n8000000000000000000\n"
    );
}

// ---- bool ----

/// `+` is not the identity on `bool`: `+True` is the integer `1` in Python, so
/// the operand crosses into `int` under both operators.
#[test]
fn unary_operators_promote_a_bool_operand_to_int() {
    assert_eq!(
        build_and_run(
            "bool_promote",
            "t = True\nf = False\nprint(-t)\nprint(+t)\nprint(-f)\nprint(+f)\nprint(-t + 1)\n",
        ),
        "-1\n1\n0\n0\n0\n"
    );
}

// ---- float ----

#[test]
fn negating_a_float_name_prints_the_negated_value() {
    assert_eq!(
        build_and_run(
            "float_name",
            "p = 2.5\nprint(-p)\nprint(+p)\nprint(-(-p))\n"
        ),
        "-2.5\n2.5\n2.5\n"
    );
}

/// The `x * -1.0` rewrite (rather than `0.0 - x`) is what makes this exact:
/// `0.0 - 0.0` is `+0.0`, so subtraction would lose the sign of a negated
/// zero.
#[test]
fn negating_a_float_zero_produces_a_negative_zero() {
    assert_eq!(
        build_and_run(
            "float_zero",
            "zero = 0.0\nprint(-zero)\nprint(+zero)\nprint(-(-zero))\n",
        ),
        "-0.0\n0.0\n0.0\n"
    );
}

#[test]
fn negating_a_float_infinity_flips_its_sign() {
    assert_eq!(
        build_and_run(
            "float_inf",
            "inf = 1e400\nprint(-inf)\nprint(+inf)\nprint(-(-inf))\n",
        ),
        "-inf\ninf\ninf\n"
    );
}

#[test]
fn negating_a_float_parameter_inside_a_function_body_works() {
    assert_eq!(
        build_and_run(
            "float_param",
            concat!(
                "def negated(v: float) -> float:\n",
                "    return -v\n",
                "\n",
                "\n",
                "print(negated(1.25))\n",
                "print(negated(-1.25))\n",
            ),
        ),
        "-1.25\n1.25\n"
    );
}

// ---- interaction with the passes that walk expressions ----

/// A comprehension's loop variable is renamed to a synthesized internal name
/// (D-117) by `rename_name_in_expr`, which is exhaustive over `HirExpr` on
/// purpose -- so the new unary node has to be renamed through as well, in the
/// element, the condition, and a set comprehension alike.
#[test]
fn a_unary_operand_inside_a_comprehension_is_renamed_with_the_loop_variable() {
    assert_eq!(
        build_and_run(
            "comprehension",
            concat!(
                "xs = [1, 2, 3]\n",
                "ys = [-v for v in xs]\n",
                "print(ys[0])\n",
                "print(ys[2])\n",
                "zs = [v for v in xs if -v < -1]\n",
                "print(zs[0])\n",
                "ss = {-v for v in xs}\n",
                "print(len(ss))\n",
            ),
        ),
        "-1\n-3\n2\n3\n"
    );
}

/// The generic-call passes in `pycc_types` walk expressions to find and
/// monomorphize calls, so a unary node must be transparent to them in both
/// directions: a unary operand *inside* a generic call's argument, and a
/// generic call *inside* a unary operand.
#[test]
fn a_unary_operand_composes_with_a_generic_call_in_both_directions() {
    assert_eq!(
        build_and_run(
            "generic",
            concat!(
                "def ident[T](v: T) -> T:\n",
                "    return v\n",
                "\n",
                "\n",
                "x = 5\n",
                "print(ident(-x))\n",
                "print(-ident(x))\n",
            ),
        ),
        "-5\n-5\n"
    );
}

/// A generic function's own body is walked separately, by the pass that
/// rejects a generic function calling another generic function. The unary
/// node has to be transparent there too.
#[test]
fn a_unary_operand_inside_a_generic_function_body_is_walked_through() {
    assert_eq!(
        build_and_run(
            "generic_body",
            concat!(
                "def negate[T](v: T, n: int) -> int:\n",
                "    return -n\n",
                "\n",
                "\n",
                "print(negate(1, 5))\n",
            ),
        ),
        "-5\n"
    );
}

/// Same for the protocol-call specialization pass.
#[test]
fn a_unary_operand_composes_with_a_protocol_dispatched_call() {
    assert_eq!(
        build_and_run(
            "protocol",
            concat!(
                "from typing import Protocol\n",
                "\n",
                "\n",
                "class Scaled(Protocol):\n",
                "    def scale(self) -> int: ...\n",
                "\n",
                "\n",
                "class Two:\n",
                "    def __init__(self, v: int) -> None:\n",
                "        self.v = v\n",
                "\n",
                "    def scale(self) -> int:\n",
                "        return self.v * 2\n",
                "\n",
                "\n",
                "def apply(s: Scaled) -> int:\n",
                "    return -s.scale()\n",
                "\n",
                "\n",
                "print(apply(Two(4)))\n",
            ),
        ),
        "-8\n"
    );
}

// ---- rejected operands ----

/// A concrete non-numeric operand keeps the unary spelling in the diagnostic
/// rather than mentioning the binary rewrite's synthetic `0`.
#[test]
fn negating_a_str_is_rejected_with_the_unary_diagnostic() {
    let message = check_err("str_operand", "s = \"abc\"\nprint(-s)\n");
    assert!(
        message.contains("T0021") && message.contains("unary operator USub"),
        "unexpected diagnostic: {message}"
    );
    assert!(
        message.contains("`str`"),
        "diagnostic should name the operand type: {message}"
    );
}

#[test]
fn unary_plus_on_a_list_is_rejected() {
    let message = check_err("list_operand", "xs = [1, 2]\nprint(+xs)\n");
    assert!(
        message.contains("T0021") && message.contains("unary operator UAdd"),
        "unexpected diagnostic: {message}"
    );
}

/// A private helper's parameter type is still an inference variable when the
/// unary node is collected, so the solver defers it as the binary constraint
/// MIR will lower it to. The diagnostic therefore names that rewrite -- which
/// is why the concrete path above exists to keep the common case readable.
#[test]
fn negating_an_inferred_helper_parameter_of_the_wrong_type_is_rejected() {
    let message = check_err(
        "inferred_operand",
        "def _bad(s):\n    return -s\n\n\nt = \"ab\"\nprint(_bad(t))\n",
    );
    assert!(
        message.contains("T0021"),
        "unexpected diagnostic: {message}"
    );
}

/// The mirror of the case above that must keep working: an inferred helper
/// parameter that really is numeric solves through the same deferred
/// constraint.
#[test]
fn negating_an_inferred_helper_parameter_of_a_numeric_type_is_accepted() {
    assert_eq!(
        build_and_run(
            "inferred_ok",
            "def _neg(n):\n    return -n\n\n\nx: int = 4\nprint(_neg(x))\n",
        ),
        "-4\n"
    );
}

/// `+` takes the same deferred path as `-` but with the `0 + x` shape, so it
/// needs its own inferred-helper case rather than riding on the `-` one.
#[test]
fn unary_plus_on_an_inferred_helper_parameter_is_accepted() {
    assert_eq!(
        build_and_run(
            "inferred_plus",
            "def _pos(n):\n    return +n\n\n\nx: int = 4\nprint(_pos(x))\n",
        ),
        "4\n"
    );
}

/// A maybe-bound operand (assigned in only one branch) has no type term at
/// all, so the solver yields no constraint and the helper's return type stays
/// uninferable -- the unary node must propagate that rather than inventing a
/// type for it.
#[test]
fn negating_a_maybe_bound_operand_leaves_the_helper_uninferable() {
    let message = check_err(
        "maybe_bound",
        "def _f(flag: bool):\n    if flag:\n        m = 1\n    return -m\n\n\nprint(_f(True))\n",
    );
    assert!(
        message.contains("cannot infer return type of private helper `_f`"),
        "unexpected diagnostic: {message}"
    );
}

// `not` and `~` were Part 3 (#604): they are now implemented, with their own
// dedicated public-CLI differential coverage in
// `tests/issue_604_unary_not_invert.rs` (mirroring this file's own style),
// rather than a rejection test living here.
