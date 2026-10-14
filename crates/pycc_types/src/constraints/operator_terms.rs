//! Constraint terms of operators, comparisons, conditional expressions and f-strings.

use super::*;

pub(crate) fn collect_compare_constraints(
    signatures: &HashMap<String, SignatureTerms>,
    parents: &mut Vec<usize>,
    concrete: &mut Vec<Option<Ty>>,
    deferred: &mut DeferredConstraints,
    env: &ConstraintEnvironment<'_, '_>,
    expr: &HirExpr,
) -> Result<Option<TypeTerm>, Diagnostic> {
    let HirExpr::Compare { op, left, right } = expr else {
        unreachable!()
    };
    let left_ty = collect_expr_constraints(signatures, parents, concrete, deferred, env, left)?;
    let right_ty = collect_expr_constraints(signatures, parents, concrete, deferred, env, right)?;
    // Part 1 of #1371: a rich comparison with a CPython object
    // operand is an object, mirroring `compare_link_ty`.
    let is_object = |ty: &Option<Result<Ty, usize>>| matches!(ty, Some(Ok(Ty::Object)));
    // Part 2b of #1371: a membership test is `bool` whatever its
    // operands are, mirroring `foreign::compare::membership_ty`.
    if !matches!(
        op,
        pycc_hir::CmpOpKind::Is
            | pycc_hir::CmpOpKind::IsNot
            | pycc_hir::CmpOpKind::In
            | pycc_hir::CmpOpKind::NotIn
    ) && (is_object(&left_ty) || is_object(&right_ty))
    {
        return Ok(Some(Ok(Ty::Object)));
    }
    Ok(Some(Ok(Ty::Bool)))
}

// #1211 (Part 3 of #1018): `and`/`or`. A truth-context node is
// `bool` whatever its operands are, exactly like `Not` below. A
// value-context node over two concrete operand types takes the one
// join rule `pycc_types::boolop` and `pycc_mir` also use; a pair with
// no join yields no term, and `infer_expr_in` then reports the
// `T0021`. An operand that is still an inference variable is unified
// with the other operand -- a sound restriction that infers only the
// equal-type case, which `infer_expr_in` re-checks afterwards.
pub(crate) fn collect_bool_op_constraints(
    signatures: &HashMap<String, SignatureTerms>,
    parents: &mut Vec<usize>,
    concrete: &mut Vec<Option<Ty>>,
    deferred: &mut DeferredConstraints,
    env: &ConstraintEnvironment<'_, '_>,
    expr: &HirExpr,
) -> Result<Option<TypeTerm>, Diagnostic> {
    let HirExpr::BoolOp {
        op,
        left,
        right,
        truth_only,
    } = expr
    else {
        unreachable!()
    };
    let left = collect_expr_constraints(signatures, parents, concrete, deferred, env, left)?;
    let right = collect_expr_constraints(signatures, parents, concrete, deferred, env, right)?;
    if *truth_only {
        return Ok(Some(Ok(Ty::Bool)));
    }
    match (left, right) {
        (Some(Ok(left)), Some(Ok(right))) => Ok(bool_op_result_ty(*op, &left, &right).map(Ok)),
        (Some(left), Some(right)) => {
            unify_terms(
                left.clone(),
                right,
                parents,
                concrete,
                "T0021",
                &format!("`{}` operands", op.as_str()),
            )?;
            Ok(Some(left))
        }
        _ => Ok(None),
    }
}

// #1395: `body if test else orelse`. `test` is collected for its
// own constraints only; its type does not reach the result. Two
// concrete branch types take the one join rule `pycc_types::if_exp`
// and `pycc_mir` also use, and a pair with no join yields no term,
// so `infer_expr_in` reports the `T0021`. A branch that is still an
// inference variable is unified with the other branch, inferring
// only the equal-type case -- except against `None`, which would
// bind the variable to `None` where the join means `T | None`, so
// that pair yields no term.
pub(crate) fn collect_if_exp_constraints(
    signatures: &HashMap<String, SignatureTerms>,
    parents: &mut Vec<usize>,
    concrete: &mut Vec<Option<Ty>>,
    deferred: &mut DeferredConstraints,
    env: &ConstraintEnvironment<'_, '_>,
    expr: &HirExpr,
) -> Result<Option<TypeTerm>, Diagnostic> {
    let HirExpr::IfExp { test, body, orelse } = expr else {
        unreachable!()
    };
    collect_expr_constraints(signatures, parents, concrete, deferred, env, test)?;
    let body = collect_expr_constraints(signatures, parents, concrete, deferred, env, body)?;
    let orelse = collect_expr_constraints(signatures, parents, concrete, deferred, env, orelse)?;
    match (body, orelse) {
        (Some(Ok(body)), Some(Ok(orelse))) => {
            Ok(pycc_hir::if_exp_result_ty(&body, &orelse).map(Ok))
        }
        (Some(Ok(Ty::None)), _) | (_, Some(Ok(Ty::None))) => Ok(None),
        (Some(body), Some(orelse)) => {
            unify_terms(
                body.clone(),
                orelse,
                parents,
                concrete,
                "T0021",
                "conditional expression branches",
            )?;
            Ok(Some(body))
        }
        _ => Ok(None),
    }
}

