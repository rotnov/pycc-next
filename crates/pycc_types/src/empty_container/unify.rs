//! Bidirectional element-type inference for empty containers.
//!
//! The sources of [`super::resolve`] type an empty display bound to a name
//! from that name's own evidence. This pass types *every* remaining empty
//! container -- `[]`, `{}`, `set()`, `list()`, `dict()` -- from the types it
//! flows into and out of, by unification over the whole module:
//!
//! * each empty container is a container type with fresh element variables;
//! * a name is one variable per scope, unified with everything assigned to
//!   it, and a module global is shared with every function that reads it
//!   without binding it;
//! * producers add element evidence: `xs.append(v)`, `s.add(v)`,
//!   `d[k] = v`, `xs.insert(i, v)`, `xs.extend(ys)`, `d.setdefault(k, v)`;
//! * expected types flow in: an argument passed to a function parameter of a
//!   known type, a returned value against the declared return type, either
//!   operand of `==`/`!=`/`+`, an element read (`t = xs[i]`, `for t in xs`)
//!   against the container's element;
//! * values flow through: an unannotated helper's call returns what its
//!   `return`s unify to (the solver types helpers only after this pass),
//!   `sorted(xs)` / `list(xs)` / a comprehension list `xs`'s elements, an
//!   unpacking target is an element (or tuple item) of its value, and a
//!   call the checker cannot type yet is typed again from what its
//!   arguments unified to (`list(map(int, sr().split()))`);
//! * a container of empty containers (`[[] for _ in range(n)]`, `[[]] * n`,
//!   `[set(), set()]`) gets its element type from how its elements are used
//!   (`g[i].append(v)`, which the desugaring reads through a temporary).
//!
//! A variable that meets two different concrete types is *conflicted*, and
//! no empty container whose type touches it is rewritten. Every rewrite is
//! re-validated by the check phase in program order, so a resolution can
//! only ever be refused there, never compiled wrongly: an empty container
//! this pass leaves alone keeps the behavior it had (`T0003` in strict mode,
//! a CPython container in non-strict mode).

use super::*;
use std::collections::{HashMap, HashSet};

mod settle;
mod sources;
mod store;
mod walk;
use store::{Shape, Store};
pub(super) use walk::{stmt_exprs_mut, walk_expr_mut};
use walk::all_stmts_mut;

/// Which empty container a site is.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SiteKind {
    List,
    Dict,
    Set,
    FrozenSet,
}

/// A deferred constraint, applied once its container's shape is known.
enum Deferred {
    /// `result` is an element read of `base` (`base[i]`, a `for` target).
    Element {
        base: usize,
        result: usize,
        iterate: bool,
    },
    /// `items` are `base` unpacked (`a, b = base`, a `for a, b in xs`
    /// target): each a list's element, or a tuple's item at its position.
    Unpack { base: usize, items: Vec<usize> },
    /// `base[key] = value`.
    Store {
        base: usize,
        key: usize,
        value: usize,
    },
    /// `result = left <op> right` over scalars not known when visited (an
    /// element read of a container typed later).
    BinOp {
        op: pycc_hir::BinOpKind,
        left: usize,
        right: usize,
        result: usize,
    },
}

/// The engine's state for one module.
struct Engine {
    store: Store,
    /// Site number -> its container variable, made at the site's first
    /// visit.
    sites: Vec<Option<usize>>,
    site_kinds: Vec<SiteKind>,
    /// (scope, name) -> variable; scope `0` is the module.
    names: HashMap<(usize, String), usize>,
    /// Module function name -> the variable its `return`s unify with.
    returns: HashMap<String, usize>,
    /// Module function name -> whether every parameter is concretely
    /// annotated.
    helpers: HashMap<String, bool>,
    deferred: Vec<Deferred>,
}

/// The scope a statement list is checked in.
struct Scope<'a> {
    id: usize,
    env: &'a Environment,
    local_names: &'a [&'a str],
    /// The names this scope binds (a function's locals); every other name a
    /// function reads is the module's.
    locals: Option<&'a [&'a str]>,
    return_ty: Option<&'a Ty>,
    /// The module function this scope is the body of.
    function: Option<&'a str>,
    /// Names bound by more than one syntactic site in this scope: the
    /// environment's flat type for one of them may come from a different,
    /// mutually exclusive site, so it is never trusted (each site's own
    /// value is unified instead).
    multi_site: HashSet<String>,
}

