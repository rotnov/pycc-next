//! Private-helper constraint solver: the type-inference seam that runs when
//! a module's function signatures cannot be read off their annotations.
//!
//! This submodule holds the whole constraint-solving seam, extracted from
//! [`lib.rs`](crate) per the repository's source-file decomposition rule
//! (AGENTS.md "Keep source files decomposable", [D-185]), following the
//! precedent set by [`monomorphize`](crate::monomorphize),
//! [`binop`](crate::binop), and [`class`](crate::class).
//!
//! The seam is cohesive because every item in it exists to serve one
//! pipeline, entered exactly once from the module-level driver
//! ([`crate::module`], issue #868): `check_and_resolve_all` calls
//! `checked_function_signatures_all` there, which either accepts the
//! fully-annotated fast path ([`concrete_function_signatures`]) or falls
//! through to [`infer_function_signatures_with_solver_all`], both in the
//! [`signatures`] submodule. Everything else here is one of that pipeline's
//! stages, and none of them has any other caller:
//!
//! * its own type vocabulary -- `TypeTerm`, `SignatureTerms`,
//!   `BinOpConstraint`, [`SolverConstraints`],
//!   [`AnnotationDefaultConstraint`], and [`ConstraintEnvironment`], none of
//!   which appear anywhere in the annotation-driven checker;
//! * the union-find over inference variables ([`fresh_variable`],
//!   [`fresh_term`], [`root`], [`resolved_term`], [`unify_terms`],
//!   [`merge_inferred_types`], [`term_for_type`]);
//! * constraint collection ([`collect_expr_constraints`],
//!   [`bind_comp_loop_var`], [`collect_block_constraints`], whose shared
//!   `try`/`except`/`except*` walk lives in the [`try_stmt`] submodule);
//! * constraint application ([`propagate_binop_constraints`],
//!   [`apply_annotation_defaults`]) and the signature materialization that
//!   validates a solved signature set against the ordinary checker
//!   ([`concrete_function_signatures`], [`infer_function_signatures_with_solver_all`],
//!   in [`signatures`]; `checked_function_signatures_all` itself moved to
//!   [`crate::module`] with the rest of the driver).
//!
//! Deliberately left in `lib.rs` (or, for the driver, `module.rs`): the
//! annotation-driven checker itself. `check_and_resolve`,
//! `infer_expr`/`infer_expr_in`, [`check_stmt`](crate::check_stmt),
//! `check_stmt_in_function`, `check_with_signatures_all`, `check_assignment`,
//! [`Environment`], and the `bind_local_types_*` helpers
//! stay behind, together with the four issue-#118 `*_in_place*` fast-path
//! wrappers around `check_stmt`/`check_stmt_in_function`, which the solver
//! never calls. The solver *calls* the checker (it validates every candidate
//! signature set by re-running it), but it is the crate's primary entry
//! surface and is used by every other seam in the crate -- moving it would be
//! a different extraction, not this one.
//!
//! Deliberately *not* merged into [`solver`]: that module
//! already exists and holds a narrower, differently-motivated scope -- the
//! solver's definite-assignment control-flow join helpers, extracted for
//! issue #359. Before this extraction, `lib.rs` reached it through eight
//! qualified `solver::` paths and deliberately had no `pub use solver::*;`.
//! Folding this seam into `solver` would force adding that glob, which would
//! re-export those pre-existing definite-assignment helpers at the crate root
//! as a side effect -- a change to code outside this seam. A new sibling
//! module keeps the extraction a pure relocation. All eight of those call
//! sites moved here with the rest of the seam, so `constraints` is now the
//! only caller of `solver`, reaching it through the same `solver::` paths.
//!
//! [D-185]: https://github.com/rotnov/pycc/blob/main/docs/decisions/D-185-permit-a-dedicated-tracking-issue-per-oversized.md

mod constructor_call;
mod method_return;
mod object_lift;
mod object_narrow;
mod set_comp;
mod signatures;
mod try_stmt;

#[cfg(test)]
pub(crate) use set_comp::ContainerDefault;
pub(crate) use set_comp::{DeferredConstraints, deferred_frozenset};
pub(crate) use signatures::*;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::binop::numeric_result_type;
use crate::std_receiver::{shadowed_std_receiver, std_qualified_symbol, std_receiver_shadowed};
use crate::unop::unary_result_type;
use crate::{
    Environment, annotation_marker_is_not_a_value, cast_marker_is_not_a_value,
    enum_marker_is_not_a_value, is_assignable, is_generic_signature, is_known_callable_builtin,
    is_local, is_marker_kind, marker_is_not_a_value, non_callable_binding, solver,
    std_constant_is_not_callable, std_function_used_as_a_value, std_scalar_to_ty, t0042,
    ty_contains_param, type_checking_marker_is_not_a_value, typing_form_is_not_a_value,
    unbound_local, unsupported_callable_builtin,
};
use pycc_diag::{Diagnostic, Span};
use pycc_hir::{
    BinOpKind, CompElt, CompIter, ContainerReceiver, FStringPart, HirExpr, HirItem, HirModule,
    HirStmt, Ty, UnaryOpKind, bool_op_result_ty,
};

type TypeTerm = Result<Ty, usize>;
type SignatureTerms = (Vec<String>, Vec<TypeTerm>, TypeTerm);
type BinOpConstraint = (BinOpKind, TypeTerm, TypeTerm, TypeTerm);

fn is_private_solver_scalar(ty: &Ty) -> bool {
    matches!(ty, Ty::Int | Ty::Float | Ty::Bool | Ty::Str | Ty::None)
}

/// D-146 (#239): determines whether a list literal's collected element terms
/// represent a homogeneous scalar-element list this solver can carry as a
/// `Ty::List` element-type carrier. Returns the shared element `Ty` when every
/// element produced `Some(Ok(ty))`, all element types are exactly equal (exact
/// `Ty` equality, matching `infer_expr_in`'s own homogeneity rule -- NOT
/// `merge_inferred_types`, which would silently widen `bool` to `int`), and the
/// shared element type is a private-solver scalar (`is_private_solver_scalar`).
/// Returns `None` for heterogeneous lists, empty lists, non-scalar element
/// lists, or any element producing `None`/`Err` -- those keep the historical
/// `Ok(None)` behavior. The `is_private_solver_scalar` gate prevents
/// nested-container carriers (`list[list[int]]`) and non-scalar element types
/// this solver has no representation for. The returned carrier is destructured
/// by the `Subscript`/`ListPop` arms, never unified -- `unify_terms` and
/// `merge_inferred_types` are unchanged.
pub(crate) fn homogeneous_private_solver_scalar_list_element(
    element_terms: &[Option<TypeTerm>],
) -> Option<Ty> {
    let first = element_terms.first()?.as_ref()?.as_ref().ok()?;
    if !is_private_solver_scalar(first) {
        return None;
    }
    if !element_terms
        .iter()
        .all(|term| matches!(term, Some(Ok(ty)) if ty == first))
    {
        return None;
    }
    Some(first.clone())
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct AnnotationDefaultConstraint {
    pub(crate) initializer: TypeTerm,
    pub(crate) annotation: Ty,
}

#[derive(Debug, Default)]
pub(crate) struct SolverConstraints {
    pub(crate) deferred: DeferredConstraints,
    pub(crate) annotation_defaults: Vec<AnnotationDefaultConstraint>,
    non_scalar_local_terms: Vec<usize>,
}

#[derive(Debug, Clone)]
pub(crate) struct ConstraintEnvironment<'scope, 'hir> {
    pub(crate) bindings: HashMap<String, TypeTerm>,
    pub(crate) local_names: &'scope [&'hir str],
    /// Mirror of `Environment::def_rebound` (D-110): names whose net
    /// source-order module binding is a `def`, kept apart from the term
    /// bindings for the same reason -- terms must survive a `def` for
    /// representation purposes.
    pub(crate) defs_rebound: HashSet<String>,
    /// Issue #359 (Part 2 of #118): names whose binding is *maybe* —
    /// assigned in only one branch of an `if` (no `else`), or only in a
    /// loop body, or introduced as a `for` loop variable (the loop may
    /// execute zero times). Mirrors the validation pass's
    /// `BindingState::Maybe` distinction (D-147): a maybe-bound name's
    /// type term is still in `bindings` (if it IS bound, it has that
    /// type), but `collect_expr_constraints`'s `Name` arm skips
    /// unification for maybe-bound names — the validation pass's `T0041`
    /// diagnostic is the user-facing gate, not the solver's inferred type.
    pub(crate) maybe_bindings: HashSet<String>,
    /// Issue #771: names whose binding is definite (unconditionally
    /// assigned, unlike `maybe_bindings`) but whose right-hand-side
    /// expression is one the solver cannot represent as a type term at all
    /// (e.g. a class-target `cast`, a `.pop()`, an attribute read, a method
    /// call, or a heterogeneous container literal — see the `Ok(None)`
    /// arms throughout `collect_expr_constraints`). Without this set, such
    /// a name is left out of `bindings` entirely, and a later read of it
    /// falls through to the "not a solver-tracked local" case even though
    /// it *is* a syntactic local — producing a misleading `unbound_local`
    /// (`T0021`) diagnostic instead of `Ok(None)` (no term, but not an
    /// error). `HirExpr::Name` consults this set after `maybe_bindings`
    /// and after `bindings` itself, so a real term always takes priority
    /// over a stale opaque marker for the same name.
    pub(crate) opaque_bindings: HashSet<String>,
    /// Part 1 of #1026: the subset of `opaque_bindings` that a foreign
    /// `import` bound to a CPython module object. Unlike every other
    /// opaque binding, this one *does* have a type -- `Ty::Object` -- so
    /// the `Name` arm hands the solver `Ok(Ty::Object)` for it instead of
    /// "no term at all"; see that arm for why the distinction matters and
    /// why these names must nonetheless stay out of `bindings`.
    ///
    /// The set is seeded once from the module's import table and never
    /// mutated afterwards, because neither way of displacing a foreign
    /// name can be misled by a stale entry. A module-level rebinding puts
    /// a real term in `bindings`, which the `Name` arm consults first; a
    /// function-local name shadowing a foreign one is dropped from
    /// `opaque_bindings` by the per-function seeding in `signatures`, and
    /// the arm that reads this set is reached only through that one.
    pub(crate) foreign_objects: HashSet<String>,
    /// Part 1 of #883 (#962): mirror of `Environment::std_module_aliases`
    /// -- every `(alias, module)` pair the module's import table binds,
    /// from `std_receiver::bind_std_module_aliases`. Populated on the
    /// globals environment in `constraints::signatures` and copied into
    /// each per-function environment there; the stdlib receiver shadow
    /// check in `collect_expr_constraints` reads it.
    pub(crate) std_module_aliases: Vec<(String, pycc_std::StdModule)>,
    /// Part 2a of #1142 (#1165): the solver's mirror of
    /// `Environment::owned_buffers` -- names bound to buffer storage this
    /// artifact allocated rather than to a host-borrowed parameter.
    ///
    /// A mirror is required rather than convenient. The solver runs before
    /// the check phase, so the `Name` arm's `reject_memoryview_read` above
    /// is the *first* seam any owned read reaches; without this set every
    /// one of them would report the parameter message and the provenance
    /// distinction would be unobservable no matter what the check phase
    /// went on to record.
    pub(crate) owned_buffers: HashSet<String>,
    /// Part 2a of #1142 (#1165): `true` while a *function body* is being
    /// collected, `false` for the module's own top-level statements.
    ///
    /// The solver's counterpart of `Environment::in_function_body`, and
    /// needed for the same one reason: the buffer producer is refused at
    /// module scope with its own diagnostic, and `local_names` cannot stand
    /// in for the distinction (a function with no locals also has an empty
    /// slice).
    pub(crate) in_function_body: bool,
    /// The solver's counterpart of `Environment::returns_inside_finally`,
    /// and the solver half of review round 5's egress narrowing.
    ///
    /// Computed from the same `pycc_hir::body_returns_inside_finally` walk,
    /// so the two phases cannot drift: the predicate exists once, in
    /// `pycc_hir`, and both environments carry its answer rather than
    /// re-deriving it.
    ///
    /// Unlike the check phase's, this one is *not* independently observable
    /// -- the solver runs first, so `crate::module::merge_solver_first`
    /// reports whichever refusal fires, and both phases raise the same
    /// `crate::buffer::buffer_return_inside_finally` text. What it buys is
    /// that the solver can never admit an egress the check phase refuses.
    pub(crate) returns_inside_finally: bool,
    /// Part 2a of #1142 (#1165): the buffer-producer spellings this module
    /// binds itself, which therefore keep the program's own meaning
    /// (D-244 #1129 statement (h)).
    ///
    /// The check phase needs no equivalent -- `env.lookup_class` and
    /// `env.lookup_generic` already run ahead of its interception -- but
    /// this buffer-producer seam does not consult `ConstraintEnvironment`'s class table
    /// (`class_defs`, keyed by the canonical class name), and `signatures`
    /// covers only `def`s. Seeded once per module in
    /// `constraints::signatures` from the HIR's own class table, per
    /// *spelling*: a module defining `class ndarray` must still be able to
    /// call `NDArray(n)`. Since Part 1 of #1319 it also holds `frozenset`
    /// when the module defines `class frozenset`, for the same reason: the
    /// solver's `frozenset(...)` builtin arm must yield to the class.
    pub(crate) shadowed_producers: HashSet<String>,
    /// #1165 review round 8: the names a `Final` annotation has bound in
    /// this scope, the solver's counterpart of `Environment::finals`.
    ///
    /// Consulted at the buffer producer's admitting seam only
    /// (`reject_final_rebinding`), exactly as the parameter-rebinding guard
    /// beside it mirrors one `crate::check_assignment` refusal rather than
    /// the whole function: the solver has no `Final` model of its own, and
    /// the reason this one member of it is needed is that a producer
    /// assignment the solver admits records artifact-owned provenance whose
    /// later use raises a `C0001` that displaces the check phase's `T0045`.
    /// Every other `Final` reassignment is refused by the check phase with
    /// no solver diagnostic to compete with it.
    ///
    /// Seeded empty for each function body rather than inherited from
    /// module scope, on the `owned_buffers` precedent, and the two phases
    /// were confirmed to agree there rather than argued to: rebinding a
    /// module-level `Final` name to a producer call inside a function body
    /// makes the *check* phase admit the producer and raise the same
    /// owned-buffer `C0001`, because a function-local assignment binds a
    /// new local instead of rebinding the module-level name. An empty start
    /// therefore matches the check phase exactly here, and can otherwise
    /// only ever be *narrower* than it -- the safe direction for a seam
    /// whose answer displaces the check phase's.
    pub(crate) finals: HashSet<String>,
    /// #1420: the module's class table (`HirModule::class_defs`), read
    /// by the `MethodCall` arm to resolve `recv.m(...)` on a user-class
    /// instance to the method's return term
    /// (`method_return::method_call_on_instance`) and, since #1342, by the
    /// `Call` arm's constructor term (`constructor_call`). Shared by every
    /// environment of one module; empty in a unit-test environment.
    pub(crate) class_defs: &'hir [(String, pycc_hir::HirClassDef)],
    /// #1476 (Part 3 of #1387): the solver's mirror of
    /// `Environment::narrowed` for an `object` name an `isinstance` guard
    /// narrowed to a native type; `object_narrow` sets and drops it.
    /// Starts empty in every scope.
    pub(crate) narrowed: HashMap<String, Ty>,
}

impl<'scope, 'hir> ConstraintEnvironment<'scope, 'hir> {
    /// An environment with no bindings of any kind and the given local
    /// names -- the struct-update base every unit-test literal starts
    /// from, so adding a field here never means editing a hundred test
    /// literals.
    #[cfg(test)]
    pub(crate) fn empty(local_names: &'scope [&'hir str]) -> Self {
        Self {
            bindings: HashMap::new(),
            local_names,
            defs_rebound: HashSet::new(),
            maybe_bindings: HashSet::new(),
            opaque_bindings: HashSet::new(),
            foreign_objects: HashSet::new(),
            std_module_aliases: Vec::new(),
            owned_buffers: HashSet::new(),
            in_function_body: false,
            returns_inside_finally: false,
            shadowed_producers: HashSet::new(),
            finals: HashSet::new(),
            class_defs: &[],
            narrowed: HashMap::new(),
        }
    }

    /// Part 2a of #1142 (#1165), round-11 review finding 2: drop `name`'s
    /// artifact-owned buffer provenance because `name` is about to stop
    /// denoting that buffer.
    ///
    /// Provenance is flat and flow-insensitive, which `crate::buffer`'s
    /// `buffer_parameter_rebinding` doc justifies with "no name can change
    /// provenance mid-function". A *parameter*-bound name cannot, because
    /// rebinding one is refused outright. An artifact-*owned* name can:
    /// reassigning one is deliberately admitted (codegen frees the previous
    /// allocation before the store, D-074), and every other binder --
    /// `for`, a comprehension target, a walrus, a `match` capture, an
    /// `except ... as` name -- rebinds without asking this seam at all.
    ///
    /// Clearing the marker alone is not the fix: `crate::expr::
    /// reject_memoryview_read` takes the provenance flag *and* the term, so
    /// a cleared marker over a surviving `Ok(Ty::MemoryView)` binding swaps
    /// the owned refusal for the parameter one -- a different wrong answer,
    /// not an answer. The binding is therefore dropped with it, and the
    /// name is recorded as opaquely-but-definitely bound so a later read
    /// reports "no term" (`Ok(None)`) rather than `unbound_local`. A caller
    /// that immediately rebinds `name` to a real term overwrites that
    /// entry, and `HirExpr::Name`'s lookup prefers `bindings` over the
    /// opaque marker, so the marker it leaves behind is inert.
    ///
    /// With the solver no longer claiming a buffer that is gone, the read
    /// stops raising `C0001` and the check phase's own answer for the
    /// reassignment -- `T0023`, from `crate::check_assignment` -- is the
    /// one `crate::module::merge_solver_first` reports.
    pub(crate) fn rebind_over_owned_buffer(&mut self, name: &str) {
        if self.owned_buffers.remove(name) {
            self.bindings.remove(name);
            self.opaque_bindings.insert(name.to_string());
        }
    }

    /// Whether `receiver` is bound at this use site, for the solver's
    /// stdlib-receiver shadow check (`shadowed_std_receiver`): a term
    /// binding (a maybe-bound name's term stays in `bindings` -- only
    /// unification consults `maybe_bindings` -- so `Maybe` is covered), a
    /// `def` rebinding (the `defs_rebound` mirror of the validation pass's
    /// `def_rebound`, never a position-blind signature lookup), or a
    /// syntactic local of the enclosing function.
    fn is_std_receiver_bound(&self, receiver: &str) -> bool {
        self.bindings.contains_key(receiver)
            || self.defs_rebound.contains(receiver)
            || is_local(self.local_names, receiver)
    }
}

fn fresh_variable(parents: &mut Vec<usize>, concrete: &mut Vec<Option<Ty>>) -> usize {
    let id = parents.len();
    parents.push(id);
    concrete.push(None);
    id
}

pub(crate) fn fresh_term(parents: &mut Vec<usize>, concrete: &mut Vec<Option<Ty>>) -> TypeTerm {
    Err(fresh_variable(parents, concrete))
}

fn root(parents: &mut [usize], var: usize) -> usize {
    let parent = parents[var];
    if parent == var {
        parent
    } else {
        let root = root(parents, parent);
        parents[var] = root;
        root
    }
}

pub(crate) fn resolved_term(
    term: TypeTerm,
    parents: &mut [usize],
    concrete: &[Option<Ty>],
) -> Option<Ty> {
    match term {
        Ok(ty) => Some(ty),
        Err(var) => concrete[root(parents, var)].clone(),
    }
}

