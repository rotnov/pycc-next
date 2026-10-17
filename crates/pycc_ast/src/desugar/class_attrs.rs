//! Strict mode: a class attribute the program assigns through its class
//! (`Counter.count += 1`) becomes a module variable.
//!
//! pycc folds a class attribute into every read as a constant, so it has no
//! storage to write. When every access of attribute `X` anywhere in the
//! program is spelled `C.X` for the one class `C` that binds it, `C.X` is
//! indistinguishable from a module variable `_pycc_cattr_C_X`: this pass
//! binds that variable right after the class statement (to the class body's
//! value), rewrites every `C.X` to it, and declares it `global` in each
//! function that assigns it. [`super::globals`] then gives it the cell every
//! function-rebound module variable gets.
//!
//! The rewrite is exact only in that shape, so nothing is rewritten when any
//! access could observe the difference:
//!
//! * the class is a plain top-level class: no decorator (a dataclass field
//!   is not a class attribute), no type parameters, no keywords
//!   (`metaclass=`), and only plain names of the program's classes or
//!   `object` as bases;
//! * the class body binds `X` once, as `X = v` or `X: T = v`, and nothing
//!   else in the class body reads `X`; `v` calls nothing and reads no name
//!   the class body binds, so evaluating it after the class statement
//!   gives the same value;
//! * an attribute `X` is read or written on any other base (`self.X`,
//!   `Sub.X`, `obj.X`), except as the callee of a call (`text.count(...)`
//!   is a method of another type), or `C.X` is deleted, called, or written
//!   directly in a class body.
//!
//! A class whose attribute is never written keeps its constant.

use super::build::{ann_assign, assign, load, store};
use ruff_python_ast::visitor::transformer::{self, Transformer};
use ruff_python_ast::visitor::{self, Visitor};
use ruff_python_ast::{AtomicNodeIndex, Expr, ExprContext, Identifier, Stmt, StmtGlobal};
use std::collections::{HashMap, HashSet};

/// The prefix of a class attribute's module variable.
const PREFIX: &str = "_pycc_cattr_";

fn variable(class: &str, attr: &str) -> String {
    format!("{PREFIX}{class}_{attr}")
}

/// Rewrites every assigned class attribute of `body` (module doc).
pub(crate) fn mutable_class_attrs(body: &mut Vec<Stmt>) {
    if super::dataclasses::is_non_strict() {
        return;
    }
    let classes: HashSet<String> = body
        .iter()
        .filter_map(|stmt| match stmt {
            Stmt::ClassDef(class) => Some(class.name.to_string()),
            _ => None,
        })
        .collect();
    // (class, attr) -> the class statement's index.
    let mut candidates: HashMap<(String, String), usize> = HashMap::new();
    for (index, stmt) in body.iter().enumerate() {
        if let Stmt::ClassDef(class) = stmt
            && plain_class(class, &classes)
        {
            for attr in single_bindings(&class.body) {
                candidates.insert((class.name.to_string(), attr), index);
            }
        }
    }
    if candidates.is_empty() {
        return;
    }
    let attrs: HashSet<String> = candidates.keys().map(|(_, attr)| attr.clone()).collect();
    let mut uses = Uses {
        attrs: &attrs,
        candidates: &candidates,
        scopes: Vec::new(),
        refused: HashSet::new(),
        stored: HashSet::new(),
        names: HashSet::new(),
    };
    for stmt in body.iter() {
        uses.visit_stmt(stmt);
    }
    if uses.names.iter().any(|name| name.starts_with(PREFIX)) {
        return;
    }
    let rewritten: HashSet<(String, String)> = candidates
        .keys()
        .filter(|key| uses.stored.contains(*key) && !uses.refused.contains(&key.1))
        .cloned()
        .collect();
    if rewritten.is_empty() {
        return;
    }
    let rename = Rename { rewritten: &rewritten };
    for stmt in body.iter_mut() {
        rename.visit_stmt(stmt);
    }
    let variables: HashSet<String> = rewritten.iter().map(|(c, a)| variable(c, a)).collect();
    declare_globals(body, &variables);
    // The module variables, right after their class statements (last
    // class first, so earlier indices stay valid).
    let mut bindings: Vec<(usize, Stmt)> = Vec::new();
    for key in &rewritten {
        let index = candidates[key];
        let Stmt::ClassDef(class) = &body[index] else {
            unreachable!("a candidate index names a class statement")
        };
        bindings.push((index, module_binding(&class.body, &key.0, &key.1)));
    }
    bindings.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| stmt_name(&b.1).cmp(&stmt_name(&a.1))));
    for (index, binding) in bindings {
        body.insert(index + 1, binding);
    }
}

