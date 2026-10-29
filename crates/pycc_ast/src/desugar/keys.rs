//! `key=` and `reverse=` arguments of `sorted`, `min`, `max` and
//! `list.sort`.
//!
//! CPython evaluates the iterable once, calls the key function once per
//! element in order, and then orders the elements by their keys (stably,
//! and for `reverse=True` keeping equal keys in their original order). The
//! rewrite does exactly that with one list comprehension of `(key, element)`
//! pairs -- a lambda inlined with its parameter as the comprehension
//! variable -- handed to a native builtin that orders by the first item and
//! keeps the second:
//!
//! * `sorted(xs, key=f, reverse=r)` -> `__pycc_sorted_keyed([(f(v), v) for v in xs], r)`
//! * `xs.sort(key=f, reverse=r)` -> `__pycc_sort_keyed(xs, [(f(v), v) for v in xs], r)`
//!   (`xs` a name or attribute chain, so reading it twice is unobservable)
//! * `min(xs, key=f)` / `max(xs, key=f)` -> `__pycc_min_keyed([(f(v), v) for v in xs])`

use super::Fresh;
use super::build::{call, call_name, list_comp, load, store, tuple};
use ruff_python_ast::{Expr, ExprCall, ExprContext};
use ruff_text_size::TextRange;

fn not(operand: Expr, range: TextRange) -> Expr {
    Expr::UnaryOp(ruff_python_ast::ExprUnaryOp {
        node_index: Default::default(),
        range,
        op: ruff_python_ast::UnaryOp::Not,
        operand: Box::new(operand),
    })
}

/// Whether `expr` can be evaluated twice with no observable difference: a
/// name, an attribute chain over one, or a subscript of one by a name or
/// literal.
fn is_rereadable(expr: &Expr) -> bool {
    match expr {
        Expr::Name(_) => true,
        Expr::Attribute(attr) => is_rereadable(&attr.value),
        // `g[i].sort(reverse=True)`: a name or literal index reads the same
        // item each time.
        Expr::Subscript(sub) => {
            is_rereadable(&sub.value)
                && matches!(sub.slice.as_ref(), Expr::Name(_) | Expr::NumberLiteral(_))
        }
        _ => false,
    }
}

/// `f` applied to a fresh comprehension variable: a one-parameter lambda is
/// inlined (its parameter becomes the variable), a name or attribute is
/// called, and so is a nested function's lifted function. `None` for
/// anything else.
pub(super) fn apply(f: &Expr, fresh: &Fresh, range: TextRange) -> Option<(String, Expr)> {
    match f {
        Expr::Lambda(lambda) => {
            let params = lambda.parameters.as_deref()?;
            let [param] = params.args.as_slice() else {
                return None;
            };
            if param.default.is_some()
                || !params.posonlyargs.is_empty()
                || params.vararg.is_some()
                || params.kwarg.is_some()
                || !params.kwonlyargs.is_empty()
            {
                return None;
            }
            Some((param.name().to_string(), (*lambda.body).clone()))
        }
        Expr::Name(_) | Expr::Attribute(_) => {
            let var = fresh.name();
            Some((var.clone(), call(f.clone(), vec![load(&var, range)], range)))
        }
        // A nested function used as a value (`__pycc_closure__(lifted,
        // captures...)`, `super::closures`): its lifted function called with
        // the captures first. Each capture is a name that keeps one value
        // from before the value is made (`closures::values`), so reading it
        // at each call is reading the value the closure bound.
        Expr::Call(closure)
            if matches!(closure.func.as_ref(), Expr::Name(n) if n.id.as_str() == crate::CLOSURE_MARKER)
                && closure.arguments.keywords.is_empty()
                && closure.arguments.args.iter().all(|a| matches!(a, Expr::Name(_))) =>
        {
            let [lifted, captures @ ..] = &*closure.arguments.args else {
                return None;
            };
            let var = fresh.name();
            let mut args: Vec<Expr> = captures.to_vec();
            args.push(load(&var, range));
            Some((var.clone(), call(lifted.clone(), args, range)))
        }
        _ => None,
    }
}

