//! Dead unannotated definitions of a non-strict native program's entry
//! module (strict mode keeps checking them).
//!
//! A native build compiles a closed program: nothing outside the entry
//! module can call one of its functions except through a name the module
//! itself spells. A module-level `def` (or `name = lambda ...`) without any
//! annotation whose name nothing else mentions, and a method of a plain
//! class whose name no attribute access spells, can never run, and binding
//! it has no other effect -- so it is not compiled at all. Its signature
//! would otherwise have to be inferred from call sites that do not exist
//! (strict mode's `T0001`, non-strict mode's CPython fallback).
//!
//! The pass is conservative: it does nothing in a module that spells any
//! name able to reach a definition by string (`globals`, `getattr`,
//! `eval`, ...), it keeps every annotated or decorated definition, dunder
//! methods and the few non-dunder method names CPython calls implicitly,
//! and a method of a class with a base or a decorator. It repeats until
//! nothing changes, since a dead function may be the only user of another.

use ruff_python_ast::visitor::{Visitor, walk_expr, walk_stmt};
use ruff_python_ast::{Expr, ModModule, Stmt};
use std::collections::HashMap;

/// Names whose presence means a definition may be reached by a string.
const REFLECTIVE: &[&str] = &[
    "globals",
    "locals",
    "vars",
    "eval",
    "exec",
    "compile",
    "getattr",
    "hasattr",
    "setattr",
    "delattr",
    "dir",
    "__import__",
    "__main__",
    "inspect",
    "pickle",
    "operator",
];

/// Non-dunder method names CPython code calls on an object it is given.
const IMPLICIT_METHODS: &[&str] = &["keys", "read", "readline", "write", "flush", "fileno", "close"];

/// Drops the entry module's dead unannotated definitions (module doc).
pub fn drop_unused_defs(module: &mut ModModule) {
    let mut body: Vec<Stmt> = std::mem::take(&mut module.body).into_iter().collect();
    let mut counted = Counted::new(&body);
    if !counted.all.reflective {
        while drop_one(&mut body, &mut counted) {}
    }
    module.body = body.into();
}

/// The mentions of the module body, kept in step with it as definitions
/// are dropped: each statement's and each class method's are counted once,
/// and a drop subtracts what it removes from the total, where recounting
/// the whole module for every drop made the pass quadratic in the module
/// (v0.5 accept 4).
struct Counted {
    /// The whole body's.
    all: Mentions,
    /// Each top-level statement's, in body order.
    own: Vec<Mentions>,
    /// For each top-level statement, each of its class body's statements'
    /// (empty for a statement that is not a class).
    methods: Vec<Vec<Mentions>>,
}

impl Counted {
    fn new(body: &[Stmt]) -> Self {
        let own: Vec<Mentions> = body.iter().map(|stmt| mentions(std::iter::once(stmt))).collect();
        let methods = body
            .iter()
            .map(|stmt| match stmt {
                Stmt::ClassDef(class) => class
                    .body
                    .iter()
                    .map(|stmt| mentions(std::iter::once(stmt)))
                    .collect(),
                _ => Vec::new(),
            })
            .collect();
        let mut all = Mentions::default();
        for found in &own {
            all.add(found);
        }
        Self { all, own, methods }
    }
}

#[derive(Default)]
struct Mentions {
    names: HashMap<String, usize>,
    attrs: HashMap<String, usize>,
    reflective: bool,
}

impl Mentions {
    fn name(&mut self, name: &str) {
        if REFLECTIVE.contains(&name) {
            self.reflective = true;
        }
        *self.names.entry(name.to_string()).or_default() += 1;
    }

    fn attr(&mut self, name: &str) {
        *self.attrs.entry(name.to_string()).or_default() += 1;
    }

