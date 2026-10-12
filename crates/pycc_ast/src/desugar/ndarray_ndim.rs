//! Writes the number of dimensions into every typed numpy array annotation
//! that leaves it out (`pycc_hir::ndarray`).
//!
//! numpy's own spelling, `NDArray[np.float64]`, names the dtype but not the
//! number of dimensions, while pycc compiles `a[i]` of a 1-D array and
//! `a[i, j]` of a 2-D one to different code. The dimension count is read off
//! how the scope uses the annotated name:
//!
//! * `a[i, j]` -- a tuple index of `k` elements -- means `k` dimensions;
//! * `a[i][j]` means 2;
//! * `n, m = a.shape` means 2, and `a.shape[1]` at least 2;
//! * an initializer `np.zeros((n, m))` (`ones`, `empty`, `full`, a
//!   `reshape(n, m)`) means the length of its shape tuple, and
//!   `np.array([[...]])` the nesting depth of its list display;
//! * a name passed as the `k`-th argument of a function of the same module
//!   whose `k`-th parameter has a known count takes that count.
//!
//! The largest piece of evidence wins; a name with none is one-dimensional.
//! The count is written as a second argument, `NDArray[np.float64, 2]`,
//! which is the only form `pycc_hir`'s annotation lowering reads it from. A
//! return annotation takes the count of the names and initializers its
//! `return` statements name. An attribute declared in a class body
//! (`grid: NDArray[np.float64]`) is keyed `self.grid` and takes the
//! evidence of every method of the module.
//!
//! An annotation that already carries a count (`NDArray[np.float64, 2]`,
//! `np.ndarray[tuple[int, int], np.dtype[np.float64]]`) is left alone.

use super::build;
use ruff_python_ast::visitor::{self, Visitor};
use ruff_python_ast::{Expr, Number, Stmt, StmtFunctionDef};
use std::collections::HashMap;

/// Evidence: key (`a`, `self.grid`) -> the largest dimension count seen.
type Evidence = HashMap<String, u8>;

/// The key an array-valued expression is tracked under: a name, or a
/// dotted attribute path over a name (`self.grid`).
fn key_of(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Name(name) => Some(name.id.to_string()),
        Expr::Attribute(attr) => key_of(&attr.value).map(|base| format!("{base}.{}", attr.attr)),
        _ => None,
    }
}

fn record(evidence: &mut Evidence, key: String, ndim: u8) {
    let slot = evidence.entry(key).or_insert(0);
    *slot = (*slot).max(ndim);
}

/// Whether `expr` is an unresolved typed-array annotation: `NDArray[D]` /
/// `ndarray[D]` (bare or attribute base) whose single argument is a numpy
/// dtype attribute.
fn is_open_annotation(expr: &Expr) -> bool {
    let Expr::Subscript(sub) = expr else {
        return false;
    };
    let base_ok = match sub.value.as_ref() {
        Expr::Name(name) => matches!(name.id.as_str(), "NDArray" | "ndarray"),
        Expr::Attribute(attr) => matches!(attr.attr.as_str(), "NDArray" | "ndarray"),
        _ => false,
    };
    base_ok && is_dtype(&sub.slice)
}

/// `np.float64`, `numpy.int64`, `np.bool_`, `np.dtype[np.float64]`.
fn is_dtype(expr: &Expr) -> bool {
    match expr {
        Expr::Attribute(attr) => matches!(
            attr.attr.as_str(),
            "float64" | "double" | "float_" | "int64" | "int_" | "intp" | "bool_" | "bool"
        ),
        Expr::Subscript(sub) => {
            matches!(sub.value.as_ref(), Expr::Attribute(a) if a.attr.as_str() == "dtype")
                && is_dtype(&sub.slice)
        }
        _ => false,
    }
}

/// Rewrites `NDArray[D]` into `NDArray[D, ndim]`.
fn close_annotation(expr: &mut Expr, ndim: u8) {
    let Expr::Subscript(sub) = expr else {
        return;
    };
    let range = sub.range;
    let dtype = std::mem::replace(sub.slice.as_mut(), build::int(0, range));
    *sub.slice = build::tuple(
        vec![dtype, build::int(u64::from(ndim), range)],
        ruff_python_ast::ExprContext::Load,
        range,
    );
}

/// The nesting depth of a list display (`[[1, 2], [3, 4]]` is 2).
fn list_depth(expr: &Expr) -> u8 {
    match expr {
        Expr::List(list) => 1 + list.elts.first().map_or(0, list_depth),
        _ => 0,
    }
}

