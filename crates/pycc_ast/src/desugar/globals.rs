//! `global` declarations: a module global that a function rebinds lives in
//! a one-element list (a *cell*), `_pycc_global_<name>`.
//!
//! `cnt = 0` at module level becomes `_pycc_global_cnt = [0]`, and every
//! later use of `cnt` that resolves to the module global -- at module level,
//! in a function that declares `global cnt`, and in a function or
//! comprehension that only reads it -- becomes `_pycc_global_cnt[0]`. A
//! function's write then goes through the cell, so module code and every
//! other function see it, as they see a module attribute in CPython.
//!
//! A name is celled only when the shape is one this rewrite reproduces
//! exactly: its first module-level binding is a plain top-level `name =
//! value` (or `name: T = value`) statement with no module-level use before
//! it, and every other binding of it -- at module level or in a function
//! declaring it `global` -- is an assignment, augmented assignment or `for`
//! target. Anything else keeps its `global` statement, which the compiler
//! reports as before. A `global` statement naming a variable the function
//! never binds has no effect and is dropped.

use super::Fresh;
use super::build::{assign, int, load, store, subscript};
use ruff_python_ast::visitor::transformer::{self, Transformer};
use ruff_python_ast::visitor::{self, Visitor};
use ruff_python_ast::{
    AtomicNodeIndex, Comprehension, Expr, ExprContext, ExprList, Stmt, StmtFunctionDef, StmtPass,
};
use ruff_text_size::{Ranged, TextRange};
use std::cell::RefCell;
use std::collections::HashSet;

fn cell_name(name: &str) -> String {
    format!("_pycc_global_{name}")
}

/// The names a scope body binds (not those of nested scopes), and the
/// names it declares `global`.
#[derive(Default)]
pub(super) struct Bound {
    pub(super) names: HashSet<String>,
    pub(super) globals: HashSet<String>,
    /// Bound by a form the cell rewrite does not reproduce (`import`,
    /// `def`, `class`, `del`, `with ... as`, `except ... as`, a walrus, a
    /// `match` capture).
    unsupported: HashSet<String>,
}

fn store_names(target: &Expr, out: &mut HashSet<String>) {
    match target {
        Expr::Name(n) => {
            out.insert(n.id.to_string());
        }
        Expr::Tuple(t) => t.elts.iter().for_each(|e| store_names(e, out)),
        Expr::List(l) => l.elts.iter().for_each(|e| store_names(e, out)),
        Expr::Starred(s) => store_names(&s.value, out),
        _ => {}
    }
}

struct CollectBound<'b>(&'b mut Bound);

impl<'a> Visitor<'a> for CollectBound<'_> {
    fn visit_stmt(&mut self, stmt: &'a Stmt) {
        match stmt {
            Stmt::FunctionDef(def) => {
                self.0.names.insert(def.name.to_string());
                self.0.unsupported.insert(def.name.to_string());
                return;
            }
            Stmt::ClassDef(def) => {
                self.0.names.insert(def.name.to_string());
                self.0.unsupported.insert(def.name.to_string());
                return;
            }
            Stmt::Global(g) => {
                self.0.globals.extend(g.names.iter().map(|n| n.to_string()));
            }
            Stmt::Assign(a) => {
                for t in &a.targets {
                    store_names(t, &mut self.0.names);
                }
            }
            Stmt::AugAssign(a) => store_names(&a.target, &mut self.0.names),
            Stmt::AnnAssign(a) => {
                let mut names = HashSet::new();
                store_names(&a.target, &mut names);
                if a.value.is_none() {
                    self.0.unsupported.extend(names.iter().cloned());
                }
                self.0.names.extend(names);
            }
            Stmt::For(f) => store_names(&f.target, &mut self.0.names),
            _ => {}
        }
        visitor::walk_stmt(self, stmt);
    }

    fn visit_expr(&mut self, expr: &'a Expr) {
        match expr {
            Expr::Lambda(_)
            | Expr::ListComp(_)
            | Expr::SetComp(_)
            | Expr::DictComp(_)
            | Expr::Generator(_) => {
                // Only a walrus inside binds here.
                struct Walrus<'w>(&'w mut Bound);
                impl<'b> Visitor<'b> for Walrus<'_> {
                    fn visit_expr(&mut self, expr: &'b Expr) {
                        if let Expr::Named(named) = expr
                            && let Expr::Name(n) = named.target.as_ref()
                        {
                            self.0.names.insert(n.id.to_string());
                            self.0.unsupported.insert(n.id.to_string());
                        }
                        visitor::walk_expr(self, expr);
                    }
                }
                Walrus(self.0).visit_expr(expr);
                return;
            }
            Expr::Named(named) => {
                if let Expr::Name(n) = named.target.as_ref() {
                    self.0.names.insert(n.id.to_string());
                    self.0.unsupported.insert(n.id.to_string());
                }
            }
            // Every other binding form a `Store`/`Del` name appears in.
            Expr::Name(n) if n.ctx != ExprContext::Load => {
                self.0.names.insert(n.id.to_string());
            }
            _ => {}
        }
        visitor::walk_expr(self, expr);
    }
}

