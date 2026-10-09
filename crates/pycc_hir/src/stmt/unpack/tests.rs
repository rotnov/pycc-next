//! Unit tests for the tuple-unpacking lowering (Part 1 of #891): the
//! admitted flat-name shape at module level, in a function and inside a
//! chain, and every refusal on its own span.

use crate::expr::rename_name_in_expr;
use crate::{HirExpr, HirItem, HirStmt, lower_checked};
use pycc_diag::{Diagnostic, Span};

fn lower_err(source: &str) -> Diagnostic {
    let module = crate::pycc_parser_test_helper::parse(source);
    lower_checked(&module).unwrap_err()
}

/// The module-level statements `source` lowers to.
fn top_level(source: &str) -> Vec<HirStmt> {
    let module = crate::pycc_parser_test_helper::parse(source);
    lower_checked(&module)
        .expect("test fixture should lower successfully")
        .items
        .into_iter()
        .filter_map(|item| match item {
            HirItem::TopLevelStmt(stmt) => Some(stmt),
            _ => None,
        })
        .collect()
}

/// The span of the first occurrence of `needle` in `source`.
fn span_of(source: &str, needle: &str) -> Span {
    let start = source.find(needle).expect("source carries the needle");
    let start = u32::try_from(start).expect("test source fits a span");
    Span::new(start, start + u32::try_from(needle.len()).unwrap())
}

fn assert_refused(source: &str, message: &str, needle: &str) {
    let diagnostic = lower_err(source);
    assert_eq!(diagnostic.code, "C0001", "source: {source:?}");
    assert_eq!(diagnostic.message, message, "source: {source:?}");
    assert_eq!(
        diagnostic.span,
        Some(span_of(source, needle)),
        "source: {source:?}"
    );
}

fn assign(target: &str, value: HirExpr) -> HirStmt {
    HirStmt::Assign {
        target: target.to_string(),
        value,
    }
}

fn element(temp: &str, index: i64) -> HirExpr {
    HirExpr::Subscript {
        base: Box::new(HirExpr::Name(temp.to_string())),
        index: Box::new(HirExpr::IntLiteral(index)),
    }
}

fn unpack(value: HirExpr, arity: usize) -> HirExpr {
    HirExpr::Unpack {
        value: Box::new(value),
        arity,
    }
}

#[test]
fn a_flat_tuple_target_binds_a_temporary_then_each_name_left_to_right() {
    let source = "t = (1, 2)\na, b = t\n";
    let stmts = top_level(source);
    let temp = "0unpack_11";
    assert_eq!(
        stmts[1..],
        [
            assign(temp, unpack(HirExpr::Name("t".to_string()), 2)),
            assign("a", element(temp, 0)),
            assign("b", element(temp, 1)),
        ]
    );
}

#[test]
fn a_list_display_target_lowers_like_a_tuple_target() {
    let stmts = top_level("[a, b, c] = t\n");
    assert_eq!(
        stmts,
        [
            assign("0unpack_0", unpack(HirExpr::Name("t".to_string()), 3)),
            assign("a", element("0unpack_0", 0)),
            assign("b", element("0unpack_0", 1)),
            assign("c", element("0unpack_0", 2)),
        ]
    );
}

#[test]
fn a_one_element_target_still_unpacks() {
    // `a, = t` is an arity-1 unpack, not a plain assignment.
    let stmts = top_level("a, = t\n");
    assert_eq!(
        stmts,
        [
            assign("0unpack_0", unpack(HirExpr::Name("t".to_string()), 1)),
            assign("a", element("0unpack_0", 0)),
        ]
    );
}

#[test]
fn a_swap_reads_both_sources_before_any_target_is_bound() {
    // The value tuple `b, a` is built inside the `Unpack` before `a` is
    // rebound, so the swap reads the old values.
    let stmts = top_level("a, b = b, a\n");
    let HirStmt::Assign { target, value } = &stmts[0] else {
        panic!("expected an assignment, got {:?}", stmts[0]);
    };
    assert_eq!(target, "0unpack_0");
    let HirExpr::Unpack { value, arity: 2 } = value else {
        panic!("expected an arity-2 unpack, got {value:?}");
    };
    assert!(matches!(**value, HirExpr::TupleLiteral(_)), "{value:?}");
    assert_eq!(stmts[1], assign("a", element("0unpack_0", 0)));
}