/// The dimension count an array-producing expression has, when it can be
/// read off the syntax: a numpy creation call or a `reshape`.
fn produced_ndim(expr: &Expr, known: &Evidence) -> Option<u8> {
    let Expr::Call(call) = expr else {
        return key_of(expr).and_then(|k| known.get(&k).copied());
    };
    let Expr::Attribute(func) = call.func.as_ref() else {
        return None;
    };
    let args = &call.arguments.args;
    let shape = |arg: Option<&Expr>| -> Option<u8> {
        match arg? {
            Expr::Tuple(t) => u8::try_from(t.elts.len()).ok(),
            Expr::List(l) => u8::try_from(l.elts.len()).ok(),
            _ => Some(1),
        }
    };
    let shape_kw = || {
        call.arguments
            .keywords
            .iter()
            .find(|k| k.arg.as_ref().is_some_and(|a| a.as_str() == "shape"))
            .map(|k| &k.value)
    };
    match func.attr.as_str() {
        "zeros" | "ones" | "empty" | "full" => shape(args.first().or_else(shape_kw)),
        "arange" | "linspace" => Some(1),
        "array" | "asarray" => args.first().map(list_depth).filter(|d| *d > 0),
        "zeros_like" | "ones_like" | "empty_like" | "copy" => {
            args.first().and_then(|a| produced_ndim(a, known))
        }
        "reshape" if args.len() == 1 => shape(args.first()),
        "reshape" => u8::try_from(args.len()).ok(),
        _ => None,
    }
}

/// Collects usage evidence over a body (nested bodies included unless
/// `top_level_only`).
struct Usage<'k> {
    evidence: Evidence,
    /// `(row, array)` for `row = array[i]` and `for row in array`: once the
    /// walk is done, a row that is itself indexed adds a dimension to its
    /// array.
    rows: Vec<(String, String)>,
    /// Skip the bodies of nested `def`s and `class`es (the module scope's
    /// own statements).
    top_level_only: bool,
    /// Resolved parameter counts of the module's functions, for the
    /// argument rule.
    signatures: &'k HashMap<String, Vec<Option<u8>>>,
}

impl<'a> Visitor<'a> for Usage<'_> {
    fn visit_expr(&mut self, expr: &'a Expr) {
        match expr {
            Expr::Subscript(sub) => {
                if let Some(key) = key_of(&sub.value) {
                    let n = match sub.slice.as_ref() {
                        Expr::Tuple(t) => u8::try_from(t.elts.len()).unwrap_or(0),
                        _ => 1,
                    };
                    record(&mut self.evidence, key, n);
                }
                if let Expr::Subscript(inner) = sub.value.as_ref()
                    && let Some(key) = key_of(&inner.value)
                    && !matches!(inner.slice.as_ref(), Expr::Tuple(_) | Expr::Slice(_))
                {
                    record(&mut self.evidence, key, 2);
                }
                if let Expr::Attribute(attr) = sub.value.as_ref()
                    && attr.attr.as_str() == "shape"
                    && let Some(key) = key_of(&attr.value)
                    && let Expr::NumberLiteral(n) = sub.slice.as_ref()
                    && let Number::Int(i) = &n.value
                    && let Some(i) = i.as_i64()
                    && (0..2).contains(&i)
                {
                    record(&mut self.evidence, key, u8::try_from(i + 1).unwrap_or(1));
                }
            }
            Expr::Call(call) => {
                if let Expr::Name(func) = call.func.as_ref()
                    && let Some(params) = self.signatures.get(func.id.as_str())
                {
                    for (arg, ndim) in call.arguments.args.iter().zip(params) {
                        if let (Some(key), Some(ndim)) = (key_of(arg), ndim) {
                            record(&mut self.evidence, key, *ndim);
                        }
                    }
                }
            }
            _ => {}
        }
        visitor::walk_expr(self, expr);
    }

    fn visit_stmt(&mut self, stmt: &'a Stmt) {
        match stmt {
            Stmt::FunctionDef(_) | Stmt::ClassDef(_) if self.top_level_only => return,
            Stmt::For(for_stmt) => {
                if let (Some(row), Some(array)) = (key_of(&for_stmt.target), key_of(&for_stmt.iter)) {
                    self.rows.push((row, array));
                }
            }
            Stmt::Assign(assign) => {
                if let [target] = assign.targets.as_slice()
                    && let Some(row) = key_of(target)
                    && let Expr::Subscript(sub) = assign.value.as_ref()
                    && !matches!(sub.slice.as_ref(), Expr::Tuple(_) | Expr::Slice(_))
                    && let Some(array) = key_of(&sub.value)
                {
                    self.rows.push((row, array));
                }
                if let [Expr::Tuple(target)] = assign.targets.as_slice()
                    && let Expr::Attribute(attr) = assign.value.as_ref()
                    && attr.attr.as_str() == "shape"
                    && let Some(key) = key_of(&attr.value)
                {
                    record(
                        &mut self.evidence,
                        key,
                        u8::try_from(target.elts.len()).unwrap_or(0),
                    );
                }
                if let [target] = assign.targets.as_slice()
                    && let Some(key) = key_of(target)
                    && let Some(n) = produced_ndim(&assign.value, &self.evidence)
                {
                    record(&mut self.evidence, key, n);
                }
            }
            Stmt::AnnAssign(ann) => {
                if let Some(value) = &ann.value
                    && let Some(key) = key_of(&ann.target)
                    && let Some(n) = produced_ndim(value, &self.evidence)
                {
                    record(&mut self.evidence, key, n);
                }
            }
            _ => {}
        }
        visitor::walk_stmt(self, stmt);
    }
}

