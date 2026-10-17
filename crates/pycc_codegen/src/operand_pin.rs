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
//! The exception is an `object` name, which may be a module global that
//! CPython code run by a sibling rebinds through a nested exec of the same
//! module body.
//! An operand followed only by side-effect-free siblings -- reads,
//! primitive arithmetic and comparisons -- is not pinned either, so hot
//! numeric code pays nothing.
//!
//! **The consuming operation itself** can free an operand too. An `object`
//! operation runs CPython code ([`borrowed_while_running_code`]): `g == h`
//! calls `g.__eq__`, which may re-exec the module body and rebind `h`, and
//! then calls the reflected `h.__eq__`; `d[k]` hashes `k`, whose `__hash__` may
//! rebind `d`, before probing `d`; `print(xs)` runs each element's
//! `__repr__` while it walks `xs`. CPython's C API borrows every operand
//! for the call's duration and relies on the caller's own reference -- the
//! one on CPython's value stack -- to keep it alive. So every `object`
//! operand of such an operation that some *other* activation can rebind is
//! pinned for the operation's duration (`Planner::pins_across_operation`):
//! a module global (a nested exec shares the module's slots with every
//! other activation of the body and of its functions) and a read through
//! an attribute or element (a host setter, or the release of the instance
//! or list it lives in, can drop it). A frame-local `object` slot stays
//! borrowed: only its own frame stores to it -- a callee cannot rebind the
//! caller's locals (a `nonlocal` variable lives in a list cell and is read
//! through an element, which is pinned; compiled frames have no
//! introspection), and a nested exec runs in an activation of its own --
//! and no operand evaluation stores to it. Native operations run no CPython code
//! and are never consumers, so code without `object` operations is
//! unchanged.
//!
//! The marks are keyed by node address and live for one module's code
//! generation ([`plan_module`] installs them, the returned guard clears
//! them); the MIR is not mutated while it is emitted.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashSet};

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
    let globals: HashSet<String> = crate::collect_module_bindings(mir).into_keys().collect();
    for item in &mir.items {
        match item {
            MirItem::Function { params, body, .. } => {
                // A name the function binds -- a parameter or a local slot,
                // the set codegen declares for it -- shadows the global.
                let mut locals = BTreeMap::new();
                for stmt in body {
                    crate::collect_stmt_bindings(stmt, &mut locals);
                }
                let visible: HashSet<String> = globals
                    .iter()
                    .filter(|name| {
                        !locals.contains_key(*name) && !params.iter().any(|(p, _)| p == *name)
                    })
                    .cloned()
                    .collect();
                Planner {
                    rt,
                    globals: &visible,
                }
                .body(body);
            }
            MirItem::TopLevelStmt(stmt) => Planner {
                rt,
                globals: &globals,
            }
            .stmt(stmt),
            MirItem::ForeignImport { .. } => {}
        }
    }
    guard
}

/// The planning walk over one function body or the module body.
struct Planner<'a, 'ctx> {
    rt: &'a RtFns<'ctx>,
    /// The module globals a bare name reads here.
    globals: &'a HashSet<String>,
}

impl Planner<'_, '_> {
    fn body(&self, body: &[MirStmt]) {
        for stmt in body {
            self.stmt(stmt);
        }
    }

    fn stmt(&self, stmt: &MirStmt) {
        let exprs = stmt.exprs();
        for expr in &exprs {
            self.expr(expr);
        }
        // A statement's own operands run in sequence too: `p.q.x = f(p)`
        // evaluates the target's base before the value.
        pin_operands(self.rt, &exprs);
        if stmt_runs_code_on_operands(stmt) {
            self.pin_across_operation(&exprs);
        }
        for body in stmt.bodies() {
            self.body(body);
        }
    }

    fn expr(&self, expr: &MirExpr) {
        let children = expr.children();
        for child in &children {
            self.expr(child);
        }
        if evaluates_operands_in_sequence(expr) {
            pin_operands(self.rt, &children);
        }
        self.pin_across_operation(&borrowed_while_running_code(expr));
    }

    /// Pins each operand of a code-running operation that another
    /// activation can drop ([`Self::pins_across_operation`]).
    fn pin_across_operation(&self, operands: &[&MirExpr]) {
        for operand in operands {
            if self.pins_across_operation(operand) {
                PINNED.with(|pinned| pinned.borrow_mut().insert(key(operand)));
            }
        }
    }

