//! Spanned rejection of a call to an enum class (#921, #944).
//!
//! An enum class is constructor-less by design (`lower_enum_class`
//! early-returns before `class::init::ensure_init`, D-225): its members are
//! compile-time singletons, and CPython's `EnumType.__call__` value lookup
//! (`Color(1)`) is not implemented. #921 added a span-less guard in
//! `pycc_types::class::resolve_instantiation`, which renders the `C0001` at
//! `1:1` because `HirExpr` carries no spans. The one place a diagnostic can
//! still point at the call expression without threading spans through
//! `pycc_types` (#877) is an AST-level scan, so `lower_module` runs this
//! module once per top-level item, right after that item is lowered (`Ok`
//! or `Err` alike), and appends its diagnostics after the item's own so the
//! collection order stays the loop order D-217 rule 3 pins (see the D-233
//! amendment of D-219). `resolve_instantiation` keeps its guard behind this
//! scan (defense in depth), so the two "no `__init__` in the MRO" panics
//! describe an invariant that holds; the two rejections share
//! [`enum_class_call_message`] so they render identically.
//!
//! # Scope-local bindings
//!
//! The scan keys on a bare callee *name*, so it has to know when that name
//! is not the enum class at all: `def f(Color: int) -> None: Color()` is a
//! call on an `int` (`T0021` from `pycc_types`), and a bare-name scan would
//! turn that accurate diagnostic into a misleading enum-call `C0001`. The
//! walk therefore keeps a stack of *frames* -- the set of names bound
//! directly in one scope -- and reports a call only when no frame binds
//! the callee. There is one frame for the module body (computed once by
//! `lower_module` through [`module_bindings`]), one pushed around each
//! `def` (its parameter names plus the names its body binds), and one
//! pushed around each `lambda` (its parameter names plus the names its body
//! binds). A frame records every
//! `Expr::Name` in `Store` context (assignment, augmented and annotated
//! assignment, `for`/`with`/walrus targets, tuple and starred targets,
//! comprehension targets), every `except ... as name`, and every `match`
//! capture (`case Color:`, `case [*Color]:`, `case {**Color}:`) -- those
//! last two groups are `Identifier`s rather than `Expr::Name`s, so the
//! `Store` rule alone would miss them. It records nothing else: not the
//! names of nested `def`/`class` statements and not `import` aliases (limit
//! (ii) below says why neither can matter on an item that lowers), and it
//! does not descend into a nested `def`, `class`, or `lambda`, so
//! `def g(): Color = 1` never suppresses a module-level `Color()`.
//!
//! # `TYPE_CHECKING` guards
//!
//! `lower_stmt` constant-folds an `if TYPE_CHECKING:` / `elif
//! TYPE_CHECKING:` body away as dead code (#790, D-223: a capability gap
//! inside such a body is deliberately not an error), and the scan walks the
//! original AST, so it has to fold the same bodies or it would reject a
//! module the lowering accepts (`if TYPE_CHECKING: Color(1)`, caught in
//! review of PR #971). Both walkers route every `if` through
//! `walk_if_as_lowered`, which skips a guarded body -- calls *and*
//! bindings, so a dead `Color = 1` does not shadow either -- using the very
//! `is_type_checking_guard` predicate the fold uses, against the same
//! import bindings the fold saw for that item. The live clauses around a
//! guard (a non-guard `if`, an `elif`, an `else`) are scanned normally.
//!
//! # Limits
//!
//! All of these are stated here rather than discovered later:
//!
//! - (i) **For an item that lowers, over-suppression is the only failure
//!   mode, never a false report.** A scope that binds the name *anywhere*
//!   suppresses the scan for the whole scope -- Python's own rule for a
//!   function; for the module frame it also hides a `Color()` that precedes
//!   a later module-level `Color = 1`, and a comprehension target
//!   `[Color for Color in range(3)]` suppresses a sibling `Color(1)` in the
//!   same `def`. A value-less module-level annotation (`Color: int`) is
//!   not a binding: it declares metadata and leaves the class bound, so
//!   the call after it is still attributed (inside a function the same
//!   statement makes the name local and does suppress). Every suppressed
//!   call still fails in `pycc_types` (the
//!   span-less guard at `1:1`, or `T0021`). The one false-kind report is
//!   confined to a **class body**, which gets no frame: `class K:` with
//!   `Color = 1` and `X = Color()` reports the class-attribute `C0001` for
//!   `X = Color()` *and* an enum-call `C0001` at `Color()`, although Python
//!   resolves that name to the class-body `int`. Every class-body-level
//!   call already sits in a failing position (class attribute, base, or
//!   decorator), so this is only ever a second diagnostic on an item that
//!   fails anyway; a class frame was rejected because method bodies would
//!   inherit it and `class K: Color = 1` plus `def m(self): Color()` --
//!   correctly reported -- would be suppressed.
//! - (ii) `def`/`class` names and import aliases are not bindings in any
//!   frame, and that loses nothing on an item that lowers: at module level
//!   a `def Color`, a non-enum `class Color`, a `type Color = ...`, or an
//!   `import ... as Color` next to `class Color(Enum)` is a collision
//!   `C0001` in either source order, which poisons `Color` and so filters
//!   the scan; a `Color = 1` after `from colors import Color` is the
//!   "already defined by" `C0001`; and inside a function a nested
//!   `def`/`class`/`import` is itself a failing item, so the only effect of
//!   not modelling it is a possible second diagnostic on an item that
//!   already fails -- the same residual class as the class body. An
//!   imported enum (the `state.class_defs` half of `lower_module`'s name
//!   set) stays scannable.
//! - (iii) A walrus in a decorator, type parameter, default value,
//!   annotation, return annotation, or class base binds in the enclosing
//!   frame (Python's rule: those expressions are evaluated at definition
//!   time in the enclosing scope), so a later `Color()` in that scope is
//!   left to the type checker. Those expressions are additionally walked
//!   under the `def`'s own frame (the frame is pushed around ruff's whole
//!   `FunctionDef` walk), a harmless over-suppression whose only
//!   observable consequence is a missed *second* diagnostic on an item
//!   that already fails: a decorator or a default expression is
//!   `C0001 ... not supported yet` on its own.
//! - (iv) **Poison is order-dependent.** A `def` that calls `Color(1)` and
//!   *precedes* a failing `class Color(Enum): pass` is scanned before the
//!   class item fails and poisons `Color` (`poisoned` is filled only in the
//!   `Err` arm of `lower_module`'s loop), so that program yields two
//!   diagnostics -- the enum `C0001` at `Color(1)` first, then the
//!   class-body `C0001`. The extra report is true (the call *is* an enum
//!   call), so this is an accepted, pinned limit; the reverse order stays
//!   suppressed as a D-219 cascade. Likewise an enum imported by a
//!   statement *after* a `def` that calls it is unknown when that `def` is
//!   scanned and falls through to `pycc_types`' span-less guard.
//! - (v) `global`/`nonlocal` are not consulted (pycc does not lower them),
//!   and PEP 695 type parameters (`def f[Color](x: Color)`,
//!   `class K[Color]`) are not bindings -- they are `Identifier`s, not
//!   `Store` names. Both shapes lower and already reported the enum-call
//!   guard at `1:1`, so reporting at the call is the same diagnostic kind.
//! - (vi) **The module frame sees only the imports known before the
//!   loop.** `module_bindings` runs once, before `lower_module` lowers the
//!   module's own `import` statements, so a module-level guard spelled
//!   through an alias (`import typing as t` then `if t.TYPE_CHECKING:`)
//!   is not recognized there and a dead `Color = 1` inside it still lands
//!   in the module frame -- over-suppression only (the call fails in
//!   `pycc_types`). The bare `TYPE_CHECKING` and `typing.TYPE_CHECKING`
//!   spellings need no import binding and fold in the module frame too;
//!   inside a `def` the scan runs with the item's own import view, so the
//!   aliased spelling folds there.
//! - (vii) **A name bound by more than one module-level `def`, `class`,
//!   `import`, or `type` statement is never scanned** (an identical
//!   repeated import, `from colors import Color` twice, binds the same
//!   definition twice and is not a rebinding; one enum imported under one
//!   local name through two module paths, `from colors import Color`
//!   beside `from palette import Color` where `palette` re-exports it, is
//!   indistinguishable here -- HIR records no defining-module provenance
//!   for a re-exported class -- so that name stays suppressed and the
//!   call is reported by the span-less guard at `1:1`; a `from __future__`
//!   import binds nothing and never counts, so an enum that shares a
//!   feature name, `class annotations(Enum)`, stays scannable). `def Color()`,
//!   an ordinary `class Color`, `from m import Color`, or
//!   `type Color = int` beside `class Color(Enum)` is a collision that the
//!   class item reports itself (`... collides with a function / an import /
//!   a type alias of the same name ...`, `... defined more than once ...`),
//!   whichever comes second. A module-level `Color()` between the two
//!   resolves to the earlier binding, so scanning it against the syntactic
//!   pre-collection would report a *false-kind* `C0001` ahead of the real
//!   one (PR #971 review, two rounds). `lower_module` therefore drops every
//!   name in `module_rebound_names` from the scan's name set for the whole
//!   module, in either order: the collision diagnostic owns such a
//!   program, and a `def`-body call to the rebound name falls through to
//!   `pycc_types`' span-less guard as before #944. A plain assignment
//!   rebinding the name is the module frame's case already.
//! - A call with a keyword argument (`Color(value=1)`) is skipped: it
//!   reports exactly one `C0001 keyword call arguments are not supported
//!   yet` at that call -- from `pycc_types` since Part 8 of #1371, which
//!   refuses a `HirExpr::KeywordCall` of anything but a CPython object --
//!   so without the skip that program would carry two `C0001`s at one
//!   span. The skip is *not* extended to a starred argument (`Color(*xs)`):
//!   the starred `C0001` sits at `*xs`, a different span, and the call is a
//!   genuine enum call.

