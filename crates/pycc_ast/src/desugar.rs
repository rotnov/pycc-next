//! Source-level desugaring: rewrites of valid Python into equivalent valid
//! Python that the rest of the compiler already supports, run once on the
//! parsed module before HIR lowering.
//!
//! Every rewrite preserves CPython's observable behaviour -- evaluation
//! order, which exception is raised -- except where `docs/DEVIATIONS.md`
//! says otherwise (`map`/`filter` produce a list instead of a lazy
//! iterator). The rewrites:
//!
//! * Strict mode: a class attribute the program assigns through its class
//!   (`C.count += 1`) becomes a module variable when every access spells
//!   `C.count` (see [`class_attrs`]).
//! * A `@dataclass` class becomes a plain class with the methods
//!   `dataclasses` generates (see [`dataclasses`]); `Color(value)` of an enum
//!   class becomes its synthesized value lookup (see [`enums`]); a
//!   `@functools.cache`/`@lru_cache` function gets a native memo table (see
//!   [`functools`]).
//! * `sys.stdin.readline()`, `sys.stdin.read()`, `sys.stdin.readlines()`,
//!   `sys.stdout.write(s)`, `sys.stdout.flush()` and
//!   `sys.setrecursionlimit(n)` become calls of native builtins
//!   (`__pycc_stdin_readline` and so on), so stdin is read through one
//!   buffer whichever spelling a program uses; an `import sys` with no use
//!   left is dropped. A module-level `input = sys.stdin.readline` becomes
//!   a `def`, and `X = sys.stdin` reads `X` as the stream itself (see
//!   [`aliases`]). The builtins `exit`/`quit` and `from sys
//!   import exit` read as `sys.exit`, CPython's own (see [`exit`]).
//! * A module-level `if` whose test is known before the program runs (a
//!   flag bound once to a literal, `sys.gettrace()`) is replaced by the
//!   branch that runs (see [`static_branches`]); `x in c` of a constant
//!   tuple `c` tests a list display (see [`const_tuples`]); a parameter whose
//!   default is a builtin conversion no call passes is that builtin (see
//!   [`builtin_defaults`]).
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
//! * `itertools` calls become native builtins `__pycc_it_<function>`
//!   (see [`itertools`]); `collections.deque`/`Counter`/`defaultdict`
//!   become flavored lists and dicts (see [`collections`]).
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
mod ann_attrs;
mod class_lambdas;
mod instance_defaults;
mod class_of_self;
mod build;
mod builtin_defaults;
mod chr_ranges;
mod class_attrs;
mod closures;
mod collections;
mod displays;
mod comps;
mod const_sets;
mod const_tuples;
mod dataclasses;
mod enums;
mod exit;
mod exprs;
mod functools;
mod ext_generators;
mod ext_signatures;
mod globals;
mod helpers;
mod intrinsics;
mod item_dunders;
mod iter_alias;
mod itertools;
mod hoist;
mod keys;
mod pure_calls;
mod range_values;
mod ndarray_ndim;
mod object_except;
mod aug_target;
mod with_stmt;
mod signatures;
mod specialize;
mod starred;
mod static_branches;
mod stdin_bytes;
mod stdlib;
mod stmts;
mod str_format;
mod webs;
mod wildcard;

use ruff_python_ast::{Expr, ModModule, Stmt};

pub use closures::qualname::function_qualname;
pub use dataclasses::start_program;
pub use aliases::module_lambda_aliases;
pub use intrinsics::set_intrinsic_hints;
pub use instance_defaults::rewritten_classes as instance_default_classes;

/// Replaces each module-level `if` whose test is known before the program
/// runs by the branch that runs (see [`static_branches`]). [`desugar`] runs
/// it too; the driver runs it first, so the dead-definition pass sees the
/// definitions of the branch that runs at module level.
pub fn fold_static_branches(module: &mut ModModule) {
    let mut body: Vec<Stmt> = std::mem::take(&mut module.body).into_iter().collect();
    static_branches::fold_static_branches(&mut body);
    module.body = body.into();
}
pub use ext_generators::split_ext_generators;
pub use object_except::object_except_handlers;
pub use aug_target::desugar_computed_aug_target;
pub use with_stmt::desugar_with;
pub use specialize::{
    BYTEARRAY_ANNOTATION, FALLBACK_BUILTINS, NATIVE_PREFIX, Specialized, specialize_untyped,
};
pub use ext_signatures::{
    ExtSignature, expand_dataclasses_in as expand_dataclasses, ext_entry_name, ext_entry_target, ext_signatures,
    needs_wrapper as ext_needs_wrapper,
    split_ext_signatures,
};

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
        static_branches::fold_static_branches(&mut body);
        range_values::range_values(&mut body);
        chr_ranges::native_chr_calls(&mut body);
        class_lambdas::class_lambdas(&mut body);
        instance_defaults::instance_defaults(&mut body);
        class_attrs::mutable_class_attrs(&mut body);
        dataclasses::expand_dataclasses(&mut body);
        enums::enum_lookups(&mut body);
        functools::expand_caches(&mut body);
        item_dunders::instance_item_access(&mut body);
        const_tuples::constant_tuple_membership(&mut body);
        const_sets::hoist_constant_sets(&mut body);
        displays::canonical_displays(&mut body);
        wildcard::expand_wildcards(&mut body);
        ndarray_ndim::close_ndims(&mut body);
        exit::sys_exit_calls(&mut body);
        pure_calls::drop_effectless_calls(&mut body);
        intrinsics::native_intrinsics(&mut body);
        aliases::builtin_aliases(&mut body);
        aliases::stream_aliases(&mut body);
        builtin_defaults::inline_builtin_defaults(&mut body);
        iter_alias::iterator_readers(&mut body);
        aliases::drop_object_bases(&mut body);
        class_of_self::class_of_self(&mut body);
        stdlib::native_stdlib_calls(&mut body);
        itertools::native_itertools_calls(&mut body);
        collections::native_collections(&mut body);
        webs::split_webs(&mut body);
        module.body = body.into();
    }
    let fresh = Fresh {
        import_roots: import_roots(&module.body),
        ..Fresh::default()
    };
    {
        let mut body: Vec<Stmt> = std::mem::take(&mut module.body).into_iter().collect();
        ann_attrs::split_attribute_annotations(&mut body, &fresh);
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
    stmts::expand_body(&mut body, &fresh);
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
