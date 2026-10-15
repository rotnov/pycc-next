//! `chr(i)` whose argument is provably a code point pycc's `str` holds.
//!
//! Non-strict mode runs `chr` in CPython, whose `str` may hold a lone
//! surrogate (`chr(0xD800)`) that pycc's UTF-8 `str` cannot
//! (`docs/DEVIATIONS.md`). A call whose argument is known to be outside the
//! surrogate range is the same in both, so it is compiled natively in every
//! mode: it is spelled [`NATIVE_CHR`], which `pycc_hir::builtins` lowers as
//! `chr`. Known means an `int` literal, or a comprehension's target bound
//! by `range(...)` of `int` literals (`frozenset(chr(i) for i in
//! range(32))`), whose every value is a non-surrogate code point. A module
//! that binds or imports `chr` itself is left as written.

use super::aliases::rebound_names;
use ruff_python_ast::visitor::transformer::{self, Transformer};
use ruff_python_ast::{Alias, Comprehension, Expr, Number, Stmt, UnaryOp};
use std::cell::RefCell;

/// The spelling of a `chr` call compiled natively in every mode.
pub const NATIVE_CHR: &str = "__pycc_chr";

/// The first surrogate code point; the last is `0xDFFF`.
const SURROGATES: std::ops::RangeInclusive<i64> = 0xD800..=0xDFFF;

/// The largest code point.
const MAX_CODE_POINT: i64 = 0x10_FFFF;

/// Rewrites every provable `chr(...)` call of `body` (module doc).
pub(crate) fn native_chr_calls(body: &mut [Stmt]) {
    if rebound_names(body).contains_key("chr") || imports_chr(body) {
        return;
    }
    let rewrite = Rewrite {
        bounded: RefCell::new(Vec::new()),
    };
    for stmt in body.iter_mut() {
        rewrite.visit_stmt(stmt);
    }
}

/// Whether an import statement of `body`, at any depth, binds `chr` (or
/// is a `*` import, which may).
fn imports_chr(body: &[Stmt]) -> bool {
    struct Find(bool);
    impl<'a> ruff_python_ast::visitor::Visitor<'a> for Find {
        fn visit_stmt(&mut self, stmt: &'a Stmt) {
            let binds = |alias: &Alias| {
                alias.name.as_str() == "*"
                    || alias.asname.as_ref().unwrap_or(&alias.name).as_str() == "chr"
            };
            match stmt {
                Stmt::Import(import) if import.names.iter().any(binds) => self.0 = true,
                Stmt::ImportFrom(import) if import.names.iter().any(binds) => self.0 = true,
                _ => ruff_python_ast::visitor::walk_stmt(self, stmt),
            }
        }
    }
    let mut find = Find(false);
    for stmt in body {
        ruff_python_ast::visitor::Visitor::visit_stmt(&mut find, stmt);
    }
    find.0
}

/// An `int` literal's value (a negated one included).
fn int_literal(expr: &Expr) -> Option<i64> {
    match expr {
        Expr::NumberLiteral(n) => match &n.value {
            Number::Int(i) => i.as_i64(),
            _ => None,
        },
        Expr::UnaryOp(u) if u.op == UnaryOp::USub => int_literal(&u.operand)?.checked_neg(),
        _ => None,
    }
}

/// Whether every value in `low..=high` is a code point pycc's `str` holds.
fn holds(low: i64, high: i64) -> bool {
    low >= 0
        && high <= MAX_CODE_POINT
        && (high < *SURROGATES.start() || low > *SURROGATES.end())
}

/// The target of `comp` when it iterates `range(...)` of `int` literals
/// whose every value [`holds`].
fn bounded_target(comp: &Comprehension) -> Option<String> {
    let Expr::Name(target) = &comp.target else {
        return None;
    };
    let Expr::Call(call) = &comp.iter else {
        return None;
    };
    if !matches!(call.func.as_ref(), Expr::Name(n) if n.id.as_str() == "range")
        || !call.arguments.keywords.is_empty()
    {
        return None;
    }
    let args: Vec<i64> = call
        .arguments
        .args
        .iter()
        .map(int_literal)
        .collect::<Option<_>>()?;
    let (start, stop, step) = match args.as_slice() {
        [stop] => (0, *stop, 1),
        [start, stop] => (*start, *stop, 1),
        [start, stop, step] if *step != 0 => (*start, *stop, *step),
        _ => return None,
    };
    // An empty range yields nothing to check.
    let (low, high) = if step > 0 {
        (start, stop.checked_sub(1)?)
    } else {
        (stop.checked_add(1)?, start)
    };
    (low > high || holds(low, high)).then(|| target.id.to_string())
}

/// Every name a comprehension target binds.
fn target_names(target: &Expr) -> Vec<String> {
    match target {
        Expr::Name(name) => vec![name.id.to_string()],
        Expr::Tuple(t) => t.elts.iter().flat_map(target_names).collect(),
        Expr::List(l) => l.elts.iter().flat_map(target_names).collect(),
        Expr::Starred(s) => target_names(&s.value),
        _ => Vec::new(),
    }
}

/// The rewriter: the comprehension targets in scope known to hold.
struct Rewrite {
    bounded: RefCell<Vec<String>>,
}

impl Rewrite {
    fn holds(&self, arg: &Expr) -> bool {
        match arg {
            Expr::Name(name) => self.bounded.borrow().iter().any(|b| b == name.id.as_str()),
            other => int_literal(other).is_some_and(|v| holds(v, v)),
        }
    }

