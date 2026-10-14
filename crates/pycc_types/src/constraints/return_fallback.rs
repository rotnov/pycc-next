//! Strict mode: the return type of an unannotated function the solver could
//! not type from its `return`s alone, typed from its body once its
//! parameters are.
//!
//! The solver's terms have no structure: a returned tuple display
//! (`return up, down`), or a value its body computes from a local the solver
//! did not type, gives it no term, so the function is refused (`T0001` or
//! `T0021`) even when every parameter is pinned by its call sites. When the
//! solver fails, [`recover_returns`] collects the signature table it *can*
//! settle, types each such function's first `return` value that infers
//! from its body (the checker's flat local binder, every concretely typed
//! callee registered: `yield_fallback`'s method), writes that type in as
//! the function's return annotation and runs the solver again -- now its
//! callers, and the parameters they feed, can be typed too. Every parameter
//! and return the solver did settle is written in the same way first, so a
//! later round sees it at each use, whatever order the uses come in. The
//! check phase still validates every `return` of the function against it.
//!
//! Non-strict mode never gets here: an untyped return there is the CPython
//! object, as before.

use super::infer_function_signatures_with_solver_all;
use crate::empty_container::contains_infer;
use crate::module::FunctionSignatures;
use pycc_hir::{HirExpr, HirItem, HirModule, HirStmt, Ty};
use std::cell::Cell;

thread_local! {
    static PARTIAL: Cell<bool> = const { Cell::new(false) };
}

/// Whether the solver is collecting a partial table: a position it cannot
/// type is `Ty::Infer` rather than an error.
pub(crate) fn collecting_partial() -> bool {
    PARTIAL.with(Cell::get)
}

/// How many functions' returns may be typed one after another (each round
/// types at least one).
const MAX_ROUNDS: usize = 16;

/// The solver's signatures for `hir` after typing unresolved returns from
/// their bodies (module doc); `None` when that types nothing or the solver
/// still fails.
pub(crate) fn recover_returns(
    hir: &HirModule,
    function_local_names: &[Vec<&str>],
) -> Option<FunctionSignatures> {
    if crate::non_strict::is_non_strict() {
        return None;
    }
    let mut module = hir.clone();
    for _ in 0..MAX_ROUNDS {
        let partial = partial_signatures(&module, function_local_names)?;
        // What the solver did settle is written in first: a later round
        // then sees it at every use, whatever order the uses come in
        // (`min(a)` of a helper's result types the target it is unpacked to).
        let settled = write_settled(&mut module, &partial);
        let typed = type_returns_from_bodies(&mut module, &partial, false);
        if !settled && !typed {
            return None;
        }
        if let Ok(signatures) =
            infer_function_signatures_with_solver_all(&module, function_local_names)
        {
            return Some(signatures);
        }
    }
    None
}

/// The solver's signatures for `hir` in non-strict mode, after writing in
/// what it settles and solving again until nothing more settles: a use the
/// first pass met before its operand was typed (`min(a)` of a helper's
/// result) is typed in a later one instead of becoming a CPython object. A
/// return the solver leaves open is typed from the body as strict mode
/// types it, but only when every `return` of the function infers to the
/// same type -- otherwise it stays the CPython object it was. Only types the
/// solver or the checker settled are written, so every position keeps the
/// type the single pass would have given it or gains a concrete one.
pub(crate) fn solve_non_strict(
    hir: &HirModule,
    function_local_names: &[Vec<&str>],
) -> Result<FunctionSignatures, crate::module::KeyedDiagnostics> {
    let mut module: Option<HirModule> = None;
    for _ in 0..MAX_ROUNDS {
        let current = module.as_ref().unwrap_or(hir);
        let Some(partial) = partial_signatures(current, function_local_names) else {
            break;
        };
        let mut next = current.clone();
        let settled = write_settled(&mut next, &partial);
        let typed = type_returns_from_bodies(&mut next, &partial, true);
        if !settled && !typed {
            break;
        }
        module = Some(next);
    }
    infer_function_signatures_with_solver_all(module.as_ref().unwrap_or(hir), function_local_names)
}

/// Writes in, as its return annotation, the type of the first `return`
/// value of each unannotated function whose return `partial` leaves open
/// and whose parameters it settles (module doc); with `every_return`, only
/// when every `return` value infers to that type. Whether it wrote any.
fn type_returns_from_bodies(
    module: &mut HirModule,
    partial: &FunctionSignatures,
    every_return: bool,
) -> bool {
    let resolved: FunctionSignatures = partial
        .iter()
        .filter(|(_, (params, ret))| !params.iter().any(contains_infer) && !contains_infer(ret))
        .map(|(name, sig)| (name.clone(), sig.clone()))
        .collect();
    let mut progress = false;
    for index in 0..module.items.len() {
        let HirItem::Function {
            name,
            return_ty,
            body,
            ..
        } = &module.items[index]
        else {
            continue;
        };
        let Some((params, ret)) = partial.get(name) else {
            continue;
        };
        if *return_ty != Ty::Infer
            || *ret != Ty::Infer
            || params.iter().any(contains_infer)
            || pycc_hir::body_yields(body)
            || owner_is_generic(module, name)
        {
            continue;
        }
        let Some(found) =
            super::yield_fallback::first_value_from_body(module, name, params, &resolved, &return_value)
        else {
            continue;
        };
        if !admissible(module, &found)
            || (every_return
                && (contains_object(&found)
                    || !super::yield_fallback::every_value_from_body_is(
                        module, name, params, &resolved, &return_value, &found,
                    )))
        {
            continue;
        }
        if let HirItem::Function { return_ty, .. } = &mut module.items[index] {
            *return_ty = found;
            progress = true;
        }
    }
    progress
}

