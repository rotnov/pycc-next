//! Closure conversion: a `def` nested in a function becomes a module-level
//! function that receives the enclosing locals it reads as leading
//! parameters (lambda lifting).
//!
//! `def outer(): n = 3; def inner(x): return x + n; return inner(1)`
//! becomes `def _pycc_outer_inner(n, x): return x + n` before `outer`, and
//! `inner(1)` becomes `_pycc_outer_inner(n, 1)`. A call passes the
//! enclosing variable's *current* value, which is what the closure would
//! read when it runs, so rebinding the variable between the `def` and the
//! call is seen. Recursion, mutual recursion between sibling nested
//! functions and nested functions inside nested functions all lift (inner
//! first; a function also receives what the functions it calls capture).
//!
//! A variable a nested function writes with `nonlocal` lives in a
//! one-element list (a *cell*) in the enclosing function: every binding of
//! it there makes a new cell, every read and augmented assignment goes
//! through `cell[0]`, and the cell itself is what the lifted functions
//! receive; a lifted function's own assignment stores through `cell[0]`, so
//! its writes are seen by the enclosing function and by each other.
//!
//! A nested function is lifted only when every use of its name is a direct
//! call (or, see [`values`], it may be made a function value), the name is
//! bound once (by the `def`), it has no decorators and no non-literal
//! defaults, and every variable a
//! `nonlocal` names is bound only by plain assignments. A nested `name =
//! lambda ...` is first turned into the equivalent `def`. Anything else
//! stays nested, and the compiler reports it as before.

mod conditional;
mod lift;
pub(crate) mod qualname;
#[cfg(test)]
mod tests;
mod values;

use super::Fresh;
use super::INFER_MARKER;
use super::build::{assign, int, load, store, subscript};
use ruff_python_ast::visitor::transformer::{self, Transformer};
use ruff_python_ast::visitor::{self, Visitor};
use ruff_python_ast::{
    AtomicNodeIndex, Expr, ExprContext, ExprList, Identifier, Parameter, ParameterWithDefault,
    Stmt, StmtFunctionDef, StmtPass, StmtReturn,
};
use ruff_text_size::TextRange;
use std::collections::{BTreeSet, HashMap, HashSet};
use lift::lift_function;

/// The names a function's own scope binds, the `global`/`nonlocal` names it
/// declares, and the names it reads from enclosing scopes.
#[derive(Default, Debug)]
struct ScopeNames {
    bound: HashSet<String>,
    globals: HashSet<String>,
    nonlocals: HashSet<String>,
    /// Names read anywhere in the function (comprehension and lambda
    /// scopes included) that the function itself does not bind.
    free: BTreeSet<String>,
    /// How many binding sites each name has in the function's own scope
    /// (its parameters aside); a `global` or `nonlocal` declaration counts
    /// as two, so it is never mistaken for one assignment.
    sites: HashMap<String, usize>,
}

/// Collects a function body's own bindings (not those of nested functions,
/// lambdas or comprehensions): the one binding analysis both lifting and
/// function values (`values`) read, so the two cannot disagree. Every
/// binding form counts: assignment, augmented, annotated, `for`, `with`
/// and `del` targets, a walrus (also inside a comprehension), `except ...
/// as`, match captures (`case x`, `case [*rest]`, `case {**rest}`, `case
/// P(x=a)`, `case _ as a`), `type` aliases, imports and nested `def` and
/// `class` statements.
struct Bindings<'s>(&'s mut ScopeNames);

impl Bindings<'_> {
    fn bind(&mut self, name: &str) {
        self.0.bound.insert(name.to_string());
        *self.0.sites.entry(name.to_string()).or_default() += 1;
    }

    /// A `global`/`nonlocal` declaration: not a local, but never stable.
    fn declare(&mut self, name: &str) {
        *self.0.sites.entry(name.to_string()).or_default() += 2;
    }
}

