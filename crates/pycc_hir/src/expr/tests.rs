//! Unit tests for `expr.rs`, moved out of that file's inline `mod tests`
//! per AGENTS.md's file-decomposition rule (issue #890; tracking issue
//! #552). Test names and bodies are unchanged.

use crate::expr::keyword_bind::SignatureTable;
use crate::{BinOpKind, FStringPart, HirExpr, HirItem, HirStmt, UnaryOpKind};

/// Lowers `source` and asserts that the value expression of its first
/// top-level assignment is `expected`, so each case below states the
/// folded literal it wants directly.
fn assert_first_assign(source: &str, expected: HirExpr) {
    let module = pycc_parser::parse(source).expect("test fixture must parse");
    let hir = crate::lower_checked(&module).expect("fixture must lower");
    assert!(matches!(
        &hir.items[0],
        HirItem::TopLevelStmt(HirStmt::Assign { value, .. }) if *value == expected
    ));
}

fn lower_err_code(source: &str) -> String {
    let module = pycc_parser::parse(source).expect("test fixture must parse");
    crate::lower_checked(&module)
        .expect_err("fixture must be rejected")
        .code
        .to_string()
}

fn lower_err_message(source: &str) -> String {
    let module = pycc_parser::parse(source).expect("test fixture must parse");
    crate::lower_checked(&module)
        .expect_err("fixture must be rejected")
        .message
}

#[test]
fn unary_minus_folds_into_an_int_literal() {
    assert_first_assign("x = -5\n", HirExpr::IntLiteral(-5));
}

#[test]
fn unary_plus_leaves_an_int_literal_positive() {
    assert_first_assign("x = +5\n", HirExpr::IntLiteral(5));
}

#[test]
fn unary_minus_folds_into_a_float_literal() {
    assert_first_assign("x = -1.5\n", HirExpr::FloatLiteral(-1.5));
}

#[test]
fn unary_plus_leaves_a_float_literal_positive() {
    assert_first_assign("x = +1.5\n", HirExpr::FloatLiteral(1.5));
}

#[test]
fn unary_minus_accepts_i64_min() {
    // `ruff` stores the magnitude as a `u64`, so this source is `USub`
    // applied to `9223372036854775808` -- a magnitude that does not fit in
    // an `i64` even though its negation is exactly `i64::MIN`. Folding the
    // sign before the range check is what accepts it.
    assert_first_assign("x = -9223372036854775808\n", HirExpr::IntLiteral(i64::MIN));
}

#[test]
fn unary_plus_rejects_i64_mins_magnitude() {
    // The same magnitude without a negative sign is genuinely out of
    // range: `i64::MAX` is one less.
    assert_eq!(lower_err_code("x = +9223372036854775808\n"), "C0001");
}

#[test]
fn unary_minus_rejects_a_magnitude_past_u64() {
    // A magnitude too large for `u64` reaches the fold as `Number::Big`,
    // whose `as_u64()` is `None`.
    assert_eq!(lower_err_code("x = -99999999999999999999999\n"), "C0001");
}

#[test]
fn unary_sign_on_a_complex_literal_is_unsupported() {
    let message = lower_err_message("x = -1j\n");
    assert!(
        message.contains("numeric literal kind not supported yet"),
        "unexpected message: {message}"
    );
}

/// The `assert_first_assign` counterpart for the *second* top-level
/// assignment -- the unary fixtures below all need a bound name first,
/// so `assert_first_assign` would look at the binding rather than at the
/// unary expression under test.
fn assert_second_assign(source: &str, expected: HirExpr) {
    let module = pycc_parser::parse(source).expect("test fixture must parse");
    let hir = crate::lower_checked(&module).expect("fixture must lower");
    assert!(matches!(
        &hir.items[1],
        HirItem::TopLevelStmt(HirStmt::Assign { value, .. }) if *value == expected
    ));
}

