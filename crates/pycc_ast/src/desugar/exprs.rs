//! Expression-level rewrites: `sys` stream calls, `map`/`filter`, starred
//! list displays and `print(*xs)`.

use super::aliases::{SysNames, stdin_lines};
use super::build::{attribute, call, call_kw, call_name, list_comp, load, store, string};
use super::{Fresh, stored_names};
use ruff_python_ast::visitor::transformer::{self, Transformer};
use ruff_python_ast::{Expr, ExprCall, ExprContext, Operator, Stmt};
use ruff_text_size::{Ranged, TextRange};
use std::collections::HashSet;

struct Rewriter<'a> {
    sys: &'a SysNames,
    fresh: &'a Fresh,
    /// Builtin names the program rebinds, which are left alone.
    shadowed: HashSet<String>,
}

pub(crate) fn rewrite_exprs(body: &mut [Stmt], sys: &SysNames, fresh: &Fresh) {
    let rewriter = Rewriter {
        sys,
        fresh,
        shadowed: stored_names(body),
    };
    for stmt in body {
        rewriter.visit_stmt(stmt);
    }
}

fn binop(left: Expr, right: Expr, range: TextRange) -> Expr {
    Expr::BinOp(ruff_python_ast::ExprBinOp {
        node_index: Default::default(),
        range,
        left: Box::new(left),
        op: Operator::Add,
        right: Box::new(right),
    })
}

impl Rewriter<'_> {
    fn builtin(&self, name: &str) -> bool {
        !self.shadowed.contains(name)
    }

    /// `f` applied to a fresh comprehension variable (`keys::apply`).
    fn apply(&self, f: &Expr, range: TextRange) -> Option<(String, Expr)> {
        super::keys::apply(f, self.fresh, range)
    }

    /// `map(f, xs)` -> `[f(v) for v in xs]`; `filter(f, xs)` ->
    /// `[v for v in xs if f(v)]` (`filter(None, xs)` tests `v` itself).
    fn map_or_filter(&self, call_expr: &ExprCall) -> Option<Expr> {
        let Expr::Name(func) = call_expr.func.as_ref() else {
            return None;
        };
        let [f, xs] = &*call_expr.arguments.args else {
            return None;
        };
        if !call_expr.arguments.keywords.is_empty() || matches!(xs, Expr::Starred(_)) {
            return None;
        }
        let range = call_expr.range;
        match func.id.as_str() {
            "map" if self.builtin("map") => {
                let (var, elt) = self.apply(f, range)?;
                Some(list_comp(
                    elt,
                    store(&var, range),
                    xs.clone(),
                    Vec::new(),
                    range,
                ))
            }
            "filter" if self.builtin("filter") => {
                let (var, test) = if matches!(f, Expr::NoneLiteral(_)) {
                    let var = self.fresh.name();
                    let test = load(&var, range);
                    (var, test)
                } else {
                    self.apply(f, range)?
                };
                Some(list_comp(
                    load(&var, range),
                    store(&var, range),
                    xs.clone(),
                    vec![test],
                    range,
                ))
            }
            _ => None,
        }
    }

    /// A list display with starred items: `[*xs]` -> `list(xs)`,
    /// `[a, *xs]` -> `[a] + list(xs)`.
    fn starred_list(&self, elts: &[Expr], range: TextRange) -> Option<Expr> {
        if !elts.iter().any(|e| matches!(e, Expr::Starred(_))) || !self.builtin("list") {
            return None;
        }
        let mut pieces: Vec<Expr> = Vec::new();
        let mut run: Vec<Expr> = Vec::new();
        let flush = |run: &mut Vec<Expr>, pieces: &mut Vec<Expr>| {
            if !run.is_empty() {
                pieces.push(Expr::List(ruff_python_ast::ExprList {
                    node_index: Default::default(),
                    range,
                    elts: std::mem::take(run),
                    ctx: ExprContext::Load,
                }));
            }
        };
        for elt in elts {
            match elt {
                Expr::Starred(starred) => {
                    flush(&mut run, &mut pieces);
                    pieces.push(match starred.value.as_ref() {
                        comp @ Expr::ListComp(_) => comp.clone(),
                        other => call_name("list", vec![other.clone()], range),
                    });
                }
                other => run.push(other.clone()),
            }
        }
        flush(&mut run, &mut pieces);
        let mut iter = pieces.into_iter();
        let first = iter.next()?;
        Some(iter.fold(first, |acc, next| binop(acc, next, range)))
    }

    /// `print(a, *xs, sep=s, end=e)` -> `print(s.join([str(a)] + [str(v)
    /// for v in xs]), end=e)`.
    fn starred_print(&self, call_expr: &ExprCall) -> Option<Expr> {
        let Expr::Name(func) = call_expr.func.as_ref() else {
            return None;
        };
        let args = &call_expr.arguments.args;
        if func.id.as_str() != "print"
            || !self.builtin("print")
            || !self.builtin("str")
            || !args.iter().any(|a| matches!(a, Expr::Starred(_)))
        {
            return None;
        }
        let range = call_expr.range;
        let mut sep = string(" ", range);
        let mut keywords = Vec::new();
        for kw in &call_expr.arguments.keywords {
            match kw.arg.as_ref().map(|a| a.as_str()) {
                Some("sep") => sep = kw.value.clone(),
                Some(_) => keywords.push(kw.clone()),
                None => return None,
            }
        }
        let mut pieces: Vec<Expr> = Vec::new();
        for arg in args.iter() {
            pieces.push(match arg {
                Expr::Starred(starred) => {
                    let var = self.fresh.name();
                    list_comp(
                        call_name("str", vec![load(&var, range)], range),
                        store(&var, range),
                        (*starred.value).clone(),
                        Vec::new(),
                        range,
                    )
                }
                other => Expr::List(ruff_python_ast::ExprList {
                    node_index: Default::default(),
                    range,
                    elts: vec![call_name("str", vec![other.clone()], range)],
                    ctx: ExprContext::Load,
                }),
            });
        }
        let mut iter = pieces.into_iter();
        let first = iter.next()?;
        let joined_list = iter.fold(first, |acc, next| binop(acc, next, range));
        let joined = call(attribute(sep, "join", range), vec![joined_list], range);
        Some(call_kw(load("print", range), vec![joined], keywords, range))
    }
}