impl<'a> Visitor<'a> for Bindings<'_> {
    fn visit_stmt(&mut self, stmt: &'a Stmt) {
        match stmt {
            Stmt::FunctionDef(def) => {
                self.bind(def.name.as_str());
                // Defaults and decorators are evaluated here; the body is
                // another scope.
                for decorator in &def.decorator_list {
                    self.visit_expr(&decorator.expression);
                }
                return;
            }
            Stmt::ClassDef(def) => {
                self.bind(def.name.as_str());
                return;
            }
            Stmt::Global(global) => {
                for name in &global.names {
                    self.0.globals.insert(name.to_string());
                    self.declare(name.as_str());
                }
            }
            Stmt::Nonlocal(nonlocal) => {
                for name in &nonlocal.names {
                    self.0.nonlocals.insert(name.to_string());
                    self.declare(name.as_str());
                }
            }
            Stmt::Import(import) => {
                for alias in &import.names {
                    let bound = alias.asname.as_ref().unwrap_or(&alias.name);
                    self.bind(bound.split('.').next().unwrap_or(""));
                }
            }
            Stmt::ImportFrom(import) => {
                for alias in &import.names {
                    let bound = alias.asname.as_ref().unwrap_or(&alias.name);
                    // `from m import *` may bind anything.
                    if bound.as_str() == "*" {
                        self.0.sites.insert("*".to_string(), 2);
                    }
                    self.bind(bound.as_str());
                }
            }
            Stmt::Try(try_stmt) => {
                for handler in &try_stmt.handlers {
                    let ruff_python_ast::ExceptHandler::ExceptHandler(h) = handler;
                    if let Some(name) = &h.name {
                        // Bound, then deleted at the handler's end.
                        self.bind(name.as_str());
                        self.bind(name.as_str());
                    }
                }
            }
            _ => {}
        }
        visitor::walk_stmt(self, stmt);
    }

    fn visit_pattern(&mut self, pattern: &'a ruff_python_ast::Pattern) {
        use ruff_python_ast::Pattern;
        let name = match pattern {
            Pattern::MatchAs(p) => p.name.as_ref(),
            Pattern::MatchStar(p) => p.name.as_ref(),
            Pattern::MatchMapping(p) => p.rest.as_ref(),
            _ => None,
        };
        if let Some(name) = name {
            self.bind(name.as_str());
        }
        visitor::walk_pattern(self, pattern);
    }

    fn visit_expr(&mut self, expr: &'a Expr) {
        match expr {
            Expr::Name(name) if name.ctx != ExprContext::Load => {
                self.bind(name.id.as_str());
            }
            // A walrus in a comprehension binds in the enclosing function.
            Expr::Named(named) => {
                if let Expr::Name(name) = named.target.as_ref() {
                    self.bind(name.id.as_str());
                }
                self.visit_expr(&named.value);
                return;
            }
            Expr::Lambda(_)
            | Expr::ListComp(_)
            | Expr::SetComp(_)
            | Expr::DictComp(_)
            | Expr::Generator(_) => {
                // Their own bindings are local to them; walrus targets
                // inside comprehensions are found by the walk below.
                struct Walrus<'w, 's>(&'w mut Bindings<'s>);
                impl<'b> Visitor<'b> for Walrus<'_, '_> {
                    fn visit_expr(&mut self, expr: &'b Expr) {
                        if let Expr::Named(named) = expr
                            && let Expr::Name(name) = named.target.as_ref()
                        {
                            self.0.bind(name.id.as_str());
                        }
                        visitor::walk_expr(self, expr);
                    }
                }
                Walrus(self).visit_expr(expr);
                return;
            }
            _ => {}
        }
        visitor::walk_expr(self, expr);
    }
}

/// The binding sites of `body`'s own scope ([`Bindings`]), by name.
fn binding_sites(body: &[Stmt]) -> HashMap<String, usize> {
    let mut names = ScopeNames::default();
    let mut bindings = Bindings(&mut names);
    for stmt in body {
        bindings.visit_stmt(stmt);
    }
    names.sites
}

/// Collects every name read in a function body (nested functions
/// excluded) that no scope inside the body binds.
struct Reads<'s> {
    reads: &'s mut BTreeSet<String>,
    /// Names bound by the lambda and comprehension scopes being walked.
    inner: Vec<HashSet<String>>,
}

