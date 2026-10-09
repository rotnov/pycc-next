//! Parameterized container annotation lowering (D-228, issue #918):
//! `list[T]`, `set[T]`, `frozenset[T]`, `dict[K, V]`, `tuple[A, B, ...]`, and
//! since #1378 their pre-PEP 585 `typing` spellings `List[T]`, `Set[T]`,
//! `FrozenSet[T]`, `Dict[K, V]`, `Tuple[A, B, ...]`.
//!
//! Extracted from `func.rs` per AGENTS.md's file-decomposition rule when
//! #1378 taught [`container_annotation_to_ty`] the legacy spellings. Not
//! named `container.rs`: [`crate::container::check_container_ty`], the
//! element-type capability gate this function ends with, already owns that
//! name.

use super::annotation_to_ty;
use super::bare_container::{bare_container_example, container_family};
use crate::Ty;
use crate::class::ClassAnnotationInfo;
use pycc_ast::Expr;
use pycc_diag::{Diagnostic, Span};

/// Lowers a parameterized builtin container annotation -- `list[T]`,
/// `set[T]`, `frozenset[T]`, `dict[K, V]` or `tuple[A, B, ...]` -- to its
/// `Ty` (D-228, issue #918).
///
/// `spelling` is the base as written: a builtin container name or, since
/// #1378, one of the legacy `typing` aliases (`Dict`, `List`, ...).
/// [`container_family`] maps it to the builtin family, which alone decides
/// the arity and the `Ty` constructor, so `Dict[str, int]` lowers to exactly
/// the `Ty` `dict[str, int]` does. Every message and help string of checks 1
/// and 2 below renders `spelling`, so `Dict[str]` is reported as
/// `Dict[...]`, the form the user wrote. The capability gate at the end
/// ([`crate::container::check_container_ty`]) prints the canonical `Ty`
/// instead (`dict[int, int]`), because it runs on the lowered type and is
/// shared with container literals.
///
/// Three checks run in a fixed order, so the reported diagnostic always
/// describes the outermost thing that is wrong:
///
/// 1. an `...` type argument (the homogeneous-variadic `tuple[int, ...]`),
///    rejected with `T0053` because a runtime-length tuple has no fixed-arity
///    `Ty::Tuple` representation;
/// 2. arity -- `list`/`set`/`frozenset` take exactly one argument, `dict`
///    exactly two, `tuple` at least one (so the empty `tuple[()]`, which reaches here as a
///    zero-element `Expr::Tuple`, is rejected here rather than silently
///    lowering to a zero-field tuple);
/// 3. each argument's own type, recursively through [`annotation_to_ty`],
///    then the shared element-type capability gate
///    ([`crate::container::check_container_ty`]) that container *literals*
///    also run.
///
/// Between 3's recursion and the capability gate, a `Ty::Param` element is
/// rejected with `T0042` -- the same code and wording `pycc_types`' own
/// signature scan uses, but carrying the annotation's real span instead of
/// that scan's `Span::new(0, 0)`. Catching it here rather than relying on the
/// downstream scan is not just a nicer caret: `substitute_ty` is not
/// recursive, so a `Ty::Param` buried inside a container would not be
/// substituted at a call site even where the scan did let it through.
pub(super) fn container_annotation_to_ty(
    spelling: &str,
    slice: &Expr,
    annotation: &Expr,
    type_param: Option<&str>,
    class_name: Option<&str>,
    aliases: &[(String, Ty)],
    class_defs: &[ClassAnnotationInfo],
) -> Result<Ty, Diagnostic> {
    let family = container_family(spelling).expect("the caller gates on `container_family`");
    let example =
        bare_container_example(spelling).expect("every container spelling has an example");
    let span = {
        let range = pycc_ast::expr_range(annotation);
        Span::new(range.start, range.end)
    };
    // A single type argument arrives as the bare expression; two or more (and
    // the empty `tuple[()]`) arrive as an `Expr::Tuple`.
    let args: Vec<&Expr> = match slice {
        Expr::Tuple(tuple) => tuple.elts.iter().collect(),
        other => vec![other],
    };
    if args
        .iter()
        .any(|arg| matches!(arg, Expr::EllipsisLiteral(_)))
    {
        // The advice is per family. `tuple[X, ...]` is the one spelling that
        // means something in Python -- a homogeneous variadic tuple -- so it
        // gets the length explanation and a fixed-arity `tuple`. For
        // `list`/`set`/`frozenset`/`dict`, `...` is simply not a type, and
        // recommending a `tuple` there would change the container the user
        // asked for.
        //
        // The advice is split into the reason and the imperative fix, because
        // the fix is also published as structured `help` (D-152's "the message
        // already embeds the fix" family): the message keeps the whole
        // sentence, while `help` carries the imperative alone so a JSON or IDE
        // consumer reads an instruction rather than a restatement.
        let (advice, help) = if family == "tuple" {
            let help = format!("write an explicit fixed-arity annotation such as `{example}`");
            (
                format!(
                    "a homogeneous-variadic container has no compile-time length, so {help} instead"
                ),
                help,
            )
        } else {
            let help = format!("write the element type, e.g. `{example}`");
            (format!("`...` is not a type argument here; {help}"), help)
        };
        return Err(Diagnostic::error(
            "T0053",
            format!(
                "the `...` type argument in `{spelling}[...]` is not supported yet -- {advice}"
            ),
            span,
        )
        .with_help(help));
    }
    let exact_arity = match family {
        "list" | "set" | "frozenset" => Some(1usize),
        "dict" => Some(2usize),
        // `tuple` is variadic in arity: any count of one or more.
        _ => None,
    };
    match exact_arity {
        Some(expected) if args.len() != expected => {
            return Err(Diagnostic::error(
                "T0053",
                format!(
                    "container type annotation `{spelling}[...]` takes exactly {expected} type argument{}, got {}",
                    if expected == 1 { "" } else { "s" },
                    args.len()
                ),
                span,
            )
            .with_help(format!(
                "write exactly {expected} type argument{}, e.g. `{example}`",
                if expected == 1 { "" } else { "s" }
            )));
        }
        None if args.is_empty() => {
            return Err(Diagnostic::error(
                "T0053",
                format!(
                    "container type annotation `{spelling}[...]` takes at least 1 type argument -- the empty tuple `{spelling}[()]` is not supported yet"
                ),
                span,
            )
            .with_help(format!("write at least one element type, e.g. `{spelling}[int]`")));
        }
        _ => {}
    }
    let mut elements = Vec::with_capacity(args.len());
    for arg in &args {
        let element = annotation_to_ty(arg, type_param, class_name, aliases, class_defs)?;
        if let Ty::Param(name) = &element {
            return Err(Diagnostic::error(
                "T0042",
                format!(
                    "type parameter `{name}` used inside a container position is not supported yet -- v0.2 only instantiates a bare type-parameter position, matching D-105's own fixed-container-element-type restriction"
                ),
                span,
            ));
        }
        elements.push(element);
    }
    // D-258 rule 4 (#1397): in an `ext` module one object-typed argument
    // anywhere makes the whole value the opaque CPython container --
    // `List[StateT]` over a foreign type variable, `Dict[str, tuple]` over a
    // bare `tuple`, `tuple[int, Any]` -- never a native container of objects.
    // Checked before the D-105/D-122 capability gate, which governs native
    // containers only; a container whose arguments are all native is
    // unaffected and still reaches that gate.
    if super::is_ext_module(aliases) && elements.contains(&Ty::Object) {
        return Ok(Ty::Object);
    }
    let mut elements = elements.into_iter();
    let ty = match family {
        "list" => Ty::List(Box::new(elements.next().expect("arity checked above"))),
        "set" => Ty::Set(Box::new(elements.next().expect("arity checked above"))),
        "frozenset" => Ty::FrozenSet(Box::new(elements.next().expect("arity checked above"))),
        "dict" => {
            let key = elements.next().expect("arity checked above");
            let value = elements.next().expect("arity checked above");
            Ty::Dict(Box::new((key, value)))
        }
        _ => Ty::Tuple(Box::new(elements.collect())),
    };
    crate::container::check_container_ty(&ty, span)?;
    Ok(ty)
}
