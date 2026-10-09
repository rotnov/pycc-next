//! Unit tests for the empty-container pass's own helpers.

use super::*;

/// `contains_infer` is what keeps the private-helper solver's placeholder
/// out of a rewritten node (see [`resolve`]). Its recursive arms are
/// exercised directly here rather than through source: the placeholder
/// reaches this pass as a bare `Ty::Infer` from an unannotated parameter,
/// so the nested shapes have no compact spelling in a `.py` fixture, and
/// the guarantee they carry -- a placeholder anywhere inside a resolved
/// type is still a placeholder -- is worth pinning independently of
/// whether today's inference happens to produce one.
#[test]
fn contains_infer_finds_the_placeholder_at_every_depth() {
    assert!(contains_infer(&Ty::Infer));
    assert!(contains_infer(&Ty::List(Box::new(Ty::Infer))));
    assert!(contains_infer(&Ty::Set(Box::new(Ty::Infer))));
    assert!(contains_infer(&Ty::Optional(Box::new(Ty::Infer))));
    assert!(contains_infer(&Ty::List(Box::new(Ty::List(Box::new(
        Ty::Infer
    ))))));
    assert!(contains_infer(&Ty::Dict(Box::new((Ty::Str, Ty::Infer)))));
    assert!(contains_infer(&Ty::Dict(Box::new((Ty::Infer, Ty::Int)))));
    assert!(contains_infer(&Ty::Tuple(Box::new(vec![
        Ty::Int,
        Ty::Infer
    ]))));
}

/// The complement: every fully concrete shape must pass, including the
/// `Ty::Param` one this check deliberately admits -- both spellings of a
/// generic element already report the same `T0034`, so discarding it
/// would introduce the asymmetry this check exists to remove.
#[test]
fn contains_infer_admits_every_fully_concrete_shape() {
    for ty in [
        Ty::Int,
        Ty::Float,
        Ty::Bool,
        Ty::Str,
        Ty::None,
        Ty::Param(Box::new("T".to_string())),
        Ty::Instance(Box::new("C".to_string())),
        Ty::Protocol(Box::new("P".to_string())),
        Ty::List(Box::new(Ty::Int)),
        Ty::Set(Box::new(Ty::Int)),
        Ty::Optional(Box::new(Ty::Int)),
        Ty::Dict(Box::new((Ty::Str, Ty::Int))),
        Ty::Tuple(Box::new(vec![Ty::Int, Ty::Str])),
    ] {
        assert!(!contains_infer(&ty), "{ty:?} is fully concrete");
    }
}

/// `concrete` applies that check to both halves of a dict resolution, not
/// just the value: a placeholder key is as unusable as a placeholder
/// value.
#[test]
fn concrete_discards_either_half_of_a_dict_resolution() {
    assert!(concrete(Resolution::List(Ty::Int)).is_some());
    assert!(concrete(Resolution::List(Ty::Infer)).is_none());
    assert!(concrete(Resolution::Dict(Ty::Str, Ty::Int)).is_some());
    assert!(concrete(Resolution::Dict(Ty::Str, Ty::Infer)).is_none());
    assert!(concrete(Resolution::Dict(Ty::Infer, Ty::Int)).is_none());
}

/// `contains_optional` is what keeps a flow-narrowable type out of a
/// rewritten node (see [`resolve`]). The wrapper answers at the node, and
/// every nested position recurses.
#[test]
fn contains_optional_finds_the_wrapper_at_every_depth() {
    assert!(contains_optional(&Ty::Optional(Box::new(Ty::Int))));
    assert!(contains_optional(&Ty::List(Box::new(Ty::Optional(
        Box::new(Ty::Int)
    )))));
    assert!(contains_optional(&Ty::Set(Box::new(Ty::Optional(
        Box::new(Ty::Int)
    )))));
    assert!(contains_optional(&Ty::List(Box::new(Ty::List(Box::new(
        Ty::Optional(Box::new(Ty::Int))
    ))))));
    assert!(contains_optional(&Ty::Dict(Box::new((
        Ty::Str,
        Ty::Optional(Box::new(Ty::Int))
    )))));
    assert!(contains_optional(&Ty::Dict(Box::new((
        Ty::Optional(Box::new(Ty::Str)),
        Ty::Int
    )))));
    assert!(contains_optional(&Ty::Tuple(Box::new(vec![
        Ty::Int,
        Ty::Optional(Box::new(Ty::Int))
    ]))));
}

/// The complement: every shape with no `Optional` wrapper anywhere must
/// pass, `Ty::Infer` included -- [`concrete`] is what rejects that one,
/// and this check must not silently duplicate its job.
#[test]
fn contains_optional_admits_every_shape_without_a_wrapper() {
    for ty in [
        Ty::Int,
        Ty::Float,
        Ty::Bool,
        Ty::Str,
        Ty::None,
        Ty::Infer,
        Ty::Param(Box::new("T".to_string())),
        Ty::Instance(Box::new("C".to_string())),
        Ty::Protocol(Box::new("P".to_string())),
        Ty::List(Box::new(Ty::Int)),
        Ty::Set(Box::new(Ty::Int)),
        Ty::Dict(Box::new((Ty::Str, Ty::Int))),
        Ty::Tuple(Box::new(vec![Ty::Int, Ty::Str])),
    ] {
        assert!(!contains_optional(&ty), "{ty:?} carries no wrapper");
    }
}

/// `inferred` is `concrete` *plus* the wrapper check, on either half of a
/// dict resolution -- and `concrete` alone still admits what only the
/// annotation source is allowed to store.
#[test]
fn inferred_discards_what_concrete_alone_admits() {
    assert!(inferred(Resolution::List(Ty::Int)).is_some());
    assert!(inferred(Resolution::List(Ty::Infer)).is_none());
    assert!(inferred(Resolution::List(Ty::Optional(Box::new(Ty::Int)))).is_none());
    assert!(concrete(Resolution::List(Ty::Optional(Box::new(Ty::Int)))).is_some());
    assert!(inferred(Resolution::Dict(Ty::Str, Ty::Int)).is_some());
    assert!(inferred(Resolution::Dict(Ty::Str, Ty::Optional(Box::new(Ty::Int)))).is_none());
    assert!(inferred(Resolution::Dict(Ty::Optional(Box::new(Ty::Str)), Ty::Int)).is_none());
}
