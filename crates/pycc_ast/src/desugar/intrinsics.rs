//! Non-strict mode: calls of a standard-library function that the runtime
//! computes natively when the callee is that function.
//!
//! `escape(s)`, `escape(s, q)` and `escape(s, quote=q)` where `escape` is a
//! name `from html import escape [as e]` binds (anywhere in the module, a
//! `try` included), and `html.escape(...)` where `html` is a name `import
//! html [as h]` binds, become `__pycc_html_escape(callee, s, q, mode)`
//! (`pycc_hir`'s object builtins, `pycc_ext_obj_html_escape`). The callee
//! is still evaluated first, as CPython evaluates it; the runtime compares
//! it with CPython's own `html.escape` by identity and, for a `str`,
//! computes the result without a Python frame; anything else -- a rebound
//! name, `cgi.escape`, a `str` subclass -- is the very call CPython makes,
//! `mode` saying how `quote` was passed (0 not at all, 1 positionally, 2 by
//! keyword). `re.escape(s)` likewise becomes `__pycc_re_escape(callee, s)`
//! (`pycc_ext_obj_re_escape`). A function or class that binds the name
//! itself keeps its calls as written.

use super::build::{call_name, int};
use ruff_python_ast::visitor::transformer::{self, Transformer};
use ruff_python_ast::visitor::{self, Visitor};
use ruff_python_ast::{Expr, ExprContext, Parameters, Stmt};
use std::cell::Cell;
use std::collections::{HashMap, HashSet};

/// The object builtin a rewritten `html.escape` call becomes.
pub const HTML_ESCAPE: &str = "__pycc_html_escape";
/// The object builtin a rewritten `re.escape` call becomes.
pub const RE_ESCAPE: &str = "__pycc_re_escape";

/// A standard-library function computed natively.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Intrinsic {
    HtmlEscape,
    ReEscape,
}

/// Each intrinsic's module and function.
const TABLE: [(&str, &str, Intrinsic); 2] = [
    ("html", "escape", Intrinsic::HtmlEscape),
    ("re", "escape", Intrinsic::ReEscape),
];

