//! Per-call-site copies of a nested helper (issue #7): each direct call of
//! a lifted nested function with inferred parameters, after its first,
//! calls a copy of its own, so the solver types each call's arguments
//! separately.
//!
//! A candidate is a lifted nested function (`pycc_ast`'s `_pycc_q...`
//! spelling) with at least one inferred parameter, called directly at least
//! twice, never used as a value, and calling no candidate itself (so no
//! copy needs copies of its own, and recursion is left alone). A copy is
//! named `_pycc_site<k><helper>` (`pycc_hir::SITE_COPY_PREFIX`): like a
//! monomorphized generic it is appended after every item and called
//! directly, so where it sits does not matter, and it is never exported.

use super::{for_each_expr, for_each_item_expr};
use crate::fx::HashMap;
use pycc_hir::{HirExpr, HirItem, HirModule};

/// The spelling `pycc_ast` lifts a nested function to.
const LIFTED_PREFIX: &str = "_pycc_q";

/// The item indices of the candidates of `hir` (see the module
/// documentation).
pub(super) fn candidates(hir: &HirModule) -> Vec<usize> {
    let lifted: Vec<(usize, &str)> = hir
        .items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| match item {
            HirItem::Function {
                name, params, body, ..
            } if name.starts_with(LIFTED_PREFIX)
                && !name.contains('.')
                && params
                    .iter()
                    .any(|(_, ty)| crate::empty_container::contains_infer(ty))
                && !pycc_hir::body_yields(body) =>
            {
                Some((index, name.as_str()))
            }
            _ => None,
        })
        .collect();
    if lifted.is_empty() {
        return Vec::new();
    }
    // Per lifted name: direct calls, value uses, and the item of each call.
    let mut calls: HashMap<&str, Vec<usize>> = HashMap::default();
    let mut values: Vec<String> = Vec::new();
    let mut module = hir.clone();
    for (index, item) in module.items.iter_mut().enumerate() {
        for_each_item_expr(item, &mut |expr| {
            match expr {
                HirExpr::Call { callee, .. } => {
                    if let Some((_, name)) = lifted.iter().find(|(_, name)| name == callee) {
                        calls.entry(name).or_default().push(index);
                    }
                }
                HirExpr::Name(name) => values.push(name.clone()),
                _ => {}
            }
            false
        });
    }
    let callers: Vec<(usize, &str)> = lifted
        .iter()
        .copied()
        .filter(|(_, name)| {
            calls.get(name).is_some_and(|sites| sites.len() >= 2)
                && !values.iter().any(|v| v == name)
        })
        .collect();
    let chosen: Vec<&str> = callers.iter().map(|(_, name)| *name).collect();
    callers
        .into_iter()
        .filter(|(index, _)| {
            // A candidate calling a candidate (itself included) is left alone.
            !calls
                .iter()
                .any(|(callee, sites)| chosen.contains(callee) && sites.contains(index))
        })
        .map(|(index, _)| index)
        .collect()
}

/// `hir` with every call of each candidate after its first redirected to a
/// copy of its own, and the item each copy was made from.
pub(super) fn copy(hir: &HirModule, candidates: &[usize]) -> (HirModule, HashMap<usize, usize>) {
    let mut module = hir.clone();
    let mut origins = HashMap::default();
    if candidates.is_empty() {
        return (module, origins);
    }
    let names: Vec<(usize, String)> = candidates
        .iter()
        .filter_map(|&index| match &hir.items[index] {
            HirItem::Function { name, .. } => Some((index, name.clone())),
            HirItem::TopLevelStmt(_) => None,
        })
        .collect();
    let mut seen: HashMap<String, usize> = HashMap::default();
    let mut copies: Vec<(usize, String)> = Vec::new();
    for_each_expr(&mut module, &mut |expr| {
        if let HirExpr::Call { callee, .. } = expr
            && let Some((index, name)) = names.iter().find(|(_, name)| name == callee)
        {
            let site = seen.entry(name.clone()).or_insert(0);
            if *site > 0 {
                let copy = format!("{}{site}{name}", pycc_hir::SITE_COPY_PREFIX);
                copies.push((*index, copy.clone()));
                *callee = copy;
            }
            *site += 1;
        }
        false
    });
    for (origin, name) in copies {
        let HirItem::Function {
            params,
            return_ty,
            body,
            ..
        } = &module.items[origin]
        else {
            continue;
        };
        let (params, return_ty, body) = (params.clone(), return_ty.clone(), body.clone());
        origins.insert(module.items.len(), origin);
        module.items.push(HirItem::Function {
            name,
            params,
            return_ty,
            body,
        });
    }
    (module, origins)
}
