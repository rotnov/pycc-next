//! Annotation-to-`Ty` conversion ([`annotation_to_ty`]) and the module
//! markers it reads, extracted from `func.rs` per AGENTS.md's
//! file-decomposition rule when the whole-package build added sibling
//! classes to its name resolution.

use super::bare_container::container_family;
use super::container_annotation::container_annotation_to_ty;
use super::string_annotation::string_annotation_to_ty;
use super::{buffer_spelling, callable_shape, union};
use crate::class::ClassAnnotationInfo;
use crate::{Ty, unsupported};
use pycc_ast::{Expr, Operator};
use pycc_diag::{Diagnostic, Span};

/// The six names [`annotation_to_ty`]'s `Expr::Name` arm answers *before* it
/// consults `class_defs` or the alias table **and that the `Expr::Subscript`
/// arm can reach**. Canonical statement of the precedence rule D-244
/// statements (h) and (k) describe; every site that dispatches on it derives
/// from here rather than restating the list. `ClassVar` is answered before
/// both tables too but is deliberately absent: the subscript arm intercepts
/// `ClassVar[...]` at the top of its own match, so it never reaches this
/// helper. The carrier's two ordinary-identifier spellings `ndarray` (#1129)
/// and `NDArray` (#1134) are deliberately absent too -- both are ordinary
/// identifiers resolved *after* both tables (D-244 statement (h)). The
/// legacy `typing` container aliases `Dict`/`List`/`Set`/`FrozenSet`/`Tuple`
/// (#1378) are deliberately absent as well: like the builtin `dict`/`list`
/// they alias, they are lowered only after the known-class ladder, so a user
/// class named `List` wins over the typing form.
fn name_resolves_before_class_defs(base: &str) -> bool {
    matches!(
        base,
        "int" | "float" | "bool" | "str" | "Any" | "memoryview"
    )
}

/// The alias-table entry that marks a module compiled into an `ext`
/// artifact (D-258, #1397).
///
/// D-258 rule 6 records that `pycc_hir` has no artifact-mode awareness of
/// its own, and that unconditional lowering with a mode-aware refusal
/// elsewhere is one admissible shape. It is not the shape used here:
/// lowering `Any` unconditionally would change `native` output beyond the
/// `T0002` itself (`list[Any]` would reach `T0034`, `Any[int]` would be
/// accepted), so the mode is told to the lowering instead, which rule 6's
/// "any other shape that keeps `native` output byte-identical" admits. The
/// driver sets [`crate::ResolvedImports::set_ext_module`], and
/// `module::lower_module` records this entry in the alias table every
/// [`annotation_to_ty`] call site already receives, so no call site needs a
/// new parameter. The name is not a Python identifier, so no source name
/// can ever look it up, and it is stripped before the `HirModule` is built.
pub(crate) const EXT_MODULE_MARKER: &str = "<ext module>";

/// The alias-table entry that marks a module lowered in non-strict mode
/// (#3), recorded and stripped exactly like [`EXT_MODULE_MARKER`].
pub(crate) const NON_STRICT_MARKER: &str = "<non-strict module>";

/// Whether `aliases` carries [`NON_STRICT_MARKER`].
pub(crate) fn is_non_strict(aliases: &[(String, Ty)]) -> bool {
    aliases.iter().any(|(name, _)| name == NON_STRICT_MARKER)
}

/// Whether `Any`, `object` and an object container spell `Ty::Object`: in
/// an `ext` module (D-258) and in non-strict mode (#3).
pub(crate) fn objects_spellable(aliases: &[(String, Ty)]) -> bool {
    is_ext_module(aliases) || is_non_strict(aliases)
}

/// Whether `aliases` carries [`EXT_MODULE_MARKER`], that is, whether the
/// annotation being lowered belongs to an `ext` module (D-258 rule 1).
pub(crate) fn is_ext_module(aliases: &[(String, Ty)]) -> bool {
    aliases.iter().any(|(name, _)| name == EXT_MODULE_MARKER)
}

/// The four bare container spellings D-258 rule 4 makes the opaque object in
/// an `ext` module. Exactly the four the decision names: a bare `frozenset`,
/// and the bare legacy `typing` aliases (`List`, `Dict`, ...), keep their
/// `C0001` everywhere.
fn is_ext_object_container(name: &str) -> bool {
    matches!(name, "list" | "dict" | "tuple" | "set")
}

