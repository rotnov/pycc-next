//! The one mapping from a Python binary operator to the [`BinOpKind`] pycc
//! lowers. `Expr::BinOp` and augmented assignment (`x op= e`, #1209) both
//! call it, so the operators augmented assignment admits are exactly the
//! operators a plain binary expression admits, and widening one widens both.

use crate::BinOpKind;
use pycc_ast::Operator;

/// The [`BinOpKind`] for `op`, or `None` for the one operator pycc does not
/// lower yet: `@`. The bitwise and shift operators map since #1210.
pub(crate) fn bin_op_kind(op: Operator) -> Option<BinOpKind> {
    match op {
        Operator::Add => Some(BinOpKind::Add),
        Operator::Sub => Some(BinOpKind::Sub),
        Operator::Mult => Some(BinOpKind::Mul),
        Operator::Div => Some(BinOpKind::Div),
        Operator::FloorDiv => Some(BinOpKind::FloorDiv),
        Operator::Mod => Some(BinOpKind::Mod),
        Operator::Pow => Some(BinOpKind::Pow),
        Operator::LShift => Some(BinOpKind::LShift),
        Operator::RShift => Some(BinOpKind::RShift),
        Operator::BitAnd => Some(BinOpKind::BitAnd),
        Operator::BitOr => Some(BinOpKind::BitOr),
        Operator::BitXor => Some(BinOpKind::BitXor),
        Operator::MatMult => None,
    }
}

impl BinOpKind {
    /// The operator's Python spelling, for diagnostics that name it.
    pub fn as_str(self) -> &'static str {
        match self {
            BinOpKind::Add => "+",
            BinOpKind::Sub => "-",
            BinOpKind::Mul => "*",
            BinOpKind::Div => "/",
            BinOpKind::FloorDiv => "//",
            BinOpKind::Mod => "%",
            BinOpKind::Pow => "**",
            BinOpKind::LShift => "<<",
            BinOpKind::RShift => ">>",
            BinOpKind::BitAnd => "&",
            BinOpKind::BitOr => "|",
            BinOpKind::BitXor => "^",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::bin_op_kind;
    use crate::BinOpKind;
    use pycc_ast::Operator;

    #[test]
    fn every_operator_maps_as_the_binary_expression_lowering_expects() {
        for (op, expected) in [
            (Operator::Add, Some(BinOpKind::Add)),
            (Operator::Sub, Some(BinOpKind::Sub)),
            (Operator::Mult, Some(BinOpKind::Mul)),
            (Operator::Div, Some(BinOpKind::Div)),
            (Operator::FloorDiv, Some(BinOpKind::FloorDiv)),
            (Operator::Mod, Some(BinOpKind::Mod)),
            (Operator::Pow, Some(BinOpKind::Pow)),
            (Operator::MatMult, None),
            (Operator::LShift, Some(BinOpKind::LShift)),
            (Operator::RShift, Some(BinOpKind::RShift)),
            (Operator::BitOr, Some(BinOpKind::BitOr)),
            (Operator::BitXor, Some(BinOpKind::BitXor)),
            (Operator::BitAnd, Some(BinOpKind::BitAnd)),
        ] {
            assert_eq!(bin_op_kind(op), expected, "{op:?}");
        }
    }

    /// Every kind spells as the `pycc_ast` operator it is lowered from, so a
    /// diagnostic naming a `BinOpKind` reads exactly like the source.
    #[test]
    fn every_kind_spells_its_python_operator() {
        for op in [
            Operator::Add,
            Operator::Sub,
            Operator::Mult,
            Operator::Div,
            Operator::FloorDiv,
            Operator::Mod,
            Operator::Pow,
            Operator::LShift,
            Operator::RShift,
            Operator::BitAnd,
            Operator::BitOr,
            Operator::BitXor,
        ] {
            let kind = bin_op_kind(op).expect("every operator but `@` maps");
            assert_eq!(kind.as_str(), op.as_str(), "{op:?}");
        }
    }
}
