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
//!
//! The same retry makes `T | None` of a name bound to `None` and to a `T`
//! (`best = None` ... `best = guy`, a `to_fill = None` a loop later binds
//! to a `str`): its first plain assignment becomes `name: T | None =
//! value` ([`request`] records the `T`).

use crate::non_strict::{WidenKey, scope_keys, widen_to};
use pycc_hir::{BinOpKind, HirExpr, HirItem, HirModule, HirStmt, Ty};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};

thread_local! {
    static REQUESTS: RefCell<BTreeSet<WidenKey>> = const { RefCell::new(BTreeSet::new()) };
    /// Names to widen to `T | None`, with the `T`.
    static OPTIONAL_REQUESTS: RefCell<BTreeMap<WidenKey, Ty>> =
        const { RefCell::new(BTreeMap::new()) };
    static LIST_REQUESTS: RefCell<BTreeSet<WidenKey>> = const { RefCell::new(BTreeSet::new()) };
    /// Lists to widen to `list[int | None]`.
    static OPTIONAL_LIST_REQUESTS: RefCell<BTreeSet<WidenKey>> =
        const { RefCell::new(BTreeSet::new()) };
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
    } else if let Some(inner) = optional_of(previous, new) {
        let item = crate::locate::current_item();
        OPTIONAL_REQUESTS
            .with(|requests| requests.borrow_mut().insert((item, name.to_string()), inner));
    }
}

/// The `T` of `T | None` when one of `previous` and `new` is `None` and the
/// other a `T` that can be optional, or `new` is the `T | None` of a `T`
/// `previous`.
fn optional_of(previous: &Ty, new: &Ty) -> Option<Ty> {
    // Non-strict mode widens such a name to a CPython object instead.
    let optional = |ty: &Ty| {
        matches!(ty, Ty::Int | Ty::Float | Ty::Bool)
            || (!crate::non_strict::is_non_strict() && pycc_hir::is_pointer_optional_inner(ty))
    };
    match (previous, new) {
        (Ty::None, other) | (other, Ty::None) if optional(other) => Some(other.clone()),
        // A `T` name later bound to a `T | None` (`curr = parents[curr]`);
        // a name already `T | None` needs no widening.
        (other, Ty::Optional(inner)) if **inner == *other => Some(other.clone()),
        _ => None,
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
    } else if let Ty::List(element) = list_ty
        && !crate::non_strict::is_non_strict()
        && mixes_none(element, value)
    {
        let item = crate::locate::current_item();
        OPTIONAL_LIST_REQUESTS
            .with(|requests| requests.borrow_mut().insert((item, name.to_string())));
    }
}

/// Whether a list of `element`s stored a `value` that makes it a list of
/// `int | None`: `None` into a list of `int`s, or an `int` into a list of
/// `None`s (`parents = [None] * n` ... `parents[v] = u`).
fn mixes_none(element: &Ty, value: &Ty) -> bool {
    let int = |ty: &Ty| matches!(ty, Ty::Int | Ty::Bool);
    let optional_int = |ty: &Ty| matches!(ty, Ty::Optional(inner) if **inner == Ty::Int);
    (int(element) && (*value == Ty::None || optional_int(value)))
        || (*element == Ty::None && (int(value) || optional_int(value)))
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
    let mut optionals: BTreeMap<WidenKey, Ty> = BTreeMap::new();
    // A list whose stores are evidently of the other kind is widened up
    // front: a check can fail elsewhere first (the private-helper solver
    // stops at its first error) and never reach the store that asks.
    let mut lists: BTreeSet<WidenKey> = evidently_mixed_lists(hir);
    let mut optional_lists: BTreeSet<WidenKey> = evidently_none_lists(hir);
    let mut current: Option<HirModule> = if lists.is_empty() && optional_lists.is_empty() {
        None
    } else {
        let mut module = hir.clone();
        widen_lists(&mut module, &lists, pycc_hir::NUM_MARKER);
        widen_lists(&mut module, &optional_lists, pycc_hir::OPTIONAL_INT_MARKER);
        Some(module)
    };
    loop {
        take(&REQUESTS);
        take(&LIST_REQUESTS);
        take(&OPTIONAL_LIST_REQUESTS);
        OPTIONAL_REQUESTS.with(|requests| requests.borrow_mut().clear());
        let result = check(current.as_ref().unwrap_or(hir));
        let requested = scope_keys(hir, take(&REQUESTS));
        let requested_lists = scope_keys(hir, take(&LIST_REQUESTS));
        let requested_optional_lists = scope_keys(hir, take(&OPTIONAL_LIST_REQUESTS));
        let requested_optionals: BTreeMap<WidenKey, Ty> = OPTIONAL_REQUESTS
            .with(|requests| std::mem::take(&mut *requests.borrow_mut()))
            .into_iter()
            .flat_map(|(key, inner)| {
                scope_keys(hir, BTreeSet::from([key]))
                    .into_iter()
                    .map(move |key| (key, inner.clone()))
            })
            .collect();
        let new_optional = requested_optionals.keys().any(|key| !optionals.contains_key(key));
        if result.is_ok()
            || (requested.is_subset(&widened)
                && requested_lists.is_subset(&lists)
                && requested_optional_lists.is_subset(&optional_lists)
                && !new_optional)
        {
            return result;
        }
        widened.extend(requested);
        lists.extend(requested_lists);
        optional_lists.extend(requested_optional_lists);
        optionals.extend(requested_optionals);
        let mut module = widen_to(hir, &widened, &BTreeSet::new(), &Ty::Num);
        for (key, inner) in &optionals {
            if !widened.contains(key) {
                let keys = BTreeSet::from([key.clone()]);
                let annotation = Ty::Optional(Box::new(inner.clone()));
                module = widen_to(&module, &keys, &BTreeSet::new(), &annotation);
            }
        }
        widen_lists(&mut module, &lists, pycc_hir::NUM_MARKER);
        widen_lists(&mut module, &optional_lists, pycc_hir::OPTIONAL_INT_MARKER);
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
/// scope, in `marker` ([`pycc_hir::NUM_MARKER`] or
/// [`pycc_hir::OPTIONAL_INT_MARKER`]).
fn widen_lists(hir: &mut HirModule, keys: &BTreeSet<WidenKey>, marker: &str) {
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
                widen_list_block(body, &names, &mut BTreeSet::new(), marker);
            }
            HirItem::TopLevelStmt(stmt) => {
                widen_list_block(std::slice::from_mut(stmt), &everywhere, &mut top_level_seen, marker);
            }
        }
    }
}

