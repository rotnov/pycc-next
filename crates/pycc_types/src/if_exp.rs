//! Conditional-expression typing (#1395): `body if test else orelse`.
//!
//! * `test` is evaluated once, for its truth only. It must be a type whose
//!   truth codegen can test with CPython's rule: what `not` admits
//!   ([`is_truth_testable`]), a CPython object (its own `__bool__`/`__len__`
//!   runs, as for `if obj:`), or a `set`/`frozenset` (non-empty is true). A
//!   class instance whose class, or any class in its MRO, defines
//!   `__bool__` or `__len__` is refused, as for an `and`/`or` operand:
//!   codegen treats every instance as truthy.
//! * The node yields the selected branch, so the two branch types must join
//!   under [`if_exp_result_ty`], which `pycc_mir` recomputes with the same
//!   function. A pair with no common type is refused with `T0021` naming
//!   both types: pycc has no union types.

use crate::Environment;
use crate::boolop::class_truth_dunder;
use crate::expr::infer_expr_in;
use crate::unop::is_truth_testable;
use pycc_diag::{Diagnostic, Span};
use pycc_hir::{HirExpr, Ty, if_exp_result_ty};

/// Types one conditional expression (see the module doc comment).
pub(crate) fn infer_if_exp(
    env: &Environment,
    local_names: &[&str],
    test: &HirExpr,
    body: &HirExpr,
    orelse: &HirExpr,
) -> Result<Ty, Diagnostic> {
    let test_ty = infer_expr_in(env, local_names, test)?;
    admit_condition(env, &test_ty)?;
    let body_ty = infer_expr_in(env, local_names, body)?;
    let orelse_ty = infer_expr_in(env, local_names, orelse)?;
    if_exp_result_ty(&body_ty, &orelse_ty).ok_or_else(|| no_common_type(&body_ty, &orelse_ty))
}

/// `Ok(())` when a value of type `ty` may be a conditional expression's
/// condition.
fn admit_condition(env: &Environment, ty: &Ty) -> Result<(), Diagnostic> {
    if !(is_truth_testable(ty) || matches!(ty, Ty::Object | Ty::Set(_) | Ty::FrozenSet(_))) {
        return Err(t0021(format!(
            "conditional expression condition of type `{}` has no truth value pycc can test",
            ty.name()
        )));
    }
    if let Ty::Instance(class_name) = ty
        && let Some((owner, dunder)) = class_truth_dunder(env, class_name)
    {
        return Err(t0021(format!(
            "conditional expression condition of type `{class_name}` is not supported: class \
             `{owner}` defines `{dunder}`, and pycc does not call it for a truth test yet"
        )));
    }
    Ok(())
}

/// The `T0021` for two branch types [`if_exp_result_ty`] cannot join.
pub(crate) fn no_common_type(body: &Ty, orelse: &Ty) -> Diagnostic {
    t0021(format!(
        "conditional expression branches have no common type: {} and {} (pycc has no union types)",
        body.name(),
        orelse.name()
    ))
}

fn t0021(message: String) -> Diagnostic {
    Diagnostic::error("T0021", message, Span::new(0, 0))
}
