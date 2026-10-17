//! `range` objects kept in a variable and only ever iterated.
//!
//! ```python
//! r = list(map(range, (len(a), len(b))))
//! if fix is not None:
//!     r[fix] = range(len(a) - 1, len(a))
//! for i in r[0]:
//!     for j in r[1]:
//!         ...
//! ```
//!
//! A `range` object as a value has no compiled type. When every use of a
//! name is one this pass understands, its ranges are compiled as their
//! `(start, stop, step)` triples instead, which no use can tell apart:
//!
//! * a *range name* is bound only to `range(...)` calls and only iterated
//!   (`for v in n`, or a comprehension's `for v in n`), which becomes
//!   `range(n[0], n[1], n[2])`;
//! * a *range list* is bound only to a list of them -- `list(map(range,
//!   xs))`, a list display of `range(...)` calls or a list comprehension
//!   building one -- and otherwise only has an element replaced by a
//!   `range(...)` call (`n[i] = range(...)`), an element iterated (`for v
//!   in n[i]`, becoming `range(n[i][0], n[i][1], n[i][2])`) or its length
//!   taken (`len(n)`), where `i` is a name or an integer literal (reading
//!   `n[i]` three times instead of once is then unobservable).
//!
//! A `range(...)` call becomes `(0, stop, 1)`, `(start, stop, 1)` or
//! `(start, stop, step)` for a nonzero integer literal `step` (CPython's
//! `ValueError` for a zero step is raised when the object is made, so any
//! other step keeps the name as it is). Arguments are evaluated in the same
//! order; one that is not an `int` is a compile error when the triple is
//! iterated, where CPython would raise `TypeError` when the range is made.
//! A name read anywhere else -- printed, passed, compared, read in a nested
//! function, lambda or generator, declared `global`/`nonlocal` -- is left
//! alone, so the program is compiled (or refused) exactly as before.

use super::build;
use ruff_python_ast::visitor::transformer::{self, Transformer};
use ruff_python_ast::visitor::{self, Visitor};
use ruff_python_ast::{Expr, ExprContext, Number, Stmt, UnaryOp};
use ruff_text_size::{Ranged, TextRange};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Range,
    List,
}

/// Rewrites every range name and range list of `body` (module doc).
pub(crate) fn range_values(body: &mut [Stmt]) {
    let stored = super::stored_names(body);
    if ["range", "list", "map", "len"]
        .iter()
        .any(|b| stored.contains(*b))
    {
        return;
    }
    let mut names = Names::default();
    for stmt in body.iter() {
        names.visit_stmt(stmt);
    }
    let taken = std::cell::RefCell::new(names.0);
    rewrite_scope(body, &HashSet::new(), &taken);
}

/// One scope (with parameters `params`): its own names, then each nested
/// function's.
fn rewrite_scope(
    body: &mut [Stmt],
    params: &HashSet<String>,
    taken: &std::cell::RefCell<HashSet<String>>,
) {
    let mut scan = Scan::default();
    for stmt in body.iter() {
        scan.visit_stmt(stmt);
    }
    let kinds: HashMap<String, Kind> = scan
        .kinds
        .iter()
        .filter(|(name, kind)| {
            kind.is_some()
                && scan.bound.contains(*name)
                && !params.contains(*name)
                && !scan.escaped.contains(*name)
                && scan.allowed.get(*name) == scan.reads.get(*name)
        })
        .map(|(name, kind)| (name.clone(), kind.expect("filtered")))
        .collect();
    if !kinds.is_empty() {
        let rewrite = Rewrite { kinds, taken };
        for stmt in body.iter_mut() {
            rewrite.visit_stmt(stmt);
        }
    }
    for stmt in body.iter_mut() {
        nested_scopes(stmt, taken);
    }
}

