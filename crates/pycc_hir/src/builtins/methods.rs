//! Builtin-type methods (`receiver.method(args)`) that lower to a plain
//! call into `pycc_rt`, split out of `builtins.rs` by cohesion.

use super::{
    BuiltinSig, hashed_set, is_intlike, list_of, orderable, same_elem, searches_elem, sig,
    value_comparable, word_sig,
};
use crate::Ty;

/// `args` with a generator argument of `set.update` / `list.extend` seen as
/// the list of its values (`pycc_mir` materializes it first, as the
/// consuming builtins do), or `None` when nothing changes.
#[must_use]
pub fn materialized_method_args(receiver: &Ty, method: &str, args: &[Ty]) -> Option<Vec<Ty>> {
    match (receiver, method, args) {
        (Ty::Set(_), "update", [Ty::Generator(sig)])
        | (Ty::List(_), "extend", [Ty::Generator(sig)]) => {
            Some(vec![Ty::List(Box::new(sig.0.clone()))])
        }
        _ => None,
    }
}

/// Resolves `receiver.method(args)` for a builtin receiver type. The
/// receiver becomes the runtime function's first argument.
#[must_use]
pub fn resolve_method(
    receiver: &Ty,
    method: &str,
    args: &[Ty],
) -> Option<Result<BuiltinSig, String>> {
    let (int, str_) = (Ty::Int, Ty::Str);
    let s = |symbol: &'static str, rest: &[Ty], ret: Ty| {
        let mut params = vec![receiver.clone()];
        params.extend_from_slice(rest);
        sig(symbol, &params, ret)
    };
    let found = match (receiver, method, args) {
        (Ty::Str, "split" | "rsplit", []) => s("pycc_rt_str_split_ws", &[], list_of(str_)),
        (Ty::Str, "split", [Ty::Str]) => {
            s("pycc_rt_str_split", std::slice::from_ref(&str_), list_of(str_.clone()))
        }
        (Ty::Str, "split", [Ty::Str, m]) if is_intlike(m) => {
            s("pycc_rt_str_split_max", &[str_.clone(), int], list_of(str_))
        }
        (Ty::Str, "splitlines", []) => s("pycc_rt_str_splitlines", &[], list_of(str_)),
        (Ty::Str, "strip", []) => s("pycc_rt_str_strip", &[], str_),
        (Ty::Str, "lstrip", []) => s("pycc_rt_str_lstrip", &[], str_),
        (Ty::Str, "rstrip", []) => s("pycc_rt_str_rstrip", &[], str_),
        (Ty::Str, "strip", [Ty::Str]) => {
            s("pycc_rt_str_strip_chars", std::slice::from_ref(&str_), str_.clone())
        }
        (Ty::Str, "lstrip", [Ty::Str]) => {
            s("pycc_rt_str_lstrip_chars", std::slice::from_ref(&str_), str_.clone())
        }
        (Ty::Str, "rstrip", [Ty::Str]) => {
            s("pycc_rt_str_rstrip_chars", std::slice::from_ref(&str_), str_.clone())
        }
        (Ty::Str, "join", [Ty::List(elem)]) if **elem == Ty::Str => {
            s("pycc_rt_str_join", &[args[0].clone()], str_)
        }
        (Ty::Str, "join", [Ty::Str]) => {
            s("pycc_rt_str_join_chars", std::slice::from_ref(&str_), str_.clone())
        }
        (Ty::Str, "count", [Ty::Str]) => s("pycc_rt_str_count", &[str_], int),
        (Ty::Str, "find", [Ty::Str]) => s("pycc_rt_str_find", &[str_], int),
        (Ty::Str, "rfind", [Ty::Str]) => s("pycc_rt_str_rfind", &[str_], int),
        (Ty::Str, "index", [Ty::Str]) => s("pycc_rt_str_index", &[str_], int),
        (Ty::Str, "find", [Ty::Str, start]) if is_intlike(start) => {
            s("pycc_rt_str_find_from", &[str_.clone(), int.clone()], int)
        }
        (Ty::Str, "index", [Ty::Str, start]) if is_intlike(start) => {
            s("pycc_rt_str_index_from", &[str_.clone(), int.clone()], int)
        }
        (Ty::Str, "rindex", [Ty::Str]) => s("pycc_rt_str_rindex", &[str_], int),
        (Ty::Str, "replace", [Ty::Str, Ty::Str]) => {
            s("pycc_rt_str_replace", &[str_.clone(), str_.clone()], str_)
        }
        (Ty::Str, "upper", []) => s("pycc_rt_str_upper", &[], str_),
        (Ty::Str, "lower", []) => s("pycc_rt_str_lower", &[], str_),
        (Ty::Str, "swapcase", []) => s("pycc_rt_str_swapcase", &[], str_),
        (Ty::Str, "startswith", [Ty::Str]) => s("pycc_rt_str_startswith", &[str_], Ty::Bool),
        (Ty::Str, "startswith", [Ty::Str, start]) if is_intlike(start) => {
            s("pycc_rt_str_startswith_from", &[str_, int], Ty::Bool)
        }
        (Ty::Str, "endswith", [Ty::Str]) => s("pycc_rt_str_endswith", &[str_], Ty::Bool),
        (Ty::Str, "isdigit", []) => s("pycc_rt_str_isdigit", &[], Ty::Bool),
        (Ty::Str, "isalpha", []) => s("pycc_rt_str_isalpha", &[], Ty::Bool),
        (Ty::Str, "isupper", []) => s("pycc_rt_str_isupper", &[], Ty::Bool),
        (Ty::Str, "islower", []) => s("pycc_rt_str_islower", &[], Ty::Bool),
        (Ty::Str, "isspace", []) => s("pycc_rt_str_isspace", &[], Ty::Bool),
        (Ty::Str, "isalnum", []) => s("pycc_rt_str_isalnum", &[], Ty::Bool),
        (Ty::Str, "zfill", [w]) if is_intlike(w) => s("pycc_rt_str_zfill", &[int], str_),
        (Ty::Str, "ljust", [w]) if is_intlike(w) => s("pycc_rt_str_ljust", &[int], str_),
        (Ty::Str, "rjust", [w]) if is_intlike(w) => s("pycc_rt_str_rjust", &[int], str_),
        (Ty::Str, "center", [w]) if is_intlike(w) => s("pycc_rt_str_center", &[int], str_),
        (Ty::List(elem), "sort", []) if orderable(elem) => s("pycc_rt_list_sort", &[], Ty::None),
        (Ty::List(_), "reverse", []) => s("pycc_rt_list_reverse", &[], Ty::None),
        (Ty::List(_), "clear", []) => s("pycc_rt_list_clear", &[], Ty::None),
        // Keeps a deque's flavor (`list(xs)` is `pycc_rt_list_copy`).
        (Ty::List(_), "copy", []) => s("pycc_rt_list_copy_method", &[], receiver.clone()),
        // `collections.deque`'s own methods (`pycc_rt::list::deque`); a
        // plain list raises CPython's `AttributeError` at run time.
        (Ty::List(elem), "appendleft", [item]) if same_elem(item, elem) => word_sig(
            "pycc_rt_deque_appendleft",
            &[receiver.clone(), (**elem).clone()],
            &[false, true],
            Ty::None,
            false,
            &[],
        ),
        (Ty::List(elem), "popleft", []) => word_sig(
            "pycc_rt_deque_popleft",
            std::slice::from_ref(receiver),
            &[false],
            (**elem).clone(),
            true,
            &[],
        ),
        (Ty::List(elem), "extendleft", [Ty::List(other)]) if elem == other => {
            s("pycc_rt_deque_extendleft", std::slice::from_ref(receiver), Ty::None)
        }
        (Ty::List(_), "rotate", []) => s("pycc_rt_deque_rotate1", &[], Ty::None),
        (Ty::List(_), "rotate", [n]) if is_intlike(n) => {
            s("pycc_rt_deque_rotate", &[Ty::Int], Ty::None)
        }
        (Ty::List(elem), "extend", [Ty::List(other)]) if elem == other => {
            s("pycc_rt_list_extend", std::slice::from_ref(receiver), Ty::None)
        }
        (Ty::List(elem), "count" | "index" | "remove", [item])
            if searches_elem(item, elem) && value_comparable(elem) =>
        {
            let (symbol, ret) = match method {
                "count" => ("pycc_rt_list_count", int),
                "index" => ("pycc_rt_list_index", int),
                _ => ("pycc_rt_list_remove", Ty::None),
            };
            let mut found = s(symbol, &[(**elem).clone()], ret);
            found.word_args = vec![false, true];
            found
        }
        // `del d[k]` / `del xs[i]` (`HirStmt::DeleteItem` on a native base).
        (Ty::Dict(kv), "__pycc_delitem__", [key]) if same_elem(key, &kv.0) => word_sig(
            "pycc_rt_dict_del",
            &[receiver.clone(), kv.0.clone()],
            &[false, true],
            Ty::None,
            false,
            &[],
        ),
        (Ty::List(_), "__pycc_delitem__", [index]) if is_intlike(index) => {
            s("pycc_rt_list_delitem", &[Ty::Int], Ty::None)
        }
        (Ty::Dict(kv), "keys", []) => s("pycc_rt_dict_keys", &[], list_of(kv.0.clone())),
        (Ty::Dict(kv), "values", []) => s("pycc_rt_dict_values", &[], list_of(kv.1.clone())),
        (Ty::Dict(kv), "items", []) => s(
            "pycc_rt_dict_items",
            &[],
            list_of(Ty::Tuple(Box::new(vec![kv.0.clone(), kv.1.clone()]))),
        ),
        // Keeps a `Counter`'s or `defaultdict`'s flavor (`dict(d)` is
        // `pycc_rt_dict_copy_of`).
        (Ty::Dict(_), "copy", []) => s("pycc_rt_dict_copy_method", &[], receiver.clone()),
        // `collections.Counter`'s own methods (`pycc_rt::dict::flavor`); a
        // plain dict raises CPython's `AttributeError` at run time.
        (Ty::Dict(kv), "most_common", []) if kv.1 == Ty::Int => s(
            "pycc_rt_counter_most_common_all",
            &[],
            list_of(Ty::Tuple(Box::new(vec![kv.0.clone(), Ty::Int]))),
        ),
        (Ty::Dict(kv), "most_common", [n]) if kv.1 == Ty::Int && is_intlike(n) => s(
            "pycc_rt_counter_most_common",
            &[Ty::Int],
            list_of(Ty::Tuple(Box::new(vec![kv.0.clone(), Ty::Int]))),
        ),
        (Ty::Dict(kv), "total", []) if kv.1 == Ty::Int => s("pycc_rt_counter_total", &[], Ty::Int),
        (Ty::Dict(kv), "elements", []) if kv.1 == Ty::Int => {
            s("pycc_rt_counter_elements", &[], list_of(kv.0.clone()))
        }
        (Ty::Dict(kv), "update", [Ty::List(elem)]) if kv.1 == Ty::Int && **elem == kv.0 => {
            s("pycc_rt_counter_update_list", &[args[0].clone()], Ty::None)
        }
        (Ty::Dict(kv), "update", [Ty::Str]) if kv == &Box::new((Ty::Str, Ty::Int)) => {
            s("pycc_rt_counter_update_str", &[Ty::Str], Ty::None)
        }
        (Ty::Dict(kv), "subtract", [Ty::List(elem)]) if kv.1 == Ty::Int && **elem == kv.0 => {
            s("pycc_rt_counter_subtract_list", &[args[0].clone()], Ty::None)
        }
        (Ty::Dict(kv), "subtract", [other]) if kv.1 == Ty::Int && other == receiver => {
            s("pycc_rt_counter_subtract_dict", std::slice::from_ref(receiver), Ty::None)
        }
        (Ty::Dict(_), "clear", []) => s("pycc_rt_dict_clear", &[], Ty::None),
        (Ty::Dict(_), "update", [other]) if other == receiver => {
            s("pycc_rt_dict_update", std::slice::from_ref(receiver), Ty::None)
        }
        (Ty::Dict(kv), "popitem", []) => s(
            "pycc_rt_dict_popitem",
            &[],
            Ty::Tuple(Box::new(vec![kv.0.clone(), kv.1.clone()])),
        ),
        (Ty::Dict(kv), "pop", [key]) if same_elem(key, &kv.0) => word_sig(
            "pycc_rt_dict_pop_key",
            &[receiver.clone(), kv.0.clone()],
            &[false, true],
            kv.1.clone(),
            true,
            &[],
        ),
        (Ty::Dict(kv), "pop", [key, default])
            if same_elem(key, &kv.0) && same_elem(default, &kv.1) =>
        {
            word_sig(
                "pycc_rt_dict_pop_or",
                &[receiver.clone(), kv.0.clone(), kv.1.clone()],
                &[false, true, true],
                kv.1.clone(),
                true,
                std::slice::from_ref(&kv.1),
            )
        }
        (Ty::Dict(kv), "setdefault", [key, default])
            if same_elem(key, &kv.0) && same_elem(default, &kv.1) =>
        {
            word_sig(
                "pycc_rt_dict_setdefault_of",
                &[receiver.clone(), kv.0.clone(), kv.1.clone()],
                &[false, true, true],
                kv.1.clone(),
                true,
                &[kv.0.clone(), kv.1.clone()],
            )
        }
        (set, "discard" | "remove", [item])
            if matches!(set, Ty::Set(_)) && hashed_set(set).is_some_and(|e| same_elem(item, e)) =>
        {
            let elem = hashed_set(set)?.clone();
            word_sig(
                if method == "discard" {
                    "pycc_rt_set_discard"
                } else {
                    "pycc_rt_set_remove"
                },
                &[receiver.clone(), elem],
                &[false, true],
                Ty::None,
                false,
                &[],
            )
        }
        (Ty::Set(elem), "pop", []) if crate::is_hashable_ty(elem) => word_sig(
            "pycc_rt_set_pop",
            std::slice::from_ref(receiver),
            &[false],
            (**elem).clone(),
            true,
            &[],
        ),
        (Ty::Set(elem), "clear", []) if crate::is_hashable_ty(elem) => {
            s("pycc_rt_set_clear", &[], Ty::None)
        }
        (set, "copy", []) if hashed_set(set).is_some() => {
            s("pycc_rt_set_copy_method", &[], receiver.clone())
        }
        (Ty::Set(elem), "update", [Ty::List(other)])
            if crate::is_hashable_ty(elem) && same_elem(other, elem) =>
        {
            s("pycc_rt_set_update_list", &[args[0].clone()], Ty::None)
        }
        (Ty::Set(elem), "update", [other])
            if crate::is_hashable_ty(elem) && hashed_set(other) == Some(elem) =>
        {
            s("pycc_rt_set_update", std::slice::from_ref(other), Ty::None)
        }
        (set, "union" | "intersection" | "difference" | "symmetric_difference", [other])
            if hashed_set(set).is_some() && hashed_set(other) == hashed_set(set) =>
        {
            let symbol = match method {
                "union" => "pycc_rt_set_union",
                "intersection" => "pycc_rt_set_intersection",
                "difference" => "pycc_rt_set_difference",
                _ => "pycc_rt_set_symmetric_difference",
            };
            s(symbol, std::slice::from_ref(other), receiver.clone())
        }
        // A `str` argument is the iterable of its characters
        // (`ILLEGAL_CHARS.isdisjoint(text)`).
        (set, "issubset" | "issuperset" | "isdisjoint", [Ty::Str])
            if hashed_set(set) == Some(&Ty::Str) =>
        {
            let symbol = match method {
                "issubset" => "pycc_rt_set_issubset_str",
                "issuperset" => "pycc_rt_set_issuperset_str",
                _ => "pycc_rt_set_isdisjoint_str",
            };
            s(symbol, &[Ty::Str], Ty::Bool)
        }
        (set, "issubset" | "issuperset" | "isdisjoint", [other])
            if hashed_set(set).is_some() && hashed_set(other) == hashed_set(set) =>
        {
            let symbol = match method {
                "issubset" => "pycc_rt_set_issubset",
                "issuperset" => "pycc_rt_set_issuperset",
                _ => "pycc_rt_set_isdisjoint",
            };
            s(symbol, std::slice::from_ref(other), Ty::Bool)
        }
        _ => return None,
    };
    Some(Ok(found))
}
