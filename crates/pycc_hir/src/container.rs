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
    // A CPython object element (an `ext` or non-strict `tuple[int, Any]`)
    // is admitted by the annotation that spells it
    // (`func::container_annotation`), not here: plain strict code builds a
    // tuple display holding an object as the CPython tuple.
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
    // The empty tuple is boxed: it is then a `tuple[T, ...]` of any `T`
    // with no conversion (`Ty::VarTuple`).
    elements.is_empty()
        || !elements
            .iter()
            .all(|element| matches!(element, Ty::Int | Ty::Bool | Ty::Float))
}

/// Whether `tuple[element, ...]` is compiled natively (`Ty::VarTuple`): an
/// element a list can hold that is itself counted -- a `str`, an instance,
/// a container or a function value -- so every fixed-arity tuple of it is a
/// boxed tuple object already and flows into a `tuple[T, ...]` slot with
/// no conversion. A tuple of `int`/`float`/`bool` is an SSA struct, so
/// `tuple[int, ...]` is not compiled yet (the CPython `tuple` in an `ext`
/// or non-strict module, `T0053` otherwise).
#[must_use]
pub fn is_var_tuple_element_ty(element: &Ty) -> bool {
    is_list_element_ty(element) && !matches!(element, Ty::Int | Ty::Float | Ty::Bool)
}

/// Whether a `tuple[element, ...]` value is compiled: one of an
/// [`is_var_tuple_element_ty`] element, or the tuple object `tuple(xs)`
/// builds of a list of `int`s or `float`s (`builtins::conversions`). No
/// fixed-arity tuple of those flows into one (it is an SSA struct), so
/// such a value meets only its own kind, and the annotation stays refused.
#[must_use]
pub fn is_held_var_tuple_element_ty(element: &Ty) -> bool {
    is_var_tuple_element_ty(element) || matches!(element, Ty::Int | Ty::Float)
}

/// Whether a fixed-arity tuple of type `tuple[elements]` is, as it is
/// represented, also a `tuple[element, ...]` (`Ty::VarTuple`): a boxed
/// tuple object whose elements all have type `element`, or `()`.
#[must_use]
pub fn tuple_fits_var_tuple(elements: &[Ty], element: &Ty) -> bool {
    tuple_is_boxed(elements) && elements.iter().all(|e| e == element)
}

/// The element type of a `list[T]` or a `tuple[T, ...]`: the two share a
/// run-time object (a `pycc_rt` list; the tuple is marked frozen), so
/// every read -- `len`, indexing, slicing, iteration, membership -- is the
/// same operation on either.
#[must_use]
pub fn sequence_element(ty: &Ty) -> Option<&Ty> {
    match ty {
        Ty::List(element) | Ty::VarTuple(element) => Some(element),
        _ => None,
    }
}

