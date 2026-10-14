//! Module-level `if` statements whose test is known before the program
//! runs, replaced by the branch that runs.
//!
//! Competitive-programming templates choose their I/O setup with a flag
//! (`debug = 0` ... `if debug: sys.stdin = open("in.txt")`) or with
//! `sys.gettrace()` (set only under a debugger). The test of a top-level
//! `if` is known when it is
//!
//! * a name bound exactly once in the whole module, by an earlier
//!   top-level `name = <literal>` (`int`, `bool`, `str` or `None`), read
//!   for its truth value; or
//! * `gettrace()` / `sys.gettrace()` of the `sys` module, in a module that
//!   never names `settrace`: a compiled program runs no trace function, so
//!   it is `None`.
//!
//! The `if` then runs exactly the statements of the taken branch, at module
//! level, in the same scope -- which is what replacing it with them does. A
//! taken branch holding an `import` keeps its `if` (an import nested in a
//! block is resolved at run time, and refused as such).
//! The test itself has no side effect, so dropping it changes nothing else.

use super::aliases::rebound_names;
use ruff_python_ast::visitor::{self, Visitor};
use ruff_python_ast::{Expr, Number, Stmt};
use std::collections::HashMap;

/// Replaces each such `if` (module doc).
pub(crate) fn fold_static_branches(body: &mut Vec<Stmt>) {
    let mut rebound = rebound_names(body);
    // An import binds a name too (`rebound_names` leaves imports out).
    for name in imported_names(body) {
        *rebound.entry(name).or_default() += 1;
    }
    let gettrace = gettrace_names(body);
    let mut known: HashMap<String, bool> = HashMap::new();
    fold(body, &rebound, &gettrace, &mut known);
}

/// [`fold_static_branches`] over one top-level statement list (the module
/// body, or the statements of a folded branch).
fn fold(
    body: &mut Vec<Stmt>,
    rebound: &HashMap<String, usize>,
    gettrace: &Gettrace,
    known: &mut HashMap<String, bool>,
) {
    let mut out: Vec<Stmt> = Vec::with_capacity(body.len());
    for stmt in std::mem::take(body) {
        match stmt {
            Stmt::Assign(ref assign) => {
                if let ([Expr::Name(target)], Some(truth)) =
                    (assign.targets.as_slice(), literal_truth(&assign.value))
                    && rebound.get(target.id.as_str()) == Some(&1)
                {
                    known.insert(target.id.to_string(), truth);
                }
                out.push(stmt);
            }
            Stmt::If(if_stmt) => {
                let mut branches: Vec<(Option<Expr>, Vec<Stmt>)> =
                    vec![(Some(*if_stmt.test.clone()), if_stmt.body.to_vec())];
                branches.extend(
                    if_stmt
                        .elif_else_clauses
                        .iter()
                        .map(|clause| (clause.test.clone(), clause.body.to_vec())),
                );
                // The first branch whose test is known true, when every test
                // before it is known false; a test not known stops the fold.
                let mut taken: Option<Vec<Stmt>> = None;
                let mut folded = true;
                for (test, branch) in branches {
                    match test.map(|t| test_truth(&t, known, gettrace)) {
                        None | Some(Some(true)) => {
                            taken = Some(branch);
                            break;
                        }
                        Some(Some(false)) => {}
                        Some(None) => {
                            folded = false;
                            break;
                        }
                    }
                }
                // An import keeps its block: a nested import is resolved
                // (and refused) as one, at run time.
                let imports = taken.as_ref().is_some_and(|branch| {
                    branch
                        .iter()
                        .any(|stmt| matches!(stmt, Stmt::Import(_) | Stmt::ImportFrom(_)))
                });
                if folded && !imports {
                    let mut taken = taken.unwrap_or_default();
                    fold(&mut taken, rebound, gettrace, known);
                    // A `pass` does nothing at module level either.
                    out.extend(taken.into_iter().filter(|stmt| !matches!(stmt, Stmt::Pass(_))));
                } else {
                    out.push(Stmt::If(if_stmt));
                }
            }
            other => out.push(other),
        }
    }
    *body = out;
}

