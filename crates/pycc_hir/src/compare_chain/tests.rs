//! Unit tests for chained-comparison lowering (#1212): the node shape, the
//! per-link `is None` gate, the walrus rule, and the walkers' chain arms.

use super::*;
use crate::expr::rename_name_in_expr;
use crate::{HirItem, HirStmt, lower_checked, optional_none_test, pycc_parser_test_helper};

fn name(n: &str) -> HirExpr {
    HirExpr::Name(n.to_string())
}

fn link(op: CmpOpKind, right: HirExpr) -> CompareLink {
    CompareLink { op, right }
}

fn chain(first: HirExpr, links: Vec<CompareLink>) -> HirExpr {
    HirExpr::CompareChain {
        first: Box::new(first),
        links,
    }
}

/// The body of the first function `source` defines.
fn function_body(source: &str) -> Vec<HirStmt> {
    let module = lower_checked(&pycc_parser_test_helper::parse(source)).unwrap();
    module
        .items
        .into_iter()
        .find_map(|item| match item {
            HirItem::Function { body, .. } => Some(body),
            HirItem::TopLevelStmt(_) => None,
        })
        .expect("the source defines a function")
}

/// The value of the first statement of the first function, which must be
/// a `return <value>`.
fn returned(source: &str) -> HirExpr {
    match function_body(source).remove(0) {
        HirStmt::Return(Some(value)) => value,
        other => panic!("expected a return, got {other:?}"),
    }
}

fn lowering_error(source: &str) -> pycc_diag::Diagnostic {
    lower_checked(&pycc_parser_test_helper::parse(source)).unwrap_err()
}

#[test]
fn a_two_operator_chain_lowers_to_one_chain_node() {
    assert_eq!(
        returned("def f(a: int, b: int, c: int) -> bool:\n    return a < b <= c\n"),
        chain(
            name("a"),
            vec![
                link(CmpOpKind::Lt, name("b")),
                link(CmpOpKind::LtE, name("c"))
            ]
        )
    );
}

#[test]
fn a_four_operand_chain_keeps_every_link_in_order() {
    assert_eq!(
        returned("def f(a: int, b: int, c: int, d: int) -> bool:\n    return a == b != c > d\n"),
        chain(
            name("a"),
            vec![
                link(CmpOpKind::Eq, name("b")),
                link(CmpOpKind::NotEq, name("c")),
                link(CmpOpKind::Gt, name("d")),
            ]
        )
    );
}

#[test]
fn a_single_comparison_stays_a_compare_node() {
    assert_eq!(
        returned("def f(a: int, b: int) -> bool:\n    return a >= b\n"),
        HirExpr::Compare {
            op: CmpOpKind::GtE,
            left: Box::new(name("a")),
            right: Box::new(name("b")),
        }
    );
}

#[test]
fn the_is_none_gate_is_applied_per_link() {
    // `a is None` then `None is None`: each link has a `None` literal.
    assert_eq!(
        returned("def f(a: int | None) -> bool:\n    return a is None is not None\n"),
        chain(
            name("a"),
            vec![
                link(CmpOpKind::Is, HirExpr::NoneLiteral),
                link(CmpOpKind::IsNot, HirExpr::NoneLiteral),
            ]
        )
    );
    // Part 11 of #1371: `a is b` has no `None` operand, but neither side
    // is a literal, so the link lowers and `pycc_types` decides (a CPython
    // object operand gets the chain `I0404`, a native pair `C0001`).
    assert_eq!(
        returned("def f(a: int, b: int) -> bool:\n    return a is b < None\n"),
        chain(
            name("a"),
            vec![
                link(CmpOpKind::Is, name("b")),
                link(CmpOpKind::Lt, HirExpr::NoneLiteral),
            ]
        )
    );
    assert_eq!(
        returned("def f(a: int, b: int) -> bool:\n    return a < b is not a\n"),
        chain(
            name("a"),
            vec![
                link(CmpOpKind::Lt, name("b")),
                link(CmpOpKind::IsNot, name("a"))
            ]
        )
    );
    // A literal operand of an identity link keeps the located rejection.
    let error = lowering_error("def f(a: int, b: int) -> bool:\n    return a < b is 1\n");
    assert_eq!(error.code, "C0001");
    assert!(
        error
            .message
            .contains("comparison operator not supported yet: Is")
    );
    let error = lowering_error("def f(a: int, b: int) -> bool:\n    return 's' is not a < b\n");
    assert_eq!(error.code, "C0001");
    assert!(
        error
            .message
            .contains("comparison operator not supported yet: IsNot")
    );
}

