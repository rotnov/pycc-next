//! Placing type diagnostics at their source (#10).
//!
//! Most checker diagnostics are raised far from any source position: the
//! HIR has no spans. The driver hands the spans `pycc_hir::locate`
//! recovers for every item to [`with_item_spans`]; each check entry point
//! then pairs them with the very HIR nodes it is about to check
//! ([`with_locator`]), keyed by node address, which is stable because the
//! module is borrowed for the whole check.
//!
//! A diagnostic leaving the checker without a span is placed on its way
//! out: the innermost expression whose check failed ([`place_expr`]), else
//! its statement ([`place_stmt`]), else the item's header ([`place_item`]).
//! A site that knows better -- the argument of a mismatched call, the value
//! of an incompatible assignment -- places its own diagnostic first
//! ([`expr_span`]), and the outer fallbacks leave a placed one alone.
//!
//! Only nodes of the item being checked place a diagnostic
//! ([`in_item`]): a diagnostic keyed to an item is rendered against that
//! item's file, and a module-level diagnostic, which the driver cannot
//! attribute to a file, stays unplaced.

use pycc_diag::{Diagnostic, Span};
use pycc_hir::locate::{ItemSpans, Located, locate_item};
use pycc_hir::{HirExpr, HirModule, HirStmt};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

thread_local! {
    static ITEM_SPANS: RefCell<Option<Rc<Vec<Option<ItemSpans>>>>> = const { RefCell::new(None) };
    static LOCATORS: RefCell<Vec<Locator>> = const { RefCell::new(Vec::new()) };
    static CURRENT_ITEM: Cell<Option<usize>> = const { Cell::new(None) };
}

/// Where the nodes of one checked module are.
#[derive(Default)]
struct Locator {
    stmts: HashMap<usize, (Span, usize)>,
    exprs: HashMap<usize, (Span, usize)>,
    headers: HashMap<usize, Span>,
    returns: HashMap<usize, Span>,
}

/// Runs `f` with `spans[i]` as the source spans of item `i` of the module
/// it checks (`None` for an item no source statement wrote). Every check
/// entry point run inside places its diagnostics.
pub fn with_item_spans<R>(spans: Vec<Option<ItemSpans>>, f: impl FnOnce() -> R) -> R {
    let previous = ITEM_SPANS.with(|cell| cell.replace(Some(Rc::new(spans))));
    let _restore = Restore(Some(move || {
        ITEM_SPANS.with(|cell| *cell.borrow_mut() = previous)
    }));
    f()
}

/// Runs its closure on drop, so a panic unwinding through a check still
/// restores the thread-local state.
struct Restore<F: FnOnce()>(Option<F>);

impl<F: FnOnce()> Drop for Restore<F> {
    fn drop(&mut self) {
        if let Some(restore) = self.0.take() {
            restore();
        }
    }
}

/// Runs `f` with the nodes of `hir` located. `origin(i)` names the item
/// whose spans item `i` has when it is not item `i` itself (an inherited
/// method's receiver-exact copy, `crate::inherited_copies`).
pub(crate) fn with_locator<R>(
    hir: &HirModule,
    origin: impl Fn(usize) -> Option<usize>,
    f: impl FnOnce() -> R,
) -> R {
    let Some(spans) = ITEM_SPANS.with(|cell| cell.borrow().clone()) else {
        return f();
    };
    let mut locator = Locator::default();
    for (index, item) in hir.items.iter().enumerate() {
        let source = if index < spans.len() {
            Some(index)
        } else {
            origin(index)
        };
        let Some(Some(item_spans)) = source.and_then(|source| spans.get(source)) else {
            continue;
        };
        if let Some(header) = item_spans.header {
            locator.headers.insert(index, header);
        }
        if let Some(returns) = item_spans.returns {
            locator.returns.insert(index, returns);
        }
        locate_item(item, item_spans, &mut |node, span| match node {
            Located::Stmt(stmt) => {
                locator.stmts.insert(stmt_key(stmt), (span, index));
            }
            Located::Expr(expr) => {
                locator.exprs.insert(expr_key(expr), (span, index));
            }
        });
    }
    LOCATORS.with(|cell| cell.borrow_mut().push(locator));
    let _pop = Restore(Some(|| {
        LOCATORS.with(|cell| cell.borrow_mut().pop());
    }));
    f()
}

fn stmt_key(stmt: &HirStmt) -> usize {
    std::ptr::from_ref(stmt) as usize
}

fn expr_key(expr: &HirExpr) -> usize {
    std::ptr::from_ref(expr) as usize
}

