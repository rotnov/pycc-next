//! Non-strict mode (v0.10 accept 2): native `str`-keyed dicts of CPython
//! objects.
//!
//! A dict display holding a CPython object (`{"type": "text", "raw": s}`)
//! is a CPython `dict` (`pycc_hir::display_is_object`), so every read of it
//! is a C API call. A function's local whose every binding is such a display
//! with `str` literal keys, and that the function only reads an item of,
//! stores an item into, tests a key against, measures, iterates or tests for
//! truth, never escapes: nothing but the function can see it, so it is a
//! native `dict[str, object]` instead (the display rewritten into a call of
//! [`pycc_hir::OBJECT_DICT_MARKER`]), its `str` keys hashed natively and its
//! values the CPython objects themselves. Escape is decided here, per
//! binding, at build time; a dict that may escape -- passed, returned,
//! stored, captured, compared, printed, a method called on it -- stays the
//! CPython dict it was, and every dict CPython can see is one. Insertion
//! order, `KeyError` and the iteration's `RuntimeError` on a change of size
//! are the native dict's, which are CPython's.
//!
//! A function the checker does not admit with its native dicts (an object
//! key, say) gets its displays back as they were ([`super::with_widening`]
//! checks again), rather than running in CPython for them.

use super::container_widen::nested;
use pycc_hir::{CmpOpKind, HirExpr, HirItem, HirModule, HirStmt, Ty};
use std::cell::RefCell;
use std::collections::BTreeSet;

thread_local! {
    /// The `(function index, name)` pairs [`find_candidates`] found, while the
    /// first check runs.
    static CANDIDATES: RefCell<BTreeSet<(usize, String)>> = const { RefCell::new(BTreeSet::new()) };
    /// The candidates the first check bound to a CPython dict display.
    static OBJECT_BOUND: RefCell<BTreeSet<(usize, String)>> = const { RefCell::new(BTreeSet::new()) };
}

/// The `(function index, name)` of every local that qualifies by its uses
/// (module doc), recorded for [`note_binding`].
pub(super) fn find_candidates(hir: &HirModule) -> bool {
    let module_names = module_bindings(hir);
    let mut found = BTreeSet::new();
    for (index, item) in hir.items.iter().enumerate() {
        let HirItem::Function { params, body, .. } = item else {
            continue;
        };
        let mut names = BTreeSet::new();
        collect_candidates(body, &mut names);
        for name in names {
            if !params.iter().any(|(param, _)| *param == name)
                && !module_names.contains(&name)
                && only_display_bindings(body, &name)
                && !escapes_in(body, &name)
            {
                found.insert((index, name));
            }
        }
    }
    let any = !found.is_empty();
    CANDIDATES.with(|c| *c.borrow_mut() = found);
    OBJECT_BOUND.with(|b| b.borrow_mut().clear());
    any
}

/// Records that the checker bound `target` to `value` of type `ty`: a
/// candidate's display that is a CPython dict (`ty` is `object`) is one to
/// make native.
pub(crate) fn note_binding(target: &str, value: &HirExpr, ty: &Ty) {
    if *ty != Ty::Object || !matches!(value, HirExpr::DictLiteral(_)) {
        return;
    }
    let Some(item) = crate::locate::current_item() else {
        return;
    };
    let key = (item, target.to_string());
    if CANDIDATES.with(|c| c.borrow().contains(&key)) {
        OBJECT_BOUND.with(|b| b.borrow_mut().insert(key));
    }
}

/// `hir` with the displays of every candidate the first check bound to a
/// CPython dict rewritten into the marker call, and the indices of the
/// functions holding one; `None` when there is none. Clears the record.
pub(super) fn rewrite(hir: &HirModule) -> Option<(HirModule, BTreeSet<usize>)> {
    let chosen = OBJECT_BOUND.with(|b| std::mem::take(&mut *b.borrow_mut()));
    CANDIDATES.with(|c| c.borrow_mut().clear());
    if chosen.is_empty() {
        return None;
    }
    let mut module = hir.clone();
    let mut marked = BTreeSet::new();
    for (index, item) in module.items.iter_mut().enumerate() {
        let HirItem::Function { body, .. } = item else {
            continue;
        };
        let names: BTreeSet<String> = chosen
            .iter()
            .filter(|(item, _)| *item == index)
            .map(|(_, name)| name.clone())
            .collect();
        if names.is_empty() {
            continue;
        }
        marked.insert(index);
        rewrite_bindings(body, &names);
    }
    Some((module, marked))
}