#[test]
fn in_inside_a_chain_keeps_its_c0001() {
    let error = lowering_error("def f(a: int, b: list[int]) -> bool:\n    return 0 < a in b\n");
    assert_eq!(error.code, "C0001");
    assert!(
        error
            .message
            .contains("comparison operator not supported yet: In")
    );
}

#[test]
fn a_walrus_in_operand_one_is_admitted() {
    let body = function_body(
        "def f(a: int, b: int) -> int:\n    if 0 < (n := a) < b:\n        return n\n    return 0\n",
    );
    let HirStmt::If { test, .. } = &body[0] else {
        panic!("expected an if, got {:?}", body[0]);
    };
    assert_eq!(
        *test,
        chain(
            HirExpr::IntLiteral(0),
            vec![
                link(
                    CmpOpKind::Lt,
                    HirExpr::NamedExpr {
                        name: "n".to_string(),
                        value: Box::new(name("a")),
                    }
                ),
                link(CmpOpKind::Lt, name("b")),
            ]
        )
    );
}

#[test]
fn a_walrus_in_operand_two_or_later_is_refused() {
    let source =
        "def f(a: int, b: int) -> int:\n    if 0 < a < (n := b):\n        return n\n    return 0\n";
    let error = lowering_error(source);
    assert_eq!(error.code, "C0001");
    assert!(
        error
            .message
            .contains("a walrus assignment (`:=`) in a short-circuited chained-comparison operand")
    );
    let span = error.span.expect("the refusal carries a span");
    assert_eq!(&source[span.start as usize..span.end as usize], "n := b");
}

#[test]
fn a_chain_is_not_a_narrowing_test() {
    let test = returned("def f(a: int | None) -> bool:\n    return a is not None is not None\n");
    assert_eq!(optional_none_test(&test), None);
}

#[test]
fn rename_visits_every_chain_operand() {
    let renamed = rename_name_in_expr(
        chain(
            name("x"),
            vec![
                link(CmpOpKind::Lt, name("x")),
                link(CmpOpKind::Lt, name("y")),
            ],
        ),
        "x",
        "z",
    );
    assert_eq!(
        renamed,
        chain(
            name("z"),
            vec![
                link(CmpOpKind::Lt, name("z")),
                link(CmpOpKind::Lt, name("y"))
            ]
        )
    );
}

#[test]
fn contains_named_expr_sees_the_first_operand_and_every_link() {
    let walrus = HirExpr::NamedExpr {
        name: "n".to_string(),
        value: Box::new(name("a")),
    };
    assert!(contains_named_expr(&chain(
        walrus.clone(),
        vec![
            link(CmpOpKind::Lt, name("b")),
            link(CmpOpKind::Lt, name("c"))
        ]
    )));
    assert!(contains_named_expr(&chain(
        name("a"),
        vec![link(CmpOpKind::Lt, name("b")), link(CmpOpKind::Lt, walrus)]
    )));
    assert!(!contains_named_expr(&chain(
        name("a"),
        vec![
            link(CmpOpKind::Lt, name("b")),
            link(CmpOpKind::Lt, name("c"))
        ]
    )));
}

