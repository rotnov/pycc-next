//! `isinstance` tests a value's static type decides (issue #5), and the
//! per-call-site copies of nested helpers that let each call give its own
//! argument types to such a test (issue #7).
//!
//! ```python
//! def mat_pow(mat, init, K, MOD):
//!     def mat_dot(A, B, MOD):
//!         if not isinstance(B[0], list):  # B: list[int] at one call only
//!             B = [[b] for b in B]
//!         ...
//!     res = _mat_pow(mat, K, MOD)    # calls mat_dot(res, mat, MOD)
//!     res = mat_dot(res, init, MOD)
//! ```
//!
//! A test `isinstance(x, C)` of a builtin class `C` whose subject's type
//! is a builtin type (`list[...]`, `str`, `dict[...]`, ...) has one value
//! for every run: `pycc_types` folds it and keeps only the branch that
//! runs, as mypy's unreachable-branch analysis does. The subjects the test
//! evaluates (`B[0]` above, which may raise `IndexError`) still run, as
//! expression statements, in CPython's order (`and`/`or` short-circuit).
//! `pycc_ast`'s web analysis gives a parameter such a guard rebinds a
//! variable per path (`desugar::webs`, its `guard_joins`), so the branch
//! kept types on its own.
//!
//! Only a top-level `if` of a function body is folded, and only when its
//! whole test is decided: a combination of `isinstance` calls with `not`,
//! `and` and `or`. A guard's subject is typed from the signatures the
//! solver settles for a probe of the module in which every function stops
//! at its first undecided guard (what follows may need the fold to type),
//! and from the statements before the guard. Each round decides the guards
//! it can and splices them; a guard after a decided one is decided in a
//! later round.
//!
//! A nested helper called with different argument types (`mat_dot` gets
//! `list[list[int]]` twice and `list[int]` once) has no one signature.
//! When the module does not solve as written, each call site of such a
//! helper calls a copy of its own ([`site_copies`]), and its guards then
//! fold per copy. Both rewrites are adopted only when the result solves:
//! a module that does not keeps its own diagnostics.

use crate::fx::HashMap;
use crate::{Environment, bind_local_types_in_body, infer_expr_in};
use pycc_hir::{BoolOpKind, HirExpr, HirItem, HirModule, HirStmt, Ty, UnaryOpKind};

mod site_copies;

/// The builtin classes a folded test may name.
const CLASSES: [&str; 9] = [
    "list",
    "tuple",
    "dict",
    "set",
    "frozenset",
    "str",
    "int",
    "float",
    "bool",
];

/// At most this many rounds of guard folding (each one solves the module
/// once).
const MAX_ROUNDS: usize = 8;

/// A module whose guards were folded and whose helpers may have been copied
/// per call site, with the item each appended copy was made from.
pub(crate) struct Specialized {
    pub(crate) module: HirModule,
    pub(crate) origins: HashMap<usize, usize>,
    pub(crate) folds: Vec<Fold>,
}

/// One folded guard: the test that stood at statement `at` of `function`
/// and the value it was folded to. [`verify`] re-decides it under the
/// signatures the checker accepts.
#[derive(Debug, Clone)]
pub(crate) struct Fold {
    function: String,
    at: usize,
    test: HirExpr,
    value: bool,
}

/// `raw` with its nested helpers copied per call site, `prepare`d (the
/// passes that type containers from their uses, which made `current` from
/// `raw`) and with its guards folded -- when the solver finds conflicting
/// constraints in `current` and none in the result (see the module
/// documentation).
pub(crate) fn copy_call_sites(
    raw: &HirModule,
    current: &HirModule,
    prepare: impl Fn(&HirModule) -> Option<HirModule>,
) -> Option<Specialized> {
    let candidates = site_copies::candidates(raw);
    if candidates.is_empty() || !conflicts(current) {
        return None;
    }
    let (copied, origins) = site_copies::copy(raw, &candidates);
    let prepared = prepare(&copied).unwrap_or(copied);
    let (module, folds) = fold_guards(&prepared).unwrap_or((prepared, Vec::new()));
    (!conflicts(&module)).then_some(Specialized {
        module,
        origins,
        folds,
    })
}

/// `hir` with every guard it can decide folded, or `None` when it has none
/// or decides none.
pub(crate) fn fold(hir: &HirModule) -> Option<(HirModule, Vec<Fold>)> {
    fold_guards(hir)
}

