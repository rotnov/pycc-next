//! The variables of a compiled function a nested statement keeps in a
//! cell (`nested`): which binding forms can be rewritten, and the rewrite.

use super::super::*;
use super::scopes::*;
use pycc_ast::Expr;
use pycc_ast::visitor::{self, Visitor};

/// Checks that every binding of a celled name is one [`CellRewriter`]
/// rewrites.
pub(super) struct CellCheck<'c> {
    pub(super) celled: &'c BTreeSet<String>,
    pub(super) source: &'c str,
    pub(super) ok: bool,
}

impl<'a> Visitor<'a> for CellCheck<'_> {
    fn visit_stmt(&mut self, stmt: &'a Stmt) {
        match stmt {
            // A nested `def` binding a celled name is rewritten with it.
            Stmt::FunctionDef(_) | Stmt::ClassDef(_) => {
                if let Stmt::ClassDef(class) = stmt
                    && self.celled.contains(class.name.as_str())
                {
                    self.ok = false;
                }
            }
            // A loop target is rebound through a plain name first
            // (`CellRewriter`), which needs the body on lines of its own.
            Stmt::For(s) if binds_any(&s.target, self.celled) => {
                let header = pycc_ast::stmt_range(stmt).start;
                let body = s
                    .body
                    .first()
                    .map_or(header, |b| pycc_ast::stmt_range(b).start);
                if !plain_target(&s.target)
                    || !self.source[header as usize..body as usize].contains('\n')
                {
                    self.ok = false;
                }
                visitor::walk_stmt(self, stmt);
            }
            Stmt::With(s)
                if s.items.iter().any(|i| {
                    i.optional_vars
                        .as_deref()
                        .is_some_and(|v| binds_any(v, self.celled))
                }) =>
            {
                self.ok = false
            }
            Stmt::Import(_) | Stmt::ImportFrom(_)
                if names::bound_names(std::slice::from_ref(stmt))
                    .iter()
                    .any(|n| self.celled.contains(n)) =>
            {
                self.ok = false
            }
            Stmt::TypeAlias(alias) if binds_any(&alias.name, self.celled) => self.ok = false,
            Stmt::Nonlocal(_) | Stmt::Global(_) => {}
            _ => visitor::walk_stmt(self, stmt),
        }
    }

    fn visit_expr(&mut self, expr: &'a Expr) {
        match expr {
            Expr::Named(named) if binds_any(&named.target, self.celled) => self.ok = false,
            Expr::Lambda(_) => {}
            _ => visitor::walk_expr(self, expr),
        }
    }

    fn visit_except_handler(&mut self, handler: &'a pycc_ast::ExceptHandler) {
        let pycc_ast::ExceptHandler::ExceptHandler(h) = handler;
        if h.name
            .as_ref()
            .is_some_and(|n| self.celled.contains(n.as_str()))
        {
            self.ok = false;
        }
        visitor::walk_except_handler(self, handler);
    }

    fn visit_pattern(&mut self, pattern: &'a pycc_ast::Pattern) {
        let mut binder = Refs::default();
        binder.visit_pattern(pattern);
        if binder.names.iter().any(|n| self.celled.contains(n)) {
            self.ok = false;
        }
    }
}

/// Whether a loop target is a name or a tuple or list of names.
pub(super) fn plain_target(target: &Expr) -> bool {
    match target {
        Expr::Name(_) => true,
        Expr::Tuple(t) => t.elts.iter().all(|e| matches!(e, Expr::Name(_))),
        Expr::List(l) => l.elts.iter().all(|e| matches!(e, Expr::Name(_))),
        _ => false,
    }
}

pub(super) fn binds_any(target: &Expr, names: &BTreeSet<String>) -> bool {
    names.iter().any(|name| target_binds(target, name))
}

/// Rewrites every reference to a celled variable in the enclosing
/// function's own scope (and the compiled scopes nested in it: a
/// comprehension) to its cell's `cell_contents`, outside the nested
/// statements CPython runs (`skip`).
pub(super) struct CellRewriter<'c> {
    pub(super) celled: &'c BTreeSet<String>,
    pub(super) source: &'c str,
    pub(super) skip: &'c [Range<u32>],
    /// The names each nested compiled scope binds itself, which shadow.
    pub(super) scopes: Vec<BTreeSet<String>>,
    pub(super) edits: Vec<names::Edit>,
    /// The expression of a celled name's cell (`__pycc_cell_x`).
    pub(super) cell: &'c dyn Fn(&str) -> String,
}

impl CellRewriter<'_> {
    fn skipped(&self, start: u32) -> bool {
        self.skip.iter().any(|range| range.contains(&start))
    }

    fn shadowed(&self, name: &str) -> bool {
        self.scopes.iter().skip(1).any(|scope| scope.contains(name))
    }
}

