//! Unit tests for subscripted type annotations (`Base[...]`): acceptance
//! when the base names a type (#1130), the PEP 560 `__class_getitem__` hook
//! return type (#693), `type` alias transparency, and #931's rejection of a
//! subscript on a base that is not a nameable type -- a PEP 695 type
//! parameter, a builtin scalar, `Self`, or a non-class alias.
//!
//! Extracted from `tests.rs` (#663) when #931 touched the block; the
//! `class_with_hook`/`annassign_ty` helpers and every caller moved together.
//! `assert_capability_error_message` stays in the parent and is reached
//! through `use super::*`.
use super::*;
use crate::func::subscripted_base_description;

/// PEP 560 (#611): a class body defining `__class_getitem__` with the
/// given decorator, plus an `__init__` so the class is instantiable.
fn class_with_hook(decorator: &str) -> String {
    // A `@classmethod` hook declares `cls` explicitly, exactly as
    // `pycc_types`' own value-position tests spell it; the
    // `@staticmethod` form does not.
    let cls_param = if decorator == "@classmethod" {
        "cls, "
    } else {
        ""
    };
    format!(
        "class C:\n    {decorator}\n    def __class_getitem__({cls_param}key: int) -> int:\n        return key\n\n    def __init__(self) -> None:\n        self.x = 1\n"
    )
}

#[test]
fn a_subscripted_annotation_on_a_class_defining_the_hook_is_accepted() {
    // A hook-bearing class stays accepted after #1130 removed the gate
    // #611 put here -- the hook is now irrelevant to *whether* `C[int]` is
    // accepted and matters only to what it resolves to (#693, below). Both
    // spellings CPython accepts for the hook -- the explicit
    // `@staticmethod` and the `@classmethod` one -- are checked, mirroring
    // `pycc_types`' own value-position dispatch (#610), which still gates.
    for decorator in ["@staticmethod", "@classmethod"] {
        let src = format!("{}\nv: C[int] = C()\n", class_with_hook(decorator));
        let module = pycc_parser_test_helper::parse(&src);
        assert!(
            lower_checked(&module).is_ok(),
            "`C[int]` must be accepted for a class whose hook is spelled `{decorator}`"
        );
    }
}

#[test]
fn a_subscripted_annotation_on_a_class_inheriting_the_hook_is_accepted() {
    // A hook declared on a base class is reached through the MRO. After
    // #1130 that no longer decides acceptance -- `D[int]` would be accepted
    // with no hook anywhere -- but it still decides the resolved type, so
    // the MRO walk is pinned here and its result in
    // `an_annotation_subscript_on_an_inherited_hook_resolves_through_the_mro`.
    let src = format!(
        "{}\nclass D(C):\n    def value(self) -> int:\n        return self.x\n\nv: D[int] = D()\n",
        class_with_hook("@staticmethod")
    );
    let module = pycc_parser_test_helper::parse(&src);
    assert!(lower_checked(&module).is_ok());
}

#[test]
fn a_subscripted_annotation_on_a_generic_class_is_accepted() {
    // A PEP 695 generic class (`class G[T]:`) declares no
    // `__class_getitem__` of its own -- CPython gives it one implicitly
    // through `Generic`. `G[int]` in an annotation lowered successfully
    // under #611's gate and must keep doing so now that #1130 has removed
    // it; nothing about the generic shape is special any more.
    let module = pycc_parser_test_helper::parse(
        "class G[T]:\n    def __init__(self, v: T) -> None:\n        self.v = v\n\nv: G[int] = G[int](1)\n",
    );
    assert!(lower_checked(&module).is_ok());
}

