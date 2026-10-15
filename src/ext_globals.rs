//! A strict `--ext` module's native module-level variables as module
//! attributes (ROADMAP v0.4 accept 5).
//!
//! CPython binds every module global in the module's `__dict__`, so
//! `mod.LIMIT` and `from mod import LIMIT` read a module variable. A
//! compiled global is a native value, so the entry module is rewritten to
//! copy it onto the module object after every store, as the non-strict
//! `--ext` build does (`crate::fallback`'s mirrors):
//!
//! ```text
//! """Doc."""; import sys as __pycc_ext_sys; __pycc_ext_module = __pycc_ext_sys.modules[__name__]
//! LIMIT = 10; __pycc_ext_module.LIMIT = LIMIT
//! def grow() -> None:
//!     global LIMIT
//!     LIMIT += 1; __pycc_ext_module.LIMIT = LIMIT
//! ```
//!
//! CPython puts the extension module in `sys.modules` before its body runs
//! (PEP 489), so the copy is visible to Python code as soon as the store
//! runs. A name (other than the compiler's own and the module attributes
//! CPython itself manages) bound at module scope by an assignment, an
//! augmented or annotated assignment is copied, at module scope and in a
//! function declaring it `global`, when its value is a native one: the
//! annotation or one module-level value says so (literals, arithmetic on
//! them, a call of a function annotated to return one, `str()`/`int()`/
//! `len()`, ...). A CPython object (an import, a foreign call's result, an
//! instance or enum member, which cross the boundary as one) is not
//! copied: CPython's module holds the only reference to a global, and a
//! second one would be visible (`sys.getrefcount`, when the object is freed). A name also
//! bound another way (a `for` or `with` target, `del`, a walrus, a `match`
//! capture), or whose value the boundary cannot carry (a native `list`,
//! `dict` or `set`: a copy would not share the object), is left
//! unpublished: the rewrite is compiled, and a name whose copy fails to
//! compile is dropped and the rewrite compiled again.
//!
//! The rewrite adds no line, so every diagnostic keeps its position: the
//! preamble follows the docstring or `from __future__` imports, or else the
//! first simple statement, on its line.

use pycc_ast::visitor::{self, Visitor};
use pycc_ast::{Expr, ExprContext, Stmt};
use std::collections::BTreeSet;
use std::ops::Range;

mod native;

/// The `sys` module's name in the rewritten source.
const SYS: &str = "__pycc_ext_sys";
/// The module object's name in the rewritten source.
pub(crate) const MODULE: &str = "__pycc_ext_module";

/// A rendered rewrite: the text and where each copy of a name went.
pub(crate) struct Rewrite {
    pub(crate) text: String,
    pub(crate) copies: Vec<(Range<u32>, String)>,
}

/// Module attributes CPython manages itself, and the PEP 562 hooks the
/// shim publishes after the body (#1467): never copied.
const MANAGED: &[&str] = &[
    "__getattr__",
    "__dir__",
    "__name__",
    "__doc__",
    "__file__",
    "__spec__",
    "__loader__",
    "__package__",
    "__path__",
    "__builtins__",
    "__dict__",
    "__class__",
    "__all__",
];

/// The public module globals `body` binds with a copyable store and a
/// native value, minus those also bound another way.
fn published_names(body: &[Stmt]) -> BTreeSet<String> {
    let mut finder = Bindings::default();
    for stmt in body {
        finder.module_stmt(stmt);
    }
    let native = native::native_names(body);
    finder
        .simple
        .difference(&finder.other)
        .filter(|name| native.contains(*name))
        .filter(|name| {
            !(name.starts_with("__pycc")
                || pycc_ast::is_reserved_name(name)
                || MANAGED.contains(&name.as_str()))
        })
        .cloned()
        .collect()
}

/// Module-scope bindings, split by whether a copy can follow them.
#[derive(Default)]
struct Bindings {
    simple: BTreeSet<String>,
    other: BTreeSet<String>,
}

