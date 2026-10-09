//! PEP 562 module hooks at the `--ext` boundary (#1467).
//!
//! CPython consults a module's own `__getattr__` when an attribute read on
//! the module finds nothing in its `__dict__`, and builtin `dir(module)`
//! calls the module's own `__dir__`. Both are looked up in the module's
//! dict, so the hook is honoured exactly when the dict holds it. D-244
//! rule 1's export set borrows D-038's public-name predicate, which refuses
//! every dunder, so before #1467 a compiled `def __getattr__` never reached
//! the extension's dict and the host silently ignored it.
//!
//! An export named here is published as an ordinary `METH_FASTCALL`
//! module function, but only **after** the module body has run: the
//! wrapper calls through a `fnptr_` slot the body fills, and importlib's
//! `_init_module_attrs` reads attributes of the half-initialised module
//! before `Py_mod_exec`, so a hook installed at module creation would be
//! called through a null slot. `pycc_ext_exec_module` adds the
//! `pycc_ext_module_hooks[]` table once the body has succeeded. Every other
//! export is bound as its definition executes (#1199), as CPython binds a
//! name; a hook is not, so a host read from inside the body does not see
//! it yet (D-244's #1467 amendment records that residual).
//!
//! Which hooks are published is read from the **entry** module's own
//! source, because the export set is collected from D-222's linked program,
//! where a helper module's `def __getattr__` is an indistinguishable
//! `HirItem::Function`. CPython never applies a helper's hook to the entry
//! module, so only a `def` at the entry module's top level counts. Any
//! other module-scope binding of a hook name in the entry module -- a `def`
//! inside a module-level `if`/`try`/loop, an import, an assignment, a
//! `class` statement, an `except ... as` name -- would put a value in
//! CPython's dict that this boundary cannot publish, so it is refused with
//! a located `C0001` rather than ignored.

use std::collections::BTreeSet;

use pycc_ast::visitor::{self, Visitor};
use pycc_ast::{
    ExceptHandler, ExceptHandlerExceptHandler, Expr, ExprContext, ModModule, Pattern,
    PatternMatchAs, PatternMatchMapping, PatternMatchStar, Stmt,
};
use pycc_diag::{Diagnostic, Span};

/// The module-level names CPython calls implicitly (PEP 562).
pub(crate) const MODULE_HOOKS: [&str; 2] = ["__getattr__", "__dir__"];

/// Whether `name` is one of [`MODULE_HOOKS`].
pub(crate) fn is_module_hook(name: &str) -> bool {
    MODULE_HOOKS.contains(&name)
}

/// The hooks the entry module defines with a top-level `def`.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct EntryHooks {
    defined: BTreeSet<String>,
}

impl EntryHooks {
    /// The hooks of `source`, or none when it does not parse. The frontend
    /// has already parsed the same text successfully, so the empty set is a
    /// fail-closed default rather than a reachable outcome, exactly as for
    /// [`super::SourceSignatures::from_source`].
    pub(crate) fn from_source(source: &str) -> Result<Self, Vec<Diagnostic>> {
        pycc_parser::parse(source)
            .map_or_else(|_| Ok(Self::default()), |module| Self::scan(&module))
    }

    /// Scans the entry module: a top-level `def` of a hook name defines it,
    /// a hook `def` nested in a module-level block is refused, and so is
    /// every other binding of a hook name -- an import alias, a store or
    /// `del` target, a `class` statement, an `except ... as` name, a `match`
    /// capture -- at the top level, anywhere inside a top-level
    /// compound statement or in a `def`/`class` header, but not inside a
    /// function or class body, whose bindings are not module
    /// attributes; each refusal is one located `C0001`: CPython would call (or lose) a
    /// hook the boundary cannot publish. The scan is deliberately
    /// over-inclusive: it does not evaluate a guard, so a binding CPython
    /// never executes -- under `if TYPE_CHECKING:`, say -- is refused too,
    /// and a comprehension target or a walrus in a `lambda` body, which bind
    /// in their own scope, are refused as well. That costs a spurious refusal of a rare
    /// spelling, never a silently ignored hook. A `global` rebinding from
    /// inside a function body is not modelled.
    pub(crate) fn scan(module: &ModModule) -> Result<Self, Vec<Diagnostic>> {
        let defined = module
            .body
            .iter()
            .filter_map(|stmt| match stmt {
                Stmt::FunctionDef(def) if is_module_hook(def.name.as_str()) => {
                    Some(def.name.to_string())
                }
                _ => None,
            })
            .collect();
        let mut bindings = HookBindings::default();
        bindings.visit_body(&module.body);
        if bindings.refusals.is_empty() {
            Ok(Self { defined })
        } else {
            Err(bindings.refusals)
        }
    }

    /// The hook names to publish.
    pub(crate) fn defined(&self) -> &BTreeSet<String> {
        &self.defined
    }
}

