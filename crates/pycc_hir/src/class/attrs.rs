//! Class-body attribute lowering and collision checking, extracted from
//! [`super::body`] (D-185: decompose the part a change touches).
//!
//! `super::body` owns the statement walk itself -- classifying each statement
//! and lowering methods. This module owns everything specific to a class-level
//! *attribute*: stripping a `ClassVar[...]` wrapper, lowering an annotated
//! declaration into a `(name, type, constant value)` entry, extracting the
//! compile-time constant from its right-hand side, and rejecting a class
//! attribute whose name collides with something else the class exposes.

use super::attr_initializer::{
    InitializerScope, Spelling, class_attr_value, classify_non_literal, infer_class_attr_ty,
    no_class_attr_value,
};
use super::reserved_names::{ClassBodyRoute, reject_reserved_class_attr_name};
use super::{ClassAnnotationInfo, ClassAttrValue, HirClassDef, PropertyDef, is_scalar_slot_type};
use crate::{Ty, unsupported};
use pycc_ast::Expr;
use pycc_diag::Diagnostic;

/// #911: Strips a class-body-only `ClassVar[...]` wrapper from an annotation.
///
/// Returns the inner annotation and whether a wrapper was present. Unlike
/// `Final`/`Annotated`, `ClassVar` is **not** unwrapped by the shared
/// `pycc_hir::func::annotation_to_ty` -- it is valid only here, on a class
/// body attribute declaration, so `annotation_to_ty` rejects it outright and
/// this is the one caller that strips it first.
///
/// A bare `ClassVar` (no subscript) and a multi-argument `ClassVar[T, U]` are
/// both `C0001`, mirroring `Final`'s own "takes exactly one type argument".
pub(super) struct StrippedAnnotation<'a> {
    /// The annotation with any `ClassVar[...]` wrapper removed.
    pub(super) expr: &'a Expr,
    /// Whether a `ClassVar[...]` wrapper was actually present.
    ///
    /// This survives the stripping because it is not recoverable from
    /// `expr` afterwards, and [`lower_class_attr`] needs it: PEP 591 forbids
    /// `ClassVar[Final[T]]`, which is byte-identical to a valid `Final[T]`
    /// once the wrapper is gone.
    pub(super) is_class_var: bool,
}

pub(super) fn strip_class_var(annotation: &Expr) -> Result<StrippedAnnotation<'_>, Diagnostic> {
    match annotation {
        Expr::Name(name) if name.id.as_str() == "ClassVar" => Err(unsupported(
            "a bare `ClassVar` is not a valid annotation -- write `ClassVar[<type>]` with \
             exactly one type argument",
            pycc_ast::expr_range(annotation),
        )),
        Expr::Subscript(sub) if matches!(sub.value.as_ref(), Expr::Name(n) if n.id.as_str() == "ClassVar") =>
        {
            if matches!(sub.slice.as_ref(), Expr::Tuple(_)) {
                return Err(unsupported(
                    "ClassVar takes exactly one type argument",
                    pycc_ast::expr_range(&sub.slice),
                ));
            }
            Ok(StrippedAnnotation {
                expr: sub.slice.as_ref(),
                is_class_var: true,
            })
        }
        other => Ok(StrippedAnnotation {
            expr: other,
            is_class_var: false,
        }),
    }
}

/// #916: Strips a `Final[...]` wrapper from a class-body attribute
/// annotation, mirroring [`strip_class_var`]'s shape.
///
/// Returns the *inner* annotation and whether a wrapper was present. A bare
/// `Final` (no subscript) is valid here -- PEP 591 allows `X: Final = 1` and
/// the type is then inferred from the literal right-hand side, exactly as
/// #910's un-annotated spelling does -- so it yields `None` rather than an
/// inner expression. `Final[T, U]` reuses `annotation_to_ty`'s own
/// "takes exactly one type argument" wording, and `Final[T,]` (a one-element
/// tuple slice) unwraps to `T`, so both spellings behave identically to the
/// variable-level position.
///
/// Unlike [`strip_class_var`], stripping here is a convenience rather than a
/// necessity: the shared `annotation_to_ty` unwraps `Final[X]` on its own.
/// The wrapper is intercepted anyway because the two PEP 591-invalid nestings
/// (`ClassVar[Final[T]]`, `Final[ClassVar[T]]`) are only distinguishable at
/// this seam -- see [`lower_class_attr`].
///
/// The attribute spelling `typing.Final[...]` is deliberately not handled,
/// exactly as [`strip_class_var`] does not handle `typing.ClassVar[...]`: it
/// falls through to `annotation_to_ty` and keeps its existing `C0001`.
fn strip_final(annotation: &Expr) -> Result<(Option<&Expr>, bool), Diagnostic> {
    match annotation {
        Expr::Name(name) if name.id.as_str() == "Final" => Ok((None, true)),
        Expr::Subscript(sub) if matches!(sub.value.as_ref(), Expr::Name(n) if n.id.as_str() == "Final") =>
        {
            let inner = match sub.slice.as_ref() {
                Expr::Tuple(tuple) if tuple.elts.len() != 1 => {
                    return Err(unsupported(
                        "Final takes exactly one type argument",
                        pycc_ast::expr_range(&sub.slice),
                    ));
                }
                Expr::Tuple(tuple) => &tuple.elts[0],
                other => other,
            };
            Ok((Some(inner), true))
        }
        other => Ok((Some(other), false)),
    }
}