#[test]
fn a_subscripted_annotation_on_a_class_without_the_hook_is_accepted_with_the_argument_erased() {
    // #1130 reverses #611's annotation-position gate: the same program the
    // gate used to reject now lowers, resolving to `Ty::Instance(D)` with
    // `[int]` erased and never lowered. CPython 3.14 evaluates annotations
    // lazily (PEP 649/749), so no `TypeError` is raised at definition time
    // and pycc builds no runtime `__annotations__` object through which one
    // could surface; the value-position path (#610) keeps its own `T0044`
    // and that asymmetry is CPython's own.
    assert_eq!(
        annassign_ty(
            "class D:\n    def __init__(self) -> None:\n        self.x = 1\n\nv: D[int] = D()\n"
        ),
        Ty::Instance(Box::new("D".to_string()))
    );
}

#[test]
fn a_subscripted_annotation_inside_the_class_s_own_body_resolves_to_the_enclosing_class() {
    // The self-referential `ClassAnnotationInfo` entry `lower_class` pushes
    // is what makes the class's own name resolve inside its own body. #1130
    // removed the subscriptability gate that entry used to feed, so the
    // shape it now guards is the `class_name` carve-out in the subscript
    // arm's direct `class_defs` lookup: `D[int]` inside `D`'s body is the
    // class, with the argument erased.
    let module = pycc_parser_test_helper::parse(
        "class D:\n    def __init__(self) -> None:\n        self.x = 1\n\n    def me(self) -> D[int]:\n        return self\n",
    );
    let hir = lower_checked(&module).expect("a self-referential subscript must lower");
    let return_ty = hir
        .items
        .iter()
        .find_map(|item| match item {
            HirItem::Function {
                name, return_ty, ..
            } if name == "D.me" => Some(return_ty.clone()),
            _ => None,
        })
        .expect("expected `D.me` to lower to an `HirItem::Function`");
    assert_eq!(return_ty, Ty::Instance(Box::new("D".to_string())));
}

#[test]
fn a_subscripted_annotation_inside_a_hooked_class_s_own_body_is_accepted() {
    // The hooked twin of the self-reference test above. The hook is not
    // why this lowers: #1130 deleted the class-body pre-scan
    // (`declares_own_class_getitem`) that used to find it, and the
    // self-referential `ClassAnnotationInfo` entry carries
    // `class_getitem_return: None` unconditionally. It lowers because
    // `annotation_to_ty`'s subscript arm keeps the direct `class_defs`
    // lookup for the enclosing class's own name (the `class_name`
    // carve-out) and then falls through to the bare-name recursion, which
    // resolves `C` through that self-referential entry. What it resolves
    // *to* is pinned in
    // `a_self_referential_annotation_inside_the_hook_s_own_class_body_still_falls_back_to_instance`.
    let module = pycc_parser_test_helper::parse(
        "class C:\n    @staticmethod\n    def __class_getitem__(key: int) -> int:\n        return key\n\n    def __init__(self) -> None:\n        self.x = 1\n\n    def me(self) -> C[int]:\n        return self\n",
    );
    assert!(lower_checked(&module).is_ok());
}

#[test]
fn an_undecorated_class_getitem_does_not_route_the_annotation_through_its_return_type() {
    // pycc's value-position dispatch resolves `__class_getitem__` only
    // through the static-method and class-method tables (#610), so a plain
    // `def __class_getitem__(self)` is an ordinary method and not a PEP 560
    // hook. Since #1130 that no longer decides whether the annotation is
    // *accepted* -- it decides what it resolves to. An undecorated method
    // leaves `class_getitem_return` `None`, so `D[int]` falls through to the
    // bare-name recursion and is the class itself, not the method's `-> int`.
    assert_eq!(
        annassign_ty(
            "class D:\n    def __init__(self) -> None:\n        self.x = 1\n\n    def __class_getitem__(self) -> int:\n        return 1\n\nv: D[int] = D()\n"
        ),
        Ty::Instance(Box::new("D".to_string()))
    );
}