#[test]
fn unary_minus_on_a_non_literal_builds_a_unary_node() {
    assert_second_assign(
        "y = 1\nx = -y\n",
        HirExpr::UnaryOp {
            op: UnaryOpKind::USub,
            operand: Box::new(HirExpr::Name("y".to_string())),
        },
    );
}

#[test]
fn unary_plus_on_a_non_literal_builds_a_unary_node() {
    assert_second_assign(
        "y = 1\nx = +y\n",
        HirExpr::UnaryOp {
            op: UnaryOpKind::UAdd,
            operand: Box::new(HirExpr::Name("y".to_string())),
        },
    );
}

#[test]
fn unary_minus_on_a_parenthesized_sum_lowers_the_whole_operand() {
    assert_second_assign(
        "y = 1\nx = -(y + 2)\n",
        HirExpr::UnaryOp {
            op: UnaryOpKind::USub,
            operand: Box::new(HirExpr::BinOp {
                op: BinOpKind::Add,
                left: Box::new(HirExpr::Name("y".to_string())),
                right: Box::new(HirExpr::IntLiteral(2)),
            }),
        },
    );
}

#[test]
fn a_lowering_error_inside_a_unary_operand_propagates() {
    // The operand is lowered recursively, so its own rejection must
    // surface unchanged rather than being masked by the unary arm.
    let message = lower_err_message("y = 1\nx = -(-1j)\n");
    assert!(
        message.contains("numeric literal kind not supported yet"),
        "unexpected message: {message}"
    );
}

#[test]
fn unary_operator_kinds_render_their_source_spelling() {
    assert_eq!(UnaryOpKind::USub.as_str(), "-");
    assert_eq!(UnaryOpKind::UAdd.as_str(), "+");
    assert_eq!(UnaryOpKind::Not.as_str(), "not");
    assert_eq!(UnaryOpKind::Invert.as_str(), "~");
}

#[test]
fn logical_not_on_a_non_literal_builds_a_unary_node() {
    // #604, Part 3 of #573: `not` has no literal-folding arm (unlike
    // `-`/`+`), so it always lowers through this same generic path.
    assert_second_assign(
        "y = True\nx = not y\n",
        HirExpr::UnaryOp {
            op: UnaryOpKind::Not,
            operand: Box::new(HirExpr::Name("y".to_string())),
        },
    );
}

#[test]
fn bitwise_invert_on_a_non_literal_builds_a_unary_node() {
    // #604, Part 3 of #573.
    assert_second_assign(
        "y = 1\nx = ~y\n",
        HirExpr::UnaryOp {
            op: UnaryOpKind::Invert,
            operand: Box::new(HirExpr::Name("y".to_string())),
        },
    );
}

#[test]
fn a_lowering_error_inside_a_logical_not_operand_propagates() {
    // #604, Part 3 of #573: `not`'s own `(UnaryOp::Not, operand) =>`
    // arm lowers its operand through the same recursive `lower_expr(operand,
    // ..)?` call every other unary arm uses, so a rejection inside the
    // operand must surface unchanged instead of being masked here --
    // mirrors `a_lowering_error_inside_a_unary_operand_propagates` above,
    // which only exercises `USub`/`UAdd`'s own non-literal arm.
    let message = lower_err_message("x = not 1j\n");
    assert!(
        message.contains("numeric literal kind not supported yet"),
        "unexpected message: {message}"
    );
}

#[test]
fn a_lowering_error_inside_a_bitwise_invert_operand_propagates() {
    // #604, Part 3 of #573: same as the `not` case above, for `~`'s arm.
    let message = lower_err_message("x = ~1j\n");
    assert!(
        message.contains("numeric literal kind not supported yet"),
        "unexpected message: {message}"
    );
}

