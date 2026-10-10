//! Constraint terms of container literals, subscripts, slices, methods and comprehensions.

use super::*;

pub(crate) fn collect_list_literal_constraints(
    signatures: &HashMap<String, SignatureTerms>,
    parents: &mut Vec<usize>,
    concrete: &mut Vec<Option<Ty>>,
    deferred: &mut DeferredConstraints,
    env: &ConstraintEnvironment<'_, '_>,
    expr: &HirExpr,
) -> Result<Option<TypeTerm>, Diagnostic> {
    let HirExpr::ListLiteral(elements) = expr else {
        unreachable!()
    };
    let mut element_terms = Vec::with_capacity(elements.len());
    for element in elements {
        element_terms.push(collect_expr_constraints(
            signatures, parents, concrete, deferred, env, element,
        )?);
    }
    if let Some(element_ty) = homogeneous_private_solver_scalar_list_element(&element_terms) {
        Ok(Some(Ok(Ty::List(Box::new(element_ty)))))
    } else {
        Ok(None)
    }
}

pub(crate) fn collect_subscript_constraints(
    signatures: &HashMap<String, SignatureTerms>,
    parents: &mut Vec<usize>,
    concrete: &mut Vec<Option<Ty>>,
    deferred: &mut DeferredConstraints,
    env: &ConstraintEnvironment<'_, '_>,
    expr: &HirExpr,
) -> Result<Option<TypeTerm>, Diagnostic> {
    let HirExpr::Subscript { base, index } = expr else {
        unreachable!()
    };
    // Part 2 of #1027, the solver half of `crate::expr`'s own
    // `Subscript` interception: `b[i]` on a `memoryview`-bound name
    // is a `Ty::Float` element load. It must run before the base
    // recursion below, because that recursion reaches the `Name`
    // seam above, which calls `reject_memoryview_read` and would
    // report the `C0001` capability gap for the expression that
    // closes it.
    //
    // The binding is read raw out of `env.bindings` and resolved
    // with `resolved_term`, the same shape that seam uses. The
    // index is still collected -- an undefined name or an
    // unsatisfiable constraint inside it is a real error and must
    // surface -- but its *term* is discarded exactly as the list
    // path discards it: the index type gate is the check phase's.
    //
    // Still only a bare `Name` base after Part 2a of #1142 (#1165),
    // for the reason `crate::expr`'s own arm records: the producer
    // is admitted only as an assignment's whole right-hand side, so
    // `ndarray(4)[0]` is refused by this walker's `Call` arm before
    // the base recursion below can reach it.
    if let HirExpr::Name(buffer_name) = base.as_ref()
        && let Some(term) = env.bindings.get(buffer_name).cloned()
        && let Some(Ty::MemoryView) = resolved_term(term, parents, concrete)
    {
        collect_expr_constraints(signatures, parents, concrete, deferred, env, index)?;
        return Ok(Some(Ok(Ty::Float)));
    }
    let base_term = collect_expr_constraints(signatures, parents, concrete, deferred, env, base)?;
    collect_expr_constraints(signatures, parents, concrete, deferred, env, index)?;
    // D-146 (#239): when the base resolves to a `Ty::List` element-
    // type carrier (produced by the `ListLiteral` arm above or a
    // `Ty::List`-bound name), extract the scalar element type -- the
    // carrier is destructured, never unified. Otherwise keep the
    // historical `Ok(None)` behavior (the base/index recursion above
    // already propagated genuine errors).
    // Part 3 of #1026 (PR 3b of #1082): `o[k]` is a term, not a
    // hole, on exactly the `AttrGet` arm's own reasoning below --
    // outside an `--ext` module `object` is unspellable in an
    // annotation (`docs/TYPE_SYSTEM.md`'s `object` row; D-258 lets
    // an ext module write `Any`/`object`), so discarding the term
    // would leave an unannotated
    // `def _h(): return gc.garbage[0]` reporting a `T0021` asking
    // for an annotation no source can write, in place of the
    // `I0404` the check phase once reported for the read itself.
    // Since Part 1 of #1333 the helper returning it is admitted.
    // Changing this arm without the `AttrGet` one, or the reverse,
    // is the drift both comments exist to prevent.
    if matches!(base_term, Some(Ok(Ty::Object))) {
        return Ok(Some(Ok(Ty::Object)));
    }
    if let Some(Ok(Ty::List(element_ty))) = base_term {
        Ok(Some(Ok(*element_ty)))
    } else {
        Ok(None)
    }
}

