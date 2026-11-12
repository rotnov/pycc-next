//! What a scope binds: the one scan of binding forms that the per-function
//! fallback (which names a function or the module binds) and the
//! specializations (which parameters a body rebinds, which builtins it
//! shadows) share. Every binding form counts: assignment, augmented and
//! annotated assignment, `for`, `with` and comprehension targets, walrus,
//! `del`, `import`, `def` and `class` names, `except ... as`, `case`
//! captures, and `global` declarations.

use ruff_python_ast::visitor::{self, Visitor};
use ruff_python_ast::{ExceptHandler, Expr, ExprContext, Pattern, Stmt};
use std::collections::BTreeSet;

/// The names `body` binds in the scope it runs in, without descending into
/// nested function, lambda, class or comprehension scopes (a nested `def`
/// or `class` binds its own name here, and a walrus in a comprehension its
/// target, PEP 572; so does a walrus in the decorators or parameter
/// defaults of a nested `def` or lambda, or in a nested class's decorators,
/// bases or keywords, all of which run in this scope). Names declared `global` are
/// included: from the module's point of view they are bound too.
pub fn bound_names(body: &[Stmt]) -> BTreeSet<String> {
    let mut collector = Binder::default();
    collector.visit_body(body);
    collector.names
}

/// The module-scope names `stmt` binds through something other than a
/// plain name target, which compiled code cannot redirect to the
/// namespace dictionary.
pub fn opaque_bound_names(stmt: &Stmt) -> BTreeSet<String> {
    let mut collector = Binder {
        opaque: true,
        ..Binder::default()
    };
    collector.visit_stmt(stmt);
    collector.names
}

/// The names the target `target` (of an assignment, a loop or a
/// comprehension) binds.
pub fn target_names(target: &Expr) -> BTreeSet<String> {
    let mut collector = Binder::default();
    collector.visit_expr(target);
    collector.names
}

/// The names `body`, a module's statements, binds in the module's scope:
/// its own [`bound_names`] and every name a function or class anywhere in
/// it declares `global`.
pub fn module_scope_names(body: &[Stmt]) -> BTreeSet<String> {
    struct Globals(BTreeSet<String>);
    impl<'a> Visitor<'a> for Globals {
        fn visit_stmt(&mut self, stmt: &'a Stmt) {
            if let Stmt::Global(global) = stmt {
                self.0.extend(global.names.iter().map(ToString::to_string));
            }
            visitor::walk_stmt(self, stmt);
        }
    }
    let mut globals = Globals(bound_names(body));
    for stmt in body {
        globals.visit_stmt(stmt);
    }
    globals.0
}

/// Every name `body` may bind in any scope it contains, nested functions,
/// lambdas, classes and comprehensions included: each scope's
/// [`bound_names`], every parameter of a nested function or lambda, every
/// name target (a comprehension's, a walrus inside one, a lambda's), and
/// every `nonlocal` declaration. A superset of what `body`'s own scope
/// binds, for the conservative questions "may this body rebind `x`" and
/// "may `len` here be another than the builtin".
pub fn bound_anywhere(body: &[Stmt]) -> BTreeSet<String> {
    struct Anywhere(BTreeSet<String>);
    impl<'a> Visitor<'a> for Anywhere {
        fn visit_stmt(&mut self, stmt: &'a Stmt) {
            match stmt {
                Stmt::FunctionDef(def) => {
                    self.0.extend(def.parameters.iter().map(|p| p.name().to_string()));
                    self.0.extend(bound_names(&def.body));
                }
                Stmt::ClassDef(class) => self.0.extend(bound_names(&class.body)),
                Stmt::Nonlocal(nonlocal) => self.0.extend(nonlocal.names.iter().map(ToString::to_string)),
                _ => {}
            }
            visitor::walk_stmt(self, stmt);
        }
        fn visit_expr(&mut self, expr: &'a Expr) {
            match expr {
                Expr::Name(name) if name.ctx != ExprContext::Load => {
                    self.0.insert(name.id.to_string());
                }
                Expr::Lambda(lambda) => {
                    if let Some(parameters) = &lambda.parameters {
                        self.0.extend(parameters.iter().map(|p| p.name().to_string()));
                    }
                }
                _ => {}
            }
            visitor::walk_expr(self, expr);
        }
    }
    let mut anywhere = Anywhere(bound_names(body));
    for stmt in body {
        anywhere.visit_stmt(stmt);
    }
    anywhere.0
}

