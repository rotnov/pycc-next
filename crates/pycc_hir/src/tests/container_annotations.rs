//! Class-body attributes and parameterized container annotations.

use super::*;

// -- #911 (Part 1 of #885): class-level attributes at the lowering seam ----
//
// The end-to-end suite (`tests/issue_911_class_attrs.rs`) drives these same
// shapes through the compiled `pycc` binary. These crate-local tests pin the
// same behavior directly at `lower_checked`, the seam that actually owns it:
// `class::body`'s `AnnAssign` arm, `func::annotation_to_ty`'s two `ClassVar`
// rejections, and `lower_class`'s post-walk collision reconciliation.

#[test]
fn an_annotated_class_body_attribute_lowers_to_a_class_attr() {
    let module = pycc_parser_test_helper::parse(
        "class C:\n    X: int = 1\n    K: str = \"a\"\n    S: float = -1.5\n    D: bool = True\n\n    def __init__(self) -> None:\n        self.n = 0\n",
    );
    let hir = lower_checked(&module).expect("the class body must lower");
    let (_, class_def) = hir
        .class_defs
        .iter()
        .find(|(name, _)| name == "C")
        .expect("class `C` must be lowered");

    assert_eq!(
        class_def.class_attrs,
        vec![
            ("X".to_string(), Ty::Int, ClassAttrValue::Int(1)),
            (
                "K".to_string(),
                Ty::Str,
                ClassAttrValue::Str("a".to_string())
            ),
            ("S".to_string(), Ty::Float, ClassAttrValue::Float(-1.5)),
            ("D".to_string(), Ty::Bool, ClassAttrValue::Bool(true)),
        ]
    );
    // D-154: a class attribute is a compile-time constant, so it must never
    // appear among the instance attribute slots.
    assert_eq!(class_def.attrs, vec![("n".to_string(), Ty::Int)]);
}

#[test]
fn a_class_var_wrapped_class_body_attribute_lowers_to_the_same_class_attr() {
    let module = pycc_parser_test_helper::parse(
        "from typing import ClassVar\n\n\nclass C:\n    X: ClassVar[int] = 7\n\n    def __init__(self) -> None:\n        self.n = 0\n",
    );
    let hir = lower_checked(&module).expect("the class body must lower");
    let (_, class_def) = hir
        .class_defs
        .iter()
        .find(|(name, _)| name == "C")
        .expect("class `C` must be lowered");

    assert_eq!(
        class_def.class_attrs,
        vec![("X".to_string(), Ty::Int, ClassAttrValue::Int(7))]
    );
}

#[test]
fn a_bare_class_var_class_body_annotation_propagates_the_strip_error() {
    // `strip_class_var`'s own error propagating out of the `AnnAssign` arm,
    // before the annotation is ever resolved.
    assert_capability_error_message(
        "class C:\n    X: ClassVar = 1\n\n    def __init__(self) -> None:\n        self.n = 0\n",
        "a bare `ClassVar` is not a valid annotation",
    );
}

#[test]
fn a_class_var_annotation_outside_a_class_body_is_rejected_at_lowering() {
    // Both `annotation_to_ty` arms: the bare name and the subscripted form.
    assert_capability_error_message(
        "def f(x: ClassVar) -> int:\n    return 1\n",
        "only valid on a class-body attribute declaration",
    );
    assert_capability_error_message(
        "def f(x: ClassVar[int]) -> int:\n    return 1\n",
        "only valid on a class-body attribute declaration",
    );
}

#[test]
fn a_class_attribute_colliding_with_an_instance_slot_is_rejected_at_lowering() {
    // `lower_class`'s post-walk `reject_class_attr_collisions` call, in both
    // declaration orders -- the check deliberately does not run at the
    // `AnnAssign` site, where `attrs` is still empty.
    assert_capability_error_message(
        "class C:\n    x: int = 1\n\n    def __init__(self) -> None:\n        self.x = 2\n",
        "collides with an instance attribute",
    );
    assert_capability_error_message(
        "class C:\n    def __init__(self) -> None:\n        self.x = 2\n\n    x: int = 1\n",
        "collides with an instance attribute",
    );
}

