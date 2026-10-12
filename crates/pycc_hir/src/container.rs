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
/// represent (`T0039`): every element a list can hold is admitted
/// ([`is_list_element_ty`]). A tuple of only `int`/`bool`/`float` elements is
/// an SSA struct; any other tuple is a boxed runtime object
/// ([`tuple_is_boxed`]).
///
/// This is deliberately *element*-shaped rather than a whole-`Ty` postcheck:
/// `pycc_types`' tuple-literal inference gates the elements in source order.
pub fn check_tuple_element_ty(element: &Ty, span: Span) -> Result<(), Diagnostic> {
    if is_list_element_ty(element) {
        return Ok(());
    }
    Err(Diagnostic::error(
        "T0039",
        format!(
            "tuple element type `{}` is not compiled yet -- a tuple element must be int, float, \
             bool, str, a user-class instance, or a list, tuple, dict or set of those",
            element.name()
        ),
        span,
    ))
}

/// Whether a `tuple[elements]` value is a boxed runtime object (a
/// `pycc_rt` tuple object, held by pointer and reference counted like a
/// list) rather than an SSA struct: any element other than `int`, `bool`
/// and `float` makes it boxed.
#[must_use]
pub fn tuple_is_boxed(elements: &[Ty]) -> bool {
    !elements
        .iter()
        .all(|element| matches!(element, Ty::Int | Ty::Bool | Ty::Float))
}

/// Whether `ty` can be a dict key or a set element with a natively computed
/// hash: `int`, `bool`, `float`, `str`, and tuples of those.
#[must_use]
pub fn is_hashable_ty(ty: &Ty) -> bool {
    match ty {
        Ty::Int | Ty::Bool | Ty::Float | Ty::Str => true,
        Ty::Tuple(elements) => !elements.is_empty() && elements.iter().all(is_hashable_ty),
        _ => false,
    }
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
/// cannot represent: a `list[T]` whose element fails [`is_list_element_ty`]
/// (`T0034`), a `dict[K, V]` whose key is not [`is_hashable_ty`] or whose
/// value is not a list element type (`T0036`), a `set[T]`/`frozenset[T]` of
/// anything but a hashable type or a user-class instance (`T0038`), and a
/// `tuple` with an element [`check_tuple_element_ty`] rejects.
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
                         str, a user-class instance, or a list, tuple, dict or set of those",
                        ty.name()
                    ),
                    span,
                ));
            }
            Ok(())
        }
        Ty::Dict(pair) => {
            if !is_hashable_ty(&pair.0) || !is_list_element_ty(&pair.1) {
                return Err(Diagnostic::error(
                    "T0036",
                    format!(
                        "{} is not compiled yet -- a dict key must be int, float, bool, str or \
                         a tuple of those, and a value anything a list can hold",
                        ty.name()
                    ),
                    span,
                ));
            }
            Ok(())
        }
        // A generator yields what a list can hold (its value area holds one
        // element in the same representation).
        Ty::Generator(element) => {
            if !is_list_element_ty(element) {
                return Err(Diagnostic::error(
                    "T0034",
                    format!(
                        "{} is not compiled yet -- a generator must yield int, float, bool, \
                         str, a user-class instance, or a list, tuple, dict or set of those",
                        ty.name()
                    ),
                    span,
                ));
            }
            Ok(())
        }
        Ty::Set(element) => check_set_element(ty, element, "set", span),
        Ty::FrozenSet(element) => check_set_element(ty, element, "frozenset", span),
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
        | Ty::MemoryView
        | Ty::NdArray(_)
        | Ty::Callable(_) => Ok(()),
    }
}

/// Whether `element` can be stored as one native container word: `int`,
/// `float`, `bool`, `str`, a user-class instance, or (recursively) a list,
/// a non-empty tuple, a dict or a set whose own element types are
/// compiled.
#[must_use]
pub fn is_list_element_ty(element: &Ty) -> bool {
    match element {
        Ty::Int | Ty::Float | Ty::Bool | Ty::Str | Ty::Instance(_) => true,
        Ty::List(inner) => is_list_element_ty(inner),
        Ty::Tuple(elements) => !elements.is_empty() && elements.iter().all(is_list_element_ty),
        Ty::Dict(pair) => is_hashable_ty(&pair.0) && is_list_element_ty(&pair.1),
        Ty::Set(inner) | Ty::FrozenSet(inner) => is_hashable_ty(inner),
        _ => false,
    }
}

/// The `T0038` gate shared by `set[T]` and `frozenset[T]`: a hashable
/// element ([`is_hashable_ty`]) and (#1343) a user-class instance element
/// are compiled. Whether that class is actually hashable is checked where an
/// element is inserted (`pycc_types::set_element`), which has the class
/// table this gate lacks.
fn check_set_element(ty: &Ty, element: &Ty, family: &str, span: Span) -> Result<(), Diagnostic> {
    if !is_hashable_ty(element) && !matches!(element, Ty::Instance(_)) {
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
        "{} is not compiled yet -- a {family} element must be int, float, bool, str, a tuple of \
         those or a user-class instance",
        ty.name()
    )
}

#[cfg(test)]
mod tests;