impl<'a> Visitor<'a> for CellRewriter<'_> {
    fn visit_stmt(&mut self, stmt: &'a Stmt) {
        let start = pycc_ast::stmt_range(stmt).start;
        if self.skipped(start) {
            return;
        }
        match stmt {
            Stmt::AnnAssign(assign)
                if matches!(assign.target.as_ref(), Expr::Name(n)
                    if self.celled.contains(n.id.as_str()) && !self.shadowed(n.id.as_str())) =>
            {
                let Expr::Name(name) = assign.target.as_ref() else {
                    unreachable!()
                };
                let range = pycc_ast::stmt_range(stmt);
                match &assign.value {
                    Some(value) => {
                        self.edits.push(names::Edit {
                            range: range.start..pycc_ast::expr_range(value).start,
                            text: format!("{}.cell_contents = ", (self.cell)(name.id.as_str())),
                            mirror: None,
                        });
                        self.visit_expr(value);
                    }
                    None => self.edits.push(names::Edit {
                        range,
                        text: "pass".to_string(),
                        mirror: None,
                    }),
                }
            }
            // A nested `def` pycc compiles (a closure it converts) reads a
            // celled variable through the cell too, which it captures; its
            // own `nonlocal` of one is dropped, the cell being the variable.
            Stmt::FunctionDef(def) => {
                for decorator in &def.decorator_list {
                    self.visit_decorator(decorator);
                }
                for param in def.parameters.iter() {
                    if let Some(default) = param.default() {
                        self.visit_expr(default);
                    }
                }
                let nonlocals = declared_nonlocals(&def.body);
                let mut own = names::bound_names(&def.body);
                own.extend(parameter_names(&def.parameters));
                own.retain(|name| !nonlocals.contains(name));
                self.scopes.push(own);
                for stmt in &def.body {
                    self.visit_stmt(stmt);
                }
                self.scopes.pop();
            }
            Stmt::Nonlocal(nonlocal)
                if nonlocal
                    .names
                    .iter()
                    .any(|n| self.celled.contains(n.as_str())) =>
            {
                let kept: Vec<&str> = nonlocal
                    .names
                    .iter()
                    .map(|n| n.as_str())
                    .filter(|n| !self.celled.contains(*n))
                    .collect();
                self.edits.push(names::Edit {
                    range: pycc_ast::stmt_range(stmt),
                    text: if kept.is_empty() {
                        "pass".to_string()
                    } else {
                        format!("nonlocal {}", kept.join(", "))
                    },
                    mirror: None,
                });
            }
            // `for x in xs:` of a celled `x` binds a plain name, stored
            // into the cell first thing in the body.
            Stmt::For(s) if binds_any(&s.target, self.celled) && self.scopes.len() == 1 => {
                let mut stores = Vec::new();
                let targets: Vec<&Expr> = match s.target.as_ref() {
                    Expr::Tuple(t) => t.elts.iter().collect(),
                    Expr::List(l) => l.elts.iter().collect(),
                    other => vec![other],
                };
                for target in targets {
                    if let Expr::Name(name) = target
                        && self.celled.contains(name.id.as_str())
                    {
                        self.edits.push(names::Edit {
                            range: name.range.start().to_u32()..name.range.end().to_u32(),
                            text: format!("__pycc_loop_{}", name.id),
                            mirror: None,
                        });
                        stores.push(format!(
                            "{}.cell_contents = __pycc_loop_{}",
                            (self.cell)(name.id.as_str()),
                            name.id
                        ));
                    }
                }
                if let Some(first) = s.body.first() {
                    let first = pycc_ast::stmt_range(first).start;
                    let line_start = self.source[..first as usize]
                        .rfind('\n')
                        .map_or(0, |i| i + 1) as u32;
                    let indent = &self.source[line_start as usize..first as usize];
                    self.edits.push(names::Edit {
                        range: line_start..line_start,
                        text: format!("{indent}{}\n", stores.join("; ")),
                        mirror: None,
                    });
                }
                self.visit_expr(&s.iter);
                for stmt in s.body.iter().chain(&s.orelse) {
                    self.visit_stmt(stmt);
                }
            }
            Stmt::ClassDef(_) => {}
            _ => visitor::walk_stmt(self, stmt),
        }
    }

    fn visit_expr(&mut self, expr: &'a Expr) {
        match expr {
            Expr::Name(name)
                if self.celled.contains(name.id.as_str())
                    && !self.shadowed(name.id.as_str())
                    && !self.skipped(name.range.start().to_u32()) =>
            {
                self.edits.push(names::Edit {
                    range: name.range.start().to_u32()..name.range.end().to_u32(),
                    text: format!("{}.cell_contents", (self.cell)(name.id.as_str())),
                    mirror: None,
                });
            }
            Expr::Lambda(lambda) if self.skipped(lambda.range.start().to_u32()) => {}
            Expr::Lambda(lambda) => {
                let params = lambda
                    .parameters
                    .as_ref()
                    .map(|p| parameter_names(p))
                    .unwrap_or_default();
                if let Some(parameters) = &lambda.parameters {
                    for param in parameters.iter() {
                        if let Some(default) = param.default() {
                            self.visit_expr(default);
                        }
                    }
                }
                self.scopes.push(params);
                self.visit_expr(&lambda.body);
                self.scopes.pop();
            }
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
}

impl CellRewriter<'_> {
    fn comprehension(&mut self, generators: &[pycc_ast::Comprehension], results: &[&Expr]) {
        let Some(first) = generators.first() else {
            return;
        };
        self.visit_expr(&first.iter);
        let mut targets = Refs::default();
        for generator in generators {
            targets.visit_expr(&generator.target);
        }
        self.scopes.push(targets.names);
        for (index, generator) in generators.iter().enumerate() {
            if index > 0 {
                self.visit_expr(&generator.iter);
            }
            self.visit_expr(&generator.target);
            for condition in &generator.ifs {
                self.visit_expr(condition);
            }
        }
        for result in results {
            self.visit_expr(result);
        }
        self.scopes.pop();
    }
}
