//! `collections.deque`, `collections.Counter` and
//! `collections.defaultdict` compile natively, as flavors of the list and
//! dict runtime objects (`pycc_rt::list::deque`, `pycc_rt::dict::flavor`):
//! through `import collections [as c]` or `from collections import deque
//! [as d]`, the name bound nowhere else,
//!
//! * an annotation `deque[T]` is `list[T]`, `Counter[K]` is `dict[K, int]`
//!   and `defaultdict[K, V]` is `dict[K, V]` -- the static type is the
//!   list's or dict's, the run-time object carries what differs (its
//!   `repr`, the deque's end operations and `maxlen`, a missing key's
//!   default);
//! * `deque(it, maxlen)` is `__pycc_deque(list(it), maxlen, has_maxlen)`
//!   (an empty `[]` for `deque()`, a list display passed as written), which
//!   marks the fresh list a deque;
//! * `Counter()` is `__pycc_counter({})`, `Counter(it)` is
//!   `__pycc_counter_of(it)`;
//! * `defaultdict(f)` is `__pycc_defaultdict({}, proto, name)` when `f` is a
//!   type (`int`, `list`, ...), a nested `defaultdict`/`Counter`/`deque` or a
//!   lambda returning a literal or one of those: a missing key gets a fresh
//!   copy of the value `proto`, which is what calling `f` gives. Any other
//!   function `f` is `__pycc_defaultdict_call({}, f, name)`, called by the
//!   runtime for each missing key.
//!
//! A call the rewrite does not understand is left alone, and an import left
//! unread is dropped.

use super::aliases::rebound_names;
use super::build::{call_name, int, load, string};
use ruff_python_ast::visitor::transformer::{self, Transformer};
use ruff_python_ast::visitor::{self, Visitor};
use ruff_python_ast::{Expr, ExprCall, ExprContext, Stmt};
use ruff_text_size::TextRange;
use std::collections::{HashMap, HashSet};

/// The `collections` names compiled natively.
const NAMES: &[&str] = &["deque"];

struct Rewrite {
    /// Local names bound to the `collections` module.
    modules: HashSet<String>,
    /// Local name -> `collections` name.
    names: HashMap<String, String>,
    rebound: HashMap<String, usize>,
}

fn empty_list(range: TextRange) -> Expr {
    Expr::List(ruff_python_ast::ExprList {
        node_index: Default::default(),
        range,
        elts: Vec::new(),
        ctx: ExprContext::Load,
    })
}

fn empty_dict(range: TextRange) -> Expr {
    Expr::Dict(ruff_python_ast::ExprDict {
        node_index: Default::default(),
        range,
        items: Vec::new(),
    })
}

fn subscript(base: &str, slice: Expr, range: TextRange) -> Expr {
    super::build::subscript(load(base, range), slice, ExprContext::Load, range)
}

/// Whether `expr` is a literal whose value a shallow copy reproduces: a
/// number, `str`, `bool` or `None`, or a list, tuple, set or dict display of
/// such literals.
fn copyable_literal(expr: &Expr) -> bool {
    match expr {
        Expr::NumberLiteral(_)
        | Expr::StringLiteral(_)
        | Expr::BooleanLiteral(_)
        | Expr::NoneLiteral(_) => true,
        Expr::UnaryOp(unary) => matches!(unary.operand.as_ref(), Expr::NumberLiteral(_)),
        Expr::List(list) => list.elts.iter().all(scalar_literal),
        Expr::Tuple(tuple) => tuple.elts.iter().all(copyable_literal),
        Expr::Set(set) => set.elts.iter().all(scalar_literal),
        Expr::Dict(dict) => dict.items.iter().all(|item| {
            item.key.as_ref().is_some_and(scalar_literal) && scalar_literal(&item.value)
        }),
        _ => false,
    }
}

/// An immutable literal: a number, `str`, `bool`, `None`, or a tuple of
/// those.
fn scalar_literal(expr: &Expr) -> bool {
    match expr {
        Expr::NumberLiteral(_)
        | Expr::StringLiteral(_)
        | Expr::BooleanLiteral(_)
        | Expr::NoneLiteral(_) => true,
        Expr::UnaryOp(unary) => matches!(unary.operand.as_ref(), Expr::NumberLiteral(_)),
        Expr::Tuple(tuple) => tuple.elts.iter().all(scalar_literal),
        _ => false,
    }
}