fn nested_scopes(stmt: &mut Stmt, taken: &std::cell::RefCell<HashSet<String>>) {
    match stmt {
        Stmt::FunctionDef(def) => {
            let params = def
                .parameters
                .iter()
                .map(|p| p.name().to_string())
                .collect();
            rewrite_scope(&mut def.body, &params, taken);
        }
        Stmt::ClassDef(class) => {
            for stmt in class.body.iter_mut() {
                nested_scopes(stmt, taken);
            }
        }
        Stmt::If(if_stmt) => {
            if_stmt
                .body
                .iter_mut()
                .for_each(|s| nested_scopes(s, taken));
            for clause in &mut if_stmt.elif_else_clauses {
                clause.body.iter_mut().for_each(|s| nested_scopes(s, taken));
            }
        }
        Stmt::For(f) => {
            f.body
                .iter_mut()
                .chain(f.orelse.iter_mut())
                .for_each(|s| nested_scopes(s, taken));
        }
        Stmt::While(w) => {
            w.body
                .iter_mut()
                .chain(w.orelse.iter_mut())
                .for_each(|s| nested_scopes(s, taken));
        }
        Stmt::With(w) => w.body.iter_mut().for_each(|s| nested_scopes(s, taken)),
        Stmt::Try(t) => {
            t.body.iter_mut().for_each(|s| nested_scopes(s, taken));
            for handler in &mut t.handlers {
                let ruff_python_ast::ExceptHandler::ExceptHandler(handler) = handler;
                handler
                    .body
                    .iter_mut()
                    .for_each(|s| nested_scopes(s, taken));
            }
            t.orelse
                .iter_mut()
                .chain(t.finalbody.iter_mut())
                .for_each(|s| nested_scopes(s, taken));
        }
        _ => {}
    }
}

/// `range(a)`, `range(a, b)` or `range(a, b, step)` with a nonzero integer
/// literal `step`: its triple's three expressions.
fn range_call(expr: &Expr) -> Option<[Expr; 3]> {
    let Expr::Call(call) = expr else {
        return None;
    };
    if !matches!(call.func.as_ref(), Expr::Name(f) if f.id.as_str() == "range")
        || !call.arguments.keywords.is_empty()
        || call
            .arguments
            .args
            .iter()
            .any(|a| matches!(a, Expr::Starred(_)))
    {
        return None;
    }
    let range = call.range;
    let one = || build::int(1, range);
    match call.arguments.args.as_ref() {
        [stop] => Some([build::int(0, range), stop.clone(), one()]),
        [start, stop] => Some([start.clone(), stop.clone(), one()]),
        [start, stop, step] if nonzero_int_literal(step) => {
            Some([start.clone(), stop.clone(), step.clone()])
        }
        _ => None,
    }
}

fn nonzero_int_literal(expr: &Expr) -> bool {
    match expr {
        Expr::NumberLiteral(n) => matches!(&n.value, Number::Int(i) if i.as_u64() != Some(0)),
        Expr::UnaryOp(u) if u.op == UnaryOp::USub => nonzero_int_literal(&u.operand),
        _ => false,
    }
}

/// The element of `list(map(range, xs))`: `xs`.
fn map_range(expr: &Expr) -> Option<&Expr> {
    let Expr::Call(call) = expr else {
        return None;
    };
    if !matches!(call.func.as_ref(), Expr::Name(f) if f.id.as_str() == "list")
        || !call.arguments.keywords.is_empty()
    {
        return None;
    }
    let [Expr::Call(inner)] = call.arguments.args.as_ref() else {
        return None;
    };
    if !matches!(inner.func.as_ref(), Expr::Name(f) if f.id.as_str() == "map")
        || !inner.arguments.keywords.is_empty()
    {
        return None;
    }
    match inner.arguments.args.as_ref() {
        [Expr::Name(f), xs] if f.id.as_str() == "range" && !matches!(xs, Expr::Starred(_)) => {
            Some(xs)
        }
        _ => None,
    }
}

/// Whether `expr` is a list of ranges this pass builds.
fn range_list(expr: &Expr) -> bool {
    match expr {
        Expr::List(list) => {
            !list.elts.is_empty() && list.elts.iter().all(|e| range_call(e).is_some())
        }
        Expr::ListComp(comp) => range_call(&comp.elt).is_some(),
        other => map_range(other).is_some(),
    }
}

/// A subscript `n[i]` with a name or integer-literal index: `n`.
fn simple_item(expr: &Expr) -> Option<&str> {
    let Expr::Subscript(sub) = expr else {
        return None;
    };
    let Expr::Name(base) = sub.value.as_ref() else {
        return None;
    };
    matches!(sub.slice.as_ref(), Expr::Name(_) | Expr::NumberLiteral(_)).then_some(base.id.as_str())
}