/// The `key`/`reverse` keywords of a call (`key=None` counts as absent), or
/// `None` when it has any other keyword.
fn key_and_reverse(call_expr: &ExprCall) -> Option<(Option<&Expr>, Option<&Expr>)> {
    let mut key = None;
    let mut reverse = None;
    for keyword in &call_expr.arguments.keywords {
        match keyword.arg.as_ref().map(|a| a.as_str()) {
            Some("key") if !matches!(keyword.value, Expr::NoneLiteral(_)) => {
                key = Some(&keyword.value)
            }
            Some("key") => {}
            Some("reverse") => reverse = Some(&keyword.value),
            _ => return None,
        }
    }
    Some((key, reverse))
}

/// `[(f(v), v) for v in xs]` for the key function `key`, or `[(v, v) for v
/// in xs]` when there is none.
fn pairs(xs: &Expr, key: Option<&Expr>, fresh: &Fresh, range: TextRange) -> Option<Expr> {
    let (var, key_of) = match key {
        None => {
            let var = fresh.name();
            let key_of = load(&var, range);
            (var, key_of)
        }
        Some(f) => apply(f, fresh, range)?,
    };
    let pair = tuple(vec![key_of, load(&var, range)], ExprContext::Load, range);
    Some(list_comp(
        pair,
        store(&var, range),
        xs.clone(),
        Vec::new(),
        range,
    ))
}

/// The rewrite of a `sorted`/`min`/`max`/`.sort` call with a `key` or
/// `reverse` keyword; `None` leaves the call alone. `shadowed` answers
/// whether a builtin name is rebound.
pub(super) fn rewrite_key_call(
    call_expr: &ExprCall,
    fresh: &Fresh,
    shadowed: &dyn Fn(&str) -> bool,
) -> Option<Expr> {
    if call_expr.arguments.keywords.is_empty() {
        return None;
    }
    let (key, reverse) = key_and_reverse(call_expr)?;
    // CPython evaluates every argument before calling the key, and the
    // rewrite below calls it while building the pairs, ahead of `reverse`:
    // with a key, only a literal `reverse` (which evaluating cannot be
    // seen) is rewritten.
    if key.is_some()
        && reverse.is_some_and(|r| {
            !matches!(
                r,
                Expr::BooleanLiteral(_) | Expr::NumberLiteral(_) | Expr::NoneLiteral(_)
            )
        })
    {
        return None;
    }
    let range = call_expr.range;
    let reverse = reverse.cloned().unwrap_or_else(|| {
        Expr::BooleanLiteral(ruff_python_ast::ExprBooleanLiteral {
            node_index: Default::default(),
            range,
            value: false,
        })
    });
    // `not not r` is `bool(r)`, which the native builtins take.
    let reverse = match reverse {
        literal @ Expr::BooleanLiteral(_) => literal,
        other => not(not(other, range), range),
    };
    match call_expr.func.as_ref() {
        Expr::Name(func) if !shadowed(func.id.as_str()) => {
            let [xs] = &*call_expr.arguments.args else {
                return None;
            };
            if matches!(xs, Expr::Starred(_)) {
                return None;
            }
            match func.id.as_str() {
                "sorted" => {
                    let pairs = pairs(xs, key, fresh, range)?;
                    Some(call_name(
                        "__pycc_sorted_keyed",
                        vec![pairs, reverse],
                        range,
                    ))
                }
                "min" | "max" if key.is_some() && call_expr.arguments.keywords.len() == 1 => {
                    let pairs = pairs(xs, key, fresh, range)?;
                    let name = if func.id.as_str() == "min" {
                        "__pycc_min_keyed"
                    } else {
                        "__pycc_max_keyed"
                    };
                    Some(call_name(name, vec![pairs], range))
                }
                _ => None,
            }
        }
        Expr::Attribute(attr)
            if attr.attr.as_str() == "sort" && call_expr.arguments.args.is_empty() =>
        {
            let xs = attr.value.as_ref();
            if !is_rereadable(xs) {
                return None;
            }
            let pairs = pairs(xs, key, fresh, range)?;
            Some(call_name(
                "__pycc_sort_keyed",
                vec![xs.clone(), pairs, reverse],
                range,
            ))
        }
        _ => None,
    }
}