/// The names bound to intrinsic functions and to their modules.
type Bound = (HashMap<String, Intrinsic>, HashMap<String, &'static str>);

thread_local! {
    /// The names [`set_intrinsic_hints`] recorded for the next module
    /// desugared on this thread.
    static HINTS: std::cell::RefCell<Option<Bound>> =
        const { std::cell::RefCell::new(None) };
}

/// Records the names the module's original source binds by its imports
/// (`from html import escape`, `import html`), for the next module
/// desugared on this thread: the non-strict fallback rewrite replaces a
/// statement CPython runs (a `try` around the import) with a call that
/// runs it, which hides those bindings from the rewritten source.
pub fn set_intrinsic_hints(original: &[Stmt]) {
    HINTS.with(|hints| *hints.borrow_mut() = Some(bound_names(original)));
}

/// Rewrites the module's `html.escape` calls (module doc).
pub(crate) fn native_intrinsics(body: &mut [Stmt]) {
    let hinted = HINTS.with(|hints| hints.borrow_mut().take());
    if !super::dataclasses::is_non_strict() {
        return;
    }
    let (mut functions, mut modules) = bound_names(body);
    if let Some((hinted_functions, hinted_modules)) = hinted {
        functions.extend(hinted_functions);
        modules.extend(hinted_modules);
    }
    if functions.is_empty() && modules.is_empty() {
        return;
    }
    let rewrite = Rewrite {
        functions,
        modules,
        shadowed: Cell::new(0),
    };
    for stmt in body.iter_mut() {
        rewrite.visit_stmt(stmt);
    }
}

/// The names `from html import escape` (and the other [`TABLE`] imports)
/// bind, and the names `import html` (or `re`) binds, at any depth.
fn bound_names(body: &[Stmt]) -> Bound {
    #[derive(Default)]
    struct Find(HashMap<String, Intrinsic>, HashMap<String, &'static str>);
    impl<'a> Visitor<'a> for Find {
        fn visit_stmt(&mut self, stmt: &'a Stmt) {
            match stmt {
                Stmt::ImportFrom(import) if import.level == 0 => {
                    let module = import.module.as_ref().map_or("", |m| m.as_str());
                    for alias in &import.names {
                        if let Some((.., intrinsic)) = TABLE
                            .iter()
                            .find(|(m, f, _)| *m == module && *f == alias.name.as_str())
                        {
                            let bound = alias.asname.as_ref().unwrap_or(&alias.name);
                            self.0.insert(bound.to_string(), *intrinsic);
                        }
                    }
                }
                Stmt::Import(import) => {
                    for alias in &import.names {
                        if let Some((module, ..)) =
                            TABLE.iter().find(|(m, ..)| *m == alias.name.as_str())
                        {
                            let bound = alias.asname.as_ref().unwrap_or(&alias.name);
                            self.1.insert(bound.to_string(), module);
                        }
                    }
                }
                _ => visitor::walk_stmt(self, stmt),
            }
        }
    }
    let mut find = Find::default();
    for stmt in body {
        find.visit_stmt(stmt);
    }
    (find.0, find.1)
}

struct Rewrite {
    functions: HashMap<String, Intrinsic>,
    modules: HashMap<String, &'static str>,
    /// How many enclosing functions or classes bind one of the names.
    shadowed: Cell<usize>,
}

impl Rewrite {
    fn names(&self) -> impl Iterator<Item = &String> {
        self.functions.keys().chain(self.modules.keys())
    }

    /// The intrinsic `callee` may be: a read of a `from html import escape`
    /// name or `<import html name>.escape` (and the other [`TABLE`] ones).
    fn candidate(&self, callee: &Expr) -> Option<Intrinsic> {
        match callee {
            Expr::Name(name) => self.functions.get(name.id.as_str()).copied(),
            // A global the fallback rewrite reads from the module's
            // namespace (`__pycc_fallback_ns['escape']`, `src/fallback.rs`).
            Expr::Subscript(sub) => match (&*sub.value, &*sub.slice) {
                (Expr::Name(ns), Expr::StringLiteral(key))
                    if ns.id.as_str() == "__pycc_fallback_ns" =>
                {
                    self.functions.get(key.value.to_str()).copied()
                }
                _ => None,
            },
            Expr::Attribute(attr) => {
                let Expr::Name(m) = &*attr.value else {
                    return None;
                };
                let module = self.modules.get(m.id.as_str())?;
                TABLE
                    .iter()
                    .find(|(tm, tf, _)| tm == module && *tf == attr.attr.as_str())
                    .map(|(.., intrinsic)| *intrinsic)
            }
            _ => None,
        }
    }
}

/// Whether a function's parameters or body bind any of `names` (any store,
/// at any depth: conservative).
fn binds_any<'n>(
    parameters: Option<&Parameters>,
    body: &[Stmt],
    mut names: impl Iterator<Item = &'n String>,
) -> bool {
    struct Stores(HashSet<String>);
    impl<'a> Visitor<'a> for Stores {
        fn visit_expr(&mut self, expr: &'a Expr) {
            if let Expr::Name(name) = expr
                && name.ctx != ExprContext::Load
            {
                self.0.insert(name.id.to_string());
            }
            visitor::walk_expr(self, expr);
        }
        fn visit_stmt(&mut self, stmt: &'a Stmt) {
            match stmt {
                Stmt::FunctionDef(def) => {
                    self.0.insert(def.name.to_string());
                }
                Stmt::ClassDef(class) => {
                    self.0.insert(class.name.to_string());
                }
                Stmt::Import(import) => {
                    for alias in &import.names {
                        let bound = alias.asname.as_ref().unwrap_or(&alias.name);
                        self.0.insert(bound.split('.').next().unwrap_or("").to_string());
                    }
                }
                Stmt::ImportFrom(import) => {
                    for alias in &import.names {
                        self.0.insert(alias.asname.as_ref().unwrap_or(&alias.name).to_string());
                    }
                }
                Stmt::Global(global) => self.0.extend(global.names.iter().map(|n| n.to_string())),
                Stmt::Nonlocal(nonlocal) => {
                    self.0.extend(nonlocal.names.iter().map(|n| n.to_string()));
                }
                Stmt::Try(try_stmt) => {
                    for handler in &try_stmt.handlers {
                        let ruff_python_ast::ExceptHandler::ExceptHandler(h) = handler;
                        if let Some(name) = &h.name {
                            self.0.insert(name.to_string());
                        }
                    }
                }
                _ => {}
            }
            visitor::walk_stmt(self, stmt);
        }
    }
    let mut stores = Stores(HashSet::new());
    if let Some(parameters) = parameters {
        for p in parameters.iter() {
            stores.0.insert(p.name().to_string());
        }
    }
    for stmt in body {
        stores.visit_stmt(stmt);
    }
    names.any(|name| stores.0.contains(name))
}

impl Transformer for Rewrite {
    fn visit_stmt(&self, stmt: &mut Stmt) {
        let shadows = match stmt {
            Stmt::FunctionDef(def) => binds_any(Some(&def.parameters), &def.body, self.names()),
            Stmt::ClassDef(class) => binds_any(None, &class.body, self.names()),
            _ => false,
        };
        if shadows {
            self.shadowed.set(self.shadowed.get() + 1);
        }
        transformer::walk_stmt(self, stmt);
        if shadows {
            self.shadowed.set(self.shadowed.get() - 1);
        }
    }

    fn visit_expr(&self, expr: &mut Expr) {
        if let Expr::Lambda(lambda) = expr
            && let Some(parameters) = lambda.parameters.as_deref()
            && parameters.iter().any(|p| self.names().any(|n| n == p.name().as_str()))
        {
            return;
        }
        transformer::walk_expr(self, expr);
        if self.shadowed.get() > 0 {
            return;
        }
        let Expr::Call(call) = expr else {
            return;
        };
        let Some(intrinsic) = self.candidate(&call.func) else {
            return;
        };
        let arguments = &call.arguments;
        let range = call.range;
        if intrinsic == Intrinsic::ReEscape {
            if let ([s], []) = (&arguments.args[..], &arguments.keywords[..])
                && !matches!(s, Expr::Starred(_))
            {
                let args = vec![(*call.func).clone(), s.clone()];
                *expr = call_name(RE_ESCAPE, args, range);
            }
            return;
        }
        let (quote, mode) = match (&arguments.args[..], &arguments.keywords[..]) {
            ([s], []) if !matches!(s, Expr::Starred(_)) => (None, 0),
            ([s, q], []) if ![s, q].iter().any(|e| matches!(e, Expr::Starred(_))) => {
                (Some(q.clone()), 1)
            }
            ([s], [k])
                if !matches!(s, Expr::Starred(_))
                    && k.arg.as_ref().is_some_and(|a| a.as_str() == "quote") =>
            {
                (Some(k.value.clone()), 2)
            }
            _ => return,
        };
        let s = arguments.args[0].clone();
        let quote = quote.unwrap_or_else(|| {
            Expr::BooleanLiteral(ruff_python_ast::ExprBooleanLiteral {
                node_index: ruff_python_ast::AtomicNodeIndex::default(),
                range,
                value: true,
            })
        });
        let callee = (*call.func).clone();
        *expr = call_name(HTML_ESCAPE, vec![callee, s, quote, int(mode, range)], range);
    }
}
