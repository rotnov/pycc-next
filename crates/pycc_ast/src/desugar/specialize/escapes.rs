//! What keeps an observed-type specialization ([`super::specialize_observed`])
//! the function it copies, though its parameters hold native values.
//!
//! A native `int`, `float` or `str` parameter is a converted copy of the
//! argument: a value of it that leaves the function as an object (returned,
//! stored, passed on, put in a container, compared with `is`) would be a new
//! object, which `is` tells from the argument, and which a `float('nan')`
//! argument does not even equal inside a container (`[f(x)] == [x]`
//! compares items by identity first). So the copy takes each such
//! parameter's original object as well, in a parameter of its own, and
//! every use of the parameter that may let its value out reads that object
//! ([`keep_objects`]); only the uses that cannot (an operand of arithmetic,
//! an index, `len()`, a loop's iterable, a `str` method's receiver or
//! argument, ...) read the native value. Whether the result of such an
//! operation is a new object or its operand (`s.strip()` of a string with
//! nothing to strip) is CPython's implementation detail, which the native
//! value need not reproduce; that an argument passed on is the argument is
//! the language's. A `bytearray` parameter's native form is the object
//! itself, and needs no such parameter.
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

/// Makes every use of a `params` parameter in `body` that may let its value
/// out read its original object (module doc), and returns the parameters
/// whose object it reads, in `params` order. `bytearray` parameters are
/// left alone. A builtin `shadowed` names, or that `body` binds, is not
/// taken to consume its arguments.
pub(super) fn keep_objects(
    body: &mut [Stmt],
    params: &[(String, &'static str)],
    shadowed: &BTreeSet<&str>,
) -> Vec<String> {
    let consuming = CONSUMING_BUILTINS
        .into_iter()
        .filter(|f| !shadowed.contains(f) && super::super::ext_generators::bindings(body, f) == 0)
        .collect();
    let objects = Objects {
        consuming,
        params: params
            .iter()
            .filter(|(_, ty)| *ty != "bytearray")
            .cloned()
            .collect(),
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

/// The builtins whose result never is (or holds) their argument.
pub(super) const CONSUMING_BUILTINS: [&str; 7] = ["len", "ord", "chr", "range", "bool", "hash", "isinstance"];

struct Objects {
    /// The [`CONSUMING_BUILTINS`] the body calls as builtins.
    consuming: Vec<&'static str>,
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

    /// Visits `expr`, whose value is consumed where it stands: a parameter
    /// read there keeps its native value.
    fn consumed(&self, expr: &mut Expr) {
        if self.param(expr).is_none() {
            self.visit_expr(expr);
        }
    }

    /// As [`Self::consumed`], but where a `float` may still be the object
    /// itself (`x.real`) or is compared by identity first (a key, or the
    /// item `in` looks for), so only a NaN's object finds itself.
    fn consumed_by_value(&self, expr: &mut Expr) {
        match self.param(expr) {
            Some("float") => self.visit_expr(expr),
            Some(_) => {}
            None => self.visit_expr(expr),
        }
    }

    /// Whether `expr` is a `str` (a literal, a `str` parameter or a
    /// subscript of one), whose methods never return or keep an argument.
    fn is_text(&self, expr: &Expr) -> bool {
        match expr {
            Expr::StringLiteral(_) => true,
            Expr::Subscript(sub) => self.is_text(&sub.value),
            _ => self.param(expr) == Some("str"),
        }
    }
}

impl Transformer for Objects {
    fn visit_stmt(&self, stmt: &mut Stmt) {
        match stmt {
            Stmt::If(s) => {
                self.consumed(&mut s.test);
                self.visit_body(&mut s.body);
                for clause in &mut s.elif_else_clauses {
                    if let Some(test) = &mut clause.test {
                        self.consumed(test);
                    }
                    self.visit_body(&mut clause.body);
                }
            }
            Stmt::While(s) => {
                self.consumed(&mut s.test);
                self.visit_body(&mut s.body);
                self.visit_body(&mut s.orelse);
            }
            Stmt::For(s) => {
                self.visit_expr(&mut s.target);
                self.consumed(&mut s.iter);
                self.visit_body(&mut s.body);
                self.visit_body(&mut s.orelse);
            }
            Stmt::AugAssign(s) => {
                self.visit_expr(&mut s.target);
                self.consumed(&mut s.value);
            }
            Stmt::Assert(s) => {
                self.consumed(&mut s.test);
                if let Some(msg) = &mut s.msg {
                    self.visit_expr(msg);
                }
            }
            _ => transformer::walk_stmt(self, stmt),
        }
    }

    fn visit_comprehension(&self, comprehension: &mut Comprehension) {
        self.visit_expr(&mut comprehension.target);
        self.consumed(&mut comprehension.iter);
        for test in &mut comprehension.ifs {
            self.consumed(test);
        }
    }

    fn visit_expr(&self, expr: &mut Expr) {
        if let Expr::Name(n) = expr
            && n.ctx == ExprContext::Load
            && self.params.iter().any(|(param, _)| param == n.id.as_str())
        {
            self.used.borrow_mut().insert(n.id.to_string());
            n.id = format!("{OBJECT_PREFIX}{}", n.id).into();
            return;
        }
        match expr {
            Expr::BinOp(e) => {
                self.consumed(&mut e.left);
                self.consumed(&mut e.right);
            }
            Expr::UnaryOp(e) => self.consumed(&mut e.operand),
            Expr::If(e) => {
                self.consumed(&mut e.test);
                self.visit_expr(&mut e.body);
                self.visit_expr(&mut e.orelse);
            }
            Expr::Subscript(e) => {
                self.consumed(&mut e.value);
                self.consumed_by_value(&mut e.slice);
            }
            Expr::Slice(e) => {
                for bound in [&mut e.lower, &mut e.upper, &mut e.step].into_iter().flatten() {
                    self.consumed(bound);
                }
            }
            // `x.real` is a float `x` itself.
            Expr::Attribute(e) => self.consumed_by_value(&mut e.value),
            Expr::Compare(e) => {
                let ops = e.ops.clone();
                // Operand `i` stands between `ops[i - 1]` and `ops[i]`.
                let side = |i: usize| {
                    let before = i.checked_sub(1).map(|j| ops[j]);
                    let after = ops.get(i).copied();
                    let identity = [before, after]
                        .into_iter()
                        .flatten()
                        .any(|op| matches!(op, CmpOp::Is | CmpOp::IsNot));
                    let sought = matches!(after, Some(CmpOp::In | CmpOp::NotIn));
                    (identity, sought)
                };
                let operands = std::iter::once(e.left.as_mut()).chain(e.comparators.iter_mut());
                for (i, operand) in operands.enumerate() {
                    match side(i) {
                        (true, _) => self.visit_expr(operand),
                        (false, true) => self.consumed_by_value(operand),
                        (false, false) => self.consumed(operand),
                    }
                }
            }
            Expr::Call(call) => {
                let consuming = match call.func.as_ref() {
                    Expr::Name(f) => self.consuming.contains(&f.id.as_str()),
                    Expr::Attribute(method) => self.is_text(&method.value),
                    _ => false,
                };
                if consuming && call.arguments.keywords.is_empty() {
                    self.visit_expr(&mut call.func);
                    for arg in &mut call.arguments.args {
                        self.consumed(arg);
                    }
                } else {
                    transformer::walk_expr(self, expr);
                }
            }
            _ => transformer::walk_expr(self, expr),
        }
    }
}
