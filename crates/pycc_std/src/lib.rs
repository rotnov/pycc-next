//! `pycc_std`: a static registry of stdlib symbols this pycc version
//! hand-recognizes at compile time (D-136).
//!
//! This crate is deliberately a plain data crate: no proc-macro, no build
//! script, and no dependency on any other workspace crate (including
//! `pycc_hir`, which owns the compiler's real `Ty` representation). Callers
//! (`pycc_hir` for import binding, `pycc_types` for type-checking) convert
//! [`ScalarKind`] into their own `Ty` at the call site instead of this crate
//! depending on either of them -- keeping the dependency direction strictly
//! one-way (`pycc_hir`/`pycc_types` -> `pycc_std`, never the reverse).
//!
//! Scope (D-088, D-136): this PR ships exactly `math.sqrt` and `math.pi`.
//! `sys` is not registered at all -- the original plan considered
//! `sys.argv`/`sys.exit`, but both need machinery (a `list[str]`-shaped
//! runtime value for `argv`, `NoReturn`-style divergence handling in
//! `pycc_types`/`pycc_mir` for `exit`) that has no existing precedent in
//! this codebase and no concrete lowering slice in this PR's Task 4 scope.
//! Recorded as a D-136 addendum in
//! `docs/decisions/D-136-pycc-std-is-a-plain-data-crate-math-sys-symbols.md`
//! rather than silently narrowing the ADR's own text. `math.floor`/`math.pow`
//! are likewise withheld from the registry for the same reason: D-136 (via
//! D-088's established series precedent) requires that nothing
//! type-checkable here be left unlowerable in `pycc_mir`/`pycc_codegen`, and
//! only `sqrt`/`pi` have a concrete lowering path landing in this PR.

/// A stdlib module name this compiler recognizes as an `import`/
/// `from ... import ...` target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StdModule {
    Math,
    /// The `enum` module, recognized solely so `from enum import Enum`
    /// resolves (#379, PR-19). `Enum` is a marker symbol, not a value —
    /// it is only valid as the sole base class of `class C(Enum):`.
    Enum,
    /// The `typing` module, recognized so `from typing import Protocol`
    /// and `from typing import runtime_checkable` resolve (#380, PR-20).
    /// `Protocol` is a marker symbol (base-class marker), and
    /// `runtime_checkable` is a decorator-marker symbol. Both are
    /// recognized as bare names without requiring the import, matching
    /// the `Final`/`Annotated`/`Enum` precedent.
    Typing,
    /// The `abc` module, recognized so `from abc import ABC` and
    /// `from abc import abstractmethod` resolve (#380, PR-20). `ABC` is
    /// a base-class marker, and `abstractmethod` is a decorator-marker
    /// symbol. Both are recognized as bare names without requiring the
    /// import.
    Abc,
    /// The `dataclasses` module, recognized so `from dataclasses import
    /// dataclass` resolves (#579, Part 3 of #572). `dataclass` is a
    /// decorator-marker symbol. Like every other marker in this registry
    /// it is also recognized as a bare name without requiring the import
    /// (`pycc_hir`'s class-decorator classifier matches the bare name
    /// `dataclass`), so registering the module changes nothing about how
    /// pycc lowers `@dataclass` -- it only makes the import CPython
    /// itself requires resolve, which is what lets the PEP 557/698/3129
    /// conformance fixtures run under the pinned oracle at all.
    Dataclasses,
}

/// The scalar shape of a registered symbol's argument/return type, kept
/// local to this crate (D-136's dependency-direction ruling) rather than
/// reusing `pycc_hir::Ty` directly. Deliberately covers only the concrete
/// cases the registered symbols actually need -- not a general type
/// representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScalarKind {
    Float,
}