/// A sort key for deterministic insertion order.
fn stmt_name(stmt: &Stmt) -> String {
    let target = match stmt {
        Stmt::Assign(a) => &a.targets[0],
        Stmt::AnnAssign(a) => &*a.target,
        _ => return String::new(),
    };
    match target {
        Expr::Name(name) => name.id.to_string(),
        _ => String::new(),
    }
}

fn plain_class(class: &ruff_python_ast::StmtClassDef, classes: &HashSet<String>) -> bool {
    if !class.decorator_list.is_empty() || class.type_params.is_some() {
        return false;
    }
    match &class.arguments {
        None => true,
        Some(arguments) => {
            arguments.keywords.is_empty()
                && arguments.args.iter().all(|base| {
                    matches!(base, Expr::Name(name)
                        if name.id.as_str() == "object" || classes.contains(name.id.as_str()))
                })
        }
    }
}

/// The attributes `body` (a class body) binds exactly once, by a plain
/// assignment whose value is safe to evaluate after the class statement.
fn single_bindings(body: &[Stmt]) -> Vec<String> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    let mut safe: Vec<String> = Vec::new();
    let bound = class_body_names(body);
    for stmt in body {
        let (target, value) = match stmt {
            Stmt::Assign(a) if a.targets.len() == 1 => (&a.targets[0], Some(&*a.value)),
            Stmt::AnnAssign(a) => (&*a.target, a.value.as_deref()),
            _ => continue,
        };
        let Expr::Name(name) = target else { continue };
        *counts.entry(name.id.to_string()).or_default() += 1;
        if let Some(value) = value
            && safe_value(value, &bound)
        {
            safe.push(name.id.to_string());
        }
    }
    let mut reads = ClassBodyReads::default();
    for stmt in body {
        reads.visit_stmt(stmt);
    }
    safe.retain(|attr| counts[attr] == 1 && !reads.0.contains(attr));
    safe
}

/// Every name a class body binds at its own level.
fn class_body_names(body: &[Stmt]) -> HashSet<String> {
    let mut names = HashSet::new();
    for stmt in body {
        match stmt {
            Stmt::Assign(a) => a.targets.iter().for_each(|t| target_names(t, &mut names)),
            Stmt::AnnAssign(a) => target_names(&a.target, &mut names),
            Stmt::AugAssign(a) => target_names(&a.target, &mut names),
            Stmt::FunctionDef(f) => {
                names.insert(f.name.to_string());
            }
            Stmt::ClassDef(c) => {
                names.insert(c.name.to_string());
            }
            _ => {}
        }
    }
    names
}

fn target_names(target: &Expr, out: &mut HashSet<String>) {
    match target {
        Expr::Name(name) => {
            out.insert(name.id.to_string());
        }
        Expr::Tuple(t) => t.elts.iter().for_each(|e| target_names(e, out)),
        Expr::List(l) => l.elts.iter().for_each(|e| target_names(e, out)),
        Expr::Starred(s) => target_names(&s.value, out),
        _ => {}
    }
}

/// Whether evaluating `value` after the class statement gives the value
/// the class body computed: it calls nothing, binds nothing and reads no
/// name of the class body.
fn safe_value(value: &Expr, bound: &HashSet<String>) -> bool {
    struct Safe<'a> {
        bound: &'a HashSet<String>,
        ok: bool,
    }
    impl<'a> Visitor<'a> for Safe<'_> {
        fn visit_expr(&mut self, expr: &'a Expr) {
            match expr {
                Expr::Call(_)
                | Expr::Lambda(_)
                | Expr::Named(_)
                | Expr::Yield(_)
                | Expr::YieldFrom(_)
                | Expr::Await(_)
                | Expr::ListComp(_)
                | Expr::SetComp(_)
                | Expr::DictComp(_)
                | Expr::Generator(_)
                | Expr::FString(_)
                | Expr::Subscript(_)
                | Expr::Attribute(_) => self.ok = false,
                Expr::Name(name) if self.bound.contains(name.id.as_str()) => self.ok = false,
                _ => visitor::walk_expr(self, expr),
            }
        }
    }
    let mut safe = Safe { bound, ok: true };
    safe.visit_expr(value);
    safe.ok
}