/// The noun for [`annotation_to_ty`]'s non-class `T0044` (#931): what the
/// base of a subscripted annotation resolved to, when it is neither a class
/// nor an alias to one. The arms follow the **same precedence the
/// `Expr::Name` arm resolves a bare name in**, so the noun always agrees with
/// what the recursion that precedes the reject actually resolved: a type
/// parameter first, then `Self` inside a class, then the enclosing class's
/// own name, then the builtin scalars, and otherwise a `type` alias. Only the
/// *precedence* is mirrored, not the resolved `Ty`: since #948 a self-reference
/// inside a protocol body resolves to `Ty::Protocol` rather than
/// `Ty::Instance`, but the recursion's `Ok` value is discarded here and the
/// noun is keyed on the spelling alone, so the wording is unaffected.
///
/// The `class_name` arm is defensive: every current caller that passes
/// `class_name` also has that class in `class_defs` (`lower_class` pushes the
/// self-referential entry before lowering the body), so the known-class
/// ladder catches it first. The arm exists so a future caller cannot make
/// the noun say "type alias" for a class.
pub(crate) fn subscripted_base_description(
    base: &str,
    type_param: Option<&str>,
    class_name: Option<&str>,
) -> String {
    if Some(base) == type_param {
        format!("type parameter `{base}`")
    } else if base == "Self" && class_name.is_some() {
        "`Self`".to_string()
    } else if Some(base) == class_name {
        format!("class `{base}`")
    } else {
        match base {
            "int" | "float" | "bool" | "str" => format!("builtin type `{base}`"),
            // Reached only in an `ext` module (D-258, #1397): a `native`
            // build answers `Any[...]` with `T0002` before any noun is
            // needed. `Any` is reserved before both tables, so no alias of
            // that name can make it mean anything else.
            "Any" => "`Any`".to_string(),
            _ => format!("type alias `{base}`"),
        }
    }
}

/// The type an annotation naming the enclosing class resolves to.
///
/// Both self-reference spellings -- PEP 673 `Self` and PEP 649/749's bare use
/// of the class's own name ([#387](https://github.com/rotnov/pycc/issues/387))
/// -- denote the enclosing class, so both go through this helper. A protocol
/// enclosing class yields `Ty::Protocol`, exactly as the cross-class
/// `class_defs` lookup in `annotation_to_ty`'s general `Expr::Name` arm does
/// for any other protocol name ([#948](https://github.com/rotnov/pycc/issues/948));
/// every other class yields `Ty::Instance`, the type `self` has.
fn enclosing_class_ty(class_name: &str, class_defs: &[ClassAnnotationInfo]) -> Ty {
    if class_defs
        .iter()
        .any(|info| info.name == class_name && info.is_protocol)
    {
        return Ty::Protocol(Box::new(class_name.to_string()));
    }
    Ty::Instance(Box::new(class_name.to_string()))
}

/// Resolves an annotation expression to a `Ty`. `aliases` is the D-135 type
/// alias table (`(name, Ty)` pairs recorded by `module::lower_all` for every
/// `type X = ...`/legacy `X: TypeAlias = ...` statement reached so far, in
/// source order): checked as the last resort for a bare name before falling
/// through to the `C0001` "not supported yet" catch-all, so an alias name
/// resolves exactly like any other recognized bare-name annotation.
///
/// `class_name` is the enclosing class's name when lowering a method's
/// annotations (PEP 673 `Self` and PEP 649/749 self-referential deferred
/// annotations, #387): `Some(name)` makes both `"Self"` and the class's own
/// name resolve through `enclosing_class_ty` — `Ty::Instance(Box::new(name))`
/// for an ordinary class, the type `self` has, and `Ty::Protocol` when the
/// enclosing class is itself a protocol (#948). `None` for top-level functions
/// and all other annotation contexts
/// (module-level `AnnAssign`, type aliases), where `"Self"` and a bare class
/// name remain unrecognized (C0001), matching CPython's own scope rule that
/// `Self` is only valid inside a class body.
///
/// `class_defs` is the projected slice of already-defined classes
/// (#380, PR-20): a bare name matching a known class resolves to
/// `Ty::Instance` (or `Ty::Protocol` if the class is a protocol). This
/// fixes the pre-existing gap where cross-class annotations
/// (`def f(a: A) -> int` where `A` is a user-defined class) produced `C0001`.
/// Checked after the builtin/`Self`/self-referential arms and before the
/// alias table, so a class name takes priority over an alias of the same
/// name (matching Python's own scope rule where class definitions bind the
/// class name in the enclosing namespace).
/// Whether a `|` annotation is `int | float` or `float | int`: the one
/// general union compiled natively (`Ty::Num`).
pub(super) fn is_int_float_union_names(left: &Expr, right: &Expr) -> bool {
    let name = |expr: &Expr| match expr {
        Expr::Name(name) => Some(name.id.to_string()),
        _ => None,
    };
    matches!(
        (name(left).as_deref(), name(right).as_deref()),
        (Some("int"), Some("float")) | (Some("float"), Some("int"))
    )
}