/// The names bound by more than one site among `bodies`.
fn multi_site_names(bodies: &[&[HirStmt]]) -> HashSet<String> {
    let mut sites: Vec<&str> = Vec::new();
    for body in bodies {
        super::collect_binding_sites(body, &mut sites);
    }
    sites
        .iter()
        .filter(|name| super::bound_by_several_sites(&sites, name))
        .map(|name| (*name).to_string())
        .collect()
}

impl Engine {
    fn name_var(&mut self, scope: &Scope<'_>, name: &str) -> usize {
        let scope_id = match scope.locals {
            Some(locals) if !locals.contains(&name) => 0,
            _ => scope.id,
        };
        if let Some(var) = self.names.get(&(scope_id, name.to_string())) {
            return *var;
        }
        let var = self.store.fresh();
        self.names.insert((scope_id, name.to_string()), var);
        var
    }

    /// A call of module function `name` whose return type is not inferred
    /// yet (the solver runs after this pass): what its `return`s unify to,
    /// when no parameter is left to infer -- a helper generic over one
    /// would return a different type per call.
    fn helper_return(&mut self, name: &str) -> Option<usize> {
        self.helpers
            .get(name)
            .copied()
            .unwrap_or(false)
            .then(|| self.return_var(name))
    }

    /// The variable of module function `name`'s returned values.
    fn return_var(&mut self, name: &str) -> usize {
        if let Some(var) = self.returns.get(name) {
            return *var;
        }
        let var = self.store.fresh();
        self.returns.insert(name.to_string(), var);
        var
    }

    fn site_var(&mut self, number: usize) -> usize {
        if let Some(var) = self.sites[number] {
            return var;
        }
        let var = match self.site_kinds[number] {
            SiteKind::List => {
                let e = self.store.fresh();
                self.store.with_shape(Shape::List(e))
            }
            SiteKind::Set => {
                let e = self.store.fresh();
                self.store.with_shape(Shape::Set(e))
            }
            SiteKind::FrozenSet => {
                let e = self.store.fresh();
                self.store.with_shape(Shape::FrozenSet(e))
            }
            SiteKind::Dict => {
                let k = self.store.fresh();
                let v = self.store.fresh();
                self.store.with_shape(Shape::Dict(k, v))
            }
        };
        self.sites[number] = Some(var);
        var
    }

