//! Module-level check driver: the two-phase concrete/solver selection, the
//! per-function diagnostic collection, and the public `check*` entry points.
//!
//! Extracted from `lib.rs` per AGENTS.md's file-decomposition rule (issue
//! #868, tracked by #544), mirroring `pycc_hir::module`: the crate root keeps
//! the annotation-driven statement and expression checker; the driver that
//! sequences the redefinition pre-checks, the concrete fast path, the solver,
//! and the signature-validated check pass lives here. `lib.rs` re-exports
//! `check`, `check_all`, `check_and_resolve`, `check_and_resolve_all`, and
//! the crate-private helpers the tests and `monomorphize` reach through the
//! crate root, so no path changed. `checked_function_signatures_all` moved here
//! from `constraints.rs` with them: it is `check_and_resolve`'s half of the
//! same driver.
//!
//! Part 3 of #864 (#868, D-220): the driver collects one diagnostic per
//! failing function instead of stopping at the first. Every collector
//! returns a [`KeyedDiagnostics`] list whose key is the failing function's
//! index in `hir.items` (a `TopLevel`/`Module` key for a module-level
//! failure; see [`DiagnosticKey`]), and
//! [`merge_solver_first`] combines the solver's list with the concrete
//! checker's so that, per function, the solver's diagnostic wins when both
//! phases flagged it -- the historical solver-first rule, applied per
//! function instead of once per module. The first collected diagnostic is
//! byte-identical to what `check` reported before this part (D-217 rule 2).

use super::*;
use std::collections::HashSet;

/// The private-helper signature table: function name to `(parameter
/// types, return type)`.
pub(crate) type FunctionSignatures = HashMap<String, (Vec<Ty>, Ty)>;

/// Where in `hir.items` a collected diagnostic belongs (D-220; #898 made
/// it public and split the old `None` key in two so the driver can map a
/// linked program's diagnostic back to the file that owns the item).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DiagnosticKey {
    /// A failing function body: the `HirItem::Function`'s index.
    Function(usize),
    /// A failing top-level statement: the `HirItem::TopLevelStmt`'s index.
    TopLevel(usize),
    /// A whole-program failure with no owning item: a pre-check (an
    /// incompatible redefinition or attribute redeclaration), a post-body
    /// solver phase, or a post-check phase (`monomorphize`,
    /// `unroll_enum_loops`).
    Module,
}

impl DiagnosticKey {
    /// The owning item's index, when there is one.
    pub fn item_index(self) -> Option<usize> {
        match self {
            Self::Function(index) | Self::TopLevel(index) => Some(index),
            Self::Module => None,
        }
    }
}

/// One diagnostic per failing function, keyed by the function's index in
/// `hir.items`; a `TopLevel`/`Module` key is a module-level failure (a
/// top-level statement, or a whole-module solver phase).
///
/// Invariant kept by every collector in this crate: a non-`Function`-keyed
/// entry is always the *only* entry (a module-level failure stops that
/// collector at once), so a list is either exactly one `TopLevel`/`Module`
/// entry or entirely `Function`-keyed. [`merge_solver_first`] relies on it.
pub type KeyedDiagnostics = Vec<(DiagnosticKey, Diagnostic)>;

/// Wraps a whole-module failure as a one-element keyed list. A named `fn`
/// rather than a closure at every `map_err` site so the five solver
/// post-phase paths do not each grow their own closure region under the
/// 100%-region gate (D-014).
pub(crate) fn module_level(diagnostic: Diagnostic) -> KeyedDiagnostics {
    vec![(DiagnosticKey::Module, diagnostic)]
}

/// Wraps a failing top-level statement's diagnostic as a one-element keyed
/// list under `DiagnosticKey::TopLevel(index)`.
pub(crate) fn top_level(index: usize, diagnostic: Diagnostic) -> KeyedDiagnostics {
    vec![(DiagnosticKey::TopLevel(index), diagnostic)]
}

