//! MIR lowering of chained comparisons `a < b < c` (#1212, Part 4 of
//! #1018), and the same-class dataclass `__eq__` lookup it shares with the
//! single comparison.

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
    /// `==`/`!=` between two instances of the same dataclass: a call to the
    /// synthesized `__eq__` (`callee`, the mangled method name) with the
    /// link's two already-evaluated operands, inverted when `negate`. A
    /// single comparison rewrites this shape into a [`MirExpr::Call`]; a
    /// chain cannot, because the call would own the middle operand's
    /// expression and evaluate it a second time in the next link.
    DataclassEq { callee: String, negate: bool },
}

/// The mangled `__eq__` of `left_ty`'s dataclass when `left_ty` and
/// `right_ty` are instances of the same dataclass, else `None`.
///
/// The type checker admits an instance operand of `==`/`!=` only in this
/// shape (#378); other classes and other operators are `T0021` before MIR.
pub(crate) fn dataclass_eq_callee(
    left_ty: &Ty,
    right_ty: &Ty,
    classes: &HashMap<String, HirClassDef>,
) -> Option<String> {
    let (Ty::Instance(left_class), Ty::Instance(right_class)) = (left_ty, right_ty) else {
        return None;
    };
    if left_class != right_class {
        return None;
    }
    let class_def = classes.get(left_class.as_str())?;
    if !class_def.is_dataclass {
        return None;
    }
    let eq_mangled = class_def.mro.iter().find_map(|mro_class| {
        // Every class in the MRO was registered when the class was lowered;
        // `.expect` (whose panic path lives in libcore, outside this crate's
        // instrumented regions) avoids a `?` whose `None` branch is
        // structurally unreachable and would stay permanently uncovered
        // under D-014's 100% coverage gate.
        let mro_def = classes
            .get(mro_class.as_str())
            .expect("MRO class must be registered");
        mro_def
            .methods
            .iter()
            .find(|(mn, _)| mn == "__eq__")
            .map(|(_, mangled)| mangled.clone())
    });
    // A dataclass always has a synthesized `__eq__` in its MRO; the
    // `.expect` keeps the structurally unreachable `None` out of this
    // crate's instrumented regions, as above.
    Some(eq_mangled.expect("dataclass must have __eq__"))
}

/// Builds a [`MirExpr::CompareChain`] from its already-lowered operands.
/// Each `==`/`!=` link between same-dataclass instances becomes a
/// [`MirCompareKind::DataclassEq`]; every other link is
/// [`MirCompareKind::Plain`].
pub(crate) fn lower_compare_chain(
    first: MirExpr,
    links: Vec<(CmpOpKind, MirExpr)>,
    classes: &HashMap<String, HirClassDef>,
) -> MirExpr {
    let mut left_ty = first.ty();
    let mut lowered = Vec::with_capacity(links.len());
    for (op, right) in links {
        let right_ty = right.ty();
        let kind = match op {
            CmpOpKind::Eq | CmpOpKind::NotEq => {
                match dataclass_eq_callee(&left_ty, &right_ty, classes) {
                    Some(callee) => MirCompareKind::DataclassEq {
                        callee,
                        negate: op == CmpOpKind::NotEq,
                    },
                    None => MirCompareKind::Plain(op),
                }
            }
            _ => MirCompareKind::Plain(op),
        };
        lowered.push(MirCompareLink { kind, right });
        left_ty = right_ty;
    }
    MirExpr::CompareChain {
        first: Box::new(first),
        links: lowered,
    }
}

#[cfg(test)]
mod tests;
