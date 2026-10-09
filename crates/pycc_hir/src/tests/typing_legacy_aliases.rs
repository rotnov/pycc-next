//! #1378 (Part 6 of #882): the pre-PEP 585 `typing` container aliases
//! `Dict`, `List`, `Set`, `FrozenSet` and `Tuple` in annotation position.
//! Each lowers to exactly the `Ty` of the builtin container it aliases,
//! through the same guards (a PEP 695 type parameter, a `type` alias or a
//! user class of the same name still wins); the arity and `...` messages
//! name the spelling as written, while the shared element-capability gate
//! names the canonical `Ty`.
use super::*;

/// Each legacy spelling paired with its builtin one. Every pair has the
/// same length, so the lowered modules -- spans included -- are equal.
const PAIRS: [(&str, &str); 5] = [
    ("Dict[str, int]", "dict[str, int]"),
    ("List[int]", "list[int]"),
    ("Set[int]", "set[int]"),
    ("FrozenSet[int]", "frozenset[int]"),
    ("Tuple[int, bool]", "tuple[int, bool]"),
];

fn lower(source: &str) -> HirModule {
    lower_checked(&pycc_parser_test_helper::parse(source))
        .unwrap_or_else(|error| panic!("expected {source:?} to lower, got {error:?}"))
}

fn lower_err(source: &str) -> Diagnostic {
    lower_checked(&pycc_parser_test_helper::parse(source))
        .expect_err("expected the annotation to be rejected")
}

#[test]
fn each_legacy_alias_lowers_to_the_builtin_containers_ty_in_every_position() {
    for (legacy, builtin) in PAIRS {
        for template in [
            // Parameter.
            "def f(x: {}) -> None:\n    return\n",
            // Return.
            "def f() -> {}:\n    return f()\n",
            // Local annotated assignment.
            "def f() -> None:\n    x: {} = f()\n    return\n",
            // Module-level annotated assignment, through `Final`.
            "def f() -> None:\n    return\n\nx: Final[{}] = f()\n",
            // A PEP 695 `type` alias whose target is the spelling.
            "type A = {}\n\ndef f(x: A) -> None:\n    return\n",
        ] {
            let legacy_source = template.replace("{}", legacy);
            let builtin_source = template.replace("{}", builtin);
            assert_eq!(
                lower(&legacy_source),
                lower(&builtin_source),
                "{legacy_source:?}"
            );
        }
    }
    // A class-body instance attribute declaration lowers only `list[int]` and
    // `dict[str, int]` (#1266), so only those two spellings are compared.
    // The #1264 annotated attribute target admits the same two, each with its
    // empty literal (`EMPTY`) as the value.
    for (legacy, builtin, empty) in [
        ("List[int]", "list[int]", "[]"),
        ("Dict[str, int]", "dict[str, int]", "{}"),
    ] {
        for template in [
            "class C:\n    xs: ANN\n\n    def __init__(self, xs: ANN) -> None:\n        self.xs = xs\n",
            "class C:\n    def __init__(self) -> None:\n        self.xs: ANN = EMPTY\n",
        ] {
            let template = template.replace("EMPTY", empty);
            assert_eq!(
                lower(&template.replace("ANN", legacy)),
                lower(&template.replace("ANN", builtin)),
                "{legacy} in {template:?}"
            );
        }
    }
}

#[test]
fn the_typing_import_line_of_the_subject_module_lowers() {
    // The line #1378 exists for (lark `lalr_parser_state.py`, line 2): with
    // the import, the annotations lower exactly as they do without it.
    let source = "from typing import Dict, Any, Generic, List\n\n\
                  def f(d: Dict[str, int]) -> List[int]:\n    return [1]\n";
    let with_import = lower(source);
    let without_import = lower("def f(d: dict[str, int]) -> list[int]:\n    return [1]\n");
    assert_eq!(with_import.items, without_import.items);
}

#[test]
fn a_nested_legacy_alias_gets_the_builtin_spellings_diagnostic() {
    let legacy = lower_err("def f(x: Dict[str, List[int]]) -> None:\n    return\n");
    let builtin = lower_err("def f(x: dict[str, list[int]]) -> None:\n    return\n");
    assert_eq!(legacy, builtin);
}

