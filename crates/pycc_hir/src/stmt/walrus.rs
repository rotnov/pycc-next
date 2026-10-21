//! PEP 572 (#774): where a walrus assignment may stand in a lowered
//! statement.

use crate::{CompIter, HirStmt};
use crate::expr::contains_named_expr;

/// PEP 572 (#774): whether `lowered`'s own immediate expression fields hold a
/// walrus outside the permitted placements; see `lower_stmt`'s comment above.
pub(super) fn violates_walrus_placement(lowered: &HirStmt) -> bool {
    match lowered {
        HirStmt::If { .. }
        | HirStmt::While { .. }
        | HirStmt::ExprStmt(_)
        | HirStmt::ForList { .. } => false,
        HirStmt::ForObject { iter, .. } => contains_named_expr(iter),
        HirStmt::Assign { value, .. } => contains_named_expr(value),
        HirStmt::AnnAssign { value, .. } => value.as_ref().is_some_and(contains_named_expr),
        HirStmt::ForRange {
            start, stop, step, ..
        } => contains_named_expr(start) || contains_named_expr(stop) || contains_named_expr(step),
        HirStmt::DictSet { key, value, .. } => {
            contains_named_expr(key) || contains_named_expr(value)
        }
        HirStmt::ListCompAssign {
            iter, cond, elt, ..
        } => {
            comp_iter_contains_named_expr(iter)
                || cond.as_deref().is_some_and(contains_named_expr)
                || contains_named_expr(elt)
        }
        HirStmt::DictCompAssign {
            iter,
            cond,
            key,
            value,
            ..
        } => {
            comp_iter_contains_named_expr(iter)
                || cond.as_deref().is_some_and(contains_named_expr)
                || contains_named_expr(key)
                || contains_named_expr(value)
        }
        HirStmt::SetCompAssign {
            iter, cond, elt, ..
        } => {
            comp_iter_contains_named_expr(iter)
                || cond.as_deref().is_some_and(contains_named_expr)
                || contains_named_expr(elt)
        }
        HirStmt::Return(value) => value.as_ref().is_some_and(contains_named_expr),
        HirStmt::AttrSet { base, value, .. } => {
            contains_named_expr(base) || contains_named_expr(value)
        }
        HirStmt::Match { subject, .. } => contains_named_expr(subject),
        // `handlers`' own only expression-shaped content is each handler's
        // own `body: Vec<HirStmt>`, already independently checked (see this
        // block's own doc comment above).
        // `del` never reaches here (`lower_stmt_expanded` lowers it, and
        // `stmt::del::lower_delete` refuses a walrus in a slice target).
        // A nested foreign import (#1291) holds only names.
        HirStmt::Try { .. }
        | HirStmt::TryStar { .. }
        | HirStmt::Delete { .. }
        | HirStmt::DeleteSlice { .. }
        | HirStmt::SetSlice { .. }
        | HirStmt::DeleteAttr { .. }
        | HirStmt::DeleteItem { .. }
        | HirStmt::ForeignImport { .. } => false,
        HirStmt::Raise { exc, cause } => {
            exc.as_ref().is_some_and(contains_named_expr)
                || cause.as_ref().is_some_and(contains_named_expr)
        }
        HirStmt::Break | HirStmt::Continue | HirStmt::Pos(_) => false,
        // The `else:` clause's statements were checked by their own
        // `lower_stmt` calls; only the wrapped loop's header remains.
        HirStmt::LoopElse { loop_stmt, .. } => violates_walrus_placement(loop_stmt),
    }
}

/// PEP 572 (#774): the one wording of the walrus-placement refusal, shared
/// by `lower_stmt` and `del::lower_delete` (a walrus in a slice `del`
/// target, Part 2c of #1371).
pub(crate) const WALRUS_PLACEMENT_MESSAGE: &str = "a walrus assignment (`:=`) is only supported in an `if`/`while` \
     condition or as a bare expression statement (#774)";

/// PEP 572 (#774): `CompIter`'s own `contains_named_expr` counterpart --
/// `CompIter::Range`'s `start`/`stop`/`step` are ordinary `HirExpr` fields
/// (e.g. `[x for x in range(n := 5)]`); `CompIter::Name` is a bare variable
/// name with nothing to walk. A walrus found here is rejected for the same
/// reason every other comprehension field is: #774's own explicit
/// permitted-scope-cut leaves comprehension-embedded walrus scoping
/// unimplemented, so this is a `core` gap recorded in the
/// conformance-breadth manifest, not a silent one.
pub(super) fn comp_iter_contains_named_expr(iter: &CompIter) -> bool {
    match iter {
        CompIter::Range { start, stop, step } => {
            contains_named_expr(start) || contains_named_expr(stop) || contains_named_expr(step)
        }
        CompIter::Iterable(iterable) => contains_named_expr(iterable),
        CompIter::Name(_) => false,
    }
}
