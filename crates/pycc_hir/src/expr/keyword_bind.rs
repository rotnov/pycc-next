//! Binding keyword call arguments to parameters by name (Part 1 of #884,
//! issue #1125).
//!
//! `HirExpr::Call` is purely positional: it carries `args: Vec<HirExpr>` and
//! nothing that could name a parameter. Rather than widen that node — and
//! every consumer of it in `pycc_types`, `pycc_mir` and `pycc_codegen` — this
//! module resolves `f(b=2, a=1)` to the positional vector `f(1, 2)` *during
//! lowering*, where the call site's source spans still exist. Everything
//! downstream keeps seeing a complete positional argument list and needs no
//! change at all.
//!
//! Two pieces make that possible:
//!
//! * [`SignatureTable`], collected once per module from its top-level `def`s
//!   before any item is lowered, so a call written above its own `def` binds
//!   just as well as one written below it; and
//! * [`bind_keyword_arguments`], which implements the subset of CPython's
//!   call-binding rules this part supports.
//!
//! Part 2 of #884 (issue #1189) reuses the same machinery for **default
//! parameter values** on a module-level `def`. A `def`'s admitted literal
//! defaults are recorded alongside its parameter names, and a call that
//! omits a defaulted trailing parameter has the default's already-lowered
//! expression spliced into the same positional vector. The filled node is
//! byte-identical to the same literal written explicitly at the call site,
//! which is the design's load-bearing invariant.
//!
//! The supported subset is deliberately narrow, and a call outside it is
//! rejected exactly as before Part 1 (`C0001`, "keyword call arguments
//! are not supported yet") rather than being bound wrongly: only a call whose
//! callee is a bare name naming a module-level `def` whose parameters are all
//! positional, whose defaults (if any) are all in the admitted literal
//! subset, and whose name is bound nowhere else in module scope, is
//! bindable, provided binding would not observably reorder its argument
//! values (issue #1204, the `eval_order` submodule). A method call, a `super().m()` call, a
//! container or stdlib-intrinsic call, a class instantiation and a `**kwargs`
//! unpacking all keep the old rejection — see [`is_bindable_call`].
//!
//! The ways a bindable call can still be wrong -- more positional arguments
//! than the callee has parameters, an unexpected keyword name, a
//! positional-only parameter passed as a keyword, a parameter supplied both
//! positionally and by keyword, and a parameter left unsupplied *that has no
//! default* -- are errors
//! CPython itself raises as `TypeError`, so they are `T0021`, not `C0001`
//! (`docs/DIAGNOSTICS.md`). A parameter left unsupplied that *does* carry a
//! default is the one case that is not an error at all: it is filled.
//! `pycc_hir` already emits `T0021` with a real source span at `import.rs`
//! (D-222); `pycc_types`' positional-arity twin cannot, because HIR carries no
//! spans by the time it runs.

use std::collections::{BTreeSet, HashMap, HashSet};

use pycc_ast::{Expr, ExprCall, Parameters, Stmt, StmtImportFrom};
use pycc_diag::{Diagnostic, Span};

use crate::HirExpr;

mod eval_order;
mod rebound;

/// One module-level `def`'s keyword-bindable signature.
struct Signature {
    /// Parameter names in exactly the order `func::lower_params` builds
    /// them: `posonlyargs` first, then `args`. Index `i` here is index `i`
    /// of the `HirItem::Function::params` vector, and therefore index `i`
    /// of the positional `HirExpr::Call::args` vector this module produces.
    names: Vec<String>,
    /// How many leading [`Signature::names`] entries came from
    /// `posonlyargs`. They fill positionally like any other parameter but
    /// can never be named by a keyword (PEP 570).
    posonly_count: usize,
    /// Parallel to [`Signature::names`]: the already-lowered default value
    /// of each parameter, or `None` when the parameter is required (Part 2
    /// of #884, #1189).
    ///
    /// Python requires every parameter after a defaulted one to be
    /// defaulted too, and the parser enforces it, so this is always a
    /// required prefix followed by a defaulted suffix.
    defaults: Vec<Option<HirExpr>>,
}

