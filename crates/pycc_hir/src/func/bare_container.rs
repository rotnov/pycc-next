//! Bare-container annotation advice (D-228, issue #918): upgrading
//! [`crate::annotation_to_ty`]'s generic unknown-name `C0001` for a bare
//! `list`/`set`/`frozenset`/`dict`/`tuple` (or a legacy `typing` alias such
//! as `List`, #1378) into a message naming the parameterized form
//! to write, in the positions that lower one. Extracted from `func.rs` per
//! AGENTS.md's file-decomposition rule when #1264 added
//! [`with_bare_list_or_dict_advice`]. The moved items keep their code
//! verbatim; only their visibility and doc comments changed.

use crate::unsupported;
use pycc_ast::Expr;
use pycc_diag::Diagnostic;

/// The five builtin container types this version lowers from a parameterized
/// annotation (D-228, issue #918; `frozenset` since Part 1 of #1319).
/// `type[T]` is absent on purpose: it has no `Ty` variant and no run-time
/// representation. (D-109's 16-byte `size_of::<Ty>()` ceiling is not the
/// obstacle: a thin-pointer variant such as `Ty::FrozenSet(Box<Ty>)`
/// measurably keeps `Ty` at 16 bytes.)
const CONTAINER_ANNOTATION_NAMES: [&str; 5] = ["list", "set", "frozenset", "dict", "tuple"];

/// The pre-PEP 585 `typing` aliases of the builtin containers (#1378), each
/// paired with the builtin family it lowers as. `typing.Type` is absent for
/// the same reason `type` is absent above.
const LEGACY_TYPING_CONTAINER_ALIASES: [(&str, &str); 5] = [
    ("List", "list"),
    ("Set", "set"),
    ("FrozenSet", "frozenset"),
    ("Dict", "dict"),
    ("Tuple", "tuple"),
];

/// The builtin container family a parameterized annotation base spells --
/// the name itself for a builtin container, the aliased builtin for a legacy
/// `typing` alias (`Dict` -> `dict`) -- or `None` for any other name.
pub(super) fn container_family(spelling: &str) -> Option<&'static str> {
    CONTAINER_ANNOTATION_NAMES
        .into_iter()
        .find(|name| *name == spelling)
        .or_else(|| {
            LEGACY_TYPING_CONTAINER_ALIASES
                .into_iter()
                .find(|(alias, _)| *alias == spelling)
                .map(|(_, family)| family)
        })
}

/// A worked parameterized example for a bare container annotation's `C0001`,
/// or `None` for a name that is not one of the five builtin containers or
/// their five legacy `typing` aliases. Each example keeps the spelling it was
/// asked about (`List[int]` for `List`, #1378). `tuple` gets its own
/// two-argument example: `tuple[int]` is legal but atypical, and a
/// single-element example would read as if `tuple` were homogeneous.
pub(super) fn bare_container_example(name: &str) -> Option<&'static str> {
    match name {
        "list" => Some("list[int]"),
        "set" => Some("set[int]"),
        "frozenset" => Some("frozenset[int]"),
        "dict" => Some("dict[str, int]"),
        "tuple" => Some("tuple[int, int]"),
        "List" => Some("List[int]"),
        "Set" => Some("Set[int]"),
        "FrozenSet" => Some("FrozenSet[int]"),
        "Dict" => Some("Dict[str, int]"),
        "Tuple" => Some("Tuple[int, int]"),
        _ => None,
    }
}

