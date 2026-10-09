//! Binary-operator result typing.
//!
//! Issue #574 (Part 1 of #123) makes binary result typing
//! operator-sensitive for mixed `str`/`int` pairs so that string
//! repetition (`"ab" * 3`) type-checks. This submodule holds
//! [`numeric_result_type`], extracted from [`lib.rs`](crate) per the
//! repository's source-file decomposition rule (AGENTS.md "Keep source
//! files decomposable"), following the precedent set by
//! [`solver`](crate::solver).
//!
//! Two closely related helpers deliberately stayed in `lib.rs`:
//!
//! * `numeric_or_bool_compatible` types *comparison* operands
//!   (`HirExpr::Compare`), not `HirExpr::BinOp`. Repetition never reaches
//!   it, so it is a different cohesion boundary and moving it would be the
//!   unrelated-code churn the decomposition rule forbids.
//! * `is_assignable` is a general type-relation helper with call sites
//!   spread across `lib.rs` and `class.rs`; it is merely adjacent in the
//!   file, not part of this seam.

use pycc_diag::{Diagnostic, Span};
use pycc_hir::{BinOpKind, Ty};

/// Types a binary expression from its operator and its two operand types.
///
/// This helper serves both consumers of binary typing — the validation
/// pass (`infer_expr_in`'s `HirExpr::BinOp` arm) and the private-helper
/// constraint solver (`propagate_binop_constraints`) — so its rules apply
/// identically on the `pycc check` and `pycc build` paths.
///
/// String rules, in the order they are tested:
///
/// * **Repetition (#574).** `str * int` and `int * str` produce `str`, in
///   either operand order, with `bool` accepted as the count consistently
///   with `bool <: int`. This is a `Mul`-only guard clause rather than a
///   widening of the `as_numeric` mapper below: that mapper is
///   operator-blind, so admitting `Ty::Str` there would also accept
///   `str - int`, `str / int`, and every other numeric operator.
///
///   Acceptance here is frontend-only in this milestone. `pycc build`
///   stops at `pycc_codegen`'s named D-072 boundary (exit 101); native
///   repetition is #575.
/// * **Concatenation.** `str + str` produces `str`; every other operator
///   over two `str` operands is `T0021`.
/// * Every remaining mixed pair, including `str * float` and
///   `float * str`, falls through to the generic `T0021` arm.
///
/// The bitwise and shift operators (#1210) are tested first, before either
/// string rule, by [`bitwise_result_type`].
pub(crate) fn numeric_result_type(op: BinOpKind, left: Ty, right: Ty) -> Result<Ty, Diagnostic> {
    if is_bitwise(op) {
        return bitwise_result_type(op, &left, &right);
    }
    // #574: string repetition. Tested before the `str`/`str` arm so that
    // `str * str` still falls through to that arm's `T0021`.
    if op == BinOpKind::Mul
        && ((left == Ty::Str && matches!(right, Ty::Int | Ty::Bool))
            || (right == Ty::Str && matches!(left, Ty::Int | Ty::Bool)))
    {
        return Ok(Ty::Str);
    }
    if left == Ty::Str && right == Ty::Str {
        return if op == BinOpKind::Add {
            Ok(Ty::Str)
        } else {
            Err(Diagnostic::error(
                "T0021",
                format!("operator {op:?} is not defined for `str` and `str`"),
                Span::new(0, 0),
            ))
        };
    }
    let as_numeric = |t: &Ty| match t {
        Ty::Bool | Ty::Int => Some(Ty::Int),
        Ty::Float => Some(Ty::Float),
        _ => None,
    };
    match (as_numeric(&left), as_numeric(&right)) {
        (Some(_), Some(_)) if op == BinOpKind::Div => Ok(Ty::Float),
        (Some(Ty::Int), Some(Ty::Int)) => Ok(Ty::Int),
        (Some(_), Some(_)) => Ok(Ty::Float),
        _ => Err(Diagnostic::error(
            "T0021",
            format!(
                "operator {op:?} is not defined for `{}` and `{}`",
                left.name(),
                right.name()
            ),
            Span::new(0, 0),
        )),
    }
}

/// `<< >> & | ^`: the operators whose operands must be integers.
fn is_bitwise(op: BinOpKind) -> bool {
    matches!(
        op,
        BinOpKind::LShift
            | BinOpKind::RShift
            | BinOpKind::BitAnd
            | BinOpKind::BitOr
            | BinOpKind::BitXor
    )
}

