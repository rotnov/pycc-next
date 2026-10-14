//! The element type of an unannotated generator the solver could not type
//! from its `yield`s alone, typed from its body once its parameters are.
//!
//! The solver's terms have no structure: a yielded tuple display
//! (`yield (k, v)`) or a value read out of a local container gives it no
//! term, so a generator helper -- including every lifted generator
//! expression, `((f(x), x) for x in xs)` -- whose parameters it *did* type
//! still had no element type. Once every parameter type is known the body
//! is ordinary code: the checker's flat local binder types its locals, and
//! the first `yield` whose value then infers gives the element type. The
//! check phase still validates every `yield` against it.

use crate::module::FunctionSignatures;
use crate::{Environment, bind_local_types_in_body, function_local_names, infer_expr_in};
use pycc_hir::{HirExpr, HirItem, HirModule, HirStmt, Ty};
use std::sync::Arc;

/// The values the generator `name` yields, from its body with `params`
/// bound and `resolved` registered for the calls it makes.
pub(super) fn infer_yield_from_body(
    hir: &HirModule,
    name: &str,
    params: &[Ty],
    resolved: &FunctionSignatures,
) -> Option<Ty> {
    let (param_names, body) = hir.items.iter().find_map(|item| match item {
        HirItem::Function {
            name: item_name,
            params,
            body,
            ..
        } if item_name == name => Some((params, body)),
        _ => None,
    })?;
    if params.iter().any(crate::empty_container::contains_infer) {
        return None;
    }
    let typed: Vec<(String, Ty)> = param_names
        .iter()
        .map(|(n, _)| n.clone())
        .zip(params.iter().cloned())
        .collect();
    let local_names = function_local_names(&typed, body);
    let mut module = super::annotated_function_environment(hir);
    let mut functions = (*module.functions).clone();
    for (callee, signature) in resolved {
        functions.insert(callee.clone(), signature.clone());
    }
    module.functions = Arc::new(functions);
    let mut env = module.child_for_function(&local_names);
    for (param, ty) in &typed {
        env.bind(param.clone(), ty.clone());
    }
    bind_local_types_in_body(&mut env, &local_names, body);
    first_yield(&env, &local_names, body)
}

/// The type of the first `yield` value in `body` that infers.
fn first_yield(env: &Environment, local_names: &[&str], body: &[HirStmt]) -> Option<Ty> {
    for stmt in body {
        let found = match stmt {
            HirStmt::ExprStmt(HirExpr::Call { callee, args })
                if callee == pycc_hir::YIELD_MARKER && args.len() == 1 =>
            {
                infer_expr_in(env, local_names, &args[0])
                    .ok()
                    .filter(|ty| !crate::empty_container::contains_infer(ty))
            }
            HirStmt::If { body, orelse, .. } => first_yield(env, local_names, body)
                .or_else(|| first_yield(env, local_names, orelse)),
            HirStmt::While { body, .. }
            | HirStmt::ForRange { body, .. }
            | HirStmt::ForList { body, .. }
            | HirStmt::ForObject { body, .. } => first_yield(env, local_names, body),
            HirStmt::LoopElse { loop_stmt, orelse } => {
                first_yield(env, local_names, std::slice::from_ref(loop_stmt.as_ref()))
                    .or_else(|| first_yield(env, local_names, orelse))
            }
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
            } => first_yield(env, local_names, body)
                .or_else(|| {
                    handlers
                        .iter()
                        .find_map(|h| first_yield(env, local_names, &h.body))
                })
                .or_else(|| first_yield(env, local_names, orelse))
                .or_else(|| first_yield(env, local_names, finalbody)),
            HirStmt::Match { cases, .. } => cases
                .iter()
                .find_map(|case| first_yield(env, local_names, &case.body)),
            _ => None,
        };
        if found.is_some() {
            return found;
        }
    }
    None
}