pub(crate) fn annotation_to_ty(
    annotation: &Expr,
    type_param: Option<&str>,
    class_name: Option<&str>,
    aliases: &[(String, Ty)],
    class_defs: &[ClassAnnotationInfo],
) -> Result<Ty, Diagnostic> {
    match annotation {
        Expr::NoneLiteral(_) => Ok(Ty::None),
        Expr::Name(name) if Some(name.id.as_str()) == type_param => {
            Ok(Ty::Param(Box::new(name.id.to_string())))
        }
        // PEP 673 (#387): `Self` inside a class method's annotation resolves
        // to the enclosing class's own type — `Ty::Instance`, the type `self`
        // has, for an ordinary class. Inside a *protocol* body it resolves to
        // `Ty::Protocol` instead (#948): `self` itself stays `Ty::Instance`
        // there, but the annotation denotes the protocol, so the two diverge
        // and a `-> Self` member return reaches #934's `C0001` gate exactly as
        // the bare `-> P` spelling does. Outside a class (`class_name` is
        // `None`), `"Self"` falls through to the alias/C0001 path below,
        // matching CPython's own scoping rule.
        // #911 (Part 1 of #885): `ClassVar` is a *class-body-only* annotation
        // wrapper -- PEP 526 defines it as "this name is a class variable,
        // not an instance one", which is meaningless on a parameter, a
        // return type, or a local `AnnAssign`. It is deliberately **not**
        // unwrapped here the way `Final`/`Annotated` are: silently accepting
        // `def f(x: ClassVar[int])` would make the spelling look supported
        // when it carries no meaning at all. `pycc_hir::class::body` strips
        // the wrapper before calling this function, so the class-body
        // position never reaches this arm.
        Expr::Name(name) if name.id.as_str() == "ClassVar" => Err(unsupported(
            "`ClassVar` is only valid on a class-body attribute declaration \
             (`X: ClassVar[int] = 1` inside a `class` body), and takes exactly \
             one type argument",
            pycc_ast::expr_range(annotation),
        )),
        Expr::Name(name) if name.id.as_str() == "Self" && class_name.is_some() => {
            Ok(enclosing_class_ty(class_name.unwrap(), class_defs))
        }
        // PEP 649/749 (#387): a method's return-type annotation may reference
        // the enclosing class's own name (self-referential deferred
        // annotation, e.g. `class Node: def next(self) -> Node: ...`). Inside
        // a class body (`class_name` is `Some`), the class's own name resolves
        // to `Ty::Instance(class_name)` — or to `Ty::Protocol(class_name)`
        // when the enclosing class is a protocol (#948), matching what the
        // general `Expr::Name` arm below resolves any *other* protocol name
        // to. This is specifically for the self-referential case —
        // cross-class references are not in scope.
        Expr::Name(name) if Some(name.id.as_str()) == class_name => {
            Ok(enclosing_class_ty(name.id.as_str(), class_defs))
        }
        Expr::Name(name) => match name.id.as_str() {
            "int" => Ok(Ty::Int),
            "float" => Ok(Ty::Float),
            "bool" => Ok(Ty::Bool),
            "str" => Ok(Ty::Str),
            // Part 1 of #1027: a one-dimensional `float` `memoryview`, the
            // carrier a `pycc build --ext` boundary admits so a host can
            // hand compiled code a NumPy-shaped array without copying it.
            // Lowered unconditionally here, in both parameter and return
            // position: the mode-dependent refusal lives where the mode is
            // known -- `src/memoryview_mode.rs` refuses a `memoryview`
            // *signature* with `I0405` in a native build. Since #1397 this
            // crate does know one artifact-mode fact, `EXT_MODULE_MARKER`
            // (D-258 rule 6), but only `Any`, `object` and the
            // object-carrying containers consult it; the buffer carrier
            // keeps its unconditional lowering.
            //
            // This arm is also the parser of a bare `x: memoryview`
            // declaration, which neither mode-dependent refusal reaches --
            // both walk a `HirItem::Function`'s signature, not a statement
            // body. `crates/pycc_types`'s `reject_memoryview_declaration`
            // owns that position, in both modes.
            "memoryview" => Ok(Ty::MemoryView),
            // The byte buffer a guarded specialization of an untyped
            // function types an exact-`bytearray` argument as
            // (`crate::ndarray`'s module doc); only that pass spells it.
            pycc_ast::BYTEARRAY_ANNOTATION if is_ext_module(aliases) => {
                Ok(Ty::NdArray(crate::ndarray::NdTy::bytearray()))
            }
            // D-258 rule 2 (#1397): in an `ext` module `Any` is the opaque
            // CPython object a foreign import binds. Everywhere else it keeps
            // `T0002`, with the same code, message and span.
            "Any" if objects_spellable(aliases) => Ok(Ty::Object),
            "Any" => Err(Diagnostic::error(
                "T0002",
                "`Any` is not permitted in pycc code outside a declared interop boundary"
                    .to_string(),
                Span::new(0, 0),
            )),
            other => {
                // #380 (PR-20): resolve a bare name matching a known class
                // to `Ty::Instance` (or `Ty::Protocol` if the class is a
                // protocol). This fixes the pre-existing gap where
                // cross-class annotations (`def f(a: A) -> int` where `A`
                // is a user-defined class) produced `C0001`. Checked before
                // the alias table so a class name takes priority over an
                // alias of the same name.
                if let Some(info) = class_defs.iter().find(|info| info.name == other) {
                    if info.is_protocol {
                        return Ok(Ty::Protocol(Box::new(other.to_string())));
                    }
                    return Ok(Ty::Instance(Box::new(other.to_string())));
                }
                if let Some(ty) = aliases
                    .iter()
                    .rev()
                    .find(|(alias_name, _)| alias_name == other)
                    .map(|(_, ty)| ty.clone())
                {
                    return Ok(ty);
                }
                // A package built whole: a class another module of the
                // package compiled, imported by name, is read natively
                // through its sibling class (`crate::SIBLING_CLASS_PREFIX`).
                match crate::sibling_class_for(other) {
                    Some(Some(class)) if class_defs.iter().any(|info| info.name == class) => {
                        return Ok(Ty::Instance(Box::new(class)));
                    }
                    Some(_) if objects_spellable(aliases) => return Ok(Ty::Object),
                    _ => {}
                }
                // #1129/#1134: the buffer-carrier spellings, resolved only
                // after `class_defs` and the alias table above, which is the
                // rule; see `buffer_spelling` for why.
                if let Some(ty) = buffer_spelling::buffer_carrier_spelling(other) {
                    return Ok(ty);
                }
                // D-258 rules 3 and 4 (#1397): in an `ext` module the builtin
                // `object` and a bare `list`/`dict`/`tuple`/`set` are the
                // opaque CPython object. Resolved *after* `class_defs` and the
                // alias table, on the `ndarray` terms above: a module's own
                // `class list:` or `type object = ...` shadows the builtin, as
                // it does in Python.
                if objects_spellable(aliases)
                    && (other == "object" || is_ext_object_container(other))
                {
                    return Ok(Ty::Object);
                }
                Err({
                    // The message is built in `module` so #867's cascade
                    // classifier can parse it back (D-219).
                    //
                    // A *bare* builtin container name (`list`, `dict`,
                    // ...) is deliberately not special-cased here.
                    // `annotation_to_ty` has no idea which annotation
                    // position it is lowering, and the parameterized
                    // form it would advise -- `list[int]` -- is rejected
                    // in half the positions that reach this function. The
                    // generic message is correct in all of them, so the
                    // advice is opted into by the callers that can
                    // honour it, through `with_bare_container_advice`.
                    unsupported(
                        crate::module::unknown_annotation_name_message(other),
                        pycc_ast::expr_range(annotation),
                    )
                })
            }
        },
        // Issue #435 (Part D, __class_getitem__): `ClassName[type_arg]` as a
        // type annotation (PEP 560). Since #1130 a subscript whose base
        // resolves to a type pycc can name nominally is accepted whether or
        // not the class defines `__class_getitem__`: in pycc's static type
        // system it resolves to the hook's declared return type when the
        // class has one (#693), and otherwise to `Ty::Instance(ClassName)` —
        // the class itself, ignoring the type argument (consistent with how
        // generic classes are handled by PEP 695's `GenericClassInstantiate`
        // for actual instantiation, not annotation). A `type A = C` alias is
        // transparent: `A[int]` behaves exactly as `C[int]`, and so is a
        // `type Arr = memoryview` alias to the buffer carrier. The base must
        // be a bare name; any other subscript shape is rejected, and (#931,
        // narrowed by #1130) a bare name that resolves to something that is
        // *not* a nameable type -- a type parameter, a builtin scalar,
        // `Self`, or an alias to a non-class, non-carrier `Ty` -- is
        // rejected with `T0044` rather than having its type argument
        // silently discarded (see the `_ =>` arm).
        //
        // PEP 593 (#383): `Annotated[X, ...]` is recognized as a bare name
        // (no `from typing import Annotated` required, matching the existing
        // `TypeAlias`/`Any` precedent) and unwrapped to `X`, discarding all
        // metadata arguments. Per PEP 593's own spec, a static type checker
        // that does not understand a piece of metadata must treat
        // `Annotated[X, ...]` as `X` — this is correct, not a shortcut. The
        // first subscript argument is `X`; for the tuple form
        // `Annotated[X, meta1, meta2, ...]` the first element is `X`.
        // PEP 593 requires at least two arguments (the type and at least one
        // metadata element); `Annotated[X]` without metadata is rejected,
        // matching CPython's own `TypeError`.
        Expr::Subscript(sub) => {
            if let Some(ty) = crate::ndarray::typed_array_annotation(sub, aliases, class_defs)? {
                return Ok(ty);
            }
            let Expr::Name(base_name) = sub.value.as_ref() else {
                // Non-strict mode: a library generic (`mod.Type[int]`) is
                // an object.
                if is_non_strict(aliases) && matches!(sub.value.as_ref(), Expr::Attribute(_)) {
                    return Ok(Ty::Object);
                }
                return Err(unsupported(
                    "a subscripted type annotation's base must be a bare class name",
                    pycc_ast::expr_range(&sub.value),
                ));
            };
            match base_name.id.as_str() {
                // A generator: `Iterator[T]`, `Iterable[T]` or
                // `Generator[T, S, R]` (send type `S`, return type `R`).
                "Iterator" | "Iterable" | "Generator"
                    if !class_defs
                        .iter()
                        .any(|info| info.name == base_name.id.as_str()) =>
                {
                    let args: Vec<&Expr> = match sub.slice.as_ref() {
                        Expr::Tuple(tuple) => tuple.elts.iter().collect(),
                        other => vec![other],
                    };
                    let well_formed = match base_name.id.as_str() {
                        "Generator" => matches!(args.len(), 1 | 3),
                        _ => args.len() == 1,
                    };
                    if !well_formed {
                        return Err(unsupported(
                            format!(
                                "`{}[...]` takes one type argument{}",
                                base_name.id,
                                if base_name.id.as_str() == "Generator" {
                                    " or three (`Generator[Y, S, R]`)"
                                } else {
                                    ""
                                }
                            ),
                            pycc_ast::expr_range(annotation),
                        ));
                    }
                    let lower = |arg: &Expr| {
                        annotation_to_ty(arg, type_param, class_name, aliases, class_defs)
                    };
                    let element = lower(args[0])?;
                    let (sent, returned) = if args.len() == 3 {
                        (lower(args[1])?, lower(args[2])?)
                    } else {
                        (Ty::None, Ty::None)
                    };
                    let ty = Ty::generator_of(element, sent, returned);
                    crate::check_container_ty(&ty, Span::new(0, 0)).map_err(|mut d| {
                        let r = pycc_ast::expr_range(annotation);
                        d.span = Some(Span::new(r.start, r.end));
                        d
                    })?;
                    Ok(ty)
                }
                // A function value: `Callable[[A, B], R]`.
                "Callable" if !class_defs.iter().any(|info| info.name == "Callable") => {
                    // An `--ext --non-strict` module's callables come from
                    // Python too (a builtin, a lambda, a bound method),
                    // which no native function type can hold: the object.
                    if is_ext_module(aliases) && is_non_strict(aliases) {
                        return Ok(Ty::Object);
                    }
                    let (Expr::Tuple(tuple), true) = (sub.slice.as_ref(), true) else {
                        return Err(callable_shape(annotation));
                    };
                    let [Expr::List(params), ret] = &tuple.elts[..] else {
                        return Err(callable_shape(annotation));
                    };
                    let params = params
                        .elts
                        .iter()
                        .map(|p| annotation_to_ty(p, type_param, class_name, aliases, class_defs))
                        .collect::<Result<Vec<_>, _>>()?;
                    let ret = annotation_to_ty(ret, type_param, class_name, aliases, class_defs)?;
                    Ok(Ty::Callable(Box::new((params, ret))))
                }
                // `typing.Optional[X]` and `typing.Union[X, ...]`: the
                // unions `X | None` and `X | ...` spell (`func::union`),
                // unless a class, an alias or a type parameter shadows the
                // name.
                "Optional" | "Union"
                    if Some(base_name.id.as_str()) != type_param
                        && !class_defs
                            .iter()
                            .any(|info| info.name == base_name.id.as_str())
                        && !aliases
                            .iter()
                            .any(|(name, _)| name == base_name.id.as_str()) =>
                {
                    let members = union::subscript_members(&sub.slice);
                    let range = pycc_ast::expr_range(annotation);
                    let optional = base_name.id.as_str() == "Optional";
                    if optional && members.len() != 1 {
                        return Err(unsupported(
                            "`Optional[...]` takes exactly one type argument",
                            range,
                        ));
                    }
                    let scope = union::Scope {
                        type_param,
                        class_name,
                        aliases,
                        class_defs,
                    };
                    union::union_to_ty(&members, optional, range, &scope)
                }
                // #911: see the bare-`ClassVar` arm above -- `ClassVar[T]`
                // is legal only on a class-body attribute declaration, where
                // `pycc_hir::class::body::strip_class_var` removes it before
                // this function ever sees it.
                "ClassVar" => Err(unsupported(
                    "`ClassVar` is only valid on a class-body attribute declaration \
                     (`X: ClassVar[int] = 1` inside a `class` body)",
                    pycc_ast::expr_range(annotation),
                )),
                "Annotated" => {
                    let Expr::Tuple(tuple) = sub.slice.as_ref() else {
                        return Err(unsupported(
                            "Annotated requires at least two arguments: the type and at least one metadata element",
                            pycc_ast::expr_range(&sub.slice),
                        ));
                    };
                    if tuple.elts.len() < 2 {
                        return Err(unsupported(
                            "Annotated requires at least two arguments: the type and at least one metadata element",
                            pycc_ast::expr_range(&sub.slice),
                        ));
                    }
                    annotation_to_ty(&tuple.elts[0], type_param, class_name, aliases, class_defs)
                }
                // PEP 591 (#383): `Final[X]` unwraps to `X`. `Final` is a
                // binding-level property (this name may not be reassigned),
                // not a type-level property — the type is just `X`. The
                // non-reassignability is tracked separately by the type
                // checker's `Environment.finals` set, populated from
                // `HirStmt::AnnAssign`'s `is_final` flag (set at lowering
                // time in `stmt.rs`). `Final` takes exactly one type
                // argument; `Final[X, Y]` is rejected.
                "Final" => {
                    let x = match sub.slice.as_ref() {
                        Expr::Tuple(tuple) if tuple.elts.len() != 1 => {
                            return Err(unsupported(
                                "Final takes exactly one type argument",
                                pycc_ast::expr_range(&sub.slice),
                            ));
                        }
                        Expr::Tuple(tuple) => &tuple.elts[0],
                        other => other,
                    };
                    annotation_to_ty(x, type_param, class_name, aliases, class_defs)
                }
                // PEP 560 (#611, reversed in annotation position by #1130):
                // a subscript on a known class used to be rejected unless
                // the class defined `__class_getitem__`. It no longer is.
                // CPython 3.14 evaluates annotations lazily (PEP 649/749),
                // so `class C: pass` + `def f(a: C[int])` raises no
                // `TypeError` at definition time -- that error surfaces only
                // through `typing.get_type_hints`, and pycc builds no
                // runtime `__annotations__` object for any function, class
                // or module, so no pycc-compiled program can reach it.
                // Accepting the annotation therefore moves toward CPython
                // 3.14's observable behavior, not away from it. **Value
                // position is the opposite and is deliberately unchanged**:
                // `x = C[int]` is evaluated eagerly and really does raise
                // `TypeError: type 'C' is not subscriptable`, so
                // `pycc_types`' value-position `C[x]` path (#610) computes
                // its own subscriptability answer and keeps reporting
                // `T0044`. The resulting asymmetry is CPython's own, not an
                // invention of this change. The forfeiture #1130 accepts in
                // exchange: `C[int]` where `C` is a genuinely non-generic
                // user class is an ordinary typo that #611 caught and that
                // pycc now accepts silently.
                //
                // #931 rejects every resolvable base that is *not* a
                // nameable type: a PEP 695 type parameter (`T[int]`), a
                // builtin scalar (`int[str]`), `Self` inside a class, and a
                // `type` alias to a non-class, non-carrier type. Each of
                // those used to fall through to the bare-name recursion
                // below, which resolved the base and silently discarded the
                // type argument. CPython reports all of them with the same
                // `TypeError: ... is not subscriptable`, so they share the
                // `T0044` code and differ only in the noun
                // (`subscripted_base_description`). #1130 carves the buffer
                // carrier out of the alias half of that list -- see step 4.
                //
                // Two bases keep their pre-#931 diagnostic on purpose: an
                // undefined name still gets the exact `C0001` that
                // `module::cascade_name` parses back (D-219), and `Any`
                // still gets `T0002` outside an `ext` module. Both come out
                // of the final recursion on the bare base before the reject
                // can fire. Inside an `ext` module `Any[...]` and
                // `object[...]` get this `T0044` instead (D-258, #1397).
                _ => {
                    let base = base_name.id.as_str();
                    let range = pycc_ast::expr_range(annotation);
                    // Step 1: resolve the class the base denotes -- directly,
                    // or through a `type A = C` alias, since PEP 695 aliases
                    // are transparent and `A[int]` must behave exactly as
                    // `C[int]`. This deliberately consults the alias *table*
                    // rather than recursing `annotation_to_ty` on the bare
                    // name: `Self` and the enclosing class's own name also
                    // resolve to `Ty::Instance` through the `Expr::Name` arm,
                    // and the alias path must not be how a class is reached
                    // for them (the self-referential `class_defs` entry
                    // `lower_class` pushes is).
                    //
                    // The alias table is consulted only when the `Expr::Name`
                    // arm would itself reach it. A type parameter, `Self`
                    // inside a class, the enclosing class's own name, the
                    // builtin scalar names and `Any` all resolve *before*
                    // `class_defs` and the alias table there, so an alias
                    // that happens to share such a name must not win here
                    // either (`type int = C` + bare `x: int` is `Int`; `type
                    // Any = C` + `Any[str]` is `T0002` -- `T0044` in an `ext`
                    // module, D-258 -- and both stay that way).
                    //
                    // #1129: `memoryview` is on that list for the same reason
                    // -- it is a reserved keyword the `Expr::Name` arm answers
                    // before either table, so `type memoryview = C` never
                    // makes the name mean `C`, and `memoryview[...]` must not
                    // be reported against `C` either. Its sibling spellings
                    // `ndarray` (#1129) and `NDArray` (#1134) are deliberately
                    // absent: those are ordinary identifiers resolved *after*
                    // both tables, so an alias of either name genuinely does
                    // win here.
                    let name_resolves_before_aliases = Some(base) == type_param
                        || (base == "Self" && class_name.is_some())
                        || Some(base) == class_name
                        || name_resolves_before_class_defs(base);
                    let alias_target = if name_resolves_before_aliases {
                        None
                    } else {
                        aliases
                            .iter()
                            .rev()
                            .find(|(n, _)| n == base)
                            .map(|(_, ty)| ty)
                    };
                    // The *direct* class lookup is gated on the type
                    // parameter only: a type parameter shadows a same-named
                    // class in the bare-name arm, and before #931 the
                    // subscript arm disagreed with that -- `class G[U]:` +
                    // `def f[G](x: G[int])` passed the class ladder and then
                    // the ladder's final recursion resolved `G` to
                    // `Ty::Param("G")`, silently dropping `[int]`. It is NOT
                    // gated on `Self`/`class_name`: their class is the
                    // self-referential entry `lower_class` pushes, and
                    // `G[int]` inside `class G[T]`'s own body must stay
                    // accepted.
                    let known_class = if Some(base) == type_param
                        || (Some(base) != class_name && name_resolves_before_class_defs(base))
                    {
                        None
                    } else {
                        class_defs.iter().find(|info| info.name == base)
                    }
                    .or_else(|| {
                        alias_target.and_then(|ty| match ty {
                            Ty::Instance(n) | Ty::Protocol(n) => {
                                class_defs.iter().find(|info| info.name == n.as_str())
                            }
                            _ => None,
                        })
                    });
                    // Step 2: the known-class ladder (#611, #693). The first
                    // clause names the class; the trailing clause spells the
                    // *written* base, which differs from the class name when
                    // the base is an alias (`type A = C` / `x: A[int]`), so
                    // the text agrees with the caret.
                    if let Some(info) = known_class {
                        // Issue #693 (PEP 560): when the class's `__class_getitem__`
                        // hook has a resolvable declared return type, the
                        // annotation resolves to *that* type -- matching
                        // `pycc_types::resolve_static_or_class_method_call`'s
                        // identical use of the hook's declared return type for
                        // value-position `C[x]` (#610) -- rather than to
                        // `Ty::Instance(ClassName)`. `class_getitem_return` is
                        // `None` when the class is generic only through a PEP 695
                        // type parameter with no explicit hook (that case is
                        // handled by `GenericClassInstantiate`, not here), from
                        // the self-referential entry `lower_class` pushes for the
                        // class it is currently lowering, or when the hook has no
                        // explicit return annotation -- `class_getitem_return_ty`
                        // deliberately treats a raw, pre-inference `Ty::Infer` as
                        // unresolved rather than propagating it here, since this
                        // crate never runs its own inference pass (see
                        // `lower_method`'s doc comment). In every such case the
                        // annotation resolves to `Ty::Instance(ClassName)` (or
                        // `Ty::Protocol`) through the bare-name recursion, the
                        // class itself, ignoring the type argument -- the same
                        // way a class name, an alias to one, and the
                        // self-referential name all resolve as a bare
                        // annotation.
                        if let Some(return_ty) = &info.class_getitem_return {
                            return Ok(return_ty.clone());
                        }
                        return annotation_to_ty(
                            &Expr::Name(base_name.clone()),
                            type_param,
                            class_name,
                            aliases,
                            class_defs,
                        );
                    }
                    // Step 2b: an alias whose target is a class that is *not*
                    // in `class_defs` -- `from lib_a import A` where `lib_a`
                    // has `type A = G`: `import.rs`'s TypeAlias binding pushes
                    // the alias, but only the Class binding runs
                    // `copy_class_with_ancestors`, so `G` is absent here. That
                    // program was accepted before #931 and stays accepted
                    // exactly as before (fail-open on the class-like `Ty`,
                    // never a spurious reject). The missing copy is a
                    // pre-existing #881-area gap, out of scope here.
                    if matches!(alias_target, Some(Ty::Instance(_) | Ty::Protocol(_))) {
                        return annotation_to_ty(
                            &Expr::Name(base_name.clone()),
                            type_param,
                            class_name,
                            aliases,
                            class_defs,
                        );
                    }
                    // Step 3, D-228 (issue #918): the builtin container
                    // types (`frozenset` since Part 1 of #1319) are lowered
                    // here, from a *parameterized* annotation only -- the
                    // bare `list`/`dict`/`set`/`frozenset`/`tuple`
                    // spelling still falls through the `Expr::Name` arm's
                    // `other =>` branch and gets its own `C0001`.
                    //
                    // Deliberately checked *after* the known-class ladder
                    // above (which now also covers an alias to a class) and
                    // gated on the alias table: `class list:` and
                    // `type list = ...` both legally shadow the builtin in
                    // Python, and silently retyping `x: list[int]` as a builtin
                    // list when the user defined their own `list` would be a
                    // miscompile, not merely a worse diagnostic. (The plan for
                    // #918 proposed a dedicated match arm ahead of this one,
                    // which would have shadowed both; the ordering here costs
                    // nothing and keeps the user's own definition winning.)
                    //
                    // A PEP 695 type parameter shadows the builtin for the
                    // same reason a user-defined class does, and the
                    // `Expr::Name` arm above already gives `type_param` the
                    // first word: `def f[list](x: list[int])` declares `list`
                    // as a type variable, so the annotation subscripts that
                    // variable -- invalid Python, rejected below (#931) --
                    // rather than naming the builtin. Lowering it as
                    // `Ty::List(Int)` would silently drop the function's
                    // genericity.
                    //
                    // #1378: the pre-PEP 585 `typing` aliases (`Dict[K, V]`,
                    // `List[T]`, `Set[T]`, `FrozenSet[T]`, `Tuple[...]`)
                    // take this same step through `container_family`, and the
                    // three guards test the spelling *as written*, so a type
                    // parameter, a `type` alias or a user class named `Dict`
                    // still wins over the typing form exactly as one named
                    // `dict` wins over the builtin. Like `Final`, the legacy
                    // spellings resolve with or without `from typing import`.
                    if Some(base) != type_param
                        && container_family(base).is_some()
                        && !aliases.iter().any(|(name, _)| name == base)
                    {
                        return container_annotation_to_ty(
                            base,
                            sub.slice.as_ref(),
                            annotation,
                            type_param,
                            class_name,
                            aliases,
                            class_defs,
                        );
                    }
                    // Step 4 (#931, narrowed by #1130): resolve the bare base
                    // so an undefined name keeps its cascade-shaped `C0001`
                    // (D-219) and `Any` keeps `T0002` in a `native` build
                    // (in an `ext` module it resolves, and is refused by
                    // the `non_generic_object` guard below). A base that resolves
                    // here is either not a class at all -- a type parameter,
                    // `Self`, a builtin scalar, or an alias to a
                    // scalar/container/Optional/type parameter -- or it is one
                    // of the six names the `Expr::Name` arm answers before
                    // `class_defs` and a class of that name exists
                    // (`class int[T]: pass` + `def f(a: int[str])` reaches
                    // here with `int` in `class_defs`, because the gate on
                    // the direct lookup above skips it: the reserved name,
                    // not the shadow class, is what the bare form means).
                    // None of those accepts a type argument, so the subscript
                    // is rejected instead of silently discarding it -- except
                    // for the buffer carrier, which is a nameable type.
                    // #1130: the buffer carrier is a nameable type, so a
                    // subscript on it is erased exactly as a class's is --
                    // `memoryview[float]`, `ndarray[float]`,
                    // `NDArray[float]` and `type Arr = memoryview` +
                    // `Arr[float]` all lower to the carrier. Keyed on the
                    // resolved `Ty` rather than on the spelling, so a further
                    // carrier spelling is admitted by its registration alone
                    // -- which is exactly how #1134's `NDArray` reached this
                    // arm without an edit here, as #1129's `ndarray` did
                    // before it.
                    //
                    // Keying on the resolved type is safe only because the
                    // `Expr::Name` arm answers a PEP 695 type parameter
                    // (level 1) and the enclosing class's own name (level 3)
                    // *before* the `memoryview` keyword arm (level 4): a type
                    // parameter named `memoryview`, or a class named
                    // `memoryview` seen from inside its own body, resolves to
                    // `Ty::Param`/`Ty::Instance` here and the accept cannot
                    // swallow it. `Self[int]` likewise resolves to
                    // `Ty::Instance` through this same recursion and keeps
                    // #931's rejection, which is why the accept is keyed on
                    // `Ty::MemoryView` only and not on "any nominal type".
                    let resolved = annotation_to_ty(
                        &Expr::Name(base_name.clone()),
                        type_param,
                        class_name,
                        aliases,
                        class_defs,
                    )?;
                    // Part 1 of #1367: a name a foreign import binds resolves
                    // to the opaque `Ty::Object`, and its subscript
                    // (`Queue[int]`, `ParseTableBase[StateT]`) is erased the
                    // same way, its arguments never resolved -- CPython 3.14
                    // never evaluates an annotation (PEP 649).
                    //
                    // D-258 (#1397): `Any` and `object` resolve to the same
                    // `Ty::Object` in an `ext` module, but neither is generic,
                    // so `Any[int]` and `object[int]` are refused with the
                    // `T0044` every other non-subscriptable base gets rather
                    // than being erased like a foreign class's subscript.
                    // `Any` is reserved before both tables, so it is always
                    // the builtin here; `object` is the builtin only when no
                    // alias or foreign import of that name shadows it (a
                    // module's own `class object` took the known-class
                    // ladder above). A foreign import named `object` is a
                    // foreign class and keeps the erasure.
                    let builtin_object =
                        base == "object" && !aliases.iter().any(|(name, _)| name == base);
                    let non_generic_object = base == "Any" || builtin_object;
                    if resolved == Ty::MemoryView || (resolved == Ty::Object && !non_generic_object)
                    {
                        return Ok(resolved);
                    }
                    // The noun is decided here rather than in
                    // `subscripted_base_description`, which sees only the
                    // spelling: `type object = int` + `object[str]` must
                    // keep its `native` noun, "type alias `object`".
                    let noun = if builtin_object {
                        "builtin type `object`".to_string()
                    } else {
                        subscripted_base_description(base, type_param, class_name)
                    };
                    Err(Diagnostic::error(
                        "T0044",
                        format!(
                            "{noun} is not subscriptable, so `{base}[...]` is not a valid type \
                             annotation"
                        ),
                        Span::new(range.start, range.end),
                    ))
                }
            }
        }
        // `T | None` / `None | T` (PEP 604, D-197, #763, Part 1 of #747):
        // accept exactly the 2-operand shape where one side is
        // `Expr::NoneLiteral`, recursing into the other side so
        // `list[int] | None`, `SomeClass | None`, and `T | None` (a generic
        // param) all parse for free through their own existing arms above.
        // A 2-operand union where *neither* side is `None` (`int | str`), or
        // any chain (`ops.len() != 1`, e.g. `A | B | None`), is a general
        // union: explicitly out of scope for this PR (Part 2+), rejected
        // with `T0048` rather than silently misparsed or falling through to
        // the generic catch-all below.
        Expr::BinOp(bin_op) if bin_op.op == Operator::BitOr => union::bitor_to_ty(
            bin_op,
            &union::Scope {
                type_param,
                class_name,
                aliases,
                class_defs,
            },
        ),
        // Part 1 of #889: a string the parser did not already unquote --
        // one nested in an annotation (`list["C"]`) or one that does not
        // parse -- resolves like its unquoted spelling, in this same scope.
        Expr::StringLiteral(literal) => {
            string_annotation_to_ty(literal, type_param, class_name, aliases, class_defs)
        }
        // Non-strict mode (v0.7 accept 2): a library type (`mod.Type`,
        // `mod.Type[...]`) is an object, which its values are.
        Expr::Attribute(_) if is_non_strict(aliases) => Ok(Ty::Object),
        other => Err(unsupported(
            format!(
                "only a bare name type annotation is supported so far, got {}",
                pycc_ast::expr_kind_name(other)
            ),
            pycc_ast::expr_range(other),
        )),
    }
}
