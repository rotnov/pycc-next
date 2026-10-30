//! v0.10 accept 5: whether code can reach an exception layout class
//! (`exc_layout`) other than as the planner sees it.
//!
//! The planner reads class statements' bases by name. A layout class
//! reached otherwise -- bound to another name (`Alias = A`), stored,
//! passed, returned, put in a container, looked up by its name as a string
//! -- could become a base the planner never sees (`class C(Alias,
//! OSError)`, `type(...)`), so it gets no layout. Its name may appear only
//! as a base in a class statement's base list, an `except` type, a
//! `raise`d class, a call's callee, `isinstance`/`issubclass`'s second
//! argument, or an annotation. And a class statement whose base the
//! planner cannot resolve to a known class (a name bound otherwise than by
//! a class statement or an import, an attribute, a call, a subscript) could
//! derive from any layout class of the module: there is none then. A base
//! name needs a positive binding (a class statement, an explicit import, or
//! a builtin class the module never rebinds), and a wildcard import of the
//! module -- or of one importing from it, which may re-export its classes
//! -- binds names out of this scan's sight: no layout then either.

use pycc_ast::visitor::{self, Visitor};
use pycc_ast::{Expr, ExprContext, Stmt};
use std::collections::{BTreeMap, BTreeSet};

/// What a module does with the layout candidates it can reach.
#[derive(Debug, Default, Clone, PartialEq)]
pub(super) struct Reach {
    /// The candidates (by their names in the module defining them) it
    /// reaches other than as a base, an `except` type, a raised class, a
    /// callee, an `isinstance`/`issubclass` class or an annotation.
    pub(super) escaped: BTreeSet<String>,
    /// Whether one of its class statements has a base the planner cannot
    /// resolve to a known class.
    pub(super) unknown_base: bool,
}

impl Reach {
    /// Adds what `other` reaches.
    pub(super) fn extend(&mut self, other: Reach) {
        self.escaped.extend(other.escaped);
        self.unknown_base |= other.unknown_base;
    }
}

/// What `body` (a module's) does with the candidates: `local` maps each
/// name binding one in the module to the candidate's name (each top-level
/// class of the defining module itself, `defining`; the names another
/// module imports them as), `modules` holds the names binding the defining
/// module (`from . import common`, whose `common.A` reaches `A`), and
/// `candidates` every candidate's name.
pub(super) fn reach(
    body: &[Stmt],
    defining: bool,
    local: &BTreeMap<String, String>,
    modules: &BTreeSet<String>,
    candidates: &BTreeSet<String>,
) -> Reach {
    let mut bound = BindingScan::default();
    for stmt in body {
        bound.visit_stmt(stmt);
    }
    let mut imported: BTreeSet<String> = BTreeSet::new();
    let mut classes: BTreeSet<String> = BTreeSet::new();
    for stmt in body {
        match stmt {
            Stmt::Import(import) => {
                for alias in &import.names {
                    let name = alias.asname.as_ref().unwrap_or(&alias.name).as_str();
                    imported.insert(name.split('.').next().unwrap_or(name).to_string());
                }
            }
            Stmt::ImportFrom(import) => {
                for alias in &import.names {
                    imported.insert(alias.asname.as_ref().unwrap_or(&alias.name).to_string());
                }
            }
            Stmt::ClassDef(class) => {
                classes.insert(class.name.to_string());
            }
            _ => {}
        }
    }
    let mut scan = ReachScan {
        defining,
        local,
        modules,
        candidates,
        // A name bound otherwise than by a top-level class statement or an
        // import may be anything; one never bound in the module is a
        // builtin class only when `builtins` has it, and anything (a
        // wildcard import's) otherwise.
        unknown_names: bound
            .names
            .into_iter()
            .filter(|name| !local.contains_key(name))
            .collect(),
        known: classes.union(&imported).cloned().collect(),
        out: Reach::default(),
    };
    for stmt in body {
        scan.visit_stmt(stmt);
    }
    // A wildcard import may rebind any name of the module defining the
    // candidates, a class's included, out of the planner's sight.
    if defining && has_wildcard(body) {
        scan.out.unknown_base = true;
    }
    scan.out
}

/// Whether `body` holds a wildcard import (`from m import *`), at any depth
/// of its top-level compound statements.
pub(super) fn has_wildcard(body: &[Stmt]) -> bool {
    wildcard_sources(body).next().is_some()
}