/// Whether `annotation` names `ClassVar`, in either the bare (`ClassVar`) or
/// the subscripted (`ClassVar[T]`) spelling.
///
/// [`strip_class_var`] cannot answer this question: it returns `Err` for the
/// bare spelling, so a caller that only needs "is this a `ClassVar` at all"
/// would either propagate a message about the wrong defect or lose the bare
/// case entirely.
fn is_class_var_annotation(annotation: &Expr) -> bool {
    match annotation {
        Expr::Name(name) => name.id.as_str() == "ClassVar",
        Expr::Subscript(sub) => {
            matches!(sub.value.as_ref(), Expr::Name(n) if n.id.as_str() == "ClassVar")
        }
        _ => false,
    }
}

/// #911: Lowers one annotated class-body attribute declaration
/// (`MIN_WIDTH: int = -1024`, `LIMIT: ClassVar[int] = 8`) into a
/// `(name, type, constant value)` entry for [`HirClassDef::class_attrs`].
///
/// `stripped` is the `ClassVar`-stripped annotation plus whether that
/// wrapper was present; `already` is the entries accumulated so far in this
/// body, for duplicate detection.
///
/// **Named invariant -- class attributes are restricted to scalar slot
/// types.** Beyond D-154's single-word storage constraint, this is what
/// keeps `__set_name__` untriggerable per #585/D-213: rejecting a
/// non-scalar annotation rejects a descriptor-valued class attribute
/// (`x: SomeDescriptor = SomeDescriptor()`) along with it, so
/// `__set_name__`'s own precondition never arises. Relaxing this
/// restriction requires revisiting #585 in the same change. The annotated
/// spelling of #1345's foreign `staticmethod` shape is refused by
/// [`classify_non_literal`], so this path still only ever yields a scalar.
#[allow(clippy::too_many_arguments)]
pub(super) fn lower_class_attr(
    ann: &pycc_ast::StmtAnnAssign,
    stripped: StrippedAnnotation<'_>,
    class_name: &str,
    type_param: Option<&str>,
    aliases: &[(String, Ty)],
    class_name_defs: &[ClassAnnotationInfo],
    already: &[(String, Ty, ClassAttrValue)],
    scope: &InitializerScope<'_>,
) -> Result<(String, Ty, ClassAttrValue), Diagnostic> {
    let Expr::Name(target_name) = ann.target.as_ref() else {
        return Err(unsupported(
            "a class-level attribute annotation must target a bare name (`X: int = 1`), not an \
             attribute access, subscript, or other expression",
            pycc_ast::expr_range(&ann.target),
        ));
    };
    let attr_name = target_name.id.to_string();
    let non_literal = |value: &Expr| {
        classify_non_literal(
            value,
            &attr_name,
            Spelling::Annotated,
            scope,
            ann.range.into(),
        )
        .expect_err("the annotated spelling never admits a non-literal initializer")
    };
    reject_reserved_class_attr_name(&attr_name, ClassBodyRoute::Plain, ann.range.into())?;
    if already.iter().any(|(name, _, _)| name == &attr_name) {
        return Err(unsupported(
            format!(
                "class attribute `{attr_name}` is already defined in class `{class_name}` -- \
                 duplicate class attribute names are not allowed"
            ),
            ann.range,
        ));
    }
    // PEP 591 (#916): `Final[X]` on a class-body attribute is accepted and
    // stripped to `X`. #911 rejected the spelling outright because
    // `Final`'s non-reassignability is tracked by the type checker's
    // `Environment.finals` set, populated from `HirStmt::AnnAssign` -- and a
    // class attribute produces no `HirStmt` at all, so a stripped `Final`
    // would have carried none of `Final`'s meaning. That objection does not
    // survive #911's own model: every write path to a class attribute is
    // already `T0044`, so the binding is immutable without `Environment`
    // knowing anything about it, and `Final[T]` here is a documentation-only
    // wrapper over an already-final binding rather than an unmodelled
    // guarantee.
    //
    // These two checks run *after* the reserved-name and duplicate-name
    // checks above, so a program that both duplicates a name and misnests the
    // wrappers keeps reporting the duplicate, exactly as it did before #916.
    let StrippedAnnotation {
        expr: annotation,
        is_class_var,
    } = stripped;
    // Part 1 of #889: the two nesting checks match by shape, so they read a
    // copy with every nested string annotation unquoted --
    // `ClassVar["Final[int]"]` fails exactly as `ClassVar[Final[int]]` does.
    // The type itself is still resolved from the original, so its spans stay
    // on the literal.
    let unquoted = pycc_ast::unquote_nested_string_annotations(annotation);
    let (unquoted_inner, is_final) = strip_final(&unquoted)?;
    let (annotation, _) = strip_final(annotation)?;
    if is_final && is_class_var {
        return Err(unsupported(
            format!(
                "`ClassVar[Final[...]]` on the class-level attribute `{attr_name}` is not a \
                 valid annotation -- PEP 591 forbids nesting `Final` inside `ClassVar`; a class \
                 attribute is a class variable already, so write `{attr_name}: Final[int] = 1`"
            ),
            ann.range,
        ));
    }
    if is_final && unquoted_inner.is_some_and(is_class_var_annotation) {
        return Err(unsupported(
            format!(
                "`Final[ClassVar[...]]` on the class-level attribute `{attr_name}` is not a \
                 valid annotation -- PEP 591 forbids nesting `ClassVar` inside `Final`; a class \
                 attribute is a class variable already, so write `{attr_name}: Final[int] = 1`"
            ),
            ann.range,
        ));
    }
    let attr_ty = match annotation {
        Some(annotation) => {
            let attr_ty = crate::annotation_to_ty(
                annotation,
                type_param,
                Some(class_name),
                aliases,
                class_name_defs,
            )?;
            // A type parameter has no compile-time constant value to fold, so
            // a generic class's own `T` is rejected here even though it *is*
            // one of `is_scalar_slot_type`'s accepted types.
            if matches!(attr_ty, Ty::Param(_)) {
                return Err(unsupported(
                    format!(
                        "class attribute `{attr_name}` is annotated with the type parameter \
                         `{}` -- a class attribute is a compile-time constant, and a type \
                         parameter has no constant value to fold",
                        attr_ty.name()
                    ),
                    ann.range,
                ));
            }
            if !is_scalar_slot_type(&attr_ty) {
                return Err(unsupported(
                    format!(
                        "class attribute `{attr_name}` has type `{}`, which is not a scalar \
                         slot type -- only `int`, `float`, `bool`, and `str` are supported (a \
                         class attribute is a compile-time constant folded at every read; \
                         restricting it to scalars is also what keeps `__set_name__` \
                         untriggerable, see #585)",
                        attr_ty.name()
                    ),
                    ann.range,
                ));
            }
            attr_ty
        }
        // #916: a bare `X: Final = 1` has no inner annotation to resolve, so
        // the type comes from the literal right-hand side through #910's
        // existing `infer_class_attr_ty`. That function only ever yields a
        // scalar, so the `Ty::Param` and scalar-slot checks above are
        // correctly skipped here rather than left unreachable.
        None => {
            let Some(value) = &ann.value else {
                return Err(no_class_attr_value(&attr_name, ann.range.into()));
            };
            let Some(attr_ty) = infer_class_attr_ty(value) else {
                return Err(non_literal(value));
            };
            attr_ty
        }
    };
    let Some(value) = &ann.value else {
        return Err(no_class_attr_value(&attr_name, ann.range.into()));
    };
    // #1345: a non-literal initializer gets the classifier's precise
    // message from inside the extractor; a literal one keeps its own checks.
    let attr_value = class_attr_value(
        value,
        &attr_ty,
        &attr_name,
        Spelling::Annotated,
        scope,
        ann.range.into(),
    )?;
    Ok((attr_name, attr_ty, attr_value))
}

