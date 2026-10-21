//! MIR lowering of chained comparisons `a < b < c` (#1212, Part 4 of
//! #1018), and the comparison dunder lookup between two instances of one
//! class it shares with the single comparison.

use crate::{CmpOpKind, HirClassDef, MirExpr, Ty};
use std::collections::HashMap;

/// One `op right` link of a [`MirExpr::CompareChain`]. The link's left
/// operand is the previous link's `right`, or the chain's `first`.
#[derive(Debug, Clone, PartialEq)]
pub struct MirCompareLink {
    pub kind: MirCompareKind,
    pub right: MirExpr,
}

/// How one chain link compares its two operands.
#[derive(Debug, Clone, PartialEq)]
pub enum MirCompareKind {
    /// A primitive comparison, emitted exactly as a single
    /// [`MirExpr::Compare`] with this operator would be.
    Plain(CmpOpKind),
    /// A comparison between two instances of one class that defines the
    /// operator's dunder (`__eq__`, `__lt__`, ...): a call to it (`callee`,
    /// the mangled method name) with the link's two already-evaluated
    /// operands, inverted when `negate` (`!=` through `__eq__`). A single
    /// comparison rewrites this shape into a [`MirExpr::Call`]; a chain
    /// cannot, because the call would own the middle operand's expression
    /// and evaluate it a second time in the next link.
    InstanceDunder { callee: String, negate: bool },
}

/// The comparison dunder `op` calls on an instance: `!=` falls back to
/// `__eq__`, inverted, when the class defines no `__ne__`.
fn dunder_candidates(op: CmpOpKind) -> &'static [(&'static str, bool)] {
    match op {
        CmpOpKind::Eq => &[("__eq__", false)],
        CmpOpKind::NotEq => &[("__ne__", false), ("__eq__", true)],
        CmpOpKind::Lt => &[("__lt__", false)],
        CmpOpKind::LtE => &[("__le__", false)],
        CmpOpKind::Gt => &[("__gt__", false)],
        CmpOpKind::GtE => &[("__ge__", false)],
        _ => &[],
    }
}

/// The mangled comparison dunder (and whether its result is inverted) that
/// `left op right` calls when both operands are instances of the same class
/// and a class of its MRO defines it, else `None`: two instances whose
/// class defines no `__eq__` compare by identity (a plain
/// [`MirExpr::Compare`] of the two pointers).
///
/// The type checker admits an ordering operator between instances only when
/// the dunder exists (`pycc_types::compare_chain`), and checks its
/// signature `(self, other) -> bool`.
pub(crate) fn instance_compare_callee(
    op: CmpOpKind,
    left_ty: &Ty,
    right_ty: &Ty,
    classes: &HashMap<String, HirClassDef>,
    scopes: &[HashMap<String, Ty>],
) -> Option<(String, bool)> {
    let (Ty::Instance(left_class), Ty::Instance(right_class)) = (left_ty, right_ty) else {
        return None;
    };
    if left_class != right_class {
        return None;
    }
    let class_def = classes.get(left_class.as_str())?;
    for (method, negate) in dunder_candidates(op) {
        let found = class_def.mro.iter().find_map(|mro_class| {
            classes.get(mro_class.as_str()).and_then(|mro_def| {
                mro_def
                    .methods
                    .iter()
                    .find(|(mn, _)| mn == method)
                    .map(|(_, mangled)| (mro_class, mangled.clone()))
            })
        });
        if let Some((owner, mangled)) = found {
            let callee = crate::receiver_exact::exact_callee(
                left_class.as_str(),
                owner,
                mangled,
                scopes,
                classes,
            );
            // v0.9: a subclass overriding the dunder (`crate::dispatch`).
            let callee = crate::dispatch::route_member(
                class_def,
                method,
                callee,
                scopes,
                classes,
                crate::dispatch::Member::Dunder,
            );
            return Some((callee, *negate));
        }
    }
    None
}

/// Builds a [`MirExpr::CompareChain`] from its already-lowered operands.
/// Each link between two instances of a class defining the operator's
/// dunder becomes a [`MirCompareKind::InstanceDunder`]; an identity link
/// (`is`, or `==` without `__eq__`) between instances compares pointers;
/// every other link is [`MirCompareKind::Plain`].
pub(crate) fn lower_compare_chain(
    first: MirExpr,
    links: Vec<(CmpOpKind, MirExpr)>,
    classes: &HashMap<String, HirClassDef>,
    scopes: &[HashMap<String, Ty>],
) -> MirExpr {
    let mut left_ty = first.ty();
    let mut lowered = Vec::with_capacity(links.len());
    for (op, right) in links {
        let right_ty = right.ty();
        let kind = match instance_compare_callee(op, &left_ty, &right_ty, classes, scopes) {
            Some((callee, negate)) => MirCompareKind::InstanceDunder { callee, negate },
            None => MirCompareKind::Plain(instance_identity_op(op, &left_ty, &right_ty)),
        };
        lowered.push(MirCompareLink { kind, right });
        left_ty = right_ty;
    }
    MirExpr::CompareChain {
        first: Box::new(first),
        links: lowered,
    }
}

/// `is`/`is not` between two class instances is pointer equality, which
/// codegen emits for `==`/`!=` of two instance operands.
pub(crate) fn instance_identity_op(op: CmpOpKind, left_ty: &Ty, right_ty: &Ty) -> CmpOpKind {
    match (op, left_ty, right_ty) {
        (CmpOpKind::Is, Ty::Instance(_), Ty::Instance(_)) => CmpOpKind::Eq,
        (CmpOpKind::IsNot, Ty::Instance(_), Ty::Instance(_)) => CmpOpKind::NotEq,
        _ => op,
    }
}

#[cfg(test)]
mod tests;