/// Every top-level `def` in one module whose parameters this part can bind
/// keyword arguments against, and whose name that `def` alone binds in
/// module scope.
///
/// Absence from the table is not an error: it means "keep the pre-#1125
/// behaviour for this callee", i.e. reject a keyword call against it with
/// the unchanged `C0001` capability message, and — since Part 2 of #884
/// (#1189) — never fill a default for it either.
#[derive(Default)]
pub(crate) struct SignatureTable {
    by_name: HashMap<String, Signature>,
    /// Top-level names bound more than once (`from math import gcd` then
    /// `def gcd`): which binding a call reaches depends on source order.
    rebound: HashSet<String>,
    /// Issue #1188: which of `append`, `pop`, `get` and `add` some class
    /// reachable from this module defines as a method -- this module's own
    /// top-level classes, plus every class in its transitive import closure
    /// (see `ResolvedImports::inherit_container_method_names`). A call to a
    /// name in this set lowers to `HirExpr::ReceiverDispatchedCall`; every
    /// other call lowers exactly as before.
    ///
    /// It lives here because this table is already threaded to every
    /// expression-lowering site of the module.
    container_method_names: BTreeSet<&'static str>,
    /// Issue #1095: the module can hold a CPython object (`Ty::Object`) --
    /// it is compiled into an `ext` artifact, where `Any`, `object` and an
    /// object container annotation all spell one (D-258), its body binds a
    /// foreign import anywhere (D-244 rule 3; decided before the first
    /// statement is lowered since #1482), or (#1425) a direct project
    /// dependency, or a package `__init__` on its path, can hold one (see
    /// `ResolvedImports::inherit_object_receivers`). Then every one of the
    /// four container method names lowers with both readings, so that
    /// `o.append(x)` on an object receiver can take the foreign method-call
    /// reading. Kept apart from `container_method_names`, which carries the
    /// narrower meaning "a class defines this method"; importers inherit the
    /// two separately (#1188, #1425).
    object_receivers: bool,
    /// Non-strict mode (#3): a value's container may turn out to be a CPython
    /// object, so a check that depends on the native container (an
    /// out-of-range `.append()` literal) waits for `pycc_types`.
    non_strict: bool,
    /// Part 2a of #1371: the *class-like* names of the module -- each bound
    /// exactly once in module scope (the same [`rebound::binding_counts`]
    /// rule `by_name` uses), by either a top-level `class` statement or a
    /// top-level `from M import X` whose `M` resolved to a project module.
    /// A call `X[Y](args)` whose base is one of these keeps lowering to
    /// `HirExpr::GenericClassInstantiate` with its located type-argument
    /// diagnostics; with any other base it is a call of a subscript result
    /// (`HirExpr::ExprCall`), which `pycc_types` admits only on a CPython
    /// object (see `expr::subscript_call`).
    ///
    /// A name imported from a project module that is not a class (a
    /// function, or an `object` global) is counted too: the driver's answer
    /// says only that the module resolved. That can only turn a call into a
    /// refused generic instantiation, never into wrong code.
    class_like_names: HashSet<String>,
    /// v0.9: the names bound once in module scope by a top-level `import` or
    /// `from ... import` that does not resolve to a project module: CPython
    /// objects, so an `except` naming one tests the caught exception against
    /// the object at run time (`pycc_ast::object_except_handlers`).
    foreign_import_names: HashSet<String>,
}