/// The kind of a registered stdlib symbol: a callable function with a fixed
/// argument/return shape, a constant with a fixed type, or a class marker
/// that is only valid as a base class (not a first-class value).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StdSymbolKind {
    Function {
        arg_tys: &'static [ScalarKind],
        ret_ty: ScalarKind,
    },
    Constant {
        ty: ScalarKind,
    },
    /// A class marker symbol (e.g. `enum.Enum`) that is only valid as the
    /// sole base class of a class definition (`class C(Enum):`). It is not
    /// a first-class value — referencing it as a value or calling it is
    /// rejected by the type checker.
    EnumMarker,
    /// A protocol marker symbol (`typing.Protocol`) that is only valid as
    /// the sole base class of a protocol class definition
    /// (`class P(Protocol):`). It is not a first-class value —
    /// referencing it as a value or calling it is rejected by the type
    /// checker (#380, PR-20).
    ProtocolMarker,
    /// An ABC marker symbol (`abc.ABC`) that is only valid as a base
    /// class of an abstract class definition (`class C(ABC):`). It is
    /// not a first-class value (#380, PR-20).
    AbcMarker,
    /// A decorator-marker symbol (e.g. `typing.runtime_checkable`,
    /// `abc.abstractmethod`) that is only valid as a class or method
    /// decorator. It is not a first-class value — referencing it as a
    /// value or calling it is rejected by the type checker (#380, PR-20).
    DecoratorMarker,
    /// PEP 435 (#892): `enum.auto`, the member-value placeholder usable only
    /// inside an enum class body (`RED = auto()`). Like every other marker
    /// kind it has no value of its own -- `class/enum_class.rs` recognizes
    /// the call syntactically and derives the member's value from the
    /// class's own value type and its preceding members; naming `auto`
    /// anywhere else is the same "marker is not a value" error every other
    /// marker kind produces.
    EnumAutoMarker,
    /// An annotation-only marker symbol (`typing.Final`, `typing.Annotated`,
    /// `typing.ClassVar`) that is only valid as a bare-name annotation
    /// subscript (`Final[X]`, `Annotated[X, ...]`, `ClassVar[X]`,
    /// PEP 591/593/526). Unlike the other
    /// marker kinds it is not a base-class or decorator marker either — it
    /// has no role outside annotation position. `pycc_hir::func::
    /// annotation_to_ty` already recognizes `Final`/`Annotated` by bare
    /// name regardless of whether this registry entry exists; registering
    /// the symbol here only makes `from typing import Final`/`Annotated`/
    /// `ClassVar` itself resolve instead of failing with `C0002` (#762,
    /// #911). `ClassVar[X]` differs from the other two in *where* it is
    /// accepted: it is a class-body-only annotation wrapper, stripped by
    /// `pycc_hir::class::body`'s own class-attribute lowering and rejected
    /// with `C0001` by the shared `pycc_hir::func::annotation_to_ty`
    /// everywhere else. It is not a
    /// first-class value — referencing it as a value or calling it is
    /// rejected by the type checker, the same as every other marker kind.
    AnnotationMarker,
    /// The `typing.cast` marker symbol (#767). Unlike every other marker
    /// kind, `cast` *is* callable in Python — `cast(T, value)` is a runtime
    /// no-op that only changes a static checker's view of `value`'s type.
    /// pycc models it as a compile-time-evaluated builtin call intercepted
    /// by bare callee name in `pycc_types` (the same way `isinstance` and
    /// `issubclass` are), because its first argument is a *type* expression
    /// rather than a value expression and so must not go through ordinary
    /// argument inference. Registering the symbol here only makes
    /// `from typing import cast` itself resolve instead of failing with
    /// `C0002`; the bare name `cast` is never looked up through this
    /// registry at a call site. Referencing it as a first-class value, or
    /// calling it through its qualified name (`typing.cast(...)`), is
    /// rejected by the type checker.
    CastMarker,
    /// The `typing.TYPE_CHECKING` marker symbol (#790). Unlike every other
    /// marker kind, `TYPE_CHECKING` *is* a genuine runtime `bool` constant
    /// in CPython (always `False`) -- but pycc supports it for exactly one
    /// purpose: the standard `if TYPE_CHECKING: ...` idiom that guards
    /// imports/statements meant only for static type checkers. `pycc_hir`'s
    /// `Stmt::If` lowering (`crates/pycc_hir/src/stmt.rs`) recognizes the
    /// bare name `TYPE_CHECKING` or the qualified `typing.TYPE_CHECKING`
    /// spelling as the `if`/`elif` test *before* this registry entry is
    /// ever consulted (matching the existing `Final` bare-name precedent),
    /// and constant-folds the branch the same way CPython evaluates it --
    /// the guarded body is never lowered or type-checked at all, so it may
    /// freely contain constructs pycc doesn't support elsewhere (forward-
    /// reference-only imports, typing-only names). This registry entry
    /// exists only so `from typing import TYPE_CHECKING` itself resolves
    /// instead of failing with `C0002`; a general reference to
    /// `TYPE_CHECKING` as a first-class value (assigned, printed, passed as
    /// an argument, or used inside a larger boolean expression such as `if
    /// TYPE_CHECKING and x:`) is out of this issue's scope and is rejected
    /// by the type checker exactly like every other marker kind.
    TypeCheckingMarker,
    /// A typing-form marker symbol (#1378): the pre-PEP 585 container
    /// aliases `typing.Dict`, `List`, `Set`, `FrozenSet`, `Tuple`, plus
    /// `typing.Any`, `typing.Generic` and (#1394) `typing.TypeVar`. Like
    /// `Final`, the container
    /// aliases and `Any` are resolved by their bare spelling in annotation
    /// position (`pycc_hir::func::annotation_to_ty` lowers `Dict[K, V]` as
    /// `dict[K, V]`, and `Any` keeps its by-design `T0002` -- or, in an
    /// `ext` module, lowers to the CPython object, D-258) whether or not
    /// this registry entry exists; registering the symbols here only makes
    /// `from typing import Dict, List, Any, Generic, ...` itself resolve
    /// instead of failing with `C0002`. `Generic` and `TypeVar` are never
    /// resolved by spelling, only through the import binding (Part 1 of
    /// #886, #1394): `pycc_hir` erases a `Generic[T, ...]` base whose
    /// arguments are type variables, and a module-level `T = TypeVar("T")`
    /// statement is a compile-time-only declaration. None of them is a
    /// first-class value -- referencing one as a value or calling it
    /// anywhere else is rejected by the type checker with its own "typing
    /// construct" message.
    TypingFormMarker,
}

