//! Constraint term of a name read.

use super::*;

pub(crate) fn collect_name_constraints(
    parents: &mut [usize],
    concrete: &mut [Option<Ty>],
    env: &ConstraintEnvironment<'_, '_>,
    expr: &HirExpr,
) -> Result<Option<TypeTerm>, Diagnostic> {
    let HirExpr::Name(name) = expr else {
        unreachable!()
    };
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
            pycc_std::StdSymbolKind::Function { .. } => Err(std_function_used_as_a_value(name)),
            pycc_std::StdSymbolKind::EnumMarker => Err(enum_marker_is_not_a_value(name)),
            pycc_std::StdSymbolKind::AnnotationMarker => {
                Err(annotation_marker_is_not_a_value(name))
            }
            pycc_std::StdSymbolKind::CastMarker => Err(cast_marker_is_not_a_value(name)),
            pycc_std::StdSymbolKind::TypeCheckingMarker => {
                Err(type_checking_marker_is_not_a_value(name))
            }
            pycc_std::StdSymbolKind::TypingFormMarker => Err(typing_form_is_not_a_value(name)),
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
                crate::expr::reject_memoryview_read(name, &ty, env.owned_buffers.contains(name))?;
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
