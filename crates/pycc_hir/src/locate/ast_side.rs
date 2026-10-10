//! The AST half of [`super`]: item spans from the desugared AST.

use super::{ItemSpans, NodeKey, StmtSpans};
use crate::HirItem;
use pycc_ast::visitor::{Visitor, walk_expr};
use pycc_ast::{
    Arguments, ElifElseClause, ExceptHandler, Expr, ExprContext, ModModule, Number, Stmt,
    StmtFunctionDef, UnaryOp,
};
use pycc_diag::Span;

/// The spans of every item of a lowered module, `None` for an item no
/// source statement wrote. `module` is the desugared AST the items were
/// lowered from and `origins` the start of each item's top-level statement
/// (`pycc_hir::last_item_origins`, `u32::MAX` for none).
pub fn item_spans(
    module: &ModModule,
    source: &str,
    items: &[HirItem],
    origins: &[u32],
) -> Vec<Option<ItemSpans>> {
    let mut spans: Vec<Option<ItemSpans>> = items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let origin = *origins.get(index)?;
            let stmt = module
                .body
                .iter()
                .find(|stmt| pycc_ast::stmt_range(stmt).start == origin)?;
            match item {
                HirItem::TopLevelStmt(_) => Some(ItemSpans {
                    header: span(stmt_header(stmt)),
                    returns: None,
                    stmts: collect_body(std::slice::from_ref(stmt)),
                }),
                HirItem::Function { name, .. } => {
                    // A property's getter and setter, or a redefinition in a
                    // class body, share one name: the n-th such item from
                    // this statement is the n-th matching `def`.
                    let ordinal = items[..index]
                        .iter()
                        .zip(origins)
                        .filter(|(earlier, earlier_origin)| {
                            **earlier_origin == origin
                                && matches!(earlier, HirItem::Function { name: n, .. } if n == name)
                        })
                        .count();
                    let def = find_def(stmt, name, ordinal)?;
                    Some(function_spans(def))
                }
            }
        })
        .collect();
    for item in spans.iter_mut().flatten() {
        item.header = item.header.map(|span| first_line(source, span));
        for stmt in &mut item.stmts {
            stmt.span = stmt.span.map(|span| first_line(source, span));
        }
    }
    spans
}

/// `span` cut at the end of its first line, so a statement's caret never
/// runs into its body.
fn first_line(source: &str, span: Span) -> Span {
    let start = span.start as usize;
    let end = source
        .get(start..)
        .and_then(|rest| rest.find('\n'))
        .map_or(span.end, |newline| (start + newline) as u32);
    let end = end.min(span.end);
    let trimmed = source
        .get(start..end as usize)
        .map_or(end, |text| start as u32 + text.trim_end().len() as u32);
    if trimmed > span.start {
        Span::new(span.start, trimmed)
    } else {
        span
    }
}

/// The `def` that HIR function `name` was lowered from, inside top-level
/// statement `stmt`: the statement itself, or a method of the class it
/// defines (`Class.method`, `Class.method.classmethod`, ...).
fn find_def<'a>(stmt: &'a Stmt, name: &str, ordinal: usize) -> Option<&'a StmtFunctionDef> {
    match stmt {
        Stmt::FunctionDef(def) if def.name.as_str() == name => Some(def),
        Stmt::ClassDef(class) => {
            let mut parts = name.split('.');
            if parts.next()? != class.name.as_str() {
                return None;
            }
            let method = parts.next()?;
            class
                .body
                .iter()
                .filter_map(|stmt| match stmt {
                    Stmt::FunctionDef(def) if def.name.as_str() == method => Some(def),
                    _ => None,
                })
                .nth(ordinal)
        }
        _ => None,
    }
}

fn function_spans(def: &StmtFunctionDef) -> ItemSpans {
    let start = u32::from(def.name.range.start());
    let mut end = std::ops::Range::<u32>::from(def.parameters.range).end;
    let returns = def.returns.as_deref().map(pycc_ast::expr_range);
    if let Some(returns) = &returns {
        end = end.max(returns.end);
    }
    ItemSpans {
        header: span(start..end),
        returns: returns.and_then(span),
        stmts: collect_body(&def.body),
    }
}

fn span(range: std::ops::Range<u32>) -> Option<Span> {
    // A node the desugaring synthesized without a source construct of its
    // own carries an empty range; it still counts for pairing, unplaced.
    (range.end > range.start).then(|| Span::new(range.start, range.end))
}

/// A statement's header: the line a caret should underline.
fn stmt_header(stmt: &Stmt) -> std::ops::Range<u32> {
    let range = pycc_ast::stmt_range(stmt);
    let through = |expr: &Expr| range.start..pycc_ast::expr_range(expr).end;
    match stmt {
        Stmt::If(s) => through(&s.test),
        Stmt::While(s) => through(&s.test),
        Stmt::For(s) => through(&s.iter),
        Stmt::Match(s) => through(&s.subject),
        Stmt::Try(_) => range.start..range.start + 3,
        Stmt::With(s) => s
            .items
            .last()
            .map_or(range.clone(), |item| through(&item.context_expr)),
        _ => range,
    }
}