    /// Visits a comprehension's generators and then `parts` (its element
    /// or key and value), each generator's target in scope after it. A
    /// later generator's target shadows an earlier one of its name.
    fn visit_comprehension_scope(&self, generators: &mut [Comprehension], parts: &mut [&mut Expr]) {
        let depth = self.bounded.borrow().len();
        for comp in generators.iter_mut() {
            // The first iterable is evaluated outside the comprehension, a
            // later one inside: either way, before this target binds.
            self.visit_expr(&mut comp.iter);
            let targets = target_names(&comp.target);
            let bound = bounded_target(comp);
            let mut scope = self.bounded.borrow_mut();
            scope.retain(|b| !targets.contains(b));
            scope.extend(bound);
            drop(scope);
            for cond in &mut comp.ifs {
                self.visit_expr(cond);
            }
        }
        for part in parts {
            self.visit_expr(part);
        }
        self.bounded.borrow_mut().truncate(depth);
    }
}

impl Transformer for Rewrite {
    fn visit_expr(&self, expr: &mut Expr) {
        match expr {
            Expr::ListComp(c) => self.visit_comprehension_scope(&mut c.generators, &mut [&mut c.elt]),
            Expr::SetComp(c) => self.visit_comprehension_scope(&mut c.generators, &mut [&mut c.elt]),
            Expr::Generator(c) => self.visit_comprehension_scope(&mut c.generators, &mut [&mut c.elt]),
            Expr::DictComp(c) => {
                let mut parts: Vec<&mut Expr> = c.key.as_deref_mut().into_iter().collect();
                parts.push(&mut c.value);
                self.visit_comprehension_scope(&mut c.generators, &mut parts);
            }
            // A lambda's parameters shadow the targets.
            Expr::Lambda(_) => {
                let saved = std::mem::take(&mut *self.bounded.borrow_mut());
                transformer::walk_expr(self, expr);
                *self.bounded.borrow_mut() = saved;
            }
            _ => {
                transformer::walk_expr(self, expr);
                if let Expr::Call(call) = expr
                    && call.arguments.keywords.is_empty()
                    && let [arg] = &*call.arguments.args
                    && self.holds(arg)
                    && let Expr::Name(callee) = call.func.as_mut()
                    && callee.id.as_str() == "chr"
                {
                    callee.id = NATIVE_CHR.into();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rewrite(source: &str) -> String {
        let mut module = ruff_python_parser::parse_module(source)
            .expect("parses")
            .into_syntax();
        let mut body: Vec<Stmt> = std::mem::take(&mut module.body).into_iter().collect();
        native_chr_calls(&mut body);
        let calls = std::cell::RefCell::new(Vec::new());
        struct Names<'c>(&'c RefCell<Vec<String>>);
        impl<'a> ruff_python_ast::visitor::Visitor<'a> for Names<'_> {
            fn visit_expr(&mut self, expr: &'a Expr) {
                if let Expr::Call(call) = expr
                    && let Expr::Name(n) = call.func.as_ref()
                {
                    self.0.borrow_mut().push(n.id.to_string());
                }
                ruff_python_ast::visitor::walk_expr(self, expr);
            }
        }
        let mut names = Names(&calls);
        for stmt in &body {
            ruff_python_ast::visitor::Visitor::visit_stmt(&mut names, stmt);
        }
        let mut calls = calls.into_inner();
        // Sorted: the visitor's order is not the source's.
        calls.sort();
        calls.join(",")
    }

    #[test]
    fn provable_code_points_are_native() {
        assert_eq!(rewrite("x = chr(127)\n"), "__pycc_chr");
        assert_eq!(rewrite("x = chr(0x10FFFF)\n"), "__pycc_chr");
        assert_eq!(
            rewrite("s = frozenset(chr(i) for i in range(32))\n"),
            "__pycc_chr,frozenset,range"
        );
        assert_eq!(rewrite("s = [chr(i) for i in range(0xE000, 0xE010)]\n"), "__pycc_chr,range");
        assert_eq!(rewrite("s = [chr(i) for i in range(10, -1, -1)]\n"), "__pycc_chr,range");
        // An empty range.
        assert_eq!(rewrite("s = [chr(i) for i in range(5, 5)]\n"), "__pycc_chr,range");
    }

    #[test]
    fn anything_else_stays_chr() {
        // A surrogate, a negative or too large code point, a computed one.
        assert_eq!(rewrite("x = chr(0xD800)\n"), "chr");
        assert_eq!(rewrite("x = chr(-1)\n"), "chr");
        assert_eq!(rewrite("x = chr(0x110000)\n"), "chr");
        assert_eq!(rewrite("x = chr(n)\n"), "chr");
        assert_eq!(rewrite("s = [chr(i) for i in range(0xD7F0, 0xD810)]\n"), "chr,range");
        assert_eq!(rewrite("s = [chr(i) for i in range(n)]\n"), "chr,range");
        // Out of the comprehension, and shadowed by a later target.
        assert_eq!(rewrite("s = [i for i in range(3)]\nx = chr(i)\n"), "chr,range");
        assert_eq!(
            rewrite("s = [chr(i) for i in range(3) for i in xs]\n"),
            "chr,range"
        );
        assert_eq!(
            rewrite("s = [chr(i) for i in range(3) for (j, i) in xs]\n"),
            "chr,range"
        );
        // A lambda parameter shadows the target.
        assert_eq!(
            rewrite("s = [(lambda i: chr(i)) for i in range(3)]\n"),
            "chr,range"
        );
    }

    #[test]
    fn a_module_binding_chr_is_left_alone() {
        assert_eq!(rewrite("def chr(x: int) -> str:\n    return ''\nx = chr(1)\n"), "chr");
        assert_eq!(rewrite("from m import chr\nx = chr(1)\n"), "chr");
        assert_eq!(rewrite("from m import *\nx = chr(1)\n"), "chr");
    }
}
