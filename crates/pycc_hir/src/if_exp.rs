//! The conditional expression `body if test else orelse` (#1395): the
//! result-type join shared by `pycc_types` and `pycc_mir`.
//!
//! Lowering itself lives in `crate::expr::lower_expr`'s `Expr::If` arm; the
//! node is [`HirExpr::IfExp`](crate::HirExpr::IfExp).

use crate::Ty;

/// The result type of `body if test else orelse`, given the two branch
/// types, or `None` when they have no common type pycc can represent.
///
/// The one owner of this rule: `pycc_types` checks with it and `pycc_mir`
/// recomputes each node's type with it, so the two can never disagree.
///
/// | branches | result |
/// |---|---|
/// | two equal types, other than `None`, a protocol, `memoryview` or an unresolved type | that type (a generic function's own `T` included, substituted per call site) |
/// | `T` with `None`, `T` with `T \| None`, or `T \| None` with `None`, for `T` in `int`/`float`/`bool` | `T \| None` |
/// | anything else | `None` (refused) |
///
/// Unlike [`bool_op_result_ty`](crate::bool_op_result_ty), `bool` with `int`
/// is refused rather than widened: `True if c else 2` yields the `bool`
/// `True`, which prints `True`, and an `int` result would print `1`. An
/// `int` with a `float` is refused for the same reason. A CPython object
/// joins only with another CPython object.
pub fn if_exp_result_ty(body: &Ty, orelse: &Ty) -> Option<Ty> {
    match (body, orelse) {
        (
            Ty::Bool
            | Ty::Int
            | Ty::Float
            | Ty::Str
            | Ty::Object
            | Ty::Optional(_)
            | Ty::List(_)
            | Ty::Dict(_)
            | Ty::Set(_)
            | Ty::FrozenSet(_)
            | Ty::Tuple(_)
            | Ty::Instance(_)
            | Ty::Param(_),
            _,
        ) if body == orelse => Some(body.clone()),
        (bare, Ty::None) | (Ty::None, bare) if optional_payload(bare) => {
            Some(Ty::Optional(Box::new(bare.clone())))
        }
        (optional @ Ty::Optional(_), Ty::None) | (Ty::None, optional @ Ty::Optional(_)) => {
            Some(optional.clone())
        }
        (bare, Ty::Optional(inner)) | (Ty::Optional(inner), bare)
            if inner.as_ref() == bare && optional_payload(bare) =>
        {
            Some(Ty::Optional(inner.clone()))
        }
        _ => None,
    }
}