fn collect_body(body: &[Stmt]) -> Vec<StmtSpans> {
    let mut out = Vec::new();
    for stmt in body {
        collect_stmt(stmt, &mut out);
    }
    out
}

/// Records `stmt`'s entries, then those of its nested statements, in the
/// pre-order `hir_side::collect_stmts` walks the HIR in.
fn collect_stmt(stmt: &Stmt, out: &mut Vec<StmtSpans>) {
    let header = span(stmt_header(stmt));
    let mut entry = |key: String, exprs: Vec<(NodeKey, Option<Span>)>| {
        out.push(StmtSpans {
            key: NodeKey::new(key),
            span: header,
            exprs,
        });
    };
    match stmt {
        Stmt::Expr(s) => entry("expr".into(), exprs_of([&*s.value])),
        Stmt::Assign(s) => {
            // A chained assignment, and a tuple unpacking, first bind the
            // value to a temporary named after the statement's offset.
            let start = pycc_ast::stmt_range(stmt).start;
            if s.targets.len() > 1 {
                let temp = crate::stmt::chain_assign::synthesize_chain_temp_name(start);
                entry(format!("={temp}"), exprs_of([&*s.value]));
            } else if matches!(s.targets.first(), Some(Expr::Tuple(_) | Expr::List(_))) {
                let temp = crate::stmt::unpack::synthesize_unpack_temp_name(start);
                entry(format!("={temp}"), exprs_of([&*s.value]));
            }
            for target in &s.targets {
                for (key, exprs) in assign_target(target, &s.value) {
                    entry(key, exprs);
                }
            }
        }
        Stmt::AnnAssign(s) => {
            if let Expr::Name(name) = &*s.target {
                entry(format!("={}", name.id), exprs_of(s.value.as_deref()));
            } else {
                for (key, exprs) in
                    assign_target(&s.target, s.value.as_deref().unwrap_or(&s.target))
                {
                    entry(key, exprs);
                }
            }
        }
        Stmt::AugAssign(s) => {
            // `t op= v` lowers to `t = t op v`: the operator node and the
            // read of the target come before the value.
            let mut collector = Collector::default();
            let whole = span(pycc_ast::stmt_range(stmt));
            let key = match &*s.target {
                Expr::Name(name) => format!("={}", name.id),
                Expr::Attribute(attr) => {
                    collector.visit_expr(&attr.value);
                    format!("attr=:{}", attr.attr)
                }
                Expr::Subscript(sub) => {
                    collector.visit_expr(&sub.slice);
                    "[]=".into()
                }
                _ => "=".into(),
            };
            collector.push(format!("bin:{}", s.op.as_str()), whole);
            collector.visit_expr_as_load(&s.target);
            collector.visit_expr(&s.value);
            entry(key, collector.out);
        }
        Stmt::If(s) => entry("if".into(), exprs_of([&*s.test])),
        Stmt::While(s) => entry("while".into(), exprs_of([&*s.test])),
        Stmt::For(s) => {
            let var = match &*s.target {
                Expr::Name(name) => name.id.to_string(),
                _ => String::new(),
            };
            entry(format!("for:{var}"), exprs_of([&*s.iter]));
        }
        Stmt::Return(s) => entry("return".into(), exprs_of(s.value.as_deref())),
        Stmt::Raise(s) => entry(
            "raise".into(),
            exprs_of(s.exc.as_deref().into_iter().chain(s.cause.as_deref())),
        ),
        Stmt::Assert(s) => {
            // `assert t, m` lowers to `if t: pass` / `else: raise
            // AssertionError(m)`.
            entry("if".into(), exprs_of([&*s.test]));
            entry("raise".into(), exprs_of(s.msg.as_deref()));
        }
        Stmt::Try(_) => entry("try".into(), Vec::new()),
        Stmt::Match(s) => entry(
            "match".into(),
            exprs_of(
                std::iter::once(&*s.subject)
                    .chain(s.cases.iter().filter_map(|case| case.guard.as_deref())),
            ),
        ),
        Stmt::Delete(s) => {
            for target in &s.targets {
                match target {
                    Expr::Name(name) => entry(format!("del:{}", name.id), Vec::new()),
                    Expr::Subscript(sub) => {
                        let mut collector = Collector::default();
                        collector.visit_expr(&sub.value);
                        collector.visit_slice(&sub.slice);
                        entry("del".into(), collector.out);
                    }
                    Expr::Attribute(attr) => entry("del".into(), exprs_of([&*attr.value])),
                    _ => {}
                }
            }
        }
        Stmt::Break(_) => entry("break".into(), Vec::new()),
        Stmt::Continue(_) => entry("continue".into(), Vec::new()),
        Stmt::Import(_) | Stmt::ImportFrom(_) => entry("import".into(), Vec::new()),
        _ => {}
    }
    match stmt {
        Stmt::If(s) => {
            push_all(&s.body, out);
            for clause in &s.elif_else_clauses {
                elif_clause(clause, out);
            }
        }
        Stmt::While(s) => {
            push_all(&s.body, out);
            push_all(&s.orelse, out);
        }
        Stmt::For(s) => {
            push_all(&s.body, out);
            push_all(&s.orelse, out);
        }
        Stmt::Try(s) => {
            push_all(&s.body, out);
            for ExceptHandler::ExceptHandler(handler) in &s.handlers {
                push_all(&handler.body, out);
            }
            push_all(&s.orelse, out);
            push_all(&s.finalbody, out);
        }
        Stmt::Match(s) => {
            for case in &s.cases {
                push_all(&case.body, out);
            }
        }
        Stmt::With(s) => push_all(&s.body, out),
        _ => {}
    }
}