/// A non-empty dict display whose keys are distinct `str` literals. A
/// repeated key stays a CPython display: CPython evaluates every value before
/// it stores any, so an overwritten value is released only after the later
/// values ran, while the native dict would release it at its insert.
fn is_str_keyed_display(expr: &HirExpr) -> bool {
    let HirExpr::DictLiteral(pairs) = expr else {
        return false;
    };
    let mut seen = BTreeSet::new();
    !pairs.is_empty()
        && pairs
            .iter()
            .all(|(key, _)| matches!(key, HirExpr::StringLiteral(s) if seen.insert(s.clone())))
}

/// The names a top-level statement binds (any name a function might also
/// declare `global`).
fn module_bindings(hir: &HirModule) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    fn walk(body: &[HirStmt], out: &mut BTreeSet<String>) {
        for stmt in body {
            match stmt {
                HirStmt::Assign { target, .. } | HirStmt::AnnAssign { target, .. } => {
                    out.insert(target.clone());
                }
                HirStmt::ForRange { var, .. }
                | HirStmt::ForList { var, .. }
                | HirStmt::ForObject { var, .. } => {
                    out.insert(var.clone());
                }
                HirStmt::DictSet { dict, .. } => {
                    out.insert(dict.clone());
                }
                _ => {}
            }
            for inner in nested(stmt) {
                walk(inner, out);
            }
        }
    }
    for item in &hir.items {
        if let HirItem::TopLevelStmt(stmt) = item {
            walk(std::slice::from_ref(stmt), &mut out);
        }
    }
    out
}

/// Every name some statement of `body` binds to a `str`-keyed display.
fn collect_candidates(body: &[HirStmt], out: &mut BTreeSet<String>) {
    for stmt in body {
        if let HirStmt::Assign { target, value } = stmt
            && is_str_keyed_display(value)
        {
            out.insert(target.clone());
        }
        for inner in nested(stmt) {
            collect_candidates(inner, out);
        }
    }
}

/// Whether every binding of `name` in `body` is a `str`-keyed display.
fn only_display_bindings(body: &[HirStmt], name: &str) -> bool {
    body.iter().all(|stmt| {
        let ok = match stmt {
            HirStmt::Assign { target, value } if target == name => is_str_keyed_display(value),
            HirStmt::AnnAssign { target, .. } => target != name,
            HirStmt::ForRange { var, .. }
            | HirStmt::ForList { var, .. }
            | HirStmt::ForObject { var, .. } => var != name,
            HirStmt::Try { handlers, .. } | HirStmt::TryStar { handlers, .. } => handlers
                .iter()
                .all(|handler| handler.name.as_deref() != Some(name)),
            HirStmt::Delete { name: deleted } => deleted != name,
            HirStmt::ListCompAssign { target, .. }
            | HirStmt::SetCompAssign { target, .. }
            | HirStmt::DictCompAssign { target, .. } => target != name,
            // A `match` pattern may bind it.
            HirStmt::Match { .. } => false,
            _ => true,
        };
        ok && nested(stmt)
            .into_iter()
            .all(|inner| only_display_bindings(inner, name))
    })
}

/// Whether some use of `name` in `body` is not one of the uses the module
/// doc admits.
fn escapes_in(body: &[HirStmt], name: &str) -> bool {
    let is_name = |e: &HirExpr| matches!(e, HirExpr::Name(n) if n == name);
    body.iter().any(|stmt| {
        let own = match stmt {
            HirStmt::Assign { target, value } if target == name => pairs_escape(value, name),
            HirStmt::Assign { value, .. } | HirStmt::ExprStmt(value) => escapes(value, name),
            HirStmt::AnnAssign { value, .. } => value.as_ref().is_some_and(|v| escapes(v, name)),
            // `if d:` / `while d:` test its truth.
            HirStmt::If { test, .. } | HirStmt::While { test, .. } => {
                !is_name(test) && escapes(test, name)
            }
            HirStmt::DictSet { key, value, .. } => escapes(key, name) || escapes(value, name),
            HirStmt::Return(Some(value)) => escapes(value, name),
            HirStmt::ForRange {
                start, stop, step, ..
            } => [start, stop, step].iter().any(|e| escapes(e, name)),
            HirStmt::ForObject { iter, .. } => escapes(iter, name),
            HirStmt::Raise { exc, cause } => [exc, cause]
                .into_iter()
                .flatten()
                .any(|e| escapes(e, name)),
            // `for k in d:` iterates its keys.
            HirStmt::ForList { .. }
            | HirStmt::Return(None)
            | HirStmt::Break
            | HirStmt::Continue
            | HirStmt::Pos(_)
            | HirStmt::LoopElse { .. }
            | HirStmt::Try { .. }
            | HirStmt::TryStar { .. }
            | HirStmt::Delete { .. } => false,
            // Any other statement may use `name` in a way this walk does not
            // follow; refuse rather than guess.
            _ => true,
        };
        own || nested(stmt).into_iter().any(|inner| escapes_in(inner, name))
    })
}