fn resolved_private_signature_term(
    term: TypeTerm,
    parents: &mut [usize],
    concrete: &[Option<Ty>],
    non_scalar_local_roots: &HashSet<usize>,
) -> Option<Ty> {
    match term {
        Ok(ty) => Some(ty),
        Err(var) => {
            let term_root = root(parents, var);
            let resolved = concrete[term_root].clone()?;
            (!non_scalar_local_roots.contains(&term_root) || is_private_solver_scalar(&resolved))
                .then_some(resolved)
        }
    }
}

/// Builds the conflict diagnostic for a failed `unify_terms` merge.
///
/// `declared` is `Some` only when the caller knows one of the two terms is a
/// *written return annotation* rather than an inferred type -- today that is
/// exactly `collect_block_constraints`' `HirStmt::Return` arm for an annotated
/// function (#949). In that case the conflict is not an ambiguous
/// inference clash between two equally-derived types: the annotation is the
/// canonical "correct" side and the other operand is what the body actually
/// produced, so the message says so and, per D-152's standing contract on the
/// return-type family (`docs/DIAGNOSTICS.md`'s quality bar), carries a `help`
/// suggestion. The wording deliberately follows this crate's existing
/// `<noun> mismatch: expected `X`, found `Y`` shape (see `dict key type
/// mismatch` in `expr.rs`) and stays textually distinct from the annotation
/// checker's own `expected return type `X`, got `Y`` for the same program --
/// `module::tests`' merge tests assert the two phases word it differently so
/// they can prove the solver's text is the one that survives de-duplication.
///
/// The `actual` operand is selected by comparing against `declared` rather
/// than by position: `unify_terms`' second arm calls this with the
/// already-inferred type *first* and the known type second, the reverse of the
/// `(Ok, Ok)` arm's order, so a positional assumption would silently invert
/// the message. See the ordering note in `unify_terms` itself.
fn inference_conflict(
    code: &'static str,
    context: &str,
    left: Ty,
    right: Ty,
    declared: Option<&Ty>,
) -> Diagnostic {
    let Some(declared) = declared else {
        return Diagnostic::error(
            code,
            format!(
                "{context}: conflicting inferred types `{}` and `{}`",
                left.name(),
                right.name()
            ),
            Span::new(0, 0),
        );
    };
    let actual = if left == *declared { right } else { left };
    let help = crate::foreign::object_into_scalar_help(&actual, declared)
        .unwrap_or_else(|| format!("return a `{}` value", declared.name()));
    Diagnostic::error(
        code,
        format!(
            "return type mismatch: expected `{}`, found `{}`",
            declared.name(),
            actual.name()
        ),
        Span::new(0, 0),
    )
    .with_help(help)
}

pub(crate) fn unify_terms(
    left: TypeTerm,
    right: TypeTerm,
    parents: &mut [usize],
    concrete: &mut [Option<Ty>],
    code: &'static str,
    context: &str,
) -> Result<bool, Diagnostic> {
    unify_terms_with_declared(left, right, parents, concrete, code, context, None)
}

/// `unify_terms`, plus the caller's knowledge of which side (if either) is a
/// written annotation rather than an inferred type -- see `inference_conflict`.
///
/// Ordering invariant this relies on (#949): `declared` is `Some` only at the
/// `HirStmt::Return` call site, which passes the return term *first*. So with
/// `declared.is_some()`, `left` is `Ok(declared)` in the `(Ok, Ok)` arm below,
/// the `Ok` side is always the declared type in the merged
/// `(Err(var), Ok(ty)) | (Ok(ty), Err(var))` arm, and the `(Err, Err)` arm is
/// unreachable -- an annotated return never lowers to an inference variable
/// (`signatures::term_for_type` returns `Ok(ty)` for every `ty != Ty::Infer`).
/// A future edit that swaps that call's argument order would not break the
/// message (`inference_conflict` compares rather than indexes), but it would
/// invalidate this note; no `debug_assert!` guards it because that would be an
/// unreachable region under D-014's 100% region gate.
pub(crate) fn unify_terms_with_declared(
    left: TypeTerm,
    right: TypeTerm,
    parents: &mut [usize],
    concrete: &mut [Option<Ty>],
    code: &'static str,
    context: &str,
    declared: Option<&Ty>,
) -> Result<bool, Diagnostic> {
    match (left, right) {
        (Ok(left), Ok(right)) => merge_inferred_types(left.clone(), right.clone())
            .map(|_| false)
            .ok_or_else(|| inference_conflict(code, context, left, right, declared)),
        (Err(var), Ok(ty)) | (Ok(ty), Err(var)) => {
            // D-133/D-134: this constraint solver exists only to infer a
            // `Ty::Infer` parameter/return of an unannotated private
            // helper -- it has no notion of PEP 695 call-site substitution
            // the way `infer_expr_in`'s own generic-dispatch arm does. A
            // `Ty::Param` merging into a genuinely unresolved inference
            // variable here (as opposed to the `(Ok, Ok)` arm above, an
            // ordinary structural comparison -- e.g. a generic function's
            // own body unifying its parameter's term against its return
            // term, both already `Ok(Ty::Param(_))`, ordinary and correct
            // per Task 2's "opaque, self-consistent" design) would
            // otherwise silently leak the internal type-parameter name
            // into a private helper's inferred signature, surfacing later
            // as a confusing, unnamed failure (e.g. "operator Add is not
            // defined for `T` and `int`") at a span pointing at the
            // generic `def` rather than the actual call site -- this
            // project's own practice (D-105) says a precisely nameable
            // case like this should instead get a clean, specific
            // diagnostic. Reuses `T0042` (the same family of
            // "generic-function shape or call-site instantiation
            // rejected" failures Task 2 introduced) rather than inventing
            // a new code, since this is exactly that family's
            // "instantiation rejected" shape, just discovered from the
            // inference side instead of the call side.
            //
            // Checked as a plain `if` inside this arm (not a separate
            // `(Err(_), Ok(ty)) | (Ok(ty), Err(_))`-guarded arm above it)
            // deliberately: every current production call site only ever
            // reaches this function with the "known-or-generic" side in
            // one fixed position per call site (e.g. `Return`'s own
            // `unify_terms(return_term, actual)` always puts the declared
            // return term first), so a guard on a second, differently-
            // ordered `Ok`/`Err` pattern alternative would be permanently
            // unreachable dead code under the D-014 coverage gate -- this
            // single shared check covers both orderings without splitting
            // into an unreachable region.
            if ty_contains_param(&ty) {
                return Err(t0042(format!(
                    "{context} cannot be inferred through a PEP 695 generic function's own type parameter -- add an explicit type annotation instead of relying on inference here"
                )));
            }
            let root = root(parents, var);
            let merged = match concrete[root].clone() {
                Some(current) => merge_inferred_types(current.clone(), ty.clone())
                    .ok_or_else(|| inference_conflict(code, context, current, ty, declared))?,
                None => ty,
            };
            let changed = concrete[root] != Some(merged.clone());
            concrete[root] = Some(merged);
            Ok(changed)
        }
        (Err(left), Err(right)) => {
            let left_root = root(parents, left);
            let right_root = root(parents, right);
            if left_root == right_root {
                return Ok(false);
            }
            let merged = match (concrete[left_root].clone(), concrete[right_root].clone()) {
                // `declared` is threaded through for consistency even though
                // this arm is unreachable while it is `Some` (see the ordering
                // note above): a future caller that made it reachable would get
                // the right message rather than a silently stale one.
                (Some(left), Some(right)) => Some(
                    merge_inferred_types(left.clone(), right.clone())
                        .ok_or_else(|| inference_conflict(code, context, left, right, declared))?,
                ),
                (Some(ty), None) | (None, Some(ty)) => Some(ty),
                (None, None) => None,
            };
            parents[right_root] = left_root;
            concrete[left_root] = merged;
            Ok(true)
        }
    }
}

fn merge_inferred_types(left: Ty, right: Ty) -> Option<Ty> {
    if left == right {
        Some(left)
    } else if matches!((left, right), (Ty::Bool, Ty::Int) | (Ty::Int, Ty::Bool)) {
        Some(Ty::Int)
    } else {
        None
    }
}

fn term_for_type(ty: Ty, parents: &mut Vec<usize>, concrete: &mut Vec<Option<Ty>>) -> TypeTerm {
    if ty == Ty::Infer {
        fresh_term(parents, concrete)
    } else {
        Ok(ty)
    }
}

/// Part 2a of #1142 (#1165): the solver's answer to "is this expression a
/// buffer producer *here*?", returning the spelling and the length argument
/// when it is.
///
/// # The subset invariant this seam and its callers owe the check phase
///
/// `crate::module::merge_solver_first` reports the solver's diagnostic for a
/// function whenever it has one, so the solver's *admitted* set must be a
/// subset of `crate::buffer::producer_assignment_ty`'s: admitting one
/// program that mirror refuses means recording an artifact-owned binding
/// whose later use raises the owned-buffer `C0001`, and that wrong message
/// displaces the check phase's correct one. Three consecutive review rounds
/// on this seam each found one missing member of the check phase's
/// admission conditions (`std_module_aliases`, `foreign_objects`, and the
/// length-type check below), so the conditions are enumerated here in full
/// and split across this predicate and its callers exactly as the check
/// phase splits them:
///
/// * here -- the call shape, the spelling, statement (h), the arity, and
///   the function-body scope;
/// * at each caller -- the parameter-rebinding guard
///   (`reject_buffer_parameter_rebinding`), the length type
///   (`reject_non_int_producer_length`), and, on the `AnnAssign` arm only,
///   the declared annotation (`reject_producer_annotation_mismatch`).
///
/// One condition of the check phase's is deliberately *not* mirrored, and
/// the difference is recorded rather than closed: an unresolved length term
/// (`def _h(n): a = ndarray(n)`, where the caller later fixes `n` to `str`)
/// is admitted here, because refusing it needs a length constraint carried
/// to the end of solving rather than one more guard at this seam. See
/// `reject_non_int_producer_length`'s own doc comment.
///
/// The solver's own statement (h): a `def` of the spelling is in
/// `signatures`, a `class` of it is in `shadowed_producers` (the class
/// table is not consulted here), a module-level value binding of it is in `bindings`, and
/// a *function-local* binding of it is in `local_names`. Any of the four
/// means the program's own meaning wins.
///
/// `local_names` cannot be folded into the `bindings` check and is not
/// redundant with it: `constraints::signatures` deliberately *removes* every
/// local name from the per-function `bindings` map it seeds, and a local is
/// re-entered there only once the walk reaches its binding statement. A body
/// that binds the spelling after using it therefore has an empty `bindings`
/// answer at the use site, while CPython makes the name local for the whole
/// body and raises `UnboundLocalError`. The check-phase mirror is
/// `crate::buffer::producer_assignment_ty`; declining here hands the value to
/// the ordinary `Call` walk, whose own `is_local` gate (further down this
/// file, ahead of the producer refusal) reports `unbound_local`.
///
/// `std_module_aliases` is the mirror of that same check's fifth arm: a
/// stdlib module alias (`import math as ndarray`) binds the spelling but is
/// recorded in no other table, so without it this solver -- which runs over
/// unannotated private helpers -- allocated a buffer for a program whose own
/// binding makes the call CPython's `TypeError`. See
/// `crate::buffer::producer_assignment_ty` for the full reason, and
/// `docs/TYPE_SYSTEM.md`'s `memoryview` row for the canonical enumeration.
///
/// `foreign_objects` is the sixth arm, and it is one the check-phase mirror
/// does not need: `crate::foreign::bind_foreign_objects_at` binds a foreign
/// `import ndarray` into the check phase's own `bindings` as `Ty::Object`,
/// so that mirror's `bindings` arm already declines, while the solver
/// deliberately keeps foreign names *out* of `bindings` and records them in
/// this separate table instead (see the `Name` arm for why). Without this
/// arm the solver treats the foreign call as the intrinsic producer and
/// marks the assigned name artifact-owned, so a second use of it raises the
/// owned-buffer `C0001` -- and `crate::module`'s `merge_solver_first` makes
/// that the reported diagnostic, displacing the `I0404` foreign refusal the
/// check phase correctly produces and pointing the span at the `import`
/// line. The program is refused either way; only the message is wrong. See
/// `docs/TYPE_SYSTEM.md`'s `memoryview` row for the canonical enumeration.
fn resolved_producer_call<'a>(
    signatures: &HashMap<String, SignatureTerms>,
    env: &ConstraintEnvironment<'_, '_>,
    expr: &'a HirExpr,
) -> Option<(&'a str, &'a HirExpr)> {
    let HirExpr::Call { callee, args } = expr else {
        return None;
    };
    if !crate::buffer::is_producer_spelling(callee)
        || signatures.contains_key(callee)
        || env.shadowed_producers.contains(callee.as_str())
        || env.bindings.contains_key(callee.as_str())
        || is_local(env.local_names, callee)
        || env
            .std_module_aliases
            .iter()
            .any(|(alias, _)| alias == callee)
        || env.foreign_objects.contains(callee.as_str())
        || args.len() != 1
        || !env.in_function_body
    {
        return None;
    }
    Some((callee.as_str(), &args[0]))
}

/// Part 2a of #1142 (#1165): the solver's mirror of
/// `crate::buffer::producer_assignment_ty`'s own `Int | Bool` length check,
/// applied to the term the producer's length argument collected.
///
/// Without it the solver discarded that term and recorded an owned buffer
/// for `a = ndarray("x")`, so a later use of `a` raised the owned-buffer
/// `C0001` and `crate::module::merge_solver_first` displaced the check
/// phase's correct `T0033` with it -- the same shape as the
/// `std_module_aliases` and `foreign_objects` omissions before it. See
/// `resolved_producer_call`'s doc comment for the invariant all three
/// violate.
///
/// `bool` is admitted alongside `int` for the reason
/// `crate::buffer::producer_assignment_ty` spells out: the representation
/// table makes a `bool` an `int` (`docs/TYPE_SYSTEM.md`, rule 4/D-086).
///
/// The guard fires only on a term that *resolves* to a concrete type. An
/// unresolved term is admitted rather than refused, and that is the one
/// place the solver stays deliberately wider than the check phase: the
/// producer's length is very often the helper's own unannotated parameter
/// (`a = ndarray(n)`), whose term is still a variable at this point in the
/// walk, and refusing it would refuse the admitted shape this seam exists
/// for. A term that a *later* call-site constraint resolves to a non-`int`
/// therefore still reaches the check phase's `T0033` rather than this one --
/// unless a use of the owned name in the same body raises the owned `C0001`
/// first, which is the residue this guard does not reach and which needs a
/// deferred length constraint, not a seventh mirrored arm.
fn reject_non_int_producer_length(
    callee: &str,
    len_term: Option<TypeTerm>,
    parents: &mut [usize],
    concrete: &[Option<Ty>],
) -> Result<(), Diagnostic> {
    if let Some(term) = len_term
        && let Some(len_ty) = resolved_term(term, parents, concrete)
        && !matches!(len_ty, Ty::Int | Ty::Bool)
    {
        return Err(crate::buffer::producer_length_not_an_int(callee, &len_ty));
    }
    Ok(())
}

/// Part 2a of #1142 (#1165): the solver's mirror of
/// `check_stmt_in_function`'s `AnnAssign` assignability test, applied where
/// the producer is admitted under a declared annotation.
///
/// Without it `a: int = ndarray(4)` bound `a` as artifact-owned storage in
/// the solver while the check phase correctly refused the statement, so a
/// later use of `a` raised the owned-buffer `C0001` and
/// `crate::module::merge_solver_first` reported that instead of the
/// `T0025`. See `resolved_producer_call`'s doc comment for the invariant.
///
/// `crate::is_assignable` rather than a `matches!` on `Ty::MemoryView`
/// because it is the *same* predicate the check phase reaches: that arm
/// calls `class::is_assignable_env`, whose `Instance`/`Protocol` arms cannot
/// match a `from` of `Ty::MemoryView`, so it falls through to this function
/// -- including its `Optional` clause, which a hand-rolled match would drop.
/// The one residual difference is the *message* for a protocol annotation,
/// where the check phase raises a detailed `T0046` conformance error it can
/// only build from a class table the solver does not have; the refusal
/// itself agrees.
fn reject_producer_annotation_mismatch(target: &str, annotation: &Ty) -> Result<(), Diagnostic> {
    if crate::is_assignable(Ty::MemoryView, annotation.clone()) {
        return Ok(());
    }
    Err(crate::annotation_initializer_mismatch(
        target,
        &Ty::MemoryView,
        annotation,
    ))
}

/// Part 2a of #1142 (#1165): the solver's mirror of `crate::check_assignment`'s
/// buffer-parameter guard, applied at the producer's admitting seam.
///
/// Without it the solver admits `b = ndarray(4)` on a `memoryview`
/// *parameter* `b` and marks `b` artifact-owned, so a later read of `b` in
/// the same pass reaches `reject_memoryview_read` with `owned = true` and
/// raises the *owned* refusal. `crate::module`'s `merge_solver_first` makes
/// that wrong wording the one the compiler emits, overruling the check
/// phase, which flags the same statement with the correct *parameter*
/// wording. See `buffer::buffer_parameter_rebinding` for the two independent
/// grounds the refusal rests on.
///
/// Keying on `Some(Ok(Ty::MemoryView))` rather than on "a term that may
/// unify to `MemoryView`" is exact here, not an approximation: an `Err(var)`
/// term is only ever resolved to a concrete type by
/// `apply_annotation_defaults`, whose `is_private_solver_scalar` guard
/// admits `Int | Float | Bool | Str | None` only, so no inferred term can
/// become `MemoryView`. A `memoryview` parameter's term is therefore always
/// the concrete `Ok(Ty::MemoryView)` this guard matches.
fn reject_buffer_parameter_rebinding(
    env: &ConstraintEnvironment<'_, '_>,
    target: &str,
) -> Result<(), Diagnostic> {
    if matches!(env.bindings.get(target), Some(Ok(Ty::MemoryView)))
        && !env.owned_buffers.contains(target)
    {
        return Err(crate::buffer::buffer_parameter_rebinding(target));
    }
    Ok(())
}

/// #1165 review round 8: the solver's mirror of `crate::check_assignment`'s
/// PEP 591 `Final` refusal, applied at the producer's admitting seam
/// alongside `reject_buffer_parameter_rebinding`.
///
/// The condition is that refusal's, minus one conjunct the solver cannot
/// reach. The check phase also requires the name to be in `bindings`,
/// because its `finals` set additionally holds a *value-less* `x:
/// Final[int]` declaration, whose own first assignment must stay admitted.
/// This solver records a `Final` name only at the producer seam, and only
/// after that statement's binding -- its value-less `AnnAssign` arm is a
/// deliberate no-op -- so membership in `finals` already implies membership
/// in `bindings`, and repeating the conjunct here would be a branch no test
/// could kill.
///
/// Needed for the same reason the parameter guard beside it is: without it
/// the solver admitted `a = ndarray(8)` on a `Final` name, recorded
/// artifact-owned provenance for it, and a later use of that name raised
/// the owned-buffer `C0001` that `crate::module::merge_solver_first` then
/// reported in place of the check phase's correct `T0045`.
fn reject_final_rebinding(
    env: &ConstraintEnvironment<'_, '_>,
    target: &str,
) -> Result<(), Diagnostic> {
    if env.finals.contains(target) {
        return Err(crate::final_reassignment(target));
    }
    Ok(())
}

