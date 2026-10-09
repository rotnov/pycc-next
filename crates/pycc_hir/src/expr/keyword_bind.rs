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
                _ => {}
            }
        }
        Self {
            by_name,
            class_like_names,
            object_receivers: false,
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
    pub(crate) fn binds(&self, name: &str) -> bool {
        self.by_name.contains_key(name)
    }

    /// Part 2a of #1371: whether `name` is one of the module's class-like
    /// names (see the field's own documentation).
    pub(crate) fn is_class_like(&self, name: &str) -> bool {
        self.class_like_names.contains(name)
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
mod tests {
    use crate::{HirExpr, HirItem, HirStmt};

    /// The lowered HIR of `source`, which must lower cleanly.
    fn lower(source: &str) -> crate::HirModule {
        let module = pycc_parser::parse(source).expect("test fixture must parse");
        crate::lower_checked(&module).expect("fixture must lower")
    }

    fn lower_err(source: &str) -> pycc_diag::Diagnostic {
        let module = pycc_parser::parse(source).expect("test fixture must parse");
        crate::lower_checked(&module).expect_err("fixture must be rejected")
    }

    /// Part 8 of #1371: a keyword call the binder cannot bind is either
    /// refused here with the keyword `C0001` (`true`), or -- when its callee
    /// could be a CPython object -- deferred to `pycc_types` as a
    /// `HirExpr::KeywordCall` (`false`), which reports the same `C0001` for
    /// any callee that is not one (`pycc_types`' `foreign::keyword_call`).
    /// Either way the keywords are never bound or erased.
    fn keyword_call_refused_here(source: &str) -> bool {
        let module = pycc_parser::parse(source).expect("test fixture must parse");
        match crate::lower_checked(&module) {
            Err(diagnostic) => {
                assert_eq!(diagnostic.code, "C0001", "source: {source}");
                assert_eq!(
                    diagnostic.message,
                    crate::KEYWORD_CALL_UNSUPPORTED,
                    "source: {source}"
                );
                true
            }
            Ok(hir) => {
                let rendered = format!("{hir:?}");
                assert!(
                    rendered.contains("KeywordCall {"),
                    "source: {source}: {rendered}"
                );
                false
            }
        }
    }

    /// The argument vector of the first top-level `HirExpr::Call` statement.
    fn first_call_args(source: &str) -> Vec<HirExpr> {
        lower(source)
            .items
            .iter()
            .find_map(|item| match item {
                HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call { args, .. })) => {
                    Some(args.clone())
                }
                _ => None,
            })
            .expect("fixture has no top-level call statement")
    }

    /// Every lowered function's name and body, in module order.
    fn function_bodies(hir: &crate::HirModule) -> Vec<(&str, &Vec<HirStmt>)> {
        hir.items
            .iter()
            .filter_map(|item| match item {
                HirItem::Function { name, body, .. } => Some((name.as_str(), body)),
                HirItem::TopLevelStmt(_) => None,
            })
            .collect()
    }

    const DEF_AB: &str = "def f(a: int, b: int) -> None:\n    print(a)\n    print(b)\n\n";

    #[test]
    fn a_keyword_call_binds_to_the_same_arguments_as_its_positional_twin() {
        let keyword = first_call_args(&format!("{DEF_AB}f(b=2, a=1)\n"));
        let positional = first_call_args(&format!("{DEF_AB}f(1, 2)\n"));
        assert_eq!(keyword, positional);
        assert_eq!(
            keyword,
            vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)]
        );
    }

    #[test]
    fn a_mixed_positional_and_keyword_call_binds_the_keyword_to_the_later_parameter() {
        assert_eq!(
            first_call_args(&format!("{DEF_AB}f(1, b=2)\n")),
            vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)]
        );
    }

    #[test]
    fn a_keyword_call_written_before_its_own_def_still_binds() {
        // The table is collected from the whole module body before any item
        // is lowered, so forward references bind like any other call.
        assert_eq!(
            first_call_args(&format!(
                "def g() -> None:\n    f(b=2, a=1)\n\n{DEF_AB}g()\n"
            )),
            vec![]
        );
        let hir = lower(&format!(
            "def g() -> None:\n    f(b=2, a=1)\n\n{DEF_AB}g()\n"
        ));
        let body = function_bodies(&hir)
            .into_iter()
            .find_map(|(name, body)| (name == "g").then_some(body))
            .expect("the module must hold `g`");
        assert_eq!(
            body[0],
            HirStmt::ExprStmt(HirExpr::Call {
                callee: "f".to_string(),
                args: vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)],
            })
        );
    }

    #[test]
    fn a_keyword_call_inside_a_method_body_binds_too() {
        // `ClassBodyInput` forwards the table to every method body. The
        // compiler cannot catch a missing forward there -- an empty table
        // would merely make this call fall back to `C0001` -- so this test
        // is the only guard on that seam.
        let hir = lower(&format!(
            "{DEF_AB}class C:\n    def m(self) -> None:\n        f(b=2, a=1)\n"
        ));
        let call = function_bodies(&hir)
            .into_iter()
            .find_map(|(_, body)| {
                body.iter().find_map(|stmt| match stmt {
                    HirStmt::ExprStmt(call @ HirExpr::Call { callee, .. }) if callee == "f" => {
                        Some(call.clone())
                    }
                    _ => None,
                })
            })
            .expect("the method body must hold the bound call");
        assert_eq!(
            call,
            HirExpr::Call {
                callee: "f".to_string(),
                args: vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)],
            }
        );
    }

    #[test]
    fn a_keyword_naming_a_parameter_after_the_slash_marker_binds() {
        assert_eq!(
            first_call_args(
                "def f(a: int, /, b: int) -> None:\n    print(a)\n    print(b)\n\nf(1, b=2)\n"
            ),
            vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)]
        );
    }

    #[test]
    fn an_unexpected_keyword_name_is_a_type_error_at_the_keyword_span() {
        let source = format!("{DEF_AB}f(1, 2, c=3)\n");
        let diagnostic = lower_err(&source);
        assert_eq!(diagnostic.code, "T0021");
        assert_eq!(
            diagnostic.message,
            "`f` got an unexpected keyword argument `c`"
        );
        let start = u32::try_from(source.find("c=3").expect("fixture holds `c=3`"))
            .expect("fixture is short");
        assert_eq!(
            diagnostic.span,
            Some(pycc_diag::Span::new(start, start + 3))
        );
        assert_eq!(diagnostic.help.as_deref(), Some("`f` accepts `a`, `b`"));
    }

    #[test]
    fn a_stray_keyword_on_a_zero_parameter_callee_reports_no_bindable_names() {
        let diagnostic = lower_err("def f() -> None:\n    return\n\nf(extra=1)\n");
        assert_eq!(diagnostic.code, "T0021");
        assert_eq!(
            diagnostic.message,
            "`f` got an unexpected keyword argument `extra`"
        );
        assert_eq!(
            diagnostic.help.as_deref(),
            Some("`f` accepts no keyword arguments")
        );
    }

    #[test]
    fn a_positional_only_parameter_passed_as_a_keyword_is_a_type_error() {
        let diagnostic = lower_err(
            "def f(a: int, /, b: int) -> None:\n    print(a)\n    print(b)\n\nf(a=1, b=2)\n",
        );
        assert_eq!(diagnostic.code, "T0021");
        assert_eq!(
            diagnostic.message,
            "`f` got positional-only parameter `a` passed as a keyword argument"
        );
        assert_eq!(diagnostic.help.as_deref(), Some("pass `a` positionally"));
    }

    #[test]
    fn a_parameter_supplied_twice_is_a_type_error() {
        let diagnostic = lower_err(&format!("{DEF_AB}f(1, a=2, b=3)\n"));
        assert_eq!(diagnostic.code, "T0021");
        assert_eq!(
            diagnostic.message,
            "`f` got multiple values for argument `a`"
        );
        assert_eq!(
            diagnostic.help.as_deref(),
            Some("`a` is already supplied positionally at this call")
        );
    }

    #[test]
    fn a_parameter_left_unsupplied_is_a_type_error_at_the_call_span() {
        let diagnostic = lower_err(&format!("{DEF_AB}f(a=1)\n"));
        assert_eq!(diagnostic.code, "T0021");
        assert_eq!(
            diagnostic.message,
            "`f` is missing required argument(s): `b`"
        );
        assert_eq!(
            diagnostic.help.as_deref(),
            Some("pass exactly 2 argument(s)")
        );
    }

    #[test]
    fn more_positional_arguments_than_parameters_is_a_type_error() {
        let diagnostic = lower_err("def f(a: int) -> None:\n    print(a)\n\nf(1, 2, a=3)\n");
        assert_eq!(diagnostic.code, "T0021");
        assert_eq!(diagnostic.message, "`f` expects 1 argument(s), got 3");
        assert_eq!(
            diagnostic.help.as_deref(),
            Some("pass exactly 1 argument(s)")
        );
    }

    #[test]
    fn a_keyword_call_to_a_name_bound_twice_keeps_the_capability_rejection() {
        // Runtime dispatch of a redefined name is source-order sensitive,
        // while the table is static: neither `def`'s parameter order may
        // bind the keywords, so the name is left out of the table entirely.
        // Since Part 8 of #1371 the bare-name call is deferred to
        // `pycc_types` (`keyword_call_refused_here`), which refuses it
        // because the name is not a CPython object.
        for (source, refused_here) in [
            (
                "def f(a: int, b: int) -> None:\n    print(a - b)\n\nf(a=10, b=1)\n\n\
                 def f(b: int, a: int) -> None:\n    print(a - b)\n\nf(a=10, b=1)\n",
                false,
            ),
            (
                "f = 3\ndef f(a: int, b: int) -> None:\n    print(a - b)\n\nf(a=10, b=1)\n",
                false,
            ),
            (
                "from math import sqrt\ndef sqrt(a: float, b: float) -> float:\n    return a - b\n\nprint(sqrt(b=1.0, a=10.0))\n",
                false,
            ),
        ] {
            assert_eq!(
                keyword_call_refused_here(source),
                refused_here,
                "source: {source}"
            );
        }
    }

    #[test]
    fn a_kwargs_unpacking_keeps_the_unchanged_capability_rejection() {
        let diagnostic = lower_err(&format!("{DEF_AB}d = {{}}\nf(**d)\n"));
        assert_eq!(diagnostic.code, "C0001");
        assert_eq!(
            diagnostic.message,
            "keyword call arguments are not supported yet"
        );
    }

    #[test]
    fn a_def_using_a_parameter_kind_the_table_excludes_is_never_bindable() {
        // Each of these `def`s is rejected by `func::lower_params` on its own
        // terms; the point here is that `signature_of` returns `None` for it
        // rather than recording a signature pycc cannot represent.
        for (source, message) in [
            (
                "def f(*args) -> None:\n    return\n",
                "`*args` is not supported yet",
            ),
            (
                "def f(*, a: int) -> None:\n    print(a)\n",
                "keyword-only parameters are not supported yet",
            ),
            (
                "def f(**kwargs) -> None:\n    return\n",
                "`**kwargs` is not supported yet",
            ),
            // A default value used to be the fourth tuple here. Part 2 of
            // #884 (#1189) admits an admitted-literal default into the
            // table, so that shape now binds -- see
            // `a_default_is_filled_at_a_short_call` below. A default the
            // recognizer does *not* admit is still excluded, which the
            // syntactically-unadmitted tests below cover at the `def`.
        ] {
            let diagnostic = lower_err(&format!("{source}\nf(a=1)\n"));
            assert_eq!(diagnostic.code, "C0001", "source: {source}");
            assert_eq!(diagnostic.message, message, "source: {source}");
        }
    }

    #[test]
    fn every_call_shape_outside_the_bindable_one_is_refused_or_deferred() {
        // `false`: deferred to `pycc_types` since Part 8 of #1371, because
        // the callee could be a CPython object (`keyword_call_refused_here`).
        for (source, refused_here) in [
            // An undefined callee is not in the table.
            ("undefined_callee(extra=1)\n", false),
            // A method call.
            (
                "class C:\n    def m(self, a: int) -> None:\n        print(a)\n\nc = C()\nc.m(a=1)\n",
                false,
            ),
            // A `super()` method call.
            (
                "class B:\n    def m(self, a: int) -> None:\n        print(a)\n\nclass C(B):\n    def m(self, a: int) -> None:\n        super().m(a=1)\n",
                true,
            ),
            // A bare `super()` call.
            (
                "class C:\n    def m(self) -> None:\n        super(x=1)\n",
                true,
            ),
            // A class instantiation.
            (
                "class C:\n    def __init__(self, a: int) -> None:\n        self.a = a\n\nc = C(a=1)\n",
                false,
            ),
            // A builtin.
            ("print(sep=1)\n", false),
            // A container method.
            ("xs = [1]\nxs.append(value=2)\n", true),
            // A stdlib intrinsic through an aliased receiver.
            ("import math as m\n\nx = m.sqrt(x=1.0)\n", true),
            // A generic class instantiation through a subscript callee.
            (
                "class C[T]:\n    def __init__(self, a: int) -> None:\n        self.a = a\n\nc = C[int](a=1)\n",
                true,
            ),
            // `**` unpacking.
            ("d = {}\nundefined_callee(**d)\n", true),
        ] {
            assert_eq!(
                keyword_call_refused_here(source),
                refused_here,
                "source: {source}"
            );
        }
    }

    #[test]
    fn a_keyword_range_argument_stays_rejected_in_both_positions() {
        for source in [
            "for i in range(stop=3):\n    print(i)\n",
            "xs = [i for i in range(stop=3)]\n",
        ] {
            let diagnostic = lower_err(source);
            assert_eq!(diagnostic.code, "C0001", "source: {source}");
        }
    }

    // --- Part 2 of #884 (#1189): defaults filled at a short call ---------

    const DEF_A_B2: &str = "def f(a: int, b: int = 2) -> None:\n    print(a)\n    print(b)\n\n";

    /// Asserts that omitting a defaulted argument produces *exactly* the
    /// argument vector writing the same literal explicitly produces.
    ///
    /// This is the design's load-bearing invariant stated as a test: a
    /// filled default is byte-identical to its explicit twin, so it can
    /// never be checked more or less strictly than the same value at the
    /// call site.
    fn assert_default_matches_explicit(annotation: &str, literal: &str, expected: HirExpr) {
        let def = format!("def f(a: {annotation} = {literal}) -> None:\n    return\n\n");
        let filled = first_call_args(&format!("{def}f()\n"));
        let explicit = first_call_args(&format!("{def}f({literal})\n"));
        assert_eq!(
            filled, explicit,
            "annotation `{annotation}`, default `{literal}`"
        );
        assert_eq!(
            filled,
            vec![expected],
            "annotation `{annotation}`, default `{literal}`"
        );
    }

    #[test]
    fn a_default_is_filled_at_a_short_call() {
        assert_eq!(
            first_call_args(&format!("{DEF_A_B2}f(1)\n")),
            first_call_args(&format!("{DEF_A_B2}f(1, 2)\n"))
        );
        assert_eq!(
            first_call_args(&format!("{DEF_A_B2}f(1)\n")),
            vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)]
        );
    }

    #[test]
    fn every_call_shape_reaches_the_same_argument_vector() {
        let full = vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(3)];
        for call in ["f(1, 3)", "f(1, b=3)", "f(b=3, a=1)", "f(a=1, b=3)"] {
            assert_eq!(
                first_call_args(&format!("{DEF_A_B2}{call}\n")),
                full,
                "call: {call}"
            );
        }
        let defaulted = vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)];
        for call in ["f(1)", "f(a=1)"] {
            assert_eq!(
                first_call_args(&format!("{DEF_A_B2}{call}\n")),
                defaulted,
                "call: {call}"
            );
        }
    }

    #[test]
    fn each_admitted_literal_default_matches_its_explicit_twin() {
        assert_default_matches_explicit("int", "7", HirExpr::IntLiteral(7));
        assert_default_matches_explicit("float", "1.5", HirExpr::FloatLiteral(1.5));
        assert_default_matches_explicit("bool", "True", HirExpr::BoolLiteral(true));
        assert_default_matches_explicit("bool", "False", HirExpr::BoolLiteral(false));
        assert_default_matches_explicit("str", "\"hi\"", HirExpr::StringLiteral("hi".to_string()));
        assert_default_matches_explicit("int | None", "None", HirExpr::NoneLiteral);
    }

    #[test]
    fn a_folded_sign_on_a_numeric_default_matches_its_explicit_twin() {
        assert_default_matches_explicit("int", "-1", HirExpr::IntLiteral(-1));
        assert_default_matches_explicit("int", "+1", HirExpr::IntLiteral(1));
        assert_default_matches_explicit("float", "-1.5", HirExpr::FloatLiteral(-1.5));
        assert_default_matches_explicit("float", "+1.5", HirExpr::FloatLiteral(1.5));
    }

    #[test]
    fn both_i64_extremes_are_admitted_as_defaults() {
        assert_default_matches_explicit(
            "int",
            "9223372036854775807",
            HirExpr::IntLiteral(i64::MAX),
        );
        // The magnitude alone overflows `i64`; `fold_int_literal_sign`
        // applies the sign before the range check, exactly as the same
        // literal written at the call site is folded.
        assert_default_matches_explicit(
            "int",
            "-9223372036854775808",
            HirExpr::IntLiteral(i64::MIN),
        );
    }

    #[test]
    fn a_def_whose_parameters_all_have_defaults_accepts_a_zero_argument_call() {
        assert_eq!(
            first_call_args("def f(a: int = 1, b: str = \"s\") -> None:\n    return\n\nf()\n"),
            vec![
                HirExpr::IntLiteral(1),
                HirExpr::StringLiteral("s".to_string())
            ]
        );
    }

    #[test]
    fn a_default_on_either_side_of_the_slash_marker_is_filled() {
        assert_eq!(
            first_call_args("def f(a: int = 1, /, b: int = 2) -> None:\n    return\n\nf()\n"),
            vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)]
        );
        assert_eq!(
            first_call_args("def f(a: int = 1, /, b: int = 2) -> None:\n    return\n\nf(9)\n"),
            vec![HirExpr::IntLiteral(9), HirExpr::IntLiteral(2)]
        );
    }

    /// A parameter after the `/` marker stays bindable by name, so one call
    /// can drive the keyword loop and the default-fill loop together: `b` is
    /// named, `a` is filled from its default. This is the only shape where
    /// both loops act on the same call.
    #[test]
    fn a_keyword_after_the_slash_marker_binds_while_the_default_before_it_fills() {
        const DEF: &str = "def f(a: int = 1, /, b: int = 2) -> None:\n    return\n\n";
        assert_eq!(
            first_call_args(&format!("{DEF}f(b=3)\n")),
            vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(3)]
        );
        assert_eq!(
            first_call_args(&format!("{DEF}f(9, b=3)\n")),
            vec![HirExpr::IntLiteral(9), HirExpr::IntLiteral(3)]
        );
    }

    #[test]
    fn a_short_call_to_a_name_bound_twice_is_left_unfilled() {
        // Neither `def`'s default is spliced: the call reaches `pycc_types`
        // exactly as written, and its arity check rejects it there.
        assert_eq!(
            first_call_args(
                "def f(a: int = 1) -> None:\n    return\n\nf()\n\ndef f(a: int = 2) -> None:\n    return\n\nf()\n"
            ),
            vec![]
        );
    }

    #[test]
    fn an_unannotated_private_def_still_fills_its_default() {
        // No type is inferred from a default: `a` keeps `Ty::Infer`, and the
        // fill is the same literal an explicit call would carry.
        assert_eq!(
            first_call_args("def _f(a = 1):\n    return\n\n_f()\n"),
            vec![HirExpr::IntLiteral(1)]
        );
    }

    #[test]
    fn a_def_with_no_default_leaves_a_short_call_untouched() {
        // `needs_default_fill` answers `false` for a signature without a
        // single default, so a zero-keyword short call reaches `pycc_types`
        // with exactly the argument vector the user wrote, and the arity
        // error stays that crate's to report.
        assert_eq!(
            first_call_args(&format!("{DEF_AB}f(1)\n")),
            vec![HirExpr::IntLiteral(1)]
        );
    }

    #[test]
    fn a_short_call_to_a_callee_outside_the_table_is_left_alone() {
        // `needs_default_fill` answers `false` for an unknown callee, so the
        // zero-keyword path must not touch the argument vector.
        assert_eq!(
            first_call_args("undefined_callee(1)\n"),
            vec![HirExpr::IntLiteral(1)]
        );
    }

    #[test]
    fn a_default_the_recognizer_rejects_keeps_the_def_out_of_the_table() {
        // `signature_of` is infallible and purely syntactic: a `def` whose
        // default it cannot represent is simply absent from the table, so a
        // keyword call against it is never bound against a signature pycc
        // cannot materialize. Since Part 8 of #1371 the call is deferred to
        // `pycc_types` (which refuses it, `f` not being a CPython object),
        // so the `def`'s own `C0001` is the only diagnostic lowering reports.
        let source = "def f(a: int = len(\"x\")) -> None:\n    return\n\nf(a=1)\n";
        let module = crate::pycc_parser_test_helper::parse(source);
        let table = super::SignatureTable::collect(&module.body, |_| false);
        assert!(!table.binds("f"));
        let diagnostics = crate::lower_all(&module).expect_err("fixture must be rejected");
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert_ne!(
            diagnostics[0].message,
            crate::KEYWORD_CALL_UNSUPPORTED,
            "{diagnostics:?}"
        );
    }

    #[test]
    fn a_default_that_only_the_type_rule_rejects_is_still_in_the_table() {
        // The syntactic recognizer admits `1` at a `float` parameter; the
        // *type* rule at the `def` rejects it. The `def`'s own diagnostic is
        // what the user sees -- the call site adds nothing.
        let diagnostic = lower_err("def f(a: float = 1) -> None:\n    return\n\nf()\n");
        assert_eq!(diagnostic.code, "T0021");
        assert_eq!(
            diagnostic.message,
            "default value of parameter `a` of `f` expects `float`, got `int`"
        );
    }

    #[test]
    fn a_defaulted_def_leaves_an_over_long_call_untouched() {
        // `needs_default_fill` gates on `positional_len < names.len()`, so a
        // call with too many arguments is never rewritten and `pycc_types`
        // still sees the vector the user wrote.
        assert_eq!(
            first_call_args(&format!("{DEF_A_B2}f(1, 2, 3)\n")),
            vec![
                HirExpr::IntLiteral(1),
                HirExpr::IntLiteral(2),
                HirExpr::IntLiteral(3)
            ]
        );
    }

    #[test]
    fn a_required_parameter_before_a_defaulted_one_is_still_required() {
        let diagnostic = lower_err(&format!("{DEF_A_B2}f(b=3)\n"));
        assert_eq!(diagnostic.code, "T0021");
        assert_eq!(
            diagnostic.message,
            "`f` is missing required argument(s): `a`"
        );
    }

    /// The zero-keyword default-fill route in `expr.rs` can still fail: a
    /// call short of the callee's arity reaches the binder, the defaulted
    /// parameters are filled, and a *required* parameter that is still
    /// unsupplied is reported from there. This pins the error-propagation
    /// arm of that route, which the keyword route above does not exercise.
    #[test]
    fn a_zero_keyword_call_missing_a_required_argument_is_reported_from_the_binder() {
        let diagnostic = lower_err(&format!("{DEF_A_B2}f()\n"));
        assert_eq!(diagnostic.code, "T0021");
        assert_eq!(
            diagnostic.message,
            "`f` is missing required argument(s): `a`"
        );
        assert_eq!(
            diagnostic.help.as_deref(),
            Some("pass exactly 2 argument(s)")
        );
    }
}
