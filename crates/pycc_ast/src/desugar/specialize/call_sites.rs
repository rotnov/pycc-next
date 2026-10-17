//! What the module's own calls of a function or method say about the types
//! of its parameters: an argument that is certainly a `bytearray` -- a
//! `bytearray(...)` call, or a local of the calling function bound only to
//! such calls -- is direct evidence for the parameter it binds, which the
//! callee's body may lack (`raw[i]` and `raw[i:j]` are what a `str` does
//! too; `_deinterlace(self, raw)` gets its `raw` from
//! `bs = bytearray(...)`).
//!
//! Calls are matched by name: `f(...)` with a module function `f`, and
//! `<name>.m(...)` with every method `m` of any class (the receiver's class
//! is not known here). Only positional arguments count; the receiver of a
//! method call is not one. A parameter has call-site evidence only when the
//! module calls its function at least once and every call passes a
//! `bytearray` there; anything else (another argument, a keyword argument,
//! `*args`, a call through any other expression) leaves it none.

use ruff_python_ast::visitor::{self, Visitor};
use ruff_python_ast::{Expr, ModModule, Stmt};
use std::collections::{HashMap, HashSet};

/// What the calls of one callee (by name) pass at each positional index.
#[derive(Debug, Default, Clone)]
struct Calls {
    /// index -> whether every call so far passed a `bytearray` there.
    bytearray: HashMap<usize, bool>,
    /// The number of positional arguments of the shortest call: an index at
    /// or past it was not passed by some call.
    min_args: Option<usize>,
}

/// The module's calls, keyed by `("", f)` for a function and `(".", m)` for
/// a method.
#[derive(Debug, Default)]
pub(super) struct CallSites {
    calls: HashMap<(&'static str, String), Calls>,
}

impl CallSites {
    pub(super) fn of(module: &ModModule) -> Self {
        let mut sites = CallSites::default();
        scan_scope(&module.body, &mut sites);
        sites
    }

