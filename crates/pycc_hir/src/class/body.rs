//! The class-body statement walk extracted from [`super::lower_class`]
//! (D-185: decompose the part a change touches).
//!
//! `lower_class` handles a class's *shape* -- decorators, bases, MRO
//! resolution, dataclass validation, and the synthesized `__init__`/`__eq__`/
//! `__repr__` of a `@dataclass`. This module owns the middle phase: the
//! single pass over `def.body` that classifies each statement, lowers every
//! method into a mangled `HirItem::Function`, and accumulates the tables
//! (`methods`, `attrs`, `properties`, `static_methods`, `class_methods`,
//! `dataclass_fields`, `abstract_methods`) that `lower_class` then folds into
//! the finished [`HirClassDef`].
//!
//! A plain (non-dataclass) body's value-less annotations are instance
//! attribute declarations (#1266): `super::declared_attrs` collects them
//! before the pass, the pass skips them, and `collect_init_attrs` takes their
//! declared types for the slots `__init__` establishes.
//!
//! Enum and protocol class bodies never reach this walk -- `lower_class`
//! returns through `lower_enum_class`/`lower_protocol_class` before it.

use super::attr_initializer::InitializerScope;
use super::attrs::{lower_class_attr, lower_unannotated_class_attr, strip_class_var};
use super::declared_attrs::{
    ClassMethodTables, collect_declared_attrs, instance_declaration_name,
    reject_unestablished_or_colliding,
};
use super::init_slot::DeclaredAttrs;
use super::reserved_names::reject_reserved_method_name;
use super::{
    ClassAnnotationInfo, ClassAttrValue, HirClassDef, MethodKind, PropertyDef, classify_decorator,
    collect_init_attrs, is_declaration_body, is_scalar_slot_type, lower_method,
};
use crate::expr::keyword_bind::SignatureTable;
use crate::{HirExpr, HirItem, ImportBinding, Ty, unsupported};
use pycc_ast::{Expr, Stmt};
use pycc_diag::{Diagnostic, Span};

/// The dunder names a `@dataclass` body may not bind as a `ClassVar` (#913).
///
/// The rule that generates this set, rather than the list itself, is what a
/// future change must re-apply: *the three methods pycc synthesizes for a
/// dataclass, plus every dunder CPython consults for an operation pycc
/// rewrites through one of those three.*
///
/// D-236 records that this rule is **not** the whole story: it cannot
/// generate `__new__` or `__init_subclass__`, which Python's instantiation
/// and class-creation protocols call implicitly without pycc rewriting
/// anything through them. Those two are owned by
/// [`super::reserved_names::reject_reserved_class_attr_name`], which applies
/// in *every* class body rather than only a dataclass one, so this set stays
/// exactly as D-235 accepted it. Keeping the two sets disjoint is deliberate:
/// this check runs before [`super::attrs::lower_class_attr`], so every
/// message below stays byte-for-byte what D-235 pinned, while the two names
/// it omits fall through to the universal guard.
///
/// - `__init__`, `__eq__`, `__repr__` -- synthesized unconditionally by
///   [`super::init::synthesize_dataclass_init`] and its siblings.
/// - `__ne__` -- `pycc_mir`'s `Eq`/`NotEq` rewrite spells `a != b` as
///   `not a.__eq__(b)` and never consults `__ne__`, which CPython tries
///   first.
/// - `__str__`, `__format__` -- `pycc_mir::class::rewrite_instance_to_repr`
///   rewrites both `print(instance)` and an f-string interpolation of one
///   straight to `__repr__`, while CPython routes them through `__str__` and
///   `__format__` respectively.
///
/// Verified against CPython 3.13.9: each of the six leaves the class
/// attribute in place, so the corresponding program raises
/// `TypeError: 'int' object is not callable` where pycc would silently
/// succeed. Dunders pycc does *not* rewrite for a dataclass stay off the
/// list: `__lt__` is already `T0021` ("cannot compare") before MIR, and
/// `__hash__` agrees with CPython because `dataclasses` leaves an explicitly
/// bound `__hash__` alone. `__slots__` is not here either -- it is rejected
/// earlier and with its own message by
/// [`super::reserved_names::reject_reserved_class_attr_name`].
const DATACLASS_IMPLICIT_DUNDERS: [&str; 6] = [
    "__init__",
    "__eq__",
    "__repr__",
    "__ne__",
    "__str__",
    "__format__",
];

