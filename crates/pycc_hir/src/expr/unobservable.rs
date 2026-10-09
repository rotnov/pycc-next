//! The one purity predicate three rules share: whether evaluating an
//! expression early, late or twice can be observed.
//!
//! Keyword binding (#1204, `docs/TYPE_SYSTEM.md` "Keyword argument
//! evaluation order") uses it to decide whether moving a value is safe,
//! augmented assignment (#1209, "Augmented assignment") uses it to decide
//! whether a subscript index may be read twice by the desugared assignment,
//! and chained assignment (#1213, "Chained assignment") uses it to decide
//! whether the value may be copied to every target instead of bound once to
//! a temporary. Widening it widens all three rules at once, so it must stay
//! limited to expressions that can neither raise differently nor have an
//! effect.

use pycc_ast::Expr;

/// Whether evaluating `value` early, late or twice cannot be observed: a
/// literal in the subset a parameter default admits, or a bare name.
pub(crate) fn is_unobservable(value: &Expr) -> bool {
    matches!(value, Expr::Name(_)) || crate::func::params::literal_default(value).is_some()
}
