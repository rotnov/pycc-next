//! `and`/`or` (#1211, Part 3 of #1018): the operator kind, the value-context
//! result-type join shared by `pycc_types` and `pycc_mir`, and the
//! truth-context marker HIR lowering applies to condition positions.
//!
//! A boolean operator is lowered in one of two contexts (see
//! [`HirExpr::BoolOp`]'s `truth_only` field):
//!
//! * **Truth context** -- an `if`/`elif`/`while` test, a comprehension `if`
//!   filter, or the operand of `not`. Only the truth of the result is
//!   observed, so the node is typed `bool` and its operands need only be
//!   truth-testable; `if n > 0 and name:` over `int` and `str` is fine.
//! * **Value context** -- everywhere else. The node yields the selected
//!   operand itself, so both operands must join to one type
//!   ([`bool_op_result_ty`]). pycc has no union types, so a pair with no
//!   common type is refused rather than widened.
//!
//! [`mark_truth_context`] descends only through a `BoolOp` and a `not`. It
//! must not descend through a walrus (`if (x := a or b):` binds `x` to the
//! selected *value*) or through a comparison, call or any other node (`if (a
//! or b) == c:` compares the selected value). A `match` guard is not a truth
//! position either: the checker requires a guard to be `bool`, and treating
//! `case _ if n and s` as truth context would admit it while `case _ if n`
//! over an `int` stays refused.

use crate::{HirExpr, Ty, UnaryOpKind};

/// `and` or `or`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoolOpKind {
    And,
    Or,
}

impl BoolOpKind {
    /// The operator's Python source spelling, for diagnostics.
    pub fn as_str(self) -> &'static str {
        match self {
            BoolOpKind::And => "and",
            BoolOpKind::Or => "or",
        }
    }
}

/// The value-context result type of `left <op> right`, or `None` when the
/// two operand types have no common type pycc can represent.
///
/// The one owner of this rule: `pycc_types` checks with it and `pycc_mir`
/// recomputes each node's type with it, so the two can never disagree.
///
/// For `or`, the left operand is selected only when it is truthy, so a left
/// `Optional[T]` is known present on that arm and joins as its payload `T`
/// (`x or default` over `x: int | None` is an `int`). `and` selects the left
/// operand only when it is falsy, which includes `None`, so no such
/// stripping applies.
///
/// | left (after stripping), right | result |
/// |---|---|
/// | equal `bool`/`int`/`float`/`str`, equal `Optional[int\|float\|bool]`, the same class | that type |
/// | `bool` with `int`, either order | `int` |
/// | `T` with `Optional[T]`, either order | `Optional[T]` |
/// | `object` with `object`, or `object` with `bool`/`int`/`float`/`str` in either order (Part 6 of #1371) | `object` |
/// | anything else | `None` (refused) |
///
/// `int` with `float` is deliberately refused rather than widened: `1 or 2.0`
/// is the `int` `1` in CPython, and a `float` result would print `1.0`.
///
/// The `object` rows are decided on the operands *as written*, before the
/// `or` stripping of a left `Optional[T]`: the selected operand of `o or n`
/// is always one of the two operands themselves, and only an `object` and a
/// scalar `pycc_codegen` can box (`pycc_ext_obj_pack_*`) have one
/// representation in common -- the CPython object. An `Optional`, `None`, a
/// container or an instance paired with an `object` has no such boxing, so
/// this function returns `None`. `pycc_types` refuses a `None`, a container
/// or an instance whose class defines `__bool__`/`__len__` earlier, with its
/// operand `T0021`; only an `Optional[T]` or a dunder-free instance reaches
/// the join and is reported as `I0404`.
pub fn bool_op_result_ty(op: BoolOpKind, left: &Ty, right: &Ty) -> Option<Ty> {
    if matches!(left, Ty::Object) || matches!(right, Ty::Object) {
        return (is_object_joinable(left) && is_object_joinable(right)).then_some(Ty::Object);
    }
    let left = match (op, left) {
        (BoolOpKind::Or, Ty::Optional(inner)) => inner.as_ref(),
        _ => left,
    };
    match (left, right) {
        (Ty::Bool | Ty::Int | Ty::Float | Ty::Str | Ty::Optional(_), _) if left == right => {
            Some(left.clone())
        }
        (Ty::Instance(l), Ty::Instance(r)) if l == r => Some(left.clone()),
        (Ty::Bool, Ty::Int) | (Ty::Int, Ty::Bool) => Some(Ty::Int),
        (bare, Ty::Optional(inner)) | (Ty::Optional(inner), bare) if inner.as_ref() == bare => {
            Some(Ty::Optional(inner.clone()))
        }
        _ => None,
    }
}

/// Whether a value of type `ty` may be one operand of an `and`/`or` whose
/// result is the CPython object: the object itself, or a scalar
/// `pycc_codegen` boxes on the arm that selects it.
fn is_object_joinable(ty: &Ty) -> bool {
    matches!(ty, Ty::Object | Ty::Bool | Ty::Int | Ty::Float | Ty::Str)
}

/// Marks `expr` as consumed only for its truth, when it is a boolean
/// operator, and recurses into that operator's operands and into the operand
/// of a `not`. Every other node is left alone: its own `BoolOp` descendants
/// stay value context (see this module's doc comment).
pub(crate) fn mark_truth_context(expr: &mut HirExpr) {
    match expr {
        HirExpr::BoolOp {
            left,
            right,
            truth_only,
            ..
        } => {
            *truth_only = true;
            mark_truth_context(left);
            mark_truth_context(right);
        }
        HirExpr::UnaryOp {
            op: UnaryOpKind::Not,
            operand,
        } => mark_truth_context(operand),
        _ => {}
    }
}

/// Right-folds an n-ary boolean operator's already-lowered operands:
/// `[a, b, c]` becomes `op(a, op(b, c))`.
///
/// A right fold tests each operand's truth exactly once, as CPython does. A
/// left fold `op(op(a, b), c)` would test the selected inner operand's truth
/// a second time, which is observable once a truth test has side effects.
/// The parser never produces fewer than two operands.
pub(crate) fn fold_bool_op(op: BoolOpKind, operands: Vec<HirExpr>) -> HirExpr {
    let mut operands = operands.into_iter().rev();
    let last = operands
        .next()
        .expect("the parser gives a boolean operator at least two operands");
    operands.fold(last, |right, left| HirExpr::BoolOp {
        op,
        left: Box::new(left),
        right: Box::new(right),
        truth_only: false,
    })
}

#[cfg(test)]
mod tests;
