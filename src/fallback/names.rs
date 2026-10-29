//! Name analysis for the per-function fallback (#12): which module-level
//! names a statement binds, which names fallback code reads or declares
//! `global`, and where compiled code refers to a module global that has to
//! live in the module's namespace dictionary instead of a native slot.
//!
//! The analysis follows Python's scoping: a function (or lambda, or
//! comprehension) binds a name locally when it assigns it anywhere in its
//! body and does not declare it `global`; a class body's bindings are
//! visible only inside the class body itself.

use pycc_ast::visitor::{self, Visitor};
use pycc_ast::{Expr, ExprContext, Pattern, Stmt};
use std::collections::BTreeSet;
use std::ops::Range;

/// The names `body` binds in the scope it runs in, without descending into
/// nested function, lambda, class or comprehension scopes (a nested `def`
/// or `class` binds its own name here). Names declared `global` are
/// included: from the module's point of view they are bound too.
pub(crate) fn bound_names(body: &[Stmt]) -> BTreeSet<String> {
    let mut collector = Binder::default();
    collector.visit_body(body);
    collector.names
}

/// The names declared `global` directly in `body` (not in nested scopes).
pub(crate) fn declared_globals(body: &[Stmt]) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    for stmt in body {
        scope_globals(stmt, &mut found);
    }
    found
}

fn scope_globals(stmt: &Stmt, found: &mut BTreeSet<String>) {
    match stmt {
        Stmt::Global(global) => found.extend(global.names.iter().map(|n| n.as_str().to_string())),
        Stmt::If(s) => {
            s.body.iter().for_each(|s| scope_globals(s, found));
            for clause in &s.elif_else_clauses {
                clause.body.iter().for_each(|s| scope_globals(s, found));
            }
        }
        Stmt::For(s) => {
            s.body
                .iter()
                .chain(&s.orelse)
                .for_each(|s| scope_globals(s, found));
        }
        Stmt::While(s) => {
            s.body
                .iter()
                .chain(&s.orelse)
                .for_each(|s| scope_globals(s, found));
        }
        Stmt::With(s) => s.body.iter().for_each(|s| scope_globals(s, found)),
        Stmt::Try(s) => {
            let handlers = s.handlers.iter().flat_map(|h| match h {
                pycc_ast::ExceptHandler::ExceptHandler(h) => h.body.iter(),
            });
            s.body
                .iter()
                .chain(handlers)
                .chain(&s.orelse)
                .chain(&s.finalbody)
                .for_each(|s| scope_globals(s, found));
        }
        Stmt::Match(s) => {
            for case in &s.cases {
                case.body.iter().for_each(|s| scope_globals(s, found));
            }
        }
        _ => {}
    }
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
                // Decorators and defaults run in this scope; the body does not.
                for decorator in &def.decorator_list {
                    self.visit_decorator(decorator);
                }
            }
            Stmt::ClassDef(class) => {
                self.names.insert(class.name.as_str().to_string());
                for decorator in &class.decorator_list {
                    self.visit_decorator(decorator);
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
            Expr::Lambda(_)
            | Expr::ListComp(_)
            | Expr::SetComp(_)
            | Expr::DictComp(_)
            | Expr::Generator(_) => {}
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

/// Whether `name`, referenced in the innermost of `scopes`, is the module's.
fn is_module_global_in(scopes: &[Scope], name: &str) -> bool {
    for (depth, scope) in scopes.iter().rev().enumerate() {
        match scope.kind {
            ScopeKind::Module => return true,
            ScopeKind::Function => {
                if scope.globals.contains(name) {
                    return true;
                }
                if scope.locals.contains(name) {
                    return false;
                }
            }
            ScopeKind::Class => {
                if depth == 0 && scope.locals.contains(name) {
                    return false;
                }
            }
        }
    }
    true
}

/// One textual edit of compiled code: replace `range` with `text`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Edit {
    pub(crate) range: Range<u32>,
    pub(crate) text: String,
    /// The native global whose store this inserted text copies into the
    /// namespace (`Rewriter::mirrors`).
    pub(crate) mirror: Option<String>,
}

#[derive(Debug)]
enum ScopeKind {
    Module,
    Function,
    Class,
}

#[derive(Debug)]
struct Scope {
    kind: ScopeKind,
    locals: BTreeSet<String>,
    globals: BTreeSet<String>,
}

/// Rewrites every reference compiled code makes to a module global in
/// `dict_globals` into a subscript of the module namespace `ns`, and drops
/// those names from `global` statements.
pub(crate) struct Rewriter<'g> {
    dict_globals: &'g BTreeSet<String>,
    ns: &'g str,
    scopes: Vec<Scope>,
    pub(crate) edits: Vec<Edit>,
    /// Module globals fallback code reads but never binds that compiled
    /// code keeps native: every compiled store of one is followed by a copy
    /// into the namespace (`; ns['n'] = n` after the simple statement).
    mirrors: BTreeSet<String>,
    /// The end of the simple statement (`=`, `+=`, annotated `=`) being
    /// visited, where a mirror copy goes.
    simple_end: Option<u32>,
    /// Mirrored names bound somewhere a copy cannot follow (a `for` or
    /// `with` target, `del`, ...): they must live in the namespace.
    pub(crate) unmirrorable: BTreeSet<String>,
    /// When set, every module-global reference is recorded here instead of
    /// rewritten (`module_references`).
    record: Option<BTreeSet<String>>,
}