impl Reads<'_> {
    fn shadowed(&self, name: &str) -> bool {
        self.inner.iter().any(|scope| scope.contains(name))
    }
}

impl<'a> Visitor<'a> for Reads<'_> {
    fn visit_stmt(&mut self, stmt: &'a Stmt) {
        if let Stmt::FunctionDef(def) = stmt {
            for decorator in &def.decorator_list {
                self.visit_expr(&decorator.expression);
            }
            for param in def.parameters.iter() {
                if let Some(default) = param.default() {
                    self.visit_expr(default);
                }
            }
            // Its free names are read here too, when it reads ours: they
            // are added by the caller once it is lifted (a call to it).
            return;
        }
        if let Stmt::ClassDef(_) = stmt {
            return;
        }
        visitor::walk_stmt(self, stmt);
    }

    fn visit_expr(&mut self, expr: &'a Expr) {
        match expr {
            Expr::Name(name) => {
                if !self.shadowed(name.id.as_str()) {
                    self.reads.insert(name.id.to_string());
                }
            }
            Expr::Lambda(lambda) => {
                let mut scope = HashSet::new();
                if let Some(params) = lambda.parameters.as_deref() {
                    for param in params.iter() {
                        if let Some(default) = param.default() {
                            self.visit_expr(default);
                        }
                        scope.insert(param.name().to_string());
                    }
                }
                self.inner.push(scope);
                self.visit_expr(&lambda.body);
                self.inner.pop();
            }
            Expr::ListComp(_) | Expr::SetComp(_) | Expr::DictComp(_) | Expr::Generator(_) => {
                let generators = match expr {
                    Expr::ListComp(c) => &c.generators,
                    Expr::SetComp(c) => &c.generators,
                    Expr::DictComp(c) => &c.generators,
                    Expr::Generator(c) => &c.generators,
                    _ => unreachable!(),
                };
                let mut scope = HashSet::new();
                for (i, generator) in generators.iter().enumerate() {
                    // The first iterable is evaluated in the enclosing scope.
                    if i == 0 {
                        self.visit_expr(&generator.iter);
                    }
                    collect_store_names(&generator.target, &mut scope);
                }
                self.inner.push(scope);
                for (i, generator) in generators.iter().enumerate() {
                    if i > 0 {
                        self.visit_expr(&generator.iter);
                    }
                    for cond in &generator.ifs {
                        self.visit_expr(cond);
                    }
                }
                match expr {
                    Expr::ListComp(c) => self.visit_expr(&c.elt),
                    Expr::SetComp(c) => self.visit_expr(&c.elt),
                    Expr::Generator(c) => self.visit_expr(&c.elt),
                    Expr::DictComp(c) => {
                        if let Some(key) = &c.key {
                            self.visit_expr(key);
                        }
                        self.visit_expr(&c.value);
                    }
                    _ => unreachable!(),
                }
                self.inner.pop();
            }
            _ => visitor::walk_expr(self, expr),
        }
    }
}

fn collect_store_names(target: &Expr, out: &mut HashSet<String>) {
    match target {
        Expr::Name(name) => {
            out.insert(name.id.to_string());
        }
        Expr::Tuple(tuple) => tuple.elts.iter().for_each(|e| collect_store_names(e, out)),
        Expr::List(list) => list.elts.iter().for_each(|e| collect_store_names(e, out)),
        Expr::Starred(starred) => collect_store_names(&starred.value, out),
        _ => {}
    }
}

/// The scope names of a function (`params` bound, `body` walked).
fn scope_names(def: &StmtFunctionDef) -> ScopeNames {
    let mut names = ScopeNames::default();
    for param in def.parameters.iter() {
        names.bound.insert(param.name().to_string());
    }
    let mut bindings = Bindings(&mut names);
    for stmt in def.body.iter() {
        bindings.visit_stmt(stmt);
    }
    for global in names.globals.clone() {
        names.bound.remove(&global);
    }
    for nonlocal in names.nonlocals.clone() {
        names.bound.remove(&nonlocal);
    }
    let mut reads = BTreeSet::new();
    let mut walker = Reads {
        reads: &mut reads,
        inner: Vec::new(),
    };
    for stmt in def.body.iter() {
        walker.visit_stmt(stmt);
    }
    names.free = reads
        .into_iter()
        .filter(|name| !names.bound.contains(name))
        .collect();
    names
}