#[test]
fn a_derived_class_attribute_with_no_mro_collision_lowers_at_the_hir_seam() {
    // The "this base is clean, keep walking" path through
    // `reject_class_attr_collisions`: every rejection test returns on the
    // first base it inspects, so none of them completes a loop iteration.
    let module = pycc_parser_test_helper::parse(
        "class Base:\n    def __init__(self) -> None:\n        self.n = 3\n\n    @property\n    def twice(self) -> int:\n        return self.n * 2\n\n\nclass Derived(Base):\n    LIMIT: int = 7\n\n    def __init__(self) -> None:\n        self.n = 5\n",
    );
    let hir = lower_checked(&module).expect("the derived class must lower");
    let (_, derived) = hir
        .class_defs
        .iter()
        .find(|(name, _)| name == "Derived")
        .expect("class `Derived` must be lowered");

    assert_eq!(
        derived.class_attrs,
        vec![("LIMIT".to_string(), Ty::Int, ClassAttrValue::Int(7))]
    );
}

// --- D-228 / issue #918: parameterized container annotations ---------------

/// Lowers `source` and returns the single diagnostic it must fail with.
fn container_annotation_err(source: &str) -> Diagnostic {
    let module = pycc_parser_test_helper::parse(source);
    lower_checked(&module).expect_err("expected the container annotation to be rejected")
}

#[test]
fn a_parameterized_container_annotation_lowers_in_every_supported_position() {
    // The §6 positions Part 1 covers, exercised through the real lowering
    // entry point: a parameter, a module-level `AnnAssign`, a local
    // `AnnAssign` and a PEP 695 `type` alias. A protocol *attribute*'s type
    // is *not* one of them: it is rejected with `C0001` (D-228 decision 10).
    // A protocol *method*'s parameter is, and is covered by its own test
    // below.
    for source in [
        "def f(x: list[int]) -> None:\n    return\n",
        "def f(x: dict[str, int]) -> None:\n    return\n",
        "def f(x: set[int]) -> None:\n    return\n",
        "def f(x: tuple[int, bool, float]) -> None:\n    return\n",
        "x: list[int] = [1]\n",
        "def f() -> None:\n    y: dict[str, int] = {\"a\": 1}\n    return\n",
        "type Ints = list[int]\ndef f(x: Ints) -> None:\n    return\n",
    ] {
        let module = pycc_parser_test_helper::parse(source);
        assert!(lower_checked(&module).is_ok(), "should lower: {source:?}");
    }
}

#[test]
fn a_container_annotation_reports_a_later_unlowerable_argument_before_an_earlier_element_gate() {
    // The annotation path's ordering is the mirror image of the tuple-literal
    // path's (D-228 decision 4): here every type argument is lowered by
    // `annotation_to_ty` first, and `check_container_ty` only runs afterwards
    // as a whole-`Ty` postcheck. So a *later* argument that cannot be lowered
    // at all wins over an *earlier* argument's element-type gate, while the
    // literal path reports the earlier element's `T0039` first (pinned by
    // `a_tuple_literal_reports_an_earlier_element_s_t0039_before_a_later_undefined_name`
    // in `pycc_types`). Both orderings are pinned so that folding either
    // entry point into the other becomes a visible test failure.
    let err =
        container_annotation_err("def f(x: tuple[int, str, Undefined]) -> None:\n    return\n");
    assert_eq!(err.code, "C0001");
    assert_eq!(
        err.message,
        "type annotation `Undefined` is not supported yet"
    );
}

