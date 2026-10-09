//! `None` into an `object` return slot (#1387, D-258's return-position
//! amendment).
//!
//! D-258 makes `object` (and, in an `--ext` module, `Any`) the opaque
//! CPython top type, and `None` is a CPython object -- but `None` stays a
//! native `Ty::None` value inside a compiled body, and `crate::is_assignable`
//! deliberately keeps it out of every `object` slot.
//!
//! The return position was the first seam to narrow that, for the two
//! spellings whose value is CPython's `None` by construction: a bare
//! `return` and `return None` (the literal). Since Part 2 of #1387 every
//! `None`-typed *value* -- the literal, or a call such as `return g()` with
//! `g -> None` -- is boxed into an `object` slot at every seam
//! (`crate::object_box`), so what this module still owns is the bare
//! `return`, which has no value for that rule to see. Falling off the end of
//! an `object` function stays `T0022` -- an implicit `None` is not written
//! down anywhere and `pycc_hir::block_always_returns` keeps its existing contract.
//!
//! Both walkers -- `crate::check_stmt_in_function` and the solver's
//! `HirStmt::Return` arm -- call [`admits_none_return`], so the two can
//! never disagree about the admitted set. Codegen's counterpart is
//! `pycc_codegen`'s `object_return` module.

use pycc_hir::HirExpr;
use pycc_hir::Ty;

/// Whether `return <value>` (`value == None` for a bare `return`) in a
/// function declared to return `declared` returns CPython's `None` into an
/// `object` slot -- and so is admitted without the assignability check that
/// would otherwise report `T0022`.
pub(crate) fn admits_none_return(declared: &Ty, value: Option<&HirExpr>) -> bool {
    *declared == Ty::Object && matches!(value, None | Some(HirExpr::NoneLiteral))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admits_only_bare_and_literal_none_into_object() {
        assert!(admits_none_return(&Ty::Object, None));
        assert!(admits_none_return(&Ty::Object, Some(&HirExpr::NoneLiteral)));
        assert!(!admits_none_return(
            &Ty::Object,
            Some(&HirExpr::IntLiteral(0))
        ));
        assert!(!admits_none_return(&Ty::Int, None));
        assert!(!admits_none_return(&Ty::Int, Some(&HirExpr::NoneLiteral)));
    }
}
