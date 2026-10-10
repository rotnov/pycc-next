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

/// A type with unification variables in it.
#[derive(Clone, Debug)]
enum Shape {
    Known(Ty),
    List(usize),
    Set(usize),
    FrozenSet(usize),
    Dict(usize, usize),
}

#[derive(Default)]
struct Store {
    parent: Vec<usize>,
    shape: Vec<Option<Shape>>,
    conflicted: Vec<bool>,
}

impl Store {
    fn fresh(&mut self) -> usize {
        self.parent.push(self.parent.len());
        self.shape.push(None);
        self.conflicted.push(false);
        self.parent.len() - 1
    }

    fn with_shape(&mut self, shape: Shape) -> usize {
        let var = self.fresh();
        self.shape[var] = Some(shape);
        var
    }

    fn known(&mut self, ty: Ty) -> usize {
        self.with_shape(Shape::Known(ty))
    }

    fn find(&mut self, var: usize) -> usize {
        let mut root = var;
        while self.parent[root] != root {
            root = self.parent[root];
        }
        let mut walk = var;
        while self.parent[walk] != root {
            let next = self.parent[walk];
            self.parent[walk] = root;
            walk = next;
        }
        root
    }

    /// The shape of `var`'s class, with a `Known` container opened into
    /// variables so it unifies structurally.
    fn open(&mut self, var: usize) -> Option<Shape> {
        let root = self.find(var);
        let shape = self.shape[root].clone()?;
        let opened = match shape {
            Shape::Known(Ty::List(elem)) => {
                let e = self.known(*elem);
                Shape::List(e)
            }
            Shape::Known(Ty::Set(elem)) => {
                let e = self.known(*elem);
                Shape::Set(e)
            }
            Shape::Known(Ty::FrozenSet(elem)) => {
                let e = self.known(*elem);
                Shape::FrozenSet(e)
            }
            Shape::Known(Ty::Dict(pair)) => {
                let (k, v) = *pair;
                let k = self.known(k);
                let v = self.known(v);
                Shape::Dict(k, v)
            }
            other => other,
        };
        self.shape[root] = Some(opened.clone());
        Some(opened)
    }

    fn conflict(&mut self, var: usize) {
        let root = self.find(var);
        self.conflicted[root] = true;
    }