/// Whether a binding display's values use `name` (`d = {"a": d}`).
fn pairs_escape(display: &HirExpr, name: &str) -> bool {
    let HirExpr::DictLiteral(pairs) = display else {
        return true;
    };
    pairs.iter().any(|(_, value)| escapes(value, name))
}

/// Whether `expr` holds a use of `name` other than an item read, a key test
/// (`k in d`, `k not in d`) or a `len`.
fn escapes(expr: &HirExpr, name: &str) -> bool {
    let is_name = |e: &HirExpr| matches!(e, HirExpr::Name(n) if n == name);
    match expr {
        HirExpr::Name(n) => n == name,
        HirExpr::NamedExpr { name: bound, .. } if bound == name => true,
        HirExpr::Subscript { base, index } if is_name(base) => escapes(index, name),
        HirExpr::Compare {
            op: CmpOpKind::In | CmpOpKind::NotIn,
            left,
            right,
        } if is_name(right) => escapes(left, name),
        HirExpr::Call { callee, args } if callee == "len" && args.len() == 1 && is_name(&args[0]) => {
            false
        }
        other => pycc_hir::ndarray::expr_children(other)
            .into_iter()
            .any(|child| escapes(child, name)),
    }
}

/// Rewrites each binding of a `chosen` name into the marker call.
fn rewrite_bindings(body: &mut [HirStmt], chosen: &BTreeSet<String>) {
    for stmt in body.iter_mut() {
        if let HirStmt::Assign { target, value } = stmt
            && chosen.contains(target)
            && let HirExpr::DictLiteral(pairs) = value
        {
            let args = std::mem::take(pairs)
                .into_iter()
                .flat_map(|(key, value)| [key, value])
                .collect();
            *value = HirExpr::Call {
                callee: pycc_hir::OBJECT_DICT_MARKER.to_string(),
                args,
            };
        }
        for inner in nested_mut(stmt) {
            rewrite_bindings(inner, chosen);
        }
    }
}

/// [`nested`], mutably.
fn nested_mut(stmt: &mut HirStmt) -> Vec<&mut [HirStmt]> {
    match stmt {
        HirStmt::If { body, orelse, .. } => vec![body, orelse],
        HirStmt::While { body, .. }
        | HirStmt::ForRange { body, .. }
        | HirStmt::ForList { body, .. }
        | HirStmt::ForObject { body, .. } => vec![body],
        HirStmt::LoopElse { loop_stmt, orelse } => {
            vec![std::slice::from_mut(loop_stmt.as_mut()), orelse]
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
            let mut out: Vec<&mut [HirStmt]> = vec![body];
            out.extend(handlers.iter_mut().map(|h| h.body.as_mut_slice()));
            out.push(orelse);
            out.push(finalbody);
            out
        }
        HirStmt::Match { cases, .. } => cases.iter_mut().map(|c| c.body.as_mut_slice()).collect(),
        _ => Vec::new(),
    }
}

/// Every marker call of item `index` back as the display it was.
pub(super) fn unmark_item(module: &mut HirModule, index: usize) {
    if let Some(HirItem::Function { body, .. }) = module.items.get_mut(index) {
        unmark_block(body);
    }
}

fn unmark_block(body: &mut [HirStmt]) {
    for stmt in body.iter_mut() {
        if let HirStmt::Assign { value, .. } = stmt {
            unmark(value);
        }
        for inner in nested_mut(stmt) {
            unmark_block(inner);
        }
    }
}

/// `value` back as the display it was, when it is the marker call (a
/// widened name's binding: the name escapes after all).
pub(super) fn unmark(value: &mut HirExpr) {
    if let HirExpr::Call { callee, args } = value
        && callee == pycc_hir::OBJECT_DICT_MARKER
    {
        let args = std::mem::take(args);
        let mut pairs = Vec::with_capacity(args.len() / 2);
        let mut items = args.into_iter();
        while let (Some(key), Some(item)) = (items.next(), items.next()) {
            pairs.push((key, item));
        }
        *value = HirExpr::DictLiteral(pairs);
    }
}

/// Whether `value` is the marker call (a fresh display, for widening).
pub(super) fn is_marked(value: &HirExpr) -> bool {
    matches!(value, HirExpr::Call { callee, .. } if callee == pycc_hir::OBJECT_DICT_MARKER)
}
