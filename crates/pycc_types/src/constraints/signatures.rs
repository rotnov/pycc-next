//! Module-level signature entry points of the constraint solver: the
//! fully-annotated fast path (`concrete_function_signatures`,
//! `concrete_function_environment`) and the solver walk that infers every
//! private-helper signature (`infer_function_signatures_with_solver_all`).
//!
//! Extracted from `constraints.rs` per AGENTS.md's file-decomposition rule
//! (issue #868, tracked by #544), laid out beside it the way `class.rs` and
//! `class/` already are. `constraints.rs` re-exports everything here with
//! `pub(crate) use signatures::*`, so every crate-root path is unchanged.
//! The driver that sequences these entry points (`checked_function_signatures_all`
//! and the public `check*` functions) lives in [`crate::module`].

use super::*;
#[cfg(test)]
use crate::module::first_keyed;
use crate::module::{DiagnosticKey, FunctionSignatures, KeyedDiagnostics, module_level, top_level};

pub(crate) fn concrete_function_signatures(hir: &HirModule) -> Option<FunctionSignatures> {
    let mut signatures = HashMap::default();
    for item in &hir.items {
        let HirItem::Function {
            name,
            params,
            return_ty,
            ..
        } = item
        else {
            continue;
        };
        if *return_ty == Ty::Infer || params.iter().any(|(_, ty)| *ty == Ty::Infer) {
            return None;
        }
        signatures.insert(
            name.clone(),
            (
                params.iter().map(|(_, ty)| ty.clone()).collect(),
                return_ty.clone(),
            ),
        );
    }
    Some(signatures)
}

/// Builds the function registry for a fully annotated module directly from
/// HIR. Unlike [`concrete_function_signatures`] followed by
/// [`check_with_signatures_all`], this creates each owned name and parameter vector
/// only once. `check` does not need to materialize a second signature map for a
/// downstream consumer, so its overwhelmingly common concrete, valid path can
/// validate with this registry directly.
pub(crate) fn concrete_function_environment(hir: &HirModule) -> Option<Environment> {
    if hir.items.iter().any(|item| match item {
        HirItem::Function {
            params, return_ty, ..
        } => *return_ty == Ty::Infer || params.iter().any(|(_, ty)| *ty == Ty::Infer),
        HirItem::TopLevelStmt(_) => false,
    }) {
        return None;
    }
    Some(annotated_function_environment(hir))
}

