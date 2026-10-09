use super::{check_container_ty, check_tuple_element_ty};
use crate::Ty;
use pycc_diag::Span;

fn span() -> Span {
    Span::new(7, 19)
}

#[test]
fn tuple_element_gate_accepts_every_scalar_it_documents() {
    for ty in [Ty::Int, Ty::Bool, Ty::Float] {
        assert!(check_tuple_element_ty(&ty, span()).is_ok(), "{ty:?}");
    }
}

#[test]
fn tuple_element_gate_rejects_a_non_scalar_element_with_the_caller_s_span() {
    let error = check_tuple_element_ty(&Ty::Str, span()).expect_err("str is not a tuple element");
    assert_eq!(error.code, "T0039");
    assert_eq!(
        error.message,
        "tuple element type `str` is not compiled yet (D-116) -- only int/bool/float elements are"
    );
    assert_eq!(error.span, Some(span()));
}

#[test]
fn container_gate_accepts_the_one_compiled_shape_of_each_family() {
    for ty in [
        Ty::List(Box::new(Ty::Int)),
        Ty::Dict(Box::new((Ty::Str, Ty::Int))),
        Ty::Set(Box::new(Ty::Int)),
        Ty::Tuple(Box::new(vec![Ty::Int, Ty::Bool, Ty::Float])),
    ] {
        assert!(check_container_ty(&ty, span()).is_ok(), "{ty:?}");
    }
}

#[test]
fn container_gate_rejects_a_list_element_type_codegen_cannot_represent() {
    let error = check_container_ty(&Ty::List(Box::new(Ty::Str)), span())
        .expect_err("list[str] is not compiled");
    assert_eq!(error.code, "T0034");
    assert_eq!(
        error.message,
        "list[str] is not compiled yet (D-105) -- only list[int] is"
    );
    assert_eq!(error.span, Some(span()));
}

#[test]
fn container_gate_rejects_a_dict_key_value_pair_codegen_cannot_represent() {
    let error = check_container_ty(&Ty::Dict(Box::new((Ty::Int, Ty::Int))), span())
        .expect_err("dict[int, int] is not compiled");
    assert_eq!(error.code, "T0036");
    assert_eq!(
        error.message,
        "dict[int, int] is not compiled yet (D-122) -- only dict[str, int] is"
    );
}

#[test]
fn container_gate_rejects_a_set_element_type_codegen_cannot_represent() {
    let error = check_container_ty(&Ty::Set(Box::new(Ty::Str)), span())
        .expect_err("set[str] is not compiled");
    assert_eq!(error.code, "T0038");
    assert_eq!(
        error.message,
        "set[str] is not compiled yet (D-122) -- only set[int] and a set of a user-class instance are"
    );
}

#[test]
fn container_gate_checks_every_tuple_element_not_only_the_first() {
    let error = check_container_ty(&Ty::Tuple(Box::new(vec![Ty::Int, Ty::Str])), span())
        .expect_err("tuple[int, str] is not compiled");
    assert_eq!(error.code, "T0039");
    assert!(error.message.contains("`str`"), "{}", error.message);
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