/// First-diagnostic view of a keyed list, for the test-only wrappers that
/// keep the pre-#868 single-`Diagnostic` signatures.
///
/// The `.expect` follows D-219's `lower_checked` precedent: every collector
/// returns a non-empty `Err` by construction, and the panic path lives in
/// libcore, adding no in-crate region.
#[cfg(test)]
pub(crate) fn first_keyed(diagnostics: KeyedDiagnostics) -> Diagnostic {
    diagnostics
        .into_iter()
        .map(|(_, diagnostic)| diagnostic)
        .next()
        .expect("a keyed diagnostic list is never empty by construction")
}

fn first_diagnostic(diagnostics: Vec<Diagnostic>) -> Diagnostic {
    diagnostics
        .into_iter()
        .next()
        .expect("check_all's Err is never empty by construction")
}

fn drop_keys(diagnostics: KeyedDiagnostics) -> Vec<Diagnostic> {
    diagnostics
        .into_iter()
        .map(|(_, diagnostic)| diagnostic)
        .collect()
}

pub(super) fn module_function_local_names(hir: &HirModule) -> Vec<Vec<&str>> {
    hir.items
        .iter()
        .map(|item| match item {
            HirItem::Function { params, body, .. } => function_local_names(params, body),
            HirItem::TopLevelStmt(_) => Vec::new(),
        })
        .collect()
}

/// Type-checks a module and returns a cloned HIR whose function signatures
/// contain only the concrete types resolved by private-helper inference.
/// Consumers after the type boundary must use this module rather than the
/// unresolved lowering result so `Ty::Infer` can never leak into MIR or code
/// generation. PR-13 Task 3 (D-133/D-134): also monomorphizes every PEP 695
/// generic function call site into a call to a concrete, mangled
/// specialization (see `monomorphize`) -- the returned module contains only
/// ordinary concrete-`Ty` functions, exactly what `pycc_mir::build` expects.
///
/// First-diagnostic view of [`check_and_resolve_all`] (D-217's `parse`/
/// `parse_all` precedent): the `Err` is exactly that function's first
/// collected diagnostic, byte-identical to what this function reported
/// before per-function collection landed (D-220).
pub fn check_and_resolve(hir: &HirModule) -> Result<HirModule, Diagnostic> {
    check_and_resolve_all(hir).map_err(first_diagnostic)
}

/// [`check_and_resolve`] collecting one diagnostic per failing function
/// (Part 3 of #864, D-220); the `Err` is never empty. The check phases
/// collect exactly as [`check_all`] does; the post-check phases
/// (`monomorphize`, `unroll_enum_loops`) run only once checking passed and
/// still report a single diagnostic, wrapped as a one-element list -- they
/// are not part of the type pass's collection.
pub fn check_and_resolve_all(hir: &HirModule) -> Result<HirModule, Vec<Diagnostic>> {
    check_and_resolve_all_keyed(hir).map_err(drop_keys)
}

/// [`check_and_resolve_all`] keeping each diagnostic's [`DiagnosticKey`]
/// (#898): the driver maps a linked program's item index back to the file
/// that owns it. The post-check phases' single diagnostic is keyed
/// `Module`.
pub fn check_and_resolve_all_keyed(hir: &HirModule) -> Result<HirModule, KeyedDiagnostics> {
    // #1021 (D-245): the same pre-pass `check_all_keyed` runs, in the same
    // place, over the same input -- which is precisely why `pycc check`
    // cannot accept a program `pycc build` then panics on in
    // `MirExpr::ty()`. The rewritten module is what everything downstream
    // (checker, solver, `monomorphize`, MIR lowering, codegen) consumes.
    let prepared = Prepared::new(hir)?;
    resolve_prepared(prepared.module(hir)).map_err(|d| prepared.rekey(d))
}