impl<'g> Rewriter<'g> {
    pub(crate) fn new(dict_globals: &'g BTreeSet<String>, ns: &'g str) -> Self {
        Self {
            dict_globals,
            ns,
            scopes: vec![Scope {
                kind: ScopeKind::Module,
                locals: BTreeSet::new(),
                globals: BTreeSet::new(),
            }],
            edits: Vec::new(),
            mirrors: BTreeSet::new(),
            simple_end: None,
            unmirrorable: BTreeSet::new(),
            record: None,
        }
    }

    /// Mirrors the module globals in `mirrors` (see the field).
    pub(crate) fn with_mirrors(mut self, mirrors: BTreeSet<String>) -> Self {
        self.mirrors = mirrors;
        self
    }

    /// Whether `name`, referenced in the innermost scope, is the module's.
    fn is_module_global(&self, name: &str) -> bool {
        is_module_global_in(&self.scopes, name)
    }

    fn push_function(&mut self, params: BTreeSet<String>, body: &[Stmt]) {
        let globals = declared_globals(body);
        let mut locals = bound_names(body);
        locals.extend(params);
        locals.retain(|name| !globals.contains(name));
        self.scopes.push(Scope {
            kind: ScopeKind::Function,
            locals,
            globals,
        });
    }

    fn subscript(&self, name: &str) -> String {
        format!("{}['{}']", self.ns, name)
    }
}

fn parameter_names(parameters: &pycc_ast::Parameters) -> BTreeSet<String> {
    parameters
        .iter()
        .map(|p| p.name().as_str().to_string())
        .collect()
}

impl<'a> Visitor<'a> for Rewriter<'_> {
    fn visit_stmt(&mut self, stmt: &'a Stmt) {
        let simple = matches!(stmt, Stmt::Assign(_) | Stmt::AugAssign(_))
            || matches!(stmt, Stmt::AnnAssign(a) if a.value.is_some());
        let outer = std::mem::replace(
            &mut self.simple_end,
            simple.then(|| pycc_ast::stmt_range(stmt).end),
        );
        self.visit_stmt_inner(stmt);
        self.simple_end = outer;
    }

    fn visit_expr(&mut self, expr: &'a Expr) {
        self.visit_expr_inner(expr);
    }

    fn visit_annotation(&mut self, _expr: &'a Expr) {}
}