/// Every name an `import` statement anywhere in `body` binds.
fn imported_names(body: &[Stmt]) -> Vec<String> {
    struct Find(Vec<String>);
    impl<'a> Visitor<'a> for Find {
        fn visit_stmt(&mut self, stmt: &'a Stmt) {
            match stmt {
                Stmt::Import(import) => self.0.extend(import.names.iter().map(|alias| {
                    let bound = alias.asname.as_ref().unwrap_or(&alias.name).as_str();
                    bound.split('.').next().unwrap_or(bound).to_string()
                })),
                Stmt::ImportFrom(import) => self.0.extend(
                    import
                        .names
                        .iter()
                        .map(|alias| alias.asname.as_ref().unwrap_or(&alias.name).to_string()),
                ),
                _ => {}
            }
            visitor::walk_stmt(self, stmt);
        }
    }
    let mut find = Find(Vec::new());
    for stmt in body {
        find.visit_stmt(stmt);
    }
    find.0
}

/// The truth value of a literal, for `int`/`bool`/`str`/`None`.
fn literal_truth(expr: &Expr) -> Option<bool> {
    match expr {
        Expr::BooleanLiteral(b) => Some(b.value),
        Expr::NoneLiteral(_) => Some(false),
        Expr::NumberLiteral(n) => match &n.value {
            Number::Int(i) => Some(i.as_u64() != Some(0)),
            _ => None,
        },
        Expr::StringLiteral(s) => Some(!s.value.to_str().is_empty()),
        _ => None,
    }
}

/// The truth value of an `if` test known before the program runs.
fn test_truth(test: &Expr, known: &HashMap<String, bool>, gettrace: &Gettrace) -> Option<bool> {
    match test {
        Expr::Name(name) => known.get(name.id.as_str()).copied(),
        Expr::UnaryOp(u) if u.op == ruff_python_ast::UnaryOp::Not => {
            test_truth(&u.operand, known, gettrace).map(|t| !t)
        }
        Expr::Call(call)
            if call.arguments.args.is_empty()
                && call.arguments.keywords.is_empty()
                && gettrace.is_call(&call.func) =>
        {
            Some(false)
        }
        _ => None,
    }
}

/// How the module reaches `sys.gettrace`, when it never names `settrace`.
#[derive(Default)]
struct Gettrace {
    /// Names bound to the `sys` module by `import sys [as s]`.
    sys: Vec<String>,
    /// Names bound to `sys.gettrace` by `from sys import gettrace [as g]`.
    direct: Vec<String>,
}

impl Gettrace {
    fn is_call(&self, func: &Expr) -> bool {
        match func {
            Expr::Name(n) => self.direct.iter().any(|d| d == n.id.as_str()),
            Expr::Attribute(a) => {
                a.attr.as_str() == "gettrace"
                    && matches!(a.value.as_ref(), Expr::Name(n) if self.sys.iter().any(|s| s == n.id.as_str()))
            }
            _ => false,
        }
    }
}

fn gettrace_names(body: &[Stmt]) -> Gettrace {
    struct Settrace(bool);
    impl<'a> Visitor<'a> for Settrace {
        fn visit_expr(&mut self, expr: &'a Expr) {
            match expr {
                Expr::Name(n) if n.id.as_str() == "settrace" => self.0 = true,
                Expr::Attribute(a) if a.attr.as_str() == "settrace" => self.0 = true,
                _ => {}
            }
            visitor::walk_expr(self, expr);
        }
        fn visit_alias(&mut self, alias: &'a ruff_python_ast::Alias) {
            if alias.name.as_str() == "settrace" {
                self.0 = true;
            }
        }
    }
    let mut settrace = Settrace(false);
    for stmt in body {
        settrace.visit_stmt(stmt);
    }
    if settrace.0 {
        return Gettrace::default();
    }
    let rebound = rebound_names(body);
    let mut names = Gettrace::default();
    for stmt in body {
        match stmt {
            Stmt::Import(import) => {
                for alias in &import.names {
                    let bound = alias.asname.as_ref().unwrap_or(&alias.name).to_string();
                    if alias.name.as_str() == "sys" && !rebound.contains_key(&bound) {
                        names.sys.push(bound);
                    }
                }
            }
            Stmt::ImportFrom(import)
                if import.level == 0
                    && import.module.as_ref().is_some_and(|m| m.as_str() == "sys") =>
            {
                for alias in &import.names {
                    let bound = alias.asname.as_ref().unwrap_or(&alias.name).to_string();
                    if alias.name.as_str() == "gettrace" && !rebound.contains_key(&bound) {
                        names.direct.push(bound);
                    }
                }
            }
            _ => {}
        }
    }
    names
}
