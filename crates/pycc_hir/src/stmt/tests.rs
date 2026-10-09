//! Unit tests for `stmt.rs`, moved out of that file's inline `mod tests`
//! per AGENTS.md's file-decomposition rule (issue #890). Test names and
//! bodies are unchanged.

use super::*;
use crate::HirItem;
use crate::expr::keyword_bind::SignatureTable;

#[test]
fn lower_pattern_rejects_bare_match_star() {
    let star = Pattern::MatchStar(pycc_ast::PatternMatchStar {
        node_index: Default::default(),
        range: Default::default(),
        name: None,
    });
    let err = lower_pattern(&star, false, None, &[], &SignatureTable::default()).unwrap_err();
    assert_eq!(err.code, "C0001");
    assert!(
        err.message
            .contains("a `*` pattern is only valid inside a sequence pattern")
    );
}

#[test]
fn lower_pattern_as_with_no_name_produces_empty_name() {
    let inner = Pattern::MatchValue(pycc_ast::PatternMatchValue {
        node_index: Default::default(),
        range: Default::default(),
        value: Box::new(Expr::NumberLiteral(pycc_ast::ExprNumberLiteral {
            node_index: Default::default(),
            range: Default::default(),
            value: pycc_ast::Number::Float(1.0),
        })),
    });
    let as_pat = Pattern::MatchAs(pycc_ast::PatternMatchAs {
        node_index: Default::default(),
        range: Default::default(),
        pattern: Some(Box::new(inner)),
        name: None,
    });
    let result = lower_pattern(&as_pat, false, None, &[], &SignatureTable::default()).unwrap();
    assert_eq!(
        result,
        HirPattern::As(
            Box::new(HirPattern::Literal(HirExpr::FloatLiteral(1.0))),
            String::new(),
        )
    );
}

#[test]
fn lower_match_with_unsupported_body_emits_c0001() {
    let module = pycc_parser::parse(
        "x = 1\nmatch x:\n    case 1:\n        while True:\n            pass\n        else:\n            pass\n    case _:\n        pass\n",
    ).expect("test fixture must parse");
    let err = crate::lower_checked(&module).unwrap_err();
    assert_eq!(err.code, "C0001");
}

#[test]
fn lower_match_subject_expr_error_propagates() {
    // `{**x}` is a dict-unpacking expression that `lower_expr` rejects
    // with C0001; used as the match subject it propagates through the
    // `?` on the subject expression.
    let module = pycc_parser::parse("match {**x}:\n    case _:\n        pass\n")
        .expect("test fixture must parse");
    let err = crate::lower_checked(&module).unwrap_err();
    assert_eq!(err.code, "C0001");
}

#[test]
fn lower_match_guard_expr_error_propagates() {
    // `{**y}` as a guard expression causes `lower_expr` to fail with
    // C0001, propagating through the `?` on the guard.
    let module = pycc_parser::parse(
        "x = 1\nmatch x:\n    case 1 if {**y}:\n        pass\n    case _:\n        pass\n",
    )
    .expect("test fixture must parse");
    let err = crate::lower_checked(&module).unwrap_err();
    assert_eq!(err.code, "C0001");
}

#[test]
fn lower_match_value_pattern_folds_a_negative_literal() {
    // #602: `case -1:` is a value pattern whose expression is `USub`
    // applied to the literal `1`. The fold makes it an ordinary
    // `HirExpr::IntLiteral(-1)`, so it is an accepted literal pattern.
    let module = pycc_parser::parse(
        "x = 1\nmatch x:\n    case -1:\n        pass\n    case _:\n        pass\n",
    )
    .expect("test fixture must parse");
    let hir = crate::lower_checked(&module).expect("a negative literal pattern must lower");
    assert!(matches!(
        &hir.items[1],
        HirItem::TopLevelStmt(HirStmt::Match { cases, .. })
            if cases[0].pattern == HirPattern::Literal(HirExpr::IntLiteral(-1))
    ));
}

#[test]
fn lower_match_value_pattern_expr_error_propagates() {
    // A magnitude past `i64`'s range still fails in `lower_expr`, so the
    // `?` on the value pattern's own expression keeps its error path
    // covered after #602 made `case -1:` succeed.
    let module = pycc_parser::parse(
        "x = 1\nmatch x:\n    case -99999999999999999999999:\n        pass\n    case _:\n        pass\n",
    )
    .expect("test fixture must parse");
    let err = crate::lower_checked(&module).unwrap_err();
    assert_eq!(err.code, "C0001");
}