impl<'a> Rewriter<'_> {
    fn visit_stmt_inner(&mut self, stmt: &'a Stmt) {
        match stmt {
            Stmt::FunctionDef(def) => {
                for decorator in &def.decorator_list {
                    self.visit_decorator(decorator);
                }
                for param in def.parameters.iter_non_variadic_params() {
                    if let Some(default) = &param.default {
                        self.visit_expr(default);
                    }
                }
                self.push_function(parameter_names(&def.parameters), &def.body);
                self.visit_body(&def.body);
                self.scopes.pop();
            }
            Stmt::ClassDef(class) => {
                for decorator in &class.decorator_list {
                    self.visit_decorator(decorator);
                }
                if let Some(arguments) = &class.arguments {
                    self.visit_arguments(arguments);
                }
                self.scopes.push(Scope {
                    kind: ScopeKind::Class,
                    locals: bound_names(&class.body),
                    globals: BTreeSet::new(),
                });
                self.visit_body(&class.body);
                self.scopes.pop();
            }
            Stmt::Global(global) => {
                let kept: Vec<&str> = global
                    .names
                    .iter()
                    .map(|n| n.as_str())
                    .filter(|n| !self.dict_globals.contains(*n))
                    .collect();
                if kept.len() != global.names.len() {
                    let text = if kept.is_empty() {
                        "pass".to_string()
                    } else {
                        format!("global {}", kept.join(", "))
                    };
                    let range = pycc_ast::stmt_range(stmt);
                    self.edits.push(Edit {
                        range,
                        text,
                        mirror: None,
                    });
                }
            }
            _ => visitor::walk_stmt(self, stmt),
        }
    }

    fn visit_expr_inner(&mut self, expr: &'a Expr) {
        match expr {
            Expr::Name(name)
                if self.record.is_none()
                    && self.mirrors.contains(name.id.as_str())
                    && self.is_module_global(name.id.as_str()) =>
            {
                let id = name.id.as_str();
                match (name.ctx, self.simple_end) {
                    (ExprContext::Load, _) => {}
                    (ExprContext::Store, Some(end)) => {
                        let text = format!("; {}['{id}'] = {id}", self.ns);
                        if !self.edits.iter().any(|e| e.range.start == end && e.text == text) {
                            self.edits.push(Edit {
                                range: end..end,
                                text,
                                mirror: Some(id.to_string()),
                            });
                        }
                    }
                    _ => {
                        self.unmirrorable.insert(id.to_string());
                    }
                }
            }
            Expr::Name(name) => {
                let id = name.id.as_str();
                if let Some(record) = &mut self.record {
                    let global = {
                        let scopes = &self.scopes;
                        is_module_global_in(scopes, id)
                    };
                    if global {
                        record.insert(id.to_string());
                    }
                } else if self.dict_globals.contains(id) && self.is_module_global(id) {
                    self.edits.push(Edit {
                        range: name.range.start().to_u32()..name.range.end().to_u32(),
                        text: self.subscript(id),
                        mirror: None,
                    });
                }
            }
            Expr::Lambda(lambda) => {
                let params = match &lambda.parameters {
                    Some(parameters) => {
                        for param in parameters.iter_non_variadic_params() {
                            if let Some(default) = &param.default {
                                self.visit_expr(default);
                            }
                        }
                        parameter_names(parameters)
                    }
                    None => BTreeSet::new(),
                };
                self.push_function(params, &[]);
                self.visit_expr(&lambda.body);
                self.scopes.pop();
            }
            Expr::ListComp(comp) => {
                self.comprehension(&comp.generators, &[&comp.elt]);
            }
            Expr::SetComp(comp) => {
                self.comprehension(&comp.generators, &[&comp.elt]);
            }
            Expr::Generator(comp) => {
                self.comprehension(&comp.generators, &[&comp.elt]);
            }
            Expr::DictComp(comp) => {
                let mut results: Vec<&Expr> = comp.key.iter().map(|k| &**k).collect();
                results.push(&comp.value);
                self.comprehension(&comp.generators, &results);
            }
            Expr::FString(fstring) => {
                let before = self.edits.len();
                visitor::walk_expr(self, expr);
                self.keep_debug_texts(fstring, before);
            }
            _ => visitor::walk_expr(self, expr),
        }
    }

