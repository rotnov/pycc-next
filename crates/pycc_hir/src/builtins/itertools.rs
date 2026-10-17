//! The native `itertools` functions (`pycc_rt::itertools`).
//!
//! `pycc_ast`'s `desugar::itertools` rewrites a call such as
//! `itertools.permutations(xs, 2)` into `__pycc_it_permutations_2(xs, 2)`:
//! what the result type depends on beyond the argument types (a tuple's
//! arity, `accumulate`'s function) is spelled in the name, and the runtime
//! receives it again as an ordinary argument. Each call returns a generator
//! (`Ty::Generator`) over values of a type a container can hold, so every
//! generator consumer takes the result unchanged.

use super::{BuiltinSig, DescArg, is_intlike, word_sig};
use crate::Ty;

/// The prefix of every native `itertools` builtin name.
const PREFIX: &str = "__pycc_it_";

/// Whether `name` is a native `itertools` builtin.
#[must_use]
pub fn is_itertools_call(name: &str) -> bool {
    name.starts_with(PREFIX)
}

/// The most elements a tuple produced natively may have (four bits of a
/// value descriptor each).
const MAX_ARITY: usize = 12;

/// The values an iterable argument yields: a list's or a generator's
/// elements (a `str` arrives as the list of its characters).
fn element(ty: &Ty) -> Option<Ty> {
    match ty {
        Ty::List(elem) => Some((**elem).clone()),
        Ty::Generator(sig) => Some(sig.0.clone()),
        Ty::Str => Some(Ty::Str),
        _ => None,
    }
}

/// The source descriptor argument of an iterable argument of type `ty`.
fn source(ty: &Ty) -> DescArg {
    match ty {
        Ty::Str => DescArg::Source(Ty::List(Box::new(Ty::Str))),
        other => DescArg::Source(other.clone()),
    }
}

/// Whether native iterators can carry a value of `ty`: a container element
/// type, a tuple of at most [`MAX_ARITY`] such elements.
fn carried(ty: &Ty) -> bool {
    match ty {
        Ty::Tuple(elements) => {
            elements.len() <= MAX_ARITY && crate::container::is_list_element_ty(ty)
        }
        other => crate::container::is_list_element_ty(other),
    }
}

/// An iterable argument's element type when native iterators carry it.
fn carried_element(ty: &Ty) -> Option<Ty> {
    element(ty).filter(carried)
}

fn tuple(elements: Vec<Ty>) -> Ty {
    Ty::Tuple(Box::new(elements))
}

/// A call of runtime `symbol` over `params` yielding `out` values, with the
/// given word-argument flags and descriptor arguments (`out`'s own value
/// descriptor is appended last).
fn iterator(
    symbol: &'static str,
    params: &[Ty],
    words: &[bool],
    mut descs: Vec<DescArg>,
    out: Ty,
) -> BuiltinSig {
    descs.push(DescArg::Value(out.clone()));
    let mut found = word_sig(symbol, params, words, Ty::generator(out), false, &[]);
    found.desc_args = descs;
    found
}

/// `accumulate`'s binary function codes (`pycc_rt::itertools::lazy`).
fn accumulate_op(name: &str) -> Option<i64> {
    Some(match name {
        "add" => 0,
        "mul" => 1,
        "sub" => 2,
        "max" => 3,
        "min" => 4,
        "and" => 5,
        "or" => 6,
        "xor" => 7,
        _ => return None,
    })
}

