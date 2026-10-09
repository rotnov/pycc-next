//! #1343 (Part 1 of #1336): constraints the solver defers until every call
//! and operator fact has settled, and the container defaults among them.
//!
//! Before #1343 the solver typed a set comprehension's container as a hard
//! `set[int]` and a `frozenset(x)` call as a hard `frozenset[int]`, whatever
//! the element. Once a set may hold a user-class instance that guess meets a
//! declared `set[R]`/`frozenset[R]` as a false `T0022`, and
//! `module::merge_solver_first` reports the solver's diagnostic over the
//! check phase's. Each is now a fresh term plus a [`ContainerDefault`]:
//! whatever the program pins the term to wins, and a term nothing pins ends
//! as today's `int` container, so unannotated-helper inference of `set[int]`
//! and `frozenset[int]` is unchanged.
//!
//! #1344 (Part 2 of #1336) types a set comprehension's container from its
//! element: `set[C]` for an element the solver resolved to an instance of
//! `C`, including a class-constructor call `C(...)` since #1342. An element
//! it has no term for (#1360) still ends as `set[int]`, and the
//! check phase relabels the mismatch that leaves
//! (`crate::comprehension::inferred_set_return_limit`).

use super::{BinOpConstraint, TypeTerm, fresh_variable, resolved_term, root};
use pycc_hir::Ty;

/// The solver's deferred constraints: operator facts propagated to a
/// fixpoint, and the container defaults applied after them.
#[derive(Debug, Default)]
pub(crate) struct DeferredConstraints {
    pub(crate) binops: Vec<BinOpConstraint>,
    pub(crate) container_defaults: Vec<ContainerDefault>,
}

/// A container term whose element the solver could not fix while collecting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ContainerDefault {
    /// A set comprehension's container: `set[C]` once the element term
    /// `elt` resolves to a user-class instance (#1344), else `set[int]`.
    /// `elt` is `None` for an element the solver has no variable for.
    SetComp { var: usize, elt: Option<usize> },
    /// A `frozenset(source)` call's result: `frozenset[e]` once `source`
    /// resolves to `set[e]`/`frozenset[e]`, else `frozenset[int]`. `source`
    /// is `None` for an argument the solver has no term for.
    FrozenSetOf {
        source: Option<usize>,
        result: usize,
    },
}

/// The container term of a set comprehension whose element term is `elt`:
/// `set[int]` for an element already known to be `int` or `bool` (today's
/// result), `set[C]` for one already known to be an instance of `C` (#1344),
/// and otherwise a fresh term defaulted by [`apply_container_defaults`],
/// which remembers the element's variable so an instance it resolves to
/// later still types the container.
pub(crate) fn set_comp_container(
    elt: Option<TypeTerm>,
    parents: &mut Vec<usize>,
    concrete: &mut Vec<Option<Ty>>,
    deferred: &mut DeferredConstraints,
) -> TypeTerm {
    let elt_var = match elt {
        Some(Err(var)) => Some(var),
        _ => None,
    };
    match elt.and_then(|term| resolved_term(term, parents, concrete)) {
        Some(Ty::Int | Ty::Bool) => return Ok(Ty::Set(Box::new(Ty::Int))),
        Some(instance @ Ty::Instance(_)) => return Ok(Ty::Set(Box::new(instance))),
        _ => {}
    }
    let var = fresh_variable(parents, concrete);
    deferred
        .container_defaults
        .push(ContainerDefault::SetComp { var, elt: elt_var });
    Err(var)
}

/// A fresh term for a `frozenset(...)` result whose source is not resolved
/// yet, defaulted by [`apply_container_defaults`].
pub(crate) fn deferred_frozenset(
    source: Option<usize>,
    parents: &mut Vec<usize>,
    concrete: &mut Vec<Option<Ty>>,
    deferred: &mut DeferredConstraints,
) -> TypeTerm {
    let result = fresh_variable(parents, concrete);
    deferred
        .container_defaults
        .push(ContainerDefault::FrozenSetOf { source, result });
    Err(result)
}

/// Applies `defaults` to every root still unresolved, writing each default
/// to the root so every variable unified into it sees it. Set comprehensions
/// first (`set[C]` when the element's root resolved to an instance of `C`,
/// else `set[int]`) (a `set` root cannot share a root with a `frozenset` one without a
/// real conflict), then `frozenset(...)` results to a fixpoint -- one
/// entry's source can be another entry's result, across functions -- and
/// finally `frozenset[int]` for every result still unresolved. A root the
/// program already pinned (a declared return) is left alone for the check
/// phase to validate.
pub(super) fn apply_container_defaults(
    defaults: &[ContainerDefault],
    parents: &mut [usize],
    concrete: &mut [Option<Ty>],
) {
    for default in defaults {
        if let ContainerDefault::SetComp { var, elt } = default {
            let element = match elt.map(|elt| &concrete[root(parents, elt)]) {
                Some(Some(instance @ Ty::Instance(_))) => instance.clone(),
                _ => Ty::Int,
            };
            let root = root(parents, *var);
            concrete[root].get_or_insert_with(|| Ty::Set(Box::new(element)));
        }
    }
    loop {
        let mut changed = false;
        for default in defaults {
            if let ContainerDefault::FrozenSetOf {
                source: Some(source),
                result,
            } = default
            {
                let result = root(parents, *result);
                let source = root(parents, *source);
                if concrete[result].is_none()
                    && let Some(Ty::Set(element) | Ty::FrozenSet(element)) = &concrete[source]
                {
                    concrete[result] = Some(Ty::FrozenSet(element.clone()));
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
    for default in defaults {
        if let ContainerDefault::FrozenSetOf { result, .. } = default {
            let root = root(parents, *result);
            concrete[root].get_or_insert_with(|| Ty::FrozenSet(Box::new(Ty::Int)));
        }
    }
}

#[cfg(test)]
#[path = "set_comp_tests.rs"]
mod tests;