/// #910: Lowers one *un-annotated* class-body assignment (`X = 1`,
/// `SCALE = -1.5`) into the same `(name, type, constant value)` entry
/// [`lower_class_attr`] produces for the annotated spelling.
///
/// The two spellings differ only in where the type comes from: the annotated
/// one resolves it from the annotation, this one infers it from the literal
/// via [`infer_class_attr_ty`]. Everything downstream -- the literal
/// extraction in [`class_attr_value`], the reserved-name check, the duplicate
/// check, and [`reject_class_attr_collisions`] -- is shared, so both
/// spellings accept and reject exactly the same programs.
///
/// The #585/D-224 scalar-only invariant documented on [`lower_class_attr`]
/// holds for every *literal* attribute by construction: `infer_class_attr_ty`
/// only ever yields a scalar. A non-literal initializer goes to
/// [`classify_non_literal`], whose one admitted shape,
/// `name = staticmethod(<foreign ref>)` (#1345, D-256), is the single
/// non-scalar exception: a `staticmethod` object is a descriptor but has no
/// `__set_name__` (measured on CPython 3.14.7), so `__set_name__`'s
/// precondition still never arises.
pub(super) fn lower_unannotated_class_attr(
    assign: &pycc_ast::StmtAssign,
    class_name: &str,
    already: &[(String, Ty, ClassAttrValue)],
    scope: &InitializerScope<'_>,
) -> Result<(String, Ty, ClassAttrValue), Diagnostic> {
    // `a = b = 1` binds both names to one value. Modelling it would mean
    // pushing two entries from one statement, each needing its own duplicate
    // and collision check; there is no demand for it, so it stays `C0001`.
    if assign.targets.len() != 1 {
        return Err(unsupported(
            "a class-level attribute assignment must have a single target (`X = 1`), not \
             multiple targets",
            assign.range,
        ));
    }
    let target = &assign.targets[0];
    let Expr::Name(target_name) = target else {
        return Err(unsupported(
            "a class-level attribute assignment must target a bare name (`X = 1`), not an \
             attribute access, subscript, or other expression",
            pycc_ast::expr_range(target),
        ));
    };
    let attr_name = target_name.id.to_string();
    reject_reserved_class_attr_name(&attr_name, ClassBodyRoute::Plain, assign.range.into())?;
    if already.iter().any(|(name, _, _)| name == &attr_name) {
        return Err(unsupported(
            format!(
                "class attribute `{attr_name}` is already defined in class `{class_name}` -- \
                 duplicate class attribute names are not allowed"
            ),
            assign.range,
        ));
    }
    let value = assign.value.as_ref();
    let Some(attr_ty) = infer_class_attr_ty(value) else {
        let target = classify_non_literal(
            value,
            &attr_name,
            Spelling::Unannotated,
            scope,
            assign.range.into(),
        )?;
        return Ok((attr_name, Ty::Object, ClassAttrValue::ForeignStatic(target)));
    };
    let attr_value = class_attr_value(
        value,
        &attr_ty,
        &attr_name,
        Spelling::Unannotated,
        scope,
        assign.range.into(),
    )?;
    Ok((attr_name, attr_ty, attr_value))
}

