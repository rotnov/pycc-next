//! `itertools` calls compile natively: a call of `count`, `islice`,
//! `chain` (and `chain.from_iterable`), `accumulate`, `pairwise`,
//! `groupby`, `product`, `permutations`, `combinations` or
//! `combinations_with_replacement` -- through `import itertools [as it]` or
//! `from itertools import f [as g]`, the name bound nowhere else -- is
//! spelled as a `pycc_hir::builtins` name `__pycc_it_<function>`, typed by
//! its arguments and run by a native iterator (`pycc_rt::itertools`).
//!
//! What the result type depends on beyond the argument types is spelled in
//! the name and passed again as an argument: a tuple's arity
//! (`permutations(xs, 2)` -> `__pycc_it_permutations_2(xs, 2)`), `product`'s
//! `repeat`, `accumulate`'s function (`operator.mul`, `max`, `min`, or a
//! two-parameter lambda whose body is one of those operations). A
//! `groupby` key function becomes the generator of `(key, element)` pairs
//! the native `groupby` groups, a lambda inlined. A `range(...)` argument
//! becomes a native range iterator (lazy, as CPython iterates it), a tuple
//! display a list display. A call the rewrite does not understand
//! is left alone, and an import left unread is dropped.

use super::aliases::rebound_names;
use super::build::{call_name, int, load, store};
use ruff_python_ast::visitor::transformer::{self, Transformer};
use ruff_python_ast::visitor::{self, Visitor};
use ruff_python_ast::{Comprehension, Expr, ExprCall, ExprContext, Number, Operator, Stmt};
use ruff_text_size::TextRange;
use std::collections::{HashMap, HashSet};

/// The `itertools` functions compiled natively.
const FUNCTIONS: &[&str] = &[
    "count",
    "islice",
    "chain",
    "accumulate",
    "pairwise",
    "groupby",
    "product",
    "permutations",
    "combinations",
    "combinations_with_replacement",
];

/// `operator` functions `accumulate` takes, by the runtime's operation name.
fn operator_op(function: &str) -> Option<&'static str> {
    Some(match function {
        "add" => "add",
        "mul" => "mul",
        "sub" => "sub",
        "and_" => "and",
        "or_" => "or",
        "xor" => "xor",
        _ => return None,
    })
}

/// The runtime code of an `accumulate` operation (`pycc_rt::itertools`).
fn op_code(op: &str) -> u64 {
    match op {
        "add" => 0,
        "mul" => 1,
        "sub" => 2,
        "max" => 3,
        "min" => 4,
        "and" => 5,
        "or" => 6,
        _ => 7,
    }
}

struct Rewrite {
    /// Local names bound to the `itertools` module.
    modules: HashSet<String>,
    /// Local name -> `itertools` function.
    functions: HashMap<String, String>,
    /// Local names bound to the `operator` module.
    operator_modules: HashSet<String>,
    /// Local name -> `operator` function.
    operator_functions: HashMap<String, String>,
    /// Names rebound somewhere in the module (so not the builtins).
    rebound: HashMap<String, usize>,
}

impl Rewrite {
    fn builtin(&self, name: &str) -> bool {
        !self.rebound.contains_key(name)
    }

    /// The `itertools` function `func` names (`chain.from_iterable` for the
    /// method), if any.
    fn target(&self, func: &Expr) -> Option<String> {
        match func {
            Expr::Name(name) => self.functions.get(name.id.as_str()).cloned(),
            Expr::Attribute(attr) => match attr.value.as_ref() {
                Expr::Name(module) if self.modules.contains(module.id.as_str()) => {
                    let function = attr.attr.as_str();
                    FUNCTIONS.contains(&function).then(|| function.to_string())
                }
                inner @ (Expr::Name(_) | Expr::Attribute(_))
                    if attr.attr.as_str() == "from_iterable"
                        && self.target(inner).as_deref() == Some("chain") =>
                {
                    Some("chain.from_iterable".to_string())
                }
                _ => None,
            },
            _ => None,
        }
    }