/// [`check_and_resolve_all_keyed`] over an already [`Prepared`] module.
fn resolve_prepared(hir: &HirModule) -> Result<HirModule, KeyedDiagnostics> {
    let function_local_names = module_function_local_names(hir);
    let signatures = checked_function_signatures_all(hir, &function_local_names)?;

    let mut resolved_hir = hir.clone();
    for item in &mut resolved_hir.items {
        let HirItem::Function {
            name,
            params,
            return_ty,
            ..
        } = item
        else {
            continue;
        };
        let (resolved_params, resolved_return) = signatures
            .get(name)
            .expect("every HIR function received an inferred signature");
        for ((_, param_ty), resolved_ty) in params.iter_mut().zip(resolved_params) {
            *param_ty = resolved_ty.clone();
        }
        *return_ty = resolved_return.clone();
    }

    // Issue #22/#402: no post-resolution redefinition recheck is needed
    // here. `checked_function_signatures_all` (called above) already runs
    // `check_incompatible_redefinitions` pre-resolution, and that function
    // now compares the full raw shape unconditionally, including any
    // `Ty::Infer` position (see its own doc comment) -- so every
    // redefinition pair that reaches this point is already raw-shape-
    // identical and is guaranteed to resolve to the same concrete
    // signature. A second check here would be unreachable dead code.
    let monomorphized = monomorphize(&resolved_hir).map_err(module_level)?;
    unroll_enum_loops(monomorphized).map_err(module_level)
}

/// The module both entry points check: the shared pre-pass, run in the same
/// place over the same input by `pycc check` and `pycc build`.
///
/// 1. #1021 (D-245): every empty container literal is resolved before any
///    checking. `None` when the module has none, which keeps the no-clone
///    property for every other program. See `crate::empty_container`.
/// 2. #1265 (D-245's 2026-09-24 amendment): a provisional `self.xs = []`
///    slot step 1 could not type is refused here, ahead of D-210's
///    redeclaration check -- which would otherwise report the placeholder as
///    a `T0052` naming `list[<inferred>]` -- and ahead of every other check.
/// 3. #1337 (D-254): every inherited method body whose behaviour depends on
///    its receiver's class gets a receiver-exact copy (see
///    `crate::inherited_copies`). It runs after step 1 so the copies inherit
///    the resolved slot types. A diagnostic raised inside a copy is re-keyed
///    to the copied item by [`Prepared::rekey`].
struct Prepared {
    resolved: Option<HirModule>,
    copied: Option<inherited_copies::CopiedModule>,
}

impl Prepared {
    fn new(hir: &HirModule) -> Result<Self, KeyedDiagnostics> {
        let resolved = empty_container::resolve_empty_containers(hir);
        let base = resolved.as_ref().unwrap_or(hir);
        empty_container::attr_slot::reject_unresolved_attr_slots(base)?;
        let copied = inherited_copies::add_inherited_copies(base);
        Ok(Prepared { resolved, copied })
    }

    fn module<'a>(&'a self, hir: &'a HirModule) -> &'a HirModule {
        match (&self.copied, &self.resolved) {
            (Some(copied), _) => &copied.module,
            (None, Some(resolved)) => resolved,
            (None, None) => hir,
        }
    }

    fn rekey(&self, diagnostics: KeyedDiagnostics) -> KeyedDiagnostics {
        match &self.copied {
            Some(copied) => copied.rekey(diagnostics),
            None => diagnostics,
        }
    }
}

/// First-diagnostic view of [`check_with_signatures_all`], kept for the
/// crate's unit tests that compare the checker's own first pick against the
/// solver's.
#[cfg(test)]
pub(super) fn check_with_signatures(
    hir: &HirModule,
    signatures: &FunctionSignatures,
    function_local_names: &[Vec<&str>],
) -> Result<(), Diagnostic> {
    check_with_signatures_all(hir, signatures, function_local_names).map_err(first_keyed)
}

/// Registers every class and every function signature, then runs
/// [`check_with_environment_all`]: one diagnostic per failing function.
pub(super) fn check_with_signatures_all(
    hir: &HirModule,
    signatures: &FunctionSignatures,
    function_local_names: &[Vec<&str>],
) -> Result<(), KeyedDiagnostics> {
    let mut env = Environment::new();
    // D-154 (Part 1 of #375): register every declared class before
    // checking any statement body -- a class must be usable (instantiated,
    // its instances passed around) from anywhere in the module, the same
    // "visible regardless of source position" requirement functions
    // already get from pass 1 below.
    class::bind_classes(&mut env, hir);
    // Pass 1: register every function's signature before checking any
    // statement body, matching Python's own "a module runs top to bottom,
    // but any def already executed is callable" semantics -- top-level
    // code and other function bodies (D-040) both need to see every
    // function regardless of its position in the file.
    for item in &hir.items {
        if let HirItem::Function {
            name,
            params,
            return_ty: item_return_ty,
            ..
        } = item
        {
            let (param_tys, return_ty) = signatures
                .get(name)
                .expect("every HIR function received an inferred signature");
            // D-133/D-134: a generic function's *original* body (still
            // carrying `Ty::Param`) is registered separately so a call
            // site can be resolved via `instantiate_generic_call` --
            // `signatures` itself already carries the same `Ty::Param`
            // entries (never `Ty::Infer`, so it survives both the
            // concrete-fast-path and solver-inferred paths unchanged), but
            // has no room for the body substitution needs.
            if is_generic_signature(params, item_return_ty) {
                env.bind_generic(name.clone(), item.clone());
            }
            env.bind_function(name.clone(), param_tys.clone(), return_ty.clone());
        }
    }
    check_with_environment_all(hir, env, function_local_names)
}