/// Re-decides every fold of `hir` (the module the folds produced) under
/// `signatures`, the ones the checker validated: a guard decided from a
/// solver guess the checker does not confirm is refused, never compiled
/// with the branch it may not take.
pub(crate) fn verify(
    hir: &HirModule,
    signatures: &crate::module::FunctionSignatures,
    folds: &[Fold],
) -> Result<(), pycc_diag::Diagnostic> {
    if folds.is_empty() {
        return Ok(());
    }
    let env = module_environment(hir, signatures);
    let local_names = crate::module::module_function_local_names(hir);
    let deques = creates_deques(hir);
    for fold in folds {
        let confirmed = hir
            .items
            .iter()
            .zip(&local_names)
            .any(|(item, names)| match item {
                HirItem::Function {
                    name, params, body, ..
                } if *name == fold.function && fold.at <= body.len() => {
                    let decided = signatures.get(name).and_then(|(param_tys, _)| {
                        decide_at(
                            &env,
                            names,
                            params,
                            param_tys,
                            &body[..fold.at],
                            &fold.test,
                            deques,
                        )
                    });
                    decided.map(|(value, _)| value) == Some(fold.value)
                }
                _ => false,
            });
        if !confirmed {
            return Err(pycc_diag::Diagnostic::error(
                "C0001",
                format!(
                    "an `isinstance` test in `{}` whose value its subject's inferred type would decide is not compiled yet: that type did not settle",
                    pycc_hir::namespace::plain_name(&fold.function)
                ),
                pycc_diag::Span::new(0, 0),
            ));
        }
    }
    Ok(())
}

/// The value `test` has after `prefix` in a function with `params` typed
/// `param_tys` (an open type is left unbound), and the subjects it
/// evaluates; see [`decide`].
fn decide_at(
    env: &Environment,
    names: &[&str],
    params: &[(String, Ty)],
    param_tys: &[Ty],
    prefix: &[HirStmt],
    test: &HirExpr,
    deques: bool,
) -> Option<(bool, Vec<HirExpr>)> {
    let mut fn_env = env.child_for_function(names);
    for ((param, _), ty) in params.iter().zip(param_tys) {
        if concrete(ty) {
            fn_env.bind(param.clone(), ty.clone());
        }
    }
    bind_local_types_in_body(&mut fn_env, names, prefix);
    let type_of = |expr: &HirExpr| infer_expr_in(&fn_env, names, expr).ok().filter(concrete);
    decide(test, &type_of, deques)
}

/// Whether the solver finds constraints of `hir` that conflict (it may
/// still leave positions open for the later passes to type).
fn conflicts(hir: &HirModule) -> bool {
    let names = crate::module::module_function_local_names(hir);
    crate::constraints::return_fallback::partial_signatures(hir, &names).is_none()
}

/// `hir` with every guard it can decide folded, or `None` when it decides
/// none.
fn fold_guards(hir: &HirModule) -> Option<(HirModule, Vec<Fold>)> {
    let shadowed = &shadowed_builtins(hir)[..];
    let guarded = hir.items.iter().any(|item| match item {
        HirItem::Function { body, .. } => {
            body.iter().any(|stmt| guard_test(stmt, shadowed).is_some())
        }
        HirItem::TopLevelStmt(_) => false,
    });
    if !guarded {
        return None;
    }
    let deques = creates_deques(hir);
    let mut current = hir.clone();
    let mut folds = Vec::new();
    for _ in 0..MAX_ROUNDS {
        // Each function's first guard, by item and statement index.
        let pending: Vec<(usize, usize)> = current
            .items
            .iter()
            .enumerate()
            .filter_map(|(item, it)| match it {
                // A method is called through its receiver, not its name.
                HirItem::Function { name, body, .. } if !name.contains('.') => body
                    .iter()
                    .position(|stmt| guard_test(stmt, shadowed).is_some())
                    .map(|stmt| (item, stmt)),
                _ => None,
            })
            .collect();
        if pending.is_empty() {
            break;
        }
        let mut probe = current.clone();
        for &(item, stmt) in &pending {
            if let HirItem::Function {
                name, params, body, ..
            } = &mut probe.items[item]
            {
                // The probe ends with `return f(params)`, which constrains
                // nothing: the cut-off returns must not type the function
                // as returning `None`.
                body.truncate(stmt);
                body.push(HirStmt::Return(Some(HirExpr::Call {
                    callee: name.clone(),
                    args: params
                        .iter()
                        .map(|(param, _)| HirExpr::Name(param.clone()))
                        .collect(),
                })));
            }
        }
        let probe_names = crate::module::module_function_local_names(&probe);
        let Some(signatures) =
            crate::constraints::return_fallback::partial_signatures(&probe, &probe_names)
        else {
            break;
        };
        let env = module_environment(&probe, &signatures);
        let local_names = crate::module::module_function_local_names(&current);
        let mut decisions: Vec<(usize, usize, bool, Vec<HirExpr>)> = Vec::new();
        for &(item, stmt) in &pending {
            let HirItem::Function {
                name, params, body, ..
            } = &current.items[item]
            else {
                continue;
            };
            let Some((param_tys, _)) = signatures.get(name) else {
                continue;
            };
            let Some(test) = guard_test(&body[stmt], shadowed) else {
                continue;
            };
            let names = &local_names[item];
            if let Some((value, subjects)) =
                decide_at(&env, names, params, param_tys, &body[..stmt], test, deques)
            {
                folds.push(Fold {
                    function: name.clone(),
                    at: stmt,
                    test: test.clone(),
                    value,
                });
                decisions.push((item, stmt, value, subjects));
            }
        }
        if decisions.is_empty() {
            break;
        }
        for (item, stmt, value, subjects) in decisions {
            let HirItem::Function { body, .. } = &mut current.items[item] else {
                continue;
            };
            let HirStmt::If {
                body: then, orelse, ..
            } = body.remove(stmt)
            else {
                unreachable!("a pending guard is an `if`");
            };
            let kept = if value { then } else { orelse };
            let spliced: Vec<HirStmt> = subjects
                .into_iter()
                .map(HirStmt::ExprStmt)
                .chain(kept)
                .collect();
            body.splice(stmt..stmt, spliced);
        }
    }
    (!folds.is_empty()).then_some((current, folds))
}