#[test]
fn the_arity_and_ellipsis_messages_name_the_spelling_as_written() {
    for (source, message, help) in [
        (
            "def f(x: Dict[str]) -> None:\n    return\n",
            "container type annotation `Dict[...]` takes exactly 2 type arguments, got 1",
            "write exactly 2 type arguments, e.g. `Dict[str, int]`",
        ),
        (
            "def f(x: List[int, int]) -> None:\n    return\n",
            "container type annotation `List[...]` takes exactly 1 type argument, got 2",
            "write exactly 1 type argument, e.g. `List[int]`",
        ),
        (
            "def f(x: Tuple[int, ...]) -> None:\n    return\n",
            "the `...` type argument in `Tuple[...]` is not supported yet -- a homogeneous-variadic container has no compile-time length, so write an explicit fixed-arity annotation such as `Tuple[int, int]` instead",
            "write an explicit fixed-arity annotation such as `Tuple[int, int]`",
        ),
        (
            "def f(x: List[...]) -> None:\n    return\n",
            "the `...` type argument in `List[...]` is not supported yet -- `...` is not a type argument here; write the element type, e.g. `List[int]`",
            "write the element type, e.g. `List[int]`",
        ),
        (
            "def f(x: Tuple[()]) -> None:\n    return\n",
            "container type annotation `Tuple[...]` takes at least 1 type argument -- the empty tuple `Tuple[()]` is not supported yet",
            "write at least one element type, e.g. `Tuple[int]`",
        ),
    ] {
        let diagnostic = lower_err(source);
        assert_eq!(diagnostic.code, "T0053", "{source:?}");
        assert_eq!(diagnostic.message, message, "{source:?}");
        assert_eq!(diagnostic.help.as_deref(), Some(help), "{source:?}");
    }
}

#[test]
fn the_element_capability_gate_names_the_canonical_ty() {
    // The deliberate split: the gate runs on the lowered `Ty`, shared with
    // container literals, so it prints `dict[...]` for a `Dict[...]`.
    let diagnostic = lower_err("def f(x: Dict[int, int]) -> None:\n    return\n");
    assert_eq!(diagnostic.code, "T0036");
    assert_eq!(
        diagnostic.message,
        "dict[int, int] is not compiled yet (D-122) -- only dict[str, int] is"
    );
}

#[test]
fn a_type_parameter_inside_a_legacy_alias_is_t0042() {
    let diagnostic = lower_err("def f[T](x: List[T]) -> None:\n    return\n");
    assert_eq!(diagnostic.code, "T0042");
}

#[test]
fn a_user_class_named_like_a_legacy_alias_wins() {
    let hir = lower(
        "class Dict:\n    def __init__(self) -> None:\n        self.v = 0\n\n\
         def f(x: Dict[int, int]) -> None:\n    return\n",
    );
    let param_ty = hir
        .items
        .iter()
        .find_map(|item| match item {
            HirItem::Function { name, params, .. } if name == "f" => Some(params[0].1.clone()),
            _ => None,
        })
        .expect("`f` lowers to a function item");
    assert_eq!(param_ty, Ty::Instance(Box::new("Dict".to_string())));
}

#[test]
fn a_type_alias_named_like_a_legacy_alias_wins() {
    // The alias path, not the container one: an alias to a scalar is not
    // subscriptable (#931).
    let diagnostic = lower_err("type List = int\ndef f(x: List[int]) -> None:\n    return\n");
    assert_eq!(diagnostic.code, "T0044");
    assert!(
        diagnostic.message.contains("type alias `List`"),
        "{}",
        diagnostic.message
    );
}

#[test]
fn a_type_parameter_named_like_a_legacy_alias_wins() {
    let diagnostic = lower_err("def f[Dict](x: Dict[int]) -> None:\n    return\n");
    assert_eq!(diagnostic.code, "T0044");
    assert!(
        diagnostic.message.contains("type parameter `Dict`"),
        "{}",
        diagnostic.message
    );
}

