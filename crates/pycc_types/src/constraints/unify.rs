//! Term unification: union-find roots, resolved terms and inference conflicts.

use super::*;

pub(crate) fn root(parents: &mut [usize], var: usize) -> usize {
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

pub(crate) fn resolved_private_signature_term(
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
pub(crate) fn inference_conflict(
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
        // Two known types: an `int` and a `float` conflict (only an
        // inference variable widens to `int | float`), unless one side is
        // already `int | float`.
        // A returned value the declared return type holds (`return x` of
        // a `str` from `-> str | None`).
        (Ok(left), Ok(right))
            if declared == Some(&left)
                && crate::assign_check::is_assignable(right.clone(), left.clone()) =>
        {
            Ok(false)
        }
        (Ok(left), Ok(right)) => merge_inferred_types(left.clone(), right.clone())
            .filter(|merged| *merged != Ty::Num || left == Ty::Num || right == Ty::Num)
            // A written return annotation is the type: a value it does not
            // hold conflicts even where two inferred types would join
            // (`int` and `int | None`).
            .filter(|merged| declared.is_none_or(|declared| merged == declared))
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
            let demanded = demanded_root(root);
            let merged = match concrete[root].clone() {
                Some(current) => {
                    with_demand(demanded, || merge_inferred_types(current.clone(), ty.clone()))
                        .ok_or_else(|| inference_conflict(code, context, current, ty, declared))?
                }
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
            let demanded = demanded_root(left_root) | demanded_root(right_root);
            let merged = match (concrete[left_root].clone(), concrete[right_root].clone()) {
                // `declared` is threaded through for consistency even though
                // this arm is unreachable while it is `Some` (see the ordering
                // note above): a future caller that made it reachable would get
                // the right message rather than a silently stale one.
                (Some(left), Some(right)) => Some(
                    with_demand(demanded, || merge_inferred_types(left.clone(), right.clone()))
                        .ok_or_else(|| inference_conflict(code, context, left, right, declared))?,
                ),
                (Some(ty), None) | (None, Some(ty)) => Some(ty),
                (None, None) => None,
            };
            parents[right_root] = left_root;
            concrete[left_root] = merged;
            if demanded {
                DEMANDED.with(|roots| roots.borrow_mut().insert(left_root));
            }
            Ok(true)
        }
    }
}

/// The one type two inferred types of a term agree on, or `None` when they
/// conflict: equal types, `bool` with `int` (`int`), a fixed tuple of `T`s
/// with `tuple[T, ...]` (the variadic tuple) and, in non-strict mode, a
/// scalar with a CPython object (the object).
pub(super) fn merge_inferred_types(left: Ty, right: Ty) -> Option<Ty> {
    if left == right {
        Some(left)
    } else if matches!((&left, &right), (Ty::Bool, Ty::Int) | (Ty::Int, Ty::Bool)) {
        Some(Ty::Int)
    } else if !DEMANDING.with(std::cell::Cell::get)
        && [&left, &right]
            .iter()
            .all(|ty| matches!(ty, Ty::Int | Ty::Bool | Ty::Float | Ty::Num))
    {
        // An `int` and a `float` (a helper called with both, a return of
        // each) are both held by `int | float`.
        Some(Ty::Num)
    } else if let (Ty::VarTuple(element), Ty::Tuple(items)) | (Ty::Tuple(items), Ty::VarTuple(element)) =
        (&left, &right)
        && pycc_hir::tuple_fits_var_tuple(items, element)
    {
        // A fixed-arity tuple of `T`s is a `tuple[T, ...]`.
        Some(Ty::VarTuple(element.clone()))
    } else if crate::non_strict::is_non_strict()
        && [&left, &right].contains(&&Ty::Object)
        && [&left, &right].iter().all(|ty| {
            matches!(ty, Ty::Object | Ty::Int | Ty::Float | Ty::Bool | Ty::Str | Ty::None)
                || optional_join(ty, &Ty::None).is_some()
        })
    {
        // Non-strict mode (#3): a term inferred both as a CPython object
        // and as a scalar (a helper called with an object and with an
        // `int`) is the object, which holds either.
        Some(Ty::Object)
    } else if crate::non_strict::is_non_strict()
        && [&left, &right]
            .iter()
            .all(|ty| matches!(ty, Ty::Object | Ty::Instance(_)) || crate::foreign::is_boxable_callable(ty))
    {
        // v0.9: so is one inferred as instances of two classes, or as an
        // instance and an object (a helper called with each node class,
        // `_format(self)`): the instance crosses as its carrier, a function
        // value of object parameters as a CPython callable over it.
        Some(Ty::Object)
    } else {
        // A term inferred both as `None` and as a value (a helper parameter
        // defaulting to `None` and called with an `int`, a helper that
        // returns `None` on one path and a list on another) is the
        // optional value.
        optional_join(&left, &right).map(|inner| Ty::Optional(Box::new(inner)))
    }
}

/// The `T` when `left` and `right` are `None`, `T` or `T | None` with at
/// least one `None` side and one `T` side, for a `T` that can be optional.
fn optional_join(left: &Ty, right: &Ty) -> Option<Ty> {
    // Non-strict mode keeps a pointer `T | None` other than `str | None`
    // (`pycc_mir::optional_str`) a CPython object.
    let optional_inner = |ty: &Ty| {
        matches!(ty, Ty::Int | Ty::Float | Ty::Bool | Ty::Str)
            || (!crate::non_strict::is_non_strict() && pycc_hir::is_pointer_optional_inner(ty))
    };
    let scalar = |ty: &Ty| match ty {
        Ty::Optional(inner) if optional_inner(inner) => Some((**inner).clone()),
        ty if optional_inner(ty) => Some(ty.clone()),
        _ => None,
    };
    match (left, right) {
        (Ty::None, other) | (other, Ty::None) => scalar(other),
        (Ty::Optional(_), other) | (other, Ty::Optional(_)) => {
            let (a, b) = (scalar(left)?, scalar(other)?);
            (a == b).then_some(a)
        }
        _ => None,
    }
}

thread_local! {
    /// Set while a merge must not widen an `int` and a `float` to
    /// `int | float` ([`unify_demanded`]).
    static DEMANDING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// The union-find roots of the running solve a use demanded a type of
    /// ([`unify_demanded`]): a later value of the other numeric kind
    /// conflicts with the demand, whichever comes first.
    static DEMANDED: std::cell::RefCell<std::collections::BTreeSet<usize>> =
        const { std::cell::RefCell::new(std::collections::BTreeSet::new()) };
}

/// Whether `root` holds a demand; while one is being made, records it.
fn demanded_root(root: usize) -> bool {
    if DEMANDING.with(std::cell::Cell::get) {
        DEMANDED.with(|roots| roots.borrow_mut().insert(root));
        return true;
    }
    DEMANDED.with(|roots| roots.borrow().contains(&root))
}

/// Runs `merge` with the no-widening rule on when `demanded`.
fn with_demand<T>(demanded: bool, merge: impl FnOnce() -> T) -> T {
    let before = DEMANDING.with(|demanding| demanding.replace(demanded || demanding.get()));
    let result = merge();
    DEMANDING.with(|demanding| demanding.set(before));
    result
}

/// Runs one solve (`signatures::infer_function_signatures_with_solver_all`)
/// with its own, initially empty set of demanded roots.
pub(crate) fn with_fresh_demands<T>(solve: impl FnOnce() -> T) -> T {
    let saved = DEMANDED.with(|roots| std::mem::take(&mut *roots.borrow_mut()));
    let result = solve();
    DEMANDED.with(|roots| *roots.borrow_mut() = saved);
    result
}

/// [`unify_terms`] for a use that demands `ty` of `term` (a `range()`
/// bound): a `float` term conflicts with a demanded `int` instead of the
/// two widening to `int | float`, which only values bound together do.
pub(crate) fn unify_demanded(
    term: TypeTerm,
    ty: Ty,
    parents: &mut [usize],
    concrete: &mut [Option<Ty>],
    code: &'static str,
    context: &str,
) -> Result<bool, Diagnostic> {
    DEMANDING.with(|demanding| demanding.set(true));
    let result = unify_terms(term, Ok(ty), parents, concrete, code, context);
    DEMANDING.with(|demanding| demanding.set(false));
    result
}