/// Upgrades [`crate::annotation_to_ty`]'s generic unknown-name `C0001` into the
/// bare-container message that names the parameterized form (D-228, issue
/// #918) -- for the callers whose annotation position actually lowers a
/// container.
///
/// Only these do: a function or method parameter, a function or method
/// return annotation (#925), a local or module-level `AnnAssign`, and a type
/// alias. An annotated attribute target (#1264) and a class-body instance
/// attribute declaration (#1266) lower only `list[int]` and `dict[str, int]`,
/// so they opt in through [`with_bare_list_or_dict_advice`]. Class-constant
/// (an annotation with a value), dataclass-field and protocol-attribute
/// positions each reject `list[int]` with a `C0001` of their own, so advising the
/// parameterized form there would walk the user straight into a second
/// error. They opt out simply by not calling this, which is why the advice
/// is an opt-in upgrade rather than a position argument threaded through
/// `annotation_to_ty`: a position added later is correct without touching
/// this file -- return position joined the advising set that way, by adding
/// one `map_err` in `lower_return_annotation`.
///
/// Discarding `error` in the upgrade arm is sound because a bare
/// `Expr::Name` has exactly one failure mode in `annotation_to_ty` -- the
/// alias-table miss that builds `unknown_annotation_name_message` -- so the
/// message being replaced is always that one.
///
/// The name is reached through [`strip_transparent_wrappers`], because
/// `annotation_to_ty` propagates that same failure out of `Final[list]` and
/// `Annotated[list, "meta"]` unchanged while accepting `Final[list[int]]`:
/// matching only the outermost expression would drop the advice in exactly
/// the positions that can act on it.
pub(crate) fn with_bare_container_advice(error: Diagnostic, annotation: &Expr) -> Diagnostic {
    // Part 1 of #889: a quoted wrapped name (`Final["list"]`) gets the
    // advice its unquoted spelling gets; its nodes carry the literal's span.
    let unquoted = pycc_ast::unquote_nested_string_annotations(annotation);
    let stripped = strip_transparent_wrappers(&unquoted);
    let Expr::Name(name) = stripped else {
        return error;
    };
    match bare_container_example(name.id.as_str()) {
        // The span is the bare name, not the wrapper: that is the token the
        // user replaces, and it is where `annotation_to_ty` already pointed.
        Some(example) => unsupported(
            crate::module::bare_container_annotation_message(name.id.as_str(), example),
            pycc_ast::expr_range(stripped),
        ),
        None => error,
    }
}

/// [`with_bare_container_advice`] restricted to a bare `list`/`dict` (or
/// their `typing` spellings `List`/`Dict`, #1378), for
/// a position that lowers only those two parameterized forms (an annotated
/// attribute target, #1264, or a class-body instance attribute declaration,
/// #1266): a bare `set`/`tuple` there keeps `error`, so the
/// advice never names a form the position refuses too.
pub(crate) fn with_bare_list_or_dict_advice(error: Diagnostic, annotation: &Expr) -> Diagnostic {
    let unquoted = pycc_ast::unquote_nested_string_annotations(annotation);
    match strip_transparent_wrappers(&unquoted) {
        Expr::Name(name) if matches!(name.id.as_str(), "list" | "dict" | "List" | "Dict") => {
            with_bare_container_advice(error, annotation)
        }
        _ => error,
    }
}

/// Peels the wrappers `annotation_to_ty` lowers by recursing into their
/// inner type, so a diagnostic about that inner type can be recognized from
/// the outside.
///
/// Only `Final[X]` (PEP 591) and `Annotated[X, ...]` (PEP 593) qualify: both
/// lower to `X` itself. The shapes accepted here mirror `annotation_to_ty`'s
/// own arms exactly -- `Final` takes one argument, `Annotated` takes a tuple
/// of at least two -- so a malformed wrapper keeps its own diagnostic rather
/// than being reported against whatever it wraps.
fn strip_transparent_wrappers(annotation: &Expr) -> &Expr {
    let Expr::Subscript(sub) = annotation else {
        return annotation;
    };
    let Expr::Name(base) = sub.value.as_ref() else {
        return annotation;
    };
    let inner = match base.id.as_str() {
        "Final" => match sub.slice.as_ref() {
            Expr::Tuple(tuple) if tuple.elts.len() != 1 => return annotation,
            Expr::Tuple(tuple) => &tuple.elts[0],
            other => other,
        },
        "Annotated" => match sub.slice.as_ref() {
            Expr::Tuple(tuple) if tuple.elts.len() >= 2 => &tuple.elts[0],
            _ => return annotation,
        },
        _ => return annotation,
    };
    strip_transparent_wrappers(inner)
}