    /// The variable of an expression's type, visiting every sub-expression
    /// (so every site gets its number in a fixed order).
    fn term(&mut self, scope: &Scope<'_>, expr: &HirExpr) -> Option<usize> {
        if let HirExpr::Name(name) = expr
            && let Some(number) = site_number(name)
        {
            return Some(self.site_var(number));
        }
        match expr {
            HirExpr::Name(name) => {
                let var = self.name_var(scope, name);
                let root = self.store.find(var);
                if self.store.shape[root].is_none()
                    && !scope.multi_site.contains(name)
                    && let Some(ty) = known_binding(scope.env, name)
                {
                    let known = self.store.known(ty);
                    self.store.unify(var, known);
                }
                Some(var)
            }
            HirExpr::ListLiteral(elements) => {
                let elem = self.store.fresh();
                for element in elements {
                    if let Some(t) = self.term(scope, element) {
                        self.store.unify(elem, t);
                    }
                }
                Some(self.store.with_shape(Shape::List(elem)))
            }
            HirExpr::SetLiteral(elements) => {
                let elem = self.store.fresh();
                for element in elements {
                    if let Some(t) = self.term(scope, element) {
                        self.store.unify(elem, t);
                    }
                }
                Some(self.store.with_shape(Shape::Set(elem)))
            }
            HirExpr::DictLiteral(pairs) => {
                let (k, v) = (self.store.fresh(), self.store.fresh());
                for (key, value) in pairs {
                    if let Some(t) = self.term(scope, key) {
                        self.store.unify(k, t);
                    }
                    if let Some(t) = self.term(scope, value) {
                        self.store.unify(v, t);
                    }
                }
                Some(self.store.with_shape(Shape::Dict(k, v)))
            }
            HirExpr::Subscript { base, index } => {
                let base_var = self.term(scope, base);
                self.term(scope, index);
                let base_var = base_var?;
                // A constant index into a tuple (an unpacking temporary's
                // `t[0]`): that item.
                if let HirExpr::IntLiteral(i) = index.as_ref()
                    && let Some(Shape::Tuple(items)) = self.store.open(base_var)
                {
                    let len = items.len() as i64;
                    let at = if *i < 0 { *i + len } else { *i };
                    return usize::try_from(at).ok().and_then(|at| items.get(at).copied());
                }
                let result = self.store.fresh();
                self.deferred.push(Deferred::Element {
                    base: base_var,
                    result,
                    iterate: false,
                });
                Some(result)
            }
            HirExpr::BinOp { op, left, right } | HirExpr::AugBinOp { op, left, right } => {
                let l = self.term(scope, left);
                let r = self.term(scope, right);
                match op {
                    // `xs + ys` joins two lists; `[[]] * n` is its list.
                    pycc_hir::BinOpKind::Add => {
                        if let (Some(l), Some(r)) = (l, r)
                            && (self.is_container(l) || self.is_container(r))
                        {
                            self.store.unify(l, r);
                            return Some(l);
                        }
                    }
                    pycc_hir::BinOpKind::Mul => {
                        if let Some(l) = l
                            && self.is_list(l)
                        {
                            return Some(l);
                        }
                        if let Some(r) = r
                            && self.is_list(r)
                        {
                            return Some(r);
                        }
                    }
                    _ => {}
                }
                // Two operands already known as scalars: the operator's own
                // result type (`d.get(k, 0) + 1` is an `int`).
                if let (Some(l), Some(r)) = (l, r)
                    && let (Some(lt), Some(rt)) =
                        (self.store.resolve(l, 0), self.store.resolve(r, 0))
                    && matches!(lt, Ty::Int | Ty::Float | Ty::Bool | Ty::Str)
                    && matches!(rt, Ty::Int | Ty::Float | Ty::Bool | Ty::Str)
                    && let Ok(ty) = crate::binop::numeric_result_type(*op, lt, rt)
                {
                    return Some(self.store.known(ty));
                }
                if let Some(known) = self.inferred(scope, expr) {
                    return Some(known);
                }
                let (left, right) = (l?, r?);
                let result = self.store.fresh();
                self.deferred.push(Deferred::BinOp {
                    op: *op,
                    left,
                    right,
                    result,
                });
                Some(result)
            }
            HirExpr::DictGetOrDefault {
                dict: pycc_hir::ContainerReceiver::Name(name),
                key,
                default,
            } => {
                let base = self.name_var(scope, name);
                let k = self.term(scope, key);
                let d = self.term(scope, default);
                let result = self.store.fresh();
                self.deferred.push(Deferred::Element {
                    base,
                    result,
                    iterate: false,
                });
                if let Some(k) = k {
                    self.deferred.push(Deferred::Element {
                        base,
                        result: k,
                        iterate: true,
                    });
                }
                if let Some(d) = d {
                    self.store.unify(result, d);
                }
                Some(result)
            }
            // `-x`, `+x`, `~x` over a scalar typed later: the type of `0 - x`
            // (or `0 ^ x`), which is CPython's for every numeric operand
            // (`-True` is an `int`; `~` of a float has none).
            HirExpr::UnaryOp {
                op: op @ (pycc_hir::UnaryOpKind::USub
                | pycc_hir::UnaryOpKind::UAdd
                | pycc_hir::UnaryOpKind::Invert),
                operand,
            } => {
                let operand_var = self.term(scope, operand);
                if let Some(known) = self.inferred(scope, expr) {
                    return Some(known);
                }
                let right = operand_var?;
                let left = self.store.known(Ty::Int);
                let result = self.store.fresh();
                self.deferred.push(Deferred::BinOp {
                    op: if matches!(op, pycc_hir::UnaryOpKind::Invert) {
                        pycc_hir::BinOpKind::BitXor
                    } else {
                        pycc_hir::BinOpKind::Sub
                    },
                    left,
                    right,
                    result,
                });
                Some(result)
            }
            HirExpr::MethodCall { base, method, args } => {
                self.method_call(scope, expr, base, method, args)
            }
            HirExpr::Unpack { value, arity } => {
                let base = self.term(scope, value);
                let items: Vec<usize> = (0..*arity).map(|_| self.store.fresh()).collect();
                if let Some(base) = base {
                    self.deferred.push(Deferred::Unpack {
                        base,
                        items: items.clone(),
                    });
                }
                Some(self.store.with_shape(Shape::Tuple(items)))
            }
            HirExpr::TupleLiteral(items) if !items.is_empty() => {
                let vars: Vec<Option<usize>> =
                    items.iter().map(|item| self.term(scope, item)).collect();
                let vars = vars
                    .into_iter()
                    .map(|var| var.unwrap_or_else(|| self.store.fresh()))
                    .collect();
                Some(self.store.with_shape(Shape::Tuple(vars)))
            }
            HirExpr::Compare { op, left, right } => {
                let l = self.term(scope, left);
                let r = self.term(scope, right);
                if matches!(op, pycc_hir::CmpOpKind::Eq | pycc_hir::CmpOpKind::NotEq)
                    && let (Some(l), Some(r)) = (l, r)
                    && (self.is_container(l) || self.is_container(r))
                {
                    self.store.unify(l, r);
                }
                Some(self.store.known(Ty::Bool))
            }
            HirExpr::IfExp { test, body, orelse } => {
                self.term(scope, test);
                let b = self.term(scope, body);
                let o = self.term(scope, orelse);
                match (b, o) {
                    (Some(b), Some(o)) => {
                        self.store.unify(b, o);
                        Some(b)
                    }
                    _ => None,
                }
            }
            HirExpr::Call { callee, args } => {
                let arg_vars: Vec<Option<usize>> =
                    args.iter().map(|arg| self.term(scope, arg)).collect();
                if let Some((params, ret)) = scope.env.functions.get(callee).cloned() {
                    for (var, param) in arg_vars.iter().zip(params.iter()) {
                        if let Some(var) = var
                            && concrete_ty(param)
                        {
                            let known = self.store.known(param.clone());
                            self.store.unify(*var, known);
                        }
                    }
                    if concrete_ty(&ret) {
                        return Some(self.store.known(ret));
                    }
                    return self.helper_return(callee);
                }
                if self.helpers.contains_key(callee) {
                    return self.helper_return(callee);
                }
                if let Some(native) = self.native_container_call(callee, &arg_vars) {
                    return Some(native);
                }
                if let Some(known) = self.inferred(scope, expr) {
                    return Some(known);
                }
                if let Some(known) = self.retyped_call(scope, callee, args, &arg_vars) {
                    return Some(known);
                }
                self.element_preserving_call(callee, &arg_vars)
            }
            HirExpr::Comprehension(comp) => {
                let shaped = self.comprehension(scope, comp);
                if let Some(known) = self.inferred(scope, expr) {
                    self.store.unify(shaped, known);
                }
                Some(shaped)
            }
            _ => {
                for sub in direct_children(expr) {
                    self.term(scope, sub);
                }
                self.inferred(scope, expr)
            }
        }
    }

