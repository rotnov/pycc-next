//! Unary operators through generics and `Optional`/`is None` checks.

use super::*;

// -- #603: a unary operand is a general expression -------------------
//
// The unary arms these cases drive live in this crate's generic-call,
// generic-class, and protocol walkers. `tests/issue_603_unary_general_operand.rs`
// already pins the observable program output end to end; these unit
// tests exist so the same arms are executed by this crate's own test
// binary too, which is what the D-014 region gate actually measures.

#[test]
fn a_unary_operand_composes_with_a_generic_call_in_both_directions() {
    let result = check_source(
        "def ident[T](v: T) -> T:\n    return v\n\n\nx = 5\nprint(ident(-x))\nprint(-ident(x))\n",
    );
    assert!(
        result.is_ok(),
        "a unary operand must compose with a generic call: {result:?}"
    );
}

#[test]
fn a_unary_operand_inside_a_generic_function_body_is_walked_through() {
    let result = check_source(
        "def negate[T](v: T, n: int) -> int:\n    return -n\n\n\nprint(negate(1, 5))\n",
    );
    assert!(
        result.is_ok(),
        "a unary operand inside a generic body must resolve: {result:?}"
    );
}

#[test]
fn a_unary_operand_composes_with_a_generic_class_instantiation() {
    let result = check_source(
        "class Box[T]:\n    def __init__(self, v: T) -> None:\n        self.v = v\n\n    def size(self) -> int:\n        return 1\n\n\nb = Box(5)\nprint(-b.size())\n",
    );
    assert!(
        result.is_ok(),
        "a unary operand must compose with a generic class instantiation: {result:?}"
    );
}

#[test]
fn a_unary_operand_composes_with_a_protocol_dispatched_call() {
    let result = check_source(
        "from typing import Protocol\n\n\nclass Scaled(Protocol):\n    def scale(self) -> int: ...\n\n\nclass Two:\n    def __init__(self, v: int) -> None:\n        self.v = v\n\n    def scale(self) -> int:\n        return self.v * 2\n\n\ndef apply(s: Scaled) -> int:\n    return -s.scale()\n\n\nprint(apply(Two(4)))\n",
    );
    assert!(
        result.is_ok(),
        "a unary operand must compose with a protocol call: {result:?}"
    );
}

#[test]
fn an_error_inside_a_unary_operand_propagates_through_a_generic_body() {
    let result = check_source(
        "def negate[T](v: T, n: int) -> int:\n    return -missing(n)\n\n\nprint(negate(1, 5))\n",
    );
    assert!(
        result.is_err(),
        "an unresolvable unary operand must be rejected: {result:?}"
    );
}

#[test]
fn an_error_inside_a_unary_operand_propagates_at_module_level() {
    let result = check_source("x = -missing(1)\n");
    assert!(
        result.is_err(),
        "an unresolvable unary operand must be rejected: {result:?}"
    );
}

#[test]
fn an_unbound_local_under_a_unary_in_a_generic_body_propagates() {
    let result = check_source(
        "def negate[T](v: T) -> int:\n    a = -b\n    b = 1\n    return a\n\n\nprint(negate(1))\n",
    );
    let message = result
        .expect_err("an unbound local must be rejected")
        .message;
    assert!(
        message.contains("local name `b` is not bound"),
        "unexpected message: {message}"
    );
}

#[test]
fn a_failing_generic_call_under_a_unary_propagates() {
    let result =
        check_source("def ident[T](v: T) -> T:\n    return v\n\n\nx = -ident(\"s\")\nprint(x)\n");
    assert!(
        result.is_err(),
        "a unary over a `str`-instantiated generic call must be rejected: {result:?}"
    );
}

// A unary whose operand is still an inference variable cannot be checked
// with `unary_result_type` yet, so the solver defers it as the exact
// binary constraint `pycc_mir` lowers it to. Both operators take that
// path, and only an unannotated helper parameter -- whose type comes from
// signature inference rather than an annotation -- reaches it.

#[test]
fn a_negated_inferred_parameter_defers_to_a_subtraction_constraint() {
    let result = check_source("def _neg(n):\n    return -n\n\n\nx: int = 4\nprint(_neg(x))\n");
    assert!(
        result.is_ok(),
        "a negated inferred parameter must resolve through the deferred constraint: {result:?}"
    );
}

#[test]
fn a_unary_plus_inferred_parameter_defers_to_an_addition_constraint() {
    let result = check_source("def _pos(n):\n    return +n\n\n\nx: int = 4\nprint(_pos(x))\n");
    assert!(
        result.is_ok(),
        "a unary-plus inferred parameter must resolve through the deferred constraint: {result:?}"
    );
}

#[test]
fn a_deferred_unary_constraint_still_rejects_a_non_numeric_operand() {
    let result = check_source("def _bad(s):\n    return -s\n\n\nt = \"ab\"\nprint(_bad(t))\n");
    assert!(
        result.is_err(),
        "a deferred unary constraint over `str` must be rejected: {result:?}"
    );
}