pub(crate) fn collect_expr_constraints(
    signatures: &HashMap<String, SignatureTerms>,
    parents: &mut Vec<usize>,
    concrete: &mut Vec<Option<Ty>>,
    deferred: &mut DeferredConstraints,
    env: &ConstraintEnvironment<'_, '_>,
    expr: &HirExpr,
) -> Result<Option<TypeTerm>, Diagnostic> {
    match expr {
        HirExpr::IntLiteral(_) => Ok(Some(Ok(Ty::Int))),
        HirExpr::FloatLiteral(_) => Ok(Some(Ok(Ty::Float))),
        HirExpr::BoolLiteral(_) => Ok(Some(Ok(Ty::Bool))),
        HirExpr::StringLiteral(_) => Ok(Some(Ok(Ty::Str))),
        HirExpr::NoneLiteral => Ok(Some(Ok(Ty::None))),
        // #1418: `return NotImplemented` in a comparison method of an `ext`
        // module is CPython's singleton, a CPython object.
        HirExpr::NotImplemented => Ok(Some(Ok(Ty::Object))),
        HirExpr::Name(name) => {
            // D-136: a `pycc_hir`-qualified stdlib name (`"math.pi"`) is
            // checked before ordinary binding lookup. Post-review finding:
            // the qualified string itself can never collide with a real
            // binding (see `std_qualified_symbol`'s own doc comment), but
            // its *receiver* (`"math"`) can -- a real local/parameter
            // legally named `math` shadows the stdlib module the same way
            // `float`'s own user-definition-takes-priority guard elsewhere
            // in this function handles for that hand-recognized name (see
            // `std_receiver_shadowed`'s own doc comment).
            if let Some(symbol) = std_qualified_symbol(name) {
                if let Some(shadowed) =
                    shadowed_std_receiver(symbol.module, &env.std_module_aliases, |receiver| {
                        env.is_std_receiver_bound(receiver)
                    })
                {
                    return Err(std_receiver_shadowed(shadowed, symbol.module));
                }
                return match symbol.kind {
                    pycc_std::StdSymbolKind::Constant { ty } => Ok(Some(Ok(std_scalar_to_ty(ty)))),
                    pycc_std::StdSymbolKind::Function { .. } => {
                        Err(std_function_used_as_a_value(name))
                    }
                    pycc_std::StdSymbolKind::EnumMarker => Err(enum_marker_is_not_a_value(name)),
                    pycc_std::StdSymbolKind::AnnotationMarker => {
                        Err(annotation_marker_is_not_a_value(name))
                    }
                    pycc_std::StdSymbolKind::CastMarker => Err(cast_marker_is_not_a_value(name)),
                    pycc_std::StdSymbolKind::TypeCheckingMarker => {
                        Err(type_checking_marker_is_not_a_value(name))
                    }
                    pycc_std::StdSymbolKind::TypingFormMarker => {
                        Err(typing_form_is_not_a_value(name))
                    }
                    pycc_std::StdSymbolKind::ProtocolMarker
                    | pycc_std::StdSymbolKind::AbcMarker
                    | pycc_std::StdSymbolKind::DecoratorMarker
                    | pycc_std::StdSymbolKind::EnumAutoMarker => Err(marker_is_not_a_value(name)),
                };
            }
            // Issue #359 (Part 2 of #118): a maybe-bound name (assigned
            // in only one branch of an `if`, or only in a loop body) is
            // not safely readable — the validation pass's T0041 diagnostic
            // is the user-facing gate. In the solver, skip unification for
            // such names by returning `Ok(None)` (no type term available),
            // so the solver does not infer a return type from a value that
            // might not exist. This mirrors the validation pass's
            // `BindingState::Maybe` distinction (D-147).
            if env.maybe_bindings.contains(name.as_str()) {
                return Ok(None);
            }
            // #1476: a read an `isinstance` guard narrowed.
            if let Some(term) = object_narrow::narrowed_read(env, name) {
                return Ok(Some(term));
            }
            match env.bindings.get(name).cloned() {
                Some(term) => {
                    // Part 1 of #1027: this arm is the solver's shared read
                    // seam, and the solver runs first, so any operation that
                    // inspects a concrete argument term -- `len(v)`, an
                    // alias, a `for` -- would otherwise report its own type
                    // error (`T0033`, ...) about a `memoryview` before the
                    // check phase's documented capability gap could fire.
                    // Reading the name is the gap wherever it appears, so it
                    // is refused here for every reader at once rather than
                    // one caller at a time. The `Call` arm's own gate below
                    // stays: a call's callee is a bare string, not a `Name`
                    // expression, so it never reaches this seam.
                    if let Some(ty) = resolved_term(term.clone(), parents, concrete) {
                        crate::expr::reject_memoryview_read(
                            name,
                            &ty,
                            env.owned_buffers.contains(name),
                        )?;
                    }
                    Ok(Some(term))
                }
                // Issue #771: a definitely-assigned name whose initializer
                // the solver couldn't represent as a term (see
                // `opaque_bindings`'s doc comment) is not an unbound local
                // — it just has no type term to offer. Return `Ok(None)`
                // instead of falling through to `unbound_local` below.
                //
                // Part 1 of #1026 carves out the one opaque binding that
                // *does* have a term: a foreign `import` binds `Ty::Object`,
                // and `TypeTerm` is `Result<Ty, usize>`, so `Ok(Ty::Object)`
                // is an ordinary concrete term. Offering it matters because
                // `Ok(None)` leaves a helper that returns the module object
                // with an unresolved return variable, and signature
                // materialization then reports `T0021: cannot infer return
                // type ...; add an annotation` -- advice the user cannot
                // act on, because in a native build the foreign object type
                // is deliberately unspellable (only a class a foreign import
                // binds resolves to it, Part 1 of #1367; an `--ext` module
                // can also write `Any`/`object` under D-258). With the term, the return materializes and
                // the check phase's own `I0404` (choke point 1 in
                // `crate::foreign`) reports the real refusal instead.
                //
                // Part 2 of #1026 (#1081) rewrote the paragraph that stood
                // here. Part 1 claimed this arm was the only way
                // `Ty::Object` could enter the solver, and that the check
                // phase refused every expression-position read of a foreign
                // binding. Both claims are now false: a *module-scope* read
                // is admitted (`crate::foreign`'s module doc) and the
                // `AttrGet` arm below is a second entry point.
                //
                // The solver runs before the check phase, and this arm is
                // load-bearing precisely there: a body it can offer no term
                // for is reported as `T0021: cannot infer return type`,
                // which would pre-empt the `I0404` the check phase owes an
                // unannotated `def _helper(): return numpy.pi`. What the
                // term does *not* do is let such a signature reach
                // `pycc_codegen`: the check phase refuses returning a
                // CPython object from a function (#1316, its `Return` arm),
                // so the helper is rejected right after the solver hands
                // its return type over. The read itself is admitted in a
                // body since #1316 (`crate::expr`'s `Name` arm), and there
                // as at module scope the consumer-side refusals are what
                // keep a `Ty::Object` sound.
                //
                // The names still deliberately stay out of `bindings`: the
                // `Call` arm below refuses any bound non-`def` callee with
                // `non_callable_binding`, which would pre-empt the `Call`
                // arm's own foreign branch (#1313) with a `T0021`.
                None if env.opaque_bindings.contains(name.as_str()) => {
                    if env.foreign_objects.contains(name.as_str()) {
                        Ok(Some(Ok(Ty::Object)))
                    } else {
                        Ok(None)
                    }
                }
                None if is_local(env.local_names, name) => Err(unbound_local(name)),
                None => Ok(None),
            }
        }
        HirExpr::FString(parts) => {
            for part in parts {
                if let FStringPart::Interpolation(expr) = part {
                    collect_expr_constraints(signatures, parents, concrete, deferred, env, expr)?;
                }
            }
            Ok(Some(Ok(Ty::Str)))
        }
        HirExpr::Compare { op, left, right } => {
            let left_ty =
                collect_expr_constraints(signatures, parents, concrete, deferred, env, left)?;
            let right_ty =
                collect_expr_constraints(signatures, parents, concrete, deferred, env, right)?;
            // Part 1 of #1371: a rich comparison with a CPython object
            // operand is an object, mirroring `compare_link_ty`.
            let is_object = |ty: &Option<Result<Ty, usize>>| matches!(ty, Some(Ok(Ty::Object)));
            // Part 2b of #1371: a membership test is `bool` whatever its
            // operands are, mirroring `foreign::compare::membership_ty`.
            if !matches!(
                op,
                pycc_hir::CmpOpKind::Is
                    | pycc_hir::CmpOpKind::IsNot
                    | pycc_hir::CmpOpKind::In
                    | pycc_hir::CmpOpKind::NotIn
            ) && (is_object(&left_ty) || is_object(&right_ty))
            {
                return Ok(Some(Ok(Ty::Object)));
            }
            Ok(Some(Ok(Ty::Bool)))
        }
        // #1212: a chained comparison mirrors `Compare` -- every operand is
        // collected, and the result is `bool`.
        HirExpr::CompareChain { first, links } => {
            for operand in pycc_hir::compare_chain_operands(first, links) {
                collect_expr_constraints(signatures, parents, concrete, deferred, env, operand)?;
            }
            Ok(Some(Ok(Ty::Bool)))
        }
        // #1211 (Part 3 of #1018): `and`/`or`. A truth-context node is
        // `bool` whatever its operands are, exactly like `Not` below. A
        // value-context node over two concrete operand types takes the one
        // join rule `pycc_types::boolop` and `pycc_mir` also use; a pair with
        // no join yields no term, and `infer_expr_in` then reports the
        // `T0021`. An operand that is still an inference variable is unified
        // with the other operand -- a sound restriction that infers only the
        // equal-type case, which `infer_expr_in` re-checks afterwards.
        HirExpr::BoolOp {
            op,
            left,
            right,
            truth_only,
        } => {
            let left =
                collect_expr_constraints(signatures, parents, concrete, deferred, env, left)?;
            let right =
                collect_expr_constraints(signatures, parents, concrete, deferred, env, right)?;
            if *truth_only {
                return Ok(Some(Ok(Ty::Bool)));
            }
            match (left, right) {
                (Some(Ok(left)), Some(Ok(right))) => {
                    Ok(bool_op_result_ty(*op, &left, &right).map(Ok))
                }
                (Some(left), Some(right)) => {
                    unify_terms(
                        left.clone(),
                        right,
                        parents,
                        concrete,
                        "T0021",
                        &format!("`{}` operands", op.as_str()),
                    )?;
                    Ok(Some(left))
                }
                _ => Ok(None),
            }
        }
        // #1395: `body if test else orelse`. `test` is collected for its
        // own constraints only; its type does not reach the result. Two
        // concrete branch types take the one join rule `pycc_types::if_exp`
        // and `pycc_mir` also use, and a pair with no join yields no term,
        // so `infer_expr_in` reports the `T0021`. A branch that is still an
        // inference variable is unified with the other branch, inferring
        // only the equal-type case -- except against `None`, which would
        // bind the variable to `None` where the join means `T | None`, so
        // that pair yields no term.
        HirExpr::IfExp { test, body, orelse } => {
            collect_expr_constraints(signatures, parents, concrete, deferred, env, test)?;
            let body =
                collect_expr_constraints(signatures, parents, concrete, deferred, env, body)?;
            let orelse =
                collect_expr_constraints(signatures, parents, concrete, deferred, env, orelse)?;
            match (body, orelse) {
                (Some(Ok(body)), Some(Ok(orelse))) => {
                    Ok(pycc_hir::if_exp_result_ty(&body, &orelse).map(Ok))
                }
                (Some(Ok(Ty::None)), _) | (_, Some(Ok(Ty::None))) => Ok(None),
                (Some(body), Some(orelse)) => {
                    unify_terms(
                        body.clone(),
                        orelse,
                        parents,
                        concrete,
                        "T0021",
                        "conditional expression branches",
                    )?;
                    Ok(Some(body))
                }
                _ => Ok(None),
            }
        }
        // #603 (Part 2 of #573). An operand whose type is already concrete
        // is typed directly by `unary_result_type`, so a bad operand keeps
        // the unary diagnostic ("unary operator USub is not defined for
        // `str`") rather than a confusing binary one about the rewrite's
        // synthetic `0`. That path covers every operand the solver can see
        // a type for, which is the overwhelming majority.
        //
        // An operand that is still an inference variable has no type to
        // check yet, so `USub`/`UAdd`/`Invert` defer to the exact binary
        // constraint `pycc_mir` will lower the expression to -- `0 - x`,
        // `0 + x`, or (for `Invert`) the first leg of `0 - x - 1`.
        // `numeric_result_type(Sub | Add, Int, operand)` *is* the unary
        // rule (`Int`/`Bool` give `Int` since `-True == -1`, `Float` gives
        // `Float`, anything else is `T0021`), so reusing it keeps the
        // solver's view of the expression identical to MIR's own rewrite
        // and the two cannot drift apart. `Invert` shares that same `Sub`
        // shape rather than getting a distinct one: it happens to accept
        // exactly the same concrete types (`Int`/`Bool`) `unary_result_type`
        // gives it, so the one place this deferred path is looser than
        // `unary_result_type` -- permitting a still-unresolved `Float`
        // operand, which `Invert`'s own concrete rule at line 430 above
        // rejects with `T0021` -- is unreachable from any HIR this
        // compiler's own generic-inference lowering produces today (no
        // generic-context construct here yields a `Float`-typed inference
        // variable feeding `~x`).
        //
        // #604 (Part 3 of #573): `not x` needs none of this. Its result is
        // `Ty::Bool` regardless of the operand's type -- `Compare` just
        // above hardcodes its own `Ty::Bool` result the exact same way,
        // without ever inspecting `left`/`right`'s resolved types either --
        // so `Not` only has to walk the operand for its own nested
        // constraints (a walrus inside `not (n := f())`, say) and never
        // needs a `binops` entry at all.
        HirExpr::UnaryOp {
            op: UnaryOpKind::Not,
            operand,
        } => {
            collect_expr_constraints(signatures, parents, concrete, deferred, env, operand)?;
            Ok(Some(Ok(Ty::Bool)))
        }
        HirExpr::UnaryOp { op, operand } => {
            let operand =
                collect_expr_constraints(signatures, parents, concrete, deferred, env, operand)?;
            match operand {
                Some(Ok(operand_ty)) => Ok(Some(Ok(unary_result_type(*op, operand_ty)?))),
                Some(operand) => {
                    let result = fresh_term(parents, concrete);
                    // Only `USub`/`UAdd`/`Invert` ever reach this arm --
                    // `Not` is peeled off by its own dedicated arm above,
                    // before `collect_expr_constraints` recurses into this
                    // one -- and `USub`/`Invert` share the same `Sub`
                    // shape, so a plain `UAdd` check (rather than an
                    // exhaustive match that would need a structurally
                    // unreachable `Not` arm the coverage gate could never
                    // exercise) is enough to pick the right one.
                    let bin_op = if matches!(op, UnaryOpKind::UAdd) {
                        BinOpKind::Add
                    } else {
                        BinOpKind::Sub
                    };
                    deferred
                        .binops
                        .push((bin_op, Ok(Ty::Int), operand, result.clone()));
                    Ok(Some(result))
                }
                None => Ok(None),
            }
        }
        // Part 1 of #891: the unpacked value's own term passes through.
        // Whether the value can be unpacked at all, and into how many
        // names, is `crate::unpack::infer_unpack`'s check-phase gate; the
        // element reads that follow are ordinary `Subscript`s.
        HirExpr::Unpack { value, .. } => {
            collect_expr_constraints(signatures, parents, concrete, deferred, env, value)
        }
        HirExpr::BinOp { op, left, right } => {
            let left =
                collect_expr_constraints(signatures, parents, concrete, deferred, env, left)?;
            let right =
                collect_expr_constraints(signatures, parents, concrete, deferred, env, right)?;
            match (left, right) {
                (Some(left), Some(right)) => {
                    let result = fresh_term(parents, concrete);
                    deferred.binops.push((*op, left, right, result.clone()));
                    Ok(Some(result))
                }
                _ => Ok(None),
            }
        }
        HirExpr::Call { callee, args } => {
            // Mirror of `infer_expr_in`'s D-110 call-target rule (#133): an
            // active value binding shadows builtin and function lookup. At
            // module level `local_names` is empty and `bindings` holds the
            // accumulated top-level assignments in source order; for a
            // private-helper body the environment is seeded from those module
            // globals with the helper's own local names stripped and its
            // parameters re-inserted, so this gate sees a module binding
            // exactly when Python's name resolution would. Binding-first
            // ordering preserves the local diagnostics unchanged, as in
            // `infer_expr_in`. This mirror is load-bearing on its own when
            // the bound callee is neither `print` nor any `def`: without it,
            // `signatures.get` misses, the call stays unresolved, and the
            // solver dead-ends in the misleading "cannot infer return type"
            // error *before* pass 3 ever runs. For a shadowed `print`
            // specifically, the special case below would resolve the call
            // and pass 3's own gate would still catch the shadowing later --
            // there this mirror is fail-fast defense-in-depth, not the only
            // line of defense.
            if let Some(term) = env.bindings.get(callee).cloned()
                && !env.defs_rebound.contains(callee)
            {
                // Part 1 of #1333: a callee whose term is concretely
                // `object` is a call of a CPython object, not a D-110
                // non-callable value. Keyed on the unresolved term, never on
                // `resolved_term`, so admission cannot depend on source
                // order (`object_lift`'s module doc).
                if let Some(result) = object_lift::call_of_object_binding(
                    signatures, parents, concrete, deferred, env, &term, args,
                )? {
                    return Ok(Some(result));
                }
                // Part 1 of #1027: this gate, not `infer_expr_in`'s own
                // D-110 arm, is the one a `memoryview` *parameter* reaches
                // -- the solver runs first, and a parameter's annotation is
                // already a binding here while the check phase never gets to
                // look at the call. Calling the name is a *read* of it, so
                // it is the capability gap every other use of a `memoryview`
                // is (`C0001`), not D-110's "no value in the current subset
                // is callable" (`T0021`).
                if let Some(ty) = resolved_term(term, parents, concrete) {
                    crate::expr::reject_memoryview_read(
                        callee,
                        &ty,
                        env.owned_buffers.contains(callee),
                    )?;
                }
                return Err(non_callable_binding(callee));
            }
            // #1313: a foreign import binds its name to a CPython object,
            // and the name deliberately stays out of `bindings` (see the
            // `Name` arm), so the gate above cannot see it. A call of it is
            // an `object` producer, exactly like the `MethodCall` arm's
            // `o.method(...)`: the arguments are collected as ordinary
            // expressions and the call answers the concrete `object` term.
            // Position is the check phase's job, as for the `Name` arm's
            // read: a call inside an unannotated private helper resolves
            // the helper's return to `object` here -- rather than leaving
            // it unresolved, which signature materialization would report
            // as a `T0021` asking for an annotation the type cannot be
            // spelled in. Since Part 1 of #1333 the check phase admits the
            // helper returning it.
            if env.foreign_objects.contains(callee.as_str()) {
                for arg in args {
                    collect_expr_constraints(signatures, parents, concrete, deferred, env, arg)?;
                }
                return Ok(Some(Ok(Ty::Object)));
            }
            if is_local(env.local_names, callee) {
                return Err(unbound_local(callee));
            }
            // #1116, the solver half of `crate::expr`'s own `len`
            // interception: `len(b)` on a `memoryview`-bound name is a
            // `Ty::Int` element count. Like the `Subscript` interception
            // further below it must run *before* the argument recursion
            // that follows, because that recursion reaches the `Name` seam
            // above, which calls `reject_memoryview_read` and would report
            // the `C0001` capability gap for the very expression that
            // closes it. The arity is checked here rather than deferred to
            // the `callee == "len"` block below for the same ordering
            // reason -- a two-argument `len(b, x)` must still reach that
            // block's `T0033`, so only the one-argument shape is claimed.
            if args.len() == 1
                && callee == "len"
                && let HirExpr::Name(buffer_name) = &args[0]
                && let Some(term) = env.bindings.get(buffer_name).cloned()
                && let Some(Ty::MemoryView) = resolved_term(term, parents, concrete)
            {
                return Ok(Some(Ok(Ty::Int)));
            }
            // Part 2a of #1142 (#1165), the solver half of `crate::expr`'s
            // own producer refusal: `ndarray(n)`/`NDArray(n)` is admitted
            // only as an assignment's whole right-hand side, which
            // `collect_block_constraints` handles before the value reaches
            // this walk. Placed before the argument recursion for the same
            // ordering reason the `len` interception above is: the recursion
            // reaches the `Name` seam, which would report a read refusal for
            // an argument of the very expression being refused.
            //
            // The callee-bound gate further above already returned for a
            // name the module binds to a *value*, so only the two spelling
            // shadows and the module-alias binding remain to check here. The
            // last of those is statement (h)'s fifth arm: a stdlib module
            // alias binds the spelling but is a value in no table, so
            // without it this line refused the program's own call with the
            // producer's position message instead of letting the walk below
            // report it. See `crate::buffer::producer_assignment_ty`.
            if crate::buffer::is_producer_spelling(callee)
                && !signatures.contains_key(callee)
                && !env.shadowed_producers.contains(callee.as_str())
                && !env
                    .std_module_aliases
                    .iter()
                    .any(|(alias, _)| alias == callee)
            {
                return Err(if env.in_function_body {
                    crate::buffer::producer_position_unsupported(callee)
                } else {
                    crate::buffer::producer_at_module_scope(callee)
                });
            }
            let mut arg_terms = Vec::with_capacity(args.len());
            for arg in args {
                arg_terms.push(collect_expr_constraints(
                    signatures, parents, concrete, deferred, env, arg,
                )?);
            }
            if callee == "print" {
                return Ok(Some(Ok(Ty::None)));
            }
            if callee == "len" {
                // D-105 point 3: `len(lst)` is a hand-recognized builtin
                // call, same as `print` above, not a user-declarable
                // signature. Its own return (`Ty::Int`) never depends on
                // the list's element type, so it's always producible here
                // regardless of whether the argument's own term has
                // resolved yet -- unlike `ListLiteral`/`Subscript`/
                // `ListAppend` above, there's no homogeneity-style check
                // to defer. Only a term that is *already* a known concrete
                // type can be validated at this point in constraint
                // collection (union-find resolution hasn't run yet); an
                // unresolved argument is left to the real check pass
                // (`infer_expr_in`) below, matching this solver's existing
                // lenient-until-known pattern. PR-11 Task 3 (D-123) relaxed
                // the argument-type check below to also accept `Ty::Dict`;
                // PR-11 Task 7 (D-123) relaxes it once more to also accept
                // `Ty::Set`.
                if arg_terms.len() != 1 {
                    return Err(Diagnostic::error(
                        "T0033",
                        format!("`len` expects exactly 1 argument, got {}", arg_terms.len()),
                        Span::new(0, 0),
                    )
                    .with_help("pass exactly 1 argument"));
                }
                // Part 3 of #1026 (PR 3a of #1082): mirrors `expr.rs`'s own
                // `len` guard, which now also admits `Ty::Object`. The
                // `Ok(Ty::Int)` result term below was already unconditional,
                // so accepting the new argument type here is the whole
                // change the solver needs.
                if let Some(Ok(arg_ty)) = &arg_terms[0]
                    && !matches!(
                        arg_ty,
                        Ty::List(_) | Ty::Dict(_) | Ty::Set(_) | Ty::FrozenSet(_) | Ty::Object
                    )
                {
                    return Err(Diagnostic::error(
                        "T0033",
                        format!(
                            "`len` expects a `list[T]`, `dict[K, V]`, `set[T]`, or `frozenset[T]` argument, got `{}`",
                            arg_ty.name()
                        ),
                        Span::new(0, 0),
                    ).with_help("pass a `list[T]`, `dict[K, V]`, `set[T]`, or `frozenset[T]` value"));
                }
                return Ok(Some(Ok(Ty::Int)));
            }
            // #435: `isinstance`/`issubclass` are compile-time-evaluated
            // builtins that always return `Ty::Bool`. The constraint solver
            // only needs the result type — the actual validation and
            // compile-time evaluation happen in `infer_expr_in`'s own Call
            // arm. The class arguments (bare names or tuples of bare names)
            // produce `Ok(None)` terms in the solver (class names are not
            // value bindings), which is harmless.
            // A user-defined function named `isinstance`/`issubclass` takes
            // priority over the builtin (same pattern as `float` above).
            if (callee == "isinstance" || callee == "issubclass")
                && !signatures.contains_key(callee)
            {
                return Ok(Some(Ok(Ty::Bool)));
            }
            // #767: `cast(T, value)` is a compile-time-only construct whose
            // type is exactly `T`. Mirrors `infer_expr_in`'s own `cast`
            // interception; without this mirror the callee misses
            // `signatures`, the call stays unresolved, and the solver
            // dead-ends in "cannot infer return type" before the validation
            // pass ever reports the real diagnostic. `args[0]` is a bare
            // type name, which the `Name` arm above already resolved to a
            // harmless `Ok(None)` term (a type name is not a value
            // binding), exactly as for `isinstance`'s class argument.
            //
            // A malformed call shape (wrong arity, or a first argument that
            // is not a bare name) is reported here, from the same shared
            // helper `check_cast` uses, so the two passes cannot drift.
            //
            // The class table is not consulted here (`class_defs` serves
            // the method-call and constructor-call terms only), so it produces a *term* only for the
            // four builtin scalar target names it can recognize on its own;
            // an unverified class name yields `Ok(None)` and leaves the
            // decision to `check_cast`. Producing an unverified
            // `Ty::Instance(name)` here instead was measurably worse: for
            // `cast(Nope, x)` the solver's term reached the return-type
            // comparison first and reported `T0022` ("conflicting inferred
            // types `int` and `Nope`") in place of `check_cast`'s accurate
            // `T0001` ("`Nope` is not a known class or builtin type").
            //
            // #767 review fix (D-198): unlike `check_cast`, this arm does
            // not reject a representation-changing target (`cast(str, 5)`,
            // `cast(int, flag)`) — the
            // solver has no resolved `Ty` for `value` to compare against at
            // this point, only an unsolved term. This is the same asymmetry
            // `check_isinstance`'s own doc comment already accepts between
            // the two passes: full validation runs on the validation-pass
            // route (an annotated function's body, module level,
            // `AnnAssign`) and is absent for a return-type-inferred private
            // helper, which only ever reaches this solver arm.
            if callee == "cast" && !signatures.contains_key(callee) {
                let target = crate::class::cast_target_name(args)?;
                if pycc_hir::is_builtin_type_name(target) {
                    return Ok(Some(Ok(crate::class::cast_target_ty(target))));
                }
                return Ok(None);
            }
            if let Some(symbol) = std_qualified_symbol(callee) {
                // See `shadowed_std_receiver`'s own doc comment -- a real
                // local/parameter named `math` (or an alias of it, #962)
                // shadows the stdlib module.
                if let Some(shadowed) =
                    shadowed_std_receiver(symbol.module, &env.std_module_aliases, |receiver| {
                        env.is_std_receiver_bound(receiver)
                    })
                {
                    return Err(std_receiver_shadowed(shadowed, symbol.module));
                }
                let pycc_std::StdSymbolKind::Function {
                    arg_tys: expected_arg_tys,
                    ret_ty,
                } = symbol.kind
                else {
                    return Err(if is_marker_kind(symbol.kind) {
                        if matches!(symbol.kind, pycc_std::StdSymbolKind::EnumMarker) {
                            enum_marker_is_not_a_value(callee)
                        } else if matches!(symbol.kind, pycc_std::StdSymbolKind::AnnotationMarker) {
                            annotation_marker_is_not_a_value(callee)
                        } else if matches!(symbol.kind, pycc_std::StdSymbolKind::CastMarker) {
                            cast_marker_is_not_a_value(callee)
                        } else if matches!(symbol.kind, pycc_std::StdSymbolKind::TypeCheckingMarker)
                        {
                            type_checking_marker_is_not_a_value(callee)
                        } else if matches!(symbol.kind, pycc_std::StdSymbolKind::TypingFormMarker) {
                            typing_form_is_not_a_value(callee)
                        } else {
                            marker_is_not_a_value(callee)
                        }
                    } else {
                        std_constant_is_not_callable(callee)
                    });
                };
                if arg_terms.len() != expected_arg_tys.len() {
                    return Err(Diagnostic::error(
                        "T0021",
                        format!(
                            "`{callee}` expects {} argument(s), got {}",
                            expected_arg_tys.len(),
                            arg_terms.len()
                        ),
                        Span::new(0, 0),
                    )
                    .with_help(format!(
                        "pass exactly {} argument(s)",
                        expected_arg_tys.len()
                    )));
                }
                for (term, expected) in arg_terms.iter().zip(expected_arg_tys) {
                    if let Some(Ok(arg_ty)) = term
                        && *arg_ty != std_scalar_to_ty(*expected)
                    {
                        return Err(Diagnostic::error(
                            "T0021",
                            format!(
                                "`{callee}` expects `{}`, got `{}`",
                                std_scalar_to_ty(*expected).name(),
                                arg_ty.name()
                            ),
                            Span::new(0, 0),
                        )
                        .with_help(format!(
                            "pass a `{}` value",
                            std_scalar_to_ty(*expected).name()
                        )));
                    }
                }
                return Ok(Some(Ok(std_scalar_to_ty(ret_ty))));
            }
            if callee == "float" && !signatures.contains_key(callee) {
                // A user-defined `float` takes priority over the builtin -- see
                // `infer_expr_in`'s own identical guard and its comment for why
                // this differs from `len`/`print`, which need no such guard.
                // Mirrors the `len` arm immediately above for the same reason (D-105
                // point 3's rationale applies identically): `float`'s own return type
                // (`Ty::Float`) never depends on the argument's resolved type, so it
                // is always producible here regardless of whether the argument's own
                // term has resolved yet. Only an already-concretely-resolved argument
                // can be validated at this point (union-find resolution hasn't run);
                // an unresolved argument is left to the real check pass
                // (`infer_expr_in`) above, matching this solver's existing
                // lenient-until-known pattern.
                if arg_terms.len() != 1 {
                    return Err(Diagnostic::error(
                        "T0021",
                        format!(
                            "`float` expects exactly 1 argument, got {}",
                            arg_terms.len()
                        ),
                        Span::new(0, 0),
                    )
                    .with_help("pass exactly 1 argument"));
                }
                // Part 4 of #1026 (PR 4a of #1083) admits `Ty::Object` here
                // exactly as `infer_expr_in`'s own arm does; the two must not
                // drift. Its comment carries the D-244 rule-7 reasoning and the
                // reason the message still enumerates only the three original
                // types.
                //
                // It does *not* carry that arm's user-defined-class guard: the
                // class table is not consulted here. Nor
                // does it need one -- a `Ty::Object` term can only come from a
                // foreign name, and `I0404` refuses a foreign name used in a
                // function body at all, which is the only place this solver
                // runs (issue #142's unannotated private helpers). Verified by
                // compiling a private helper returning `float(gc)`: `I0404`,
                // with and without a user-defined `class float`.
                if let Some(Ok(arg_ty)) = &arg_terms[0]
                    && !matches!(arg_ty, Ty::Int | Ty::Float | Ty::Bool | Ty::Object)
                {
                    return Err(Diagnostic::error(
                        "T0021",
                        format!(
                            "`float` expects an `int`, `float`, or `bool` argument, got `{}`",
                            arg_ty.name()
                        ),
                        Span::new(0, 0),
                    )
                    .with_help("pass an `int`, `float`, or `bool` value"));
                }
                return Ok(Some(Ok(Ty::Float)));
            }
            if callee == "bool" && !signatures.contains_key(callee) {
                // Part 4 of #1026 (PR 4a of #1083): the solver-side mirror of
                // `infer_expr_in`'s `bool` arm, which owns the reasoning --
                // `Ty::Object` only, user-defined `bool` first.
                //
                // Unlike the `float` arm above, this one does not defer an
                // unresolved term. `bool` admits exactly one argument type,
                // so every other argument -- resolved to something else, or
                // not yet resolved at all -- falls through to
                // `unsupported_callable_builtin`'s C0001 below, which is how
                // issue #142 deliberately classifies a known callable builtin
                // in the solver. `float` can be lenient only because its arm
                // ends in an unconditional `Ty::Float`, so an unresolved term
                // there still has a result to fall through to; here there is
                // none.
                //
                // The user-defined-class guard `infer_expr_in`'s arm carries is
                // absent here for the reason the `float` arm above states: no
                // class table, and `I0404` keeps `Ty::Object` out of every
                // function body this solver runs on.
                if let [Some(Ok(Ty::Object))] = arg_terms.as_slice() {
                    return Ok(Some(Ok(Ty::Bool)));
                }
            }
            if (callee == "int" || callee == "str") && !signatures.contains_key(callee) {
                // Part 4 of #1026 (PR 4b of #1083): the solver-side mirrors of
                // `infer_expr_in`'s `int`/`str` arms, which own the reasoning
                // -- `Ty::Object` only, a user-defined `int`/`str` first.
                //
                // They follow the `bool` arm above rather than the `float`
                // one in not deferring an unresolved term, for the reason that
                // arm states: each admits exactly one argument type, so every
                // other argument -- resolved to something else or not yet
                // resolved at all -- falls through to
                // `unsupported_callable_builtin`'s `C0001` below, which is how
                // issue #142 deliberately classifies a known callable builtin
                // here.
                //
                // The user-defined-*class* guard `infer_expr_in`'s arms carry
                // is absent here for the reason the `float` arm above states:
                // the class table is not consulted here, and `I0404`
                // keeps a foreign object out of every function body it runs
                // on. `infer_expr_in` is the authority for a class-shadowed
                // name and refuses the program there.
                if let [Some(Ok(Ty::Object))] = arg_terms.as_slice() {
                    return Ok(Some(Ok(if callee == "int" { Ty::Int } else { Ty::Str })));
                }
            }
            // Part 2b of #1142 (#1164), the solver half of `crate::expr`'s
            // own buffer-returning-call refusal. The two must not drift: this
            // solver runs first, so without the mirror an intra-artifact call
            // to a buffer-returning export resolves to a `Ty::MemoryView`
            // term here and is reported as some later type error rather than
            // as the named capability gap it is.
            if let Some((_, _, Ok(Ty::MemoryView))) = signatures.get(callee) {
                return Err(crate::buffer::buffer_returning_call_unsupported(callee));
            }
            let Some(signature) = signatures.get(callee) else {
                // Issue #142: a private helper calling a known callable
                // builtin (e.g. `ValueError("x")`) gets the same `C0001`
                // classification as the final validation pass, rather than
                // deferring with `Ok(None)` -- the builtin genuinely exists
                // in Python 3.14, so it is a capability gap, not an
                // unresolved callee. A name that is neither a signature, a known
                // builtin, nor a user class still returns `Ok(None)` and
                // defers to final validation's `T0021`.
                //
                // Part 1 of #1319: the solver half of `crate::expr`'s
                // `frozenset(...)` arm. `signatures` has already missed, so a
                // `def frozenset` cannot reach here; `shadowed_producers`
                // carries a module `class frozenset`, which skips this arm and
                // falls through to `is_known_callable_builtin`'s `C0001`
                // below -- the pre-existing behavior every builtin-named
                // class shares (`class range:` reports the same), since the
                // class-constructor term below (#1342) is consulted only
                // after that `C0001`.
                if callee == crate::frozenset::FROZENSET
                    && !env.shadowed_producers.contains(callee.as_str())
                {
                    return Ok(Some(crate::frozenset::check_call_terms(
                        &arg_terms, parents, concrete, deferred,
                    )?));
                }
                // #1331: the solver half of `crate::expr`'s `hash(...)` arm,
                // guarded like the `frozenset` arm directly above and, like
                // `crate::expr`'s, against a stdlib module alias spelled
                // `hash`.
                if callee == crate::hash::HASH
                    && !env.shadowed_producers.contains(callee.as_str())
                    && !env
                        .std_module_aliases
                        .iter()
                        .any(|(alias, _)| alias == callee)
                {
                    return Ok(Some(Ok(crate::hash::check_call_terms(&arg_terms)?)));
                }
                // Part 11 of #1371: the solver half of `crate::expr`'s
                // `type(o)` arm, guarded like the `hash` arm above. An
                // argument whose term is still unresolved (an unannotated
                // parameter a caller passes an object to) is read as an
                // object, as `hash`'s arm reads it as hashable: the final
                // check pass re-types the call with the resolved argument
                // and refuses a native one with the known-builtin `C0001`.
                // A bare narrowed `object` read is the object itself (#1476).
                if !env.shadowed_producers.contains(callee.as_str())
                    && let [Some(arg_term)] = arg_terms.as_slice()
                    && crate::foreign::type_call::is_object_type_call(
                        &env.std_module_aliases,
                        callee,
                        &[match arg_term {
                            _ if object_narrow::is_bare_narrowed_read(env, &args[0]) => Ty::Object,
                            Ok(arg_ty) => arg_ty.clone(),
                            Err(_) => Ty::Object,
                        }],
                    )
                {
                    return Ok(Some(Ok(Ty::Object)));
                }
                if is_known_callable_builtin(callee) {
                    return Err(unsupported_callable_builtin(callee));
                }
                // #1342: `C(args)` on a non-generic user class answers its
                // instance; placed last so every reading above keeps
                // precedence (`constructor_call`'s module doc).
                return Ok(constructor_call::constructor_call_term(env, callee));
            };
            for (index, (arg, parameter)) in arg_terms.into_iter().zip(&signature.1).enumerate() {
                // Unify whenever either side is still an inference variable --
                // not just when the callee's own parameter is unresolved.
                // This used to only match `parameter: Err(_)`, so a concrete
                // (e.g. explicitly annotated) callee parameter never
                // constrained an unresolved *caller* argument variable in the
                // reverse direction, even though `unify_terms` itself already
                // handles that case symmetrically (self-review finding,
                // pre-merge).
                if let Some(arg) = arg
                    && matches!((&arg, parameter), (Err(_), _) | (_, Err(_)))
                {
                    // Defense in depth against a `Ty::Param` leak (finding
                    // #2, PR-13 fix round): this solver has no notion of
                    // `instantiate_generic_call` -- it constrains an
                    // unannotated parameter's fresh inference variable
                    // directly against whichever concrete side is
                    // available. When that "concrete" side is actually a
                    // still-generic function's own uninstantiated `x: T`
                    // parameter type, or a still-generic call's `-> T`
                    // return type flowing in as an argument, unifying it
                    // in would bind the unannotated parameter's resolved
                    // type to the raw internal `Ty::Param` representation
                    // -- which then surfaces to the user in a later
                    // diagnostic (e.g. an `Add`-not-defined error
                    // mentioning `T` instead of a real type). Reject this
                    // shape here, before that leak can happen, with a
                    // clear, dedicated diagnostic instead.
                    if matches!(&arg, Ok(ty) if ty_contains_param(ty))
                        || matches!(parameter, Ok(ty) if ty_contains_param(ty))
                    {
                        return Err(Diagnostic::error(
                            "T0042",
                            format!(
                                "cannot infer the type of argument {} of private helper `{callee}` from a generic function's uninstantiated type; add an explicit type annotation",
                                index + 1
                            ),
                            Span::new(0, 0),
                        ).with_help("add an explicit type annotation"));
                    }
                    unify_terms(
                        parameter.clone(),
                        arg,
                        parents,
                        concrete,
                        "T0021",
                        &format!("argument {} of private helper `{callee}`", index + 1),
                    )?;
                }
            }
            Ok(Some(signature.2.clone()))
        }
        // D-146 (#239): `TypeTerm` (`Result<Ty, usize>`) has no unification-
        // friendly representation for `Ty::List` -- this solver only exists to
        // infer scalar `Ty::Infer` parameters/returns of underscore-prefixed
        // private helpers (D-045), and list homogeneity/element-type checking
        // is `infer_expr_in`'s job, not this constraint collector's (see that
        // function's `HirExpr::ListLiteral`/`Subscript`/`ListAppend` arms
        // below). Recurse into every element to keep propagating genuine
        // errors (e.g. an unbound local used as a list element). When every
        // element produces `Some(Ok(ty))`, all element types are exactly equal
        // (exact `Ty` equality, matching `infer_expr_in`'s own homogeneity
        // rule -- NOT `merge_inferred_types`, which would silently widen
        // `bool` to `int`), and the shared element type is a private-solver
        // scalar (`is_private_solver_scalar` -- `Ty::Int`/`Ty::Float`/
        // `Ty::Bool`/`Ty::Str`/`Ty::None`), return `Some(Ok(Ty::List(...)))`
        // as a destructured element-type carrier -- never unified, only
        // destructured by the `Subscript`/`ListPop` arms below to extract the
        // scalar element type for a scalar return-type inference. The
        // `is_private_solver_scalar` gate prevents nested-container carriers
        // (`list[list[int]]`) and non-scalar element types this solver has no
        // representation for. Heterogeneous lists, empty lists, non-scalar
        // element lists, or any element producing `None`/`Err` keep the
        // historical `Ok(None)` behavior -- returning `Err` here for a case
        // this solver can't actually validate would wrongly preempt
        // `checked_function_signatures_all`'s fallback to the real, list-aware
        // check pass (`check_with_signatures_all`) that runs after this solver.
        // `unify_terms` and `merge_inferred_types` are unchanged -- the
        // carrier is destructured, never unified.
        // #1021: the empty-container pre-pass rewrites a resolvable `[]`
        // into `EmptyList(element_ty)`, so this arm carries the same
        // destructured element-type carrier the `ListLiteral` arm below
        // produces for the equivalent `[v]`. Without it, an unannotated
        // private helper writing `xs = []; xs.append(1); return xs.pop()`
        // leaves its return type unconstrained and reports `T0021`, while
        // the `xs = [1]` spelling infers cleanly -- an asymmetry introduced
        // by #1021's own feature. The `is_private_solver_scalar` gate is the
        // same one `homogeneous_private_solver_scalar_list_element` applies:
        // `EmptyList` hands over its element `Ty` directly, with no element
        // terms to check, so a nested `list[list[int]]` or a `Ty::Param`
        // element -- both of which reach this solver before the D-228
        // element gate fires -- must keep the historical `Ok(None)`. The
        // carrier is destructured by the `Subscript`/`ListPop` arms, never
        // unified. `EmptyDict` stays `Ok(None)` because the `DictLiteral`
        // arm below never produces a term either.
        HirExpr::EmptyList(element_ty) => {
            if is_private_solver_scalar(element_ty) {
                Ok(Some(Ok(Ty::List(Box::new(element_ty.clone())))))
            } else {
                Ok(None)
            }
        }
        HirExpr::EmptyDict(_) => Ok(None),
        // Part 2d of #1371: a list display built as a CPython `list` is an
        // object whatever its elements are; their packability is
        // `crate::foreign::list_display`'s check, run by the checker walk.
        HirExpr::ObjectList(elements) => {
            for element in elements {
                collect_expr_constraints(signatures, parents, concrete, deferred, env, element)?;
            }
            Ok(Some(Ok(Ty::Object)))
        }
        HirExpr::ListLiteral(elements) => {
            let mut element_terms = Vec::with_capacity(elements.len());
            for element in elements {
                element_terms.push(collect_expr_constraints(
                    signatures, parents, concrete, deferred, env, element,
                )?);
            }
            if let Some(element_ty) = homogeneous_private_solver_scalar_list_element(&element_terms)
            {
                Ok(Some(Ok(Ty::List(Box::new(element_ty)))))
            } else {
                Ok(None)
            }
        }
        HirExpr::Subscript { base, index } => {
            // Part 2 of #1027, the solver half of `crate::expr`'s own
            // `Subscript` interception: `b[i]` on a `memoryview`-bound name
            // is a `Ty::Float` element load. It must run before the base
            // recursion below, because that recursion reaches the `Name`
            // seam above, which calls `reject_memoryview_read` and would
            // report the `C0001` capability gap for the expression that
            // closes it.
            //
            // The binding is read raw out of `env.bindings` and resolved
            // with `resolved_term`, the same shape that seam uses. The
            // index is still collected -- an undefined name or an
            // unsatisfiable constraint inside it is a real error and must
            // surface -- but its *term* is discarded exactly as the list
            // path discards it: the index type gate is the check phase's.
            //
            // Still only a bare `Name` base after Part 2a of #1142 (#1165),
            // for the reason `crate::expr`'s own arm records: the producer
            // is admitted only as an assignment's whole right-hand side, so
            // `ndarray(4)[0]` is refused by this walker's `Call` arm before
            // the base recursion below can reach it.
            if let HirExpr::Name(buffer_name) = base.as_ref()
                && let Some(term) = env.bindings.get(buffer_name).cloned()
                && let Some(Ty::MemoryView) = resolved_term(term, parents, concrete)
            {
                collect_expr_constraints(signatures, parents, concrete, deferred, env, index)?;
                return Ok(Some(Ok(Ty::Float)));
            }
            let base_term =
                collect_expr_constraints(signatures, parents, concrete, deferred, env, base)?;
            collect_expr_constraints(signatures, parents, concrete, deferred, env, index)?;
            // D-146 (#239): when the base resolves to a `Ty::List` element-
            // type carrier (produced by the `ListLiteral` arm above or a
            // `Ty::List`-bound name), extract the scalar element type -- the
            // carrier is destructured, never unified. Otherwise keep the
            // historical `Ok(None)` behavior (the base/index recursion above
            // already propagated genuine errors).
            // Part 3 of #1026 (PR 3b of #1082): `o[k]` is a term, not a
            // hole, on exactly the `AttrGet` arm's own reasoning below --
            // outside an `--ext` module `object` is unspellable in an
            // annotation (`docs/TYPE_SYSTEM.md`'s `object` row; D-258 lets
            // an ext module write `Any`/`object`), so discarding the term
            // would leave an unannotated
            // `def _h(): return gc.garbage[0]` reporting a `T0021` asking
            // for an annotation no source can write, in place of the
            // `I0404` the check phase once reported for the read itself.
            // Since Part 1 of #1333 the helper returning it is admitted.
            // Changing this arm without the `AttrGet` one, or the reverse,
            // is the drift both comments exist to prevent.
            if matches!(base_term, Some(Ok(Ty::Object))) {
                return Ok(Some(Ok(Ty::Object)));
            }
            if let Some(Ok(Ty::List(element_ty))) = base_term {
                Ok(Some(Ok(*element_ty)))
            } else {
                Ok(None)
            }
        }
        // PR-12 Task 7 (D-118): structurally identical to `Subscript` above
        // -- a slice's base/bounds are, like a subscript's base/index,
        // ordinary sub-expressions this solver needs to keep walking into
        // (e.g. `some_param[1:3]` inside a private helper, where
        // `some_param`'s own type is exactly what the solver is trying to
        // pin down), but a `Ty::List`/`Ty::Int` base-type or bound-type gate
        // is `infer_expr_in`'s job, not this constraint collector's. Recurse
        // into `base` and every present bound only to keep propagating
        // genuine errors (e.g. an unbound local used as a bound); produce no
        // term for the `Slice` expression's own overall type.
        HirExpr::Slice {
            base,
            start,
            stop,
            step,
        } => {
            let base_term =
                collect_expr_constraints(signatures, parents, concrete, deferred, env, base)?;
            for bound in [start, stop, step].into_iter().flatten() {
                collect_expr_constraints(signatures, parents, concrete, deferred, env, bound)?;
            }
            // Part 2b of #1371: `o[a:b]` is a term on the `Subscript` arm's
            // own reasoning above -- an unannotated helper returning it
            // must not ask for an annotation.
            if matches!(base_term, Some(Ok(Ty::Object))) {
                return Ok(Some(Ok(Ty::Object)));
            }
            Ok(None)
        }
        // #1263: an attribute receiver is collected first (CPython's
        // order), only to propagate its own genuine errors.
        HirExpr::ListAppend { list, value } => {
            for sub in list.attr_expr().into_iter().chain([value.as_ref()]) {
                collect_expr_constraints(signatures, parents, concrete, deferred, env, sub)?;
            }
            Ok(None)
        }
        // Same reasoning as `ListLiteral` above (PR-11 Task 3): dict
        // key/value homogeneity and the `dict[str, int]`-only gate are
        // `infer_expr_in`'s job, not this solver's. Recurse into every
        // key and value only to keep propagating genuine errors.
        HirExpr::DictLiteral(pairs) => {
            for (key, value) in pairs {
                collect_expr_constraints(signatures, parents, concrete, deferred, env, key)?;
                collect_expr_constraints(signatures, parents, concrete, deferred, env, value)?;
            }
            Ok(None)
        }
        // Same reasoning as `ListLiteral`/`DictLiteral` above (PR-11 Task
        // 7): set element homogeneity and the `set[int]`-only gate are
        // `infer_expr_in`'s job, not this solver's. Recurse into every
        // element only to keep propagating genuine errors.
        HirExpr::SetLiteral(elements) => {
            for element in elements {
                collect_expr_constraints(signatures, parents, concrete, deferred, env, element)?;
            }
            Ok(None)
        }
        // Same reasoning as `ListLiteral`/`DictLiteral`/`SetLiteral` above
        // (PR-11b Task 3, D-116): the per-element int/bool/float membership
        // gate is `infer_expr_in`'s job, not this solver's. Recurse into
        // every element only to keep propagating genuine errors.
        HirExpr::TupleLiteral(elements) => {
            for element in elements {
                collect_expr_constraints(signatures, parents, concrete, deferred, env, element)?;
            }
            Ok(None)
        }
        // PR-12 Task 10 (D-119): the base-type gate (`T0033`) and the
        // value/key/default-type gate (`T0021`) are `infer_expr_in`'s job,
        // not this solver's -- same reasoning as `ListAppend` above. `list`
        // is a plain `String`, not a sub-expression, so there is nothing to
        // recurse into for `ListPop`. D-146 (#239): when `list` is bound in
        // `env.bindings` to a `Ty::List` element-type carrier (produced by
        // the `ListLiteral` arm above or a `Ty::List`-typed parameter),
        // extract the scalar element type -- the carrier is destructured,
        // never unified. Otherwise keep the historical `Ok(None)` behavior.
        //
        // #1263: an attribute receiver (`self.xs.pop()`) gives no term, the
        // same as this solver's `AttrGet` arm gives an instance attribute
        // read in any other position (`self.xs[i]`): an unannotated private
        // helper returning it asks for an annotation (`T0021`), and the
        // check phase still types the node against the real slot type.
        HirExpr::ListPop {
            list: ContainerReceiver::Name(list),
        } => {
            if let Some(Ok(Ty::List(element_ty))) = env.bindings.get(list).cloned() {
                Ok(Some(Ok(*element_ty)))
            } else {
                Ok(None)
            }
        }
        HirExpr::ListPop {
            list: ContainerReceiver::Attr(receiver),
        } => {
            collect_expr_constraints(signatures, parents, concrete, deferred, env, receiver)?;
            Ok(None)
        }
        HirExpr::DictGetOrDefault { dict, key, default } => {
            for sub in dict
                .attr_expr()
                .into_iter()
                .chain([key.as_ref(), default.as_ref()])
            {
                collect_expr_constraints(signatures, parents, concrete, deferred, env, sub)?;
            }
            Ok(None)
        }
        HirExpr::SetAdd { set: _, value } => {
            collect_expr_constraints(signatures, parents, concrete, deferred, env, value)?;
            Ok(None)
        }
        // D-154 (Part 1 of #375): same reasoning as `Subscript`/`ListPop`
        // above -- the base-instance-type gate and attribute/method
        // resolution are `infer_expr_in`'s job, not this solver's (which
        // only ever runs for a *private, unannotated* function's body).
        // Recurse into `base` (and, for `MethodCall`, every argument) only
        // to keep propagating genuine errors; produce no term for either
        // expression's own overall type, mirroring `HirExpr::Subscript`'s
        // own pre-D-146 "no unification term" default and `ListPop`'s own
        // doc comment's documented consequence (a private function
        // assigning from one of these expressions registers no real type
        // term for the target -- a pre-existing, not novel, gap this
        // project already accepts for every other container-shaped
        // expression). Issue #771/D-199: the target is still tracked as
        // definitely-but-opaquely bound (`opaque_bindings`), so a later
        // read of it no longer misfires as an unbound local; only the
        // missing type term itself remains unchanged.
        HirExpr::AttrGet { base, .. } => {
            let base_term =
                collect_expr_constraints(signatures, parents, concrete, deferred, env, base)?;
            // Part 2 of #1026 (#1081): `numpy.pi` is a term, not a hole.
            // Discarding it here would recreate one level up the exact dead
            // end the `Name` arm above was carved out to avoid: an
            // unannotated `def _helper(): return numpy.pi` would leave the
            // return variable unresolved and signature materialization
            // would report `T0021: ... add an annotation` -- advice no
            // annotation can satisfy in a native build, because there
            // `object` is unspellable (`annotation_to_ty` rejects it with
            // `C0001` outside an `--ext` module, D-258; since Part 1 of
            // #1367 only a class a foreign import binds resolves to it, and
            // `numpy.pi` is no class). Offering the
            // term lets the return materialize as `Ty::Object`. Since Part 1
            // of #1333 the check phase admits a function body binding or
            // returning the value, so the term keeps the program admitted.
            //
            // Only `Ty::Object` is lifted. Every other base keeps the
            // container-shaped "no unification term" default described
            // above, which the rest of this arm's contract rests on.
            if matches!(base_term, Some(Ok(Ty::Object))) {
                return Ok(Some(Ok(Ty::Object)));
            }
            Ok(None)
        }
        HirExpr::MethodCall { base, method, args } => {
            let base_term =
                collect_expr_constraints(signatures, parents, concrete, deferred, env, base)?;
            for arg in args {
                collect_expr_constraints(signatures, parents, concrete, deferred, env, arg)?;
            }
            // Part 1 of #1333: `o.method(...)` on a concrete `object`
            // receiver answers `object`, on the `AttrGet` and `Subscript`
            // arms' own reasoning above; the three arms change together.
            // #1420: on a concrete user-class instance it answers the
            // resolved method's return term.
            Ok(
                object_lift::method_call_on_object(base_term.as_ref()).or_else(|| {
                    method_return::method_call_on_instance(
                        signatures,
                        env,
                        base_term.as_ref(),
                        method,
                    )
                }),
            )
        }
        // Issue #1188: this solver never picks a reading and never rejects.
        // An admitted container reading is collected exactly as the
        // container node it would have been (so a `ListPop` still yields its
        // element type); a refused one is collected as the method call.
        // Issue #1095: so is an admitted one whose bare-name receiver is
        // bound to a concrete `object` term, so `o.pop()` answers `object`
        // on the `MethodCall` arm's own reasoning.
        HirExpr::ReceiverDispatchedCall { call, container } => {
            let reading = match container {
                pycc_hir::ContainerFallback::Admitted
                    if matches!(
                        call.method_receiver(),
                        Some((HirExpr::Name(name), _))
                            if matches!(env.bindings.get(name), Some(Ok(Ty::Object)))
                    ) =>
                {
                    None
                }
                pycc_hir::ContainerFallback::Admitted => call.container_form(),
                pycc_hir::ContainerFallback::Refused(_) => None,
            };
            collect_expr_constraints(
                signatures,
                parents,
                concrete,
                deferred,
                env,
                reading.as_ref().unwrap_or(call),
            )
        }
        // PEP 695 (#387): `C[type_arg](args)` — a generic class
        // instantiation. The args are recursed into for constraint
        // collection. The expression itself produces a concrete
        // `Ty::Instance(class)` type term (not `Ok(None)` like
        // container-shaped expressions), because a GCI's result is a
        // class instance that the solver needs to track so a local
        // assigned from it (`b = C[int](1)`) gets bound in the solver's
        // environment. The type_arg is a compile-time `Ty`, not a runtime
        // expression, so it needs no constraint traversal. The actual
        // type-checking (verifying the class is generic, the type_arg is
        // scalar, and the args match `__init__`) happens in `infer_expr_in`,
        // not here.
        HirExpr::GenericClassInstantiate { class, args, .. } => {
            for arg in args {
                collect_expr_constraints(signatures, parents, concrete, deferred, env, arg)?;
            }
            Ok(Some(Ok(Ty::Instance(Box::new(class.clone())))))
        }
        // Part 2a of #1371: `table[k](args)`. A callee whose term is a
        // concrete `object` answers `object`, on the `MethodCall` arm's own
        // reasoning; any other callee yields no term and the check phase
        // refuses it (`foreign::subscript_call`).
        HirExpr::ExprCall { callee, args } => {
            let callee_term =
                collect_expr_constraints(signatures, parents, concrete, deferred, env, callee)?;
            for arg in args {
                collect_expr_constraints(signatures, parents, concrete, deferred, env, arg)?;
            }
            Ok(object_lift::method_call_on_object(callee_term.as_ref()))
        }
        // Part 8 of #1371: collected as its positional half, whose term it
        // answers (an `object` callee answers `object` through that half's
        // own arm), plus each keyword value. This solver never refuses the
        // keywords; the check phase does, for every non-object callee
        // (`foreign::keyword_call`).
        HirExpr::KeywordCall { call, keywords, .. } => {
            let term =
                collect_expr_constraints(signatures, parents, concrete, deferred, env, call)?;
            for (_, value) in keywords {
                collect_expr_constraints(signatures, parents, concrete, deferred, env, value)?;
            }
            Ok(term)
        }
        // #1411: `type(self)(args)` constructs the class `self` is typed
        // as, so its term is `self`'s own; an unbound `self` (a
        // `@classmethod` or `@staticmethod`) offers none and the check
        // phase refuses the call (`class::receiver_class_call`).
        HirExpr::ReceiverClassCall { args } => {
            for arg in args {
                collect_expr_constraints(signatures, parents, concrete, deferred, env, arg)?;
            }
            Ok(env.bindings.get("self").cloned())
        }
        // #433: `Super` carries no sub-expressions to recurse into and
        // produces no unification term — it is a compile-time marker only
        // meaningful as the base of a `MethodCall`/`AttrGet`, which the
        // solver's own `MethodCall`/`AttrGet` arms already recurse past
        // (both return `Ok(None)` after recursing into `base`).
        HirExpr::Super => Ok(None),
        // PEP 572 (#774), deep-review follow-up (round 4): `target :=
        // value`. `name`'s own binding is registered by
        // `bind_named_expr_targets`, a separate pre-pass `collect_block_
        // constraints`'s own `If`/`While`/`ExprStmt` arms run over the whole
        // test/expression *before* reaching this function -- this function
        // takes `env: &ConstraintEnvironment`, immutable, since nearly every
        // other arm here only ever recurses for constraints without
        // touching bindings, so binding here directly is not possible
        // without widening every call site to `&mut` for this one arm's
        // sake. `value` is still recursed into here for its own constraint
        // collection (re-deriving the same constraints `bind_named_expr_
        // targets` already collected for it, exactly as `crate::
        // collect_named_expr_bindings`'s own doc comment documents and
        // accepts for the same reason in the full flow-sensitive checker),
        // and its term is propagated as this expression's own overall term,
        // since a walrus's value *is* the expression's value (mirroring
        // `infer_expr_in`'s own `NamedExpr` arm exactly).
        HirExpr::NamedExpr { name: _, value } => {
            collect_expr_constraints(signatures, parents, concrete, deferred, env, value)
        }
        // #1254 (D-250): the loop variable is bound in a clone, so it never
        // becomes a binding of the enclosing scope. `bind_comp_loop_var`
        // collects the range operands itself, against the enclosing
        // bindings (the clone is still identical to `env` at that point).
        // The element gate is `infer_expr_in`'s, as for a display; the
        // container type is returned so `return [..]` from an unannotated
        // helper still infers its return type.
        HirExpr::Comprehension(comp) => {
            let mut scoped = env.clone();
            bind_comp_loop_var(
                signatures,
                parents,
                concrete,
                deferred,
                &mut scoped,
                &comp.var,
                &comp.iter,
            )?;
            let mut elt_term = None;
            for sub in comp.body_exprs() {
                elt_term = collect_expr_constraints(
                    signatures, parents, concrete, deferred, &scoped, sub,
                )?;
            }
            // Part 1 of #1255: a comprehension over a CPython object
            // produces a CPython object, whatever its element.
            if let CompIter::Iterable(_) = comp.iter {
                return Ok(Some(Ok(Ty::Object)));
            }
            // A set comprehension's container term is chosen by
            // [`set_comp::set_comp_container`] (#1343, #1344), which owns the
            // rule. The element is the last body expression (`body_exprs`
            // puts the condition first).
            if let CompElt::Set(_) = comp.elt {
                return Ok(Some(set_comp::set_comp_container(
                    elt_term, parents, concrete, deferred,
                )));
            }
            Ok(Some(Ok(crate::comprehension::comp_container_of(&comp.elt))))
        }
    }
}

