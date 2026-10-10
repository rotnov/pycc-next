//! Operands that must hold their own reference while a later sibling runs.
//!
//! `heap_rc`'s ownership model lets a *borrowed* read (`p.name`, `xs[0]`)
//! be used without touching its count: the storage it was read from keeps
//! owning it. That holds only while nothing can drop the storage's
//! reference, and in `p.name == f(p)` the call runs after `p.name` was
//! read and before `==` uses it -- if `f` rebinds `p.name`, the string is
//! freed under the comparison. CPython never has this problem because
//! every operand on its value stack is a new reference.
//!
//! This module finds, before any code is emitted, every operand that is a
//! borrowed read of a counted value through an attribute or element
//! (`AttrGet`/`Subscript` chains) and is followed, within the same
//! operation, by a sibling that [`may_release`] -- one that can run user
//! code or store anywhere. Such an operand is **pinned**: `emit_expr`
//! takes a reference right after evaluating it, and
//! `heap_rc::is_borrowed_read` answers `false` for it, so every consumer
//! treats it as the fresh reference it now is and releases (or stores) it
//! exactly once. Both decisions read the same per-node mark, so they cannot
//! disagree.
//!
//! A bare name is never pinned: a frame slot cannot be rebound by a call
//! (functions have no `global`/`nonlocal` rebinding, and a walrus binds only
//! uncounted scalars, T0050), and it owns its value for the whole frame.
//! An operand followed only by side-effect-free siblings -- reads,
//! primitive arithmetic and comparisons -- is not pinned either, so hot
//! numeric code pays nothing.
//!
//! The marks are keyed by node address and live for one module's code
//! generation ([`plan_module`] installs them, the returned guard clears
//! them); the MIR is not mutated while it is emitted.

use std::cell::RefCell;
use std::collections::HashSet;

use pycc_mir::{MirExpr, MirItem, MirModule, MirStmt, Ty};

use crate::RtFns;
use crate::heap_rc;

thread_local! {
    static PINNED: RefCell<HashSet<usize>> = RefCell::new(HashSet::new());
}

fn key(expr: &MirExpr) -> usize {
    std::ptr::from_ref(expr) as usize
}

/// Whether `expr` was pinned by the current module's plan.
pub(crate) fn is_pinned(expr: &MirExpr) -> bool {
    PINNED.with(|pinned| {
        let pinned = pinned.borrow();
        !pinned.is_empty() && pinned.contains(&key(expr))
    })
}

/// Clears the marks when a module's code generation ends.
#[must_use = "the marks stay installed until the guard is dropped"]
pub(crate) struct PlanGuard(());

impl Drop for PlanGuard {
    fn drop(&mut self) {
        PINNED.with(|pinned| pinned.borrow_mut().clear());
    }
}

/// Marks every operand of `mir` that must be pinned.
pub(crate) fn plan_module(rt: &RtFns<'_>, mir: &MirModule) -> PlanGuard {
    PINNED.with(|pinned| pinned.borrow_mut().clear());
    let guard = PlanGuard(());
    for item in &mir.items {
        match item {
            MirItem::Function { body, .. } => plan_body(rt, body),
            MirItem::TopLevelStmt(stmt) => plan_stmt(rt, stmt),
            MirItem::ForeignImport { .. } => {}
        }
    }
    guard
}

fn plan_body(rt: &RtFns<'_>, body: &[MirStmt]) {
    for stmt in body {
        plan_stmt(rt, stmt);
    }
}

fn plan_stmt(rt: &RtFns<'_>, stmt: &MirStmt) {
    let exprs = stmt.exprs();
    for expr in &exprs {
        plan_expr(rt, expr);
    }
    // A statement's own operands run in sequence too: `p.q.x = f(p)`
    // evaluates the target's base before the value.
    pin_operands(rt, &exprs);
    for body in stmt.bodies() {
        plan_body(rt, body);
    }
}

fn plan_expr(rt: &RtFns<'_>, expr: &MirExpr) {
    let children = expr.children();
    for child in &children {
        plan_expr(rt, child);
    }
    if evaluates_operands_in_sequence(expr) {
        pin_operands(rt, &children);
    }
}

/// Whether every child of `expr` is evaluated before `expr` uses any of
/// them. Control-flow nodes (`and`/`or`, a conditional expression, a
/// comprehension) use each child before the next one runs, and a
/// `Sequence` discards its first child.
fn evaluates_operands_in_sequence(expr: &MirExpr) -> bool {
    !matches!(
        expr,
        MirExpr::BoolOp { .. }
            | MirExpr::IfExp { .. }
            | MirExpr::Comprehension(_)
            | MirExpr::Sequence { .. }
            | MirExpr::NamedExpr { .. }
    )
}