/// The class-level tables [`reject_class_attr_collisions`] checks a class
/// attribute's name against.
///
/// Grouped into a struct rather than passed positionally: the check needs the
/// class's own attribute, property, and three method tables plus its MRO and
/// the module's class table, which is well past the point where positional
/// arguments stop being readable (and past `clippy::too_many_arguments`).
pub(super) struct ClassAttrCollisionInput<'a> {
    pub class_attrs: &'a [(String, Ty, ClassAttrValue)],
    pub attrs: &'a [(String, Ty)],
    pub properties: &'a [PropertyDef],
    pub methods: &'a [(String, String)],
    pub static_methods: &'a [(String, String)],
    pub class_methods: &'a [(String, String)],
    pub class_name: &'a str,
    pub mro: &'a [String],
    pub defined_classes: &'a [(String, HirClassDef)],
    pub range: std::ops::Range<u32>,
}

/// #911: Rejects a class attribute that collides with an instance attribute
/// slot, a `@property`, or a method of the same name, in either declaration
/// order.
///
/// This runs **after** the body walk rather than at the `AnnAssign` site:
/// `attrs` is populated by `collect_init_attrs` when the walk reaches
/// `__init__`, so a class attribute declared *before* `__init__` would see an
/// empty `attrs` and slip through a statement-site check.
///
/// The direction checked here is exactly one way round: every entry of *this*
/// class's `class_attrs` against this class's own tables and against every MRO
/// base's tables. The reverse direction -- this class's `attrs` (or
/// `properties`) shadowing an *ancestor's* `class_attrs` -- is deliberately
/// **not** checked here, because the write itself is what is ill-formed there,
/// and `pycc_types::class::check_attr_set` already rejects it with `T0044`
/// through `lookup_class_attr_through_mro`'s full-MRO walk, pointing at the
/// offending assignment rather than at the class as a whole.
///
/// A collision is `C0001`, not `T0052`: `T0052`'s existing condition fires
/// only when a redeclaration's `Ty` *differs*, and a same-typed class
/// attribute shadowing an instance slot is just as broken -- the read would
/// fold to a constant while the write targeted a slot.
///
/// #910 added the three method tables to this check. They were missing while
/// only the annotated spelling existed, and the gap is observable in both
/// directions:
///
/// * `class A: f: int = 2` alongside `def f(self)` printed `2` for `a.f`,
///   where CPython's later class-body binding wins and prints a bound method.
/// * `class B(A): f: int = 2` over an inherited `A.f` printed `1` for `b.f()`,
///   where CPython raises `TypeError: 'int' object is not callable`.
///
/// Neither divergence is modellable while a class attribute folds to a
/// constant at every read, so both spellings are rejected outright.
pub(super) fn reject_class_attr_collisions(
    input: &ClassAttrCollisionInput<'_>,
) -> Result<(), Diagnostic> {
    let &ClassAttrCollisionInput {
        class_attrs,
        attrs,
        properties,
        methods,
        static_methods,
        class_methods,
        class_name,
        mro,
        defined_classes,
        ref range,
    } = input;
    for (attr_name, _, value) in class_attrs {
        let foreign = matches!(value, ClassAttrValue::ForeignStatic(_));
        if let Some(what) = collision_kind(
            attr_name,
            attrs,
            properties,
            methods,
            static_methods,
            class_methods,
        ) {
            return Err(class_attr_collision(
                class_name,
                attr_name,
                what,
                foreign,
                range.clone(),
            ));
        }
        for base in mro.iter().skip(1) {
            // Every class in the MRO was placed there by `compute_c3_mro`,
            // which only references classes from `defined_classes` -- so
            // this lookup always succeeds. `.expect()`'s panic path lives in
            // libcore, outside this crate's instrumented regions (D-014).
            let (_, base_def) = defined_classes
                .iter()
                .find(|(name, _)| name == base)
                .expect("every class in the MRO must be in defined_classes");
            if let Some(what) = collision_kind(
                attr_name,
                &base_def.attrs,
                &base_def.properties,
                &base_def.methods,
                &base_def.static_methods,
                &base_def.class_methods,
            ) {
                return Err(class_attr_collision(
                    class_name,
                    attr_name,
                    &format!("{what} inherited from `{base}`"),
                    foreign,
                    range.clone(),
                ));
            }
        }
    }
    Ok(())
}

