//! A generator expression becomes a call of a module-level generator helper:
//!
//! ```python
//! any(x * k > t for x in xs if x)      # in a function with locals k, t, xs
//! # ->
//! def _pycc_genexpr_N(_pycc_1, k, t):
//!     for x in _pycc_1:
//!         if x:
//!             yield x * k > t
//! any(_pycc_genexpr_N(xs, k, t))
//! ```
//!
//! CPython evaluates a generator expression's first iterable where the
//! expression is, and everything else -- conditions, later iterables, the
//! element -- lazily, reading the enclosing function's variables as they
//! are when each value is produced. The helper receives the first
//! iterable's value as an argument (for `range`/`enumerate`/`zip`/
//! `reversed` and `.items()`/`.keys()`/`.values()`, the call's arguments or
//! receiver, the call itself rebuilt inside), so it is evaluated at the
//! same point. Every other enclosing local is passed by value, which reads
//! the same value CPython would only when the variable cannot change before
//! the generator runs: the expression is consumed by the builtin it is
//! passed to (`any`, `all`, `next`, the materializing consumers) with no
//! code of the enclosing function running in between, or each such local
//! is a parameter never rebound or a variable assigned once by a top-level
//! statement of the function. Otherwise the expression is left alone.
//! Module globals stay free names of the helper and are read live, as
//! CPython reads them.

use super::super::build::{call_kw, call_name, load};
use super::{Lift, free_names, liftable, nest};
use ruff_python_ast::{AtomicNodeIndex, Expr, ExprGenerator, ExprYield, Keyword, Stmt};
use ruff_text_size::{Ranged, TextRange};
use std::collections::BTreeSet;

/// The builtins that run a generator argument to completion (or to its
/// first deciding value) before the caller continues.
pub(super) fn is_consumer(name: &str) -> bool {
    matches!(name, "any" | "all" | "next") || super::super::comps::is_materializing_consumer(name)
}

impl Lift<'_> {
    /// `(elt for ...)` -> a generator helper call. `consumed`: the
    /// expression is an argument of a consuming builtin.
    pub(super) fn generator_expression(&self, expr: &Expr, consumed: bool) -> Option<Expr> {
        let Expr::Generator(generator) = expr else {
            return None;
        };
        if generator.generators.iter().any(|g| g.is_async) || !liftable(expr) {
            return None;
        }
        let range = expr.range();
        let (carried, first_iter) = self.carry_first_iterable(&generator.generators[0].iter, range);
        let mut rest = generator.clone();
        rest.generators[0].iter = first_iter;
        let rest = Expr::Generator(rest);
        let mut reads = BTreeSet::new();
        free_names(&rest, &mut reads);
        let captured: Vec<String> = reads.into_iter().filter(|n| self.is_local(n)).collect();
        if !consumed && !captured.iter().all(|n| self.is_stable(n)) {
            return None;
        }
        let Expr::Generator(ExprGenerator { elt, generators, .. }) = rest else {
            unreachable!()
        };
        let mut loops = vec![Stmt::Expr(ruff_python_ast::StmtExpr {
            node_index: AtomicNodeIndex::default(),
            range,
            value: Box::new(Expr::Yield(ExprYield {
                node_index: AtomicNodeIndex::default(),
                range,
                value: Some(elt),
            })),
        })];
        for g in generators.iter().rev() {
            loops = nest(g, loops, range);
        }
        let mut params: Vec<String> = carried.iter().map(|(name, _)| name.clone()).collect();
        params.extend(captured.iter().cloned());
        let name = self.helper_name("genexpr");
        self.emit(&name, &params, loops, range);
        let mut args: Vec<Expr> = carried.into_iter().map(|(_, value)| value).collect();
        args.extend(captured.iter().map(|n| load(n, range)));
        Some(call_name(&name, args, range))
    }

    /// The first iterable's values evaluated where the expression is, as
    /// `(parameter, value)` pairs, and the iterable rebuilt from the
    /// parameters inside the helper.
    fn carry_first_iterable(&self, iter: &Expr, range: TextRange) -> (Vec<(String, Expr)>, Expr) {
        let mut carried = Vec::new();
        let mut carry = |value: &Expr| {
            let name = self.fresh.name();
            carried.push((name.clone(), value.clone()));
            load(&name, range)
        };
        if let Expr::Call(c) = iter {
            match c.func.as_ref() {
                Expr::Name(f)
                    if matches!(f.id.as_str(), "range" | "enumerate" | "zip" | "reversed")
                        && !self.is_local(f.id.as_str())
                        && !self.module_names.contains(f.id.as_str())
                        && c.arguments.args.iter().all(|a| !matches!(a, Expr::Starred(_)))
                        && c.arguments.keywords.iter().all(|k| k.arg.is_some()) =>
                {
                    let args: Vec<Expr> = c.arguments.args.iter().map(&mut carry).collect();
                    let keywords: Vec<Keyword> = c
                        .arguments
                        .keywords
                        .iter()
                        .map(|k| Keyword {
                            value: carry(&k.value),
                            ..k.clone()
                        })
                        .collect();
                    let rebuilt = call_kw((*c.func).clone(), args, keywords, range);
                    return (carried, rebuilt);
                }
                Expr::Attribute(a)
                    if matches!(a.attr.as_str(), "items" | "keys" | "values")
                        && c.arguments.args.is_empty()
                        && c.arguments.keywords.is_empty() =>
                {
                    let receiver = carry(&a.value);
                    let mut rebuilt = c.clone();
                    if let Expr::Attribute(a) = rebuilt.func.as_mut() {
                        *a.value = receiver;
                    }
                    return (carried, Expr::Call(rebuilt));
                }
                _ => {}
            }
        }
        let rebuilt = carry(iter);
        (carried, rebuilt)
    }
}
