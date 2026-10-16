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
    let first = first_value_from_body(hir, name, params, resolved, &yielded)?;
    // Non-strict mode: a generator that also yields CPython objects (`yield
    // INF` beside `yield data[i]` over a CPython list) yields objects, the
    // native values boxed.
    if crate::non_strict::is_non_strict()
        && first != Ty::Object
        && mixes_objects(hir, name, params, resolved, &yielded)
    {
        return Some(Ty::Object);
    }
    Some(first)
}

/// The value a `yield` statement yields.
fn yielded(stmt: &HirStmt) -> Option<&HirExpr> {
    match stmt {
        HirStmt::ExprStmt(HirExpr::Call { callee, args })
            if callee == pycc_hir::YIELD_MARKER && args.len() == 1 =>
        {
            Some(&args[0])
        }
        _ => None,
    }
}

/// Whether some value `pick` finds in the body of `name` is a CPython
/// object and every other one infers to a value an `object` slot boxes.
fn mixes_objects(
    hir: &HirModule,
    name: &str,
    params: &[Ty],
    resolved: &FunctionSignatures,
    pick: &dyn Fn(&HirStmt) -> Option<&HirExpr>,
) -> bool {
    with_body_env(hir, name, params, resolved, &mut |env, local_names, body| {
        let mut values = Vec::new();
        collect_values(body, pick, &mut values);
        let types: Vec<Option<Ty>> = values
            .iter()
            .map(|value| infer_expr_in(env, local_names, value).ok())
            .collect();
        types.iter().any(|ty| *ty == Some(Ty::Object))
            && types.iter().all(|ty| match ty {
                Some(Ty::Object) => true,
                Some(ty) => crate::object_box::admits(env, ty, &Ty::Object),
                None => false,
            })
    })
    .unwrap_or(false)
}

/// The type of the first value `pick` finds in the body of `name` (in
/// statement order, at any block depth) that infers, with `params` bound
/// and `resolved` registered for the calls it makes.
pub(super) fn first_value_from_body(
    hir: &HirModule,
    name: &str,
    params: &[Ty],
    resolved: &FunctionSignatures,
    pick: &dyn Fn(&HirStmt) -> Option<&HirExpr>,
) -> Option<Ty> {
    with_body_env(hir, name, params, resolved, &mut |env, local_names, body| {
        first_yield(env, local_names, body, pick)
    })?
}

/// Whether every value `pick` finds in the body of `name` (typed as
/// [`first_value_from_body`] types them) infers to `expected`.
pub(super) fn every_value_from_body_is(
    hir: &HirModule,
    name: &str,
    params: &[Ty],
    resolved: &FunctionSignatures,
    pick: &dyn Fn(&HirStmt) -> Option<&HirExpr>,
    expected: &Ty,
) -> bool {
    with_body_env(hir, name, params, resolved, &mut |env, local_names, body| {
        let mut values = Vec::new();
        collect_values(body, pick, &mut values);
        values
            .iter()
            .all(|value| infer_expr_in(env, local_names, value).ok().as_ref() == Some(expected))
    })
    .unwrap_or(false)
}

/// Runs `f` over the body of `name` with the checker's flat local binder
/// over it: `params` bound and `resolved` registered for the calls it makes.
fn with_body_env<T>(
    hir: &HirModule,
    name: &str,
    params: &[Ty],
    resolved: &FunctionSignatures,
    f: &mut dyn FnMut(&Environment, &[&str], &[HirStmt]) -> T,
) -> Option<T> {
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
    Some(f(&env, &local_names, body))
}

/// Every value `pick` finds in `body`, at any block depth, in order.
fn collect_values<'a>(
    body: &'a [HirStmt],
    pick: &dyn Fn(&HirStmt) -> Option<&HirExpr>,
    out: &mut Vec<&'a HirExpr>,
) {
    for stmt in body {
        if let Some(value) = pick(stmt) {
            out.push(value);
            continue;
        }
        match stmt {
            HirStmt::If { body, orelse, .. } => {
                collect_values(body, pick, out);
                collect_values(orelse, pick, out);
            }
            HirStmt::While { body, .. }
            | HirStmt::ForRange { body, .. }
            | HirStmt::ForList { body, .. }
            | HirStmt::ForObject { body, .. } => collect_values(body, pick, out),
            HirStmt::LoopElse { loop_stmt, orelse } => {
                collect_values(std::slice::from_ref(loop_stmt.as_ref()), pick, out);
                collect_values(orelse, pick, out);
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
            } => {
                collect_values(body, pick, out);
                for handler in handlers {
                    collect_values(&handler.body, pick, out);
                }
                collect_values(orelse, pick, out);
                collect_values(finalbody, pick, out);
            }
            HirStmt::Match { cases, .. } => {
                for case in cases {
                    collect_values(&case.body, pick, out);
                }
            }
            _ => {}
        }
    }
}

/// The type of the first value `pick` finds in `body` that infers.
fn first_yield(
    env: &Environment,
    local_names: &[&str],
    body: &[HirStmt],
    pick: &dyn Fn(&HirStmt) -> Option<&HirExpr>,
) -> Option<Ty> {
    for stmt in body {
        if let Some(value) = pick(stmt) {
            let found = infer_expr_in(env, local_names, value)
                .ok()
                .filter(|ty| !crate::empty_container::contains_infer(ty));
            if found.is_some() {
                return found;
            }
            continue;
        }
        let found = match stmt {
            HirStmt::If { body, orelse, .. } => first_yield(env, local_names, body, pick)
                .or_else(|| first_yield(env, local_names, orelse, pick)),
            HirStmt::While { body, .. }
            | HirStmt::ForRange { body, .. }
            | HirStmt::ForList { body, .. }
            | HirStmt::ForObject { body, .. } => first_yield(env, local_names, body, pick),
            HirStmt::LoopElse { loop_stmt, orelse } => {
                first_yield(env, local_names, std::slice::from_ref(loop_stmt.as_ref()), pick)
                    .or_else(|| first_yield(env, local_names, orelse, pick))
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
            } => first_yield(env, local_names, body, pick)
                .or_else(|| {
                    handlers
                        .iter()
                        .find_map(|h| first_yield(env, local_names, &h.body, pick))
                })
                .or_else(|| first_yield(env, local_names, orelse, pick))
                .or_else(|| first_yield(env, local_names, finalbody, pick)),
            HirStmt::Match { cases, .. } => cases
                .iter()
                .find_map(|case| first_yield(env, local_names, &case.body, pick)),
            _ => None,
        };
        if found.is_some() {
            return found;
        }
    }
    None
}