/// The `from` imports of `body` that bind `*` (top level and inside
/// `if`/`try` blocks): each one's level and module.
pub(super) fn wildcard_sources(
    body: &[Stmt],
) -> impl Iterator<Item = &pycc_ast::StmtImportFrom> + '_ {
    let mut out: Vec<&pycc_ast::StmtImportFrom> = Vec::new();
    fn walk<'a>(body: &'a [Stmt], out: &mut Vec<&'a pycc_ast::StmtImportFrom>) {
        for stmt in body {
            match stmt {
                Stmt::ImportFrom(import)
                    if import.names.iter().any(|alias| alias.name.as_str() == "*") =>
                {
                    out.push(import);
                }
                Stmt::If(s) => {
                    walk(&s.body, out);
                    for clause in &s.elif_else_clauses {
                        walk(&clause.body, out);
                    }
                }
                Stmt::Try(s) => {
                    walk(&s.body, out);
                    for handler in &s.handlers {
                        let pycc_ast::ExceptHandler::ExceptHandler(h) = handler;
                        walk(&h.body, out);
                    }
                    walk(&s.orelse, out);
                    walk(&s.finalbody, out);
                }
                _ => {}
            }
        }
    }
    walk(body, &mut out);
    out.into_iter()
}

/// Every name bound in a module otherwise than by a top-level class
/// statement or an import: an assignment target, a `for`/`with`/`except`
/// /`match` name, a function's name or parameter, a nested class, a
/// `global` or `nonlocal` one.
#[derive(Default)]
struct BindingScan {
    names: BTreeSet<String>,
    depth: usize,
}

impl<'a> Visitor<'a> for BindingScan {
    fn visit_stmt(&mut self, stmt: &'a Stmt) {
        match stmt {
            Stmt::FunctionDef(def) => {
                self.names.insert(def.name.to_string());
                for param in def.parameters.iter() {
                    self.names.insert(param.name().to_string());
                }
            }
            Stmt::ClassDef(class) if self.depth > 0 => {
                self.names.insert(class.name.to_string());
            }
            Stmt::Global(global) => self
                .names
                .extend(global.names.iter().map(|n| n.to_string())),
            Stmt::Nonlocal(nonlocal) => {
                self.names
                    .extend(nonlocal.names.iter().map(|n| n.to_string()));
            }
            Stmt::Import(_) | Stmt::ImportFrom(_) if self.depth > 0 => {
                self.names
                    .extend(super::names::bound_names(std::slice::from_ref(stmt)));
            }
            _ => {}
        }
        let nested = matches!(stmt, Stmt::FunctionDef(_) | Stmt::ClassDef(_));
        self.depth += usize::from(nested);
        visitor::walk_stmt(self, stmt);
        self.depth -= usize::from(nested);
    }

    fn visit_expr(&mut self, expr: &'a Expr) {
        match expr {
            Expr::Name(name) if !matches!(name.ctx, ExprContext::Load) => {
                self.names.insert(name.id.to_string());
            }
            Expr::Lambda(lambda) => {
                for param in lambda.parameters.iter().flat_map(|p| p.iter()) {
                    self.names.insert(param.name().to_string());
                }
            }
            _ => {}
        }
        visitor::walk_expr(self, expr);
    }

    fn visit_except_handler(&mut self, handler: &'a pycc_ast::ExceptHandler) {
        let pycc_ast::ExceptHandler::ExceptHandler(h) = handler;
        if let Some(name) = &h.name {
            self.names.insert(name.to_string());
        }
        visitor::walk_except_handler(self, handler);
    }

    fn visit_pattern(&mut self, pattern: &'a pycc_ast::Pattern) {
        let name = match pattern {
            pycc_ast::Pattern::MatchAs(p) => p.name.as_ref(),
            pycc_ast::Pattern::MatchStar(p) => p.name.as_ref(),
            pycc_ast::Pattern::MatchMapping(p) => p.rest.as_ref(),
            _ => None,
        };
        if let Some(name) = name {
            self.names.insert(name.to_string());
        }
        visitor::walk_pattern(self, pattern);
    }
}

/// The walk recording what reaches a candidate.
struct ReachScan<'r> {
    defining: bool,
    local: &'r BTreeMap<String, String>,
    modules: &'r BTreeSet<String>,
    candidates: &'r BTreeSet<String>,
    unknown_names: BTreeSet<String>,
    known: BTreeSet<String>,
    out: Reach,
}

impl ReachScan<'_> {
    /// The candidate `expr` names directly (`A`, `common.A`), if any.
    fn candidate(&self, expr: &Expr) -> Option<String> {
        match expr {
            Expr::Name(name) if matches!(name.ctx, ExprContext::Load) => {
                self.local.get(name.id.as_str()).cloned()
            }
            Expr::Attribute(attr) => match &*attr.value {
                Expr::Name(module)
                    if self.modules.contains(module.id.as_str())
                        && self.candidates.contains(attr.attr.as_str()) =>
                {
                    Some(attr.attr.to_string())
                }
                _ => None,
            },
            _ => None,
        }
    }

    /// Walks `expr` where a class may stand (an `except` type, the classes
    /// of `isinstance`): a candidate or a tuple of them is no escape.
    fn class_position(&mut self, expr: &Expr) {
        match expr {
            Expr::Tuple(tuple) => {
                for elt in &tuple.elts {
                    self.class_position(elt);
                }
            }
            _ if self.candidate(expr).is_some() => {}
            _ => self.visit_expr(expr),
        }
    }

    /// Whether base `expr` of a class statement is a class the planner
    /// knows: a candidate, a top-level class, an imported name (an
    /// attribute of one, which the package scan resolves), or a builtin.
    fn known_base(&self, expr: &Expr) -> bool {
        match expr {
            // A positive binding: a candidate, a top-level class statement,
            // an explicit import, or a builtin class the module never
            // binds otherwise.
            Expr::Name(name) => {
                let name = name.id.as_str();
                self.local.contains_key(name)
                    || self.known.contains(name)
                    || (super::exc_names::BUILTIN_CLASSES.contains(&name)
                        && !self.unknown_names.contains(name))
            }
            _ => self.candidate(expr).is_some(),
        }
    }
}