/// The annotation-driven check pass over a module whose functions are
/// already registered in `env`, collecting one diagnostic per failing
/// function (D-220 rule C2).
///
/// Pass 2 (top-level statements) is sequential over a growing environment,
/// so its first failure is reported alone as `(TopLevel(i), d)` and pass 3
/// does *not* run: a function reading a global bound after the failing
/// statement would otherwise produce a false `T0021`. Pass 3 checks
/// **every** function body and records `(Function(i), d)` for each failure -- one diagnostic per
/// function, the function's own first error. Methods are `HirItem::Function`s
/// with mangled `Class.method` names, so a class with two broken methods
/// yields two entries with no special casing.
pub(super) fn check_with_environment_all(
    hir: &HirModule,
    mut env: Environment,
    function_local_names: &[Vec<&str>],
) -> Result<(), KeyedDiagnostics> {
    // Part 1 of #883 (#962): this is the common sink of both `Environment`
    // constructors (`check_with_signatures_all` and
    // `concrete_function_environment`), so the alias table is filled here
    // exactly once rather than in each constructor -- a fully annotated
    // module takes the concrete path and would otherwise never see it.
    env.std_module_aliases = crate::std_receiver::bind_std_module_aliases(&hir.imports);
    // #1316: which names a function body may read as a module-level foreign
    // `object` (see the field's own doc comment).
    env.foreign_globals = crate::foreign::foreign_object_names(&hir.imports)
        .into_iter()
        .map(str::to_string)
        .collect();
    // Part 1 of #1026 pre-seeded every foreign import as a definitely-bound
    // `Ty::Object` here, before the source-order pass. PR 2a of #1081
    // removed that seed: Part 1's refusal of the *read* was what made the
    // seed harmless, and Part 2 lifted the read (`expr.rs`'s `Name` arm),
    // so the seed started admitting a module-body read placed *above* its
    // own `import` -- which then compiled into a global-initialization trap
    // instead of the compile error CPython's `NameError` justifies.
    // `bind_foreign_objects_at` below binds each import at its recorded
    // position instead, so such a read is now an ordinary unbound-name
    // `T0021`. Function bodies are unaffected: D-041's pass 3 runs after
    // the whole loop, by which point every import is bound.
    // Issue #22: clear `defined_functions` before the top-level source-order
    // pass. `bind_function` (called by `check_with_signatures_all`'s pass 1
    // or `concrete_function_environment`) adds every function to this set,
    // but for top-level checking we need to track which `def`s have actually
    // been *executed* in source order -- a call before the `def`'s position
    // is a NameError in CPython and must be rejected here. Function bodies
    // (pass 3) get a fresh seed of all function names via
    // `child_for_function`, so they're unaffected.
    env.defined_functions.clear();
    // Pass 2: check every top-level statement in source order, growing
    // `env`'s bindings as module-level assignments are encountered --
    // ordinary top-level code is still checked top-to-bottom (a top-level
    // forward reference to a not-yet-assigned name is a genuine error).
    // A `def` executes at its own position in that order and rebinds its
    // name to the function (D-110, refined by PR #252's review): later
    // `helper()` calls resolve the function exactly as CPython does, while
    // a value assignment *after* the `def` shadows it again. The `def` only
    // marks the name def-rebound -- it must NOT erase the representation
    // record in `bindings`, which D-040's sticky-representation rule keeps
    // consulting so an incompatible later reassignment still fails T0023.
    // The gate therefore tests the net source-order binding, in this pass
    // and in pass 3's final environment alike.
    for (index, item) in hir.items.iter().enumerate() {
        // PR 1c of #1080 review finding 2: an `import` runs at its own
        // position too. Re-binding here is what lets a foreign import
        // supersede an earlier same-named `def` -- the pre-seed above
        // cannot, since the `def` below rebinds the name after it.
        crate::foreign::bind_foreign_objects_at(&mut env, &hir.imports, index);
        match item {
            HirItem::TopLevelStmt(stmt) => {
                check_stmt(&mut env, stmt).map_err(|diagnostic| top_level(index, diagnostic))?;
                // Issue #769 (Part 2 of #747): applied uniformly with every
                // other sequential-statement-list call site in this crate
                // for consistency, even though `HirStmt::Return` (the only
                // terminator `definitely_terminates` recognizes) cannot
                // syntactically appear at module top level -- so this is a
                // structural no-op here today, not dead functionality.
                narrow::apply_post_if_narrowing(&mut env, stmt);
            }
            HirItem::Function { name, .. } => {
                env.def_rebound.insert(name.clone());
                // Issue #22: a `def` at its source position makes the
                // function name callable from this point forward in
                // top-level code. Calls before this point (the name is
                // not yet in `defined_functions`) are rejected by
                // `infer_expr_in`'s `HirExpr::Call` arm.
                env.defined_functions.insert(name.clone());
            }
        }
    }
    // An `import` after the last item records the item count itself, which
    // the loop above never reaches -- the common shape for a trailing
    // import, and the one D-041's pass 3 then checks bodies against.
    crate::foreign::bind_foreign_objects_at(&mut env, &hir.imports, hir.items.len());
    // Pass 3: check every function body against a clone of `env` as it
    // stands once the whole module's top-level code has been processed
    // (D-041) -- a function can read any module-level global regardless of
    // whether its own `def` appears before or after that global's
    // assignment in the file, since real Python only evaluates a function
    // body when it's *called*, typically after the module has finished
    // running top to bottom.
    let mut collected = KeyedDiagnostics::new();
    for (index, (item, local_names)) in hir.items.iter().zip(function_local_names).enumerate() {
        if let HirItem::Function {
            name,
            params,
            return_ty,
            body,
            ..
        } = item
        {
            // D-133/D-134: a generic function's body is checked through
            // `check_generic_function_in` -- the shape gate, the Critical
            // self/mutual-generic-recursion rejection, and then the same
            // ordinary sibling-aware `check_function_in` body check every
            // non-generic function gets (PR-13 final review I3: the earlier
            // env-less variant could not see any sibling function, so a
            // generic body calling an ordinary sibling wrongly reported
            // "call to undefined function").
            let checked = if is_generic_signature(params, return_ty) {
                check_generic_function_in(&env, item, local_names)
            } else {
                check_function_in(&env, item, local_names)
            }
            // #1420: a receiver-exact copy must not infer a return its
            // callers, typed through the origin, cannot see.
            .and_then(|()| inherited_copies::check_copy_return(&env, name));
            if let Err(diagnostic) = checked {
                // #1418: the check phase's half of the widened comparison
                // method's `T0022` help (the solver's is in
                // `constraints::signatures`).
                let diagnostic = crate::not_implemented::widened_return_help(body, diagnostic);
                collected.push((DiagnosticKey::Function(index), diagnostic));
            }
        }
    }
    if collected.is_empty() {
        Ok(())
    } else {
        Err(collected)
    }
}

