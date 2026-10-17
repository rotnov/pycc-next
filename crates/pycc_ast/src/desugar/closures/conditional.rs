//! A nested function defined more than once in its enclosing function --
//! in the branches of an `if`, say -- and only ever called directly:
//!
//! ```python
//! if wide:                                if wide:
//!     def new():                              def new_pycc_alt1():
//!         return array("H", [0])                  return array("H", [0])
//! else:                         ==>           __pycc_alt_new = 1
//!     def new():                          else:
//!         return bytearray([0])               def new_pycc_alt2():
//! row = new()                                     return bytearray([0])
//!                                             __pycc_alt_new = 2
//!                                         row = (new_pycc_alt1() if __pycc_alt_new == 1
//!                                                else new_pycc_alt2())
//! ```
//!
//! Each definition gets a name of its own and records which one ran; each
//! call (in the enclosing function and in its other nested functions)
//! calls the one that ran last, evaluating its arguments once. Each
//! definition is then bound once, so it lifts like any other nested
//! function ([`super::lift_function`]).

use super::count_uses;
use crate::desugar::build::{assign, int, load, store};
use ruff_python_ast::visitor::transformer::{self, Transformer};
use ruff_python_ast::{
    AtomicNodeIndex, CmpOp, Expr, ExprCompare, ExprIf, Identifier, Stmt, StmtFunctionDef,
};
use std::collections::HashMap;

/// The variable recording which definition of `name` ran.
fn selector(name: &str) -> String {
    format!("__pycc_alt_{name}")
}

/// The name of the `k`-th definition of `name` (from 1).
fn alternative(name: &str, k: usize) -> String {
    format!("{name}_pycc_alt{k}")
}

/// The names of the `def`s directly in `body` or in its blocks (not inside
/// a nested `def` or class), with how many times each is defined.
fn block_defs(body: &[Stmt], out: &mut HashMap<String, usize>) {
    for stmt in body {
        match stmt {
            Stmt::FunctionDef(def) => *out.entry(def.name.to_string()).or_default() += 1,
            Stmt::If(s) => {
                block_defs(&s.body, out);
                for clause in &s.elif_else_clauses {
                    block_defs(&clause.body, out);
                }
            }
            Stmt::While(s) => {
                block_defs(&s.body, out);
                block_defs(&s.orelse, out);
            }
            Stmt::For(s) => {
                block_defs(&s.body, out);
                block_defs(&s.orelse, out);
            }
            Stmt::With(s) => block_defs(&s.body, out),
            Stmt::Try(s) => {
                block_defs(&s.body, out);
                for handler in &s.handlers {
                    let ruff_python_ast::ExceptHandler::ExceptHandler(h) = handler;
                    block_defs(&h.body, out);
                }
                block_defs(&s.orelse, out);
                block_defs(&s.finalbody, out);
            }
            _ => {}
        }
    }
}

/// Renames the definitions of `name` in `body` and its blocks in order,
/// each followed by the selector's store.
fn rename_defs(body: &mut thin_vec::ThinVec<Stmt>, name: &str, next: &mut usize) {
    let old = std::mem::take(body);
    for mut stmt in old {
        let mut store_after = None;
        match &mut stmt {
            Stmt::FunctionDef(def) if def.name.as_str() == name => {
                *next += 1;
                def.name = Identifier::new(alternative(name, *next), def.name.range);
                store_after = Some((*next, def.range));
            }
            Stmt::If(s) => {
                rename_defs(&mut s.body, name, next);
                for clause in &mut s.elif_else_clauses {
                    rename_defs(&mut clause.body, name, next);
                }
            }
            Stmt::While(s) => {
                rename_defs(&mut s.body, name, next);
                rename_defs(&mut s.orelse, name, next);
            }
            Stmt::For(s) => {
                rename_defs(&mut s.body, name, next);
                rename_defs(&mut s.orelse, name, next);
            }
            Stmt::With(s) => rename_defs(&mut s.body, name, next),
            Stmt::Try(s) => {
                rename_defs(&mut s.body, name, next);
                for handler in &mut s.handlers {
                    let ruff_python_ast::ExceptHandler::ExceptHandler(h) = handler;
                    rename_defs(&mut h.body, name, next);
                }
                rename_defs(&mut s.orelse, name, next);
                rename_defs(&mut s.finalbody, name, next);
            }
            _ => {}
        }
        body.push(stmt);
        if let Some((k, range)) = store_after {
            body.push(assign(store(&selector(name), range), int(k as u64, range), range));
        }
    }
}

/// Rewrites every call of a renamed function into a choice between its
/// definitions.
struct Calls<'a> {
    /// Name -> number of definitions.
    names: &'a HashMap<String, usize>,
}

impl Transformer for Calls<'_> {
    fn visit_expr(&self, expr: &mut Expr) {
        transformer::walk_expr(self, expr);
        let Expr::Call(call) = expr else {
            return;
        };
        let Expr::Name(func) = call.func.as_ref() else {
            return;
        };
        let Some(&count) = self.names.get(func.id.as_str()) else {
            return;
        };
        let name = func.id.to_string();
        let range = call.range;
        let call_of = |k: usize| {
            let mut c = call.clone();
            c.func = Box::new(load(&alternative(&name, k), func.range));
            Expr::Call(c)
        };
        let mut choice = call_of(count);
        for k in (1..count).rev() {
            choice = Expr::If(ExprIf {
                node_index: AtomicNodeIndex::default(),
                range,
                test: Box::new(Expr::Compare(ExprCompare {
                    node_index: AtomicNodeIndex::default(),
                    range,
                    left: Box::new(load(&selector(&name), range)),
                    ops: vec![CmpOp::Eq].into(),
                    comparators: vec![int(k as u64, range)].into(),
                })),
                body: Box::new(call_of(k)),
                orelse: Box::new(choice),
            });
        }
        *expr = choice;
    }
}

/// Gives each nested function `def` defines more than once a name per
/// definition (module doc).
pub(super) fn split_conditional_defs(def: &mut StmtFunctionDef) {
    let mut defined = HashMap::new();
    block_defs(&def.body, &mut defined);
    let names: HashMap<String, usize> = defined
        .into_iter()
        .filter(|(name, count)| {
            let uses = count_uses(&def.body, name);
            *count > 1
                && uses.bindings == *count
                && uses.other == 0
                && def.parameters.iter().all(|p| p.name().as_str() != name)
        })
        .collect();
    if names.is_empty() {
        return;
    }
    for name in names.keys() {
        let mut next = 0;
        rename_defs(&mut def.body, name, &mut next);
    }
    let calls = Calls { names: &names };
    for stmt in def.body.iter_mut() {
        calls.visit_stmt(stmt);
    }
}