/// The test of `stmt` when it is an `if` whose test is a guard.
fn guard_test<'a>(stmt: &'a HirStmt, shadowed: &[&str]) -> Option<&'a HirExpr> {
    match stmt {
        HirStmt::If { test, .. } if is_guard(test, shadowed) => Some(test),
        _ => None,
    }
}

/// Whether `test` combines builtin-class `isinstance` calls with `not`,
/// `and` and `or`.
fn is_guard(test: &HirExpr, shadowed: &[&str]) -> bool {
    match test {
        HirExpr::UnaryOp {
            op: UnaryOpKind::Not,
            operand,
        } => is_guard(operand, shadowed),
        HirExpr::BoolOp { left, right, .. } => {
            is_guard(left, shadowed) && is_guard(right, shadowed)
        }
        HirExpr::Call { callee, args } if callee == "isinstance" => {
            matches!(args.as_slice(), [_, class] if classes(class, shadowed).is_some())
        }
        _ => false,
    }
}

/// The builtin classes `class` names (one, or a tuple of them).
fn classes<'a>(class: &'a HirExpr, shadowed: &[&str]) -> Option<Vec<&'a str>> {
    let builtin = |expr: &'a HirExpr| match expr {
        HirExpr::Name(name)
            if CLASSES.contains(&name.as_str()) && !shadowed.contains(&name.as_str()) =>
        {
            Some(name.as_str())
        }
        _ => None,
    };
    match class {
        HirExpr::TupleLiteral(items) if !items.is_empty() => items.iter().map(builtin).collect(),
        other => builtin(other).map(|name| vec![name]),
    }
}

/// The value `test` has on every run, and the subjects of the `isinstance`
/// calls it evaluates, in order; `None` when a subject it evaluates has no
/// type that decides its test.
fn decide(
    test: &HirExpr,
    type_of: &dyn Fn(&HirExpr) -> Option<Ty>,
    deques: bool,
) -> Option<(bool, Vec<HirExpr>)> {
    match test {
        HirExpr::UnaryOp {
            op: UnaryOpKind::Not,
            operand,
        } => decide(operand, type_of, deques).map(|(value, subjects)| (!value, subjects)),
        HirExpr::BoolOp {
            op, left, right, ..
        } => {
            let (left, mut subjects) = decide(left, type_of, deques)?;
            // `a and b` stops at a false `a`, `a or b` at a true one.
            if left == (*op == BoolOpKind::Or) {
                return Some((left, subjects));
            }
            let (right, more) = decide(right, type_of, deques)?;
            subjects.extend(more);
            Some((right, subjects))
        }
        HirExpr::Call { args, .. } => {
            let [subject, class] = args.as_slice() else {
                return None;
            };
            let names = classes(class, &[])?;
            let ty = type_of(subject)?;
            let mut value = Some(false);
            for name in names {
                match instance_of(&ty, name, deques) {
                    Some(true) => {
                        value = Some(true);
                        break;
                    }
                    Some(false) => {}
                    None => value = None,
                }
            }
            value.map(|value| (value, vec![subject.clone()]))
        }
        _ => None,
    }
}

