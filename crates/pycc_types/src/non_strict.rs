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
}

/// Selects (or clears) non-strict checking for this thread.
pub fn set_non_strict(non_strict: bool) {
    NON_STRICT.with(|cell| cell.set(non_strict));
}

/// Whether this thread checks in non-strict mode.
pub(crate) fn is_non_strict() -> bool {
    NON_STRICT.with(Cell::get)
}

/// Whether a value of type `from` reaches a parameter or return slot of type
/// `to` through the run-time conversion: only in non-strict mode, and only
/// from `object` to one of the four scalars.
pub(crate) fn admits_coercion(from: &Ty, to: &Ty) -> bool {
    is_non_strict() && *from == Ty::Object && matches!(to, Ty::Int | Ty::Float | Ty::Bool | Ty::Str)
}

/// Records that `name` was rebound with a value of another type, when both
/// types fit an `object` slot (`boxable`). The caller still reports its
/// diagnostic; [`with_widening`] decides whether a retry removes it.
pub(crate) fn request_widen(name: &str, previous: &Ty, new: &Ty, boxable: impl Fn(&Ty) -> bool) {
    if is_non_strict() && boxable(previous) && boxable(new) {
        WIDEN.with(|widen| widen.borrow_mut().insert(name.to_string()));
    }
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
    let mut widened = BTreeSet::new();
    let mut current = None;
    loop {
        take_widen_requests();
        let result = check(current.as_ref().unwrap_or(hir));
        let requested = take_widen_requests();
        if result.is_ok() || requested.is_subset(&widened) {
            return result;
        }
        widened.extend(requested);
        current = Some(widen(hir, &widened));
    }
}

/// `hir` with the first plain assignment of each name in `names`, in every
/// function body and in the module body, rewritten into `name: object =
/// value`.
fn widen(hir: &HirModule, names: &BTreeSet<String>) -> HirModule {
    let mut module = hir.clone();
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
            HirStmt::If { body, orelse, .. } => {
                widen_block(body, names, seen);
                widen_block(orelse, names, seen);
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