#[test]
fn a_container_annotation_validates_its_arity_before_its_element_types() {
    // `T0053` is an arity check that runs first, so a wrong-arity annotation
    // never reports a misleading element-type diagnostic.
    for (source, expected, expected_help) in [
        (
            "def f(x: list[int, str]) -> None:\n    return\n",
            "container type annotation `list[...]` takes exactly 1 type argument, got 2",
            "write exactly 1 type argument, e.g. `list[int]`",
        ),
        (
            "def f(x: set[int, int]) -> None:\n    return\n",
            "container type annotation `set[...]` takes exactly 1 type argument, got 2",
            "write exactly 1 type argument, e.g. `set[int]`",
        ),
        (
            "def f(x: dict[int]) -> None:\n    return\n",
            "container type annotation `dict[...]` takes exactly 2 type arguments, got 1",
            "write exactly 2 type arguments, e.g. `dict[str, int]`",
        ),
        (
            "def f(x: dict[int, str, bool]) -> None:\n    return\n",
            "container type annotation `dict[...]` takes exactly 2 type arguments, got 3",
            "write exactly 2 type arguments, e.g. `dict[str, int]`",
        ),
        (
            "def f(x: list[()]) -> None:\n    return\n",
            "container type annotation `list[...]` takes exactly 1 type argument, got 0",
            "write exactly 1 type argument, e.g. `list[int]`",
        ),
        (
            "def f(x: tuple[()]) -> None:\n    return\n",
            "container type annotation `tuple[...]` takes at least 1 type argument -- the empty tuple `tuple[()]` is not supported yet",
            "write at least one element type, e.g. `tuple[int]`",
        ),
    ] {
        let diagnostic = container_annotation_err(source);
        assert_eq!(diagnostic.code, "T0053", "{source:?}");
        assert_eq!(diagnostic.message, expected, "{source:?}");
        assert!(diagnostic.span.is_some(), "{source:?}");
        // `docs/DIAGNOSTICS.md`'s quality bar publishes structured help for
        // the arity family (D-152); the human format never renders it, so it
        // is pinned here as well as in `tests/diagnostics/*.expected.json`.
        assert_eq!(
            diagnostic.help.as_deref(),
            Some(expected_help),
            "{source:?}"
        );
    }
}

#[test]
fn a_failing_inner_annotation_propagates_out_of_the_container_position() {
    // The recursive `annotation_to_ty(arg, ..)?` inside
    // `container_annotation_to_ty` is the one error path in that function
    // reached by neither the `T0053` arity check that runs before it nor the
    // `T0042`/element-type gates that run after it: the inner annotation is
    // not lowerable at all. The inner diagnostic is propagated verbatim,
    // carrying the *inner* annotation's own span rather than the whole
    // subscript's, so the caret points at the part the user must change.
    for (source, expected) in [
        (
            // The generic message, not the bare-container advice: a container
            // *element* is one of the positions that rejects the
            // parameterized form (`list[list[int]]` is `T0034`), so it does
            // not opt into `with_bare_container_advice`. The parameter
            // position's own upgrade cannot reach this diagnostic either --
            // its annotation is the whole `list[list]` subscript, not a bare
            // name.
            "def f(x: list[list]) -> None:\n    return\n",
            "type annotation `list` is not supported yet",
        ),
        (
            "def f(x: dict[str, 3]) -> None:\n    return\n",
            "only a bare name type annotation is supported so far, got a number literal",
        ),
    ] {
        let diagnostic = container_annotation_err(source);
        assert_eq!(diagnostic.code, "C0001", "{source:?}");
        assert_eq!(diagnostic.message, expected, "{source:?}");
        let span = diagnostic.span.expect("inner annotation span");
        let inner_start = source.rfind("[").expect("subscript") as u32 + 1;
        assert!(
            span.start >= inner_start,
            "span should start inside the subscript, not at the container name: {source:?}"
        );
    }
}

