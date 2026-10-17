//! Statement-level rewrites: loops over tuple targets and non-name
//! iterables, stores through computed subscripts, tuple targets with
//! non-name elements, and augmented assignment through computed indices.

use super::build::{
    and_all, assign, aug_assign, call_name, for_loop, int, less_than, load, store, subscript, tuple,
    while_loop,
};
use super::{Fresh, is_name, is_simple};
use ruff_python_ast::{Expr, ExprContext, Operator, Stmt};
use ruff_text_size::TextRange;

/// Rewrites every statement of `body`, recursing into nested bodies.
/// `module_level` is true for the module body itself (class bodies are
/// left as written).
pub(crate) fn expand_body(body: &mut Vec<Stmt>, fresh: &Fresh, module_level: bool) {
    let old = std::mem::take(body);
    for stmt in old {
        expand_stmt(stmt, fresh, body, module_level);
    }
}

fn expand_thin(body: &mut thin_vec::ThinVec<Stmt>, fresh: &Fresh) {
    let mut v: Vec<Stmt> = std::mem::take(body).into_iter().collect();
    expand_body(&mut v, fresh, false);
    *body = v.into();
}

fn expand_stmt(stmt: Stmt, fresh: &Fresh, out: &mut Vec<Stmt>, module_level: bool) {
    let stmt = match stmt {
        Stmt::For(s) if !s.is_async => {
            expand_for(s, fresh, out);
            return;
        }
        // `a() if c else b()` as a statement is `if c: a()` / `else: b()`:
        // the branches need no common type.
        Stmt::Expr(s) if matches!(*s.value, Expr::If(_)) => {
            let Expr::If(if_exp) = *s.value else {
                unreachable!()
            };
            let expr_stmt = |value: Expr| {
                Stmt::Expr(ruff_python_ast::StmtExpr {
                    node_index: Default::default(),
                    range: s.range,
                    value: Box::new(value),
                })
            };
            let rewritten = Stmt::If(ruff_python_ast::StmtIf {
                node_index: Default::default(),
                range: s.range,
                test: if_exp.test,
                body: vec![expr_stmt(*if_exp.body)].into(),
                elif_else_clauses: vec![ruff_python_ast::ElifElseClause {
                    node_index: Default::default(),
                    range: s.range,
                    test: None,
                    body: vec![expr_stmt(*if_exp.orelse)].into(),
                }],
            });
            expand_stmt(rewritten, fresh, out, module_level);
            return;
        }
        other => other,
    };
    let mut stmt = stmt;
    match &mut stmt {
        Stmt::FunctionDef(def) => {
            yield_from_loops(&mut def.body, fresh);
            expand_thin(&mut def.body, fresh)
        }
        Stmt::ClassDef(def) => {
            for item in def.body.iter_mut() {
                if let Stmt::FunctionDef(method) = item {
                    yield_from_loops(&mut method.body, fresh);
                    expand_thin(&mut method.body, fresh);
                }
            }
        }
        Stmt::If(s) => {
            expand_thin(&mut s.body, fresh);
            for clause in &mut s.elif_else_clauses {
                expand_thin(&mut clause.body, fresh);
            }
        }
        Stmt::While(s) => {
            expand_thin(&mut s.body, fresh);
            expand_thin(&mut s.orelse, fresh);
        }
        Stmt::With(s) => expand_thin(&mut s.body, fresh),
        Stmt::Try(s) => {
            expand_thin(&mut s.body, fresh);
            for handler in &mut s.handlers {
                let ruff_python_ast::ExceptHandler::ExceptHandler(h) = handler;
                expand_thin(&mut h.body, fresh);
            }
            expand_thin(&mut s.orelse, fresh);
            expand_thin(&mut s.finalbody, fresh);
        }
        Stmt::Match(s) => {
            for case in &mut s.cases {
                expand_thin(&mut case.body, fresh);
            }
        }
        Stmt::Expr(s) => {
            let mut pre = Vec::new();
            hoist_container_receiver(&mut s.value, fresh, &mut pre, s.range);
            if !pre.is_empty() {
                out.extend(pre);
            }
        }
        Stmt::Assign(s) if s.targets.len() == 1 && is_name(&s.targets[0]) => {
            let mut pre = Vec::new();
            hoist_container_receiver(&mut s.value, fresh, &mut pre, s.range);
            if !pre.is_empty() {
                out.extend(pre);
            }
        }
        Stmt::Assign(s) => {
            if let Some(stmts) = super::starred::expand_starred(s, fresh) {
                for stmt in stmts {
                    expand_stmt(stmt, fresh, out, module_level);
                }
                return;
            }
            if let Some(stmts) = expand_assign(s, fresh) {
                for stmt in stmts {
                    expand_stmt(stmt, fresh, out, module_level);
                }
                return;
            }
        }
        Stmt::AugAssign(s) => {
            if let Some(stmts) = expand_aug_assign(s, fresh) {
                out.extend(stmts);
                return;
            }
        }
        _ => {}
    }
    out.push(stmt);
}