/// The names `body` binds by a form [`CollectBound`] does not model as an
/// assignment: imports, `del`, `with ... as`, `except ... as`, `match`
/// captures.
struct OtherBindings<'b>(&'b mut HashSet<String>);

impl<'a> Visitor<'a> for OtherBindings<'_> {
    fn visit_stmt(&mut self, stmt: &'a Stmt) {
        match stmt {
            Stmt::FunctionDef(_) | Stmt::ClassDef(_) => return,
            Stmt::Import(import) => {
                for alias in &import.names {
                    let bound = alias.asname.as_ref().unwrap_or(&alias.name);
                    self.0.insert(bound.split('.').next().unwrap_or("").to_string());
                }
            }
            Stmt::ImportFrom(import) => {
                for alias in &import.names {
                    self.0
                        .insert(alias.asname.as_ref().unwrap_or(&alias.name).to_string());
                }
            }
            Stmt::Delete(d) => {
                for t in &d.targets {
                    store_names(t, self.0);
                }
            }
            Stmt::With(w) => {
                for item in &w.items {
                    if let Some(v) = &item.optional_vars {
                        store_names(v, self.0);
                    }
                }
            }
            Stmt::Try(t) => {
                for handler in &t.handlers {
                    let ruff_python_ast::ExceptHandler::ExceptHandler(h) = handler;
                    if let Some(n) = &h.name {
                        self.0.insert(n.to_string());
                    }
                }
            }
            Stmt::Match(_) => {
                // Captures could bind anything: treat every stored name
                // as unsupported.
                let mut all = HashSet::new();
                struct Names<'n>(&'n mut HashSet<String>);
                impl<'b> Visitor<'b> for Names<'_> {
                    fn visit_pattern(&mut self, pattern: &'b ruff_python_ast::Pattern) {
                        match pattern {
                            ruff_python_ast::Pattern::MatchAs(p) => {
                                if let Some(n) = &p.name {
                                    self.0.insert(n.to_string());
                                }
                            }
                            ruff_python_ast::Pattern::MatchStar(p) => {
                                if let Some(n) = &p.name {
                                    self.0.insert(n.to_string());
                                }
                            }
                            ruff_python_ast::Pattern::MatchMapping(p) => {
                                if let Some(n) = &p.rest {
                                    self.0.insert(n.to_string());
                                }
                            }
                            _ => {}
                        }
                        visitor::walk_pattern(self, pattern);
                    }
                }
                Names(&mut all).visit_stmt(stmt);
                self.0.extend(all);
            }
            _ => {}
        }
        visitor::walk_stmt(self, stmt);
    }
}

pub(super) fn bound_of(body: &[Stmt]) -> Bound {
    let mut bound = Bound::default();
    let mut collect = CollectBound(&mut bound);
    for stmt in body {
        collect.visit_stmt(stmt);
    }
    let mut other = HashSet::new();
    let mut walk = OtherBindings(&mut other);
    for stmt in body {
        walk.visit_stmt(stmt);
    }
    bound.names.extend(other.iter().cloned());
    bound.unsupported.extend(other);
    bound
}

/// Every function (at any depth, methods included) in `body`.
fn functions<'a>(body: &'a [Stmt], out: &mut Vec<&'a StmtFunctionDef>) {
    struct Collect<'a, 'o>(&'o mut Vec<&'a StmtFunctionDef>);
    impl<'a> Visitor<'a> for Collect<'a, '_> {
        fn visit_stmt(&mut self, stmt: &'a Stmt) {
            if let Stmt::FunctionDef(def) = stmt {
                self.0.push(def);
            }
            visitor::walk_stmt(self, stmt);
        }
    }
    let mut c = Collect(out);
    for stmt in body {
        c.visit_stmt(stmt);
    }
}

