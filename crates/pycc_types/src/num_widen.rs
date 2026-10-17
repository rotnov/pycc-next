//! Inferring `int | float` (`Ty::Num`) for a name bound to both, and for a
//! list that holds both.
//!
//! The checker gives a name the type of its first binding. When a later
//! binding of the same name has the other numeric type -- `best =
//! float('inf')` then `best = min(best, cost)` with an `int` cost, an
//! accumulator `total = 0` that receives a `float`, branches that bind an
//! `int` and a `float` -- the rebinding fails with `T0023`, and the failure
//! records the name here ([`request`]). [`with_num_widening`] then checks
//! the module again with the name's first plain assignment in each scope
//! rewritten into `name: int | float = value`, whose slot holds either,
//! until the check passes or asks for no new name. This is the same
//! retry `crate::non_strict` uses to widen a name to `object`, in both
//! modes.
//!
//! A list works the same way one level down: storing or appending a number
//! of the other kind into a `list[int]` or `list[float]` (`dist =
//! [float('inf')] * n` then `dist[v] = 0`) records the list
//! ([`request_list`]), and the retry wraps every element its first binding
//! builds -- a display, a repeated display or a comprehension -- in
//! [`pycc_hir::NUM_MARKER`], so the list is a `list[int | float]`.

use crate::non_strict::{WidenKey, scope_keys, widen_to};
use pycc_hir::{BinOpKind, HirExpr, HirItem, HirModule, HirStmt, Ty};
use std::cell::RefCell;
use std::collections::BTreeSet;

thread_local! {
    static REQUESTS: RefCell<BTreeSet<WidenKey>> = const { RefCell::new(BTreeSet::new()) };
    static LIST_REQUESTS: RefCell<BTreeSet<WidenKey>> = const { RefCell::new(BTreeSet::new()) };
}

/// Whether `ty` is a number an `int | float` slot holds.
fn is_number(ty: &Ty) -> bool {
    matches!(ty, Ty::Int | Ty::Bool | Ty::Float | Ty::Num)
}

/// Whether `previous` and `new` are numbers of which one may be a `float`
/// (an `int`/`bool` pair is already one `int` slot).
fn mixes_kinds(previous: &Ty, new: &Ty) -> bool {
    let floating = |ty: &Ty| matches!(ty, Ty::Float | Ty::Num);
    is_number(previous) && is_number(new) && previous != new && (floating(previous) || floating(new))
}

/// Records that `name`, bound to `previous`, was rebound to `new`, when the
/// two are numbers of different kinds.
pub(crate) fn request(name: &str, previous: &Ty, new: &Ty) {
    if mixes_kinds(previous, new) {
        let item = crate::locate::current_item();
        REQUESTS.with(|requests| requests.borrow_mut().insert((item, name.to_string())));
    }
}

/// Records that the list `name` of type `list_ty` was stored (or appended)
/// a `value` number of another kind than its elements.
pub(crate) fn request_list(name: &str, list_ty: &Ty, value: &Ty) {
    if let Ty::List(element) = list_ty
        && mixes_kinds(element, value)
    {
        let item = crate::locate::current_item();
        LIST_REQUESTS.with(|requests| requests.borrow_mut().insert((item, name.to_string())));
    }
}

fn take(requests: &'static std::thread::LocalKey<RefCell<BTreeSet<WidenKey>>>) -> BTreeSet<WidenKey> {
    requests.with(|requests| std::mem::take(&mut *requests.borrow_mut()))
}

/// Runs `check` over `hir`, re-running it with every requested name
/// widened to `int | float` (and every requested list's elements made
/// `int | float`) until it passes or a run requests nothing new (its
/// diagnostics are then the answer).
pub(crate) fn with_num_widening<T, E>(
    hir: &HirModule,
    check: impl Fn(&HirModule) -> Result<T, E>,
) -> Result<T, E> {
    let mut widened: BTreeSet<WidenKey> = BTreeSet::new();
    // A list whose stores are evidently of the other kind is widened up
    // front: a check can fail elsewhere first (the private-helper solver
    // stops at its first error) and never reach the store that asks.
    let mut lists: BTreeSet<WidenKey> = evidently_mixed_lists(hir);
    let mut current: Option<HirModule> = if lists.is_empty() {
        None
    } else {
        let mut module = hir.clone();
        widen_lists(&mut module, &lists);
        Some(module)
    };
    loop {
        take(&REQUESTS);
        take(&LIST_REQUESTS);
        let result = check(current.as_ref().unwrap_or(hir));
        let requested = scope_keys(hir, take(&REQUESTS));
        let requested_lists = scope_keys(hir, take(&LIST_REQUESTS));
        if result.is_ok() || (requested.is_subset(&widened) && requested_lists.is_subset(&lists)) {
            return result;
        }
        widened.extend(requested);
        lists.extend(requested_lists);
        let mut module = widen_to(hir, &widened, &BTreeSet::new(), &Ty::Num);
        widen_lists(&mut module, &lists);
        current = Some(module);
    }
}

/// A number's kind as its syntax shows it: an `int` or `float` literal, a
/// `float(...)` call, a true division; `None` when only types tell.
fn evident_kind(expr: &HirExpr) -> Option<bool> {
    match expr {
        HirExpr::IntLiteral(_) => Some(false),
        HirExpr::FloatLiteral(_) => Some(true),
        HirExpr::Call { callee, .. } if callee == "float" => Some(true),
        HirExpr::BinOp { op: BinOpKind::Div, .. } => Some(true),
        HirExpr::UnaryOp { operand, .. } => evident_kind(operand),
        _ => None,
    }
}

