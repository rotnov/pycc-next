//! A parameter whose default is a builtin conversion (`def rl(t=int):`)
//! that no call passes.
//!
//! A module-level function called only by name, and never with that
//! parameter (no call has as many positional arguments, a keyword of its
//! name, or a `*`/`**` argument), always runs with the default: the
//! parameter is dropped and its reads in the body are the builtin itself
//! (`map(t, ...)` is `map(int, ...)`). The function must not rebind the
//! parameter, and no lambda, comprehension or nested definition in it may
//! bind the same name.

use super::aliases::rebound_names;
use ruff_python_ast::visitor::transformer::{self, Transformer};
use ruff_python_ast::visitor::{self, Visitor};
use ruff_python_ast::{Expr, ExprContext, Stmt, StmtFunctionDef};

/// The builtins a default may name.
const CONVERSIONS: &[&str] = &["int", "float", "str", "bool"];

/// Inlines every such default (module doc).
pub(crate) fn inline_builtin_defaults(body: &mut [Stmt]) {
    let rebound = rebound_names(body);
    let candidates: Vec<(String, usize, String, String)> = body
        .iter()
        .filter_map(|stmt| match stmt {
            Stmt::FunctionDef(def) => Some(def),
            _ => None,
        })
        .filter(|def| def.decorator_list.is_empty() && rebound.get(def.name.as_str()) == Some(&1))
        .flat_map(|def| {
            let params = &def.parameters;
            let simple = params.posonlyargs.is_empty()
                && params.vararg.is_none()
                && params.kwarg.is_none()
                && params.kwonlyargs.is_empty();
            params
                .args
                .iter()
                .enumerate()
                .filter(move |_| simple)
                .filter_map(|(index, param)| {
                    let Some(Expr::Name(default)) = param.default.as_deref() else {
                        return None;
                    };
                    (param.parameter.annotation.is_none()
                        && CONVERSIONS.contains(&default.id.as_str()))
                    .then(|| {
                        (
                            def.name.to_string(),
                            index,
                            param.parameter.name.to_string(),
                            default.id.to_string(),
                        )
                    })
                })
                .collect::<Vec<_>>()
        })
        .collect();
    for (function, index, param, builtin) in candidates {
        if rebound.contains_key(&builtin) || !never_passed(body, &function, index, &param) {
            continue;
        }
        let Some(def) = body.iter_mut().find_map(|stmt| match stmt {
            Stmt::FunctionDef(def) if def.name.as_str() == function => Some(def),
            _ => None,
        }) else {
            continue;
        };
        if rebound_names(&def.body).contains_key(&param) {
            continue;
        }
        inline(def, &param, &builtin);
    }
}

/// Drops `param` from `def` and reads `builtin` in its place.
fn inline(def: &mut StmtFunctionDef, param: &str, builtin: &str) {
    def.parameters
        .args
        .retain(|p| p.parameter.name.as_str() != param);
    struct Substitute<'a>(&'a str, &'a str);
    impl Transformer for Substitute<'_> {
        fn visit_expr(&self, expr: &mut Expr) {
            if let Expr::Name(name) = expr
                && name.ctx == ExprContext::Load
                && name.id.as_str() == self.0
            {
                name.id = self.1.into();
            }
            transformer::walk_expr(self, expr);
        }
    }
    let substitute = Substitute(param, builtin);
    for stmt in def.body.iter_mut() {
        substitute.visit_stmt(stmt);
    }
}

/// Whether every read of `function` in `body` is a call by name that does
/// not pass parameter `index` (`param`), and there is at least one.
fn never_passed(body: &[Stmt], function: &str, index: usize, param: &str) -> bool {
    struct Check<'a> {
        function: &'a str,
        index: usize,
        param: &'a str,
        calls: usize,
        reads: usize,
        ok: bool,
    }
    impl<'a> Visitor<'a> for Check<'_> {
        fn visit_expr(&mut self, expr: &'a Expr) {
            match expr {
                Expr::Call(call) if matches!(call.func.as_ref(), Expr::Name(n) if n.id.as_str() == self.function) =>
                {
                    self.calls += 1;
                    let args = &call.arguments;
                    if args.args.len() > self.index
                        || args.args.iter().any(|a| matches!(a, Expr::Starred(_)))
                        || args.keywords.iter().any(|k| {
                            k.arg
                                .as_ref()
                                .is_none_or(|name| name.as_str() == self.param)
                        })
                    {
                        self.ok = false;
                    }
                    for arg in &args.args {
                        self.visit_expr(arg);
                    }
                    for keyword in &args.keywords {
                        self.visit_expr(&keyword.value);
                    }
                    return;
                }
                Expr::Name(n) if n.id.as_str() == self.function => self.reads += 1,
                _ => {}
            }
            visitor::walk_expr(self, expr);
        }
    }
    let mut check = Check {
        function,
        index,
        param,
        calls: 0,
        reads: 0,
        ok: true,
    };
    for stmt in body {
        check.visit_stmt(stmt);
    }
    check.ok && check.calls > 0 && check.reads == 0
}
