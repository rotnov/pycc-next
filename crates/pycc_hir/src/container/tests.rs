use super::{check_container_ty, check_tuple_element_ty, is_hashable_ty, tuple_is_boxed};
use crate::Ty;
use pycc_diag::Span;

fn span() -> Span {
    Span::new(7, 19)
}

fn list(elem: Ty) -> Ty {
    Ty::List(Box::new(elem))
}

fn set(elem: Ty) -> Ty {
    Ty::Set(Box::new(elem))
}

fn dict(key: Ty, value: Ty) -> Ty {
    Ty::Dict(Box::new((key, value)))
}

fn tuple(elements: Vec<Ty>) -> Ty {
    Ty::Tuple(Box::new(elements))
}

#[test]
fn tuple_element_gate_accepts_every_container_element() {
    for ty in [
        Ty::Int,
        Ty::Bool,
        Ty::Float,
        Ty::Str,
        list(Ty::Int),
        tuple(vec![Ty::Int, Ty::Str]),
        dict(Ty::Int, Ty::Int),
        set(Ty::Int),
    ] {
        assert!(check_tuple_element_ty(&ty, span()).is_ok(), "{ty:?}");
    }
}

#[test]
fn tuple_element_gate_rejects_a_non_element_with_the_caller_s_span() {
    let error = check_tuple_element_ty(&Ty::None, span()).expect_err("None is not an element");
    assert_eq!(error.code, "T0039");
    assert_eq!(
        error.message,
        "tuple element type `None` is not compiled yet -- a tuple element must be int, float, \
         bool, str, a user-class instance, or a list, tuple, dict or set of those"
    );
    assert_eq!(error.span, Some(span()));
}

#[test]
fn only_a_tuple_of_int_bool_and_float_stays_an_unboxed_struct() {
    assert!(!tuple_is_boxed(&[Ty::Int, Ty::Bool, Ty::Float]));
    assert!(tuple_is_boxed(&[Ty::Int, Ty::Str]));
    assert!(tuple_is_boxed(&[list(Ty::Int)]));
}

#[test]
fn hashable_types_are_the_immutable_ones() {
    for ty in [
        Ty::Int,
        Ty::Float,
        Ty::Bool,
        Ty::Str,
        tuple(vec![Ty::Int, Ty::Str]),
    ] {
        assert!(is_hashable_ty(&ty), "{ty:?}");
    }
    for ty in [
        list(Ty::Int),
        set(Ty::Int),
        dict(Ty::Int, Ty::Int),
        tuple(vec![list(Ty::Int)]),
    ] {
        assert!(!is_hashable_ty(&ty), "{ty:?}");
    }
}

#[test]
fn container_gate_accepts_every_compiled_shape() {
    for ty in [
        list(Ty::Int),
        list(tuple(vec![Ty::Int, Ty::Str])),
        list(dict(Ty::Str, list(Ty::Int))),
        list(set(Ty::Str)),
        dict(Ty::Str, Ty::Int),
        dict(Ty::Int, list(Ty::Int)),
        dict(tuple(vec![Ty::Int, Ty::Int]), Ty::Float),
        set(Ty::Int),
        set(Ty::Str),
        set(tuple(vec![Ty::Int, Ty::Int])),
        tuple(vec![Ty::Int, Ty::Bool, Ty::Float]),
        tuple(vec![Ty::Str, list(Ty::Int)]),
    ] {
        assert!(check_container_ty(&ty, span()).is_ok(), "{ty:?}");
    }
}

#[test]
fn container_gate_accepts_every_one_word_list_element() {
    for element in [
        Ty::Int,
        Ty::Float,
        Ty::Bool,
        Ty::Str,
        Ty::Instance(Box::new("C".to_string())),
        list(list(Ty::Str)),
    ] {
        let ty = list(element);
        assert!(check_container_ty(&ty, span()).is_ok(), "{ty:?}");
    }
}

#[test]
fn container_gate_rejects_a_list_element_type_codegen_cannot_represent() {
    let error = check_container_ty(&list(Ty::None), span()).expect_err("list[None]");
    assert_eq!(error.code, "T0034");
    assert_eq!(
        error.message,
        "list[None] is not compiled yet -- a list element must be int, float, bool, str, a \
         user-class instance, or a list, tuple, dict or set of those"
    );
    assert_eq!(error.span, Some(span()));
}

#[test]
fn container_gate_rejects_an_unhashable_key_or_an_unstorable_value() {
    for ty in [dict(list(Ty::Int), Ty::Int), dict(Ty::Int, Ty::None)] {
        let error = check_container_ty(&ty, span()).expect_err("not a compiled dict");
        assert_eq!(error.code, "T0036", "{ty:?}");
        assert!(
            error.message.contains("is not compiled yet"),
            "{}",
            error.message
        );
    }
}

#[test]
fn container_gate_rejects_an_unhashable_set_element() {
    let error = check_container_ty(&set(list(Ty::Int)), span()).expect_err("set[list[int]]");
    assert_eq!(error.code, "T0038");
    assert_eq!(
        error.message,
        "set[list[int]] is not compiled yet -- a set element must be int, float, bool, str, a \
         tuple of those or a user-class instance"
    );
}

#[test]
fn container_gate_checks_every_tuple_element_not_only_the_first() {
    let error = check_container_ty(&tuple(vec![Ty::Int, Ty::None]), span())
        .expect_err("tuple[int, None] is not compiled");
    assert_eq!(error.code, "T0039");
    assert!(error.message.contains("`None`"), "{}", error.message);
}

#[test]
fn container_gate_passes_a_non_container_type_through_untouched() {
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
        Ty::Optional(Box::new(Ty::Int)),
    ] {
        assert!(check_container_ty(&ty, span()).is_ok(), "{ty:?}");
    }
}