/// How each use of a name occurs in a body: direct calls, and any other
/// use (a read as a value, a binding, a `del`).
#[derive(Default)]
pub(super) struct Uses {
    pub(super) calls: usize,
    pub(super) other: usize,
    pub(super) bindings: usize,
}

pub(super) fn count_uses(body: &[Stmt], name: &str) -> Uses {
    struct Count<'n> {
        name: &'n str,
        uses: Uses,
    }
    impl<'a> Visitor<'a> for Count<'_> {
        fn visit_stmt(&mut self, stmt: &'a Stmt) {
            match stmt {
                Stmt::FunctionDef(def) if def.name.as_str() == self.name => self.uses.bindings += 1,
                Stmt::ClassDef(def) if def.name.as_str() == self.name => self.uses.bindings += 1,
                Stmt::Global(g) if g.names.iter().any(|n| n.as_str() == self.name) => {
                    self.uses.other += 1
                }
                Stmt::Nonlocal(g) if g.names.iter().any(|n| n.as_str() == self.name) => {
                    self.uses.other += 1
                }
                _ => {}
            }
            visitor::walk_stmt(self, stmt);
        }
        fn visit_expr(&mut self, expr: &'a Expr) {
            if let Expr::Call(call) = expr
                && let Expr::Name(func) = call.func.as_ref()
                && func.id.as_str() == self.name
            {
                if call
                    .arguments
                    .args
                    .iter()
                    .any(|a| matches!(a, Expr::Starred(_)))
                    || call.arguments.keywords.iter().any(|k| k.arg.is_none())
                {
                    self.uses.other += 1;
                } else {
                    self.uses.calls += 1;
                }
                for arg in call.arguments.args.iter() {
                    self.visit_expr(arg);
                }
                for keyword in call.arguments.keywords.iter() {
                    self.visit_expr(&keyword.value);
                }
                return;
            }
            if let Expr::Name(n) = expr
                && n.id.as_str() == self.name
            {
                if n.ctx == ExprContext::Load {
                    self.uses.other += 1;
                } else {
                    self.uses.bindings += 1;
                }
            }
            visitor::walk_expr(self, expr);
        }
    }
    let mut count = Count {
        name,
        uses: Uses::default(),
    };
    for stmt in body {
        count.visit_stmt(stmt);
    }
    count.uses
}

/// Whether a default value is a literal (evaluating it once, at module
/// level, is then the same as evaluating it at every `def`).
fn literal_default(expr: &Expr) -> bool {
    match expr {
        Expr::NumberLiteral(_)
        | Expr::StringLiteral(_)
        | Expr::BooleanLiteral(_)
        | Expr::NoneLiteral(_) => true,
        Expr::UnaryOp(unary) => matches!(*unary.operand, Expr::NumberLiteral(_)),
        _ => false,
    }
}

/// Whether `def` can be lifted at all, judged on its own shape.
pub(super) fn liftable_shape(def: &StmtFunctionDef) -> bool {
    !def.is_async
        && def.decorator_list.is_empty()
        && def.type_params.is_none()
        && def
            .parameters
            .posonlyargs
            .iter()
            .chain(&def.parameters.args)
            .chain(&def.parameters.kwonlyargs)
            .all(|p| p.default.as_deref().is_none_or(literal_default))
}

/// One nested function chosen for lifting.
struct Lift {
    /// The nested function's name and its module-level name.
    name: String,
    lifted: String,
    /// The enclosing variables it receives, in order; a celled one is
    /// passed as its cell.
    captures: Vec<String>,
}

fn cell_name(name: &str) -> String {
    format!("_pycc_cell_{name}")
}

/// Rewrites, in a body: calls of lifted functions (prepending their
/// captures), and every use of a celled variable (through its cell).
struct Rewrite<'a> {
    lifts: &'a HashMap<String, Lift>,
    cells: &'a HashSet<String>,
    /// Rewriting a lifted function's body, which receives the cells: an
    /// assignment stores into the cell rather than making a new one.
    in_lifted: bool,
}

