//! Unit tests for `class/declared_attrs.rs` and the declaration override in
//! `class/init_slot.rs` (#1266, Part 5 of #1218): one test per row of the
//! issue plan's rule table, each new `C0001` pinned by its full wording.

use crate::Ty;
use crate::class::tests::lower_ok;
use pycc_diag::Diagnostic;

fn error(source: &str) -> Diagnostic {
    crate::lower_checked(&crate::pycc_parser_test_helper::parse(source)).unwrap_err()
}

fn c0001(source: &str) -> String {
    let diagnostic = error(source);
    assert_eq!(diagnostic.code, "C0001", "source: {source:?}");
    diagnostic.message
}

/// `class C:` declaring `x: <annotation>` and an `__init__` assigning
/// `self.x = <value>`.
fn declared(annotation: &str, params: &str, value: &str) -> String {
    format!(
        "class C:\n    x: {annotation}\n\n    def __init__(self{params}) -> None:\n        \
         self.x = {value}\n"
    )
}

fn attrs(source: &str) -> Vec<(String, Ty)> {
    lower_ok(source).class_defs[0].1.attrs.clone()
}

fn x(ty: Ty) -> Vec<(String, Ty)> {
    vec![("x".to_string(), ty)]
}

#[test]
fn a_scalar_declaration_types_the_slot_its_init_establishes() {
    for (annotation, value, ty) in [
        ("int", "5", Ty::Int),
        ("float", "1.5", Ty::Float),
        ("bool", "True", Ty::Bool),
        ("str", "\"s\"", Ty::Str),
        ("Annotated[int, \"m\"]", "5", Ty::Int),
    ] {
        let source = format!(
            "from typing import Annotated\n{}",
            declared(annotation, "", value)
        );
        assert_eq!(attrs(&source), x(ty), "{annotation}");
    }
}

#[test]
fn the_declared_type_replaces_the_inferred_one() {
    // `pycc_types::check_attr_set` then judges `2` against the `float`
    // slot (`T0021`), exactly as for a second assignment.
    assert_eq!(attrs(&declared("float", "", "2")), x(Ty::Float));
    assert_eq!(attrs(&declared("int", ", n: int", "n")), x(Ty::Int));
}

#[test]
fn a_container_declaration_types_the_empty_literal_that_establishes_it() {
    assert_eq!(
        attrs(&declared("list[int]", "", "[]")),
        x(Ty::List(Box::new(Ty::Int)))
    );
    assert_eq!(
        attrs(&declared("dict[str, int]", "", "{}")),
        x(Ty::Dict(Box::new((Ty::Str, Ty::Int))))
    );
    assert_eq!(
        attrs(&declared("list[int]", ", xs: list[int]", "xs")),
        x(Ty::List(Box::new(Ty::Int)))
    );
}

#[test]
fn an_empty_literal_of_the_wrong_shape_keeps_the_declared_type() {
    // The checker's wrong-shape `T0003` is what refuses these; `pycc_hir`
    // records the declared slot and never consults the RHS inference, so no
    // `list[<inferred>]` or #891 message can appear.
    assert_eq!(attrs(&declared("int", "", "[]")), x(Ty::Int));
    assert_eq!(attrs(&declared("int", "", "{}")), x(Ty::Int));
    assert_eq!(
        attrs(&declared("dict[str, int]", "", "[]")),
        x(Ty::Dict(Box::new((Ty::Str, Ty::Int))))
    );
    let generic =
        "class Box[T]:\n    v: T\n\n    def __init__(self) -> None:\n        self.v = {}\n";
    assert_eq!(
        lower_ok(generic).class_defs[0].1.attrs,
        vec![("v".to_string(), Ty::Param(Box::new("T".to_string())))]
    );
}

#[test]
fn a_type_parameter_declaration_admits_a_value_of_that_parameter() {
    let source = "class Box[T]:\n    v: T\n\n    def __init__(self, v: T) -> None:\n        \
                  self.v = v\n";
    assert_eq!(
        lower_ok(source).class_defs[0].1.attrs,
        vec![("v".to_string(), Ty::Param(Box::new("T".to_string())))]
    );
}

#[test]
fn a_type_parameter_and_a_concrete_type_never_meet_in_one_slot() {
    assert_eq!(
        c0001(
            "class Box[T]:\n    v: T\n\n    def __init__(self, v: T) -> None:\n        \
             self.v = 3\n"
        ),
        "instance attribute `v` declared in class `Box` as `T` is assigned a value of type \
         `int` -- a type-parameter attribute must be assigned a value of that same type \
         parameter, and a concrete attribute cannot hold one"
    );
    assert_eq!(
        c0001(
            "class Box[T]:\n    n: int\n\n    def __init__(self, v: T) -> None:\n        \
             self.n = v\n"
        ),
        "instance attribute `n` declared in class `Box` as `int` is assigned a value of type \
         `T` -- a type-parameter attribute must be assigned a value of that same type \
         parameter, and a concrete attribute cannot hold one"
    );
}