/// Collects a refusal for every binding of a hook name at module scope
/// other than a top-level `def`.
#[derive(Default)]
struct HookBindings {
    refusals: Vec<Diagnostic>,
    /// How many module-level compound statements enclose the statement
    /// being visited; `0` is the module's own top level.
    depth: u32,
}

impl HookBindings {
    fn check(&mut self, bound: &str, range: impl Into<std::ops::Range<u32>>, how: &str) {
        if is_module_hook(bound) {
            let range = range.into();
            self.refusals.push(binding_refusal(
                bound,
                how,
                Span::new(range.start, range.end),
            ));
        }
    }
}

impl<'a> Visitor<'a> for HookBindings {
    fn visit_stmt(&mut self, stmt: &'a Stmt) {
        match stmt {
            // The body is a local scope: nothing bound in it is a module
            // attribute, and a top-level `def` of a hook name is the
            // published form. One nested in a module-level block binds the
            // hook only when its branch runs, which `defined` (top level
            // only) cannot model, so it is refused rather than dropped. The
            // header runs at module scope, so a walrus in a decorator, a
            // default or an annotation binds there.
            Stmt::FunctionDef(def) => {
                if self.depth > 0 {
                    self.check(
                        def.name.as_str(),
                        def.name.range,
                        "defined by a `def` inside a module-level block",
                    );
                }
                for decorator in &def.decorator_list {
                    self.visit_decorator(decorator);
                }
                self.visit_parameters(&def.parameters);
                if let Some(returns) = &def.returns {
                    self.visit_annotation(returns);
                }
                return;
            }
            Stmt::ClassDef(class) => {
                self.check(
                    class.name.as_str(),
                    class.name.range,
                    "bound by a class statement",
                );
                for decorator in &class.decorator_list {
                    self.visit_decorator(decorator);
                }
                if let Some(arguments) = &class.arguments {
                    self.visit_arguments(arguments);
                }
                return;
            }
            // `import a.b` binds `a`; `import a.b as c` binds `c`.
            Stmt::Import(import) => {
                for alias in &import.names {
                    let bound = alias.asname.as_ref().map_or_else(
                        || alias.name.as_str().split('.').next().unwrap_or_default(),
                        |asname| asname.as_str(),
                    );
                    self.check(bound, alias.range, "bound by an import");
                }
            }
            Stmt::ImportFrom(import) => {
                for alias in &import.names {
                    let bound = alias.asname.as_ref().unwrap_or(&alias.name).as_str();
                    self.check(bound, alias.range, "bound by an import");
                }
            }
            _ => {}
        }
        self.depth += 1;
        visitor::walk_stmt(self, stmt);
        self.depth -= 1;
    }

    /// Every store or `del` target: an assignment, an augmented or
    /// annotated one, a `for` or `with` target, a walrus, a `type` alias.
    fn visit_expr(&mut self, expr: &'a Expr) {
        if let Expr::Name(name) = expr {
            match name.ctx {
                ExprContext::Store => {
                    self.check(name.id.as_str(), name.range, "bound by an assignment");
                }
                ExprContext::Del => {
                    self.check(name.id.as_str(), name.range, "deleted by a `del` statement");
                }
                _ => {}
            }
        }
        visitor::walk_expr(self, expr);
    }

    /// `except E as name` (and `except*`) binds the name, then deletes it
    /// when the handler ends.
    fn visit_except_handler(&mut self, handler: &'a ExceptHandler) {
        let ExceptHandler::ExceptHandler(ExceptHandlerExceptHandler { name, .. }) = handler;
        if let Some(name) = name {
            self.check(name.as_str(), name.range, "bound by an except clause");
        }
        visitor::walk_except_handler(self, handler);
    }

    /// A capture in a `match` pattern binds its name as an assignment does.
    fn visit_pattern(&mut self, pattern: &'a Pattern) {
        let captured = match pattern {
            Pattern::MatchAs(PatternMatchAs { name, .. })
            | Pattern::MatchStar(PatternMatchStar { name, .. }) => name.as_ref(),
            Pattern::MatchMapping(PatternMatchMapping { rest, .. }) => rest.as_ref(),
            _ => None,
        };
        if let Some(name) = captured {
            self.check(name.as_str(), name.range, "bound by a match pattern");
        }
        visitor::walk_pattern(self, pattern);
    }
}

/// The located `C0001` for a module-scope binding of the hook `hook` that
/// is not a top-level `def`; `how` names the binding form.
fn binding_refusal(hook: &str, how: &str, span: Span) -> Diagnostic {
    Diagnostic::error(
        "C0001",
        format!(
            "--ext cannot publish a module `{hook}` {how} -- CPython calls a \
             module's own `{hook}` implicitly (PEP 562), and pycc publishes only a \
             `def {hook}` written at the top level of the entry module; define it there, or \
             build without --ext (#1467)"
        ),
        span,
    )
}

#[cfg(test)]
#[path = "module_hooks_tests.rs"]
mod tests;
