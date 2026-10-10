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
}

fn sig(symbol: &'static str, params: &[Ty], ret: Ty) -> BuiltinSig {
    BuiltinSig {
        symbol,
        params: params.to_vec(),
        ret,
        ret_word: false,
        word_args: vec![false; params.len()],
    }
}

/// Whether `pycc_rt_list_repr` renders `ty` itself (a class instance needs
/// its `__repr__`, which the runtime cannot call).
fn renders_natively(ty: &Ty) -> bool {
    match ty {
        Ty::Int | Ty::Float | Ty::Bool | Ty::Str => true,
        Ty::List(elem) => renders_natively(elem),
        _ => false,
    }
}

fn is_intlike(ty: &Ty) -> bool {
    matches!(ty, Ty::Int | Ty::Bool)
}

/// Whether a list's elements are ordered by a natively supported `<`.
fn orderable(elem: &Ty) -> bool {
    matches!(elem, Ty::Int | Ty::Bool | Ty::Float | Ty::Str)
}

/// Whether `item` can be compared for equality against a list of `elem`
/// by the runtime's element-word equality.
fn same_elem(item: &Ty, elem: &Ty) -> bool {
    item == elem || (is_intlike(item) && is_intlike(elem))
}

fn list_of(elem: Ty) -> Ty {
    Ty::List(Box::new(elem))
}

/// Every builtin name [`resolve_call`] claims for some argument types.
pub const CALL_NAMES: &[&str] = &[
    "__pycc_print_str",
    "__pycc_setrecursionlimit",
    "__pycc_stdin_read",
    "__pycc_stdin_readline",
    "__pycc_stdin_readlines",
    "__pycc_stdout_flush",
    "__pycc_stdout_write",
    "all",
    "any",
    "bin",
    "chr",
    "float",
    "hex",
    "input",
    "int",
    "len",
    "list",
    "math.ceil",
    "math.floor",
    "math.gcd",
    "math.isqrt",
    "math.lcm",
    "max",
    "min",
    "oct",
    "ord",
    "pow",
    "reversed",
    "sorted",
    "str",
    "sum",
];

/// The result type of builtin `name` whatever its (admitted) argument
/// types are, when it has one.
#[must_use]
pub fn fixed_return(name: &str) -> Option<Ty> {
    Some(match name {
        "input" | "str" | "chr" | "bin" | "hex" | "oct" | "__pycc_stdin_readline"
        | "__pycc_stdin_read" => Ty::Str,
        "int" | "len" | "ord" | "pow" | "__pycc_stdout_write" | "math.gcd" | "math.lcm"
        | "math.isqrt" | "math.floor" | "math.ceil" => Ty::Int,
        "float" => Ty::Float,
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
    let (int, float, str_) = (Ty::Int, Ty::Float, Ty::Str);
    let found = match (name, args) {
        ("input", []) => sig("pycc_rt_input", &[], str_),
        // The `sys` stream operations `pycc_ast::desugar` rewrites to.
        ("__pycc_stdin_readline", []) => sig("pycc_rt_stdin_readline", &[], str_),
        ("__pycc_stdin_read", []) => sig("pycc_rt_stdin_read", &[], str_),
        ("__pycc_stdin_readlines", []) => {
            sig("pycc_rt_stdin_readlines", &[], list_of(str_))
        }
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
        ("pow", [a, b, c]) if is_intlike(a) && is_intlike(b) && is_intlike(c) => sig(
            "pycc_rt_int_powmod",
            &[int.clone(), int.clone(), int.clone()],
            int,
        ),
        ("min" | "max", [a, b]) if a == b || (is_intlike(a) && is_intlike(b)) => {
            let max = name == "max";
            let (symbol, ty) = match a {
                Ty::Int | Ty::Bool => (
                    if max { "pycc_rt_int_max2" } else { "pycc_rt_int_min2" },
                    int,
                ),
                Ty::Float => (
                    if max { "pycc_rt_float_max2" } else { "pycc_rt_float_min2" },
                    float,
                ),
                Ty::Str => (
                    if max { "pycc_rt_str_max2" } else { "pycc_rt_str_min2" },
                    str_,
                ),
                _ => return None,
            };
            let mut found = sig(symbol, &[ty.clone(), ty.clone()], ty);
            // `max(True, False)` is the `bool` `True`, not the int `1`.
            if a == b && *a == Ty::Bool {
                found.ret = Ty::Bool;
                found.params = vec![Ty::Int, Ty::Int];
                found.symbol = if max { "pycc_rt_bool_max2" } else { "pycc_rt_bool_min2" };
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
        _ => return None,
    };
    Some(Ok(found))
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
        (Ty::Str, "split", [Ty::Str, m]) if is_intlike(m) => s(
            "pycc_rt_str_split_max",
            &[str_.clone(), int],
            list_of(str_),
        ),
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
            if same_elem(item, elem) && orderable(elem) =>
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
        _ => return None,
    };
    Some(Ok(found))
}