/// #1388: a declaration admits any right-hand side the body lowers -- the
/// declared type is the slot's, whatever the expression -- including a read
/// of a slot `__init__` assigned earlier, and one it has not assigned yet,
/// which raises `AttributeError` at run time instead.
#[test]
fn a_declaration_admits_any_establishing_expression() {
    for (params, value) in [
        (", n: int", "n + 1"),
        (", n: int", "abs(n)"),
        (", d: dict[str, int], k: str", "d[k]"),
        ("", "self.x"),
    ] {
        assert_eq!(
            attrs(&declared("int", params, value)),
            x(Ty::Int),
            "{value}"
        );
    }
    assert_eq!(
        attrs(
            "class C:\n    a: int\n    b: int\n\n    def __init__(self, n: int) -> None:\n        \
             self.a = n\n        self.b = self.a * 2\n"
        ),
        vec![("a".to_string(), Ty::Int), ("b".to_string(), Ty::Int)]
    );
}

/// #1388: inside a PEP 695 generic class the shape gate stays, in both
/// directions the checker cannot judge -- a concrete declaration assigned an
/// expression and a type-parameter declaration assigned one.
#[test]
fn the_rhs_shape_gate_still_applies_in_a_generic_class() {
    for (annotation, value) in [("int", "k + 1"), ("T", "v if k else v")] {
        let message = c0001(&format!(
            "class Box[T]:\n    x: {annotation}\n\n    def __init__(self, v: T, k: int) -> None:\n        \
             self.x = {value}\n"
        ));
        assert!(
            message.starts_with("an instance attribute's first assignment inside `__init__` must"),
            "{value}: {message}"
        );
    }
}

/// #1388: an undeclared attribute keeps the gate, and its message names the
/// declaration that lifts it.
#[test]
fn an_undeclared_attribute_keeps_the_gate_and_names_the_declaration() {
    let message =
        c0001("class C:\n    def __init__(self, n: int) -> None:\n        self.x = n + 1\n");
    assert!(
        message.ends_with(
            "is known at compile time -- or declare it in the class body (`x: <type>`), which \
             admits any expression"
        ),
        "{message}"
    );
}

/// In a PEP 695 generic class a declaration keeps the gate, so the
/// diagnostic does not suggest one.
#[test]
fn an_undeclared_attribute_of_a_generic_class_gets_no_declaration_hint() {
    let message =
        c0001("class Box[T]:\n    def __init__(self, n: int) -> None:\n        self.x = n + 1\n");
    assert!(message.ends_with("is known at compile time"), "{message}");
}

#[test]
fn an_annotated_establishing_assignment_must_agree_with_the_declaration() {
    let source = |annotation: &str, value: &str| {
        format!(
            "class C:\n    xs: list[int]\n\n    def __init__(self) -> None:\n        \
             self.xs: {annotation} = {value}\n"
        )
    };
    assert_eq!(
        lower_ok(&source("list[int]", "[]")).class_defs[0].1.attrs,
        vec![("xs".to_string(), Ty::List(Box::new(Ty::Int)))]
    );
    assert_eq!(
        c0001(&source("dict[str, int]", "{}")),
        "instance attribute `xs` is declared in class `C` as `list[int]` but annotated \
         `dict[str, int]` in `__init__` -- the two annotations must agree"
    );
}

#[test]
fn every_target_of_a_chained_assignment_gets_its_own_declared_type() {
    let hir = lower_ok(
        "class C:\n    a: float\n\n    def __init__(self) -> None:\n        self.a = self.b = 0\n",
    );
    assert_eq!(
        hir.class_defs[0].1.attrs,
        vec![("a".to_string(), Ty::Float), ("b".to_string(), Ty::Int)]
    );
}

#[test]
fn a_declaration_may_follow_init_and_follows_a_renamed_receiver() {
    let hir = lower_ok(
        "class C:\n    def __init__(this, n: int) -> None:\n        this.n = n\n        \
         this.f = 1\n\n    f: float\n",
    );
    assert_eq!(
        hir.class_defs[0].1.attrs,
        vec![("n".to_string(), Ty::Int), ("f".to_string(), Ty::Float)]
    );
    assert!(hir.class_defs[0].1.class_attrs.is_empty());
}

