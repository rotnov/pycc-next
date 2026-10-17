//! `break`/`continue` across a `finally` clause, and the loop `else:`
//! wrapper shared by the `while` and `for` lowering arms.
//!
//! A `break` or `continue` inside a `try` body, handler or `else:` clause
//! whose `try` also has a `finally:` must run that `finally` block before it
//! reaches its loop target, as CPython does. Codegen routes only `return`
//! and exceptions through a pending `finally`, so lowering rewrites such a
//! `try` ([`route_jumps_through_finally`]): the jump records itself in a
//! flag and leaves a one-iteration loop wrapped around the guarded suites,
//! the `finally` block runs as for any other exit, and the jump is made
//! after it. An `except*` `try` still refuses the jump with a `C0001`.

use crate::{CmpOpKind, HirExpr, HirStmt, unsupported};
use pycc_ast::{ExceptHandler, Stmt};
use pycc_diag::Diagnostic;
use std::cell::Cell;

thread_local! {
    /// Numbers the flag of each rewritten `try`.
    static NEXT_FLAG: Cell<usize> = const { Cell::new(0) };
}

/// The flag value a `break` records.
const BREAK: i64 = 1;
/// The flag value a `continue` records.
const CONTINUE: i64 = 2;

/// `try` (not `except*`) with a non-empty `finally:` whose guarded suites
/// hold a `break`/`continue` bound to an enclosing loop, rewritten so the
/// `finally` block runs before the jump:
///
/// ```text
/// if True:
///     _pycc_jump_N = 0
///     try:
///         while True:
///             try: BODY except ...: ... else: ...   # each jump: flag, break
///             break
///     finally:
///         FINALLY
///     if _pycc_jump_N == 1: break
///     if _pycc_jump_N == 2: continue
/// ```
///
/// Any other statement is returned unchanged.
pub(super) fn route_jumps_through_finally(stmt: HirStmt) -> HirStmt {
    let HirStmt::Try {
        body,
        handlers,
        orelse,
        finalbody,
    } = stmt
    else {
        return stmt;
    };
    let jumps = crate::body_jumps_in_loop(&body)
        || handlers.iter().any(|handler| crate::body_jumps_in_loop(&handler.body))
        || crate::body_jumps_in_loop(&orelse);
    if finalbody.is_empty() || !jumps {
        return HirStmt::Try {
            body,
            handlers,
            orelse,
            finalbody,
        };
    }
    let flag = NEXT_FLAG.with(|next| {
        let n = next.get();
        next.set(n + 1);
        format!("_pycc_jump_{n}")
    });
    let body = retarget(body, &flag);
    let handlers = handlers
        .into_iter()
        .map(|mut handler| {
            handler.body = retarget(std::mem::take(&mut handler.body), &flag);
            handler
        })
        .collect::<Vec<_>>();
    let orelse = retarget(orelse, &flag);
    let guarded = if handlers.is_empty() && orelse.is_empty() {
        body
    } else {
        vec![HirStmt::Try {
            body,
            handlers,
            orelse,
            finalbody: Vec::new(),
        }]
    };
    let mut once = guarded;
    once.push(HirStmt::Break);
    let jump_if = |value: i64, jump: HirStmt| HirStmt::If {
        test: HirExpr::Compare {
            op: CmpOpKind::Eq,
            left: Box::new(HirExpr::Name(flag.clone())),
            right: Box::new(HirExpr::IntLiteral(value)),
        },
        body: vec![jump],
        orelse: Vec::new(),
    };
    HirStmt::If {
        test: HirExpr::BoolLiteral(true),
        body: vec![
            set_flag(&flag, 0),
            HirStmt::Try {
                body: vec![HirStmt::While {
                    test: HirExpr::BoolLiteral(true),
                    body: once,
                }],
                handlers: Vec::new(),
                orelse: Vec::new(),
                finalbody,
            },
            jump_if(BREAK, HirStmt::Break),
            jump_if(CONTINUE, HirStmt::Continue),
        ],
        orelse: Vec::new(),
    }
}

fn set_flag(flag: &str, value: i64) -> HirStmt {
    HirStmt::Assign {
        target: flag.to_string(),
        value: HirExpr::IntLiteral(value),
    }
}

/// `body` with every `break`/`continue` bound to the loop enclosing it
/// (under [`crate::body_jumps_in_loop`]'s binding rule) replaced by setting
/// `flag` and breaking out of the one-iteration loop.
fn retarget(body: Vec<HirStmt>, flag: &str) -> Vec<HirStmt> {
    let mut out = Vec::with_capacity(body.len());
    for stmt in body {
        match stmt {
            HirStmt::Break => {
                out.push(set_flag(flag, BREAK));
                out.push(HirStmt::Break);
            }
            HirStmt::Continue => {
                out.push(set_flag(flag, CONTINUE));
                out.push(HirStmt::Break);
            }
            HirStmt::If { test, body, orelse } => out.push(HirStmt::If {
                test,
                body: retarget(body, flag),
                orelse: retarget(orelse, flag),
            }),
            HirStmt::Match { subject, cases } => out.push(HirStmt::Match {
                subject,
                cases: cases
                    .into_iter()
                    .map(|mut case| {
                        case.body = retarget(std::mem::take(&mut case.body), flag);
                        case
                    })
                    .collect(),
            }),
            HirStmt::Try {
                body,
                handlers,
                orelse,
                finalbody,
            } => out.push(HirStmt::Try {
                body: retarget(body, flag),
                handlers: retarget_handlers(handlers, flag),
                orelse: retarget(orelse, flag),
                finalbody: retarget(finalbody, flag),
            }),
            HirStmt::TryStar {
                body,
                handlers,
                orelse,
                finalbody,
            } => out.push(HirStmt::TryStar {
                body: retarget(body, flag),
                handlers: retarget_handlers(handlers, flag),
                orelse: retarget(orelse, flag),
                finalbody: retarget(finalbody, flag),
            }),
            HirStmt::LoopElse { loop_stmt, orelse } => out.push(HirStmt::LoopElse {
                loop_stmt,
                orelse: retarget(orelse, flag),
            }),
            other => out.push(other),
        }
    }
    out
}

fn retarget_handlers(
    handlers: Vec<crate::HirExceptHandler>,
    flag: &str,
) -> Vec<crate::HirExceptHandler> {
    handlers
        .into_iter()
        .map(|mut handler| {
            handler.body = retarget(std::mem::take(&mut handler.body), flag);
            handler
        })
        .collect()
}

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