    /// Adds `other`'s counts (and reflection) to these.
    fn add(&mut self, other: &Mentions) {
        for (name, n) in &other.names {
            *self.names.entry(name.clone()).or_default() += n;
        }
        for (name, n) in &other.attrs {
            *self.attrs.entry(name.clone()).or_default() += n;
        }
        self.reflective |= other.reflective;
    }

    /// Takes `other`'s counts, a part of these, out of these.
    fn subtract(&mut self, other: &Mentions) {
        for (name, n) in &other.names {
            if let Some(count) = self.names.get_mut(name) {
                *count -= n;
            }
        }
        for (name, n) in &other.attrs {
            if let Some(count) = self.attrs.get_mut(name) {
                *count -= n;
            }
        }
    }

    /// The mentions of a fallback rewrite's `exec`uted source
    /// (`builtins.compile("<source>", ...)`): CPython code there reads
    /// compiled definitions by name.
    fn fallback_source(&mut self, source: &str) {
        match ruff_python_parser::parse_module(source) {
            Ok(parsed) => {
                for stmt in &parsed.syntax().body {
                    self.visit_stmt(stmt);
                }
            }
            Err(_) => self.reflective = true,
        }
    }
}

/// The fallback rewrite's namespace dictionary, `builtins` and `sys`
/// (`src/fallback.rs`).
const FALLBACK_NS: &str = "__pycc_fallback_ns";
const FALLBACK_BUILTINS: &str = "__pycc_fallback_builtins";
const FALLBACK_SYS: &str = "__pycc_fallback_sys";

/// Whether `expr` is the rewrite preamble's
/// `__pycc_fallback_sys.modules[__name__]`.
fn is_fallback_module(expr: &Expr) -> bool {
    matches!(expr, Expr::Subscript(sub) if matches!(sub.value.as_ref(),
        Expr::Attribute(attr) if attr.attr.as_str() == "modules"
            && matches!(attr.value.as_ref(), Expr::Name(n) if n.id.as_str() == FALLBACK_SYS)))
}

impl<'a> Visitor<'a> for Mentions {
    fn visit_stmt(&mut self, stmt: &'a Stmt) {
        match stmt {
            Stmt::FunctionDef(def) => {
                self.name(def.name.as_str());
                for param in def.parameters.iter() {
                    self.name(param.name().as_str());
                }
            }
            Stmt::ClassDef(def) => self.name(def.name.as_str()),
            Stmt::Import(import) => {
                for alias in &import.names {
                    let bound = alias.asname.as_ref().unwrap_or(&alias.name);
                    self.name(bound.split('.').next().unwrap_or(""));
                    self.name(alias.name.as_str());
                }
            }
            Stmt::ImportFrom(import) => {
                if let Some(module) = &import.module {
                    self.name(module.as_str());
                }
                for alias in &import.names {
                    self.name(alias.name.as_str());
                    if let Some(asname) = &alias.asname {
                        self.name(asname.as_str());
                    }
                }
            }
            Stmt::Global(global) => global.names.iter().for_each(|n| self.name(n.as_str())),
            Stmt::Nonlocal(nonlocal) => nonlocal.names.iter().for_each(|n| self.name(n.as_str())),
            _ => {}
        }
        walk_stmt(self, stmt);
    }

    fn visit_expr(&mut self, expr: &'a Expr) {
        match expr {
            Expr::Name(name) => self.name(name.id.as_str()),
            Expr::Call(call)
                if matches!(call.func.as_ref(), Expr::Attribute(attr)
                    if matches!(attr.value.as_ref(), Expr::Name(n) if n.id.as_str() == FALLBACK_BUILTINS)) =>
            {
                if let Some(Expr::StringLiteral(source)) = call.arguments.args.first() {
                    self.fallback_source(source.value.to_str());
                }
                for arg in call.arguments.args.iter().skip(1) {
                    self.visit_expr(arg);
                }
                return;
            }
            Expr::Subscript(sub)
                if matches!(sub.value.as_ref(), Expr::Name(n) if n.id.as_str() == FALLBACK_NS) =>
            {
                match sub.slice.as_ref() {
                    Expr::StringLiteral(name) => self.name(name.value.to_str()),
                    _ => self.reflective = true,
                }
                return;
            }
            Expr::Attribute(attr) if attr.attr.as_str() == "__dict__" => {
                if !is_fallback_module(&attr.value) {
                    self.reflective = true;
                }
            }
            Expr::Attribute(attr) => self.attr(attr.attr.as_str()),
            _ => {}
        }
        walk_expr(self, expr);
    }
}

