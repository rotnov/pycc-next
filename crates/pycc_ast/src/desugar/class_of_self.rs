//! `self.__class__(...)` in a method is `type(self)(...)` (v0.8).
//!
//! `child = self.__class__(self)` is how library code constructs "another
//! one of these" (mistune's `BlockState.child_state`, `InlineState.copy`).
//! pycc constructs the receiver's class with `type(self)(...)` inside a
//! method, but had no meaning for the attribute `__class__` of an instance
//! (`T0044`), so such a method sent its whole class to CPython. Both spell
//! the same call in CPython -- an instance's `__class__` is its type unless
//! the class overrides `__class__`, which a class pycc compiles cannot -- so
//! the call is rewritten to the form pycc compiles. Only the call is: a bare
//! `self.__class__` read (`self.__class__.__name__`) stays as written.
//!
//! The rewrite applies inside every undecorated method of a class whose
//! receiver the method never rebinds, not inside a nested function, lambda
//! or class (whose `self` may be another object), and not when the module
//! binds the name `type` itself.

use super::aliases::rebound_names;
use super::build::{call, load};
use ruff_python_ast::visitor::transformer::{self, Transformer};
use ruff_python_ast::{Expr, ExprContext, Stmt};

/// Rewrites the methods of every class of `body` (module doc).
pub(crate) fn class_of_self(body: &mut [Stmt]) {
    if rebound_names(body).contains_key("type") {
        return;
    }
    for stmt in body.iter_mut() {
        let Stmt::ClassDef(class) = stmt else {
            continue;
        };
        for item in class.body.iter_mut() {
            let Stmt::FunctionDef(method) = item else {
                continue;
            };
            if !method.decorator_list.is_empty() {
                continue;
            }
            let Some(receiver) = method
                .parameters
                .posonlyargs
                .iter()
                .chain(&method.parameters.args)
                .next()
                .map(|p| p.parameter.name.to_string())
            else {
                continue;
            };
            if super::stored_names(&method.body).contains(&receiver) {
                continue;
            }
            let rewrite = Rewrite { receiver: &receiver };
            for stmt in method.body.iter_mut() {
                rewrite.visit_stmt(stmt);
            }
        }
    }
}

struct Rewrite<'a> {
    receiver: &'a str,
}

impl Transformer for Rewrite<'_> {
    fn visit_stmt(&self, stmt: &mut Stmt) {
        if matches!(stmt, Stmt::FunctionDef(_) | Stmt::ClassDef(_)) {
            return;
        }
        transformer::walk_stmt(self, stmt);
    }

    fn visit_expr(&self, expr: &mut Expr) {
        if matches!(expr, Expr::Lambda(_)) {
            return;
        }
        if let Expr::Call(called) = expr
            && let Expr::Attribute(attr) = &*called.func
            && attr.ctx == ExprContext::Load
            && attr.attr.as_str() == "__class__"
            && matches!(&*attr.value, Expr::Name(n) if n.id.as_str() == self.receiver)
        {
            let range = attr.range;
            *called.func = call(load("type", range), vec![load(self.receiver, range)], range);
        }
        transformer::walk_expr(self, expr);
    }
}