/// What one scope does with each name.
#[derive(Default)]
struct Scan {
    /// Each bound name's kind, `None` once a binding is neither.
    kinds: HashMap<String, Option<Kind>>,
    /// Reads and writes of each name the rewrite understands.
    allowed: HashMap<String, usize>,
    /// Every read and write of each name in this scope.
    reads: HashMap<String, usize>,
    /// Names a nested scope sees, or declared `global`/`nonlocal`.
    escaped: HashSet<String>,
    /// Names some assignment binds to a range or a list of them.
    bound: HashSet<String>,
}

impl Scan {
    fn bind(&mut self, name: &str, kind: Option<Kind>) {
        let entry = self.kinds.entry(name.to_string()).or_insert(kind);
        if *entry != kind {
            *entry = None;
        }
    }

    fn allow(&mut self, name: &str) {
        *self.allowed.entry(name.to_string()).or_default() += 1;
    }

    /// An iterated expression: a range name or an element of a range list.
    fn iterated(&mut self, iter: &Expr) {
        match iter {
            Expr::Name(name) => {
                self.allow(name.id.as_str());
                self.require(name.id.as_str(), Kind::Range);
            }
            other => {
                if let Some(base) = simple_item(other) {
                    self.allow(base);
                    self.require(base, Kind::List);
                }
            }
        }
    }

    /// A use that only a name of `kind` admits.
    fn require(&mut self, name: &str, kind: Kind) {
        if let Some(bound) = self.kinds.get_mut(name)
            && *bound != Some(kind)
        {
            *bound = None;
        }
        self.kinds.entry(name.to_string()).or_insert(Some(kind));
    }

    fn escape_all(&mut self, node: &dyn Fn(&mut Names)) {
        let mut names = Names::default();
        node(&mut names);
        self.escaped.extend(names.0);
    }
}

#[derive(Default)]
struct Names(HashSet<String>);

impl<'a> Visitor<'a> for Names {
    fn visit_expr(&mut self, expr: &'a Expr) {
        if let Expr::Name(name) = expr {
            self.0.insert(name.id.to_string());
        }
        visitor::walk_expr(self, expr);
    }
}

impl<'a> Visitor<'a> for Scan {
    fn visit_stmt(&mut self, stmt: &'a Stmt) {
        match stmt {
            Stmt::FunctionDef(_) | Stmt::ClassDef(_) => {
                self.escape_all(&|names| names.visit_stmt(stmt));
                return;
            }
            Stmt::Global(g) => self.escaped.extend(g.names.iter().map(|n| n.to_string())),
            Stmt::Nonlocal(n) => self.escaped.extend(n.names.iter().map(|n| n.to_string())),
            Stmt::Assign(assign) => match assign.targets.as_slice() {
                [Expr::Name(target)] => {
                    let kind = if range_list(&assign.value) {
                        Some(Kind::List)
                    } else if range_call(&assign.value).is_some() {
                        Some(Kind::Range)
                    } else {
                        None
                    };
                    self.bind(target.id.as_str(), kind);
                    if kind.is_some() {
                        self.allow(target.id.as_str());
                        self.bound.insert(target.id.to_string());
                    }
                }
                [target] => {
                    if let Some(base) = simple_item(target)
                        && range_call(&assign.value).is_some()
                    {
                        self.allow(base);
                        self.require(base, Kind::List);
                    }
                }
                _ => {}
            },
            Stmt::For(for_stmt) => self.iterated(&for_stmt.iter),
            _ => {}
        }
        visitor::walk_stmt(self, stmt);
    }

    fn visit_expr(&mut self, expr: &'a Expr) {
        match expr {
            Expr::Name(name) => {
                *self.reads.entry(name.id.to_string()).or_default() += 1;
                if name.ctx != ExprContext::Load && name.ctx != ExprContext::Store {
                    self.escaped.insert(name.id.to_string());
                }
            }
            Expr::Lambda(_) | Expr::Generator(_) => {
                self.escape_all(&|names| names.visit_expr(expr));
                return;
            }
            Expr::ListComp(comp) => {
                for generator in &comp.generators {
                    self.iterated(&generator.iter);
                }
            }
            Expr::SetComp(comp) => {
                for generator in &comp.generators {
                    self.iterated(&generator.iter);
                }
            }
            Expr::DictComp(comp) => {
                for generator in &comp.generators {
                    self.iterated(&generator.iter);
                }
            }
            Expr::Call(call) => {
                if matches!(call.func.as_ref(), Expr::Name(f) if f.id.as_str() == "len")
                    && let [Expr::Name(arg)] = call.arguments.args.as_ref()
                    && call.arguments.keywords.is_empty()
                {
                    self.allow(arg.id.as_str());
                    self.require(arg.id.as_str(), Kind::List);
                }
            }
            _ => {}
        }
        visitor::walk_expr(self, expr);
    }
}

