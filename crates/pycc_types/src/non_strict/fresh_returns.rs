//! Non-strict mode: the functions whose every `return` hands back a list
//! nothing else holds, so a call of one is as fresh as a display
//! (`crate::object_box::fresh_container`, `super::container_widen`).
//!
//! A return value qualifies when it is a display, a comprehension or a
//! repetition or concatenation of one, or a local name every binding of
//! which is such a value and whose every other use reads, stores into,
//! appends to, measures or iterates it -- none passes it on, stores it
//! elsewhere or returns it from anywhere but the function itself. A
//! parameter never qualifies: the caller holds it.

use super::container_widen::{fresh, nested};
use pycc_hir::{HirExpr, HirItem, HirModule, HirStmt};
use std::cell::RefCell;
use std::collections::BTreeSet;

thread_local! {
    static FRESH: RefCell<BTreeSet<String>> = const { RefCell::new(BTreeSet::new()) };
}

/// Records the functions of `hir` that return fresh lists (module doc).
pub(super) fn record(hir: &HirModule) {
    let names = hir
        .items
        .iter()
        .filter_map(|item| match item {
            HirItem::Function {
                name, params, body, ..
            } if returns_fresh(params, body) => Some(name.clone()),
            _ => None,
        })
        .collect();
    FRESH.with(|fresh| *fresh.borrow_mut() = names);
}

/// Whether the function `name` returns a fresh list (module doc).
pub(crate) fn is_fresh_returning(name: &str) -> bool {
    FRESH.with(|fresh| fresh.borrow().contains(name))
}

fn returns_fresh(params: &[(String, pycc_hir::Ty)], body: &[HirStmt]) -> bool {
    let mut returned = Vec::new();
    if !collect_returns(body, &mut returned) || returned.is_empty() {
        return false;
    }
    returned.into_iter().all(|value| match value {
        HirExpr::Name(local) => {
            !params.iter().any(|(param, _)| param == local)
                && only_fresh_bindings(body, local)
                && !escapes_in(body, local)
        }
        other => fresh(other),
    })
}

/// Every `return` value of `body`; `false` for a bare `return`.
fn collect_returns<'a>(body: &'a [HirStmt], out: &mut Vec<&'a HirExpr>) -> bool {
    for stmt in body {
        match stmt {
            HirStmt::Return(Some(value)) => out.push(value),
            HirStmt::Return(None) => return false,
            _ => {}
        }
        for inner in nested(stmt) {
            if !collect_returns(inner, out) {
                return false;
            }
        }
    }
    true
}

/// Whether every binding of `local` in `body` is a fresh value.
fn only_fresh_bindings(body: &[HirStmt], local: &str) -> bool {
    body.iter().all(|stmt| {
        let ok = match stmt {
            HirStmt::Assign { target, value } if target == local => fresh(value),
            HirStmt::AnnAssign { target, .. } => target != local,
            HirStmt::ForRange { var, .. }
            | HirStmt::ForList { var, .. }
            | HirStmt::ForObject { var, .. } => var != local,
            HirStmt::Try { handlers, .. } | HirStmt::TryStar { handlers, .. } => handlers
                .iter()
                .all(|handler| handler.name.as_deref() != Some(local)),
            HirStmt::Delete { name } => name != local,
            // A `match` pattern may bind it.
            HirStmt::Match { .. } => false,
            _ => true,
        };
        ok && nested(stmt)
            .into_iter()
            .all(|inner| only_fresh_bindings(inner, local))
    })
}