/// Type-checks a module without materializing a resolved HIR clone.
///
/// Use [`check_and_resolve`] when a downstream compiler stage needs concrete
/// private-helper signatures in the returned HIR.
///
/// First-diagnostic view of [`check_all`] (D-217's `parse`/`parse_all`
/// precedent) for the crate's many test, bench, and downstream callers that
/// consume a single `Diagnostic`: the `Err` is exactly `check_all`'s first
/// collected diagnostic, byte-identical to what this function reported
/// before per-function collection landed (D-220). The `.expect` in
/// `first_diagnostic` follows D-219's `lower_checked` precedent: the panic
/// path lives in libcore and adds no in-crate region.
pub fn check(hir: &HirModule) -> Result<(), Diagnostic> {
    check_all(hir).map_err(first_diagnostic)
}

/// Type-checks a module, collecting one diagnostic per failing function and
/// at most one module-level diagnostic (Part 3 of #864, D-220); the `Err` is
/// never empty.
///
/// Order (D-220 rule 4): a pre-check failure (an incompatible
/// redefinition or attribute redeclaration) is reported alone. Otherwise,
/// if the solver's list is module-level (a failure in its top-level walk
/// or in a post-body phase such as `propagate_binop_constraints`), that
/// one diagnostic is reported alone and the checker's list is dropped,
/// because a post-body solver diagnostic cannot be matched by function to
/// the checker's entry for the same error -- see `merge_solver_first`.
/// Otherwise the solver's per-function diagnostics are reported in item
/// order, then every checker entry -- per-function or module-level --
/// whose function the solver did not flag, in the checker's order. If the
/// solver passes, the checker's list against the solved signatures is
/// reported on its own.
pub fn check_all(hir: &HirModule) -> Result<(), Vec<Diagnostic>> {
    check_all_keyed(hir).map_err(drop_keys)
}