fn collect(body: &[Stmt], signatures: &HashMap<String, Vec<Option<u8>>>) -> Evidence {
    collect_scope(body, signatures, false)
}

fn collect_scope(
    body: &[Stmt],
    signatures: &HashMap<String, Vec<Option<u8>>>,
    top_level_only: bool,
) -> Evidence {
    let mut usage = Usage {
        evidence: Evidence::new(),
        rows: Vec::new(),
        top_level_only,
        signatures,
    };
    for stmt in body {
        usage.visit_stmt(stmt);
    }
    let Usage {
        mut evidence, rows, ..
    } = usage;
    // A row indexed `k` times is one more dimension of its array; rows of
    // rows chain, so repeat to a fixpoint (bounded by the dimension cap).
    for _ in 0..3 {
        for (row, array) in &rows {
            if let Some(n) = evidence.get(row).copied().filter(|n| *n > 0) {
                record(&mut evidence, array.clone(), n.saturating_add(1));
            }
        }
    }
    evidence
}

/// The module scope's evidence: its own statements, plus every function
/// (and method) body that reads a name without binding it -- a global it
/// uses. A dotted key (`self.cells`) takes every body's evidence.
fn module_evidence(body: &[Stmt], signatures: &HashMap<String, Vec<Option<u8>>>) -> Evidence {
    let mut evidence = collect_scope(body, signatures, true);
    let mut functions: Vec<&StmtFunctionDef> = Vec::new();
    for stmt in body {
        match stmt {
            Stmt::FunctionDef(def) => functions.push(def),
            Stmt::ClassDef(class) => functions.extend(class.body.iter().filter_map(|s| match s {
                Stmt::FunctionDef(def) => Some(def),
                _ => None,
            })),
            _ => {}
        }
    }
    for def in functions {
        let mut bound = super::stored_names(&def.body);
        let params = &def.parameters;
        for p in params.posonlyargs.iter().chain(&params.args).chain(&params.kwonlyargs) {
            bound.insert(p.parameter.name.to_string());
        }
        for (key, n) in collect(&def.body, signatures) {
            let root = key.split('.').next().unwrap_or("");
            if key.contains('.') || !bound.contains(root) {
                record(&mut evidence, key, n);
            }
        }
    }
    evidence
}

/// The resolved count for `key`: local evidence, else module evidence for a
/// dotted key or a module-level name, else 1.
fn resolve(key: &str, local: &Evidence, module: &Evidence, use_module: bool) -> u8 {
    let found = local.get(key).copied().filter(|n| *n > 0).or_else(|| {
        (use_module || key.contains('.'))
            .then(|| module.get(key).copied())
            .flatten()
    });
    found.filter(|n| *n > 0).unwrap_or(1)
}

/// Closes the open annotations of one function: its parameters, the
/// annotated assignments of its body (not nested functions) and its return.
fn close_function(
    def: &mut StmtFunctionDef,
    module: &Evidence,
    signatures: &HashMap<String, Vec<Option<u8>>>,
) {
    let mut local = collect(&def.body, signatures);
    let params = &mut *def.parameters;
    for param in params
        .posonlyargs
        .iter_mut()
        .chain(params.args.iter_mut())
        .chain(params.kwonlyargs.iter_mut())
    {
        if let Some(ann) = param.parameter.annotation.as_deref_mut()
            && is_open_annotation(ann)
        {
            let key = param.parameter.name.to_string();
            let ndim = resolve(&key, &local, module, false);
            record(&mut local, key, ndim);
            close_annotation(ann, ndim);
        }
    }
    close_body(&mut def.body, &mut local, module, signatures, false);
    if let Some(ret) = def.returns.as_deref_mut()
        && is_open_annotation(ret)
    {
        let mut returns = Returns {
            local: &local,
            ndim: 0,
        };
        for stmt in &def.body {
            returns.visit_stmt(stmt);
        }
        close_annotation(ret, returns.ndim.max(1));
    }
}