/// Whether `name` is read (as a `Load`) anywhere in `stmt` outside the
/// bodies of functions and classes it defines.
fn reads_at_module_level(stmt: &Stmt, name: &str) -> bool {
    struct Find<'n> {
        name: &'n str,
        found: bool,
    }
    impl<'a> Visitor<'a> for Find<'_> {
        fn visit_stmt(&mut self, stmt: &'a Stmt) {
            match stmt {
                Stmt::FunctionDef(def) => {
                    for d in &def.decorator_list {
                        self.visit_expr(&d.expression);
                    }
                    for p in def.parameters.iter() {
                        if let Some(default) = p.default() {
                            self.visit_expr(default);
                        }
                    }
                }
                Stmt::ClassDef(_) => {}
                _ => visitor::walk_stmt(self, stmt),
            }
        }
        fn visit_expr(&mut self, expr: &'a Expr) {
            if let Expr::Name(n) = expr
                && n.id.as_str() == self.name
                && n.ctx == ExprContext::Load
            {
                self.found = true;
            }
            visitor::walk_expr(self, expr);
        }
    }
    let mut f = Find { name, found: false };
    f.visit_stmt(stmt);
    f.found
}

/// The index of the top-level statement creating the cell of `name`, when
/// `name` can be celled at module level.
fn creation_index(body: &[Stmt], name: &str) -> Option<usize> {
    let module = bound_of(body);
    if module.unsupported.contains(name) {
        return None;
    }
    for (i, stmt) in body.iter().enumerate() {
        let bound = bound_of(std::slice::from_ref(stmt));
        if !bound.names.contains(name) {
            if reads_at_module_level(stmt, name) {
                return None;
            }
            continue;
        }
        let plain = match stmt {
            Stmt::Assign(a) => {
                a.targets.len() == 1
                    && matches!(&a.targets[0], Expr::Name(n) if n.id.as_str() == name)
                    && !reads_at_module_level(stmt, name)
            }
            Stmt::AnnAssign(a) => {
                matches!(a.target.as_ref(), Expr::Name(n) if n.id.as_str() == name)
                    && a.value.is_some()
                    && a.simple
                    && !reads_at_module_level(stmt, name)
            }
            _ => false,
        };
        return plain.then_some(i);
    }
    None
}

/// Rewrites `global` declarations in `body` (a module) into cells (see the
/// module documentation).
pub(crate) fn cell_globals(body: &mut [Stmt], fresh: &Fresh) {
    let mut defs = Vec::new();
    functions(body, &mut defs);
    // Names some function declares `global` and binds.
    let mut wanted: HashSet<String> = HashSet::new();
    let mut refused: HashSet<String> = HashSet::new();
    for def in &defs {
        let bound = bound_of(&def.body);
        for g in &bound.globals {
            if !bound.names.contains(g) {
                continue;
            }
            if bound.unsupported.contains(g) || def.parameters.iter().any(|p| p.name().as_str() == g) {
                refused.insert(g.clone());
            }
            wanted.insert(g.clone());
        }
    }
    let mut cells: HashSet<String> = HashSet::new();
    let mut creations: Vec<(usize, String)> = Vec::new();
    for name in wanted {
        if refused.contains(&name) {
            continue;
        }
        if let Some(i) = creation_index(body, &name) {
            creations.push((i, name.clone()));
            cells.insert(name);
        }
    }
    // Drop every `global` statement that has nothing left to do: one
    // naming a cell, or a variable the function never binds.
    let rewriter = Rewriter {
        cells: &cells,
        scopes: RefCell::new(vec![Scope {
            active: cells.clone(),
            class: false,
        }]),
        fresh,
    };
    for (i, stmt) in body.iter_mut().enumerate() {
        if let Some((_, name)) = creations.iter().find(|(at, _)| *at == i) {
            create_cell(stmt, name);
            // The value itself is still rewritten.
            match stmt {
                Stmt::Assign(a) => rewriter.visit_expr(&mut a.value),
                Stmt::AnnAssign(a) => {
                    if let Some(v) = a.value.as_mut() {
                        rewriter.visit_expr(v);
                    }
                }
                _ => {}
            }
            continue;
        }
        rewriter.visit_stmt(stmt);
    }
}