use crate::ImportBinding;
use crate::stmt::is_type_checking_guard;
use pycc_ast::visitor::{self, Visitor};
use pycc_ast::{ExceptHandler, Expr, ExprContext, Pattern, Stmt, StmtClassDef, StmtIf};
use pycc_diag::Diagnostic;

/// Walks an `if` statement the way `lower_stmt` lowers it (#790): a
/// `TYPE_CHECKING`-guarded body -- the leading `if` or any `elif` clause,
/// recognized by the very same `is_type_checking_guard` the fold uses,
/// against the same `imports` -- is dead code that never reaches HIR, so
/// neither the calls nor the bindings inside it exist for the scan; the
/// guard's own test expression is folded to `False` and is skipped too.
/// Every other clause (a live `if`, `elif`, or `else`) is walked normally.
/// Both walkers below route `Stmt::If` through here, because ruff's
/// `walk_stmt` visits an `if` body and every clause unconditionally.
fn walk_if_as_lowered<'a, V: Visitor<'a>>(
    visitor: &mut V,
    if_stmt: &'a StmtIf,
    imports: &[ImportBinding],
) {
    if !is_type_checking_guard(&if_stmt.test, imports) {
        visitor.visit_expr(&if_stmt.test);
        visitor.visit_body(&if_stmt.body);
    }
    for clause in &if_stmt.elif_else_clauses {
        if let Some(test) = &clause.test
            && is_type_checking_guard(test, imports)
        {
            continue;
        }
        visitor::walk_elif_else_clause(visitor, clause);
    }
}