/// Resolves a native `itertools` call (see [`is_itertools_call`]): `None`
/// for a name that is not one, `Some(Err)` for argument types the native
/// iterators do not take.
#[must_use]
pub fn itertools_call(name: &str, args: &[Ty]) -> Option<Result<BuiltinSig, String>> {
    let rest = name.strip_prefix(PREFIX)?;
    let refused = |what: &str| Some(Err(what.to_string()));
    let int = Ty::Int;
    match (rest, args) {
        ("count", [start, step]) if is_intlike(start) && is_intlike(step) => Some(Ok(iterator(
            "pycc_rt_it_count_int",
            &[int.clone(), int.clone()],
            &[false, false],
            Vec::new(),
            int,
        ))),
        ("count", [Ty::Float, step]) if *step == Ty::Float || is_intlike(step) => {
            Some(Ok(iterator(
                "pycc_rt_it_count_float",
                &[Ty::Float, Ty::Float],
                &[false, false],
                Vec::new(),
                Ty::Float,
            )))
        }
        ("count", _) => {
            refused("itertools.count() is compiled for an int start and step, or a float start")
        }
        ("range", [start, stop, step]) if [start, stop, step].iter().all(|t| is_intlike(t)) => {
            Some(Ok(iterator(
                "pycc_rt_it_range",
                &[int.clone(), int.clone(), int.clone()],
                &[false; 3],
                Vec::new(),
                int,
            )))
        }
        ("islice", [it, start, stop, step, has_stop])
            if [start, stop, step, has_stop].iter().all(|t| is_intlike(t)) =>
        {
            let Some(elem) = carried_element(it) else {
                return refused("itertools.islice() is compiled over a list, str or generator");
            };
            Some(Ok(iterator(
                "pycc_rt_it_islice",
                &[source_param(it), int.clone(), int.clone(), int.clone(), int],
                &[false; 5],
                vec![source(it)],
                elem,
            )))
        }
        ("islice", _) => {
            refused("itertools.islice() is compiled with int (or None) start, stop and step")
        }
        ("chain1", [a]) => {
            let elem = carried_element(a)?;
            Some(Ok(iterator(
                "pycc_rt_it_chain1",
                &[source_param(a)],
                &[false],
                vec![source(a)],
                elem,
            )))
        }
        ("chain2", [a, b]) => match (carried_element(a), carried_element(b)) {
            (Some(x), Some(y)) if x == y => Some(Ok(iterator(
                "pycc_rt_it_chain2",
                &[source_param(a), source_param(b)],
                &[false, false],
                vec![source(a), source(b)],
                x,
            ))),
            (Some(x), Some(y)) => Some(Err(format!(
                "itertools.chain() over iterables of `{}` and `{}` is not compiled; \
                 every iterable must yield one type",
                x.name(),
                y.name()
            ))),
            _ => refused("itertools.chain() is compiled over lists, strs and generators"),
        },
        ("chain_from", [it]) => {
            let inner = carried_element(it).and_then(|inner| match inner {
                Ty::Str => Some((Ty::List(Box::new(Ty::Str)), Ty::Str)),
                Ty::List(elem) => Some((Ty::List(elem.clone()), *elem)),
                _ => None,
            });
            let Some((inner, elem)) = inner.filter(|(_, elem)| carried(elem)) else {
                return refused(
                    "itertools.chain.from_iterable() is compiled over an iterable of lists or strs",
                );
            };
            Some(Ok(iterator(
                "pycc_rt_it_chain_from",
                &[source_param(it)],
                &[false],
                vec![source(it), DescArg::Source(inner)],
                elem,
            )))
        }
        (accumulate, [it, op, rest @ ..]) if accumulate.starts_with("accumulate_") => {
            let spec = &accumulate["accumulate_".len()..];
            let (op_name, initial) = match spec.strip_suffix("_initial") {
                Some(op_name) => (op_name, true),
                None => (spec, false),
            };
            let code = accumulate_op(op_name)?;
            let elem = element(it)?;
            let admitted = match &elem {
                Ty::Int => true,
                Ty::Float => matches!(code, 0..=4),
                Ty::Str => matches!(code, 0 | 3 | 4),
                _ => false,
            };
            if !admitted || !is_intlike(op) {
                return Some(Err(format!(
                    "itertools.accumulate() of `{}` values with `{op_name}` is not compiled",
                    elem.name()
                )));
            }
            match (initial, rest) {
                (false, []) => Some(Ok(iterator(
                    "pycc_rt_it_accumulate",
                    &[source_param(it), Ty::Int],
                    &[false, false],
                    vec![source(it)],
                    elem,
                ))),
                (true, [init]) if *init == elem => Some(Ok(iterator(
                    "pycc_rt_it_accumulate_initial",
                    &[source_param(it), Ty::Int, elem.clone()],
                    &[false, false, true],
                    vec![source(it)],
                    elem,
                ))),
                (true, [init]) => Some(Err(format!(
                    "the initial value of itertools.accumulate() must be a `{}`, got `{}`",
                    elem.name(),
                    init.name()
                ))),
                _ => None,
            }
        }
        ("pairwise", [it]) => {
            let elem = carried_element(it)
                .filter(|elem| carried(&tuple(vec![elem.clone(), elem.clone()])))?;
            Some(Ok(iterator(
                "pycc_rt_it_pairwise",
                &[source_param(it)],
                &[false],
                vec![source(it)],
                tuple(vec![elem.clone(), elem]),
            )))
        }
        // `takewhile`/`dropwhile` over `(predicate value, element)` pairs
        // (`pycc_ast` rewrites the predicate into the pair's first field).
        ("while", [it, mode]) if is_intlike(mode) => {
            let Some(Ty::Tuple(pair)) = carried_element(it) else {
                return None;
            };
            let [Ty::Bool, elem] = pair.as_slice() else {
                return None;
            };
            Some(Ok(iterator(
                "pycc_rt_it_while",
                &[source_param(it), Ty::Int],
                &[false, false],
                vec![source(it)],
                elem.clone(),
            )))
        }
        ("groupby" | "groupby_keyed", [it, keyed]) if is_intlike(keyed) => {
            let elem = carried_element(it)?;
            let (key, value) = if rest == "groupby_keyed" {
                match elem {
                    Ty::Tuple(pair) if pair.len() == 2 => (pair[0].clone(), pair[1].clone()),
                    _ => return None,
                }
            } else {
                (elem.clone(), elem)
            };
            if !super::value_comparable(&key) {
                return Some(Err(format!(
                    "itertools.groupby() with keys of type `{}` is not compiled",
                    key.name()
                )));
            }
            let out = tuple(vec![key, Ty::List(Box::new(value))]);
            if !carried(&out) {
                return refused("itertools.groupby() over these values is not compiled");
            }
            Some(Ok(iterator(
                "pycc_rt_it_groupby",
                &[source_param(it), Ty::Int],
                &[false, false],
                vec![source(it)],
                out,
            )))
        }
        (combinatoric, _) => combinatoric_call(combinatoric, args),
    }
}

