//! `@functools.cache` and `@functools.lru_cache` on a module-level function
//! become a native memo table. For
//!
//! ```python
//! @lru_cache(maxsize=2)
//! def f(a: int, b: str) -> int: ...
//! ```
//!
//! the module gets a global `_pycc_cache_f: dict[tuple[int, str], int] = {}`
//! (the key is the argument itself for one parameter), the body moves to
//! `_pycc_uncached_f`, and `f` looks the arguments up, calls the body on a
//! miss and stores the result. A bounded cache (`maxsize=N`) keeps the
//! table in least-recently-used order -- a hit moves its entry to the end, a
//! store past `N` entries drops the first -- as `lru_cache` does;
//! `maxsize=0` stores nothing. `f.cache_clear()` empties the table.
//!
//! Only a function whose every parameter is annotated, positional-or-keyword
//! and has a hashable scalar annotation (`int`, `float`, `bool`, `str`), and
//! whose return is annotated, is rewritten; any other stays decorated, and
//! HIR refuses it (`C0001`; `--non-strict` runs it in CPython).

use super::signatures::parse_helper;
use ruff_python_ast::visitor::transformer::{self, Transformer};
use ruff_python_ast::{Expr, Stmt, StmtFunctionDef};
use ruff_text_size::Ranged;
use std::collections::HashMap;

/// A recognized cache decorator: `None` for an unbounded cache.
fn cache_bound(decorator: &Expr) -> Option<Option<u64>> {
    let is_named = |expr: &Expr, name: &str| match expr {
        Expr::Name(n) => n.id.as_str() == name,
        Expr::Attribute(a) => {
            a.attr.as_str() == name
                && matches!(&*a.value, Expr::Name(n) if n.id.as_str() == "functools")
        }
        _ => false,
    };
    match decorator {
        e if is_named(e, "cache") => Some(None),
        // `@lru_cache` without a call: the default `maxsize=128`.
        e if is_named(e, "lru_cache") => Some(Some(128)),
        Expr::Call(call) if is_named(&call.func, "lru_cache") => {
            let mut maxsize = Some(128);
            let mut args = call.arguments.args.iter();
            let first = args.next();
            if args.next().is_some() {
                return None;
            }
            let mut size_expr = first;
            for keyword in call.arguments.keywords.iter() {
                match keyword.arg.as_ref().map(|a| a.as_str()) {
                    Some("maxsize") if size_expr.is_none() => size_expr = Some(&keyword.value),
                    // `typed=False` changes nothing for typed parameters of
                    // distinct types; `typed=True` would.
                    Some("typed") if matches!(&keyword.value, Expr::BooleanLiteral(b) if !b.value) => {}
                    _ => return None,
                }
            }
            if let Some(size) = size_expr {
                maxsize = match size {
                    Expr::NoneLiteral(_) => None,
                    Expr::NumberLiteral(n) => Some(n.value.as_int()?.as_u64()?),
                    _ => return None,
                };
            }
            Some(maxsize)
        }
        _ => None,
    }
}

/// The source text of a hashable scalar annotation, if `annotation` is one.
fn scalar(annotation: Option<&Expr>) -> Option<&'static str> {
    match annotation? {
        Expr::Name(n) => match n.id.as_str() {
            "int" => Some("int"),
            "float" => Some("float"),
            "bool" => Some("bool"),
            "str" => Some("str"),
            _ => None,
        },
        _ => None,
    }
}

/// A rewritable function's parameter names and annotations.
fn plain_params(def: &StmtFunctionDef) -> Option<Vec<(String, &'static str)>> {
    let p = &def.parameters;
    if !p.posonlyargs.is_empty() || p.vararg.is_some() || !p.kwonlyargs.is_empty() || p.kwarg.is_some() {
        return None;
    }
    p.args
        .iter()
        .map(|a| Some((a.parameter.name.to_string(), scalar(a.parameter.annotation.as_deref())?)))
        .collect()
}