    /// `expr`'s type as the checker infers it now, when that succeeds and
    /// is concrete.
    fn inferred(&mut self, scope: &Scope<'_>, expr: &HirExpr) -> Option<usize> {
        let ty = crate::expr::infer_expr_in(scope.env, scope.local_names, expr).ok()?;
        concrete_ty(&ty).then(|| self.store.known(ty))
    }

    fn is_container(&mut self, var: usize) -> bool {
        matches!(
            self.store.open(var),
            Some(Shape::List(_) | Shape::Set(_) | Shape::Dict(..))
        )
    }

    fn is_list(&mut self, var: usize) -> bool {
        matches!(self.store.open(var), Some(Shape::List(_)))
    }

    /// Unifies `name`'s variable with `shape(elem)` for a producer.
    fn produce(&mut self, scope: &Scope<'_>, name: &str, make: impl FnOnce(&mut Store) -> Shape) {
        let var = self.name_var(scope, name);
        let shape = make(&mut self.store);
        let produced = self.store.with_shape(shape);
        self.store.unify(var, produced);
    }

    fn stmts(&mut self, scope: &Scope<'_>, body: &[HirStmt]) {
        for stmt in body {
            self.stmt(scope, stmt);
        }
    }

    fn stmt(&mut self, scope: &Scope<'_>, stmt: &HirStmt) {
        match stmt {
            HirStmt::ForRange {
                var,
                start,
                stop,
                step,
                body,
            } => {
                for bound in [start, stop, step] {
                    self.term(scope, bound);
                }
                let target = self.name_var(scope, var);
                let int = self.store.known(Ty::Int);
                self.store.unify(target, int);
                self.stmts(scope, body);
            }
            HirStmt::Assign { target, value } => {
                let var = self.name_var(scope, target);
                if let Some(t) = self.term(scope, value) {
                    // A name rebound with a different scalar type (`v =
                    // True` here, `v = 1` there) has no one type: the
                    // `bool`-into-`int` join is for container slots only.
                    if let (Some(old), Some(new)) =
                        (self.store.resolve(var, 0), self.store.resolve(t, 0))
                        && old != new
                    {
                        self.store.conflict(var);
                    }
                    self.store.unify(var, t);
                }
            }
            HirStmt::AnnAssign {
                target,
                annotation,
                value,
                ..
            } => {
                let var = self.name_var(scope, target);
                if concrete_ty(annotation) {
                    let known = self.store.known(annotation.clone());
                    self.store.unify(var, known);
                }
                if let Some(value) = value
                    && let Some(t) = self.term(scope, value)
                {
                    self.store.unify(var, t);
                }
            }
            HirStmt::ExprStmt(expr) => self.effect(scope, expr),
            HirStmt::DictSet { dict, key, value } => {
                let base = self.name_var(scope, dict);
                let (k, v) = (self.term(scope, key), self.term(scope, value));
                if let (Some(key), Some(value)) = (k, v) {
                    self.deferred.push(Deferred::Store { base, key, value });
                }
            }
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
                let elt = if matches!(stmt, HirStmt::SetCompAssign { .. }) {
                    pycc_hir::CompElt::Set((**elt).clone())
                } else {
                    pycc_hir::CompElt::List((**elt).clone())
                };
                self.comprehension_assign(scope, target, var, iter, cond, elt);
            }
            HirStmt::DictCompAssign {
                target,
                var,
                iter,
                cond,
                key,
                value,
            } => {
                let elt = pycc_hir::CompElt::Dict {
                    key: (**key).clone(),
                    value: (**value).clone(),
                };
                self.comprehension_assign(scope, target, var, iter, cond, elt);
            }
            HirStmt::Return(Some(value)) => {
                let Some(t) = self.term(scope, value) else {
                    return;
                };
                if let Some(ret) = scope.return_ty
                    && concrete_ty(ret)
                {
                    let known = self.store.known(ret.clone());
                    self.store.unify(t, known);
                } else if let Some(function) = scope.function {
                    // An unannotated helper: its calls read this variable.
                    let ret = self.return_var(function);
                    self.store.unify(ret, t);
                }
            }
            HirStmt::ForList {
                var, list, body, ..
            } => {
                // Through `term`, so a concretely bound iterable or loop
                // variable is seeded with its type.
                let Some(base) = self.term(scope, &HirExpr::Name(list.clone())) else {
                    self.stmts(scope, body);
                    return;
                };
                let Some(target) = self.term(scope, &HirExpr::Name(var.clone())) else {
                    self.stmts(scope, body);
                    return;
                };
                self.deferred.push(Deferred::Element {
                    base,
                    result: target,
                    iterate: true,
                });
                self.stmts(scope, body);
            }
            HirStmt::If { test, body, orelse } => {
                self.term(scope, test);
                self.stmts(scope, body);
                self.stmts(scope, orelse);
            }
            HirStmt::While { test, body } => {
                self.term(scope, test);
                self.stmts(scope, body);
            }
            other => {
                for nested in nested_bodies(other) {
                    self.stmts(scope, nested);
                }
            }
        }
    }

    /// An expression statement: a producer call, or an expression visited
    /// for the sites it holds.
    fn effect(&mut self, scope: &Scope<'_>, expr: &HirExpr) {
        match expr {
            HirExpr::ListAppend {
                list: pycc_hir::ContainerReceiver::Name(name),
                value,
            } => {
                if let Some(v) = self.term(scope, value) {
                    self.produce(scope, name, |_| Shape::List(v));
                }
            }
            HirExpr::SetAdd { set, value } => {
                if let Some(v) = self.term(scope, value) {
                    self.produce(scope, set, |_| Shape::Set(v));
                }
            }
            HirExpr::MethodCall { base, method, args } => {
                let base_name = match base.as_ref() {
                    HirExpr::Name(name) => Some(name.clone()),
                    _ => None,
                };
                let arg_vars: Vec<Option<usize>> =
                    args.iter().map(|arg| self.term(scope, arg)).collect();
                self.term(scope, base);
                let Some(name) = base_name else {
                    return;
                };
                match (method.as_str(), arg_vars.as_slice()) {
                    ("append", [Some(v)]) => self.produce(scope, &name, |_| Shape::List(*v)),
                    ("add", [Some(v)]) => self.produce(scope, &name, |_| Shape::Set(*v)),
                    ("insert", [_, Some(v)]) => self.produce(scope, &name, |_| Shape::List(*v)),
                    ("extend" | "update", [Some(other)]) => {
                        let var = self.name_var(scope, &name);
                        if self.is_container(*other) {
                            self.store.unify(var, *other);
                        }
                    }
                    ("setdefault", [Some(k), Some(v)]) => {
                        self.produce(scope, &name, |_| Shape::Dict(*k, *v))
                    }
                    _ => {}
                }
            }
            HirExpr::ReceiverDispatchedCall { call, .. } => self.effect(scope, call),
            // `heapq.heappush(h, v)` / `bisect.insort(a, v)` add `v` to the
            // list (`pycc_ast::desugar::stdlib`'s spelling).
            HirExpr::Call { callee, args }
                if matches!(
                    callee.as_str(),
                    "__pycc_heappush" | "__pycc_insort_left" | "__pycc_insort_right"
                ) && args.len() == 2
                    && matches!(&args[0], HirExpr::Name(_)) =>
            {
                let HirExpr::Name(name) = &args[0] else {
                    unreachable!()
                };
                self.term(scope, &args[0]);
                if let Some(v) = self.term(scope, &args[1]) {
                    self.produce(scope, name, |_| Shape::List(v));
                }
            }
            other => {
                self.term(scope, other);
            }
        }
    }
}