/// The parameter type an iterable argument passes as (a `str` is converted
/// to a list before the call).
fn source_param(ty: &Ty) -> Ty {
    match ty {
        Ty::Str => Ty::List(Box::new(Ty::Str)),
        other => other.clone(),
    }
}

/// `product_<repeat>`, `permutations_<r>`, `combinations_<r>` and
/// `cwr_<r>` (`combinations_with_replacement`).
fn combinatoric_call(rest: &str, args: &[Ty]) -> Option<Result<BuiltinSig, String>> {
    let (family, count) = rest.rsplit_once('_')?;
    if count == "r" {
        return Some(Err(format!(
            "itertools.{}() is compiled only with `r` given as an int literal \
             (the length of the tuples it yields is part of their type)",
            match family {
                "cwr" => "combinations_with_replacement",
                other => other,
            }
        )));
    }
    let count: usize = count.parse().ok()?;
    if family == "product" {
        let (iterables, [repeat]) = args.split_at(args.len().checked_sub(1)?) else {
            return None;
        };
        if !is_intlike(repeat) || iterables.is_empty() || iterables.len() > 4 {
            return None;
        }
        let elems = iterables
            .iter()
            .map(carried_element)
            .collect::<Option<Vec<_>>>()?;
        let out: Vec<Ty> = (0..count).flat_map(|_| elems.iter().cloned()).collect();
        if out.is_empty() || out.len() > MAX_ARITY {
            return Some(Err(format!(
                "itertools.product() yielding tuples of {} elements is not compiled (1 to {MAX_ARITY} are)",
                out.len()
            )));
        }
        let symbol = match iterables.len() {
            1 => "pycc_rt_it_product1",
            2 => "pycc_rt_it_product2",
            3 => "pycc_rt_it_product3",
            _ => "pycc_rt_it_product4",
        };
        let mut params: Vec<Ty> = iterables.iter().map(source_param).collect();
        params.push(Ty::Int);
        let words = vec![false; params.len()];
        return Some(Ok(iterator(
            symbol,
            &params,
            &words,
            iterables.iter().map(source).collect(),
            tuple(out),
        )));
    }
    let symbol = match family {
        "permutations" => "pycc_rt_it_permutations",
        "combinations" => "pycc_rt_it_combinations",
        "cwr" => "pycc_rt_it_combinations_with_replacement",
        _ => return None,
    };
    let [it, r] = args else {
        return None;
    };
    let elem = carried_element(it)?;
    if !is_intlike(r) || count == 0 || count > MAX_ARITY {
        return Some(Err(format!(
            "tuples of {count} elements from itertools are not compiled (1 to {MAX_ARITY} are)"
        )));
    }
    Some(Ok(iterator(
        symbol,
        &[source_param(it), Ty::Int],
        &[false, false],
        vec![source(it)],
        tuple(vec![elem; count]),
    )))
}