impl Rewriter<'_> {
    /// `print(a, b, sep=s, end=e)` -> `__pycc_print_str(s.join([str(a),
    /// str(b)]) + e)`, for a literal or name `sep`/`end` (so evaluation
    /// order is unchanged). A literal `flush` is dropped: `print` already
    /// writes through stdout's line buffer.
    fn print_keywords(&self, call_expr: &ExprCall) -> Option<Expr> {
        let Expr::Name(func) = call_expr.func.as_ref() else {
            return None;
        };
        if func.id.as_str() != "print"
            || !self.builtin("print")
            || !self.builtin("str")
            || call_expr.arguments.keywords.is_empty()
            || call_expr
                .arguments
                .args
                .iter()
                .any(|a| matches!(a, Expr::Starred(_)))
        {
            return None;
        }
        let range = call_expr.range;
        let (mut sep, mut end) = (string(" ", range), string("\n", range));
        for kw in &call_expr.arguments.keywords {
            match kw.arg.as_ref().map(|a| a.as_str()) {
                Some("sep") if super::is_simple(&kw.value) => sep = kw.value.clone(),
                Some("end") if super::is_simple(&kw.value) => end = kw.value.clone(),
                Some("flush") if super::is_simple(&kw.value) => {}
                _ => return None,
            }
        }
        let pieces = Expr::List(ruff_python_ast::ExprList {
            node_index: Default::default(),
            range,
            elts: call_expr
                .arguments
                .args
                .iter()
                .map(|a| call_name("str", vec![a.clone()], range))
                .collect(),
            ctx: ExprContext::Load,
        });
        let joined = call(attribute(sep, "join", range), vec![pieces], range);
        Some(call_name(
            "__pycc_print_str",
            vec![binop(joined, end, range)],
            range,
        ))
    }
}

