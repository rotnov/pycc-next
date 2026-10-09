//! Part 8 of #1371: checking `HirExpr::KeywordCall`, a call with keyword
//! arguments that `pycc_hir`'s keyword binder could not bind.
//!
//! HIR wraps the positional half of such a call -- a `Call` of a bare name,
//! a `MethodCall` or an `ExprCall` -- whenever the callee *could* be a
//! CPython object (`pycc_hir`'s `expr::object_keyword_call`). The one
//! admitted reading is a call of one: the callee is an `object`-typed name
//! (a foreign function or class, an `object`-typed local or parameter), the
//! receiver of a method call is an `object`, or a subscript callee is one.
//! The positional half is then checked exactly as the keyword-free call is
//! (`check_object_call_args` over its arguments), every keyword value is
//! checked under the same argument rule (so since #1435 a regular class's
//! instance may be a keyword value, as lark's `state=self` is), and the
//! result is another opaque `Ty::Object`. Every other callee -- a pycc function, class or method, a
//! builtin -- keeps the `C0001` HIR reported for every keyword call before
//! Part 8, at the call's own span, which the node carries.

use pycc_diag::{Diagnostic, Span};
use pycc_hir::{HirExpr, Ty};

use crate::{Environment, expr::infer_expr_in};

/// The `C0001` a keyword call of anything but a CPython object gets.
pub(crate) fn keyword_call_unsupported(span: Span) -> Diagnostic {
    Diagnostic::error("C0001", pycc_hir::KEYWORD_CALL_UNSUPPORTED, span)
}

/// Whether the positional half `call` calls a CPython object (module doc).
///
/// Decided before anything inside `call` is inferred, so a non-object
/// callee reports the keyword refusal rather than whatever its positional
/// half would (a pycc method's arity mismatch, say). A receiver or
/// subscript callee whose own inference fails is not an object; the
/// keyword refusal is the diagnostic then too.
pub(super) fn calls_an_object(env: &Environment, local_names: &[&str], call: &HirExpr) -> bool {
    match call {
        HirExpr::Call { callee, .. } => {
            matches!(env.lookup(callee), Some(Ty::Object)) && !env.def_rebound.contains(callee)
        }
        HirExpr::MethodCall { base: callee, .. } | HirExpr::ExprCall { callee, .. } => {
            matches!(infer_expr_in(env, local_names, callee), Ok(Ty::Object))
        }
        _ => false,
    }
}

/// Infers a `HirExpr::KeywordCall` (module doc). The positional half is
/// inferred first and each keyword value after it, left to right, which is
/// CPython's evaluation order.
pub(crate) fn infer_keyword_call(
    env: &Environment,
    local_names: &[&str],
    call: &HirExpr,
    keywords: &[(String, HirExpr)],
    span: Span,
) -> Result<Ty, Diagnostic> {
    if !calls_an_object(env, local_names, call) {
        return Err(keyword_call_unsupported(span));
    }
    let ty = infer_expr_in(env, local_names, call)?;
    let keyword_tys = keywords
        .iter()
        .map(|(_, value)| infer_expr_in(env, local_names, value))
        .collect::<Result<Vec<_>, _>>()?;
    let what = if matches!(call, HirExpr::MethodCall { .. }) {
        "method"
    } else {
        "call"
    };
    super::check_object_call_args(
        env,
        keywords.iter().map(|(_, value)| value),
        &keyword_tys,
        what,
    )?;
    Ok(ty)
}
