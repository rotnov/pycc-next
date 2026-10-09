//! `del o[a:b:c]` lowering (Part 2c of #1371) and `del o.x` lowering (#1457).

use crate::{HirExpr, HirItem, HirStmt};
use pycc_diag::Span;

fn lower(source: &str) -> Result<crate::HirModule, pycc_diag::Diagnostic> {
    crate::lower_checked(&pycc_parser::parse(source).expect("test source must parse"))
}

fn top_level(source: &str) -> Vec<HirStmt> {
    lower(source)
        .expect("test source must lower")
        .items
        .into_iter()
        .filter_map(|item| match item {
            HirItem::TopLevelStmt(stmt) => Some(stmt),
            HirItem::Function { .. } => None,
        })
        .collect()
}

fn name(id: &str) -> Box<HirExpr> {
    Box::new(HirExpr::Name(id.to_string()))
}

/// Each bound is lowered where it is present and `None` where it is
/// omitted, and the span is the slice target's own range.
#[test]
fn a_slice_target_lowers_to_one_delete_slice_per_target() {
    let stmts = top_level("o = 1\nn = 2\ndel o[n:], o[:n:n], o[::]\n");
    assert_eq!(
        stmts[2..],
        [
            HirStmt::DeleteSlice {
                base: name("o"),
                start: Some(name("n")),
                stop: None,
                step: None,
                span: Span::new(16, 21),
            },
            HirStmt::DeleteSlice {
                base: name("o"),
                start: None,
                stop: Some(name("n")),
                step: Some(name("n")),
                span: Span::new(23, 30),
            },
            HirStmt::DeleteSlice {
                base: name("o"),
                start: None,
                stop: None,
                step: None,
                span: Span::new(32, 37),
            },
        ]
    );
}

/// A name and a slice in one statement keep CPython's left-to-right order,
/// and deleting a slice of `o` does not unbind `o`.
#[test]
fn a_mixed_del_keeps_source_order_and_unbinds_only_the_name() {
    let stmts = top_level("a = 1\no = 2\ndel a, o[1:]\n");
    assert!(matches!(&stmts[2], HirStmt::Delete { name, .. } if name == "a"));
    assert!(matches!(&stmts[3], HirStmt::DeleteSlice { .. }));
    let deleted = super::deleted_names(&stmts);
    assert!(deleted.contains("a"), "{deleted:?}");
    assert!(!deleted.contains("o"), "{deleted:?}");
}

/// A walrus anywhere in the operands is refused at the slice target.
#[test]
fn a_walrus_operand_is_refused_at_the_slice_target() {
    for source in [
        "o = 1\ndel (n := o)[1:]\n",
        "o = 1\ndel o[(n := 1):]\n",
        "o = 1\ndel o[:(n := 1)]\n",
        "o = 1\ndel o[::(n := 1)]\n",
    ] {
        let err = lower(source).expect_err(source);
        assert_eq!(err.code, "C0001", "{source}");
        assert!(
            err.message.contains("a walrus assignment"),
            "{source}: {}",
            err.message
        );
        assert_eq!(err.span.expect("located").start, 10, "{source}");
    }
}

/// An operand the expression lowering itself refuses keeps that refusal.
#[test]
fn an_unlowerable_operand_keeps_its_own_refusal() {
    for source in [
        "o = 1\ndel (lambda: 1)[1:]\n",
        "o = 1\ndel o[lambda: 1:]\n",
        "o = 1\ndel o[:lambda: 1]\n",
        "o = 1\ndel o[::lambda: 1]\n",
    ] {
        let err = lower(source).expect_err(source);
        assert_eq!(err.code, "C0001", "{source}");
        assert!(!err.message.contains("walrus"), "{source}: {}", err.message);
    }
}

/// A non-slice subscript keeps its own located refusal.
#[test]
fn a_non_slice_subscript_is_still_refused() {
    let err = lower("o = 1\ndel o[1]\n").expect_err("an item del");
    assert_eq!(err.code, "C0001");
    assert!(
        err.message.contains("a `del` of a subscript"),
        "{}",
        err.message
    );
    assert_eq!(err.span.expect("located").start, 10);
}

/// In a function body the bounds are lowered with the function's context.
#[test]
fn a_function_body_slice_target_lowers() {
    let module =
        lower("def f(o: list[int], size: int) -> None:\n    del o[-size:]\n").expect("must lower");
    let HirItem::Function { body, .. } = &module.items[0] else {
        panic!("expected a function: {:?}", module.items);
    };
    assert!(
        matches!(
            &body[0],
            HirStmt::DeleteSlice {
                start: Some(_),
                stop: None,
                step: None,
                ..
            }
        ),
        "{body:?}"
    );
}

/// #1457: an attribute target lowers to one `DeleteAttr` per target, in
/// source order next to the other target kinds, located at the target and
/// unbinding no name.
#[test]
fn an_attribute_target_lowers_to_one_delete_attr_per_target() {
    let stmts = top_level("a = 1\no = 2\ndel a, o.x, (o.y, o[1:])\n");
    assert!(matches!(&stmts[2], HirStmt::Delete { name } if name == "a"));
    assert_eq!(
        stmts[3],
        HirStmt::DeleteAttr {
            base: name("o"),
            attr: "x".to_string(),
            span: Span::new(19, 22),
        }
    );
    assert_eq!(
        stmts[4],
        HirStmt::DeleteAttr {
            base: name("o"),
            attr: "y".to_string(),
            span: Span::new(25, 28),
        }
    );
    assert!(matches!(&stmts[5], HirStmt::DeleteSlice { .. }));
    let deleted = super::deleted_names(&stmts);
    assert!(!deleted.contains("o"), "{deleted:?}");
}

/// A walrus in the base is refused at the attribute target, and a base the
/// expression lowering refuses keeps that refusal.
#[test]
fn an_attribute_target_refuses_a_walrus_or_unlowerable_base() {
    let err = lower("o = 1\ndel (n := o).x\n").expect_err("a walrus base");
    assert_eq!(err.code, "C0001");
    assert!(
        err.message.contains("a walrus assignment"),
        "{}",
        err.message
    );
    assert_eq!(err.span.expect("located").start, 10);
    let err = lower("o = 1\ndel (lambda: 1).x\n").expect_err("a lambda base");
    assert_eq!(err.code, "C0001");
    assert!(!err.message.contains("walrus"), "{}", err.message);
}
