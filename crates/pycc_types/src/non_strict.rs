//! Non-strict mode's type-checking rules (#3). What non-strict mode mostly
//! changes is the HIR lowering, which turns a missing annotation and `Any`
//! into `object` (`pycc_hir`'s `NON_STRICT_MARKER`); the checker adds three
//! rules, each admitted only in non-strict mode:
//!
//! - **Run-time conversion.** A CPython object passed to a compiled
//!   function's `int`/`float`/`bool`/`str` parameter, or returned from a
//!   function declared to return one, is converted with a type check
//!   (`TypeError` when the object is not of that type) instead of being
//!   refused ([`admits_coercion`]).
//! - **Object displays.** A display with an object element, or whose
//!   elements have no native representation, is the CPython container
//!   (`foreign::arith::object_display_ty`).
//! - **Widening.** A name rebound with a value of another type (`t = 0`
//!   then `t += x` for an object `x`, or `x = 1` then `x = "a"`) cannot keep
//!   one native type, so it becomes `object`, as every Python name is: the
//!   checker records the name ([`request_widen`]), and [`with_widening`]
//!   re-checks the module with the name's first plain assignment in each
//!   scope rewritten into `name: object = value`. Later rebindings box into
//!   that slot, which the checker already admits.
//!
//! The mode is per check run and per thread: the driver sets it with
//! [`set_non_strict`] right before it links and checks a program.

use pycc_hir::{HirItem, HirModule, HirStmt, Ty};
use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;

thread_local! {
    static NON_STRICT: Cell<bool> = const { Cell::new(false) };
    static WIDEN: RefCell<BTreeSet<String>> = const { RefCell::new(BTreeSet::new()) };
    /// `(class, attribute)` slots to widen ([`request_attr_widen`]).
    static WIDEN_ATTRS: RefCell<BTreeSet<(String, String)>> =
        const { RefCell::new(BTreeSet::new()) };
}

/// Selects (or clears) non-strict checking for this thread.
pub fn set_non_strict(non_strict: bool) {
    NON_STRICT.with(|cell| cell.set(non_strict));
}

/// Whether this thread checks in non-strict mode.
pub(crate) fn is_non_strict() -> bool {
    NON_STRICT.with(Cell::get)
}

/// Whether the program this thread last checked was checked in non-strict
/// mode ([`set_non_strict`]), for the driver's later stages.
pub fn non_strict_checked() -> bool {
    is_non_strict()
}

thread_local! {
    static EMPTY_CONTAINER_PASS: Cell<bool> = const { Cell::new(false) };
}

/// Runs `f` as the empty-container pre-pass: an empty display does not
/// infer as the CPython container there. That rule is for a `[]` the pass
/// left unresolved; inside the pass it would bind `xs = []` to an object
/// and make the pass type `xs` from that binding instead of from
/// `xs.append(v)`.
pub(crate) fn in_empty_container_pass<T>(f: impl FnOnce() -> T) -> T {
    let previous = EMPTY_CONTAINER_PASS.with(|cell| cell.replace(true));
    let result = f();
    EMPTY_CONTAINER_PASS.with(|cell| cell.set(previous));
    result
}

/// Whether the empty-container pre-pass is running ([`in_empty_container_pass`]).
pub(crate) fn empty_display_untyped() -> bool {
    EMPTY_CONTAINER_PASS.with(Cell::get)
}

/// Whether a value of type `from` reaches a parameter or return slot of type
/// `to` through the run-time conversion: only in non-strict mode, and only
/// from `object` to one of the four scalars.
pub(crate) fn admits_coercion(from: &Ty, to: &Ty) -> bool {
    is_non_strict() && *from == Ty::Object && matches!(to, Ty::Int | Ty::Float | Ty::Bool | Ty::Str)
}

/// Whether an index (a list, tuple or `str` subscript, a `range()`
/// argument) of type `ty` is read through `__index__` at run time: an
/// object, in non-strict mode only (`pycc_mir::MirExpr::ObjectIndex`).
pub(crate) fn admits_index(ty: &Ty) -> bool {
    is_non_strict() && *ty == Ty::Object
}