/// PEP 572 (#774), deep-review follow-up (round 4): binds each walrus
/// target's own inferred type into the solver's environment before the
/// containing `If`/`While` test or bare `ExprStmt` is walked for
/// unification by `collect_expr_constraints` above -- mirroring `crate::
/// collect_named_expr_bindings`'s own pre-pass pattern in the full
/// flow-sensitive checker exactly, including its documented tradeoff of
/// recomputing `value`'s constraints rather than threading the term through
/// from this pre-pass (see that function's own doc comment). Without this,
/// a walrus target reaches `local_names` (via `collect_named_expr_names_in_
/// expr`) but never gets a `bindings` entry, so a later read of that name
/// wrongly fails `collect_expr_constraints`'s own `Name` arm's `unbound_
/// local` check even though the walrus unconditionally executes whenever
/// the statement containing it is reached.
///
/// Walked in the same left-to-right evaluation order as `collect_expr_
/// constraints`'s own traversal, so a later sibling sub-expression that
/// reads an earlier walrus-bound name (`(a := 1) + (b := a + 1)`) resolves
/// correctly. Each target is bound as a *definite* binding -- not a
/// maybe-binding -- mirroring the `Assign` arm above exactly (`defs_
/// rebound.remove`, `maybe_bindings.remove`, `bindings.entry(name).or_
/// insert(term)`): a walrus nested anywhere in an `if`/`while` test
/// executes whenever the statement itself is reached, unlike a name
/// assigned in only one branch of the `if`, which is bound separately by
/// `collect_block_constraints`'s own branch-join logic, not by this walk.
fn bind_named_expr_targets(
    signatures: &HashMap<String, SignatureTerms>,
    parents: &mut Vec<usize>,
    concrete: &mut Vec<Option<Ty>>,
    deferred: &mut DeferredConstraints,
    env: &mut ConstraintEnvironment<'_, '_>,
    expr: &HirExpr,
) -> Result<(), Diagnostic> {
    match expr {
        HirExpr::NamedExpr { name, value } => {
            bind_named_expr_targets(signatures, parents, concrete, deferred, env, value)?;
            // Issue #771 (D-199), mirrored here for the walrus operator: a
            // `:=` target is unconditionally (re)bound wherever it is
            // evaluated, so any earlier `def` shadow, maybe-bound marker, or
            // stale opaque marker for `name` is cleared regardless of what
            // the new value's term turns out to be below.
            env.defs_rebound.remove(name.as_str());
            env.maybe_bindings.remove(name.as_str());
            env.opaque_bindings.remove(name.as_str());
            let term =
                collect_expr_constraints(signatures, parents, concrete, deferred, env, value)?;
            // Round-11 review finding 2: a walrus rebinds its target, so it
            // drops any artifact-owned buffer provenance the name carried,
            // exactly as the `Assign` arm does and after the value is
            // collected for the same reason.
            env.rebind_over_owned_buffer(name);
            if let Some(term) = term {
                env.bindings.entry(name.clone()).or_insert(term);
            } else {
                // The solver produced no term for the walrus value (e.g. a
                // class-target `cast`, `.pop()`, an attribute read, a method
                // call, or a heterogeneous container literal). Track `name`
                // as opaquely-but-definitely bound so a later read reports
                // "no term" instead of misreporting it as unbound.
                env.opaque_bindings.insert(name.clone());
            }
            Ok(())
        }
        HirExpr::IntLiteral(_)
        | HirExpr::FloatLiteral(_)
        | HirExpr::BoolLiteral(_)
        | HirExpr::StringLiteral(_)
        | HirExpr::EmptyList(_)
        | HirExpr::EmptyDict(_)
        | HirExpr::NoneLiteral
        | HirExpr::NotImplemented
        | HirExpr::Name(_)
        | HirExpr::Super => Ok(()),
        HirExpr::ListPop { list } => {
            if let Some(receiver) = list.attr_expr() {
                bind_named_expr_targets(signatures, parents, concrete, deferred, env, receiver)?;
            }
            Ok(())
        }
        // #1254 (D-250): lowering refuses a walrus inside a comprehension.
        HirExpr::Comprehension(_) => Ok(()),
        HirExpr::Call { args, .. } => {
            for arg in args {
                bind_named_expr_targets(signatures, parents, concrete, deferred, env, arg)?;
            }
            Ok(())
        }
        HirExpr::BinOp { left, right, .. }
        | HirExpr::Compare { left, right, .. }
        | HirExpr::BoolOp { left, right, .. } => {
            bind_named_expr_targets(signatures, parents, concrete, deferred, env, left)?;
            bind_named_expr_targets(signatures, parents, concrete, deferred, env, right)
        }
        HirExpr::IfExp { test, body, orelse } => {
            for part in [test, body, orelse] {
                bind_named_expr_targets(signatures, parents, concrete, deferred, env, part)?;
            }
            Ok(())
        }
        HirExpr::CompareChain { first, links } => {
            for operand in pycc_hir::compare_chain_operands(first, links) {
                bind_named_expr_targets(signatures, parents, concrete, deferred, env, operand)?;
            }
            Ok(())
        }
        HirExpr::UnaryOp { operand, .. } | HirExpr::Unpack { value: operand, .. } => {
            bind_named_expr_targets(signatures, parents, concrete, deferred, env, operand)
        }
        HirExpr::FString(parts) => {
            for part in parts {
                if let FStringPart::Interpolation(inner) = part {
                    bind_named_expr_targets(signatures, parents, concrete, deferred, env, inner)?;
                }
            }
            Ok(())
        }
        HirExpr::ListLiteral(es)
        | HirExpr::ObjectList(es)
        | HirExpr::SetLiteral(es)
        | HirExpr::TupleLiteral(es) => {
            for e in es {
                bind_named_expr_targets(signatures, parents, concrete, deferred, env, e)?;
            }
            Ok(())
        }
        HirExpr::Subscript { base, index } => {
            bind_named_expr_targets(signatures, parents, concrete, deferred, env, base)?;
            bind_named_expr_targets(signatures, parents, concrete, deferred, env, index)
        }
        HirExpr::Slice {
            base,
            start,
            stop,
            step,
        } => {
            bind_named_expr_targets(signatures, parents, concrete, deferred, env, base)?;
            for bound in [start, stop, step].into_iter().flatten() {
                bind_named_expr_targets(signatures, parents, concrete, deferred, env, bound)?;
            }
            Ok(())
        }
        HirExpr::ListAppend { list, value } => {
            for sub in list.attr_expr().into_iter().chain([value.as_ref()]) {
                bind_named_expr_targets(signatures, parents, concrete, deferred, env, sub)?;
            }
            Ok(())
        }
        HirExpr::SetAdd { value, .. } => {
            bind_named_expr_targets(signatures, parents, concrete, deferred, env, value)
        }
        HirExpr::DictLiteral(pairs) => {
            for (k, v) in pairs {
                bind_named_expr_targets(signatures, parents, concrete, deferred, env, k)?;
                bind_named_expr_targets(signatures, parents, concrete, deferred, env, v)?;
            }
            Ok(())
        }
        HirExpr::DictGetOrDefault { dict, key, default } => {
            for sub in dict
                .attr_expr()
                .into_iter()
                .chain([key.as_ref(), default.as_ref()])
            {
                bind_named_expr_targets(signatures, parents, concrete, deferred, env, sub)?;
            }
            Ok(())
        }
        HirExpr::AttrGet { base, .. } => {
            bind_named_expr_targets(signatures, parents, concrete, deferred, env, base)
        }
        HirExpr::MethodCall { base, args, .. } => {
            bind_named_expr_targets(signatures, parents, concrete, deferred, env, base)?;
            for arg in args {
                bind_named_expr_targets(signatures, parents, concrete, deferred, env, arg)?;
            }
            Ok(())
        }
        HirExpr::ReceiverDispatchedCall { call, .. } => {
            bind_named_expr_targets(signatures, parents, concrete, deferred, env, call)
        }
        HirExpr::KeywordCall { call, keywords, .. } => {
            bind_named_expr_targets(signatures, parents, concrete, deferred, env, call)?;
            for (_, value) in keywords {
                bind_named_expr_targets(signatures, parents, concrete, deferred, env, value)?;
            }
            Ok(())
        }
        // #1411: `type(self)(args)` walks its arguments the same way.
        HirExpr::GenericClassInstantiate { args, .. } | HirExpr::ReceiverClassCall { args } => {
            for arg in args {
                bind_named_expr_targets(signatures, parents, concrete, deferred, env, arg)?;
            }
            Ok(())
        }
        HirExpr::ExprCall { callee, args } => {
            bind_named_expr_targets(signatures, parents, concrete, deferred, env, callee)?;
            for arg in args {
                bind_named_expr_targets(signatures, parents, concrete, deferred, env, arg)?;
            }
            Ok(())
        }
    }
}