impl Rewriter<'_> {
    /// A generator expression passed as the only argument of a consumer
    /// that materializes it (`sum(x for x in xs)`, `", ".join(... for ...)`)
    /// becomes the equivalent list comprehension (`comps.rs`).
    fn materialize_generator_argument(&self, call_expr: &mut ExprCall) {
        if call_expr.arguments.args.len() != 1 || !call_expr.arguments.keywords.is_empty() {
            return;
        }
        let consumer = match call_expr.func.as_ref() {
            Expr::Name(func)
                if super::comps::is_materializing_consumer(func.id.as_str())
                    && self.builtin(func.id.as_str()) =>
            {
                func.id.to_string()
            }
            Expr::Attribute(attr) if attr.attr.as_str() == "join" => "join".to_string(),
            _ => return,
        };
        if let Some(list) =
            super::comps::materialize_generator(&consumer, &call_expr.arguments.args[0])
        {
            call_expr.arguments.args[0] = list;
        }
    }
}

impl Rewriter<'_> {
    /// A `range(...)` argument of a builtin that consumes an iterable
    /// (`list(range(n))`, `set(range(a, b))`, `zip(range(n), xs)`) becomes
    /// the list comprehension `[v for v in range(...)]`, which the
    /// compiler iterates natively: `range` itself is not a value it has.
    fn materialize_range_arguments(&self, call_expr: &mut ExprCall) {
        let Expr::Name(func) = call_expr.func.as_ref() else {
            return;
        };
        if !matches!(
            func.id.as_str(),
            "list"
                | "set"
                | "frozenset"
                | "sorted"
                | "sum"
                | "min"
                | "max"
                | "zip"
                | "enumerate"
                | "tuple"
                | "any"
                | "all"
                | "reversed"
                | "map"
                | "filter"
        ) || !self.builtin(func.id.as_str())
            || !self.builtin("range")
        {
            return;
        }
        for arg in call_expr.arguments.args.iter_mut() {
            let Expr::Call(inner) = arg else { continue };
            if !matches!(inner.func.as_ref(), Expr::Name(name) if name.id.as_str() == "range")
                || !inner.arguments.keywords.is_empty()
                || inner
                    .arguments
                    .args
                    .iter()
                    .any(|a| matches!(a, Expr::Starred(_)))
            {
                continue;
            }
            let range = inner.range;
            let var = self.fresh.name();
            *arg = list_comp(
                load(&var, range),
                store(&var, range),
                arg.clone(),
                Vec::new(),
                range,
            );
        }
    }
}

impl Transformer for Rewriter<'_> {
    fn visit_expr(&self, expr: &mut Expr) {
        // `sys` stream operations first: their callee chain contains
        // `sys.stdin`, which on its own means something else.
        if let Expr::Call(call_expr) = expr
            && let Some(op) = self.sys.classify(&call_expr.func)
        {
            let range = call_expr.range;
            let mut args: Vec<Expr> = call_expr.arguments.args.to_vec();
            for arg in &mut args {
                self.visit_expr(arg);
            }
            *expr = call_name(op.builtin(), args, range);
            return;
        }
        // `next(sys.stdin)`: the stream's next line.
        if let Expr::Call(call_expr) = expr
            && matches!(call_expr.func.as_ref(), Expr::Name(n) if n.id.as_str() == "next")
            && self.builtin("next")
            && call_expr.arguments.keywords.is_empty()
            && matches!(call_expr.arguments.args.as_ref(), [arg] if self.sys.is_stdin(arg))
        {
            *expr = call_name("__pycc_stdin_next", Vec::new(), call_expr.range);
            return;
        }
        if self.sys.is_stdin(expr) {
            *expr = stdin_lines(expr.range());
            return;
        }
        transformer::walk_expr(self, expr);
        super::comps::flatten_tuple_targets(expr, self.fresh);
        if let Expr::Call(call_expr) = expr {
            self.materialize_range_arguments(call_expr);
            self.materialize_generator_argument(call_expr);
        }
        let replacement = match expr {
            Expr::Call(call_expr) => {
                super::keys::rewrite_key_call(call_expr, self.fresh, &|name| !self.builtin(name))
                    .or_else(|| self.map_or_filter(call_expr))
                    .or_else(|| {
                        let starred = self.starred_print(call_expr);
                        match &starred {
                            Some(Expr::Call(rewritten)) => {
                                self.print_keywords(rewritten).or(starred.clone())
                            }
                            _ => self.print_keywords(call_expr),
                        }
                    })
            }
            Expr::List(list) => self.starred_list(&list.elts, list.range),
            _ => None,
        };
        if let Some(replacement) = replacement {
            *expr = replacement;
        }
    }
}
