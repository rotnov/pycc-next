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

mod builtin_call;
mod constructor_call;
mod method_return;
mod object_lift;
mod object_narrow;
mod set_comp;
mod signatures;
mod try_stmt;
mod yield_fallback;

// Large `collect_expr_constraints` arms (mechanical extraction: each helper
// re-destructures its own node and runs the arm body verbatim) and the
// statement-level collector.
mod attr_terms;
mod bindings;
mod block;
mod block_stmt;
mod call_term;
mod container_terms;
mod name_term;
mod operator_terms;
mod producer_checks;
mod public_inference;
pub(crate) mod return_fallback;
pub(crate) use public_inference::parameter_needs_annotation;
mod unify;
pub(crate) use bindings::*;
pub(crate) use block::*;
pub(crate) use producer_checks::*;
pub(crate) use unify::*;

#[cfg(test)]
pub(crate) use set_comp::ContainerDefault;
pub(crate) use set_comp::{DeferredConstraints, deferred_frozenset};
pub(crate) use signatures::*;

use crate::fx::{HashMap, HashSet};
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
/// A deferred binary operation: operator, operand and result terms, and
/// where the expression is (`crate::locate::expr_location`), so a
/// failure found once every body is collected still points at it (#10).
type BinOpConstraint = (
    BinOpKind,
    TypeTerm,
    TypeTerm,
    TypeTerm,
    Option<crate::locate::Location>,
);

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
    /// Non-strict mode (#3): the inferred return terms some `return`
    /// gives a value the solver has no term for (a tuple display, say).
    /// One that resolves to `None` from its other returns is a CPython
    /// object, which holds both.
    pub(crate) opaque_returns: Vec<usize>,
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
    /// Whether the body is a generator function's: its return term is the
    /// element type its `yield`s unify with, and its `return`s carry no
    /// value.
    pub(crate) in_generator: bool,
    /// The annotated `(yield, send, return)` types of the generator whose
    /// body is collected, for the value of a `yield` (the send type) and
    /// of a `return value`; `None` elsewhere and for an inferred generator.
    pub(crate) generator_sig: Option<(Ty, Ty, Ty)>,
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
            bindings: HashMap::default(),
            local_names,
            defs_rebound: HashSet::default(),
            maybe_bindings: HashSet::default(),
            opaque_bindings: HashSet::default(),
            foreign_objects: HashSet::default(),
            std_module_aliases: Vec::new(),
            owned_buffers: HashSet::default(),
            in_function_body: false,
            in_generator: false,
            generator_sig: None,
            returns_inside_finally: false,
            shadowed_producers: HashSet::default(),
            finals: HashSet::default(),
            class_defs: &[],
            narrowed: HashMap::default(),
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