impl Rewrite<'_> {
    fn cell_item(&self, name: &str, ctx: ExprContext, range: TextRange) -> Expr {
        subscript(load(&cell_name(name), range), int(0, range), ctx, range)
    }

    fn capture_arg(&self, capture: &str, range: TextRange) -> Expr {
        if self.cells.contains(capture) {
            load(&cell_name(capture), range)
        } else {
            load(capture, range)
        }
    }
}

impl Transformer for Rewrite<'_> {
    fn visit_stmt(&self, stmt: &mut Stmt) {
        match stmt {
            // A nested function not lifted keeps its own scope: leave it.
            Stmt::FunctionDef(_) | Stmt::ClassDef(_) => {}
            Stmt::Nonlocal(nonlocal) => {
                nonlocal.names.retain(|n| !self.cells.contains(n.as_str()));
                if nonlocal.names.is_empty() {
                    *stmt = Stmt::Pass(StmtPass {
                        node_index: AtomicNodeIndex::default(),
                        range: nonlocal.range,
                    });
                }
            }
            // `x = v` of a celled variable makes a new cell.
            Stmt::Assign(assign)
                if assign.targets.len() == 1
                    && matches!(&assign.targets[0], Expr::Name(n) if self.cells.contains(n.id.as_str())) =>
            {
                let Expr::Name(target) = &assign.targets[0] else {
                    unreachable!()
                };
                let name = target.id.to_string();
                let range = assign.range;
                self.visit_expr(&mut assign.value);
                if self.in_lifted {
                    assign.targets[0] = self.cell_item(&name, ExprContext::Store, range);
                    return;
                }
                let value = std::mem::replace(assign.value.as_mut(), load("_", range));
                *stmt = assign_cell(&name, value, range);
            }
            _ => transformer::walk_stmt(self, stmt),
        }
    }

    fn visit_expr(&self, expr: &mut Expr) {
        if let Expr::Call(call) = expr
            && let Expr::Name(func) = call.func.as_ref()
            && let Some(lift) = self.lifts.get(func.id.as_str())
        {
            let range = call.range;
            for arg in call.arguments.args.iter_mut() {
                self.visit_expr(arg);
            }
            for keyword in call.arguments.keywords.iter_mut() {
                self.visit_expr(&mut keyword.value);
            }
            let mut args: Vec<Expr> = lift
                .captures
                .iter()
                .map(|capture| self.capture_arg(capture, range))
                .collect();
            args.extend(call.arguments.args.iter().cloned());
            call.arguments.args = args.into_boxed_slice();
            call.func = Box::new(load(&lift.lifted, range));
            return;
        }
        // A nested function read as a value: its function value, with its
        // captures bound now (`values`).
        if let Expr::Name(name) = expr
            && name.ctx == ExprContext::Load
            && let Some(lift) = self.lifts.get(name.id.as_str())
        {
            let range = name.range;
            let mut args = vec![load(&lift.lifted, range)];
            args.extend(
                lift.captures
                    .iter()
                    .map(|capture| self.capture_arg(capture, range)),
            );
            *expr = super::build::call_name(super::CLOSURE_MARKER, args, range);
            return;
        }
        if let Expr::Name(name) = expr
            && self.cells.contains(name.id.as_str())
        {
            *expr = self.cell_item(name.id.as_str(), name.ctx, name.range);
            return;
        }
        transformer::walk_expr(self, expr);
    }
}

fn assign_cell(name: &str, value: Expr, range: TextRange) -> Stmt {
    let cell = Expr::List(ExprList {
        node_index: AtomicNodeIndex::default(),
        range,
        elts: vec![value],
        ctx: ExprContext::Load,
    });
    assign(store(&cell_name(name), range), cell, range)
}

fn param(name: &str, range: TextRange) -> ParameterWithDefault {
    ParameterWithDefault {
        range,
        node_index: AtomicNodeIndex::default(),
        parameter: Parameter {
            range,
            node_index: AtomicNodeIndex::default(),
            name: Identifier::new(name, range),
            annotation: None,
        },
        default: None,
    }
}