/// Issue #693: lowers `src` (expected to be exactly one top-level
/// `AnnAssign`) and returns the `Ty` it resolved the annotation to, panicking
/// with the full module on any other shape -- mirroring this file's other
/// single-purpose lowering-result extractors.
fn annassign_ty(src: &str) -> Ty {
    let module = pycc_parser_test_helper::parse(src);
    let hir = lower_checked(&module).unwrap_or_else(|e| {
        panic!("expected `{src}` to lower successfully, got {e:?}");
    });
    hir.items
        .iter()
        .find_map(|item| match item {
            HirItem::TopLevelStmt(HirStmt::AnnAssign { annotation, .. }) => {
                Some(annotation.clone())
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("expected exactly one top-level `AnnAssign` in `{src}`"))
}

#[test]
fn an_annotation_subscript_on_a_class_defining_the_hook_resolves_to_the_hook_s_return_type() {
    // Issue #693 (PEP 560): `ClassName[type_arg]` in annotation position
    // used to resolve to `Ty::Instance(ClassName)` unconditionally,
    // discarding the type argument. It must instead route through
    // `__class_getitem__`'s declared return type -- `-> int` here -- exactly
    // as `pycc_types::resolve_static_or_class_method_call` already does for
    // the value-position spelling `C[3]` (#610).
    let src = format!("{}\nv: C[3] = 1\n", class_with_hook("@staticmethod"));
    assert_eq!(annassign_ty(&src), Ty::Int);
}

#[test]
fn an_annotation_subscript_on_a_classmethod_hook_also_resolves_to_the_return_type() {
    // Issue #693: the `@classmethod` spelling of the hook (`cls` as the
    // first parameter) must resolve identically to the `@staticmethod`
    // spelling above -- the two decorator forms are equally valid PEP 560
    // hooks and must not diverge in annotation position.
    let src = format!("{}\nv: C[3] = 1\n", class_with_hook("@classmethod"));
    assert_eq!(annassign_ty(&src), Ty::Int);
}

#[test]
fn an_annotation_subscript_on_an_inherited_hook_resolves_through_the_mro() {
    // Issue #693: `D` defines no `__class_getitem__` of its own but
    // inherits `C`'s through the MRO -- the same inheritance
    // `a_subscripted_annotation_on_a_class_inheriting_the_hook_is_accepted`
    // already exercises; this proves the *resolved type* follows the MRO to
    // `C`'s hook. Since #1130 that is the only thing the MRO walk decides.
    let src = format!(
        "{}\nclass D(C):\n    def value(self) -> int:\n        return self.x\n\nv: D[3] = 1\n",
        class_with_hook("@staticmethod")
    );
    assert_eq!(annassign_ty(&src), Ty::Int);
}

#[test]
fn an_annotation_subscript_prefers_a_base_s_staticmethod_hook_over_a_derived_classmethod_override()
{
    // Issue #693 deep-review, Finding 1: `class_getitem_return_ty` must walk
    // the MRO in the exact same two-pass order as `pycc_types`'
    // `resolve_static_or_class_method_call` (every MRO entry's
    // `static_methods` first, then, only if none declare the hook, every
    // MRO entry's `class_methods`) -- not a single combined pass that lets
    // whichever MRO entry comes first win regardless of decorator kind.
    //
    // `C` declares `__class_getitem__` as a `@staticmethod` returning `int`;
    // `D(C)` overrides it as a `@classmethod` returning `str`. A single
    // combined pass over the MRO (most-derived first: `D`, then `C`) would
    // find `D`'s classmethod entry first and resolve `D[3]` to `str`. The
    // correct two-pass order finds no `static_methods` entry on `D`, then
    // finds `C`'s on the *second* pass over the full MRO -- resolving to
    // `int`, exactly as `pycc_types::resolve_static_or_class_method_call`
    // resolves the value-position `D[3]` for the same hierarchy (see
    // `pycc_types::tests::class_getitem_value_position_prefers_a_base_s_staticmethod_hook_over_a_derived_classmethod_override`).
    let src = "\
class C:
    @staticmethod
    def __class_getitem__(key: int) -> int:
        return key

    def __init__(self) -> None:
        self.x = 1

class D(C):
    @classmethod
    def __class_getitem__(cls, key: int) -> str:
        return \"overridden\"

v: D[3] = 1
";
    assert_eq!(annassign_ty(src), Ty::Int);
}

#[test]
fn a_generic_class_s_annotation_subscript_is_unaffected_by_the_hook_return_type_field() {
    // Issue #693: a PEP 695 generic class (`class G[T]:`) gets its
    // subscript from `Generic`, not from an explicit `__class_getitem__`
    // hook, so `class_getitem_return` must stay `None` for it and `G[int]` must
    // keep resolving to `Ty::Instance(G)` -- the `GenericClassInstantiate`
    // mechanism, not this issue's field, owns actual generic instantiation.
    // Guards against a regression where `type_param.is_some()` alone would
    // be mistaken for "has a resolvable hook return type".
    let src = "class G[T]:\n    def __init__(self, v: T) -> None:\n        self.v = v\n\nv: G[int] = G[int](1)\n";
    assert_eq!(annassign_ty(src), Ty::Instance(Box::new("G".to_string())));
}

#[test]
fn a_self_referential_annotation_inside_the_hook_s_own_class_body_still_falls_back_to_instance() {
    // Issue #693: `lower_class`'s self-referential `ClassAnnotationInfo`
    // entry (pushed before the class's own methods are lowered) cannot yet
    // know `__class_getitem__`'s return type, so a `C[int]` annotation used
    // *inside* `C`'s own body -- the same shape
    // `a_subscripted_annotation_inside_a_hooked_class_s_own_body_is_accepted`
    // already proves is accepted -- keeps resolving to `Ty::Instance(C)`,
    // exactly as it did before this issue. This documents the accepted
    // narrow limitation rather than silently losing coverage of it.
    let module = pycc_parser_test_helper::parse(
        "class C:\n    @staticmethod\n    def __class_getitem__(key: int) -> int:\n        return key\n\n    def __init__(self) -> None:\n        self.x = 1\n\n    def me(self) -> C[int]:\n        return self\n",
    );
    let hir = lower_checked(&module).expect("self-referential annotation must still lower");
    let return_ty = hir
        .items
        .iter()
        .find_map(|item| match item {
            HirItem::Function {
                name, return_ty, ..
            } if name == "C.me" => Some(return_ty.clone()),
            _ => None,
        })
        .expect("expected `C.me` to lower to an `HirItem::Function`");
    assert_eq!(return_ty, Ty::Instance(Box::new("C".to_string())));
}

#[test]
fn an_annotation_subscript_on_a_class_with_an_unannotated_hook_falls_back_to_instance() {
    // Issue #693 review (codex finding): `__class_getitem__` with no
    // explicit return annotation lowers, at this crate's own HIR-lowering
    // time, to a raw `Ty::Infer` placeholder -- this crate never runs its
    // own type-inference pass (see `lower_method`'s doc comment), so the
    // hook's return type is only resolved later, by
    // `pycc_types::check_and_resolve`. `class_getitem_return_ty` must treat
    // that `Ty::Infer` as unresolved rather than propagating the internal
    // placeholder into a resolved annotation type (which previously caused
    // a spurious `T0025` on `x: C[3]`), falling back to
    // `Ty::Instance(ClassName)` exactly as the self-referential and
    // PEP-695-generic cases above already do.
    let src = "class C:\n    @staticmethod\n    def __class_getitem__(key: int):\n        return key\n\n    def __init__(self) -> None:\n        self.x = 1\n\nv: C[3] = C()\n";
    assert_eq!(annassign_ty(src), Ty::Instance(Box::new("C".to_string())));
}

#[test]
fn subscripted_type_annotation_with_non_name_base_is_rejected() {
    // #435 (Part D): a subscripted annotation whose base is not a bare
    // name (e.g. `a.b[int]`) is rejected — only a bare class name is
    // supported as the base of a subscripted type annotation.
    assert_capability_error_message(
        "x: a.b[int] = 1\n",
        "a subscripted type annotation's base must be a bare class name",
    );
}

/// A PEP 695 type parameter named after a builtin container shadows it, the
/// same way a user-defined class or a type alias of that name does. Review
/// finding on #918: the subscript arm consulted `known_class` and the alias
/// table but not `type_param`, so `def f[list](x: list[int])` lowered as the
/// builtin and silently dropped the function's genericity.
///
/// Before #931 the annotation then lowered to `Ty::Param("list")`, discarding
/// the `[int]` argument -- the same silent discard subscripting any type
/// parameter produced. #931 rejects that instead: the shadowing still wins
/// (the diagnostic names a type parameter, not the builtin container), and
/// the subscript is now an error.
#[test]
fn a_type_parameter_shadows_a_builtin_container_of_the_same_name() {
    let diagnostic = first_error("def f[list](x: list[int]) -> None:\n    return\n");
    assert_eq!(diagnostic.code, "T0044");
    assert!(
        diagnostic
            .message
            .contains("type parameter `list` is not subscriptable"),
        "{}",
        diagnostic.message
    );
    // The control: with a type parameter that does *not* shadow it, the same
    // annotation is the builtin container.
    assert_eq!(
        param_ty("def f[T](x: list[int]) -> None:\n    return\n"),
        Ty::List(Box::new(Ty::Int))
    );
}

#[test]
fn a_type_alias_named_list_still_wins_over_the_builtin_container() {
    // The alias table is consulted before the container branch for the same
    // reason: `type list = int` legally shadows the builtin in Python, so
    // `list[int]` must not be reinterpreted as a builtin container here.
    // Before #918 the subscript fell through to the bare-name recursion, the
    // alias resolved it to `int`, and the type argument was silently
    // discarded. The alias still wins -- it just no longer accepts a type
    // argument (#931): an alias to `int` is not subscriptable.
    let module = pycc_parser_test_helper::parse("type list = int\nx: list = 1\n");
    assert!(lower_checked(&module).is_ok());
    let diagnostic = first_error("type list = int\nx: list[int] = 1\n");
    assert_eq!(diagnostic.code, "T0044");
    assert!(
        diagnostic
            .message
            .contains("type alias `list` is not subscriptable"),
        "the alias must win over the builtin container: {}",
        diagnostic.message
    );
}

// ---------------------------------------------------------------------------
// #931: a subscript on a base that is not a class is rejected.
// ---------------------------------------------------------------------------

/// Lowers `source` and returns the diagnostic it must fail with.
fn first_error(source: &str) -> Diagnostic {
    let module = pycc_parser_test_helper::parse(source);
    lower_checked(&module).expect_err("expected the annotation to be rejected")
}

/// Lowers `source` (whose first item must be a function) and returns the
/// `Ty` its first parameter's annotation resolved to.
fn param_ty(source: &str) -> Ty {
    let module = pycc_parser_test_helper::parse(source);
    let hir = lower_checked(&module).unwrap_or_else(|e| {
        panic!("expected `{source}` to lower successfully, got {e:?}");
    });
    let HirItem::Function { params, .. } = &hir.items[0] else {
        panic!("expected a function item for {source:?}");
    };
    params[0].1.clone()
}

/// Asserts `source` fails with the non-class `T0044` whose message contains
/// `noun` and spells `base[...]`, spanning the whole subscript `annotation`.
fn assert_not_subscriptable(source: &str, noun: &str, annotation: &str) {
    let diagnostic = first_error(source);
    assert_eq!(
        diagnostic.code, "T0044",
        "{source:?}: {}",
        diagnostic.message
    );
    let base = annotation.split('[').next().unwrap();
    let expected =
        format!("{noun} is not subscriptable, so `{base}[...]` is not a valid type annotation");
    assert_eq!(diagnostic.message, expected, "{source:?}");
    // D-152: a capability/name-shape rejection with no determinate safe
    // replacement carries no structured help.
    assert_eq!(diagnostic.help, None, "{source:?}");
    let start = source
        .find(annotation)
        .unwrap_or_else(|| panic!("{annotation:?} not in {source:?}"));
    assert_eq!(
        diagnostic.span,
        Some(Span::new(start as u32, (start + annotation.len()) as u32)),
        "{source:?}"
    );
}

#[test]
fn a_subscripted_type_parameter_is_rejected_in_every_position() {
    // Before #931 each of these lowered to `Ty::Param("T")`, silently
    // discarding the type argument(s); CPython raises `TypeError: 'TypeVar'
    // object is not subscriptable`.
    for (source, annotation) in [
        ("def f[T](x: T[int]) -> None:\n    return\n", "T[int]"),
        (
            "def f[T](x: T[int, str, bool]) -> None:\n    return\n",
            "T[int, str, bool]",
        ),
        ("def f[T](x: int) -> T[int]:\n    return x\n", "T[int]"),
    ] {
        assert_not_subscriptable(source, "type parameter `T`", annotation);
    }
}

#[test]
fn a_subscripted_builtin_scalar_is_rejected_in_every_position() {
    for (source, annotation) in [
        ("def f(x: int[str]) -> None:\n    return\n", "int[str]"),
        ("def f(x: int) -> str[int]:\n    return \"a\"\n", "str[int]"),
        ("def f(x: float[int]) -> None:\n    return\n", "float[int]"),
        ("def f(x: bool[int]) -> None:\n    return\n", "bool[int]"),
        ("def f() -> None:\n    y: int[str] = 1\n", "int[str]"),
        ("x: int[str] = 1\n", "int[str]"),
        ("class C:\n    x: int[str] = 1\n", "int[str]"),
        (
            "from dataclasses import dataclass\n\n@dataclass\nclass P:\n    x: int[str]\n",
            "int[str]",
        ),
        (
            "from typing import Protocol\n\nclass P(Protocol):\n    x: int[str]\n",
            "int[str]",
        ),
        (
            "def f(x: list[int[str]]) -> None:\n    return\n",
            "int[str]",
        ),
        ("x: Final[int[str]] = 1\n", "int[str]"),
        (
            "def f(x: int[str] | None) -> None:\n    return\n",
            "int[str]",
        ),
        ("type A = int[str]\n", "int[str]"),
    ] {
        let base = annotation.split('[').next().unwrap();
        assert_not_subscriptable(source, &format!("builtin type `{base}`"), annotation);
    }
}

#[test]
fn a_subscripted_self_inside_a_class_is_rejected() {
    // PEP 673's `Self` names the enclosing class's instance type and takes
    // no type argument -- in a plain class and in a PEP 695 generic one.
    for source in [
        "class C:\n    def __init__(self) -> None:\n        self.v = 1\n\n    def m(self, x: Self[int]) -> int:\n        return 1\n",
        "class G[T]:\n    def __init__(self) -> None:\n        self.v = 1\n\n    def m(self, x: Self[int]) -> int:\n        return 1\n",
    ] {
        assert_not_subscriptable(source, "`Self`", "Self[int]");
    }
}

#[test]
fn a_subscripted_self_outside_a_class_keeps_its_unknown_name_c0001() {
    // Outside a class `Self` is not recognized at all (CPython's own scope
    // rule); the base's C0001 fires before the reject can.
    assert_capability_error_message(
        "def f(x: Self[int]) -> None:\n    return\n",
        "type annotation `Self` is not supported yet",
    );
}

#[test]
fn an_alias_named_self_is_a_type_alias_outside_a_class_and_self_inside_one() {
    // `type Self = int` at module level is an ordinary alias where `Self`
    // has no special meaning, so the noun is "type alias".
    assert_not_subscriptable(
        "type Self = int\n\ndef f(x: Self[int]) -> None:\n    return\n",
        "type alias `Self`",
        "Self[int]",
    );
    // Inside a class the PEP 673 meaning wins over a same-named alias, even
    // an alias to a class: the alias table is not consulted for a name the
    // bare-name arm resolves before it. Before #931 this lowered silently.
    assert_not_subscriptable(
        "class D:\n    def __init__(self) -> None:\n        self.v = 1\n\ntype Self = D\n\nclass C:\n    def __init__(self) -> None:\n        self.v = 1\n\n    def m(self, x: Self[int]) -> int:\n        return 1\n",
        "`Self`",
        "Self[int]",
    );
}

#[test]
fn a_subscripted_alias_to_a_non_class_type_is_rejected() {
    assert_not_subscriptable("type A = int\nx: A[str] = 1\n", "type alias `A`", "A[str]");
}

#[test]
fn a_type_parameter_shadowing_a_class_is_the_type_parameter_in_a_subscript() {
    // The direct `class_defs` lookup is gated on the type parameter: inside
    // `def f[G]`, `G` is the TypeVar, exactly as the bare-name arm resolves
    // it. Before #931 the first case passed the class ladder (`G` is a
    // subscriptable generic class) and then lowered to `Ty::Param("G")`,
    // silently dropping `[int]`; the second reported the class-flavored
    // message with the wrong noun.
    assert_not_subscriptable(
        "class G[U]:\n    def __init__(self) -> None:\n        self.v = 1\n\ndef f[G](x: G[int]) -> int:\n    return 1\n",
        "type parameter `G`",
        "G[int]",
    );
    assert_not_subscriptable(
        "class C:\n    def __init__(self) -> None:\n        self.v = 1\n\ndef f[C](x: C[int]) -> int:\n    return 1\n",
        "type parameter `C`",
        "C[int]",
    );
    // Alias-to-class transparency must not fire for a name the type
    // parameter shadows either (before #931: exit 0).
    assert_not_subscriptable(
        "class C:\n    def __init__(self) -> None:\n        self.v = 1\n\ntype T = C\n\ndef f[T](x: T[int]) -> int:\n    return 1\n",
        "type parameter `T`",
        "T[int]",
    );
}

#[test]
fn an_alias_sharing_a_builtin_scalar_or_any_name_does_not_win_in_a_subscript() {
    // The bare-name arm resolves `int` and `Any` before the alias table, so
    // the subscript arm must not let a same-named alias reach the class
    // ladder. Before #931 `int[str]` here lowered to `Int`, args dropped.
    assert_not_subscriptable(
        "class C:\n    def __init__(self) -> None:\n        self.v = 1\n\ntype int = C\nx: int[str] = 1\n",
        "builtin type `int`",
        "int[str]",
    );
    let diagnostic = first_error(
        "class C:\n    def __init__(self) -> None:\n        self.v = 1\n\ntype Any = C\nx: Any[int] = 1\n",
    );
    assert_eq!(diagnostic.code, "T0002", "{}", diagnostic.message);
}

#[test]
fn an_alias_to_a_class_is_transparent_in_a_subscript() {
    // PEP 695 aliases are transparent: `A[int]` behaves exactly as `C[int]`.
    // A hook class resolves to the hook's declared return type (#693)...
    let source = format!(
        "{}type A = C\nv: A[int] = 1\n",
        class_with_hook("@staticmethod")
    );
    assert_eq!(annassign_ty(&source), Ty::Int);
    // ...a PEP 695 generic class to its instance type...
    assert_eq!(
        param_ty(
            "class G[T]:\n    def __init__(self) -> None:\n        self.v = 1\n\ntype A = G\n\ndef f(x: A[int]) -> int:\n    return 1\n"
        ),
        Ty::Instance(Box::new("G".to_string()))
    );
    // ...and, since #1130, a protocol or a plain hook-less class to that
    // class's own nominal type rather than to a `T0044`. The alias is still
    // transparent: what the subscript resolves to is named after the alias
    // *target*, never after the written base. The third row keeps the
    // #918 precedence point -- `type list = C` beats the builtin container,
    // so `list[int]` is `C` and not `Ty::List(Int)`.
    assert_eq!(
        param_ty(
            "from typing import Protocol\n\nclass P(Protocol):\n    def m(self) -> int: ...\n\ntype A = P\n\ndef f(x: A[int]) -> int:\n    return 1\n"
        ),
        Ty::Protocol(Box::new("P".to_string()))
    );
    for source in [
        "class C:\n    def __init__(self) -> None:\n        self.v = 1\n\ntype A = C\nx: A[int] = C()\n",
        "class C:\n    def __init__(self) -> None:\n        self.v = 1\n\ntype list = C\nx: list[int] = C()\n",
    ] {
        assert_eq!(
            annassign_ty(source),
            Ty::Instance(Box::new("C".to_string())),
            "{source:?}"
        );
    }
}

#[test]
fn the_pre_931_subscript_outcomes_that_must_not_change_are_pinned() {
    // A PEP 695 generic class is still the instance type.
    assert_eq!(
        param_ty(
            "class G[T]:\n    def __init__(self) -> None:\n        self.v = 1\n\ndef f(x: G[int]) -> int:\n    return 1\n"
        ),
        Ty::Instance(Box::new("G".to_string()))
    );
    // A known hook-less class is the one row #1130 deliberately moves: it
    // used to carry the class-flavored `T0044` and is now the class itself.
    // Every other row below is untouched, which is what makes this table the
    // guard that #931 is not being reverted wholesale.
    assert_eq!(
        annassign_ty(
            "class C:\n    def __init__(self) -> None:\n        self.v = 1\n\nx: C[int] = C()\n"
        ),
        Ty::Instance(Box::new("C".to_string()))
    );
    // An undefined base keeps the exact C0001 `module::cascade_name` parses
    // back (D-219).
    assert_capability_error_message(
        "x: Foo[int] = 1\n",
        "type annotation `Foo` is not supported yet",
    );
    // `Any` keeps T0002.
    assert_eq!(first_error("x: Any[int] = 1\n").code, "T0002");
    // A builtin container with a type parameter that does not shadow it.
    assert_eq!(
        param_ty("def f[T](x: list[int]) -> None:\n    return\n"),
        Ty::List(Box::new(Ty::Int))
    );
}

#[test]
fn a_subscripted_type_parameter_inside_a_container_is_rejected_before_the_t0042_element_gate() {
    // Before #931 the element lowered to `Ty::Param("T")` and the container
    // element gate reported `T0042`; now the inner `T[int]` is rejected
    // first. Bare `list[T]` is unaffected and still `T0042`.
    assert_not_subscriptable(
        "def f[T](x: list[T[int]]) -> None:\n    return\n",
        "type parameter `T`",
        "T[int]",
    );
}

#[test]
fn subscripted_base_description_follows_the_bare_name_arm_s_precedence() {
    // Direct unit test of every arm, including the defensive `class_name`
    // arm that lowering never reaches (the self-referential `class_defs`
    // entry catches the class's own name first) but D-014 still requires.
    assert_eq!(
        subscripted_base_description("T", Some("T"), None),
        "type parameter `T`"
    );
    // The type parameter wins over every later arm, even for `Self`.
    assert_eq!(
        subscripted_base_description("Self", Some("Self"), Some("C")),
        "type parameter `Self`"
    );
    assert_eq!(
        subscripted_base_description("Self", None, Some("C")),
        "`Self`"
    );
    assert_eq!(
        subscripted_base_description("Self", None, None),
        "type alias `Self`"
    );
    assert_eq!(
        subscripted_base_description("C", None, Some("C")),
        "class `C`"
    );
    for scalar in ["int", "float", "bool", "str"] {
        assert_eq!(
            subscripted_base_description(scalar, Some("T"), Some("C")),
            format!("builtin type `{scalar}`")
        );
    }
    assert_eq!(
        subscripted_base_description("A", Some("T"), Some("C")),
        "type alias `A`"
    );
}