impl SignatureTable {
    /// Collects the table from a module's top-level statements.
    ///
    /// Only a name bound exactly once in module scope -- by its one
    /// top-level `def` and nothing else -- is entered. A name bound more
    /// than once (two `def`s, or a `def` plus an assignment, an import, a
    /// `type` alias, a walrus, a `match` capture or any other module-scope
    /// binding, see [`rebound::binding_counts`]) is left out, because the
    /// runtime dispatches such a name in source order while this table is
    /// static and module-wide: no call site can be tied to one of its
    /// signatures. A keyword call to it keeps `C0001` and a short call gets
    /// the ordinary arity error (`docs/TYPE_SYSTEM.md`, "Keyword arguments
    /// and default parameter values on a redefined name").
    ///
    /// `is_project_import` answers whether a top-level `from ... import`
    /// statement resolved to a project module; it feeds
    /// [`Self::is_class_like`] only.
    pub(crate) fn collect(
        body: &[Stmt],
        is_project_import: impl Fn(&StmtImportFrom) -> bool,
    ) -> Self {
        let counts = rebound::binding_counts(body);
        let bound_once = |name: &str| counts.get(name) == Some(&1);
        let mut by_name = HashMap::new();
        let mut class_like_names = HashSet::new();
        let mut foreign_import_names = HashSet::new();
        for stmt in body {
            match stmt {
                Stmt::FunctionDef(def) if bound_once(def.name.as_str()) => {
                    if let Some(signature) = signature_of(&def.parameters) {
                        by_name.insert(def.name.as_str().to_string(), signature);
                    }
                }
                Stmt::ClassDef(def) if bound_once(def.name.as_str()) => {
                    class_like_names.insert(def.name.as_str().to_string());
                }
                Stmt::ImportFrom(import) if is_project_import(import) => {
                    for alias in &import.names {
                        let bound = alias.asname.as_ref().unwrap_or(&alias.name).as_str();
                        if bound_once(bound) {
                            class_like_names.insert(bound.to_string());
                        }
                    }
                }
                Stmt::ImportFrom(import) => {
                    for alias in &import.names {
                        let bound = alias.asname.as_ref().unwrap_or(&alias.name).as_str();
                        if bound_once(bound) && bound != "*" {
                            foreign_import_names.insert(bound.to_string());
                        }
                    }
                }
                _ => {}
            }
        }
        let rebound = counts
            .iter()
            .filter(|(_, count)| **count > 1)
            .map(|(name, _)| name.to_string())
            .collect();
        Self {
            by_name,
            rebound,
            class_like_names,
            foreign_import_names,
            object_receivers: false,
            non_strict: false,
            container_method_names: crate::expr::receiver_dispatch::defined_container_method_names(
                body,
            )
            .into_iter()
            .collect(),
        }
    }

    /// Adds the container method names that the module's dependencies can
    /// reach (issue #1188).
    pub(crate) fn inherit_container_method_names(
        &mut self,
        names: impl IntoIterator<Item = &'static str>,
    ) {
        self.container_method_names.extend(names);
    }

    /// Part 8 of #1371: whether `name` is a module `def` this table can bind
    /// keyword arguments against.
    /// Whether top-level `name` is bound more than once.
    pub(crate) fn is_rebound(&self, name: &str) -> bool {
        self.rebound.contains(name)
    }

    pub(crate) fn binds(&self, name: &str) -> bool {
        self.by_name.contains_key(name)
    }

    /// Part 2a of #1371: whether `name` is one of the module's class-like
    /// names (see the field's own documentation).
    pub(crate) fn is_class_like(&self, name: &str) -> bool {
        self.class_like_names.contains(name)
    }

    /// v0.9: whether `name` is bound once by a module-level import of a
    /// module that is not a project module (see the field's documentation).
    pub(crate) fn is_foreign_import(&self, name: &str) -> bool {
        self.foreign_import_names.contains(name)
    }

    /// Issue #1188: every container method name some class reachable from
    /// this module defines as a method.
    pub(crate) fn container_method_names(&self) -> &BTreeSet<&'static str> {
        &self.container_method_names
    }

    /// Issue #1095: records that the module can hold a CPython object, so
    /// every container method name dispatches on its receiver. Called before
    /// the module's first statement is lowered, so the admission covers the
    /// whole module (#1425, #1482; see the `object_receivers` field).
    pub(crate) fn admit_object_receivers(&mut self) {
        self.object_receivers = true;
    }

    /// Records non-strict mode (see the `non_strict` field).
    pub(crate) fn admit_non_strict(&mut self) {
        self.non_strict = true;
    }

    /// Whether the module is compiled in non-strict mode.
    pub(crate) fn non_strict(&self) -> bool {
        self.non_strict
    }

    /// Issue #1425: whether the module can hold a CPython object (see the
    /// `object_receivers` field). Read after the whole module is lowered, so
    /// it is the module's final state, which importers inherit.
    pub(crate) fn object_receivers(&self) -> bool {
        self.object_receivers
    }

    /// Whether a call to `method` must be lowered with both readings, because
    /// a user class reachable from this module defines a method of that name
    /// (issue #1188), or because the module can hold a CPython object and
    /// `method` is one of the four container names (issue #1095).
    pub(crate) fn dispatches_on_receiver(&self, method: &str) -> bool {
        self.container_method_names.contains(method)
            || (self.object_receivers
                && crate::expr::receiver_dispatch::RECEIVER_DISPATCHED_NAMES.contains(&method))
    }

    fn get(&self, callee: &str) -> Option<&Signature> {
        self.by_name.get(callee)
    }
}