/// A single registered stdlib symbol: which module it lives in, its source
/// name, and its type shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StdSymbol {
    pub module: StdModule,
    pub name: &'static str,
    pub kind: StdSymbolKind,
}

/// The full registry of hand-recognized stdlib symbols (D-136). A plain
/// `const` slice, linearly scanned by [`resolve_symbol`] -- a few dozen
/// entries at most, so no `HashMap`/`OnceCell`/macro-generated dispatch table
/// is warranted, and a linear scan keeps every branch trivially unit-tested
/// (D-014's 100% line/region coverage gate applies to this crate too).
const REGISTRY: &[StdSymbol] = &[
    StdSymbol {
        module: StdModule::Math,
        name: "sqrt",
        kind: StdSymbolKind::Function {
            arg_tys: &[ScalarKind::Float],
            ret_ty: ScalarKind::Float,
        },
    },
    StdSymbol {
        module: StdModule::Math,
        name: "pi",
        kind: StdSymbolKind::Constant {
            ty: ScalarKind::Float,
        },
    },
    StdSymbol {
        module: StdModule::Enum,
        name: "Enum",
        kind: StdSymbolKind::EnumMarker,
    },
    StdSymbol {
        module: StdModule::Enum,
        name: "StrEnum",
        kind: StdSymbolKind::EnumMarker,
    },
    StdSymbol {
        module: StdModule::Enum,
        name: "auto",
        kind: StdSymbolKind::EnumAutoMarker,
    },
    StdSymbol {
        module: StdModule::Typing,
        name: "Protocol",
        kind: StdSymbolKind::ProtocolMarker,
    },
    StdSymbol {
        module: StdModule::Typing,
        name: "runtime_checkable",
        kind: StdSymbolKind::DecoratorMarker,
    },
    StdSymbol {
        module: StdModule::Abc,
        name: "ABC",
        kind: StdSymbolKind::AbcMarker,
    },
    StdSymbol {
        module: StdModule::Abc,
        name: "abstractmethod",
        kind: StdSymbolKind::DecoratorMarker,
    },
    StdSymbol {
        module: StdModule::Typing,
        name: "override",
        kind: StdSymbolKind::DecoratorMarker,
    },
    StdSymbol {
        module: StdModule::Typing,
        name: "dataclass_transform",
        kind: StdSymbolKind::DecoratorMarker,
    },
    StdSymbol {
        module: StdModule::Dataclasses,
        name: "dataclass",
        kind: StdSymbolKind::DecoratorMarker,
    },
    StdSymbol {
        module: StdModule::Typing,
        name: "Final",
        kind: StdSymbolKind::AnnotationMarker,
    },
    StdSymbol {
        module: StdModule::Typing,
        name: "Annotated",
        kind: StdSymbolKind::AnnotationMarker,
    },
    StdSymbol {
        module: StdModule::Typing,
        name: "ClassVar",
        kind: StdSymbolKind::AnnotationMarker,
    },
    StdSymbol {
        module: StdModule::Typing,
        name: "cast",
        kind: StdSymbolKind::CastMarker,
    },
    StdSymbol {
        module: StdModule::Typing,
        name: "TYPE_CHECKING",
        kind: StdSymbolKind::TypeCheckingMarker,
    },
    StdSymbol {
        module: StdModule::Typing,
        name: "Dict",
        kind: StdSymbolKind::TypingFormMarker,
    },
    StdSymbol {
        module: StdModule::Typing,
        name: "List",
        kind: StdSymbolKind::TypingFormMarker,
    },
    StdSymbol {
        module: StdModule::Typing,
        name: "Set",
        kind: StdSymbolKind::TypingFormMarker,
    },
    StdSymbol {
        module: StdModule::Typing,
        name: "FrozenSet",
        kind: StdSymbolKind::TypingFormMarker,
    },
    StdSymbol {
        module: StdModule::Typing,
        name: "Tuple",
        kind: StdSymbolKind::TypingFormMarker,
    },
    StdSymbol {
        module: StdModule::Typing,
        name: "Any",
        kind: StdSymbolKind::TypingFormMarker,
    },
    StdSymbol {
        module: StdModule::Typing,
        name: "Generic",
        kind: StdSymbolKind::TypingFormMarker,
    },
    StdSymbol {
        module: StdModule::Typing,
        name: "TypeVar",
        kind: StdSymbolKind::TypingFormMarker,
    },
];