#[test]
fn a_bare_legacy_alias_gets_the_parameterized_form_advice() {
    for (bare, example) in [
        ("List", "List[int]"),
        ("Set", "Set[int]"),
        ("FrozenSet", "FrozenSet[int]"),
        ("Dict", "Dict[str, int]"),
        ("Tuple", "Tuple[int, int]"),
    ] {
        assert_capability_error_message(
            &format!("def f(x: {bare}) -> None:\n    return\n"),
            &format!(
                "a bare `{bare}` type annotation is not supported yet -- write the parameterized form, e.g. `{example}`"
            ),
        );
    }
    // The two positions that lower only `list[int]`/`dict[str, int]` advise
    // `List`/`Dict` too, and keep the generic message for `Set`/`Tuple`.
    for (bare, expected) in [
        (
            "List",
            "a bare `List` type annotation is not supported yet -- write the parameterized form, e.g. `List[int]`",
        ),
        (
            "Dict",
            "a bare `Dict` type annotation is not supported yet -- write the parameterized form, e.g. `Dict[str, int]`",
        ),
        ("Set", "type annotation `Set` is not supported yet"),
        ("Tuple", "type annotation `Tuple` is not supported yet"),
    ] {
        assert_capability_error_message(&format!("class C:\n    xs: {bare}\n"), expected);
        assert_capability_error_message(
            &format!(
                "class C:\n    def __init__(self) -> None:\n        self.xs: {bare} = f()\n\ndef f() -> int:\n    return 1\n"
            ),
            expected,
        );
    }
}

#[test]
fn an_aliased_typing_from_import_is_still_refused() {
    let diagnostic = lower_err("from typing import Dict as D\n");
    assert_eq!(diagnostic.code, "C0001");
    assert_eq!(
        diagnostic.message,
        "`from ... import x as y` aliasing is not supported yet"
    );
}

#[test]
fn a_plain_generic_base_keeps_a_c0001() {
    // A bare `Generic` base names no type variable: #1394 admits only
    // `Generic[T, ...]`, so the plain form keeps an explicit refusal.
    let diagnostic = lower_err("from typing import Generic\n\n\nclass X(Generic):\n    pass\n");
    assert_eq!(diagnostic.code, "C0001");
    assert_eq!(
        diagnostic.message,
        "class `X` lists a plain `Generic` base -- write `Generic[T]` with the class's type \
         variables"
    );
}

#[test]
fn a_generic_base_over_a_concrete_type_keeps_its_c0001() {
    // `Generic[...]` is admitted only over type variables (#1394);
    // `Generic[int]` is a TypeError in CPython and stays refused here.
    let diagnostic =
        lower_err("from typing import Generic\n\n\nclass X(Generic[int]):\n    pass\n");
    assert_eq!(diagnostic.code, "C0001");
    assert!(
        diagnostic
            .message
            .starts_with("`int` in a `Generic[...]` base is not a type variable"),
        "{}",
        diagnostic.message
    );
}

#[test]
fn a_legacy_alias_poisoned_by_a_failed_import_still_resolves_by_its_spelling() {
    // `Callable` is unregistered, so the whole `from typing import` fails and
    // poisons its names; `List` still lowers by spelling (as `Final` does),
    // so the only diagnostic is the import's own `C0002`.
    // The bare `List` is refused with the parameterized-form advice, and that
    // refusal names a poisoned name, so the cascade suppresses it too.
    for source in [
        "from typing import Callable, List\n\nx: List[int] = [1]\n",
        "from typing import Callable, List\n\nx: List = [1]\n",
    ] {
        let diagnostics = lower_module(
            &pycc_parser_test_helper::parse(source),
            &ResolvedImports::default(),
            None,
        )
        .expect_err("the `Callable` import fails");
        assert_eq!(diagnostics.len(), 1, "{source:?}: {diagnostics:#?}");
        assert_eq!(diagnostics[0].code, "C0002");
        assert!(
            diagnostics[0].message.contains("`Callable`"),
            "{}",
            diagnostics[0].message
        );
    }
}