impl Bindings {
    fn module_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::FunctionDef(def) => {
                self.other.insert(def.name.to_string());
                let globals = declared_globals(&def.body);
                if !globals.is_empty() {
                    let mut inner = Bindings::default();
                    for stmt in &def.body {
                        inner.function_stmt(stmt);
                    }
                    self.simple
                        .extend(inner.simple.intersection(&globals).cloned());
                    self.other.extend(inner.other.intersection(&globals).cloned());
                }
            }
            Stmt::ClassDef(class) => {
                self.other.insert(class.name.to_string());
            }
            _ => {
                if let Some(names) = simple_targets(stmt) {
                    self.simple.extend(names);
                    // A walrus inside the value is another kind of binding.
                    let mut walrus = StoreNames::default();
                    walrus.visit_stmt(stmt);
                    self.other.extend(walrus.walrus);
                } else {
                    self.other.extend(own_bindings(stmt));
                    // A compound statement's nested statements.
                    for nested in nested_bodies(stmt) {
                        for stmt in nested {
                            self.module_stmt(stmt);
                        }
                    }
                }
            }
        }
    }

    fn function_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::FunctionDef(_) | Stmt::ClassDef(_) => {}
            _ => {
                if let Some(names) = simple_targets(stmt) {
                    self.simple.extend(names);
                } else {
                    self.other.extend(own_bindings(stmt));
                    for nested in nested_bodies(stmt) {
                        for stmt in nested {
                            self.function_stmt(stmt);
                        }
                    }
                }
            }
        }
    }
}

/// The names a statement that is not a copyable simple statement binds
/// itself, outside its nested bodies: a `for` or `with` target, an `except`
/// name, a `match` capture, `del`, a walrus in a header or expression.
fn own_bindings(stmt: &Stmt) -> BTreeSet<String> {
    let mut stores = StoreNames::default();
    match stmt {
        Stmt::For(s) => {
            stores.visit_expr(&s.target);
            stores.visit_expr(&s.iter);
        }
        Stmt::While(s) => stores.visit_expr(&s.test),
        Stmt::If(s) => {
            stores.visit_expr(&s.test);
            for clause in &s.elif_else_clauses {
                if let Some(test) = &clause.test {
                    stores.visit_expr(test);
                }
            }
        }
        Stmt::With(s) => {
            for item in &s.items {
                stores.visit_expr(&item.context_expr);
                if let Some(vars) = &item.optional_vars {
                    stores.visit_expr(vars);
                }
            }
        }
        Stmt::Try(s) => {
            for handler in &s.handlers {
                let pycc_ast::ExceptHandler::ExceptHandler(h) = handler;
                if let Some(name) = &h.name {
                    stores.all.insert(name.to_string());
                }
            }
        }
        Stmt::Match(s) => {
            stores.visit_expr(&s.subject);
            for case in &s.cases {
                stores.visit_pattern(&case.pattern);
            }
        }
        other => stores.visit_stmt(other),
    }
    stores.all
}

/// The bodies nested in a compound statement (not a `def` or `class`).
fn nested_bodies(stmt: &Stmt) -> Vec<&[Stmt]> {
    match stmt {
        Stmt::If(s) => {
            let mut out = vec![s.body.as_slice()];
            out.extend(s.elif_else_clauses.iter().map(|c| c.body.as_slice()));
            out
        }
        Stmt::While(s) => vec![s.body.as_slice(), s.orelse.as_slice()],
        Stmt::For(s) => vec![s.body.as_slice(), s.orelse.as_slice()],
        Stmt::With(s) => vec![s.body.as_slice()],
        Stmt::Try(s) => {
            let mut out = vec![s.body.as_slice(), s.orelse.as_slice(), s.finalbody.as_slice()];
            for handler in &s.handlers {
                let pycc_ast::ExceptHandler::ExceptHandler(h) = handler;
                out.push(h.body.as_slice());
            }
            out
        }
        Stmt::Match(s) => s.cases.iter().map(|c| c.body.as_slice()).collect(),
        _ => Vec::new(),
    }
}

