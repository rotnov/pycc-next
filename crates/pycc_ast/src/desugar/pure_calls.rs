//! Builtin calls whose argument shape can be simplified without changing
//! what CPython does:
//!
//! * a bare `zip()` statement builds an empty iterator nobody reads, with no
//!   effect at all, so it becomes `pass`; its element type is otherwise
//!   unknowable;
//! * `sorted((a, b))` (also `min`, `max`, `sum`) iterates a tuple display
//!   once, element by element, exactly as it iterates the list display
//!   `[a, b]`, so the display becomes a list: the elements are evaluated in
//!   the same order and the call returns the same value.
//!
//! Neither applies when the module binds or imports the builtin's name.

use super::aliases::rebound_names;
use ruff_python_ast::visitor::transformer::{self, Transformer};
use ruff_python_ast::{Expr, ExprList, Stmt, StmtPass};

/// The builtins whose single positional argument may be a tuple display
/// read as a list display.
const ITERATING: [&str; 4] = ["sorted", "min", "max", "sum"];

/// Rewrites every bare `zip()` statement in `body` to `pass`, and a tuple
/// display passed alone to `sorted`/`min`/`max`/`sum` to a list display.
pub(crate) fn drop_effectless_calls(body: &mut [Stmt]) {
    let rebound = rebound_names(body);
    let free = |name: &str| {
        !rebound.contains_key(name)
            && !body.iter().any(|stmt| match stmt {
                Stmt::Import(import) => import
                    .names
                    .iter()
                    .any(|a| a.asname.as_ref().unwrap_or(&a.name).as_str() == name),
                Stmt::ImportFrom(import) => import
                    .names
                    .iter()
                    .any(|a| a.asname.as_ref().unwrap_or(&a.name).as_str() == name),
                _ => false,
            })
    };
    let zip_free = free("zip");
    let names: Vec<&str> = ITERATING.into_iter().filter(|name| free(name)).collect();
    if zip_free {
        let pass = Drop;
        for stmt in body.iter_mut() {
            pass.visit_stmt(stmt);
        }
    }
    if !names.is_empty() {
        let pass = TupleArgument { names };
        for stmt in body.iter_mut() {
            pass.visit_stmt(stmt);
        }
    }
}

struct TupleArgument<'a> {
    names: Vec<&'a str>,
}

impl Transformer for TupleArgument<'_> {
    fn visit_expr(&self, expr: &mut Expr) {
        transformer::walk_expr(self, expr);
        let Expr::Call(call) = expr else {
            return;
        };
        if !matches!(call.func.as_ref(), Expr::Name(name) if self.names.contains(&name.id.as_str()))
        {
            return;
        }
        let [Expr::Tuple(tuple)] = &mut call.arguments.args[..] else {
            return;
        };
        if tuple.elts.iter().any(|elt| matches!(elt, Expr::Starred(_))) {
            return;
        }
        let list = Expr::List(ExprList {
            node_index: Default::default(),
            range: tuple.range,
            elts: std::mem::take(&mut tuple.elts),
            ctx: tuple.ctx,
        });
        call.arguments.args[0] = list;
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