fn push_all(body: &[Stmt], out: &mut Vec<StmtSpans>) {
    for stmt in body {
        collect_stmt(stmt, out);
    }
}

/// An `elif` lowers to an `if` nested in the enclosing one's `else`.
fn elif_clause(clause: &ElifElseClause, out: &mut Vec<StmtSpans>) {
    if let Some(test) = &clause.test {
        let start = std::ops::Range::<u32>::from(clause.range).start;
        out.push(StmtSpans {
            key: NodeKey::new("if"),
            span: span(start..pycc_ast::expr_range(test).end),
            exprs: exprs_of([test]),
        });
    }
    push_all(&clause.body, out);
}

/// The entries one assignment target contributes: a name binds `=name`,
/// an attribute `attr=:a`, a subscript `[]=`, and a tuple or list target
/// one `=name` per name it unpacks into.
fn assign_target(target: &Expr, value: &Expr) -> Vec<(String, Vec<(NodeKey, Option<Span>)>)> {
    match target {
        Expr::Name(name) => vec![(format!("={}", name.id), exprs_of([value]))],
        Expr::Attribute(attr) => vec![(
            format!("attr=:{}", attr.attr),
            exprs_of([&*attr.value, value]),
        )],
        Expr::Subscript(sub) => {
            let mut collector = Collector::default();
            collector.visit_slice(&sub.slice);
            collector.visit_expr(value);
            vec![("[]=".into(), collector.out)]
        }
        Expr::Tuple(tuple) => unpack_targets(&tuple.elts, value),
        Expr::List(list) => unpack_targets(&list.elts, value),
        _ => Vec::new(),
    }
}

fn unpack_targets(elts: &[Expr], value: &Expr) -> Vec<(String, Vec<(NodeKey, Option<Span>)>)> {
    elts.iter()
        .flat_map(|elt| assign_target(elt, value))
        .collect()
}

fn exprs_of<'a>(exprs: impl IntoIterator<Item = &'a Expr>) -> Vec<(NodeKey, Option<Span>)> {
    let mut collector = Collector::default();
    for expr in exprs {
        collector.visit_expr(expr);
    }
    collector.out
}

/// Records the key of every expression it visits, in the order
/// `hir_side::collect_exprs` walks the HIR.
#[derive(Default)]
struct Collector {
    out: Vec<(NodeKey, Option<Span>)>,
}

impl Collector {
    fn push(&mut self, key: String, at: Option<Span>) {
        self.out.push((NodeKey::new(key), at));
    }

    /// The target of an augmented assignment, which lowering reads.
    fn visit_expr_as_load(&mut self, target: &Expr) {
        match target {
            Expr::Name(name) => {
                self.push(format!("n:{}", name.id), span(pycc_ast::expr_range(target)))
            }
            other => self.visit_expr(other),
        }
    }

    /// A subscript's index: a slice's bounds, or the index itself.
    fn visit_slice(&mut self, slice: &Expr) {
        match slice {
            Expr::Slice(bounds) => {
                for bound in [&bounds.lower, &bounds.upper, &bounds.step]
                    .into_iter()
                    .flatten()
                {
                    self.visit_expr(bound);
                }
            }
            other => self.visit_expr(other),
        }
    }

    fn visit_call_arguments(&mut self, arguments: &Arguments) {
        for arg in &arguments.args {
            self.visit_expr(arg);
        }
        for keyword in &arguments.keywords {
            self.visit_expr(&keyword.value);
        }
    }
}