/// Rewrites the names [`Scan`] admitted.
struct Rewrite<'t> {
    kinds: HashMap<String, Kind>,
    taken: &'t std::cell::RefCell<HashSet<String>>,
}

impl Rewrite<'_> {
    fn fresh(&self) -> String {
        let mut taken = self.taken.borrow_mut();
        let mut k = 0;
        loop {
            let name = format!("_pycc_range_{k}");
            if taken.insert(name.clone()) {
                return name;
            }
            k += 1;
        }
    }

    /// `range(t[0], t[1], t[2])` for a triple read by `t` (re-evaluated).
    fn iterate(t: &Expr, range: TextRange) -> Expr {
        let item =
            |i: u64| build::subscript(t.clone(), build::int(i, range), ExprContext::Load, range);
        build::call_name("range", vec![item(0), item(1), item(2)], range)
    }

    fn triple(expr: &Expr) -> Option<Expr> {
        range_call(expr).map(|items| build::tuple(items.into(), ExprContext::Load, expr.range()))
    }

    /// The rewritten iterable of a loop over `iter`, if it is one of ours.
    fn iterable(&self, iter: &Expr) -> Option<Expr> {
        match iter {
            Expr::Name(name) if self.kinds.get(name.id.as_str()) == Some(&Kind::Range) => {
                Some(Self::iterate(iter, iter.range()))
            }
            other => simple_item(other)
                .filter(|base| self.kinds.get(*base) == Some(&Kind::List))
                .map(|_| Self::iterate(other, other.range())),
        }
    }

    fn list_value(&self, value: &mut Expr) {
        if let Some(xs) = map_range(value) {
            let range = value.range();
            let var = self.fresh();
            let triple = build::tuple(
                vec![
                    build::int(0, range),
                    build::load(&var, range),
                    build::int(1, range),
                ],
                ExprContext::Load,
                range,
            );
            *value = build::list_comp(
                triple,
                build::store(&var, range),
                xs.clone(),
                Vec::new(),
                range,
            );
            return;
        }
        match value {
            Expr::List(list) => {
                for elt in list.elts.iter_mut() {
                    if let Some(triple) = Self::triple(elt) {
                        *elt = triple;
                    }
                }
            }
            Expr::ListComp(comp) => {
                if let Some(triple) = Self::triple(&comp.elt) {
                    *comp.elt = triple;
                }
            }
            _ => {}
        }
    }
}

impl Transformer for Rewrite<'_> {
    fn visit_stmt(&self, stmt: &mut Stmt) {
        match stmt {
            Stmt::Assign(assign) => match assign.targets.as_slice() {
                [Expr::Name(target)] => match self.kinds.get(target.id.as_str()) {
                    Some(Kind::List) => self.list_value(&mut assign.value),
                    Some(Kind::Range) => {
                        if let Some(triple) = Self::triple(&assign.value) {
                            *assign.value = triple;
                        }
                    }
                    None => {}
                },
                [target] => {
                    if simple_item(target)
                        .is_some_and(|base| self.kinds.get(base) == Some(&Kind::List))
                        && let Some(triple) = Self::triple(&assign.value)
                    {
                        *assign.value = triple;
                    }
                }
                _ => {}
            },
            Stmt::For(for_stmt) => {
                if let Some(iter) = self.iterable(&for_stmt.iter) {
                    *for_stmt.iter = iter;
                }
            }
            // Nested scopes are rewritten on their own.
            Stmt::FunctionDef(_) | Stmt::ClassDef(_) => return,
            _ => {}
        }
        transformer::walk_stmt(self, stmt);
    }

    fn visit_expr(&self, expr: &mut Expr) {
        let generators = match expr {
            Expr::ListComp(comp) => Some(&mut comp.generators),
            Expr::SetComp(comp) => Some(&mut comp.generators),
            Expr::DictComp(comp) => Some(&mut comp.generators),
            _ => None,
        };
        if let Some(generators) = generators {
            for generator in generators.iter_mut() {
                if let Some(iter) = self.iterable(&generator.iter) {
                    generator.iter = iter;
                }
            }
        }
        transformer::walk_expr(self, expr);
    }
}

#[cfg(test)]
mod tests;