#[test]
fn a_variadic_ellipsis_type_argument_is_rejected_in_every_family() {
    // The `...` path becomes reachable for the first time with #918, so its
    // text is pinned rather than assumed to be covered elsewhere. Checked
    // ahead of arity so `tuple[int, ...]` (arity 2, which `tuple` would
    // otherwise accept) reports the variadic form specifically.
    //
    // The advice is per family, and that is the point of covering all four
    // here: `tuple[X, ...]` is the one spelling that means something in
    // Python, so only it gets the compile-time-length explanation and a
    // fixed-arity `tuple`. Recommending a `tuple` for a `list`/`set`/`dict`
    // would answer a question the user did not ask -- it changes the
    // container rather than correcting its type arguments (review finding on
    // this pull request).
    for (source, expected, expected_help) in [
        (
            "def f(x: tuple[int, ...]) -> None:\n    return\n",
            "the `...` type argument in `tuple[...]` is not supported yet -- a homogeneous-variadic container has no compile-time length, so write an explicit fixed-arity annotation such as `tuple[int, int]` instead",
            "write an explicit fixed-arity annotation such as `tuple[int, int]`",
        ),
        (
            "def f(x: list[...]) -> None:\n    return\n",
            "the `...` type argument in `list[...]` is not supported yet -- `...` is not a type argument here; write the element type, e.g. `list[int]`",
            "write the element type, e.g. `list[int]`",
        ),
        (
            "def f(x: set[...]) -> None:\n    return\n",
            "the `...` type argument in `set[...]` is not supported yet -- `...` is not a type argument here; write the element type, e.g. `set[int]`",
            "write the element type, e.g. `set[int]`",
        ),
        (
            "def f(x: dict[str, ...]) -> None:\n    return\n",
            "the `...` type argument in `dict[...]` is not supported yet -- `...` is not a type argument here; write the element type, e.g. `dict[str, int]`",
            "write the element type, e.g. `dict[str, int]`",
        ),
    ] {
        let diagnostic = container_annotation_err(source);
        assert_eq!(diagnostic.code, "T0053", "{source:?}");
        assert_eq!(diagnostic.message, expected, "{source:?}");
        // The per-family imperative is also the structured `help` (D-152), so
        // a JSON consumer reads the fix rather than only the prose sentence.
        assert_eq!(
            diagnostic.help.as_deref(),
            Some(expected_help),
            "{source:?}"
        );
    }
}

#[test]
fn a_container_annotation_runs_the_same_element_gate_as_a_container_literal() {
    for (source, code, message) in [
        (
            "def f(x: list[None]) -> None:\n    return\n",
            "T0034",
            "list[None] is not compiled yet -- a list element must be int, float, bool, str, a user-class instance, or a list, tuple, dict or set of those",
        ),
        (
            "def f(x: dict[list[int], int]) -> None:\n    return\n",
            "T0036",
            "dict[list[int], int] is not compiled yet -- a dict key must be int, float, bool, str or a tuple of those, and a value anything a list can hold",
        ),
        (
            "def f(x: set[list[int]]) -> None:\n    return\n",
            "T0038",
            "set[list[int]] is not compiled yet -- a set element must be int, float, bool, str, a tuple of those or a user-class instance",
        ),
        (
            "def f(x: tuple[int, None]) -> None:\n    return\n",
            "T0039",
            "tuple element type `None` is not compiled yet -- a tuple element must be int, float, bool, str, a user-class instance, or a list, tuple, dict or set of those",
        ),
        (
            "def f(x: list[list[None]]) -> None:\n    return\n",
            "T0034",
            "list[None] is not compiled yet -- a list element must be int, float, bool, str, a user-class instance, or a list, tuple, dict or set of those",
        ),
    ] {
        let diagnostic = container_annotation_err(source);
        assert_eq!(diagnostic.code, code, "{source:?}");
        assert_eq!(diagnostic.message, message, "{source:?}");
        // Unlike the literal call sites, which keep `Span::new(0, 0)`, the
        // annotation path carries the annotation's real range.
        assert_ne!(diagnostic.span, Some(Span::new(0, 0)), "{source:?}");
    }
}

#[test]
fn a_type_parameter_inside_a_container_annotation_is_rejected_with_a_real_span() {
    // D-133/D-134: `substitute_ty` is not recursive, so a `Ty::Param` nested
    // inside a container would never be substituted at a call site. Rejected
    // here, in `pycc_hir`, with `pycc_types`' own `T0042` wording but a real
    // span rather than that scan's `Span::new(0, 0)`.
    let diagnostic = container_annotation_err("def f[T](x: list[T]) -> None:\n    return\n");
    assert_eq!(diagnostic.code, "T0042");
    assert_eq!(
        diagnostic.message,
        "type parameter `T` used inside a container position is not supported yet -- v0.2 only instantiates a bare type-parameter position, matching D-105's own fixed-container-element-type restriction"
    );
    assert_ne!(diagnostic.span, Some(Span::new(0, 0)));
}