    /// Whether every call of the function (`method` false) or method named
    /// `name` passes a `bytearray` as positional argument `index` (counting
    /// a method's parameters after the receiver).
    pub(super) fn passes_bytearray(&self, method: bool, name: &str, index: usize) -> bool {
        let key = (if method { "." } else { "" }, name.to_string());
        self.calls.get(&key).is_some_and(|calls| {
            calls.min_args.is_some_and(|n| index < n)
                && calls.bytearray.get(&index).copied().unwrap_or(false)
        })
    }
}

/// Whether `expr` is a call of the builtin `bytearray`, directly or through
/// the fallback planner's builtins module ([`super::FALLBACK_BUILTINS`]).
fn is_bytearray_call(expr: &Expr) -> bool {
    let Expr::Call(call) = expr else {
        return false;
    };
    match call.func.as_ref() {
        Expr::Name(f) => f.id.as_str() == "bytearray",
        Expr::Attribute(attr) => {
            attr.attr.as_str() == "bytearray"
                && matches!(attr.value.as_ref(),
                    Expr::Name(m) if m.id.as_str() == super::FALLBACK_BUILTINS)
        }
        _ => false,
    }
}

/// The locals of the scope `body` bound only by `name = bytearray(...)`
/// (any other binding of a name, or none, leaves it out).
fn bytearray_locals(body: &[Stmt]) -> HashSet<String> {
    struct Collect {
        only: HashMap<String, bool>,
    }
    impl Collect {
        fn bind(&mut self, target: &Expr, is_bytearray: bool) {
            match target {
                Expr::Name(n) => {
                    let entry = self.only.entry(n.id.to_string()).or_insert(true);
                    *entry &= is_bytearray;
                }
                Expr::Tuple(t) => t.elts.iter().for_each(|e| self.bind(e, false)),
                Expr::List(l) => l.elts.iter().for_each(|e| self.bind(e, false)),
                Expr::Starred(s) => self.bind(&s.value, false),
                _ => {}
            }
        }
    }
    impl<'a> Visitor<'a> for Collect {
        fn visit_stmt(&mut self, stmt: &'a Stmt) {
            match stmt {
                // A nested scope's names are its own; its name is bound here.
                Stmt::FunctionDef(def) => {
                    self.only.insert(def.name.to_string(), false);
                    return;
                }
                Stmt::ClassDef(class) => {
                    self.only.insert(class.name.to_string(), false);
                    return;
                }
                Stmt::Assign(assign) => {
                    let is_bytearray = is_bytearray_call(&assign.value);
                    for target in &assign.targets {
                        self.bind(target, is_bytearray);
                    }
                }
                Stmt::AugAssign(aug) => self.bind(&aug.target, false),
                Stmt::AnnAssign(ann) => self.bind(&ann.target, false),
                Stmt::For(f) => self.bind(&f.target, false),
                Stmt::With(w) => {
                    for item in &w.items {
                        if let Some(var) = &item.optional_vars {
                            self.bind(var, false);
                        }
                    }
                }
                Stmt::Import(_) | Stmt::ImportFrom(_) | Stmt::Global(_) | Stmt::Nonlocal(_) => {
                    // Names bound or redirected another way.
                    for name in import_names(stmt) {
                        self.only.insert(name, false);
                    }
                }
                Stmt::Try(t) => {
                    for handler in &t.handlers {
                        let ruff_python_ast::ExceptHandler::ExceptHandler(h) = handler;
                        if let Some(name) = &h.name {
                            self.only.insert(name.to_string(), false);
                        }
                    }
                }
                Stmt::Delete(del) => del.targets.iter().for_each(|e| self.bind(e, false)),
                _ => {}
            }
            visitor::walk_stmt(self, stmt);
        }
        fn visit_expr(&mut self, expr: &'a Expr) {
            match expr {
                Expr::Lambda(_) => return,
                Expr::Named(named) => self.bind(&named.target, false),
                // A comprehension's targets are its own scope's.
                Expr::ListComp(_) | Expr::SetComp(_) | Expr::DictComp(_) | Expr::Generator(_) => {}
                _ => {}
            }
            visitor::walk_expr(self, expr);
        }
    }
    let mut collect = Collect {
        only: HashMap::new(),
    };
    for stmt in body {
        collect.visit_stmt(stmt);
    }
    collect
        .only
        .into_iter()
        .filter_map(|(name, only)| only.then_some(name))
        .collect()
}

fn import_names(stmt: &Stmt) -> Vec<String> {
    match stmt {
        Stmt::Import(import) => import
            .names
            .iter()
            .map(|alias| {
                alias
                    .asname
                    .as_ref()
                    .unwrap_or(&alias.name)
                    .as_str()
                    .split('.')
                    .next()
                    .unwrap_or_default()
                    .to_string()
            })
            .collect(),
        Stmt::ImportFrom(import) => import
            .names
            .iter()
            .map(|alias| alias.asname.as_ref().unwrap_or(&alias.name).to_string())
            .collect(),
        Stmt::Global(g) => g.names.iter().map(ToString::to_string).collect(),
        Stmt::Nonlocal(n) => n.names.iter().map(ToString::to_string).collect(),
        _ => Vec::new(),
    }
}

/// Records the calls of the scope `body` (and of the scopes nested in it).
fn scan_scope(body: &[Stmt], sites: &mut CallSites) {
    struct Scan<'a, 's> {
        locals: HashSet<String>,
        sites: &'s mut CallSites,
        nested: Vec<&'a [Stmt]>,
    }
    impl Scan<'_, '_> {
        fn record(&mut self, key: (&'static str, String), args: &[Expr]) {
            let calls = self.sites.calls.entry(key).or_default();
            let positional = args.iter().take_while(|a| !a.is_starred_expr()).count();
            let n = if positional == args.len() {
                positional
            } else {
                0
            };
            calls.min_args = Some(calls.min_args.map_or(n, |m| m.min(n)));
            for (index, arg) in args.iter().enumerate().take(n) {
                let is_bytearray = is_bytearray_call(arg)
                    || matches!(arg, Expr::Name(a) if self.locals.contains(a.id.as_str()));
                let entry = calls.bytearray.entry(index).or_insert(true);
                *entry &= is_bytearray;
            }
        }
    }
    impl<'a> Visitor<'a> for Scan<'a, '_> {
        fn visit_stmt(&mut self, stmt: &'a Stmt) {
            match stmt {
                Stmt::FunctionDef(def) => self.nested.push(&def.body),
                Stmt::ClassDef(class) => {
                    for item in &class.body {
                        self.visit_stmt(item);
                    }
                }
                _ => visitor::walk_stmt(self, stmt),
            }
        }
        fn visit_expr(&mut self, expr: &'a Expr) {
            if let Expr::Call(call) = expr {
                let key = match call.func.as_ref() {
                    Expr::Name(f) => Some(("", f.id.to_string())),
                    Expr::Attribute(attr) if attr.value.is_name_expr() => {
                        Some((".", attr.attr.to_string()))
                    }
                    _ => None,
                };
                if let Some(key) = key {
                    // A keyword argument may bind any parameter.
                    let args = if call.arguments.keywords.is_empty() {
                        &call.arguments.args[..]
                    } else {
                        &[]
                    };
                    self.record(key, args);
                }
            }
            visitor::walk_expr(self, expr);
        }
    }
    let mut scan = Scan {
        locals: bytearray_locals(body),
        sites,
        nested: Vec::new(),
    };
    for stmt in body {
        scan.visit_stmt(stmt);
    }
    let nested = std::mem::take(&mut scan.nested);
    for inner in nested {
        scan_scope(inner, scan.sites);
    }
}