#[test]
fn renaming_walks_through_a_unary_operand() {
    let renamed = super::rename_name_in_expr(
        HirExpr::UnaryOp {
            op: UnaryOpKind::USub,
            operand: Box::new(HirExpr::Name("v".to_string())),
        },
        "v",
        "$comp0",
    );
    assert_eq!(
        renamed,
        HirExpr::UnaryOp {
            op: UnaryOpKind::USub,
            operand: Box::new(HirExpr::Name("$comp0".to_string())),
        }
    );
}

#[test]
fn an_ordinary_fstring_interpolation_lowers_successfully() {
    // Baseline: an interpolation with no `=` debug specifier, no
    // conversion flag, and no format spec still lowers, distinguishing
    // this from the three rejection branches exercised below.
    assert_first_assign(
        "x = f\"{1}\"\n",
        HirExpr::FString(vec![FStringPart::Interpolation(Box::new(
            HirExpr::IntLiteral(1),
        ))]),
    );
}

#[test]
fn fstring_debug_specifier_is_rejected() {
    // #720: `f"{n=}"` silently dropped the `=` debug specifier and
    // printed the bare value instead of `n=5`. Reject it explicitly
    // rather than compiling clean with wrong output.
    assert_eq!(lower_err_code("n = 5\nx = f\"{n=}\"\n"), "C0001");
    let message = lower_err_message("n = 5\nx = f\"{n=}\"\n");
    assert!(
        message.contains("f-string debug specifier (=) is not supported yet"),
        "unexpected message: {message}"
    );
}

#[test]
fn fstring_debug_specifier_is_rejected_even_with_a_format_spec() {
    // The debug-specifier check must fire before the format-spec check
    // reaches its own arm, since `f"{n=:.2f}"` carries both a debug
    // specifier and a format spec -- this exercises that ordering
    // rather than only the debug-specifier-alone case above.
    let message = lower_err_message("n = 5\nx = f\"{n=:.2f}\"\n");
    assert!(
        message.contains("f-string debug specifier (=) is not supported yet"),
        "unexpected message: {message}"
    );
}

// PEP 572 (#774): CPython's own grammar never actually parses a
// non-name walrus target -- `ruff_python_parser` agrees -- so the only
// way to exercise `lower_expr`'s own defensive rejection is a hand-built
// `Expr::Named` whose `target` is some other expression kind, bypassing
// the parser entirely (mirroring `pycc_types::tests`'s own hand-built-HIR
// convention for a similarly unreachable-via-the-parser guard).
#[test]
fn a_walrus_target_that_is_not_a_bare_name_is_rejected() {
    let named = pycc_ast::Expr::Named(pycc_ast::ExprNamed {
        node_index: Default::default(),
        range: Default::default(),
        target: Box::new(pycc_ast::Expr::NumberLiteral(pycc_ast::ExprNumberLiteral {
            node_index: Default::default(),
            range: Default::default(),
            value: pycc_ast::Number::Int(pycc_ast::Int::from(0u8)),
        })),
        value: Box::new(pycc_ast::Expr::NumberLiteral(pycc_ast::ExprNumberLiteral {
            node_index: Default::default(),
            range: Default::default(),
            value: pycc_ast::Number::Int(pycc_ast::Int::from(1u8)),
        })),
    });
    let err = super::lower_expr(&named, false, None, &[], &SignatureTable::default()).unwrap_err();
    assert_eq!(err.code, "C0001");
    assert!(
        err.message
            .contains("a walrus assignment target must be a bare name"),
        "unexpected message: {}",
        err.message
    );
}

// PEP 572 (#774): `rename_name_in_expr`'s own `HirExpr::NamedExpr` arm --
// structurally required for the match to compile (see that arm's own
// doc comment on why a walrus can never actually reach a real
// comprehension body in practice) but otherwise untested by any source
// fixture, since #774's scope cut keeps a walrus out of every
// comprehension field. Exercised directly here, mirroring
// `renaming_walks_through_a_unary_operand` just above.
#[test]
fn renaming_walks_through_a_named_expr_target_and_value() {
    let renamed = super::rename_name_in_expr(
        HirExpr::NamedExpr {
            name: "v".to_string(),
            value: Box::new(HirExpr::Name("v".to_string())),
        },
        "v",
        "$comp0",
    );
    assert_eq!(
        renamed,
        HirExpr::NamedExpr {
            name: "$comp0".to_string(),
            value: Box::new(HirExpr::Name("$comp0".to_string())),
        }
    );
}