impl<'a> Visitor<'a> for Collector {
    fn visit_stmt(&mut self, _stmt: &'a Stmt) {}

    fn visit_annotation(&mut self, _expr: &'a Expr) {}

    fn visit_expr(&mut self, expr: &'a Expr) {
        let at = span(pycc_ast::expr_range(expr));
        match expr {
            Expr::Name(name) => {
                if matches!(name.ctx, ExprContext::Load) {
                    self.push(format!("n:{}", name.id), at);
                }
            }
            Expr::NumberLiteral(lit) => {
                if let Some(key) = number_key(&lit.value, false) {
                    self.push(key, at);
                }
            }
            // `-1` folds into one literal node.
            Expr::UnaryOp(unary)
                if matches!(unary.op, UnaryOp::USub | UnaryOp::UAdd)
                    && matches!(&*unary.operand, Expr::NumberLiteral(_)) =>
            {
                if let Expr::NumberLiteral(lit) = &*unary.operand
                    && let Some(key) = number_key(&lit.value, matches!(unary.op, UnaryOp::USub))
                {
                    self.push(key, at);
                }
            }
            Expr::UnaryOp(unary) => {
                self.push(format!("un:{}", unary.op.as_str()), at);
                self.visit_expr(&unary.operand);
            }
            Expr::BooleanLiteral(lit) => {
                self.push(
                    format!("b:{}", if lit.value { "True" } else { "False" }),
                    at,
                );
            }
            Expr::NoneLiteral(_) => self.push("none".into(), at),
            Expr::StringLiteral(lit) => self.push(format!("s:{}", lit.value.to_str()), at),
            Expr::FString(_) => {
                self.push("fstr".into(), at);
                walk_expr(self, expr);
            }
            Expr::BinOp(bin) => {
                self.push(format!("bin:{}", bin.op.as_str()), at);
                walk_expr(self, expr);
            }
            Expr::BoolOp(bool_op) => {
                // `a or b or c` lowers to two nested binary nodes.
                for _ in 1..bool_op.values.len() {
                    self.push(format!("bool:{}", bool_op.op.as_str()), at);
                }
                walk_expr(self, expr);
            }
            Expr::Compare(compare) => {
                let key = match compare.ops.as_ref() {
                    [op] => format!("cmp:{}", op.as_str()),
                    _ => "cmpchain".into(),
                };
                self.push(key, at);
                walk_expr(self, expr);
            }
            Expr::If(_) => {
                self.push("ifexp".into(), at);
                walk_expr(self, expr);
            }
            Expr::List(_) => {
                self.push("list".into(), at);
                walk_expr(self, expr);
            }
            Expr::Tuple(_) => {
                self.push("tuple".into(), at);
                walk_expr(self, expr);
            }
            Expr::Dict(_) => {
                self.push("dict".into(), at);
                walk_expr(self, expr);
            }
            Expr::Set(_) => {
                self.push("set".into(), at);
                walk_expr(self, expr);
            }
            Expr::ListComp(_) | Expr::SetComp(_) | Expr::DictComp(_) | Expr::Generator(_) => {
                self.push("comp".into(), at);
                walk_expr(self, expr);
            }
            Expr::Attribute(attr) => {
                self.push(format!("a:{}", attr.attr), at);
                self.visit_expr(&attr.value);
            }
            Expr::Subscript(sub) => {
                let key = if matches!(&*sub.slice, Expr::Slice(_)) {
                    "[:]"
                } else {
                    "[]"
                };
                self.push(key.into(), at);
                self.visit_expr(&sub.value);
                self.visit_slice(&sub.slice);
            }
            Expr::Named(named) => {
                if let Expr::Name(target) = &*named.target {
                    self.push(format!("walrus:{}", target.id), at);
                }
                self.visit_expr(&named.value);
            }
            Expr::Call(call) => {
                match &*call.func {
                    Expr::Name(name) => self.push(format!("c:{}", name.id), at),
                    Expr::Attribute(attr) => {
                        self.push(format!("m:{}", attr.attr), at);
                        self.visit_expr(&attr.value);
                    }
                    Expr::Subscript(sub) if matches!(&*sub.value, Expr::Name(_)) => {
                        if let Expr::Name(name) = &*sub.value {
                            self.push(format!("c:{}", name.id), at);
                        }
                    }
                    func => {
                        self.push("call()".into(), at);
                        self.visit_expr(func);
                    }
                }
                self.visit_call_arguments(&call.arguments);
            }
            _ => walk_expr(self, expr),
        }
    }
}

fn number_key(value: &Number, negate: bool) -> Option<String> {
    let sign = if negate { "-" } else { "" };
    match value {
        Number::Int(int) => Some(format!("i:{sign}{int}")),
        Number::Float(float) => Some(format!(
            "f:{}",
            (if negate { -float } else { *float }).to_bits()
        )),
        Number::Complex { .. } => None,
    }
}