/// Runs `f` as the check of item `index`: only its own nodes place
/// diagnostics.
pub(crate) fn in_item<R>(index: usize, f: impl FnOnce() -> R) -> R {
    let previous = CURRENT_ITEM.with(|cell| cell.replace(Some(index)));
    let _restore = Restore(Some(move || CURRENT_ITEM.with(|cell| cell.set(previous))));
    f()
}

/// The index of the item being checked ([`in_item`]), if any.
pub(crate) fn current_item() -> Option<usize> {
    CURRENT_ITEM.with(Cell::get)
}

fn lookup(f: impl FnOnce(&Locator, usize) -> Option<Span>) -> Option<Span> {
    let item = CURRENT_ITEM.with(Cell::get)?;
    LOCATORS.with(|cell| cell.borrow().last().and_then(|locator| f(locator, item)))
}

/// A node's span and the item holding it.
pub(crate) type Location = (Span, usize);

/// Where `expr` is, when it belongs to the item being checked.
pub(crate) fn expr_location(expr: &HirExpr) -> Option<Location> {
    let item = CURRENT_ITEM.with(Cell::get)?;
    expr_span(expr).map(|span| (span, item))
}

/// The span of `expr`, when it belongs to the item being checked.
pub(crate) fn expr_span(expr: &HirExpr) -> Option<Span> {
    lookup(|locator, item| {
        locator
            .exprs
            .get(&expr_key(expr))
            .filter(|(_, owner)| *owner == item)
            .map(|(span, _)| *span)
    })
}

/// The span of `stmt`, when it belongs to the item being checked.
pub(crate) fn stmt_span(stmt: &HirStmt) -> Option<Span> {
    lookup(|locator, item| {
        locator
            .stmts
            .get(&stmt_key(stmt))
            .filter(|(_, owner)| *owner == item)
            .map(|(span, _)| *span)
    })
}

/// The return annotation of the function being checked.
pub(crate) fn returns_span() -> Option<Span> {
    lookup(|locator, item| locator.returns.get(&item).copied())
}

/// Whether `diagnostic` still has no source position.
pub(crate) fn is_unplaced(diagnostic: &Diagnostic) -> bool {
    diagnostic
        .span
        .is_none_or(|span| (span.start, span.end) == (0, 0))
}

/// Places an unplaced `diagnostic` at `span`, with a short label in place
/// of the headline repeated (see [`short_label`]).
pub(crate) fn place_at(mut diagnostic: Diagnostic, span: Option<Span>) -> Diagnostic {
    let Some(span) = span else {
        return diagnostic;
    };
    if !is_unplaced(&diagnostic) || diagnostic.severity != pycc_diag::Severity::Error {
        return diagnostic;
    }
    diagnostic.span = Some(span);
    if diagnostic.label.as_deref() == Some(diagnostic.message.as_str()) {
        diagnostic.label = short_label(&diagnostic);
    }
    diagnostic
}

/// Like [`place_at`], with an explicit label.
pub(crate) fn place_labeled(
    diagnostic: Diagnostic,
    span: Option<Span>,
    label: impl Into<String>,
) -> Diagnostic {
    if span.is_none() || !is_unplaced(&diagnostic) {
        return diagnostic;
    }
    let mut placed = place_at(diagnostic, span);
    placed.label = Some(label.into());
    placed
}

pub(crate) fn place_expr(diagnostic: Diagnostic, expr: &HirExpr) -> Diagnostic {
    if !is_unplaced(&diagnostic) {
        return diagnostic;
    }
    place_at(diagnostic, expr_span(expr))
}

pub(crate) fn place_stmt(diagnostic: Diagnostic, stmt: &HirStmt) -> Diagnostic {
    if !is_unplaced(&diagnostic) {
        return diagnostic;
    }
    place_at(diagnostic, stmt_span(stmt))
}

/// Places an unplaced diagnostic of item `index` at its header.
pub(crate) fn place_item(diagnostic: Diagnostic, index: usize) -> Diagnostic {
    if !is_unplaced(&diagnostic) {
        return diagnostic;
    }
    let header = LOCATORS.with(|cell| {
        cell.borrow()
            .last()
            .and_then(|locator| locator.headers.get(&index).copied())
    });
    place_at(diagnostic, header)
}

