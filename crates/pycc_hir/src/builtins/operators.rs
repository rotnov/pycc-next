//! The binary-operator and augmented-assignment calls of sets, counters
//! and lists that lower to `pycc_rt` (`set_operator`, `list_inplace_operator`).

use super::{BuiltinSig, hashed_set, is_intlike, sig};
use crate::Ty;

/// `left <op> right` between two sets (or frozensets) of one hashable
/// element type, for `|`, `&`, `-` and `^`: the runtime call, whose result
/// has the left operand's type as in CPython. `in_place` (`a |= b`) on a
/// `set` updates `left` itself (`pycc_rt_set_inplace` with the operator's
/// index as a trailing int).
#[must_use]
pub fn set_operator(op: &str, in_place: bool, left: &Ty, right: &Ty) -> Option<BuiltinSig> {
    // `Counter` arithmetic (`+`, `-`, `|`, `&`) and the dict union `|`
    // (`pycc_rt::dict::flavor`): the runtime picks by flavor.
    if let (Ty::Dict(a), Ty::Dict(b)) = (left, right)
        && a == b
        && (op == "|" || (a.1 == Ty::Int && matches!(op, "&" | "-" | "+")))
    {
        return Some(sig(
            "pycc_rt_counter_binop",
            &[left.clone(), right.clone(), Ty::Int],
            left.clone(),
        ));
    }
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

/// `list += other` / `list *= n` (an augmented assignment): CPython
/// mutates the list in place, so the runtime extends or repeats `left`
/// itself and returns it as the value stored back.
#[must_use]
pub fn list_inplace_operator(op: &str, left: &Ty, right: &Ty) -> Option<BuiltinSig> {
    let Ty::List(_) = left else {
        return None;
    };
    match op {
        "+" if right == left => Some(sig(
            "pycc_rt_list_iadd",
            &[left.clone(), left.clone()],
            left.clone(),
        )),
        "*" if is_intlike(right) => Some(sig(
            "pycc_rt_list_imul",
            &[left.clone(), Ty::Int],
            left.clone(),
        )),
        _ => None,
    }
}

/// The operator index `pycc_rt_set_inplace` takes for `op`.
#[must_use]
pub fn set_inplace_index(op: &str) -> i64 {
    match op {
        "|" => 0,
        "&" => 1,
        "-" => 2,
        // `^` of sets, `+` of counters.
        _ => 3,
    }
}
