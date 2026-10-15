//! Native `str | None` locals in non-strict code.
//!
//! A local annotated `str | None` (or bound only to `str` values and
//! `None`) is a CPython object in non-strict mode, so `char = src[pos]`
//! boxes the character, and `char == "x"` or `char in CHARS` goes through
//! CPython. When a function binds such a local *only* to native `str`
//! values and `None`, this pass keeps it native: a `str` slot that holds
//! the `str`, or null for `None`. Its comparisons with a string literal,
//! its membership tests against a `frozenset[str]` and its `is None` tests
//! become runtime calls that read null as `None`
//! (`pycc_rt::optional_str`); any other read boxes it back into the object
//! it stands for -- CPython's `None`, or the `str` -- so every other use is
//! unchanged.
//!
//! A local qualifies only when every mention of its name in the function
//! is one of those bindings or a plain read this pass rewrites: a
//! parameter, a module global, a name bound any other way (a loop, an
//! `except ... as`, a walrus, a comprehension, `del`) or read where this
//! pass cannot see it keeps its object slot. Functions that build closures
//! or yield are left alone.

use crate::{MirExpr, MirItem, MirModule, MirStmt, Ty};
use pycc_hir::CmpOpKind;
use pycc_hir::builtins::BuiltinSig;
use std::collections::{HashMap, HashSet};

/// `pycc_rt_optional_str_none() -> str`: the null `str` standing for `None`.
const NONE: &str = "pycc_rt_optional_str_none";
/// `pycc_rt_optional_str_is_none(s) -> bool`.
const IS_NONE: &str = "pycc_rt_optional_str_is_none";
/// `pycc_rt_optional_str_eq(s, text) -> bool`: `s == text`, false for `None`.
const EQ: &str = "pycc_rt_optional_str_eq";
/// `pycc_rt_optional_str_in(s, set) -> bool`: `s in set`, false for `None`.
const IN: &str = "pycc_rt_optional_str_in";

/// Keeps every qualifying `str | None` local of every function native.
pub(crate) fn rewrite(module: &mut MirModule) {
    let globals = module_names(module);
    for item in &mut module.items {
        if let MirItem::Function { params, body, .. } = item {
            let params: HashSet<&str> = params.iter().map(|(name, _)| name.as_str()).collect();
            let locals = qualifying(body, &params, &globals);
            if !locals.is_empty() {
                for stmt in body.iter_mut() {
                    rewrite_stmt(stmt, &locals);
                }
            }
        }
    }
}

/// Every name the module binds at its top level, and every function name.
fn module_names(module: &MirModule) -> HashSet<String> {
    let mut names = HashSet::new();
    for item in &module.items {
        match item {
            MirItem::Function { name, .. } => {
                names.insert(name.clone());
            }
            MirItem::ForeignImport { local_name, .. } => {
                names.insert(local_name.clone());
            }
            MirItem::TopLevelStmt(stmt) => assigned(stmt, &mut names),
        }
    }
    names
}

fn assigned(stmt: &MirStmt, names: &mut HashSet<String>) {
    if let MirStmt::Assign { target, .. } = stmt {
        names.insert(target.clone());
    }
    for body in stmt.bodies() {
        for inner in body {
            assigned(inner, names);
        }
    }
}

/// The locals of `body` that qualify.
fn qualifying(
    body: &[MirStmt],
    params: &HashSet<&str>,
    globals: &HashSet<String>,
) -> HashSet<String> {
    if body.iter().any(builds_closure_or_yields) {
        return HashSet::new();
    }
    // name -> (binds a str, every binding a str or None, bindings, reads)
    let mut seen: HashMap<String, (bool, bool, usize)> = HashMap::new();
    let mut reads: HashMap<String, usize> = HashMap::new();
    for stmt in body {
        survey(stmt, &mut seen, &mut reads);
    }
    let text = format!("{body:?}");
    seen.into_iter()
        .filter(|(name, (some_str, all_ok, bindings))| {
            *some_str
                && *all_ok
                && !params.contains(name.as_str())
                && !globals.contains(name)
                && !name.starts_with('0')
                && mentions(&text, name) == bindings + reads.get(name).copied().unwrap_or(0)
        })
        .map(|(name, _)| name)
        .collect()
}

