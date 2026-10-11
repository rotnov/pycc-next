//! A `def` inside a module-level block (`for _ in range(t): def solve():
//! ...`) becomes a module-level `def`, placed right before the top-level
//! statement that contains it.
//!
//! A function defined in a module-level block is a module-level function:
//! its free names are module globals either way. Hoisting it changes only
//! *when* the name is bound, so it is done only when that cannot be seen:
//! the name is bound nowhere else in the module, and every use of it is in
//! the same block, after the `def` (or inside the function itself). Each
//! use then runs after the `def` has run, in CPython as in the hoisted
//! program. The function must also have the shape a module-level lift
//! keeps exact: no decorators, and only literal defaults (evaluated once
//! either way).

use super::closures::{count_uses, liftable_shape};
use ruff_python_ast::Stmt;

/// Hoists the eligible block-level `def`s of `body` (a module).
pub(crate) fn hoist_block_defs(body: &mut thin_vec::ThinVec<Stmt>) {
    let snapshot: Vec<Stmt> = body.iter().cloned().collect();
    let old = std::mem::take(body);
    for mut stmt in old {
        let mut hoisted = Vec::new();
        if !matches!(stmt, Stmt::FunctionDef(_) | Stmt::ClassDef(_)) {
            hoist_in_stmt(&mut stmt, &snapshot, &mut hoisted);
        }
        body.extend(hoisted);
        body.push(stmt);
    }
}

fn hoist_in_stmt(stmt: &mut Stmt, module: &[Stmt], out: &mut Vec<Stmt>) {
    match stmt {
        Stmt::If(s) => {
            hoist_in_block(&mut s.body, module, out);
            for clause in s.elif_else_clauses.iter_mut() {
                hoist_in_block(&mut clause.body, module, out);
            }
        }
        Stmt::While(s) => {
            hoist_in_block(&mut s.body, module, out);
            hoist_in_block(&mut s.orelse, module, out);
        }
        Stmt::For(s) => {
            hoist_in_block(&mut s.body, module, out);
            hoist_in_block(&mut s.orelse, module, out);
        }
        Stmt::With(s) => hoist_in_block(&mut s.body, module, out),
        Stmt::Try(s) => {
            hoist_in_block(&mut s.body, module, out);
            for handler in s.handlers.iter_mut() {
                let ruff_python_ast::ExceptHandler::ExceptHandler(h) = handler;
                hoist_in_block(&mut h.body, module, out);
            }
            hoist_in_block(&mut s.orelse, module, out);
            hoist_in_block(&mut s.finalbody, module, out);
        }
        _ => {}
    }
}

fn hoist_in_block(block: &mut thin_vec::ThinVec<Stmt>, module: &[Stmt], out: &mut Vec<Stmt>) {
    let mut i = 0;
    while i < block.len() {
        if let Stmt::FunctionDef(def) = &block[i] {
            let name = def.name.as_str();
            let all = count_uses(module, name);
            let here = count_uses(&block[i..], name);
            // A module hook (`__getattr__`, `__dir__`) keeps its own
            // refusal: what it means depends on where it is bound.
            let dunder = name.starts_with("__") && name.ends_with("__");
            if !dunder
                && liftable_shape(def)
                && all.bindings == 1
                && here.bindings == 1
                && all.calls == here.calls
                && all.other == here.other
            {
                out.push(block.remove(i));
                continue;
            }
        } else {
            hoist_in_stmt(&mut block[i], module, out);
        }
        i += 1;
    }
    if block.is_empty() {
        block.push(Stmt::Pass(ruff_python_ast::StmtPass {
            node_index: ruff_python_ast::AtomicNodeIndex::default(),
            range: ruff_text_size::TextRange::default(),
        }));
    }
}