// #604 (Part 3 of #573). `~` shares the same deferred `Sub`-shaped path as
// `-`/`+` above (only the `matches!(op, UnaryOpKind::UAdd)` check tells them
// apart), so this is the `Invert` counterpart of
// `a_negated_inferred_parameter_defers_to_a_subtraction_constraint`.
#[test]
fn a_bitwise_invert_inferred_parameter_defers_to_a_subtraction_constraint() {
    let result = check_source("def _inv(n):\n    return ~n\n\n\nx: int = 4\nprint(_inv(x))\n");
    assert!(
        result.is_ok(),
        "an inverted inferred parameter must resolve through the deferred constraint: {result:?}"
    );
}

// #604: `not` has its own dedicated arm in `collect_expr_constraints`
// (unlike `-`/`+`/`~`, it never defers to a binary constraint at all -- its
// result is always `Ty::Bool` regardless of the operand's resolved type),
// exercised here through an inferred parameter specifically so the operand
// is still an unresolved inference variable when this arm runs.
#[test]
fn a_logical_not_inferred_parameter_always_resolves_to_bool() {
    let result = check_source("def _not(n):\n    return not n\n\n\nx: int = 4\nprint(_not(x))\n");
    assert!(
        result.is_ok(),
        "a logical-not inferred parameter must resolve to bool: {result:?}"
    );
}

// -- D-197 (#763, Part 1 of #747): `Ty::Optional`, `T | None` assignability,
// and `is`/`is not` on `None` ------------------------------------------

#[test]
fn a_bare_int_value_widens_to_an_optional_int_annotation() {
    // `is_assignable`'s one-way `Ty::Optional` widening rule: a bare `int`
    // value is assignable to an `int | None` annotation.
    let result = check_source("x: int | None = 5\nprint(x is None)\n");
    assert!(
        result.is_ok(),
        "a bare int must widen to `int | None`: {result:?}"
    );
}

#[test]
fn a_none_literal_widens_to_an_optional_int_annotation() {
    let result = check_source("x: int | None = None\nprint(x is not None)\n");
    assert!(
        result.is_ok(),
        "`None` must widen to `int | None`: {result:?}"
    );
}

#[test]
fn reversed_operand_order_none_or_int_is_equivalent_to_int_or_none() {
    let result = check_source("x: None | int = None\nprint(x is None)\n");
    assert!(
        result.is_ok(),
        "`None | int` must accept the same values as `int | None`: {result:?}"
    );
}

#[test]
fn an_optional_int_value_narrows_back_to_a_bare_int_annotation_inside_is_not_none() {
    // Superseded by issue #769 (Part 2 of #747, D-205): flow-sensitive
    // narrowing of a top-level `is not None` test now exists, so an
    // `Optional[int]` read *inside* the `is not None` body resolves to
    // plain `int` and is assignable to a bare `int`-annotated target. See
    // `narrowing`'s own test module for the full narrowing test suite;
    // this one specifically guards the exact shape this test used to
    // assert was rejected, so a future regression here is caught by name.
    let result =
        check_source("x: int | None = 5\nif x is not None:\n    y: int = x\n    print(y)\n");
    assert!(
        result.is_ok(),
        "a narrowed `Optional[int]` read inside `is not None` must be assignable to plain `int`: {result:?}"
    );
}

#[test]
fn is_none_on_an_optional_int_type_checks_as_bool() {
    let result = check_source(
        "def f(flag: bool) -> int | None:\n    if flag:\n        return 5\n    return None\n\n\nprint(f(True) is None)\n",
    );
    assert!(
        result.is_ok(),
        "`is None` against an `Optional[int]` expression must type-check: {result:?}"
    );
}

#[test]
fn is_not_none_on_a_plain_none_typed_expression_type_checks_as_bool() {
    let result = check_source("x = None\nprint(x is not None)\n");
    assert!(
        result.is_ok(),
        "`is not None` against a `None`-typed expression must type-check: {result:?}"
    );
}

#[test]
fn is_none_against_a_non_optional_operand_is_rejected_with_t0021() {
    // HIR lowering already guarantees one operand of an `is`/`is not`
    // comparison is syntactically `None` (`Expr::Compare`'s `is_none_operand_shape`
    // check); the *other* operand's static type must be `Ty::Optional(_)` or
    // `Ty::None` for `infer_expr_in`'s `Compare` arm to accept it here.
    let result = check_source("x: int = 5\nprint(x is None)\n");
    let err = result.expect_err("`is None` against a plain `int` must be rejected");
    assert_eq!(err.code, "T0021");
    assert!(
        err.message.contains("is") && err.message.to_lowercase().contains("none"),
        "unexpected message: {}",
        err.message
    );
}

#[test]
fn is_not_none_against_a_str_operand_is_rejected_with_t0021() {
    let result = check_source("x: str = \"a\"\nprint(x is not None)\n");
    let err = result.expect_err("`is not None` against a plain `str` must be rejected");
    assert_eq!(err.code, "T0021");
}

#[test]
fn none_on_the_left_of_is_still_reads_the_other_operands_type() {
    // Mirrors `is_none_on_an_optional_int_type_checks_as_bool` but with the
    // `None` literal as the *left* operand (`None is x`) instead of the
    // right (`x is None`). `infer_expr_in`'s `Compare` arm picks the "other"
    // operand's type from whichever side is *not* the `NoneLiteral`, so this
    // exercises the `left` branch of that selection specifically.
    let result = check_source("x: int | None = 5\nprint(None is x)\n");
    assert!(
        result.is_ok(),
        "`None is x` against an `Optional[int]` must type-check as bool: {result:?}"
    );
}