    /// `f"{x=}"` prints the source text of its field, so a field whose
    /// names this pass rewrote (`__pycc_fallback_ns['x']`) keeps the text
    /// as written: it becomes literal text before the field, and the `=`
    /// becomes the `!r` it implies (none with a conversion or a format
    /// spec). A debug text holding a quote or a backslash is left alone
    /// (it cannot be copied into the literal part as is).
    fn keep_debug_texts(&mut self, fstring: &pycc_ast::ExprFString, before: usize) {
        let mut fixes = Vec::new();
        for part in fstring.value.iter() {
            let pycc_ast::FStringPart::FString(f) = part else {
                continue;
            };
            for element in f.elements.iter() {
                let pycc_ast::InterpolatedStringElement::Interpolation(interp) = element else {
                    continue;
                };
                let Some(debug) = &interp.debug_text else {
                    continue;
                };
                let text = debug.as_str();
                let expr = pycc_ast::expr_range(&interp.expression);
                let rewritten = self.edits[before..]
                    .iter()
                    .any(|e| expr.start <= e.range.start && e.range.end <= expr.end);
                if !rewritten || text.contains(['\'', '"', '\\']) {
                    continue;
                }
                let open = interp.range.start().to_u32();
                let debug_end = open + 1 + text.len() as u32;
                let literal = text.replace('{', "{{").replace('}', "}}");
                let implied = interp.conversion == pycc_ast::ConversionFlag::None
                    && interp.format_spec.is_none();
                fixes.push(Edit {
                    range: open..open,
                    text: literal,
                    mirror: None,
                });
                fixes.push(Edit {
                    range: expr.end..debug_end,
                    text: if implied { "!r" } else { "" }.to_string(),
                    mirror: None,
                });
            }
        }
        self.edits.extend(fixes);
    }
}

impl Rewriter<'_> {
    /// A comprehension: its first iterable runs in the enclosing scope, the
    /// rest in a scope of its own where its targets are local.
    fn comprehension(&mut self, generators: &[pycc_ast::Comprehension], results: &[&Expr]) {
        let Some(first) = generators.first() else {
            return;
        };
        self.visit_expr(&first.iter);
        let mut targets = Binder::default();
        for generator in generators {
            targets.visit_expr(&generator.target);
        }
        self.scopes.push(Scope {
            kind: ScopeKind::Function,
            locals: targets.names,
            globals: BTreeSet::new(),
        });
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

/// The module globals `stmt` (a top-level statement) reads, binds or
/// declares, honoring every nested scope.
pub(crate) fn module_references(stmt: &Stmt) -> BTreeSet<String> {
    let none = BTreeSet::new();
    let mut rewriter = Rewriter::new(&none, "");
    rewriter.record = Some(BTreeSet::new());
    rewriter.visit_stmt(stmt);
    let mut found = rewriter.record.unwrap_or_default();
    found.extend(all_global_declarations(stmt));
    found.extend(evaluated_annotation_names(stmt));
    found
}

/// The names read by the annotations CPython evaluates when `stmt` runs:
/// every `def`'s parameter and return annotations (evaluated when the
/// `def` runs, PEP 649 aside) and a module- or class-level annotated
/// assignment's. Fallback code reading them needs them in the namespace
/// (a compiled `from typing import Callable` is mirrored for it). A local
/// of a function body shadowing one is counted too, which only mirrors
/// more.
fn evaluated_annotation_names(stmt: &Stmt) -> BTreeSet<String> {
    #[derive(Default)]
    struct Names {
        found: BTreeSet<String>,
        /// Inside a function body, where `x: T = v` is not evaluated.
        in_function: bool,
    }
    impl Names {
        fn annotation(&mut self, expr: &Expr) {
            struct Reads<'n>(&'n mut BTreeSet<String>);
            impl<'a> Visitor<'a> for Reads<'_> {
                fn visit_expr(&mut self, expr: &'a Expr) {
                    if let Expr::Name(name) = expr {
                        self.0.insert(name.id.to_string());
                    }
                    visitor::walk_expr(self, expr);
                }
            }
            Reads(&mut self.found).visit_expr(expr);
        }
    }
    impl<'a> Visitor<'a> for Names {
        fn visit_stmt(&mut self, stmt: &'a Stmt) {
            match stmt {
                Stmt::FunctionDef(def) => {
                    for param in def.parameters.iter() {
                        if let Some(annotation) = param.annotation() {
                            self.annotation(annotation);
                        }
                    }
                    if let Some(returns) = &def.returns {
                        self.annotation(returns);
                    }
                    let outer = std::mem::replace(&mut self.in_function, true);
                    self.visit_body(&def.body);
                    self.in_function = outer;
                    return;
                }
                Stmt::ClassDef(class) => {
                    let outer = std::mem::replace(&mut self.in_function, false);
                    self.visit_body(&class.body);
                    self.in_function = outer;
                    return;
                }
                Stmt::AnnAssign(assign) if !self.in_function => {
                    self.annotation(&assign.annotation);
                }
                _ => {}
            }
            visitor::walk_stmt(self, stmt);
        }

        fn visit_annotation(&mut self, _expr: &'a Expr) {}
    }
    let mut names = Names::default();
    names.visit_stmt(stmt);
    names.found
}

