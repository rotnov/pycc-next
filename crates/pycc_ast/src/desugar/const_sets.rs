//! Non-strict mode: a constant set operand of a comparison in a function
//! (`set(bytearray(t)) <= set(range(65, 91)) | set(range(97, 123))`) is
//! built once, as a module-level constant, rather than at every call.
//!
//! The operand is a set made of literals only: a `set`/`frozenset` display
//! of `int`/`str` literals, `set(...)`/`frozenset(...)` of such a display,
//! a string literal or a `range(...)` of `int` literals, and `|`, `&`, `-`
//! and `^` of those. Its value is the same at every evaluation, and a
//! comparison (`<`, `<=`, `>`, `>=`, `==`, `!=`, `in`, `not in`) only reads
//! it, so evaluating it once first gives every test the same result; it
//! raises nothing; and a comparison cannot tell a `set` from a `frozenset`
//! of the same elements, so the constant is a `frozenset`, as CPython makes
//! the constant of `x in {1, 2}`. A comparison of a CPython object with it
//! boxes it once (`pycc_rt::set`'s cached CPython copy of a frozenset), so
//! the operand costs nothing per call.
//!
//! Nothing is hoisted when the module binds `set`, `frozenset` or `range`
//! anywhere (the constant is read at module level, outside every scope
//! that could shadow them).

use super::build::{assign, call, name};
use super::stored_names;
use ruff_python_ast::visitor::transformer::{self, Transformer};
use ruff_python_ast::{CmpOp, Expr, ExprContext, Number, Operator, Stmt};
use ruff_text_size::TextRange;
use std::cell::RefCell;

/// The prefix of a hoisted constant's name.
const PREFIX: &str = "__pycc_const_set_";

/// Hoists every such operand of a function body in `body` (module doc).
pub(crate) fn hoist_constant_sets(body: &mut Vec<Stmt>) {
    if !super::dataclasses::is_non_strict() {
        return;
    }
    let stored = stored_names(body);
    if ["set", "frozenset", "range"]
        .iter()
        .any(|builtin| stored.contains(*builtin))
        || stored.iter().any(|name| name.starts_with(PREFIX))
    {
        return;
    }
    let hoist = Hoist {
        constants: RefCell::new(Vec::new()),
    };
    for stmt in body.iter_mut() {
        hoist.definitions(stmt);
    }
    let constants = hoist.constants.into_inner();
    if constants.is_empty() {
        return;
    }
    // After the docstring and the `__future__` imports.
    let mut at = usize::from(matches!(body.first(), Some(Stmt::Expr(e))
        if e.value.is_string_literal_expr()));
    while matches!(body.get(at), Some(Stmt::ImportFrom(import))
        if import.module.as_ref().map(|m| m.as_str()) == Some("__future__"))
    {
        at += 1;
    }
    for (offset, (id, value)) in constants.into_iter().enumerate() {
        let target = name(&id, ExprContext::Store, TextRange::default());
        body.insert(at + offset, assign(target, value, TextRange::default()));
    }
}

struct Hoist {
    /// The hoisted constants: name and value, in order.
    constants: RefCell<Vec<(String, Expr)>>,
}

impl Hoist {
    /// Rewrites the function bodies `stmt` defines (a `def`, or the
    /// methods of a `class`, at any depth).
    fn definitions(&self, stmt: &mut Stmt) {
        match stmt {
            Stmt::FunctionDef(def) => {
                for stmt in def.body.iter_mut() {
                    self.visit_stmt(stmt);
                }
            }
            Stmt::ClassDef(class) => {
                for stmt in class.body.iter_mut() {
                    self.definitions(stmt);
                }
            }
            _ => {}
        }
    }

