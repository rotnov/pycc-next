//! Unary-operator result typing.
//!
//! Issue #603 (Part 2 of #573) lowers `-x` and `+x` over non-literal
//! operands; issue #604 (Part 3) adds `not x` and `~x`. This submodule
//! holds [`unary_result_type`], a direct sibling of [`binop`](crate::binop)
//! placed here for the same reason that one was extracted from
//! [`lib.rs`](crate): the repository's source-file decomposition rule
//! (AGENTS.md "Keep source files decomposable").
//!
//! `USub`/`UAdd` share one rule, the *unary projection* of
//! [`numeric_result_type`](crate::binop::numeric_result_type)'s numeric
//! mapper: `bool` and `int` both yield `int` (`-True` is `-1` in Python, so
//! the operand crosses into `int`), `float` yields `float`, and everything
//! else is `T0021`. `str` has no unary form at all -- unlike `str * int`,
//! there is no `-"ab"` -- so it needs no guard clause and falls through to
//! the error arm with every other non-numeric type.
//!
//! `Not` and `Invert` each need their own rule instead of sharing that one:
//!
//! * `not x` is defined by truthiness, not by numeric promotion. Its result
//!   is always `bool`, for every operand type this compiler can actually
//!   compute a truth value for at codegen time --
//!   [`truthy`](../../pycc_codegen/fn.truthy.html) in `pycc_codegen`
//!   handles `bool`, `int`, `float`, `str`, `None`, `Optional`, and a class
//!   instance, but panics on `list`/`dict` (no `pycc_rt_*_truthy`
//!   entry point exists for them yet). Accepting those container types
//!   here would let a `not` expression reach that panic, so they are
//!   rejected with `T0021` instead. `set[int]`/`frozenset[int]` truthiness
//!   is real since Part 1 of #1319, which widened only `if`/`while`
//!   conditions; `not` over a set stays `T0021` until a later part admits
//!   it -- the same "every type pycc models a
//!   truth value for" reading the issue's own completion criteria use,
//!   not a narrowing of them. `None` needs no runtime call at all: a
//!   `Ty::None`-typed `MirExpr::Name` read lowers to `Scalar::Bool` backed
//!   by an always-zero `i8` slot, and `MirExpr::NoneLiteral` lowers to
//!   `Scalar::Optional`, so `truthy`'s existing `Bool`/`Optional` arms
//!   already compute the correct (always-`False`) truth value for it.
//!   `Ty::Instance(_)` needs no runtime call either (D-154, Part 1 of
//!   #375): this compiler ships no `__bool__`/`__len__`, so every
//!   instance is unconditionally truthy, and `truthy`'s `Scalar::Instance`
//!   arm already returns the constant `1` for it -- `not <instance>` is
//!   always `False`.
//! * Since Part 10 of #1371, `not o` on a CPython object (`Ty::Object`) is
//!   `bool` too: codegen's `truthy` runs the object's own
//!   `PyObject_IsTrue` (`pycc_ext_obj_truthy`), whose failure takes the
//!   foreign failure edge, and inverts it.
//! * `~x` is `int -> int` only (`bool` included, since `pycc_types` treats
//!   it as a numeric subtype of `int`); every other operand is `T0021`.

use pycc_diag::{Diagnostic, Span};
use pycc_hir::{Ty, UnaryOpKind};

/// Types a unary expression from its operator and its operand type.
pub(crate) fn unary_result_type(op: UnaryOpKind, operand: Ty) -> Result<Ty, Diagnostic> {
    match op {
        UnaryOpKind::USub | UnaryOpKind::UAdd => match operand {
            Ty::Bool | Ty::Int => Ok(Ty::Int),
            Ty::Float => Ok(Ty::Float),
            _ => Err(unary_type_error(op, operand)),
        },
        UnaryOpKind::Not if is_truth_testable(&operand) => Ok(Ty::Bool),
        // Part 10 of #1371: `not o` on a CPython object is CPython's own
        // `PyObject_Not`, the negated `PyObject_IsTrue` that codegen's
        // `truthy` already runs for an `if o:` condition. The result is a
        // native `bool`, because CPython's `not` always yields one (unlike
        // `==`, whose object result Part 1 keeps as `Ty::Object`).
        UnaryOpKind::Not if operand == Ty::Object => Ok(Ty::Bool),
        UnaryOpKind::Not => Err(unary_type_error(op, operand)),
        UnaryOpKind::Invert => match operand {
            Ty::Bool | Ty::Int => Ok(Ty::Int),
            _ => Err(unary_type_error(op, operand)),
        },
    }
}

