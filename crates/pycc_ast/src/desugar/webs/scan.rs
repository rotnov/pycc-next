//! The free names of nested scopes: what a function, lambda or class body
//! may read or write in the scope enclosing it.

use super::parameter_names;
use ruff_python_ast::visitor::{self, Visitor};
use ruff_python_ast::{ExceptHandler, Expr, ExprContext, Stmt, StmtClassDef, StmtFunctionDef};
use std::collections::HashSet;

/// Whether `value` is never a container: a number, string or boolean
/// literal, an f-string, a comparison or `not`, or a call of a builtin
/// conversion to a scalar. A name the module rebinds is still read as the
/// builtin here; at worst a web is split that a reset would have joined,
/// which only loses an element type.
pub(super) fn is_scalar_value(value: &Expr) -> bool {
    match value {
        Expr::NumberLiteral(_)
        | Expr::StringLiteral(_)
        | Expr::BooleanLiteral(_)
        | Expr::FString(_)
        | Expr::Compare(_) => true,
        Expr::UnaryOp(unary) => {
            unary.op == ruff_python_ast::UnaryOp::Not || is_scalar_value(&unary.operand)
        }
        Expr::Call(call) => matches!(call.func.as_ref(), Expr::Name(name)
            if matches!(name.id.as_str(), "int" | "float" | "str" | "bool" | "ord" | "chr" | "len" | "abs" | "round")),
        _ => false,
    }
}

/// The names a nested function, lambda or class body may read or write in
/// an enclosing scope: everything it reads that it does not bind itself,
/// plus everything it declares `global` or `nonlocal`.
pub(super) fn free_names_of_def(def: &StmtFunctionDef) -> HashSet<String> {
    let mut local: HashSet<String> = parameter_names(&def.parameters).into_iter().collect();
    let mut scan = ScopeScan::default();
    for stmt in &def.body {
        scan.visit_stmt(stmt);
    }
    local.extend(scan.bound.iter().cloned());
    for name in &scan.declared {
        local.remove(name);
    }
    let mut free: HashSet<String> = scan
        .read
        .into_iter()
        .filter(|name| !local.contains(name))
        .collect();
    free.extend(scan.declared);
    free.extend(scan.nested_free);
    free
}

/// Every name a class body (and its methods) reads or declares.
pub(super) fn free_names_of_class(class: &StmtClassDef) -> HashSet<String> {
    let mut scan = ScopeScan::default();
    for stmt in &class.body {
        scan.visit_stmt(stmt);
    }
    let mut free: HashSet<String> = scan.read.into_iter().collect();
    free.extend(scan.bound);
    free.extend(scan.declared);
    free.extend(scan.nested_free);
    free
}

/// The reads, bindings and declarations of one scope, with each nested
/// scope folded in as its free names.
#[derive(Default)]
pub(super) struct ScopeScan {
    pub(super) read: HashSet<String>,
    pub(super) bound: HashSet<String>,
    pub(super) declared: HashSet<String>,
    pub(super) nested_free: HashSet<String>,
}

impl<'a> Visitor<'a> for ScopeScan {
    fn visit_stmt(&mut self, stmt: &'a Stmt) {
        match stmt {
            Stmt::FunctionDef(def) => {
                self.bound.insert(def.name.to_string());
                self.nested_free.extend(free_names_of_def(def));
                for decorator in &def.decorator_list {
                    self.visit_expr(&decorator.expression);
                }
                return;
            }
            Stmt::ClassDef(class) => {
                self.bound.insert(class.name.to_string());
                self.nested_free.extend(free_names_of_class(class));
                return;
            }
            Stmt::Global(global) => {
                self.declared
                    .extend(global.names.iter().map(|n| n.to_string()));
            }
            Stmt::Nonlocal(nonlocal) => {
                self.declared
                    .extend(nonlocal.names.iter().map(|n| n.to_string()));
            }
            Stmt::Import(import) => {
                for alias in &import.names {
                    let bound = alias.asname.as_ref().unwrap_or(&alias.name);
                    self.bound
                        .insert(bound.split('.').next().unwrap_or("").to_string());
                }
            }
            Stmt::ImportFrom(import) => {
                for alias in &import.names {
                    self.bound
                        .insert(alias.asname.as_ref().unwrap_or(&alias.name).to_string());
                }
            }
            Stmt::Try(try_stmt) => {
                for handler in &try_stmt.handlers {
                    let ExceptHandler::ExceptHandler(handler) = handler;
                    if let Some(name) = &handler.name {
                        self.bound.insert(name.to_string());
                    }
                }
            }
            _ => {}
        }
        visitor::walk_stmt(self, stmt);
    }

    fn visit_expr(&mut self, expr: &'a Expr) {
        match expr {
            Expr::Name(name) => {
                if name.ctx == ExprContext::Load {
                    self.read.insert(name.id.to_string());
                } else {
                    self.bound.insert(name.id.to_string());
                }
            }
            Expr::Lambda(lambda) => {
                let mut inner = ScopeScan::default();
                inner.visit_expr(&lambda.body);
                let params: HashSet<String> = lambda
                    .parameters
                    .iter()
                    .flat_map(|p| p.iter())
                    .map(|p| p.name().to_string())
                    .collect();
                if let Some(parameters) = &lambda.parameters {
                    for p in parameters.iter_non_variadic_params() {
                        if let Some(default) = &p.default {
                            self.visit_expr(default);
                        }
                    }
                }
                self.nested_free.extend(
                    inner
                        .read
                        .into_iter()
                        .chain(inner.nested_free)
                        .filter(|name| !params.contains(name)),
                );
                return;
            }
            _ => {}
        }
        visitor::walk_expr(self, expr);
    }
}