/// The one `C0001` message for every positional-argument call shape on an
/// enum class (`Color()`, `Color(1)`, `Color(1, 2)`). Shared with
/// `pycc_types::class::resolve_instantiation` so the spanned and the
/// span-less rejection render identically (#942's wording, unchanged by
/// #944). The "not supported yet" clause attaches only to the by-value
/// lookup, which a later slice can implement; `Color()` is a CPython error
/// too and no slice will accept it (`docs/DIAGNOSTICS.md`'s `C0001` is a
/// versioned capability code).
pub fn enum_class_call_message(class_name: &str) -> String {
    format!(
        "cannot call enum class `{class_name}` -- enum members are accessed \
         by name (`{class_name}.MEMBER`); looking a member up by value \
         (`{class_name}(1)`) is not supported yet, and a zero-argument call \
         (`{class_name}()`) is a `TypeError` in CPython as well"
    )
}

/// The names of every top-level class in `body` whose header is exactly one
/// bare-name enum marker base (`class Color(Enum):`, `class S(StrEnum):`),
/// the same predicate `lower_class` uses to route a class to
/// `lower_enum_class`. Computed syntactically *before* the top-level loop so
/// a call inside a `def` that precedes the class definition in source is
/// still found. A class whose base is not a bare name
/// (`class Color(enum.Enum):`) is not an enum class to `lower_class` either
/// and is left to its own `C0001`.
pub(crate) fn syntactic_enum_class_names(body: &[Stmt]) -> Vec<String> {
    body.iter()
        .filter_map(|stmt| match stmt {
            Stmt::ClassDef(def) if has_single_enum_marker_base(def) => Some(def.name.to_string()),
            _ => None,
        })
        .collect()
}