/// Binds a comprehension's synthesized loop variable (PR-12 Task 3, D-117)
/// in the constraint solver's environment, mirroring `collect_block_constraints`'s
/// own `ForRange`/`ForList` arms exactly rather than duplicating their logic
/// a third time: a `CompIter::Range` iterable gives the loop variable the
/// concrete `Ty::Int` fact (unifying against any existing term, exactly like
/// `ForRange`'s own loop variable), while a `CompIter::Name` iterable gives
/// it a fresh, unconstrained term (exactly like `ForList`'s own loop
/// variable) since this solver doesn't track a list/dict/set-typed name's
/// element type at all. Shared by all three comprehension statement kinds.
fn bind_comp_loop_var(
    signatures: &HashMap<String, SignatureTerms>,
    parents: &mut Vec<usize>,
    concrete: &mut Vec<Option<Ty>>,
    deferred: &mut DeferredConstraints,
    env: &mut ConstraintEnvironment<'_, '_>,
    var: &str,
    iter: &CompIter,
) -> Result<(), Diagnostic> {
    match iter {
        CompIter::Range { start, stop, step } => {
            for (position, expr) in [("start", start), ("stop", stop), ("step", step)] {
                if let Some(term @ Err(_)) =
                    collect_expr_constraints(signatures, parents, concrete, deferred, env, expr)?
                {
                    unify_terms(
                        term,
                        Ok(Ty::Int),
                        parents,
                        concrete,
                        "T0021",
                        &format!("comprehension range {position}"),
                    )?;
                }
            }
            if let Some(existing) = env.bindings.get(var).cloned() {
                unify_terms(
                    existing,
                    Ok(Ty::Int),
                    parents,
                    concrete,
                    "T0023",
                    &format!("assignment to comprehension loop variable `{var}`"),
                )?;
            } else {
                env.bindings.insert(var.to_string(), Ok(Ty::Int));
            }
        }
        CompIter::Name(_) => {
            if !env.bindings.contains_key(var) {
                let term = fresh_term(parents, concrete);
                env.bindings.insert(var.to_string(), term);
            }
        }
        // Part 1 of #1255: the iterable's own constraints are collected
        // against the enclosing bindings; its loop variable is a CPython
        // object, which the check phase verifies.
        CompIter::Iterable(iterable) => {
            collect_expr_constraints(signatures, parents, concrete, deferred, env, iterable)?;
            env.bindings.insert(var.to_string(), Ok(Ty::Object));
        }
    }
    Ok(())
}

