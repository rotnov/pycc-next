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

mod byte_buffer;
mod conversions;
mod itertools;
mod methods;
mod object_calls;
mod object_defaultdict;
mod operators;

pub use object_defaultdict::object_defaultdict_source;
pub use operators::{list_inplace_operator, set_inplace_index, set_operator};

pub use byte_buffer::{BYTEARRAY_APPEND, BYTEARRAY_EXTEND, byte_buffer_method_call};
pub use itertools::{is_itertools_call, itertools_call};

/// Non-strict mode: the callee a `bytes` literal lowers to, with its bytes
/// as Latin-1 text (one code point per byte); CPython's own `bytes` object
/// (`pycc_ext_obj_bytes`).
pub const BYTES_LITERAL: &str = "__pycc_bytes";

/// Non-strict mode: `__pycc_exc_match(e, T)`, whether the caught exception
/// `e` matches the CPython object `T` an `except` clause names
/// (`pycc_ext_obj_exc_match`).
pub const EXC_MATCH: &str = "__pycc_exc_match";

/// Non-strict mode: `__pycc_exc_object(e)`, the caught exception `e` as the
/// CPython exception object it is (`pycc_ext_obj_exc_object`).
pub const EXC_OBJECT: &str = "__pycc_exc_object";
pub use methods::{materialized_method_args, resolve_method};

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
    /// Trailing `i64` arguments after the kind arguments: the value
    /// descriptor (`pycc_rt::itertools`) of each type -- a generator or list
    /// type describes an iterable source, any other type a value.
    pub desc_args: Vec<DescArg>,
}

/// One descriptor argument of a [`BuiltinSig`]: the shape a native
/// iterator (`pycc_rt::itertools`) is told about.
#[derive(Debug, Clone, PartialEq)]
pub enum DescArg {
    /// An iterable argument of this type (a list or a generator).
    Source(Ty),
    /// A value of this type.
    Value(Ty),
}

fn sig(symbol: &'static str, params: &[Ty], ret: Ty) -> BuiltinSig {
    BuiltinSig {
        symbol,
        params: params.to_vec(),
        ret,
        ret_word: false,
        word_args: vec![false; params.len()],
        kind_args: Vec::new(),
        desc_args: Vec::new(),
    }
}

/// `pycc_rt_num_isinstance(x, mask)`: whether the `int | float` `x` holds
/// an `int` (mask bit 1, a `bool` included), a `float` (bit 2) or a `bool`
/// (bit 4).
#[must_use]
pub fn num_isinstance_sig() -> BuiltinSig {
    sig("pycc_rt_num_isinstance", &[Ty::Num, Ty::Int], Ty::Bool)
}

/// The runtime function reading an `int | float` index as an `int`.
#[must_use]
pub fn num_index_sig() -> BuiltinSig {
    sig("pycc_rt_num_index", &[Ty::Num, Ty::Int], Ty::Int)
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
        desc_args: Vec::new(),
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
        Ty::Int | Ty::Float | Ty::Num | Ty::Bool | Ty::Str => true,
        Ty::List(elem) | Ty::VarTuple(elem) | Ty::Set(elem) | Ty::FrozenSet(elem) => {
            renders_natively(elem)
        }
        Ty::Tuple(elements) | Ty::ShapedList(elements) => elements.iter().all(renders_natively),
        Ty::Dict(kv) => renders_natively(&kv.0) && renders_natively(&kv.1),
        _ => false,
    }
}

fn is_intlike(ty: &Ty) -> bool {
    matches!(ty, Ty::Int | Ty::Bool)
}

fn is_numeric(ty: &Ty) -> bool {
    matches!(ty, Ty::Int | Ty::Bool | Ty::Float | Ty::Num)
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
        // An `int | float` argument converts as `float(x)` does.
        Ty::Float | Ty::Num => sig(float_symbol, &[Ty::Float], Ty::Float),
        a if is_intlike(a) => sig(int_symbol, &[Ty::Int], Ty::Float),
        _ => return None,
    })
}

