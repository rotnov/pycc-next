//! Constraint terms of attribute reads, method calls and receiver-dispatched calls.

use super::*;

// D-154 (Part 1 of #375): same reasoning as `Subscript`/`ListPop`
// above -- the base-instance-type gate and attribute/method
// resolution are `infer_expr_in`'s job, not this solver's (which
// only ever runs for a *private, unannotated* function's body).
// Recurse into `base` (and, for `MethodCall`, every argument) only
// to keep propagating genuine errors; produce no term for either
// expression's own overall type, mirroring `HirExpr::Subscript`'s
// own pre-D-146 "no unification term" default and `ListPop`'s own
// doc comment's documented consequence (a private function
// assigning from one of these expressions registers no real type
// term for the target -- a pre-existing, not novel, gap this
// project already accepts for every other container-shaped
// expression). Issue #771/D-199: the target is still tracked as
// definitely-but-opaquely bound (`opaque_bindings`), so a later
// read of it no longer misfires as an unbound local; only the
// missing type term itself remains unchanged.
pub(crate) fn collect_attr_get_constraints(
    signatures: &HashMap<String, SignatureTerms>,
    parents: &mut Vec<usize>,
    concrete: &mut Vec<Option<Ty>>,
    deferred: &mut DeferredConstraints,
    env: &ConstraintEnvironment<'_, '_>,
    expr: &HirExpr,
) -> Result<Option<TypeTerm>, Diagnostic> {
    let HirExpr::AttrGet { base, .. } = expr else {
        unreachable!()
    };
    let base_term = collect_expr_constraints(signatures, parents, concrete, deferred, env, base)?;
    // Part 2 of #1026 (#1081): `numpy.pi` is a term, not a hole.
    // Discarding it here would recreate one level up the exact dead
    // end the `Name` arm above was carved out to avoid: an
    // unannotated `def _helper(): return numpy.pi` would leave the
    // return variable unresolved and signature materialization
    // would report `T0021: ... add an annotation` -- advice no
    // annotation can satisfy in a native build, because there
    // `object` is unspellable (`annotation_to_ty` rejects it with
    // `C0001` outside an `--ext` module, D-258; since Part 1 of
    // #1367 only a class a foreign import binds resolves to it, and
    // `numpy.pi` is no class). Offering the
    // term lets the return materialize as `Ty::Object`. Since Part 1
    // of #1333 the check phase admits a function body binding or
    // returning the value, so the term keeps the program admitted.
    //
    // Only `Ty::Object` is lifted. Every other base keeps the
    // container-shaped "no unification term" default described
    // above, which the rest of this arm's contract rests on.
    if matches!(base_term, Some(Ok(Ty::Object))) {
        return Ok(Some(Ok(Ty::Object)));
    }
    Ok(None)
}

pub(crate) fn collect_method_call_constraints(
    signatures: &HashMap<String, SignatureTerms>,
    parents: &mut Vec<usize>,
    concrete: &mut Vec<Option<Ty>>,
    deferred: &mut DeferredConstraints,
    env: &ConstraintEnvironment<'_, '_>,
    expr: &HirExpr,
) -> Result<Option<TypeTerm>, Diagnostic> {
    let HirExpr::MethodCall { base, method, args } = expr else {
        unreachable!()
    };
    let base_term = collect_expr_constraints(signatures, parents, concrete, deferred, env, base)?;
    // A generator's `send()`/`throw()`/`close()`: `throw`'s operand is an
    // exception, which only `pycc_types`' checker types.
    if let Some(Ok(Ty::Generator(sig))) = &base_term
        && matches!(method.as_str(), "send" | "throw" | "close")
    {
        if method == "send" {
            for arg in args {
                collect_expr_constraints(signatures, parents, concrete, deferred, env, arg)?;
            }
        }
        return Ok(Some(Ok(if method == "close" {
            Ty::None
        } else {
            sig.0.clone()
        })));
    }
    let mut arg_terms = Vec::with_capacity(args.len());
    for arg in args {
        arg_terms.push(collect_expr_constraints(
            signatures, parents, concrete, deferred, env, arg,
        )?);
    }
    if let Some(term) =
        builtin_call::builtin_method_term(&base_term, method, &arg_terms, parents, concrete)
    {
        return Ok(Some(term));
    }
    // Part 1 of #1333: `o.method(...)` on a concrete `object`
    // receiver answers `object`, on the `AttrGet` and `Subscript`
    // arms' own reasoning above; the three arms change together.
    // #1420: on a concrete user-class instance it answers the
    // resolved method's return term.
    Ok(
        object_lift::method_call_on_object(base_term.as_ref()).or_else(|| {
            method_return::method_call_on_instance(signatures, env, base_term.as_ref(), method)
        }),
    )
}

// Issue #1188: this solver never picks a reading and never rejects.
// An admitted container reading is collected exactly as the
// container node it would have been (so a `ListPop` still yields its
// element type); a refused one is collected as the method call.
// Issue #1095: so is an admitted one whose bare-name receiver is
// bound to a concrete `object` term, so `o.pop()` answers `object`
// on the `MethodCall` arm's own reasoning.
pub(crate) fn collect_receiver_dispatched_call_constraints(
    signatures: &HashMap<String, SignatureTerms>,
    parents: &mut Vec<usize>,
    concrete: &mut Vec<Option<Ty>>,
    deferred: &mut DeferredConstraints,
    env: &ConstraintEnvironment<'_, '_>,
    expr: &HirExpr,
) -> Result<Option<TypeTerm>, Diagnostic> {
    let HirExpr::ReceiverDispatchedCall { call, container } = expr else {
        unreachable!()
    };
    let reading = match container {
        pycc_hir::ContainerFallback::Admitted
            if matches!(
                call.method_receiver(),
                Some((HirExpr::Name(name), _))
                    if matches!(env.bindings.get(name), Some(Ok(Ty::Object)))
            ) =>
        {
            None
        }
        pycc_hir::ContainerFallback::Admitted => call.container_form(),
        pycc_hir::ContainerFallback::Refused(_) => None,
    };
    collect_expr_constraints(
        signatures,
        parents,
        concrete,
        deferred,
        env,
        reading.as_ref().unwrap_or(call),
    )
}