/// Reads one `def`'s parameter list, or `None` when it uses a parameter kind
/// this part does not bind against.
///
/// `*args`, keyword-only parameters and `**kwargs` are all rejected outright
/// by `func::lower_params`, so a module containing such a `def` never
/// compiles either way; leaving those names out of the table keeps the call
/// site's own diagnostic the pre-existing capability message instead of a
/// `T0021` derived from a signature pycc cannot represent. A default value
/// whose expression `func::params::literal_default` does not recognize is
/// excluded on the same terms, so no call site can ever bind against a
/// default this part cannot materialize.
///
/// This function is deliberately **infallible and purely syntactic**. It
/// runs from `SignatureTable::collect` while `ModuleState` is constructed,
/// before the per-item lowering loop that owns cascade suppression and
/// #878's multi-diagnostic collection, so a diagnostic raised here would
/// have no item to attach to and would collapse a module's whole diagnostic
/// set to one error. It also has no `aliases`, no `class_defs` and no
/// `lower_expr` context, so it could not perform a *type* check even if it
/// wanted to: `func::params::check_default` owns every one of those, once
/// per `def`. The consequence is scoped: a default the recognizer accepts
/// but the type rules reject (`def f(x: float = 1)`) *is* in the table and
/// will be spliced at a short call site in the same module — the module is
/// rejected by that `def`'s own diagnostic regardless, so this is a
/// diagnostic-count question, never a wrong-code one.
fn signature_of(parameters: &Parameters) -> Option<Signature> {
    if parameters.vararg.is_some()
        || !parameters.kwonlyargs.is_empty()
        || parameters.kwarg.is_some()
    {
        return None;
    }
    let declared = parameters.posonlyargs.iter().chain(parameters.args.iter());
    let mut defaults = Vec::new();
    for param in declared.clone() {
        match &param.default {
            None => defaults.push(None),
            Some(default) => defaults.push(Some(crate::func::params::literal_default(default)?)),
        }
    }
    debug_assert!(
        defaults
            .iter()
            .skip_while(|d| d.is_none())
            .all(Option::is_some),
        "the parser guarantees a required prefix followed by a defaulted suffix"
    );
    Some(Signature {
        names: declared
            .map(|param| param.parameter.name.as_str().to_string())
            .collect(),
        posonly_count: parameters.posonlyargs.len(),
        defaults,
    })
}

/// Whether a zero-keyword call of `callee` with `positional_len` arguments
/// must be routed through [`bind_keyword_arguments`] to have its trailing
/// defaults filled (Part 2 of #884, #1189).
///
/// Deliberately narrow: every *other* all-positional call keeps taking the
/// unbound path, so the "too many positional arguments" diagnostic for every
/// existing program stays `pycc_types`' span-less `T0021` rather than moving
/// to this module's spanned one — a behaviour change this part does not own.
pub(crate) fn needs_default_fill(
    signatures: &SignatureTable,
    callee: &str,
    positional_len: usize,
) -> bool {
    signatures.get(callee).is_some_and(|signature| {
        positional_len < signature.names.len() && signature.defaults.iter().any(Option::is_some)
    })
}

/// Whether `call`'s keyword arguments can be bound by name.
///
/// This is the single predicate that narrows `expr.rs`'s blanket keyword
/// rejection. It is deliberately evaluated *before* the call-shape arms that
/// follow it there, and it answers `false` for every shape those arms handle,
/// so none of them can silently erase a keyword it never inspects.
///
/// It also answers `false` for a call whose binding would observably change
/// the order its argument values are evaluated in (issue #1204,
/// `docs/TYPE_SYSTEM.md`, "Keyword argument evaluation order"), so such a
/// call keeps the same `C0001` every other unbindable keyword call reports.
pub(crate) fn is_bindable_call(signatures: &SignatureTable, call: &ExprCall) -> bool {
    let Expr::Name(callee) = call.func.as_ref() else {
        return false;
    };
    // `arg: None` is a `**kwargs` unpacking, which has no parameter name to
    // bind and stays rejected.
    call.arguments.keywords.iter().all(|kw| kw.arg.is_some())
        && signatures
            .get(callee.id.as_str())
            .is_some_and(|signature| !eval_order::observably_reorders(signature, call))
}