/// Whether a list's elements are ordered by a natively supported `<`
/// (`pycc_rt::value::lt`, lexicographic for lists and tuples).
/// Whether values of `ty` are immutable builtin values that compare without
/// running any code and without raising: numbers, `bool`, `str` and tuples
/// of them.
fn inert(ty: &Ty) -> bool {
    match ty {
        Ty::Int | Ty::Bool | Ty::Float | Ty::Num | Ty::Str => true,
        Ty::Tuple(elements) => elements.iter().all(inert),
        Ty::VarTuple(inner) => inert(inner),
        _ => false,
    }
}

fn orderable(elem: &Ty) -> bool {
    match elem {
        Ty::Int | Ty::Bool | Ty::Float | Ty::Num | Ty::Str => true,
        Ty::List(inner) | Ty::VarTuple(inner) => orderable(inner),
        Ty::Tuple(elements) | Ty::ShapedList(elements) => elements.iter().all(orderable),
        _ => false,
    }
}

/// Whether two values of `elem` compare by value in the runtime
/// (`pycc_rt::value::eq`): everything but a user-class instance, which
/// compares by identity there.
#[must_use]
pub fn value_comparable(elem: &Ty) -> bool {
    match elem {
        // A CPython object element compares through CPython, which the
        // runtime's word equality cannot call.
        Ty::Instance(_) | Ty::Object => false,
        Ty::List(inner) | Ty::VarTuple(inner) | Ty::Set(inner) | Ty::FrozenSet(inner) => {
            value_comparable(inner)
        }
        Ty::Tuple(elements) | Ty::ShapedList(elements) => elements.iter().all(value_comparable),
        Ty::Dict(kv) => value_comparable(&kv.1),
        _ => true,
    }
}

/// Whether `item` can be compared for equality against a list of `elem`
/// by the runtime's element-word equality.
fn same_elem(item: &Ty, elem: &Ty) -> bool {
    item == elem || (is_intlike(item) && is_intlike(elem))
}

/// [`same_elem`], or any number searched for in a `list[int | float]`
/// (`xs.index(2.5)`): a lookup only borrows the item, so a converted
/// `float` word is released after the call rather than stored.
fn searches_elem(item: &Ty, elem: &Ty) -> bool {
    same_elem(item, elem)
        || (*elem == Ty::Num && matches!(item, Ty::Int | Ty::Bool | Ty::Float | Ty::Num))
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

/// Whether `ty` is a tuple compiled code holds by value (only `int`,
/// `float` and `bool` elements), which no runtime function returns as a
/// word.
fn unboxed_tuple(ty: &Ty) -> bool {
    matches!(ty, Ty::Tuple(elements) if !crate::tuple_is_boxed(elements))
}

/// Whether builtin `name` takes a `str` iterable argument as the list of its
/// characters (`zip`'s every argument, `enumerate`'s first): MIR lowering
/// converts it with `pycc_rt_str_to_list` before the call.
#[must_use]
pub fn iterates_str_as_list(name: &str) -> bool {
    matches!(name, "zip" | "enumerate" | "__pycc_counter_of") || is_itertools_call(name)
}

/// Whether argument `index` of builtin `name` (one
/// [`iterates_str_as_list`] claims) is an iterable a `str` is converted for.
#[must_use]
pub fn str_iterable_arg(name: &str, index: usize) -> bool {
    index == 0
        || name == "zip"
        || name == "__pycc_it_chain2"
        || name.starts_with("__pycc_it_product_")
}

/// Non-strict mode: whether `callee(args)` is `format(value, spec)` (or an
/// f-string's `{value:spec}`) of a CPython object, which is the object's
/// own `value.__format__(spec)`.
#[must_use]
pub fn is_object_format(callee: &str, args: &[Ty]) -> bool {
    matches!(callee, "format" | "__pycc_format")
        && matches!(args, [Ty::Object, Ty::Str | Ty::Object])
}

/// Every builtin name [`resolve_call`] claims for some argument types.
pub const CALL_NAMES: &[&str] = &[
    "__pycc_bytes",
    "__pycc_ba_append",
    "__pycc_ba_extend",
    "__pycc_exc_match",
    "__pycc_exc_object",
    "__pycc_obj_defaultdict",
    "__pycc_counter",
    "__pycc_counter_of",
    "__pycc_defaultdict",
    "__pycc_defaultdict_call",
    "__pycc_defaultdict_none",
    "__pycc_deque",
    "__pycc_format",
    "__pycc_bisect_left",
    "__pycc_bisect_right",
    "__pycc_heapify",
    "__pycc_heappop",
    "__pycc_heappush",
    "__pycc_heappushpop",
    "__pycc_heapreplace",
    "__pycc_insort_left",
    "__pycc_insort_right",
    "__pycc_mark_tuple",
    "__pycc_max_keyed",
    "__pycc_min_keyed",
    "__pycc_print_str",
    "__pycc_printf",
    "__pycc_chr",
    "__pycc_setrecursionlimit",
    "__pycc_sort_keyed",
    "__pycc_sorted_keyed",
    "__pycc_stdin_next",
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
    "format",
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
    "repr",
    "reversed",
    "set",
    "sorted",
    "str",
    "sum",
    "tuple",
    "zip",
];

/// Non-strict mode: a builtin whose one parameter takes a CPython object,
/// converted with a run-time type check: the parameter's type and the
/// subject of the conversion's `TypeError`, CPython's own wording
/// (`write() argument must be str, not int`).
#[must_use]
pub fn object_param(name: &str) -> Option<(Ty, &'static str)> {
    match name {
        "__pycc_stdout_write" => Some((Ty::Str, "write() argument")),
        _ => None,
    }
}

/// [`object_param`]'s `TypeError` subject by runtime symbol.
#[must_use]
pub fn object_param_subject(symbol: &str) -> Option<&'static str> {
    match symbol {
        "pycc_rt_stdout_write" => object_param("__pycc_stdout_write").map(|(_, what)| what),
        _ => None,
    }
}

