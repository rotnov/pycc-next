//! MIR-level fallthrough proof for LLVM terminator placement.
//!
//! Extracted from `exception.rs` per AGENTS.md's file-decomposition rule.

use pycc_mir::{MirExpr, MirStmt};

/// Mirrors the type checker's fallthrough proof at MIR level. Structured
/// exception code generation sometimes leaves an LLVM continuation block
/// whose no-exception edge is statically impossible (for example,
/// `try: raise ... finally: cleanup`). Such a block still needs an LLVM
/// terminator even though it cannot be reached at runtime.
pub(crate) fn block_always_terminates(body: &[MirStmt]) -> bool {
    for stmt in body {
        let terminates = match stmt {
            MirStmt::Return(_)
            // Part 2 of #1175 (#1179): a buffer sub-range egress is a
            // `return`, so it terminates its block exactly as `Return`
            // does. Classified explicitly rather than left to inherit any
            // grouping, because a wrong answer here is a missing LLVM
            // terminator that still compiles.
            | MirStmt::ReturnBufferSlice { .. }
            | MirStmt::Raise { .. }
            | MirStmt::RaiseFrom { .. }
            // Part 9 of #1371: `raise o` always raises, whatever `o` is.
            | MirStmt::ObjRaise { .. }
            | MirStmt::Reraise
            | MirStmt::Unreachable => true,
            MirStmt::If { body, orelse, .. } => {
                !orelse.is_empty() & block_always_terminates(body) & block_always_terminates(orelse)
            }
            MirStmt::Seq(stmts) => block_always_terminates(stmts),
            // #1370: mirrors `pycc_hir::return_coverage`'s `While` arm. A
            // constant-true loop with no `break` bound to it leaves only
            // through a `return` or a raise in its body, so the block after
            // it is unreachable and gets an `unreachable` terminator.
            MirStmt::While { test, body } => is_constant_true(test) & !body_breaks_loop(body),
            // Mirrors `pycc_hir::return_coverage`'s `LoopElse` arm.
            MirStmt::LoopElse { loop_stmt, orelse } => {
                let breaks = loop_body(loop_stmt).is_some_and(body_breaks_loop);
                !breaks
                    & (block_always_terminates(std::slice::from_ref(loop_stmt.as_ref()))
                        | block_always_terminates(orelse))
            }
            // Control continues after (or at the head of) the loop.
            MirStmt::Break | MirStmt::Continue => false,
            MirStmt::Try {
                body,
                handlers,
                orelse,
                finalbody,
            }
            | MirStmt::TryStar {
                body,
                handlers,
                orelse,
                finalbody,
            } => {
                // These are pure structural predicates. Non-short-circuit boolean
                // operators keep every component explicit to coverage tooling and
                // make the fallthrough proof auditable as a complete truth table.
                // `except*` shares `Try`'s exact fallthrough shape: normal path
                // (body or `else`) plus every handler must terminate, unless
                // `finally` already does.
                let normal_path_terminates = block_always_terminates(body)
                    | ((!orelse.is_empty()) & block_always_terminates(orelse));
                let mut handled_paths_terminate = true;
                for handler in handlers {
                    handled_paths_terminate &= block_always_terminates(&handler.body);
                }
                block_always_terminates(finalbody)
                    | (normal_path_terminates & handled_paths_terminate)
            }
            MirStmt::ExprStmt(_)
            | MirStmt::Assign { .. }
            | MirStmt::NoOp
            | MirStmt::ReleaseName(_)
            | MirStmt::Yield(_)
            | MirStmt::GenReturn(_)
            | MirStmt::ForGen { .. }
            | MirStmt::ForRange { .. }
            | MirStmt::ForList { .. }
            | MirStmt::ForObject { .. }
            | MirStmt::DictSet { .. }
            | MirStmt::BufferSet { .. }
            | MirStmt::ListSet { .. }
            | MirStmt::ForDict { .. }
            | MirStmt::ForSet { .. }
            | MirStmt::ListCompAssign { .. }
            | MirStmt::DictCompAssign { .. }
            | MirStmt::SetCompAssign { .. }
            // #1291: the success path falls through. A failed nested foreign
            // import either raises a pycc exception to the innermost target
            // (#1293) or returns from the entry point on its own edge.
            | MirStmt::ForeignImport { .. }
            // Part 2c of #1371 / #1457: a raising `__delitem__`,
            // `__setattr__` or `__delattr__` leaves on the foreign-failure
            // edge; the success path falls through.
            | MirStmt::ObjDelSlice { .. }
            | MirStmt::ObjAttrSet { .. }
            | MirStmt::ObjDelAttr { .. }
            | MirStmt::ObjSetItem { .. }
            | MirStmt::ObjDelItem { .. }
            | MirStmt::AttrSet { .. } => false,
        };
        if terminates {
            return true;
        }
    }
    false
}

/// The MIR form of `pycc_hir::body_breaks_loop`: whether `body` holds a
/// `break` bound to the loop whose body it is (not one inside a nested
/// loop's body; a nested loop's `else:` clause does count).
pub(crate) fn body_breaks_loop(body: &[MirStmt]) -> bool {
    body.iter().any(|stmt| match stmt {
        MirStmt::Break => true,
        MirStmt::If { body, orelse, .. } => body_breaks_loop(body) || body_breaks_loop(orelse),
        MirStmt::Seq(stmts) => body_breaks_loop(stmts),
        MirStmt::Try {
            body,
            handlers,
            orelse,
            finalbody,
        }
        | MirStmt::TryStar {
            body,
            handlers,
            orelse,
            finalbody,
        } => {
            body_breaks_loop(body)
                || handlers.iter().any(|h| body_breaks_loop(&h.body))
                || body_breaks_loop(orelse)
                || body_breaks_loop(finalbody)
        }
        MirStmt::LoopElse { orelse, .. } => body_breaks_loop(orelse),
        _ => false,
    })
}

/// The body of a MIR loop statement, or `None` for any other statement.
pub(crate) fn loop_body(stmt: &MirStmt) -> Option<&[MirStmt]> {
    match stmt {
        MirStmt::While { body, .. }
        | MirStmt::ForRange { body, .. }
        | MirStmt::ForList { body, .. }
        | MirStmt::ForObject { body, .. }
        | MirStmt::ForDict { body, .. }
        | MirStmt::ForSet { body, .. }
        | MirStmt::ForGen { body, .. } => Some(body),
        _ => None,
    }
}

/// The MIR form of `pycc_hir::return_coverage`'s constant-true test: the
/// literal `True` or a non-zero integer literal.
fn is_constant_true(test: &MirExpr) -> bool {
    matches!(test, MirExpr::BoolLiteral(true)) | matches!(test, MirExpr::IntLiteral(n) if *n != 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loop_returning(test: MirExpr) -> Vec<MirStmt> {
        vec![MirStmt::While {
            test,
            body: vec![MirStmt::Return(Some(MirExpr::IntLiteral(1)))],
        }]
    }

    /// #1370: only a constant-true loop test makes the loop terminal.
    #[test]
    fn only_a_constant_true_loop_test_terminates() {
        assert!(block_always_terminates(&loop_returning(
            MirExpr::BoolLiteral(true)
        )));
        assert!(block_always_terminates(&loop_returning(
            MirExpr::IntLiteral(1)
        )));
        assert!(!block_always_terminates(&loop_returning(
            MirExpr::BoolLiteral(false)
        )));
        assert!(!block_always_terminates(&loop_returning(
            MirExpr::IntLiteral(0)
        )));
        assert!(!block_always_terminates(&loop_returning(
            MirExpr::FloatLiteral(1.0)
        )));
    }
}
