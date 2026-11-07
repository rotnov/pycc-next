//! What keeps an observed-type specialization ([`super::specialize_observed`])
//! the function it copies, though its parameters hold native values.
//!
//! A native `int`, `float` or `str` parameter is a converted copy of the
//! argument: a value of it that reaches Python code as an object (returned,
//! stored, passed on, put in a container, compared with `is`, handed to
//! another operand's reflected method, hashed) would be a new object, which
//! `is` tells from the argument, and which a `float('nan')` argument does
//! not even equal inside a container (`[f(x)] == [x]` compares items by
//! identity first) nor hash alike. So the copy takes each such parameter's
//! original object as well, in a parameter of its own, and every use of the
//! parameter reads that object ([`keep_objects`]) unless the use provably
//! keeps the value inside builtin code, and its result is consumed there
//! or can never be the argument itself:
//! - arithmetic, a unary operation or a comparison (not `is`) whose
//!   operands are all provably native: specialized parameters, literals,
//!   items of native sequences, or such operations on them;
//! - an item, a slice, an index or a slice bound of a native `str` or
//!   `bytearray`, and the `str` an `in` searches;
//! - a test (`if x:`), or a loop over a native sequence;
//! - `len`, `ord` or `range` of native values, unless a binding of the name
//!   anywhere (module, formal parameter, body) shadows it;
//! - a native `str`'s method called on it, and its native arguments but to
//!   `format`.
//!
//! CPython returns the operand itself from some of these: `+x` of an exact
//! `int` or `float`, the full slice `s[:]`, `s + ''`, `s * 1`, a `str`
//! method with nothing to change (`s.strip()`, `s.split()`'s one item). So
//! where the result of such an operation may itself leave the function,
//! its operands read their objects too ([`Objects::visit_in`]); only an
//! operation whose result is a new value (a comparison, `-x`, `len`, an
//! item, `s.find(t)`) consumes its operands whatever becomes of the result.
//! An item of a `str` is at most a one-character string CPython caches,
//! which boxing the native item gives as well.
//!
//! Anything else (another builtin, `hash`, an operand of unknown type,
//! which may dispatch to a Python-defined method) reads the object. A
//! `bytearray` parameter's native form is the object itself, and needs no
//! such parameter.
//!
//! That holds only while the parameter names the argument, so a parameter
//! the body may bind again (`crate::namespace::bound_anywhere`) is not specialized: its observed
//! type says nothing of the values bound to it later, which the native slot
//! would refuse.

use ruff_python_ast::visitor::transformer::{self, Transformer};
use ruff_python_ast::{CmpOp, Comprehension, Expr, ExprContext, Stmt};
use std::cell::RefCell;
use std::collections::BTreeSet;

/// The prefix of the parameter that carries a specialized parameter's
/// original object.
pub(super) const OBJECT_PREFIX: &str = "_pycc_object_";