/// The read-only inputs the class-body walk needs from [`super::lower_class`].
///
/// Grouped into a struct rather than passed positionally: the walk takes
/// eight distinct read-only inputs, well past the point where positional
/// arguments stop being readable (and past `clippy::too_many_arguments`).
pub(super) struct ClassBodyInput<'a> {
    /// The class body's statements (`def.body`).
    pub(super) body: &'a [Stmt],
    /// The class's own name, used for method mangling and diagnostics.
    pub(super) class_name: &'a str,
    /// Whether the class carries `@dataclass` (#378).
    pub(super) is_dataclass: bool,
    /// The PEP 695 type parameter name, if the class is generic.
    pub(super) type_param: Option<&'a str>,
    /// Module-level type aliases, for annotation resolution.
    pub(super) aliases: &'a [(String, Ty)],
    /// Class annotation info for every class visible here, including a
    /// self-entry for the class being lowered (PEP 560, #611, narrowed by
    /// #1130 -- see `lower_class`'s own comment at that push for what the
    /// self-entry is still for now that the subscriptability gate is gone).
    pub(super) class_name_defs: &'a [ClassAnnotationInfo],
    /// The class's resolved C3 MRO, most-derived first.
    pub(super) mro: &'a [String],
    /// Every class lowered before this one.
    pub(super) defined_classes: &'a [(String, HirClassDef)],
    /// The module's import table as it stands at this class (Part 1 of
    /// #883, #962), forwarded to every method body so an aliased stdlib
    /// receiver (`m.sqrt(x)` after `import math as m`) lowers there too.
    pub(super) imports: &'a [ImportBinding],
    /// The module's keyword-bindable signature table (Part 1 of #884,
    /// #1125), forwarded to every method body alongside `imports` so a
    /// keyword call to a module-level `def` binds inside a method exactly
    /// as it does at module scope. The compiler cannot flag a missing
    /// forward here -- an empty table would simply make every such call
    /// fall back to the old capability rejection -- so this field is
    /// covered by its own regression test.
    pub(super) signatures: &'a SignatureTable,
    /// Whether the module body binds `staticmethod` anywhere (#1345), so a
    /// class attribute `name = staticmethod(...)` is not the builtin.
    pub(super) staticmethod_rebound: bool,
}

/// The tables the class-body walk accumulates, handed back to
/// [`super::lower_class`] for the final [`HirClassDef`].
pub(super) struct ClassBodyOutput {
    /// `(source name, mangled name)` for every regular method.
    pub(super) methods: Vec<(String, String)>,
    /// The lowered `HirItem::Function` for every method in the body.
    pub(super) items: Vec<HirItem>,
    /// Instance attribute slots derived from `__init__` (D-154), each typed
    /// by its class-body declaration when one exists (#1266).
    pub(super) attrs: Vec<(String, Ty)>,
    /// `@property` getters (and their setters).
    pub(super) properties: Vec<PropertyDef>,
    /// `(source name, mangled name)` for every `@staticmethod`.
    pub(super) static_methods: Vec<(String, String)>,
    /// `(source name, mangled name)` for every `@classmethod`.
    pub(super) class_methods: Vec<(String, String)>,
    /// `@dataclass` fields declared in this body, in source order (#378).
    pub(super) dataclass_fields: Vec<(String, Ty)>,
    /// Names of `@abstractmethod`s declared in this body (#380).
    pub(super) abstract_methods: Vec<String>,
    /// Class-level attributes in source order, in either the annotated
    /// PEP 526 spelling (#911) or the bare `X = 1` one (#910).
    pub(super) class_attrs: Vec<(String, Ty, ClassAttrValue)>,
    /// `(mangled name, defaults)` for every method that declares a default
    /// value -- see `HirClassDef::method_defaults` (the method part of
    /// #1140).
    pub(super) method_defaults: Vec<(String, Vec<Option<HirExpr>>)>,
}