/// Every name declared `global` anywhere in `stmt`.
pub(crate) fn all_global_declarations(stmt: &Stmt) -> BTreeSet<String> {
    #[derive(Default)]
    struct Globals(BTreeSet<String>);
    impl<'a> Visitor<'a> for Globals {
        fn visit_stmt(&mut self, stmt: &'a Stmt) {
            if let Stmt::Global(global) = stmt {
                self.0
                    .extend(global.names.iter().map(|n| n.as_str().to_string()));
            }
            visitor::walk_stmt(self, stmt);
        }
    }
    let mut globals = Globals::default();
    globals.visit_stmt(stmt);
    globals.0
}

/// The module-scope names `stmt` binds through something other than a
/// plain name target, which compiled code cannot redirect to the
/// namespace dictionary.
pub(crate) fn opaque_bound_names(stmt: &Stmt) -> BTreeSet<String> {
    let mut collector = Binder {
        opaque: true,
        ..Binder::default()
    };
    collector.visit_stmt(stmt);
    collector.names
}

/// The bare names the bases of the class statements in `stmt` read (the
/// root of an attribute chain, `mod` of `mod.Base`, included), at any depth
/// outside a function body.
pub(crate) fn base_names(stmt: &Stmt) -> BTreeSet<String> {
    fn root(expr: &Expr, out: &mut BTreeSet<String>) {
        match expr {
            Expr::Name(name) => {
                out.insert(name.id.as_str().to_string());
            }
            Expr::Attribute(attr) => root(&attr.value, out),
            Expr::Subscript(sub) => root(&sub.value, out),
            Expr::Call(call) => root(&call.func, out),
            _ => {}
        }
    }
    fn walk(body: &[Stmt], out: &mut BTreeSet<String>) {
        for stmt in body {
            match stmt {
                Stmt::ClassDef(class) => {
                    for base in class.bases() {
                        root(base, out);
                    }
                    walk(&class.body, out);
                }
                Stmt::If(s) => {
                    walk(&s.body, out);
                    for clause in &s.elif_else_clauses {
                        walk(&clause.body, out);
                    }
                }
                Stmt::For(s) => {
                    walk(&s.body, out);
                    walk(&s.orelse, out);
                }
                Stmt::While(s) => {
                    walk(&s.body, out);
                    walk(&s.orelse, out);
                }
                Stmt::With(s) => walk(&s.body, out),
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
    let mut out = BTreeSet::new();
    walk(std::slice::from_ref(stmt), &mut out);
    out
}