/// Turns `name = lambda ...` statements directly in `body` into `def`s
/// when `name` is bound nowhere else in `body`.
fn lambdas_to_defs(body: &mut [Stmt]) {
    let names: Vec<String> = body
        .iter()
        .filter_map(|stmt| match stmt {
            Stmt::Assign(assign) => match (assign.targets.as_slice(), assign.value.as_ref()) {
                ([Expr::Name(target)], Expr::Lambda(_)) => Some(target.id.to_string()),
                _ => None,
            },
            _ => None,
        })
        .collect();
    for name in names {
        // Only a lambda every use of which is a direct call can be lifted;
        // any other stays a lambda (and is reported as one).
        let uses = count_uses(body, &name);
        if uses.bindings != 1 || uses.other != 0 {
            continue;
        }
        for stmt in body.iter_mut() {
            let Stmt::Assign(assign) = stmt else { continue };
            let ([Expr::Name(target)], Expr::Lambda(lambda)) =
                (assign.targets.as_slice(), assign.value.as_ref())
            else {
                continue;
            };
            if target.id.as_str() != name {
                continue;
            }
            let range = assign.range;
            let params = lambda.parameters.as_deref().cloned().unwrap_or_else(|| {
                ruff_python_ast::Parameters {
                    range,
                    node_index: AtomicNodeIndex::default(),
                    posonlyargs: Vec::new().into(),
                    args: Vec::new().into(),
                    vararg: None,
                    kwonlyargs: Vec::new().into(),
                    kwarg: None,
                }
            });
            *stmt = Stmt::FunctionDef(StmtFunctionDef {
                node_index: AtomicNodeIndex::default(),
                range,
                is_async: false,
                decorator_list: Vec::new().into(),
                name: Identifier::new(name.as_str(), range),
                type_params: None,
                parameters: Box::new(params),
                returns: Some(Box::new(load(INFER_MARKER, range))),
                body: vec![Stmt::Return(StmtReturn {
                    node_index: AtomicNodeIndex::default(),
                    range,
                    value: Some(lambda.body.clone()),
                })]
                .into(),
            });
        }
    }
}

/// Every nested `def` directly in `body` or in its blocks (not inside a
/// further nested `def`), mutably.
fn nested_defs_mut(body: &mut [Stmt], out: &mut Vec<*mut StmtFunctionDef>) {
    for stmt in body.iter_mut() {
        match stmt {
            Stmt::FunctionDef(def) => out.push(def as *mut StmtFunctionDef),
            Stmt::If(s) => {
                nested_defs_mut(&mut s.body, out);
                for clause in &mut s.elif_else_clauses {
                    nested_defs_mut(&mut clause.body, out);
                }
            }
            Stmt::While(s) => {
                nested_defs_mut(&mut s.body, out);
                nested_defs_mut(&mut s.orelse, out);
            }
            Stmt::For(s) => {
                nested_defs_mut(&mut s.body, out);
                nested_defs_mut(&mut s.orelse, out);
            }
            Stmt::With(s) => nested_defs_mut(&mut s.body, out),
            Stmt::Try(s) => {
                nested_defs_mut(&mut s.body, out);
                for handler in &mut s.handlers {
                    let ruff_python_ast::ExceptHandler::ExceptHandler(h) = handler;
                    nested_defs_mut(&mut h.body, out);
                }
                nested_defs_mut(&mut s.orelse, out);
                nested_defs_mut(&mut s.finalbody, out);
            }
            _ => {}
        }
    }
}