/// Types a bitwise or shift expression (#1210).
///
/// * `bool & bool`, `bool | bool` and `bool ^ bool` are `bool`, as CPython's
///   `bool.__and__`/`__or__`/`__xor__` return; a shift of two `bool`s is an
///   `int` (`True << 1 == 2`).
/// * Any other `int`/`bool` pair is `int`.
/// * Everything else is `T0021`. A `float`, `str`, container or instance
///   operand is "not defined", which is what CPython raises as `TypeError`.
///   The pairs CPython *does* define but pycc does not implement -- `set`
///   op `set` for `& | ^` and `dict | dict` -- say "not supported yet"
///   instead, keeping the code `T0021` so no mutable operand reaches the
///   augmented-assignment rewrite (the S1 condition in
///   `docs/TYPE_SYSTEM.md`).
fn bitwise_result_type(op: BinOpKind, left: &Ty, right: &Ty) -> Result<Ty, Diagnostic> {
    let is_int = |t: &Ty| matches!(t, Ty::Bool | Ty::Int);
    if is_int(left) && is_int(right) {
        let keeps_bool = matches!(op, BinOpKind::BitAnd | BinOpKind::BitOr | BinOpKind::BitXor);
        return Ok(if keeps_bool && *left == Ty::Bool && *right == Ty::Bool {
            Ty::Bool
        } else {
            Ty::Int
        });
    }
    let defined_by_cpython = match (left, right) {
        // `set` and `frozenset` combine with each other in CPython, in any
        // mix (Part 1 of #1319).
        (Ty::Set(_) | Ty::FrozenSet(_), Ty::Set(_) | Ty::FrozenSet(_)) => {
            op != BinOpKind::LShift && op != BinOpKind::RShift
        }
        (Ty::Dict(_), Ty::Dict(_)) => op == BinOpKind::BitOr,
        _ => false,
    };
    let reason = if defined_by_cpython {
        "is not supported yet"
    } else {
        "is not defined"
    };
    Err(Diagnostic::error(
        "T0021",
        format!(
            "operator `{}` on `{}` and `{}` {reason}",
            op.as_str(),
            left.name(),
            right.name()
        ),
        Span::new(0, 0),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- #1210: bitwise and shift operators ----

    const BITWISE: [BinOpKind; 5] = [
        BinOpKind::LShift,
        BinOpKind::RShift,
        BinOpKind::BitAnd,
        BinOpKind::BitOr,
        BinOpKind::BitXor,
    ];

    #[test]
    fn a_bitwise_operator_over_two_bools_keeps_bool_but_a_shift_does_not() {
        for op in BITWISE {
            let expected = if matches!(op, BinOpKind::LShift | BinOpKind::RShift) {
                Ty::Int
            } else {
                Ty::Bool
            };
            assert_eq!(
                numeric_result_type(op, Ty::Bool, Ty::Bool),
                Ok(expected),
                "{op:?}"
            );
        }
    }

    #[test]
    fn a_bitwise_operator_over_any_other_int_pair_is_int() {
        for op in BITWISE {
            for (left, right) in [(Ty::Bool, Ty::Int), (Ty::Int, Ty::Bool), (Ty::Int, Ty::Int)] {
                assert_eq!(
                    numeric_result_type(op, left.clone(), right.clone()),
                    Ok(Ty::Int),
                    "{op:?} {left:?} {right:?}"
                );
            }
        }
    }

    #[test]
    fn a_bitwise_operator_with_a_float_or_str_operand_is_not_defined() {
        for op in BITWISE {
            for (left, right) in [
                (Ty::Float, Ty::Int),
                (Ty::Int, Ty::Float),
                (Ty::Int, Ty::Str),
                (Ty::Str, Ty::Str),
            ] {
                let err = numeric_result_type(op, left.clone(), right.clone()).unwrap_err();
                assert_eq!(err.code, "T0021");
                assert_eq!(
                    err.message,
                    format!(
                        "operator `{}` on `{}` and `{}` is not defined",
                        op.as_str(),
                        left.name(),
                        right.name()
                    )
                );
            }
        }
    }

    #[test]
    fn the_set_and_dict_operators_cpython_defines_are_not_supported_yet() {
        let set = || Ty::Set(Box::new(Ty::Int));
        let frozen = || Ty::FrozenSet(Box::new(Ty::Int));
        let dict = || Ty::Dict(Box::new((Ty::Str, Ty::Int)));
        for (op, left, right, reason) in [
            (BinOpKind::BitOr, set(), set(), "is not supported yet"),
            (BinOpKind::BitAnd, set(), set(), "is not supported yet"),
            (BinOpKind::BitXor, set(), set(), "is not supported yet"),
            (BinOpKind::LShift, set(), set(), "is not defined"),
            (BinOpKind::RShift, set(), set(), "is not defined"),
            (BinOpKind::BitOr, dict(), dict(), "is not supported yet"),
            (BinOpKind::BitAnd, dict(), dict(), "is not defined"),
            (BinOpKind::BitXor, dict(), dict(), "is not defined"),
            (BinOpKind::BitOr, set(), Ty::Int, "is not defined"),
            (BinOpKind::BitOr, frozen(), frozen(), "is not supported yet"),
            (BinOpKind::BitAnd, set(), frozen(), "is not supported yet"),
            (BinOpKind::BitXor, frozen(), set(), "is not supported yet"),
            (BinOpKind::LShift, frozen(), frozen(), "is not defined"),
            (BinOpKind::BitOr, frozen(), Ty::Int, "is not defined"),
        ] {
            let err = numeric_result_type(op, left.clone(), right.clone()).unwrap_err();
            assert_eq!(err.code, "T0021");
            assert_eq!(
                err.message,
                format!(
                    "operator `{}` on `{}` and `{}` {reason}",
                    op.as_str(),
                    left.name(),
                    right.name()
                )
            );
        }
    }

    #[test]
    fn numeric_result_type_covers_every_int_float_combination() {
        assert_eq!(
            numeric_result_type(BinOpKind::Add, Ty::Float, Ty::Float),
            Ok(Ty::Float)
        );
        assert_eq!(
            numeric_result_type(BinOpKind::Add, Ty::Float, Ty::Int),
            Ok(Ty::Float)
        );
    }

    #[test]
    fn numeric_result_type_rejects_a_hypothetical_incompatible_pair() {
        let err = numeric_result_type(BinOpKind::Add, Ty::Int, Ty::None).unwrap_err();
        assert_eq!(err.code, "T0021");
    }

    #[test]
    fn numeric_result_type_accepts_float_and_bool_since_bool_is_numeric_like() {
        // Task 7 makes `bool` numeric-like everywhere (`True + 1.5 == 2.5` is
        // legal Python), so this pair is no longer an error -- see
        // `a_binop_treats_bool_and_float_as_float` for the `infer_expr`-level
        // version of this same rule.
        assert_eq!(
            numeric_result_type(BinOpKind::Add, Ty::Float, Ty::Bool),
            Ok(Ty::Float)
        );
    }

    #[test]
    fn numeric_result_type_rejects_a_float_and_a_hypothetical_none() {
        // Exercises `.name()` for `Float` in the error arm now that
        // `Float`+`Bool` no longer takes that path.
        let err = numeric_result_type(BinOpKind::Add, Ty::Float, Ty::None).unwrap_err();
        assert!(err.message.contains("float") && err.message.contains("None"));
    }

    #[test]
    fn numeric_result_type_rejects_a_hypothetical_str_operand() {
        // #574: the repetition clause is `Mul`-only, so a non-`Mul` operator
        // over a `bool`/`str` pair is still `T0021`.
        let err = numeric_result_type(BinOpKind::Add, Ty::Bool, Ty::Str).unwrap_err();
        assert!(err.message.contains("str"));
    }

    // ---- #574: string repetition ----

    #[test]
    fn multiplying_a_str_by_an_int_infers_str() {
        assert_eq!(
            numeric_result_type(BinOpKind::Mul, Ty::Str, Ty::Int),
            Ok(Ty::Str)
        );
    }

    #[test]
    fn multiplying_an_int_by_a_str_infers_str() {
        assert_eq!(
            numeric_result_type(BinOpKind::Mul, Ty::Int, Ty::Str),
            Ok(Ty::Str)
        );
    }

    #[test]
    fn multiplying_a_str_by_a_bool_infers_str_since_bool_is_an_int() {
        assert_eq!(
            numeric_result_type(BinOpKind::Mul, Ty::Str, Ty::Bool),
            Ok(Ty::Str)
        );
    }

    #[test]
    fn multiplying_a_bool_by_a_str_infers_str_since_bool_is_an_int() {
        assert_eq!(
            numeric_result_type(BinOpKind::Mul, Ty::Bool, Ty::Str),
            Ok(Ty::Str)
        );
    }

    #[test]
    fn multiplying_a_str_by_a_float_is_still_rejected() {
        let err = numeric_result_type(BinOpKind::Mul, Ty::Str, Ty::Float).unwrap_err();
        assert_eq!(err.code, "T0021");
        assert!(err.message.contains("str") && err.message.contains("float"));
    }

    #[test]
    fn multiplying_a_float_by_a_str_is_still_rejected() {
        let err = numeric_result_type(BinOpKind::Mul, Ty::Float, Ty::Str).unwrap_err();
        assert_eq!(err.code, "T0021");
        assert!(err.message.contains("str") && err.message.contains("float"));
    }

    #[test]
    fn multiplying_two_strs_is_still_rejected() {
        let err = numeric_result_type(BinOpKind::Mul, Ty::Str, Ty::Str).unwrap_err();
        assert_eq!(err.code, "T0021");
        assert!(err.message.contains("`str` and `str`"));
    }

    #[test]
    fn subtracting_an_int_from_a_str_is_still_rejected() {
        // The repetition clause must not widen `as_numeric`: every other
        // numeric operator over a `str`/`int` pair stays `T0021`.
        let err = numeric_result_type(BinOpKind::Sub, Ty::Str, Ty::Int).unwrap_err();
        assert_eq!(err.code, "T0021");
    }

    #[test]
    fn dividing_a_str_by_an_int_is_still_rejected() {
        let err = numeric_result_type(BinOpKind::Div, Ty::Str, Ty::Int).unwrap_err();
        assert_eq!(err.code, "T0021");
    }

    #[test]
    fn adding_an_int_to_a_str_is_still_rejected() {
        let err = numeric_result_type(BinOpKind::Add, Ty::Str, Ty::Int).unwrap_err();
        assert_eq!(err.code, "T0021");
    }

    #[test]
    fn concatenating_two_strs_still_infers_str() {
        assert_eq!(
            numeric_result_type(BinOpKind::Add, Ty::Str, Ty::Str),
            Ok(Ty::Str)
        );
    }
}
