//! Which functions -- the module's own, lifted nested ones
//! (`super::closures`) among them, builtins and lambdas -- are free of
//! effects on state anything else can see, so a `min`/`max` key or a lazy
//! `map`/`filter` function of one may be called at another moment than
//! CPython calls it (`super::keys::apply`): all the keys before the first
//! comparison, a lazy `map` eagerly. (`sorted` and `.sort` compute every
//! key first in CPython too; a `sorted` nested function key with an effect
//! keys a copy.)
//!
//! A body qualifies when it is `return`s, `if`s, `pass` and assignments to
//! plain names (its own locals: a write to an enclosing variable goes
//! through its cell, `cell[0] = v`, a subscript store), over expressions
//! that are names, literals, operators, comparisons, conditional
//! expressions, displays, subscript and attribute reads, f-strings and
//! calls of the builtins below (none rebound by the program). Anything else
//! -- a call of anything else, a method call, a store into an attribute or
//! item, `global`/`nonlocal`, a loop, `del`, `yield` -- leaves the function
//! out.

use ruff_python_ast::{Expr, Stmt};
use std::collections::HashSet;

/// Builtins whose call has no effect beyond its result.
const PURE_BUILTINS: &[&str] = &[
    "len", "abs", "str", "int", "float", "bool", "ord", "chr", "round", "tuple", "min", "max",
    "isinstance", "repr",
];

/// The names of `body`'s module-level functions that qualify (module doc);
/// `shadowed` are the builtin names the program rebinds.
pub(crate) fn pure_functions(body: &[Stmt], shadowed: &HashSet<String>) -> HashSet<String> {
    body.iter()
        .filter_map(|stmt| match stmt {
            Stmt::FunctionDef(def)
                if def.decorator_list.is_empty()
                    && !def.is_async
                    && def.body.iter().all(|s| pure_stmt(s, shadowed)) =>
            {
                Some(def.name.to_string())
            }
            _ => None,
        })
        .collect()
}

fn pure_stmt(stmt: &Stmt, shadowed: &HashSet<String>) -> bool {
    match stmt {
        Stmt::Return(ret) => ret.value.as_deref().is_none_or(|v| pure_expr(v, shadowed)),
        Stmt::Pass(_) => true,
        Stmt::Expr(e) => matches!(&*e.value, Expr::StringLiteral(_)),
        Stmt::Assign(assign) => {
            assign.targets.iter().all(|t| matches!(t, Expr::Name(_)))
                && pure_expr(&assign.value, shadowed)
        }
        Stmt::AnnAssign(assign) => {
            matches!(&*assign.target, Expr::Name(_))
                && assign.value.as_deref().is_none_or(|v| pure_expr(v, shadowed))
        }
        Stmt::If(s) => {
            pure_expr(&s.test, shadowed)
                && s.body.iter().all(|x| pure_stmt(x, shadowed))
                && s.elif_else_clauses.iter().all(|clause| {
                    clause.test.as_ref().is_none_or(|t| pure_expr(t, shadowed))
                        && clause.body.iter().all(|x| pure_stmt(x, shadowed))
                })
        }
        _ => false,
    }
}

/// Whether `name` is an effect-free builtin the program does not rebind.
pub(crate) fn pure_builtin(name: &str, shadowed: &HashSet<String>) -> bool {
    PURE_BUILTINS.contains(&name) && !shadowed.contains(name)
}

/// Whether evaluating `expr` has no effect anything else can see (module
/// doc).
pub(crate) fn pure_expr(expr: &Expr, shadowed: &HashSet<String>) -> bool {
    let all = |items: &[Expr]| items.iter().all(|e| pure_expr(e, shadowed));
    match expr {
        Expr::Name(_)
        | Expr::NumberLiteral(_)
        | Expr::StringLiteral(_)
        | Expr::BytesLiteral(_)
        | Expr::BooleanLiteral(_)
        | Expr::NoneLiteral(_)
        | Expr::EllipsisLiteral(_) => true,
        Expr::BinOp(e) => pure_expr(&e.left, shadowed) && pure_expr(&e.right, shadowed),
        Expr::UnaryOp(e) => pure_expr(&e.operand, shadowed),
        Expr::BoolOp(e) => all(&e.values),
        Expr::Compare(e) => pure_expr(&e.left, shadowed) && all(&e.comparators),
        Expr::If(e) => {
            pure_expr(&e.test, shadowed)
                && pure_expr(&e.body, shadowed)
                && pure_expr(&e.orelse, shadowed)
        }
        Expr::Tuple(e) => all(&e.elts),
        Expr::List(e) => all(&e.elts),
        Expr::Subscript(e) => pure_expr(&e.value, shadowed) && pure_expr(&e.slice, shadowed),
        Expr::Slice(e) => [&e.lower, &e.upper, &e.step]
            .into_iter()
            .flatten()
            .all(|b| pure_expr(b, shadowed)),
        Expr::Attribute(e) => pure_expr(&e.value, shadowed),
        Expr::FString(f) => f.value.elements().all(|element| match element {
            ruff_python_ast::InterpolatedStringElement::Literal(_) => true,
            ruff_python_ast::InterpolatedStringElement::Interpolation(i) => {
                pure_expr(&i.expression, shadowed)
            }
        }),
        Expr::Call(call) => {
            matches!(call.func.as_ref(), Expr::Name(n) if pure_builtin(n.id.as_str(), shadowed))
                && call.arguments.keywords.is_empty()
                && all(&call.arguments.args)
        }
        _ => false,
    }
}