fn widen_list_block(
    body: &mut [HirStmt],
    names: &BTreeSet<&str>,
    seen: &mut BTreeSet<String>,
    marker: &str,
) {
    crate::empty_container::for_each_stmt_mut(body, &mut |stmt| match stmt {
        HirStmt::Assign { target, value } if names.contains(target.as_str()) && !seen.contains(target) => {
            seen.insert(target.clone());
            wrap_built_elements(value, marker);
        }
        HirStmt::ListCompAssign { target, elt, .. }
            if names.contains(target.as_str()) && !seen.contains(target) =>
        {
            seen.insert(target.clone());
            wrap(elt, marker);
        }
        _ => {}
    });
}

/// Wraps each element a list display, a repeated display (`[x] * n`) or a
/// list comprehension `value` builds.
fn wrap_built_elements(value: &mut HirExpr, marker: &str) {
    match value {
        HirExpr::ListLiteral(elements) => elements.iter_mut().for_each(|e| wrap(e, marker)),
        HirExpr::BinOp {
            op: BinOpKind::Mul,
            left,
            right,
        } => {
            wrap_built_elements(left, marker);
            wrap_built_elements(right, marker);
        }
        HirExpr::Comprehension(comprehension) => {
            if let pycc_hir::CompElt::List(elt) = &mut comprehension.elt {
                wrap(elt, marker);
            }
        }
        _ => {}
    }
}

fn wrap(element: &mut HirExpr, marker: &str) {
    let inner = std::mem::replace(element, HirExpr::IntLiteral(0));
    *element = HirExpr::Call {
        callee: marker.to_string(),
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

/// Every list (by scope) whose first binding builds only `None`s (`parents =
/// [None] * n`) and that a later store or `.append` in the same scope gives
/// something else: a `list[None]` is not a type a program means, so it is
/// widened to `list[int | None]` up front (its first binding is refused
/// before any store could ask).
fn evidently_none_lists(hir: &HirModule) -> BTreeSet<WidenKey> {
    let mut out = BTreeSet::new();
    // Non-strict mode keeps such a list the CPython one (`[None]` is an
    // object display), whose unnarrowed reads CPython runs as written.
    if crate::non_strict::is_non_strict() {
        return out;
    }
    let mut top_level = Vec::new();
    for (index, item) in hir.items.iter().enumerate() {
        match item {
            HirItem::Function { body, .. } => {
                for name in none_lists_in(body.clone()) {
                    out.insert((Some(index), name));
                }
            }
            HirItem::TopLevelStmt(stmt) => top_level.push(stmt.clone()),
        }
    }
    for name in none_lists_in(top_level) {
        out.insert((None, name));
    }
    out
}

/// Whether `value` builds a list of only `None`s (a display or a repeated
/// display).
fn builds_nones(value: &HirExpr) -> bool {
    match value {
        HirExpr::ListLiteral(elements) => {
            !elements.is_empty() && elements.iter().all(|e| matches!(e, HirExpr::NoneLiteral))
        }
        HirExpr::BinOp {
            op: BinOpKind::Mul,
            left,
            right,
        } => builds_nones(left) || builds_nones(right),
        _ => false,
    }
}

fn none_lists_in(mut body: Vec<HirStmt>) -> BTreeSet<String> {
    // Per list: whether its first binding builds only `None`s (`Some(true)`)
    // or only `int`s (`Some(false)`, `biggest = [0] * n` ... `biggest[v] =
    // None`).
    let mut first: std::collections::BTreeMap<String, Option<bool>> = Default::default();
    let mut widened = BTreeSet::new();
    crate::empty_container::for_each_stmt_mut(&mut body, &mut |stmt| {
        let (name, nones, stored) = match stmt {
            HirStmt::Assign { target, value } => {
                let kind = if builds_nones(value) {
                    Some(true)
                } else if built_kind(value) == Some(false) {
                    Some(false)
                } else {
                    None
                };
                (target.clone(), kind, false)
            }
            HirStmt::DictSet { dict, value, .. } => {
                (dict.clone(), Some(matches!(value, HirExpr::NoneLiteral)), true)
            }
            HirStmt::ExprStmt(HirExpr::ListAppend {
                list: pycc_hir::ContainerReceiver::Name(name),
                value,
            }) => (name.clone(), Some(matches!(**value, HirExpr::NoneLiteral)), true),
            _ => return,
        };
        match (first.get(&name), stored) {
            (None, false) => {
                first.insert(name, nones);
            }
            // A list of `None`s storing anything else, or a list of `int`s
            // storing `None`.
            (Some(Some(built)), true) if nones != Some(*built) => {
                widened.insert(name);
            }
            _ => {}
        }
    });
    widened
}