/// Binds a comprehension statement's `target` (`name = <comp>`) exactly as
/// the `Assign` arm binds a plain assignment's target, with the produced
/// container type as its term (#1254). Before this, the solver bound no term
/// for `target` at all, so any later read of it in a module that also holds
/// an unannotated private helper (the only case the solver runs for) failed
/// with a spurious `T0021` "not bound before this use". The container term
/// is the one the comprehension's own arm produced: see
/// [`crate::comprehension::comp_container_ty`] for the element gate the
/// check phase applies and [`set_comp::set_comp_container`] for a set
/// comprehension's term.
fn bind_comp_target(env: &mut ConstraintEnvironment<'_, '_>, target: &str, container: TypeTerm) {
    env.defs_rebound.remove(target);
    env.maybe_bindings.remove(target);
    env.opaque_bindings.remove(target);
    env.rebind_over_owned_buffer(target);
    env.bindings.entry(target.to_string()).or_insert(container);
}

pub(crate) fn collect_block_constraints(
    signatures: &HashMap<String, SignatureTerms>,
    parents: &mut Vec<usize>,
    concrete: &mut Vec<Option<Ty>>,
    constraints: &mut SolverConstraints,
    env: &mut ConstraintEnvironment<'_, '_>,
    body: &[HirStmt],
    return_term: Option<TypeTerm>,
) -> Result<(), Diagnostic> {
    // #1476: the narrowing overlay is updated after each statement at the
    // top of the next iteration, because several arms below `continue`.
    let mut previous: Option<&HirStmt> = None;
    for stmt in body {
        if let Some(previous) = previous {
            object_narrow::after_statement(signatures, env, previous);
        }
        object_narrow::before_statement(env, stmt);
        previous = Some(stmt);
        match stmt {
            HirStmt::Assign { target, value } => {
                // A value assignment re-shadows any earlier same-named `def`
                // (D-110), independent of the first-term-wins rule below.
                env.defs_rebound.remove(target.as_str());
                // Issue #359 (Part 2 of #118): an unconditional assignment
                // upgrades a maybe-bound name back to definitely bound
                // (mirrors the validation pass's contract: `if c: x = 1`
                // followed by `x = 2` makes `x` readable).
                env.maybe_bindings.remove(target.as_str());
                // Issue #771: this is a fresh unconditional (re)assignment,
                // so any previous opaque marker for `target` is stale
                // regardless of what the new initializer's term turns out
                // to be — it is either replaced by a real term below, or
                // reinstated as opaque by the `else` arm.
                env.opaque_bindings.remove(target.as_str());
                // Part 2a of #1142 (#1165): the solver's one admitting seam
                // for the buffer producer, mirroring `crate::check_stmt`'s.
                // The length argument is still collected so its own
                // constraints (and its own diagnostics) are not skipped.
                if let Some((callee, len_arg)) = resolved_producer_call(signatures, env, value) {
                    // Guard first, matching `check_assignment`'s order: a
                    // parameter rebinding is refused before the length
                    // argument's own constraints are collected.
                    reject_buffer_parameter_rebinding(env, target)?;
                    reject_final_rebinding(env, target)?;
                    // The collected term is *used*, not discarded: it is the
                    // length type the check phase's own seam refuses when it
                    // is not an `int`. See `reject_non_int_producer_length`,
                    // and `resolved_producer_call` for why an admitted set
                    // wider than the check phase's shows up as a wrong
                    // message rather than a wrong program.
                    let len_term = collect_expr_constraints(
                        signatures,
                        parents,
                        concrete,
                        &mut constraints.deferred,
                        env,
                        len_arg,
                    )?;
                    reject_non_int_producer_length(callee, len_term, parents, concrete)?;
                    env.bindings
                        .entry(target.clone())
                        .or_insert(Ok(Ty::MemoryView));
                    env.owned_buffers.insert(target.clone());
                    continue;
                }
                let term = collect_expr_constraints(
                    signatures,
                    parents,
                    concrete,
                    &mut constraints.deferred,
                    env,
                    value,
                )?;
                // Round-11 review finding 2: the initializer is collected
                // first and the stale provenance dropped only afterwards,
                // because the initializer may still read the buffer the
                // name is about to stop denoting (`a = a[0]`).
                env.rebind_over_owned_buffer(target);
                // #1476: a first binding from a bare narrowed read keeps the
                // `object` term (`object_narrow::is_bare_narrowed_read`).
                let term = if object_narrow::is_bare_narrowed_read(env, value) {
                    Some(Ok(Ty::Object))
                } else {
                    term
                };
                if let Some(term) = term {
                    env.bindings.entry(target.clone()).or_insert(term);
                } else {
                    // The target is unconditionally assigned, but the
                    // solver produced no term for the initializer (e.g. a
                    // class-target `cast`, `.pop()`, an attribute read, a
                    // method call, or a heterogeneous container literal).
                    // Track it as opaquely-but-definitely bound so a later
                    // read reports "no term" instead of the misleading
                    // "not bound before this use".
                    env.opaque_bindings.insert(target.clone());
                }
            }
            HirStmt::AnnAssign {
                target,
                value: Some(value),
                annotation,
                is_final,
            } => {
                // Part 4 of #1026 (PR 4c of #1083) deliberately adds **no
                // branch here**, unlike PRs 4a and 4b, whose `float`/`bool`/
                // `int`/`str` arms this solver mirrors above. The relaxation
                // is in `check_stmt`'s own `AnnAssign` arm only, and two
                // independent facts keep this arm correct without it --
                // both re-verified against this tree:
                //
                // 1. A foreign name's term is `Ok(Ty::Object)`, never
                //    `Err(var)`, so the `AnnotationDefaultConstraint` pushed
                //    below is skipped by `apply_annotation_defaults`' very
                //    first statement (`let Err(var) = ... else { continue }`).
                // 2. Even for an `Err(var)` initializer, the annotation a
                //    4c assignment carries is a `Ty::Tuple`, and
                //    `is_private_solver_scalar` is `Int | Float | Bool |
                //    Str | None` -- so the same loop's second guard skips it
                //    anyway. That guard is not incidental: private-helper
                //    inference is scalar-only on purpose, so an annotated
                //    local cannot leak a container type into an otherwise
                //    unresolved signature.
                //
                // A third fact makes the question moot in practice: this
                // solver runs only over unannotated private helpers (#142),
                // i.e. inside a function body, where reading a foreign name
                // is `I0404` before any of this is reached.
                //
                // Issue #359 (Part 2 of #118): an unconditional annotated
                // assignment upgrades a maybe-bound name back to definitely
                // bound, same as a plain `Assign`.
                env.maybe_bindings.remove(target.as_str());
                // Issue #771: this arm always binds `target` into
                // `env.bindings` below regardless of the initializer term
                // (see the comment further down), so `target` is never
                // opaque by the time this arm finishes. Clear any stale
                // opaque marker for hygiene, mirroring the `maybe_bindings`
                // removal above; `HirExpr::Name`'s lookup already prefers
                // `bindings` over `opaque_bindings` either way.
                env.opaque_bindings.remove(target.as_str());
                // Part 2a of #1142 (#1165): see the plain `Assign` arm. No
                // `AnnotationDefaultConstraint` is pushed for a producer --
                // the term is already the concrete `Ty::MemoryView`, never
                // an `Err(var)`, so `apply_annotation_defaults` would skip
                // it anyway.
                if let Some((callee, len_arg)) = resolved_producer_call(signatures, env, value) {
                    // See the plain `Assign` arm: guard before collecting.
                    reject_buffer_parameter_rebinding(env, target)?;
                    reject_final_rebinding(env, target)?;
                    let len_term = collect_expr_constraints(
                        signatures,
                        parents,
                        concrete,
                        &mut constraints.deferred,
                        env,
                        len_arg,
                    )?;
                    reject_non_int_producer_length(callee, len_term, parents, concrete)?;
                    // The one condition this arm adds over the plain
                    // `Assign` arm, mirroring the check phase's own extra
                    // step: the declared annotation must admit a buffer.
                    reject_producer_annotation_mismatch(target, annotation)?;
                    env.bindings
                        .entry(target.clone())
                        .or_insert(Ok(Ty::MemoryView));
                    env.owned_buffers.insert(target.clone());
                    // Recorded *after* the binding, exactly where
                    // `check_stmt_in_function` records it, and that order is
                    // the rule rather than a detail: a declaration's own
                    // first assignment is not a reassignment, so recording
                    // before `reject_final_rebinding` above would refuse the
                    // declaration itself.
                    //
                    // Recorded here only, and not on this arm's ordinary
                    // (non-producer) path, because this is the only binding
                    // whose later reassignment the solver can *mask*: a
                    // `Final` name the solver does not bind to
                    // `Ty::MemoryView` records no artifact-owned provenance,
                    // so no later use of it produces a solver diagnostic to
                    // displace the check phase's `T0045` with. A recording
                    // that changes no outcome is a guard no test can kill.
                    if *is_final {
                        env.finals.insert(target.clone());
                    }
                    continue;
                }
                if let Some(term) = collect_expr_constraints(
                    signatures,
                    parents,
                    concrete,
                    &mut constraints.deferred,
                    env,
                    value,
                )? {
                    // The annotation is a directional bound on the initializer,
                    // not a symmetric equality. Defer it until every hard
                    // call/operator constraint has been collected so later
                    // `bool` evidence is not widened to `int` merely because
                    // this body was visited first.
                    constraints
                        .annotation_defaults
                        .push(AnnotationDefaultConstraint {
                            initializer: term,
                            annotation: annotation.clone(),
                        });
                }
                // Round-11 review finding 2: see the plain `Assign` arm --
                // collected first, invalidated after, bound below.
                env.rebind_over_owned_buffer(target);
                // A scalar target has the declared type even when the
                // collector cannot produce an initializer term. A non-scalar
                // target is still bound, but deliberately remains unresolved:
                // otherwise a hand-built HIR module could use that annotation
                // to materialize an inferred container signature through a
                // later `return target`. Keep the existing first-binding-wins
                // representation rule; final validation checks initializer
                // compatibility and re-declarations whenever signature
                // inference can otherwise complete.
                if !env.bindings.contains_key(target) {
                    let target_term = if is_private_solver_scalar(annotation) {
                        Ok(annotation.clone())
                    } else {
                        let var = fresh_variable(parents, concrete);
                        constraints.non_scalar_local_terms.push(var);
                        Err(var)
                    };
                    env.bindings.insert(target.clone(), target_term);
                }
            }
            // Deliberately out of scope for issue #245: `ConstraintEnvironment`
            // (unlike the checker's `Environment`) has no declared-but-unbound
            // side-table, and every entry in its `env.bindings` is treated as
            // resolved and readable. Registering the annotation here the same
            // way the `Some(value)` arm above does would make the name
            // readable in this solver, silently accepting `def _f():\n    x:
            // int\n    return x` (should still raise T0021, unbound) instead
            // of only `def _f():\n    x: int\n    x = 1\n    return x`. A
            // parallel declared-side-table for `ConstraintEnvironment` is a
            // separate, independently-testable follow-up if solver-scope
            // coverage of this gap is wanted later.
            // #1244: a `del` binds nothing to infer; the checker owns its
            // binding-state rule. A nested foreign import (#1291) needs no constraint: its names
            // are seeded as `Ty::Object` globals position-blind, like a
            // top-level foreign import's (`constraints/signatures.rs`).
            HirStmt::AnnAssign { value: None, .. }
            | HirStmt::Delete { .. }
            | HirStmt::ForeignImport { .. } => {}
            // Part 2c of #1371: a slice `del` binds nothing, but its base and
            // bounds are ordinary reads -- a call to an unannotated helper
            // in one still needs its call-site constraints. `pycc_hir`
            // refuses a walrus in them, so there is nothing to pre-bind.
            HirStmt::DeleteSlice {
                base,
                start,
                stop,
                step,
                ..
            } => {
                for operand in std::iter::once(&**base)
                    .chain([start, stop, step].into_iter().flatten().map(|b| &**b))
                {
                    collect_expr_constraints(
                        signatures,
                        parents,
                        concrete,
                        &mut constraints.deferred,
                        env,
                        operand,
                    )?;
                }
            }
            // #1457: an attribute `del` reads its base, for the same reason.
            HirStmt::DeleteAttr { base, .. } => {
                collect_expr_constraints(
                    signatures,
                    parents,
                    concrete,
                    &mut constraints.deferred,
                    env,
                    base,
                )?;
            }
            HirStmt::ExprStmt(expr) => {
                // PEP 572 (#774), deep-review follow-up (round 4): bind
                // before unifying -- see `bind_named_expr_targets`'s own
                // doc comment for why this pre-pass is required.
                bind_named_expr_targets(
                    signatures,
                    parents,
                    concrete,
                    &mut constraints.deferred,
                    env,
                    expr,
                )?;
                collect_expr_constraints(
                    signatures,
                    parents,
                    concrete,
                    &mut constraints.deferred,
                    env,
                    expr,
                )?;
            }
            HirStmt::If { test, body, orelse } => {
                // PEP 572 (#774), deep-review follow-up (round 4): bind
                // before unifying -- the test always executes, so this
                // binding is unconditional relative to the branch join
                // below (mirroring `crate::collect_named_expr_bindings`'s
                // own `If` arm ordering rationale in the full checker).
                bind_named_expr_targets(
                    signatures,
                    parents,
                    concrete,
                    &mut constraints.deferred,
                    env,
                    test,
                )?;
                collect_expr_constraints(
                    signatures,
                    parents,
                    concrete,
                    &mut constraints.deferred,
                    env,
                    test,
                )?;
                // Issue #359 (Part 2 of #118): clone the environment for
                // each branch so bindings from one branch do not leak into
                // the other, then join them back — mirroring the validation
                // pass's `join_if_branches` (D-147). Names introduced by
                // only one branch are tracked as maybe-bound in
                // `maybe_bindings`; names introduced by both branches are
                // definitely bound.
                // Issue #771 join-site follow-up: include `opaque_bindings`
                // alongside `bindings` here. A name definitely-but-opaquely
                // bound before this construct must count as pre-existing
                // too -- otherwise a branch that reassigns it to a real,
                // solver-representable term looks "newly introduced" to the
                // join helper below, and when only one branch performs that
                // reassignment the name is misclassified as bound in both
                // branches (the other, untouched branch still carries the
                // opaque marker), unmasking a term that only reflects one
                // path as if it were unconditionally correct. Confirmed as a
                // real gap by the pinned local reviewer's second pass.
                let pre_existing: HashSet<String> = env
                    .bindings
                    .keys()
                    .chain(env.opaque_bindings.iter())
                    .cloned()
                    .collect();
                // #1476: an `isinstance` guard narrows the branch it admits.
                let narrowing = object_narrow::branch_target(signatures, env, test);
                let mut body_env = env.clone();
                let mut orelse_env = env.clone();
                if let Some((name, inner, narrows_body)) = narrowing {
                    let branch_env = if narrows_body {
                        &mut body_env
                    } else {
                        &mut orelse_env
                    };
                    branch_env.narrowed.insert(name, inner);
                }
                collect_block_constraints(
                    signatures,
                    parents,
                    concrete,
                    constraints,
                    &mut body_env,
                    body,
                    return_term.clone(),
                )?;
                collect_block_constraints(
                    signatures,
                    parents,
                    concrete,
                    constraints,
                    &mut orelse_env,
                    orelse,
                    return_term.clone(),
                )?;
                solver::join_if_branches_solver(env, &body_env, &orelse_env, &pre_existing);
            }
            HirStmt::While { test, body } => {
                // PEP 572 (#774), deep-review follow-up (round 4): bind
                // before unifying -- see the `If` arm's own doc comment
                // just above for why this order is required.
                bind_named_expr_targets(
                    signatures,
                    parents,
                    concrete,
                    &mut constraints.deferred,
                    env,
                    test,
                )?;
                collect_expr_constraints(
                    signatures,
                    parents,
                    concrete,
                    &mut constraints.deferred,
                    env,
                    test,
                )?;
                // Issue #359 (Part 2 of #118): clone the environment for
                // the loop body so body-only bindings do not leak into the
                // post-loop environment as definitely bound. A `while` body
                // may execute zero times, so every body-only binding is
                // maybe-bound — mirroring the validation pass's
                // `join_loop_body` (D-147).
                // Issue #771 join-site follow-up: include `opaque_bindings`
                // alongside `bindings` here. A name definitely-but-opaquely
                // bound before this construct must count as pre-existing
                // too -- otherwise a branch that reassigns it to a real,
                // solver-representable term looks "newly introduced" to the
                // join helper below, and when only one branch performs that
                // reassignment the name is misclassified as bound in both
                // branches (the other, untouched branch still carries the
                // opaque marker), unmasking a term that only reflects one
                // path as if it were unconditionally correct. Confirmed as a
                // real gap by the pinned local reviewer's second pass.
                let pre_existing: HashSet<String> = env
                    .bindings
                    .keys()
                    .chain(env.opaque_bindings.iter())
                    .cloned()
                    .collect();
                let mut body_env = env.clone();
                collect_block_constraints(
                    signatures,
                    parents,
                    concrete,
                    constraints,
                    &mut body_env,
                    body,
                    return_term.clone(),
                )?;
                solver::join_loop_body_solver(env, &body_env, &pre_existing);
            }
            HirStmt::ForRange {
                var,
                start,
                stop,
                step,
                body,
            } => {
                for (position, expr) in [("start", start), ("stop", stop), ("step", step)] {
                    if let Some(term @ Err(_)) = collect_expr_constraints(
                        signatures,
                        parents,
                        concrete,
                        &mut constraints.deferred,
                        env,
                        expr,
                    )? {
                        unify_terms(
                            term,
                            Ok(Ty::Int),
                            parents,
                            concrete,
                            "T0021",
                            &format!("range {position}"),
                        )?;
                    }
                }
                // Issue #359 (Part 2 of #118): snapshot the pre-loop
                // binding names so the loop variable and body-only
                // bindings can be tracked as maybe-bound after the loop
                // (a `for` loop may execute zero times).
                // Issue #771 join-site follow-up: include `opaque_bindings`
                // alongside `bindings` here. A name definitely-but-opaquely
                // bound before this construct must count as pre-existing
                // too -- otherwise a branch that reassigns it to a real,
                // solver-representable term looks "newly introduced" to the
                // join helper below, and when only one branch performs that
                // reassignment the name is misclassified as bound in both
                // branches (the other, untouched branch still carries the
                // opaque marker), unmasking a term that only reflects one
                // path as if it were unconditionally correct. Confirmed as a
                // real gap by the pinned local reviewer's second pass.
                let pre_existing: HashSet<String> = env
                    .bindings
                    .keys()
                    .chain(env.opaque_bindings.iter())
                    .cloned()
                    .collect();
                if let Some(existing) = env.bindings.get(var).cloned() {
                    unify_terms(
                        existing,
                        Ok(Ty::Int),
                        parents,
                        concrete,
                        "T0023",
                        &format!("assignment to for-loop target `{var}`"),
                    )?;
                } else {
                    env.bindings.insert(var.clone(), Ok(Ty::Int));
                }
                let mut body_env = env.clone();
                collect_block_constraints(
                    signatures,
                    parents,
                    concrete,
                    constraints,
                    &mut body_env,
                    body,
                    return_term.clone(),
                )?;
                solver::join_loop_body_solver(env, &body_env, &pre_existing);
                // The loop variable itself is maybe-bound after the loop
                // if it was newly introduced (the loop may not execute).
                if !pre_existing.contains(var) {
                    env.maybe_bindings.insert(var.clone());
                }
            }
            HirStmt::ForList { var, list: _, body } => {
                // Unlike `ForRange`, we have no concrete `Ty::Int` fact to
                // unify the loop variable against -- this solver doesn't
                // track a `list`-typed name's element type at all (see the
                // `HirExpr::ListLiteral`/`Subscript`/`ListAppend` arms
                // above). Give `var` a fresh, unconstrained term so a body
                // reference to it doesn't spuriously fail as "not bound"
                // (it *is* locally bound, just not solver-typed); real
                // element-type checking happens in the second, real check
                // pass (`check_with_signatures_all`).
                // Issue #359 (Part 2 of #118): snapshot the pre-loop binding
                // names so the loop variable and body-only bindings can be
                // tracked as maybe-bound after the loop (a `for` loop may
                // execute zero times).
                // Issue #771 join-site follow-up: include `opaque_bindings`
                // alongside `bindings` here. A name definitely-but-opaquely
                // bound before this construct must count as pre-existing
                // too -- otherwise a branch that reassigns it to a real,
                // solver-representable term looks "newly introduced" to the
                // join helper below, and when only one branch performs that
                // reassignment the name is misclassified as bound in both
                // branches (the other, untouched branch still carries the
                // opaque marker), unmasking a term that only reflects one
                // path as if it were unconditionally correct. Confirmed as a
                // real gap by the pinned local reviewer's second pass.
                let pre_existing: HashSet<String> = env
                    .bindings
                    .keys()
                    .chain(env.opaque_bindings.iter())
                    .cloned()
                    .collect();
                // Round-11 review finding 2: the loop variable rebinds the
                // name on every iteration, so any artifact-owned buffer
                // provenance it carried is stale from here on.
                env.rebind_over_owned_buffer(var);
                if !env.bindings.contains_key(var) {
                    let term = fresh_term(parents, concrete);
                    env.bindings.insert(var.clone(), term);
                }
                let mut body_env = env.clone();
                collect_block_constraints(
                    signatures,
                    parents,
                    concrete,
                    constraints,
                    &mut body_env,
                    body,
                    return_term.clone(),
                )?;
                solver::join_loop_body_solver(env, &body_env, &pre_existing);
                // The loop variable itself is maybe-bound after the loop
                // if it was newly introduced (the loop may not execute).
                if !pre_existing.contains(var) {
                    env.maybe_bindings.insert(var.clone());
                }
            }
            // PR 3c of #1082: `for x in o.attr:` / `for x in o.method(...):`.
            // The iterable is a real expression, unlike `ForList`'s bare
            // name, so it is walked; the loop variable gets a fresh
            // unconstrained term for exactly the reason `ForList`'s does
            // (this solver has no `Ty::Object` fact to unify against).
            HirStmt::ForObject { var, iter, body } => {
                collect_expr_constraints(
                    signatures,
                    parents,
                    concrete,
                    &mut constraints.deferred,
                    env,
                    iter,
                )?;
                // Round-11 review finding 2: see the `ForList` arm.
                env.rebind_over_owned_buffer(var);
                if !env.bindings.contains_key(var) {
                    let term = fresh_term(parents, concrete);
                    env.bindings.insert(var.clone(), term);
                }
                collect_block_constraints(
                    signatures,
                    parents,
                    concrete,
                    constraints,
                    env,
                    body,
                    return_term.clone(),
                )?;
            }
            HirStmt::Return(value) => {
                let Some(return_term) = return_term.clone() else {
                    continue;
                };
                // Part 2b of #1142 (#1164), the solver half of
                // `crate::check_stmt_in_function`'s own egress interception.
                // It must run here, before `collect_expr_constraints` reaches
                // the `Name` seam above, for the ordering reason the `len`
                // and `Subscript` interceptions already record: that seam
                // calls `reject_memoryview_read` and would report the owned
                // refusal for the very expression this admits. The declared
                // return type is `return_term`'s `Ok` arm -- an *inferred*
                // return (an `Err(var)` inference variable standing in for an
                // unannotated helper) declines, so only a written
                // `-> memoryview` annotation admits an egress.
                //
                // The third conjunct is this walker's half of the
                // definite-assignment check `crate::check_stmt_in_function`'s
                // own comment enumerates (review round 3 of #1164). The two
                // walkers must agree on the admission predicate, and this is
                // the only spelling available here: `ConstraintEnvironment`
                // carries no three-way lattice, only the `maybe_bindings`
                // side-table, so the test is `!maybe` rather than
                // `== Definitely`. Like the check phase's conjunct it reads
                // the *result* of a join and so closes every join form at
                // once -- `join_if_branches_solver`, `join_loop_body_solver`
                // (the `while`, `for`-`range`, `for`-list, `match`-case and
                // `try` arms all route through it) and the `for` loop
                // variable's own insertion.
                //
                // It is deliberately **not independently observable**, and
                // stands for consistency rather than for a diagnostic of its
                // own: `collect_expr_constraints`'s `Name` arm tests
                // `maybe_bindings` before it reaches `bindings`, so a
                // maybe-bound operand answers `Ok(None)` and the fall-through
                // unifies nothing either way. The check phase's `T0041` is
                // the user-facing gate, exactly as D-147 places it. What the
                // conjunct buys is that the solver can never *widen* the
                // admission past the check phase's -- the subset property
                // `docs/TYPE_SYSTEM.md`'s `memoryview` row requires of this
                // mirror -- if either arm's `Name` ordering later changes.
                //
                // Review round 5's `returns_inside_finally` test is a refusal
                // rather than a decline, exactly as in the check phase: see
                // `crate::buffer::buffer_return_inside_finally`. Part 1 of
                // #1175 runs it once, after the provenance verdict, for both
                // provenances -- the same restructure the check phase made,
                // for the same reason.
                //
                // Part 1 of #1175's caller-owned provenance is spelled here
                // as `Some(Ok(Ty::MemoryView))`, which is
                // `reject_buffer_parameter_rebinding`'s own spelling and
                // exact for the same reason its doc comment gives: an
                // `Err(var)` term is only ever resolved by
                // `apply_annotation_defaults`, whose `is_private_solver_
                // scalar` guard admits `Int | Float | Bool | Str | None`
                // only, so no inferred term can become `MemoryView`. A
                // `memoryview` parameter's term is therefore always this
                // concrete one, and the solver's admission stays a subset of
                // the check phase's -- the property `docs/TYPE_SYSTEM.md`'s
                // `memoryview` row requires of this mirror. The shared
                // `admits_buffer_egress` is what makes that subset hold by
                // construction rather than by two parallel edits.
                if let Some(expr) = value
                    && let Some((name, shape)) =
                        crate::buffer::admitted_buffer_return(expr, return_term.as_ref().ok())
                    && crate::buffer::admits_buffer_egress(
                        env.owned_buffers.contains(name),
                        !env.maybe_bindings.contains(name),
                        matches!(env.bindings.get(name), Some(Ok(Ty::MemoryView))),
                        shape,
                    )
                {
                    // The post-admission block, kept line-for-line parallel
                    // with the check phase's own (`crate::check_function`'s
                    // `HirStmt::Return` arm) so a future divergence is
                    // visible in review. See that block for why the `!owned`
                    // gate is deliberately absent from both (#1179).
                    if let crate::buffer::AdmittedBufferReturn::Slice {
                        start,
                        stop,
                        has_step,
                    } = shape
                    {
                        if has_step {
                            return Err(crate::buffer::buffer_slice_step_unsupported(name));
                        }
                        // D1: this branch `continue`s before
                        // `collect_expr_constraints` ever sees the operand,
                        // so the bounds are walked here -- both to keep
                        // propagating a genuine error such as an unbound
                        // local, exactly as this file's own `HirExpr::Slice`
                        // arm does, and to apply the `T0021` gate the
                        // admitted branch owes. The gate fires only on a
                        // term this solver actually pinned down, so the
                        // solver's admission stays a subset of the check
                        // phase's rather than refusing what the check phase
                        // would still infer.
                        for (label, bound) in [("start", start), ("stop", stop)] {
                            if let Some(bound) = bound {
                                let bound_term = collect_expr_constraints(
                                    signatures,
                                    parents,
                                    concrete,
                                    &mut constraints.deferred,
                                    env,
                                    bound,
                                )?;
                                if let Some(Ok(bound_ty)) = bound_term
                                    && !crate::is_assignable(bound_ty.clone(), Ty::Int)
                                {
                                    return Err(crate::buffer::buffer_slice_bound_not_an_int(
                                        label, &bound_ty,
                                    ));
                                }
                            }
                        }
                    }
                    if env.returns_inside_finally {
                        return Err(crate::buffer::buffer_return_inside_finally(name));
                    }
                    continue;
                }
                // #1387: a bare `return` / `return None` into a declared
                // `object` slot, the check phase's own admission through the
                // one shared predicate (`crate::object_none`). An inferred
                // return (`Err(var)`) declines, as the buffer egress does.
                if let Ok(declared) = &return_term
                    && crate::object_none::admits_none_return(declared, value.as_ref())
                {
                    continue;
                }
                let actual = match value {
                    Some(expr) => collect_expr_constraints(
                        signatures,
                        parents,
                        concrete,
                        &mut constraints.deferred,
                        env,
                        expr,
                    )?,
                    None => Some(Ok(Ty::None)),
                };
                // #1475: a concrete value returned into a declared `object`
                // slot is a boxing seam, not a unification. The solver has no
                // class environment to tell a boxable value from a refused
                // one (a container, an `Optional`), so it defers to the check
                // phase's `crate::object_box::admits_value`, which decides
                // every such return.
                if let (Ok(Ty::Object), Some(Ok(actual_ty))) = (&return_term, &actual)
                    && *actual_ty != Ty::Object
                {
                    continue;
                }
                if let Some(actual) = actual {
                    // #949: this solver runs over *every* module-level
                    // function, not only D-038 private helpers, so the context
                    // string cannot be "private helper ..." unconditionally --
                    // that misdescribed every annotated (and every public)
                    // function whose body returned the wrong type. The real
                    // discriminator is the return term itself: `Ok(ty)` is a
                    // written annotation, `Err(var)` an inference variable
                    // standing in for an unannotated helper's return type.
                    // Only the latter is genuinely an "inferred" return.
                    //
                    // The declared case therefore gets both a different
                    // `context` -- which `unify_terms` interpolates into its
                    // `T0042` "cannot be inferred through a PEP 695 generic
                    // function's own type parameter" message, the same misnomer
                    // one message over -- and, via `declared`, a
                    // declared-vs-actual `T0022` message with a `help`
                    // suggestion. The inferred case keeps the original wording
                    // verbatim so `_helper` diagnostics are unchanged.
                    let declared = return_term.as_ref().ok().cloned();
                    let context = if declared.is_some() {
                        "declared return type"
                    } else {
                        "private helper return type"
                    };
                    unify_terms_with_declared(
                        return_term,
                        actual,
                        parents,
                        concrete,
                        "T0022",
                        context,
                        declared.as_ref(),
                    )?;
                }
            }
            // PR-11 Task 3 (D-123): `dict`'s own type isn't tracked by this
            // solver either (same reasoning as `ForList`'s `list` field
            // above) -- recurse into `key`/`value` only to keep propagating
            // genuine errors; real item-assignment type-checking is
            // `check_stmt`/`check_stmt_in_function`'s job.
            HirStmt::DictSet {
                dict: _,
                key,
                value,
            } => {
                collect_expr_constraints(
                    signatures,
                    parents,
                    concrete,
                    &mut constraints.deferred,
                    env,
                    key,
                )?;
                collect_expr_constraints(
                    signatures,
                    parents,
                    concrete,
                    &mut constraints.deferred,
                    env,
                    value,
                )?;
            }
            // D-154 (Part 1 of #375): `base`'s own type isn't tracked by
            // this solver either (same reasoning as `DictSet` above) --
            // recurse into `base`/`value` only to keep propagating genuine
            // errors; real attribute-slot type-checking is
            // `check_stmt`/`check_stmt_in_function`'s job.
            HirStmt::AttrSet { base, value, .. } => {
                collect_expr_constraints(
                    signatures,
                    parents,
                    concrete,
                    &mut constraints.deferred,
                    env,
                    base,
                )?;
                collect_expr_constraints(
                    signatures,
                    parents,
                    concrete,
                    &mut constraints.deferred,
                    env,
                    value,
                )?;
            }
            // PR-12 Task 3 (D-117): no unification term is registered for
            // `target` -- per D-116's own correction note ("a
            // container-literal assignment's target never receives a solver
            // binding at all... confirmed empirically for all four container
            // types"), a comprehension's own `target` gets that same
            // treatment, matching `ListLiteral`/`DictLiteral`/`SetLiteral`
            // above. Unlike a bare no-op, though, this arm still binds the
            // loop variable (mirroring `ForRange`/`ForList` above, via the
            // shared `bind_comp_loop_var` helper) and recurses into
            // `cond`/`elt` (or `key`/`value`) to keep propagating genuine
            // errors and, critically, to let a call inside a comprehension's
            // own sub-expressions still participate in this solver's
            // argument<->parameter unification (self-review finding: an
            // earlier no-op-only version of this arm made an unannotated
            // private-helper parameter used only inside a comprehension's
            // `elt` spuriously fail to infer with "cannot infer type of
            // parameter ...; add an annotation", since the call inside `elt`
            // was never visited at all -- see
            // `private_helper_parameter_is_inferred_through_a_comprehension_s_elt`).
            HirStmt::ListCompAssign {
                target,
                var,
                iter,
                cond,
                elt,
            }
            | HirStmt::SetCompAssign {
                target,
                var,
                iter,
                cond,
                elt,
            } => {
                bind_comp_loop_var(
                    signatures,
                    parents,
                    concrete,
                    &mut constraints.deferred,
                    env,
                    var,
                    iter,
                )?;
                if let Some(cond) = cond {
                    collect_expr_constraints(
                        signatures,
                        parents,
                        concrete,
                        &mut constraints.deferred,
                        env,
                        cond,
                    )?;
                }
                let elt_term = collect_expr_constraints(
                    signatures,
                    parents,
                    concrete,
                    &mut constraints.deferred,
                    env,
                    elt,
                )?;
                // Round-11 review finding 2: this solver binds no term for a
                // comprehension `target` at all, so before this call the
                // name kept whatever it was bound to before -- including an
                // artifact-owned `Ty::MemoryView`, which a later read then
                // refused with the owned-buffer `C0001` instead of letting
                // the check phase report its `T0023` for the reassignment.
                // The comprehension's own sub-expressions are collected
                // first, since they may still read that buffer.
                //
                // `var` needs no such call: it is the D-117 synthesized
                // internal loop name (see `HirStmt::ListCompAssign`'s own
                // doc comment), which cannot collide with a source name.
                //
                // #1343: a set comprehension's container is the same
                // defaulted term as the expression form's.
                let container = match stmt {
                    HirStmt::SetCompAssign { .. } => set_comp::set_comp_container(
                        elt_term,
                        parents,
                        concrete,
                        &mut constraints.deferred,
                    ),
                    _ => Ok(Ty::List(Box::new(Ty::Int))),
                };
                bind_comp_target(env, target, container);
            }
            HirStmt::DictCompAssign {
                target,
                var,
                iter,
                cond,
                key,
                value,
            } => {
                bind_comp_loop_var(
                    signatures,
                    parents,
                    concrete,
                    &mut constraints.deferred,
                    env,
                    var,
                    iter,
                )?;
                if let Some(cond) = cond {
                    collect_expr_constraints(
                        signatures,
                        parents,
                        concrete,
                        &mut constraints.deferred,
                        env,
                        cond,
                    )?;
                }
                collect_expr_constraints(
                    signatures,
                    parents,
                    concrete,
                    &mut constraints.deferred,
                    env,
                    key,
                )?;
                collect_expr_constraints(
                    signatures,
                    parents,
                    concrete,
                    &mut constraints.deferred,
                    env,
                    value,
                )?;
                // Round-11 review finding 2: see the list/set arm above.
                bind_comp_target(env, target, Ok(Ty::Dict(Box::new((Ty::Str, Ty::Int)))));
            }
            HirStmt::Match { subject, cases } => {
                collect_expr_constraints(
                    signatures,
                    parents,
                    concrete,
                    &mut constraints.deferred,
                    env,
                    subject,
                )?;
                for case in cases {
                    // Issue #771 join-site follow-up: include `opaque_bindings`
                    // alongside `bindings` here. A name definitely-but-opaquely
                    // bound before this construct must count as pre-existing
                    // too -- otherwise a branch that reassigns it to a real,
                    // solver-representable term looks "newly introduced" to the
                    // join helper below, and when only one branch performs that
                    // reassignment the name is misclassified as bound in both
                    // branches (the other, untouched branch still carries the
                    // opaque marker), unmasking a term that only reflects one
                    // path as if it were unconditionally correct. Confirmed as a
                    // real gap by the pinned local reviewer's second pass.
                    let pre_existing: HashSet<String> = env
                        .bindings
                        .keys()
                        .chain(env.opaque_bindings.iter())
                        .cloned()
                        .collect();
                    let mut case_env = env.clone();
                    // Round-11 review finding 2: a `case` pattern's capture
                    // names rebind inside the case body. This solver binds
                    // no term for them (a capture's type is the subject's,
                    // which the check phase resolves), so without this the
                    // name kept its artifact-owned `Ty::MemoryView` binding
                    // and a read of it -- in the body or, through the join
                    // below, after the `match` -- raised the owned-buffer
                    // `C0001` over the check phase's `T0023`. Dropped in the
                    // case environment rather than in `env` so the join
                    // helper carries it out only for the paths that rebind.
                    let mut captures = Vec::new();
                    crate::collect_pattern_capture_names(&case.pattern, &mut captures);
                    for capture in captures {
                        case_env.rebind_over_owned_buffer(capture);
                    }
                    collect_block_constraints(
                        signatures,
                        parents,
                        concrete,
                        constraints,
                        &mut case_env,
                        &case.body,
                        return_term.clone(),
                    )?;
                    solver::join_loop_body_solver(env, &case_env, &pre_existing);
                }
            }
            HirStmt::Try {
                body,
                handlers,
                orelse,
                finalbody,
            }
            | HirStmt::TryStar {
                body,
                handlers,
                orelse,
                finalbody,
            } => try_stmt::collect_try_constraints(
                signatures,
                parents,
                concrete,
                constraints,
                env,
                try_stmt::TryShape {
                    body,
                    handlers,
                    orelse,
                    finalbody,
                    star: matches!(stmt, HirStmt::TryStar { .. }),
                },
                return_term.clone(),
            )?,
            HirStmt::Raise { exc, cause } => {
                // #382 (PR-22 Part 1): A raise expression that is a direct
                // call to a builtin exception class (e.g.
                // `ValueError("msg")`) is classified as C0001 by
                // `collect_expr_constraints`: that collector resolves a
                // callee against the `signatures` map it was handed, which
                // holds only `HirItem::Function` entries, so a class name
                // misses and falls through to the known-callable-builtin
                // arm. Part 1 of #541 seeded the seven builtin exception
                // names into `Environment::classes`, but that table is not
                // what this collector consults, so the classification is
                // unchanged. The actual validation is done by
                // `check_raise_stmt` in the check pass, so errors from
                // constraint collection for raise operands are deliberately
                // ignored here — they would otherwise prevent the solver
                // path from reaching `check_with_signatures_all`, where the
                // real check succeeds.
                if let Some(exc_expr) = exc {
                    let _ = collect_expr_constraints(
                        signatures,
                        parents,
                        concrete,
                        &mut constraints.deferred,
                        env,
                        exc_expr,
                    );
                }
                if let Some(cause_expr) = cause {
                    let _ = collect_expr_constraints(
                        signatures,
                        parents,
                        concrete,
                        &mut constraints.deferred,
                        env,
                        cause_expr,
                    );
                }
            }
        }
    }
    // The last statement's update, so a block's final environment (a `try`
    // body's, which its `else` continues) reflects every rebinding in it.
    if let Some(previous) = previous {
        object_narrow::after_statement(signatures, env, previous);
    }
    Ok(())
}