/// Binds `positional` and `keywords` to `callee`'s parameters, returning the
/// complete positional argument vector `HirExpr::Call` needs.
///
/// `keywords` carries each keyword's name, its own source range, and its
/// already-lowered value. The caller has established that `callee` is in
/// `signatures`, through either [`is_bindable_call`] (a call carrying at
/// least one keyword argument) or [`needs_default_fill`] (a zero-keyword
/// call short of the callee's arity, Part 2 of #884). An unsupplied
/// parameter that carries a default is filled from it rather than reported
/// missing.
pub(crate) fn bind_keyword_arguments(
    signatures: &SignatureTable,
    callee: &str,
    positional: Vec<HirExpr>,
    keywords: Vec<(String, std::ops::Range<u32>, HirExpr)>,
    call_range: std::ops::Range<u32>,
) -> Result<Vec<HirExpr>, Diagnostic> {
    let signature = signatures
        .get(callee)
        .expect("caller checked `is_bindable_call` or `needs_default_fill`");
    let arity = signature.names.len();
    if positional.len() > arity {
        return Err(Diagnostic::error(
            "T0021",
            format!(
                "`{callee}` expects {arity} argument(s), got {}",
                positional.len() + keywords.len()
            ),
            span_of(call_range),
        )
        .with_help(format!("pass exactly {arity} argument(s)")));
    }
    let mut slots: Vec<Option<HirExpr>> = Vec::with_capacity(arity);
    slots.extend(positional.into_iter().map(Some));
    slots.resize_with(arity, || None);
    for (name, range, value) in keywords {
        let Some(index) = signature.names.iter().position(|param| *param == name) else {
            return Err(Diagnostic::error(
                "T0021",
                format!("`{callee}` got an unexpected keyword argument `{name}`"),
                span_of(range),
            )
            .with_help(format!(
                "`{callee}` accepts {}",
                describe_bindable(signature)
            )));
        };
        if index < signature.posonly_count {
            return Err(Diagnostic::error(
                "T0021",
                format!(
                    "`{callee}` got positional-only parameter `{name}` passed as a keyword argument"
                ),
                span_of(range),
            )
            .with_help(format!("pass `{name}` positionally")));
        }
        if slots[index].is_some() {
            return Err(Diagnostic::error(
                "T0021",
                format!("`{callee}` got multiple values for argument `{name}`"),
                span_of(range),
            )
            .with_help(format!(
                "`{name}` is already supplied positionally at this call"
            )));
        }
        slots[index] = Some(value);
    }
    // Part 2 of #884 (#1189): fill every unsupplied parameter that declares
    // a default before deciding what is still missing. The spliced node is
    // the one `func::params::literal_default` produced from the `def`, which
    // is byte-identical to the same literal written explicitly here.
    for (slot, default) in slots.iter_mut().zip(signature.defaults.iter()) {
        if slot.is_none()
            && let Some(default) = default
        {
            *slot = Some(default.clone());
        }
    }
    let missing: Vec<String> = signature
        .names
        .iter()
        .zip(slots.iter())
        .filter(|(_, slot)| slot.is_none())
        .map(|(name, _)| format!("`{name}`"))
        .collect();
    if !missing.is_empty() {
        return Err(Diagnostic::error(
            "T0021",
            format!(
                "`{callee}` is missing required argument(s): {}",
                missing.join(", ")
            ),
            span_of(call_range),
        )
        .with_help(format!("pass exactly {arity} argument(s)")));
    }
    Ok(slots
        .into_iter()
        .map(|slot| slot.expect("every slot was filled or reported missing"))
        .collect())
}

/// The human-readable list of parameter names a keyword may name, for the
/// unexpected-keyword help text.
fn describe_bindable(signature: &Signature) -> String {
    let bindable: Vec<String> = signature.names[signature.posonly_count..]
        .iter()
        .map(|name| format!("`{name}`"))
        .collect();
    if bindable.is_empty() {
        return "no keyword arguments".to_string();
    }
    bindable.join(", ")
}

fn span_of(range: std::ops::Range<u32>) -> Span {
    Span::new(range.start, range.end)
}

#[cfg(test)]
mod tests;