#[test]
fn a_container_return_annotation_lowers_in_every_family() {
    // #925 (Part 2 of #918) removed D-228's return-position `C0001`: a
    // container-typed call result no longer reaches an unhandled codegen
    // case (`crates/pycc_codegen/src/call_result.rs` carries an arm per
    // family), so the annotation lowers to the container `Ty` itself.
    for (family, expected) in [
        ("list[int]", Ty::List(Box::new(Ty::Int))),
        ("dict[str, int]", Ty::Dict(Box::new((Ty::Str, Ty::Int)))),
        ("set[int]", Ty::Set(Box::new(Ty::Int))),
        // Part 1 of #1319.
        ("frozenset[int]", Ty::FrozenSet(Box::new(Ty::Int))),
        (
            "tuple[int, int]",
            Ty::Tuple(Box::new(vec![Ty::Int, Ty::Int])),
        ),
    ] {
        let module =
            pycc_parser_test_helper::parse(&format!("def f() -> {family}:\n    return f()\n"));
        let hir = lower_checked(&module).unwrap_or_else(|error| {
            panic!("`-> {family}` must lower: {error:?}");
        });
        let HirItem::Function { return_ty, .. } = &hir.items[0] else {
            panic!("expected a function item for {family}");
        };
        assert_eq!(*return_ty, expected, "{family}");
    }
    // Reached through a type alias too, not only a literal annotation.
    let module =
        pycc_parser_test_helper::parse("type Ints = list[int]\ndef f() -> Ints:\n    return f()\n");
    let hir = lower_checked(&module).expect("an aliased container return type must lower");
    let HirItem::Function { return_ty, .. } = hir
        .items
        .iter()
        .find(|item| matches!(item, HirItem::Function { .. }))
        .expect("the function item must be lowered")
    else {
        unreachable!("filtered to `HirItem::Function` above")
    };
    assert_eq!(*return_ty, Ty::List(Box::new(Ty::Int)));
}

#[test]
fn every_position_the_bare_container_advice_names_really_lowers_a_container() {
    // The bare-container `C0001` advice enumerates the positions that *do*
    // accept the parameterized form, so each one is pinned here: a message
    // that names a position the compiler rejects, or omits one it accepts,
    // is misleading guidance. Return position joined the list in #925.
    for source in [
        // Parameter.
        "def f(x: list[int]) -> None:\n    return\n",
        // Return.
        "def f() -> list[int]:\n    return [1]\n",
        // Local variable.
        "def f() -> None:\n    x: list[int] = []\n    return\n",
        // Module variable.
        "x: list[int] = []\n",
        // PEP 695 type alias.
        "type Ints = list[int]\ndef f(x: Ints) -> None:\n    return\n",
        // Legacy `TypeAlias` assignment.
        "Ints: TypeAlias = list[int]\ndef f(x: Ints) -> None:\n    return\n",
    ] {
        let module = pycc_parser_test_helper::parse(source);
        assert!(
            lower_checked(&module).is_ok(),
            "the bare-container advice names this position as supported: {source:?}"
        );
    }
}

#[test]
fn a_container_return_annotation_still_meets_every_element_and_arity_gate() {
    // Removing the position gate did not remove the annotation's own checks:
    // `lower_return_annotation` lowers through `annotation_to_ty`, so
    // decisions 1 and 2 still fire, now as the *only* rejection a return
    // annotation can produce. Measured, not inferred: this is the diagnostic
    // a user sees, reported against the return annotation's own span.
    //
    // Nested containers are enumerated concretely rather than as a category,
    // because the two shapes do not share a gate: `list[list[set[int]]]` is
    // the list element gate (`T0034`), while `tuple[list[int], int]` is
    // the *tuple* element gate (`T0039`, D-116) -- D-116 rejects a tuple
    // element that is not int/bool/float before D-105 ever sees the inner
    // `list[int]`.
    for (source, code) in [
        ("def f() -> list[None]:\n    return f()\n", "T0034"),
        ("def f() -> list[list[None]]:\n    return f()\n", "T0034"),
        ("def f() -> set[list[int]]:\n    return f()\n", "T0038"),
        (
            "def f() -> dict[list[int], int]:\n    return f()\n",
            "T0036",
        ),
        ("def f() -> tuple[int, None]:\n    return f()\n", "T0039"),
        (
            "def f() -> tuple[list[None], int]:\n    return f()\n",
            "T0034",
        ),
        ("def f() -> tuple[int, ...]:\n    return f()\n", "T0053"),
        ("def f() -> tuple[()]:\n    return f()\n", "T0053"),
        ("def f() -> dict[str]:\n    return f()\n", "T0053"),
        ("def f[T](x: T) -> list[T]:\n    return f(x)\n", "T0042"),
        ("def f() -> list[int] | None:\n    return f()\n", "T0049"),
    ] {
        let diagnostic = container_annotation_err(source);
        assert_eq!(diagnostic.code, code, "{source}");
        assert_ne!(diagnostic.span, Some(Span::new(0, 0)), "{source}");
    }
}

