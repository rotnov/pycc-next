//! Lowering `type(self)(args)`, a construction of the receiver's own class
//! (#1411).
//!
//! THE RULE: a call whose callee is itself a call of the bare name `type`
//! lowers to [`HirExpr::ReceiverClassCall`] exactly when the inner call is
//! `type(self)` -- one positional argument, no keywords, the argument the
//! bare name `self` -- and the call sits in a function body nested in a
//! class (`in_function` with a `class_name`). Every other `type(...)(...)`
//! is refused here, at the callee's span, with a message naming the
//! construct: `type(x)(...)` for any other `x`, and `type(self)(...)` at
//! module level, in a class body or in a module-level function.
//!
//! The receiver must be spelled `self`. A method whose receiver is spelled
//! otherwise (`def m(this)`) lowers it as the canonical `self` and aliases
//! the source name (`class::receiver`), and that module's guard already
//! refuses any mention of `self` in such a body; `type(this)(...)` is
//! refused here as "not the receiver". Lowering has no method-kind
//! information, so `pycc_types` decides a `@staticmethod` or `@classmethod`
//! body by `self`'s type instead (refused unless `self` is an instance of a
//! class), and refuses a module whose own binding shadows the builtin `type`.
//!
//! The class is deliberately not resolved here: an inherited body is
//! compiled once more for each subclass with `self` retyped (D-254), so the
//! class is recovered from `self`'s static type in the body being compiled.

use pycc_ast::{Expr, ExprCall};
use pycc_diag::Diagnostic;

use super::keyword_bind::SignatureTable;
use super::lower_expr;
use crate::{HirExpr, ImportBinding, unsupported};

/// The canonical receiver name `type(...)` must be applied to.
const RECEIVER: &str = "self";

/// `call`'s callee when it is a call of the bare name `type`, the shape this
/// module owns; `None` for every other callee.
pub(super) fn type_call_callee(call: &ExprCall) -> Option<&ExprCall> {
    match call.func.as_ref() {
        Expr::Call(inner) if matches!(inner.func.as_ref(), Expr::Name(name) if name.id.as_str() == "type") => {
            Some(inner)
        }
        _ => None,
    }
}

/// Lowers `call`, whose callee `inner` is a call of `type` (module doc).
pub(super) fn lower(
    call: &ExprCall,
    inner: &ExprCall,
    in_function: bool,
    class_name: Option<&str>,
    imports: &[ImportBinding],
    signatures: &SignatureTable,
) -> Result<HirExpr, Diagnostic> {
    let is_receiver = inner.arguments.keywords.is_empty()
        && matches!(inner.arguments.args.as_ref(),
            [Expr::Name(name)] if name.id.as_str() == RECEIVER);
    if !is_receiver {
        return Err(unsupported(
            "calling `type(...)` is supported only as `type(self)(...)`, which constructs the \
             receiver's own class inside a method",
            inner.range,
        ));
    }
    if !in_function || class_name.is_none() {
        return Err(unsupported(
            "`type(self)(...)` is supported only inside a method of a class, where `self` is \
             the receiver",
            inner.range,
        ));
    }
    let args = call
        .arguments
        .args
        .iter()
        .map(|e| lower_expr(e, in_function, class_name, imports, signatures))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(HirExpr::ReceiverClassCall { args })
}

#[cfg(test)]
#[path = "receiver_class_call_tests.rs"]
mod tests;