/// How many times `name` appears as a string in a `Debug` rendering.
fn mentions(text: &str, name: &str) -> usize {
    text.matches(&format!("{name:?}")).count()
}

fn builds_closure_or_yields(stmt: &MirStmt) -> bool {
    matches!(stmt, MirStmt::Yield(_) | MirStmt::GenReturn(_))
        || stmt.exprs().into_iter().any(expr_closure_or_yield)
        || stmt
            .bodies()
            .into_iter()
            .any(|body| body.iter().any(builds_closure_or_yields))
}

fn expr_closure_or_yield(expr: &MirExpr) -> bool {
    matches!(
        expr,
        MirExpr::MakeClosure { .. } | MirExpr::GenYield { .. } | MirExpr::GenYieldFrom { .. }
    ) || expr.children().into_iter().any(expr_closure_or_yield)
}

/// Records every `Assign` binding and every `Name` read.
fn survey(
    stmt: &MirStmt,
    seen: &mut HashMap<String, (bool, bool, usize)>,
    reads: &mut HashMap<String, usize>,
) {
    if let MirStmt::Assign { target, value } = stmt {
        let entry = seen.entry(target.clone()).or_insert((false, true, 0));
        entry.2 += 1;
        match native_value(value) {
            Some(Bound::Str) => entry.0 = true,
            Some(Bound::None) => {}
            None => entry.1 = false,
        }
    }
    for expr in stmt.exprs() {
        count_reads(expr, reads);
    }
    for body in stmt.bodies() {
        for inner in body {
            survey(inner, seen, reads);
        }
    }
}

fn count_reads(expr: &MirExpr, reads: &mut HashMap<String, usize>) {
    if let MirExpr::Name { name, .. } = expr {
        *reads.entry(name.clone()).or_insert(0) += 1;
    }
    for child in expr.children() {
        count_reads(child, reads);
    }
}

enum Bound {
    Str,
    None,
}

/// What an object binding `value` boxes: a native `str` or `None`.
fn native_value(value: &MirExpr) -> Option<Bound> {
    let MirExpr::ObjectBox(inner) = value else {
        return None;
    };
    match inner.as_ref() {
        MirExpr::NoneLiteral => Some(Bound::None),
        inner if inner.ty() == Ty::Str => Some(Bound::Str),
        _ => None,
    }
}

fn rt_call(symbol: &'static str, args: Vec<MirExpr>, ret: Ty) -> MirExpr {
    let params: Vec<Ty> = args.iter().map(MirExpr::ty).collect();
    MirExpr::RtCall(Box::new(crate::RtCallExpr {
        sig: BuiltinSig {
            symbol,
            word_args: vec![false; params.len()],
            params,
            ret,
            ret_word: false,
            kind_args: Vec::new(),
            desc_args: Vec::new(),
        },
        args,
    }))
}

fn native_read(name: &str) -> MirExpr {
    MirExpr::Name {
        name: name.to_string(),
        ty: Ty::Str,
    }
}

fn negated(expr: MirExpr, negate: bool) -> MirExpr {
    if negate {
        MirExpr::Not(Box::new(expr))
    } else {
        expr
    }
}

fn rewrite_stmt(stmt: &mut MirStmt, locals: &HashSet<String>) {
    if let MirStmt::Assign { target, value } = stmt
        && locals.contains(target)
    {
        let MirExpr::ObjectBox(inner) = value else {
            unreachable!("a qualifying local is only bound by boxed values")
        };
        *value = match inner.as_ref() {
            MirExpr::NoneLiteral => rt_call(NONE, Vec::new(), Ty::Str),
            _ => {
                let mut inner = (**inner).clone();
                rewrite_expr(&mut inner, locals, false);
                inner
            }
        };
        return;
    }
    let truth = matches!(stmt, MirStmt::If { .. } | MirStmt::While { .. });
    for expr in stmt.exprs_mut() {
        rewrite_expr(expr, locals, truth);
    }
    for body in stmt.bodies_mut() {
        for inner in body.iter_mut() {
            rewrite_stmt(inner, locals);
        }
    }
}

