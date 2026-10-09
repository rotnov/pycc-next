//! Class-body instance attribute declarations (#1266, Part 5 of #1218).
//!
//! A value-less class-body annotation (`n: int`, `items: list[int]`) that is
//! neither `ClassVar`- nor `Final`-wrapped is, in CPython, an annotation only:
//! it stores nothing in the class `__dict__`, and the attribute exists once
//! `__init__` assigns it. pycc models it the same way. A declaration creates
//! no class attribute and no slot by itself; its one effect is to fix the
//! slot type of the attribute the class's own `__init__` establishes at top
//! level (D-154's slot invariant: every slot is assigned by `__init__` before
//! any read). [`super::init_slot::collect_init_attrs`] applies that override.
//!
//! This module owns the class-body half: recognizing a declaration
//! ([`instance_declaration_name`], shared with `super::body`'s walk so the
//! two cannot drift), resolving and gating its type
//! ([`collect_declared_attrs`]), and refusing a declaration that no own
//! `__init__` establishes or that collides with a method of the same body
//! ([`reject_unestablished_or_colliding`]). `docs/TYPE_SYSTEM.md`'s class
//! "Current state" paragraph is the contract.
//!
//! Only a plain class body reaches here: a `@dataclass` body treats the same
//! spelling as a field, and `Protocol`/`Enum` bodies return before the walk.

use super::reserved_names::{ClassBodyRoute, reject_reserved_class_attr_name};
use super::{ClassAnnotationInfo, PropertyDef};
use crate::{Ty, unsupported};
use pycc_ast::{Expr, StmtAnnAssign};
use pycc_diag::Diagnostic;

/// One class-body instance attribute declaration, in source order.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct DeclaredAttr {
    /// The declared attribute name.
    pub(super) name: String,
    /// The declared type, already resolved and admitted.
    pub(super) ty: Ty,
    /// The declaration statement's range, for diagnostics.
    pub(super) range: std::ops::Range<u32>,
}

/// The attribute name `ann` declares when it is an instance attribute
/// declaration, and `None` otherwise.
///
/// A declaration is a value-less annotation on a bare name whose annotation is
/// not `ClassVar`-wrapped and not `Final`-wrapped (after peeling any
/// `Annotated[X, ...]` layers, see [`is_final_wrapper`]). A malformed
/// `ClassVar` (bare, or with two arguments) is not a declaration either: the
/// walk's own `strip_class_var` call reports it exactly as before.
pub(super) fn instance_declaration_name(ann: &StmtAnnAssign) -> Option<&str> {
    let Expr::Name(target) = ann.target.as_ref() else {
        return None;
    };
    let plain = ann.value.is_none()
        && super::attrs::strip_class_var(&ann.annotation).is_ok_and(|s| !s.is_class_var)
        // Part 1 of #889: a `Final` quoted inside `Annotated[...]` is seen
        // exactly where its unquoted spelling is.
        && !is_final_wrapper(&pycc_ast::unquote_nested_string_annotations(&ann.annotation));
    plain.then_some(target.id.as_str())
}

/// Whether `annotation` is a `Final` wrapper (bare `Final` or `Final[...]`),
/// seen through any number of `Annotated[X, ...]` layers.
///
/// The `Annotated` peel mirrors `func::bare_container`'s
/// `strip_transparent_wrappers`: only a tuple slice of at least two elements
/// is peeled, so a malformed `Annotated[X]` keeps `annotation_to_ty`'s own
/// arity message. The two `Final` arms mirror `attrs::strip_final`. Without
/// the peel, a value-less `x: Annotated[Final[int], "m"]` would silently
/// become a mutable declaration instead of keeping today's "has no value"
/// refusal.
fn is_final_wrapper(annotation: &Expr) -> bool {
    match annotation {
        Expr::Name(name) => name.id.as_str() == "Final",
        Expr::Subscript(sub) => match sub.value.as_ref() {
            Expr::Name(base) if base.id.as_str() == "Final" => true,
            Expr::Name(base) if base.id.as_str() == "Annotated" => match sub.slice.as_ref() {
                Expr::Tuple(tuple) if tuple.elts.len() >= 2 => is_final_wrapper(&tuple.elts[0]),
                _ => false,
            },
            _ => false,
        },
        _ => false,
    }
}