/// Names what one class exposes under `attr_name`, or `None` if nothing does.
///
/// Shared by [`reject_class_attr_collisions`]'s two call sites -- the class's
/// own tables and each MRO base's -- so a base is checked against exactly the
/// same five tables, in the same priority order, as the class itself. The
/// returned string carries its own article, because the caller may prefix it
/// (`"a method"` becomes `"a method inherited from `A`"`).
fn collision_kind(
    attr_name: &str,
    attrs: &[(String, Ty)],
    properties: &[PropertyDef],
    methods: &[(String, String)],
    static_methods: &[(String, String)],
    class_methods: &[(String, String)],
) -> Option<&'static str> {
    let bound = |table: &[(String, String)]| table.iter().any(|(name, _)| name == attr_name);
    if attrs.iter().any(|(name, _)| name == attr_name) {
        return Some("an instance attribute");
    }
    if properties.iter().any(|p| p.name == attr_name) {
        return Some("an `@property`");
    }
    if bound(methods) {
        return Some("a method");
    }
    if bound(static_methods) {
        return Some("a `@staticmethod`");
    }
    if bound(class_methods) {
        return Some("a `@classmethod`");
    }
    None
}

/// The single `C0001` a [`reject_class_attr_collisions`] collision produces.
///
/// `foreign` selects the reason: a literal attribute is folded to a
/// constant at every read, while a `staticmethod(<foreign callable>)`
/// attribute (Part 1 of #1284) is rewritten to its foreign reference at
/// every read and call -- either way the read never consults the other
/// binding, so the two cannot share a name.
fn class_attr_collision(
    class_name: &str,
    attr_name: &str,
    what: &str,
    foreign: bool,
    range: std::ops::Range<u32>,
) -> Diagnostic {
    let reason = if foreign {
        "a `staticmethod(...)` class attribute is rewritten to its foreign callable at every \
         read and call"
    } else {
        "a class attribute is folded to a constant at every read"
    };
    unsupported(
        format!(
            "class attribute `{class_name}.{attr_name}` collides with {what} of the same \
             name -- {reason}, so it can never share a name with a value that lives in an \
             instance slot, behind a descriptor, or in the class's method table"
        ),
        range,
    )
}

/// #913/D-235: Rejects a merged dataclass field whose name is also a
/// `ClassVar` anywhere in the class's MRO.
///
/// Extracted from `super::lower_class` (AGENTS.md "Keep source files
/// decomposable"): this module already owns every other class-attribute
/// collision rule, so the policy and its rationale belong here rather than
/// in the 4.6k-line parent. `lower_class` keeps the call, positioned on the
/// *merged* field list, because that is the only place all three reachable
/// shapes meet:
///
/// * same body, either order (`x: int` alongside `x: ClassVar[int] = 1`) --
///   unchecked before #913, because the dataclass duplicate check scans only
///   `dataclass_fields`, [`lower_class_attr`]'s scans only `class_attrs`, and
///   [`reject_class_attr_collisions`] runs against an `attrs` that is still
///   empty for a dataclass (it is filled from that very merge, immediately
///   after this call). CPython's two orders disagree with each other:
///   field-first drops the field, `ClassVar`-first turns it into a field with
///   a *default*.
/// * an own field over a base's `ClassVar` -- CPython turns the base's value
///   into the field's default, so its `__init__` takes an optional parameter
///   where pycc's synthesized one would take a required one.
/// * a cross-base split -- `D(A, B)` where `A` contributes the `ClassVar` and
///   sibling `B` the field. CPython processes fields in reverse-MRO order, so
///   `A`'s `ClassVar` *removes* `B`'s field; reversing the bases keeps it.
///   #969's slot-layout gate does not catch this, because a class
///   contributing only a `ClassVar` declares no instance attributes at all.
///
/// Every one of these is a dataclass field default in CPython's model or an
/// order-dependent field removal, neither of which this version represents,
/// so all of them are rejected (D-198: never diverge silently). The `D(B, A)`
/// base order is a program CPython runs and pycc could compile; rejecting it
/// too is the deliberate conservative narrowing recorded in D-235.
///
/// `range` is the class definition's own span, already normalized to the
/// `Range<u32>` [`crate::unsupported`] ultimately wants, so this module does
/// not depend on the AST's range type.
pub(super) fn reject_dataclass_field_class_var_collisions(
    merged_fields: &[(String, Ty)],
    own_class_attrs: &[(String, Ty, ClassAttrValue)],
    class_name: &str,
    mro: &[String],
    defined_classes: &[(String, HirClassDef)],
    range: std::ops::Range<u32>,
) -> Result<(), Diagnostic> {
    for (field_name, _) in merged_fields {
        if let Some(owner) = class_attr_owner(
            field_name,
            own_class_attrs,
            class_name,
            mro,
            defined_classes,
        ) {
            return Err(unsupported(
                format!(
                    "dataclass field `{field_name}` of class `{class_name}` shares its \
                     name with the class attribute `{owner}.{field_name}` -- a \
                     `ClassVar` is not a dataclass field, so CPython would either drop \
                     the field or give it the class attribute's value as a default, and \
                     dataclass field defaults are not supported in this version"
                ),
                range,
            ));
        }
    }
    Ok(())
}

