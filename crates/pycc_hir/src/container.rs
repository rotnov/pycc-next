//! Shared element-type gates for pycc's container types (D-228, issue #918).
//!
//! Before #918 the only way a `Ty::List`/`Ty::Dict`/`Ty::Set`/`Ty::Tuple`
//! could come into existence was by inferring a container *literal*'s type in
//! `pycc_types`, so the four codegen-capability gates (`T0034`, `T0036`,
//! `T0038`, `T0039`) lived there, next to the inference that produced the
//! type. Lowering a written `list[int]`-style annotation (D-228) creates the
//! same types in `pycc_hir`, which sits *below* `pycc_types` in the crate
//! graph and so cannot call into it. The gates therefore move down here and
//! `pycc_types` calls in, keeping a single definition of "which container
//! types does this version actually compile" for both producers.
//!
//! Seven further copies of the same *capability* rule deliberately stay where
//! they are rather than routing through here, because each states the rule in
//! terms of its own position and routing it would change user-visible text:
//! the slice-position `T0034` in `pycc_types::expr` ("cannot slice
//! `list[...]`") and the six comprehension gates in `pycc_types` (module-scope
//! and function-scope `ListCompAssign`/`SetCompAssign`/`DictCompAssign`, "got
//! a comprehension producing `list[...]`"). Giving this helper a
//! message-variant parameter to absorb them would make it harder to read than
//! the seven three-line checks it replaced, so the shared definition covers
//! the two *type-producing* sites -- container literals and container
//! annotations -- which are the two that must agree.
//!
//! Both entry points take an explicit [`Span`]. The `pycc_types` literal
//! callers pass what they passed before (`Span::new(0, 0)`, rendering as a
//! `1:1` caret) so their diagnostic output is byte-identical to the previous
//! release; annotation lowering passes the annotation's own source range, so
//! the newly reachable annotation diagnostics carry a real caret.

use crate::Ty;
use pycc_diag::{Diagnostic, Span};

/// Rejects one `tuple` element type this version's tuple codegen cannot
/// represent (`T0039`, D-116): a tuple is a fixed SSA struct of scalars, so
/// only `int`, `bool` and `float` elements are compilable.
///
/// This is deliberately *element*-shaped rather than a whole-`Ty` postcheck.
/// `pycc_types`' tuple-literal inference calls it from inside its own
/// element loop, immediately after inferring each element, so the elements
/// are gated in source order: an earlier element's type gate is reported
/// ahead of a later element's own inference failure (an undefined name,
/// say). Folding it into a whole-`Ty` check would silently reorder those
/// diagnostics -- `(1, "a", undefined_name)` would report the undefined
/// name instead of `T0039`.
pub fn check_tuple_element_ty(element: &Ty, span: Span) -> Result<(), Diagnostic> {
    if matches!(element, Ty::Int | Ty::Bool | Ty::Float) {
        return Ok(());
    }
    Err(Diagnostic::error(
        "T0039",
        format!(
            "tuple element type `{}` is not compiled yet (D-116) -- only int/bool/float elements are",
            element.name()
        ),
        span,
    ))
}

/// The kind of display [`display_is_object`] judges.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayKind {
    List,
    Tuple,
    Set,
    /// Element types alternate key and value.
    Dict,
}

/// Whether a display with element types `elements` is built as the
/// CPython container (non-strict mode, #3) rather than a native one: some
/// element is an object, or the elements have no native representation --
/// they differ (CPython containers are heterogeneous) or the native shape
/// is not compiled ([`check_container_ty`]). `pycc_types` (which admits it
/// only in non-strict mode) and `pycc_mir` (which builds it) share this one
/// rule, so the checked type and the built node always agree.
pub fn display_is_object(kind: DisplayKind, elements: &[Ty]) -> bool {
    // An empty display reaches this rule only when the empty-container
    // pre-pass found no native element type for it (`pycc_types` refuses it
    // with `T0003` in strict mode).
    if elements.is_empty() {
        return true;
    }
    if elements.contains(&Ty::Object) {
        return true;
    }
    fn uniform<'a>(mut tys: impl Iterator<Item = &'a Ty>) -> bool {
        let first = tys.next();
        tys.all(|ty| Some(ty) == first)
    }
    let span = Span::new(0, 0);
    match kind {
        // A uniform list keeps its native lowering even where that shape is
        // not compiled yet: the checker reports the `T0034` then.
        DisplayKind::List => !uniform(elements.iter()),
        DisplayKind::Set => {
            !uniform(elements.iter())
                || check_container_ty(&Ty::Set(Box::new(elements[0].clone())), span).is_err()
        }
        DisplayKind::Dict => {
            !uniform(elements.iter().step_by(2))
                || !uniform(elements.iter().skip(1).step_by(2))
                || check_container_ty(
                    &Ty::Dict(Box::new((elements[0].clone(), elements[1].clone()))),
                    span,
                )
                .is_err()
        }
        DisplayKind::Tuple => elements
            .iter()
            .any(|ty| check_tuple_element_ty(ty, span).is_err()),
    }
}

