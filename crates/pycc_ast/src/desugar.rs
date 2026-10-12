//! Source-level desugaring: rewrites of valid Python into equivalent valid
//! Python that the rest of the compiler already supports, run once on the
//! parsed module before HIR lowering.
//!
//! Every rewrite preserves CPython's observable behaviour -- evaluation
//! order, which exception is raised -- except where `docs/DEVIATIONS.md`
//! says otherwise (`map`/`filter` produce a list instead of a lazy
//! iterator). The rewrites:
//!
//! * `sys.stdin.readline()`, `sys.stdin.read()`, `sys.stdin.readlines()`,
//!   `sys.stdout.write(s)`, `sys.stdout.flush()` and
//!   `sys.setrecursionlimit(n)` become calls of native builtins
//!   (`__pycc_stdin_readline` and so on), so stdin is read through one
//!   buffer whichever spelling a program uses; an `import sys` with no use
//!   left is dropped. A module-level `input = sys.stdin.readline` becomes
//!   a `def` (see [`aliases`]).
//! * A typed numpy array annotation without a dimension count,
//!   `NDArray[np.float64]`, gets the count its name's uses imply written in
//!   as a second argument, `NDArray[np.float64, 2]` (see [`ndarray_ndim`]).
//! * A module-level `name = lambda params: body` becomes `def name(params):
//!   return body`, its signature inferred (the [`INFER_MARKER`] return
//!   annotation tells HIR lowering to treat it like a private helper).
//! * A function nested in a function becomes a module-level function
//!   receiving the enclosing variables it reads (see [`closures`]).
//! * `key=`/`reverse=` arguments of `sorted`, `min`, `max` and
//!   `list.sort` become a list of `(key, element)` pairs (see [`keys`]);
//!   comprehension tuple targets become subscripts of a fresh variable and
//!   a generator argument of `sum`/`min`/`max`/`str.join` becomes a list
//!   comprehension (see [`comps`]).
//! * Any other generator expression becomes a call of a module-level
//!   generator helper, comprehensions with several clauses (or over a
//!   generator) helpers with nested loops, and a starred call a helper
//!   checking the count (see [`helpers`]); `yield from it` becomes `for v
//!   in it: yield v`. An `--ext` module's public generators are split by
//!   [`split_ext_generators`], which HIR lowering calls.
//! * `map(f, xs)` and `filter(f, xs)` become list comprehensions, a lambda
//!   argument inlined; `[*xs]` becomes `list(xs)`; `print(*xs, sep=s)`
//!   becomes `print(s.join([str(x) for x in xs]))`.
//! * `for` loops over a tuple target, `enumerate(...)`, `zip(...)`,
//!   `d.items()` or any non-name iterable, a subscript store through a
//!   computed base (`g[i][j] = v`), tuple targets with non-name elements
//!   (`a[i], a[j] = a[j], a[i]`) and augmented assignment through a
//!   computed index (`a[i + 1] += x`) are rewritten into statements binding
//!   fresh temporaries (`_pycc_N`).

mod aliases;
mod build;
mod closures;
mod displays;
mod comps;
mod exprs;
mod ext_generators;
mod ext_signatures;
mod globals;
mod helpers;
mod iter_alias;
mod hoist;
mod keys;
mod ndarray_ndim;
mod signatures;
mod starred;
mod stdin_bytes;
mod stdlib;
mod stmts;
mod webs;
mod wildcard;

use ruff_python_ast::{Expr, ModModule, Stmt};

pub use closures::qualname::function_qualname;
pub use ext_generators::split_ext_generators;
pub use ext_signatures::{ExtSignature, ext_entry_target, ext_signatures, split_ext_signatures};

/// The first `yield`/`yield from` inside `expr` (not inside a nested
/// lambda), for a context where CPython refuses one (a comprehension).
pub fn find_yield(expr: &Expr) -> Option<&Expr> {
    use ruff_python_ast::visitor::{self, Visitor};
    struct Find<'a>(Option<&'a Expr>);
    impl<'a> Visitor<'a> for Find<'a> {
        fn visit_expr(&mut self, expr: &'a Expr) {
            if self.0.is_some() {
                return;
            }
            match expr {
                Expr::Yield(_) | Expr::YieldFrom(_) => self.0 = Some(expr),
                Expr::Lambda(_) => {}
                _ => visitor::walk_expr(self, expr),
            }
        }
    }
    let mut find = Find(None);
    find.visit_expr(expr);
    find.0
}

/// Whether a function body contains a `yield` (or `yield from`) of its own
/// -- not of a nested function, class, lambda or comprehension: whether the
/// function is a generator.
pub fn body_yields(body: &[Stmt]) -> bool {
    ext_generators::yields(body)
}

/// The return annotation [`desugar`] gives a synthesized `def` whose
/// signature is to be inferred rather than declared.
pub const INFER_MARKER: &str = "__pycc_infer__";

/// The callee of `__pycc_closure__(f, a, b)`: the function value of `f` with
/// its first parameters bound to `a` and `b` -- a nested function used as a
/// value, lifted with the enclosing variables it reads ([`closures`]).
pub const CLOSURE_MARKER: &str = "__pycc_closure__";

/// Fresh temporary names, `_pycc_0`, `_pycc_1`, ...
#[derive(Default)]
pub(crate) struct Fresh {
    next: std::cell::Cell<usize>,
    /// Names bound by an `import` other than `sys`: a value reached
    /// through one (`m.attr`, `m.f()`) may be a CPython object, so a loop
    /// over it is left to `for x in o.attr` / `o.method()` object iteration
    /// (`HirStmt::ForObject`), which releases the iterable as CPython does,
    /// instead of being bound to a module-lifetime temporary.
    pub(crate) import_roots: std::collections::HashSet<String>,
}