/// [`check_all`] keeping each diagnostic's [`DiagnosticKey`] (#898): the
/// driver maps a linked program's item index back to the file that owns
/// it. A pre-check failure is keyed `Module`.
pub fn check_all_keyed(hir: &HirModule) -> Result<(), KeyedDiagnostics> {
    // #1021 (D-245): resolve every empty container literal *before* any
    // checking, so the check path and the build path see the identical
    // module (see [`Prepared`]).
    let prepared = Prepared::new(hir)?;
    check_prepared(prepared.module(hir)).map_err(|d| prepared.rekey(d))
}

/// [`check_all_keyed`] over an already [`Prepared`] module.
fn check_prepared(hir: &HirModule) -> Result<(), KeyedDiagnostics> {
    let function_local_names = module_function_local_names(hir);
    // Issue #22: reject incompatible redefinitions before trying either the
    // concrete or solver path -- including a same-arity, `Ty::Infer`-
    // involving mismatch (see `check_incompatible_redefinitions`'s own doc
    // comment). Calling it here (not inside `check_with_environment_all`)
    // ensures the error is returned directly rather than being masked by
    // the concrete-path fallback to the solver path.
    check_incompatible_redefinitions(hir).map_err(module_level)?;
    // #676 (D-210): reject a cross-MRO attribute redeclaration with a
    // differing declared type before any expression using that attribute
    // is type-checked -- see the function's own doc comment for why this
    // must be a class-definition-time rejection rather than a coercion.
    check_incompatible_attribute_redeclarations(hir).map_err(module_level)?;
    // The public validation-only API has no resolved-signature result to
    // return. Avoid building a temporary concrete signature map and then
    // cloning it into an `Environment`: construct that environment directly.
    // On validation failure, keep the concrete pass's per-function list and
    // merge it solver-first per function (D-220), exactly as
    // `checked_function_signatures_all` does.
    let concrete = match concrete_function_environment(hir) {
        Some(env) => match check_with_environment_all(hir, env, &function_local_names) {
            Ok(()) => return Ok(()),
            Err(collected) => Some(collected),
        },
        None => None,
    };
    match infer_function_signatures_with_solver_all(hir, &function_local_names) {
        Err(solver) => Err(merge_solver_first(solver, concrete)),
        Ok(signatures) => check_with_signatures_all(hir, &signatures, &function_local_names),
    }
}

/// First-diagnostic view of [`checked_function_signatures_all`], kept for
/// the crate's unit tests that pin the solver-first selection.
#[cfg(test)]
pub(crate) fn checked_function_signatures(
    hir: &HirModule,
    function_local_names: &[Vec<&str>],
) -> Result<FunctionSignatures, Diagnostic> {
    checked_function_signatures_all(hir, function_local_names).map_err(first_keyed)
}