/// Hoists `expr` into a fresh temporary unless it is already a name.
fn hoist(expr: Expr, fresh: &Fresh, out: &mut Vec<Stmt>, range: TextRange) -> Expr {
    if is_name(&expr) {
        return expr;
    }
    let tmp = fresh.name();
    out.push(assign(store(&tmp, range), expr, range));
    load(&tmp, range)
}

fn is_call_to<'e>(expr: &'e Expr, name: &str) -> Option<&'e [Expr]> {
    match expr {
        Expr::Call(call) if call.arguments.keywords.is_empty() => match call.func.as_ref() {
            Expr::Name(func) if func.id.as_str() == name => Some(&call.arguments.args),
            _ => None,
        },
        _ => None,
    }
}

fn tuple_elts(expr: &Expr) -> Option<&[Expr]> {
    match expr {
        Expr::Tuple(t) => Some(&t.elts),
        Expr::List(l) => Some(&l.elts),
        _ => None,
    }
}

fn expand_for(mut s: ruff_python_ast::StmtFor, fresh: &Fresh, out: &mut Vec<Stmt>) {
    let range = s.range;
    let mut body: Vec<Stmt> = std::mem::take(&mut s.body).into_iter().collect();
    let mut orelse: Vec<Stmt> = std::mem::take(&mut s.orelse).into_iter().collect();
    expand_body(&mut orelse, fresh, false);
    let target = *s.target;
    let iter = *s.iter;
    let mut pre: Vec<Stmt> = Vec::new();
    let mut head: Vec<Stmt> = Vec::new();
    let (target, iter) = if let (Some([i, x]), Some(args)) =
        (tuple_elts(&target), is_call_to(&iter, "enumerate"))
        && matches!(args.len(), 1 | 2)
        && !args.iter().any(|a| matches!(a, Expr::Starred(_)))
    {
        // `for i, x in enumerate(xs, start)`: a counter beside a loop over
        // `xs`, bumped before the body so `continue` keeps it right.
        let counter = fresh.name();
        let start = args.get(1).cloned().unwrap_or_else(|| int(0, range));
        pre.push(assign(store(&counter, range), start, range));
        head.push(assign(i.clone(), load(&counter, range), range));
        head.push(aug_assign(
            store(&counter, range),
            Operator::Add,
            int(1, range),
            range,
        ));
        (x.clone(), args[0].clone())
    } else if let (Some(targets), Some(args)) = (tuple_elts(&target), is_call_to(&iter, "zip"))
        && targets.len() == args.len()
        && args.len() >= 2
        && !args.iter().any(|a| matches!(a, Expr::Starred(_)))
    {
        // `for a, b in zip(xs, ys)`: an index loop that, like CPython's
        // iterators, compares the index with each sequence's length as it
        // stands at every step (the body may grow or shrink them), the
        // sequences held in temporaries (the body may rebind their names).
        // A `while` keeps the `else` clause's meaning.
        let seqs: Vec<Expr> = args
            .iter()
            .map(|a| {
                let tmp = fresh.name();
                pre.push(assign(store(&tmp, range), a.clone(), range));
                load(&tmp, range)
            })
            .collect();
        let index = fresh.name();
        pre.push(assign(store(&index, range), int(0, range), range));
        let test = and_all(
            seqs.iter()
                .map(|s| {
                    less_than(load(&index, range), call_name("len", vec![s.clone()], range), range)
                })
                .collect(),
            range,
        );
        for (t, seq) in targets.iter().zip(&seqs) {
            head.push(assign(
                t.clone(),
                subscript(seq.clone(), load(&index, range), ExprContext::Load, range),
                range,
            ));
        }
        head.push(aug_assign(store(&index, range), Operator::Add, int(1, range), range));
        head.append(&mut body);
        let mut body = head;
        expand_body(&mut body, fresh, false);
        for stmt in pre {
            expand_stmt(stmt, fresh, out, false);
        }
        out.push(while_loop(test, body, orelse, range));
        return;
    } else if let (Some([k, v]), Expr::Call(call)) = (tuple_elts(&target), &iter)
        && let Expr::Attribute(attr) = call.func.as_ref()
        && attr.attr.as_str() == "items"
        && call.arguments.args.is_empty()
        && call.arguments.keywords.is_empty()
    {
        // `for k, v in d.items()`: loop over the keys, read each value.
        let dict = hoist((*attr.value).clone(), fresh, &mut pre, range);
        let key = if is_name(k) {
            k.clone()
        } else {
            store(&fresh.name(), range)
        };
        let key_load = match &key {
            Expr::Name(n) => load(n.id.as_str(), range),
            _ => unreachable!(),
        };
        if !is_name(k) {
            head.push(assign(k.clone(), key_load.clone(), range));
        }
        head.push(assign(
            v.clone(),
            subscript(dict.clone(), key_load, ExprContext::Load, range),
            range,
        ));
        (key, dict)
    } else {
        (target, iter)
    };
    // Any other tuple target -- or what is left of one above, such as the
    // item of `for i, (a, b) in enumerate(xs)`: bind a temporary, unpack it
    // right after the head.
    let target = if is_name(&target) {
        target
    } else {
        let tmp = fresh.name();
        head.push(assign(target, load(&tmp, range), range));
        store(&tmp, range)
    };
    let iter = if is_name(&iter)
        || is_call_to(&iter, "range").is_some()
        || reaches_through_import(&iter, fresh)
    {
        iter
    } else {
        hoist(iter, fresh, &mut pre, range)
    };
    head.append(&mut body);
    let mut body = head;
    // The rewritten head and the original body may themselves need
    // rewriting (an unpacking assignment, a nested loop).
    let mut loop_stmt = for_loop(target, iter, Vec::new(), orelse, range);
    expand_body(&mut body, fresh, false);
    if let Stmt::For(f) = &mut loop_stmt {
        f.body = body.into();
    }
    for stmt in pre {
        expand_stmt(stmt, fresh, out, false);
    }
    out.push(loop_stmt);
}

