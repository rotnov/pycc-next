//! Return coverage: whether a statement list can reach its end.
//!
//! [`block_always_returns`] decides `pycc_types`' `T0022` "function `f` can
//! exit without returning `T`" check, and the fall-through joins after a
//! `try` statement drop a path it says never reaches the statement after
//! it: the check phase's `exception::try_join`, the constraint solver's
//! `constraints::try_stmt`, and `pycc_mir`'s `Try` lowering. It lives here,
//! in their common dependency, so the three joins cannot disagree on which
//! paths fall through (moved from `pycc_types::return_coverage` by #1476,
//! the D-205 shared-predicate precedent).

use crate::{HirExpr, HirStmt, body_breaks_loop, loop_body};

/// Whether every path through `body` leaves it by `return`, `raise`, or a
/// loop that never completes, so no statement after `body` runs.
pub fn block_always_returns(body: &[HirStmt]) -> bool {
    block_exits(body, false)
}

/// Whether no path through `body` reaches its end: every path leaves by
/// `return`, `raise`, a loop that never completes, or a `break`/`continue`
/// bound to a loop enclosing `body`. [`block_always_returns`] plus jumps.
///
/// `pycc_types`' definite-assignment joins use it to drop a path that jumps
/// away (`else: continue`, `except E: continue`) from the paths reaching the
/// statement after `body`. A jump's own state is not lost: a `break` joins
/// at the loop's exit and a `continue` flows back to the loop head, whose
/// state the loop join already treats conservatively.
pub fn block_never_completes(body: &[HirStmt]) -> bool {
    block_exits(body, true)
}

/// Whether `stmt` is a loop that can only be left through a `break`,
/// `return` or raise: a `while` whose test is constant true.
pub fn loop_never_ends_normally(stmt: &HirStmt) -> bool {
    matches!(stmt, HirStmt::While { test, .. } if is_constant_true(test))
}

/// [`block_always_returns`] when `jumps` is false, [`block_never_completes`]
/// when it is true. A loop nested in `body` is never recursed into with
/// `jumps`, so a `break` in its body (bound to it, not to the loop around
/// `body`) is never mistaken for one leaving `body`.
fn block_exits(body: &[HirStmt], jumps: bool) -> bool {
    let block_always_returns = |body: &[HirStmt]| block_exits(body, jumps);
    for stmt in body {
        let returns = match stmt {
            HirStmt::Return(_) => true,
            HirStmt::If { body, orelse, .. } => {
                !orelse.is_empty() & block_always_returns(body) & block_always_returns(orelse)
            }
            HirStmt::ExprStmt(_)
            | HirStmt::Assign { .. }
            | HirStmt::AnnAssign { .. }
            | HirStmt::ForRange { .. }
            | HirStmt::ForList { .. }
            | HirStmt::ForObject { .. }
            | HirStmt::DictSet { .. }
            | HirStmt::AttrSet { .. }
            // #1244: a `del` neither returns nor raises.
            | HirStmt::Delete { .. }
            // Part 2c of #1371 / #1457: a slice or attribute `del` never
            // returns (a raising `__delitem__`/`__delattr__` takes the
            // failure edge, like any object call).
            | HirStmt::DeleteSlice { .. }
            | HirStmt::SetSlice { .. }
            | HirStmt::DeleteAttr { .. }
            | HirStmt::DeleteItem { .. }
            // #1291: a nested foreign import never returns. Its failure is
            // either a pycc raise (an `ImportError` by #1293, any other
            // exception inside a module-level `try` by Part 1 of #1096) or a
            // direct exit from `Py_mod_exec` (a non-`ImportError` outside
            // every `try`).
            | HirStmt::ForeignImport { .. }
            // PR-12 Task 3 (D-117): a comprehension statement never contains a
            // `return` (its `elt`/`cond`/`key`/`value` are expressions, not
            // statements), so it can never make a block always return, exactly
            // like `Assign`/`ForList`/`DictSet` above.
            | HirStmt::ListCompAssign { .. }
            | HirStmt::SetCompAssign { .. }
            | HirStmt::DictCompAssign { .. } => false,
            // #1370: a loop whose test is a constant true value never
            // completes normally -- control leaves it only through a
            // `return`, a raise, or a `break` bound to it. Without such a
            // `break` the statement after it is unreachable. Any other
            // test may be false on entry or later, so the loop falls
            // through.
            //
            // `pycc_hir::definitely_terminates` (guard-clause narrowing)
            // is not a mirror of this predicate and stays unchanged: it
            // deliberately recognizes only `return` and a two-armed `if`
            // (not even `raise`), which is sound by omission.
            HirStmt::While { test, body } => is_constant_true(test) & !body_breaks_loop(body),
            // A loop with `else:` reaches the statement after it through a
            // `break` bound to the loop, or through the `else:` clause once
            // the loop ends normally. Without a `break` it never falls
            // through when the loop itself never ends or the clause never
            // falls through.
            HirStmt::LoopElse { loop_stmt, orelse } => {
                let breaks = loop_body(loop_stmt).is_some_and(body_breaks_loop);
                !breaks
                    & (block_always_returns(std::slice::from_ref(loop_stmt.as_ref()))
                        | block_always_returns(orelse))
            }
            // A `break`/`continue` leaves the rest of its suite unreached,
            // but control continues after (or at the head of) the loop, so
            // it is not a return. Without `jumps`, answering `false` is the
            // conservative direction for every caller: a `try` join keeps
            // the path, and a constant-true loop is classified by its own
            // arm above. With `jumps` the caller asks exactly whether the
            // end of `body` is reached, which a jump never does.
            HirStmt::Break | HirStmt::Continue => jumps,
            HirStmt::Pos(_) => false,
            // A raise transfers control to an exception handler/caller and
            // cannot fall through to the function's implicit return point.
            HirStmt::Raise { .. } => true,
            HirStmt::Match { cases, .. } => {
                let mut all_cases_return = !cases.is_empty();
                for case in cases {
                    all_cases_return &= block_always_returns(&case.body);
                }
                all_cases_return
            }
            // `except*` shares `Try`'s termination shape exactly: a terminal
            // `finally` replaces every earlier outcome, and otherwise the
            // normal path (body or `else`) and every matched subgroup's
            // handler must all terminate.
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
                // A terminal `finally` replaces every earlier outcome. Otherwise
                // the normal path must terminate either in the try body itself or
                // in its `else`, and every matching handler must terminate. With
                // no handlers, an exception simply propagates to the caller and
                // is already a terminal path.
                let normal_path_terminates = block_always_returns(body)
                    | ((!orelse.is_empty()) & block_always_returns(orelse));
                let mut handled_paths_terminate = true;
                for handler in handlers {
                    handled_paths_terminate &= block_always_returns(&handler.body);
                }
                block_always_returns(finalbody)
                    | (normal_path_terminates & handled_paths_terminate)
            }
        };
        if returns {
            return true;
        }
    }
    false
}

/// True for a loop test pycc treats as constant true: the literal `True`,
/// or a non-zero integer literal (`while 1:`). A string or float literal
/// test is also always truthy at run time, but pycc does not treat it as
/// constant, and a computed expression may be false; both keep reporting
/// `T0022` -- the check stays conservative.
fn is_constant_true(test: &HirExpr) -> bool {
    matches!(test, HirExpr::BoolLiteral(true)) | matches!(test, HirExpr::IntLiteral(n) if *n != 0)
}
