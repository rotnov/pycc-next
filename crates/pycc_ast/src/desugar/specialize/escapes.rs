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
//! keeps the value inside builtin code:
//! - arithmetic, a unary operation or a comparison (not `is`) whose
//!   operands are all provably native: specialized parameters, literals,
//!   items of native sequences, or such operations on them;
//! - an item, a slice, an index or a slice bound of a native `str` or
//!   `bytearray`, or the `str` an `in` searches;
//! - a test (`if x:`), or a loop over a native sequence;
//! - `len`, `ord` or `range` of native values, unless a module binding, a
//!   formal parameter or a binding in the body shadows the name;
//! - a native `str`'s method called on it, and its native arguments but to
//!   `format`.
//!
//! Anything else (another builtin, an operand of unknown type, which may
//! dispatch to a Python-defined method) reads the object. Whether the
//! result of an allowed operation is a new object or its operand
//! (`s.strip()` of a string with nothing to strip) is CPython's
//! implementation detail, which the native value need not reproduce. A
//! `bytearray` parameter's native form is the object itself, and needs no
//! such parameter.
//!
//! That holds only while the parameter names the argument, so a parameter
//! the body may bind again ([`rebound`]) is not specialized: its observed
//! type says nothing of the values bound to it later, which the native slot
//! would refuse.

use ruff_python_ast::visitor::transformer::{self, Transformer};
use ruff_python_ast::visitor::{self, Visitor};
use ruff_python_ast::{CmpOp, Comprehension, ExceptHandler, Expr, ExprContext, Pattern, Stmt};
use std::cell::RefCell;
use std::collections::BTreeSet;

/// The prefix of the parameter that carries a specialized parameter's
/// original object.
pub(super) const OBJECT_PREFIX: &str = "_pycc_object_";

