//! Source locations for type-checker diagnostics (#10).
//!
//! HIR nodes carry no spans: thousands of construction and match sites
//! across the HIR, type checker and MIR would have to change to add one.
//! Instead the source position of a HIR node is recovered on demand, after
//! the fact, by matching the HIR against the AST it was lowered from:
//!
//! * [`item_spans`] walks the (desugared) AST of each item and records,
//!   for every statement in pre-order, a [`NodeKey`] -- what kind of
//!   statement it is and which name it binds -- with its span, and for
//!   every expression the statement owns (not those of nested statements)
//!   a key such as `n:x` (a read of `x`) or `c:fib` (a call of `fib`).
//! * [`locate_item`] walks the HIR item in the same order, computes the
//!   same keys, and pairs the `k`-th HIR node with a given key with the
//!   `k`-th AST node with that key: first the statements of the item, then
//!   the expressions within each paired statement.
//!
//! Lowering rewrites some constructs (an `assert` becomes an `if`, `x += 1`
//! becomes `x = x + 1`, a chained assignment gains a temporary); the AST
//! side models the rewrites that matter, and any key whose occurrence
//! counts still differ between the two sides is left unpaired rather than
//! guessed (unless the AST side has exactly one candidate), so a node is
//! either placed at its own source or not placed at all. The type checker
//! places an unplaced node's diagnostic at the closest enclosing node that
//! is placed, then at the item's header.
//!
//! The walk runs only when a program has a type error (the driver
//! re-parses the failing files then), so a successful build pays nothing.

mod ast_side;
mod hir_side;
#[cfg(test)]
mod tests;

use crate::{HirExpr, HirItem, HirStmt};
use pycc_diag::Span;
use std::collections::HashMap;

pub use ast_side::item_spans;

/// What a statement or expression is, as far as pairing HIR with AST needs:
/// a short tag (`if`, `=x`, `n:x`, `c:fib`, ...). Two nodes with the same
/// key are told apart only by their order.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NodeKey(Box<str>);

impl NodeKey {
    fn new(key: impl Into<Box<str>>) -> Self {
        Self(key.into())
    }
}

/// One AST statement: its key, its span (the header of a compound
/// statement, so a caret stays on one line) and the keys and spans of the
/// expressions it owns, in source order.
#[derive(Debug, Clone, PartialEq)]
struct StmtSpans {
    key: NodeKey,
    span: Option<Span>,
    exprs: Vec<(NodeKey, Option<Span>)>,
}

/// The source positions of one HIR item, recovered from its AST.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ItemSpans {
    /// The item's header: `name(params) -> ret` of a `def`, or the
    /// statement itself for a top-level statement item.
    pub header: Option<Span>,
    /// A function's return annotation.
    pub returns: Option<Span>,
    stmts: Vec<StmtSpans>,
}

/// A HIR node [`locate_item`] placed.
#[derive(Debug, Clone, Copy)]
pub enum Located<'a> {
    Stmt(&'a HirStmt),
    Expr(&'a HirExpr),
}

/// Pairs the nodes of `item` with the source spans in `spans` and reports
/// each placed node to `place`. See the module documentation.
pub fn locate_item<'a>(
    item: &'a HirItem,
    spans: &ItemSpans,
    place: &mut dyn FnMut(Located<'a>, Span),
) {
    let mut hir_stmts: Vec<&'a HirStmt> = Vec::new();
    match item {
        HirItem::Function { body, .. } => hir_side::collect_stmts(body, &mut hir_stmts),
        HirItem::TopLevelStmt(stmt) => {
            hir_side::collect_stmts(std::slice::from_ref(stmt), &mut hir_stmts);
        }
    }
    let hir_keyed: Vec<(Option<NodeKey>, &'a HirStmt)> = hir_stmts
        .into_iter()
        .map(|stmt| (hir_side::stmt_key(stmt), stmt))
        .collect();
    let ast_keyed: Vec<(Option<NodeKey>, &StmtSpans)> = spans
        .stmts
        .iter()
        .map(|stmt| (Some(stmt.key.clone()), stmt))
        .collect();
    for (stmt, ast) in pair(&hir_keyed, &ast_keyed) {
        if let Some(span) = ast.span {
            place(Located::Stmt(stmt), span);
        }
        let mut hir_exprs: Vec<&'a HirExpr> = Vec::new();
        hir_side::stmt_own_exprs(stmt, &mut |expr| {
            hir_side::collect_exprs(expr, &mut hir_exprs);
        });
        let hir_expr_keys: Vec<(Option<NodeKey>, &'a HirExpr)> = hir_exprs
            .into_iter()
            .map(|expr| (hir_side::expr_key(expr), expr))
            .collect();
        let ast_expr_keys: Vec<(Option<NodeKey>, Option<Span>)> = ast
            .exprs
            .iter()
            .map(|(key, span)| (Some(key.clone()), *span))
            .collect();
        for (expr, span) in pair(&hir_expr_keys, &ast_expr_keys) {
            if let Some(span) = span {
                place(Located::Expr(expr), span);
            }
        }
    }
}

/// Pairs HIR nodes with AST nodes of the same key, by order. A key is
/// paired when both sides have the same number of nodes with it, or when
/// the AST side has exactly one (every HIR node with that key came from
/// it); any other key is ambiguous and left unpaired.
fn pair<H: Copy, A: Copy>(
    hir: &[(Option<NodeKey>, H)],
    ast: &[(Option<NodeKey>, A)],
) -> Vec<(H, A)> {
    let mut ast_by_key: HashMap<&NodeKey, Vec<A>> = HashMap::new();
    for (key, node) in ast {
        if let Some(key) = key {
            ast_by_key.entry(key).or_default().push(*node);
        }
    }
    let mut hir_counts: HashMap<&NodeKey, usize> = HashMap::new();
    for (key, _) in hir {
        if let Some(key) = key {
            *hir_counts.entry(key).or_default() += 1;
        }
    }
    let mut seen: HashMap<&NodeKey, usize> = HashMap::new();
    let mut paired = Vec::new();
    for (key, node) in hir {
        let Some(key) = key else { continue };
        let Some(candidates) = ast_by_key.get(key) else {
            continue;
        };
        let ordinal = seen.entry(key).or_default();
        let index = *ordinal;
        *ordinal += 1;
        if candidates.len() == 1 {
            paired.push((*node, candidates[0]));
        } else if candidates.len() == hir_counts[key] {
            paired.push((*node, candidates[index]));
        }
    }
    paired
}
