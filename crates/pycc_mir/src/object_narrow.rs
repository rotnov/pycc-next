//! Flow narrowing in MIR lowering: the `Optional` shape of #769 and, since
//! #1476 (Part 3 of #1387), an `object` narrowed back to a native type
//! under `isinstance`.
//!
//! Mirrors `pycc_types::narrow` one layer down. Both layers call the same
//! `pycc_hir` recognizers (`optional_none_test`, `isinstance_test`) and the
//! same class gate (`isinstance_narrow_target`); only the scope lookup is
//! per layer. Where the two walkers' overlays could differ, MIR follows the
//! constraint solver's, whose verdict decides the program (D-220): across
//! a `try`, `stmt::try_stmt` mirrors `pycc_types::constraints::try_stmt`
//! position by position.

use super::{MirExpr, narrowed_ty};
use pycc_hir::{HirClassDef, HirExpr, HirStmt, IsInstancePolarity, NoneTestPolarity, Ty};
use std::collections::HashMap;

/// Which branch of an `if` a recognized test narrows. Only `Orelse` also
/// narrows the continuation after an `if` whose body definitely terminates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum NarrowSide {
    Body,
    Orelse,
}

/// A name's recorded type, or `None` when no frame binds it.
fn scoped_ty(scopes: &[HashMap<String, Ty>], name: &str) -> Option<Ty> {
    scopes
        .iter()
        .rev()
        .find_map(|scope| scope.get(name).cloned())
}

/// The `(name, narrowed type, side)` an `if` test narrows, or `None`:
/// `name is [not] None` on an `Optional` name, or `[not] isinstance(name,
/// C)` on an `object` name with the builtin `isinstance` and an unshadowed,
/// admissible `C`.
pub(super) fn narrowing_target(
    test: &HirExpr,
    scopes: &[HashMap<String, Ty>],
    classes: &HashMap<String, HirClassDef>,
) -> Option<(String, Ty, NarrowSide)> {
    if let Some((name, polarity)) = pycc_hir::optional_none_test(test) {
        let Some(Ty::Optional(inner)) = scoped_ty(scopes, name) else {
            return None;
        };
        let side = match polarity {
            NoneTestPolarity::IsNot => NarrowSide::Body,
            NoneTestPolarity::Is => NarrowSide::Orelse,
        };
        return Some((name.to_string(), *inner, side));
    }
    let (name, class, polarity) = pycc_hir::isinstance_test(test)?;
    let shadowed = |key: &str| scopes.iter().any(|scope| scope.contains_key(key));
    if shadowed("$fn:isinstance")
        || shadowed(class)
        || shadowed(&format!("$fn:{class}"))
        || scoped_ty(scopes, name) != Some(Ty::Object)
    {
        return None;
    }
    let inner =
        pycc_hir::isinstance_narrow_target(class, |class| classes.get(class), classes.values())?;
    let side = match polarity {
        IsInstancePolarity::Positive => NarrowSide::Body,
        IsInstancePolarity::Negated => NarrowSide::Orelse,
    };
    Some((name.to_string(), inner, side))
}

/// The read of a narrowed name: `OptionalUnwrap` over an `Optional` slot,
/// `ObjectUnbox` over an `object` slot.
pub(super) fn narrowed_read(name: &str, slot_ty: Ty, inner: Ty) -> MirExpr {
    let is_object = slot_ty == Ty::Object;
    let slot = Box::new(MirExpr::Name {
        name: name.to_string(),
        ty: slot_ty,
    });
    if is_object {
        MirExpr::ObjectUnbox(slot, Box::new(inner))
    } else {
        MirExpr::OptionalUnwrap(slot, Box::new(inner))
    }
}

/// Whether `value` is a bare read of an `object` name an `isinstance` guard
/// currently narrows -- the value `pycc_types`' first-binding rule keeps at
/// `object` (`check_assignment_boxing`), so a local's first binding from it
/// declares an `object` slot here too.
pub(super) fn is_bare_narrowed_object_read(
    value: &HirExpr,
    scopes: &[HashMap<String, Ty>],
) -> bool {
    matches!(value, HirExpr::Name(name)
        if narrowed_ty(scopes, name).is_some() && scoped_ty(scopes, name) == Some(Ty::Object))
}

/// `value` with a narrowed `object` read replaced by the object itself:
/// the operand an identity test, or a first binding the checker keeps at
/// `object`, must see.
pub(super) fn object_operand(value: MirExpr) -> MirExpr {
    match value {
        MirExpr::ObjectUnbox(object, _) => *object,
        other => other,
    }
}

/// Issue #769 (Part 2 of #747), the early-return continuation shape: if
/// `stmt` is `if name is None: <body that definitely terminates>`, `name`
/// is known to be present (the `Optional`'s inner type) for every
/// statement *after* `stmt` in the same sequential statement list --
/// mirroring `pycc_types::narrow::apply_post_if_narrowing` one layer down,
/// using the same shared `pycc_hir::optional_none_test` recognizer and
/// `pycc_hir::continuation_narrows` admission test that module's own doc
/// comment explains in full. Unlike [`super::push_narrowing`]'s in-branch use in
/// `stmt::lower_stmt`'s own `HirStmt::If` arm (which pairs every push with
/// a [`super::kill_narrowing`] once that one branch finishes lowering), this
/// sentinel is deliberately never popped by its own caller -- it is meant
/// to persist for the rest of the enclosing sequence, exactly like
/// `pycc_types::narrow`'s own overlay entry does when applied directly to
/// (not a clone of) the real `env`. [`super::lower_stmt_sequence`] and the
/// module-level statement walk in `build` call this, once per statement,
/// immediately after lowering it.
///
/// Since #1476 the same holds for `if not isinstance(name, C): <body that
/// definitely terminates>` on an `object` name.
///
/// Only when the surviving `else` leaves the name alone
/// (`pycc_hir::continuation_narrows`, shared with both checker walkers):
/// an `else` that rebinds `o` reaches the rest of the block with `o`
/// rebound, and unboxing it there as the guarded class would raise where
/// CPython computes with the new value (#1476 review).
pub(super) fn apply_post_if_narrowing(
    stmt: &HirStmt,
    scopes: &mut [HashMap<String, Ty>],
    classes: &HashMap<String, HirClassDef>,
) {
    let HirStmt::If { test, body, orelse } = stmt else {
        return;
    };
    let Some((name, inner, NarrowSide::Orelse)) = narrowing_target(test, scopes, classes) else {
        return;
    };
    if pycc_hir::continuation_narrows(body, orelse, &name) {
        super::push_narrowing(scopes, &name, inner);
    }
}

#[cfg(test)]
mod tests;