/// `m.attr` / `m.f(...)` (or deeper) rooted at an imported name: the
/// shapes `HirStmt::ForObject` iterates as a CPython object.
fn reaches_through_import(expr: &Expr, fresh: &Fresh) -> bool {
    let attr = match expr {
        Expr::Attribute(attr) => attr,
        Expr::Call(call) => match call.func.as_ref() {
            Expr::Attribute(attr) => attr,
            _ => return false,
        },
        _ => return false,
    };
    let mut root = attr.value.as_ref();
    loop {
        match root {
            Expr::Attribute(inner) => root = inner.value.as_ref(),
            Expr::Name(name) => return fresh.import_roots.contains(name.id.as_str()),
            _ => return false,
        }
    }
}

/// `a, b[i] = v` -> `a, t = v; b[i] = t`; `x.y[i] = v` -> `t = x.y;
/// t[i] = v` (with `v` evaluated first, as CPython does).
fn expand_assign(s: &mut ruff_python_ast::StmtAssign, fresh: &Fresh) -> Option<Vec<Stmt>> {
    let range = s.range;
    let [target] = s.targets.as_mut_slice() else {
        return None;
    };
    match target {
        Expr::Subscript(sub) if !is_name(&sub.value) => {
            let mut out = Vec::new();
            let value = if is_simple(&s.value) {
                (*s.value).clone()
            } else {
                hoist((*s.value).clone(), fresh, &mut out, range)
            };
            let base = hoist((*sub.value).clone(), fresh, &mut out, range);
            out.push(assign(
                subscript(base, (*sub.slice).clone(), ExprContext::Store, range),
                value,
                range,
            ));
            Some(out)
        }
        Expr::Tuple(_) | Expr::List(_) => {
            let elts = match target {
                Expr::Tuple(t) => &mut t.elts,
                Expr::List(l) => &mut l.elts,
                _ => unreachable!(),
            };
            if let Some(split) = split_inert_tuple_assign(elts, &s.value, range) {
                return Some(split);
            }
            if elts.iter().all(is_name) || elts.iter().any(|e| matches!(e, Expr::Starred(_))) {
                return None;
            }
            let mut after = Vec::new();
            for elt in elts.iter_mut() {
                if !is_name(elt) {
                    let tmp = fresh.name();
                    let real = std::mem::replace(elt, store(&tmp, range));
                    after.push(assign(real, load(&tmp, range), range));
                }
            }
            let new_target = tuple(elts.clone(), ExprContext::Store, range);
            let mut out = vec![assign(new_target, (*s.value).clone(), range)];
            out.extend(after);
            Some(out)
        }
        _ => None,
    }
}