/// Rewrites every cached module-level function of `body` (module doc).
pub(super) fn expand_caches(body: &mut Vec<Stmt>) {
    let mut cached: HashMap<String, String> = HashMap::new();
    let old = std::mem::take(body);
    for stmt in old {
        let Stmt::FunctionDef(mut def) = stmt else {
            body.push(stmt);
            continue;
        };
        let bound = match def.decorator_list.as_slice() {
            [d] => cache_bound(&d.expression),
            _ => None,
        };
        let (Some(bound), Some(params), Some(returns)) =
            (bound, plain_params(&def), def.returns.clone())
        else {
            body.push(Stmt::FunctionDef(def));
            continue;
        };
        if def.is_async || def.type_params.is_some() {
            body.push(Stmt::FunctionDef(def));
            continue;
        }
        let name = def.name.to_string();
        let table = format!("_pycc_cache_{name}");
        let uncached = format!("_pycc_uncached_{name}");
        let range = def.range;
        let names: Vec<&str> = params.iter().map(|(n, _)| n.as_str()).collect();
        let (key_ty, key) = match params.as_slice() {
            [(n, ty)] => (ty.to_string(), n.clone()),
            many => (
                format!(
                    "tuple[{}]",
                    many.iter().map(|(_, t)| *t).collect::<Vec<_>>().join(", ")
                ),
                format!("({},)", names.join(", ")),
            ),
        };
        let call = format!("{uncached}({})", names.join(", "));
        // The wrapper keeps the declared parameters (defaults included) and
        // the return annotation; the hole names stand for the return.
        let lookup = match bound {
            Some(0) => format!("    return {call}\n"),
            None => format!(
                "    _pycc_key = {key}\n    if _pycc_key in {table}:\n        return {table}[_pycc_key]\n    \
                 _pycc_result = {call}\n    {table}[_pycc_key] = _pycc_result\n    return _pycc_result\n"
            ),
            Some(n) => format!(
                "    _pycc_key = {key}\n    if _pycc_key in {table}:\n        \
                 _pycc_hit = {table}.pop(_pycc_key)\n        {table}[_pycc_key] = _pycc_hit\n        \
                 return _pycc_hit\n    _pycc_result = {call}\n    \
                 if _pycc_key not in {table}:\n        {table}[_pycc_key] = _pycc_result\n        \
                 if len({table}) > {n}:\n            for _pycc_old in {table}:\n                \
                 del {table}[_pycc_old]\n                break\n    return _pycc_result\n"
            ),
        };
        let returns_hole = "__pycc_hole_ret";
        let source = format!(
            "{table}: dict[{key_ty}, {returns_hole}] = {{}}\n\
             def {name}() -> {returns_hole}:\n{lookup}"
        );
        let mut generated = parse_helper(&source, range);
        let fill = Fill(&returns);
        for stmt in &mut generated {
            fill.visit_stmt(stmt);
        }
        // The wrapper takes the original parameters as declared.
        if let Some(Stmt::FunctionDef(wrapper)) = generated.get_mut(1) {
            wrapper.parameters = def.parameters.clone();
        }
        let mut inner = def.clone();
        inner.decorator_list.clear();
        inner.name = ruff_python_ast::Identifier::new(uncached.as_str(), def.name.range);
        for p in inner.parameters.args.iter_mut() {
            p.default = None;
        }
        def.decorator_list.clear();
        body.push(Stmt::FunctionDef(inner));
        body.extend(generated);
        cached.insert(name, table);
    }
    if cached.is_empty() {
        return;
    }
    // `f.cache_clear()` empties the table.
    let rewrite = CacheClear(&cached);
    for stmt in body.iter_mut() {
        rewrite.visit_stmt(stmt);
    }
    drop_functools_imports(body);
}

/// Replaces the return-annotation hole with the declared return.
struct Fill<'a>(&'a Expr);

impl Transformer for Fill<'_> {
    fn visit_expr(&self, expr: &mut Expr) {
        if matches!(expr, Expr::Name(n) if n.id.as_str() == "__pycc_hole_ret") {
            *expr = self.0.clone();
            return;
        }
        transformer::walk_expr(self, expr);
    }
}

/// `f.cache_clear()` of a rewritten `f` becomes `_pycc_cache_f.clear()`.
struct CacheClear<'a>(&'a HashMap<String, String>);

impl Transformer for CacheClear<'_> {
    fn visit_expr(&self, expr: &mut Expr) {
        transformer::walk_expr(self, expr);
        let Expr::Call(call) = expr else { return };
        let Expr::Attribute(attr) = &*call.func else { return };
        let Expr::Name(base) = &*attr.value else { return };
        if attr.attr.as_str() != "cache_clear" || !call.arguments.args.is_empty() {
            return;
        }
        let Some(table) = self.0.get(base.id.as_str()) else { return };
        let range = call.func.range();
        call.func = Box::new(super::build::attribute(
            super::build::load(table, range),
            "clear",
            range,
        ));
    }
}

/// Drops `cache`/`lru_cache` from `from functools import ...` once no use
/// of either name is left.
fn drop_functools_imports(body: &mut Vec<Stmt>) {
    let uses = |name: &str| -> bool {
        let found = super::closures::count_uses(body, name);
        found.calls + found.other > 0
    };
    let keep_cache = uses("cache");
    let keep_lru = uses("lru_cache");
    body.retain_mut(|stmt| {
        let Stmt::ImportFrom(import) = stmt else {
            return true;
        };
        if import.module.as_deref() != Some("functools") || import.level != 0 {
            return true;
        }
        import.names.retain(|alias| {
            alias.asname.is_some()
                || match alias.name.as_str() {
                    "cache" => keep_cache,
                    "lru_cache" => keep_lru,
                    _ => true,
                }
        });
        !import.names.is_empty()
    });
}