// The `else` sibling of the test above: the walrus's own target name
// does *not* match `from`, so `rename_name_in_expr`'s `HirExpr::
// NamedExpr` arm's `if name == from { .. } else { name }` takes its
// `else` branch (the target name is left alone) while still recursing
// into `value`, which does contain a `from`-matching `Name` to rename.
#[test]
fn renaming_walks_through_a_named_exprs_value_without_renaming_a_different_target() {
    let renamed = super::rename_name_in_expr(
        HirExpr::NamedExpr {
            name: "other".to_string(),
            value: Box::new(HirExpr::Name("v".to_string())),
        },
        "v",
        "$comp0",
    );
    assert_eq!(
        renamed,
        HirExpr::NamedExpr {
            name: "other".to_string(),
            value: Box::new(HirExpr::Name("$comp0".to_string())),
        }
    );
}

// PEP 572 (#774): the `?` error-propagation branch of `lower_expr`'s own
// `Expr::Named` arm -- a walrus whose *value* itself fails to lower
// (here, a bare `yield` used as the value at module scope, rejected by
// the `Expr::Yield` arm above with `L0001` `'yield' outside function`)
// must surface that inner error rather than being swallowed. Every
// other walrus fixture in this suite has a value that lowers
// successfully, so this is the only one exercising this branch.
#[test]
fn a_walrus_whose_value_fails_to_lower_propagates_the_inner_error() {
    assert_eq!(lower_err_code("(x := (yield 1))\n"), "L0001");
}

// PEP 572 (#774): a walrus assigned into a `HirStmt::Assign`'s own RHS
// (i.e. not an `if`/`while` test or a bare expression statement) is
// rejected by `lower_stmt`'s placement check, which itself is driven by
// `contains_named_expr`. This is the only fixture in the whole test
// suite that finds a *top-level* `NamedExpr` via `contains_named_expr`
// (every other placement test only feeds it expressions with no walrus
// at all), so it is what exercises that function's own `true` arm.
#[test]
fn a_walrus_outside_if_while_or_a_bare_expression_statement_is_rejected() {
    let message = lower_err_message("x = (y := 1)\n");
    assert!(
        message.contains("only supported in an `if`/`while`"),
        "unexpected message: {message}"
    );
}

/// PR 3c of #1082 (Part 3 of #1026): a `for` over a foreign `object`
/// value carries a real expression as its iterable, so a walrus can nest
/// inside it -- and `killed_names`'s `ForObject` arm records only the loop
/// target and the body, never the iterable. A kill written there would
/// therefore survive `apply_kill_prescan` and leave a read at the top of a
/// re-enterable narrowed region checked against the pre-kill narrowing.
/// The placement check refuses it instead, exactly as it already refuses a
/// walrus among `ForRange`'s `range(...)` arguments.
#[test]
fn a_walrus_inside_a_foreign_object_loop_iterable_is_rejected() {
    let message = lower_err_message("xs = [1]\nfor y in xs.count((x := 1)):\n    pass\n");
    assert!(
        message.contains("only supported in an `if`/`while`"),
        "unexpected message: {message}"
    );
}

#[test]
fn contains_named_expr_finds_a_top_level_walrus() {
    let expr = HirExpr::NamedExpr {
        name: "y".to_string(),
        value: Box::new(HirExpr::IntLiteral(1)),
    };
    assert!(super::contains_named_expr(&expr));
}