/// `a, b = [], set()` -> `a = []` / `b = set()` when every value is inert
/// (it can observe no target, and nothing the program can catch tells
/// the orders apart): each name then gets its
/// own binding, so an empty container is typed by its own later uses as
/// a plain `a = []` is. Evaluating all values before storing any is
/// unobservable for inert values.
fn split_inert_tuple_assign(targets: &[Expr], value: &Expr, range: TextRange) -> Option<Vec<Stmt>> {
    let values = tuple_elts(value)?;
    if values.len() != targets.len() || targets.len() < 2 || !targets.iter().all(is_name) {
        return None;
    }
    let names: Vec<&str> = targets
        .iter()
        .filter_map(|t| t.as_name_expr().map(|n| n.id.as_str()))
        .collect();
    if !values.iter().all(|v| is_inert(v, &names)) {
        return None;
    }
    Some(
        targets
            .iter()
            .zip(values)
            .map(|(t, v)| assign(t.clone(), v.clone(), range))
            .collect(),
    )
}

/// A literal, a name other than `targets`, a display or comprehension of
/// inert values, or a call of `set`, `dict`, `list` or `range` with inert
/// arguments.
fn is_inert(expr: &Expr, targets: &[&str]) -> bool {
    let all = |exprs: &[Expr]| exprs.iter().all(|e| is_inert(e, targets));
    let generators = |generators: &[ruff_python_ast::Comprehension]| {
        generators
            .iter()
            .all(|g| !g.is_async && is_inert(&g.iter, targets) && all(&g.ifs))
    };
    match expr {
        Expr::Name(n) => !targets.contains(&n.id.as_str()),
        Expr::NumberLiteral(_)
        | Expr::StringLiteral(_)
        | Expr::BooleanLiteral(_)
        | Expr::NoneLiteral(_) => true,
        Expr::UnaryOp(u) => matches!(*u.operand, Expr::NumberLiteral(_)),
        Expr::List(l) => all(&l.elts),
        Expr::Tuple(t) => all(&t.elts),
        Expr::Set(s) => all(&s.elts),
        Expr::Dict(d) => d
            .items
            .iter()
            .all(|item| item.key.as_ref().is_some_and(|k| is_inert(k, targets)) && is_inert(&item.value, targets)),
        Expr::ListComp(c) => is_inert(&c.elt, targets) && generators(&c.generators),
        Expr::SetComp(c) => is_inert(&c.elt, targets) && generators(&c.generators),
        Expr::Call(call) => {
            call.arguments.keywords.is_empty()
                && all(&call.arguments.args)
                && matches!(call.func.as_ref(), Expr::Name(f)
                    if matches!(f.id.as_str(), "set" | "dict" | "list" | "range")
                        && !targets.contains(&f.id.as_str()))
        }
        _ => false,
    }
}

/// `x.y[i] += v` / `a[i + 1] += v` -> bind the base and index first.
fn expand_aug_assign(s: &mut ruff_python_ast::StmtAugAssign, fresh: &Fresh) -> Option<Vec<Stmt>> {
    let range = s.range;
    let Expr::Subscript(sub) = s.target.as_ref() else {
        return None;
    };
    let simple_index = is_simple(&sub.slice);
    if is_name(&sub.value) && simple_index {
        return None;
    }
    if matches!(sub.slice.as_ref(), Expr::Slice(_)) {
        return None;
    }
    let mut out = Vec::new();
    let base = hoist((*sub.value).clone(), fresh, &mut out, range);
    let index = if simple_index {
        (*sub.slice).clone()
    } else {
        hoist((*sub.slice).clone(), fresh, &mut out, range)
    };
    out.push(aug_assign(
        subscript(base, index, ExprContext::Store, range),
        s.op,
        (*s.value).clone(),
        range,
    ));
    Some(out)
}