#[test]
fn a_user_defined_class_named_list_still_wins_over_the_builtin_container() {
    // The container branch is checked *after* the known-class lookup, so a
    // user's own `class list` is not silently retyped as a builtin list.
    // Before #1130 that precedence was proved by a refusal (the PEP 560
    // `T0044` a hook-less class used to get); it is now proved by the
    // resolved type directly, which is the stronger statement of the same
    // #918 point -- `Ty::Instance("list")`, the user's class, and not
    // `Ty::List(Int)`.
    let module = pycc_parser_test_helper::parse(
        "class list:\n    def __init__(self) -> None:\n        self.v = 0\n\ndef f(x: list[int]) -> None:\n    return\n",
    );
    let hir = lower_checked(&module).expect("a user-defined `class list` must win and lower");
    let param_ty = hir
        .items
        .iter()
        .find_map(|item| match item {
            HirItem::Function { name, params, .. } if name == "f" => Some(params[0].1.clone()),
            _ => None,
        })
        .expect("expected `f` to lower to an `HirItem::Function`");
    assert_eq!(param_ty, Ty::Instance(Box::new("list".to_string())));
}

#[test]
fn a_container_protocol_attribute_is_rejected_but_a_scalar_one_still_lowers() {
    // The protocol-attribute `AnnAssign` branch ran no type gate at all
    // before #918, because no annotation syntax could produce a container
    // `Ty` there. A container-typed protocol attribute is not supported
    // yet -- conformance checking for one is its own seam, even though
    // #1262 lets a class hold a `list[int]`/`dict[str, int]` slot -- so it
    // is rejected, while every non-container attribute type keeps working
    // exactly as before. The asymmetry with a protocol *method*'s
    // parameter, which does lower, is deliberate and pinned by
    // `a_container_annotation_lowers_in_a_protocol_method_parameter`.
    let diagnostic = container_annotation_err(
        "from typing import Protocol\n\n\nclass P(Protocol):\n    xs: list[int]\n",
    );
    assert_eq!(diagnostic.code, "C0001");
    assert_eq!(
        diagnostic.message,
        "protocol attribute `P.xs` has container type `list[int]`, which is not supported yet as a protocol attribute; a container type in a protocol method's parameter is supported"
    );
    let module = pycc_parser_test_helper::parse(
        "from typing import Protocol\n\n\nclass P(Protocol):\n    n: int\n",
    );
    assert!(lower_checked(&module).is_ok());
}

