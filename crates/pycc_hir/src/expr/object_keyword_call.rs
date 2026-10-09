//! Lowering a call with keyword arguments that the keyword binder cannot
//! bind (Part 8 of #1371).
//!
//! Lowering has no type information, so it cannot tell `o.method(x, key=v)`
//! on a CPython object from the same call on a pycc instance. Before Part 8
//! every keyword call outside `keyword_bind::is_bindable_call` was refused
//! here with `C0001` "keyword call arguments are not supported yet". This
//! module keeps that refusal for every shape whose callee lowering can
//! never be an object, and defers the rest to `pycc_types` as a
//! [`HirExpr::KeywordCall`]:
//!
//! * the callee is lowered *as if the call were positional*, by the same
//!   `lower_expr` arm a keyword-free call takes, so the callee and the
//!   positional arguments lower exactly as they always have;
//! * the result is wrapped only when it is one of the three shapes that can
//!   reach a CPython object -- a `Call` of a bare name (a foreign function
//!   or class, or an `object`-typed local), the generic `MethodCall`
//!   fallback on a non-`super()` receiver, or an `ExprCall` of a subscript
//!   result;
//! * every other shape keeps the located `C0001`: `**` unpacking, a call of
//!   a module `def` the binder holds but cannot bind (an observable
//!   argument reordering, #1204), any call whose positional half fails to
//!   lower (`super(k=1)`, `xs.append(value=2)`), a stdlib intrinsic
//!   (`math.sqrt(x, k=1)`), a container or receiver-dispatched method
//!   call, `super().m(k=1)`, a generic class instantiation and
//!   `type(self)(k=1)`.
//!
//! `pycc_types` then admits a [`HirExpr::KeywordCall`] whose callee is a
//! CPython object and reports the same `C0001`, at the same span, for every
//! other callee -- a module `def` absent from the binder's table (a
//! redefined name, an excluded parameter kind), a pycc
//! class, a builtin such as `print(x, end="")` and a pycc instance method.

use pycc_ast::{Expr, ExprCall};
use pycc_diag::{Diagnostic, Span};

use super::keyword_bind::SignatureTable;
use super::lower_expr;
use crate::{HirExpr, ImportBinding, unsupported};

/// The message every refused keyword call gets, at HIR or at the type stage
/// (`pycc_types`' `foreign::keyword_call`).
pub const KEYWORD_CALL_UNSUPPORTED: &str = "keyword call arguments are not supported yet";

/// Lowers `call`, which passes at least one keyword argument the binder
/// cannot bind (module doc).
pub(super) fn lower(
    call: &ExprCall,
    in_function: bool,
    class_name: Option<&str>,
    imports: &[ImportBinding],
    signatures: &SignatureTable,
) -> Result<HirExpr, Diagnostic> {
    let refuse = || unsupported(KEYWORD_CALL_UNSUPPORTED, call.range);
    if call.arguments.keywords.iter().any(|k| k.arg.is_none()) {
        return Err(refuse());
    }
    // A module `def` the binder holds but cannot bind here (an observable
    // reordering, #1204) stays refused before its positional half could be
    // routed through the binder's default filling and report a missing
    // parameter instead.
    if let Expr::Name(name) = call.func.as_ref()
        && signatures.binds(name.id.as_str())
    {
        return Err(refuse());
    }
    // Any refusal of the positional half answers the keyword `C0001`, the
    // diagnostic every keyword call got before Part 8: the half without its
    // keywords is not the program the user wrote, so its own message
    // (`list.append() takes exactly one argument, got 0` for
    // `xs.append(value=2)`, the bare-`super()` refusal for `super(x=1)`)
    // would describe a call that does not exist.
    let mut positional = call.clone();
    positional.arguments.keywords = Default::default();
    let inner = lower_expr(
        &Expr::Call(positional),
        in_function,
        class_name,
        imports,
        signatures,
    )
    .map_err(|_| refuse())?;
    if !is_object_call_shape(&inner) {
        return Err(refuse());
    }
    let keywords = call
        .arguments
        .keywords
        .iter()
        .map(|keyword| {
            let name = keyword
                .arg
                .as_ref()
                .expect("`**` unpacking was refused above")
                .to_string();
            lower_expr(&keyword.value, in_function, class_name, imports, signatures)
                .map(|value| (name, value))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let range = std::ops::Range::<u32>::from(call.range);
    Ok(HirExpr::KeywordCall {
        call: Box::new(inner),
        keywords,
        span: Span::new(range.start, range.end),
    })
}

/// Whether the positional lowering `inner` is one of the three call shapes
/// whose callee can be a CPython object (module doc).
fn is_object_call_shape(inner: &HirExpr) -> bool {
    match inner {
        // A stdlib intrinsic lowers to its dotted canonical spelling
        // (`math.sqrt`); a bare name never contains a dot.
        HirExpr::Call { callee, .. } => !callee.contains('.'),
        HirExpr::MethodCall { base, .. } => !matches!(base.as_ref(), HirExpr::Super),
        HirExpr::ExprCall { .. } => true,
        _ => false,
    }
}

#[cfg(test)]
#[path = "object_keyword_call_tests.rs"]
mod tests;