/// `name = value` -> `_pycc_global_name = [value]`; `name: T = value` ->
/// `_pycc_global_name: list[T] = [value]`.
fn create_cell(stmt: &mut Stmt, name: &str) {
    let range = stmt.range();
    let wrap = |value: Expr| {
        Expr::List(ExprList {
            node_index: AtomicNodeIndex::default(),
            range,
            elts: vec![value],
            ctx: ExprContext::Load,
        })
    };
    match stmt {
        Stmt::Assign(a) => {
            a.targets = vec![store(&cell_name(name), range)];
            let value = std::mem::replace(a.value.as_mut(), load("_", range));
            *a.value = wrap(value);
        }
        Stmt::AnnAssign(a) => {
            *a.target = store(&cell_name(name), range);
            let annotation = std::mem::replace(a.annotation.as_mut(), load("_", range));
            *a.annotation = subscript(load("list", range), annotation, ExprContext::Load, range);
            if let Some(v) = a.value.as_mut() {
                let value = std::mem::replace(v.as_mut(), load("_", range));
                **v = wrap(value);
            }
        }
        _ => unreachable!("creation_index admits only assignments"),
    }
}

/// Drops, from `global` statements in a function body (not in its nested
/// scopes), the names the function never binds: such a declaration has no
/// effect.
fn drop_unbound_globals(body: &mut [Stmt], bound: &HashSet<String>) {
    struct Drop<'b>(&'b HashSet<String>);
    impl Transformer for Drop<'_> {
        fn visit_stmt(&self, stmt: &mut Stmt) {
            match stmt {
                Stmt::FunctionDef(_) | Stmt::ClassDef(_) => {}
                Stmt::Global(g) => {
                    // `self` stays: a method whose receiver is spelled
                    // otherwise binds the canonical `self` itself
                    // (`pycc_hir::class::receiver`), which refuses it.
                    g.names
                        .retain(|n| self.0.contains(n.as_str()) || n.as_str() == "self");
                    if g.names.is_empty() {
                        *stmt = Stmt::Pass(StmtPass {
                            node_index: AtomicNodeIndex::default(),
                            range: g.range,
                        });
                    }
                }
                _ => transformer::walk_stmt(self, stmt),
            }
        }
    }
    let d = Drop(bound);
    for s in body.iter_mut() {
        d.visit_stmt(s);
    }
}

fn visit_defaults(rewriter: &Rewriter<'_>, params: &mut ruff_python_ast::Parameters) {
    for p in params
        .posonlyargs
        .iter_mut()
        .chain(params.args.iter_mut())
        .chain(params.kwonlyargs.iter_mut())
    {
        if let Some(default) = p.default.as_mut() {
            rewriter.visit_expr(default);
        }
    }
}

struct Scope {
    /// The celled names a bare name resolves to in this scope.
    active: HashSet<String>,
    class: bool,
}

struct Rewriter<'a> {
    cells: &'a HashSet<String>,
    scopes: RefCell<Vec<Scope>>,
    fresh: &'a super::Fresh,
}

impl Rewriter<'_> {
    fn active(&self, name: &str) -> bool {
        self.scopes
            .borrow()
            .last()
            .is_some_and(|s| s.active.contains(name))
    }

    /// The active set of the nearest enclosing function or module scope.
    fn enclosing_active(&self) -> HashSet<String> {
        self.scopes
            .borrow()
            .iter()
            .rev()
            .find(|s| !s.class)
            .map(|s| s.active.clone())
            .unwrap_or_default()
    }

    fn with_scope(&self, scope: Scope, f: impl FnOnce()) {
        self.scopes.borrow_mut().push(scope);
        f();
        self.scopes.borrow_mut().pop();
    }

    fn cell_item(&self, name: &str, ctx: ExprContext, range: TextRange) -> Expr {
        subscript(load(&cell_name(name), range), int(0, range), ctx, range)
    }

    fn target_has_active(&self, target: &Expr) -> bool {
        let mut names = HashSet::new();
        store_names(target, &mut names);
        names.iter().any(|n| self.active(n))
    }

    fn comprehension(&self, generators: &mut [Comprehension], rest: impl FnOnce(&Self)) {
        let mut inner = self
            .scopes
            .borrow()
            .last()
            .map(|s| s.active.clone())
            .unwrap_or_default();
        for (i, generator) in generators.iter_mut().enumerate() {
            if i == 0 {
                self.visit_expr(&mut generator.iter);
            }
            let mut names = HashSet::new();
            store_names(&generator.target, &mut names);
            for n in names {
                inner.remove(&n);
            }
        }
        self.with_scope(
            Scope {
                active: inner,
                class: false,
            },
            || {
                for (i, generator) in generators.iter_mut().enumerate() {
                    if i > 0 {
                        self.visit_expr(&mut generator.iter);
                    }
                    for cond in generator.ifs.iter_mut() {
                        self.visit_expr(cond);
                    }
                }
                rest(self);
            },
        );
    }
}