/// #913: Names the class in `class_name`'s MRO that declares `attr` as a
/// class attribute (`ClassVar[T] = ...`, or #910's bare `X = 1`), or `None`
/// if no class in the MRO does.
///
/// The class's own `class_attrs` are not in `defined_classes` yet -- the
/// `HirClassDef` being built is what will be added there -- so they are
/// passed separately and checked first, matching
/// `lookup_class_attr_through_mro`'s most-derived-first order.
fn class_attr_owner<'a>(
    attr: &str,
    own_class_attrs: &[(String, Ty, ClassAttrValue)],
    class_name: &'a str,
    mro: &'a [String],
    defined_classes: &'a [(String, HirClassDef)],
) -> Option<&'a str> {
    if own_class_attrs.iter().any(|(name, _, _)| name == attr) {
        return Some(class_name);
    }
    mro.iter().skip(1).find_map(|base| {
        // Every class in the MRO was placed there by `compute_c3_mro` from
        // `defined_classes`, so `filter_map`'s miss arm is structurally
        // unreachable; using it rather than `.expect()` keeps D-014's
        // 100%-region gate satisfied without an uncoverable branch.
        defined_classes
            .iter()
            .find(|(name, _)| name == base)
            .filter(|(_, base_def)| base_def.class_attrs.iter().any(|(n, _, _)| n == attr))
            .map(|(name, _)| name.as_str())
    })
}

#[cfg(test)]
mod tests {
    use super::{ClassAttrValue, Ty};
    use crate::lower_checked;

    /// Asserts `source` is rejected with a `C0001` whose message contains
    /// `needle`. `crate::class::tests::assert_c0001` pins only the code, and
    /// every case here needs the *specific* collision partner named.
    fn assert_collision(source: &str, needle: &str) {
        let module = crate::pycc_parser_test_helper::parse(source);
        let diagnostic = lower_checked(&module).unwrap_err();
        assert_eq!(diagnostic.code, "C0001", "source: {source:?}");
        assert!(
            diagnostic.message.contains(needle),
            "expected {needle:?} in {:?}",
            diagnostic.message
        );
    }

    // -- #910: a class attribute colliding with the class's own method -----

    #[test]
    fn a_class_attribute_colliding_with_a_method_is_rejected() {
        assert_collision(
            "class C:\n    f: int = 2\n\n    def f(self) -> int:\n        return 1\n",
            "collides with a method",
        );
    }

    #[test]
    fn a_class_attribute_colliding_with_a_static_method_is_rejected() {
        assert_collision(
            "class C:\n    f: int = 2\n\n    @staticmethod\n    def f() -> int:\n        return 1\n",
            "collides with a `@staticmethod`",
        );
    }

    #[test]
    fn a_class_attribute_colliding_with_a_class_method_is_rejected() {
        assert_collision(
            "class C:\n    f: int = 2\n\n    @classmethod\n    def f(cls) -> int:\n        return 1\n",
            "collides with a `@classmethod`",
        );
    }

    // -- #910: the same three collisions against an MRO base ---------------

    #[test]
    fn a_class_attribute_colliding_with_an_inherited_method_is_rejected() {
        assert_collision(
            "class A:\n    def f(self) -> int:\n        return 1\n\n\nclass B(A):\n    f: int = 2\n",
            "collides with a method inherited from `A`",
        );
    }

    #[test]
    fn a_class_attribute_colliding_with_an_inherited_static_method_is_rejected() {
        assert_collision(
            "class A:\n    @staticmethod\n    def f() -> int:\n        return 1\n\n\nclass B(A):\n    f: int = 2\n",
            "collides with a `@staticmethod` inherited from `A`",
        );
    }

    #[test]
    fn a_class_attribute_colliding_with_an_inherited_class_method_is_rejected() {
        assert_collision(
            "class A:\n    @classmethod\n    def f(cls) -> int:\n        return 1\n\n\nclass B(A):\n    f: int = 2\n",
            "collides with a `@classmethod` inherited from `A`",
        );
    }

    #[test]
    fn an_unannotated_multi_target_assignment_is_rejected() {
        assert_collision("class C:\n    a = b = 1\n", "not multiple targets");
    }

    #[test]
    fn an_unannotated_tuple_target_is_rejected() {
        assert_collision("class C:\n    a, b = 1, 2\n", "must target a bare name");
    }

    #[test]
    fn an_unannotated_attribute_target_is_rejected() {
        assert_collision(
            "class C:\n    def __init__(self) -> None:\n        self.n = 0\n\n\nclass D:\n    C.x = 1\n",
            "must target a bare name",
        );
    }