/// Records that `name` was rebound with a value of another type, when both
/// types fit an `object` slot (`boxable`). The caller still reports its
/// diagnostic; [`with_widening`] decides whether a retry removes it.
pub(crate) fn request_widen(name: &str, previous: &Ty, new: &Ty, boxable: impl Fn(&Ty) -> bool) {
    if is_non_strict() && boxable(previous) && boxable(new) {
        WIDEN.with(|widen| widen.borrow_mut().insert(name.to_string()));
    }
}

/// Records that attribute `attr` of `class` (or of the base that declares
/// it) was stored a value of another type, when both fit an `object` slot:
/// the slot becomes `object`, as every Python attribute is
/// ([`with_widening`]).
pub(crate) fn request_attr_widen(
    env: &crate::Environment,
    class: &str,
    attr: &str,
    slot: &Ty,
    value: &Ty,
) {
    let boxable = |ty: &Ty| matches!(ty, Ty::Object | Ty::None) || crate::foreign::is_object_operand(env, ty);
    if is_non_strict() && boxable(slot) && boxable(value) {
        WIDEN_ATTRS.with(|widen| {
            widen
                .borrow_mut()
                .insert((class.to_string(), attr.to_string()))
        });
    }
}

thread_local! {
    /// The names some statement of the module being checked unbinds: a
    /// `del` target or an `except ... as` name (CPython deletes it when the
    /// handler ends). See [`admits_maybe_read`].
    static UNBOUND_BY_STATEMENT: RefCell<std::collections::HashSet<String>> =
        RefCell::new(std::collections::HashSet::new());
}

/// Whether a read of `name`, bound on some paths reaching it only, is
/// admitted: in non-strict mode it is, as every global and non-parameter
/// local carries a run-time initialized flag (`pycc_codegen`'s
/// `StorageSlot::initialized`) and an unbound read raises CPython's
/// `NameError` / `UnboundLocalError`. A name some statement unbinds again
/// (`del`, an `except ... as` name) keeps the static refusal: its flag
/// stays set after the unbinding.
pub(crate) fn admits_maybe_read(name: &str) -> bool {
    is_non_strict() && !UNBOUND_BY_STATEMENT.with(|names| names.borrow().contains(name))
}

/// Records the names `hir`'s statements unbind ([`admits_maybe_read`]).
fn record_unbinding_statements(hir: &HirModule) {
    fn handler_names(body: &[HirStmt], out: &mut std::collections::HashSet<String>) {
        for stmt in body {
            match stmt {
                HirStmt::If { body, orelse, .. } => {
                    handler_names(body, out);
                    handler_names(orelse, out);
                }
                HirStmt::While { body, .. }
                | HirStmt::ForRange { body, .. }
                | HirStmt::ForList { body, .. }
                | HirStmt::ForObject { body, .. } => handler_names(body, out),
                HirStmt::LoopElse { loop_stmt, orelse } => {
                    handler_names(std::slice::from_ref(loop_stmt.as_ref()), out);
                    handler_names(orelse, out);
                }
                HirStmt::Match { cases, .. } => {
                    for case in cases {
                        handler_names(&case.body, out);
                    }
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
                    handler_names(body, out);
                    for handler in handlers {
                        out.extend(handler.name.iter().cloned());
                        handler_names(&handler.body, out);
                    }
                    handler_names(orelse, out);
                    handler_names(finalbody, out);
                }
                _ => {}
            }
        }
    }
    let mut names = std::collections::HashSet::new();
    for item in &hir.items {
        let body = match item {
            HirItem::Function { body, .. } => body.as_slice(),
            HirItem::TopLevelStmt(stmt) => std::slice::from_ref(stmt),
        };
        names.extend(pycc_hir::deleted_names(body));
        handler_names(body, &mut names);
    }
    UNBOUND_BY_STATEMENT.with(|cell| *cell.borrow_mut() = names);
}

