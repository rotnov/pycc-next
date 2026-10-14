//! The builtins `exit`/`quit` and `from sys import exit`, read as
//! `sys.exit`.
//!
//! `exit(code)` and `quit(code)` (the `site` builtins) and `sys.exit(code)`
//! all raise `SystemExit(code)`. `sys.exit` is CPython's own, reached
//! through the embedded interpreter, which raises the real `SystemExit`:
//! `finally` blocks run, `except Exception` lets it through, and an
//! uncaught one ends the program with CPython's exit status and message.
//! So every read of the builtin names, and of a name bound by `from sys
//! import exit`, becomes `__pycc_sys.exit`, with `import sys as __pycc_sys`
//! added to the module. A module that binds `exit`/`quit` itself keeps its
//! own binding.

use super::aliases::rebound_names;
use super::build::load;
use ruff_python_ast::visitor::transformer::{self, Transformer};
use ruff_python_ast::{
    Alias, AtomicNodeIndex, Expr, ExprAttribute, ExprContext, Identifier, Stmt, StmtImport,
};
use ruff_text_size::TextRange;
use std::collections::HashSet;

/// The name the added `import sys` binds.
const SYS: &str = "__pycc_sys";

/// Rewrites the reads of `exit`/`quit` and of `from sys import exit`
/// names to `__pycc_sys.exit` (module doc).
pub(crate) fn sys_exit_calls(body: &mut Vec<Stmt>) {
    let rebound = rebound_names(body);
    let mut names: HashSet<String> = ["exit", "quit"]
        .into_iter()
        .filter(|name| !rebound.contains_key(*name) && !imported_elsewhere(body, name))
        .map(str::to_string)
        .collect();
    // `from sys import exit [as e]`: the alias is dropped, its name read
    // as `sys.exit`.
    for stmt in body.iter_mut() {
        let Stmt::ImportFrom(import) = stmt else {
            continue;
        };
        if import.level != 0 || import.module.as_ref().is_none_or(|m| m.as_str() != "sys") {
            continue;
        }
        import.names.retain(|alias| {
            let bound = alias.asname.as_ref().unwrap_or(&alias.name).as_str();
            if alias.name.as_str() == "exit" && !rebound.contains_key(bound) {
                names.insert(bound.to_string());
                false
            } else {
                true
            }
        });
    }
    body.retain(|stmt| !matches!(stmt, Stmt::ImportFrom(import) if import.names.is_empty()));
    if names.is_empty() {
        return;
    }
    struct Rewrite(HashSet<String>, std::cell::Cell<bool>);
    impl Transformer for Rewrite {
        fn visit_expr(&self, expr: &mut Expr) {
            if let Expr::Name(name) = expr
                && name.ctx == ExprContext::Load
                && self.0.contains(name.id.as_str())
            {
                let range = name.range;
                *expr = Expr::Attribute(ExprAttribute {
                    node_index: AtomicNodeIndex::default(),
                    range,
                    value: Box::new(load(SYS, range)),
                    attr: Identifier::new("exit", range),
                    ctx: ExprContext::Load,
                });
                self.1.set(true);
                return;
            }
            transformer::walk_expr(self, expr);
        }
    }
    let rewrite = Rewrite(names, std::cell::Cell::new(false));
    for stmt in body.iter_mut() {
        rewrite.visit_stmt(stmt);
    }
    if !rewrite.1.get() {
        return;
    }
    let at = body
        .iter()
        .position(|stmt| {
            !matches!(stmt, Stmt::ImportFrom(import)
                if import.module.as_ref().is_some_and(|m| m.as_str() == "__future__"))
        })
        .unwrap_or(body.len());
    let range = TextRange::default();
    body.insert(
        at,
        Stmt::Import(StmtImport {
            node_index: AtomicNodeIndex::default(),
            range,
            names: vec![Alias {
                node_index: AtomicNodeIndex::default(),
                range,
                name: Identifier::new("sys", range),
                asname: Some(Identifier::new(SYS, range)),
            }],
            is_lazy: false,
        }),
    );
}

/// Whether an import other than `from sys import exit` binds `name`.
fn imported_elsewhere(body: &[Stmt], name: &str) -> bool {
    struct Find<'a>(&'a str, bool);
    impl<'a> ruff_python_ast::visitor::Visitor<'a> for Find<'_> {
        fn visit_stmt(&mut self, stmt: &'a Stmt) {
            let binds =
                |alias: &Alias| alias.asname.as_ref().unwrap_or(&alias.name).as_str() == self.0;
            match stmt {
                Stmt::Import(import) if import.names.iter().any(binds) => self.1 = true,
                Stmt::ImportFrom(import)
                    if import.names.iter().any(|alias| {
                        binds(alias)
                            && !(import.module.as_ref().is_some_and(|m| m.as_str() == "sys")
                                && alias.name.as_str() == "exit")
                    }) || import.names.iter().any(|alias| alias.name.as_str() == "*") =>
                {
                    self.1 = true;
                }
                _ => ruff_python_ast::visitor::walk_stmt(self, stmt),
            }
        }
    }
    let mut find = Find(name, false);
    for stmt in body {
        ruff_python_ast::visitor::Visitor::visit_stmt(&mut find, stmt);
    }
    find.1
}
