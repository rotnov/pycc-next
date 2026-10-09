//! The `T0022` help for a comparison method whose `return NotImplemented`
//! widened its return to the CPython object (#1418).
//!
//! `pycc_hir`'s `class::method::lower_method` widens such a method's return
//! type to `Ty::Object` whatever its annotation says, as CPython's own
//! behaviour does. Since #1475 a native `return self.v == other.v` in the
//! same body is boxed into that `object` (`crate::object_box`), but a value
//! the object cannot hold (`return [self.v]`, an `Optional`) is refused as
//! an ordinary `T0022`, and the bare "expected `object`, found `list[int]`"
//! would leave the programmer who wrote `-> bool` wondering where the
//! `object` came from. Both phases'
//! per-function loops (the solver's in `constraints::signatures` and the
//! check phase's in `crate::module`) route a body's refusal through
//! [`widened_return_help`], so the help is attached whichever phase reports
//! first.

use pycc_diag::Diagnostic;
use pycc_hir::HirStmt;

/// `diagnostic` with [`pycc_hir::WIDENED_RETURN_HELP`] as its help when it is
/// a `T0022` return mismatch in a `body` that returns `NotImplemented`, and
/// `diagnostic` unchanged otherwise.
///
/// Only the two mismatch spellings qualify (the solver's "return type
/// mismatch: ..." and the check phase's "expected return type ..."): the
/// other `T0022`, "function `f` can exit without returning `object`", is
/// about a missing `return`, not a native value, and keeps its own help.
pub(crate) fn widened_return_help(body: &[HirStmt], diagnostic: Diagnostic) -> Diagnostic {
    let mismatch = diagnostic.message.starts_with("return type mismatch")
        || diagnostic.message.starts_with("expected return type");
    if diagnostic.code == "T0022" && mismatch && pycc_hir::body_returns_not_implemented(body) {
        diagnostic.with_help(pycc_hir::WIDENED_RETURN_HELP)
    } else {
        diagnostic
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pycc_diag::Span;
    use pycc_hir::HirExpr;

    fn mismatch(code: &'static str) -> Diagnostic {
        Diagnostic::error(code, "return type mismatch", Span::new(0, 0))
    }

    #[test]
    fn a_t0022_in_a_body_returning_not_implemented_gets_the_help() {
        let body = [HirStmt::Return(Some(HirExpr::NotImplemented))];
        let diagnostic = widened_return_help(&body, mismatch("T0022"));
        assert_eq!(
            diagnostic.help.as_deref(),
            Some(pycc_hir::WIDENED_RETURN_HELP)
        );
    }

    #[test]
    fn another_code_or_body_keeps_the_diagnostic_unchanged() {
        let widened = [HirStmt::Return(Some(HirExpr::NotImplemented))];
        assert_eq!(widened_return_help(&widened, mismatch("T0021")).help, None);
        let plain = [HirStmt::Return(Some(HirExpr::NoneLiteral))];
        assert_eq!(widened_return_help(&plain, mismatch("T0022")).help, None);
        let fall_off = Diagnostic::error(
            "T0022",
            "function `C.__eq__` can exit without returning `object`",
            Span::new(0, 0),
        );
        assert_eq!(widened_return_help(&widened, fall_off).help, None);
    }
}
