//! The comprehension arms of the generic-call rewrite (#1254, D-250), shared
//! by the expression form and the three `name = <comp>` statement forms.
//! Extracted from `monomorphize.rs` under `AGENTS.md`'s decomposability rule
//! when #1344 (Part 2 of #1336) taught the set arms to type a `set[C]` of
//! user-class instances.

use std::collections::HashSet;

use pycc_diag::Diagnostic;
use pycc_hir::{CompElt, CompIter, HirComprehension, HirExpr, Ty};

use super::{GenericInstantiation, rewrite_generic_calls_in_expr};
use crate::Environment;

/// Rewrites a comprehension's iterable, binds its synthesized loop variable
/// `var` to the iterable's element type, then rewrites `body` (the filter
/// and the element expressions) with that binding in scope. Returns the type
/// of `body`'s last expression -- the element, since callers put the filter
/// first -- or `None` for an empty `body`.
pub(super) fn rewrite_comp_parts<'a>(
    env: &mut Environment,
    local_names: &[&str],
    var: &str,
    iter: &mut CompIter,
    body: impl IntoIterator<Item = &'a mut HirExpr>,
    instantiations: &mut Vec<GenericInstantiation>,
    seen: &mut HashSet<String>,
) -> Result<Option<Ty>, Diagnostic> {
    let var_ty = rewrite_comp_iter(env, local_names, iter, instantiations, seen)?;
    env.bind(var.to_string(), var_ty);
    let mut last = None;
    for sub in body {
        last = Some(rewrite_generic_calls_in_expr(
            env,
            local_names,
            sub,
            instantiations,
            seen,
        )?);
    }
    Ok(last)
}

/// `rewrite_generic_calls_in_expr`'s `HirExpr::Comprehension` arm: rewrites
/// the comprehension and returns the container type it produces.
pub(super) fn rewrite_comprehension_expr(
    env: &mut Environment,
    local_names: &[&str],
    comp: &mut HirComprehension,
    instantiations: &mut Vec<GenericInstantiation>,
    seen: &mut HashSet<String>,
) -> Result<Ty, Diagnostic> {
    let HirComprehension {
        var,
        iter,
        cond,
        elt,
    } = comp;
    let body: Vec<&mut HirExpr> = match elt {
        CompElt::List(e) | CompElt::Set(e) => cond.iter_mut().chain([e]).collect(),
        CompElt::Dict { key, value } => cond.iter_mut().chain([key, value]).collect(),
    };
    let elt_ty = rewrite_comp_parts(env, local_names, var, iter, body, instantiations, seen)?;
    Ok(match elt {
        // Part 1 of #1255: over a CPython object the container is one too.
        _ if matches!(iter, CompIter::Iterable(_)) => Ty::Object,
        CompElt::Set(_) => set_comp_container(elt_ty),
        other => crate::comprehension::comp_container_of(other),
    })
}

/// The container a set comprehension produces from its element's type
/// `elt_ty` (the one [`rewrite_comp_parts`] returned): `set[C]` for an
/// instance of the user class `C` (#1344), else `set[int]`, the only other
/// element the check phase admits (D-122).
///
/// Infallible by construction: it reads the type the element's own rewrite
/// already produced, so it adds no failure mode to this pass. That rewrite is
/// the same fallible `infer_expr_in` walk every other expression gets here,
/// under this pass's reduced environment (no foreign names, #1105); the check
/// phase has already accepted the element before `monomorphize` runs.
pub(super) fn set_comp_container(elt_ty: Option<Ty>) -> Ty {
    let element = match elt_ty {
        Some(ty @ Ty::Instance(_)) => ty,
        _ => Ty::Int,
    };
    Ty::Set(Box::new(element))
}

/// `CompIter`'s own rewrite counterpart -- mirrors `resolve_comp_iter`
/// exactly (same three iterable shapes, same resulting loop-variable `Ty`),
/// but also rewrites any generic call reachable from a `CompIter::Range`
/// bound, which `resolve_comp_iter` (a read-only helper the parent
/// `monomorphize.rs` reuses as-is) has no reason to do.
pub(super) fn rewrite_comp_iter(
    env: &mut Environment,
    local_names: &[&str],
    iter: &mut CompIter,
    instantiations: &mut Vec<GenericInstantiation>,
    seen: &mut HashSet<String>,
) -> Result<Ty, Diagnostic> {
    match iter {
        CompIter::Range { start, stop, step } => {
            for sub in [start, stop, step] {
                rewrite_generic_calls_in_expr(env, local_names, sub, instantiations, seen)?;
            }
            Ok(Ty::Int)
        }
        CompIter::Name(name) => match env.lookup_any(name) {
            Some(Ty::List(elem)) => Ok(*elem),
            Some(Ty::Dict(kv)) => Ok(kv.0),
            Some(Ty::Set(elem) | Ty::FrozenSet(elem)) => Ok(*elem),
            // Already validated as iterable before `monomorphize` ever
            // runs. Unlike the `ForList` fallback in `monomorphize.rs`, no foreign name
            // reaches this arm: a comprehension over a CPython object is
            // refused by the check phase (`I0404`), so the missing foreign
            // names of this pass's environment never matter here.
            _ => Ok(Ty::Infer),
        },
        // Part 1 of #1255: the check phase proved the iterable a CPython
        // object, so its loop variable is one. The walk is kept, as for
        // `HirStmt::ForObject`'s iterable, so a generic call inside it
        // cannot escape this pass.
        CompIter::Iterable(iterable) => {
            rewrite_generic_calls_in_expr(env, local_names, iterable, instantiations, seen)?;
            Ok(Ty::Object)
        }
    }
}
