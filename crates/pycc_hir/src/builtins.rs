//! The table of builtin functions and builtin-type methods that lower to a
//! plain call into `pycc_rt` (`MirExpr::RtCall`).
//!
//! One resolver serves the type checker (`pycc_types`, which reads the
//! result type) and MIR lowering (`pycc_mir`, which emits the call), so the
//! two can never disagree about which calls exist. A call this table does
//! not claim falls through to the older hand-written arms in both crates;
//! the table deliberately never claims a shape one of those arms handles
//! (`len` of a list, `int`/`str` of a CPython object, `float` of a number,
//! `str` of a list, `pycc_types::int_builtins`' `abs`/`divmod`/`int` of a
//! number), so the order of the two checks does not matter.
//!
//! ABI: argument `i` is coerced to `params[i]` and passed in that type's
//! native representation (`int` as its encoded word, `float` as `f64`,
//! `bool` as `i8`, `str`/containers as a pointer), borrowed -- except an
//! argument flagged in [`BuiltinSig::word_args`], which is passed as the
//! list element word `pycc_rt::list` stores for a `params[i]` element. A
//! `str` result is a new reference. When [`BuiltinSig::ret_word`] is set
//! the runtime returns a list element word of type `ret` (a `str` word
//! again a new reference).

use crate::Ty;

/// How one builtin call lowers: the runtime symbol and its ABI.
#[derive(Debug, Clone, PartialEq)]
pub struct BuiltinSig {
    pub symbol: &'static str,
    pub params: Vec<Ty>,
    pub ret: Ty,
    pub ret_word: bool,
    pub word_args: Vec<bool>,
    /// Trailing `i64` arguments, after the call's own: the container word
    /// kind (`pycc_rt::LIST_KIND_*`) of each type, for a runtime function
    /// that builds or retains words its arguments do not describe.
    pub kind_args: Vec<Ty>,
}

fn sig(symbol: &'static str, params: &[Ty], ret: Ty) -> BuiltinSig {
    BuiltinSig {
        symbol,
        params: params.to_vec(),
        ret,
        ret_word: false,
        word_args: vec![false; params.len()],
        kind_args: Vec::new(),
    }
}

/// [`sig`] whose arguments flagged in `words` pass as container words, whose
/// result is a word when `ret_word`, and which takes `kinds` as trailing
/// kind arguments.
fn word_sig(
    symbol: &'static str,
    params: &[Ty],
    words: &[bool],
    ret: Ty,
    ret_word: bool,
    kinds: &[Ty],
) -> BuiltinSig {
    BuiltinSig {
        symbol,
        params: params.to_vec(),
        ret,
        ret_word,
        word_args: words.to_vec(),
        kind_args: kinds.to_vec(),
    }
}

/// Whether `ty` is a set or frozenset of hashable (natively hashed)
/// elements.
fn hashed_set(ty: &Ty) -> Option<&Ty> {
    match ty {
        Ty::Set(elem) | Ty::FrozenSet(elem) if crate::is_hashable_ty(elem) => Some(elem),
        _ => None,
    }
}

/// Whether `pycc_rt_list_repr` renders `ty` itself (a class instance needs
/// its `__repr__`, which the runtime cannot call).
fn renders_natively(ty: &Ty) -> bool {
    match ty {
        Ty::Int | Ty::Float | Ty::Bool | Ty::Str => true,
        Ty::List(elem) | Ty::Set(elem) | Ty::FrozenSet(elem) => renders_natively(elem),
        Ty::Tuple(elements) => elements.iter().all(renders_natively),
        Ty::Dict(kv) => renders_natively(&kv.0) && renders_natively(&kv.1),
        _ => false,
    }
}

fn is_intlike(ty: &Ty) -> bool {
    matches!(ty, Ty::Int | Ty::Bool)
}

fn is_numeric(ty: &Ty) -> bool {
    matches!(ty, Ty::Int | Ty::Bool | Ty::Float)
}