/// The result type of builtin `name` whatever its (admitted) argument
/// types are, when it has one.
#[must_use]
pub fn fixed_return(name: &str) -> Option<Ty> {
    Some(match name {
        "input"
        | "str"
        | "repr"
        | "chr"
        | "__pycc_chr"
        | "bin"
        | "hex"
        | "oct"
        | "__pycc_stdin_readline"
        | "__pycc_stdin_next"
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
        "__pycc_print_str"
        | "__pycc_stdout_flush"
        | "__pycc_setrecursionlimit"
        | "__pycc_mark_tuple" => Ty::None,
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
    // `min(s)` / `max(s)` of one `str`: of its characters (`pycc_mir`
    // converts the argument to the list of them).
    if let ("min" | "max", [Ty::Str]) = (name, args) {
        return resolve_call(name, &[list_of(Ty::Str)]);
    }
    if let Some(found) = object_calls::object_call(name, args) {
        return Some(found);
    }
    if is_itertools_call(name) {
        return itertools_call(name, args);
    }
    if iterates_str_as_list(name) && args.contains(&Ty::Str) {
        let as_lists: Vec<Ty> = args
            .iter()
            .enumerate()
            .map(|(i, ty)| match ty {
                Ty::Str if str_iterable_arg(name, i) => list_of(Ty::Str),
                other => other.clone(),
            })
            .collect();
        return resolve_call(name, &as_lists);
    }
    if let Some(found) = conversions::conversion_call(name, args) {
        return Some(Ok(found));
    }
    let (int, float, str_) = (Ty::Int, Ty::Float, Ty::Str);
    let found = match (name, args) {
        ("input", []) => sig("pycc_rt_input", &[], str_),
        // The `sys` stream operations `pycc_ast::desugar` rewrites to.
        ("__pycc_stdin_readline", []) => sig("pycc_rt_stdin_readline", &[], str_),
        // `next(sys.stdin)`: a line, `StopIteration` at EOF.
        ("__pycc_stdin_next", []) => sig("pycc_rt_stdin_next", &[], str_),
        ("__pycc_stdin_read", []) => sig("pycc_rt_stdin_read", &[], str_),
        ("__pycc_stdin_readlines", []) => sig("pycc_rt_stdin_readlines", &[], list_of(str_)),
        ("__pycc_stdout_write", [Ty::Str]) => sig("pycc_rt_stdout_write", &[str_], int),
        ("__pycc_print_str", [Ty::Str]) => {
            sig("pycc_rt_print_write_str", std::slice::from_ref(&str_), Ty::None)
        }
        ("__pycc_stdout_flush", []) => sig("pycc_rt_print_flush", &[], Ty::None),
        // A function's `*args` list (`pycc_ast::desugar::signatures`).
        ("__pycc_mark_tuple", [Ty::List(_)]) => {
            sig("pycc_rt_list_mark_tuple", &[args[0].clone()], Ty::None)
        }
        // A list display of a fixed shape (`Ty::ShapedList`, erased to its
        // tuple): the tuple object flagged to be the list it is.
        ("__pycc_shaped_list", [t @ Ty::Tuple(elements)]) if crate::tuple_is_boxed(elements) => {
            sig("pycc_rt_tuple_mark_list", std::slice::from_ref(t), t.clone())
        }
        // `collections.Counter` / `defaultdict` (`pycc_ast::desugar::collections`).
        ("__pycc_counter", [d @ Ty::Dict(kv)]) if kv.1 == Ty::Int => {
            sig("pycc_rt_counter_new", std::slice::from_ref(d), d.clone())
        }
        ("__pycc_counter_of", [Ty::List(elem)]) if crate::is_hashable_ty(elem) => sig(
            "pycc_rt_counter_of_list",
            &[args[0].clone()],
            Ty::Dict(Box::new(((**elem).clone(), int.clone()))),
        ),
        ("__pycc_counter_of", [d @ Ty::Dict(kv)]) if kv.1 == Ty::Int => {
            sig("pycc_rt_counter_of_dict", std::slice::from_ref(d), d.clone())
        }
        ("__pycc_defaultdict", [d @ Ty::Dict(kv), proto, Ty::Str])
            if same_elem(proto, &kv.1) && crate::is_list_element_ty(&kv.1) =>
        {
            word_sig(
                "pycc_rt_defaultdict_new",
                &[d.clone(), kv.1.clone(), str_.clone()],
                &[false, true, false],
                d.clone(),
                false,
                &[],
            )
        }
        ("__pycc_defaultdict_call", [d @ Ty::Dict(kv), f @ Ty::Callable(sig_), Ty::Str])
            if sig_.0.is_empty() && sig_.1 == kv.1 && !unboxed_tuple(&kv.1) =>
        {
            sig("pycc_rt_defaultdict_call_new", &[d.clone(), f.clone(), str_.clone()], d.clone())
        }
        ("__pycc_defaultdict_none", [d @ Ty::Dict(_)]) => {
            sig("pycc_rt_defaultdict_none_new", std::slice::from_ref(d), d.clone())
        }
        // Non-strict mode: CPython's own `collections.defaultdict`, from its
        // factory's source (`object_defaultdict_source`).
        ("__pycc_obj_defaultdict", [Ty::Str]) => {
            sig("pycc_ext_obj_defaultdict", &[Ty::Object], Ty::Object)
        }
        // `collections.deque(items, maxlen, has_maxlen)`
        // (`pycc_ast::desugar::collections`).
        ("__pycc_deque", [list @ Ty::List(_), m, h]) if is_intlike(m) && is_intlike(h) => sig(
            "pycc_rt_deque_new",
            &[list.clone(), int.clone(), int.clone()],
            list.clone(),
        ),
        ("__pycc_setrecursionlimit", [a]) if is_intlike(a) => {
            sig("pycc_rt_setrecursionlimit", &[int], Ty::None)
        }
        ("input", [Ty::Str]) => {
            sig("pycc_rt_input_prompt", std::slice::from_ref(&str_), str_.clone())
        }
        ("int", [Ty::Str]) => sig("pycc_rt_int_from_str", &[str_], int),
        ("int", [Ty::Str, b]) if is_intlike(b) => {
            sig("pycc_rt_int_from_str_base", &[str_, int.clone()], int)
        }
        ("float", [Ty::Str]) => sig("pycc_rt_float_from_str", &[str_], float),
        // `format(value, spec)`; `__pycc_format` is an f-string's
        // `{value:spec}` (`expr::fstring`), which no user `format` shadows.
        ("format" | "__pycc_format", [Ty::Int, Ty::Str]) => {
            sig("pycc_rt_format_int", &[int, str_.clone()], str_)
        }
        ("format" | "__pycc_format", [Ty::Bool, Ty::Str]) => {
            sig("pycc_rt_format_bool", &[Ty::Bool, str_.clone()], str_)
        }
        ("format" | "__pycc_format", [Ty::Float, Ty::Str]) => {
            sig("pycc_rt_format_float", &[float, str_.clone()], str_)
        }
        ("format" | "__pycc_format", [Ty::Str, Ty::Str]) => {
            sig("pycc_rt_format_str", &[str_.clone(), str_.clone()], str_)
        }
        ("format" | "__pycc_format", [Ty::Num, Ty::Str]) => {
            sig("pycc_rt_format_num", &[Ty::Num, str_.clone()], str_)
        }
        ("format" | "str" | "repr", [Ty::Num]) => sig("pycc_rt_num_to_str", &[Ty::Num], str_),
        ("format", [Ty::Int]) => sig("pycc_rt_int_to_str", &[int], str_),
        ("format", [Ty::Float]) => sig("pycc_rt_float_to_str", &[float], str_),
        ("format", [Ty::Bool]) => sig("pycc_rt_bool_to_str", &[Ty::Bool], str_),
        ("format", [Ty::Str]) => sig("pycc_rt_str_copy", std::slice::from_ref(&str_), str_.clone()),
        // One conversion of `template % args` (`crate::printf`).
        ("__pycc_printf", [Ty::Int, Ty::Str]) => {
            sig("pycc_rt_printf_int", &[int, str_.clone()], str_)
        }
        ("__pycc_printf", [Ty::Bool, Ty::Str]) => {
            sig("pycc_rt_printf_bool", &[Ty::Bool, str_.clone()], str_)
        }
        ("__pycc_printf", [Ty::Float, Ty::Str]) => {
            sig("pycc_rt_printf_float", &[float, str_.clone()], str_)
        }
        ("__pycc_printf", [Ty::Str, Ty::Str]) => {
            sig("pycc_rt_printf_str", &[str_.clone(), str_.clone()], str_)
        }
        ("str", [Ty::Int]) => sig("pycc_rt_int_to_str", &[int], str_),
        ("str", [Ty::Float]) => sig("pycc_rt_float_to_str", &[float], str_),
        ("str", [Ty::Bool]) => sig("pycc_rt_bool_to_str", &[Ty::Bool], str_),
        ("str", [Ty::Str]) => sig("pycc_rt_str_copy", std::slice::from_ref(&str_), str_.clone()),
        ("str", []) => sig("pycc_rt_str_empty", &[], str_),
        ("str" | "repr", [t @ Ty::VarTuple(_)]) if renders_natively(t) => {
            sig("pycc_rt_list_repr", std::slice::from_ref(t), str_)
        }
        ("str", [list @ Ty::List(_)]) if renders_natively(list) => {
            sig("pycc_rt_list_repr", std::slice::from_ref(list), str_)
        }
        // A boxed tuple object renders as a tuple, or as a list when it is
        // one of a fixed shape (`Ty::ShapedList`, erased to its tuple).
        ("str" | "repr", [t @ (Ty::Tuple(elements) | Ty::ShapedList(elements))])
            if crate::tuple_is_boxed(elements) && renders_natively(t) =>
        {
            sig("pycc_rt_list_repr", std::slice::from_ref(t), str_)
        }
        ("str", [dict @ Ty::Dict(_)]) => sig("pycc_rt_dict_repr", std::slice::from_ref(dict), str_),
        // A pointer-represented `T | None`: `"None"` or `str(T)`.
        ("str", [opt @ Ty::Optional(inner)])
            if crate::is_pointer_optional_inner(inner) && renders_natively(inner) =>
        {
            let inner = (**inner).clone();
            word_sig("pycc_rt_optional_word_to_str", std::slice::from_ref(opt), &[true], str_, false, &[inner])
        }
        // `<function f at 0x...>` / `<generator object g at 0x...>`.
        ("str", [f @ Ty::Callable(_)]) => {
            sig("pycc_rt_closure_repr", std::slice::from_ref(f), str_)
        }
        ("str", [g @ Ty::Generator(_)]) => sig("pycc_rt_gen_repr", std::slice::from_ref(g), str_),
        // `repr(x)`: equal to `str(x)` for every type but `str` itself, whose
        // repr is quoted and escaped (a container's `str` is its repr).
        ("repr", [Ty::Int]) => sig("pycc_rt_int_to_str", &[int], str_),
        ("repr", [Ty::Float]) => sig("pycc_rt_float_to_str", &[float], str_),
        ("repr", [Ty::Bool]) => sig("pycc_rt_bool_to_str", &[Ty::Bool], str_),
        ("repr", [Ty::Str]) => sig("pycc_rt_str_repr", std::slice::from_ref(&str_), str_.clone()),
        ("repr", [list @ Ty::List(_)]) if renders_natively(list) => {
            sig("pycc_rt_list_repr", std::slice::from_ref(list), str_)
        }
        ("repr", [dict @ Ty::Dict(_)]) => {
            sig("pycc_rt_dict_repr", std::slice::from_ref(dict), str_)
        }
        ("repr", [f @ Ty::Callable(_)]) => {
            sig("pycc_rt_closure_repr", std::slice::from_ref(f), str_)
        }
        ("repr", [g @ Ty::Generator(_)]) => sig("pycc_rt_gen_repr", std::slice::from_ref(g), str_),
        ("len", [Ty::Str]) => sig("pycc_rt_str_len", &[str_], int),
        ("ord", [Ty::Str]) => sig("pycc_rt_str_ord", &[str_], int),
        // `__pycc_chr`: a `chr` call whose argument is a code point pycc's
        // `str` holds (`pycc_ast`'s `chr_ranges`).
        ("chr" | "__pycc_chr", [a]) if is_intlike(a) => sig("pycc_rt_str_chr", &[int], str_),
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
        ("math.isqrt", [a]) if is_intlike(a) => {
            sig("pycc_rt_math_isqrt", std::slice::from_ref(&int), int.clone())
        }
        ("math.floor", [Ty::Float]) => sig("pycc_rt_math_floor", &[float], int),
        ("math.ceil", [Ty::Float]) => sig("pycc_rt_math_ceil", &[float], int),
        ("math.floor", [Ty::Num]) => sig("pycc_rt_num_floor", &[Ty::Num], int),
        ("math.ceil", [Ty::Num]) => sig("pycc_rt_num_ceil", &[Ty::Num], int),
        ("math.floor" | "math.ceil", [a]) if is_intlike(a) => {
            sig("pycc_rt_math_int_identity", std::slice::from_ref(&int), int.clone())
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
            sig("pycc_rt_math_factorial", std::slice::from_ref(&int), int.clone())
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
        // An `int | float` result: `min(best, cost)` of a `float` sentinel
        // and an `int`, or with an `int | float` operand. CPython's `min`
        // returns the first of equal values, whichever type it has.
        ("min" | "max", [a, b])
            if is_numeric(a)
                && is_numeric(b)
                && (*a == Ty::Num || *b == Ty::Num || is_intlike(a) != is_intlike(b)) =>
        {
            let symbol = if name == "max" { "pycc_rt_num_max2" } else { "pycc_rt_num_min2" };
            sig(symbol, &[Ty::Num, Ty::Num], Ty::Num)
        }
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
            desc_args: Vec::new(),
        },
        ("sum", [Ty::List(elem)]) => match **elem {
            Ty::Int | Ty::Bool => sig("pycc_rt_list_sum_int", &[args[0].clone()], int),
            Ty::Float => sig("pycc_rt_list_sum_float", &[args[0].clone()], float),
            Ty::Num => sig("pycc_rt_list_sum_num", &[args[0].clone()], Ty::Num),
            _ => return None,
        },
        ("sorted", [Ty::List(elem)]) if orderable(elem) => {
            sig("pycc_rt_list_sorted", &[args[0].clone()], args[0].clone())
        }
        ("reversed", [Ty::List(_)]) => {
            sig("pycc_rt_list_reversed", &[args[0].clone()], args[0].clone())
        }
        ("sorted", [Ty::Str]) => {
            sig("pycc_rt_str_sorted", std::slice::from_ref(&str_), list_of(str_.clone()))
        }
        ("reversed", [Ty::Str]) => {
            sig("pycc_rt_str_reversed", std::slice::from_ref(&str_), list_of(str_.clone()))
        }
        ("any" | "all", [Ty::List(elem)]) if orderable(elem) => sig(
            if name == "any" {
                "pycc_rt_list_any"
            } else {
                "pycc_rt_list_all"
            },
            &[args[0].clone()],
            Ty::Bool,
        ),
        ("sorted", [set]) if hashed_set(set).is_some_and(orderable) => sig(
            "pycc_rt_set_sorted",
            std::slice::from_ref(set),
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
        // The same calls over a CPython object (whose pairs list is a
        // CPython list too), and `heapq`/`bisect` over one: the shim's
        // `object_stdlib.c` runs them in CPython. A native argument beside
        // the object is boxed (`pycc_mir::expr::builtin_call`).
        ("__pycc_sorted_keyed", [Ty::Object, Ty::Bool]) => sig(
            "pycc_ext_obj_sorted_keyed",
            &[Ty::Object, Ty::Bool],
            Ty::Object,
        ),
        ("__pycc_sort_keyed", [Ty::Object, Ty::Object, Ty::Bool]) => sig(
            "pycc_ext_obj_sort_keyed",
            &[Ty::Object, Ty::Object, Ty::Bool],
            Ty::None,
        ),
        ("__pycc_heapify", [Ty::Object]) => sig("pycc_ext_obj_heapify", &[Ty::Object], Ty::None),
        ("__pycc_heappop", [Ty::Object]) => sig("pycc_ext_obj_heappop", &[Ty::Object], Ty::Object),
        (
            "__pycc_heappush"
            | "__pycc_heapreplace"
            | "__pycc_heappushpop"
            | "__pycc_bisect_left"
            | "__pycc_bisect_right"
            | "__pycc_insort_left"
            | "__pycc_insort_right",
            [Ty::Object, item],
        ) if boxes_as_object(item) => {
            let (symbol, ret) = match name {
                "__pycc_heappush" => ("pycc_ext_obj_heappush", Ty::None),
                "__pycc_heapreplace" => ("pycc_ext_obj_heapreplace", Ty::Object),
                "__pycc_heappushpop" => ("pycc_ext_obj_heappushpop", Ty::Object),
                "__pycc_bisect_left" => ("pycc_ext_obj_bisect_left", Ty::Object),
                "__pycc_bisect_right" => ("pycc_ext_obj_bisect_right", Ty::Object),
                "__pycc_insort_left" => ("pycc_ext_obj_insort_left", Ty::None),
                _ => ("pycc_ext_obj_insort_right", Ty::None),
            };
            sig(symbol, &[Ty::Object, Ty::Object], ret)
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
                std::slice::from_ref(elem),
            )
        }
        ("__pycc_sort_keyed", [Ty::List(target), Ty::List(pair), Ty::Bool]) => {
            let (key, elem) = keyed_pair(pair)?;
            if !orderable(key) || elem != target.as_ref() {
                return None;
            }
            sig("pycc_rt_list_sort_keyed", args, Ty::None)
        }
        // CPython's `min`/`max` compare each key as soon as it is computed;
        // the pairs list computes every key first, which nothing can see only
        // when comparing keys runs no code and a key cannot change after it
        // is made: an immutable builtin value.
        ("__pycc_min_keyed" | "__pycc_max_keyed", [Ty::List(pair)]) => {
            let (key, elem) = keyed_pair(pair)?;
            if !inert(key) {
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
                std::slice::from_ref(elem),
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
            desc_args: Vec::new(),
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

/// Whether a native value of type `ty` crosses into a CPython call as the
/// object CPython would hold (`pycc_codegen::object_box`): a scalar, `None`,
/// a CPython object, or a tuple of `int`/`float`/`bool`.
pub(super) fn boxes_as_object(ty: &Ty) -> bool {
    match ty {
        Ty::Int | Ty::Float | Ty::Bool | Ty::Str | Ty::None | Ty::Object => true,
        Ty::Tuple(elements) => {
            !elements.is_empty()
                && elements
                    .iter()
                    .all(|e| matches!(e, Ty::Int | Ty::Float | Ty::Bool))
        }
        _ => false,
    }
}