fn take_widen_requests() -> BTreeSet<String> {
    WIDEN.with(|widen| std::mem::take(&mut *widen.borrow_mut()))
}

/// Runs `check` over `hir`, and in non-strict mode re-runs it with every
/// requested name widened to `object` until it passes or a run requests no
/// new name (its diagnostics are then the answer).
pub(crate) fn with_widening<T, E>(
    hir: &HirModule,
    check: impl Fn(&HirModule) -> Result<T, E>,
) -> Result<T, E> {
    if !is_non_strict() {
        return check(hir);
    }
    record_unbinding_statements(hir);
    // A name first bound to `None` and rebound to another value in the same
    // scope is widened up front: a read reached before any rebinding in
    // the checker's single pass (`last = None` before a loop that sets
    // `last = s` in one branch and reads `len(last)` in another) would
    // otherwise see `None` and never request the widening.
    let mut widened = none_first_names(hir);
    let mut widened_attrs = BTreeSet::new();
    let mut current = (!widened.is_empty()).then(|| widen(hir, &widened, &widened_attrs));
    loop {
        take_widen_requests();
        let result = check(current.as_ref().unwrap_or(hir));
        let requested = retarget_temporaries(hir, take_widen_requests());
        let requested_attrs = WIDEN_ATTRS.with(|widen| std::mem::take(&mut *widen.borrow_mut()));
        if result.is_ok()
            || (requested.is_subset(&widened) && requested_attrs.is_subset(&widened_attrs))
        {
            return result;
        }
        widened.extend(requested);
        widened_attrs.extend(requested_attrs);
        current = Some(widen(hir, &widened, &widened_attrs));
    }
}

/// Each requested desugaring temporary (`_pycc_N`) that holds an item of a
/// named container (`_pycc_1 = rows[i]`, for `rows[i][j] = v`) is replaced
/// by that container's name: widening the temporary alone would copy the
/// item out of its container, while a widened container hands out its own
/// CPython items, so a store through one is seen through the other.
fn retarget_temporaries(hir: &HirModule, requested: BTreeSet<String>) -> BTreeSet<String> {
    requested
        .into_iter()
        .map(|name| {
            if !name.starts_with("_pycc_") {
                return name;
            }
            let mut root = None;
            for item in &hir.items {
                let body = match item {
                    HirItem::Function { body, .. } => body.as_slice(),
                    HirItem::TopLevelStmt(stmt) => std::slice::from_ref(stmt),
                };
                if root.is_none() {
                    root = item_root(body, &name);
                }
            }
            root.unwrap_or(name)
        })
        .collect()
}

/// The container a temporary's first assignment reads an item of.
fn item_root(body: &[HirStmt], name: &str) -> Option<String> {
    for stmt in body {
        let found = match stmt {
            HirStmt::Assign { target, value } if target == name => {
                let mut expr = value;
                let mut subscripted = false;
                while let pycc_hir::HirExpr::Subscript { base, .. } = expr {
                    expr = base;
                    subscripted = true;
                }
                return match expr {
                    pycc_hir::HirExpr::Name(root) if subscripted => Some(root.clone()),
                    _ => None,
                };
            }
            HirStmt::If { body, orelse, .. } => {
                item_root(body, name).or_else(|| item_root(orelse, name))
            }
            HirStmt::While { body, .. }
            | HirStmt::ForRange { body, .. }
            | HirStmt::ForList { body, .. }
            | HirStmt::ForObject { body, .. } => item_root(body, name),
            HirStmt::LoopElse { loop_stmt, orelse } => {
                item_root(std::slice::from_ref(loop_stmt.as_ref()), name)
                    .or_else(|| item_root(orelse, name))
            }
            HirStmt::Try {
                body,
                handlers,
                orelse,
                finalbody,
            } => item_root(body, name)
                .or_else(|| handlers.iter().find_map(|h| item_root(&h.body, name)))
                .or_else(|| item_root(orelse, name))
                .or_else(|| item_root(finalbody, name)),
            _ => None,
        };
        if found.is_some() {
            return found;
        }
    }
    None
}

