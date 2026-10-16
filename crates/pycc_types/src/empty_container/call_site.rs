//! Container-typed parameters of private helpers, from their call sites.
//!
//! The D-146 solver infers an unannotated private-helper parameter only
//! when it is a scalar; a list, dict, set, tuple or instance argument
//! leaves it unresolved (`T0021`, "cannot infer type of parameter"). That
//! is the common shape of a helper called with a container the caller
//! built (`def _dfs(adj, seen, u)`), and of every function
//! `pycc_ast::desugar::closures` lifts out of its enclosing function, which
//! receives the enclosing locals it reads as leading parameters.
//!
//! This pass runs after the empty-container resolution and before the
//! solver. For each still-unannotated parameter of a function, it types the
//! argument of every call site the checker can type in its own scope (the
//! same per-function environments the empty-container pass builds); when
//! every such argument has one concrete, non-scalar type, the parameter
//! takes it. Scalars are left to the solver, whose `int`/`bool` widening
//! rules this pass does not repeat. Resolving one helper can make the
//! arguments of the calls inside it typable, so the pass repeats until
//! nothing changes.
//!
//! The pass only ever writes a type every typable call site agrees on; the
//! checker that follows still checks every call against it, so a call site
//! this pass could not type is still checked, never trusted.

use super::module_environment;
use super::unify::concrete_ty;
use crate::infer_expr_in;
use pycc_hir::{HirExpr, HirItem, HirModule, HirStmt, Ty};
use crate::fx::HashMap;

/// The number of rounds after which the pass stops even if a later round
/// could still resolve more (each round resolves at least one parameter, so
/// this bounds nesting depth, not correctness).
const MAX_ROUNDS: usize = 8;

/// Whether any function has an unannotated parameter.
fn has_infer_params(hir: &HirModule) -> bool {
    hir.items.iter().any(|item| {
        matches!(item, HirItem::Function { params, .. } if params.iter().any(|(_, ty)| *ty == Ty::Infer))
    })
}

/// Whether `ty` is a type this pass writes: concrete and not a scalar.
fn is_container_param(ty: &Ty) -> bool {
    concrete_ty(ty) && !matches!(ty, Ty::Int | Ty::Float | Ty::Bool | Ty::Str | Ty::None)
}

/// Resolves container parameters of `hir` from their call sites (see the
/// module documentation); `None` when nothing changed.
pub(crate) fn resolve_call_site_params(hir: &HirModule) -> Option<HirModule> {
    if !has_infer_params(hir) {
        return None;
    }
    let mut resolved = hir.clone();
    let mut changed_any = false;
    for _ in 0..MAX_ROUNDS {
        let (observed, incomplete) = observe_calls(&resolved);
        // A parameter some call site cannot type yet (its argument reads a
        // parameter still unresolved) waits for a later round, which may
        // type that site too and find it disagreeing; it is written from
        // the sites it has only once nothing else resolves.
        let mut changed = apply_observed(&mut resolved, &observed, &|key| !incomplete.contains(key));
        if !changed {
            changed = apply_observed(&mut resolved, &observed, &|_| true);
        }
        if !changed {
            break;
        }
        changed_any = true;
    }
    changed_any.then_some(resolved)
}

/// Writes every still-unannotated parameter of `hir` that `observed` types
/// and `ready` admits; whether one was written.
fn apply_observed(
    hir: &mut HirModule,
    observed: &HashMap<(String, usize), Option<Ty>>,
    ready: &dyn Fn(&(String, usize)) -> bool,
) -> bool {
    let mut changed = false;
    for item in &mut hir.items {
        let HirItem::Function { name, params, .. } = item else {
            continue;
        };
        for (index, (_, ty)) in params.iter_mut().enumerate() {
            if *ty != Ty::Infer {
                continue;
            }
            let key = (name.clone(), index);
            if let Some(Some(found)) = observed.get(&key)
                && ready(&key)
            {
                *ty = found.clone();
                changed = true;
            }
        }
    }
    changed
}

/// The parameters some call site passes an argument to that does not type
/// yet.
type Incomplete = crate::fx::HashSet<(String, usize)>;

/// For every `(function, parameter index)` some call site passes a typable
/// argument to: `Some(ty)` when every typable argument has container type
/// `ty`, `None` when they disagree or one is not a container; and the
/// parameters some call site passes an untypable argument to.
fn observe_calls(hir: &HirModule) -> (HashMap<(String, usize), Option<Ty>>, Incomplete) {
    let infer_params: HashMap<&str, Vec<bool>> = hir
        .items
        .iter()
        .filter_map(|item| match item {
            HirItem::Function { name, params, .. } => Some((
                name.as_str(),
                params.iter().map(|(_, ty)| *ty == Ty::Infer).collect(),
            )),
            HirItem::TopLevelStmt(_) => None,
        })
        .collect();
    let mut observed: HashMap<(String, usize), Option<Ty>> = HashMap::default();
    let mut incomplete = Incomplete::default();
    for_each_call(hir, &mut |env, names, callee, args| {
        // A nested function used as a value: its captured values are its
        // lifted function's leading arguments.
        let (callee, args, partial) = match (callee, args) {
            (pycc_hir::CLOSURE_MARKER, [HirExpr::Name(function), captures @ ..]) => {
                (function.as_str(), captures, true)
            }
            _ => (callee, args, false),
        };
        let Some(flags) = infer_params.get(callee) else {
            return;
        };
        if flags.len() < args.len() || (!partial && flags.len() != args.len()) {
            return;
        }
        for (index, arg) in args.iter().enumerate() {
            if !flags[index] {
                continue;
            }
            // An argument still typed by an unresolved parameter (a
            // recursive call) is not evidence either way.
            let Ok(ty) = infer_expr_in(env, names, arg) else {
                incomplete.insert((callee.to_string(), index));
                continue;
            };
            if super::contains_infer(&ty) {
                incomplete.insert((callee.to_string(), index));
                continue;
            }
            let entry = observed
                .entry((callee.to_string(), index))
                .or_insert(Some(ty.clone()));
            // A function value is typed only by such evidence.
            if !(is_container_param(&ty) || matches!(ty, Ty::Callable(_)))
                || entry.as_ref() != Some(&ty)
            {
                *entry = None;
            }
        }
    });
    (observed, incomplete)
}