/// Resolves a dotted-import module name (e.g. `"math"`) to a [`StdModule`],
/// or `None` if this pycc version does not recognize it (every other
/// stdlib/third-party module name, including every PEP-594 dead battery).
pub fn resolve_module(name: &str) -> Option<StdModule> {
    match name {
        "math" => Some(StdModule::Math),
        "enum" => Some(StdModule::Enum),
        "typing" => Some(StdModule::Typing),
        "abc" => Some(StdModule::Abc),
        "dataclasses" => Some(StdModule::Dataclasses),
        _ => None,
    }
}

/// The canonical source spelling of a [`StdModule`] (`"math"` for
/// [`StdModule::Math`]) -- the exact inverse of [`resolve_module`], so
/// `resolve_module(module_name(m)) == Some(m)` for every variant. `pycc_hir`
/// emits every `<module>.<symbol>` string through this accessor rather than
/// from the receiver the user wrote, so an aliased receiver (`import math as
/// m` then `m.sqrt(x)`) still produces the canonical `"math.sqrt"` that
/// `pycc_types`, `pycc_mir`, and `pycc_codegen` key on.
pub fn module_name(module: StdModule) -> &'static str {
    match module {
        StdModule::Math => "math",
        StdModule::Enum => "enum",
        StdModule::Typing => "typing",
        StdModule::Abc => "abc",
        StdModule::Dataclasses => "dataclasses",
    }
}

/// Every name `module` exports, in registry order -- exactly the names a
/// `from <module> import *` would have bound. [`resolve_symbol`] answers
/// "is this one name exported"; a caller reasoning about the whole
/// statement (HIR's failed-import poisoning) needs the set instead.
pub fn module_symbol_names(module: StdModule) -> impl Iterator<Item = &'static str> {
    REGISTRY
        .iter()
        .filter(move |sym| sym.module == module)
        .map(|sym| sym.name)
}