    /// The annotated path has its own bare-name check, and D-014 measures each
    /// crate's regions from its own in-crate tests -- the unannotated pin above
    /// exercises a different function, so the annotated arm needs its own.
    #[test]
    fn an_annotated_attribute_target_is_rejected() {
        assert_collision(
            "class C:\n    def __init__(self) -> None:\n        self.n = 0\n\n\nclass D:\n    C.x: int = 1\n",
            "must target a bare name",
        );
    }

    #[test]
    fn a_duplicate_unannotated_class_attribute_is_rejected() {
        assert_collision(
            "class C:\n    X = 1\n    X = 2\n",
            "is already defined in class",
        );
    }

    #[test]
    fn an_unannotated_attribute_duplicating_an_annotated_one_is_rejected() {
        assert_collision(
            "class C:\n    X: int = 1\n    X = 2\n",
            "is already defined in class",
        );
    }

    #[test]
    fn an_annotated_attribute_duplicating_an_unannotated_one_is_rejected() {
        assert_collision(
            "class C:\n    X = 1\n    X: int = 2\n",
            "is already defined in class",
        );
    }

    #[test]
    fn an_unannotated_attribute_colliding_with_an_instance_slot_is_rejected() {
        assert_collision(
            "class C:\n    x = 1\n\n    def __init__(self) -> None:\n        self.x = 2\n",
            "collides with an instance attribute",
        );
    }

    #[test]
    fn an_unannotated_attribute_colliding_with_a_property_is_rejected() {
        assert_collision(
            "class C:\n    x = 1\n\n    def __init__(self) -> None:\n        self.n = 0\n\n    @property\n    def x(self) -> int:\n        return self.n\n",
            "collides with an `@property`",
        );
    }

    #[test]
    fn an_unannotated_attribute_colliding_with_a_method_is_rejected() {
        assert_collision(
            "class C:\n    f = 2\n\n    def f(self) -> int:\n        return 1\n",
            "collides with a method",
        );
    }

    #[test]
    fn an_unannotated_attribute_colliding_with_a_static_method_is_rejected() {
        assert_collision(
            "class C:\n    f = 2\n\n    @staticmethod\n    def f() -> int:\n        return 1\n",
            "collides with a `@staticmethod`",
        );
    }

    #[test]
    fn an_unannotated_attribute_colliding_with_a_class_method_is_rejected() {
        assert_collision(
            "class C:\n    f = 2\n\n    @classmethod\n    def f(cls) -> int:\n        return 1\n",
            "collides with a `@classmethod`",
        );
    }

    // -- #910/#1368: the `__slots__` spellings `class::slots` does not admit -

    #[test]
    fn a_value_less_or_dataclass_slots_binding_is_rejected() {
        for source in [
            "class C:\n    __slots__: str\n",
            "from dataclasses import dataclass\nfrom typing import ClassVar\n\n\n@dataclass\n\
             class C:\n    __slots__: ClassVar[str] = \"a\"\n",
        ] {
            assert_collision(source, "this `__slots__` spelling is not supported yet");
        }
    }

    // -- #916: `Final[...]` on a class-body attribute ----------------------

    /// Lowers `source` and returns class `C`'s class attributes.
    fn class_c_attrs(source: &str) -> Vec<(String, Ty, ClassAttrValue)> {
        let module = crate::pycc_parser_test_helper::parse(source);
        let hir = lower_checked(&module).expect("the class body must lower");
        let (_, class_def) = hir
            .class_defs
            .iter()
            .find(|(name, _)| name == "C")
            .expect("class `C` must be lowered");
        class_def.class_attrs.clone()
    }

    /// PEP 591 (#916): every `Final` spelling of the same declaration lowers
    /// to the byte-identical entry the plain annotation produces. `Final` is
    /// a binding-level property and the binding is already immutable here
    /// (every write path is `T0044`), so the wrapper contributes nothing to
    /// the lowered form -- which is exactly why accepting it is safe.
    #[test]
    fn every_final_spelling_lowers_identically_to_the_plain_annotation() {
        let plain = class_c_attrs("class C:\n    X: int = 1\n");
        assert_eq!(
            plain,
            vec![("X".to_string(), Ty::Int, ClassAttrValue::Int(1))]
        );
        for source in [
            // `Final[T]`, the subscripted spelling.
            "class C:\n    X: Final[int] = 1\n",
            // `Final[T,]`, a one-element tuple slice -- the same shape
            // `annotation_to_ty` already unwraps in variable position.
            "class C:\n    X: Final[int,] = 1\n",
            // A bare `Final`, whose type is inferred from the literal.
            "class C:\n    X: Final = 1\n",
            // A redundant `Final[Final[T]]`: `strip_final` unwraps one layer
            // and the shared `annotation_to_ty` unwraps the rest, so the class
            // body accepts exactly what the variable-level position already
            // accepts rather than becoming a stricter special case.
            "class C:\n    X: Final[Final[int]] = 1\n",
        ] {
            assert_eq!(class_c_attrs(source), plain, "source: {source:?}");
        }
    }