/// The names a simple statement binds where a copy can follow it: the
/// name targets of an assignment (a tuple of names included), an augmented
/// or valued annotated assignment, or the bindings of an import; `None` for
/// any other statement.
fn simple_targets(stmt: &Stmt) -> Option<Vec<String>> {
    fn names(target: &Expr, out: &mut Vec<String>) -> bool {
        match target {
            Expr::Name(n) => {
                out.push(n.id.to_string());
                true
            }
            Expr::Tuple(t) => t.elts.iter().all(|e| names(e, out)),
            Expr::List(l) => l.elts.iter().all(|e| names(e, out)),
            // An attribute or subscript store binds no global.
            Expr::Attribute(_) | Expr::Subscript(_) => true,
            _ => false,
        }
    }
    let mut out = Vec::new();
    let ok = match stmt {
        Stmt::Assign(a) => a.targets.iter().all(|t| names(t, &mut out)),
        Stmt::AugAssign(a) => names(&a.target, &mut out),
        Stmt::AnnAssign(a) if a.value.is_some() => names(&a.target, &mut out),
        Stmt::Import(i) => {
            for alias in &i.names {
                let bound = alias.asname.as_ref().unwrap_or(&alias.name);
                out.push(bound.split('.').next().unwrap_or_default().to_string());
            }
            true
        }
        Stmt::ImportFrom(i) => {
            if i.module.as_deref() == Some("__future__") {
                return None;
            }
            for alias in &i.names {
                if alias.name.as_str() == "*" {
                    return None;
                }
                out.push(alias.asname.as_ref().unwrap_or(&alias.name).to_string());
            }
            true
        }
        _ => false,
    };
    ok.then_some(out)
}

/// The names a function body declares `global`.
fn declared_globals(body: &[Stmt]) -> BTreeSet<String> {
    #[derive(Default)]
    struct Find(BTreeSet<String>);
    impl<'a> Visitor<'a> for Find {
        fn visit_stmt(&mut self, stmt: &'a Stmt) {
            match stmt {
                Stmt::Global(g) => self.0.extend(g.names.iter().map(|n| n.to_string())),
                Stmt::FunctionDef(_) | Stmt::ClassDef(_) => {}
                _ => visitor::walk_stmt(self, stmt),
            }
        }
    }
    let mut find = Find::default();
    for stmt in body {
        find.visit_stmt(stmt);
    }
    find.0
}

/// Every name a statement stores (outside nested scopes), and those a
/// walrus stores.
#[derive(Default)]
struct StoreNames {
    all: BTreeSet<String>,
    walrus: BTreeSet<String>,
}

impl<'a> Visitor<'a> for StoreNames {
    fn visit_stmt(&mut self, stmt: &'a Stmt) {
        match stmt {
            Stmt::FunctionDef(_) | Stmt::ClassDef(_) => {}
            Stmt::Delete(d) => {
                for target in &d.targets {
                    if let Expr::Name(n) = target {
                        self.all.insert(n.id.to_string());
                    }
                }
            }
            _ => visitor::walk_stmt(self, stmt),
        }
    }

    fn visit_expr(&mut self, expr: &'a Expr) {
        match expr {
            Expr::Name(n) if n.ctx == ExprContext::Store => {
                self.all.insert(n.id.to_string());
            }
            Expr::Named(named) => {
                if let Expr::Name(n) = &*named.target {
                    self.walrus.insert(n.id.to_string());
                    self.all.insert(n.id.to_string());
                }
                self.visit_expr(&named.value);
            }
            Expr::Lambda(_) => {}
            Expr::ListComp(_) | Expr::SetComp(_) | Expr::DictComp(_) | Expr::Generator(_) => {}
            _ => visitor::walk_expr(self, expr),
        }
    }

