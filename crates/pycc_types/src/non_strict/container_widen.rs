//! Non-strict mode: a name holding a native container that meets a CPython
//! object -- an argument of an object call, the operand of a comparison
//! with one -- becomes an object itself, so the object sees the very
//! container the name holds and no copy breaks aliasing.
//!
//! A widened name is checked as `object` at every binding, so the request is
//! granted only when every binding of the name in its scope stores a value
//! that is boxed into a fresh CPython container (a display, a comprehension,
//! an empty literal) or another name granted the same way. Any other binding
//! -- a call result, a subscript, a loop target, a parameter -- would be a
//! native container the name cannot hold as an object, and the request is
//! dropped, leaving the original diagnostic (and its CPython fallback) in
//! place.

use super::WidenKey;
use pycc_hir::{HirExpr, HirItem, HirModule, HirStmt, Ty};
use std::cell::RefCell;
use std::collections::BTreeSet;

thread_local! {
    static REQUESTS: RefCell<BTreeSet<WidenKey>> = const { RefCell::new(BTreeSet::new()) };
}

/// Records a request to widen `expr` when it is a name holding a native
/// container of type `ty` (module doc).
///
/// A `frozenset` of scalars is not one: it crosses into CPython as an
/// immutable copy (`foreign::is_packable_frozenset`), which no aliasing can
/// tell apart, so widening it would only make a module's frozenset constant
/// (`TOML_WS = frozenset(" \t")`) an object everywhere -- every `in` on it a
/// C API call, and the module's calls no longer able to take a native
/// `Iterable[str]` specialization (`pycc_ast::desugar::specialize`).
pub(crate) fn request(expr: &HirExpr, ty: &Ty) {
    if super::is_non_strict()
        && let HirExpr::Name(name) = expr
        && matches!(ty, Ty::List(_) | Ty::Set(_) | Ty::Dict(_) | Ty::FrozenSet(_))
        && !crate::foreign::is_packable_frozenset(ty)
    {
        let item = crate::locate::current_item();
        REQUESTS.with(|requests| requests.borrow_mut().insert((item, name.clone())));
    }
}

pub(super) fn take() -> BTreeSet<WidenKey> {
    REQUESTS.with(|requests| std::mem::take(&mut *requests.borrow_mut()))
}

/// The requests (already scope-keyed) every binding of which is fresh,
/// together with the names those bindings copy from, granted the same way.
pub(super) fn granted(hir: &HirModule, requests: BTreeSet<WidenKey>) -> BTreeSet<WidenKey> {
    let mut granted = BTreeSet::new();
    for (key, name) in requests {
        let mut pending = vec![name];
        let mut names = BTreeSet::new();
        let mut ok = true;
        while let Some(name) = pending.pop() {
            if !names.insert(name.clone()) {
                continue;
            }
            match sources(hir, key, &name) {
                Some(copied) => pending.extend(copied),
                None => {
                    ok = false;
                    break;
                }
            }
        }
        if ok {
            granted.extend(names.into_iter().map(|name| (key, name)));
        }
    }
    granted
}

/// The names `name`'s bindings in scope `key` copy from, when every binding
/// is fresh or such a copy; `None` when one is neither, or there is none.
fn sources(hir: &HirModule, key: Option<usize>, name: &str) -> Option<Vec<String>> {
    let mut copied = Vec::new();
    let mut bound = false;
    for (index, item) in hir.items.iter().enumerate() {
        let body = match item {
            HirItem::Function { params, body, .. } if key == Some(index) => {
                if params.iter().any(|(param, _)| param == name) {
                    return None;
                }
                body.as_slice()
            }
            HirItem::TopLevelStmt(stmt) if key.is_none() => std::slice::from_ref(stmt),
            // A module-wide name may also be rebound by a `global`
            // declaration in a function: any binding there must qualify too.
            HirItem::Function { body, .. } if key.is_none() => body.as_slice(),
            _ => continue,
        };
        if !bindings(body, name, &mut copied, &mut bound) {
            return None;
        }
    }
    bound.then_some(copied)
}

