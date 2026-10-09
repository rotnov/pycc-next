//! Comparison typing shared by a single comparison and a chained
//! comparison `a < b < c` (#1212, Part 4 of #1018).
//!
//! A chain admits exactly what a single comparison admits, link by link:
//! each adjacent operand pair is typed by [`compare_link_ty`] exactly as
//! `a op b` would be, and the chain's result is `bool`.

use crate::Environment;
use crate::expr::infer_expr_in;
use crate::numeric_or_bool_compatible;
use pycc_diag::{Diagnostic, Span};
use pycc_hir::{CmpOpKind as CmpOp, CompareLink, HirExpr, Ty};

/// Types one comparison link `left op right` whose operand types are
/// already inferred. The `is`/`is not` arm needs both operand expressions,
/// not only their types, because it keys on whether either side is the
/// `None` literal.
pub(crate) fn compare_link_ty(
    env: &Environment,
    op: CmpOp,
    left: &HirExpr,
    right: &HirExpr,
    left_ty: &Ty,
    right_ty: &Ty,
) -> Result<Ty, Diagnostic> {
    // `is`/`is not` (D-197, #763, Part 1 of #747): with a `None` literal
    // operand, the *other* operand's static type must be `Ty::Optional(_)`,
    // `Ty::None` itself, or (Part 1 of #1371) a CPython object, whose
    // `None` test is pointer identity against `Py_None`. Without a `None`
    // literal operand, HIR lowering (`pycc_hir::compare_chain::lower_cmp_op`)
    // admits a single comparison or (Part 11 of #1371) a chain link
    // between two non-literal operands, and
    // `foreign::compare::general_identity_ty` admits only two objects.
    if matches!(op, CmpOp::Is | CmpOp::IsNot) {
        // #1476: identity is the object's own, so a narrowed `object` name
        // is typed as the object it still is, never its narrowed native
        // copy (`pycc_mir` reads it raw the same way).
        let raw_ty = |operand: &HirExpr, ty: &Ty| {
            if crate::narrow::is_bare_narrowed_object_read(env, operand) {
                Ty::Object
            } else {
                ty.clone()
            }
        };
        let (left_ty, right_ty) = (&raw_ty(left, left_ty), &raw_ty(right, right_ty));
        let other_ty = if matches!(left, HirExpr::NoneLiteral) {
            right_ty
        } else if matches!(right, HirExpr::NoneLiteral) {
            left_ty
        } else {
            return crate::foreign::compare::general_identity_ty(op, left_ty, right_ty);
        };
        return match other_ty {
            Ty::Optional(_) | Ty::None | Ty::Object => Ok(Ty::Bool),
            other => Err(Diagnostic::error(
                "T0021",
                format!(
                    "cannot compare `{}` and `None` with `is`/`is not` -- only an `Optional[T]` (or `None`) operand is supported",
                    other.name()
                ),
                Span::new(0, 0),
            )),
        };
    }
    // Part 2b of #1371: membership answers for every pair, so a native
    // pair never falls through to the numeric rule below.
    if matches!(op, CmpOp::In | CmpOp::NotIn) {
        return crate::foreign::compare::membership_ty(op, left_ty, right_ty);
    }
    // Part 1 of #1371: a rich comparison with a CPython object operand is
    // CPython's own `PyObject_RichCompare`, whose result is an object.
    if let Some(result) =
        crate::foreign::compare::rich_compare_ty(env, (left, left_ty), (right, right_ty))
    {
        return result;
    }
    // #378 (PR-18): `==`/`!=` between same-class dataclass instances
    // is accepted -- the compiler-synthesized `__eq__` method has a
    // known-correct signature `(self, other: SameClass) -> bool`.
    // This is restricted to dataclass classes (not any class with a
    // user-defined `__eq__`) because the MIR rewrite assumes the
    // synthesized signature; a user-defined `__eq__` with wrong
    // arity or return type would reach codegen and panic. Ordering
    // operators (`<`, `<=`, `>`, `>=`) between instances are always
    // rejected with T0021 -- pycc has no `__lt__`/`__le__`/`__gt__`/
    // `__ge__` dispatch. Different-class comparisons also stay T0021.
    if matches!(op, CmpOp::Eq | CmpOp::NotEq)
        && let (Ty::Instance(left_class), Ty::Instance(right_class)) = (left_ty, right_ty)
        && left_class == right_class
        && let Some(class_def) = env.lookup_class(left_class)
        && class_def.is_dataclass
    {
        return Ok(Ty::Bool);
    }
    if numeric_or_bool_compatible(left_ty.clone(), right_ty.clone()) {
        Ok(Ty::Bool)
    } else {
        Err(Diagnostic::error(
            "T0021",
            format!(
                "cannot compare `{}` and `{}`",
                left_ty.name(),
                right_ty.name()
            ),
            Span::new(0, 0),
        ))
    }
}

/// Types a [`HirExpr::CompareChain`]: every operand is inferred once, left
/// to right, then each adjacent pair is typed by [`compare_link_ty`]. The
/// first failing link's diagnostic wins, and the result is `bool`.
pub(crate) fn infer_compare_chain(
    env: &Environment,
    local_names: &[&str],
    first: &HirExpr,
    links: &[CompareLink],
) -> Result<Ty, Diagnostic> {
    let mut operands = Vec::with_capacity(links.len() + 1);
    operands.push((first, infer_expr_in(env, local_names, first)?));
    for link in links {
        operands.push((&link.right, infer_expr_in(env, local_names, &link.right)?));
    }
    // #1476: an `is`/`is not` link compares the object itself, which a
    // chain cannot carry, so a bare narrowed read on either side of such a
    // link is the object here and refused like any object operand; a
    // narrowed read in an ordered link (`0 < o < 10`) stays native.
    let operand_tys: Vec<Ty> = operands
        .iter()
        .enumerate()
        .map(|(index, (operand, ty))| {
            let in_identity_link = [index.checked_sub(1), Some(index)]
                .into_iter()
                .flatten()
                .filter_map(|link| links.get(link))
                .any(|link| matches!(link.op, CmpOp::Is | CmpOp::IsNot));
            if in_identity_link && crate::narrow::is_bare_narrowed_object_read(env, operand) {
                Ty::Object
            } else {
                ty.clone()
            }
        })
        .collect();
    crate::foreign::compare::reject_object_in_chain(&operand_tys)?;
    for (index, link) in links.iter().enumerate() {
        let (left, left_ty) = &operands[index];
        let (right, right_ty) = &operands[index + 1];
        compare_link_ty(env, link.op, left, right, left_ty, right_ty)?;
    }
    Ok(Ty::Bool)
}