/// The value of a `return` statement.
fn return_value(stmt: &HirStmt) -> Option<&HirExpr> {
    match stmt {
        HirStmt::Return(Some(value)) => Some(value),
        _ => None,
    }
}

/// Whether `ty` mentions the CPython object anywhere.
fn contains_object(ty: &Ty) -> bool {
    match ty {
        Ty::Object => true,
        Ty::List(inner) | Ty::Set(inner) | Ty::FrozenSet(inner) | Ty::Optional(inner) => {
            contains_object(inner)
        }
        Ty::Dict(pair) => contains_object(&pair.0) || contains_object(&pair.1),
        Ty::Tuple(items) => items.iter().any(contains_object),
        _ => false,
    }
}

/// Writes every parameter and return type `partial` settled concretely
/// into the unannotated positions of `module`'s functions (generators and
/// generic-class methods left alone); whether it wrote any.
fn write_settled(module: &mut HirModule, partial: &FunctionSignatures) -> bool {
    let mut wrote = false;
    let generic: Vec<bool> = module
        .items
        .iter()
        .map(|item| match item {
            HirItem::Function { name, .. } => owner_is_generic(module, name),
            HirItem::TopLevelStmt(_) => true,
        })
        .collect();
    let snapshot = module.clone();
    for (item, generic) in module.items.iter_mut().zip(generic) {
        let HirItem::Function {
            name,
            params,
            return_ty,
            body,
            ..
        } = item
        else {
            continue;
        };
        let Some((solved_params, solved_return)) = partial.get(name.as_str()) else {
            continue;
        };
        if generic || pycc_hir::body_yields(body) {
            continue;
        }
        for ((_, ty), solved) in params.iter_mut().zip(solved_params) {
            if *ty == Ty::Infer && settled(&snapshot, solved) {
                *ty = solved.clone();
                wrote = true;
            }
        }
        if *return_ty == Ty::Infer && settled(&snapshot, solved_return) {
            *return_ty = solved_return.clone();
            wrote = true;
        }
    }
    wrote
}

/// Whether a partially solved type is concrete enough to write in.
fn settled(hir: &HirModule, ty: &Ty) -> bool {
    !contains_infer(ty) && *ty != Ty::Object && admissible(hir, ty)
}

/// The signature table the solver settles for `hir`, every position it
/// cannot type left `Ty::Infer`; `None` when a constraint conflicts.
pub(crate) fn partial_signatures(
    hir: &HirModule,
    function_local_names: &[Vec<&str>],
) -> Option<FunctionSignatures> {
    PARTIAL.with(|partial| partial.set(true));
    let signatures = infer_function_signatures_with_solver_all(hir, function_local_names);
    PARTIAL.with(|partial| partial.set(false));
    signatures.ok()
}

/// Whether `function` is a method of a generic class: its types may mention
/// the class's type parameter, which this step has no instantiation for.
fn owner_is_generic(hir: &HirModule, function: &str) -> bool {
    let Some((class, _)) = function.split_once('.') else {
        return false;
    };
    hir.class_defs
        .iter()
        .any(|(name, def)| name == class && def.type_param.is_some())
}

/// Whether a body-typed return may be written in as an annotation: no type
/// parameter, function value, protocol or instance of a generic class
/// anywhere in it.
fn admissible(hir: &HirModule, ty: &Ty) -> bool {
    match ty {
        Ty::Param(_) | Ty::Callable(_) | Ty::Protocol(_) => false,
        Ty::Instance(name) => !hir
            .class_defs
            .iter()
            .any(|(class, def)| class == name.as_str() && def.type_param.is_some()),
        Ty::List(inner) | Ty::Set(inner) | Ty::FrozenSet(inner) | Ty::Optional(inner) => {
            admissible(hir, inner)
        }
        Ty::Dict(pair) => admissible(hir, &pair.0) && admissible(hir, &pair.1),
        Ty::Tuple(items) => items.iter().all(|item| admissible(hir, item)),
        Ty::Generator(sig) => {
            admissible(hir, &sig.0) && admissible(hir, &sig.1) && admissible(hir, &sig.2)
        }
        _ => true,
    }
}
