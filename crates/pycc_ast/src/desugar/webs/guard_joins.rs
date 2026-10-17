//! Join copies for a parameter an `isinstance` guard normalizes:
//!
//! ```python
//! def dot(A, B):
//!     if not isinstance(B[0], list):
//!         B = [[b] for b in B]
//!     return len(B[0])
//! ```
//!
//! Both bindings of `B` reach the read after the `if`, so they form one web
//! and `B` keeps one type for the whole function. A guard like this one
//! tests the shape of a value whose static type usually decides it, and
//! `pycc_types` folds such a test (`pycc_types::static_isinstance`). For
//! the branch it keeps to type on its own, each path through the `if`
//! ends with a copy of the parameter into one variable read afterwards:
//!
//! ```python
//!     if not isinstance(B[0], list):
//!         B = [[b] for b in B]
//!         B = B
//!     else:
//!         B = B
//! ```
//!
//! A copy of a bound name is a no-op, so this changes no behavior; the web
//! analysis then gives the rebinding and the join variables of their own
//! (`B_1 = [[b] for b in B]; B_2 = B_1` / `B_2 = B`).
//!
//! Only a top-level `if` of a function body without `elif`/`else` is
//! rewritten, whose test combines `isinstance(x, C)` calls (`C` a builtin
//! class or a tuple of them, `x` not a bare name: a bare name is the
//! `object` narrowing of `pycc_types::narrow`) with `not`/`and`/`or`, and
//! whose body rebinds an unannotated parameter the test reads. The copies
//! get offsets past the end of the module while webs are computed (web
//! analysis keys every name by its offset); [`restore`] then gives them the
//! range of the `if` they belong to.

use ruff_python_ast::visitor::transformer::{self, Transformer};
use ruff_python_ast::visitor::{self, Visitor};
use ruff_python_ast::{
    AtomicNodeIndex, BoolOp, ElifElseClause, Expr, ExprContext, ExprName, Stmt, StmtAssign, UnaryOp,
};
use ruff_text_size::{TextRange, TextSize};
use std::collections::{HashMap, HashSet};

/// The builtin classes a guard may test.
const CLASSES: [&str; 9] = [
    "list",
    "tuple",
    "dict",
    "set",
    "frozenset",
    "str",
    "int",
    "float",
    "bool",
];

/// Synthetic offsets handed out, and the range each one is restored to.
pub(super) struct Joins {
    next: u32,
    ranges: HashMap<u32, TextRange>,
}

/// Inserts the join copies into every function `body` defines (see the
/// module documentation); `None` when there is nothing to insert.
pub(super) fn insert(body: &mut [Stmt]) -> Option<Joins> {
    let shadowed = shadowed_names(body);
    if CLASSES.iter().all(|c| shadowed.contains(*c)) || shadowed.contains("isinstance") {
        return None;
    }
    let mut joins = Joins {
        next: end_offset(body) + 1,
        ranges: HashMap::new(),
    };
    each_def(body, &mut |def| joins.insert_into(def, &shadowed));
    (!joins.ranges.is_empty()).then_some(joins)
}

impl Joins {
    /// Inserts the copies into the top-level guards of `def`'s body.
    fn insert_into(
        &mut self,
        def: &mut ruff_python_ast::StmtFunctionDef,
        shadowed: &HashSet<String>,
    ) {
        let unannotated: Vec<String> = def
            .parameters
            .iter()
            .filter(|p| p.annotation().is_none())
            .map(|p| p.name().to_string())
            .collect();
        for stmt in def.body.iter_mut() {
            let Stmt::If(if_stmt) = stmt else { continue };
            if !if_stmt.elif_else_clauses.is_empty() || !is_guard(&if_stmt.test, shadowed) {
                continue;
            }
            let read = read_names(&if_stmt.test);
            let mut rebound: Vec<String> = Vec::new();
            for inner in &if_stmt.body {
                if let Stmt::Assign(assign) = inner
                    && let [Expr::Name(target)] = assign.targets.as_slice()
                {
                    let name = target.id.to_string();
                    if unannotated.contains(&name)
                        && read.contains(&name)
                        && !rebound.contains(&name)
                    {
                        rebound.push(name);
                    }
                }
            }
            if rebound.is_empty() {
                continue;
            }
            let range = if_stmt.range;
            let mut orelse = Vec::new();
            for name in &rebound {
                if_stmt.body.push(self.copy(name, range));
                orelse.push(self.copy(name, range));
            }
            if_stmt.elif_else_clauses.push(ElifElseClause {
                range,
                node_index: AtomicNodeIndex::default(),
                test: None,
                body: orelse.into(),
            });
        }
    }

    /// `name = name` at fresh synthetic offsets.
    fn copy(&mut self, name: &str, range: TextRange) -> Stmt {
        let target = self.name(name, ExprContext::Store, range);
        let value = self.name(name, ExprContext::Load, range);
        Stmt::Assign(StmtAssign {
            node_index: AtomicNodeIndex::default(),
            range,
            targets: vec![target],
            value: Box::new(value),
        })
    }