/// The names a class body reads at its own level (not in its methods).
#[derive(Default)]
struct ClassBodyReads(HashSet<String>);

impl<'a> Visitor<'a> for ClassBodyReads {
    fn visit_stmt(&mut self, stmt: &'a Stmt) {
        if !matches!(stmt, Stmt::FunctionDef(_) | Stmt::ClassDef(_)) {
            visitor::walk_stmt(self, stmt);
        }
    }

    fn visit_expr(&mut self, expr: &'a Expr) {
        if let Expr::Name(name) = expr
            && name.ctx == ExprContext::Load
        {
            self.0.insert(name.id.to_string());
        }
        visitor::walk_expr(self, expr);
    }
}

/// What the program does with each candidate attribute.
struct Uses<'c> {
    attrs: &'c HashSet<String>,
    candidates: &'c HashMap<(String, String), usize>,
    /// Whether each enclosing scope is a function (`true`) or a class body.
    scopes: Vec<bool>,
    /// Attributes some access refuses the rewrite for.
    refused: HashSet<String>,
    /// Candidates the program assigns through their class.
    stored: HashSet<(String, String)>,
    /// Every name the program mentions (to keep the variables fresh).
    names: HashSet<String>,
}

impl Uses<'_> {
    fn candidate(&self, expr: &Expr) -> Option<(String, String)> {
        let Expr::Attribute(attribute) = expr else {
            return None;
        };
        let Expr::Name(base) = &*attribute.value else {
            return None;
        };
        let key = (base.id.to_string(), attribute.attr.to_string());
        self.candidates.contains_key(&key).then_some(key)
    }

    fn store(&mut self, target: &Expr) {
        if let Some(key) = self.candidate(target) {
            if self.scopes.last() == Some(&false) {
                self.refused.insert(key.1.clone());
            }
            self.stored.insert(key);
        }
    }
}

impl<'a> Visitor<'a> for Uses<'_> {
    fn visit_stmt(&mut self, stmt: &'a Stmt) {
        match stmt {
            Stmt::FunctionDef(_) | Stmt::ClassDef(_) => {
                self.scopes.push(matches!(stmt, Stmt::FunctionDef(_)));
                visitor::walk_stmt(self, stmt);
                self.scopes.pop();
                return;
            }
            Stmt::Assign(a) => a.targets.iter().for_each(|t| self.store(t)),
            Stmt::AugAssign(a) => self.store(&a.target),
            Stmt::AnnAssign(a) => self.store(&a.target),
            _ => {}
        }
        visitor::walk_stmt(self, stmt);
    }

    fn visit_expr(&mut self, expr: &'a Expr) {
        match expr {
            Expr::Name(name) => {
                self.names.insert(name.id.to_string());
            }
            Expr::Call(call) => {
                if let Expr::Attribute(attribute) = &*call.func
                    && self.attrs.contains(attribute.attr.as_str())
                {
                    if let Some(key) = self.candidate(&call.func) {
                        self.refused.insert(key.1);
                    }
                    // A method of another type: only its receiver is a use.
                    self.visit_expr(&attribute.value);
                    for arg in &call.arguments.args {
                        self.visit_expr(arg);
                    }
                    for keyword in &call.arguments.keywords {
                        self.visit_expr(&keyword.value);
                    }
                    return;
                }
            }
            Expr::Attribute(attribute) if self.attrs.contains(attribute.attr.as_str()) => {
                match self.candidate(expr) {
                    Some(key) if attribute.ctx == ExprContext::Del => {
                        self.refused.insert(key.1);
                    }
                    Some(_) => {}
                    None => {
                        self.refused.insert(attribute.attr.to_string());
                    }
                }
            }
            _ => {}
        }
        visitor::walk_expr(self, expr);
    }
}

/// Rewrites `C.X` to `C.X`'s module variable.
struct Rename<'r> {
    rewritten: &'r HashSet<(String, String)>,
}