fn merge_inferred_types(left: Ty, right: Ty) -> Option<Ty> {
    if left == right {
        Some(left)
    } else if matches!((&left, &right), (Ty::Bool, Ty::Int) | (Ty::Int, Ty::Bool)) {
        Some(Ty::Int)
    } else if crate::non_strict::is_non_strict()
        && [&left, &right].contains(&&Ty::Object)
        && [&left, &right].iter().all(|ty| {
            matches!(ty, Ty::Object | Ty::Int | Ty::Float | Ty::Bool | Ty::Str | Ty::None)
        })
    {
        // Non-strict mode (#3): a term inferred both as a CPython object
        // and as a scalar (a helper called with an object and with an
        // `int`) is the object, which holds either.
        Some(Ty::Object)
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

pub(crate) fn collect_expr_constraints(
    signatures: &HashMap<String, SignatureTerms>,
    parents: &mut Vec<usize>,
    concrete: &mut Vec<Option<Ty>>,
    deferred: &mut DeferredConstraints,
    env: &ConstraintEnvironment<'_, '_>,
    expr: &HirExpr,
) -> Result<Option<TypeTerm>, Diagnostic> {
    collect_expr_constraints_here(signatures, parents, concrete, deferred, env, expr)
        .map_err(|diagnostic| crate::locate::place_expr(diagnostic, expr))
}

/// [`collect_expr_constraints`] before its diagnostic is placed at `expr`.
fn collect_expr_constraints_here(
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
        HirExpr::BoolLiteral(_) | HirExpr::OpaqueBool => Ok(Some(Ok(Ty::Bool))),
        HirExpr::StringLiteral(_) => Ok(Some(Ok(Ty::Str))),
        HirExpr::NoneLiteral => Ok(Some(Ok(Ty::None))),
        // #1418: `return NotImplemented` in a comparison method of an `ext`
        // module is CPython's singleton, a CPython object.
        HirExpr::NotImplemented => Ok(Some(Ok(Ty::Object))),
        HirExpr::Name(..) => name_term::collect_name_constraints(parents, concrete, env, expr),
        HirExpr::FString(parts) => {
            for part in parts {
                if let FStringPart::Interpolation(expr) = part {
                    collect_expr_constraints(signatures, parents, concrete, deferred, env, expr)?;
                }
            }
            Ok(Some(Ok(Ty::Str)))
        }
        HirExpr::Compare { .. } => operator_terms::collect_compare_constraints(
            signatures, parents, concrete, deferred, env, expr,
        ),
        // #1212: a chained comparison mirrors `Compare` -- every operand is
        // collected, and the result is `bool`.
        HirExpr::CompareChain { first, links } => {
            for operand in pycc_hir::compare_chain_operands(first, links) {
                collect_expr_constraints(signatures, parents, concrete, deferred, env, operand)?;
            }
            Ok(Some(Ok(Ty::Bool)))
        }
        HirExpr::BoolOp { .. } => operator_terms::collect_bool_op_constraints(
            signatures, parents, concrete, deferred, env, expr,
        ),
        HirExpr::IfExp { .. } => operator_terms::collect_if_exp_constraints(
            signatures, parents, concrete, deferred, env, expr,
        ),
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
        HirExpr::UnaryOp { .. } => operator_terms::collect_unary_op_constraints(
            signatures, parents, concrete, deferred, env, expr,
        ),
        // Part 1 of #891: the unpacked value's own term passes through.
        // Whether the value can be unpacked at all, and into how many
        // names, is `crate::unpack::infer_unpack`'s check-phase gate; the
        // element reads that follow are ordinary `Subscript`s.
        HirExpr::Unpack { value, .. } => {
            collect_expr_constraints(signatures, parents, concrete, deferred, env, value)
        }
        HirExpr::BinOp { .. } | HirExpr::AugBinOp { .. } => {
            operator_terms::collect_bin_op_constraints(
                signatures, parents, concrete, deferred, env, expr,
            )
        }
        HirExpr::Call { .. } => {
            call_term::collect_call_constraints(signatures, parents, concrete, deferred, env, expr)
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
        // A container element the empty-container pass resolved is concrete
        // too (that pass stores only fully concrete types); the check phase
        // still applies the element gate to it.
        HirExpr::EmptyList(element_ty) => {
            if is_private_solver_scalar(element_ty)
                || (!crate::empty_container::contains_infer(element_ty)
                    && !contains_param(element_ty)
                    && matches!(
                        element_ty,
                        Ty::List(_) | Ty::Tuple(_) | Ty::Dict(_) | Ty::Set(_)
                    ))
            {
                Ok(Some(Ok(Ty::List(Box::new(element_ty.clone())))))
            } else {
                Ok(None)
            }
        }
        // The same for a resolved empty dict or set: its type is concrete.
        HirExpr::EmptyDict(kv)
            if !crate::empty_container::contains_infer(&kv.0)
                && !crate::empty_container::contains_infer(&kv.1)
                && !contains_param(&kv.0)
                && !contains_param(&kv.1) =>
        {
            Ok(Some(Ok(Ty::Dict(kv.clone()))))
        }
        HirExpr::EmptySet(element)
            if !crate::empty_container::contains_infer(element) && !contains_param(element) =>
        {
            Ok(Some(Ok(Ty::Set(Box::new(element.clone())))))
        }
        HirExpr::EmptyDict(_) | HirExpr::EmptySet(_) => Ok(None),
        // Part 2d of #1371: a list display built as a CPython `list` is an
        // object whatever its elements are; their packability is
        // `crate::foreign::list_display`'s check, run by the checker walk.
        HirExpr::ObjectList(elements) => {
            for element in elements {
                collect_expr_constraints(signatures, parents, concrete, deferred, env, element)?;
            }
            Ok(Some(Ok(Ty::Object)))
        }
        HirExpr::ListLiteral(..) => container_terms::collect_list_literal_constraints(
            signatures, parents, concrete, deferred, env, expr,
        ),
        HirExpr::Subscript { .. } => container_terms::collect_subscript_constraints(
            signatures, parents, concrete, deferred, env, expr,
        ),
        HirExpr::Slice { .. } => container_terms::collect_slice_constraints(
            signatures, parents, concrete, deferred, env, expr,
        ),
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
            // Every element already concrete: the tuple's type (`a, b = 1,
            // 2` reads its items back); otherwise no term, as before.
            let mut items = Some(Vec::with_capacity(elements.len()));
            for element in elements {
                let term =
                    collect_expr_constraints(signatures, parents, concrete, deferred, env, element)?;
                let item = term.and_then(|term| resolved_term(term, parents, concrete));
                items = items.zip(item).map(|(mut items, item)| {
                    items.push(item);
                    items
                });
            }
            Ok(items
                .filter(|items| !items.is_empty() && items.iter().all(is_private_solver_scalar))
                .map(|items| Ok(Ty::Tuple(Box::new(items)))))
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
        HirExpr::AttrGet { .. } => attr_terms::collect_attr_get_constraints(
            signatures, parents, concrete, deferred, env, expr,
        ),
        HirExpr::MethodCall { .. } => attr_terms::collect_method_call_constraints(
            signatures, parents, concrete, deferred, env, expr,
        ),
        HirExpr::ReceiverDispatchedCall { .. } => {
            attr_terms::collect_receiver_dispatched_call_constraints(
                signatures, parents, concrete, deferred, env, expr,
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
            // A call of a function value answers its return type.
            if let Some(Ty::Callable(sig)) = callee_term
                .clone()
                .and_then(|term| resolved_term(term, parents, concrete))
            {
                return Ok(Some(Ok(sig.1)));
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
        HirExpr::Comprehension(..) => container_terms::collect_comprehension_constraints(
            signatures, parents, concrete, deferred, env, expr,
        ),
        // A typed array operation: its operands are walked for their own
        // constraints, and its type is fixed by the node.
        HirExpr::Nd(nd) => {
            for sub in pycc_hir::ndarray::expr_children(expr) {
                collect_expr_constraints(signatures, parents, concrete, deferred, env, sub)?;
            }
            Ok(Some(Ok(nd.ty())))
        }
    }
}

/// Settles every deferred binary operation. A failure is keyed to the
/// item holding the failing expression, still as a module-level failure
/// (`crate::module::module_level_at`).
fn propagate_binop_constraints(
    binops: &[BinOpConstraint],
    parents: &mut [usize],
    concrete: &mut [Option<Ty>],
) -> Result<(), crate::KeyedDiagnostics> {
    loop {
        let mut changed = false;
        for (op, left_term, right_term, result_term, location) in binops {
            let placed = |diagnostic| crate::module::module_level_at(diagnostic, *location);
            changed |= propagate_binop(*op, left_term, right_term, result_term, parents, concrete)
                .map_err(placed)?;
        }
        if !changed {
            return Ok(());
        }
    }
}

/// One step of [`propagate_binop_constraints`]: whether it changed a term.
fn propagate_binop(
    op: BinOpKind,
    left_term: &TypeTerm,
    right_term: &TypeTerm,
    result_term: &TypeTerm,
    parents: &mut [usize],
    concrete: &mut [Option<Ty>],
) -> Result<bool, Diagnostic> {
    let mut changed = false;
    {
        {
            let left = resolved_term(left_term.clone(), parents, concrete);
            let right = resolved_term(right_term.clone(), parents, concrete);
            let result = resolved_term(result_term.clone(), parents, concrete);
            if let (Some(left), Some(right)) = (left, right) {
                // Two native containers of different kinds (`t |= {x}` on
                // a list) are left to the check phase too, which widens
                // the name in non-strict mode.
                let object_operand = left == Ty::Object
                    || right == Ty::Object
                    || (crate::assign_check::is_container_ty(&left)
                        && crate::assign_check::is_container_ty(&right));
                let result_ty = match numeric_result_type(op, left, right) {
                    Ok(ty) => ty,
                    // Non-strict mode: an operator on a CPython object is
                    // CPython's own, whose result is an object; the check
                    // phase decides which other operands it admits (a fresh
                    // native list, say), which only it can see.
                    Err(_) if object_operand && crate::non_strict::is_non_strict() => Ty::Object,
                    Err(diagnostic) => return Err(diagnostic),
                };
                changed |= unify_terms(
                    result_term.clone(),
                    Ok(result_ty),
                    parents,
                    concrete,
                    "T0021",
                    "binary expression",
                )?;
                return Ok(changed);
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
    }
    Ok(changed)
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
        HirStmt::LoopElse { loop_stmt, orelse } => {
            contains_return(std::slice::from_ref(loop_stmt.as_ref())) || contains_return(orelse)
        }
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
        | HirStmt::SetSlice { .. }
        | HirStmt::DeleteAttr { .. }
        | HirStmt::DeleteItem { .. }
        | HirStmt::ForeignImport { .. }
        | HirStmt::Break
        | HirStmt::Continue
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
        HirStmt::LoopElse { loop_stmt, orelse } => {
            introduces_bindings(std::slice::from_ref(loop_stmt.as_ref()))
                || introduces_bindings(orelse)
        }
        HirStmt::Match { cases, .. } => cases.iter().any(|case| introduces_bindings(&case.body)),
        // A slice or attribute `del` (Part 2c of #1371, #1457) changes no
        // binding.
        HirStmt::Return(_)
        | HirStmt::ExprStmt(_)
        | HirStmt::DeleteSlice { .. }
        | HirStmt::SetSlice { .. }
        | HirStmt::DeleteAttr { .. }
        | HirStmt::DeleteItem { .. }
        | HirStmt::Break
        | HirStmt::Continue => false,
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

/// Whether `ty` mentions a PEP 695 type parameter anywhere.
fn contains_param(ty: &Ty) -> bool {
    match ty {
        Ty::Param(_) => true,
        Ty::List(e) | Ty::Set(e) | Ty::FrozenSet(e) | Ty::Optional(e) => contains_param(e),
        Ty::Dict(kv) => contains_param(&kv.0) || contains_param(&kv.1),
        Ty::Tuple(es) => es.iter().any(contains_param),
        _ => false,
    }
}