pub(crate) fn collect_unary_op_constraints(
    signatures: &HashMap<String, SignatureTerms>,
    parents: &mut Vec<usize>,
    concrete: &mut Vec<Option<Ty>>,
    deferred: &mut DeferredConstraints,
    env: &ConstraintEnvironment<'_, '_>,
    expr: &HirExpr,
) -> Result<Option<TypeTerm>, Diagnostic> {
    let HirExpr::UnaryOp { op, operand } = expr else {
        unreachable!()
    };
    let operand = collect_expr_constraints(signatures, parents, concrete, deferred, env, operand)?;
    match operand {
        Some(Ok(operand_ty)) => Ok(Some(Ok(unary_result_type(*op, operand_ty)?))),
        Some(operand) => {
            let result = fresh_term(parents, concrete);
            // Only `USub`/`UAdd`/`Invert` ever reach this arm --
            // `Not` is peeled off by its own dedicated arm above,
            // before `collect_expr_constraints` recurses into this
            // one -- and `USub`/`Invert` share the same `Sub`
            // shape, so a plain `UAdd` check (rather than an
            // exhaustive match that would need a structurally
            // unreachable `Not` arm the coverage gate could never
            // exercise) is enough to pick the right one.
            let bin_op = if matches!(op, UnaryOpKind::UAdd) {
                BinOpKind::Add
            } else {
                BinOpKind::Sub
            };
            deferred.binops.push((
                bin_op,
                Ok(Ty::Int),
                operand,
                result.clone(),
                crate::locate::expr_location(expr),
            ));
            Ok(Some(result))
        }
        None => Ok(None),
    }
}

pub(crate) fn collect_bin_op_constraints(
    signatures: &HashMap<String, SignatureTerms>,
    parents: &mut Vec<usize>,
    concrete: &mut Vec<Option<Ty>>,
    deferred: &mut DeferredConstraints,
    env: &ConstraintEnvironment<'_, '_>,
    expr: &HirExpr,
) -> Result<Option<TypeTerm>, Diagnostic> {
    let (HirExpr::BinOp { op, left, right } | HirExpr::AugBinOp { op, left, right }) = expr else {
        unreachable!()
    };
    let negative_exponent =
        *op == BinOpKind::Pow && matches!(right.as_ref(), HirExpr::IntLiteral(e) if *e < 0);
    let left = collect_expr_constraints(signatures, parents, concrete, deferred, env, left)?;
    let right = collect_expr_constraints(signatures, parents, concrete, deferred, env, right)?;
    // `int ** <negative int literal>` is a `float` (`expr.rs`).
    if negative_exponent
        && let Some(Ok(Ty::Int | Ty::Bool)) = left
            .clone()
            .map(|term| resolved_term(term, parents, concrete).ok_or(()))
    {
        return Ok(Some(Ok(Ty::Float)));
    }
    match (left, right) {
        (Some(left), Some(right)) => {
            let result = fresh_term(parents, concrete);
            // Two operands already concrete settle the result now, so a
            // builtin call or method over it (`a.count(m - 1)`) sees a
            // concrete argument; the deferred step below re-derives the
            // same type and still owns every refusal.
            if let (Some(left_ty), Some(right_ty)) = (
                resolved_term(left.clone(), parents, concrete),
                resolved_term(right.clone(), parents, concrete),
            ) && let Ok(ty) = numeric_result_type(*op, left_ty, right_ty)
            {
                unify_terms(
                    result.clone(),
                    Ok(ty),
                    parents,
                    concrete,
                    "T0021",
                    "binary expression",
                )?;
            }
            deferred.binops.push((
                *op,
                left,
                right,
                result.clone(),
                crate::locate::expr_location(expr),
            ));
            Ok(Some(result))
        }
        _ => Ok(None),
    }
}