/// Removes the `def`s named in `lifted` from `body` and its blocks (a block
/// left empty gets a `pass`).
fn remove_defs(body: &mut Vec<Stmt>, lifted: &HashSet<String>) {
    let range = body
        .first()
        .map(|s| ruff_text_size::Ranged::range(s))
        .unwrap_or_default();
    body.retain(
        |stmt| !matches!(stmt, Stmt::FunctionDef(def) if lifted.contains(def.name.as_str())),
    );
    for stmt in body.iter_mut() {
        match stmt {
            Stmt::If(s) => {
                remove_defs_thin(&mut s.body, lifted);
                for clause in &mut s.elif_else_clauses {
                    remove_defs_thin(&mut clause.body, lifted);
                }
            }
            Stmt::While(s) => {
                remove_defs_thin(&mut s.body, lifted);
                remove_defs_thin(&mut s.orelse, lifted);
            }
            Stmt::For(s) => {
                remove_defs_thin(&mut s.body, lifted);
                remove_defs_thin(&mut s.orelse, lifted);
            }
            Stmt::With(s) => remove_defs_thin(&mut s.body, lifted),
            Stmt::Try(s) => {
                remove_defs_thin(&mut s.body, lifted);
                for handler in &mut s.handlers {
                    let ruff_python_ast::ExceptHandler::ExceptHandler(h) = handler;
                    remove_defs_thin(&mut h.body, lifted);
                }
                remove_defs_thin(&mut s.orelse, lifted);
                remove_defs_thin(&mut s.finalbody, lifted);
            }
            _ => {}
        }
    }
    if body.is_empty() {
        body.push(Stmt::Pass(StmtPass {
            node_index: AtomicNodeIndex::default(),
            range,
        }));
    }
}

fn remove_defs_thin(body: &mut thin_vec::ThinVec<Stmt>, lifted: &HashSet<String>) {
    if body.is_empty() {
        return;
    }
    let mut v: Vec<Stmt> = std::mem::take(body).into_iter().collect();
    remove_defs(&mut v, lifted);
    *body = v.into();
}

/// Whether every binding of `name` in `body` is a plain `name = value`
/// statement (so a cell can replace it), the function's own parameters
/// aside.
fn only_plain_assignments(body: &[Stmt], name: &str) -> bool {
    struct Check<'n> {
        name: &'n str,
        ok: bool,
    }
    impl<'a> Visitor<'a> for Check<'_> {
        fn visit_stmt(&mut self, stmt: &'a Stmt) {
            match stmt {
                Stmt::FunctionDef(_) | Stmt::ClassDef(_) => return,
                Stmt::Assign(assign)
                    if assign.targets.len() == 1
                        && matches!(&assign.targets[0], Expr::Name(n) if n.id.as_str() == self.name) =>
                {
                    self.visit_expr(&assign.value);
                    return;
                }
                Stmt::AugAssign(aug) if matches!(aug.target.as_ref(), Expr::Name(n) if n.id.as_str() == self.name) =>
                {
                    self.visit_expr(&aug.value);
                    return;
                }
                _ => {}
            }
            visitor::walk_stmt(self, stmt);
        }
        fn visit_expr(&mut self, expr: &'a Expr) {
            if let Expr::Name(n) = expr
                && n.id.as_str() == self.name
                && n.ctx != ExprContext::Load
            {
                self.ok = false;
            }
            if let Expr::Named(named) = expr
                && matches!(named.target.as_ref(), Expr::Name(n) if n.id.as_str() == self.name)
            {
                self.ok = false;
            }
            visitor::walk_expr(self, expr);
        }
    }
    let mut check = Check { name, ok: true };
    for stmt in body {
        check.visit_stmt(stmt);
    }
    check.ok
}

/// Lifts every liftable nested function of the module (see the module
/// documentation), placing each lifted `def` right before the top-level
/// statement that contained it.
pub(crate) fn lift_closures(body: &mut thin_vec::ThinVec<Stmt>, fresh: &Fresh) {
    super::hoist::hoist_block_defs(body);
    let old = std::mem::take(body);
    for mut stmt in old {
        let mut lifted = Vec::new();
        match &mut stmt {
            Stmt::FunctionDef(def) => {
                let owner = def.name.to_string();
                lift_function(def, &owner, fresh, &mut lifted);
            }
            Stmt::ClassDef(class) => {
                let class_name = class.name.to_string();
                for item in class.body.iter_mut() {
                    if let Stmt::FunctionDef(method) = item {
                        let owner = format!("{class_name}.{}", method.name.as_str());
                        lift_function(method, &owner, fresh, &mut lifted);
                    }
                }
            }
            _ => {}
        }
        body.extend(lifted);
        body.push(stmt);
    }
}