/// Part 2d of #1371: a walrus inside an object-slot list display is found
/// like one inside any other display; one without a walrus is not.
#[test]
fn contains_named_expr_walks_an_object_list_display() {
    let walrus = HirExpr::ObjectList(vec![
        HirExpr::IntLiteral(0),
        HirExpr::NamedExpr {
            name: "y".to_string(),
            value: Box::new(HirExpr::IntLiteral(1)),
        },
    ]);
    assert!(super::contains_named_expr(&walrus));
    assert!(!super::contains_named_expr(&HirExpr::ObjectList(vec![
        HirExpr::IntLiteral(0)
    ])));
}

// Issue #618: an out-of-range `int` literal in a runtime int-boundary
// position (D-141) is rejected at compile time with a spanned T0051
// diagnostic, restoring `pycc check` as the catch point D-178 (#148)
// knowingly moved to run time. Every named position below is exercised
// once with `i64::MAX` (always out of D-061's tagged-smallint range,
// and always a valid `i64` literal on its own).
const OOR: &str = "9223372036854775807";

fn assert_t0051(source: &str, expected_position: &str) {
    let module = pycc_parser::parse(source).expect("test fixture must parse");
    let err = crate::lower_checked(&module).expect_err("fixture must be rejected");
    assert_eq!(err.code, "T0051", "source: {source:?}");
    assert!(
        err.message.contains(expected_position),
        "expected message to mention {expected_position:?}, got: {}",
        err.message
    );
}

#[test]
fn boundary_list_literal_element_is_t0051() {
    assert_t0051(&format!("xs = [{OOR}]\n"), "list-literal element");
}

#[test]
fn boundary_set_literal_element_is_t0051() {
    assert_t0051(&format!("s = {{{OOR}}}\n"), "set-literal element");
}

#[test]
fn boundary_dict_literal_value_is_t0051() {
    assert_t0051(&format!("d = {{\"a\": {OOR}}}\n"), "dict-literal value");
}

#[test]
fn boundary_list_index_is_t0051() {
    assert_t0051(&format!("xs = [1, 2, 3]\ny = xs[{OOR}]\n"), "list index");
}

#[test]
fn boundary_tuple_literal_index_is_not_t0051() {
    // PR #827 review finding: a tuple base has no D-141 runtime
    // `int`-boundary position at all -- `pycc_types` resolves tuple
    // indexing entirely at compile time and already rejects an
    // out-of-range literal index with its own T0040 ("non-negative
    // literal within range"). Emitting T0051 here for a tuple literal
    // base would preempt that check and mislabel the position as a
    // "list index", so HIR lowering must succeed and defer to
    // `pycc_types`.
    let source = format!("y = (1, 2)[{OOR}]\n");
    let module = pycc_parser::parse(&source).expect("test fixture must parse");
    let result = crate::lower_checked(&module);
    assert!(
        result.is_ok(),
        "a tuple-literal base's out-of-range index must not be rejected by T0051 in HIR, \
         got {result:?}"
    );
}

#[test]
fn boundary_slice_start_is_t0051() {
    assert_t0051(&format!("xs = [1, 2, 3]\ny = xs[{OOR}:2]\n"), "slice bound");
}

#[test]
fn boundary_slice_stop_is_t0051() {
    assert_t0051(&format!("xs = [1, 2, 3]\ny = xs[0:{OOR}]\n"), "slice bound");
}

#[test]
fn boundary_slice_step_is_t0051() {
    assert_t0051(
        &format!("xs = [1, 2, 3]\ny = xs[0:2:{OOR}]\n"),
        "slice bound",
    );
}

#[test]
fn boundary_list_append_value_is_t0051() {
    assert_t0051(
        &format!("xs = [1]\nxs.append({OOR})\n"),
        "`list.append()` value",
    );
}

#[test]
fn boundary_dict_get_default_is_t0051() {
    assert_t0051(
        &format!("d = {{\"a\": 1}}\nprint(d.get(\"z\", {OOR}))\n"),
        "`dict.get()` default",
    );
}