// PR-12 Task 7 (D-118): structurally identical to `Subscript` above
// -- a slice's base/bounds are, like a subscript's base/index,
// ordinary sub-expressions this solver needs to keep walking into
// (e.g. `some_param[1:3]` inside a private helper, where
// `some_param`'s own type is exactly what the solver is trying to
// pin down), but a `Ty::List`/`Ty::Int` base-type or bound-type gate
// is `infer_expr_in`'s job, not this constraint collector's. Recurse
// into `base` and every present bound only to keep propagating
// genuine errors (e.g. an unbound local used as a bound); produce no
// term for the `Slice` expression's own overall type.
pub(crate) fn collect_slice_constraints(
    signatures: &HashMap<String, SignatureTerms>,
    parents: &mut Vec<usize>,
    concrete: &mut Vec<Option<Ty>>,
    deferred: &mut DeferredConstraints,
    env: &ConstraintEnvironment<'_, '_>,
    expr: &HirExpr,
) -> Result<Option<TypeTerm>, Diagnostic> {
    let HirExpr::Slice {
        base,
        start,
        stop,
        step,
    } = expr
    else {
        unreachable!()
    };
    let base_term = collect_expr_constraints(signatures, parents, concrete, deferred, env, base)?;
    for bound in [start, stop, step].into_iter().flatten() {
        collect_expr_constraints(signatures, parents, concrete, deferred, env, bound)?;
    }
    // Part 2b of #1371: `o[a:b]` is a term on the `Subscript` arm's
    // own reasoning above -- an unannotated helper returning it
    // must not ask for an annotation.
    if matches!(base_term, Some(Ok(Ty::Object))) {
        return Ok(Some(Ok(Ty::Object)));
    }
    Ok(None)
}

// #1254 (D-250): the loop variable is bound in a clone, so it never
// becomes a binding of the enclosing scope. `bind_comp_loop_var`
// collects the range operands itself, against the enclosing
// bindings (the clone is still identical to `env` at that point).
// The element gate is `infer_expr_in`'s, as for a display; the
// container type is returned so `return [..]` from an unannotated
// helper still infers its return type.
pub(crate) fn collect_comprehension_constraints(
    signatures: &HashMap<String, SignatureTerms>,
    parents: &mut Vec<usize>,
    concrete: &mut Vec<Option<Ty>>,
    deferred: &mut DeferredConstraints,
    env: &ConstraintEnvironment<'_, '_>,
    expr: &HirExpr,
) -> Result<Option<TypeTerm>, Diagnostic> {
    let HirExpr::Comprehension(comp) = expr else {
        unreachable!()
    };
    let mut scoped = env.clone();
    bind_comp_loop_var(
        signatures,
        parents,
        concrete,
        deferred,
        &mut scoped,
        &comp.var,
        &comp.iter,
    )?;
    let mut elt_term = None;
    for sub in comp.body_exprs() {
        elt_term = collect_expr_constraints(signatures, parents, concrete, deferred, &scoped, sub)?;
    }
    // Part 1 of #1255: a comprehension over a CPython object
    // produces a CPython object, whatever its element.
    if let CompIter::Iterable(_) = comp.iter
        && matches!(scoped.bindings.get(&comp.var), Some(Ok(Ty::Object)))
    {
        return Ok(Some(Ok(Ty::Object)));
    }
    // A set comprehension's container term is chosen by
    // [`set_comp::set_comp_container`] (#1343, #1344), which owns the
    // rule. The element is the last body expression (`body_exprs`
    // puts the condition first).
    if let CompElt::Set(_) = comp.elt {
        return Ok(Some(set_comp::set_comp_container(
            elt_term, parents, concrete, deferred,
        )));
    }
    // A list comprehension whose element is already concrete is a
    // list of that element.
    if let CompElt::List(_) = comp.elt
        && let Some(elem) = elt_term.and_then(|t| resolved_term(t, parents, concrete))
        && pycc_hir::is_list_element_ty(&elem)
    {
        return Ok(Some(Ok(Ty::List(Box::new(elem)))));
    }
    Ok(Some(Ok(crate::comprehension::comp_container_of(&comp.elt))))
}
