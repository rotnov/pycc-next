//! Type predicates the empty-container pass applies to a resolution.

use super::*;

/// Whether `ty` is `Ty::Infer` or contains one anywhere inside it.
///
/// The recursion matters: a producer can yield a nested container whose
/// *element* is the placeholder (`xs.append(ys)` where `ys` is an
/// unannotated helper parameter's list), and a `list[list[<inferred>]]`
/// stored in the node is the same wrong resolution a bare
/// `list[<inferred>]` is.
pub(crate) fn contains_infer(ty: &Ty) -> bool {
    match ty {
        Ty::Infer => true,
        Ty::List(element)
        | Ty::Set(element)
        | Ty::FrozenSet(element)
        | Ty::VarTuple(element)
        | Ty::Optional(element) => contains_infer(element),
        Ty::Generator(sig) => {
            contains_infer(&sig.0) || contains_infer(&sig.1) || contains_infer(&sig.2)
        }
        Ty::Dict(pair) => contains_infer(&pair.0) || contains_infer(&pair.1),
        Ty::Tuple(elements) => elements.iter().any(contains_infer),
        Ty::Callable(sig) => sig.0.iter().any(contains_infer) || contains_infer(&sig.1),
        Ty::Int
        | Ty::Float
        | Ty::Bool
        | Ty::Str
        | Ty::None
        | Ty::Param(_)
        | Ty::Instance(_)
        | Ty::Protocol(_)
        | Ty::Object
        | Ty::MemoryView
        | Ty::NdArray(_) => false,
    }
}

/// Whether `ty` is `Ty::Optional` or contains one anywhere inside it.
///
/// The node itself answers `true` without recursing: it is the `Optional`
/// *wrapper* the check phase may have narrowed away, not anything inside it.
/// The recursion still matters for the nested shapes -- a producer can yield
/// `list[int | None]`, and freezing that into an element position is the same
/// wrong resolution a bare `int | None` is.
pub(super) fn contains_optional(ty: &Ty) -> bool {
    match ty {
        Ty::Optional(_) => true,
        Ty::List(element) | Ty::Set(element) | Ty::FrozenSet(element) | Ty::VarTuple(element) => {
            contains_optional(element)
        }
        Ty::Generator(sig) => contains_optional(&sig.0),
        Ty::Dict(pair) => contains_optional(&pair.0) || contains_optional(&pair.1),
        Ty::Tuple(elements) => elements.iter().any(contains_optional),
        Ty::Callable(_) => false,
        Ty::Int
        | Ty::Float
        | Ty::Bool
        | Ty::Str
        | Ty::None
        | Ty::Infer
        | Ty::Param(_)
        | Ty::Instance(_)
        | Ty::Protocol(_)
        | Ty::Object
        | Ty::MemoryView
        | Ty::NdArray(_) => false,
    }
}
