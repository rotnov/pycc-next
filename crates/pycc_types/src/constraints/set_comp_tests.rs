//! Unit tests for `constraints/set_comp.rs` (#1343, Part 1 of #1336).

use super::{ContainerDefault, DeferredConstraints, apply_container_defaults, set_comp_container};
use crate::constraints::unify_terms;
use pycc_hir::Ty;

fn instance() -> Ty {
    Ty::Instance(Box::new("R".to_string()))
}

fn set_of(element: Ty) -> Ty {
    Ty::Set(Box::new(element))
}

fn frozenset_of(element: Ty) -> Ty {
    Ty::FrozenSet(Box::new(element))
}

/// A fresh solver state of `count` unresolved variables.
fn state(count: usize) -> (Vec<usize>, Vec<Option<Ty>>, DeferredConstraints) {
    (
        (0..count).collect(),
        vec![None; count],
        DeferredConstraints::default(),
    )
}

#[test]
fn a_set_comprehension_of_a_known_int_or_bool_element_stays_set_int() {
    let (mut parents, mut concrete, mut deferred) = state(1);
    concrete[0] = Some(Ty::Bool);
    for elt in [Some(Ok(Ty::Int)), Some(Err(0))] {
        assert_eq!(
            set_comp_container(elt, &mut parents, &mut concrete, &mut deferred),
            Ok(set_of(Ty::Int))
        );
    }
    assert!(deferred.container_defaults.is_empty());
}

#[test]
fn an_unresolved_set_comprehension_defaults_to_set_int() {
    let (mut parents, mut concrete, mut deferred) = state(0);
    let term = set_comp_container(None, &mut parents, &mut concrete, &mut deferred);
    assert_eq!(term, Err(0));
    assert_eq!(
        deferred.container_defaults,
        vec![ContainerDefault::SetComp { var: 0, elt: None }]
    );
    apply_container_defaults(&deferred.container_defaults, &mut parents, &mut concrete);
    assert_eq!(concrete[0], Some(set_of(Ty::Int)));
}

#[test]
fn a_set_comprehension_met_by_a_declared_instance_set_keeps_it() {
    let (mut parents, mut concrete, mut deferred) = state(0);
    let term = set_comp_container(None, &mut parents, &mut concrete, &mut deferred);
    unify_terms(
        term,
        Ok(set_of(instance())),
        &mut parents,
        &mut concrete,
        "T0022",
        "return",
    )
    .unwrap();
    apply_container_defaults(&deferred.container_defaults, &mut parents, &mut concrete);
    assert_eq!(concrete[0], Some(set_of(instance())));
}

#[test]
fn a_frozenset_of_a_resolved_instance_set_source_takes_its_element() {
    let (mut parents, mut concrete, _) = state(2);
    concrete[0] = Some(set_of(instance()));
    let defaults = [ContainerDefault::FrozenSetOf {
        source: Some(0),
        result: 1,
    }];
    apply_container_defaults(&defaults, &mut parents, &mut concrete);
    assert_eq!(concrete[1], Some(frozenset_of(instance())));
}

#[test]
fn a_frozenset_of_an_opaque_or_non_set_source_defaults_to_frozenset_int() {
    let (mut parents, mut concrete, _) = state(3);
    concrete[1] = Some(Ty::List(Box::new(Ty::Int)));
    let defaults = [
        ContainerDefault::FrozenSetOf {
            source: None,
            result: 0,
        },
        ContainerDefault::FrozenSetOf {
            source: Some(1),
            result: 2,
        },
    ];
    apply_container_defaults(&defaults, &mut parents, &mut concrete);
    assert_eq!(concrete[0], Some(frozenset_of(Ty::Int)));
    assert_eq!(concrete[2], Some(frozenset_of(Ty::Int)));
}

