//! An `--ext` module's public generator functions, split so the export
//! crosses to CPython as an iterator:
//!
//! ```python
//! def count(n: int) -> Iterator[int]:   # public, exported
//!     ...
//! # ->
//! def _pycc_gen_count(n: int) -> Iterator[int]:
//!     ...
//! def count(n: int) -> object:
//!     return _pycc_gen_count(n)
//! ```
//!
//! The export returns the native generator boxed into a CPython iterator
//! that shares it (`pycc_codegen`'s `generator_box`); calls inside the
//! module use the native generator directly. Only a module-level function
//! whose name the module binds nowhere else is split, with no decorator and
//! no `*args`/`**kwargs`; HIR lowering refuses the rest at the boundary as
//! before. Public generator methods are split the same way, without the
//! call-site renaming (a call through an instance reaches the wrapper and
//! iterates the CPython iterator).

use super::build::{attribute, call_kw, load};
use ruff_python_ast::visitor::transformer::{self, Transformer};
use ruff_python_ast::visitor::{self, Visitor};
use ruff_python_ast::{
    AtomicNodeIndex, Expr, ExprContext, Identifier, Keyword, ModModule, Stmt, StmtFunctionDef,
    StmtReturn,
};
use std::collections::HashSet;

const PREFIX: &str = "_pycc_gen_";

/// The split module, or `None` when `module` has no public generator
/// function or method to split.
pub fn split_ext_generators(module: &ModModule) -> Option<ModModule> {
    let mut module = module.clone();
    let mut renamed = HashSet::new();
    let mut changed = false;
    let old = std::mem::take(&mut module.body);
    for stmt in old {
        match stmt {
            Stmt::FunctionDef(def) if splittable(&def) => {
                renamed.insert(def.name.to_string());
                let (private, wrapper) = split(def, None);
                module.body.push(Stmt::FunctionDef(private));
                module.body.push(Stmt::FunctionDef(wrapper));
                changed = true;
            }
            Stmt::ClassDef(mut class) if is_public(class.name.as_str()) => {
                let old = std::mem::take(&mut class.body);
                for item in old {
                    match item {
                        Stmt::FunctionDef(def) if splittable(&def) => {
                            let (private, wrapper) = split(def, Some("self"));
                            class.body.push(Stmt::FunctionDef(private));
                            class.body.push(Stmt::FunctionDef(wrapper));
                            changed = true;
                        }
                        other => class.body.push(other),
                    }
                }
                module.body.push(Stmt::ClassDef(class));
            }
            other => module.body.push(other),
        }
    }
    if !changed {
        return None;
    }
    // A function bound more than once keeps its exported name everywhere;
    // the others' in-module calls go to the native generator.
    let renamed: HashSet<String> = renamed
        .into_iter()
        .filter(|name| bindings(&module.body, name) == 1)
        .collect();
    let rename = Rename { names: &renamed };
    for stmt in module.body.iter_mut() {
        if let Stmt::FunctionDef(def) = stmt
            && renamed.contains(def.name.as_str())
        {
            continue;
        }
        rename.visit_stmt(stmt);
    }
    Some(module)
}

fn is_public(name: &str) -> bool {
    !name.starts_with('_')
}

/// A public, undecorated generator `def` with plain parameters.
fn splittable(def: &StmtFunctionDef) -> bool {
    let p = &def.parameters;
    is_public(def.name.as_str())
        && def.decorator_list.is_empty()
        && !def.is_async
        && p.vararg.is_none()
        && p.kwarg.is_none()
        && yields(&def.body)
}

/// Whether `body` contains a `yield` of its own (not of a nested scope).
pub(crate) fn yields(body: &[Stmt]) -> bool {
    struct Find(bool);
    impl<'a> Visitor<'a> for Find {
        fn visit_stmt(&mut self, stmt: &'a Stmt) {
            match stmt {
                Stmt::FunctionDef(_) | Stmt::ClassDef(_) => {}
                _ => visitor::walk_stmt(self, stmt),
            }
        }
        fn visit_expr(&mut self, expr: &'a Expr) {
            match expr {
                Expr::Yield(_) | Expr::YieldFrom(_) => self.0 = true,
                Expr::Lambda(_)
                | Expr::Generator(_)
                | Expr::ListComp(_)
                | Expr::SetComp(_)
                | Expr::DictComp(_) => {}
                _ => visitor::walk_expr(self, expr),
            }
        }
    }
    let mut find = Find(false);
    for stmt in body {
        find.visit_stmt(stmt);
    }
    find.0
}

