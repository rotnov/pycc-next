//! `heapq` and `bisect` calls on a list compile natively: a call of one of
//! their functions (through `import heapq`, `import heapq as hq` or
//! `from heapq import heappush [as hp]`, the name bound nowhere else) is
//! spelled as the `pycc_hir::builtins` name `__pycc_<function>`, which the
//! type checker admits for a list of ordered elements. An import left
//! unread is dropped, so a program using only these needs no embedded
//! CPython.

use super::aliases::rebound_names;
use ruff_python_ast::visitor::transformer::{self, Transformer};
use ruff_python_ast::visitor::{self, Visitor};
use ruff_python_ast::{Expr, ExprContext, Stmt};
use std::collections::{HashMap, HashSet};

/// The native builtin and arity of `module.function`.
fn native(module: &str, function: &str) -> Option<(&'static str, usize)> {
    Some(match (module, function) {
        ("heapq", "heappush") => ("__pycc_heappush", 2),
        ("heapq", "heappop") => ("__pycc_heappop", 1),
        ("heapq", "heapify") => ("__pycc_heapify", 1),
        ("heapq", "heapreplace") => ("__pycc_heapreplace", 2),
        ("heapq", "heappushpop") => ("__pycc_heappushpop", 2),
        ("bisect", "bisect_left") => ("__pycc_bisect_left", 2),
        ("bisect", "bisect_right" | "bisect") => ("__pycc_bisect_right", 2),
        ("bisect", "insort_left") => ("__pycc_insort_left", 2),
        ("bisect", "insort_right" | "insort") => ("__pycc_insort_right", 2),
        _ => return None,
    })
}

struct Rewrite {
    /// Local module name -> module.
    modules: HashMap<String, String>,
    /// Local function name -> (module, function).
    functions: HashMap<String, (String, String)>,
}

impl Rewrite {
    fn target<'s>(&'s self, func: &'s Expr) -> Option<(&'s str, &'s str)> {
        match func {
            Expr::Attribute(attr) => {
                let Expr::Name(receiver) = attr.value.as_ref() else {
                    return None;
                };
                let module = self.modules.get(receiver.id.as_str())?;
                Some((module.as_str(), attr.attr.as_str()))
            }
            Expr::Name(name) => {
                let (module, function) = self.functions.get(name.id.as_str())?;
                Some((module.as_str(), function.as_str()))
            }
            _ => None,
        }
    }
}

impl Transformer for Rewrite {
    fn visit_expr(&self, expr: &mut Expr) {
        if let Expr::Call(call) = expr
            && call.arguments.keywords.is_empty()
            && !call.arguments.args.iter().any(|a| matches!(a, Expr::Starred(_)))
            && let Some((module, function)) = self.target(&call.func)
            && let Some((builtin, arity)) = native(module, function)
            && arity == call.arguments.args.len()
        {
            let range = ruff_text_size::Ranged::range(call.func.as_ref());
            *call.func = super::build::load(builtin, range);
        }
        transformer::walk_expr(self, expr);
    }
}

fn read_names(body: &[Stmt]) -> HashSet<String> {
    struct Collect(HashSet<String>);
    impl<'a> Visitor<'a> for Collect {
        fn visit_expr(&mut self, expr: &'a Expr) {
            if let Expr::Name(name) = expr
                && name.ctx == ExprContext::Load
            {
                self.0.insert(name.id.to_string());
            }
            visitor::walk_expr(self, expr);
        }
    }
    let mut collect = Collect(HashSet::new());
    for stmt in body {
        collect.visit_stmt(stmt);
    }
    collect.0
}

/// Rewrites `heapq`/`bisect` calls in `body` (a module).
pub(crate) fn native_stdlib_calls(body: &mut Vec<Stmt>) {
    let rebound = rebound_names(body);
    let mut rewrite = Rewrite {
        modules: HashMap::new(),
        functions: HashMap::new(),
    };
    let mut seen: HashMap<String, usize> = HashMap::new();
    for stmt in body.iter() {
        match stmt {
            Stmt::Import(import) => {
                for alias in &import.names {
                    let local = alias.asname.as_ref().unwrap_or(&alias.name).to_string();
                    *seen.entry(local.clone()).or_default() += 1;
                    if matches!(alias.name.as_str(), "heapq" | "bisect") {
                        rewrite.modules.insert(local, alias.name.to_string());
                    }
                }
            }
            Stmt::ImportFrom(import) if import.level == 0 => {
                let module = import.module.as_ref().map(|m| m.as_str()).unwrap_or("");
                for alias in &import.names {
                    let local = alias.asname.as_ref().unwrap_or(&alias.name).to_string();
                    *seen.entry(local.clone()).or_default() += 1;
                    if native(module, alias.name.as_str()).is_some() {
                        rewrite
                            .functions
                            .insert(local, (module.to_string(), alias.name.to_string()));
                    }
                }
            }
            _ => {}
        }
    }
    // A name bound twice (or by anything but its import) keeps CPython's.
    let unique = |name: &String| seen.get(name) == Some(&1) && !rebound.contains_key(name);
    rewrite.modules.retain(|name, _| unique(name));
    rewrite.functions.retain(|name, _| unique(name));
    if rewrite.modules.is_empty() && rewrite.functions.is_empty() {
        return;
    }
    for stmt in body.iter_mut() {
        rewrite.visit_stmt(stmt);
    }
    let read = read_names(body);
    body.retain_mut(|stmt| match stmt {
        Stmt::Import(import) => {
            import.names.retain(|alias| {
                let local = alias.asname.as_ref().unwrap_or(&alias.name).to_string();
                !rewrite.modules.contains_key(&local) || read.contains(&local)
            });
            !import.names.is_empty()
        }
        Stmt::ImportFrom(import) => {
            import.names.retain(|alias| {
                let local = alias.asname.as_ref().unwrap_or(&alias.name).to_string();
                !rewrite.functions.contains_key(&local) || read.contains(&local)
            });
            !import.names.is_empty()
        }
        _ => true,
    });
}