/// Every name that two or more module-level `def`, `class`, `import`, or
/// `type` statements bind (limit (vii)): `lower_module` drops these from
/// the enum-call name set, because a module that binds one name twice
/// that way is reported by the collision diagnostic, never by the scan.
/// An `import a.b` binds `a`; a `from m import *` binds nothing nameable
/// here and is skipped. An import that repeats an earlier import of the
/// same definition under the same local name (`from colors import Color`
/// twice) is not a rebinding: `lower_module` accepts it as binding the
/// same class twice, so the scan keeps the name.
pub(crate) fn module_rebound_names(body: &[Stmt]) -> Vec<String> {
    // (local name, imported definition) -- `None` for a `def`, `class`, or
    // `type` binding, which always conflicts with any earlier binding.
    let mut bound: Vec<(&str, Option<String>)> = Vec::new();
    for stmt in body {
        match stmt {
            Stmt::FunctionDef(def) => bound.push((def.name.as_str(), None)),
            Stmt::ClassDef(def) => bound.push((def.name.as_str(), None)),
            // Same `.expect` as `lower_type_alias_stmt` and
            // `poisonable_names`: ruff unconditionally parses a `type`
            // statement's name as `Expr::Name`.
            Stmt::TypeAlias(alias) => bound.push((
                alias
                    .name
                    .as_name_expr()
                    .expect("ruff always parses a `type` statement's name as Expr::Name")
                    .id
                    .as_str(),
                None,
            )),
            // `str::split` always yields at least one segment.
            Stmt::Import(import) => bound.extend(import.names.iter().map(|alias| {
                let local = alias.asname.as_ref().map_or_else(
                    || {
                        alias
                            .name
                            .split('.')
                            .next()
                            .expect("`str::split` yields at least one segment")
                    },
                    |asname| asname.as_str(),
                );
                (local, Some(alias.name.to_string()))
            })),
            // `lower_future_import` binds nothing, so a feature name that an
            // enum class shares (`from __future__ import annotations` beside
            // `class annotations(Enum)`) is no rebinding (PR #971 review).
            Stmt::ImportFrom(import) if !crate::import::is_future_import(import) => bound.extend(
                import
                    .names
                    .iter()
                    .filter(|alias| alias.name.as_str() != "*")
                    .map(|alias| {
                        let local = alias
                            .asname
                            .as_ref()
                            .map_or(alias.name.as_str(), |a| a.as_str());
                        let module = import.module.as_ref().map_or("", |m| m.as_str());
                        let source = format!(
                            "{}{module}.{}",
                            ".".repeat(import.level as usize),
                            alias.name
                        );
                        (local, Some(source))
                    }),
            ),
            _ => {}
        }
    }
    let mut rebound: Vec<String> = Vec::new();
    for (index, (name, source)) in bound.iter().enumerate() {
        let conflicts = bound[..index].iter().any(|(earlier, earlier_source)| {
            earlier == name && (source.is_none() || earlier_source != source)
        });
        if conflicts && !rebound.iter().any(|seen| seen == name) {
            rebound.push((*name).to_string());
        }
    }
    rebound
}

fn has_single_enum_marker_base(def: &StmtClassDef) -> bool {
    let Some(arguments) = def.arguments.as_deref() else {
        return false;
    };
    let [base] = &*arguments.args else {
        return false;
    };
    let Expr::Name(name) = base else {
        return false;
    };
    arguments.keywords.is_empty() && crate::is_enum_base_name(name.id.as_str())
}

/// The module frame: every name the module body binds directly (see the
/// module doc's binding rule). Because the binder records no `def`, `class`,
/// or `import` name, `class Color(Enum)` itself never puts `Color` here --
/// only a plain module-level `Color = 1`, a `for`/`with`/walrus target, an
/// except-handler name, or a `match` capture does.
pub(crate) fn module_bindings(body: &[Stmt], imports: &[ImportBinding]) -> Vec<String> {
    scope_bindings(body, imports, true)
}

