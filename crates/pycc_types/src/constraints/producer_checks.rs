//! Buffer-producer calls and rebinding refusals (buffer parameters, `Final`).

use super::*;

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
pub(crate) fn resolved_producer_call<'a>(
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
pub(crate) fn reject_non_int_producer_length(
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
pub(crate) fn reject_producer_annotation_mismatch(
    target: &str,
    annotation: &Ty,
) -> Result<(), Diagnostic> {
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
pub(crate) fn reject_buffer_parameter_rebinding(
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
pub(crate) fn reject_final_rebinding(
    env: &ConstraintEnvironment<'_, '_>,
    target: &str,
) -> Result<(), Diagnostic> {
    if env.finals.contains(target) {
        return Err(crate::final_reassignment(target));
    }
    Ok(())
}
