//! Lowering of the builtin calls `pycc_hir::builtins` claims into
//! [`MirExpr::RtCall`].

use crate::{MirExpr, RtCallExpr};
use pycc_hir::Ty;
use pycc_hir::builtins::{BuiltinSig, is_pairwise_variadic, resolve_call, resolve_method};

/// `name(args)` as a runtime call, or `args` back when the table does not
/// claim the call. `min`/`max` over three or more arguments fold left,
/// which keeps CPython's evaluation order and its first-wins tie rule.
pub(super) fn lower_builtin_call(name: &str, args: Vec<MirExpr>) -> Result<MirExpr, Vec<MirExpr>> {
    let args = if pycc_hir::builtins::iterates_str_as_list(name) {
        args.into_iter()
            .enumerate()
            .map(|(i, arg)| {
                if arg.ty() == Ty::Str && pycc_hir::builtins::str_iterable_arg(name, i) {
                    crate::str_to_list(arg)
                } else {
                    arg
                }
            })
            .collect()
    } else {
        args
    };
    if is_pairwise_variadic(name) && args.len() > 2 {
        let tys: Vec<_> = args.iter().map(MirExpr::ty).collect();
        if !matches!(resolve_call(name, &tys[..2]), Some(Ok(_))) {
            return Err(args);
        }
        let mut iter = args.into_iter();
        let mut acc = iter.next().expect("more than two arguments");
        for next in iter {
            let pair = vec![acc, next];
            acc = match lower_builtin_call(name, pair) {
                Ok(call) => call,
                Err(_) => panic!(
                    "pycc_mir: internal error: `{name}` folds pairwise only over arguments \
                     pycc_types admitted"
                ),
            };
        }
        return Ok(acc);
    }
    let mut tys: Vec<_> = args.iter().map(MirExpr::ty).collect();
    // A CPython object for the one scalar parameter `pycc_types` admitted
    // (non-strict mode): codegen converts it (`object_coerce`).
    if let ([Ty::Object], Some((param, _))) =
        (tys.as_slice(), pycc_hir::builtins::object_param(name))
    {
        tys = vec![param];
    }
    match resolve_call(name, &tys) {
        Some(Ok(sig)) => {
            // A shim helper over a CPython object takes every argument as
            // an object: a native one is boxed.
            let args = if sig.symbol.starts_with("pycc_ext_obj_") {
                args.into_iter()
                    .zip(&sig.params)
                    .map(|(arg, param)| {
                        if *param == Ty::Object && arg.ty() != Ty::Object {
                            MirExpr::ObjectBox(Box::new(arg))
                        } else {
                            arg
                        }
                    })
                    .collect()
            } else {
                args
            };
            Ok(MirExpr::RtCall(Box::new(RtCallExpr { sig, args })))
        }
        _ => Err(args),
    }
}

/// `base.method(args)` as a runtime call with `base` as the first argument.
pub(crate) fn lower_builtin_method(
    base: MirExpr,
    method: &str,
    args: Vec<MirExpr>,
) -> Result<MirExpr, (MirExpr, Vec<MirExpr>)> {
    let mut tys: Vec<_> = args.iter().map(MirExpr::ty).collect();
    let mut args = args;
    if let Some(listed) = pycc_hir::builtins::materialized_method_args(&base.ty(), method, &tys) {
        args = args.into_iter().map(crate::comp_source::gen_to_list).collect();
        tys = listed;
    }
    match resolve_method(&base.ty(), method, &tys) {
        Some(Ok(sig)) => {
            let mut all = Vec::with_capacity(args.len() + 1);
            all.push(base);
            all.extend(args);
            Ok(MirExpr::RtCall(Box::new(RtCallExpr { sig, args: all })))
        }
        _ => Err((base, args)),
    }
}

fn rt(symbol: &'static str, params: Vec<Ty>, ret: Ty, args: Vec<MirExpr>) -> MirExpr {
    let word_args = vec![false; params.len()];
    MirExpr::RtCall(Box::new(RtCallExpr {
        sig: BuiltinSig {
            symbol,
            params,
            ret,
            ret_word: false,
            word_args,
            kind_args: Vec::new(),
            desc_args: Vec::new(),
        },
        args,
    }))
}

/// `s[i]` on a `str`.
pub(super) fn lower_str_getitem(base: MirExpr, index: MirExpr) -> MirExpr {
    rt(
        "pycc_rt_str_getitem",
        vec![Ty::Str, Ty::Int],
        Ty::Str,
        vec![base, index],
    )
}

/// `s[start:stop:step]` on a `str`: absent bounds pass `0` and the
/// `pycc_rt::SLICE_HAS_*` mask says which were written.
pub(super) fn lower_str_slice(base: MirExpr, bounds: [Option<Box<MirExpr>>; 3]) -> MirExpr {
    let mut present = 0;
    let mut args = vec![base];
    for (bit, bound) in bounds.into_iter().enumerate() {
        match bound {
            Some(bound) => {
                present |= 1 << bit;
                args.push(*bound);
            }
            None => args.push(MirExpr::IntLiteral(0)),
        }
    }
    args.push(MirExpr::IntLiteral(present));
    rt(
        "pycc_rt_str_slice",
        vec![Ty::Str, Ty::Int, Ty::Int, Ty::Int, Ty::Int],
        Ty::Str,
        args,
    )
}

/// `a, b = xs` on a list: the list itself, after `pycc_rt_list_unpack`
/// checked its length is `arity` (raising CPython's `ValueError`).
pub(super) fn lower_list_unpack(list: MirExpr, arity: usize) -> MirExpr {
    let ty = list.ty();
    rt(
        "pycc_rt_list_unpack",
        vec![ty.clone(), Ty::Int],
        ty,
        vec![list, MirExpr::IntLiteral(arity as i64)],
    )
}

/// `needle in haystack` (or `not in`) on two `str`s.
pub(super) fn lower_str_contains(needle: MirExpr, haystack: MirExpr, negate: bool) -> MirExpr {
    let found = rt(
        "pycc_rt_str_contains",
        vec![Ty::Str, Ty::Str],
        Ty::Bool,
        vec![needle, haystack],
    );
    if negate {
        MirExpr::Not(Box::new(found))
    } else {
        found
    }
}