/// The names bound directly by the statements of one scope, without
/// descending into a nested `def`, `class`, or `lambda` body (each of
/// those is its own scope and gets its own frame, or none). A nested
/// definition's decorators, type parameters, parameters, return
/// annotation, and class bases are walked, though: they are evaluated in
/// this scope (limit (iii)).
///
/// `module_scope` distinguishes a value-less annotation: at module level
/// `Color: int` declares metadata and leaves the existing class binding in
/// place (ruff still gives the target `Store` context), so it must not
/// suppress the scan; inside a function the same statement makes `Color`
/// a local, which the frame models as before (PR #971 review).
fn scope_bindings(body: &[Stmt], imports: &[ImportBinding], module_scope: bool) -> Vec<String> {
    let mut binder = Binder {
        imports,
        names: Vec::new(),
        module_scope,
    };
    binder.visit_body(body);
    binder.names
}

/// The names a `lambda` body binds (a walrus at any depth, short of a
/// nested `lambda`, which is its own scope again); together with the
/// parameters they make the lambda's frame (D-233 decision 5, PR #971
/// review).
fn lambda_body_bindings(body: &Expr, imports: &[ImportBinding]) -> Vec<String> {
    let mut binder = Binder {
        imports,
        names: Vec::new(),
        module_scope: false,
    };
    binder.visit_expr(body);
    binder.names
}

/// The [`Visitor`] behind [`scope_bindings`] and [`lambda_body_bindings`].
struct Binder<'i> {
    imports: &'i [ImportBinding],
    names: Vec<String>,
    module_scope: bool,
}
impl<'a> Visitor<'a> for Binder<'_> {
    fn visit_stmt(&mut self, stmt: &'a Stmt) {
        match stmt {
            // A value-less module annotation binds nothing at runtime;
            // its annotation expression is still walked.
            Stmt::AnnAssign(ann) if self.module_scope && ann.value.is_none() => {
                self.visit_annotation(&ann.annotation);
            }
            // A nested `def`/`class` is neither a binding this frame
            // models (limit (ii)) nor a scope it descends into -- but
            // its definition-time expressions (decorators, type
            // parameters, defaults, annotations, bases) are evaluated
            // in *this* scope, so a walrus there binds here.
            Stmt::FunctionDef(def) => {
                for decorator in &def.decorator_list {
                    self.visit_decorator(decorator);
                }
                if let Some(type_params) = &def.type_params {
                    self.visit_type_params(type_params);
                }
                self.visit_parameters(&def.parameters);
                if let Some(returns) = &def.returns {
                    self.visit_annotation(returns);
                }
            }
            Stmt::ClassDef(class) => {
                for decorator in &class.decorator_list {
                    self.visit_decorator(decorator);
                }
                if let Some(type_params) = &class.type_params {
                    self.visit_type_params(type_params);
                }
                if let Some(arguments) = &class.arguments {
                    self.visit_arguments(arguments);
                }
            }
            // A `TYPE_CHECKING`-guarded body binds nothing at runtime.
            Stmt::If(if_stmt) => walk_if_as_lowered(self, if_stmt, self.imports),
            _ => visitor::walk_stmt(self, stmt),
        }
    }
    fn visit_expr(&mut self, expr: &'a Expr) {
        match expr {
            Expr::Name(name) if matches!(name.ctx, ExprContext::Store) => {
                self.names.push(name.id.to_string());
            }
            // A lambda is its own scope: a walrus inside it binds there.
            Expr::Lambda(_) => return,
            _ => {}
        }
        visitor::walk_expr(self, expr);
    }
    fn visit_except_handler(&mut self, handler: &'a ExceptHandler) {
        let ExceptHandler::ExceptHandler(except) = handler;
        if let Some(name) = &except.name {
            self.names.push(name.to_string());
        }
        visitor::walk_except_handler(self, handler);
    }
    fn visit_pattern(&mut self, pattern: &'a Pattern) {
        let captured = match pattern {
            Pattern::MatchAs(p) => p.name.as_ref(),
            Pattern::MatchStar(p) => p.name.as_ref(),
            Pattern::MatchMapping(p) => p.rest.as_ref(),
            _ => None,
        };
        if let Some(name) = captured {
            self.names.push(name.to_string());
        }
        visitor::walk_pattern(self, pattern);
    }
}