/// Whether `ty` can be a dict key or a set element with a natively computed
/// hash: `int`, `bool`, `float`, `str`, and tuples of those.
#[must_use]
pub fn is_hashable_ty(ty: &Ty) -> bool {
    match ty {
        Ty::Int | Ty::Bool | Ty::Float | Ty::Num | Ty::Str => true,
        Ty::Tuple(elements) => !elements.is_empty() && elements.iter().all(is_hashable_ty),
        Ty::VarTuple(element) => is_hashable_ty(element),
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
    if kind == DisplayKind::List && is_shaped_list_display(elements) {
        return false;
    }
    // An empty display reaches this rule only when the empty-container
    // pre-pass found no native element type for it (`pycc_types` refuses it
    // with `T0003` in strict mode).
    if elements.is_empty() {
        // `()` is the native empty tuple object.
        return kind != DisplayKind::Tuple;
    }
    if elements.contains(&Ty::Object) {
        return true;
    }
    // Numbers of both kinds (`[1, 2.5]`) are one native `int | float`
    // element type (`crate::num_join`).
    fn uniform<'a>(mut tys: impl Iterator<Item = &'a Ty>) -> bool {
        let Some(first) = tys.next() else {
            return true;
        };
        let mut joined = first.clone();
        tys.all(|ty| {
            if *ty == joined {
                return true;
            }
            match crate::num_join(&joined, ty) {
                Some(num) => {
                    joined = num;
                    true
                }
                None => false,
            }
        })
    }
    let span = Span::new(0, 0);
    match kind {
        // A uniform list of an element type no native list holds (`[None]`)
        // is the CPython list too; in strict mode the checker reports the
        // `T0034` first.
        DisplayKind::List => {
            !uniform(elements.iter())
                || check_container_ty(&Ty::List(Box::new(elements[0].clone())), span).is_err()
        }
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
            if !is_list_element_ty(element) && !is_optional_element_ty(element) {
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
            // An instance key is admitted here; `pycc_types::dict_key` refuses one
            // whose class compares by `__eq__` (it has the class table).
            let key_ok = is_hashable_ty(&pair.0) || matches!(pair.0, Ty::Instance(_));
            if !key_ok || (!is_list_element_ty(&pair.1) && !is_optional_element_ty(&pair.1)) {
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
        // element in the same representation), or a CPython object (v0.7
        // accept 2: an object reference, released as any other).
        Ty::Generator(sig) => {
            let (element, sent, returned) = &**sig;
            if !is_list_element_ty(element) && *element != Ty::Object {
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
            for (what, inner) in [("send", sent), ("return", returned)] {
                if !matches!(inner, Ty::None | Ty::Infer) && !is_list_element_ty(inner) {
                    return Err(Diagnostic::error(
                        "T0034",
                        format!(
                            "{} is not compiled yet -- a generator's {what} type must be \
                             None or what a list can hold",
                            ty.name()
                        ),
                        span,
                    ));
                }
            }
            Ok(())
        }
        Ty::Set(element) => check_set_element(ty, element, "set", span),
        Ty::FrozenSet(element) => check_set_element(ty, element, "frozenset", span),
        Ty::Tuple(elements) | Ty::ShapedList(elements) => {
            for element in elements.iter() {
                check_tuple_element_ty(element, span)?;
            }
            Ok(())
        }
        Ty::VarTuple(element) if is_held_var_tuple_element_ty(element) => Ok(()),
        Ty::VarTuple(element) => check_tuple_element_ty(element, span).and(Err(Diagnostic::error(
            "T0053",
            format!(
                "`{}` is not compiled yet -- a variadic tuple's element must be str, a \
                 user-class instance, or a list, tuple, dict or set",
                ty.name()
            ),
            span,
        ))),
        Ty::Int
        | Ty::Float
        | Ty::Num
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

/// Whether a list element or dict value may be `element`, a `T | None` of
/// an `int` or a pointer-represented `T` a list holds: its word is the
/// `T`'s, or `0` for `None` (`pycc_rt::optional_kind`).
#[must_use]
pub fn is_optional_element_ty(element: &Ty) -> bool {
    matches!(element, Ty::Optional(inner)
        if is_list_element_ty(inner) && (**inner == Ty::Int || crate::is_pointer_optional_inner(inner)))
}

/// Whether a list display of elements of types `elements` is a list of a
/// fixed shape ([`Ty::ShapedList`]): in strict mode, two or more elements
/// a list can hold whose types do not join into one element type (equal,
/// or `int | float`), forming a boxed tuple's shape. `pycc_types` types
/// such a display and `pycc_mir` builds it by this one rule.
#[must_use]
pub fn is_shaped_list_display(elements: &[Ty]) -> bool {
    if crate::is_non_strict_program() || elements.len() < 2 {
        return false;
    }
    let mut joined = elements[0].clone();
    let uniform = elements[1..].iter().all(|ty| {
        if *ty == joined {
            return true;
        }
        match crate::num_join(&joined, ty) {
            Some(num) => {
                joined = num;
                true
            }
            None => false,
        }
    });
    !uniform && elements.iter().all(is_list_element_ty) && tuple_is_boxed(elements)
}

/// Whether `element` can be stored as one native container word: `int`,
/// `float`, `bool`, `str`, a user-class instance, or (recursively) a list,
/// a non-empty tuple, a dict or a set whose own element types are
/// compiled.
#[must_use]
pub fn is_list_element_ty(element: &Ty) -> bool {
    match element {
        Ty::Int
        | Ty::Float
        | Ty::Num
        | Ty::Bool
        | Ty::Str
        | Ty::Instance(_)
        | Ty::Callable(_) => true,
        Ty::List(inner) => is_list_element_ty(inner),
        Ty::Tuple(elements) | Ty::ShapedList(elements) => {
            !elements.is_empty() && elements.iter().all(is_list_element_ty)
        }
        Ty::VarTuple(element) => is_held_var_tuple_element_ty(element),
        Ty::Dict(pair) => is_hashable_ty(&pair.0) && is_list_element_ty(&pair.1),
        Ty::Set(inner) | Ty::FrozenSet(inner) => is_hashable_ty(inner),
        _ => false,
    }
}

/// The element type of a tuple whose elements all share one type a list
/// can hold (`(1, 2, 3)`, `("a", "b")`): such a tuple is iterable, as the
/// list of its elements. `None` for an empty or mixed tuple.
#[must_use]
pub fn homogeneous_tuple_element(items: &[Ty]) -> Option<&Ty> {
    let first = items.first()?;
    (is_list_element_ty(first) && items.iter().all(|item| item == first)).then_some(first)
}

/// Non-strict mode: the CPython container an empty `set()` or `dict()`
/// call builds when the empty-container pass left its element types
/// unresolved, by callee and argument count.
#[must_use]
pub fn empty_object_constructor(callee: &str, arity: usize) -> Option<DisplayKind> {
    match (callee, arity) {
        ("set", 0) => Some(DisplayKind::Set),
        ("dict", 0) => Some(DisplayKind::Dict),
        _ => None,
    }
}

/// Non-strict mode: whether a list comprehension over a CPython object
/// whose element has type `elt` collects into a native `list[elt]` rather
/// than a CPython `list`. A native container or tuple element has no
/// CPython form (packing one would copy it and break aliasing), so the
/// result stays native; every other element is packed into a CPython list.
#[must_use]
pub fn object_comp_collects_natively(elt: &Ty) -> bool {
    matches!(
        elt,
        Ty::List(_) | Ty::Set(_) | Ty::FrozenSet(_) | Ty::Dict(_) | Ty::Tuple(_) | Ty::VarTuple(_)
    ) && is_list_element_ty(elt)
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
