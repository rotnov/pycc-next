//! The result type of a lowered binary expression.
//!
//! Carved out of `lib.rs` under AGENTS.md's decomposability rule when #1210
//! added the bitwise and shift operators to [`binop_result_ty`]. It must
//! agree exactly with `pycc_types`' `numeric_result_type`: `pycc_types`
//! already accepted the program on that function's promise, so a different
//! answer here would make MIR's `ty` lie about what codegen must produce.

use crate::{BinOpKind, Ty};

pub(crate) fn binop_result_ty(op: BinOpKind, left: Ty, right: Ty) -> Ty {
    // #1210: the bitwise and shift operators take only `int`/`bool`
    // operands, so they are decided before the float rule below and can
    // never produce `Float`. `& | ^` over two `bool`s is `bool`, as
    // CPython's `bool.__and__`/`__or__`/`__xor__` return; a shift is
    // always an `int`.
    match op {
        BinOpKind::BitAnd | BinOpKind::BitOr | BinOpKind::BitXor
            if left == Ty::Bool && right == Ty::Bool =>
        {
            return Ty::Bool;
        }
        BinOpKind::LShift
        | BinOpKind::RShift
        | BinOpKind::BitAnd
        | BinOpKind::BitOr
        | BinOpKind::BitXor => return Ty::Int,
        _ => {}
    }
    // #574 (Part 1 of #123): string repetition. `pycc_types`'
    // `numeric_result_type` types `str * int` and `int * str` -- with
    // `bool` accepted as the count, since `bool <: int` -- as `str`, so
    // this function has to say the same. Codegen lowers it natively since
    // #575.
    if op == BinOpKind::Mul
        && ((left == Ty::Str && matches!(right, Ty::Int | Ty::Bool))
            || (right == Ty::Str && matches!(left, Ty::Int | Ty::Bool)))
    {
        return Ty::Str;
    }
    if left == Ty::Str && right == Ty::Str && op == BinOpKind::Add {
        return Ty::Str;
    }
    // True division always produces `float`, even for two `int`/`bool`
    // operands -- this must match `pycc_types::numeric_result_type`'s own
    // rule (`(Some(_), Some(_)) if op == BinOpKind::Div => Ok(Ty::Float)`)
    // exactly (`5 / 2` is `2.5`, not `2`).
    if op == BinOpKind::Div || left == Ty::Float || right == Ty::Float {
        return Ty::Float;
    }
    Ty::Int
}

#[cfg(test)]
mod tests {
    use super::binop_result_ty;
    use crate::{BinOpKind, Ty};

    #[test]
    fn every_rule_answers_what_pycc_types_accepted() {
        for (op, left, right, expected) in [
            (BinOpKind::BitAnd, Ty::Bool, Ty::Bool, Ty::Bool),
            (BinOpKind::BitOr, Ty::Bool, Ty::Bool, Ty::Bool),
            (BinOpKind::BitXor, Ty::Bool, Ty::Bool, Ty::Bool),
            (BinOpKind::BitAnd, Ty::Bool, Ty::Int, Ty::Int),
            (BinOpKind::BitOr, Ty::Int, Ty::Bool, Ty::Int),
            (BinOpKind::BitXor, Ty::Int, Ty::Int, Ty::Int),
            (BinOpKind::LShift, Ty::Bool, Ty::Bool, Ty::Int),
            (BinOpKind::RShift, Ty::Int, Ty::Int, Ty::Int),
            (BinOpKind::Mul, Ty::Str, Ty::Int, Ty::Str),
            (BinOpKind::Mul, Ty::Bool, Ty::Str, Ty::Str),
            (BinOpKind::Add, Ty::Str, Ty::Str, Ty::Str),
            (BinOpKind::Div, Ty::Int, Ty::Int, Ty::Float),
            (BinOpKind::Add, Ty::Float, Ty::Int, Ty::Float),
            (BinOpKind::Add, Ty::Int, Ty::Float, Ty::Float),
            (BinOpKind::Add, Ty::Bool, Ty::Int, Ty::Int),
        ] {
            assert_eq!(
                binop_result_ty(op, left.clone(), right.clone()),
                expected,
                "{op:?} {left:?} {right:?}"
            );
        }
    }
}