/// Walks a non-enum, non-protocol class body once, lowering every method and
/// accumulating the class's tables.
///
/// Returns the first `Diagnostic` any statement produces; a class body
/// statement that is not a `def`, a bare `pass`, or a docstring is `C0001`
/// (with the `@dataclass` `AnnAssign` carve-out, #378).
pub(super) fn walk_class_body(input: &ClassBodyInput<'_>) -> Result<ClassBodyOutput, Diagnostic> {
    let &ClassBodyInput {
        body,
        class_name,
        is_dataclass,
        type_param,
        aliases,
        class_name_defs,
        mro,
        defined_classes,
        imports,
        signatures,
        staticmethod_rebound,
    } = input;
    let mut methods: Vec<(String, String)> = Vec::new();
    let mut items: Vec<HirItem> = Vec::new();
    let mut attrs: Vec<(String, Ty)> = Vec::new();
    let mut properties: Vec<PropertyDef> = Vec::new();
    let mut static_methods: Vec<(String, String)> = Vec::new();
    let mut class_methods: Vec<(String, String)> = Vec::new();
    let mut dataclass_fields: Vec<(String, Ty)> = Vec::new();
    let mut abstract_methods: Vec<String> = Vec::new();
    let mut class_attrs: Vec<(String, Ty, ClassAttrValue)> = Vec::new();
    let mut method_defaults: Vec<(String, Vec<Option<HirExpr>>)> = Vec::new();
    let mut init_seen = false;
    // #1266: a value-less class-body annotation is an instance attribute
    // declaration. It is collected before the walk because it may follow
    // `def __init__`, whose pre-scan needs the full list; the walk then skips
    // every statement collected here. A `@dataclass` body gives the same
    // spelling its field meaning instead.
    let declared = if is_dataclass {
        Vec::new()
    } else {
        collect_declared_attrs(body, class_name, type_param, aliases, class_name_defs)?
    };
    for (index, stmt) in body.iter().enumerate() {
        // #1345: what a non-literal class-attribute initializer is
        // classified against, including the statements before this one.
        let scope = InitializerScope {
            imports,
            staticmethod_rebound,
            earlier: &body[..index],
        };
        // #378 (PR-18): a `@dataclass` class body accepts `AnnAssign`
        // (`x: int` or `x: int = default`) alongside method definitions.
        // An annotated field contributes to `dataclass_fields`. In a
        // non-dataclass body an `AnnAssign` is a class constant (#911) or,
        // value-less, an instance attribute declaration (#1266).
        if let Stmt::Pass(_) = stmt {
            // `pass` is a no-op in any class body (dataclass or not). A
            // zero-field dataclass (`@dataclass\nclass Empty:\n    pass`)
            // relies on this to have a valid body with no fields and no
            // methods.
            continue;
        }
        // #744: a docstring (a bare string-literal expression statement) is
        // a no-op, matching `validate_init_subclass_body`'s existing
        // precedent for the same construct.
        if let Stmt::Expr(expr_stmt) = stmt
            && matches!(*expr_stmt.value, Expr::StringLiteral(_))
        {
            continue;
        }
        // #1368: a `__slots__` binding (bare, or annotated with a value) is
        // not a class attribute; `super::slots::check_class` parses and
        // checks it once the class is lowered. A `@dataclass` body keeps
        // today's refusals.
        if !is_dataclass && super::slots::is_slots_binding(stmt) {
            continue;
        }
        if let Stmt::AnnAssign(ann) = stmt {
            // #911: `ClassVar[T]` is a class-body-only annotation wrapper.
            // The shared `annotation_to_ty` rejects it outright, so it is
            // stripped here -- the one position where it is legal -- before
            // the annotation is resolved.
            let stripped = strip_class_var(&ann.annotation)?;
            let is_class_var = stripped.is_class_var;
            if !is_dataclass {
                // #1266: already collected by `collect_declared_attrs`, with
                // the same predicate, so the two cannot drift.
                if instance_declaration_name(ann).is_some() {
                    continue;
                }
                // PEP 526 (#911, Part 1 of #885): an *annotated* class-level
                // attribute with a literal initializer is a compile-time
                // constant. An un-annotated one (`X = 1`, `Stmt::Assign`) is
                // Part 2 (#910) and still falls through to the catch-all
                // below.
                class_attrs.push(lower_class_attr(
                    ann,
                    stripped,
                    class_name,
                    type_param,
                    aliases,
                    class_name_defs,
                    &class_attrs,
                    &scope,
                )?);
                continue;
            }
            // #913: PEP 557 says a `ClassVar`-annotated name in a
            // `@dataclass` body is *not* a dataclass field, so it takes the
            // very same #911 class-attribute route the non-dataclass branch
            // above takes. Because it never enters `dataclass_fields`, it is
            // absent from the merged field list `super::lower_class` builds
            // -- and therefore from the synthesized `__init__`, `__eq__` and
            // `__repr__`, and from the D-154 instance-slot layout, which is
            // that same merged list. #911 rejected the spelling outright
            // because merely stripping the wrapper would have turned it into
            // a *required* `__init__` parameter; routing instead of stripping
            // is what makes that objection moot.
            //
            // Two guards keep the routed form honest. The first is here: a
            // `ClassVar` named after a dunder the dataclass relies on
            // implicitly cannot be modelled. CPython's `dataclasses` uses
            // `_set_new_attribute`, which does not overwrite a name already
            // in the class `__dict__`, so `__repr__: ClassVar[int] = 8`
            // really does leave `A.__repr__ == 8` and
            // `__init__: ClassVar[int] = 8` leaves the class with no
            // synthesized constructor at all. pycc synthesizes those three
            // unconditionally, and `reject_class_attr_collisions` runs before
            // synthesis pushes them into `methods`, so it cannot see the
            // clash either. The set is `DATACLASS_IMPLICIT_DUNDERS`, whose own
            // doc comment states the rule that generates it. The second guard
            // is the field/class-attribute name check in `super::lower_class`,
            // which runs after the field merge.
            if is_class_var
                && let Expr::Name(target_name) = ann.target.as_ref()
                && DATACLASS_IMPLICIT_DUNDERS.contains(&target_name.id.as_str())
            {
                return Err(unsupported(
                    format!(
                        "a `@dataclass` class implicitly relies on `{name}`; a `ClassVar` named \
                         `{name}` is not allowed in a `@dataclass` body -- CPython would keep \
                         the class attribute, so the class would either lose the synthesized \
                         method or fail at the use site, which this version does not model",
                        name = target_name.id.as_str()
                    ),
                    ann.range,
                ));
            }
            if is_class_var {
                class_attrs.push(lower_class_attr(
                    ann,
                    stripped,
                    class_name,
                    type_param,
                    aliases,
                    class_name_defs,
                    &class_attrs,
                    &scope,
                )?);
                continue;
            }
            // The target must be a single bare name.
            let Expr::Name(target_name) = ann.target.as_ref() else {
                return Err(unsupported(
                    "a dataclass field annotation must target a bare name (`x: int`), not an \
                     attribute access, subscript, or other expression",
                    pycc_ast::expr_range(&ann.target),
                ));
            };
            let field_name = target_name.id.to_string();
            // Reject duplicate field names.
            if dataclass_fields.iter().any(|(name, _)| name == &field_name) {
                return Err(unsupported(
                    format!(
                        "dataclass field `{field_name}` is already defined in class \
                         `{class_name}` -- duplicate field names are not allowed"
                    ),
                    ann.range,
                ));
            }
            let field_ty = crate::annotation_to_ty(
                &ann.annotation,
                type_param,
                Some(class_name),
                aliases,
                class_name_defs,
            )?;
            // #378 (PR-18): a dataclass field's type must be a scalar slot
            // type (int/float/bool/str, or a generic type parameter `T`
            // that is substituted with a scalar at monomorphization time).
            // The instance attribute-slot storage is a single `i64` word
            // per slot (D-154). A container-typed field (`list[T]`,
            // `dict[K, V]`, `set[T]`) is not admitted as a dataclass field,
            // and there is no slot representation at all for a by-value
            // `tuple[...]` or `None`. A class instance (`Ty::Instance`,
            // including a self-referential field like `next: Node` or
            // `next: Self`, which `annotation_to_ty` resolves to
            // `Ty::Instance` -- see its self-referential class name and
            // `Self` arms) has a pointer-word slot since #1389, but the
            // dataclass's synthesized `__eq__`/`__repr__` do not cover it.
            // Rejecting here, structurally, keeps every field type the
            // synthesized methods actually implement. A hand-written
            // `__init__` may seed a `list[int]`/`dict[str, int]` (#1262) or
            // class-instance (#1389) slot from a parameter
            // (`init_slot::slot_ty_from_init_rhs`); a container or instance
            // dataclass field is a separate follow-up (synthesized
            // `__eq__`/`__repr__` over those types), so it stays refused here.
            if !is_scalar_slot_type(&field_ty) {
                return Err(unsupported(
                    format!(
                        "dataclass field `{field_name}` has type `{}`, which is not a scalar \
                         slot type -- only `int`, `float`, `bool`, `str`, or a generic type \
                         parameter is supported as a dataclass field in this version (the \
                         instance attribute-slot storage is a single word per slot; a \
                         container-typed or class-instance dataclass field is not supported \
                         yet, since the synthesized `__eq__`/`__repr__` do not cover it, and \
                         a tuple or `None` has no slot representation)",
                        field_ty.name()
                    ),
                    ann.range,
                ));
            }
            // A field with a default value (`x: int = field(default=...)` or
            // `x: int = 42`) is recognized but rejected with C0001 -- field
            // defaults are deferred to a follow-up issue (the compiler has no
            // optional-parameter mechanism yet). A bare `field()` call with
            // no arguments is also rejected (a field with `field()` and no
            // default is meaningless).
            if let Some(value) = &ann.value {
                // Recognize `field(...)` call shapes specifically, for a
                // clearer diagnostic message.
                if let Expr::Call(call) = value.as_ref()
                    && let Expr::Name(name) = call.func.as_ref()
                    && name.id.as_str() == "field"
                {
                    return Err(unsupported(
                        "dataclass field defaults are not supported yet -- only required \
                         fields are supported in this version (`field(default=...)` and \
                         `field(default_factory=...)` are deferred to a follow-up issue)",
                        ann.range,
                    ));
                }
                return Err(unsupported(
                    "dataclass field defaults are not supported yet -- only required fields \
                     (no default value) are supported in this version",
                    ann.range,
                ));
            }
            dataclass_fields.push((field_name, field_ty));
            continue;
        }
        // #910 (Part 2 of #885): an un-annotated assignment with a literal
        // right-hand side is a class attribute whose type is inferred from
        // that literal. A `@dataclass` body is excluded deliberately: a bare
        // `x = 1` there is a class-level default for a field declared
        // elsewhere in Python's own model, not a constant, so it falls
        // through to the catch-all below and stays `C0001` (#378).
        if let Stmt::Assign(assign) = stmt
            && !is_dataclass
        {
            class_attrs.push(lower_unannotated_class_attr(
                assign,
                class_name,
                &class_attrs,
                &scope,
            )?);
            continue;
        }
        // #1244: a class-body `del` would unbind a class attribute, which the
        // class model has no notion of; it gets its own message rather than
        // the generic one below.
        if let Stmt::Delete(_) = stmt {
            return Err(unsupported(
                "a `del` statement in a class body is not supported yet",
                pycc_ast::stmt_range(stmt),
            ));
        }
        let Stmt::FunctionDef(method_def) = stmt else {
            // #910 reworded this, and split it in two. A bare assignment is
            // now an accepted class-level attribute -- but *only* outside a
            // `@dataclass`, where it still falls through to here. A single
            // wording would therefore have to be wrong for one of the two
            // callers, so each states what its own body actually accepts.
            return Err(unsupported(
                if is_dataclass {
                    "a `@dataclass` body statement must be a field declaration (`x: int`) or a \
                     method definition (`def ...`) -- no other statement kind is supported yet"
                } else {
                    "a class body statement must be a method definition (`def ...`) or a \
                     class-level attribute assignment (`X = 1`, `X: int = 1`) -- no other \
                     statement kind is supported yet"
                },
                pycc_ast::stmt_range(stmt),
            ));
        };
        let method_name = method_def.name.as_str().to_string();
        // #386: `__init__` redefinition stays C0001 -- the compile-time
        // attribute-slot pre-scan (`collect_init_attrs`) derives slot types
        // from the first `__init__` body's assignments and cannot reconcile
        // a second, different `__init__` body. A non-`__init__` method
        // redefinition is a rebind, not an error (see below).
        if method_name == "__init__" && init_seen {
            return Err(unsupported(
                "redefining `__init__` in the same class body is not supported yet \
                 -- the attribute-slot pre-scan cannot reconcile two different \
                 `__init__` bodies",
                method_def.range,
            ));
        }
        // #378 (PR-18): a `@dataclass` class auto-generates `__init__`,
        // `__eq__`, and `__repr__` -- an explicit definition of any of these
        // is rejected with C0001 (the synthesized method replaces it).
        if is_dataclass && matches!(method_name.as_str(), "__init__" | "__eq__" | "__repr__") {
            return Err(unsupported(
                format!(
                    "a `@dataclass` class auto-generates `{method_name}`; an explicit \
                     `{method_name}` is not allowed in a `@dataclass` body"
                ),
                method_def.range,
            ));
        }
        // #377: classify the method's decorator list to determine whether
        // it is a regular method, a `@property` getter, or a
        // `@<name>.setter` setter. `lower_method` uses this to compute the
        // correct mangled name (a setter uses a `.setter` suffix to avoid
        // colliding with the getter's mangled name, since both share the
        // same source method name).
        let kind = classify_decorator(
            &method_def.decorator_list,
            &method_name,
            method_def.range.into(),
        )?;
        // #975 (D-236), added in the #978 review round: a `@property`
        // getter is the fourth class-body route to a class-level binding of
        // an instantiation/class-creation protocol name, and the only one
        // that does not pass through `reject_reserved_class_attr_name`.
        // `@property def __new__(self) -> int` lands here, not in
        // `lower_class_attr`, so without this call `ensure_init` synthesizes
        // a constructor from the method table alone and pycc accepts `C()`
        // while CPython 3.13.9 raises `TypeError: 'property' object is not
        // callable`. A plain `def __init__` is a `MethodKind::Regular` and is
        // untouched -- only the property spelling is rejected. The setter arm
        // needs no matching call: `classify_decorator` requires a setter's own
        // `def` name to equal the decorated property name, so
        // `@value.setter def __new__` is rejected there as a mismatch, and the
        // matching `@__new__.setter def __new__` requires a preceding getter
        // of that name, which is rejected here first.
        //
        // #980: the same call also rejects `@property def __slots__`, under
        // its own message rather than the attribute route's. `type.__new__`
        // iterates `__slots__` while the `class` statement executes, so
        // CPython 3.13.9 raises `TypeError: 'property' object is not
        // iterable` and never creates the class -- on a plain and on a
        // `@dataclass` body alike, since both reach this one arm. The
        // dataclass pre-check above matches only `__init__`/`__eq__`/
        // `__repr__`, so `__slots__` falls through to here; an `Enum` body
        // never does, because `lower_enum_class` rejects a method definition
        // outright before this point.
        //
        // #984: the same one call now covers every *non*-`@property` spelling
        // of `def __slots__` too -- a bare `def`, `@override`,
        // `@abstractmethod`, `@staticmethod` and `@classmethod` -- which all
        // diverged identically and were all accepted here before. The
        // dispatch moved into `reject_reserved_method_name`; the getter arm's
        // behavior is unchanged. Three things about the position matter:
        // `classify_decorator` still runs first, so an unrecognized decorator
        // keeps reporting "method decorators are not supported yet"; the
        // instantiation-protocol half stays gated to the getter, so a plain
        // `def __new__` is still accepted (#981); and a `@__slots__.setter`
        // is short-circuited inside the guard, because its own "requires a
        // preceding `@property` getter" rejection below is already the
        // truthful account for that shape.
        //
        // Running before `lower_method` also preempts that function's own
        // early rejections for an `async def`, a generic `def __slots__[T]`,
        // `*args`, keyword-only parameters and `**kwargs`. That is
        // deliberate and stays truthful: CPython binds `__slots__` to a
        // plain `function` object for every one of those spellings too, so
        // `method_slots_message("function")` is the accurate account in each
        // case and the more specific "not supported yet" message it replaces
        // would have described a program CPython never gets to run.
        reject_reserved_method_name(&method_name, &kind, method_def.range.into())?;
        // #436: `@staticmethod` and `@classmethod` on `__init__` are
        // rejected -- a constructor must be a regular instance method.
        // #380 (PR-20): `@abstractmethod` on `__init__` is also rejected
        // -- an abstract `__init__` would prevent instantiation of any
        // subclass, which is not a meaningful pattern in pycc's
        // compile-time-only ABC model.
        if method_name == "__init__"
            && matches!(
                kind,
                MethodKind::StaticMethod | MethodKind::ClassMethod | MethodKind::AbstractMethod
            )
        {
            return Err(unsupported(
                "`@staticmethod`, `@classmethod`, and `@abstractmethod` cannot decorate \
                 `__init__` -- the constructor must be a regular instance method",
                method_def.range,
            ));
        }
        let (item, params, (name, defaults)) = lower_method(
            method_def,
            class_name,
            type_param,
            aliases,
            &kind,
            class_name_defs,
            imports,
            signatures,
        )?;
        // The method part of #1140: a redefinition replaces the earlier
        // entry -- or removes it, when the later `def` declares no default --
        // so the table always describes the definition that wins.
        method_defaults.retain(|(held, _)| *held != name);
        if !defaults.is_empty() {
            method_defaults.push((name, defaults));
        }
        if method_name == "__init__" {
            init_seen = true;
            // #1181: `collect_init_attrs` tests the body's receiver
            // expression against the receiver's *source* spelling, which
            // `params[0].0` no longer carries (it is always the canonical
            // `self`). Derive it here, where `method_def` is already in
            // scope, rather than widening `lower_method`'s return type.
            // `lower_method` has already run and accepted this `__init__`,
            // so it declares at least one parameter.
            let receiver_split =
                super::receiver::split_receiver(&method_def.parameters, method_def.range.into())?;
            attrs = collect_init_attrs(
                &method_def.body,
                &params,
                receiver_split.name(),
                &DeclaredAttrs {
                    attrs: &declared,
                    class_name,
                    generic: type_param.is_some(),
                },
                &|annotation| {
                    crate::annotation_to_ty(
                        annotation,
                        type_param,
                        Some(class_name),
                        aliases,
                        class_name_defs,
                    )
                },
            )?;
        }
        match &kind {
            MethodKind::Regular { is_override } => {
                // #432: if `@override` is present, verify the method name
                // exists in at least one base class's methods or
                // properties (walking the MRO, excluding the current class
                // itself). If no matching base method exists, emit T0031.
                if *is_override {
                    let found_in_base = mro.iter().skip(1).any(|mro_class| {
                        // Every class in the MRO (except the first, which is
                        // `class_name` itself and is skipped) was placed there
                        // by `compute_c3_mro`, which only references classes
                        // from `defined_classes` -- so this lookup always
                        // succeeds. Using `.expect()` (whose panic path lives
                        // in libcore, outside this crate's instrumented
                        // regions) instead of a `let .. else { return false
                        // }` avoids a permanently-uncovered else branch under
                        // D-014's 100%-region coverage gate.
                        let (_, base_def) = defined_classes
                            .iter()
                            .find(|(name, _)| name == mro_class)
                            .expect("every class in the MRO must be in defined_classes");
                        base_def
                            .methods
                            .iter()
                            .any(|(name, _)| name == &method_name)
                            || base_def.properties.iter().any(|p| p.name == method_name)
                    });
                    if !found_in_base {
                        return Err(Diagnostic::error(
                            "T0031",
                            format!(
                                "`@override` on method `{class_name}.{method_name}` does not \
                                 override any method or property of the same name in a base \
                                 class"
                            ),
                            Span::new(
                                u32::from(method_def.range.start()),
                                u32::from(method_def.range.end()),
                            ),
                        ));
                    }
                }
                let mangled = format!("{class_name}.{method_name}");
                // #377: reject a regular method whose name collides with an
                // existing property. Both would share the same `<Class>.<name>`
                // mangled symbol, and the stale method table entry would let
                // `obj.name()` (method-call syntax) resolve to the property
                // getter function — silently accepting a call shape that
                // CPython rejects after the property shadows the method.
                if properties.iter().any(|p| p.name == method_name) {
                    return Err(unsupported(
                        format!(
                            "a `@property` named `{method_name}` is already defined in this \
                             class -- a method cannot shadow a property of the same name"
                        ),
                        method_def.range,
                    ));
                }
                // #436: reject a regular method whose name collides with an
                // existing static or class method. Although the mangled
                // names differ (`.static`/`.classmethod` suffix), allowing
                // both would be confusing — the method-call syntax
                // `obj.name()` would resolve to the regular method while
                // `ClassName.name()` would resolve to the static/class
                // method, with no clear indication to the user that these
                // are different functions.
                if static_methods.iter().any(|(name, _)| name == &method_name) {
                    return Err(unsupported(
                        format!(
                            "a `@staticmethod` named `{method_name}` is already defined in \
                             this class -- a regular method cannot share a name with a \
                             `@staticmethod`"
                        ),
                        method_def.range,
                    ));
                }
                if class_methods.iter().any(|(name, _)| name == &method_name) {
                    return Err(unsupported(
                        format!(
                            "a `@classmethod` named `{method_name}` is already defined in \
                             this class -- a regular method cannot share a name with a \
                             `@classmethod`"
                        ),
                        method_def.range,
                    ));
                }
                // #386: rebind semantics for non-`__init__` method
                // redefinition. Both definitions share the same mangled
                // `<ClassName>.<method>` name, so PR #358's function-
                // pointer slot infrastructure already handles the actual
                // rebind at the codegen level (the second `def`'s source-
                // order execution stores the new function's address into
                // the slot). Here, replacing the method table entry on
                // redefinition rather than appending a duplicate keeps the
                // table clean -- the mangled name is the same either way,
                // so `resolve_method_call` and MIR lowering's
                // `.methods.iter().find(..)` resolve identically.
                if let Some(entry) = methods.iter_mut().find(|(name, _)| name == &method_name) {
                    *entry = (method_name.clone(), mangled.clone());
                } else {
                    methods.push((method_name.clone(), mangled));
                }
            }
            // #377: a `@property` getter. The getter's mangled name is
            // `<Class>.<name>` (the same scheme a regular method uses),
            // but it is NOT entered into `methods` -- it is accessed via
            // attribute syntax (`obj.x`), not method-call syntax
            // (`obj.x()`). A duplicate getter for the same property name
            // is rejected (a property is defined once, not rebound).
            MethodKind::PropertyGetter { prop_name } => {
                // #377: reject a property getter whose name collides with an
                // existing method. Both would share the same `<Class>.<name>`
                // mangled symbol, and the method table entry would let
                // `obj.name()` (method-call syntax) resolve to the property
                // getter function — silently accepting a call shape that
                // CPython rejects after the property shadows the method.
                if methods.iter().any(|(name, _)| name == prop_name) {
                    return Err(unsupported(
                        format!(
                            "a method named `{prop_name}` is already defined in this class -- \
                             a `@property` getter cannot shadow a method of the same name"
                        ),
                        method_def.range,
                    ));
                }
                if properties.iter().any(|p| &p.name == prop_name) {
                    return Err(unsupported(
                        format!(
                            "a `@property` getter for `{prop_name}` is already defined in \
                             this class -- redefining a property getter is not supported yet"
                        ),
                        method_def.range,
                    ));
                }
                properties.push(PropertyDef {
                    name: prop_name.clone(),
                    getter: format!("{class_name}.{prop_name}"),
                    setter: None,
                });
            }
            // #377: a `@<name>.setter` setter. The setter's mangled name
            // is `<Class>.<name>.setter` (the `.setter` suffix ensures it
            // cannot collide with the getter's `<Class>.<name>` mangled
            // name, since a real Python identifier can never contain a
            // `.`). The property's getter must already be defined (a
            // setter without a preceding getter is `C0001`), and a
            // duplicate setter for the same property is rejected.
            MethodKind::PropertySetter { prop_name } => {
                let Some(prop) = properties.iter_mut().find(|p| &p.name == prop_name) else {
                    return Err(unsupported(
                        format!(
                            "a `@{prop_name}.setter` decorator requires a preceding \
                             `@property` getter for `{prop_name}` in the same class"
                        ),
                        method_def.range,
                    ));
                };
                if prop.setter.is_some() {
                    return Err(unsupported(
                        format!(
                            "a setter for property `{prop_name}` is already defined in \
                             this class -- redefining a property setter is not supported yet"
                        ),
                        method_def.range,
                    ));
                }
                prop.setter = Some(format!("{class_name}.{prop_name}.setter"));
            }
            // #436: a `@staticmethod`. Registered in `static_methods`
            // (not `methods`) with a `.static` suffix mangled name. A
            // duplicate static method name is a rebind, matching regular
            // method rebind semantics (#386). A static method name must
            // not collide with a regular method, property, or class
            // method of the same name in the same class.
            MethodKind::StaticMethod => {
                if methods.iter().any(|(name, _)| name == &method_name) {
                    return Err(unsupported(
                        format!(
                            "a method named `{method_name}` is already defined in this class \
                             -- a `@staticmethod` cannot share a name with a regular method"
                        ),
                        method_def.range,
                    ));
                }
                if properties.iter().any(|p| p.name == method_name) {
                    return Err(unsupported(
                        format!(
                            "a `@property` named `{method_name}` is already defined in this \
                             class -- a `@staticmethod` cannot share a name with a property"
                        ),
                        method_def.range,
                    ));
                }
                if class_methods.iter().any(|(name, _)| name == &method_name) {
                    return Err(unsupported(
                        format!(
                            "a `@classmethod` named `{method_name}` is already defined in \
                             this class -- a `@staticmethod` cannot share a name with a \
                             `@classmethod`"
                        ),
                        method_def.range,
                    ));
                }
                let mangled = format!("{class_name}.{method_name}.static");
                if let Some(entry) = static_methods
                    .iter_mut()
                    .find(|(name, _)| name == &method_name)
                {
                    *entry = (method_name.clone(), mangled.clone());
                } else {
                    static_methods.push((method_name.clone(), mangled));
                }
            }
            // #436: a `@classmethod`. Registered in `class_methods`
            // (not `methods`) with a `.classmethod` suffix mangled name.
            // A duplicate class method name is a rebind, matching regular
            // method rebind semantics (#386). A class method name must
            // not collide with a regular method, property, or static
            // method of the same name in the same class.
            MethodKind::ClassMethod => {
                if methods.iter().any(|(name, _)| name == &method_name) {
                    return Err(unsupported(
                        format!(
                            "a method named `{method_name}` is already defined in this class \
                             -- a `@classmethod` cannot share a name with a regular method"
                        ),
                        method_def.range,
                    ));
                }
                if properties.iter().any(|p| p.name == method_name) {
                    return Err(unsupported(
                        format!(
                            "a `@property` named `{method_name}` is already defined in this \
                             class -- a `@classmethod` cannot share a name with a property"
                        ),
                        method_def.range,
                    ));
                }
                if static_methods.iter().any(|(name, _)| name == &method_name) {
                    return Err(unsupported(
                        format!(
                            "a `@staticmethod` named `{method_name}` is already defined in \
                             this class -- a `@classmethod` cannot share a name with a \
                             `@staticmethod`"
                        ),
                        method_def.range,
                    ));
                }
                let mangled = format!("{class_name}.{method_name}.classmethod");
                if let Some(entry) = class_methods
                    .iter_mut()
                    .find(|(name, _)| name == &method_name)
                {
                    *entry = (method_name.clone(), mangled.clone());
                } else {
                    class_methods.push((method_name.clone(), mangled));
                }
            }
            // #380 (PR-20, PEP 3119): an `@abstractmethod`. Registered
            // in `methods` (it is still a regular method for dispatch
            // purposes — a subclass overrides it with a regular method of
            // the same name) AND in `abstract_methods` (so `lower_class`
            // can verify concrete subclasses override every inherited
            // abstract method). The method body must be declaration-style
            // (`...` or `pass`).
            MethodKind::AbstractMethod => {
                // Verify the method body is declaration-style (`...` or
                // `pass`). A non-declaration body is rejected with C0001
                // — an abstract method with an implementation is a
                // contradiction in pycc's compile-time-only ABC model.
                if !is_declaration_body(&method_def.body) {
                    return Err(unsupported(
                        format!(
                            "an `@abstractmethod` `{class_name}.{method_name}` must have a \
                             declaration-style body (`...` or `pass`), not an implementation"
                        ),
                        method_def.range,
                    ));
                }
                let mangled = format!("{class_name}.{method_name}");
                if let Some(entry) = methods.iter_mut().find(|(name, _)| name == &method_name) {
                    *entry = (method_name.clone(), mangled.clone());
                } else {
                    methods.push((method_name.clone(), mangled));
                }
                abstract_methods.push(method_name.clone());
            }
        }
        items.push(item);
    }
    // #1266: every declaration must be established by the own `__init__`
    // (an empty `attrs` when there is none) and must not name a method.
    reject_unestablished_or_colliding(
        &declared,
        &attrs,
        &ClassMethodTables {
            methods: &methods,
            properties: &properties,
            static_methods: &static_methods,
            class_methods: &class_methods,
        },
        class_name,
    )?;
    Ok(ClassBodyOutput {
        methods,
        items,
        attrs,
        properties,
        static_methods,
        class_methods,
        dataclass_fields,
        abstract_methods,
        class_attrs,
        method_defaults,
    })
}

#[cfg(test)]
#[path = "body_tests.rs"]
mod tests;