/// `name`'s binding in `env`, when it is concrete.
fn known_binding(env: &Environment, name: &str) -> Option<Ty> {
    let ty = env.binding_state(name).map(BindingState::ty)?.clone();
    concrete_ty(&ty).then_some(ty)
}

/// Whether `ty` holds no `Ty::Infer`, type parameter, `Optional` or object.
pub(super) fn concrete_ty(ty: &Ty) -> bool {
    !contains_infer(ty)
        && !contains_optional(ty)
        && !matches!(ty, Ty::Param(_) | Ty::Object | Ty::Infer)
}

/// The direct sub-expressions of `expr` this pass does not type itself.
fn direct_children(expr: &HirExpr) -> Vec<&HirExpr> {
    match expr {
        HirExpr::UnaryOp { operand, .. } => vec![operand],
        HirExpr::BoolOp { left, right, .. } => vec![left, right],
        HirExpr::TupleLiteral(elements) => elements.iter().collect(),
        HirExpr::MethodCall { base, args, .. } => {
            let mut out: Vec<&HirExpr> = vec![base];
            out.extend(args.iter());
            out
        }
        HirExpr::ListAppend { value, .. } | HirExpr::SetAdd { value, .. } => vec![value],
        HirExpr::DictGetOrDefault { key, default, .. } => vec![key, default],
        HirExpr::AttrGet { base, .. } => vec![base],
        HirExpr::FString(parts) => parts
            .iter()
            .filter_map(|part| match part {
                FStringPart::Interpolation(inner) => Some(inner.as_ref()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// The prefix of a numbered site's placeholder name: it starts with a
/// digit, so no source name can equal it.
const SITE_PREFIX: &str = "0site";

/// The site number a placeholder name carries.
fn site_number(name: &str) -> Option<usize> {
    name.strip_prefix(SITE_PREFIX)?.parse().ok()
}

/// Replaces every empty container of `hir` with a numbered placeholder
/// name, returning the originals by number.
fn number_sites(
    hir: &mut HirModule,
    functions: &HashMap<String, (Vec<Ty>, Ty)>,
) -> Vec<(SiteKind, HirExpr)> {
    let mut sites: Vec<(SiteKind, HirExpr)> = Vec::new();
    let mut visit_stmt = |stmt: &mut HirStmt| {
        stmt_exprs_mut(stmt, &mut |expr| {
            walk_expr_mut(expr, &mut |node| {
                let Some(kind) = site_kind_in(node, functions) else {
                    return false;
                };
                let placeholder = HirExpr::Name(format!("{SITE_PREFIX}{}", sites.len()));
                sites.push((kind, std::mem::replace(node, placeholder)));
                true
            });
        });
    };
    for item in &mut hir.items {
        match item {
            HirItem::Function { body, .. } => all_stmts_mut(body, &mut visit_stmt),
            HirItem::TopLevelStmt(stmt) => {
                all_stmts_mut(std::slice::from_mut(stmt), &mut visit_stmt)
            }
        }
    }
    sites
}

/// Puts each numbered site back: its resolved typed node, or the original.
fn restore_sites(hir: &mut HirModule, sites: Vec<(SiteKind, HirExpr)>, types: &[Option<Ty>]) {
    let mut originals: Vec<Option<(SiteKind, HirExpr)>> = sites.into_iter().map(Some).collect();
    let mut visit_stmt = |stmt: &mut HirStmt| {
        stmt_exprs_mut(stmt, &mut |expr| {
            walk_expr_mut(expr, &mut |node| {
                let HirExpr::Name(name) = node else {
                    return false;
                };
                let Some(number) = site_number(name) else {
                    return false;
                };
                let Some((kind, original)) = originals.get_mut(number).and_then(Option::take)
                else {
                    return true;
                };
                *node = match (kind, types.get(number).cloned().flatten()) {
                    // Non-strict mode (#3): a list of objects is a CPython list.
                    (SiteKind::List, Some(Ty::List(elem)))
                        if *elem == Ty::Object && crate::non_strict::is_non_strict() =>
                    {
                        HirExpr::ObjectList(Vec::new())
                    }
                    (SiteKind::List, Some(Ty::List(elem))) => HirExpr::EmptyList(*elem),
                    (SiteKind::Dict, Some(Ty::Dict(pair))) => HirExpr::EmptyDict(pair),
                    (SiteKind::Set, Some(Ty::Set(elem))) => HirExpr::EmptySet(*elem),
                    // `frozenset()` of the resolved element type: the
                    // frozenset of an empty set of it.
                    (SiteKind::FrozenSet, Some(Ty::FrozenSet(elem))) => HirExpr::Call {
                        callee: "frozenset".to_string(),
                        args: vec![HirExpr::EmptySet(*elem)],
                    },
                    _ => original,
                };
                true
            });
        });
    };
    for item in &mut hir.items {
        match item {
            HirItem::Function { body, .. } => all_stmts_mut(body, &mut visit_stmt),
            HirItem::TopLevelStmt(stmt) => {
                all_stmts_mut(std::slice::from_mut(stmt), &mut visit_stmt)
            }
        }
    }
}

/// The empty container `expr` is, when it is one: a display, or a
/// zero-argument `list()`/`dict()`/`set()` call of the builtin (not of a
/// module function of that name).
fn site_kind_in(expr: &HirExpr, functions: &HashMap<String, (Vec<Ty>, Ty)>) -> Option<SiteKind> {
    match expr {
        HirExpr::ListLiteral(elements) if elements.is_empty() => Some(SiteKind::List),
        HirExpr::DictLiteral(pairs) if pairs.is_empty() => Some(SiteKind::Dict),
        HirExpr::SetLiteral(elements) if elements.is_empty() => Some(SiteKind::Set),
        HirExpr::Call { callee, args } if args.is_empty() && !functions.contains_key(callee) => {
            match callee.as_str() {
                "list" => Some(SiteKind::List),
                "dict" => Some(SiteKind::Dict),
                "set" => Some(SiteKind::Set),
                "frozenset" => Some(SiteKind::FrozenSet),
                _ => None,
            }
        }
        _ => None,
    }
}

/// Whether `hir` has an empty container anywhere this pass looks.
pub(super) fn has_sites(hir: &HirModule) -> bool {
    let mut copy = hir.clone();
    !number_sites(&mut copy, &HashMap::new()).is_empty()
}

/// Types every empty container of `hir` that unification resolves (see the
/// module doc comment). `envs[i]` is the producer environment of item `i`
/// when it is a function; `module_env` serves the module body.
pub(super) fn infer(
    hir: &mut HirModule,
    module_env: &Environment,
    envs: &[Option<Environment>],
    local_names: &[Vec<&str>],
    top_level_names: &[&str],
) {
    let functions: HashMap<String, (Vec<Ty>, Ty)> = (*module_env.functions).clone();
    let sites = number_sites(hir, &functions);
    if sites.is_empty() {
        return;
    }
    let mut engine = Engine {
        store: Store::default(),
        sites: vec![None; sites.len()],
        site_kinds: sites.iter().map(|(kind, _)| *kind).collect(),
        names: HashMap::new(),
        returns: HashMap::new(),
        helpers: hir
            .items
            .iter()
            .filter_map(|item| match item {
                HirItem::Function { name, params, .. } => Some((
                    name.clone(),
                    params.iter().all(|(_, ty)| concrete_ty(ty)),
                )),
                HirItem::TopLevelStmt(_) => None,
            })
            .collect(),
        deferred: Vec::new(),
    };
    // Two rounds: the second sees, through the environments' bindings, the
    // names the first round resolved only partially.
    let top_level_bodies: Vec<&[HirStmt]> = hir
        .items
        .iter()
        .filter_map(|item| match item {
            HirItem::TopLevelStmt(stmt) => Some(std::slice::from_ref(stmt)),
            HirItem::Function { .. } => None,
        })
        .collect();
    let module_multi_site = multi_site_names(&top_level_bodies);
    for _ in 0..2 {
        for (index, item) in hir.items.iter().enumerate() {
            match item {
                HirItem::Function {
                    name,
                    params,
                    body,
                    return_ty,
                    ..
                } => {
                    let Some(env) = envs.get(index).and_then(Option::as_ref) else {
                        continue;
                    };
                    let names = &local_names[index];
                    let mut locals: Vec<&str> = names.clone();
                    locals.extend(params.iter().map(|(name, _)| name.as_str()));
                    let scope = Scope {
                        id: index + 1,
                        env,
                        local_names: names,
                        locals: Some(&locals),
                        return_ty: Some(return_ty),
                        function: Some(name),
                        multi_site: multi_site_names(&[body]),
                    };
                    for (name, ty) in params {
                        if concrete_ty(ty) {
                            let var = engine.name_var(&scope, name);
                            let known = engine.store.known(ty.clone());
                            engine.store.unify(var, known);
                        }
                    }
                    engine.stmts(&scope, body);
                }
                HirItem::TopLevelStmt(stmt) => {
                    let scope = Scope {
                        id: 0,
                        env: module_env,
                        local_names: top_level_names,
                        locals: None,
                        return_ty: None,
                        function: None,
                        multi_site: module_multi_site.clone(),
                    };
                    engine.stmt(&scope, stmt);
                }
            }
            engine.settle();
        }
        engine.settle();
    }
    let types: Vec<Option<Ty>> = (0..sites.len())
        .map(|number| {
            let var = engine.sites[number]?;
            let ty = engine.store.resolve(var, 0)?;
            (concrete_ty(&ty) && pycc_hir::check_container_ty(&ty, Span::new(0, 0)).is_ok())
                .then_some(ty)
        })
        .collect();
    restore_sites(hir, sites, &types);
}