/// `check_and_resolve_all`'s check half: the same pre-checks, concrete fast
/// path, solver, and per-function solver-first merge as [`check_all`], but
/// returning the resolved signatures on success.
pub(crate) fn checked_function_signatures_all(
    hir: &HirModule,
    function_local_names: &[Vec<&str>],
) -> Result<FunctionSignatures, KeyedDiagnostics> {
    // Issue #22: reject incompatible redefinitions before trying either the
    // concrete or solver path (same rationale as `check_all`'s own call).
    check_incompatible_redefinitions(hir).map_err(module_level)?;
    // #676 (D-210): same rationale and call-site timing as `check_all`'s
    // own call -- this entry point (via `check_and_resolve`) is also
    // reachable from `pycc build` without an earlier `pycc check`/`check`
    // call, so it needs its own guard against a cross-MRO attribute
    // redeclaration.
    check_incompatible_attribute_redeclarations(hir).map_err(module_level)?;
    // Fully annotated valid modules have no inference variables to constrain.
    // Validate them once and avoid the preceding constraint-collection walk.
    // If validation fails, keep its per-function list and fall through to
    // the solver: per function, the solver's diagnostic wins when both
    // phases flagged it (D-220), so the first diagnostic is the same one
    // the historical solver-first sequence selected before this fast path
    // existed.
    let concrete = match concrete_function_signatures(hir) {
        Some(signatures) => {
            match check_with_signatures_all(hir, &signatures, function_local_names) {
                Ok(()) => return Ok(signatures),
                Err(collected) => Some(collected),
            }
        }
        None => None,
    };
    let signatures = match infer_function_signatures_with_solver_all(hir, function_local_names) {
        Err(solver) => return Err(merge_solver_first(solver, concrete)),
        Ok(signatures) => signatures,
    };
    check_with_signatures_all(hir, &signatures, function_local_names)?;
    Ok(signatures)
}

/// The per-function solver-first merge (D-220 rule C4).
///
/// `solver` is the solver's keyed list; `concrete` is the concrete fast
/// path's keyed list when the module is fully annotated and that pass
/// failed, `None` otherwise. The result is `solver`'s entries in their own
/// order, followed by every `concrete` entry whose key the solver did not
/// flag, in `concrete`'s order. Per key the solver's diagnostic wins when
/// both phases flagged the function -- the pre-#868 rule, which selected the
/// solver's diagnostic for the whole module, applied per function -- and a
/// function only the checker flagged is still reported, so a `return "a"`
/// typo in one function no longer hides every checker-only diagnostic
/// (`T0043`, `T0025`, arity, ...) elsewhere in the file.
///
/// The order is *not* a global item-order interleave: a checker-only entry
/// for an earlier function would then precede the solver's first entry and
/// change the first diagnostic (D-217 rule 2).
///
/// A non-`Function`-keyed solver entry (by the [`KeyedDiagnostics`]
/// invariant, the only entry) is reported alone. It comes from the top-level constraint
/// walk or from a post-body phase such as `propagate_binop_constraints`,
/// which runs after every body was walked and therefore cannot attribute
/// its diagnostic to a function even though it usually originates in one
/// (`x + "s"` inside `f`). Key-based dedup cannot see that the checker's
/// `Some(f)` entry is the same error, so appending `concrete` would report
/// it twice; keeping the one solver line also keeps "a module-level failure
/// stops the pass at one" true for the solver as well as the checker.
fn merge_solver_first(
    solver: KeyedDiagnostics,
    concrete: Option<KeyedDiagnostics>,
) -> KeyedDiagnostics {
    // One pass over `solver` to index its keys: with F failing functions both
    // lists are Θ(F) long, and rescanning `solver` per `concrete` entry would
    // make the merge Θ(F²) on error-heavy generated inputs.
    let solver_keys: HashSet<DiagnosticKey> = solver.iter().map(|(key, _)| *key).collect();
    let module_level_failure = solver_keys
        .iter()
        .any(|key| !matches!(key, DiagnosticKey::Function(_)));
    let checker_only: KeyedDiagnostics = concrete
        .filter(|_| !module_level_failure)
        .into_iter()
        .flatten()
        .filter(|(key, _)| !solver_keys.contains(key))
        .collect();
    solver.into_iter().chain(checker_only).collect()
}

#[cfg(test)]
mod tests;
