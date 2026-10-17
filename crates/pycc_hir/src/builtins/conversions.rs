//! The container conversions `list(x)`, `set(x)`, `dict(x)` and `tuple(x)`
//! that lower to `pycc_rt`.

use super::{BuiltinSig, hashed_set, list_of, sig, word_sig};
use crate::Ty;

/// The runtime call converting `args` to the `name` container, when one is
/// compiled.
pub(super) fn conversion_call(name: &str, args: &[Ty]) -> Option<BuiltinSig> {
    let str_ = Ty::Str;
    let found = match (name, args) {
        ("list", [Ty::List(_)]) => sig("pycc_rt_list_copy", &[args[0].clone()], args[0].clone()),
        ("list", [Ty::Str]) => {
            sig("pycc_rt_str_to_list", std::slice::from_ref(&str_), list_of(str_.clone()))
        }
        // Containers of the generic runtime objects (`pycc_rt::set`,
        // `pycc_rt::dict`, tuple objects).
        ("set", [Ty::List(elem)]) if crate::is_hashable_ty(elem) => sig(
            "pycc_rt_set_of_list",
            &[args[0].clone()],
            Ty::Set(elem.clone()),
        ),
        ("set", [Ty::Str]) => sig(
            "pycc_rt_set_from_str",
            std::slice::from_ref(&str_),
            Ty::Set(Box::new(str_.clone())),
        ),
        ("set", [set]) if hashed_set(set).is_some() => sig(
            "pycc_rt_set_copy",
            std::slice::from_ref(set),
            Ty::Set(Box::new(hashed_set(set)?.clone())),
        ),
        ("set", [Ty::Dict(kv)]) if crate::is_hashable_ty(&kv.0) => sig(
            "pycc_rt_set_from_dict",
            &[args[0].clone()],
            Ty::Set(Box::new(kv.0.clone())),
        ),
        ("list", [set]) if hashed_set(set).is_some() => sig(
            "pycc_rt_set_list",
            std::slice::from_ref(set),
            list_of(hashed_set(set)?.clone()),
        ),
        ("list", [Ty::Dict(kv)]) => sig(
            "pycc_rt_dict_keys",
            &[args[0].clone()],
            list_of(kv.0.clone()),
        ),
        ("list", [Ty::Tuple(elements)])
            if !elements.is_empty()
                && crate::tuple_is_boxed(elements)
                && elements.iter().all(|e| *e == elements[0]) =>
        {
            word_sig(
                "pycc_rt_tuple_to_list",
                &[args[0].clone()],
                &[false],
                list_of(elements[0].clone()),
                false,
                &[elements[0].clone()],
            )
        }
        ("dict", [Ty::List(elem)]) => match elem.as_ref() {
            Ty::Tuple(pair) if pair.len() == 2 && crate::is_hashable_ty(&pair[0]) => word_sig(
                "pycc_rt_dict_of_pairs",
                &[args[0].clone()],
                &[false],
                Ty::Dict(Box::new((pair[0].clone(), pair[1].clone()))),
                false,
                &[pair[0].clone(), pair[1].clone()],
            ),
            _ => return None,
        },
        ("dict", [Ty::Dict(_)]) => sig("pycc_rt_dict_copy_of", &[args[0].clone()], args[0].clone()),
        // `tuple(x)`: a new tuple object of the list's elements (a
        // `tuple[T, ...]`), or of a `str`'s characters. A `bool` list keeps
        // its byte storage, which no tuple object has.
        ("tuple", [Ty::List(elem)]) if crate::container::is_held_var_tuple_element_ty(elem) => {
            sig("pycc_rt_list_to_tuple", &[args[0].clone()], Ty::VarTuple(elem.clone()))
        }
        ("tuple", [Ty::Str]) => sig(
            "pycc_rt_str_to_tuple",
            std::slice::from_ref(&str_),
            Ty::VarTuple(Box::new(str_.clone())),
        ),
        _ => return None,
    };
    Some(found)
}