/// The runtime symbol of a `math` function of `float` arguments (an `int`
/// argument is converted to `float` at the call) and its arity.
fn float_math(name: &str) -> Option<(&'static str, usize)> {
    Some(match name {
        "math.sqrt" => ("pycc_rt_math_sqrt", 1),
        "math.exp" => ("pycc_rt_math_exp", 1),
        "math.sin" => ("pycc_rt_math_sin", 1),
        "math.cos" => ("pycc_rt_math_cos", 1),
        "math.tan" => ("pycc_rt_math_tan", 1),
        "math.asin" => ("pycc_rt_math_asin", 1),
        "math.acos" => ("pycc_rt_math_acos", 1),
        "math.atan" => ("pycc_rt_math_atan", 1),
        "math.fabs" => ("pycc_rt_math_fabs", 1),
        "math.atan2" => ("pycc_rt_math_atan2", 2),
        "math.hypot" => ("pycc_rt_math_hypot", 2),
        "math.copysign" => ("pycc_rt_math_copysign", 2),
        _ => return None,
    })
}

/// `math.log`/`log2`/`log10` of one argument: an `int` is taken exactly.
fn math_log(name: &str, arg: &Ty) -> Option<BuiltinSig> {
    let (float_symbol, int_symbol) = match name {
        "math.log" => ("pycc_rt_math_log", "pycc_rt_math_log_int"),
        "math.log2" => ("pycc_rt_math_log2", "pycc_rt_math_log2_int"),
        "math.log10" => ("pycc_rt_math_log10", "pycc_rt_math_log10_int"),
        _ => return None,
    };
    Some(match arg {
        Ty::Float => sig(float_symbol, &[Ty::Float], Ty::Float),
        a if is_intlike(a) => sig(int_symbol, &[Ty::Int], Ty::Float),
        _ => return None,
    })
}

/// Whether a list's elements are ordered by a natively supported `<`
/// (`pycc_rt::value::lt`, lexicographic for lists and tuples).
fn orderable(elem: &Ty) -> bool {
    match elem {
        Ty::Int | Ty::Bool | Ty::Float | Ty::Str => true,
        Ty::List(inner) => orderable(inner),
        Ty::Tuple(elements) => elements.iter().all(orderable),
        _ => false,
    }
}

/// Whether two values of `elem` compare by value in the runtime
/// (`pycc_rt::value::eq`): everything but a user-class instance, which
/// compares by identity there.
#[must_use]
pub fn value_comparable(elem: &Ty) -> bool {
    match elem {
        Ty::Instance(_) => false,
        Ty::List(inner) | Ty::Set(inner) | Ty::FrozenSet(inner) => value_comparable(inner),
        Ty::Tuple(elements) => elements.iter().all(value_comparable),
        Ty::Dict(kv) => value_comparable(&kv.1),
        _ => true,
    }
}

/// Whether `item` can be compared for equality against a list of `elem`
/// by the runtime's element-word equality.
fn same_elem(item: &Ty, elem: &Ty) -> bool {
    item == elem || (is_intlike(item) && is_intlike(elem))
}

/// The key and element types of a `(key, element)` pair element.
fn keyed_pair(pair: &Ty) -> Option<(&Ty, &Ty)> {
    match pair {
        Ty::Tuple(items) if items.len() == 2 => Some((&items[0], &items[1])),
        _ => None,
    }
}

fn list_of(elem: Ty) -> Ty {
    Ty::List(Box::new(elem))
}

/// Whether builtin `name` takes a `str` iterable argument as the list of its
/// characters (`zip`'s every argument, `enumerate`'s first): MIR lowering
/// converts it with `pycc_rt_str_to_list` before the call.
#[must_use]
pub fn iterates_str_as_list(name: &str) -> bool {
    matches!(name, "zip" | "enumerate")
}