/// Whether `body` may bind `param` anywhere, nested scopes included: an
/// assignment, augmented assignment, `for`, `with` or comprehension target,
/// walrus or `del` of it, an `except ... as` or `case` capture, a `global`
/// or `nonlocal` declaration, or a nested function, lambda or class binding
/// the name. Conservative: a nested scope's own local of that name counts.
pub(super) fn rebound(body: &[Stmt], param: &str) -> bool {
    struct Find<'n> {
        name: &'n str,
        found: bool,
    }
    impl<'a> Visitor<'a> for Find<'_> {
        fn visit_stmt(&mut self, stmt: &'a Stmt) {
            let names = match stmt {
                Stmt::Global(s) => Some(&s.names),
                Stmt::Nonlocal(s) => Some(&s.names),
                _ => None,
            };
            if names.is_some_and(|names| names.iter().any(|n| n.as_str() == self.name)) {
                self.found = true;
            }
            visitor::walk_stmt(self, stmt);
        }
        fn visit_expr(&mut self, expr: &'a Expr) {
            if let Expr::Lambda(lambda) = expr
                && lambda
                    .parameters
                    .as_ref()
                    .is_some_and(|p| p.iter().any(|p| p.name().as_str() == self.name))
            {
                self.found = true;
            }
            visitor::walk_expr(self, expr);
        }
        fn visit_except_handler(&mut self, handler: &'a ExceptHandler) {
            let ExceptHandler::ExceptHandler(h) = handler;
            if h.name.as_ref().is_some_and(|n| n.as_str() == self.name) {
                self.found = true;
            }
            visitor::walk_except_handler(self, handler);
        }
        fn visit_pattern(&mut self, pattern: &'a Pattern) {
            let name = match pattern {
                Pattern::MatchAs(p) => p.name.as_ref(),
                Pattern::MatchStar(p) => p.name.as_ref(),
                Pattern::MatchMapping(p) => p.rest.as_ref(),
                _ => None,
            };
            if name.is_some_and(|n| n.as_str() == self.name) {
                self.found = true;
            }
            visitor::walk_pattern(self, pattern);
        }
    }
    if super::super::ext_generators::bindings(body, param) > 0 {
        return true;
    }
    let mut find = Find {
        name: param,
        found: false,
    };
    for stmt in body {
        find.visit_stmt(stmt);
    }
    find.found
}
/// Makes every use of a `params` parameter in `body` read its original
/// object unless the use provably cannot let the value out (module doc),
/// and returns the parameters whose object it reads, in `params` order.
/// `bytearray` parameters keep their native form everywhere. A builtin
/// counts as one only if `shadowed` (the module's bindings and the
/// function's formal parameters) does not name it and `body` does not bind
/// it.
pub(super) fn keep_objects(
    body: &mut [Stmt],
    params: &[(String, &'static str)],
    shadowed: &BTreeSet<&str>,
) -> Vec<String> {
    let builtins = PROVEN_BUILTINS
        .into_iter()
        .filter(|f| !shadowed.contains(f) && super::super::ext_generators::bindings(body, f) == 0)
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

    /// Visits `expr` where a native value of it cannot leave: a parameter
    /// read right there keeps its native value.
    fn keep(&self, expr: &mut Expr) {
        if self.param(expr).is_none() {
            self.visit_expr(expr);
        }
    }

    /// Visits a test: the truth of a native value is its value's.
    fn test(&self, expr: &mut Expr) {
        if self.is_native(expr) {
            self.keep(expr);
        } else {
            self.visit_expr(expr);
        }
    }

    /// Visits a loop's iterable: iterating a native sequence yields new
    /// items only.
    fn iterable(&self, expr: &mut Expr) {
        if self.is_sequence(expr) {
            self.keep(expr);
        } else {
            self.visit_expr(expr);
        }
    }
}

impl Transformer for Objects {
    fn visit_stmt(&self, stmt: &mut Stmt) {
        match stmt {
            Stmt::If(s) => {
                self.test(&mut s.test);
                self.visit_body(&mut s.body);
                for clause in &mut s.elif_else_clauses {
                    if let Some(test) = &mut clause.test {
                        self.test(test);
                    }
                    self.visit_body(&mut clause.body);
                }
            }
            Stmt::While(s) => {
                self.test(&mut s.test);
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
                self.test(&mut s.test);
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
            self.test(test);
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
        match expr {
            Expr::BinOp(e) if self.is_native(&e.left) && self.is_native(&e.right) => {
                self.keep(&mut e.left);
                self.keep(&mut e.right);
            }
            Expr::UnaryOp(e) if self.is_native(&e.operand) => self.keep(&mut e.operand),
            Expr::If(e) => {
                self.test(&mut e.test);
                self.visit_expr(&mut e.body);
                self.visit_expr(&mut e.orelse);
            }
            Expr::Subscript(e) if self.is_sequence(&e.value) => {
                self.keep(&mut e.value);
                match e.slice.as_mut() {
                    Expr::Slice(slice) => {
                        for bound in [&mut slice.lower, &mut slice.upper, &mut slice.step].into_iter().flatten() {
                            if self.is_native(bound) {
                                self.keep(bound);
                            } else {
                                self.visit_expr(bound);
                            }
                        }
                    }
                    index if self.is_native(index) => self.keep(index),
                    index => self.visit_expr(index),
                }
            }
            Expr::Compare(e) if !e.ops.iter().any(|op| matches!(op, CmpOp::Is | CmpOp::IsNot)) => {
                let all_native =
                    self.is_native(&e.left) && e.comparators.iter().all(|operand| self.is_native(operand));
                if all_native {
                    self.keep(&mut e.left);
                    for operand in e.comparators.iter_mut() {
                        self.keep(operand);
                    }
                } else if matches!(*e.ops, [CmpOp::In | CmpOp::NotIn]) && self.is_text(&e.comparators[0]) {
                    // `str.__contains__` takes no method of the item it looks for.
                    self.visit_expr(&mut e.left);
                    self.keep(&mut e.comparators[0]);
                } else {
                    transformer::walk_expr(self, expr);
                }
            }
            Expr::Call(call) if self.proven_call(call) => {
                for arg in &mut call.arguments.args {
                    self.keep(arg);
                }
            }
            Expr::Call(call)
                if matches!(call.func.as_ref(), Expr::Attribute(method) if self.is_text(&method.value)) =>
            {
                let Expr::Attribute(method) = call.func.as_mut() else {
                    unreachable!("matched above")
                };
                let formatting = FORMATTING_METHODS.contains(&method.attr.as_str());
                self.keep(&mut method.value);
                for arg in &mut call.arguments.args {
                    if !formatting && self.is_native(arg) {
                        self.keep(arg);
                    } else {
                        self.visit_expr(arg);
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