/// Rewrites every use of a qualifying local in `expr`; `truth` when only
/// `expr`'s truth value is used (an `if`/`while` test, a `not` operand,
/// a conditional expression's test).
fn rewrite_expr(expr: &mut MirExpr, locals: &HashSet<String>, truth: bool) {
    if let Some(native) = native_use(expr, locals, truth) {
        *expr = native;
        return;
    }
    if let MirExpr::Name { name, .. } = expr
        && locals.contains(name.as_str())
    {
        // Any other read: the object the local stands for.
        let name = name.clone();
        *expr = MirExpr::IfExp {
            test: Box::new(rt_call(IS_NONE, vec![native_read(&name)], Ty::Bool)),
            body: Box::new(MirExpr::ObjectBox(Box::new(MirExpr::NoneLiteral))),
            orelse: Box::new(MirExpr::ObjectBox(Box::new(native_read(&name)))),
            ty: Ty::Object,
        };
        return;
    }
    match expr {
        MirExpr::Not(inner) => rewrite_expr(inner, locals, true),
        MirExpr::IfExp { test, body, orelse, .. } => {
            rewrite_expr(test, locals, true);
            rewrite_expr(body, locals, false);
            rewrite_expr(orelse, locals, false);
        }
        MirExpr::BoolOp {
            left, right, ty, ..
        } if truth => {
            // Both operands only tested: rewritten in truth position when
            // both become native `bool`s, so the operation stays one of
            // `bool`s.
            let mut l = (**left).clone();
            let mut r = (**right).clone();
            rewrite_expr(&mut l, locals, true);
            rewrite_expr(&mut r, locals, true);
            if l.ty() == Ty::Bool && r.ty() == Ty::Bool {
                **left = l;
                **right = r;
                *ty = Ty::Bool;
            } else {
                rewrite_expr(left, locals, false);
                rewrite_expr(right, locals, false);
            }
        }
        _ => {
            for child in expr.children_mut() {
                rewrite_expr(child, locals, false);
            }
        }
    }
}

/// The native form of a comparison, membership or `None` test of a
/// qualifying local, or `None`.
fn native_use(expr: &MirExpr, locals: &HashSet<String>, truth: bool) -> Option<MirExpr> {
    let local = |e: &MirExpr| match e {
        MirExpr::Name { name, .. } if locals.contains(name.as_str()) => Some(name.clone()),
        _ => None,
    };
    let is_none = |e: &MirExpr| match e {
        MirExpr::NoneLiteral => true,
        MirExpr::ObjectBox(inner) => matches!(**inner, MirExpr::NoneLiteral),
        _ => false,
    };
    match expr {
        MirExpr::ObjCompare { op, left, right } => {
            let (name, other) = match (local(left), local(right)) {
                (Some(name), _) => (name, right.as_ref()),
                (None, Some(name)) => (name, left.as_ref()),
                (None, None) => return None,
            };
            match op {
                CmpOpKind::Is | CmpOpKind::IsNot if is_none(other) => Some(negated(
                    rt_call(IS_NONE, vec![native_read(&name)], Ty::Bool),
                    *op == CmpOpKind::IsNot,
                )),
                // `==` answers an object; only its truth is native.
                CmpOpKind::Eq | CmpOpKind::NotEq if truth => match other {
                    MirExpr::StringLiteral(_) => Some(negated(
                        rt_call(EQ, vec![native_read(&name), other.clone()], Ty::Bool),
                        *op == CmpOpKind::NotEq,
                    )),
                    _ => None,
                },
                _ => None,
            }
        }
        MirExpr::ObjContains {
            negate,
            item,
            container,
        } => {
            let name = local(item)?;
            let MirExpr::ObjectBox(set) = container.as_ref() else {
                return None;
            };
            let is_text_set = matches!(set.ty(), Ty::FrozenSet(element) if *element == Ty::Str);
            (is_text_set && matches!(**set, MirExpr::Name { .. })).then(|| {
                negated(
                    rt_call(IN, vec![native_read(&name), (**set).clone()], Ty::Bool),
                    *negate,
                )
            })
        }
        _ => None,
    }
}
