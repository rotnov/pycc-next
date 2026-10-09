//! The constraint solver's half of #1476 (Part 3 of #1387): an
//! `isinstance(o, C)` guard narrows an `object`-bound name back to a native
//! type for the reads it dominates, mirroring `crate::narrow` in the check
//! phase.
//!
//! The solver runs first and its per-function verdict wins (D-220), so a
//! guarded `o + 1` it read as `object` would refuse a body the check phase
//! admits. The overlay is `ConstraintEnvironment::narrowed`, consulted by
//! the `Name` arm, and is set and dropped at the same points as the check
//! phase's:
//!
//! - the `if` arm narrows the branch the guard admits ([`branch_target`]);
//! - a negated guard whose body definitely terminates narrows the rest of
//!   the block ([`after_statement`]);
//! - any statement that rebinds the name ends the narrowing after it, and a
//!   loop that rebinds it anywhere ends it before it, because its body can
//!   run after its own rebinding ([`before_statement`], the check phase's
//!   `apply_kill_prescan`);
//! - a `try` body is walked in order, each handler starts without the
//!   names the body rebinds anywhere, the `finally` without the names any
//!   path rebinds ([`kill_names`]), and the statements after the `try`
//!   keep what every fall-through path still narrows, as the check phase's
//!   `check_try_stmt` and `join_try_outcome` do.
//!
//! The recognizer and the class gate are `pycc_hir`'s, shared with the
//! check phase and the MIR lowering; only the scope lookups are this
//! module's.

use super::*;
use pycc_hir::{IsInstancePolarity, isinstance_narrow_target, isinstance_test};

/// `test` as an `isinstance` guard this module narrows: the name, the type
/// it narrows to, and the guard's polarity. `None` unless `isinstance` is
/// the builtin, the operand is definitely bound to `object`, and the class
/// is an unshadowed scalar or admissible compiled class.
fn guard(
    signatures: &HashMap<String, SignatureTerms>,
    env: &ConstraintEnvironment<'_, '_>,
    test: &HirExpr,
) -> Option<(String, Ty, IsInstancePolarity)> {
    let (name, class, polarity) = isinstance_test(test)?;
    let bound = |name: &str| {
        env.bindings.contains_key(name)
            || env.opaque_bindings.contains(name)
            || env.defs_rebound.contains(name)
    };
    if signatures.contains_key("isinstance")
        || bound("isinstance")
        || bound(class)
        || signatures.contains_key(class)
        || env.maybe_bindings.contains(name)
        || !matches!(env.bindings.get(name), Some(Ok(Ty::Object)))
    {
        return None;
    }
    let inner = isinstance_narrow_target(
        class,
        |class| {
            env.class_defs
                .iter()
                .find(|(name, _)| name == class)
                .map(|(_, def)| def)
        },
        env.class_defs.iter().map(|(_, def)| def),
    )?;
    Some((name.to_string(), inner, polarity))
}

/// The `if` arm's narrowing: `(name, type, narrows the body)` -- `true` for
/// `isinstance(o, C)`, `false` for `not isinstance(o, C)`, which narrows
/// the `else` branch.
pub(super) fn branch_target(
    signatures: &HashMap<String, SignatureTerms>,
    env: &ConstraintEnvironment<'_, '_>,
    test: &HirExpr,
) -> Option<(String, Ty, bool)> {
    let (name, inner, polarity) = guard(signatures, env, test)?;
    Some((name, inner, polarity == IsInstancePolarity::Positive))
}

/// The narrowed type of a read of `name`, if the overlay holds one.
pub(super) fn narrowed_read(env: &ConstraintEnvironment<'_, '_>, name: &str) -> Option<TypeTerm> {
    env.narrowed.get(name).map(|ty| Ok(ty.clone()))
}

/// Whether `value` is a bare read of a narrowed name: a first binding from
/// it keeps the `object` term, as `crate::check_assignment_boxing` keeps the
/// slot `object`, so a branch binding the same local from the unnarrowed
/// name still joins.
pub(super) fn is_bare_narrowed_read(env: &ConstraintEnvironment<'_, '_>, value: &HirExpr) -> bool {
    matches!(value, HirExpr::Name(name) if env.narrowed.contains_key(name))
}

/// Before collecting `stmt`: a loop body runs again after a rebinding inside
/// it, so every name a loop rebinds anywhere stops being narrowed for the
/// whole loop. A `try` is not killed here: its body is walked sequentially
/// and [`kill_names`] drops the body's rebindings from each handler, as the
/// check phase does.
pub(super) fn before_statement(env: &mut ConstraintEnvironment<'_, '_>, stmt: &HirStmt) {
    if env.narrowed.is_empty() {
        return;
    }
    if matches!(
        stmt,
        HirStmt::While { .. }
            | HirStmt::ForRange { .. }
            | HirStmt::ForList { .. }
            | HirStmt::ForObject { .. }
    ) {
        kill(env, stmt);
    }
}

/// Drops every name in `names` from the overlay: a `try` handler can run
/// after any rebinding in the body, and a `finally` after any rebinding on
/// any path, so those names are not narrowed there.
pub(super) fn kill_names(env: &mut ConstraintEnvironment<'_, '_>, names: &HashSet<String>) {
    kill_names_in(&mut env.narrowed, names);
}

/// [`kill_names`] on an overlay held apart from its environment.
pub(super) fn kill_names_in(narrowed: &mut HashMap<String, Ty>, names: &HashSet<String>) {
    narrowed.retain(|name, _| !names.contains(name));
}

/// After collecting `stmt`: a name it rebinds anywhere is no longer
/// narrowed, and a negated guard whose body definitely terminates and whose
/// `else` leaves the name alone (`pycc_hir::continuation_narrows`) narrows
/// the rest of the block.
pub(super) fn after_statement(
    signatures: &HashMap<String, SignatureTerms>,
    env: &mut ConstraintEnvironment<'_, '_>,
    stmt: &HirStmt,
) {
    // A `try` sets the overlay after it itself, from its fall-through paths
    // (`try_stmt::collect_try_constraints`).
    if !env.narrowed.is_empty() && !matches!(stmt, HirStmt::Try { .. } | HirStmt::TryStar { .. }) {
        kill(env, stmt);
    }
    if let HirStmt::If { test, body, orelse } = stmt
        && let Some((name, inner, IsInstancePolarity::Negated)) = guard(signatures, env, test)
        && pycc_hir::continuation_narrows(body, orelse, &name)
    {
        env.narrowed.insert(name, inner);
    }
}

fn kill(env: &mut ConstraintEnvironment<'_, '_>, stmt: &HirStmt) {
    for name in pycc_hir::killed_names(std::slice::from_ref(stmt)) {
        env.narrowed.remove(&name);
    }
}