    /// The `accumulate` operation `func` denotes.
    fn accumulate_op(&self, func: &Expr) -> Option<&'static str> {
        match func {
            Expr::Name(name) if matches!(name.id.as_str(), "max" | "min") => {
                let builtin = name.id.as_str();
                self.builtin(builtin)
                    .then_some(if builtin == "max" { "max" } else { "min" })
            }
            Expr::Name(name) => operator_op(self.operator_functions.get(name.id.as_str())?),
            Expr::Attribute(attr) => match attr.value.as_ref() {
                Expr::Name(module) if self.operator_modules.contains(module.id.as_str()) => {
                    operator_op(attr.attr.as_str())
                }
                _ => None,
            },
            Expr::Lambda(lambda) => {
                let params = lambda.parameters.as_deref()?;
                let [a, b] = params.args.as_slice() else {
                    return None;
                };
                if a.default.is_some()
                    || b.default.is_some()
                    || !params.posonlyargs.is_empty()
                    || params.vararg.is_some()
                    || params.kwarg.is_some()
                    || !params.kwonlyargs.is_empty()
                {
                    return None;
                }
                let (a, b) = (a.name().as_str(), b.name().as_str());
                let is = |expr: &Expr, want: &str| matches!(expr, Expr::Name(n) if n.id.as_str() == want);
                match lambda.body.as_ref() {
                    Expr::BinOp(binop) if is(&binop.left, a) && is(&binop.right, b) => {
                        Some(match binop.op {
                            Operator::Add => "add",
                            Operator::Mult => "mul",
                            Operator::Sub => "sub",
                            Operator::BitAnd => "and",
                            Operator::BitOr => "or",
                            Operator::BitXor => "xor",
                            _ => return None,
                        })
                    }
                    Expr::Call(call)
                        if call.arguments.keywords.is_empty()
                            && matches!(call.arguments.args.as_ref(), [x, y] if is(x, a) && is(y, b)) =>
                    {
                        match call.func.as_ref() {
                            Expr::Name(f) if f.id.as_str() == "max" && self.builtin("max") => {
                                Some("max")
                            }
                            Expr::Name(f) if f.id.as_str() == "min" && self.builtin("min") => {
                                Some("min")
                            }
                            _ => None,
                        }
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// An iterable argument as the native iterators take it.
    fn iterable(&self, expr: &Expr) -> Expr {
        match expr {
            Expr::Call(call)
                if matches!(call.func.as_ref(), Expr::Name(f) if f.id.as_str() == "range")
                    && self.builtin("range")
                    && call.arguments.keywords.is_empty()
                    && !call
                        .arguments
                        .args
                        .iter()
                        .any(|a| matches!(a, Expr::Starred(_))) =>
            {
                let range = call.range;
                let (start, stop, step) = match call.arguments.args.as_ref() {
                    [stop] => (int(0, range), stop.clone(), int(1, range)),
                    [start, stop] => (start.clone(), stop.clone(), int(1, range)),
                    [start, stop, step] => (start.clone(), stop.clone(), step.clone()),
                    _ => return expr.clone(),
                };
                call_name("__pycc_it_range", vec![start, stop, step], range)
            }
            Expr::Tuple(tuple) => Expr::List(ruff_python_ast::ExprList {
                node_index: Default::default(),
                range: tuple.range,
                elts: tuple.elts.clone(),
                ctx: ExprContext::Load,
            }),
            other => other.clone(),
        }
    }

    /// The rewrite of `call` when it calls a native `itertools` function.
    fn rewrite(&self, call: &ExprCall) -> Option<Expr> {
        let function = self.target(&call.func)?;
        let range = call.range;
        let args = &call.arguments.args;
        if args.iter().any(|a| matches!(a, Expr::Starred(_))) {
            return None;
        }
        let mut keywords: HashMap<&str, &Expr> = HashMap::new();
        for keyword in &call.arguments.keywords {
            keywords.insert(keyword.arg.as_ref()?.as_str(), &keyword.value);
        }
        let only = |names: &[&str]| keywords.keys().all(|k| names.contains(k));
        let it = |name: &str, args: Vec<Expr>| {
            Some(call_name(&format!("__pycc_it_{name}"), args, range))
        };
        match function.as_str() {
            "count" => {
                if args.len() > 2 || !only(&["start", "step"]) {
                    return None;
                }
                let start = arg(args, 0, &keywords, "start").unwrap_or_else(|| int(0, range));
                let step = arg(args, 1, &keywords, "step").unwrap_or_else(|| int(1, range));
                it("count", vec![start, step])
            }
            "islice" => {
                if !keywords.is_empty() {
                    return None;
                }
                let (source, start, stop, step) = match args.as_ref() {
                    [source, stop] => (source, None, stop, None),
                    [source, start, stop] => (source, Some(start), stop, None),
                    [source, start, stop, step] => (source, Some(start), stop, Some(step)),
                    _ => return None,
                };
                let or = |bound: Option<&Expr>, default: u64| match bound {
                    Some(Expr::NoneLiteral(_)) | None => int(default, range),
                    Some(bound) => bound.clone(),
                };
                let (stop, has_stop) = match stop {
                    Expr::NoneLiteral(_) => (int(0, range), 0),
                    stop => (stop.clone(), 1),
                };
                it(
                    "islice",
                    vec![
                        self.iterable(source),
                        or(start, 0),
                        stop,
                        or(step, 1),
                        int(has_stop, range),
                    ],
                )
            }
            "chain" => {
                if !keywords.is_empty() || args.is_empty() {
                    return None;
                }
                let mut iter = args.iter().map(|a| self.iterable(a));
                let first = iter.next()?;
                let Some(second) = iter.next() else {
                    return it("chain1", vec![first]);
                };
                let mut chained = call_name("__pycc_it_chain2", vec![first, second], range);
                for next in iter {
                    chained = call_name("__pycc_it_chain2", vec![chained, next], range);
                }
                Some(chained)
            }
            "chain.from_iterable" => match (args.as_ref(), keywords.is_empty()) {
                ([source], true) => it("chain_from", vec![self.iterable(source)]),
                _ => None,
            },
            "accumulate" => {
                if args.len() > 2 || args.is_empty() || !only(&["func", "initial"]) {
                    return None;
                }
                let op = match arg(args, 1, &keywords, "func") {
                    None | Some(Expr::NoneLiteral(_)) => "add",
                    Some(func) => self.accumulate_op(&func)?,
                };
                let code = int(op_code(op), range);
                let source = self.iterable(&args[0]);
                match keywords.get("initial") {
                    Some(Expr::NoneLiteral(_)) | None => {
                        it(&format!("accumulate_{op}"), vec![source, code])
                    }
                    Some(initial) => it(
                        &format!("accumulate_{op}_initial"),
                        vec![source, code, (*initial).clone()],
                    ),
                }
            }
            "pairwise" => match (args.as_ref(), keywords.is_empty()) {
                ([source], true) => it("pairwise", vec![self.iterable(source)]),
                _ => None,
            },
            "groupby" => {
                if args.is_empty() || args.len() > 2 || !only(&["key"]) {
                    return None;
                }
                let source = self.iterable(&args[0]);
                match arg(args, 1, &keywords, "key") {
                    None | Some(Expr::NoneLiteral(_)) => it("groupby", vec![source, int(0, range)]),
                    Some(key) => {
                        let (var, key_of) = keyed(&key, range)?;
                        let pair = super::build::tuple(
                            vec![key_of, load(&var, range)],
                            ExprContext::Load,
                            range,
                        );
                        let pairs = genexp(pair, &var, source, range);
                        it("groupby_keyed", vec![pairs, int(1, range)])
                    }
                }
            }
            "product" => {
                if args.is_empty() || args.len() > 4 || !only(&["repeat"]) {
                    return None;
                }
                let repeat = match keywords.get("repeat") {
                    None => 1,
                    Some(repeat) => int_literal(repeat)?,
                };
                let mut iterables: Vec<Expr> = args.iter().map(|a| self.iterable(a)).collect();
                iterables.push(int(repeat, range));
                it(&format!("product_{repeat}"), iterables)
            }
            "permutations" | "combinations" | "combinations_with_replacement" => {
                if args.is_empty() || args.len() > 2 || !only(&["r"]) {
                    return None;
                }
                let family = match function.as_str() {
                    "combinations_with_replacement" => "cwr",
                    other => other,
                };
                let r = match arg(args, 1, &keywords, "r") {
                    Some(Expr::NoneLiteral(_)) | None if family == "permutations" => {
                        display_len(&args[0])
                    }
                    Some(r) => int_literal(&r),
                    None => None,
                };
                let source = self.iterable(&args[0]);
                match r {
                    Some(r) => it(&format!("{family}_{r}"), vec![source, int(r, range)]),
                    // A length unknown at compile time: the builtin table
                    // refuses it with a diagnostic naming `r`.
                    None => it(&format!("{family}_r"), vec![source]),
                }
            }
            _ => None,
        }
    }
}

/// The variable of the generator expressions the rewrite builds.
const VAR: &str = "_pycc_itv";

/// Positional argument `index`, or keyword `name`.
fn arg(args: &[Expr], index: usize, keywords: &HashMap<&str, &Expr>, name: &str) -> Option<Expr> {
    args.get(index)
        .or_else(|| keywords.get(name).copied())
        .cloned()
}

/// A non-negative `int` literal's value.
fn int_literal(expr: &Expr) -> Option<u64> {
    match expr {
        Expr::NumberLiteral(number) => match &number.value {
            Number::Int(value) => value.as_u64(),
            _ => None,
        },
        _ => None,
    }
}

/// The length of a list, tuple or `str` display or of a literal `range`
/// (`permutations`' default `r`).
fn display_len(expr: &Expr) -> Option<u64> {
    match expr {
        Expr::List(list) if !list.elts.iter().any(|e| matches!(e, Expr::Starred(_))) => {
            Some(list.elts.len() as u64)
        }
        Expr::Tuple(tuple) if !tuple.elts.iter().any(|e| matches!(e, Expr::Starred(_))) => {
            Some(tuple.elts.len() as u64)
        }
        Expr::StringLiteral(text) => Some(text.value.to_str().chars().count() as u64),
        // `range(n)` and `range(a, b)` over int literals.
        Expr::Call(call)
            if matches!(call.func.as_ref(), Expr::Name(f) if f.id.as_str() == "range")
                && call.arguments.keywords.is_empty() =>
        {
            match call.arguments.args.as_ref() {
                [stop] => int_literal(stop),
                [start, stop] => Some(int_literal(stop)?.saturating_sub(int_literal(start)?)),
                _ => None,
            }
        }
        _ => None,
    }
}

/// A `groupby` key function applied to a fresh variable: a one-parameter
/// lambda inlined, a name or attribute called.
fn keyed(key: &Expr, range: TextRange) -> Option<(String, Expr)> {
    match key {
        Expr::Lambda(lambda) => {
            let params = lambda.parameters.as_deref()?;
            let [param] = params.args.as_slice() else {
                return None;
            };
            if param.default.is_some()
                || !params.posonlyargs.is_empty()
                || params.vararg.is_some()
                || params.kwarg.is_some()
                || !params.kwonlyargs.is_empty()
            {
                return None;
            }
            Some((param.name().to_string(), (*lambda.body).clone()))
        }
        Expr::Name(_) | Expr::Attribute(_) => Some((
            VAR.to_string(),
            super::build::call(key.clone(), vec![load(VAR, range)], range),
        )),
        _ => None,
    }
}

/// `(elt for var in iter)`.
fn genexp(elt: Expr, var: &str, iter: Expr, range: TextRange) -> Expr {
    Expr::Generator(ruff_python_ast::ExprGenerator {
        node_index: Default::default(),
        range,
        elt: Box::new(elt),
        generators: vec![Comprehension {
            range,
            node_index: Default::default(),
            target: store(var, range),
            iter,
            ifs: Vec::new(),
            is_async: false,
        }],
        parenthesized: true,
    })
}

impl Transformer for Rewrite {
    fn visit_expr(&self, expr: &mut Expr) {
        transformer::walk_expr(self, expr);
        if let Expr::Call(call) = expr
            && let Some(rewritten) = self.rewrite(call)
        {
            *expr = rewritten;
        }
    }
}

fn read_names(body: &[Stmt]) -> HashSet<String> {
    struct Collect(HashSet<String>);
    impl<'a> Visitor<'a> for Collect {
        fn visit_expr(&mut self, expr: &'a Expr) {
            if let Expr::Name(name) = expr
                && name.ctx == ExprContext::Load
            {
                self.0.insert(name.id.to_string());
            }
            visitor::walk_expr(self, expr);
        }
    }
    let mut collect = Collect(HashSet::new());
    for stmt in body {
        collect.visit_stmt(stmt);
    }
    collect.0
}

/// Rewrites the native `itertools` calls of `body` (a module).
pub(crate) fn native_itertools_calls(body: &mut Vec<Stmt>) {
    let rebound = rebound_names(body);
    let mut rewrite = Rewrite {
        modules: HashSet::new(),
        functions: HashMap::new(),
        operator_modules: HashSet::new(),
        operator_functions: HashMap::new(),
        rebound: HashMap::new(),
    };
    let mut seen: HashMap<String, usize> = HashMap::new();
    for stmt in body.iter() {
        match stmt {
            Stmt::Import(import) => {
                for alias in &import.names {
                    let local = alias.asname.as_ref().unwrap_or(&alias.name).to_string();
                    *seen.entry(local.clone()).or_default() += 1;
                    match alias.name.as_str() {
                        "itertools" => {
                            rewrite.modules.insert(local);
                        }
                        "operator" => {
                            rewrite.operator_modules.insert(local);
                        }
                        _ => {}
                    }
                }
            }
            Stmt::ImportFrom(import) if import.level == 0 => {
                let module = import.module.as_ref().map(|m| m.as_str()).unwrap_or("");
                for alias in &import.names {
                    let local = alias.asname.as_ref().unwrap_or(&alias.name).to_string();
                    *seen.entry(local.clone()).or_default() += 1;
                    let name = alias.name.to_string();
                    match module {
                        "itertools" if FUNCTIONS.contains(&name.as_str()) => {
                            rewrite.functions.insert(local, name);
                        }
                        "operator" => {
                            rewrite.operator_functions.insert(local, name);
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    if rewrite.modules.is_empty() && rewrite.functions.is_empty() {
        return;
    }
    // A name bound twice (or by anything but its import) keeps CPython's.
    let unique = |name: &String| seen.get(name) == Some(&1) && !rebound.contains_key(name);
    rewrite.modules.retain(|name| unique(name));
    rewrite.functions.retain(|name, _| unique(name));
    rewrite.operator_modules.retain(|name| unique(name));
    rewrite.operator_functions.retain(|name, _| unique(name));
    rewrite.rebound = rebound;
    if rewrite.modules.is_empty() && rewrite.functions.is_empty() {
        return;
    }
    for stmt in body.iter_mut() {
        rewrite.visit_stmt(stmt);
    }
    let read = read_names(body);
    let droppable = |local: &str| {
        (rewrite.modules.contains(local)
            || rewrite.functions.contains_key(local)
            || rewrite.operator_modules.contains(local)
            || rewrite.operator_functions.contains_key(local))
            && !read.contains(local)
    };
    body.retain_mut(|stmt| match stmt {
        Stmt::Import(import) => {
            import.names.retain(|alias| {
                let local = alias.asname.as_ref().unwrap_or(&alias.name).to_string();
                !droppable(&local)
            });
            !import.names.is_empty()
        }
        Stmt::ImportFrom(import) => {
            import.names.retain(|alias| {
                let local = alias.asname.as_ref().unwrap_or(&alias.name).to_string();
                !droppable(&local)
            });
            !import.names.is_empty()
        }
        _ => true,
    });
}
