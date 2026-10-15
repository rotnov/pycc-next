//! A bare `zip()` statement: the call builds an empty iterator nobody
//! reads, with no effect at all, so it becomes `pass` (unless the module
//! binds `zip` itself). Its element type is otherwise unknowable.

use super::aliases::rebound_names;
use ruff_python_ast::visitor::transformer::{self, Transformer};
use ruff_python_ast::{Expr, Stmt, StmtPass};

/// Rewrites every bare `zip()` statement in `body` to `pass`.
pub(crate) fn drop_effectless_calls(body: &mut [Stmt]) {
    let imported = body.iter().any(|stmt| match stmt {
        Stmt::Import(import) => import
            .names
            .iter()
            .any(|a| a.asname.as_ref().unwrap_or(&a.name).as_str() == "zip"),
        Stmt::ImportFrom(import) => import
            .names
            .iter()
            .any(|a| a.asname.as_ref().unwrap_or(&a.name).as_str() == "zip"),
        _ => false,
    });
    if imported || rebound_names(body).contains_key("zip") {
        return;
    }
    let pass = Drop;
    for stmt in body.iter_mut() {
        pass.visit_stmt(stmt);
    }
}

struct Drop;

impl Transformer for Drop {
    fn visit_stmt(&self, stmt: &mut Stmt) {
        if let Stmt::Expr(expr) = stmt
            && let Expr::Call(call) = expr.value.as_ref()
            && matches!(call.func.as_ref(), Expr::Name(name) if name.id.as_str() == "zip")
            && call.arguments.is_empty()
        {
            *stmt = Stmt::Pass(StmtPass {
                node_index: Default::default(),
                range: expr.range,
            });
            return;
        }
        transformer::walk_stmt(self, stmt);
    }
}