    fn name(&mut self, name: &str, ctx: ExprContext, range: TextRange) -> Expr {
        let offset = self.next;
        self.next += 1;
        self.ranges.insert(offset, range);
        Expr::Name(ExprName {
            node_index: AtomicNodeIndex::default(),
            range: TextRange::empty(TextSize::new(offset)),
            id: name.into(),
            ctx,
        })
    }
}

/// Gives every synthetic name the range of the `if` it was inserted into.
pub(super) fn restore(body: &mut [Stmt], joins: &Joins) {
    struct Restore<'a>(&'a HashMap<u32, TextRange>);
    impl Transformer for Restore<'_> {
        fn visit_expr(&self, expr: &mut Expr) {
            if let Expr::Name(name) = expr
                && let Some(range) = self.0.get(&name.range.start().to_u32())
            {
                name.range = *range;
            }
            transformer::walk_expr(self, expr);
        }
    }
    let restore = Restore(&joins.ranges);
    for stmt in body.iter_mut() {
        restore.visit_stmt(stmt);
    }
}

/// Whether `test` is a combination of builtin-class `isinstance` tests
/// (see the module documentation).
fn is_guard(test: &Expr, shadowed: &HashSet<String>) -> bool {
    match test {
        Expr::UnaryOp(unary) if unary.op == UnaryOp::Not => is_guard(&unary.operand, shadowed),
        Expr::BoolOp(bool_op) if matches!(bool_op.op, BoolOp::And | BoolOp::Or) => {
            bool_op.values.iter().all(|value| is_guard(value, shadowed))
        }
        Expr::Call(call) => {
            matches!(call.func.as_ref(), Expr::Name(f) if f.id.as_str() == "isinstance")
                && call.arguments.keywords.is_empty()
                && match call.arguments.args.as_ref() {
                    [subject, class] => {
                        !matches!(subject, Expr::Name(_) | Expr::Starred(_))
                            && is_class(class, shadowed)
                    }
                    _ => false,
                }
        }
        _ => false,
    }
}

fn is_class(class: &Expr, shadowed: &HashSet<String>) -> bool {
    match class {
        Expr::Name(name) => {
            CLASSES.contains(&name.id.as_str()) && !shadowed.contains(name.id.as_str())
        }
        Expr::Tuple(tuple) => {
            !tuple.elts.is_empty() && tuple.elts.iter().all(|elt| is_class(elt, shadowed))
        }
        _ => false,
    }
}

/// Every name `expr` reads.
fn read_names(expr: &Expr) -> HashSet<String> {
    struct Reads(HashSet<String>);
    impl<'a> Visitor<'a> for Reads {
        fn visit_expr(&mut self, expr: &'a Expr) {
            if let Expr::Name(name) = expr {
                self.0.insert(name.id.to_string());
            }
            visitor::walk_expr(self, expr);
        }
    }
    let mut reads = Reads(HashSet::new());
    reads.visit_expr(expr);
    reads.0
}

/// Every name bound anywhere in the module: a rebound `list` or
/// `isinstance` is no longer the builtin.
fn shadowed_names(body: &[Stmt]) -> HashSet<String> {
    let mut names = super::super::stored_names(body);
    names.retain(|name| CLASSES.contains(&name.as_str()) || name == "isinstance");
    names
}

/// One past the largest offset any node of `body` covers.
fn end_offset(body: &[Stmt]) -> u32 {
    body.iter()
        .map(|stmt| ruff_text_size::Ranged::range(stmt).end().to_u32())
        .max()
        .unwrap_or(0)
}

/// Calls `f` on every function definition in `body`, nested ones and
/// methods included, each before the definitions inside it.
fn each_def(body: &mut [Stmt], f: &mut dyn FnMut(&mut ruff_python_ast::StmtFunctionDef)) {
    for stmt in body.iter_mut() {
        match stmt {
            Stmt::FunctionDef(def) => {
                f(def);
                each_def(&mut def.body, f);
            }
            Stmt::ClassDef(class) => each_def(&mut class.body, f),
            Stmt::If(if_stmt) => {
                each_def(&mut if_stmt.body, f);
                for clause in &mut if_stmt.elif_else_clauses {
                    each_def(&mut clause.body, f);
                }
            }
            Stmt::For(for_stmt) => {
                each_def(&mut for_stmt.body, f);
                each_def(&mut for_stmt.orelse, f);
            }
            Stmt::While(while_stmt) => {
                each_def(&mut while_stmt.body, f);
                each_def(&mut while_stmt.orelse, f);
            }
            Stmt::With(with) => each_def(&mut with.body, f),
            Stmt::Try(try_stmt) => {
                each_def(&mut try_stmt.body, f);
                for handler in &mut try_stmt.handlers {
                    let ruff_python_ast::ExceptHandler::ExceptHandler(handler) = handler;
                    each_def(&mut handler.body, f);
                }
                each_def(&mut try_stmt.orelse, f);
                each_def(&mut try_stmt.finalbody, f);
            }
            _ => {}
        }
    }
}