/// Makes every use of a `params` parameter in `body` read its original
/// object unless the use provably cannot let the value out (module doc),
/// and returns the parameters whose object it reads, in `params` order.
/// `bytearray` parameters keep their native form everywhere. A builtin
/// counts as one only if `shadowed` (every name the module's scope, the
/// function's formal parameters or any binding in its body binds) does
/// not name it.
pub(super) fn keep_objects(
    body: &mut [Stmt],
    params: &[(String, &'static str)],
    shadowed: &BTreeSet<&str>,
) -> Vec<String> {
    let builtins = PROVEN_BUILTINS
        .into_iter()
        .filter(|f| !shadowed.contains(f))
        .collect();
    let objects = Objects {
        builtins,
        params: params.to_vec(),
        used: RefCell::new(BTreeSet::new()),
    };
    for stmt in body.iter_mut() {
        objects.visit_stmt(stmt);
    }
    let used = objects.used.into_inner();
    params
        .iter()
        .map(|(param, _)| param)
        .filter(|param| used.contains(param.as_str()))
        .cloned()
        .collect()
}

/// The builtins whose result, for native arguments, depends on their values
/// alone: `len` of a `str` or `bytearray`, `ord` of a `str`, `range` of
/// `int`s. (`hash` is not one: a NaN hashes by identity.)
pub(super) const PROVEN_BUILTINS: [&str; 3] = ["len", "ord", "range"];

/// The `str` methods whose result is a number, a `bool` or `bytes`, so
/// never the receiver or an argument itself.
const VALUE_METHODS: [&str; 21] = [
    "count", "encode", "endswith", "find", "index", "isalnum", "isalpha", "isascii",
    "isdecimal", "isdigit", "isidentifier", "islower", "isnumeric", "isprintable",
    "isspace", "istitle", "isupper", "rfind", "rindex", "startswith", "__len__",
];

/// The `str` methods whose arguments may not keep their native values:
/// formatting reaches the arguments' `__format__`.
const FORMATTING_METHODS: [&str; 2] = ["format", "format_map"];

struct Objects {
    /// The [`PROVEN_BUILTINS`] the body calls as builtins.
    builtins: Vec<&'static str>,
    /// The specialized parameters and their types.
    params: Vec<(String, &'static str)>,
    used: RefCell<BTreeSet<String>>,
}

impl Objects {
    /// The type of the specialized parameter `expr` reads, if it reads one.
    fn param(&self, expr: &Expr) -> Option<&'static str> {
        let Expr::Name(n) = expr else {
            return None;
        };
        if n.ctx != ExprContext::Load {
            return None;
        }
        self.params
            .iter()
            .find(|(param, _)| param == n.id.as_str())
            .map(|(_, ty)| *ty)
    }

    /// Whether `expr` is provably an exact `int`, `float`, `str` or
    /// `bytearray` (or raises as one does), so an operation on it and a
    /// native value runs no Python-defined method: a specialized
    /// parameter, a number or string literal, an item or slice of a native
    /// sequence, or arithmetic, a unary operation or a proven builtin call
    /// on such values.
    fn is_native(&self, expr: &Expr) -> bool {
        match expr {
            Expr::NumberLiteral(n) => !matches!(n.value, ruff_python_ast::Number::Complex { .. }),
            Expr::StringLiteral(_) => true,
            Expr::BinOp(e) => self.is_native(&e.left) && self.is_native(&e.right),
            Expr::UnaryOp(e) => self.is_native(&e.operand),
            Expr::Subscript(e) => self.is_sequence(&e.value),
            Expr::Call(call) => self.proven_call(call),
            _ => self.param(expr).is_some(),
        }
    }

    /// Whether `expr` is provably an exact `str` or `bytearray`.
    fn is_sequence(&self, expr: &Expr) -> bool {
        match expr {
            Expr::StringLiteral(_) => true,
            Expr::Subscript(e) => {
                self.is_text(&e.value) || (self.is_sequence(&e.value) && matches!(e.slice.as_ref(), Expr::Slice(_)))
            }
            _ => matches!(self.param(expr), Some("str" | "bytearray")),
        }
    }

    /// Whether `expr` is provably an exact `str`.
    fn is_text(&self, expr: &Expr) -> bool {
        match expr {
            Expr::StringLiteral(_) => true,
            Expr::Subscript(e) => self.is_text(&e.value),
            _ => self.param(expr) == Some("str"),
        }
    }

    /// Whether `call` is a [`PROVEN_BUILTINS`] call with native positional
    /// arguments only.
    fn proven_call(&self, call: &ruff_python_ast::ExprCall) -> bool {
        matches!(call.func.as_ref(), Expr::Name(f) if self.builtins.contains(&f.id.as_str()))
            && call.arguments.keywords.is_empty()
            && call.arguments.args.iter().all(|arg| self.is_native(arg))
    }

    /// Visits a loop's iterable: iterating a native sequence yields new
    /// items, or one-character strings CPython caches, never the sequence.
    fn iterable(&self, expr: &mut Expr) {
        let native = self.is_sequence(expr);
        self.visit_in(expr, !native);
    }

    /// Visits `expr`, whose value leaves (`escapes`: is returned, stored,
    /// passed on, ...) or is consumed where it stands. A parameter read
    /// there keeps its native value only when consumed; an operation whose
    /// result may be its operand itself (`+x`, `s[:]`, `s.strip()`,
    /// `s + ''`) passes its own `escapes` on to that operand, and one whose
    /// result never is (a comparison, `len`, an item) consumes it.
    fn visit_in(&self, expr: &mut Expr, escapes: bool) {
        if self.param(expr).is_some() {
            if escapes {
                self.visit_expr(expr);
            }
            return;
        }
        match expr {
            Expr::BinOp(e) if self.is_native(&e.left) && self.is_native(&e.right) => {
                // `s + ''`, `s * 1`, `x + 0` may be the operand itself.
                self.visit_in(&mut e.left, escapes);
                self.visit_in(&mut e.right, escapes);
            }
            Expr::UnaryOp(e) if self.is_native(&e.operand) => {
                // `+x` is an exact `int` or `float` `x` itself; `-x`, `~x`
                // and `not x` are new values.
                let same = matches!(e.op, ruff_python_ast::UnaryOp::UAdd);
                self.visit_in(&mut e.operand, escapes && same);
            }
            Expr::If(e) => {
                self.visit_in(&mut e.test, false);
                self.visit_in(&mut e.body, escapes);
                self.visit_in(&mut e.orelse, escapes);
            }
            Expr::Subscript(e) if self.is_sequence(&e.value) => match e.slice.as_mut() {
                Expr::Slice(slice) => {
                    // `s[:]` is `s` itself.
                    self.visit_in(&mut e.value, escapes);
                    for bound in [&mut slice.lower, &mut slice.upper, &mut slice.step].into_iter().flatten() {
                        let native = self.is_native(bound);
                        self.visit_in(bound, !native);
                    }
                }
                index => {
                    // An item is a new value, or a one-character string
                    // CPython caches, which boxing the native item gives too.
                    let native = self.is_native(index);
                    self.visit_in(index, !native);
                    self.visit_in(&mut e.value, false);
                }
            },
            Expr::Compare(e) if !e.ops.iter().any(|op| matches!(op, CmpOp::Is | CmpOp::IsNot)) => {
                let all_native =
                    self.is_native(&e.left) && e.comparators.iter().all(|operand| self.is_native(operand));
                if all_native {
                    self.visit_in(&mut e.left, false);
                    for operand in e.comparators.iter_mut() {
                        self.visit_in(operand, false);
                    }
                } else if matches!(*e.ops, [CmpOp::In | CmpOp::NotIn]) && self.is_text(&e.comparators[0]) {
                    // `str.__contains__` takes no method of the item it looks for.
                    self.visit_expr(&mut e.left);
                    self.visit_in(&mut e.comparators[0], false);
                } else {
                    transformer::walk_expr(self, expr);
                }
            }
            Expr::Call(call) if self.proven_call(call) => {
                for arg in &mut call.arguments.args {
                    self.visit_in(arg, false);
                }
            }
            Expr::Call(call)
                if matches!(call.func.as_ref(), Expr::Attribute(method) if self.is_text(&method.value)) =>
            {
                let Expr::Attribute(method) = call.func.as_mut() else {
                    unreachable!("matched above")
                };
                let formatting = FORMATTING_METHODS.contains(&method.attr.as_str());
                // A `str` method may return its receiver (`s.strip()` with
                // nothing to strip, `s.split()` holding `s`) or a `str`
                // argument (`''.join(t)`), unless its result is a value.
                let escapes = escapes && !VALUE_METHODS.contains(&method.attr.as_str());
                self.visit_in(&mut method.value, escapes);
                for arg in &mut call.arguments.args {
                    match self.param(arg) {
                        _ if formatting || !self.is_native(arg) => self.visit_expr(arg),
                        Some("int" | "float") => {}
                        _ => self.visit_in(arg, escapes),
                    }
                }
                for keyword in &mut call.arguments.keywords {
                    self.visit_expr(&mut keyword.value);
                }
            }
            _ => transformer::walk_expr(self, expr),
        }
    }
}

impl Transformer for Objects {
    fn visit_stmt(&self, stmt: &mut Stmt) {
        match stmt {
            Stmt::If(s) => {
                self.visit_in(&mut s.test, false);
                self.visit_body(&mut s.body);
                for clause in &mut s.elif_else_clauses {
                    if let Some(test) = &mut clause.test {
                        self.visit_in(test, false);
                    }
                    self.visit_body(&mut clause.body);
                }
            }
            Stmt::While(s) => {
                self.visit_in(&mut s.test, false);
                self.visit_body(&mut s.body);
                self.visit_body(&mut s.orelse);
            }
            Stmt::For(s) => {
                self.visit_expr(&mut s.target);
                self.iterable(&mut s.iter);
                self.visit_body(&mut s.body);
                self.visit_body(&mut s.orelse);
            }
            Stmt::Assert(s) => {
                self.visit_in(&mut s.test, false);
                if let Some(msg) = &mut s.msg {
                    self.visit_expr(msg);
                }
            }
            _ => transformer::walk_stmt(self, stmt),
        }
    }

    fn visit_comprehension(&self, comprehension: &mut Comprehension) {
        self.visit_expr(&mut comprehension.target);
        self.iterable(&mut comprehension.iter);
        for test in &mut comprehension.ifs {
            self.visit_in(test, false);
        }
    }

    fn visit_expr(&self, expr: &mut Expr) {
        if let Expr::Name(n) = expr
            && n.ctx == ExprContext::Load
            && self
                .params
                .iter()
                .any(|(param, ty)| param == n.id.as_str() && *ty != "bytearray")
        {
            self.used.borrow_mut().insert(n.id.to_string());
            n.id = format!("{OBJECT_PREFIX}{}", n.id).into();
            return;
        }
        if self.param(expr).is_some() {
            return;
        }
        self.visit_in(expr, true);
    }
}