fn mentions<'a>(stmts: impl Iterator<Item = &'a Stmt>) -> Mentions {
    let mut found = Mentions::default();
    for stmt in stmts {
        found.visit_stmt(stmt);
    }
    found
}

/// Whether `def` carries no annotation and no decorator.
fn unannotated(def: &ruff_python_ast::StmtFunctionDef, skip_receiver: bool) -> bool {
    def.decorator_list.is_empty()
        && !def.is_async
        && def.returns.is_none()
        && def.type_params.is_none()
        && def
            .parameters
            .iter()
            .skip(usize::from(skip_receiver))
            .all(|param| param.annotation().is_none())
}

fn count(map: &HashMap<String, usize>, name: &str) -> usize {
    map.get(name).copied().unwrap_or(0)
}

/// Removes one dead definition; `false` when there is none.
fn drop_one(body: &mut Vec<Stmt>, counted: &mut Counted) -> bool {
    let all = &counted.all;
    for index in 0..body.len() {
        let own = &counted.own[index];
        match &body[index] {
            Stmt::FunctionDef(def) if unannotated(def, false) => {
                let name = def.name.as_str();
                if count(&all.names, name) == count(&own.names, name) {
                    remove_stmt(body, counted, index);
                    return true;
                }
            }
            Stmt::Assign(assign)
                if matches!(assign.value.as_ref(), Expr::Lambda(_))
                    && let [Expr::Name(target)] = assign.targets.as_slice() =>
            {
                let name = target.id.as_str();
                if count(&all.names, name) == count(&own.names, name) {
                    remove_stmt(body, counted, index);
                    return true;
                }
            }
            Stmt::ClassDef(class) if class.decorator_list.is_empty() && class.arguments.as_ref().is_none_or(|a| a.is_empty()) => {
                for (position, stmt) in class.body.iter().enumerate() {
                    let Stmt::FunctionDef(def) = stmt else { continue };
                    let name = def.name.as_str();
                    if name.starts_with("__") || IMPLICIT_METHODS.contains(&name) || !unannotated(def, true) {
                        continue;
                    }
                    let method = &counted.methods[index][position];
                    let other_attrs = count(&all.attrs, name) - count(&method.attrs, name);
                    // The `def` itself is one name mention.
                    let other_names = count(&all.names, name) - count(&method.names, name);
                    if other_attrs == 0 && other_names == 0 {
                        let method = counted.methods[index].remove(position);
                        counted.all.subtract(&method);
                        counted.own[index].subtract(&method);
                        let Stmt::ClassDef(class) = &mut body[index] else { unreachable!() };
                        let range = ruff_text_size::Ranged::range(&class.body[position]);
                        class.body.remove(position);
                        if class.body.is_empty() {
                            let pass = Stmt::Pass(ruff_python_ast::StmtPass {
                                range,
                                node_index: ruff_python_ast::AtomicNodeIndex::default(),
                            });
                            counted.methods[index].push(mentions(std::iter::once(&pass)));
                            class.body.push(pass);
                        }
                        return true;
                    }
                }
            }
            _ => {}
        }
    }
    false
}

/// Removes top-level statement `index` and its counts.
fn remove_stmt(body: &mut Vec<Stmt>, counted: &mut Counted, index: usize) {
    body.remove(index);
    let own = counted.own.remove(index);
    counted.methods.remove(index);
    counted.all.subtract(&own);
}