/// Whether `expr` is a rewritten native container constructor
/// (`__pycc_deque([], ...)`, `__pycc_counter({})`, `__pycc_defaultdict(...)`
/// of a prototype) whose fresh copy is a new empty container like it.
fn native_prototype(expr: &Expr) -> bool {
    let Expr::Call(call) = expr else {
        return false;
    };
    let Expr::Name(name) = call.func.as_ref() else {
        return false;
    };
    match (name.id.as_str(), call.arguments.args.first()) {
        ("__pycc_deque", Some(Expr::List(list))) => list.elts.iter().all(scalar_literal),
        ("__pycc_counter", Some(Expr::Dict(dict))) => dict.items.is_empty(),
        ("__pycc_defaultdict", Some(Expr::Dict(dict))) => dict.items.is_empty(),
        _ => false,
    }
}

impl Rewrite {
    /// The `collections` name `expr` refers to.
    fn target(&self, expr: &Expr) -> Option<&str> {
        match expr {
            Expr::Name(name) => self.names.get(name.id.as_str()).map(String::as_str),
            Expr::Attribute(attr) => match attr.value.as_ref() {
                Expr::Name(module) if self.modules.contains(module.id.as_str()) => {
                    let name = attr.attr.as_str();
                    NAMES.iter().copied().find(|n| *n == name)
                }
                _ => None,
            },
            _ => None,
        }
    }

    fn builtin(&self, name: &str) -> bool {
        !self.rebound.contains_key(name)
    }

    /// `deque[T]` -> `list[T]`, `Counter[K]` -> `dict[K, int]`,
    /// `defaultdict[K, V]` -> `dict[K, V]`.
    fn annotation(&self, expr: &Expr) -> Option<Expr> {
        let Expr::Subscript(sub) = expr else {
            return None;
        };
        let range = sub.range;
        match self.target(&sub.value)? {
            "deque" => Some(subscript("list", (*sub.slice).clone(), range)),
            "defaultdict" => Some(subscript("dict", (*sub.slice).clone(), range)),
            _ => {
                let key = (*sub.slice).clone();
                let pair =
                    super::build::tuple(vec![key, load("int", range)], ExprContext::Load, range);
                Some(subscript("dict", pair, range))
            }
        }
    }

    /// The prototype value a missing key of a `defaultdict(factory)` gets a
    /// copy of, and the factory's `repr`; `None` when copies of one value do
    /// not reproduce what calling `factory` gives.
    fn prototype(&self, factory: &Expr, range: TextRange) -> Option<(Expr, String)> {
        match factory {
            Expr::Name(name) if self.builtin(name.id.as_str()) => {
                let proto = match name.id.as_str() {
                    "int" => int(0, range),
                    "float" => Expr::NumberLiteral(ruff_python_ast::ExprNumberLiteral {
                        node_index: Default::default(),
                        range,
                        value: ruff_python_ast::Number::Float(0.0),
                    }),
                    "str" => string("", range),
                    "bool" => Expr::BooleanLiteral(ruff_python_ast::ExprBooleanLiteral {
                        node_index: Default::default(),
                        range,
                        value: false,
                    }),
                    "list" => empty_list(range),
                    "dict" => empty_dict(range),
                    "set" => call_name("set", Vec::new(), range),
                    _ => {
                        let native = self.target(factory)?;
                        let proto = self.construct(native, &[], &HashMap::new(), range)?;
                        return Some((proto, class_repr(native)));
                    }
                };
                Some((proto, format!("<class '{}'>", name.id)))
            }
            Expr::Attribute(_) => {
                let native = self.target(factory)?;
                let proto = self.construct(native, &[], &HashMap::new(), range)?;
                Some((proto, class_repr(native)))
            }
            Expr::Lambda(lambda) => {
                if lambda.parameters.as_ref().is_some_and(|p| !p.is_empty()) {
                    return None;
                }
                // Already rewritten: the transformer visits arguments first.
                let body = (*lambda.body).clone();
                (copyable_literal(&body) || native_prototype(&body))
                    .then(|| (body, "<function <lambda>>".to_string()))
            }
            _ => None,
        }
    }

