//! Membership in a constant tuple: `x in c`, where `c` is bound once in its
//! scope to a tuple display of `int`/`str` literals (`c = (1, 4, 6)`) by a
//! statement that runs before the test, becomes `x in [1, 4, 6]`.
//!
//! Testing membership compares the needle with each element by `==` in
//! order, exactly as a list's test does, and a literal element has no
//! identity a test could observe, so only the container's own type changes
//! -- which the test does not show. Only a test in a statement after the
//! binding, in the same statement list (at any depth below it), is
//! rewritten, so `c` is always bound when the test runs; a function reads
//! only its own constants. Every other read of `c` keeps the tuple.

use super::aliases::rebound_names;
use ruff_python_ast::visitor::transformer::{self, Transformer};
use ruff_python_ast::{AtomicNodeIndex, CmpOp, Expr, ExprContext, ExprList, Number, Stmt, UnaryOp};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

/// Rewrites every such membership test (module doc).
pub(crate) fn constant_tuple_membership(body: &mut [Stmt]) {
    let globals = declared_global(body);
    let top_level: Vec<Stmt> = body
        .iter()
        .filter(|stmt| !matches!(stmt, Stmt::FunctionDef(_) | Stmt::ClassDef(_)))
        .cloned()
        .collect();
    let module_once = once(&rebound_names(&top_level), &globals);
    Rewrite::new(module_once).block(body);
    for stmt in body.iter_mut() {
        if let Stmt::FunctionDef(def) = stmt {
            let params: HashSet<String> = def
                .parameters
                .iter()
                .map(|p| p.name().to_string())
                .collect();
            let mut local = once(&rebound_names(&def.body), &globals);
            local.retain(|name| !params.contains(name));
            Rewrite::new(local).block(&mut def.body);
        }
    }
}

/// The names `rebound` counts exactly once, `global`/`nonlocal` ones left
/// out.
fn once(rebound: &HashMap<String, usize>, globals: &HashSet<String>) -> HashSet<String> {
    rebound
        .iter()
        .filter(|(name, count)| **count == 1 && !globals.contains(*name))
        .map(|(name, _)| name.clone())
        .collect()
}

/// The names any `global`/`nonlocal` statement of the module declares.
fn declared_global(body: &[Stmt]) -> HashSet<String> {
    use ruff_python_ast::visitor::{self, Visitor};
    struct Find(HashSet<String>);
    impl<'a> Visitor<'a> for Find {
        fn visit_stmt(&mut self, stmt: &'a Stmt) {
            match stmt {
                Stmt::Global(g) => self.0.extend(g.names.iter().map(|n| n.to_string())),
                Stmt::Nonlocal(n) => self.0.extend(n.names.iter().map(|n| n.to_string())),
                _ => {}
            }
            visitor::walk_stmt(self, stmt);
        }
    }
    let mut find = Find(HashSet::new());
    for stmt in body {
        find.visit_stmt(stmt);
    }
    find.0
}

/// An `int` or `str` literal, or a negated `int` literal.
fn is_constant(expr: &Expr) -> bool {
    match expr {
        Expr::NumberLiteral(n) => matches!(n.value, Number::Int(_)),
        Expr::StringLiteral(_) => true,
        Expr::UnaryOp(u) => {
            u.op == UnaryOp::USub
                && matches!(u.operand.as_ref(), Expr::NumberLiteral(n) if matches!(n.value, Number::Int(_)))
        }
        _ => false,
    }
}

struct Rewrite {
    /// The names bound once in this scope.
    candidates: HashSet<String>,
    /// The constants bound by a statement already run, with their elements.
    active: RefCell<HashMap<String, Vec<Expr>>>,
}

impl Rewrite {
    fn new(candidates: HashSet<String>) -> Self {
        Rewrite {
            candidates,
            active: RefCell::new(HashMap::new()),
        }
    }

    /// Rewrites the statements of one statement list: a binding is active
    /// for the statements after it in the list, until the list ends.
    fn block(&self, body: &mut [Stmt]) {
        let saved = self.active.borrow().clone();
        for stmt in body.iter_mut() {
            self.visit_stmt(stmt);
            if let Stmt::Assign(assign) = stmt
                && let [Expr::Name(target)] = assign.targets.as_slice()
                && let Expr::Tuple(tuple) = assign.value.as_ref()
                && !tuple.elts.is_empty()
                && tuple.elts.iter().all(is_constant)
                && self.candidates.contains(target.id.as_str())
            {
                self.active
                    .borrow_mut()
                    .insert(target.id.to_string(), tuple.elts.clone());
            }
        }
        *self.active.borrow_mut() = saved;
    }
}

impl Transformer for Rewrite {
    fn visit_stmt(&self, stmt: &mut Stmt) {
        match stmt {
            // A nested definition is its own scope.
            Stmt::FunctionDef(_) | Stmt::ClassDef(_) => {}
            Stmt::If(s) => {
                self.visit_expr(&mut s.test);
                self.block(&mut s.body);
                for clause in &mut s.elif_else_clauses {
                    if let Some(test) = &mut clause.test {
                        self.visit_expr(test);
                    }
                    self.block(&mut clause.body);
                }
            }
            Stmt::For(s) => {
                self.visit_expr(&mut s.iter);
                self.block(&mut s.body);
                self.block(&mut s.orelse);
            }
            Stmt::While(s) => {
                self.visit_expr(&mut s.test);
                self.block(&mut s.body);
                self.block(&mut s.orelse);
            }
            Stmt::With(_) | Stmt::Try(_) | Stmt::Match(_) => {}
            other => transformer::walk_stmt(self, other),
        }
    }

    fn visit_expr(&self, expr: &mut Expr) {
        if matches!(expr, Expr::Lambda(_)) {
            return;
        }
        transformer::walk_expr(self, expr);
        let Expr::Compare(compare) = expr else {
            return;
        };
        let active = self.active.borrow();
        for (op, comparator) in compare.ops.iter().zip(compare.comparators.iter_mut()) {
            if !matches!(op, CmpOp::In | CmpOp::NotIn) {
                continue;
            }
            let Expr::Name(name) = comparator else {
                continue;
            };
            let Some(elements) = active.get(name.id.as_str()) else {
                continue;
            };
            *comparator = Expr::List(ExprList {
                node_index: AtomicNodeIndex::default(),
                range: name.range,
                elts: elements.clone(),
                ctx: ExprContext::Load,
            });
        }
    }
}