impl Fresh {
    pub(crate) fn name(&self) -> String {
        let n = self.next.get();
        self.next.set(n + 1);
        format!("_pycc_{n}")
    }
}

/// Rewrites `module` in place (see the module documentation).
pub fn desugar(module: &mut ModModule) {
    {
        let mut body: Vec<Stmt> = std::mem::take(&mut module.body).into_iter().collect();
        displays::canonical_displays(&mut body);
        wildcard::expand_wildcards(&mut body);
        ndarray_ndim::close_ndims(&mut body);
        aliases::builtin_aliases(&mut body);
        iter_alias::iterator_readers(&mut body);
        aliases::drop_object_bases(&mut body);
        stdlib::native_stdlib_calls(&mut body);
        webs::split_webs(&mut body);
        module.body = body.into();
    }
    let fresh = Fresh {
        import_roots: import_roots(&module.body),
        ..Fresh::default()
    };
    {
        let mut body: Vec<Stmt> = std::mem::take(&mut module.body).into_iter().collect();
        globals::cell_globals(&mut body, &fresh);
        module.body = body.into();
    }
    let sys = aliases::SysNames::collect(&module.body);
    aliases::module_level_defs(&mut module.body, &sys);
    closures::lift_closures(&mut module.body, &fresh);
    exprs::rewrite_exprs(&mut module.body, &sys, &fresh);
    let mut body: Vec<Stmt> = std::mem::take(&mut module.body).into_iter().collect();
    signatures::bind_signatures(&mut body, &fresh);
    helpers::lift_helpers(&mut body, &fresh);
    stmts::expand_body(&mut body, &fresh, true);
    aliases::drop_unused_sys_imports(&mut body, &sys);
    module.body = body.into();
}

/// Every top-level name an `import` binds, `sys` excepted.
fn import_roots(body: &[Stmt]) -> std::collections::HashSet<String> {
    let mut roots = std::collections::HashSet::new();
    for stmt in body {
        match stmt {
            Stmt::Import(import) => {
                for alias in &import.names {
                    let bound = alias.asname.as_ref().unwrap_or(&alias.name);
                    roots.insert(bound.split('.').next().unwrap_or("").to_string());
                }
            }
            Stmt::ImportFrom(import) if import.module.as_deref() != Some("sys") => {
                for alias in &import.names {
                    roots.insert(alias.asname.as_ref().unwrap_or(&alias.name).to_string());
                }
            }
            _ => {}
        }
    }
    roots.remove("sys");
    roots
}

/// Whether `expr` is a bare name.
pub(crate) fn is_name(expr: &Expr) -> bool {
    matches!(expr, Expr::Name(_))
}

/// Whether evaluating `expr` can have no side effect and raise nothing a
/// reordering could expose: a name or a literal.
pub(crate) fn is_simple(expr: &Expr) -> bool {
    matches!(
        expr,
        Expr::Name(_)
            | Expr::NumberLiteral(_)
            | Expr::StringLiteral(_)
            | Expr::BooleanLiteral(_)
            | Expr::NoneLiteral(_)
    ) || matches!(expr, Expr::UnaryOp(u) if matches!(*u.operand, Expr::NumberLiteral(_)))
}

/// Every name stored anywhere in `body` (assignment and loop targets,
/// `def`/`class` names, parameters, imports).
pub(crate) fn stored_names(body: &[Stmt]) -> std::collections::HashSet<String> {
    use ruff_python_ast::visitor::{self, Visitor};
    struct Collect(std::collections::HashSet<String>);
    impl<'a> Visitor<'a> for Collect {
        fn visit_stmt(&mut self, stmt: &'a Stmt) {
            match stmt {
                Stmt::FunctionDef(def) => {
                    self.0.insert(def.name.to_string());
                    for p in def.parameters.iter() {
                        self.0.insert(p.name().to_string());
                    }
                }
                Stmt::ClassDef(def) => {
                    self.0.insert(def.name.to_string());
                }
                Stmt::Import(import) => {
                    for alias in &import.names {
                        let bound = alias.asname.as_ref().unwrap_or(&alias.name);
                        self.0
                            .insert(bound.split('.').next().unwrap_or("").to_string());
                    }
                }
                Stmt::ImportFrom(import) => {
                    for alias in &import.names {
                        let bound = alias.asname.as_ref().unwrap_or(&alias.name);
                        self.0.insert(bound.to_string());
                    }
                }
                Stmt::Global(_) | Stmt::Nonlocal(_) => {}
                _ => {}
            }
            visitor::walk_stmt(self, stmt);
        }
        fn visit_expr(&mut self, expr: &'a Expr) {
            match expr {
                Expr::Name(name) if name.ctx == ruff_python_ast::ExprContext::Store => {
                    self.0.insert(name.id.to_string());
                }
                Expr::Lambda(lambda) => {
                    if let Some(params) = &lambda.parameters {
                        for p in params.iter() {
                            self.0.insert(p.name().to_string());
                        }
                    }
                }
                _ => {}
            }
            visitor::walk_expr(self, expr);
        }
    }
    let mut c = Collect(Default::default());
    for stmt in body {
        c.visit_stmt(stmt);
    }
    c.0
}
