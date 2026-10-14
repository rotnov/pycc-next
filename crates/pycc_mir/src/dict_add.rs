//! `d[k] += v` on a `dict[K, int]` as one runtime call.
//!
//! The augmented assignment reaches MIR as `d[k] = d[k] + v`: a lookup, an
//! addition and a store that looks the key up again. With `d` a name and
//! `k`, `v` names or literals (so evaluating them first, once, is
//! unobservable), `pycc_rt_dict_add_int` does all three with one lookup --
//! a `Counter`'s `c[x] += 1` idiom -- with the same results: a missing key
//! raises `KeyError`, or reads a `Counter`'s `0` or a `defaultdict`'s new
//! value.

use crate::{MirExpr, RtCallExpr};
use pycc_hir::Ty;
use pycc_hir::builtins::BuiltinSig;

/// Whether evaluating `expr` twice, or before another operand, cannot be
/// told apart from evaluating it once in place.
fn pure(expr: &MirExpr) -> bool {
    matches!(
        expr,
        MirExpr::Name { .. }
            | MirExpr::IntLiteral(_)
            | MirExpr::BoolLiteral(_)
            | MirExpr::StringLiteral(_)
    )
}

/// The fused call for `dict[key] = value` when `value` is `dict[key] + v`
/// over `int` values.
pub(crate) fn fused_dict_add(
    dict: &str,
    base_ty: &Ty,
    key: &MirExpr,
    value: &MirExpr,
) -> Option<MirExpr> {
    let Ty::Dict(kv) = base_ty else {
        return None;
    };
    if kv.1 != Ty::Int || !pure(key) {
        return None;
    }
    let MirExpr::BinOp {
        op: pycc_hir::BinOpKind::Add,
        left,
        right,
        ty: Ty::Int,
    } = value
    else {
        return None;
    };
    let MirExpr::DictGet {
        dict: read,
        key: read_key,
    } = left.as_ref()
    else {
        return None;
    };
    let same_dict = matches!(read.as_ref(), MirExpr::Name { name, .. } if name == dict);
    if !same_dict
        || read_key.as_ref() != key
        || !pure(right)
        || !matches!(right.ty(), Ty::Int | Ty::Bool)
    {
        return None;
    }
    Some(MirExpr::RtCall(Box::new(RtCallExpr {
        sig: BuiltinSig {
            symbol: "pycc_rt_dict_add_int",
            params: vec![base_ty.clone(), kv.0.clone(), Ty::Int],
            ret: Ty::None,
            ret_word: false,
            word_args: vec![false, true, false],
            kind_args: Vec::new(),
            desc_args: Vec::new(),
        },
        args: vec![(**read).clone(), key.clone(), (**right).clone()],
    })))
}