/// `g[u].append(v)` -> `t = g[u]; t.append(v)`: the container methods HIR
/// lowers to dedicated nodes (`append`, `pop`, `get`, `add`) take a name or
/// an attribute receiver, so any other receiver is bound first (it is
/// evaluated first in CPython too).
fn hoist_container_receiver(
    value: &mut Expr,
    fresh: &Fresh,
    out: &mut Vec<Stmt>,
    range: TextRange,
) {
    let Expr::Call(call) = value else {
        return;
    };
    let Expr::Attribute(attr) = call.func.as_mut() else {
        return;
    };
    if !matches!(attr.attr.as_str(), "append" | "pop" | "get" | "add")
        || matches!(attr.value.as_ref(), Expr::Name(_) | Expr::Attribute(_))
    {
        return;
    }
    let receiver = std::mem::replace(attr.value.as_mut(), load("_", range));
    *attr.value = hoist(receiver, fresh, out, range);
}

/// `yield from it` as a statement of a function's `body`, where `it` is
/// plainly not a generator (a display, a comprehension, a literal or a call
/// of a builtin such as `range`), becomes `for v in it: yield v`: such an
/// iterator has no `send`/`throw`/`close` and returns no value, so the loop
/// is delegation. Every other operand keeps its `yield from`, which
/// `pycc_hir` lowers to delegation. Nested functions and classes are left
/// to their own expansion; a `yield from` outside a function keeps its
/// `L0001`.
fn yield_from_loops(body: &mut [Stmt], fresh: &Fresh) {
    /// Whether `operand` cannot evaluate to a generator.
    fn plainly_not_generator(operand: &Expr) -> bool {
        match operand {
            Expr::List(_)
            | Expr::Tuple(_)
            | Expr::Set(_)
            | Expr::Dict(_)
            | Expr::ListComp(_)
            | Expr::SetComp(_)
            | Expr::DictComp(_)
            | Expr::Generator(_)
            | Expr::StringLiteral(_)
            | Expr::BytesLiteral(_) => true,
            Expr::Call(call) => matches!(
                call.func.as_ref(),
                Expr::Name(name) if matches!(
                    name.id.as_str(),
                    "range" | "list" | "tuple" | "set" | "dict" | "str" | "sorted"
                        | "reversed" | "enumerate" | "zip" | "map" | "filter" | "frozenset"
                )
            ),
            _ => false,
        }
    }
    use ruff_python_ast::visitor::transformer::{self, Transformer};
    struct Loops<'f>(&'f Fresh);
    impl Transformer for Loops<'_> {
        fn visit_stmt(&self, stmt: &mut Stmt) {
            match stmt {
                Stmt::FunctionDef(_) | Stmt::ClassDef(_) => {}
                Stmt::Expr(s) if matches!(&*s.value, Expr::YieldFrom(y) if plainly_not_generator(&y.value)) =>
                {
                    let range = s.range;
                    let Expr::YieldFrom(yield_from) =
                        std::mem::replace(s.value.as_mut(), load("None", range))
                    else {
                        unreachable!()
                    };
                    let value = self.0.name();
                    let yield_stmt = Stmt::Expr(ruff_python_ast::StmtExpr {
                        node_index: Default::default(),
                        range,
                        value: Box::new(Expr::Yield(ruff_python_ast::ExprYield {
                            node_index: Default::default(),
                            range,
                            value: Some(Box::new(load(&value, range))),
                        })),
                    });
                    *stmt = for_loop(
                        store(&value, range),
                        *yield_from.value,
                        vec![yield_stmt],
                        Vec::new(),
                        range,
                    );
                }
                _ => transformer::walk_stmt(self, stmt),
            }
        }
    }
    let loops = Loops(fresh);
    for stmt in body.iter_mut() {
        loops.visit_stmt(stmt);
    }
}