/// The label under the caret: shorter than the headline, saying what is
/// wrong at that spot -- `expected `A`, found `B`` for a type mismatch, a
/// short phrase otherwise. DIAGNOSTICS.md asks every error for a label, so
/// a headline with no shorter reading keeps itself as the label.
pub(crate) fn short_label(diagnostic: &Diagnostic) -> Option<String> {
    let message = diagnostic.message.as_str();
    if let Some((expected, found)) = mismatch(message) {
        return Some(format!("expected `{expected}`, found `{found}`"));
    }
    if let Some(count) = expected_argument_count(message) {
        return Some(format!("expected {count} argument(s)"));
    }
    if let Some(found) = between(message, ", got `", "`") {
        return Some(format!("found `{found}`"));
    }
    let t0021 = diagnostic.code == "T0021";
    let phrase = if !t0021 {
        match diagnostic.code {
            "T0003" => "needs a type annotation",
            "T0024" => "`return` outside a function",
            "T0030" => "not all cases are covered",
            "T0032" | "T0035" | "T0037" => "elements of different types",
            "T0033" => "unsupported on this type",
            "T0034" | "T0036" | "T0038" | "T0039" => "element type not compiled yet",
            "T0040" => "not a literal index in range",
            "T0041" => "may be unbound here",
            "T0043" => "not a class instance",
            "T0044" => "no such attribute",
            "T0045" => "cannot assign twice to a `Final` name",
            "T0046" => "does not conform to the protocol",
            "T0054" => "unhashable",
            "T0055" => "wrong number of values to unpack",
            "C0001" | "C0002" => "not supported yet",
            "I0404" => "not supported on a CPython object",
            _ => return Some(diagnostic.message.clone()),
        }
    } else if message.contains("is not bound before this use") {
        "used before assignment"
    } else if message.starts_with("call to undefined function")
        || (message.starts_with("name `") && message.contains("is not defined"))
    {
        "not defined"
    } else if message.contains("is bound to a non-callable value") {
        "not callable"
    } else if message.contains("operator") && message.contains("is not defined for") {
        "unsupported operand type"
    } else if message.starts_with("cannot compare") {
        "these types cannot be compared"
    } else if message.contains("no common type") {
        "operands have no common type"
    } else if message.contains("truth") {
        "cannot be tested for truth here"
    } else {
        return Some(diagnostic.message.clone());
    };
    Some(phrase.to_string())
}

/// `N` from a headline shaped `... expects N argument(s), got M` or
/// `... expects exactly N argument, got M`.
fn expected_argument_count(message: &str) -> Option<&str> {
    let rest = &message[message.find("expects ")? + "expects ".len()..];
    let rest = rest.strip_prefix("exactly ").unwrap_or(rest);
    let count = rest.split(' ').next()?;
    let is_count = !count.is_empty() && count.chars().all(|c| c.is_ascii_digit());
    (is_count && rest[count.len()..].starts_with(" argument")).then_some(count)
}

/// The text between the first `open` and the next `close` after it.
fn between<'a>(message: &'a str, open: &str, close: &str) -> Option<&'a str> {
    let rest = &message[message.find(open)? + open.len()..];
    Some(&rest[..rest.find(close)?])
}

/// `(expected, found)` from a headline naming both types: `... expects `A`,
/// got `B``, `... expected `A`, found `B``, `... must be `A`, got `B``,
/// `cannot assign `B` to `x`, previously inferred as `A``, `cannot add `B`
/// to a set of `A``, and the dict-method shapes.
fn mismatch(message: &str) -> Option<(&str, &str)> {
    for (left, right) in [
        ("expects `", "`, got `"),
        ("expected `", "`, found `"),
        ("expected `", "`, got `"),
        ("must be `", "`, got `"),
    ] {
        let Some(start) = message.find(left) else {
            continue;
        };
        let rest = &message[start + left.len()..];
        let Some(middle) = rest.find(right) else {
            continue;
        };
        let expected = &rest[..middle];
        if expected.contains('`') {
            continue;
        }
        let after = &rest[middle + right.len()..];
        let Some(end) = after.find('`') else {
            continue;
        };
        return Some((expected, &after[..end]));
    }
    let found = between(message, "cannot assign `", "`");
    if let (Some(found), Some(expected)) =
        (found, between(message, "previously inferred as `", "`"))
    {
        return Some((expected, found));
    }
    if let (Some(found), Some(declared)) =
        (found, between(message, "previously declared as `", "`"))
    {
        let expected = declared.split_once(": ").map_or(declared, |(_, ty)| ty);
        return Some((expected, found));
    }
    for (found_open, expected_open) in [
        ("cannot add `", "to a set of `"),
        ("cannot use a `", "for a dict of `"),
        ("cannot look up a `", "in a dict of `"),
    ] {
        if let (Some(found), Some(expected)) = (
            between(message, found_open, "`"),
            between(message, expected_open, "`"),
        ) {
            return Some((expected, found));
        }
    }
    None
}
