//! An empty list display no evidence typed, passed straight to a builtin
//! that only measures, renders, tests or copies it (`print([])`,
//! `len([])`, `sorted([])`, `bool([])`), or repeated (`[] * n`).
//!
//! No operation of such a call can observe the list's element type, so the
//! display is a `list[int]` (strict mode, after every other source of
//! evidence had its turn); a display bound to a name, returned or stored
//! keeps `T0003`, which names the binding to annotate.

use super::unify::walk::{stmt_exprs_mut, walk_expr_mut};
use crate::empty_container::for_each_stmt_mut;
use pycc_hir::{BinOpKind, HirExpr, HirItem, HirModule, Ty};
use std::collections::BTreeSet;

/// The builtins whose result does not depend on an argument list's element
/// type when the list is empty.
const CONSUMERS: &[&str] = &[
    "print", "len", "bool", "str", "repr", "sorted", "list", "reversed", "sum", "any", "all",
    "tuple", "set", "frozenset", "enumerate", "iter",
];

/// Types every such empty display of `hir` as an empty `list[int]`.
pub(super) fn default_consumed_empty_lists(hir: &mut HirModule) {
    let shadowed: BTreeSet<String> = hir
        .items
        .iter()
        .filter_map(|item| match item {
            HirItem::Function { name, .. } => Some(name.clone()),
            HirItem::TopLevelStmt(_) => None,
        })
        .chain(hir.class_defs.iter().map(|(name, _)| name.clone()))
        .collect();
    let mut visit = |expr: &mut HirExpr| {
        walk_expr_mut(expr, &mut |node| {
            match node {
                HirExpr::Call { callee, args }
                    if CONSUMERS.contains(&callee.as_str()) && !shadowed.contains(callee) =>
                {
                    args.iter_mut().for_each(default_if_empty);
                }
                HirExpr::BinOp {
                    op: BinOpKind::Mul,
                    left,
                    right,
                } => {
                    default_if_empty(left);
                    default_if_empty(right);
                }
                _ => {}
            }
            false
        });
    };
    for item in &mut hir.items {
        match item {
            HirItem::Function { body, .. } => {
                for_each_stmt_mut(body, &mut |stmt| stmt_exprs_mut(stmt, &mut visit));
            }
            HirItem::TopLevelStmt(stmt) => {
                for_each_stmt_mut(std::slice::from_mut(stmt), &mut |stmt| {
                    stmt_exprs_mut(stmt, &mut visit)
                });
            }
        }
    }
}

fn default_if_empty(expr: &mut HirExpr) {
    if matches!(expr, HirExpr::ListLiteral(elements) if elements.is_empty()) {
        *expr = HirExpr::EmptyList(Ty::Int);
    }
}