#[test]
fn an_unpack_inside_a_function_body_lowers_in_place() {
    let module = crate::pycc_parser_test_helper::parse(
        "def f(t: tuple[int, int]) -> int:\n    x, y = t\n    return x\n",
    );
    let hir = lower_checked(&module).unwrap();
    let body = hir
        .items
        .iter()
        .find_map(|item| match item {
            HirItem::Function { name, body, .. } if name == "f" => Some(body),
            _ => None,
        })
        .expect("f is lowered");
    assert_eq!(
        body[0],
        assign("0unpack_38", unpack(HirExpr::Name("t".to_string()), 2))
    );
    assert_eq!(body[2], assign("y", element("0unpack_38", 1)));
}

#[test]
fn a_tuple_piece_of_a_chained_assignment_unpacks_the_chain_temporary() {
    // `x = a, b = g()`: the chain binds `g()` once, `x` gets it, then the
    // tuple piece unpacks the same temporary.
    let stmts = top_level("def g() -> int:\n    return 1\nx = a, b = g()\n");
    let targets: Vec<&str> = stmts
        .iter()
        .map(|stmt| match stmt {
            HirStmt::Assign { target, .. } => target.as_str(),
            other => panic!("expected an assignment, got {other:?}"),
        })
        .collect();
    assert_eq!(targets, ["0chain_29", "x", "0unpack_29", "a", "b"]);
    assert_eq!(
        stmts[2],
        assign(
            "0unpack_29",
            unpack(HirExpr::Name("0chain_29".to_string()), 2)
        )
    );
}

#[test]
fn a_starred_element_is_refused_on_its_span() {
    assert_refused(
        "a, *b = t\n",
        "a starred target (`*rest`) in a tuple-unpacking assignment is not supported yet",
        "*b",
    );
}

#[test]
fn a_nested_tuple_or_list_element_is_refused_on_its_span() {
    let message =
        "a nested target (`a, (b, c) = ...`) in a tuple-unpacking assignment is not supported yet";
    assert_refused("a, (b, c) = t\n", message, "(b, c)");
    assert_refused("a, [b, c] = t\n", message, "[b, c]");
}

#[test]
fn an_attribute_or_subscript_element_is_refused_naming_its_kind() {
    let attribute = lower_err("a, o.x = t\n");
    assert_eq!(attribute.code, "C0001");
    assert!(
        attribute.message.starts_with(
            "only bare-name targets in a tuple-unpacking assignment are supported so far, got an attribute"
        ),
        "{}",
        attribute.message
    );
    assert_eq!(attribute.span, Some(span_of("a, o.x = t\n", "o.x")));
    let subscript = lower_err("a, d[0] = t\n");
    assert!(
        subscript.message.starts_with(
            "only bare-name targets in a tuple-unpacking assignment are supported so far, got a subscript"
        ),
        "{}",
        subscript.message
    );
    assert_eq!(subscript.span, Some(span_of("a, d[0] = t\n", "d[0]")));
}

#[test]
fn an_empty_target_is_refused() {
    assert_refused(
        "() = t\n",
        "an empty unpacking target (`() = ...` or `[] = ...`) is not supported",
        "()",
    );
    assert_refused(
        "[] = t\n",
        "an empty unpacking target (`() = ...` or `[] = ...`) is not supported",
        "[]",
    );
}

#[test]
fn a_refused_element_is_reported_before_the_value_is_lowered() {
    // The value (`lambda`) would itself be refused; the target refusal
    // wins because the elements are checked first.
    let diagnostic = lower_err("a, *b = lambda: 1\n");
    assert!(
        diagnostic.message.starts_with("a starred target"),
        "{}",
        diagnostic.message
    );
}

#[test]
fn a_walrus_in_the_unpacked_value_is_refused() {
    assert_refused(
        "a, b = (c := t)\n",
        "a walrus assignment (`:=`) is only supported in an `if`/`while` condition or as a \
         bare expression statement (#774)",
        "a, b = (c := t)",
    );
}

#[test]
fn renaming_reaches_the_value_of_an_unpack() {
    let renamed = rename_name_in_expr(unpack(HirExpr::Name("i".to_string()), 2), "i", "k");
    assert_eq!(renamed, unpack(HirExpr::Name("k".to_string()), 2));
}
