//! Unit tests for D-228's bare-container annotation advice (issue #918):
//! which annotation positions upgrade the generic unknown-name `C0001` for a
//! bare `list`/`set`/`dict`/`tuple` into the parameterized-form message.
//!
//! Extracted from `tests.rs` per AGENTS.md's file-decomposition rule when
//! #1266 moved the class-body declaration (`class C: xs: list`) into the
//! advising set. `assert_capability_error_message` stays in the parent and is
//! reached through `use super::*`.
use super::*;

#[test]
fn an_unsupported_annotation_type_returns_a_capability_error() {
    // D-228 (issue #918): `list[int]` now lowers, so a *bare* `list` gets its
    // own message naming the parameterized form to write instead, rather than
    // the generic unknown-name message. `type` keeps the generic one -- it
    // has no `Ty` variant, so there is no parameterized form to suggest
    // (`frozenset` left this position when Part 1 of #1319 gave it one).
    assert_capability_error_message(
        "def f(x: list) -> None:\n    return\n",
        "a bare `list` type annotation is not supported yet -- write the parameterized form, e.g. `list[int]`",
    );
    assert_capability_error_message(
        "def f(x: type) -> None:\n    return\n",
        "type annotation `type` is not supported yet",
    );
}

#[test]
fn each_bare_container_annotation_names_its_own_parameterized_form() {
    // Every arm of `bare_container_example`, including `tuple`'s deliberately
    // two-element suggestion (a one-element example would read as if `tuple`
    // were homogeneous).
    for (bare, example) in [
        ("list", "list[int]"),
        ("set", "set[int]"),
        ("frozenset", "frozenset[int]"),
        ("dict", "dict[str, int]"),
        ("tuple", "tuple[int, int]"),
    ] {
        assert_capability_error_message(
            &format!("def f(x: {bare}) -> None:\n    return\n"),
            &format!(
                "a bare `{bare}` type annotation is not supported yet -- write the parameterized form, e.g. `{example}`"
            ),
        );
    }
}

/// D-228 (issue #918) review finding: the bare-container advice names a form
/// that only some annotation positions accept, so it is opted into by those
/// positions rather than emitted unconditionally by `annotation_to_ty`. This
/// is the affected-site inventory for that split, measured position by
/// position: every position whose parameterized form is rejected must get the
/// generic message instead, or the advice walks the user into a second error.
#[test]
fn the_bare_container_advice_appears_only_where_the_parameterized_form_lowers() {
    const ADVICE: &str = "a bare `list` type annotation is not supported yet -- write the parameterized form, \
         e.g. `list[int]`";
    const GENERIC: &str = "type annotation `list` is not supported yet";

    // `list[int]` lowers here, so the advice is actionable. Return position
    // joined this set in #925 (Part 2 of #918), which removed the
    // return-position gate that once rejected the parameterized form.
    for source in [
        "def f(x: list) -> None:\n    return\n",
        "def f() -> list:\n    return []\n",
        "def f() -> None:\n    xs: list = []\n",
        "xs: list = []\n",
        "type X = list\n",
        "X: TypeAlias = list\n",
        // #1266: a value-less class-body annotation is an instance attribute
        // declaration, which lowers `list[int]`/`dict[str, int]`.
        "class C:\n    xs: list\n",
    ] {
        assert_capability_error_message(source, ADVICE);
    }

    // `list[int]` is rejected here by a `C0001` of its own -- the scalar-slot
    // rule for class constants (a class-body annotation *with* a value) and
    // dataclass fields, and D-228's own protocol-attribute gate -- so the
    // advice would name a form that fails too.
    for source in [
        "class C:\n    xs: list = []\n",
        "from dataclasses import dataclass\n\n\n@dataclass\nclass C:\n    xs: list\n",
        "from typing import Protocol\n\n\nclass P(Protocol):\n    xs: list\n",
    ] {
        assert_capability_error_message(source, GENERIC);
    }
}

/// D-228 (issue #918) round-8 review finding: `annotation_to_ty` lowers
/// `Final[X]` and `Annotated[X, ...]` by recursing into `X`, so the bare-name
/// failure it propagates out of `Final[list]` describes the *inner* name.
/// Matching only the outermost expression dropped the advice in exactly the
/// positions that can act on it -- `Final[list[int]]` is accepted there.
#[test]
fn the_bare_container_advice_survives_transparent_wrappers() {
    const ADVICE: &str = "a bare `list` type annotation is not supported yet -- write the parameterized form, \
         e.g. `list[int]`";

    for source in [
        "from typing import Final\n\nxs: Final[list] = []\n",
        "from typing import Annotated\n\nxs: Annotated[list, \"meta\"] = []\n",
        // A one-element subscript tuple is the same `Final[X]` shape.
        "from typing import Final\n\nxs: Final[list,] = []\n",
        // Wrappers nest, so the peel is recursive.
        "from typing import Annotated, Final\n\nxs: Final[Annotated[list, \"m\"]] = []\n",
        "from typing import Final\n\ndef f(xs: Final[list]) -> None:\n    return\n",
        // Part 1 of #889: a quoted wrapped name gets the same advice, through
        // both the general and the list-or-dict (class declaration) paths.
        "from typing import Final\n\nxs: Final[\"list\"] = []\n",
        "from typing import Annotated\n\nxs: Annotated[\"list\", \"meta\"] = []\n",
        "from typing import Annotated\n\n\nclass C:\n    xs: Annotated[\"list\", \"m\"]\n",
    ] {
        assert_capability_error_message(source, ADVICE);
    }

    // The parameterized form really is accepted through the wrapper, which is
    // what makes the advice actionable rather than a second dead end.
    lower_checked(&pycc_parser_test_helper::parse(
        "from typing import Final\n\nxs: Final[list[int]] = [1]\n",
    ))
    .expect("`Final[list[int]]` lowers in module-variable position");

    // A malformed wrapper keeps its own diagnostic: the peel mirrors
    // `annotation_to_ty`'s arms exactly, so it never reports a wrapper's own
    // arity error against whatever the wrapper happens to contain.
    assert_capability_error_message(
        "from typing import Annotated\n\nxs: Annotated[list] = []\n",
        "Annotated requires at least two arguments: the type and at least one metadata element",
    );
    assert_capability_error_message(
        "from typing import Final\n\nxs: Final[list, int] = []\n",
        "Final takes exactly one type argument",
    );
    // Neither a dotted base nor an unrecognized one is a transparent wrapper.
    assert_capability_error_message(
        "import typing\n\nxs: typing.Final[list] = []\n",
        "a subscripted type annotation's base must be a bare class name",
    );
    assert_capability_error_message(
        "xs: Foo[list] = []\n",
        "type annotation `Foo` is not supported yet",
    );
}