/// The private generator and the exported wrapper calling it (through
/// `receiver` for a method).
fn split(def: StmtFunctionDef, receiver: Option<&str>) -> (StmtFunctionDef, StmtFunctionDef) {
    let range = def.range;
    let private_name = format!("{PREFIX}{}", def.name);
    let p = &def.parameters;
    let mut positional: Vec<String> = p
        .posonlyargs
        .iter()
        .chain(p.args.iter())
        .map(|a| a.parameter.name.to_string())
        .collect();
    let callee = match receiver {
        Some(receiver) if positional.first().map(String::as_str) == Some(receiver) => {
            positional.remove(0);
            attribute(load(receiver, range), &private_name, range)
        }
        Some(_) => attribute(load("self", range), &private_name, range),
        None => load(&private_name, range),
    };
    let keywords: Vec<Keyword> = p
        .kwonlyargs
        .iter()
        .map(|a| Keyword {
            node_index: AtomicNodeIndex::default(),
            range,
            arg: Some(Identifier::new(a.parameter.name.as_str(), range)),
            value: load(a.parameter.name.as_str(), range),
        })
        .collect();
    let forwarded = call_kw(
        callee,
        positional.iter().map(|n| load(n, range)).collect(),
        keywords,
        range,
    );
    let mut wrapper = def.clone();
    wrapper.returns = Some(Box::new(super::build::name(
        "object",
        ExprContext::Load,
        range,
    )));
    wrapper.body = vec![Stmt::Return(StmtReturn {
        node_index: AtomicNodeIndex::default(),
        range,
        value: Some(Box::new(forwarded)),
    })]
    .into();
    let mut private = def;
    private.name = Identifier::new(private_name, range);
    (private, wrapper)
}

/// How many times `body` binds `name` at any depth (a `def`, a class, an
/// assignment target, a parameter).
fn bindings(body: &[Stmt], name: &str) -> usize {
    struct Count<'n> {
        name: &'n str,
        count: usize,
    }
    impl<'a> Visitor<'a> for Count<'_> {
        fn visit_stmt(&mut self, stmt: &'a Stmt) {
            match stmt {
                Stmt::FunctionDef(def) => {
                    if def.name.as_str() == self.name {
                        self.count += 1;
                    }
                    if def
                        .parameters
                        .iter()
                        .any(|p| p.name().as_str() == self.name)
                    {
                        self.count += 1;
                    }
                }
                Stmt::ClassDef(def) if def.name.as_str() == self.name => self.count += 1,
                Stmt::Import(import) => {
                    self.count += import
                        .names
                        .iter()
                        .filter(|a| {
                            a.asname
                                .as_ref()
                                .unwrap_or(&a.name)
                                .as_str()
                                .split('.')
                                .next()
                                == Some(self.name)
                        })
                        .count()
                }
                Stmt::ImportFrom(import) => {
                    self.count += import
                        .names
                        .iter()
                        .filter(|a| {
                            a.name.as_str() == "*"
                                || a.asname.as_ref().unwrap_or(&a.name).as_str() == self.name
                        })
                        .count()
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
                self.count += 1;
            }
            visitor::walk_expr(self, expr);
        }
    }
    let mut count = Count { name, count: 0 };
    for stmt in body {
        count.visit_stmt(stmt);
    }
    count.count
}

/// Renames reads of the split functions to their private generators.
struct Rename<'a> {
    names: &'a HashSet<String>,
}

impl Transformer for Rename<'_> {
    fn visit_expr(&self, expr: &mut Expr) {
        if let Expr::Name(n) = expr
            && n.ctx == ExprContext::Load
            && self.names.contains(n.id.as_str())
        {
            n.id = format!("{PREFIX}{}", n.id).into();
            return;
        }
        transformer::walk_expr(self, expr);
    }
}