/// Rejects a container `Ty` whose element types this version's codegen
/// cannot represent: a `list[T]` whose element fails [`is_list_element_ty`] (`T0034`), `dict[str, int]`
/// (`T0036`, D-122), `set[int]` and `frozenset[int]` (`T0038`, D-122) are
/// the only admitted shapes, and a `tuple`'s elements are each checked with
/// [`check_tuple_element_ty`].
///
/// A non-container `Ty` is accepted unchanged, so an annotation-lowering
/// caller can route every lowered type through one call without first
/// matching on the shape itself.
pub fn check_container_ty(ty: &Ty, span: Span) -> Result<(), Diagnostic> {
    match ty {
        Ty::List(element) => {
            if !is_list_element_ty(element) {
                return Err(Diagnostic::error(
                    "T0034",
                    format!(
                        "{} is not compiled yet -- a list element must be int, float, bool, \
                         str, a user-class instance or a list of those",
                        ty.name()
                    ),
                    span,
                ));
            }
            Ok(())
        }
        Ty::Dict(pair) => {
            if **pair != (Ty::Str, Ty::Int) {
                return Err(Diagnostic::error(
                    "T0036",
                    format!(
                        "{} is not compiled yet (D-122) -- only dict[str, int] is",
                        ty.name()
                    ),
                    span,
                ));
            }
            Ok(())
        }
        Ty::Set(element) => check_int_set_element(ty, element, "set", span),
        // Part 1 of #1319: `frozenset[int]` is `set[int]`'s exact sibling,
        // gated by the same code for the same reason (D-122).
        Ty::FrozenSet(element) => check_int_set_element(ty, element, "frozenset", span),
        Ty::Tuple(elements) => {
            for element in elements.iter() {
                check_tuple_element_ty(element, span)?;
            }
            Ok(())
        }
        Ty::Int
        | Ty::Float
        | Ty::Bool
        | Ty::Str
        | Ty::None
        | Ty::Infer
        | Ty::Param(_)
        | Ty::Instance(_)
        | Ty::Protocol(_)
        | Ty::Optional(_)
        | Ty::Object
        | Ty::MemoryView => Ok(()),
    }
}

/// Whether `list[element]` is compiled: one generic `list[T]` runtime object
/// stores any element that fits one native word -- `int`, `float`, `bool`,
/// `str`, a user-class instance, or (recursively) another such list.
#[must_use]
pub fn is_list_element_ty(element: &Ty) -> bool {
    match element {
        Ty::Int | Ty::Float | Ty::Bool | Ty::Str | Ty::Instance(_) => true,
        Ty::List(inner) => is_list_element_ty(inner),
        _ => false,
    }
}

/// The `T0038` gate shared by `set[T]` and `frozenset[T]`: an `int` element
/// and (#1343) a user-class instance element are compiled. Whether that
/// class is actually hashable is checked where an element is inserted
/// (`pycc_types::set_element`), which has the class table this gate lacks.
/// `family` names the admitted forms in the message.
fn check_int_set_element(
    ty: &Ty,
    element: &Ty,
    family: &str,
    span: Span,
) -> Result<(), Diagnostic> {
    if !matches!(element, Ty::Int | Ty::Instance(_)) {
        return Err(Diagnostic::error(
            "T0038",
            set_element_message(ty, family),
            span,
        ));
    }
    Ok(())
}

/// The `T0038` message for a `set[T]`/`frozenset[T]` whose element is not
/// compiled, naming both admitted element forms.
#[must_use]
pub fn set_element_message(ty: &Ty, family: &str) -> String {
    format!(
        "{} is not compiled yet (D-122) -- only {family}[int] and a {family} of a user-class \
         instance are",
        ty.name()
    )
}

#[cfg(test)]
mod tests;