impl<'a> Visitor<'a> for ReachScan<'_> {
    fn visit_stmt(&mut self, stmt: &'a Stmt) {
        match stmt {
            Stmt::ClassDef(class) => {
                for decorator in &class.decorator_list {
                    self.visit_decorator(decorator);
                }
                for base in class.bases() {
                    if !self.known_base(base) {
                        self.out.unknown_base = true;
                    }
                    if self.candidate(base).is_none() {
                        self.visit_expr(base);
                    }
                }
                for keyword in class.keywords() {
                    self.visit_expr(&keyword.value);
                }
                for stmt in &class.body {
                    self.visit_stmt(stmt);
                }
            }
            Stmt::Raise(raise) => {
                match raise.exc.as_deref() {
                    Some(Expr::Call(call)) if self.candidate(&call.func).is_some() => {
                        self.visit_arguments(&call.arguments);
                    }
                    Some(exc) => self.class_position(exc),
                    None => {}
                }
                if let Some(cause) = raise.cause.as_deref() {
                    self.visit_expr(cause);
                }
            }
            // `__all__`'s names are the module's exports, not lookups.
            Stmt::Assign(assign)
                if assign
                    .targets
                    .iter()
                    .all(|t| matches!(t, Expr::Name(n) if n.id.as_str() == "__all__")) => {}
            Stmt::AugAssign(assign) if matches!(&*assign.target, Expr::Name(n) if n.id.as_str() == "__all__") =>
                {}
            // Rebinding a candidate's name -- a `def` of it, or, in the
            // module defining it, an import -- leaves the class reachable
            // otherwise.
            Stmt::FunctionDef(def) => {
                if let Some(candidate) = self.local.get(def.name.as_str()) {
                    self.out.escaped.insert(candidate.clone());
                }
                visitor::walk_stmt(self, stmt);
            }
            Stmt::Import(_) | Stmt::ImportFrom(_) if self.defining => {
                for name in super::names::bound_names(std::slice::from_ref(stmt)) {
                    if let Some(candidate) = self.local.get(&name) {
                        self.out.escaped.insert(candidate.clone());
                    }
                }
            }
            _ => visitor::walk_stmt(self, stmt),
        }
    }

    fn visit_annotation(&mut self, _expr: &'a Expr) {}

    fn visit_except_handler(&mut self, handler: &'a pycc_ast::ExceptHandler) {
        let pycc_ast::ExceptHandler::ExceptHandler(h) = handler;
        if let Some(type_) = h.type_.as_deref() {
            self.class_position(type_);
        }
        for stmt in &h.body {
            self.visit_stmt(stmt);
        }
    }

    fn visit_expr(&mut self, expr: &'a Expr) {
        match expr {
            Expr::Call(call) if self.candidate(&call.func).is_some() => {
                self.visit_arguments(&call.arguments);
            }
            Expr::Call(call)
                if matches!(&*call.func, Expr::Name(f)
                    if matches!(f.id.as_str(), "isinstance" | "issubclass"))
                    && call.arguments.args.len() == 2
                    && call.arguments.keywords.is_empty() =>
            {
                self.visit_expr(&call.arguments.args[0]);
                self.class_position(&call.arguments.args[1]);
            }
            Expr::Name(name) => {
                if let Some(candidate) = self.local.get(name.id.as_str()) {
                    self.out.escaped.insert(candidate.clone());
                }
            }
            Expr::Attribute(attr) => {
                // `x.A` for any `x`: possibly the class, reached by name.
                if self.candidates.contains(attr.attr.as_str()) {
                    self.out.escaped.insert(attr.attr.to_string());
                }
                visitor::walk_expr(self, expr);
            }
            Expr::StringLiteral(text) => {
                // `globals()["A"]`, `getattr(module, "A")`.
                let text = text.value.to_str();
                if self.candidates.contains(text) {
                    self.out.escaped.insert(text.to_string());
                }
            }
            _ => visitor::walk_expr(self, expr),
        }
    }
}
