//! Scope analysis for `nested`: the free names of a function, lambda or
//! expression, and how often a function binds a name.

use super::super::*;
use pycc_ast::visitor::{self, Visitor};
use pycc_ast::{Expr, ExprContext, StmtFunctionDef};

/// Whether `def`'s body reads `__class__` or calls `super()` with no
/// arguments, at any depth.
pub(super) fn uses_class_cell(def: &StmtFunctionDef) -> bool {
    struct Uses(bool);
    impl<'a> Visitor<'a> for Uses {
        fn visit_expr(&mut self, expr: &'a Expr) {
            match expr {
                Expr::Name(name) if name.id.as_str() == "__class__" => self.0 = true,
                Expr::Call(call)
                    if matches!(&*call.func, Expr::Name(n) if n.id.as_str() == "super")
                        && call.arguments.args.is_empty() =>
                {
                    self.0 = true
                }
                _ => visitor::walk_expr(self, expr),
            }
        }
    }
    let mut uses = Uses(false);
    for stmt in &def.body {
        uses.visit_stmt(stmt);
    }
    uses.0
}

pub(super) fn parameter_names(parameters: &pycc_ast::Parameters) -> BTreeSet<String> {
    parameters
        .iter()
        .map(|p| p.name().as_str().to_string())
        .collect()
}

pub(super) fn is_docstring(stmt: &Stmt) -> bool {
    matches!(stmt, Stmt::Expr(e) if e.value.is_string_literal_expr())
}

/// The free names of a function scope with `parameters` and `body`: those
/// it (or a scope nested in it) reads, writes or declares `nonlocal` that
/// it does not bind itself, but the ones it declares `global`.
pub(super) fn function_free(parameters: &pycc_ast::Parameters, body: &[Stmt]) -> BTreeSet<String> {
    let mut refs = Refs::default();
    for stmt in body {
        refs.visit_stmt(stmt);
    }
    let globals = names::declared_globals(body);
    let mut bound = names::bound_names(body);
    bound.extend(parameter_names(parameters));
    let nonlocals = declared_nonlocals(body);
    refs.names
        .into_iter()
        .filter(|name| nonlocals.contains(name) || !bound.contains(name))
        .filter(|name| !globals.contains(name))
        .collect()
}

/// [`function_free`] of a `lambda`.
pub(super) fn lambda_free(parameters: &pycc_ast::Parameters, body: &Expr) -> BTreeSet<String> {
    let params = parameter_names(parameters);
    expr_free(body)
        .into_iter()
        .filter(|name| !params.contains(name))
        .collect()
}

/// The names an expression evaluated in the enclosing scope reads (a
/// walrus target included), its own nested scopes' free names among them.
pub(super) fn expr_free(expr: &Expr) -> BTreeSet<String> {
    let mut refs = Refs::default();
    refs.visit_expr(expr);
    refs.names
}

/// Whether a function or lambda nested in `body`, at any depth, declares
/// `name` `nonlocal`: it writes the variable, so its value changes.
pub(super) fn nonlocal_anywhere(body: &[Stmt], name: &str) -> bool {
    struct Find<'n>(&'n str, bool);
    impl<'a> Visitor<'a> for Find<'_> {
        fn visit_stmt(&mut self, stmt: &'a Stmt) {
            if let Stmt::Nonlocal(n) = stmt
                && n.names.iter().any(|n| n.as_str() == self.0)
            {
                self.1 = true;
            }
            visitor::walk_stmt(self, stmt);
        }
    }
    let mut find = Find(name, false);
    for stmt in body {
        find.visit_stmt(stmt);
    }
    find.1
}