fn propagate_binop_constraints(
    binops: &[BinOpConstraint],
    parents: &mut [usize],
    concrete: &mut [Option<Ty>],
) -> Result<(), Diagnostic> {
    loop {
        let mut changed = false;
        for &(op, ref left_term, ref right_term, ref result_term) in binops {
            let left = resolved_term(left_term.clone(), parents, concrete);
            let right = resolved_term(right_term.clone(), parents, concrete);
            let result = resolved_term(result_term.clone(), parents, concrete);
            if let (Some(left), Some(right)) = (left, right) {
                let result_ty = numeric_result_type(op, left, right)?;
                changed |= unify_terms(
                    result_term.clone(),
                    Ok(result_ty),
                    parents,
                    concrete,
                    "T0021",
                    "binary expression",
                )?;
                continue;
            }

            // Propagate constraints backward when the result determines a
            // unique operand representation. In particular, an annotated
            // `int` result for a non-division binary expression rules out
            // floats and strings, so unresolved operands are int-like and
            // use the merged `int` representation. This makes
            // `def _inc(x) -> int: return x + 1` infer `x: int` without a
            // call-site constraint (D-045).
            if result == Some(Ty::Int) && op != BinOpKind::Div {
                let left_changed = unify_terms(
                    left_term.clone(),
                    Ok(Ty::Int),
                    parents,
                    concrete,
                    "T0021",
                    "left operand of int binary expression",
                )?;
                let right_changed = unify_terms(
                    right_term.clone(),
                    Ok(Ty::Int),
                    parents,
                    concrete,
                    "T0021",
                    "right operand of int binary expression",
                )?;
                changed |= left_changed || right_changed;
            }
        }
        if !changed {
            return Ok(());
        }
    }
}