#[test]
fn a_frozenset_result_fixed_by_a_declared_return_is_left_alone() {
    let (mut parents, mut concrete, _) = state(2);
    concrete[0] = Some(set_of(Ty::Int));
    concrete[1] = Some(frozenset_of(instance()));
    let defaults = [ContainerDefault::FrozenSetOf {
        source: Some(0),
        result: 1,
    }];
    apply_container_defaults(&defaults, &mut parents, &mut concrete);
    assert_eq!(concrete[1], Some(frozenset_of(instance())));
}

#[test]
fn chained_frozenset_entries_resolve_through_the_fixpoint_in_any_order() {
    // Entry A's source (variable 1) is entry B's result, and B's source
    // (variable 0) is a resolved `set[R]`. A is listed first, so a single
    // pass would default it to `frozenset[int]` before B resolves.
    let (mut parents, mut concrete, _) = state(3);
    concrete[0] = Some(set_of(instance()));
    let defaults = [
        ContainerDefault::FrozenSetOf {
            source: Some(1),
            result: 2,
        },
        ContainerDefault::FrozenSetOf {
            source: Some(0),
            result: 1,
        },
    ];
    apply_container_defaults(&defaults, &mut parents, &mut concrete);
    assert_eq!(concrete[1], Some(frozenset_of(instance())));
    assert_eq!(concrete[2], Some(frozenset_of(instance())));
}

#[test]
fn a_default_is_written_to_the_root_every_unified_variable_sees() {
    let (mut parents, mut concrete, _) = state(2);
    unify_terms(Err(0), Err(1), &mut parents, &mut concrete, "T0021", "test").unwrap();
    let defaults = [ContainerDefault::SetComp { var: 0, elt: None }];
    apply_container_defaults(&defaults, &mut parents, &mut concrete);
    assert_eq!(
        crate::constraints::resolved_term(Err(1), &mut parents, &concrete),
        Some(set_of(Ty::Int))
    );
}

#[test]
fn a_set_comprehension_of_a_known_instance_element_is_set_of_it() {
    let (mut parents, mut concrete, mut deferred) = state(1);
    concrete[0] = Some(instance());
    for elt in [Some(Ok(instance())), Some(Err(0))] {
        assert_eq!(
            set_comp_container(elt, &mut parents, &mut concrete, &mut deferred),
            Ok(set_of(instance()))
        );
    }
    assert!(deferred.container_defaults.is_empty());
}

#[test]
fn a_deferred_set_comprehension_takes_an_element_resolved_later() {
    let (mut parents, mut concrete, mut deferred) = state(1);
    let term = set_comp_container(Some(Err(0)), &mut parents, &mut concrete, &mut deferred);
    assert_eq!(term, Err(1));
    assert_eq!(
        deferred.container_defaults,
        vec![ContainerDefault::SetComp {
            var: 1,
            elt: Some(0)
        }]
    );
    concrete[0] = Some(instance());
    apply_container_defaults(&deferred.container_defaults, &mut parents, &mut concrete);
    assert_eq!(concrete[1], Some(set_of(instance())));
}

#[test]
fn a_deferred_set_comprehension_of_an_unresolved_element_is_set_int() {
    let (mut parents, mut concrete, mut deferred) = state(1);
    let term = set_comp_container(Some(Err(0)), &mut parents, &mut concrete, &mut deferred);
    assert_eq!(term, Err(1));
    apply_container_defaults(&deferred.container_defaults, &mut parents, &mut concrete);
    assert_eq!(concrete[1], Some(set_of(Ty::Int)));
    // A non-instance resolution (a `str` element, which the check phase
    // refuses with `T0038`) also falls back to `set[int]`.
    let (mut parents, mut concrete, mut deferred) = state(1);
    let _ = set_comp_container(Some(Err(0)), &mut parents, &mut concrete, &mut deferred);
    concrete[0] = Some(Ty::Str);
    apply_container_defaults(&deferred.container_defaults, &mut parents, &mut concrete);
    assert_eq!(concrete[1], Some(set_of(Ty::Int)));
}