#[test]
fn a_container_annotation_lowers_in_a_protocol_method_parameter() {
    // The counterpart to the test above, pinning the deliberate asymmetry of
    // D-228 decision 10. The gate lives only in the protocol body's
    // `AnnAssign` arm, so it rejects a container-typed protocol *attribute*
    // (not supported yet as a protocol attribute). A protocol *method*'s parameter
    // is an ordinary parameter position: it routes through
    // `crate::lower_arg_list` -> `annotation_to_ty` with no container gate,
    // and the resulting program builds and runs (pinned end to end by
    // `tests/issue_918_container_annotations.rs`). Extending the gate to
    // method parameters would reject code this compiler compiles correctly.
    for (source, expected) in [
        (
            "from typing import Protocol\n\n\nclass P(Protocol):\n    def f(self, xs: list[int]) -> None: ...\n",
            "list[int]",
        ),
        (
            "from typing import Protocol\n\n\nclass P(Protocol):\n    def f(self, xs: set[int]) -> None: ...\n",
            "set[int]",
        ),
        (
            "from typing import Protocol\n\n\nclass P(Protocol):\n    def f(self, xs: dict[str, int]) -> None: ...\n",
            "dict[str, int]",
        ),
        (
            "from typing import Protocol\n\n\nclass P(Protocol):\n    def f(self, xs: tuple[int, bool]) -> None: ...\n",
            "tuple[int, bool]",
        ),
    ] {
        let module = pycc_parser_test_helper::parse(source);
        let lowered = lower_checked(&module).expect("a protocol method parameter should lower");
        let def = &lowered.class_defs[0].1;
        let ProtocolMember::Method {
            name, param_tys, ..
        } = &def.protocol_members[0]
        else {
            panic!("expected a method member: {:?}", def.protocol_members);
        };
        assert_eq!(name, "f");
        assert_eq!(param_tys.len(), 1);
        assert_eq!(param_tys[0].name(), expected);
    }
}

#[test]
fn every_ordinary_identifier_spelling_lowers_to_the_same_buffer_ty_as_memoryview() {
    // #1129 and #1134: `ndarray` and `NDArray` are further *spellings* of
    // `Ty::MemoryView`, not further types, so what this states is an
    // equality and not merely that the annotations are accepted.
    //
    // Both annotation positions are lowered and asserted on, not just the
    // parameter one. `annotation_to_ty` is one parser for every position,
    // so a parameter-only assertion would prove the arm reachable without
    // proving it reaches the position the downstream refusals key on: a
    // return annotation is what `C0003` (a public export) and the private
    // `C0001` walk both read, and `I0405` reads both.
    //
    // No import: the source mentions numpy nowhere, which is the whole
    // point of the arm -- `import numpy` builds only from a `pycc.lock`
    // closure (#1242), so a spelling gated on one would need numpy. The return-position
    // sources recurse rather than returning the parameter, because *reading*
    // a buffer parameter is its own refusal (Part 2 of #1027) and would
    // refuse the fixture for an unrelated reason.
    fn signature(source: &str) -> (Vec<(String, Ty)>, Ty) {
        let module = pycc_parser_test_helper::parse(source);
        let lowered = lower_checked(&module).expect("the buffer annotation should lower");
        let HirItem::Function {
            params, return_ty, ..
        } = &lowered.items[0]
        else {
            panic!("expected a function: {:?}", lowered.items[0]);
        };
        (params.clone(), return_ty.clone())
    }

    for source in [
        "def f(a: ndarray, n: int) -> float:\n    return 0.0\n",
        "def f(a: NDArray, n: int) -> float:\n    return 0.0\n",
        "def f(a: memoryview, n: int) -> float:\n    return 0.0\n",
    ] {
        let (params, return_ty) = signature(source);
        assert_eq!(params[0].1, Ty::MemoryView, "{source}");
        assert_eq!(return_ty, Ty::Float, "{source}");
    }

    for source in [
        "def f(n: int) -> ndarray:\n    return f(n)\n",
        "def f(n: int) -> NDArray:\n    return f(n)\n",
        "def f(n: int) -> memoryview:\n    return f(n)\n",
    ] {
        let (params, return_ty) = signature(source);
        assert_eq!(return_ty, Ty::MemoryView, "{source}");
        assert_eq!(params[0].1, Ty::Int, "{source}");
    }
}

#[test]
fn a_memoryview_ty_names_itself_with_its_python_spelling() {
    // Part 1 of #1027. `Ty::name` is the spelling every *type-system*
    // diagnostic renders a type with; `src/ext_build.rs`'s own `render_ty`
    // is the separate `ext`-boundary spelling, pinned by
    // `src/ext_build_tests/exports.rs`. The two must agree for the type to
    // read the same in a `T00xx` and in a `C0003`, and nothing else in the
    // crate reaches this arm today, so this is where that is stated.
    assert_eq!(Ty::MemoryView.name(), "memoryview");
}
