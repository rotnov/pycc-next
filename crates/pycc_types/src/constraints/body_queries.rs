//! Structural questions about a function body the solver asks: whether it
//! returns, and whether it binds names.

use super::*;

pub(crate) fn contains_return(body: &[HirStmt]) -> bool {
    body.iter().any(|stmt| match stmt {
        HirStmt::Return(_) => true,
        HirStmt::If { body, orelse, .. } => contains_return(body) || contains_return(orelse),
        HirStmt::While { body, .. }
        | HirStmt::ForRange { body, .. }
        | HirStmt::ForList { body, .. }
        | HirStmt::ForObject { body, .. } => contains_return(body),
        HirStmt::LoopElse { loop_stmt, orelse } => {
            contains_return(std::slice::from_ref(loop_stmt.as_ref())) || contains_return(orelse)
        }
        HirStmt::Match { cases, .. } => cases.iter().any(|case| contains_return(&case.body)),
        HirStmt::ExprStmt(_)
        | HirStmt::Assign { .. }
        | HirStmt::AnnAssign { .. }
        | HirStmt::DictSet { .. }
        | HirStmt::AttrSet { .. }
        | HirStmt::ListCompAssign { .. }
        | HirStmt::SetCompAssign { .. }
        | HirStmt::DictCompAssign { .. }
        | HirStmt::Delete { .. }
        | HirStmt::DeleteSlice { .. }
        | HirStmt::SetSlice { .. }
        | HirStmt::DeleteAttr { .. }
        | HirStmt::DeleteItem { .. }
        | HirStmt::ForeignImport { .. }
        | HirStmt::Break
        | HirStmt::Continue
        | HirStmt::Raise { .. } => false,
        HirStmt::Try {
            body,
            handlers,
            orelse,
            finalbody,
        }
        | HirStmt::TryStar {
            body,
            handlers,
            orelse,
            finalbody,
        } => {
            contains_return(body)
                || handlers.iter().any(|h| contains_return(&h.body))
                || contains_return(orelse)
                || contains_return(finalbody)
        }
    })
}

/// Issue #118 Part 1: returns true if any statement in `body` introduces
/// a new binding (assignment, annotated assignment, comprehension assignment,
/// or dict/set item assignment, including nested inside if/while/for). Used
/// to skip the expensive `env.clone()` + `join_if_branches` path when neither
/// branch of an `if` assigns anything -- the common case for guard-only ifs.
pub(crate) fn introduces_bindings(body: &[HirStmt]) -> bool {
    body.iter().any(|stmt| match stmt {
        HirStmt::Assign { .. }
        | HirStmt::AnnAssign { .. }
        | HirStmt::DictSet { .. }
        | HirStmt::AttrSet { .. }
        | HirStmt::ListCompAssign { .. }
        | HirStmt::SetCompAssign { .. }
        // #1244: a `del` changes a binding state, so the in-place fast
        // paths that assume an unchanged environment must not be taken.
        | HirStmt::Delete { .. }
        // #1291: a nested foreign import binds its names.
        | HirStmt::ForeignImport { .. }
        | HirStmt::DictCompAssign { .. } => true,
        HirStmt::If { body, orelse, .. } => {
            introduces_bindings(body) || introduces_bindings(orelse)
        }
        HirStmt::While { body, .. } => introduces_bindings(body),
        HirStmt::ForRange { body, .. } => introduces_bindings(body),
        HirStmt::ForList { body, .. } | HirStmt::ForObject { body, .. } => {
            introduces_bindings(body)
        }
        HirStmt::LoopElse { loop_stmt, orelse } => {
            introduces_bindings(std::slice::from_ref(loop_stmt.as_ref()))
                || introduces_bindings(orelse)
        }
        HirStmt::Match { cases, .. } => cases.iter().any(|case| introduces_bindings(&case.body)),
        // A slice or attribute `del` (Part 2c of #1371, #1457) changes no
        // binding.
        HirStmt::Return(_)
        | HirStmt::ExprStmt(_)
        | HirStmt::DeleteSlice { .. }
        | HirStmt::SetSlice { .. }
        | HirStmt::DeleteAttr { .. }
        | HirStmt::DeleteItem { .. }
        | HirStmt::Break
        | HirStmt::Continue => false,
        HirStmt::Try {
            body,
            handlers,
            orelse,
            finalbody,
        }
        | HirStmt::TryStar {
            body,
            handlers,
            orelse,
            finalbody,
        } => {
            introduces_bindings(body)
                || handlers.iter().any(|h| introduces_bindings(&h.body))
                || introduces_bindings(orelse)
                || introduces_bindings(finalbody)
        }
        HirStmt::Raise { .. } => false,
    })
}