#[test]
fn a_declared_type_without_a_slot_representation_is_refused() {
    assert_eq!(
        c0001("class C:\n    x: None\n"),
        "instance attribute `x` declared in class `C` has type `None`, which has no \
         instance-slot representation -- a class-body declaration admits only `int`, `float`, \
         `bool`, `str`, a type parameter, `list[int]`, `dict[str, int]`, a class of this \
         program, or a class a foreign import binds"
    );
    for annotation in [
        "set[int]",
        "tuple[int, int]",
        "int | None",
        "P",
        "memoryview",
    ] {
        // `P` is a protocol: a protocol-typed value has no one concrete
        // representation for a slot word to hold (#1389 admits only a
        // concrete class).
        let message = c0001(&format!(
            "from typing import Protocol\n\n\nclass P(Protocol):\n    def f(self) -> int: \
             ...\n\n\nclass C:\n    x: {annotation}\n"
        ));
        assert!(
            message.starts_with("instance attribute `x` declared in class `C` has type `"),
            "{annotation}: {message}"
        );
    }
}

#[test]
fn a_container_element_type_keeps_its_annotation_code() {
    for (annotation, code) in [
        ("list[str]", "T0034"),
        ("dict[str, float]", "T0036"),
        ("set[str]", "T0038"),
        ("tuple[int, str]", "T0039"),
    ] {
        assert_eq!(
            error(&format!("class C:\n    x: {annotation}\n")).code,
            code,
            "{annotation}"
        );
    }
    assert_eq!(error("class Box[T]:\n    x: list[T]\n").code, "T0042");
}

#[test]
fn a_bare_list_or_dict_declaration_names_the_parameterized_form() {
    for (bare, example) in [("list", "list[int]"), ("dict", "dict[str, int]")] {
        assert_eq!(
            c0001(&format!("class C:\n    x: {bare}\n")),
            format!(
                "a bare `{bare}` type annotation is not supported yet -- write the \
                 parameterized form, e.g. `{example}`"
            )
        );
    }
    // A bare `set` would advise a form this position refuses too.
    assert_eq!(
        c0001("class C:\n    x: set\n"),
        "type annotation `set` is not supported yet"
    );
}

#[test]
fn a_reserved_name_is_refused_before_anything_else() {
    let message = c0001("class C:\n    __slots__: int\n    __slots__: int\n");
    assert!(
        message.starts_with("this `__slots__` spelling is not supported yet"),
        "{message}"
    );
}

#[test]
fn a_second_declaration_of_one_name_is_refused() {
    let source = "class C:\n    x: int\n    x: str\n";
    let diagnostic = error(source);
    assert_eq!(
        diagnostic.message,
        "instance attribute `x` is declared more than once in class `C` -- declare it once"
    );
    let second = u32::try_from(source.find("x: str").expect("present")).expect("fits");
    assert_eq!(diagnostic.span.expect("spanned").start, second);
}

#[test]
fn a_declaration_named_after_a_method_like_member_is_refused() {
    for (decorator, kind) in [
        ("", "method"),
        ("@property\n    ", "property"),
        ("@staticmethod\n    ", "staticmethod"),
        ("@classmethod\n    ", "classmethod"),
    ] {
        let receiver = match kind {
            "staticmethod" => "",
            "classmethod" => "cls",
            _ => "self",
        };
        let source = format!(
            "class C:\n    x: int\n\n    def __init__(self) -> None:\n        self.x = 1\n\n    \
             {decorator}def x({receiver}) -> int:\n        return 1\n"
        );
        assert_eq!(
            c0001(&source),
            format!(
                "instance attribute `x` declared in class `C` collides with the {kind} `x` of \
                 the same class"
            )
        );
    }
}

#[test]
fn a_collision_is_reported_before_a_missing_assignment() {
    assert_eq!(
        c0001("class C:\n    x: int\n\n    def x(self) -> int:\n        return 1\n"),
        "instance attribute `x` declared in class `C` collides with the method `x` of the same \
         class"
    );
}

#[test]
fn a_declaration_its_own_init_never_establishes_is_refused() {
    assert_eq!(
        c0001("class C:\n    x: int\n    x = 1\n"),
        "instance attribute `x` declared in class `C` is never assigned at the top level of \
         `C.__init__` -- assign it there (`self.x = ...`), or give it a value (`x: int = 1`) to \
         make it a class constant"
    );
    // Each scalar gets a literal its class constant accepts; a container or
    // type-parameter declaration gets no class-constant advice at all.
    for (annotation, tail) in [
        (
            "float",
            ", or give it a value (`x: float = 1.0`) to make it a class constant",
        ),
        (
            "bool",
            ", or give it a value (`x: bool = True`) to make it a class constant",
        ),
        (
            "str",
            ", or give it a value (`x: str = \"\"`) to make it a class constant",
        ),
        ("list[int]", ""),
        ("T", ""),
    ] {
        assert_eq!(
            c0001(&format!("class C[T]:\n    x: {annotation}\n")),
            format!(
                "instance attribute `x` declared in class `C` is never assigned at the top \
                 level of `C.__init__` -- assign it there (`self.x = ...`){tail}"
            )
        );
    }
}