/// Resolves a symbol name inside an already-bound module (e.g.
/// `resolve_symbol(StdModule::Math, "sqrt")`) to its registered
/// [`StdSymbol`], or `None` if the module is recognized but this particular
/// symbol is not registered (e.g. `math.tan`).
pub fn resolve_symbol(module: StdModule, name: &str) -> Option<StdSymbol> {
    REGISTRY
        .iter()
        .find(|sym| sym.module == module && sym.name == name)
        .copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_module_recognizes_math() {
        assert_eq!(resolve_module("math"), Some(StdModule::Math));
    }

    #[test]
    fn resolve_module_rejects_unregistered_name() {
        assert_eq!(resolve_module("os"), None);
        assert_eq!(resolve_module("sys"), None);
        assert_eq!(resolve_module("cgi"), None);
    }

    #[test]
    fn resolve_module_recognizes_enum() {
        assert_eq!(resolve_module("enum"), Some(StdModule::Enum));
    }

    #[test]
    fn resolve_module_recognizes_typing() {
        assert_eq!(resolve_module("typing"), Some(StdModule::Typing));
    }

    #[test]
    fn resolve_module_recognizes_abc() {
        assert_eq!(resolve_module("abc"), Some(StdModule::Abc));
    }

    #[test]
    fn resolve_module_recognizes_dataclasses() {
        assert_eq!(resolve_module("dataclasses"), Some(StdModule::Dataclasses));
    }

    #[test]
    fn module_name_round_trips_through_resolve_module_for_every_module() {
        for module in [
            StdModule::Math,
            StdModule::Enum,
            StdModule::Typing,
            StdModule::Abc,
            StdModule::Dataclasses,
        ] {
            assert_eq!(resolve_module(module_name(module)), Some(module));
        }
        assert_eq!(module_name(StdModule::Math), "math");
        assert_eq!(module_name(StdModule::Enum), "enum");
        assert_eq!(module_name(StdModule::Typing), "typing");
        assert_eq!(module_name(StdModule::Abc), "abc");
        assert_eq!(module_name(StdModule::Dataclasses), "dataclasses");
    }

    #[test]
    fn resolve_symbol_finds_dataclasses_dataclass() {
        let sym = resolve_symbol(StdModule::Dataclasses, "dataclass")
            .expect("dataclasses.dataclass is registered");
        assert_eq!(sym.module, StdModule::Dataclasses);
        assert_eq!(sym.name, "dataclass");
        assert_eq!(sym.kind, StdSymbolKind::DecoratorMarker);
    }

    #[test]
    fn resolve_symbol_rejects_unregistered_symbol_in_dataclasses_module() {
        // `field`/`asdict`/`replace` are real `dataclasses` names that pycc
        // does not implement; they must stay `C0002`, not resolve silently.
        assert_eq!(resolve_symbol(StdModule::Dataclasses, "field"), None);
        assert_eq!(resolve_symbol(StdModule::Dataclasses, "asdict"), None);
    }

    #[test]
    fn resolve_symbol_finds_typing_override() {
        let sym =
            resolve_symbol(StdModule::Typing, "override").expect("typing.override is registered");
        assert_eq!(sym.module, StdModule::Typing);
        assert_eq!(sym.name, "override");
        assert_eq!(sym.kind, StdSymbolKind::DecoratorMarker);
    }

    #[test]
    fn resolve_symbol_finds_typing_dataclass_transform() {
        let sym = resolve_symbol(StdModule::Typing, "dataclass_transform")
            .expect("typing.dataclass_transform is registered");
        assert_eq!(sym.module, StdModule::Typing);
        assert_eq!(sym.name, "dataclass_transform");
        assert_eq!(sym.kind, StdSymbolKind::DecoratorMarker);
    }

    #[test]
    fn dataclass_is_not_reachable_through_the_typing_module() {
        // `dataclass` lives in `dataclasses`, not `typing`; the registry is
        // keyed on (module, name), so the module half must actually matter.
        assert_eq!(resolve_symbol(StdModule::Typing, "dataclass"), None);
        assert_eq!(resolve_symbol(StdModule::Dataclasses, "override"), None);
    }

    #[test]
    fn resolve_symbol_finds_math_sqrt() {
        let sym = resolve_symbol(StdModule::Math, "sqrt").expect("math.sqrt is registered");
        assert_eq!(sym.module, StdModule::Math);
        assert_eq!(sym.name, "sqrt");
        assert_eq!(
            sym.kind,
            StdSymbolKind::Function {
                arg_tys: &[ScalarKind::Float],
                ret_ty: ScalarKind::Float,
            }
        );
    }

    #[test]
    fn resolve_symbol_finds_math_pi() {
        let sym = resolve_symbol(StdModule::Math, "pi").expect("math.pi is registered");
        assert_eq!(sym.module, StdModule::Math);
        assert_eq!(sym.name, "pi");
        assert_eq!(
            sym.kind,
            StdSymbolKind::Constant {
                ty: ScalarKind::Float,
            }
        );
    }

    #[test]
    fn resolve_symbol_rejects_unregistered_symbol_in_registered_module() {
        assert_eq!(resolve_symbol(StdModule::Math, "tan"), None);
        assert_eq!(resolve_symbol(StdModule::Math, "floor"), None);
    }

    #[test]
    fn resolve_symbol_finds_enum_enum() {
        let sym = resolve_symbol(StdModule::Enum, "Enum").expect("enum.Enum is registered");
        assert_eq!(sym.module, StdModule::Enum);
        assert_eq!(sym.name, "Enum");
        assert_eq!(sym.kind, StdSymbolKind::EnumMarker);
    }

    #[test]
    fn resolve_symbol_rejects_unregistered_symbol_in_enum_module() {
        assert_eq!(resolve_symbol(StdModule::Enum, "IntEnum"), None);
        let auto_sym = resolve_symbol(StdModule::Enum, "auto").expect("enum.auto is registered");
        assert_eq!(auto_sym.kind, StdSymbolKind::EnumAutoMarker);
        assert!(format!("{auto_sym:?}").contains("EnumAutoMarker"));
        let str_enum_sym =
            resolve_symbol(StdModule::Enum, "StrEnum").expect("enum.StrEnum is registered");
        assert_eq!(str_enum_sym.kind, StdSymbolKind::EnumMarker);
    }

    #[test]
    fn resolve_symbol_finds_typing_protocol() {
        let sym =
            resolve_symbol(StdModule::Typing, "Protocol").expect("typing.Protocol is registered");
        assert_eq!(sym.module, StdModule::Typing);
        assert_eq!(sym.name, "Protocol");
        assert_eq!(sym.kind, StdSymbolKind::ProtocolMarker);
    }

    #[test]
    fn resolve_symbol_finds_typing_runtime_checkable() {
        let sym = resolve_symbol(StdModule::Typing, "runtime_checkable")
            .expect("typing.runtime_checkable is registered");
        assert_eq!(sym.module, StdModule::Typing);
        assert_eq!(sym.name, "runtime_checkable");
        assert_eq!(sym.kind, StdSymbolKind::DecoratorMarker);
    }

    #[test]
    fn resolve_symbol_rejects_unregistered_symbol_in_typing_module() {
        assert_eq!(resolve_symbol(StdModule::Typing, "Callable"), None);
        assert_eq!(resolve_symbol(StdModule::Typing, "AnyStr"), None);
    }

    const TYPING_FORM_NAMES: [&str; 8] = [
        "Dict",
        "List",
        "Set",
        "FrozenSet",
        "Tuple",
        "Any",
        "Generic",
        "TypeVar",
    ];

    #[test]
    fn resolve_symbol_finds_every_typing_form_marker() {
        for name in TYPING_FORM_NAMES {
            let sym = resolve_symbol(StdModule::Typing, name)
                .unwrap_or_else(|| panic!("typing.{name} is registered"));
            assert_eq!(sym.module, StdModule::Typing);
            assert_eq!(sym.name, name);
            assert_eq!(sym.kind, StdSymbolKind::TypingFormMarker);
        }
    }

    #[test]
    fn resolve_symbol_finds_typing_class_var() {
        let sym =
            resolve_symbol(StdModule::Typing, "ClassVar").expect("typing.ClassVar is registered");
        assert_eq!(sym.module, StdModule::Typing);
        assert_eq!(sym.name, "ClassVar");
        assert_eq!(sym.kind, StdSymbolKind::AnnotationMarker);
    }

    #[test]
    fn resolve_symbol_finds_typing_final() {
        let sym = resolve_symbol(StdModule::Typing, "Final").expect("typing.Final is registered");
        assert_eq!(sym.module, StdModule::Typing);
        assert_eq!(sym.name, "Final");
        assert_eq!(sym.kind, StdSymbolKind::AnnotationMarker);
    }

    #[test]
    fn resolve_symbol_finds_typing_annotated() {
        let sym =
            resolve_symbol(StdModule::Typing, "Annotated").expect("typing.Annotated is registered");
        assert_eq!(sym.module, StdModule::Typing);
        assert_eq!(sym.name, "Annotated");
        assert_eq!(sym.kind, StdSymbolKind::AnnotationMarker);
    }

    #[test]
    fn resolve_symbol_finds_typing_cast() {
        let sym = resolve_symbol(StdModule::Typing, "cast").expect("typing.cast is registered");
        assert_eq!(sym.module, StdModule::Typing);
        assert_eq!(sym.name, "cast");
        assert_eq!(sym.kind, StdSymbolKind::CastMarker);
    }

    #[test]
    fn resolve_symbol_finds_typing_type_checking() {
        let sym = resolve_symbol(StdModule::Typing, "TYPE_CHECKING")
            .expect("typing.TYPE_CHECKING is registered");
        assert_eq!(sym.module, StdModule::Typing);
        assert_eq!(sym.name, "TYPE_CHECKING");
        assert_eq!(sym.kind, StdSymbolKind::TypeCheckingMarker);
    }

    #[test]
    fn resolve_symbol_finds_abc_abc() {
        let sym = resolve_symbol(StdModule::Abc, "ABC").expect("abc.ABC is registered");
        assert_eq!(sym.module, StdModule::Abc);
        assert_eq!(sym.name, "ABC");
        assert_eq!(sym.kind, StdSymbolKind::AbcMarker);
    }

    #[test]
    fn resolve_symbol_finds_abc_abstractmethod() {
        let sym = resolve_symbol(StdModule::Abc, "abstractmethod")
            .expect("abc.abstractmethod is registered");
        assert_eq!(sym.module, StdModule::Abc);
        assert_eq!(sym.name, "abstractmethod");
        assert_eq!(sym.kind, StdSymbolKind::DecoratorMarker);
    }

    #[test]
    fn resolve_symbol_rejects_unregistered_symbol_in_abc_module() {
        assert_eq!(resolve_symbol(StdModule::Abc, "ABCMeta"), None);
        assert_eq!(resolve_symbol(StdModule::Abc, "getset"), None);
    }

    #[test]
    fn std_module_derives_are_exercised() {
        // Exercises `Debug`/`Clone`/`Copy`/`PartialEq`/`Eq` on `StdModule`
        // directly, since `resolve_module`'s equality checks above only
        // exercise `PartialEq` transitively.
        let m = StdModule::Math;
        let m2 = m;
        assert_eq!(format!("{m:?}"), "Math");
        assert_eq!(m, m2);

        let e = StdModule::Enum;
        let e2 = e;
        assert_eq!(format!("{e:?}"), "Enum");
        assert_eq!(e, e2);
        assert_ne!(m, e);

        let t = StdModule::Typing;
        let t2 = t;
        assert_eq!(format!("{t:?}"), "Typing");
        assert_eq!(t, t2);
        assert_ne!(t, e);

        let a = StdModule::Abc;
        let a2 = a;
        assert_eq!(format!("{a:?}"), "Abc");
        assert_eq!(a, a2);
        assert_ne!(a, t);
    }

    #[test]
    fn scalar_kind_and_symbol_derives_are_exercised() {
        let k = ScalarKind::Float;
        let k2 = k;
        assert_eq!(format!("{k:?}"), "Float");
        assert_eq!(k, k2);

        let sym = StdSymbol {
            module: StdModule::Math,
            name: "sqrt",
            kind: StdSymbolKind::Function {
                arg_tys: &[ScalarKind::Float],
                ret_ty: ScalarKind::Float,
            },
        };
        let sym2 = sym;
        assert_eq!(sym, sym2);
        assert!(format!("{sym:?}").contains("StdSymbol"));

        let enum_sym = StdSymbol {
            module: StdModule::Enum,
            name: "Enum",
            kind: StdSymbolKind::EnumMarker,
        };
        let enum_sym2 = enum_sym;
        assert_eq!(enum_sym, enum_sym2);
        assert!(format!("{enum_sym:?}").contains("EnumMarker"));

        let protocol_sym = StdSymbol {
            module: StdModule::Typing,
            name: "Protocol",
            kind: StdSymbolKind::ProtocolMarker,
        };
        let protocol_sym2 = protocol_sym;
        assert_eq!(protocol_sym, protocol_sym2);
        assert!(format!("{protocol_sym:?}").contains("ProtocolMarker"));

        let abc_sym = StdSymbol {
            module: StdModule::Abc,
            name: "ABC",
            kind: StdSymbolKind::AbcMarker,
        };
        let abc_sym2 = abc_sym;
        assert_eq!(abc_sym, abc_sym2);
        assert!(format!("{abc_sym:?}").contains("AbcMarker"));

        let decorator_sym = StdSymbol {
            module: StdModule::Typing,
            name: "runtime_checkable",
            kind: StdSymbolKind::DecoratorMarker,
        };
        let decorator_sym2 = decorator_sym;
        assert_eq!(decorator_sym, decorator_sym2);
        assert!(format!("{decorator_sym:?}").contains("DecoratorMarker"));

        let annotation_sym = StdSymbol {
            module: StdModule::Typing,
            name: "Final",
            kind: StdSymbolKind::AnnotationMarker,
        };
        let annotation_sym2 = annotation_sym;
        assert_eq!(annotation_sym, annotation_sym2);
        assert!(format!("{annotation_sym:?}").contains("AnnotationMarker"));

        let cast_sym = StdSymbol {
            module: StdModule::Typing,
            name: "cast",
            kind: StdSymbolKind::CastMarker,
        };
        let cast_sym2 = cast_sym;
        assert_eq!(cast_sym, cast_sym2);
        assert!(format!("{cast_sym:?}").contains("CastMarker"));

        let type_checking_sym = StdSymbol {
            module: StdModule::Typing,
            name: "TYPE_CHECKING",
            kind: StdSymbolKind::TypeCheckingMarker,
        };
        let type_checking_sym2 = type_checking_sym;
        assert_eq!(type_checking_sym, type_checking_sym2);
        assert!(format!("{type_checking_sym:?}").contains("TypeCheckingMarker"));

        let typing_form_sym = StdSymbol {
            module: StdModule::Typing,
            name: "Dict",
            kind: StdSymbolKind::TypingFormMarker,
        };
        let typing_form_sym2 = typing_form_sym;
        assert_eq!(typing_form_sym, typing_form_sym2);
        assert!(format!("{typing_form_sym:?}").contains("TypingFormMarker"));
    }

    #[test]
    fn module_symbol_names_lists_exactly_one_modules_exports() {
        let math: Vec<_> = module_symbol_names(StdModule::Math).collect();
        assert!(math.contains(&"sqrt"), "{math:?}");
        assert!(
            math.iter()
                .all(|name| resolve_symbol(StdModule::Math, name).is_some()),
            "{math:?}"
        );
        for name in &math {
            assert!(
                REGISTRY
                    .iter()
                    .any(|sym| sym.module == StdModule::Math && sym.name == *name),
                "{name} is not a `math` export"
            );
        }
        let typing: Vec<_> = module_symbol_names(StdModule::Typing).collect();
        assert!(typing.contains(&"cast"), "{typing:?}");
        assert!(!typing.contains(&"sqrt"), "{typing:?}");
        for name in TYPING_FORM_NAMES {
            assert!(typing.contains(&name), "{name} missing from {typing:?}");
        }
    }
}
