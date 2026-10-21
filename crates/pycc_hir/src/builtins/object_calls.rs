//! Non-strict mode: builtins over CPython objects that run CPython's own
//! implementation (`src/ext/module/object_stdlib.c`): `min`/`max`, `list`,
//! `sorted`, `sum`, `next`, `tuple`, `iter`, `enumerate`, `range` as a
//! value, and `itertools.accumulate`/`chain` with an object argument.
//! `pycc_types` admits the ones that take a native argument in non-strict
//! mode only (`builtin_call`, `expr::call_infer`).

use super::{BuiltinSig, boxes_as_object, is_intlike, sig};
use crate::Ty;

/// `pycc_ast::desugar::signatures::MARK_TUPLE`.
const MARK_TUPLE: &str = "__pycc_mark_tuple";

/// `pycc_ast::desugar::intrinsics::HTML_ESCAPE`.
const HTML_ESCAPE: &str = "__pycc_html_escape";
/// `pycc_ast::desugar::intrinsics::RE_ESCAPE`.
const RE_ESCAPE: &str = "__pycc_re_escape";

/// The CPython-backed signature of builtin `name` over `args`, when one of
/// the arms below claims the call.
pub(super) fn object_call(name: &str, args: &[Ty]) -> Option<Result<BuiltinSig, String>> {
    // A `bytes` literal (`super::BYTES_LITERAL`): its Latin-1 text encoded.
    if let (super::BYTES_LITERAL, [Ty::Str]) = (name, args) {
        return Some(Ok(sig("pycc_ext_obj_bytes", &[Ty::Object], Ty::Object)));
    }
    // A byte buffer's `append` of an `int` and `extend` by a native list of
    // them (`crate::ndarray`): stored straight into the `bytearray`.
    if let (super::BYTEARRAY_APPEND, [Ty::Object, Ty::Int]) = (name, args) {
        return Some(Ok(sig("pycc_ext_obj_ba_append", &[Ty::Object, Ty::Int], Ty::Object)));
    }
    if let (super::BYTEARRAY_EXTEND, [Ty::Object, list @ Ty::List(element)]) = (name, args)
        && **element == Ty::Int
    {
        return Some(Ok(sig("pycc_ext_obj_ba_extend", &[Ty::Object, list.clone()], Ty::Object)));
    }
    // An `except` clause whose type is a CPython object
    // (`crate::stmt::exception`'s object handlers): the caught exception
    // matched against it, and the CPython exception it is.
    if let (super::EXC_MATCH, [exc @ Ty::Instance(_), Ty::Object]) = (name, args) {
        return Some(Ok(sig("pycc_ext_obj_exc_match", &[exc.clone(), Ty::Object], Ty::Bool)));
    }
    if let (super::EXC_OBJECT, [exc @ Ty::Instance(_)]) = (name, args) {
        return Some(Ok(sig("pycc_ext_obj_exc_object", std::slice::from_ref(exc), Ty::Object)));
    }
    // v0.9: a function's `*args` that is a CPython object (an unannotated
    // vararg of a published function): `pycc_mir` rebinds it to
    // `tuple(args)`, so a call from compiled code (which passes a list)
    // hands the body a tuple, as CPython does.
    // v0.9: `html.escape(s, quote)` and `re.escape(s)` by a callee that may
    // be CPython's own (`pycc_ast::desugar::intrinsics`): computed natively
    // when it is.
    if let (HTML_ESCAPE, [Ty::Object, s, q, Ty::Int]) = (name, args)
        && boxes_as_object(s)
        && boxes_as_object(q)
    {
        return Some(Ok(sig(
            "pycc_ext_obj_html_escape",
            &[Ty::Object, Ty::Object, Ty::Object, Ty::Int],
            Ty::Object,
        )));
    }
    if let (RE_ESCAPE, [Ty::Object, s]) = (name, args)
        && boxes_as_object(s)
    {
        return Some(Ok(sig("pycc_ext_obj_re_escape", &[Ty::Object, Ty::Object], Ty::Object)));
    }
    if let (MARK_TUPLE, [Ty::Object]) = (name, args) {
        return Some(Ok(sig("pycc_ext_obj_tuple", &[Ty::Object], Ty::None)));
    }
    // Non-strict mode: `itertools.accumulate(xs)` over a CPython object runs
    // CPython's own, its result a CPython iterator.
    if let ("__pycc_it_accumulate_add", [Ty::Object, code]) = (name, args)
        && is_intlike(code)
    {
        return Some(Ok(sig(
            "pycc_ext_obj_accumulate",
            &[Ty::Object, Ty::Object],
            Ty::Object,
        )));
    }
    // Non-strict mode: `min`/`max` of a CPython object, or of a generator
    // of objects (crossing as a CPython iterator), run CPython's own.
    if let ("min" | "max", [arg]) = (name, args)
        && (*arg == Ty::Object || matches!(arg, Ty::Generator(g) if g.0 == Ty::Object))
    {
        let symbol = if name == "min" { "pycc_ext_obj_min" } else { "pycc_ext_obj_max" };
        return Some(Ok(sig(symbol, &[Ty::Object], Ty::Object)));
    }
    // Non-strict mode: `tuple(xs)` of a CPython object or of a list of
    // `bool`s (copied, which nothing can observe: the tuple is immutable
    // and its items are) is CPython's own tuple -- a list of other
    // elements makes a native one (`conversions`) -- and `iter(xs)`
    // CPython's own iterator (over a list display only: `pycc_types`);
    // both are refused in strict mode.
    if let ("tuple" | "iter", [arg]) = (name, args)
        && (*arg == Ty::Object
            || matches!(arg, Ty::List(e)
                if packs_immutably(e) && (name == "iter" || **e == Ty::Bool)))
    {
        let symbol = if name == "tuple" { "pycc_ext_obj_tuple" } else { "pycc_ext_obj_iter" };
        return Some(Ok(sig(symbol, &[Ty::Object], Ty::Object)));
    }
    // Non-strict mode: `list`, `dict`, `set`, `sorted`, `sum` and `next` of
    // a CPython object run CPython's own, the result a CPython object.
    if let ("list" | "dict" | "set" | "sorted" | "sum" | "next", [Ty::Object]) = (name, args) {
        let symbol = match name {
            "list" => "pycc_ext_obj_list",
            "dict" => "pycc_ext_obj_dict",
            "set" => "pycc_ext_obj_set",
            "sorted" => "pycc_ext_obj_sorted",
            "next" => "pycc_ext_obj_next",
            _ => "pycc_ext_obj_sum",
        };
        return Some(Ok(sig(symbol, &[Ty::Object], Ty::Object)));
    }
    // Non-strict mode: a `range` used as a value (not a loop's iterable,
    // which every loop and comprehension compiles natively) is CPython's
    // own range object; `pycc_types` refuses it in strict mode.
    if name == "range" && (1..=3).contains(&args.len()) && args.iter().all(is_intlike) {
        let symbol = match args.len() {
            1 => "pycc_ext_obj_range1",
            2 => "pycc_ext_obj_range2",
            _ => "pycc_ext_obj_range3",
        };
        return Some(Ok(sig(symbol, &vec![Ty::Object; args.len()], Ty::Object)));
    }
    // `enumerate(o[, start])` of a CPython object: CPython's own iterator.
    match (name, args) {
        ("enumerate", [Ty::Object]) => {
            return Some(Ok(sig("pycc_ext_obj_enumerate", &[Ty::Object], Ty::Object)));
        }
        ("enumerate", [Ty::Object, start]) if is_intlike(start) => {
            return Some(Ok(sig(
                "pycc_ext_obj_enumerate_from",
                &[Ty::Object, Ty::Object],
                Ty::Object,
            )));
        }
        _ => {}
    }
    // Non-strict mode: `itertools.chain(a, b)` with a CPython object
    // argument runs CPython's own over it and a list of immutable values
    // (copied: `pycc_types` admits only a list display or comprehension
    // there, which nothing else holds).
    if let ("__pycc_it_chain2", [a, b]) = (name, args)
        && (*a == Ty::Object || *b == Ty::Object)
        && [a, b]
            .iter()
            .all(|t| **t == Ty::Object || matches!(t, Ty::List(e) if packs_immutably(e)))
    {
        return Some(Ok(sig("pycc_ext_obj_chain2", &[Ty::Object, Ty::Object], Ty::Object)));
    }
    // ... and `min(a, b)` / `max(a, b)` with a CPython object operand
    // (the other one boxed).
    if let ("min" | "max", [a, b]) = (name, args)
        && (*a == Ty::Object || *b == Ty::Object)
        && boxes_as_object(a)
        && boxes_as_object(b)
    {
        let symbol = if name == "min" { "pycc_ext_obj_min2" } else { "pycc_ext_obj_max2" };
        return Some(Ok(sig(symbol, &[Ty::Object, Ty::Object], Ty::Object)));
    }
    None
}

/// Whether a list element of type `ty` is copied into CPython with nothing
/// to alias: a scalar, or a tuple of scalars.
fn packs_immutably(ty: &Ty) -> bool {
    match ty {
        Ty::Int | Ty::Float | Ty::Bool | Ty::Str => true,
        Ty::Tuple(items) => !items.is_empty() && items.iter().all(packs_immutably),
        _ => false,
    }
}