/// Whether `ty` may be the payload of an `Optional`: `int`, `float` or
/// `bool`, the inner types `T0049` admits.
fn optional_payload(ty: &Ty) -> bool {
    matches!(ty, Ty::Int | Ty::Float | Ty::Bool)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn optional(inner: Ty) -> Ty {
        Ty::Optional(Box::new(inner))
    }

    #[test]
    fn equal_branch_types_join_as_themselves() {
        for ty in [
            Ty::Bool,
            Ty::Int,
            Ty::Float,
            Ty::Str,
            Ty::Object,
            optional(Ty::Int),
            Ty::List(Box::new(Ty::Int)),
            Ty::Dict(Box::new((Ty::Str, Ty::Int))),
            Ty::Set(Box::new(Ty::Int)),
            Ty::FrozenSet(Box::new(Ty::Str)),
            Ty::Tuple(Box::new(vec![Ty::Int, Ty::Str])),
            Ty::Instance(Box::new("C".to_string())),
            Ty::Param(Box::new("T".to_string())),
        ] {
            assert_eq!(if_exp_result_ty(&ty, &ty), Some(ty.clone()), "{ty:?}");
        }
    }

    #[test]
    fn a_scalar_with_none_joins_as_its_optional_in_either_order() {
        for ty in [Ty::Int, Ty::Float, Ty::Bool] {
            assert_eq!(if_exp_result_ty(&ty, &Ty::None), Some(optional(ty.clone())));
            assert_eq!(if_exp_result_ty(&Ty::None, &ty), Some(optional(ty.clone())));
        }
    }

    #[test]
    fn an_optional_with_none_or_its_payload_stays_that_optional() {
        let opt = optional(Ty::Int);
        assert_eq!(if_exp_result_ty(&opt, &Ty::None), Some(opt.clone()));
        assert_eq!(if_exp_result_ty(&Ty::None, &opt), Some(opt.clone()));
        assert_eq!(if_exp_result_ty(&Ty::Int, &opt), Some(opt.clone()));
        assert_eq!(if_exp_result_ty(&opt, &Ty::Int), Some(opt.clone()));
    }

    #[test]
    fn every_other_pair_has_no_common_type() {
        let class_a = Ty::Instance(Box::new("A".to_string()));
        let class_b = Ty::Instance(Box::new("B".to_string()));
        for (body, orelse) in [
            (Ty::Bool, Ty::Int),
            (Ty::Int, Ty::Float),
            (Ty::Int, Ty::Str),
            (Ty::Str, Ty::None),
            (Ty::None, Ty::None),
            (Ty::Object, Ty::Int),
            (Ty::Str, Ty::Object),
            (class_a, class_b),
            (optional(Ty::Int), optional(Ty::Float)),
            (Ty::Float, optional(Ty::Int)),
            (Ty::List(Box::new(Ty::Int)), Ty::List(Box::new(Ty::Str))),
            (Ty::MemoryView, Ty::MemoryView),
            (Ty::Infer, Ty::Infer),
        ] {
            assert_eq!(
                if_exp_result_ty(&body, &orelse),
                None,
                "{body:?} {orelse:?}"
            );
        }
    }

    use crate::expr::rename_name_in_expr;
    use crate::{
        BoolOpKind, HirExpr, HirItem, HirStmt, killed_names, lower_checked, pycc_parser_test_helper,
    };

    fn name(n: &str) -> HirExpr {
        HirExpr::Name(n.to_string())
    }

    fn if_exp(test: HirExpr, body: HirExpr, orelse: HirExpr) -> HirExpr {
        HirExpr::IfExp {
            test: Box::new(test),
            body: Box::new(body),
            orelse: Box::new(orelse),
        }
    }

    /// The body of the first function `source` defines.
    fn function_body(source: &str) -> Vec<HirStmt> {
        let module = lower_checked(&pycc_parser_test_helper::parse(source)).unwrap();
        module
            .items
            .into_iter()
            .find_map(|item| match item {
                HirItem::Function { body, .. } => Some(body),
                HirItem::TopLevelStmt(_) => None,
            })
            .expect("the source defines a function")
    }

    fn lowering_error(source: &str) -> pycc_diag::Diagnostic {
        lower_checked(&pycc_parser_test_helper::parse(source)).unwrap_err()
    }

    #[test]
    fn the_condition_is_a_truth_context_and_the_branches_are_values() {
        let body = function_body(
            "def f(a: int, b: int, c: int) -> int:\n    return b or c if a and b else c or b\n",
        );
        let bool_op = |left: &str, right: &str, truth_only: bool| HirExpr::BoolOp {
            op: if truth_only {
                BoolOpKind::And
            } else {
                BoolOpKind::Or
            },
            left: Box::new(name(left)),
            right: Box::new(name(right)),
            truth_only,
        };
        assert_eq!(
            body,
            [HirStmt::Return(Some(if_exp(
                bool_op("a", "b", true),
                bool_op("b", "c", false),
                bool_op("c", "b", false)
            )))]
        );
    }

    #[test]
    fn a_walrus_in_either_branch_is_refused() {
        for source in [
            "def f(t: bool) -> int:\n    return (n := 1) if t else 2\n",
            "def f(t: bool) -> int:\n    return 1 if t else (n := 2)\n",
            // Found inside a nested conditional expression's own condition.
            "def f(t: bool) -> int:\n    return 1 if t else (2 if (n := t) else 3)\n",
        ] {
            let err = lowering_error(source);
            assert_eq!(err.code, "C0001", "{source}");
            assert_eq!(
                err.message,
                "a walrus assignment (`:=`) in a conditional expression branch is not supported"
            );
        }
    }

    #[test]
    fn a_walrus_in_the_condition_of_an_if_test_is_admitted_and_kills_its_target() {
        // The leading module-level statement is skipped by `function_body`.
        let body = function_body(
            "k = 1\n\ndef f(a: int) -> int:\n    if (1 if (n := a) else 0):\n        return n\n    return 0\n",
        );
        assert!(
            matches!(
                &body[0],
                HirStmt::If { test: HirExpr::IfExp { test: condition, .. }, .. }
                    if matches!(**condition, HirExpr::NamedExpr { .. })
            ),
            "{body:?}"
        );
        assert!(killed_names(&body).contains("n"), "{body:?}");
    }

    #[test]
    fn an_unsupported_branch_propagates_its_own_error() {
        let err = lowering_error("def f(t: bool) -> int:\n    return (lambda: 1) if t else 2\n");
        assert!(!err.message.contains("walrus"), "{}", err.message);
    }

    #[test]
    fn renaming_reaches_every_part() {
        let renamed = rename_name_in_expr(if_exp(name("i"), name("i"), name("i")), "i", "k");
        assert_eq!(renamed, if_exp(name("k"), name("k"), name("k")));
    }
}
