//! Chained comparisons `a < b < c` (#1212, Part 4 of #1018): the per-link
//! operator mapping shared with the single comparison, and the lowering of
//! an AST comparison with two or more operators into
//! [`HirExpr::CompareChain`].
//!
//! A chain admits the per-link operators `==`, `!=`, `<`, `<=`, `>`, `>=`,
//! and `is`/`is not` when one of *that link's* two operands is
//! syntactically the `None` literal (D-197). `in`/`not in` in a chain keeps
//! its `C0001`. Since Part 11 of #1371 a chain link also admits general
//! identity between two non-literal operands, exactly as a single
//! comparison does, so `pycc_types` can refuse a chain over a CPython object
//! with the `I0404` every other object chain gets. A *single* comparison also
//! admits general identity between two non-literal operands (Part 1 of
//! #1371), which `pycc_types` admits only between CPython objects, and
//! `in`/`not in` whose container is not a literal, display or comprehension
//! (Part 2b of #1371), which `pycc_types` admits only for a CPython object
//! container.

use pycc_ast::{CmpOp, Expr, ExprCompare};
use pycc_diag::Diagnostic;

use crate::expr::keyword_bind::SignatureTable;
use crate::expr::{contains_named_expr, lower_expr};
use crate::{CmpOpKind, HirExpr, ImportBinding, unsupported};

/// One `op right` link of a [`HirExpr::CompareChain`]. The link's left
/// operand is the previous link's `right`, or the chain's `first` for link
/// 0.
#[derive(Debug, Clone, PartialEq)]
pub struct CompareLink {
    pub op: CmpOpKind,
    pub right: HirExpr,
}

/// Every operand of a [`HirExpr::CompareChain`] in evaluation order:
/// `first`, then each link's `right`.
pub fn compare_chain_operands<'a>(
    first: &'a HirExpr,
    links: &'a [CompareLink],
) -> impl Iterator<Item = &'a HirExpr> {
    std::iter::once(first).chain(links.iter().map(|link| &link.right))
}

/// Maps one AST comparison operator to its HIR kind, applying the D-197
/// syntactic gate to `is`/`is not`: one of `left` and `right` -- the two
/// operands of this link -- must be the `None` literal. `range` is the
/// whole comparison's range, which every rejection reports.
///
/// `general_identity` (Part 1 of #1371) widens the gate for a single
/// comparison: `is`/`is not` between two operands neither of which is a
/// literal also lowers, because either may be a CPython object, whose
/// identity `pycc_types` admits and whose other pairs it refuses with this
/// same `C0001` message. A literal operand is never an object, so a literal
/// on either side keeps the located rejection here.
pub(crate) fn lower_cmp_op(
    op: CmpOp,
    left: &Expr,
    right: &Expr,
    range: std::ops::Range<u32>,
    general_identity: bool,
) -> Result<CmpOpKind, Diagnostic> {
    // `is`/`is not` (D-197, #763, Part 1 of #747): this compiler's first
    // support of any kind for either operator, deliberately scoped at this
    // syntactic gate to exactly the case #763 needs -- one operand is
    // literally `Expr::NoneLiteral`. Every other `is`/`is not` use falls
    // through to the `other =>` rejection below. The *type* of the
    // non-`None` operand (must be `Ty::Optional(_)` or `Ty::None`) is
    // `pycc_types`' job, not this lowering step's (D-105).
    let is_none_operand_shape = matches!(left, Expr::NoneLiteral(_))
        || matches!(right, Expr::NoneLiteral(_))
        || (general_identity && !is_literal(left) && !is_literal(right));
    Ok(match op {
        CmpOp::Eq => CmpOpKind::Eq,
        CmpOp::NotEq => CmpOpKind::NotEq,
        CmpOp::Lt => CmpOpKind::Lt,
        CmpOp::LtE => CmpOpKind::LtE,
        CmpOp::Gt => CmpOpKind::Gt,
        CmpOp::GtE => CmpOpKind::GtE,
        CmpOp::Is if is_none_operand_shape => CmpOpKind::Is,
        CmpOp::IsNot if is_none_operand_shape => CmpOpKind::IsNot,
        CmpOp::In if general_identity && can_be_object(right) => CmpOpKind::In,
        CmpOp::NotIn if general_identity && can_be_object(right) => CmpOpKind::NotIn,
        other => {
            return Err(unsupported(
                format!("comparison operator not supported yet: {other:?}"),
                range,
            ));
        }
    })
}

