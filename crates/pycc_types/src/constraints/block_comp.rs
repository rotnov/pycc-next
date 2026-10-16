//! A comprehension assigned to a name (`HirStmt::ListCompAssign`,
//! `SetCompAssign`, `DictCompAssign`): the loop variable's binding, the
//! condition, the element terms and the container term the target is
//! bound to. Split out of [`super::block_stmt::collect_stmt_constraints`].

use super::*;

pub(super) fn collect_comp_assign_constraints(
    signatures: &HashMap<String, SignatureTerms>,
    parents: &mut Vec<usize>,
    concrete: &mut Vec<Option<Ty>>,
    constraints: &mut SolverConstraints,
    env: &mut ConstraintEnvironment<'_, '_>,
    stmt: &HirStmt,
) -> Result<(), Diagnostic> {
    match stmt {
        HirStmt::ListCompAssign {
            target,
            var,
            iter,
            cond,
            elt,
        }
        | HirStmt::SetCompAssign {
            target,
            var,
            iter,
            cond,
            elt,
        } => {
            bind_comp_loop_var(
                signatures,
                parents,
                concrete,
                &mut constraints.deferred,
                env,
                var,
                iter,
            )?;
            if let Some(cond) = cond {
                collect_expr_constraints(
                    signatures,
                    parents,
                    concrete,
                    &mut constraints.deferred,
                    env,
                    cond,
                )?;
            }
            let elt_term = collect_expr_constraints(
                signatures,
                parents,
                concrete,
                &mut constraints.deferred,
                env,
                elt,
            )?;
            // Round-11 review finding 2: this solver binds no term for a
            // comprehension `target` at all, so before this call the
            // name kept whatever it was bound to before -- including an
            // artifact-owned `Ty::MemoryView`, which a later read then
            // refused with the owned-buffer `C0001` instead of letting
            // the check phase report its `T0023` for the reassignment.
            // The comprehension's own sub-expressions are collected
            // first, since they may still read that buffer.
            //
            // `var` needs no such call: it is the D-117 synthesized
            // internal loop name (see `HirStmt::ListCompAssign`'s own
            // doc comment), which cannot collide with a source name.
            //
            // #1343: a set comprehension's container is the same
            // defaulted term as the expression form's.
            let container = match stmt {
                HirStmt::SetCompAssign { .. } => set_comp::set_comp_container(
                    elt_term,
                    parents,
                    concrete,
                    &mut constraints.deferred,
                ),
                _ => set_comp::list_comp_container(
                    elt_term,
                    parents,
                    concrete,
                    &mut constraints.deferred,
                ),
            };
            bind_comp_target(env, target, container);
        }
        HirStmt::DictCompAssign {
            target,
            var,
            iter,
            cond,
            key,
            value,
        } => {
            bind_comp_loop_var(
                signatures,
                parents,
                concrete,
                &mut constraints.deferred,
                env,
                var,
                iter,
            )?;
            if let Some(cond) = cond {
                collect_expr_constraints(
                    signatures,
                    parents,
                    concrete,
                    &mut constraints.deferred,
                    env,
                    cond,
                )?;
            }
            let key_term = collect_expr_constraints(
                signatures,
                parents,
                concrete,
                &mut constraints.deferred,
                env,
                key,
            )?;
            let value_term = collect_expr_constraints(
                signatures,
                parents,
                concrete,
                &mut constraints.deferred,
                env,
                value,
            )?;
            let container = set_comp::dict_comp_container(
                key_term,
                value_term,
                parents,
                concrete,
                &mut constraints.deferred,
            );
            // Round-11 review finding 2: see the list/set arm above.
            bind_comp_target(env, target, container);
        }
        _ => unreachable!("collect_comp_assign_constraints: not a comprehension assignment"),
    }
    Ok(())
}
