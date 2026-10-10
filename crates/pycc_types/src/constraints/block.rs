//! Statement-level constraint collection (`collect_block_constraints`).

use super::*;

pub(crate) fn collect_block_constraints(
    signatures: &HashMap<String, SignatureTerms>,
    parents: &mut Vec<usize>,
    concrete: &mut Vec<Option<Ty>>,
    constraints: &mut SolverConstraints,
    env: &mut ConstraintEnvironment<'_, '_>,
    body: &[HirStmt],
    return_term: Option<TypeTerm>,
) -> Result<(), Diagnostic> {
    // #1476: the narrowing overlay is updated after each statement at the
    // top of the next iteration, because several arms below `continue`.
    let mut previous: Option<&HirStmt> = None;
    for stmt in body {
        if let Some(previous) = previous {
            object_narrow::after_statement(signatures, env, previous);
        }
        object_narrow::before_statement(env, stmt);
        previous = Some(stmt);
        super::block_stmt::collect_stmt_constraints(
            signatures,
            parents,
            concrete,
            constraints,
            env,
            stmt,
            return_term.clone(),
        )
        .map_err(|diagnostic| crate::locate::place_stmt(diagnostic, stmt))?;
    }
    // The last statement's update, so a block's final environment (a `try`
    // body's, which its `else` continues) reflects every rebinding in it.
    if let Some(previous) = previous {
        object_narrow::after_statement(signatures, env, previous);
    }
    Ok(())
}