/// The names some scope of `hir` first binds to `None` with a plain
/// assignment and binds again, in the same scope, to anything else.
fn none_first_names(hir: &HirModule) -> BTreeSet<String> {
    fn walk(
        body: &[HirStmt],
        first_none: &mut std::collections::BTreeMap<String, bool>,
        rebound: &mut BTreeSet<String>,
    ) {
        for stmt in body {
            match stmt {
                HirStmt::Assign { target, value } => {
                    let is_none = matches!(value, pycc_hir::HirExpr::NoneLiteral);
                    let first = *first_none.entry(target.clone()).or_insert(is_none);
                    if first && !is_none {
                        rebound.insert(target.clone());
                    }
                }
                HirStmt::AnnAssign { target, .. } => {
                    first_none.entry(target.clone()).or_insert(false);
                }
                HirStmt::If { body, orelse, .. } => {
                    walk(body, first_none, rebound);
                    walk(orelse, first_none, rebound);
                }
                HirStmt::While { body, .. }
                | HirStmt::ForRange { body, .. }
                | HirStmt::ForList { body, .. }
                | HirStmt::ForObject { body, .. } => walk(body, first_none, rebound),
                HirStmt::LoopElse { loop_stmt, orelse } => {
                    walk(std::slice::from_ref(loop_stmt.as_ref()), first_none, rebound);
                    walk(orelse, first_none, rebound);
                }
                HirStmt::Match { cases, .. } => {
                    for case in cases {
                        walk(&case.body, first_none, rebound);
                    }
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
                    walk(body, first_none, rebound);
                    for handler in handlers {
                        walk(&handler.body, first_none, rebound);
                    }
                    walk(orelse, first_none, rebound);
                    walk(finalbody, first_none, rebound);
                }
                _ => {}
            }
        }
    }
    let mut rebound = BTreeSet::new();
    let mut top_level = std::collections::BTreeMap::new();
    for item in &hir.items {
        match item {
            HirItem::Function { params, body, .. } => {
                let mut first_none = params
                    .iter()
                    .map(|(name, _)| (name.clone(), false))
                    .collect();
                walk(body, &mut first_none, &mut rebound);
            }
            HirItem::TopLevelStmt(stmt) => {
                walk(std::slice::from_ref(stmt), &mut top_level, &mut rebound);
            }
        }
    }
    rebound
}

/// `hir` with the first plain assignment of each name in `names`, in every
/// function body and in the module body, rewritten into `name: object =
/// value`.
fn widen(
    hir: &HirModule,
    names: &BTreeSet<String>,
    attrs: &BTreeSet<(String, String)>,
) -> HirModule {
    let mut module = hir.clone();
    // Each widened attribute slot is `object` in the class that declares
    // it and in every class of the store's MRO that redeclares it.
    for (class, attr) in attrs {
        let mro = module
            .class_defs
            .iter()
            .find(|(name, _)| name == class)
            .map(|(_, def)| def.mro.clone())
            .unwrap_or_default();
        for (name, def) in &mut module.class_defs {
            if !mro.contains(name) && name != class {
                continue;
            }
            for (slot, ty) in &mut def.attrs {
                if slot == attr {
                    *ty = Ty::Object;
                }
            }
        }
    }
    let mut top_level_seen = BTreeSet::new();
    for item in &mut module.items {
        match item {
            HirItem::Function { params, body, .. } => {
                // A parameter already has its declared slot.
                let mut seen: BTreeSet<String> =
                    params.iter().map(|(name, _)| name.clone()).collect();
                widen_block(body, names, &mut seen);
            }
            HirItem::TopLevelStmt(stmt) => {
                widen_block(std::slice::from_mut(stmt), names, &mut top_level_seen);
            }
        }
    }
    module
}

