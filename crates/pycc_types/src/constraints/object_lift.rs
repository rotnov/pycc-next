//! The constraint solver's two `object` lifts added by Part 1 of #1333: a
//! method call on a CPython object, and a call of a name bound to one.
//!
//! Both answer the concrete `object` term only when the receiver's or
//! callee's term is *already* `Ok(Ty::Object)` -- a foreign import, a
//! module-level object global, or a local bound directly from one of those.
//! A solver variable that might later resolve to `object` (an unannotated
//! helper's parameter, or a local bound from another helper's result) is
//! deliberately not consulted through `resolved_term`: whether that variable
//! is resolved yet depends on the order the helpers appear in the source, so
//! admission would too. Those shapes are Part 3 of #1333 (#1364) and keep
//! their `T0021`.
//!
//! Split out of `constraints.rs` per AGENTS.md's file-decomposition rule, so
//! that file only gains the two call sites.

use super::*;

/// `o.method(...)` on a concrete `object` receiver is an `object` producer,
/// exactly like the `AttrGet` arm's `o.name` and the `Subscript` arm's
/// `o[k]`: outside an `--ext` module `object` is unspellable in an
/// annotation (`docs/TYPE_SYSTEM.md`'s `object` row -- since Part 1 of #1367
/// only a class a foreign import binds resolves to it; D-258 lets an ext
/// module write `Any`/`object`), so leaving the term out would make an unannotated
/// `def _c(): return json.loads(s)` report a `T0021` asking for an
/// annotation the source may have no name to write. Any other receiver keeps the arm's historical `Ok(None)`.
pub(super) fn method_call_on_object(base_term: Option<&TypeTerm>) -> Option<TypeTerm> {
    matches!(base_term, Some(Ok(Ty::Object))).then_some(Ok(Ty::Object))
}

/// The D-110 bound-callee gate's exception for a callee whose term is
/// concretely `object` (`g = json.loads; return g(s)`): the call is an
/// `object` producer under the same positional-argument rule the check
/// phase applies (`crate::foreign::check_object_call_args`), so every
/// argument is collected as an ordinary expression and the call answers the
/// concrete `object` term. Answers `None` for every other callee term, which
/// then falls through to the gate's `non_callable_binding` refusal.
pub(super) fn call_of_object_binding(
    signatures: &HashMap<String, SignatureTerms>,
    parents: &mut Vec<usize>,
    concrete: &mut Vec<Option<Ty>>,
    deferred: &mut DeferredConstraints,
    env: &ConstraintEnvironment<'_, '_>,
    callee_term: &TypeTerm,
    args: &[HirExpr],
) -> Result<Option<TypeTerm>, Diagnostic> {
    if !matches!(callee_term, Ok(Ty::Object)) {
        return Ok(None);
    }
    for arg in args {
        collect_expr_constraints(signatures, parents, concrete, deferred, env, arg)?;
    }
    Ok(Some(Ok(Ty::Object)))
}
