//! Path-precise definite assignment around loops and jumps.
//!
//! The statement after a loop is reached by two kinds of edge: the loop's
//! normal exit, which runs the `else:` clause when there is one, and every
//! `break` bound to the loop, each with the bindings in effect where it
//! jumps. A name is `Definitely` bound after the loop only when it is on
//! every such edge, so
//!
//! ```python
//! for d in range(2, n):
//!     if n % d == 0:
//!         break
//! else:
//!     return -1
//! return d
//! ```
//!
//! reads `d` definitely bound: the normal exit returns, and the one `break`
//! is in a body that bound the loop target. A `continue` flows back to the
//! loop head, whose state the loop join (`join_loop_body`) already treats
//! conservatively (a name new to the loop is never bound at the head), so a
//! `continue` edge needs no record of its own; it only drops its path from
//! the join after the `if`/`try` it ends ([`refine_if_join`],
//! `exception::try_join`).
//!
//! Only definiteness is refined. Types are fixed by the conservative joins
//! that still run first, and a name is promoted to `Definitely` only when
//! every edge binds it with exactly the type the join already settled on,
//! so codegen's one slot per name is never asked to hold two types. The
//! narrowing overlay is untouched, which keeps it equal to `pycc_mir`'s.

use crate::{BindingState, Environment};
use pycc_diag::Diagnostic;
use pycc_hir::{
    HirStmt, block_never_completes, body_breaks_loop, loop_body, loop_else_as_if,
    loop_never_ends_normally,
};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

type Bindings = HashMap<String, BindingState>;

/// The bindings recorded at each `break` of every loop being checked, one
/// frame per loop, innermost last. Clones share the frames, so a `break`
/// checked in a branch's cloned environment reaches its loop's frame.
#[derive(Debug, Default, Clone)]
pub(crate) struct LoopExits(Arc<Mutex<Vec<Vec<Bindings>>>>);

impl LoopExits {
    fn frames(&self) -> MutexGuard<'_, Vec<Vec<Bindings>>> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Opens a frame for a loop and returns its depth for [`Self::exit`].
    fn enter(&self) -> usize {
        let mut frames = self.frames();
        frames.push(Vec::new());
        frames.len() - 1
    }

    /// Closes the frame at `depth` and returns its `break` records. Frames
    /// above it, left open by a nested check that failed, are dropped too.
    fn exit(&self, depth: usize) -> Vec<Bindings> {
        let mut frames = self.frames();
        frames.truncate(depth + 1);
        frames.pop().unwrap_or_default()
    }

    /// Records `bindings` as the state at a `break` of the innermost loop.
    pub(crate) fn record_break(&self, bindings: &Bindings) {
        if let Some(frame) = self.frames().last_mut() {
            frame.push(bindings.clone());
        }
    }
}

/// Checks a loop (`check_arm`) and its `else:` clause `orelse`, then sets
/// every name's definiteness after the statement from the edges that reach
/// it: the normal exit, unless the loop never ends normally (`while True:`)
/// or `orelse` never completes, and each `break` the loop body recorded.
///
/// When a `break` can skip `orelse`, the clause is still checked into `env`
/// as the opaque `if` of [`loop_else_as_if`], exactly as `pycc_mir` lowers
/// it, so types and narrowing after the loop stay the same in both passes;
/// it is checked a second time, as plain code on a copy, only to learn the
/// bindings on the normal-exit edge.
pub(crate) fn check_loop(
    env: &mut Environment,
    loop_stmt: &HirStmt,
    orelse: &[HirStmt],
    check_arm: impl FnOnce(&mut Environment) -> Result<(), Diagnostic>,
    check_suite: impl Fn(&mut Environment, &[HirStmt]) -> Result<(), Diagnostic>,
) -> Result<(), Diagnostic> {
    let exits = env.loop_exits.clone();
    let depth = exits.enter();
    let checked = check_arm(env);
    let breaks = exits.exit(depth);
    checked?;
    let normal_exit = if orelse.is_empty() {
        env.bindings.clone()
    } else if loop_body(loop_stmt).is_some_and(body_breaks_loop) {
        let mut normal = env.clone();
        check_suite(&mut normal, orelse)?;
        check_suite(env, std::slice::from_ref(&loop_else_as_if(orelse)))?;
        normal.bindings
    } else {
        check_suite(env, orelse)?;
        env.bindings.clone()
    };
    let normal_reached = !loop_never_ends_normally(loop_stmt) && !block_never_completes(orelse);
    let edges: Vec<&Bindings> = breaks
        .iter()
        .chain(normal_reached.then_some(&normal_exit))
        .collect();
    refine_definiteness(env, &edges);
    Ok(())
}

/// After an `if` whose branches were joined into `env`, drops a branch that
/// never completes (it returns, raises, `break`s or `continue`s) from the
/// definiteness join: `if c: x = 1 / else: continue` leaves `x` bound.
pub(crate) fn refine_if_join(
    env: &mut Environment,
    body: &[HirStmt],
    orelse: &[HirStmt],
    body_env: &Environment,
    orelse_env: &Environment,
) {
    let edges: Vec<&Bindings> = [(body, body_env), (orelse, orelse_env)]
        .into_iter()
        .filter(|(suite, _)| !block_never_completes(suite))
        .map(|(_, branch_env)| &branch_env.bindings)
        .collect();
    if edges.len() == 1 {
        refine_definiteness(env, &edges);
    }
}

/// Makes each name in `env` `Definitely` bound when every edge binds it
/// definitely with the type `env` holds for it, and `Maybe` when some edge
/// does not bind it definitely. No edges means the point is unreachable,
/// and `env` is left as the conservative joins computed it.
fn refine_definiteness(env: &mut Environment, edges: &[&Bindings]) {
    if edges.is_empty() {
        return;
    }
    for (name, state) in env.bindings.iter_mut() {
        let ty = state.ty().clone();
        let mut definite = true;
        let mut same_ty = true;
        for edge in edges {
            match edge.get(name) {
                Some(BindingState::Definitely(edge_ty)) => same_ty &= *edge_ty == ty,
                _ => definite = false,
            }
        }
        if !definite {
            *state = BindingState::Maybe(ty);
        } else if same_ty {
            *state = BindingState::Definitely(ty);
        }
    }
}