/// Every builtin name [`resolve_call`] claims for some argument types.
pub const CALL_NAMES: &[&str] = &[
    "__pycc_bisect_left",
    "__pycc_bisect_right",
    "__pycc_heapify",
    "__pycc_heappop",
    "__pycc_heappush",
    "__pycc_heappushpop",
    "__pycc_heapreplace",
    "__pycc_insort_left",
    "__pycc_insort_right",
    "__pycc_max_keyed",
    "__pycc_min_keyed",
    "__pycc_print_str",
    "__pycc_setrecursionlimit",
    "__pycc_sort_keyed",
    "__pycc_sorted_keyed",
    "__pycc_stdin_read",
    "__pycc_stdin_readline",
    "__pycc_stdin_readlines",
    "__pycc_stdout_flush",
    "__pycc_stdout_write",
    "all",
    "any",
    "bin",
    "chr",
    "dict",
    "enumerate",
    "float",
    "hex",
    "input",
    "int",
    "len",
    "list",
    "math.acos",
    "math.asin",
    "math.atan",
    "math.atan2",
    "math.ceil",
    "math.comb",
    "math.copysign",
    "math.cos",
    "math.exp",
    "math.fabs",
    "math.factorial",
    "math.floor",
    "math.gcd",
    "math.hypot",
    "math.isqrt",
    "math.lcm",
    "math.log",
    "math.log10",
    "math.log2",
    "math.perm",
    "math.sin",
    "math.sqrt",
    "math.tan",
    "max",
    "min",
    "oct",
    "ord",
    "pow",
    "reversed",
    "set",
    "sorted",
    "str",
    "sum",
    "zip",
];

/// The result type of builtin `name` whatever its (admitted) argument
/// types are, when it has one.
#[must_use]
pub fn fixed_return(name: &str) -> Option<Ty> {
    Some(match name {
        "input"
        | "str"
        | "chr"
        | "bin"
        | "hex"
        | "oct"
        | "__pycc_stdin_readline"
        | "__pycc_stdin_read" => Ty::Str,
        "int"
        | "len"
        | "ord"
        | "pow"
        | "__pycc_stdout_write"
        | "math.gcd"
        | "math.lcm"
        | "math.isqrt"
        | "math.floor"
        | "math.ceil"
        | "math.factorial"
        | "math.comb"
        | "math.perm"
        | "__pycc_bisect_left"
        | "__pycc_bisect_right" => Ty::Int,
        "__pycc_heappush" | "__pycc_heapify" | "__pycc_insort_left" | "__pycc_insort_right" => {
            Ty::None
        }
        "float" => Ty::Float,
        name if float_math(name).is_some() || name == "math.log" => Ty::Float,
        "any" | "all" => Ty::Bool,
        "__pycc_stdin_readlines" => Ty::List(Box::new(Ty::Str)),
        "__pycc_print_str" | "__pycc_stdout_flush" | "__pycc_setrecursionlimit" => Ty::None,
        _ => return None,
    })
}

/// `min`/`max` with three or more positional arguments fold pairwise.
#[must_use]
pub fn is_pairwise_variadic(name: &str) -> bool {
    matches!(name, "min" | "max" | "math.gcd" | "math.lcm")
}