/// Collects every instance attribute declaration of a plain class body, in
/// source order, resolving and gating each one's type.
///
/// Runs before the class-body walk, because a declaration may follow
/// `def __init__` and the `__init__` pre-scan needs the full list. Each
/// declaration is checked in this order: the reserved-name guard every
/// class-body binding route shares, the duplicate check, `annotation_to_ty`
/// (which already applies `check_container_ty`, so an element type the
/// codegen cannot compile keeps its usual `T0034`/`T0036`/... code), then the
/// admitted-type check. A bare `list`/`dict` gets D-228's parameterized-form
/// advice, because this position lowers `list[int]`/`dict[str, int]`.
pub(super) fn collect_declared_attrs(
    body: &[pycc_ast::Stmt],
    class_name: &str,
    type_param: Option<&str>,
    aliases: &[(String, Ty)],
    class_name_defs: &[ClassAnnotationInfo],
) -> Result<Vec<DeclaredAttr>, Diagnostic> {
    let mut declared: Vec<DeclaredAttr> = Vec::new();
    for stmt in body {
        let pycc_ast::Stmt::AnnAssign(ann) = stmt else {
            continue;
        };
        let Some(name) = instance_declaration_name(ann) else {
            continue;
        };
        reject_reserved_class_attr_name(name, ClassBodyRoute::Plain, ann.range.into())?;
        if declared.iter().any(|d| d.name == name) {
            return Err(unsupported(
                format!(
                    "instance attribute `{name}` is declared more than once in class \
                     `{class_name}` -- declare it once"
                ),
                ann.range,
            ));
        }
        let ty = crate::annotation_to_ty(
            &ann.annotation,
            type_param,
            Some(class_name),
            aliases,
            class_name_defs,
        )
        .map_err(|error| crate::func::with_bare_list_or_dict_advice(error, &ann.annotation))?;
        if !matches!(
            ty,
            Ty::Int
                | Ty::Float
                | Ty::Bool
                | Ty::Str
                | Ty::Param(_)
                | Ty::List(_)
                | Ty::Dict(..)
                // Part 1 of #1367: a class a foreign import binds, stored as
                // its `PyObject*` pointer word (D-154's pointer rule).
                | Ty::Object
                // #1389: an instance of a pycc class, stored as its
                // pointer word. Instances are never freed (`pycc_rt::
                // instance`), so the slot needs no refcount traffic.
                | Ty::Instance(_)
        ) {
            return Err(unsupported(
                format!(
                    "instance attribute `{name}` declared in class `{class_name}` has type `{}`, \
                     which has no instance-slot representation -- a class-body declaration \
                     admits only `int`, `float`, `bool`, `str`, a type parameter, `list[int]`, \
                     `dict[str, int]`, a class of this program, or a class a foreign import \
                     binds",
                    ty.name()
                ),
                ann.range,
            ));
        }
        declared.push(DeclaredAttr {
            name: name.to_string(),
            ty,
            range: ann.range.into(),
        });
    }
    Ok(declared)
}

/// The method-like tables of one class body, for
/// [`reject_unestablished_or_colliding`].
pub(super) struct ClassMethodTables<'a> {
    /// `(source name, mangled name)` for every regular (and abstract) method.
    pub(super) methods: &'a [(String, String)],
    /// Every `@property`.
    pub(super) properties: &'a [PropertyDef],
    /// `(source name, mangled name)` for every `@staticmethod`.
    pub(super) static_methods: &'a [(String, String)],
    /// `(source name, mangled name)` for every `@classmethod`.
    pub(super) class_methods: &'a [(String, String)],
}

/// Refuses the first declaration, in source order, that either collides with
/// a method-like member of the same body or is never established by the
/// class's own `__init__`.
///
/// `attrs` is the slot list the own `__init__`'s pre-scan built (empty when
/// the class has no own `__init__`). A collision is reported ahead of the
/// missing assignment for the same declaration, because renaming resolves
/// both.
///
/// A declaration `__init__` never assigns is refused rather than accepted as
/// an inert annotation: accepting it would either record a slot `__init__`
/// never writes (breaking D-154's read-after-assign invariant) or leave a
/// later `self.x = v` in another method failing as "no attribute" despite the
/// visible declaration. The same holds for an attribute only an ancestor
/// establishes. A refusal can be loosened later; an accepted inert form could
/// not be tightened.
pub(super) fn reject_unestablished_or_colliding(
    declared: &[DeclaredAttr],
    attrs: &[(String, Ty)],
    tables: &ClassMethodTables<'_>,
    class_name: &str,
) -> Result<(), Diagnostic> {
    for decl in declared {
        let name = decl.name.as_str();
        let clash = if tables.methods.iter().any(|(n, _)| n == name) {
            Some("method")
        } else if tables.properties.iter().any(|p| p.name == name) {
            Some("property")
        } else if tables.static_methods.iter().any(|(n, _)| n == name) {
            Some("staticmethod")
        } else if tables.class_methods.iter().any(|(n, _)| n == name) {
            Some("classmethod")
        } else {
            None
        };
        if let Some(kind) = clash {
            return Err(unsupported(
                format!(
                    "instance attribute `{name}` declared in class `{class_name}` collides with \
                     the {kind} `{name}` of the same class"
                ),
                decl.range.clone(),
            ));
        }
        if !attrs.iter().any(|(n, _)| n == name) {
            return Err(unsupported(
                format!(
                    "instance attribute `{name}` declared in class `{class_name}` is never \
                     assigned at the top level of `{class_name}.__init__` -- assign it there \
                     (`self.{name} = ...`){}",
                    class_constant_advice(name, &decl.ty)
                ),
                decl.range.clone(),
            ));
        }
    }
    Ok(())
}

/// The "or make it a class constant" clause of the not-established message,
/// offered only for a declared type a class constant accepts (a scalar;
/// D-228's rule of never advising a form the position refuses). A container
/// or type-parameter declaration gets no such clause.
fn class_constant_advice(name: &str, ty: &Ty) -> String {
    let literal = match ty {
        Ty::Int => "1",
        Ty::Float => "1.0",
        Ty::Bool => "True",
        Ty::Str => "\"\"",
        _ => return String::new(),
    };
    format!(
        ", or give it a value (`{name}: {} = {literal}`) to make it a class constant",
        ty.name()
    )
}

#[cfg(test)]
#[path = "declared_attrs_tests.rs"]
mod tests;