    fn visit_pattern(&mut self, pattern: &'a pycc_ast::Pattern) {
        if let pycc_ast::Pattern::MatchAs(p) = pattern
            && let Some(name) = &p.name
        {
            self.all.insert(name.to_string());
        }
        if let pycc_ast::Pattern::MatchStar(p) = pattern
            && let Some(name) = &p.name
        {
            self.all.insert(name.to_string());
        }
        visitor::walk_pattern(self, pattern);
    }
}

/// The insertion points of the copies: after every simple statement, at
/// module scope or in a function declaring the name `global`, that binds a
/// published name.
/// Each copy's offset and name, and whether it is in a function.
fn copy_points(body: &[Stmt], published: &BTreeSet<String>) -> Vec<(u32, String, bool)> {
    fn walk(
        stmts: &[Stmt],
        published: &BTreeSet<String>,
        globals: Option<&BTreeSet<String>>,
        out: &mut Vec<(u32, String, bool)>,
    ) {
        for stmt in stmts {
            match stmt {
                Stmt::FunctionDef(def) => {
                    let declared = declared_globals(&def.body);
                    let scope: BTreeSet<String> =
                        declared.intersection(published).cloned().collect();
                    if !scope.is_empty() {
                        walk(&def.body, published, Some(&scope), out);
                    }
                }
                Stmt::ClassDef(_) => {}
                _ => {
                    if let Some(names) = simple_targets(stmt) {
                        let end = pycc_ast::stmt_range(stmt).end;
                        let mut seen = BTreeSet::new();
                        for name in names {
                            let visible = match globals {
                                Some(scope) => scope.contains(&name),
                                None => published.contains(&name),
                            };
                            if visible && seen.insert(name.clone()) {
                                out.push((end, name, globals.is_some()));
                            }
                        }
                    } else {
                        for nested in nested_bodies(stmt) {
                            walk(nested, published, globals, out);
                        }
                    }
                }
            }
        }
    }
    let mut out = Vec::new();
    walk(body, published, None, &mut out);
    out
}

/// Where the preamble goes: the end of the last docstring or `from
/// __future__` statement, or else of the first simple statement. Only
/// `def` and `class` statements may precede that one, unless no copy can
/// run before it (none precedes it, and no function copies); `None` when
/// there is no such place.
fn anchor(body: &[Stmt], points: &[(u32, String, bool)]) -> Option<usize> {
    let prologue = body
        .iter()
        .take_while(|stmt| {
            matches!(stmt, Stmt::Expr(e) if matches!(*e.value, Expr::StringLiteral(_)))
                || matches!(stmt, Stmt::ImportFrom(i) if i.module.as_deref() == Some("__future__"))
        })
        .count();
    let (position, stmt) = if prologue > 0 {
        (prologue - 1, &body[prologue - 1])
    } else {
        body.iter().enumerate().find(|(_, stmt)| simple_statement(stmt))?
    };
    let end = pycc_ast::stmt_range(stmt).end;
    let only_definitions = body[..position]
        .iter()
        .all(|stmt| matches!(stmt, Stmt::FunctionDef(_) | Stmt::ClassDef(_)));
    let nothing_runs_first = points
        .iter()
        .all(|(at, _, in_function)| !in_function && *at >= end);
    (only_definitions || nothing_runs_first).then_some(end as usize)
}

/// Whether `stmt` is a simple statement (a preamble can follow it on its
/// line).
fn simple_statement(stmt: &Stmt) -> bool {
    !matches!(
        stmt,
        Stmt::FunctionDef(_)
            | Stmt::ClassDef(_)
            | Stmt::If(_)
            | Stmt::While(_)
            | Stmt::For(_)
            | Stmt::With(_)
            | Stmt::Try(_)
            | Stmt::Match(_)
    )
}