/// Walks `body`; `false` as soon as a binding of `name` is not fresh.
fn bindings(body: &[HirStmt], name: &str, copied: &mut Vec<String>, bound: &mut bool) -> bool {
    for stmt in body {
        let ok = match stmt {
            HirStmt::Assign { target, value } if target == name => {
                *bound = true;
                match value {
                    HirExpr::Name(source) => {
                        copied.push(source.clone());
                        true
                    }
                    // `name op= v`: an object's own in-place result.
                    HirExpr::AugBinOp { left, .. }
                        if matches!(left.as_ref(), HirExpr::Name(n) if n == name) =>
                    {
                        true
                    }
                    other => fresh(other),
                }
            }
            HirStmt::ListCompAssign { target, .. }
            | HirStmt::DictCompAssign { target, .. }
            | HirStmt::SetCompAssign { target, .. }
                if target == name =>
            {
                *bound = true;
                true
            }
            HirStmt::AnnAssign { target, .. } => target != name,
            HirStmt::ForRange { var, .. }
            | HirStmt::ForList { var, .. }
            | HirStmt::ForObject { var, .. } => var != name,
            HirStmt::Try { handlers, .. } | HirStmt::TryStar { handlers, .. } => handlers
                .iter()
                .all(|handler| handler.name.as_deref() != Some(name)),
            HirStmt::Match { .. } => false,
            _ => true,
        };
        if !ok {
            return false;
        }
        for nested in nested(stmt) {
            if !bindings(nested, name, copied, bound) {
                return false;
            }
        }
    }
    true
}

/// A value boxed into a fresh CPython container: a display, an empty
/// literal or constructor call, a comprehension, a repetition or
/// concatenation of a display (`[0] * (n + 1)`), or a call of a function
/// returning a fresh list (`super::fresh_returns`).
pub(super) fn fresh(value: &HirExpr) -> bool {
    match value {
        // `set()`, `dict()`, `list()`: an empty container (the boxing check
        // at the binding refuses one a module function shadows).
        HirExpr::Call { callee, args } if args.is_empty() && matches!(callee.as_str(), "set" | "dict" | "list") => true,
        // `set(xs)`: a new set, whose elements (hashable, so immutable or
        // carried by identity) no copy can tell apart from the originals.
        HirExpr::Call { callee, args } if args.len() == 1 && matches!(callee.as_str(), "set" | "frozenset") => true,
        // A native dict of objects (`super::object_dicts`): its display.
        HirExpr::Call { .. } if super::object_dicts::is_marked(value) => true,
        HirExpr::Call { callee, .. } => super::fresh_returns::is_fresh_returning(callee),
        HirExpr::BinOp {
            op: pycc_hir::BinOpKind::Add | pycc_hir::BinOpKind::Mul,
            left,
            right,
        } => fresh(left) || fresh(right),
        _ => matches!(
            value,
            HirExpr::ListLiteral(_)
                | HirExpr::SetLiteral(_)
                | HirExpr::DictLiteral(_)
                | HirExpr::EmptyList(_)
                | HirExpr::EmptySet(_)
                | HirExpr::EmptyDict(_)
                | HirExpr::Comprehension(_)
        ),
    }
}

pub(super) fn nested(stmt: &HirStmt) -> Vec<&[HirStmt]> {
    match stmt {
        HirStmt::If { body, orelse, .. } => vec![body, orelse],
        HirStmt::While { body, .. }
        | HirStmt::ForRange { body, .. }
        | HirStmt::ForList { body, .. }
        | HirStmt::ForObject { body, .. } => vec![body],
        HirStmt::LoopElse { loop_stmt, orelse } => {
            vec![std::slice::from_ref(loop_stmt.as_ref()), orelse]
        }
        HirStmt::Try {
            body,
            handlers,
            orelse,
            finalbody,
        }
        | HirStmt::TryStar {
            body,
            handlers,
            orelse,
            finalbody,
        } => {
            let mut bodies: Vec<&[HirStmt]> = vec![body];
            bodies.extend(handlers.iter().map(|handler| handler.body.as_slice()));
            bodies.push(orelse);
            bodies.push(finalbody);
            bodies
        }
        _ => Vec::new(),
    }
}