fn apply_annotation_defaults(
    constraints: &[AnnotationDefaultConstraint],
    parents: &mut [usize],
    concrete: &mut [Option<Ty>],
) -> Result<(), Diagnostic> {
    let mut bounds_by_root = vec![Vec::new(); parents.len()];
    for constraint in constraints {
        let Err(var) = &constraint.initializer else {
            continue;
        };
        if !is_private_solver_scalar(&constraint.annotation) {
            // Private-helper inference is deliberately scalar-only. A
            // hand-built HIR module must not use an annotated local to leak a
            // container type into an otherwise-unresolved signature.
            continue;
        }
        let root = root(parents, *var);
        if concrete[root].is_none() {
            bounds_by_root[root].push(constraint.annotation.clone());
        }
    }

    for (root, bounds) in bounds_by_root.iter_mut().enumerate() {
        if bounds.is_empty() {
            continue;
        }
        bounds.sort_by_key(Ty::name);
        bounds.dedup();
        let fallback = bounds.iter().find(|candidate| {
            bounds
                .iter()
                .all(|bound| is_assignable((*candidate).clone(), bound.clone()))
        });
        let Some(fallback) = fallback else {
            let bounds = bounds
                .iter()
                .map(|bound| format!("`{}`", bound.name()))
                .collect::<Vec<_>>()
                .join(", ");
            return Err(Diagnostic::error(
                "T0021",
                format!("annotated initializer has incompatible directional constraints: {bounds}"),
                Span::new(0, 0),
            ));
        };
        // This root was deliberately selected only while unresolved, and
        // defaults never union roots. Install the chosen directional fallback
        // directly instead of routing it through symmetric type merging.
        concrete[root] = Some(fallback.clone());
    }
    Ok(())
}

pub(crate) fn contains_return(body: &[HirStmt]) -> bool {
    body.iter().any(|stmt| match stmt {
        HirStmt::Return(_) => true,
        HirStmt::If { body, orelse, .. } => contains_return(body) || contains_return(orelse),
        HirStmt::While { body, .. }
        | HirStmt::ForRange { body, .. }
        | HirStmt::ForList { body, .. }
        | HirStmt::ForObject { body, .. } => contains_return(body),
        HirStmt::Match { cases, .. } => cases.iter().any(|case| contains_return(&case.body)),
        HirStmt::ExprStmt(_)
        | HirStmt::Assign { .. }
        | HirStmt::AnnAssign { .. }
        | HirStmt::DictSet { .. }
        | HirStmt::AttrSet { .. }
        | HirStmt::ListCompAssign { .. }
        | HirStmt::SetCompAssign { .. }
        | HirStmt::DictCompAssign { .. }
        | HirStmt::Delete { .. }
        | HirStmt::DeleteSlice { .. }
        | HirStmt::DeleteAttr { .. }
        | HirStmt::ForeignImport { .. }
        | HirStmt::Raise { .. } => false,
        HirStmt::Try {
            body,
            handlers,
            orelse,
            finalbody,
        }
        | HirStmt::TryStar {
            body,
            handlers,
            orelse,
            finalbody,
        } => {
            contains_return(body)
                || handlers.iter().any(|h| contains_return(&h.body))
                || contains_return(orelse)
                || contains_return(finalbody)
        }
    })
}

/// Issue #118 Part 1: returns true if any statement in `body` introduces
/// a new binding (assignment, annotated assignment, comprehension assignment,
/// or dict/set item assignment, including nested inside if/while/for). Used
/// to skip the expensive `env.clone()` + `join_if_branches` path when neither
/// branch of an `if` assigns anything -- the common case for guard-only ifs.
pub(crate) fn introduces_bindings(body: &[HirStmt]) -> bool {
    body.iter().any(|stmt| match stmt {
        HirStmt::Assign { .. }
        | HirStmt::AnnAssign { .. }
        | HirStmt::DictSet { .. }
        | HirStmt::AttrSet { .. }
        | HirStmt::ListCompAssign { .. }
        | HirStmt::SetCompAssign { .. }
        // #1244: a `del` changes a binding state, so the in-place fast
        // paths that assume an unchanged environment must not be taken.
        | HirStmt::Delete { .. }
        // #1291: a nested foreign import binds its names.
        | HirStmt::ForeignImport { .. }
        | HirStmt::DictCompAssign { .. } => true,
        HirStmt::If { body, orelse, .. } => {
            introduces_bindings(body) || introduces_bindings(orelse)
        }
        HirStmt::While { body, .. } => introduces_bindings(body),
        HirStmt::ForRange { body, .. } => introduces_bindings(body),
        HirStmt::ForList { body, .. } | HirStmt::ForObject { body, .. } => {
            introduces_bindings(body)
        }
        HirStmt::Match { cases, .. } => cases.iter().any(|case| introduces_bindings(&case.body)),
        // A slice or attribute `del` (Part 2c of #1371, #1457) changes no
        // binding.
        HirStmt::Return(_)
        | HirStmt::ExprStmt(_)
        | HirStmt::DeleteSlice { .. }
        | HirStmt::DeleteAttr { .. } => false,
        HirStmt::Try {
            body,
            handlers,
            orelse,
            finalbody,
        }
        | HirStmt::TryStar {
            body,
            handlers,
            orelse,
            finalbody,
        } => {
            introduces_bindings(body)
                || handlers.iter().any(|h| introduces_bindings(&h.body))
                || introduces_bindings(orelse)
                || introduces_bindings(finalbody)
        }
        HirStmt::Raise { .. } => false,
    })
}