#[test]
fn only_a_top_level_assignment_in_the_own_init_establishes_a_declaration() {
    for source in [
        // No assignment at all.
        "class C:\n    x: int\n\n    def __init__(self) -> None:\n        self.y = 1\n",
        // Only inside a nested block.
        "class C:\n    x: int\n\n    def __init__(self, c: bool) -> None:\n        if c:\n            \
         self.x = 1\n",
        // Only an ancestor establishes it.
        "class B:\n    def __init__(self) -> None:\n        self.x = 1\n\n\nclass C(B):\n    \
         x: int\n",
        // Only another method assigns it.
        "class C:\n    x: int\n\n    def __init__(self) -> None:\n        return\n\n    \
         def set(self) -> None:\n        self.x = 1\n",
    ] {
        assert!(
            c0001(source).starts_with("instance attribute `x` declared in class `C` is never"),
            "{source}"
        );
    }
}

#[test]
fn a_declaration_beside_a_class_constant_of_the_same_name_keeps_the_collision() {
    let message = c0001(
        "class C:\n    x: int\n\n    def __init__(self) -> None:\n        self.x = 5\n\n    \
         x = 1\n",
    );
    assert!(
        message.starts_with("class attribute `C.x` collides with an instance attribute"),
        "{message}"
    );
}

#[test]
fn value_less_class_var_and_final_are_not_declarations() {
    let with_init = |annotation: &str| {
        format!(
            "import typing\nfrom typing import Annotated, ClassVar, Final\n\n\nclass C:\n    \
             x: {annotation}\n\n    def __init__(self) -> None:\n        self.x = 1\n"
        )
    };
    let no_value = "class attribute `x` has no value -- a class attribute is a compile-time \
                    constant and must be initialized with a literal (`x: int = 1`)";
    for annotation in [
        "ClassVar[int]",
        "Final",
        "Final[int]",
        "Annotated[Final[int], \"m\"]",
        // Part 1 of #889: a quoted `Final` inside `Annotated` too.
        "Annotated[\"Final[int]\", \"m\"]",
    ] {
        assert_eq!(c0001(&with_init(annotation)), no_value, "{annotation}");
    }
    // Peeled to a bare `Final`, which `annotation_to_ty` does not lower.
    assert_eq!(
        c0001(&with_init("Annotated[Final, \"m\"]")),
        "type annotation `Final` is not supported yet"
    );
    // Not peeled: each is a declaration candidate whose own annotation error
    // is unchanged.
    assert_eq!(
        c0001(&with_init("Annotated[int]")),
        "Annotated requires at least two arguments: the type and at least one metadata element"
    );
    assert_eq!(
        c0001(&with_init("typing.Annotated[int, \"m\"]")),
        "a subscripted type annotation's base must be a bare class name"
    );
    assert!(
        c0001(&with_init("ClassVar")).starts_with("a bare `ClassVar` is not a valid annotation"),
    );
    assert!(c0001(&with_init("Final[Annotated[int, \"m\"]]")).starts_with("class attribute `x`"),);
}

#[test]
fn a_class_var_nested_in_annotated_keeps_its_own_refusal() {
    let source = "from typing import Annotated, ClassVar\n\n\nclass C:\n    \
                  x: Annotated[ClassVar[int], \"m\"]\n";
    let diagnostic = error(source);
    assert_eq!(
        diagnostic.message,
        "`ClassVar` is only valid on a class-body attribute declaration (`X: ClassVar[int] = 1` \
         inside a `class` body)"
    );
    let start = u32::try_from(source.find("ClassVar[int]").expect("present")).expect("fits");
    assert_eq!(diagnostic.span.expect("spanned").start, start);
}

#[test]
fn a_value_less_annotation_on_a_non_name_target_keeps_its_message() {
    assert!(
        c0001("class C:\n    o.x: int\n")
            .starts_with("a class-level attribute annotation must target a bare name")
    );
}

#[test]
fn a_dataclass_or_protocol_body_keeps_its_own_meaning() {
    let hir = lower_ok("from dataclasses import dataclass\n\n\n@dataclass\nclass C:\n    x: int\n");
    assert_eq!(
        hir.class_defs[0].1.dataclass_fields,
        vec![("x".to_string(), Ty::Int)]
    );
    lower_ok("from typing import Protocol\n\n\nclass P(Protocol):\n    n: int\n");
}