/// The evident kind of every element the list builder `value` makes (see
/// [`wrap_built_elements`]), when they share one.
fn built_kind(value: &HirExpr) -> Option<bool> {
    match value {
        HirExpr::ListLiteral(elements) => {
            let mut kinds = elements.iter().map(evident_kind);
            let first = kinds.next()??;
            kinds.all(|kind| kind == Some(first)).then_some(first)
        }
        HirExpr::BinOp {
            op: BinOpKind::Mul,
            left,
            right,
        } => built_kind(left).or_else(|| built_kind(right)),
        _ => None,
    }
}

/// Every list (by scope, as [`WidenKey`]s) whose first binding builds
/// numbers of one evident kind and that a later store or `.append` in the
/// same scope gives a number of the other evident kind (`dist =
/// [float('inf')] * n` ... `dist[v] = 0`).
fn evidently_mixed_lists(hir: &HirModule) -> BTreeSet<WidenKey> {
    let mut out = BTreeSet::new();
    let mut top_level = Vec::new();
    for (index, item) in hir.items.iter().enumerate() {
        match item {
            HirItem::Function { body, .. } => {
                for name in mixed_in(body.clone()) {
                    out.insert((Some(index), name));
                }
            }
            HirItem::TopLevelStmt(stmt) => top_level.push(stmt.clone()),
        }
    }
    for name in mixed_in(top_level) {
        out.insert((None, name));
    }
    out
}

fn mixed_in(mut body: Vec<HirStmt>) -> BTreeSet<String> {
    let mut first: std::collections::BTreeMap<String, Option<bool>> = Default::default();
    let mut mixed = BTreeSet::new();
    crate::empty_container::for_each_stmt_mut(&mut body, &mut |stmt| {
        let (name, kind, stored) = match stmt {
            HirStmt::Assign { target, value } => (target.clone(), built_kind(value), false),
            HirStmt::ListCompAssign { target, elt, .. } => (target.clone(), evident_kind(elt), false),
            HirStmt::DictSet { dict, value, .. } => (dict.clone(), evident_kind(value), true),
            HirStmt::ExprStmt(HirExpr::ListAppend {
                list: pycc_hir::ContainerReceiver::Name(name),
                value,
            }) => (name.clone(), evident_kind(value), true),
            _ => return,
        };
        match (first.get(&name), stored) {
            (None, false) => {
                first.insert(name, kind);
            }
            (Some(Some(built)), true) if kind.is_some_and(|kind| kind != *built) => {
                mixed.insert(name);
            }
            _ => {}
        }
    });
    mixed
}

/// Wraps the elements of the first binding of each list in `keys`, in its
/// scope, in [`pycc_hir::NUM_MARKER`].
fn widen_lists(hir: &mut HirModule, keys: &BTreeSet<WidenKey>) {
    let everywhere: BTreeSet<&str> = keys
        .iter()
        .filter(|(item, _)| item.is_none())
        .map(|(_, name)| name.as_str())
        .collect();
    let mut top_level_seen = BTreeSet::new();
    for (index, item) in hir.items.iter_mut().enumerate() {
        match item {
            HirItem::Function { body, .. } => {
                let names: BTreeSet<&str> = keys
                    .iter()
                    .filter(|(item, _)| *item == Some(index))
                    .map(|(_, name)| name.as_str())
                    .collect();
                widen_list_block(body, &names, &mut BTreeSet::new());
            }
            HirItem::TopLevelStmt(stmt) => {
                widen_list_block(std::slice::from_mut(stmt), &everywhere, &mut top_level_seen);
            }
        }
    }
}

fn widen_list_block(body: &mut [HirStmt], names: &BTreeSet<&str>, seen: &mut BTreeSet<String>) {
    crate::empty_container::for_each_stmt_mut(body, &mut |stmt| match stmt {
        HirStmt::Assign { target, value } if names.contains(target.as_str()) && !seen.contains(target) => {
            seen.insert(target.clone());
            wrap_built_elements(value);
        }
        HirStmt::ListCompAssign { target, elt, .. }
            if names.contains(target.as_str()) && !seen.contains(target) =>
        {
            seen.insert(target.clone());
            wrap(elt);
        }
        _ => {}
    });
}

/// Wraps each element a list display, a repeated display (`[x] * n`) or a
/// list comprehension `value` builds.
fn wrap_built_elements(value: &mut HirExpr) {
    match value {
        HirExpr::ListLiteral(elements) => elements.iter_mut().for_each(wrap),
        HirExpr::BinOp {
            op: BinOpKind::Mul,
            left,
            right,
        } => {
            wrap_built_elements(left);
            wrap_built_elements(right);
        }
        HirExpr::Comprehension(comprehension) => {
            if let pycc_hir::CompElt::List(elt) = &mut comprehension.elt {
                wrap(elt);
            }
        }
        _ => {}
    }
}

fn wrap(element: &mut HirExpr) {
    let inner = std::mem::replace(element, HirExpr::IntLiteral(0));
    *element = HirExpr::Call {
        callee: pycc_hir::NUM_MARKER.to_string(),
        args: vec![inner],
    };
}

/// Whether a list display `value` inferred as `inferred` initializes an
/// `annotation` of `list[int | float]`: a display of numbers is a new list,
/// so its elements become `int | float` words (`pycc_mir` wraps each) and no
/// other list's element type changes.
pub(crate) fn display_widens(value: &HirExpr, inferred: &Ty, annotation: &Ty) -> bool {
    matches!(value, HirExpr::ListLiteral(_))
        && matches!(annotation, Ty::List(element) if **element == Ty::Num)
        && matches!(inferred, Ty::List(element) if is_number(element))
}