/// The same registry [`concrete_function_environment`] builds, but for *any*
/// module: instead of refusing the whole module when one signature still
/// carries `Ty::Infer`, it registers every function, an inferred signature
/// included, with whatever type that signature currently has. The registry
/// invariant below requires exactly that -- skipping such a function would
/// abort the class resolvers -- so do not narrow this to the concrete
/// signatures.
///
/// This is what the #1021 empty-container pre-pass needs: that pass runs
/// before private-helper inference has resolved anything, so demanding a
/// fully annotated module would leave it with an empty environment for
/// exactly the programs it exists to serve -- and a module-level global
/// initialized from an annotated helper (`VALUE = _base()`) would then fail
/// to resolve a container the equivalent non-empty literal resolves fine.
///
/// # Registry invariant
///
/// **Every mangled name any bound class's `methods`, `properties` (getter
/// and setter), `static_methods` or `class_methods` table carries resolves
/// through [`Environment::lookup_function`].** The trailing
/// [`crate::class::bind_classes`] call records every class member
/// unconditionally, and the class resolvers (`resolve_method_call`,
/// `resolve_attr_get`'s property arm, `resolve_static_call`,
/// `resolve_class_method_call`, `resolve_instantiation`) *panic* when a
/// table entry has no ordinary-function registration. So the registry may
/// not be a subset of the class tables, and a function whose signature
/// still carries `Ty::Infer` is registered with that `Ty::Infer` rather
/// than skipped.
///
/// Dropping such an entry instead would be unsound, not merely lossy: an
/// unannotated override (`class A(Base): def _one(self): ...`) removed from
/// `A`'s table lets the MRO walk fall through to `Base._one` and resolve the
/// call to the *base* class's return type -- wrong, not missed. Registering
/// the `Ty::Infer` signature keeps the override authoritative; the call then
/// yields `Ty::Infer`, and [`crate::empty_container`]'s own `concrete`
/// acceptance guard rejects any resolution containing one. That is what
/// preserves this pass's contract: an unresolved container is a recoverable
/// inference miss reported as `T0003`, never a wrong element type and never
/// an abort.
///
/// An annotated signature is still authoritative, so the registry can fail
/// to resolve a container but can never resolve one wrongly. On
/// [`concrete_function_environment`]'s path this is a provable no-op: that
/// caller returns `None` for any module carrying a `Ty::Infer` signature, so
/// every entry it reaches here is already concrete.
pub(crate) fn annotated_function_environment(hir: &HirModule) -> Environment {
    let mut functions = HashMap::default();
    let mut generics = HashMap::default();
    for item in &hir.items {
        let HirItem::Function {
            name,
            params,
            return_ty,
            ..
        } = item
        else {
            continue;
        };
        // The generics table keeps its old membership exactly. A partially
        // annotated signature *can* satisfy both predicates at once -- a
        // private method of a PEP 695 generic class can carry `Ty::Param` in
        // its return type and `Ty::Infer` in a parameter -- and an entry whose
        // parameters are not yet known is of no use to
        // `instantiate_generic_call`, so registering the signature in
        // `functions` (which is what the class tables require) deliberately
        // does not extend `generics`.
        let carries_infer =
            *return_ty == Ty::Infer || params.iter().any(|(_, ty)| *ty == Ty::Infer);
        if !carries_infer && is_generic_signature(params, return_ty) {
            generics.insert(name.clone(), item.clone());
        }
        functions.insert(
            name.clone(),
            (
                params.iter().map(|(_, ty)| ty.clone()).collect(),
                return_ty.clone(),
            ),
        );
    }
    let mut env = Environment {
        bindings: HashMap::default(),
        // Part 2a of #1142 (#1165): a module-level environment owns no
        // buffers; the producer is refused at module scope.
        owned_buffers: HashSet::default(),
        declared: HashMap::default(),
        functions: Arc::new(functions),
        def_rebound: HashSet::default(),
        defined_functions: HashSet::default(),
        generics: Arc::new(generics),
        classes: Arc::new(HashMap::default()),
        synthetic_classes: Arc::new(HashSet::default()),
        own_type_param: None,
        generator_sig: None,
        stop_values: HashMap::default(),
        stop_value_errors: HashMap::default(),
        current_class: None,
        finals: HashSet::default(),
        in_except_handler: false,
        // A module-level environment: `child_for_function` is what flips
        // this, and this constructor's result is that same module scope.
        in_function_body: false,
        // Part 2b of #1142 (#1164): module scope has no function body to
        // walk; `check_function_in` sets this per function.
        returns_inside_finally: false,
        return_inferred: false,
        in_classmethod: false,
        narrowed: HashMap::default(),
        // Overwritten at `check_with_environment_all`'s entry, the common
        // sink of both `Environment` constructors (#962).
        std_module_aliases: Vec::new(),
        // Seeded at the same sink for the same reason (#1316).
        foreign_globals: HashSet::default(),
        loop_exits: Default::default(),
        params: HashSet::default(),
    };
    // Part 1 of #541: register the class table through `bind_class` (via
    // `bind_classes`) rather than by populating `classes` directly, so this
    // second `Environment` constructor cannot drift from the first on which
    // entries are marked synthetic. `bind_class` and `bind_synthetic_class`
    // are together the sole mutators of both tables precisely so that
    // invariant holds by construction.
    crate::class::bind_classes(&mut env, hir);
    env
}

/// First-diagnostic view of [`infer_function_signatures_with_solver_all`],
/// kept for the crate's unit tests that pin the solver's own first pick.
#[cfg(test)]
pub(crate) fn infer_function_signatures_with_solver(
    hir: &HirModule,
    function_local_names: &[Vec<&str>],
) -> Result<FunctionSignatures, Diagnostic> {
    infer_function_signatures_with_solver_all(hir, function_local_names).map_err(first_keyed)
}