/// Calls `visit` with every call `callee(args)` of `hir` -- in the module
/// body and in every function -- and the environment and local names its
/// arguments type in.
pub(super) fn for_each_call(
    hir: &HirModule,
    visit: &mut dyn FnMut(&crate::Environment, &[&str], &str, &[HirExpr]),
) {
    let local_names = crate::module::module_function_local_names(hir);
    let module_env = module_environment(hir);
    let mut record = |env: &crate::Environment, names: &[&str], body: &[HirStmt]| {
        let mut body = body.to_vec();
        super::for_each_stmt_mut(&mut body, &mut |stmt| {
            // A comprehension's element and condition read its loop
            // variable (`[Node(i) for i in range(n)]`).
            let scoped = comprehension_scope(env, names, stmt);
            let env = scoped.as_ref().unwrap_or(env);
            super::unify::stmt_exprs_mut(stmt, &mut |expr| {
                super::unify::walk_expr_mut(expr, &mut |expr| {
                    if let HirExpr::Call { callee, args } = expr {
                        visit(env, names, callee, args);
                    }
                    false
                });
            });
        });
    };
    let top_level: Vec<HirStmt> = hir
        .items
        .iter()
        .filter_map(|item| match item {
            HirItem::TopLevelStmt(stmt) => Some(stmt.clone()),
            HirItem::Function { .. } => None,
        })
        .collect();
    let top_level_names = crate::function_local_names(&[], &top_level);
    record(&module_env, &top_level_names, &top_level);
    for (index, item) in hir.items.iter().enumerate() {
        let HirItem::Function {
            name, params, body, ..
        } = item
        else {
            continue;
        };
        let names = &local_names[index];
        // Unlike the empty-container producers, no binding is demoted to
        // maybe-bound: an argument here is evidence for a parameter type
        // the checker then verifies at every call, not a value read.
        let mut env = module_env.child_for_function(names);
        env.current_class = name
            .split('.')
            .next()
            .filter(|prefix| *prefix != name)
            .map(String::from);
        for (param_name, param_ty) in params {
            env.bind(param_name.clone(), param_ty.clone());
        }
        crate::bind_local_types_in_body(&mut env, names, body);
        bind_comprehension_targets(&mut env, names, body);
        record(&env, names, body);
    }
}

/// `env` with the loop variable of comprehension statement `stmt` bound,
/// when `stmt` is one and its iterable types.
fn comprehension_scope(
    env: &crate::Environment,
    names: &[&str],
    stmt: &HirStmt,
) -> Option<crate::Environment> {
    let (var, iter) = match stmt {
        HirStmt::ListCompAssign { var, iter, .. }
        | HirStmt::SetCompAssign { var, iter, .. }
        | HirStmt::DictCompAssign { var, iter, .. } => (var, iter),
        _ => return None,
    };
    let var_ty = crate::comprehension::resolve_comp_iter(env, names, iter).ok()?;
    let mut scoped = env.clone();
    scoped.bind(var.clone(), var_ty);
    Some(scoped)
}

/// Binds the target of every comprehension statement (`xs = [... for ...]`)
/// in `body`, which `bind_local_types_in_body` leaves to the checker's own
/// comprehension check; one that does not check stays unbound.
fn bind_comprehension_targets(env: &mut crate::Environment, names: &[&str], body: &[HirStmt]) {
    use crate::comprehension::{CompElts, CompView, check_comp_assign};
    let mut body = body.to_vec();
    super::for_each_stmt_mut(&mut body, &mut |stmt| {
        let (target, var, iter, cond, elts) = match &*stmt {
            HirStmt::ListCompAssign {
                target,
                var,
                iter,
                cond,
                elt,
            } => (target, var, iter, cond, CompElts::List(elt)),
            HirStmt::SetCompAssign {
                target,
                var,
                iter,
                cond,
                elt,
            } => (target, var, iter, cond, CompElts::Set(elt)),
            HirStmt::DictCompAssign {
                target,
                var,
                iter,
                cond,
                key,
                value,
            } => (target, var, iter, cond, CompElts::Dict(key, value)),
            _ => return,
        };
        let view = CompView {
            var,
            iter,
            cond: cond.as_deref(),
            elts,
        };
        let _ = check_comp_assign(env, names, target, view);
    });
}
