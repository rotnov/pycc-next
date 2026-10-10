//! `break`/`continue` across a `finally` clause, and the loop `else:`
//! wrapper shared by the `while` and `for` lowering arms.
//!
//! A `break` or `continue` inside a `try` body, handler or `else:` clause
//! whose `try` also has a `finally:` must run that `finally` block before it
//! reaches its loop target, as CPython does. Codegen routes only `return`
//! through a pending `finally` today, so lowering refuses such a jump with
//! a `C0001` instead of compiling code that would skip the `finally` block.

use crate::{HirStmt, unsupported};
use pycc_ast::{ExceptHandler, Stmt};
use pycc_diag::Diagnostic;

/// Refuses a `break`/`continue` in `body` that would leave a `try` whose
/// `finally:` clause is non-empty. `body` is one of that `try`'s guarded
/// suites (its body, a handler body, or its `else:` clause). A jump inside
/// a loop nested in `body` stays inside that loop and is fine; a jump in
/// such a loop's own `else:` clause is not nested in it and is checked.
pub(super) fn refuse_jump_across_finally(body: &[Stmt]) -> Result<(), Diagnostic> {
    for stmt in body {
        match stmt {
            Stmt::Break(_) => {
                return Err(unsupported(
                    "statement kind not supported yet: `break` out of a `try` block \
                     that has a `finally` clause",
                    pycc_ast::stmt_range(stmt),
                ));
            }
            Stmt::Continue(_) => {
                return Err(unsupported(
                    "statement kind not supported yet: `continue` out of a `try` block \
                     that has a `finally` clause",
                    pycc_ast::stmt_range(stmt),
                ));
            }
            Stmt::If(if_stmt) => {
                refuse_jump_across_finally(&if_stmt.body)?;
                for clause in &if_stmt.elif_else_clauses {
                    refuse_jump_across_finally(&clause.body)?;
                }
            }
            Stmt::While(while_stmt) => refuse_jump_across_finally(&while_stmt.orelse)?,
            Stmt::For(for_stmt) => refuse_jump_across_finally(&for_stmt.orelse)?,
            Stmt::Try(try_stmt) => {
                refuse_jump_across_finally(&try_stmt.body)?;
                for handler in &try_stmt.handlers {
                    let ExceptHandler::ExceptHandler(handler) = handler;
                    refuse_jump_across_finally(&handler.body)?;
                }
                refuse_jump_across_finally(&try_stmt.orelse)?;
                refuse_jump_across_finally(&try_stmt.finalbody)?;
            }
            Stmt::Match(match_stmt) => {
                for case in &match_stmt.cases {
                    refuse_jump_across_finally(&case.body)?;
                }
            }
            Stmt::With(with_stmt) => refuse_jump_across_finally(&with_stmt.body)?,
            _ => {}
        }
    }
    Ok(())
}

/// Wraps a lowered loop in [`HirStmt::LoopElse`] when its lowered `else:`
/// clause is non-empty, and returns it unchanged otherwise (`else: pass`
/// lowers to an empty clause and behaves exactly like no `else:` at all).
pub(super) fn with_else(loop_stmt: HirStmt, orelse: Vec<HirStmt>) -> HirStmt {
    if !orelse.is_empty() {
        HirStmt::LoopElse {
            loop_stmt: Box::new(loop_stmt),
            orelse,
        }
    } else {
        loop_stmt
    }
}