#[test]
fn a_walrus_in_a_chain_operand_is_killed_for_module_constant_folding() {
    // A module-level walrus inside a chain rebinds its target, so the
    // `killed_names` prescan must see it in the first operand and in a
    // later one.
    for (first, later) in [
        (
            HirExpr::NamedExpr {
                name: "n".to_string(),
                value: Box::new(HirExpr::IntLiteral(2)),
            },
            name("b"),
        ),
        (
            name("a"),
            HirExpr::NamedExpr {
                name: "n".to_string(),
                value: Box::new(HirExpr::IntLiteral(2)),
            },
        ),
    ] {
        let body = [HirStmt::If {
            test: chain(first, vec![link(CmpOpKind::Lt, later)]),
            body: vec![],
            orelse: vec![],
        }];
        assert_eq!(
            crate::killed_names(&body),
            std::collections::HashSet::from(["n".to_string()])
        );
    }
}

/// Part 1 of #1371: a single `is`/`is not` between two non-literal operands
/// lowers, because either may be a CPython object (`pycc_types` decides),
/// while `optional_none_test` still recognizes only a `None` side.
#[test]
fn a_single_general_identity_between_non_literals_lowers() {
    for (source_op, op) in [("is", CmpOpKind::Is), ("is not", CmpOpKind::IsNot)] {
        let value = returned(&format!(
            "def f(a: int, b: int) -> bool:\n    return a {source_op} b\n"
        ));
        assert_eq!(
            value,
            HirExpr::Compare {
                op,
                left: Box::new(name("a")),
                right: Box::new(name("b")),
            }
        );
        assert_eq!(optional_none_test(&value), None);
    }
}

/// A literal operand is never a CPython object, so it keeps the located
/// HIR rejection on either side, for every literal kind.
#[test]
fn a_single_identity_with_a_literal_operand_keeps_its_c0001() {
    for operand in ["1", "1.5", "'s'", "b's'", "True", "..."] {
        for source in [
            format!("def f(a: int) -> bool:\n    return a is {operand}\n"),
            format!("def f(a: int) -> bool:\n    return {operand} is not a\n"),
        ] {
            let error = lowering_error(&source);
            assert_eq!(error.code, "C0001", "{source}");
            assert!(
                error
                    .message
                    .contains("comparison operator not supported yet: Is"),
                "{source}: {}",
                error.message
            );
        }
    }
}

/// Part 2b of #1371: a single `in`/`not in` whose container may be a
/// CPython object lowers -- `pycc_types` decides -- and a literal *item*
/// does not stop it (`"k" in callbacks`).
#[test]
fn a_single_membership_test_against_a_possible_object_lowers() {
    for (source_op, op) in [("in", CmpOpKind::In), ("not in", CmpOpKind::NotIn)] {
        for (item_src, item) in [
            ("a", name("a")),
            ("'k'", HirExpr::StringLiteral("k".to_string())),
        ] {
            let value = returned(&format!(
                "def f(a: int, b: int) -> bool:\n    return {item_src} {source_op} b\n"
            ));
            assert_eq!(
                value,
                HirExpr::Compare {
                    op,
                    left: Box::new(item),
                    right: Box::new(name("b")),
                }
            );
        }
    }
}

/// A literal, display or comprehension container always builds a native
/// value, so it keeps the located HIR rejection.
#[test]
fn a_membership_test_against_a_native_display_keeps_its_c0001() {
    for container in [
        "1",
        "'s'",
        "[1]",
        "(1,)",
        "{1}",
        "{1: 2}",
        "[x for x in a]",
        "{x for x in a}",
        "{x: x for x in a}",
        "f'{a}'",
    ] {
        for (source_op, needle) in [("in", "In"), ("not in", "NotIn")] {
            let source =
                format!("def f(a: list[int]) -> bool:\n    return 1 {source_op} {container}\n");
            let error = lowering_error(&source);
            assert_eq!(error.code, "C0001", "{source}");
            assert!(
                error
                    .message
                    .ends_with(&format!("comparison operator not supported yet: {needle}")),
                "{source}: {}",
                error.message
            );
        }
    }
}
