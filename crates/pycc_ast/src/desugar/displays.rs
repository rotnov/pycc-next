//! Display spellings with one meaning in two forms.
//!
//! - A list display used as an unpacking target (`for i, [a, b] in ...`,
//!   `[x, y] = v`) unpacks exactly as the tuple display does: it becomes
//!   one.
//! - A tuple display only iterated (`for z in (x, y)`, a comprehension over
//!   one) or tested for membership (`v in (a, b)`) yields and compares the
//!   same elements as the list display: it becomes one, so the iteration and
//!   `in` the compiler has for lists apply.
//! - `list()` and `dict()`, called with no argument where the module never
//!   rebinds the builtin, are the empty displays `[]` and `{}`.

use ruff_python_ast::visitor::transformer::{self, Transformer};
use ruff_python_ast::{
    AtomicNodeIndex, CmpOp, Comprehension, Expr, ExprContext, ExprDict, ExprList, ExprTuple, Stmt,
};

/// Rewrites every such display in `body`.
pub(crate) fn canonical_displays(body: &mut [Stmt]) {
    let rebound = super::aliases::rebound_names(body);
    let displays = Displays {
        empty_list: !rebound.contains_key("list"),
        empty_dict: !rebound.contains_key("dict"),
    };
    for stmt in body {
        displays.visit_stmt(stmt);
    }
}

struct Displays {
    /// `list()` is the builtin's: the module never rebinds `list`.
    empty_list: bool,
    /// `dict()` is the builtin's.
    empty_dict: bool,
}

impl Displays {
    /// `list()` / `dict()` as the empty display.
    fn empty_display(&self, expr: &mut Expr) {
        let Expr::Call(call) = expr else {
            return;
        };
        let Expr::Name(callee) = call.func.as_ref() else {
            return;
        };
        if !call.arguments.args.is_empty() || !call.arguments.keywords.is_empty() {
            return;
        }
        let range = call.range;
        match callee.id.as_str() {
            "list" if self.empty_list => {
                *expr = Expr::List(ExprList {
                    node_index: AtomicNodeIndex::default(),
                    range,
                    elts: Vec::new(),
                    ctx: ExprContext::Load,
                });
            }
            "dict" if self.empty_dict => {
                *expr = Expr::Dict(ExprDict {
                    node_index: AtomicNodeIndex::default(),
                    range,
                    items: Vec::new(),
                });
            }
            _ => {}
        }
    }
}

/// A list display target as the tuple display, nested ones included.
fn tuple_target(target: &mut Expr) {
    if let Expr::List(list) = target {
        *target = Expr::Tuple(ExprTuple {
            node_index: AtomicNodeIndex::default(),
            range: list.range,
            elts: std::mem::take(&mut list.elts),
            ctx: list.ctx,
            parenthesized: false,
        });
    }
    if let Expr::Tuple(tuple) = target {
        for element in &mut tuple.elts {
            tuple_target(element);
        }
    }
}

/// A tuple display only iterated or searched as the list display.
fn list_iterable(iterable: &mut Expr) {
    if let Expr::Tuple(tuple) = iterable
        && !tuple.elts.is_empty()
    {
        *iterable = Expr::List(ExprList {
            node_index: AtomicNodeIndex::default(),
            range: tuple.range,
            elts: std::mem::take(&mut tuple.elts),
            ctx: tuple.ctx,
        });
    }
}

impl Transformer for Displays {
    fn visit_stmt(&self, stmt: &mut Stmt) {
        match stmt {
            Stmt::For(for_stmt) => {
                tuple_target(&mut for_stmt.target);
                list_iterable(&mut for_stmt.iter);
            }
            Stmt::Assign(assign) => assign.targets.iter_mut().for_each(tuple_target),
            _ => {}
        }
        transformer::walk_stmt(self, stmt);
    }

    fn visit_comprehension(&self, comprehension: &mut Comprehension) {
        tuple_target(&mut comprehension.target);
        list_iterable(&mut comprehension.iter);
        transformer::walk_comprehension(self, comprehension);
    }

    fn visit_expr(&self, expr: &mut Expr) {
        if let Expr::Compare(compare) = expr {
            for (op, comparator) in compare.ops.iter().zip(compare.comparators.iter_mut()) {
                if matches!(op, CmpOp::In | CmpOp::NotIn) {
                    list_iterable(comparator);
                }
            }
        }
        self.empty_display(expr);
        transformer::walk_expr(self, expr);
    }
}