fn widen_block(body: &mut [HirStmt], names: &BTreeSet<String>, seen: &mut BTreeSet<String>) {
    for stmt in body {
        let is_list_comp = matches!(stmt, HirStmt::ListCompAssign { .. });
        match stmt {
            HirStmt::Assign { target, value }
                if names.contains(target) && !seen.contains(target) =>
            {
                seen.insert(target.clone());
                *stmt = HirStmt::AnnAssign {
                    target: target.clone(),
                    annotation: Ty::Object,
                    value: Some(value.clone()),
                    is_final: false,
                };
            }
            HirStmt::AnnAssign { target, .. } => {
                seen.insert(target.clone());
            }
            // A comprehension's statement form, as its expression.
            HirStmt::ListCompAssign {
                target,
                var,
                iter,
                cond,
                elt,
            }
            | HirStmt::SetCompAssign {
                target,
                var,
                iter,
                cond,
                elt,
            } if names.contains(target) && !seen.contains(target) => {
                seen.insert(target.clone());
                let elt = if is_list_comp {
                    pycc_hir::CompElt::List(elt.as_ref().clone())
                } else {
                    pycc_hir::CompElt::Set(elt.as_ref().clone())
                };
                let value =
                    pycc_hir::HirExpr::Comprehension(Box::new(pycc_hir::HirComprehension {
                        var: var.clone(),
                        iter: iter.clone(),
                        cond: cond.as_deref().cloned(),
                        elt,
                    }));
                *stmt = HirStmt::AnnAssign {
                    target: target.clone(),
                    annotation: Ty::Object,
                    value: Some(value),
                    is_final: false,
                };
            }
            // Each branch is checked from the environment before the
            // statement, so each declares the slot of a name it binds
            // first.
            HirStmt::If { body, orelse, .. } => {
                let mut orelse_seen = seen.clone();
                widen_block(body, names, seen);
                widen_block(orelse, names, &mut orelse_seen);
                seen.extend(orelse_seen);
            }
            HirStmt::While { body, .. }
            | HirStmt::ForRange { body, .. }
            | HirStmt::ForList { body, .. }
            | HirStmt::ForObject { body, .. } => widen_block(body, names, seen),
            HirStmt::LoopElse { loop_stmt, orelse } => {
                widen_block(std::slice::from_mut(loop_stmt.as_mut()), names, seen);
                widen_block(orelse, names, seen);
            }
            HirStmt::Match { cases, .. } => {
                for case in cases {
                    widen_block(&mut case.body, names, seen);
                }
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
                widen_block(body, names, seen);
                for handler in handlers {
                    widen_block(&mut handler.body, names, seen);
                }
                widen_block(orelse, names, seen);
                widen_block(finalbody, names, seen);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pycc_hir::HirExpr;

    #[test]
    fn only_non_strict_converts_an_object_into_a_scalar_slot() {
        set_non_strict(false);
        assert!(!admits_coercion(&Ty::Object, &Ty::Int));
        set_non_strict(true);
        for to in [Ty::Int, Ty::Float, Ty::Bool, Ty::Str] {
            assert!(admits_coercion(&Ty::Object, &to), "{to:?}");
        }
        assert!(!admits_coercion(&Ty::Object, &Ty::List(Box::new(Ty::Int))));
        assert!(!admits_coercion(&Ty::Int, &Ty::Str));
        set_non_strict(false);
    }

    #[test]
    fn widening_rewrites_the_first_assignment_in_each_scope_only() {
        let assign = |value| HirStmt::Assign {
            target: "t".to_string(),
            value: HirExpr::IntLiteral(value),
        };
        let mut body = vec![
            HirStmt::If {
                test: HirExpr::BoolLiteral(true),
                body: vec![assign(1)],
                orelse: vec![],
            },
            assign(2),
        ];
        let names = BTreeSet::from(["t".to_string()]);
        widen_block(&mut body, &names, &mut BTreeSet::new());
        let HirStmt::If { body: inner, .. } = &body[0] else {
            unreachable!()
        };
        assert!(matches!(
            &inner[0],
            HirStmt::AnnAssign {
                annotation: Ty::Object,
                ..
            }
        ));
        assert_eq!(body[1], assign(2));
    }
}