    /// Unifies `a` and `b`; a mismatch marks the class conflicted.
    fn unify(&mut self, a: usize, b: usize) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra == rb {
            return;
        }
        let (sa, sb) = (self.open(ra), self.open(rb));
        let conflicted = self.conflicted[ra] || self.conflicted[rb];
        self.parent[rb] = ra;
        self.conflicted[ra] = conflicted;
        match (sa, sb) {
            (None, None) => {}
            (Some(shape), None) | (None, Some(shape)) => self.shape[ra] = Some(shape),
            (Some(Shape::List(x)), Some(Shape::List(y)))
            | (Some(Shape::Set(x)), Some(Shape::Set(y)))
            | (Some(Shape::FrozenSet(x)), Some(Shape::FrozenSet(y))) => self.unify(x, y),
            (Some(Shape::Dict(k1, v1)), Some(Shape::Dict(k2, v2))) => {
                self.unify(k1, k2);
                self.unify(v1, v2);
            }
            (Some(Shape::Known(x)), Some(Shape::Known(y))) => {
                if x != y {
                    // `bool` flows into an `int` slot (`xs.append(True)` on a
                    // `list[int]`); any other pair is a conflict.
                    let joined = match (&x, &y) {
                        (Ty::Int, Ty::Bool) | (Ty::Bool, Ty::Int) => Some(Ty::Int),
                        _ => None,
                    };
                    match joined {
                        Some(ty) => self.shape[ra] = Some(Shape::Known(ty)),
                        None => self.conflicted[ra] = true,
                    }
                }
            }
            _ => self.conflicted[ra] = true,
        }
    }

    /// The concrete type of `var`, when every variable in it is resolved and
    /// none is conflicted.
    fn resolve(&mut self, var: usize, depth: usize) -> Option<Ty> {
        if depth > 16 {
            return None;
        }
        let root = self.find(var);
        if self.conflicted[root] {
            return None;
        }
        match self.shape[root].clone()? {
            Shape::Known(ty) => Some(ty),
            Shape::List(e) => Some(Ty::List(Box::new(self.resolve(e, depth + 1)?))),
            Shape::Set(e) => Some(Ty::Set(Box::new(self.resolve(e, depth + 1)?))),
            Shape::FrozenSet(e) => Some(Ty::FrozenSet(Box::new(self.resolve(e, depth + 1)?))),
            Shape::Dict(k, v) => Some(Ty::Dict(Box::new((
                self.resolve(k, depth + 1)?,
                self.resolve(v, depth + 1)?,
            )))),
        }
    }
}

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
    /// `base[key] = value`.
    Store {
        base: usize,
        key: usize,
        value: usize,
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
                self.inferred(scope, expr)
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
                    return None;
                }
                self.inferred(scope, expr)
            }
            HirExpr::Comprehension(comp) => {
                for sub in comp.sub_exprs() {
                    self.term(scope, sub);
                }
                self.inferred(scope, expr)
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
                target, elt, cond, ..
            }
            | HirStmt::SetCompAssign {
                target, elt, cond, ..
            } => {
                if let Some(cond) = cond {
                    self.term(scope, cond);
                }
                let elem = self.term(scope, elt).unwrap_or_else(|| self.store.fresh());
                let set = matches!(stmt, HirStmt::SetCompAssign { .. });
                self.produce(scope, target, |_| {
                    if set {
                        Shape::Set(elem)
                    } else {
                        Shape::List(elem)
                    }
                });
            }
            HirStmt::DictCompAssign {
                target,
                key,
                value,
                cond,
                ..
            } => {
                if let Some(cond) = cond {
                    self.term(scope, cond);
                }
                let k = self.term(scope, key).unwrap_or_else(|| self.store.fresh());
                let v = self
                    .term(scope, value)
                    .unwrap_or_else(|| self.store.fresh());
                self.produce(scope, target, |_| Shape::Dict(k, v));
            }
            HirStmt::Return(Some(value)) => {
                if let Some(t) = self.term(scope, value)
                    && let Some(ret) = scope.return_ty
                    && concrete_ty(ret)
                {
                    let known = self.store.known(ret.clone());
                    self.store.unify(t, known);
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
            other => {
                self.term(scope, other);
            }
        }
    }

    /// Applies every deferred constraint whose container shape is known,
    /// until nothing changes.
    fn settle(&mut self) {
        loop {
            let mut progressed = false;
            let pending = std::mem::take(&mut self.deferred);
            for constraint in pending {
                let base = match &constraint {
                    Deferred::Element { base, .. } | Deferred::Store { base, .. } => *base,
                };
                match (self.store.open(base), constraint) {
                    (Some(Shape::List(e)), Deferred::Element { result, .. })
                    | (
                        Some(Shape::Set(e)),
                        Deferred::Element {
                            result,
                            iterate: true,
                            ..
                        },
                    ) => {
                        self.store.unify(e, result);
                        progressed = true;
                    }
                    (
                        Some(Shape::Dict(k, v)),
                        Deferred::Element {
                            result, iterate, ..
                        },
                    ) => {
                        self.store.unify(if iterate { k } else { v }, result);
                        progressed = true;
                    }
                    (Some(Shape::List(e)), Deferred::Store { value, .. }) => {
                        self.store.unify(e, value);
                        progressed = true;
                    }
                    (Some(Shape::Dict(k, v)), Deferred::Store { key, value, .. }) => {
                        self.store.unify(k, key);
                        self.store.unify(v, value);
                        progressed = true;
                    }
                    (Some(Shape::Known(Ty::Str)), Deferred::Element { result, .. }) => {
                        let known = self.store.known(Ty::Str);
                        self.store.unify(result, known);
                        progressed = true;
                    }
                    (Some(Shape::Known(Ty::Tuple(_)) | Shape::Known(Ty::Str)), _) => {}
                    (Some(_), _) => {}
                    (None, constraint) => self.deferred.push(constraint),
                }
            }
            if !progressed {
                return;
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

/// Calls `visit` on every expression of `stmt` (not recursing into an
/// expression's children, nor into nested statement bodies).
pub(super) fn stmt_exprs_mut(stmt: &mut HirStmt, visit: &mut dyn FnMut(&mut HirExpr)) {
    match stmt {
        HirStmt::Assign { value, .. } => visit(value),
        HirStmt::AnnAssign {
            value: Some(value), ..
        } => visit(value),
        HirStmt::ExprStmt(expr) => visit(expr),
        HirStmt::If { test, .. } | HirStmt::While { test, .. } => visit(test),
        HirStmt::DictSet { key, value, .. } => {
            visit(key);
            visit(value);
        }
        HirStmt::ListCompAssign {
            iter, cond, elt, ..
        }
        | HirStmt::SetCompAssign {
            iter, cond, elt, ..
        } => {
            comp_iter_mut(iter, visit);
            if let Some(cond) = cond {
                visit(cond);
            }
            visit(elt);
        }
        HirStmt::DictCompAssign {
            iter,
            cond,
            key,
            value,
            ..
        } => {
            comp_iter_mut(iter, visit);
            if let Some(cond) = cond {
                visit(cond);
            }
            visit(key);
            visit(value);
        }
        HirStmt::Return(Some(value)) => visit(value),
        HirStmt::AttrSet { value, .. } => visit(value),
        HirStmt::ForObject { iter, .. } => visit(iter),
        _ => {}
    }
}

fn comp_iter_mut(iter: &mut pycc_hir::CompIter, visit: &mut dyn FnMut(&mut HirExpr)) {
    match iter {
        pycc_hir::CompIter::Range { start, stop, step } => {
            visit(start);
            visit(stop);
            visit(step);
        }
        pycc_hir::CompIter::Iterable(iterable) => visit(iterable),
        pycc_hir::CompIter::Name(_) => {}
    }
}

/// Calls `visit` on `expr` and then on every sub-expression, depth first.
/// `visit` returning `true` stops the descent below that node.
pub(super) fn walk_expr_mut(expr: &mut HirExpr, visit: &mut dyn FnMut(&mut HirExpr) -> bool) {
    if visit(expr) {
        return;
    }
    let mut each = |child: &mut HirExpr| walk_expr_mut(child, visit);
    match expr {
        HirExpr::Call { args, .. }
        | HirExpr::ListLiteral(args)
        | HirExpr::SetLiteral(args)
        | HirExpr::TupleLiteral(args)
        | HirExpr::ObjectList(args)
        | HirExpr::ReceiverClassCall { args }
        | HirExpr::GenericClassInstantiate { args, .. } => args.iter_mut().for_each(&mut each),
        HirExpr::BinOp { left, right, .. }
        | HirExpr::AugBinOp { left, right, .. }
        | HirExpr::Compare { left, right, .. }
        | HirExpr::BoolOp { left, right, .. } => {
            each(left);
            each(right);
        }
        HirExpr::CompareChain { first, links } => {
            each(first);
            for link in links {
                each(&mut link.right);
            }
        }
        HirExpr::UnaryOp { operand, .. } => each(operand),
        HirExpr::IfExp { test, body, orelse } => {
            each(test);
            each(body);
            each(orelse);
        }
        HirExpr::Unpack { value, .. } | HirExpr::NamedExpr { value, .. } => each(value),
        HirExpr::FString(parts) => {
            for part in parts {
                if let FStringPart::Interpolation(inner) = part {
                    each(inner);
                }
            }
        }
        HirExpr::Subscript { base, index } => {
            each(base);
            each(index);
        }
        HirExpr::Slice {
            base,
            start,
            stop,
            step,
        } => {
            each(base);
            for bound in [start, stop, step].into_iter().flatten() {
                each(bound);
            }
        }
        HirExpr::ListAppend { list, value } => {
            if let Some(attr) = list.attr_expr_mut() {
                each(attr);
            }
            each(value);
        }
        HirExpr::DictLiteral(pairs) => {
            for (key, value) in pairs {
                each(key);
                each(value);
            }
        }
        HirExpr::DictGetOrDefault { key, default, .. } => {
            each(key);
            each(default);
        }
        HirExpr::SetAdd { value, .. } => each(value),
        HirExpr::AttrGet { base, .. } => each(base),
        HirExpr::MethodCall { base, args, .. } => {
            each(base);
            args.iter_mut().for_each(&mut each);
        }
        HirExpr::ReceiverDispatchedCall { call, .. } => each(call),
        HirExpr::ExprCall { callee, args } => {
            each(callee);
            args.iter_mut().for_each(&mut each);
        }
        HirExpr::KeywordCall { call, keywords, .. } => {
            each(call);
            for (_, value) in keywords {
                each(value);
            }
        }
        HirExpr::Comprehension(comp) => {
            comp_iter_mut(&mut comp.iter, &mut each);
            for body_expr in comp.body_exprs_mut() {
                each(body_expr);
            }
        }
        _ => {}
    }
}

/// Every statement of `body` and of its nested bodies, mutably.
fn all_stmts_mut(body: &mut [HirStmt], visit: &mut dyn FnMut(&mut HirStmt)) {
    for_each_stmt_mut(body, visit);
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