#[test]
fn boundary_set_add_value_is_t0051() {
    assert_t0051(&format!("s = {{1}}\ns.add({OOR})\n"), "`set.add()` value");
}

#[test]
fn boundary_dict_subscript_assign_value_is_t0051() {
    assert_t0051(
        &format!("d = {{\"a\": 1}}\nd[\"a\"] = {OOR}\n"),
        "subscript-assign value",
    );
}

#[test]
fn range_argument_out_of_range_is_not_t0051() {
    // D-179 already removed `range` from D-141's runtime int-boundary
    // inventory: range() is fully bigint-capable (bounds, step, and a
    // mid-loop-promoting induction variable), so an out-of-range literal
    // here is ordinary supported behavior, not a T0051 candidate --
    // unlike the other 13 positions this module checks. D-207 records
    // that #618's own filed inventory wrongly copied this position
    // forward from before D-179 excluded it.
    let source = format!("for i in range({OOR}):\n    print(i)\n");
    let module = pycc_parser::parse(&source).expect("test fixture must parse");
    let result = crate::lower_checked(&module);
    assert!(
        result.is_ok(),
        "range() with an out-of-range literal must still lower successfully, got {result:?}"
    );
}

#[test]
fn range_argument_out_of_range_in_a_comprehension_is_not_t0051() {
    let source = format!("xs = [x for x in range({OOR})]\n");
    let module = pycc_parser::parse(&source).expect("test fixture must parse");
    let result = crate::lower_checked(&module);
    assert!(
        result.is_ok(),
        "range() with an out-of-range literal must still lower successfully, got {result:?}"
    );
}

#[test]
fn boundary_listcomp_element_is_t0051() {
    assert_t0051(
        &format!("xs = [1, 2]\nys = [{OOR} for x in xs]\n"),
        "listcomp element",
    );
}

#[test]
fn boundary_setcomp_element_is_t0051() {
    assert_t0051(
        &format!("xs = [1, 2]\nys = {{{OOR} for x in xs}}\n"),
        "setcomp element",
    );
}

#[test]
fn boundary_dictcomp_value_is_t0051() {
    assert_t0051(
        &format!("xs = [1, 2]\nys = {{x: {OOR} for x in xs}}\n"),
        "dictcomp value",
    );
}

#[test]
fn boundary_str_repeat_count_with_literal_string_on_the_left_is_t0051() {
    assert_t0051(&format!("s = \"ab\" * {OOR}\n"), "`str` repeat count");
}

#[test]
fn boundary_str_repeat_count_with_literal_string_on_the_right_is_t0051() {
    assert_t0051(&format!("s = {OOR} * \"ab\"\n"), "`str` repeat count");
}

// #618 criterion 2: `int * int` multiplication (no string operand at
// all) is completely unaffected -- an out-of-range literal there is
// still valid Python that D-178 materializes as a heap bigint, not a
// boundary position.
#[test]
fn plain_int_multiplication_with_an_out_of_range_literal_still_lowers() {
    let module = pycc_parser::parse(&format!("x = 2\ny = x * {OOR}\n")).expect("must parse");
    crate::lower_checked(&module).expect("plain int*int multiplication must not be rejected");
}

// #618 criterion 2: a `str`-typed *variable* multiplied by an
// out-of-range literal is the documented, narrower out-of-scope gap
// (see `crate::int_boundary`'s module doc comment) -- `pycc_hir` has no
// type information at this lowering step to tell it apart from
// ordinary `int * int` multiplication, so it still lowers successfully
// today (and keeps aborting at run time, unchanged from before this
// issue).
#[test]
fn str_typed_variable_repeat_count_out_of_range_is_not_yet_caught_here() {
    let module = pycc_parser::parse(&format!("s = \"ab\"\nt = s * {OOR}\n")).expect("must parse");
    crate::lower_checked(&module)
        .expect("a str-typed variable's repeat count is a documented out-of-scope gap, not yet rejected at HIR-lowering time");
}