impl Transformer for Rename<'_> {
    fn visit_expr(&self, expr: &mut Expr) {
        if let Expr::Attribute(attribute) = expr
            && let Expr::Name(base) = &*attribute.value
            && self
                .rewritten
                .contains(&(base.id.to_string(), attribute.attr.to_string()))
        {
            let name = variable(base.id.as_str(), attribute.attr.as_str());
            *expr = match attribute.ctx {
                ExprContext::Store => store(&name, attribute.range),
                _ => load(&name, attribute.range),
            };
            return;
        }
        transformer::walk_expr(self, expr);
    }
}

/// Declares each of `variables` `global` in every function that assigns it.
fn declare_globals(body: &mut [Stmt], variables: &HashSet<String>) {
    for stmt in body.iter_mut() {
        match stmt {
            Stmt::FunctionDef(function) => {
                let mut assigned = HashSet::new();
                scope_assignments(&function.body, &mut assigned);
                let mut names: Vec<&String> = assigned.intersection(variables).collect();
                names.sort();
                if !names.is_empty() {
                    let range = function.range;
                    let global = Stmt::Global(StmtGlobal {
                        node_index: AtomicNodeIndex::default(),
                        range,
                        names: names.iter().map(|n| Identifier::new(n.as_str(), range)).collect(),
                    });
                    let at = usize::from(matches!(function.body.first(), Some(Stmt::Expr(e))
                        if e.value.is_string_literal_expr()));
                    function.body.insert(at, global);
                }
                declare_globals(&mut function.body, variables);
            }
            Stmt::ClassDef(class) => declare_globals(&mut class.body, variables),
            _ => for_each_nested_body(stmt, &mut |inner| declare_globals(inner, variables)),
        }
    }
}

/// The names a scope's own statements assign (not those of nested scopes).
fn scope_assignments(body: &[Stmt], out: &mut HashSet<String>) {
    for stmt in body {
        match stmt {
            Stmt::Assign(a) => a.targets.iter().for_each(|t| target_names(t, out)),
            Stmt::AugAssign(a) => target_names(&a.target, out),
            Stmt::AnnAssign(a) => target_names(&a.target, out),
            Stmt::FunctionDef(_) | Stmt::ClassDef(_) => {}
            _ => {
                let mut nested: Vec<Vec<Stmt>> = Vec::new();
                let mut clone = stmt.clone();
                for_each_nested_body(&mut clone, &mut |inner| nested.push(inner.to_vec()));
                for inner in &nested {
                    scope_assignments(inner, out);
                }
            }
        }
    }
}

/// Calls `visit` on each statement body nested in `stmt` that belongs to
/// the same scope (`if`/`for`/`while`/`with`/`try`/`match` bodies).
fn for_each_nested_body(stmt: &mut Stmt, visit: &mut dyn FnMut(&mut [Stmt])) {
    match stmt {
        Stmt::If(s) => {
            visit(&mut s.body);
            for clause in &mut s.elif_else_clauses {
                visit(&mut clause.body);
            }
        }
        Stmt::For(s) => {
            visit(&mut s.body);
            visit(&mut s.orelse);
        }
        Stmt::While(s) => {
            visit(&mut s.body);
            visit(&mut s.orelse);
        }
        Stmt::With(s) => visit(&mut s.body),
        Stmt::Try(s) => {
            visit(&mut s.body);
            for handler in &mut s.handlers {
                let ruff_python_ast::ExceptHandler::ExceptHandler(h) = handler;
                visit(&mut h.body);
            }
            visit(&mut s.orelse);
            visit(&mut s.finalbody);
        }
        Stmt::Match(s) => {
            for case in &mut s.cases {
                visit(&mut case.body);
            }
        }
        _ => {}
    }
}

/// `_pycc_cattr_C_X[: T] = v` for the class body's binding of `attr`.
fn module_binding(class_body: &[Stmt], class: &str, attr: &str) -> Stmt {
    let name = variable(class, attr);
    for stmt in class_body {
        match stmt {
            Stmt::Assign(a)
                if matches!(&a.targets[..], [Expr::Name(n)] if n.id.as_str() == attr) =>
            {
                return assign(store(&name, a.range), (*a.value).clone(), a.range);
            }
            Stmt::AnnAssign(a) if matches!(&*a.target, Expr::Name(n) if n.id.as_str() == attr) => {
                let value = a.value.as_deref().expect("a candidate binding has a value");
                return ann_assign(
                    store(&name, a.range),
                    (*a.annotation).clone(),
                    value.clone(),
                    a.range,
                );
            }
            _ => {}
        }
    }
    unreachable!("a candidate attribute is bound in its class body")
}