/// Whether codegen's `truthy` can compute a truth value for a value of type
/// `ty`: `bool`, `int`, `float`, `str`, `None`, `Optional[_]` and a class
/// instance. Containers, protocol values, `memoryview` and the opaque CPython
/// object are not truth-testable here (see this module's doc comment); `not`
/// admits the object separately (Part 10 of #1371), and `and`/`or` and the
/// conditional expression do the same in their own modules.
///
/// Shared by `not` and by `and`/`or` (#1211, [`crate::boolop`]), which call
/// it on every operand so that a boolean operator never admits an operand
/// `not` refuses. `and`/`or` additionally refuse an instance whose class
/// defines `__bool__` or `__len__`; `not` does not yet, so this predicate
/// admits every instance.
pub(crate) fn is_truth_testable(ty: &Ty) -> bool {
    matches!(
        ty,
        Ty::Bool | Ty::Int | Ty::Float | Ty::Str | Ty::None | Ty::Optional(_) | Ty::Instance(_)
    )
}

fn unary_type_error(op: UnaryOpKind, operand: Ty) -> Diagnostic {
    Diagnostic::error(
        "T0021",
        format!(
            "unary operator {op:?} is not defined for `{}`",
            operand.name()
        ),
        Span::new(0, 0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn negating_an_int_stays_int() {
        assert_eq!(unary_result_type(UnaryOpKind::USub, Ty::Int), Ok(Ty::Int));
    }

    #[test]
    fn unary_plus_on_an_int_stays_int() {
        assert_eq!(unary_result_type(UnaryOpKind::UAdd, Ty::Int), Ok(Ty::Int));
    }

    #[test]
    fn negating_a_bool_promotes_to_int() {
        // `-True == -1` in Python: the operand crosses into `int` here.
        assert_eq!(unary_result_type(UnaryOpKind::USub, Ty::Bool), Ok(Ty::Int));
    }

    #[test]
    fn unary_plus_on_a_bool_promotes_to_int() {
        assert_eq!(unary_result_type(UnaryOpKind::UAdd, Ty::Bool), Ok(Ty::Int));
    }

    #[test]
    fn negating_a_float_stays_float() {
        assert_eq!(
            unary_result_type(UnaryOpKind::USub, Ty::Float),
            Ok(Ty::Float)
        );
    }

    #[test]
    fn unary_plus_on_a_float_stays_float() {
        assert_eq!(
            unary_result_type(UnaryOpKind::UAdd, Ty::Float),
            Ok(Ty::Float)
        );
    }

    #[test]
    fn negating_a_str_is_rejected() {
        // There is no `-"ab"` in Python, so `str` gets no repetition-style
        // guard clause the way `numeric_result_type` gives `str * int`.
        let err = unary_result_type(UnaryOpKind::USub, Ty::Str).unwrap_err();
        assert_eq!(err.code, "T0021");
        assert!(err.message.contains("USub") && err.message.contains("str"));
    }

    #[test]
    fn unary_plus_on_none_is_rejected() {
        let err = unary_result_type(UnaryOpKind::UAdd, Ty::None).unwrap_err();
        assert_eq!(err.code, "T0021");
        assert!(err.message.contains("UAdd") && err.message.contains("None"));
    }

    #[test]
    fn logical_not_on_none_types_as_bool() {
        // `not None` is always `True` in CPython; `truthy`'s existing
        // `Bool`/`Optional` arms already compute the correct (always
        // `False`) truthiness for both `Ty::None` representations, with no
        // codegen change needed -- see this module's own doc comment.
        assert_eq!(unary_result_type(UnaryOpKind::Not, Ty::None), Ok(Ty::Bool));
    }

    #[test]
    fn logical_not_on_a_cpython_object_types_as_bool() {
        // Part 10 of #1371: CPython's `not` always yields a `bool`, so the
        // result is native even though the operand is opaque.
        assert_eq!(
            unary_result_type(UnaryOpKind::Not, Ty::Object),
            Ok(Ty::Bool)
        );
        // The other unary operators keep refusing an object operand.
        for op in [UnaryOpKind::USub, UnaryOpKind::UAdd, UnaryOpKind::Invert] {
            let err = unary_result_type(op, Ty::Object).unwrap_err();
            assert_eq!(err.code, "T0021");
        }
    }

    #[test]
    fn logical_not_on_a_class_instance_types_as_bool() {
        // D-154 (Part 1 of #375): this compiler ships no `__bool__`/
        // `__len__`, so every instance is unconditionally truthy, and
        // `truthy`'s `Scalar::Instance` arm already returns the constant
        // `1` for it -- `not <instance>` type-checks and is always
        // `False`.
        assert_eq!(
            unary_result_type(UnaryOpKind::Not, Ty::Instance(Box::new("C".to_string()))),
            Ok(Ty::Bool)
        );
    }
}