/// Whether every value of static type `ty` is an instance of the builtin
/// class `class`: `None` when that depends on the value.
fn instance_of(ty: &Ty, class: &str, deques: bool) -> Option<bool> {
    let exactly = |owner: &str| class == owner;
    match ty {
        // A `deque` is typed `list[T]` (`pycc_ast::desugar::collections`).
        Ty::List(_) if exactly("list") => (!deques).then_some(true),
        Ty::List(_) => Some(false),
        Ty::Tuple(_) | Ty::VarTuple(_) => Some(exactly("tuple")),
        // `Counter` and `defaultdict` are `dict[K, V]`: `dict` subclasses.
        Ty::Dict(_) => Some(exactly("dict")),
        Ty::Set(_) => Some(exactly("set")),
        Ty::FrozenSet(_) => Some(exactly("frozenset")),
        Ty::Str => Some(exactly("str")),
        Ty::None => Some(false),
        // An `int` slot may hold a `bool`.
        Ty::Int => match class {
            "int" => Some(true),
            "bool" => None,
            _ => Some(false),
        },
        Ty::Bool => Some(matches!(class, "int" | "bool")),
        // A number may hold either kind.
        Ty::Float | Ty::Num => match class {
            "int" | "float" | "bool" => None,
            _ => Some(false),
        },
        Ty::Optional(inner) => match instance_of(inner, class, deques) {
            Some(false) => Some(false),
            _ => None,
        },
        _ => None,
    }
}

/// Whether `ty` is fully known.
fn concrete(ty: &Ty) -> bool {
    !crate::empty_container::contains_infer(ty) && !matches!(ty, Ty::Object)
}

/// The builtin class names (and `isinstance`) `hir` binds at module level
/// or as a function or class: no longer the builtins.
fn shadowed_builtins(hir: &HirModule) -> Vec<&'static str> {
    let mut bound: Vec<&str> = Vec::new();
    for item in &hir.items {
        match item {
            HirItem::Function { name, .. } => bound.push(name),
            HirItem::TopLevelStmt(HirStmt::Assign { target, .. }) => bound.push(target),
            HirItem::TopLevelStmt(_) => {}
        }
    }
    bound.extend(hir.class_defs.iter().map(|(name, _)| name.as_str()));
    CLASSES
        .iter()
        .chain(&["isinstance"])
        .copied()
        .filter(|builtin| bound.contains(builtin))
        .collect()
}

/// Whether `hir` builds a `deque` anywhere.
fn creates_deques(hir: &HirModule) -> bool {
    let mut module = hir.clone();
    let mut found = false;
    let mut visit = |expr: &mut HirExpr| {
        if let HirExpr::Call { callee, .. } = expr
            && callee == "__pycc_deque"
        {
            found = true;
        }
        false
    };
    for_each_expr(&mut module, &mut visit);
    found
}

/// Calls `visit` on every expression of every item of `hir`, depth first.
fn for_each_expr(hir: &mut HirModule, visit: &mut dyn FnMut(&mut HirExpr) -> bool) {
    for item in &mut hir.items {
        for_each_item_expr(item, visit);
    }
}

/// Calls `visit` on every expression of `item`, depth first.
fn for_each_item_expr(item: &mut HirItem, visit: &mut dyn FnMut(&mut HirExpr) -> bool) {
    use crate::empty_container::{for_each_stmt_mut, stmt_exprs_mut, walk_expr_mut};
    let mut each_stmt = |stmt: &mut HirStmt| {
        stmt_exprs_mut(stmt, &mut |expr| walk_expr_mut(expr, visit));
    };
    match item {
        HirItem::Function { body, .. } => for_each_stmt_mut(body, &mut each_stmt),
        HirItem::TopLevelStmt(stmt) => {
            for_each_stmt_mut(std::slice::from_mut(stmt), &mut each_stmt)
        }
    }
}

/// The module-level environment a guard's subject is typed in: every class,
/// every function under its (partially) solved signature, and the module's
/// globals.
fn module_environment(
    hir: &HirModule,
    signatures: &crate::module::FunctionSignatures,
) -> Environment {
    let mut env = Environment::new();
    crate::class::bind_classes(&mut env, hir);
    for item in &hir.items {
        if let HirItem::Function {
            name,
            params,
            return_ty,
            ..
        } = item
        {
            let (param_tys, return_ty) = signatures.get(name).cloned().unwrap_or_else(|| {
                (
                    params.iter().map(|(_, ty)| ty.clone()).collect(),
                    return_ty.clone(),
                )
            });
            env.bind_function(name.clone(), param_tys, return_ty);
        }
    }
    let globals: Vec<HirStmt> = hir
        .items
        .iter()
        .filter_map(|item| match item {
            HirItem::TopLevelStmt(stmt) => Some(stmt.clone()),
            HirItem::Function { .. } => None,
        })
        .collect();
    bind_local_types_in_body(&mut env, &[], &globals);
    env
}

#[cfg(test)]
mod tests;