impl Transformer for Rewriter<'_> {
    fn visit_stmt(&self, stmt: &mut Stmt) {
        match stmt {
            Stmt::FunctionDef(def) => {
                for d in def.decorator_list.iter_mut() {
                    self.visit_expr(&mut d.expression);
                }
                visit_defaults(self, &mut def.parameters);
                let bound = bound_of(&def.body);
                drop_unbound_globals(&mut def.body, &bound.names);
                let params: HashSet<String> =
                    def.parameters.iter().map(|p| p.name().to_string()).collect();
                let mut active = self.enclosing_active();
                active.retain(|n| {
                    bound.globals.contains(n) || (!bound.names.contains(n) && !params.contains(n))
                });
                for g in &bound.globals {
                    if self.cells.contains(g) {
                        active.insert(g.clone());
                    }
                }
                self.with_scope(
                    Scope {
                        active,
                        class: false,
                    },
                    || {
                        for s in def.body.iter_mut() {
                            self.visit_stmt(s);
                        }
                    },
                );
            }
            Stmt::ClassDef(class) => {
                for d in class.decorator_list.iter_mut() {
                    self.visit_expr(&mut d.expression);
                }
                if let Some(args) = class.arguments.as_mut() {
                    for a in args.args.iter_mut() {
                        self.visit_expr(a);
                    }
                }
                let bound = bound_of(&class.body);
                let mut active = self
                    .scopes
                    .borrow()
                    .last()
                    .map(|s| s.active.clone())
                    .unwrap_or_default();
                active.retain(|n| !bound.names.contains(n));
                self.with_scope(
                    Scope {
                        active,
                        class: true,
                    },
                    || {
                        for s in class.body.iter_mut() {
                            self.visit_stmt(s);
                        }
                    },
                );
            }
            Stmt::Global(g) => {
                g.names.retain(|n| !self.cells.contains(n.as_str()));
                if g.names.is_empty() {
                    *stmt = Stmt::Pass(StmtPass {
                        node_index: AtomicNodeIndex::default(),
                        range: g.range,
                    });
                }
            }
            Stmt::For(f) if self.target_has_active(&f.target) => {
                let range = f.target.range();
                let temp = self.fresh.name();
                let mut target = std::mem::replace(f.target.as_mut(), store(&temp, range));
                self.visit_expr(&mut target);
                self.visit_expr(&mut f.iter);
                for s in f.body.iter_mut() {
                    self.visit_stmt(s);
                }
                for s in f.orelse.iter_mut() {
                    self.visit_stmt(s);
                }
                f.body.insert(0, assign(target, load(&temp, range), range));
            }
            _ => transformer::walk_stmt(self, stmt),
        }
    }

    fn visit_expr(&self, expr: &mut Expr) {
        match expr {
            Expr::Name(n) if self.active(n.id.as_str()) => {
                *expr = self.cell_item(n.id.as_str(), n.ctx, n.range);
            }
            Expr::Lambda(lambda) => {
                let mut active = self
                    .scopes
                    .borrow()
                    .last()
                    .map(|s| s.active.clone())
                    .unwrap_or_default();
                if let Some(params) = lambda.parameters.as_deref_mut() {
                    visit_defaults(self, params);
                    for p in params.iter() {
                        active.remove(p.name().as_str());
                    }
                }
                self.with_scope(
                    Scope {
                        active,
                        class: false,
                    },
                    || self.visit_expr(&mut lambda.body),
                );
            }
            Expr::ListComp(c) => {
                let elt = &mut c.elt;
                self.comprehension(&mut c.generators, |s| s.visit_expr(elt));
            }
            Expr::SetComp(c) => {
                let elt = &mut c.elt;
                self.comprehension(&mut c.generators, |s| s.visit_expr(elt));
            }
            Expr::Generator(c) => {
                let elt = &mut c.elt;
                self.comprehension(&mut c.generators, |s| s.visit_expr(elt));
            }
            Expr::DictComp(c) => {
                let key = &mut c.key;
                let value = &mut c.value;
                self.comprehension(&mut c.generators, |s| {
                    if let Some(key) = key.as_mut() {
                        s.visit_expr(key);
                    }
                    s.visit_expr(value);
                });
            }
            _ => transformer::walk_expr(self, expr),
        }
    }
}