    /// Whether `operand` is a borrowed `object` read that code run by the
    /// operation consuming it can drop: a module global, or a read through
    /// an attribute or a list element. A frame-local name is not (see the
    /// module doc).
    fn pins_across_operation(&self, operand: &MirExpr) -> bool {
        let rebindable = match operand {
            MirExpr::Sequence { value, .. } => return self.pins_across_operation(value),
            MirExpr::Name { name, .. } => self.globals.contains(name),
            MirExpr::AttrGet { .. } => true,
            MirExpr::Subscript { base, .. } => matches!(base.ty(), Ty::List(_) | Ty::VarTuple(_)),
            _ => false,
        };
        rebindable && operand.ty() == Ty::Object && heap_rc::is_borrowed_read(operand)
    }
}

/// The operands `expr` borrows while it runs CPython code: those of an
/// attribute load, `len`, a subscript, a rich comparison, arithmetic,
/// membership, a slice, a tuple, set or dict display (hashing),
/// `isinstance`, an unpack, an f-string, and a builtin's or user
/// function's call (`print`/`str` run `__repr__`/`__str__`). An identity
/// test, `type(o)`, boxing and a list display run no code.
///
/// A call on an object holds a reference of its own to every operand the
/// call runs code with: the shim packs each argument into a new reference,
/// `pycc_ext_obj_call_borrowed` retains a borrowed callee, and a method
/// call's bound method holds its receiver -- which only the lookup before
/// it borrows. A keyword call's positional half is a node of its own.
fn borrowed_while_running_code(expr: &MirExpr) -> Vec<&MirExpr> {
    match expr {
        MirExpr::ObjCompare { op: pycc_mir::CmpOpKind::Is | pycc_mir::CmpOpKind::IsNot, .. } =>
        {
            Vec::new()
        }
        MirExpr::ObjMethodCall { base, .. } => vec![base],
        MirExpr::ObjAttrGet { .. }
        | MirExpr::ObjCompare { .. }
        | MirExpr::ObjLen { .. }
        | MirExpr::ObjSubscript { .. }
        | MirExpr::ObjBinOp { .. }
        | MirExpr::ObjUnaryOp { .. }
        | MirExpr::ObjContains { .. }
        | MirExpr::ObjSlice { .. }
        | MirExpr::ObjDisplay { .. }
        | MirExpr::ObjIsInstance { .. }
        | MirExpr::ObjUnpack { .. }
        | MirExpr::ObjUnpackFloatTuple { .. }
        | MirExpr::Call { .. }
        | MirExpr::FString(_) => expr.children(),
        _ => Vec::new(),
    }
}

/// [`borrowed_while_running_code`] for a statement: an attribute or item
/// store or deletion on a CPython object (`__setattr__`, `__setitem__`,
/// the key's `__hash__`).
fn stmt_runs_code_on_operands(stmt: &MirStmt) -> bool {
    matches!(
        stmt,
        MirStmt::ObjAttrSet { .. }
            | MirStmt::ObjDelAttr { .. }
            | MirStmt::ObjSetItem { .. }
            | MirStmt::ObjDelItem { .. }
            | MirStmt::ObjDelSlice { .. }
            | MirStmt::ObjSetSlice { .. }
    )
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
        // An `object` name may be a module global, which a sibling running
        // CPython code can rebind: a nested exec of the same module body
        // (it re-imports itself, or is imported under a second name) shares
        // the global's slot.
        MirExpr::Name { ty: Ty::Object, .. } => true,
        // A tuple element is never counted (and `ty()` of a malformed
        // tuple subscript is codegen's error to report, not this pass's).
        MirExpr::Subscript { base, .. } => matches!(base.ty(), Ty::List(_) | Ty::VarTuple(_)),
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
        | MirExpr::NumWrap(_)
        | MirExpr::OptionalUnwrap(..)
        | MirExpr::AttrGet { .. }
        // A typed array operation reads or writes the array's own memory
        // and runs no user code (`ndarray`).
        | MirExpr::Nd(_) => true,
        MirExpr::Subscript { base, .. } => {
            matches!(base.ty(), Ty::List(_) | Ty::VarTuple(_) | Ty::Tuple(_))
        }
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