/// Infers every private-helper signature by constraint solving, collecting
/// one diagnostic per failing function body (Part 3 of #864, D-220 rule
/// C3); the `Err` is never empty.
///
/// The top-level constraint walk is sequential over the module globals, so
/// its first failure is returned alone as `(None, d)`. The per-body loop
/// records `(Some(i), d)` for each body whose constraint collection or
/// implicit-return unification fails and continues with the next body; when
/// it collected anything the function returns that list *without* running
/// the post-body phases (`propagate_binop_constraints`,
/// `apply_annotation_defaults`, the `T0021` resolution loop) -- none of them
/// ran after a body failure before this part either, and skipping them
/// avoids reporting resolution failures caused by the missing constraints.
/// A post-phase failure with no body failure is `(None, d)`, as before.
///
/// Invariant (relied on by `module::merge_solver_first`): the `Err` is
/// either exactly `[(None, d)]` or entirely `Some`-keyed -- the top-level
/// walk returns at once, and the post-phases run only when no body was
/// collected, so a `None` key never coexists with a `Some` key.
///
/// Continuing past a failed body is sound because `unify_terms` returns
/// before mutating the union-find on a conflict, so it only ever holds
/// constraints that were *accepted*: the unifications a failing body made
/// before its error are genuine constraints that body imposes, and a later
/// body conflicting with them has a genuine conflict (D-220 records this
/// cross-body coupling as the one observable consequence of not rolling the
/// union-find back per body).
pub(crate) fn infer_function_signatures_with_solver_all(
    hir: &HirModule,
    function_local_names: &[Vec<&str>],
) -> Result<FunctionSignatures, KeyedDiagnostics> {
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut signatures = HashMap::default();
    for item in &hir.items {
        if let HirItem::Function {
            name,
            params,
            return_ty,
            ..
        } = item
        {
            signatures.insert(
                name.clone(),
                (
                    params.iter().map(|(name, _)| name.clone()).collect(),
                    params
                        .iter()
                        .map(|(_, ty)| term_for_type(ty.clone(), &mut parents, &mut concrete))
                        .collect(),
                    term_for_type(return_ty.clone(), &mut parents, &mut concrete),
                ),
            );
        }
    }

    // A generator function's element type: the term its `yield`s unify
    // with while its body is walked (its signature's own return term is the
    // generator type, which a call site sees as unresolved until then).
    let mut generator_elements: HashMap<String, TypeTerm> = HashMap::default();
    // Each generator's declared send and return types, which inference
    // keeps as written.
    let mut generator_rest: HashMap<String, (Ty, Ty)> = HashMap::default();
    for item in &hir.items {
        if let HirItem::Function {
            name,
            return_ty: Ty::Generator(element),
            ..
        } = item
        {
            generator_elements.insert(
                name.clone(),
                term_for_type(element.0.clone(), &mut parents, &mut concrete),
            );
            generator_rest.insert(name.clone(), (element.1.clone(), element.2.clone()));
        }
    }

    let mut constraints = SolverConstraints::default();
    let mut globals = ConstraintEnvironment {
        bindings: HashMap::default(),
        local_names: &[],
        defs_rebound: HashSet::default(),
        maybe_bindings: HashSet::default(),
        opaque_bindings: HashSet::default(),
        foreign_objects: HashSet::default(),
        // #962: the module's alias table, read by the stdlib receiver
        // shadow check; copied field-by-field into every per-function
        // environment below.
        std_module_aliases: crate::std_receiver::bind_std_module_aliases(&hir.imports),
        // Part 2a of #1142 (#1165): module-level code is not a function
        // body, and nothing at module scope can own a buffer -- the
        // producer is refused there outright.
        owned_buffers: HashSet::default(),
        in_function_body: false,
        in_generator: false,
        generator_sig: None,
        // Part 2b of #1142 (#1164): module-level code is not a function
        // body, so there is no body to walk for a `return` in a `finally`.
        returns_inside_finally: false,
        // Part 2a of #1142 (#1165): D-244 #1129 statement (h) applied per
        // spelling. A `def ndarray` is already covered by `signatures`; a
        // `class ndarray` is what this set adds, because the solver has no
        // class table of its own. Part 1 of #1319 records a `class
        // frozenset` here for the same reason: the solver's `frozenset(...)`
        // arm must yield to it, and #1331 a `class hash` for its `hash(...)`
        // arm.
        shadowed_producers: hir
            .class_defs
            .iter()
            .map(|(class_name, _)| class_name.clone())
            .filter(|class_name| {
                crate::buffer::is_producer_spelling(class_name)
                    || class_name == crate::frozenset::FROZENSET
                    || class_name == crate::hash::HASH
            })
            .collect(),
        // #1165 review round 8: module scope binds no `Final` name this
        // solver consults -- the set is read only at the buffer producer's
        // seam, which module scope refuses outright. See the field's own
        // doc comment for why each body starts empty too.
        finals: HashSet::default(),
        // #1420: the `MethodCall` arm resolves a method on a user-class
        // instance through the module's own class table.
        class_defs: &hir.class_defs,
        narrowed: HashMap::default(),
    };
    // Part 1 of #1026: a foreign import binds a definite name whose type is
    // `Ty::Object`. It is recorded in `opaque_bindings` so that every piece
    // of machinery that already knows how to keep a definitely-bound name
    // out of the "unbound local" path -- rebinding removal, join-site
    // merging -- applies unchanged, and additionally in `foreign_objects`,
    // which is what makes the solver's `Name` arm hand back the concrete
    // term `Ok(Ty::Object)` rather than "no term at all".
    //
    // Both are needed. Without the opaque marker a read of the name would
    // fall through to the "not a solver-tracked local" case; without the
    // foreign marker a helper returning the module object would keep an
    // unresolved return variable and signature materialization would report
    // a misleading `T0021: ... add an annotation` for a helper that is
    // unannotated by design and whose return the marker already types as
    // `object` (since Part 1 of #1367 a `-> <module>` annotation would
    // also spell it). The names
    // deliberately do not go into `bindings`: see the `Name` arm in
    // `super::collect_expr_constraints` for why.
    for name in crate::foreign::foreign_object_names(&hir.imports) {
        globals.opaque_bindings.insert(name.to_string());
        globals.foreign_objects.insert(name.to_string());
    }
    // The one-time pre-pass above is sound precisely because a module in
    // which any *other* top-level binding spells a foreign import's name is
    // refused outright at lowering
    // (`pycc_hir::import::reject_shadowed_foreign_imports`), so no `def` or
    // assignment can precede or follow the import under that name and the
    // seed can never be stale. The one admitted rebinding, an
    // `except ImportError` fallback of the same `try` (#1485,
    // `docs/RUNTIME.md`), stores a boxed `None` or another foreign object
    // into the same `object` slot, so it keeps the seed right too.
    for (index, item) in hir.items.iter().enumerate() {
        match item {
            HirItem::TopLevelStmt(stmt) => {
                crate::locate::in_item(index, || {
                    collect_block_constraints(
                        &signatures,
                        &mut parents,
                        &mut concrete,
                        &mut constraints,
                        &mut globals,
                        std::slice::from_ref(stmt),
                        None,
                    )
                })
                .map_err(|diagnostic| {
                    top_level(index, crate::locate::place_item(diagnostic, index))
                })?;
            }
            // Mirror of pass 2's source-order `def` rebinding (D-110): the
            // `def` marks the name def-rebound in the accumulated globals
            // (without erasing its term, which representation tracking may
            // still need), so helper-body environments seeded from them see
            // the net binding, not a stale shadowed primitive.
            HirItem::Function { name, .. } => {
                globals.defs_rebound.insert(name.clone());
            }
        }
    }
    let mut collected = KeyedDiagnostics::new();
    for (index, (item, local_names)) in hir.items.iter().zip(function_local_names).enumerate() {
        let HirItem::Function {
            name,
            body,
            params,
            return_ty,
        } = item
        else {
            continue;
        };
        let signature = &signatures[name];
        let generator_sig = match return_ty {
            Ty::Generator(sig) if !crate::empty_container::contains_infer(return_ty) => {
                Some((**sig).clone())
            }
            _ => None,
        };
        let mut env = ConstraintEnvironment {
            bindings: globals.bindings.clone(),
            local_names,
            defs_rebound: globals.defs_rebound.clone(),
            // A function body runs only when it is called, so a global it
            // reads that the module binds on some paths only (a
            // module-level loop variable, `n` bound in `for _ in
            // range(t)`) holds a value of its one type there, or the read
            // raises `NameError`: its term types the function. The check
            // phase still decides whether the read is admitted.
            maybe_bindings: HashSet::default(),
            opaque_bindings: globals.opaque_bindings.clone(),
            foreign_objects: globals.foreign_objects.clone(),
            // #962: this literal copies field by field on purpose (it is
            // not a `.clone()`), so the alias table must be named here or
            // every function body would silently get an empty one.
            std_module_aliases: globals.std_module_aliases.clone(),
            // Part 2a of #1142 (#1165): a fresh body starts with no owned
            // buffers -- module scope cannot produce one -- and this is the
            // one place the function-body flag is set.
            owned_buffers: HashSet::default(),
            in_function_body: true,
            in_generator: generator_elements.contains_key(name),
            generator_sig,
            // Part 2b of #1142 (#1164), review round 5: the same
            // whole-function predicate the check phase reads, from the same
            // `pycc_hir` walk, so the two admissions cannot drift.
            returns_inside_finally: pycc_hir::body_returns_inside_finally(body),
            shadowed_producers: globals.shadowed_producers.clone(),
            finals: HashSet::default(),
            class_defs: globals.class_defs,
            // #1476: a module-scope narrowing never reaches a function body.
            narrowed: HashMap::default(),
        };
        for local_name in local_names.iter().copied() {
            env.bindings.remove(local_name);
            // A local name (parameter or body-assigned) re-binds within this
            // body, so a stale module-level def-rebound fact must not
            // survive for it (D-110, PR #252's round-6 review): a parameter
            // colliding with a def-rebound module name would otherwise skip
            // the mirror gate and be mislabeled "not bound before this use".
            env.defs_rebound.remove(local_name);
            env.maybe_bindings.remove(local_name);
            // Issue #771: same reasoning as `maybe_bindings` above — a
            // local name re-binds within this function body, so a stale
            // module-level opaque marker must not survive for it either.
            env.opaque_bindings.remove(local_name);
            // PR 1c of #1080 review finding 2: the foreign marker is the
            // second half of the same fact and must be removed with it.
            // Left behind, a local assigned a solver-opaque value would be
            // read back through the foreign-global provenance and infer as
            // `Ty::Object`, turning an unresolved-container inference into a
            // misleading return-type mismatch.
            //
            // Part 2 of #1026 (#1081) re-derived this strip rather than
            // relaxing it. The solver runs before the check phase, so it
            // still sees bodies the checker goes on to refuse: the
            // `AttrGet` arm hands back a real `Ok(Ty::Object)` term for
            // `numpy.pi`, and an assignment inside a function body records
            // it in `bindings` here even though `crate::expr` then rejects
            // that body outright (PR 2a of #1081). What the strip removes
            // is only the *provenance marker*, which answers a different
            // question ("is this name a foreign global?") and whose stale
            // answer was never about the local's actual value. The two
            // cases are pinned separately in this crate's tests.
            env.foreign_objects.remove(local_name);
        }
        // Use the current item's own parameter names, not the last-inserted
        // signature's names (#386): a redefined method shares its mangled
        // name but has its own parameter names, and checking its body against
        // the wrong names would report false T0021 "not bound" errors. The
        // type terms (signature.1) and return type (signature.2) come from
        // the last definition, which is correct — compatible redefinitions
        // have the same raw type shape (already validated by
        // check_incompatible_redefinitions), and the last definition is the
        // one bound at call sites.
        for (param_name, param_ty) in params.iter().map(|(n, _)| n).zip(&signature.1) {
            env.bindings.insert(param_name.clone(), param_ty.clone());
        }
        // #380 (PR-20): skip the constraint solver for abstract method
        // bodies. An abstract method's HIR body is just `Return(None)`,
        // but its declared return type may be non-`None` (e.g. `-> int`).
        // Running the solver on it would unify `None` with the declared
        // type and produce a spurious `T0022`. The type checker
        // (`check_and_resolve`) also skips abstract method bodies.
        let is_abstract_method = name
            .split('.')
            .next()
            .filter(|class_name| *class_name != name)
            .and_then(|class_name| {
                hir.class_defs
                    .iter()
                    .find(|(n, _)| n == class_name)
                    .map(|(_, cd)| cd)
            })
            .is_some_and(|class_def| {
                let method_name = name.split('.').nth(1).unwrap_or("");
                class_def.abstract_methods.iter().any(|m| m == method_name)
            });
        if is_abstract_method {
            continue;
        }
        if let Err(diagnostic) = crate::locate::in_item(index, || {
            collect_block_constraints(
                &signatures,
                &mut parents,
                &mut concrete,
                &mut constraints,
                &mut env,
                body,
                Some(
                    generator_elements
                        .get(name)
                        .cloned()
                        .unwrap_or_else(|| signature.2.clone()),
                ),
            )
        }) {
            // #1418: a widened comparison method's native return explains
            // where its `object` return type came from.
            let diagnostic = crate::not_implemented::widened_return_help(body, diagnostic);
            let diagnostic = crate::locate::place_item(diagnostic, index);
            collected.push((DiagnosticKey::Function(index), diagnostic));
            continue;
        }
        if signature.2.is_err()
            && !contains_return(body)
            && let Err(diagnostic) = unify_terms(
                signature.2.clone(),
                Ok(Ty::None),
                &mut parents,
                &mut concrete,
                "T0022",
                "private helper implicit return",
            )
        {
            let diagnostic = crate::locate::place_item(diagnostic, index);
            collected.push((DiagnosticKey::Function(index), diagnostic));
        }
    }
    if !collected.is_empty() {
        return Err(collected);
    }

    // Annotation bounds are directional defaults, not hard equalities. Let
    // every call/operator fact settle first, aggregate all remaining bounds
    // per union-find root, then propagate any selected fallback back through
    // operators. This keeps inference independent of body/declaration order.
    propagate_binop_constraints(&constraints.deferred.binops, &mut parents, &mut concrete)?;
    // #1343: container defaults run before the annotation defaults, so a
    // defaulted `set[int]` root is concrete before a scalar annotation
    // bound could claim it -- the order the hard `set[int]` guess had.
    super::set_comp::apply_container_defaults(
        &constraints.deferred.container_defaults,
        &mut parents,
        &mut concrete,
    );
    apply_annotation_defaults(
        &constraints.annotation_defaults,
        &mut parents,
        &mut concrete,
    )
    .map_err(module_level)?;
    propagate_binop_constraints(&constraints.deferred.binops, &mut parents, &mut concrete)?;
    for var in &constraints.opaque_returns {
        let return_root = root(&mut parents, *var);
        if concrete[return_root] == Some(Ty::None) {
            concrete[return_root] = Some(Ty::Object);
        }
    }
    let valued_roots: HashSet<usize> = constraints
        .valued_returns
        .iter()
        .map(|var| root(&mut parents, *var))
        .collect();
    for var in &constraints.object_returns {
        let return_root = root(&mut parents, *var);
        concrete[return_root] = match &concrete[return_root] {
            Some(Ty::None) => Some(Ty::None),
            None if !valued_roots.contains(&return_root) => Some(Ty::None),
            _ => Some(Ty::Object),
        };
    }

    let non_scalar_local_roots = constraints
        .non_scalar_local_terms
        .iter()
        .map(|&var| root(&mut parents, var))
        .collect::<HashSet<_>>();

    let mut resolved = HashMap::default();
    let mut open_generators: Vec<(String, Vec<Ty>)> = Vec::new();
    for (name, signature) in &signatures {
        let param_tys = signature
            .0
            .iter()
            .zip(signature.1.iter().cloned())
            .map(|(param_name, term)| {
                resolved_private_signature_term(
                    term,
                    &mut parents,
                    &concrete,
                    &non_scalar_local_roots,
                )
                .or_else(object_when_non_strict)
                .ok_or_else(|| {
                    if super::public_inference::reports_missing_annotation(name) {
                        return super::public_inference::at_function(hir, name, super::public_inference::parameter_needs_annotation(name, param_name));
                    }
                    Diagnostic::error(
                        "T0021",
                        format!(
                            "cannot infer type of parameter `{param_name}` in private helper `{name}`; add an annotation"
                        ),
                        Span::new(0, 0),
                    ).with_help(format!("add a type annotation to parameter `{param_name}`"))
                })
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(module_level)?;
        // A generator's element type, resolved like a return type; one the
        // terms leave open is typed from the body once every other
        // signature is known (`yield_fallback`).
        if let Some(element) = generator_elements.get(name) {
            let Some(element) = resolved_private_signature_term(
                element.clone(),
                &mut parents,
                &concrete,
                &non_scalar_local_roots,
            ) else {
                open_generators.push((name.clone(), param_tys));
                continue;
            };
            let (sent, returned) = generator_rest
                .get(name)
                .cloned()
                .unwrap_or((Ty::None, Ty::None));
            resolved.insert(
                name.clone(),
                (param_tys, Ty::generator_of(element, sent, returned)),
            );
            continue;
        }
        let return_ty = resolved_private_signature_term(
            signature.2.clone(),
            &mut parents,
            &concrete,
            &non_scalar_local_roots,
        )
        .or_else(object_when_non_strict)
        .ok_or_else(|| {
            if super::public_inference::reports_missing_annotation(name) {
                return super::public_inference::at_function(
                    hir,
                    name,
                    super::public_inference::return_needs_annotation(name),
                );
            }
            Diagnostic::error(
                "T0021",
                format!("cannot infer return type of private helper `{name}`; add an annotation"),
                Span::new(0, 0),
            )
            .with_help(format!("add a return type annotation to `{name}`"))
        })
        .map_err(module_level)?;
        resolved.insert(name.clone(), (param_tys, return_ty));
    }
    // Sorted: a body-typed generator may call another one, typed first.
    open_generators.sort_by(|a, b| a.0.cmp(&b.0));
    for (name, param_tys) in open_generators {
        let element =
            super::yield_fallback::infer_yield_from_body(hir, &name, &param_tys, &resolved)
                .or_else(object_when_non_strict)
                .ok_or_else(|| {
                    if super::public_inference::reports_missing_annotation(&name) {
                        return super::public_inference::at_function(
                            hir,
                            &name,
                            super::public_inference::return_needs_annotation(&name),
                        );
                    }
                    Diagnostic::error(
                        "T0021",
                        format!(
                            "cannot infer the type of the values generator `{name}` yields; add a \
                         return annotation (`Iterator[...]`)"
                        ),
                        Span::new(0, 0),
                    )
                    .with_help(format!("add a return type annotation to `{name}`"))
                })
                .map_err(module_level)?;
        let (sent, returned) = generator_rest
            .get(&name)
            .cloned()
            .unwrap_or((Ty::None, Ty::None));
        resolved.insert(name, (param_tys, Ty::generator_of(element, sent, returned)));
    }
    Ok(resolved)
}

/// Non-strict mode (#3): a helper parameter or return the solver cannot
/// type is a CPython object, as every unannotated Python name is. While
/// [`super::return_fallback`] collects a partial table, it is `Ty::Infer`.
fn object_when_non_strict() -> Option<Ty> {
    if super::return_fallback::collecting_partial() {
        return Some(Ty::Infer);
    }
    crate::non_strict::is_non_strict().then_some(Ty::Object)
}