/// Resolves a call of builtin `name` with positional arguments `args`.
/// `None` when the table does not claim the call (the caller falls back to
/// its own handling, usually a `C0001`), `Some(Err)` for a claimed call
/// with arguments of the wrong type.
#[must_use]
pub fn resolve_call(name: &str, args: &[Ty]) -> Option<Result<BuiltinSig, String>> {
    if iterates_str_as_list(name) && args.contains(&Ty::Str) {
        let as_lists: Vec<Ty> = args
            .iter()
            .enumerate()
            .map(|(i, ty)| match ty {
                Ty::Str if i == 0 || name == "zip" => list_of(Ty::Str),
                other => other.clone(),
            })
            .collect();
        return resolve_call(name, &as_lists);
    }
    let (int, float, str_) = (Ty::Int, Ty::Float, Ty::Str);
    let found = match (name, args) {
        ("input", []) => sig("pycc_rt_input", &[], str_),
        // The `sys` stream operations `pycc_ast::desugar` rewrites to.
        ("__pycc_stdin_readline", []) => sig("pycc_rt_stdin_readline", &[], str_),
        ("__pycc_stdin_read", []) => sig("pycc_rt_stdin_read", &[], str_),
        ("__pycc_stdin_readlines", []) => sig("pycc_rt_stdin_readlines", &[], list_of(str_)),
        ("__pycc_stdout_write", [Ty::Str]) => sig("pycc_rt_stdout_write", &[str_], int),
        ("__pycc_print_str", [Ty::Str]) => {
            sig("pycc_rt_print_write_str", &[str_.clone()], Ty::None)
        }
        ("__pycc_stdout_flush", []) => sig("pycc_rt_print_flush", &[], Ty::None),
        ("__pycc_setrecursionlimit", [a]) if is_intlike(a) => {
            sig("pycc_rt_setrecursionlimit", &[int], Ty::None)
        }
        ("input", [Ty::Str]) => sig("pycc_rt_input_prompt", &[str_.clone()], str_),
        ("int", [Ty::Str]) => sig("pycc_rt_int_from_str", &[str_], int),
        ("int", [Ty::Str, b]) if is_intlike(b) => {
            sig("pycc_rt_int_from_str_base", &[str_, int.clone()], int)
        }
        ("float", [Ty::Str]) => sig("pycc_rt_float_from_str", &[str_], float),
        ("str", [Ty::Int]) => sig("pycc_rt_int_to_str", &[int], str_),
        ("str", [Ty::Float]) => sig("pycc_rt_float_to_str", &[float], str_),
        ("str", [Ty::Bool]) => sig("pycc_rt_bool_to_str", &[Ty::Bool], str_),
        ("str", [Ty::Str]) => sig("pycc_rt_str_copy", &[str_.clone()], str_),
        ("str", []) => sig("pycc_rt_str_empty", &[], str_),
        ("str", [list @ Ty::List(_)]) if renders_natively(list) => {
            sig("pycc_rt_list_repr", &[list.clone()], str_)
        }
        ("str", [dict @ Ty::Dict(_)]) => sig("pycc_rt_dict_repr", &[dict.clone()], str_),
        ("len", [Ty::Str]) => sig("pycc_rt_str_len", &[str_], int),
        ("ord", [Ty::Str]) => sig("pycc_rt_str_ord", &[str_], int),
        ("chr", [a]) if is_intlike(a) => sig("pycc_rt_str_chr", &[int], str_),
        ("bin", [a]) if is_intlike(a) => sig("pycc_rt_int_bin", &[int], str_),
        ("hex", [a]) if is_intlike(a) => sig("pycc_rt_int_hex", &[int], str_),
        ("oct", [a]) if is_intlike(a) => sig("pycc_rt_int_oct", &[int], str_),
        // `math`'s integer functions (`pycc_hir` qualifies the callee as
        // `math.<name>` whichever import spelling bound it).
        ("math.gcd", [a, b]) if is_intlike(a) && is_intlike(b) => {
            sig("pycc_rt_math_gcd", &[int.clone(), int.clone()], int)
        }
        ("math.lcm", [a, b]) if is_intlike(a) && is_intlike(b) => {
            sig("pycc_rt_math_lcm", &[int.clone(), int.clone()], int)
        }
        ("math.isqrt", [a]) if is_intlike(a) => sig("pycc_rt_math_isqrt", &[int.clone()], int),
        ("math.floor", [Ty::Float]) => sig("pycc_rt_math_floor", &[float], int),
        ("math.ceil", [Ty::Float]) => sig("pycc_rt_math_ceil", &[float], int),
        ("math.floor" | "math.ceil", [a]) if is_intlike(a) => {
            sig("pycc_rt_math_int_identity", &[int.clone()], int)
        }
        (_, [a]) if math_log(name, a).is_some() => math_log(name, a)?,
        ("math.log", [x, base]) if is_numeric(x) && is_numeric(base) => {
            let float = |ty: &Ty| *ty == Ty::Float;
            let symbol = match (float(x), float(base)) {
                (true, true) => "pycc_rt_math_log_base_ff",
                (true, false) => "pycc_rt_math_log_base_fi",
                (false, true) => "pycc_rt_math_log_base_if",
                (false, false) => "pycc_rt_math_log_base_ii",
            };
            let param = |f: bool| if f { Ty::Float } else { Ty::Int };
            sig(symbol, &[param(float(x)), param(float(base))], Ty::Float)
        }
        (_, args)
            if float_math(name).is_some_and(|(_, arity)| arity == args.len())
                && args.iter().all(is_numeric) =>
        {
            let (symbol, arity) = float_math(name)?;
            sig(symbol, &vec![Ty::Float; arity], Ty::Float)
        }
        ("math.factorial", [a]) if is_intlike(a) => {
            sig("pycc_rt_math_factorial", &[int.clone()], int)
        }
        ("math.comb" | "math.perm", [a, b]) if is_intlike(a) && is_intlike(b) => sig(
            if name == "math.comb" {
                "pycc_rt_math_comb"
            } else {
                "pycc_rt_math_perm"
            },
            &[int.clone(), int.clone()],
            int,
        ),
        ("pow", [a, b, c]) if is_intlike(a) && is_intlike(b) && is_intlike(c) => sig(
            "pycc_rt_int_powmod",
            &[int.clone(), int.clone(), int.clone()],
            int,
        ),
        ("min" | "max", [a, b]) if a == b || (is_intlike(a) && is_intlike(b)) => {
            let max = name == "max";
            let (symbol, ty) = match a {
                Ty::Int | Ty::Bool => (
                    if max {
                        "pycc_rt_int_max2"
                    } else {
                        "pycc_rt_int_min2"
                    },
                    int,
                ),
                Ty::Float => (
                    if max {
                        "pycc_rt_float_max2"
                    } else {
                        "pycc_rt_float_min2"
                    },
                    float,
                ),
                Ty::Str => (
                    if max {
                        "pycc_rt_str_max2"
                    } else {
                        "pycc_rt_str_min2"
                    },
                    str_,
                ),
                _ => return None,
            };
            let mut found = sig(symbol, &[ty.clone(), ty.clone()], ty);
            // `max(True, False)` is the `bool` `True`, not the int `1`.
            if a == b && *a == Ty::Bool {
                found.ret = Ty::Bool;
                found.params = vec![Ty::Int, Ty::Int];
                found.symbol = if max {
                    "pycc_rt_bool_max2"
                } else {
                    "pycc_rt_bool_min2"
                };
            }
            found
        }
        ("min" | "max", [Ty::List(elem)]) if orderable(elem) => BuiltinSig {
            symbol: if name == "max" {
                "pycc_rt_list_max"
            } else {
                "pycc_rt_list_min"
            },
            params: vec![args[0].clone()],
            ret: (**elem).clone(),
            ret_word: true,
            word_args: vec![false],
            kind_args: Vec::new(),
        },
        ("sum", [Ty::List(elem)]) => match **elem {
            Ty::Int | Ty::Bool => sig("pycc_rt_list_sum_int", &[args[0].clone()], int),
            Ty::Float => sig("pycc_rt_list_sum_float", &[args[0].clone()], float),
            _ => return None,
        },
        ("sorted", [Ty::List(elem)]) if orderable(elem) => {
            sig("pycc_rt_list_sorted", &[args[0].clone()], args[0].clone())
        }
        ("reversed", [Ty::List(_)]) => {
            sig("pycc_rt_list_reversed", &[args[0].clone()], args[0].clone())
        }
        ("list", [Ty::List(_)]) => sig("pycc_rt_list_copy", &[args[0].clone()], args[0].clone()),
        ("list", [Ty::Str]) => sig("pycc_rt_str_to_list", &[str_.clone()], list_of(str_)),
        ("sorted", [Ty::Str]) => sig("pycc_rt_str_sorted", &[str_.clone()], list_of(str_)),
        ("reversed", [Ty::Str]) => sig("pycc_rt_str_reversed", &[str_.clone()], list_of(str_)),
        ("any" | "all", [Ty::List(elem)]) if orderable(elem) => sig(
            if name == "any" {
                "pycc_rt_list_any"
            } else {
                "pycc_rt_list_all"
            },
            &[args[0].clone()],
            Ty::Bool,
        ),
        // Containers of the generic runtime objects (`pycc_rt::set`,
        // `pycc_rt::dict`, tuple objects).
        ("set", [Ty::List(elem)]) if crate::is_hashable_ty(elem) => sig(
            "pycc_rt_set_of_list",
            &[args[0].clone()],
            Ty::Set(elem.clone()),
        ),
        ("set", [Ty::Str]) => sig(
            "pycc_rt_set_from_str",
            &[str_.clone()],
            Ty::Set(Box::new(str_)),
        ),
        ("set", [set]) if hashed_set(set).is_some() => sig(
            "pycc_rt_set_copy",
            &[set.clone()],
            Ty::Set(Box::new(hashed_set(set)?.clone())),
        ),
        ("set", [Ty::Dict(kv)]) if crate::is_hashable_ty(&kv.0) => sig(
            "pycc_rt_set_from_dict",
            &[args[0].clone()],
            Ty::Set(Box::new(kv.0.clone())),
        ),
        ("list", [set]) if hashed_set(set).is_some() => sig(
            "pycc_rt_set_list",
            &[set.clone()],
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
        ("sorted", [set]) if hashed_set(set).is_some_and(orderable) => sig(
            "pycc_rt_set_sorted",
            &[set.clone()],
            list_of(hashed_set(set)?.clone()),
        ),
        ("sorted", [Ty::Dict(kv)]) if orderable(&kv.0) => sig(
            "pycc_rt_dict_sorted",
            &[args[0].clone()],
            list_of(kv.0.clone()),
        ),
        ("min" | "max", [set]) if hashed_set(set).is_some_and(orderable) => {
            let elem = hashed_set(set)?.clone();
            let mut found = word_sig(
                "pycc_rt_set_extreme",
                &[set.clone(), int.clone()],
                &[false, false],
                elem,
                true,
                &[],
            );
            found.symbol = if name == "max" {
                "pycc_rt_set_extreme_max"
            } else {
                "pycc_rt_set_extreme_min"
            };
            found.params.pop();
            found.word_args.pop();
            found
        }
        // `key=`/`reverse=` calls, rewritten by `pycc_ast::desugar::keys`
        // into `(key, element)` pair lists.
        ("__pycc_sorted_keyed", [Ty::List(pair), Ty::Bool]) => {
            let (key, elem) = keyed_pair(pair)?;
            if !orderable(key) {
                return None;
            }
            word_sig(
                "pycc_rt_list_sorted_keyed",
                args,
                &[false, false],
                list_of(elem.clone()),
                false,
                &[elem.clone()],
            )
        }
        ("__pycc_sort_keyed", [Ty::List(target), Ty::List(pair), Ty::Bool]) => {
            let (key, elem) = keyed_pair(pair)?;
            if !orderable(key) || elem != target.as_ref() {
                return None;
            }
            sig("pycc_rt_list_sort_keyed", args, Ty::None)
        }
        ("__pycc_min_keyed" | "__pycc_max_keyed", [Ty::List(pair)]) => {
            let (key, elem) = keyed_pair(pair)?;
            if !orderable(key) {
                return None;
            }
            word_sig(
                if name == "__pycc_min_keyed" {
                    "pycc_rt_list_min_keyed"
                } else {
                    "pycc_rt_list_max_keyed"
                },
                args,
                &[false],
                elem.clone(),
                true,
                &[elem.clone()],
            )
        }
        // `heapq`/`bisect` over a native list (`pycc_ast::desugar::stdlib`
        // spells their calls with these names).
        ("__pycc_heapify", [Ty::List(elem)]) if orderable(elem) => {
            sig("pycc_rt_heapify", &[args[0].clone()], Ty::None)
        }
        ("__pycc_heappop", [Ty::List(elem)]) if orderable(elem) => BuiltinSig {
            symbol: "pycc_rt_heappop",
            params: vec![args[0].clone()],
            ret: (**elem).clone(),
            ret_word: true,
            word_args: vec![false],
            kind_args: Vec::new(),
        },
        (
            "__pycc_heappush"
            | "__pycc_heapreplace"
            | "__pycc_heappushpop"
            | "__pycc_bisect_left"
            | "__pycc_bisect_right"
            | "__pycc_insort_left"
            | "__pycc_insort_right",
            [Ty::List(elem), item],
        ) if orderable(elem) && same_elem(item, elem) => {
            let (symbol, ret, ret_word) = match name {
                "__pycc_heappush" => ("pycc_rt_heappush", Ty::None, false),
                "__pycc_heapreplace" => ("pycc_rt_heapreplace", (**elem).clone(), true),
                "__pycc_heappushpop" => ("pycc_rt_heappushpop", (**elem).clone(), true),
                "__pycc_bisect_left" => ("pycc_rt_bisect_left", int.clone(), false),
                "__pycc_bisect_right" => ("pycc_rt_bisect_right", int.clone(), false),
                "__pycc_insort_left" => ("pycc_rt_insort_left", Ty::None, false),
                _ => ("pycc_rt_insort_right", Ty::None, false),
            };
            word_sig(
                symbol,
                &[args[0].clone(), (**elem).clone()],
                &[false, true],
                ret,
                ret_word,
                &[],
            )
        }
        ("enumerate", [Ty::List(elem)]) => sig(
            "pycc_rt_list_enumerate0",
            &[args[0].clone()],
            list_of(Ty::Tuple(Box::new(vec![int.clone(), (**elem).clone()]))),
        ),
        ("enumerate", [Ty::List(elem), start]) if is_intlike(start) => sig(
            "pycc_rt_list_enumerate",
            &[args[0].clone(), int.clone()],
            list_of(Ty::Tuple(Box::new(vec![int.clone(), (**elem).clone()]))),
        ),
        ("zip", [Ty::List(a), Ty::List(b)]) => sig(
            "pycc_rt_list_zip2",
            &[args[0].clone(), args[1].clone()],
            list_of(Ty::Tuple(Box::new(vec![(**a).clone(), (**b).clone()]))),
        ),
        ("zip", [Ty::List(a), Ty::List(b), Ty::List(c)]) => sig(
            "pycc_rt_list_zip3",
            &[args[0].clone(), args[1].clone(), args[2].clone()],
            list_of(Ty::Tuple(Box::new(vec![
                (**a).clone(),
                (**b).clone(),
                (**c).clone(),
            ]))),
        ),
        _ => return None,
    };
    Some(Ok(found))
}

/// `left <op> right` between two sets (or frozensets) of one hashable
/// element type, for `|`, `&`, `-` and `^`: the runtime call, whose result
/// has the left operand's type as in CPython. `in_place` (`a |= b`) on a
/// `set` updates `left` itself (`pycc_rt_set_inplace` with the operator's
/// index as a trailing int).
#[must_use]
pub fn set_operator(op: &str, in_place: bool, left: &Ty, right: &Ty) -> Option<BuiltinSig> {
    if hashed_set(left).is_none() || hashed_set(left) != hashed_set(right) {
        return None;
    }
    let symbol = match op {
        "|" => "pycc_rt_set_union",
        "&" => "pycc_rt_set_intersection",
        "-" => "pycc_rt_set_difference",
        "^" => "pycc_rt_set_symmetric_difference",
        _ => return None,
    };
    if in_place && matches!(left, Ty::Set(_)) {
        let found = sig(
            "pycc_rt_set_inplace",
            &[left.clone(), right.clone(), Ty::Int],
            left.clone(),
        );
        return Some(found);
    }
    Some(sig(symbol, &[left.clone(), right.clone()], left.clone()))
}

/// The operator index `pycc_rt_set_inplace` takes for `op`.
#[must_use]
pub fn set_inplace_index(op: &str) -> i64 {
    match op {
        "|" => 0,
        "&" => 1,
        "-" => 2,
        _ => 3,
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
        (Ty::Str, "split", [Ty::Str]) => s("pycc_rt_str_split", &[str_.clone()], list_of(str_)),
        (Ty::Str, "split", [Ty::Str, m]) if is_intlike(m) => {
            s("pycc_rt_str_split_max", &[str_.clone(), int], list_of(str_))
        }
        (Ty::Str, "splitlines", []) => s("pycc_rt_str_splitlines", &[], list_of(str_)),
        (Ty::Str, "strip", []) => s("pycc_rt_str_strip", &[], str_),
        (Ty::Str, "lstrip", []) => s("pycc_rt_str_lstrip", &[], str_),
        (Ty::Str, "rstrip", []) => s("pycc_rt_str_rstrip", &[], str_),
        (Ty::Str, "strip", [Ty::Str]) => s("pycc_rt_str_strip_chars", &[str_.clone()], str_),
        (Ty::Str, "lstrip", [Ty::Str]) => s("pycc_rt_str_lstrip_chars", &[str_.clone()], str_),
        (Ty::Str, "rstrip", [Ty::Str]) => s("pycc_rt_str_rstrip_chars", &[str_.clone()], str_),
        (Ty::Str, "join", [Ty::List(elem)]) if **elem == Ty::Str => {
            s("pycc_rt_str_join", &[args[0].clone()], str_)
        }
        (Ty::Str, "join", [Ty::Str]) => s("pycc_rt_str_join_chars", &[str_.clone()], str_),
        (Ty::Str, "count", [Ty::Str]) => s("pycc_rt_str_count", &[str_], int),
        (Ty::Str, "find", [Ty::Str]) => s("pycc_rt_str_find", &[str_], int),
        (Ty::Str, "rfind", [Ty::Str]) => s("pycc_rt_str_rfind", &[str_], int),
        (Ty::Str, "index", [Ty::Str]) => s("pycc_rt_str_index", &[str_], int),
        (Ty::Str, "rindex", [Ty::Str]) => s("pycc_rt_str_rindex", &[str_], int),
        (Ty::Str, "replace", [Ty::Str, Ty::Str]) => {
            s("pycc_rt_str_replace", &[str_.clone(), str_.clone()], str_)
        }
        (Ty::Str, "upper", []) => s("pycc_rt_str_upper", &[], str_),
        (Ty::Str, "lower", []) => s("pycc_rt_str_lower", &[], str_),
        (Ty::Str, "swapcase", []) => s("pycc_rt_str_swapcase", &[], str_),
        (Ty::Str, "startswith", [Ty::Str]) => s("pycc_rt_str_startswith", &[str_], Ty::Bool),
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
        (Ty::List(_), "copy", []) => s("pycc_rt_list_copy", &[], receiver.clone()),
        (Ty::List(elem), "extend", [Ty::List(other)]) if elem == other => {
            s("pycc_rt_list_extend", &[receiver.clone()], Ty::None)
        }
        (Ty::List(elem), "count" | "index" | "remove", [item])
            if same_elem(item, elem) && value_comparable(elem) =>
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
        (Ty::Dict(_), "copy", []) => s("pycc_rt_dict_copy_of", &[], receiver.clone()),
        (Ty::Dict(_), "clear", []) => s("pycc_rt_dict_clear", &[], Ty::None),
        (Ty::Dict(_), "update", [other]) if other == receiver => {
            s("pycc_rt_dict_update", &[receiver.clone()], Ty::None)
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
                &[kv.1.clone()],
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
            &[receiver.clone()],
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
            s("pycc_rt_set_update", &[other.clone()], Ty::None)
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
            s(symbol, &[other.clone()], receiver.clone())
        }
        (set, "issubset" | "issuperset" | "isdisjoint", [other])
            if hashed_set(set).is_some() && hashed_set(other) == hashed_set(set) =>
        {
            let symbol = match method {
                "issubset" => "pycc_rt_set_issubset",
                "issuperset" => "pycc_rt_set_issuperset",
                _ => "pycc_rt_set_isdisjoint",
            };
            s(symbol, &[other.clone()], Ty::Bool)
        }
        _ => return None,
    };
    Some(Ok(found))
}