/// Whether some use of `local` in `body` may hand it on (module doc).
fn escapes_in(body: &[HirStmt], local: &str) -> bool {
    body.iter().any(|stmt| {
        let own = match stmt {
            HirStmt::Return(Some(HirExpr::Name(name))) if name == local => false,
            HirStmt::ForObject { iter, .. } if matches!(iter.as_ref(), HirExpr::Name(n) if n == local) => {
                false
            }
            HirStmt::Assign { value, .. } | HirStmt::ExprStmt(value) => escapes(value, local),
            HirStmt::AnnAssign { value, .. } => value.as_ref().is_some_and(|v| escapes(v, local)),
            HirStmt::If { test, .. } | HirStmt::While { test, .. } => escapes(test, local),
            HirStmt::DictSet { key, value, .. } => escapes(key, local) || escapes(value, local),
            HirStmt::Return(Some(value)) => escapes(value, local),
            HirStmt::ForRange {
                start, stop, step, ..
            } => [start, stop, step].iter().any(|e| escapes(e, local)),
            HirStmt::ForObject { iter, .. } => escapes(iter, local),
            HirStmt::SetSlice {
                base,
                start,
                stop,
                step,
                value,
                ..
            } => {
                !matches!(base.as_ref(), HirExpr::Name(n) if n == local)
                    && escapes(base, local)
                    || [start, stop, step]
                        .into_iter()
                        .flatten()
                        .any(|e| escapes(e, local))
                    || escapes(value, local)
            }
            HirStmt::ListCompAssign {
                iter, cond, elt, ..
            }
            | HirStmt::SetCompAssign {
                iter, cond, elt, ..
            } => {
                comp_iter_escapes(iter, local)
                    || cond.as_ref().is_some_and(|c| escapes(c, local))
                    || escapes(elt, local)
            }
            HirStmt::DictCompAssign {
                iter,
                cond,
                key,
                value,
                ..
            } => {
                comp_iter_escapes(iter, local)
                    || cond.as_ref().is_some_and(|c| escapes(c, local))
                    || escapes(key, local)
                    || escapes(value, local)
            }
            HirStmt::Raise { exc, cause } => [exc, cause]
                .into_iter()
                .flatten()
                .any(|e| escapes(e, local)),
            HirStmt::Return(None)
            | HirStmt::Break
            | HirStmt::Continue
            | HirStmt::Pos(_)
            | HirStmt::ForList { .. }
            | HirStmt::LoopElse { .. }
            | HirStmt::Try { .. }
            | HirStmt::TryStar { .. }
            | HirStmt::Delete { .. } => false,
            // Any other statement may read `local` in a way this walk does
            // not follow; refuse rather than guess.
            _ => true,
        };
        own || nested(stmt)
            .into_iter()
            .any(|inner| escapes_in(inner, local))
    })
}

/// Whether a comprehension's iterable hands `local` on: iterating a name
/// only reads it.
fn comp_iter_escapes(iter: &pycc_hir::CompIter, local: &str) -> bool {
    match iter {
        pycc_hir::CompIter::Range { start, stop, step } => {
            [start, stop, step].iter().any(|e| escapes(e, local))
        }
        pycc_hir::CompIter::Iterable(iterable) => escapes(iterable, local),
        pycc_hir::CompIter::Name(_) => false,
    }
}

/// The list methods that neither keep nor return their receiver.
const KEEPING_NOTHING: &[&str] = &[
    "append", "extend", "insert", "pop", "index", "count", "sort", "reverse", "clear", "remove",
];

/// Whether `expr` holds a use of `local` other than a read of an item, a
/// `len`, or a list method that does not hand the list on.
fn escapes(expr: &HirExpr, local: &str) -> bool {
    let is_local = |e: &HirExpr| matches!(e, HirExpr::Name(n) if n == local);
    match expr {
        HirExpr::Name(name) => name == local,
        // `(local := v)` is a binding this walk does not type.
        HirExpr::NamedExpr { name, .. } if name == local => true,
        HirExpr::Subscript { base, index } if is_local(base) => escapes(index, local),
        HirExpr::Call { callee, args } if callee == "len" && args.len() == 1 && is_local(&args[0]) => {
            false
        }
        HirExpr::MethodCall { base, method, args }
            if is_local(base) && KEEPING_NOTHING.contains(&method.as_str()) =>
        {
            args.iter().any(|arg| escapes(arg, local))
        }
        other => pycc_hir::ndarray::expr_children(other)
            .into_iter()
            .any(|child| escapes(child, local)),
    }
}
