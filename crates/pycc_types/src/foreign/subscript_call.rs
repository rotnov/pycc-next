//! Part 2a of #1371: checking `HirExpr::ExprCall`, a positional call whose
//! callee is a subscript result (`callbacks[k](tok)`).
//!
//! HIR lowers a subscript callee to this node whenever the call is not a
//! generic class instantiation (`pycc_hir`'s `expr::subscript_call`). The
//! one admitted shape is a call of a CPython object: the callee infers to
//! `Ty::Object` (an object subscript load, Part 3 of #1026), every argument
//! is packable (`check_object_call_args`, shared with the method-call and
//! direct-call shapes), and the result is another opaque `Ty::Object`.
//! Every other callee keeps the `C0001` HIR reported for this shape before
//! Part 2a, reworded to name the newly admitted case; it is reported at the
//! type stage's own position until Part 5 of #1371 threads spans here.

use pycc_diag::{Diagnostic, Span};
use pycc_hir::HirExpr;

use crate::{Environment, expr::infer_expr_in};
use pycc_hir::Ty;

/// The `C0001` for a subscript callee that is not a CPython object.
pub(crate) fn subscript_callee_unsupported() -> Diagnostic {
    Diagnostic::error(
        "C0001",
        "calling a subscript expression is not supported yet (only a generic class \
         instantiation `C[type](args)` or a call of a CPython object is)"
            .to_string(),
        Span::new(0, 0),
    )
}

/// Infers `callee(args)` for a `HirExpr::ExprCall` (module doc). The callee
/// is inferred first and then each argument left to right, which is
/// CPython's evaluation order, so an unsupported operation inside the
/// callee reports its own diagnostic before an argument's.
pub(crate) fn infer_expr_call(
    env: &Environment,
    local_names: &[&str],
    callee: &HirExpr,
    args: &[HirExpr],
) -> Result<Ty, Diagnostic> {
    let callee_ty = infer_expr_in(env, local_names, callee)?;
    let arg_tys = args
        .iter()
        .map(|arg| infer_expr_in(env, local_names, arg))
        .collect::<Result<Vec<_>, _>>()?;
    if !matches!(callee_ty, Ty::Object) {
        return Err(subscript_callee_unsupported());
    }
    super::check_object_call_args(env, args, &arg_tys, "call")?;
    Ok(Ty::Object)
}