/// The largest count a function's `return` statements produce.
struct Returns<'l> {
    local: &'l Evidence,
    ndim: u8,
}

impl<'a> Visitor<'a> for Returns<'_> {
    fn visit_stmt(&mut self, stmt: &'a Stmt) {
        match stmt {
            Stmt::FunctionDef(_) | Stmt::ClassDef(_) => {}
            Stmt::Return(ret) => {
                if let Some(value) = &ret.value
                    && let Some(n) = produced_ndim(value, self.local)
                {
                    self.ndim = self.ndim.max(n);
                }
            }
            _ => visitor::walk_stmt(self, stmt),
        }
    }
}

/// Closes the annotated assignments of a body (descending into compound
/// statements, not into nested functions or classes).
fn close_body(
    body: &mut [Stmt],
    local: &mut Evidence,
    module: &Evidence,
    signatures: &HashMap<String, Vec<Option<u8>>>,
    module_level: bool,
) {
    for stmt in body {
        match stmt {
            Stmt::AnnAssign(ann) if is_open_annotation(&ann.annotation) => {
                let Some(key) = key_of(&ann.target) else {
                    continue;
                };
                let ndim = resolve(&key, local, module, module_level);
                record(local, key, ndim);
                close_annotation(&mut ann.annotation, ndim);
            }
            Stmt::If(s) => {
                close_body(&mut s.body, local, module, signatures, module_level);
                for clause in &mut s.elif_else_clauses {
                    close_body(&mut clause.body, local, module, signatures, module_level);
                }
            }
            Stmt::For(s) => {
                close_body(&mut s.body, local, module, signatures, module_level);
                close_body(&mut s.orelse, local, module, signatures, module_level);
            }
            Stmt::While(s) => {
                close_body(&mut s.body, local, module, signatures, module_level);
                close_body(&mut s.orelse, local, module, signatures, module_level);
            }
            Stmt::With(s) => close_body(&mut s.body, local, module, signatures, module_level),
            Stmt::Try(s) => {
                close_body(&mut s.body, local, module, signatures, module_level);
                for handler in &mut s.handlers {
                    let ruff_python_ast::ExceptHandler::ExceptHandler(h) = handler;
                    close_body(&mut h.body, local, module, signatures, module_level);
                }
                close_body(&mut s.orelse, local, module, signatures, module_level);
                close_body(&mut s.finalbody, local, module, signatures, module_level);
            }
            Stmt::FunctionDef(def) => close_function(def, module, signatures),
            Stmt::ClassDef(class) => {
                for item in class.body.iter_mut() {
                    match item {
                        Stmt::FunctionDef(def) => close_function(def, module, signatures),
                        Stmt::AnnAssign(ann) if is_open_annotation(&ann.annotation) => {
                            let Some(name) = key_of(&ann.target) else {
                                continue;
                            };
                            let ndim =
                                resolve(&format!("self.{name}"), &Evidence::new(), module, true);
                            close_annotation(&mut ann.annotation, ndim);
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
}

/// The parameter counts of the module's functions whose annotations are
/// already closed or resolvable from their own bodies.
fn signatures_of(body: &[Stmt], module: &Evidence) -> HashMap<String, Vec<Option<u8>>> {
    let empty = HashMap::new();
    let mut out = HashMap::new();
    for stmt in body {
        let Stmt::FunctionDef(def) = stmt else {
            continue;
        };
        let local = collect(&def.body, &empty);
        let params = def
            .parameters
            .posonlyargs
            .iter()
            .chain(def.parameters.args.iter())
            .map(|p| {
                let ann = p.parameter.annotation.as_deref()?;
                if is_open_annotation(ann) {
                    let n = resolve(p.parameter.name.as_str(), &local, module, false);
                    Some(n)
                } else if let Expr::Subscript(sub) = ann
                    && let Expr::Tuple(t) = sub.slice.as_ref()
                    && let [_, Expr::NumberLiteral(n)] = t.elts.as_slice()
                    && let Number::Int(i) = &n.value
                {
                    i.as_i64().and_then(|i| u8::try_from(i).ok())
                } else {
                    None
                }
            })
            .collect();
        out.insert(def.name.to_string(), params);
    }
    out
}

/// Closes every open typed-array annotation of the module.
pub(super) fn close_ndims(body: &mut [Stmt]) {
    let empty = HashMap::new();
    let module = module_evidence(body, &empty);
    let signatures = signatures_of(body, &module);
    let module = module_evidence(body, &signatures);
    let mut local = module.clone();
    close_body(body, &mut local, &module, &signatures, true);
}
