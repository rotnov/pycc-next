//! Empty containers of inferred functions, typed from their solved
//! signatures.
//!
//! The empty-container pass runs before the private-helper solver, so in a
//! function whose parameters or callees are unannotated a producer such as
//! `result.append(x % num)` reads `Ty::Infer` and its list stays `T0003`.
//! When one is left, this step runs the solver once over the module as it
//! stands, hands every function signature it pins concretely to a copy of
//! the module, and reruns the empty-container pass over that copy. Only the
//! rewritten bodies are kept: each function keeps its own unannotated
//! signature, which the real solver pass after it infers again from the
//! completed module, and the checker re-validates every resolved literal as
//! it does any other.
//!
//! A signature still holding an inference variable, or the CPython object
//! (non-strict mode), is not handed over: the producer then reads what it
//! read before.

use super::{any_stmt, body_has_empty_literal, contains_infer, resolve_empty_containers};
use pycc_hir::{HirExpr, HirItem, HirModule, HirStmt, Ty};

/// `hir` with the empty containers its solved signatures type rewritten;
/// `None` when no function body keeps an empty literal, the solver does not
/// succeed, or nothing more resolves.
pub(crate) fn resolve_with_solved_signatures(hir: &HirModule) -> Option<HirModule> {
    let pending = hir.items.iter().any(|item| match item {
        HirItem::Function { body, .. } => {
            body_has_empty_literal(body) || has_empty_constructor(body)
        }
        HirItem::TopLevelStmt(stmt) => {
            let body = std::slice::from_ref(stmt);
            body_has_empty_literal(body) || has_empty_constructor(body)
        }
    }) || has_provisional_slot(hir);
    if !pending {
        return None;
    }
    let names = crate::module::module_function_local_names(hir);
    let signatures =
        match crate::constraints::infer_function_signatures_with_solver_all(hir, &names) {
            Ok(signatures) => signatures,
            Err(_) => {
                crate::constraints::return_fallback::recover_returns(hir, &names).or_else(|| {
                    // What the solver does settle still types producers.
                    crate::constraints::return_fallback::partial_signatures(hir, &names)
                })?
            }
        };
    let mut typed = hir.clone();
    let mut changed = false;
    for item in &mut typed.items {
        let HirItem::Function {
            name,
            params,
            return_ty,
            ..
        } = item
        else {
            continue;
        };
        let Some((solved_params, solved_return)) = signatures.get(name.as_str()) else {
            continue;
        };
        for ((_, ty), solved) in params.iter_mut().zip(solved_params) {
            if contains_infer(ty) && usable(solved) {
                *ty = solved.clone();
                changed = true;
            }
        }
        if contains_infer(return_ty) && usable(solved_return) {
            *return_ty = solved_return.clone();
            changed = true;
        }
    }
    if !changed {
        return None;
    }
    let mut rewritten = resolve_empty_containers(&typed)?;
    // Keep the rewritten bodies under the original signatures.
    for (item, original) in rewritten.items.iter_mut().zip(&hir.items) {
        if let (
            HirItem::Function {
                params, return_ty, ..
            },
            HirItem::Function {
                params: original_params,
                return_ty: original_return,
                ..
            },
        ) = (item, original)
        {
            params.clone_from(original_params);
            return_ty.clone_from(original_return);
        }
    }
    (rewritten != *hir).then_some(rewritten)
}

/// Whether `body` binds a name to an empty `set()`/`dict()`/`list()` call,
/// which the unification engine types from its uses (`empty_container::unify`)
/// and a use may read an unannotated function's return (`s.add(c.find(i))`).
fn has_empty_constructor(body: &[HirStmt]) -> bool {
    any_stmt(body, &|stmt| {
        matches!(stmt, HirStmt::Assign {
            value: HirExpr::Call { callee, args },
            ..
        } if args.is_empty() && matches!(callee.as_str(), "set" | "dict" | "list"))
    })
}

/// Whether some class still carries a provisional attribute slot: its
/// producer (`self.xs.append(v)`, `self.d[k] = v`) may read an unannotated
/// method parameter the solver pins.
fn has_provisional_slot(hir: &HirModule) -> bool {
    hir.class_defs
        .iter()
        .any(|(_, class)| class.attrs.iter().any(|(_, ty)| contains_infer(ty)))
}

/// Whether a solved type may stand in for an unannotated position.
fn usable(ty: &Ty) -> bool {
    !contains_infer(ty) && !contains_object(ty)
}

fn contains_object(ty: &Ty) -> bool {
    match ty {
        Ty::Object => true,
        Ty::List(element) | Ty::Set(element) | Ty::FrozenSet(element) | Ty::Optional(element) => {
            contains_object(element)
        }
        Ty::Dict(pair) => contains_object(&pair.0) || contains_object(&pair.1),
        Ty::Tuple(elements) => elements.iter().any(contains_object),
        _ => false,
    }
}

/// A copy of `hir` whose unannotated function returns carry the types the
/// solver pins for them, for typing call arguments that read such a return
/// (`init_params`); `None` when the solver pins nothing more. The copy is
/// only read, never checked or lowered.
pub(crate) fn with_solved_returns(hir: &HirModule) -> Option<HirModule> {
    let names = crate::module::module_function_local_names(hir);
    let signatures = crate::constraints::infer_function_signatures_with_solver_all(hir, &names)
        .ok()
        .or_else(|| crate::constraints::return_fallback::partial_signatures(hir, &names))?;
    let mut typed = hir.clone();
    let mut changed = false;
    for item in &mut typed.items {
        let HirItem::Function {
            name, return_ty, ..
        } = item
        else {
            continue;
        };
        if let Some((_, solved_return)) = signatures.get(name.as_str())
            && contains_infer(return_ty)
            && usable(solved_return)
        {
            *return_ty = solved_return.clone();
            changed = true;
        }
    }
    changed.then_some(typed)
}