/// The names declared `nonlocal` directly in `body`.
pub(super) fn declared_nonlocals(body: &[Stmt]) -> BTreeSet<String> {
    struct Nonlocals(BTreeSet<String>);
    impl<'a> Visitor<'a> for Nonlocals {
        fn visit_stmt(&mut self, stmt: &'a Stmt) {
            match stmt {
                Stmt::Nonlocal(n) => self
                    .0
                    .extend(n.names.iter().map(|n| n.as_str().to_string())),
                Stmt::FunctionDef(_) | Stmt::ClassDef(_) => {}
                _ => visitor::walk_stmt(self, stmt),
            }
        }
        fn visit_expr(&mut self, _expr: &'a Expr) {}
    }
    let mut found = Nonlocals(BTreeSet::new());
    for stmt in body {
        found.visit_stmt(stmt);
    }
    found.0
}

/// Every name referenced in one scope, with the free names of the scopes
/// nested in it (which reach out through it).
#[derive(Default)]
pub(super) struct Refs {
    pub(super) names: BTreeSet<String>,
}

impl<'a> Visitor<'a> for Refs {
    fn visit_stmt(&mut self, stmt: &'a Stmt) {
        match stmt {
            Stmt::FunctionDef(def) => {
                for decorator in &def.decorator_list {
                    self.visit_decorator(decorator);
                }
                for param in def.parameters.iter() {
                    if let Some(default) = param.default() {
                        self.visit_expr(default);
                    }
                    if let Some(annotation) = param.annotation() {
                        self.visit_expr(annotation);
                    }
                }
                if let Some(returns) = &def.returns {
                    self.visit_expr(returns);
                }
                self.names.insert(def.name.as_str().to_string());
                self.names.extend(function_free(&def.parameters, &def.body));
            }
            Stmt::ClassDef(class) => {
                for decorator in &class.decorator_list {
                    self.visit_decorator(decorator);
                }
                if let Some(arguments) = &class.arguments {
                    self.visit_arguments(arguments);
                }
                self.names.insert(class.name.as_str().to_string());
                // A class body's own bindings are not visible to its
                // methods, so its free names are those it reads unbound,
                // and its methods' free names.
                let mut inner = Refs::default();
                for stmt in &class.body {
                    inner.visit_stmt(stmt);
                }
                let bound = names::bound_names(&class.body);
                let mut methods = BTreeSet::new();
                for stmt in &class.body {
                    if let Stmt::FunctionDef(def) = stmt {
                        methods.extend(function_free(&def.parameters, &def.body));
                    }
                }
                self.names.extend(
                    inner
                        .names
                        .into_iter()
                        .filter(|name| !bound.contains(name) || methods.contains(name)),
                );
            }
            Stmt::Global(g) => self
                .names
                .extend(g.names.iter().map(|n| n.as_str().to_string())),
            Stmt::Nonlocal(n) => self
                .names
                .extend(n.names.iter().map(|n| n.as_str().to_string())),
            Stmt::Import(import) => {
                for alias in &import.names {
                    let bound = match &alias.asname {
                        Some(asname) => asname.as_str(),
                        None => alias.name.as_str().split('.').next().unwrap_or(""),
                    };
                    self.names.insert(bound.to_string());
                }
            }
            Stmt::ImportFrom(import) => {
                for alias in &import.names {
                    let bound = alias.asname.as_ref().unwrap_or(&alias.name);
                    self.names.insert(bound.as_str().to_string());
                }
            }
            _ => visitor::walk_stmt(self, stmt),
        }
    }

    fn visit_expr(&mut self, expr: &'a Expr) {
        match expr {
            Expr::Name(name) => {
                self.names.insert(name.id.as_str().to_string());
            }
            Expr::Lambda(lambda) => match &lambda.parameters {
                Some(parameters) => {
                    for param in parameters.iter() {
                        if let Some(default) = param.default() {
                            self.visit_expr(default);
                        }
                    }
                    self.names.extend(lambda_free(parameters, &lambda.body));
                }
                None => self.names.extend(expr_free(&lambda.body)),
            },
            Expr::ListComp(comp) => self.comprehension(&comp.generators, &[&comp.elt]),
            Expr::SetComp(comp) => self.comprehension(&comp.generators, &[&comp.elt]),
            Expr::Generator(comp) => self.comprehension(&comp.generators, &[&comp.elt]),
            Expr::DictComp(comp) => {
                let mut results: Vec<&Expr> = comp.key.iter().map(|k| &**k).collect();
                results.push(&comp.value);
                self.comprehension(&comp.generators, &results)
            }
            _ => visitor::walk_expr(self, expr),
        }
    }

    fn visit_except_handler(&mut self, handler: &'a pycc_ast::ExceptHandler) {
        let pycc_ast::ExceptHandler::ExceptHandler(h) = handler;
        if let Some(name) = &h.name {
            self.names.insert(name.as_str().to_string());
        }
        visitor::walk_except_handler(self, handler);
    }

    fn visit_pattern(&mut self, pattern: &'a pycc_ast::Pattern) {
        match pattern {
            pycc_ast::Pattern::MatchAs(p) => {
                if let Some(name) = &p.name {
                    self.names.insert(name.as_str().to_string());
                }
            }
            pycc_ast::Pattern::MatchStar(p) => {
                if let Some(name) = &p.name {
                    self.names.insert(name.as_str().to_string());
                }
            }
            pycc_ast::Pattern::MatchMapping(p) => {
                if let Some(rest) = &p.rest {
                    self.names.insert(rest.as_str().to_string());
                }
            }
            _ => {}
        }
        visitor::walk_pattern(self, pattern);
    }
}

impl Refs {
    /// A comprehension: its first iterable is read in this scope, the rest
    /// in its own scope, where its targets are bound.
    fn comprehension(&mut self, generators: &[pycc_ast::Comprehension], results: &[&Expr]) {
        let Some(first) = generators.first() else {
            return;
        };
        self.visit_expr(&first.iter);
        let mut inner = Refs::default();
        let mut targets = Refs::default();
        for (index, generator) in generators.iter().enumerate() {
            if index > 0 {
                inner.visit_expr(&generator.iter);
            }
            targets.visit_expr(&generator.target);
            for condition in &generator.ifs {
                inner.visit_expr(condition);
            }
        }
        for result in results {
            inner.visit_expr(result);
        }
        self.names.extend(
            inner
                .names
                .into_iter()
                .filter(|name| !targets.names.contains(name)),
        );
    }
}

/// How many binding sites `name` has in `body`'s own scope: every store
/// or `del` of it, a nested `def`/`class`/import binding it, an `except
/// ... as` or match capture; a `global`/`nonlocal` declaration counts
/// twice, so it is never mistaken for one assignment. A comprehension's
/// own targets count too, which only keeps more names in cells.
pub(super) fn binding_sites(body: &[Stmt], name: &str) -> usize {
    struct Sites<'n> {
        name: &'n str,
        count: usize,
    }
    impl<'a> Visitor<'a> for Sites<'_> {
        fn visit_stmt(&mut self, stmt: &'a Stmt) {
            match stmt {
                Stmt::FunctionDef(def) => {
                    if def.name.as_str() == self.name {
                        self.count += 1;
                    }
                }
                Stmt::ClassDef(class) => {
                    if class.name.as_str() == self.name {
                        self.count += 1;
                    }
                }
                Stmt::Global(_) | Stmt::Nonlocal(_) | Stmt::Import(_) | Stmt::ImportFrom(_) => {
                    if names::bound_names(std::slice::from_ref(stmt)).contains(self.name) {
                        self.count += 2;
                    }
                }
                _ => visitor::walk_stmt(self, stmt),
            }
        }

        fn visit_expr(&mut self, expr: &'a Expr) {
            match expr {
                Expr::Name(n)
                    if n.id.as_str() == self.name && !matches!(n.ctx, ExprContext::Load) =>
                {
                    self.count += 1;
                }
                Expr::Lambda(_) => {}
                _ => visitor::walk_expr(self, expr),
            }
        }

        fn visit_except_handler(&mut self, handler: &'a pycc_ast::ExceptHandler) {
            let pycc_ast::ExceptHandler::ExceptHandler(h) = handler;
            if h.name.as_ref().is_some_and(|n| n.as_str() == self.name) {
                self.count += 1;
            }
            visitor::walk_except_handler(self, handler);
        }

        fn visit_pattern(&mut self, pattern: &'a pycc_ast::Pattern) {
            let mut binder = Refs::default();
            binder.visit_pattern(pattern);
            if binder.names.contains(self.name) {
                self.count += 1;
            }
        }
    }
    let mut sites = Sites { name, count: 0 };
    for stmt in body {
        sites.visit_stmt(stmt);
    }
    sites.count
}

/// Whether the target `expr` binds `name`.
pub(super) fn target_binds(expr: &Expr, name: &str) -> bool {
    match expr {
        Expr::Name(n) => n.id.as_str() == name,
        Expr::Tuple(t) => t.elts.iter().any(|e| target_binds(e, name)),
        Expr::List(l) => l.elts.iter().any(|e| target_binds(e, name)),
        Expr::Starred(s) => target_binds(&s.value, name),
        _ => false,
    }
}