/// Whether `expr` is a literal constant, which can never be a CPython
/// object (see [`lower_cmp_op`]'s `general_identity`).
fn is_literal(expr: &Expr) -> bool {
    matches!(
        expr,
        Expr::StringLiteral(_)
            | Expr::BytesLiteral(_)
            | Expr::NumberLiteral(_)
            | Expr::BooleanLiteral(_)
            | Expr::EllipsisLiteral(_)
    )
}

/// Whether `expr` may evaluate to a CPython object, so a membership test
/// against it is lowered and left to `pycc_types` (Part 2b of #1371). A
/// literal, and a list, tuple, set or dict display or comprehension, always
/// builds a native value, so `x in [1, 2]` keeps its located `C0001` here.
fn can_be_object(expr: &Expr) -> bool {
    !is_literal(expr)
        && !matches!(
            expr,
            Expr::List(_)
                | Expr::Tuple(_)
                | Expr::Set(_)
                | Expr::Dict(_)
                | Expr::ListComp(_)
                | Expr::SetComp(_)
                | Expr::DictComp(_)
                | Expr::FString(_)
        )
}

/// Lowers an AST comparison with `ops.len() >= 2` into
/// [`HirExpr::CompareChain`], lowering every operand exactly once, left to
/// right.
///
/// Operands 0 and 1 always run; an operand at index 2 or later runs only
/// when every earlier link was true. A walrus there would bind
/// conditionally, which the binding walkers (`pycc_types`' and `pycc_mir`'s)
/// cannot represent, so it is refused with `C0001` -- the same rule
/// `and`/`or` applies to its later operands.
pub(crate) fn lower_compare_chain(
    cmp: &ExprCompare,
    in_function: bool,
    class_name: Option<&str>,
    imports: &[ImportBinding],
    signatures: &SignatureTable,
) -> Result<HirExpr, Diagnostic> {
    debug_assert!(cmp.ops.len() >= 2);
    let range = std::ops::Range::<u32>::from(cmp.range);
    let mut ops = Vec::with_capacity(cmp.ops.len());
    for (index, op) in cmp.ops.iter().enumerate() {
        let left = if index == 0 {
            cmp.left.as_ref()
        } else {
            &cmp.comparators[index - 1]
        };
        // Part 11 of #1371: widen only `is`/`is not`. `general_identity`
        // also widens `in`/`not in` (`can_be_object`), which a chain keeps
        // refusing here.
        ops.push(lower_cmp_op(
            *op,
            left,
            &cmp.comparators[index],
            range.clone(),
            matches!(op, CmpOp::Is | CmpOp::IsNot),
        )?);
    }
    let first = lower_expr(&cmp.left, in_function, class_name, imports, signatures)?;
    let mut links = Vec::with_capacity(ops.len());
    for (index, (op, right)) in ops.into_iter().zip(cmp.comparators.iter()).enumerate() {
        let lowered = lower_expr(right, in_function, class_name, imports, signatures)?;
        if index >= 1 && contains_named_expr(&lowered) {
            return Err(unsupported(
                "a walrus assignment (`:=`) in a short-circuited chained-comparison operand is not supported",
                pycc_ast::expr_range(right),
            ));
        }
        links.push(CompareLink { op, right: lowered });
    }
    Ok(HirExpr::CompareChain {
        first: Box::new(first),
        links,
    })
}

#[cfg(test)]
mod tests;