    /// The name of a constant holding `value`.
    fn constant(&self, value: &Expr) -> Expr {
        let mut constants = self.constants.borrow_mut();
        let id = format!("{PREFIX}{}", constants.len());
        // A `frozenset`: immutable, it keeps the CPython copy a comparison
        // with an object boxes it into (`pycc_rt::set`'s `foreign`).
        let range = ruff_text_size::Ranged::range(value);
        let frozen = call(
            name("frozenset", ExprContext::Load, range),
            vec![value.clone()],
            range,
        );
        constants.push((id.clone(), frozen));
        name(&id, ExprContext::Load, ruff_text_size::Ranged::range(value))
    }
}

impl Transformer for Hoist {
    fn visit_stmt(&self, stmt: &mut Stmt) {
        match stmt {
            Stmt::FunctionDef(_) | Stmt::ClassDef(_) => self.definitions(stmt),
            other => transformer::walk_stmt(self, other),
        }
    }

    fn visit_expr(&self, expr: &mut Expr) {
        transformer::walk_expr(self, expr);
        let Expr::Compare(compare) = expr else {
            return;
        };
        let [op] = compare.ops.as_ref() else {
            return;
        };
        if !matches!(
            op,
            CmpOp::Lt
                | CmpOp::LtE
                | CmpOp::Gt
                | CmpOp::GtE
                | CmpOp::Eq
                | CmpOp::NotEq
                | CmpOp::In
                | CmpOp::NotIn
        ) {
            return;
        }
        // The container of `in` is the right operand only.
        if !matches!(op, CmpOp::In | CmpOp::NotIn) && is_constant_set(&compare.left) {
            *compare.left = self.constant(&compare.left);
        }
        if let Some(right) = compare.comparators.first_mut()
            && is_constant_set(right)
            && !is_display(right)
        {
            *right = self.constant(right);
        }
    }
}

/// A set display, which `pycc_hir` already builds as a constant of its own.
fn is_display(expr: &Expr) -> bool {
    matches!(expr, Expr::Set(_))
}

/// Whether `expr` is a set made of literals only (module doc).
fn is_constant_set(expr: &Expr) -> bool {
    match expr {
        Expr::Set(set) => set.elts.iter().all(is_literal),
        Expr::Call(call)
            if call.arguments.keywords.is_empty() && call.arguments.args.len() == 1 =>
        {
            matches!(call.func.as_ref(), Expr::Name(n) if matches!(n.id.as_str(), "set" | "frozenset"))
                && is_constant_iterable(&call.arguments.args[0])
        }
        Expr::BinOp(bin) => {
            matches!(
                bin.op,
                Operator::BitOr | Operator::BitAnd | Operator::Sub | Operator::BitXor
            ) && is_constant_set(&bin.left)
                && is_constant_set(&bin.right)
        }
        _ => false,
    }
}

/// An iterable of literals: a display of them, a string literal, or a
/// `range(...)` of `int` literals.
fn is_constant_iterable(expr: &Expr) -> bool {
    match expr {
        Expr::Set(set) => set.elts.iter().all(is_literal),
        Expr::List(list) => list.elts.iter().all(is_literal),
        Expr::Tuple(tuple) => tuple.elts.iter().all(is_literal),
        Expr::StringLiteral(_) => true,
        Expr::Call(call) => {
            matches!(call.func.as_ref(), Expr::Name(n) if n.id.as_str() == "range")
                && call.arguments.keywords.is_empty()
                && (1..=3).contains(&call.arguments.args.len())
                && call.arguments.args.iter().all(is_int)
        }
        _ => false,
    }
}

/// An `int` or `str` literal.
fn is_literal(expr: &Expr) -> bool {
    is_int(expr) || matches!(expr, Expr::StringLiteral(_))
}

/// An `int` literal, or a negated one.
fn is_int(expr: &Expr) -> bool {
    match expr {
        Expr::NumberLiteral(n) => matches!(n.value, Number::Int(_)),
        Expr::UnaryOp(u) => {
            u.op == ruff_python_ast::UnaryOp::USub
                && matches!(u.operand.as_ref(), Expr::NumberLiteral(n) if matches!(n.value, Number::Int(_)))
        }
        _ => false,
    }
}
