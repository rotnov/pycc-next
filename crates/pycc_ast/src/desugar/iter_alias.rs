//! `name = iter(lines).__next__`: a module-level reader of a list's items.
//!
//! The idiom reads all of a list up front (`iter(sys.stdin.read().splitlines())`)
//! and binds the iterator's bound `__next__` as a line reader. A bound method
//! of an iterator is no value pycc compiles, so the statement becomes the
//! state the iterator holds -- the list and a position, both module globals
//! -- and a function `name()` that returns the next item or raises
//! `StopIteration` once the list is exhausted, exactly what the bound
//! method does. The list is evaluated where the statement stood.

use super::INFER_MARKER;
use super::aliases::rebound_names;
use super::build::load;
use ruff_python_ast::visitor::transformer::{self, Transformer};
use ruff_python_ast::{Expr, Stmt};
use ruff_text_size::{Ranged, TextRange};

/// Rewrites every such top-level statement of `body` (module doc).
pub(crate) fn iterator_readers(body: &mut Vec<Stmt>) {
    let rebound = rebound_names(body);
    if rebound.contains_key("iter") || rebound.contains_key("len") {
        return;
    }
    let mut out = Vec::with_capacity(body.len());
    for stmt in std::mem::take(body) {
        match reader(&stmt, &rebound) {
            Some(replacement) => out.extend(replacement),
            None => out.push(stmt),
        }
    }
    *body = out;
}

/// The replacement of `stmt` when it is `name = iter(<list>).__next__`.
fn reader(stmt: &Stmt, rebound: &std::collections::HashMap<String, usize>) -> Option<Vec<Stmt>> {
    let Stmt::Assign(assign) = stmt else {
        return None;
    };
    let [Expr::Name(target)] = assign.targets.as_slice() else {
        return None;
    };
    let Expr::Attribute(next) = assign.value.as_ref() else {
        return None;
    };
    let Expr::Call(iter_call) = next.value.as_ref() else {
        return None;
    };
    if next.attr.as_str() != "__next__"
        || !matches!(iter_call.func.as_ref(), Expr::Name(f) if f.id.as_str() == "iter")
        || !iter_call.arguments.keywords.is_empty()
        || rebound.get(target.id.as_str()).copied() != Some(1)
    {
        return None;
    }
    let [items] = &iter_call.arguments.args[..] else {
        return None;
    };
    if !is_list_producer(items) {
        return None;
    }
    let name = target.id.as_str();
    let source = format!(
        "__pycc_items_{name} = None\n\
         __pycc_pos_{name} = 0\n\
         def {name}():\n    \
             global __pycc_pos_{name}\n    \
             __pycc_i = __pycc_pos_{name}\n    \
             if __pycc_i >= len(__pycc_items_{name}):\n        \
                 raise StopIteration(\"\")\n    \
             __pycc_pos_{name} = __pycc_i + 1\n    \
             return __pycc_items_{name}[__pycc_i]\n"
    );
    let mut stmts: Vec<Stmt> = ruff_python_parser::parse_module(&source)
        .ok()?
        .into_syntax()
        .body
        .into_iter()
        .collect();
    let range = stmt.range();
    let at = Relocate(range);
    for stmt in &mut stmts {
        at.visit_stmt(stmt);
    }
    if let Stmt::Assign(first) = &mut stmts[0] {
        *first.value = items.clone();
    }
    if let Stmt::FunctionDef(def) = &mut stmts[2] {
        def.returns = Some(Box::new(load(INFER_MARKER, range)));
    }
    Some(stmts)
}

/// Whether `expr` builds a new list: a `str` method that returns one
/// (`text.splitlines()`, `line.split()`), a list display or comprehension.
fn is_list_producer(expr: &Expr) -> bool {
    match expr {
        Expr::Call(call) => matches!(call.func.as_ref(),
            Expr::Attribute(a) if matches!(a.attr.as_str(), "split" | "splitlines" | "readlines")),
        Expr::List(_) | Expr::ListComp(_) => true,
        _ => false,
    }
}

/// Moves every node of the synthesized statements onto the source range of
/// the statement they replace, so a diagnostic points at what was written.
struct Relocate(TextRange);

impl Transformer for Relocate {
    fn visit_stmt(&self, stmt: &mut Stmt) {
        match stmt {
            Stmt::Assign(s) => s.range = self.0,
            Stmt::AugAssign(s) => s.range = self.0,
            Stmt::FunctionDef(s) => {
                s.range = self.0;
                s.name.range = self.0;
                s.parameters.range = self.0;
            }
            Stmt::Global(s) => {
                s.range = self.0;
                s.names.iter_mut().for_each(|n| n.range = self.0);
            }
            Stmt::If(s) => s.range = self.0,
            Stmt::Raise(s) => s.range = self.0,
            Stmt::Return(s) => s.range = self.0,
            _ => {}
        }
        transformer::walk_stmt(self, stmt);
    }

    fn visit_expr(&self, expr: &mut Expr) {
        ruff_python_ast::relocate::relocate_expr(expr, self.0);
    }
}