/// Collects the targets of the walruses in a comprehension, which bind in
/// the scope containing it: through nested comprehensions, not into a
/// lambda (a scope whose walruses bind in it).
struct WalrusTargets<'n>(&'n mut BTreeSet<String>);

impl<'a> Visitor<'a> for WalrusTargets<'_> {
    fn visit_expr(&mut self, expr: &'a Expr) {
        match expr {
            Expr::Named(named) => {
                if let Expr::Name(name) = named.target.as_ref() {
                    self.0.insert(name.id.as_str().to_string());
                }
                self.visit_expr(&named.value);
            }
            // A lambda's defaults run in the comprehension; its body is
            // its own scope.
            Expr::Lambda(lambda) => {
                if let Some(parameters) = &lambda.parameters {
                    self.visit_parameters(parameters);
                }
            }
            _ => visitor::walk_expr(self, expr),
        }
    }

    fn visit_annotation(&mut self, _expr: &'a Expr) {}
}

/// Collects bindings of one scope.
#[derive(Default)]
struct Binder {
    names: BTreeSet<String>,
    /// Only bindings that are not a plain name target (`import`,
    /// `except ... as`, a match capture, a `def`/`class` name).
    opaque: bool,
}

impl<'a> Visitor<'a> for Binder {
    fn visit_stmt(&mut self, stmt: &'a Stmt) {
        match stmt {
            Stmt::FunctionDef(def) => {
                self.names.insert(def.name.as_str().to_string());
                // Decorators and defaults run in this scope; the body does
                // not, nor do the annotations (evaluated lazily, PEP 649).
                for decorator in &def.decorator_list {
                    self.visit_decorator(decorator);
                }
                self.visit_parameters(&def.parameters);
            }
            Stmt::ClassDef(class) => {
                self.names.insert(class.name.as_str().to_string());
                // Decorators, bases and keywords run in this scope.
                for decorator in &class.decorator_list {
                    self.visit_decorator(decorator);
                }
                if let Some(arguments) = &class.arguments {
                    self.visit_arguments(arguments);
                }
            }
            Stmt::Global(global) => {
                self.names
                    .extend(global.names.iter().map(|n| n.as_str().to_string()));
            }
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
            Expr::Name(name) if !matches!(name.ctx, ExprContext::Load) => {
                if !self.opaque {
                    self.names.insert(name.id.as_str().to_string());
                }
            }
            // A lambda's defaults run in this scope; its body is its own.
            Expr::Lambda(lambda) => {
                if let Some(parameters) = &lambda.parameters {
                    self.visit_parameters(parameters);
                }
            }
            // A comprehension is a scope of its own, but a walrus in it
            // binds in the scope that contains it (PEP 572), nested
            // comprehensions included.
            Expr::ListComp(_) | Expr::SetComp(_) | Expr::DictComp(_) | Expr::Generator(_) => {
                if !self.opaque {
                    let mut walrus = WalrusTargets(&mut self.names);
                    visitor::walk_expr(&mut walrus, expr);
                }
            }
            _ => visitor::walk_expr(self, expr),
        }
    }

    fn visit_except_handler(&mut self, handler: &'a ExceptHandler) {
        let ExceptHandler::ExceptHandler(h) = handler;
        if let Some(name) = &h.name {
            self.names.insert(name.as_str().to_string());
        }
        visitor::walk_except_handler(self, handler);
    }

    fn visit_pattern(&mut self, pattern: &'a Pattern) {
        match pattern {
            Pattern::MatchAs(p) => {
                if let Some(name) = &p.name {
                    self.names.insert(name.as_str().to_string());
                }
            }
            Pattern::MatchStar(p) => {
                if let Some(name) = &p.name {
                    self.names.insert(name.as_str().to_string());
                }
            }
            Pattern::MatchMapping(p) => {
                if let Some(rest) = &p.rest {
                    self.names.insert(rest.as_str().to_string());
                }
            }
            _ => {}
        }
        visitor::walk_pattern(self, pattern);
    }

    fn visit_annotation(&mut self, _expr: &'a Expr) {}
}