/// Every call in `stmt` (at any depth -- a nested `print(Color(1))`, a call
/// inside a `def` body or a comprehension) whose callee is a bare name in
/// `enum_class_names`, whose argument list carries no keyword, and whose
/// name no enclosing frame binds (`module_frame` is the bottom of the
/// stack), as one `C0001` per call at the call expression's own span, in
/// walk order.
///
/// The walk is [`pycc_ast::visitor::Visitor`], for the reason
/// `exception::module_references_builtin_exception_name` gives: a
/// hand-rolled match misses positions silently. `visit_stmt` pushes a frame
/// around each `def` (ruff's `walk_stmt` descends into a class body, so a
/// method gets its frame the same way and a `class` itself gets none --
/// limit (i)); `visit_expr` pushes one around each `lambda`.
///
/// `lower_module` calls this only with a non-empty `enum_class_names`: the
/// walk is the whole item plus, again, each `def` body for its frame, and a
/// module with no enum class in sight must not pay it (`pycc check`'s
/// frontend bench measured ~7% for the unconditional walk).
///
/// `enum_class_names` is assembled by `lower_module` from the syntactic
/// pre-collection plus every `HirClassDef` with `is_enum` known at scan
/// time (an enum pulled in by a project import), minus the names currently
/// poisoned (D-219). The poison filter inherits D-219's rule that *any*
/// failing `class` statement poisons its name: an enum class redefined
/// under the same name (`class Color(Enum): ...` then `class Color: pass`)
/// is reported once, for the duplicate definition, and a later `Color()`
/// is suppressed with it even though the first `Color` is a real enum. The
/// module still fails on the duplicate, so nothing is accepted; the
/// enum-call diagnostic surfaces once the duplicate is removed.
pub(crate) fn reject_enum_class_calls(
    stmt: &Stmt,
    module_frame: &[String],
    enum_class_names: &[&str],
    imports: &[ImportBinding],
) -> Vec<Diagnostic> {
    struct CallScan<'n> {
        enum_class_names: &'n [&'n str],
        module_frame: &'n [String],
        imports: &'n [ImportBinding],
        frames: Vec<Vec<String>>,
        diagnostics: Vec<Diagnostic>,
    }
    impl CallScan<'_> {
        fn is_bound(&self, name: &str) -> bool {
            self.module_frame.iter().any(|n| n == name)
                || self.frames.iter().flatten().any(|n| n == name)
        }
    }
    impl<'a> Visitor<'a> for CallScan<'_> {
        fn visit_stmt(&mut self, stmt: &'a Stmt) {
            match stmt {
                Stmt::FunctionDef(def) => {
                    let mut frame: Vec<String> = def
                        .parameters
                        .iter()
                        .map(|parameter| parameter.name().to_string())
                        .collect();
                    frame.extend(scope_bindings(&def.body, self.imports, false));
                    self.frames.push(frame);
                    visitor::walk_stmt(self, stmt);
                    self.frames.pop();
                }
                // A `TYPE_CHECKING`-guarded body is dead code (#790): a call
                // inside it is never lowered, so it is never an enum call.
                Stmt::If(if_stmt) => walk_if_as_lowered(self, if_stmt, self.imports),
                _ => visitor::walk_stmt(self, stmt),
            }
        }
        fn visit_expr(&mut self, expr: &'a Expr) {
            match expr {
                Expr::Call(call)
                    if let Expr::Name(callee) = call.func.as_ref()
                        && call.arguments.keywords.is_empty()
                        && self.enum_class_names.contains(&callee.id.as_str())
                        && !self.is_bound(callee.id.as_str()) =>
                {
                    self.diagnostics.push(crate::unsupported(
                        enum_class_call_message(callee.id.as_str()),
                        call.range,
                    ));
                }
                Expr::Lambda(lambda) => {
                    // `lambda: Color()` has no parameters at all; the frame
                    // is then the body's own walrus bindings (possibly
                    // none), and the walk below still sees the body.
                    let mut frame: Vec<String> = lambda
                        .parameters
                        .as_deref()
                        .map(|parameters| {
                            parameters
                                .iter()
                                .map(|parameter| parameter.name().to_string())
                                .collect()
                        })
                        .unwrap_or_default();
                    frame.extend(lambda_body_bindings(&lambda.body, self.imports));
                    self.frames.push(frame);
                    visitor::walk_expr(self, expr);
                    self.frames.pop();
                    return;
                }
                _ => {}
            }
            // Keep walking: an argument may nest another call.
            visitor::walk_expr(self, expr);
        }
    }
    let mut scan = CallScan {
        enum_class_names,
        module_frame,
        imports,
        frames: Vec::new(),
        diagnostics: Vec::new(),
    };
    scan.visit_stmt(stmt);
    scan.diagnostics
}

#[cfg(test)]
#[path = "enum_call_tests.rs"]
mod tests;
