//! Nested functions used as values: which may be, and the expression that
//! makes the value.
//!
//! A nested function read as a value (`return add`, `apply(add, 3)`)
//! becomes `__pycc_closure__(lifted, captures...)`: the lifted function
//! with its captured enclosing variables bound when the value is made
//! (`pycc_hir::CLOSURE_MARKER`). CPython's closure reads each variable when
//! it *runs*, so binding it early is the same only when the variable cannot
//! change after the value is made: a parameter the enclosing function never
//! rebinds, or a variable one top-level statement before the `def` assigns
//! and nothing else does. A variable a nested function writes (`nonlocal`)
//! is captured as its cell, which both sides share. A function capturing
//! anything else, with parameter defaults, or with an unannotated
//! parameter (a function value's type is its annotations) is not used as
//! a value: it stays nested, and the compiler reports it as before.

use ruff_python_ast::visitor::{self, Visitor};
use ruff_python_ast::{Expr, ExprContext, Stmt, StmtFunctionDef};
use std::collections::HashSet;

/// Whether the nested function `def` may be made a value: its own shape.
pub(super) fn value_shape(def: &StmtFunctionDef) -> bool {
    def.parameters
        .args
        .iter()
        .all(|p| p.default.is_none() && p.parameter.annotation.is_some())
}

/// Whether every one of `captures` keeps one value from before the
/// top-level statement `def_index` of `body` (the enclosing function's
/// body, whose parameters are `params`) on.
pub(super) fn stable_captures(
    body: &[Stmt],
    params: &HashSet<String>,
    def_index: usize,
    captures: &[String],
) -> bool {
    captures.iter().all(|capture| {
        let bindings = own_bindings(body, capture);
        if params.contains(capture) {
            return bindings == 0;
        }
        bindings == 1
            && body[..def_index].iter().any(|stmt| match stmt {
                Stmt::Assign(a) => {
                    a.targets.len() == 1
                        && matches!(&a.targets[0], Expr::Name(n) if n.id.as_str() == capture)
                }
                Stmt::AnnAssign(a) => {
                    a.value.is_some()
                        && matches!(a.target.as_ref(), Expr::Name(n) if n.id.as_str() == capture)
                }
                _ => false,
            })
    })
}

/// The index of the top-level statement of `body` defining `name`, if one
/// does.
pub(super) fn def_index(body: &[Stmt], name: &str) -> Option<usize> {
    body.iter()
        .position(|stmt| matches!(stmt, Stmt::FunctionDef(def) if def.name.as_str() == name))
}

/// How many times `body` binds `name` in its own scope: nested functions,
/// classes and lambdas bind their own names (a nested function's
/// `nonlocal` write goes through the variable's cell). An import or a
/// `global`/`nonlocal` declaration of it counts as rebinding it.
fn own_bindings(body: &[Stmt], name: &str) -> usize {
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
                }
                Stmt::ClassDef(def) => {
                    if def.name.as_str() == self.name {
                        self.count += 1;
                    }
                }
                Stmt::Global(g) if g.names.iter().any(|n| n.as_str() == self.name) => {
                    self.count += 2;
                }
                Stmt::Nonlocal(g) if g.names.iter().any(|n| n.as_str() == self.name) => {
                    self.count += 2;
                }
                Stmt::Import(import)
                    if import.names.iter().any(|a| {
                        a.asname
                            .as_ref()
                            .unwrap_or(&a.name)
                            .as_str()
                            .split('.')
                            .next()
                            == Some(self.name)
                    }) =>
                {
                    self.count += 2;
                }
                Stmt::ImportFrom(import)
                    if import.names.iter().any(|a| {
                        a.name.as_str() == "*"
                            || a.asname.as_ref().unwrap_or(&a.name).as_str() == self.name
                    }) =>
                {
                    self.count += 2;
                }
                _ => visitor::walk_stmt(self, stmt),
            }
        }
        fn visit_expr(&mut self, expr: &'a Expr) {
            match expr {
                Expr::Lambda(_) => {}
                Expr::Name(n) if n.id.as_str() == self.name && n.ctx != ExprContext::Load => {
                    self.count += 1;
                }
                _ => visitor::walk_expr(self, expr),
            }
        }
    }
    let mut count = Count { name, count: 0 };
    for stmt in body {
        count.visit_stmt(stmt);
    }
    count.count
}