/// Pins each operand in `operands` that is a counted borrowed read through
/// an attribute or element and has a later sibling that may release.
fn pin_operands(rt: &RtFns<'_>, operands: &[&MirExpr]) {
    let Some((_, earlier)) = operands.split_last() else {
        return;
    };
    for (index, operand) in earlier.iter().enumerate() {
        if is_pinnable_read(rt, operand) && operands[index + 1..].iter().any(|o| may_release(o)) {
            PINNED.with(|pinned| pinned.borrow_mut().insert(key(operand)));
        }
    }
}

fn is_pinnable_read(rt: &RtFns<'_>, expr: &MirExpr) -> bool {
    let through_storage = match expr {
        MirExpr::Sequence { value, .. } => {
            return is_pinnable_read(rt, value);
        }
        MirExpr::AttrGet { .. } => true,
        // A tuple element is never counted (and `ty()` of a malformed
        // tuple subscript is codegen's error to report, not this pass's).
        MirExpr::Subscript { base, .. } => matches!(base.ty(), Ty::List(_)),
        _ => false,
    };
    through_storage
        && heap_rc::heap_kind(rt, &expr.ty()).is_some()
        && heap_rc::is_borrowed_read(expr)
}

/// A type whose operators run no user code.
fn is_primitive(ty: &Ty) -> bool {
    matches!(ty, Ty::Int | Ty::Float | Ty::Bool | Ty::Str)
}

/// Whether evaluating `expr` may drop a reference some storage holds: run
/// user or CPython code, store into a slot, an element or a name, or
/// mutate a container. An allowlist of shapes that cannot answers `false`;
/// every other shape answers `true`.
pub(crate) fn may_release(expr: &MirExpr) -> bool {
    let inert = match expr {
        MirExpr::IntLiteral(_)
        | MirExpr::FloatLiteral(_)
        | MirExpr::BoolLiteral(_)
        | MirExpr::StringLiteral(_)
        | MirExpr::NoneLiteral
        | MirExpr::EmptyList(_)
        | MirExpr::EmptyDict(_)
        | MirExpr::NullInstance { .. }
        | MirExpr::Name { .. }
        | MirExpr::IntBoundary(_)
        | MirExpr::OptionalWrap(..)
        | MirExpr::OptionalUnwrap(..)
        | MirExpr::AttrGet { .. } => true,
        MirExpr::Subscript { base, .. } => matches!(base.ty(), Ty::List(_) | Ty::Tuple(_)),
        MirExpr::BinOp { left, right, .. }
        | MirExpr::Compare { left, right, .. }
        | MirExpr::BoolOp { left, right, .. } => {
            is_primitive(&left.ty()) && is_primitive(&right.ty())
        }
        MirExpr::Not(operand) => is_primitive(&operand.ty()),
        MirExpr::IfExp { test, .. } => is_primitive(&test.ty()),
        MirExpr::FString(_) => expr.children().iter().all(|part| is_primitive(&part.ty())),
        MirExpr::Call { callee, args, .. } => {
            callee == "len"
                && matches!(args.as_slice(), [arg] if matches!(arg.ty(), Ty::List(_) | Ty::Str))
        }
        _ => false,
    };
    !inert || expr.children().into_iter().any(may_release)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(name: &str, ty: Ty) -> MirExpr {
        MirExpr::Name {
            name: name.to_string(),
            ty,
        }
    }

    #[test]
    fn reads_and_primitive_operators_are_inert_and_calls_are_not() {
        let xs = name("xs", Ty::List(Box::new(Ty::Str)));
        let element = MirExpr::Subscript {
            base: Box::new(xs.clone()),
            index: Box::new(MirExpr::IntLiteral(0)),
        };
        assert!(!may_release(&element));
        let concat = MirExpr::BinOp {
            op: pycc_mir::BinOpKind::Add,
            left: Box::new(element.clone()),
            right: Box::new(MirExpr::StringLiteral("a".to_string())),
            ty: Ty::Str,
        };
        assert!(!may_release(&concat));
        let call = MirExpr::Call {
            callee: "f".to_string(),
            args: vec![xs.clone()],
            ty: Ty::Str,
        };
        assert!(may_release(&call));
        let nested = MirExpr::BinOp {
            op: pycc_mir::BinOpKind::Add,
            left: Box::new(element),
            right: Box::new(call),
            ty: Ty::Str,
        };
        assert!(may_release(&nested));
        let instance = Ty::Instance(Box::new("P".to_string()));
        let instance_eq = MirExpr::Compare {
            op: pycc_mir::CmpOpKind::Eq,
            left: Box::new(name("a", instance.clone())),
            right: Box::new(name("b", instance)),
            ty: Ty::Bool,
        };
        assert!(may_release(&instance_eq));
        assert!(!may_release(&MirExpr::Call {
            callee: "len".to_string(),
            args: vec![xs],
            ty: Ty::Int,
        }));
    }
}