/// The rewrite of `source` copying every published global but `dropped`;
/// `None` when nothing is published (or the source does not parse).
pub(crate) fn rewrite(source: &str, dropped: &BTreeSet<String>) -> Option<Rewrite> {
    let module = pycc_parser::parse(source).ok()?;
    let body: Vec<Stmt> = module.body.iter().cloned().collect();
    let published: BTreeSet<String> = published_names(&body)
        .difference(dropped)
        .cloned()
        .collect();
    if published.is_empty() {
        return None;
    }
    let points = copy_points(&body, &published);
    let anchor = anchor(&body, &points)?;
    let preamble = format!("; import sys as {SYS}; {MODULE} = {SYS}.modules[__name__]");
    let mut inserts: Vec<(usize, String, Option<String>)> = vec![(anchor, preamble, None)];
    for (at, name, _) in points {
        inserts.push((at as usize, format!("; {MODULE}.{name} = {name}"), Some(name)));
    }
    inserts.sort_by_key(|(at, _, _)| *at);
    let mut text = String::with_capacity(source.len() + inserts.len() * 40);
    let mut copies = Vec::new();
    let mut cursor = 0;
    for (at, insert, name) in inserts {
        text.push_str(&source[cursor..at]);
        let start = text.len() as u32;
        text.push_str(&insert);
        if let Some(name) = name {
            copies.push((start..text.len() as u32, name));
        }
        cursor = at;
    }
    text.push_str(&source[cursor..]);
    Some(Rewrite { text, copies })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copies_follow_module_and_global_stores() {
        let source = "\"\"\"Doc.\"\"\"\nimport json\nLIMIT = 10\n__version__ = '1'\nfor i in range(3):\n    \
                      pass\n\ndef grow() -> None:\n    global LIMIT\n    LIMIT += 1\n";
        let out = super::rewrite(source, &BTreeSet::new()).expect("published");
        assert_eq!(
            out.text,
            "\"\"\"Doc.\"\"\"; import sys as __pycc_ext_sys; __pycc_ext_module = \
             __pycc_ext_sys.modules[__name__]\nimport json\nLIMIT = 10; \
             __pycc_ext_module.LIMIT = LIMIT\n__version__ = '1'; __pycc_ext_module.__version__ = \
             __version__\nfor i in range(3):\n    pass\n\ndef grow() -> \
             None:\n    global LIMIT\n    LIMIT += 1; __pycc_ext_module.LIMIT = LIMIT\n"
        );
        assert_eq!(out.text.lines().count(), source.lines().count());
        let names: Vec<&str> = out.copies.iter().map(|(_, n)| n.as_str()).collect();
        assert_eq!(names, ["LIMIT", "__version__", "LIMIT"]);
    }

    #[test]
    fn the_preamble_follows_the_first_simple_statement_or_nothing_is_published() {
        let source = "def f() -> int:\n    return 1\n\nX = 1\n";
        let out = super::rewrite(source, &BTreeSet::new()).expect("published");
        assert!(
            out.text.ends_with(
                "X = 1; import sys as __pycc_ext_sys; __pycc_ext_module = \
                 __pycc_ext_sys.modules[__name__]; __pycc_ext_module.X = X\n"
            ),
            "{}",
            out.text
        );
        // A store inside an `if` before any simple statement could run
        // before the preamble.
        let source = "if True:\n    X = 1\nY = 2\n";
        assert!(super::rewrite(source, &BTreeSet::new()).is_none());
        // A hook name and a CPython object are never copied.
        let source = "import m\n__getattr__ = 5\nT = m.Thing()\n";
        assert!(super::rewrite(source, &BTreeSet::new()).is_none());
    }

    #[test]
    fn a_name_bound_another_way_or_dropped_is_not_copied() {
        let source = "n = 1\nfor n in range(2):\n    pass\nm = 2\nk = 3\n";
        let dropped: BTreeSet<String> = ["k".to_string()].into();
        let out = super::rewrite(source, &dropped).expect("m is published");
        let names: Vec<&str> = out.copies.iter().map(|(_, n)| n.as_str()).collect();
        assert_eq!(names, ["m"]);
        assert!(super::rewrite(source, &["k", "m"].map(String::from).into()).is_none());
    }
}