    /// The rewrite of a call of `collections` name `native` with `args` and
    /// `keywords`.
    fn construct(
        &self,
        native: &str,
        args: &[Expr],
        keywords: &HashMap<&str, &Expr>,
        range: TextRange,
    ) -> Option<Expr> {
        match native {
            "deque" => {
                if args.len() > 2
                    || keywords
                        .keys()
                        .any(|k| !matches!(*k, "iterable" | "maxlen"))
                {
                    return None;
                }
                let items = match args.first().or_else(|| keywords.get("iterable").copied()) {
                    None => empty_list(range),
                    Some(list @ Expr::List(_)) => list.clone(),
                    Some(iterable) => call_name("list", vec![iterable.clone()], range),
                };
                let (maxlen, has) = match args.get(1).or_else(|| keywords.get("maxlen").copied()) {
                    None | Some(Expr::NoneLiteral(_)) => (int(0, range), 0),
                    Some(maxlen) => (maxlen.clone(), 1),
                };
                Some(call_name(
                    "__pycc_deque",
                    vec![items, maxlen, int(has, range)],
                    range,
                ))
            }
            "Counter" => match (args, keywords.is_empty()) {
                ([], true) => Some(call_name("__pycc_counter", vec![empty_dict(range)], range)),
                ([iterable], true) => Some(call_name(
                    "__pycc_counter_of",
                    vec![iterable.clone()],
                    range,
                )),
                _ => None,
            },
            _ => {
                if !keywords.is_empty() || args.len() > 2 {
                    return None;
                }
                let items = match args.get(1) {
                    None => empty_dict(range),
                    Some(mapping) => call_name("dict", vec![mapping.clone()], range),
                };
                let Some(factory) = args.first() else {
                    // `defaultdict()`: no factory, a plain dict that prints
                    // as `defaultdict(None, {...})`.
                    return Some(call_name("__pycc_defaultdict_none", vec![items], range));
                };
                if let Some((proto, repr)) = self.prototype(factory, range) {
                    return Some(call_name(
                        "__pycc_defaultdict",
                        vec![items, proto, string(&repr, range)],
                        range,
                    ));
                }
                match factory {
                    Expr::Name(name) => Some(call_name(
                        "__pycc_defaultdict_call",
                        vec![
                            items,
                            factory.clone(),
                            string(&format!("<function {}>", name.id), range),
                        ],
                        range,
                    )),
                    _ => None,
                }
            }
        }
    }

    fn rewrite_call(&self, call: &ExprCall) -> Option<Expr> {
        let native = self.target(&call.func)?;
        if call
            .arguments
            .args
            .iter()
            .any(|a| matches!(a, Expr::Starred(_)))
        {
            return None;
        }
        let mut keywords: HashMap<&str, &Expr> = HashMap::new();
        for keyword in &call.arguments.keywords {
            keywords.insert(keyword.arg.as_ref()?.as_str(), &keyword.value);
        }
        self.construct(native, &call.arguments.args, &keywords, call.range)
    }
}

/// The `repr` of a `collections` class used as a `defaultdict` factory.
fn class_repr(native: &str) -> String {
    format!("<class 'collections.{native}'>")
}

impl Transformer for Rewrite {
    fn visit_expr(&self, expr: &mut Expr) {
        transformer::walk_expr(self, expr);
        let replacement = match expr {
            Expr::Call(call) => self.rewrite_call(call),
            Expr::Subscript(_) => self.annotation(expr),
            _ => None,
        };
        if let Some(replacement) = replacement {
            *expr = replacement;
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

/// Rewrites the native `collections` constructors and annotations of `body`
/// (a module).
pub(crate) fn native_collections(body: &mut Vec<Stmt>) {
    let rebound = rebound_names(body);
    let mut rewrite = Rewrite {
        modules: HashSet::new(),
        names: HashMap::new(),
        rebound: HashMap::new(),
    };
    let mut seen: HashMap<String, usize> = HashMap::new();
    for stmt in body.iter() {
        match stmt {
            Stmt::Import(import) => {
                for alias in &import.names {
                    let local = alias.asname.as_ref().unwrap_or(&alias.name).to_string();
                    *seen.entry(local.clone()).or_default() += 1;
                    if alias.name.as_str() == "collections" {
                        rewrite.modules.insert(local);
                    }
                }
            }
            Stmt::ImportFrom(import) if import.level == 0 => {
                let module = import.module.as_ref().map(|m| m.as_str()).unwrap_or("");
                for alias in &import.names {
                    let local = alias.asname.as_ref().unwrap_or(&alias.name).to_string();
                    *seen.entry(local.clone()).or_default() += 1;
                    let name = alias.name.as_str();
                    if module == "collections" && NAMES.contains(&name) {
                        rewrite.names.insert(local, name.to_string());
                    }
                }
            }
            _ => {}
        }
    }
    let unique = |name: &String| seen.get(name) == Some(&1) && !rebound.contains_key(name);
    rewrite.modules.retain(|name| unique(name));
    rewrite.names.retain(|name, _| unique(name));
    rewrite.rebound = rebound;
    if rewrite.modules.is_empty() && rewrite.names.is_empty() {
        return;
    }
    for stmt in body.iter_mut() {
        rewrite.visit_stmt(stmt);
    }
    let read = read_names(body);
    let droppable = |local: &str| {
        (rewrite.modules.contains(local) || rewrite.names.contains_key(local))
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