    /// A bare `Final` infers its type from the literal exactly as #910's
    /// un-annotated spelling does, across every literal shape.
    #[test]
    fn a_bare_final_class_attribute_infers_every_literal_shape() {
        assert_eq!(
            class_c_attrs(
                "class C:\n    I: Final = 1\n    F: Final = 1.5\n    B: Final = True\n    S: Final = \"cfg\"\n    NI: Final = -1024\n",
            ),
            vec![
                ("I".to_string(), Ty::Int, ClassAttrValue::Int(1)),
                ("F".to_string(), Ty::Float, ClassAttrValue::Float(1.5)),
                ("B".to_string(), Ty::Bool, ClassAttrValue::Bool(true)),
                (
                    "S".to_string(),
                    Ty::Str,
                    ClassAttrValue::Str("cfg".to_string())
                ),
                ("NI".to_string(), Ty::Int, ClassAttrValue::Int(-1024)),
            ]
        );
    }

    /// `Final` takes exactly one type argument, reusing `annotation_to_ty`'s
    /// own wording so the class-body position and the variable position
    /// report identically.
    #[test]
    fn a_multi_argument_final_class_attribute_is_rejected() {
        assert_collision(
            "class C:\n    X: Final[int, str] = 1\n",
            "Final takes exactly one type argument",
        );
    }

    /// PEP 591 forbids nesting `Final` inside `ClassVar`. `class/body.rs`
    /// strips the `ClassVar` wrapper *before* calling `lower_class_attr`, so
    /// without the threaded `is_class_var` flag this annotation would arrive
    /// indistinguishable from a plain `Final[int]` and be accepted.
    #[test]
    fn a_class_var_wrapping_final_class_attribute_is_rejected() {
        assert_collision(
            "class C:\n    X: ClassVar[Final[int]] = 1\n",
            "forbids nesting `Final` inside `ClassVar`",
        );
    }

    /// PEP 591 forbids the opposite nesting too. Both the subscripted and
    /// the bare inner `ClassVar` report the nesting, not `annotation_to_ty`'s
    /// generic "`ClassVar` is only valid on a class-body attribute
    /// declaration" message -- which would be actively misleading here,
    /// since this *is* a class-body attribute declaration.
    #[test]
    fn a_final_wrapping_class_var_class_attribute_is_rejected() {
        for source in [
            "class C:\n    X: Final[ClassVar[int]] = 1\n",
            "class C:\n    X: Final[ClassVar] = 1\n",
        ] {
            assert_collision(source, "forbids nesting `ClassVar` inside `Final`");
        }
    }

    /// The nesting checks run after the duplicate-name check, so today's
    /// diagnostic precedence is unchanged by #916.
    #[test]
    fn a_duplicate_name_outranks_an_invalid_final_nesting() {
        assert_collision(
            "class C:\n    X: int = 1\n    X: ClassVar[Final[int]] = 2\n",
            "is already defined in class `C`",
        );
    }

    /// An inner annotation that is neither a bare name nor a subscript is
    /// not a `ClassVar` nesting -- it falls through to `annotation_to_ty`
    /// and keeps that position's own diagnostic.
    #[test]
    fn a_final_wrapping_an_unsupported_annotation_shape_propagates() {
        assert_collision(
            "class C:\n    X: Final[b\"int\"] = 1\n",
            "got a bytes literal",
        );
    }

    /// A `Final`-wrapped annotation that does not resolve propagates the
    /// inner failure rather than reporting the wrapper.
    #[test]
    fn a_final_wrapping_an_unknown_name_propagates_the_inner_error() {
        assert_collision("class C:\n    X: Final[Nope] = 1\n", "Nope");
    }

    /// The scalar-slot restriction (#585/D-224) applies through the wrapper.
    #[test]
    fn a_final_wrapping_a_non_scalar_annotation_is_rejected() {
        assert_collision(
            "class C:\n    X: Final[None] = 1\n",
            "which is not a scalar slot type",
        );
    }

    /// So does the type-parameter restriction.
    #[test]
    fn a_final_wrapping_a_type_parameter_is_rejected() {
        assert_collision(
            "class Box[T]:\n    X: Final[T] = 1\n\n    def __init__(self, item: T) -> None:\n        self.item = item\n",
            "has no constant value to fold",
        );
    }

    /// A value-less declaration has nothing to fold in either arm: the
    /// annotated one reaches the shared check after resolving its type, the
    /// bare-`Final` one must reach it *before* inferring one.
    #[test]
    fn a_value_less_final_class_attribute_is_rejected_in_both_arms() {
        for source in ["class C:\n    X: Final[int]\n", "class C:\n    X: Final\n"] {
            assert_collision(source, "has no value");
        }
    }

    /// A bare `Final` whose right-hand side is not a foldable literal
    /// reuses #910's shared initializer-shape diagnostic.
    #[test]
    fn a_bare_final_class_attribute_with_an_unfoldable_value_is_rejected() {
        assert_collision(
            "class C:\n    X: Final = 1j\n",
            "must be initialized with a literal",
        );
    }
}