#[test]
fn lower_match_mapping_key_folds_a_negative_literal() {
    // #602: a mapping key `-1` folds to `HirExpr::IntLiteral(-1)`.
    let module = pycc_parser::parse(
        "x = {1: 2}\nmatch x:\n    case {-1: v}:\n        pass\n    case _:\n        pass\n",
    )
    .expect("test fixture must parse");
    let hir = crate::lower_checked(&module).expect("a negative mapping key must lower");
    assert!(matches!(
        &hir.items[1],
        HirItem::TopLevelStmt(HirStmt::Match { cases, .. })
            if matches!(
                &cases[0].pattern,
                HirPattern::Mapping(entries, _)
                    if entries[0].0 == HirExpr::IntLiteral(-1)
            )
    ));
}

#[test]
fn lower_match_mapping_key_expr_error_propagates() {
    // As above, an out-of-range magnitude keeps the mapping key's own
    // `?` error path covered now that `-1` folds successfully.
    let module = pycc_parser::parse(
        "x = {1: 2}\nmatch x:\n    case {-99999999999999999999999: v}:\n        pass\n    case _:\n        pass\n",
    )
    .expect("test fixture must parse");
    let err = crate::lower_checked(&module).unwrap_err();
    assert_eq!(err.code, "C0001");
}

#[test]
fn lower_match_sequence_subpattern_error_propagates() {
    let module = pycc_parser::parse(
        "x = [1]\nmatch x:\n    case [foo.bar]:\n        pass\n    case _:\n        pass\n",
    )
    .expect("test fixture must parse");
    let err = crate::lower_checked(&module).unwrap_err();
    assert_eq!(err.code, "C0001");
}

#[test]
fn lower_match_sequence_star_subpattern_error_propagates() {
    let module = pycc_parser::parse(
        "x = [1]\nmatch x:\n    case [foo.bar, *rest]:\n        pass\n    case _:\n        pass\n",
    )
    .expect("test fixture must parse");
    let err = crate::lower_checked(&module).unwrap_err();
    assert_eq!(err.code, "C0001");
}

#[test]
fn lower_match_mapping_value_pattern_error_propagates() {
    let module = pycc_parser::parse(
        "x = {\"k\": 1}\nmatch x:\n    case {\"k\": foo.bar}:\n        pass\n    case _:\n        pass\n",
    ).expect("test fixture must parse");
    let err = crate::lower_checked(&module).unwrap_err();
    assert_eq!(err.code, "C0001");
}

#[test]
fn lower_match_class_positional_subpattern_error_propagates() {
    let module = pycc_parser::parse(
        "class P:\n    def __init__(self):\n        pass\nx = P()\nmatch x:\n    case P(foo.bar):\n        pass\n    case _:\n        pass\n",
    ).expect("test fixture must parse");
    let err = crate::lower_checked(&module).unwrap_err();
    assert_eq!(err.code, "C0001");
}

#[test]
fn lower_match_class_keyword_subpattern_error_propagates() {
    let module = pycc_parser::parse(
        "class P:\n    def __init__(self):\n        pass\nx = P()\nmatch x:\n    case P(a=foo.bar):\n        pass\n    case _:\n        pass\n",
    ).expect("test fixture must parse");
    let err = crate::lower_checked(&module).unwrap_err();
    assert_eq!(err.code, "C0001");
}

#[test]
fn lower_match_as_pattern_inner_error_propagates() {
    let module = pycc_parser::parse(
        "x = 1\nmatch x:\n    case foo.bar as y:\n        pass\n    case _:\n        pass\n",
    )
    .expect("test fixture must parse");
    let err = crate::lower_checked(&module).unwrap_err();
    assert_eq!(err.code, "C0001");
}

#[test]
fn lower_match_or_pattern_subpattern_error_propagates() {
    let module = pycc_parser::parse(
        "x = 1\nmatch x:\n    case foo.bar | 1:\n        pass\n    case _:\n        pass\n",
    )
    .expect("test fixture must parse");
    let err = crate::lower_checked(&module).unwrap_err();
    assert_eq!(err.code, "C0001");
}

/// Part 1 of #1255: `comp_assign_stmt` routes a `CompIter::Iterable`
/// comprehension to a plain `Assign`, so no statement form carries one
/// today; the walrus scan still walks the iterable expression, so a later
/// change routing one through a statement form cannot let a walrus escape
/// it. Pinned directly, since no source reaches the arm.
#[test]
fn the_walrus_scan_walks_an_iterable_comprehension_source() {
    let walrus = HirExpr::NamedExpr {
        name: "n".to_string(),
        value: Box::new(HirExpr::IntLiteral(1)),
    };
    assert!(comp_iter_contains_named_expr(&CompIter::Iterable(
        Box::new(walrus)
    )));
    assert!(!comp_iter_contains_named_expr(&CompIter::Iterable(
        Box::new(HirExpr::IntLiteral(1))
    )));
}
