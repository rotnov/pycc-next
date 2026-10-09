//! A class body that binds `__setattr__` or `__delattr__` (#1459).
//!
//! Python calls `type(obj).__setattr__` on every `obj.x = v` and
//! `type(obj).__delattr__` on every `del obj.x`. This compiler stores into a
//! compiled instance without consulting either name, so a `def` of either
//! compiled with the method silently bypassed, and `__setattr__ = 1` compiled
//! where CPython raises `TypeError` on the store. Both routes are refused by
//! `class/reserved_names.rs`, the fourth set under D-236's generating rule.
//!
//! Every spelling is a body of its own: the class-body walk returns on the
//! first error, so a combined body would only prove the first one fires.

use super::*;

/// The distinctive fragment of the `def` route's message.
const METHOD_FRAGMENT: &str = "while this compiler never calls either on a store or deletion, so \
                               the method would be silently bypassed rather than honored (#1459)";

/// The distinctive fragment of the class-attribute route's message.
const ATTR_FRAGMENT: &str = "never consults a class attribute of that name, so the binding would \
                             be silently ignored rather than honored (#1459)";

/// Lowers `source`, expects a `C0001` containing `fragment` whose span covers
/// the first occurrence of `located`, and returns the message.
fn refused_at(source: &str, located: &str, fragment: &str) -> String {
    let module = pycc_parser_test_helper::parse(source);
    let diagnostic = lower_checked(&module).unwrap_err();
    assert_eq!(diagnostic.code, "C0001", "{source}");
    assert!(
        diagnostic.message.contains(fragment),
        "{source}\n{}",
        diagnostic.message
    );
    let at = u32::try_from(source.find(located).expect(located)).unwrap();
    let span = diagnostic.span.expect("a located diagnostic");
    assert!(
        span.start <= at && at < span.end,
        "{source}: span {span:?} does not cover `{located}` at {at}"
    );
    diagnostic.message
}

fn accepted(source: &str) {
    let module = pycc_parser_test_helper::parse(source);
    if let Err(diagnostic) = lower_checked(&module) {
        panic!("{source}\nrefused: {}", diagnostic.message);
    }
}

/// The issue's own module: a plain `def` of each name, each refused at its
/// own definition and naming itself.
#[test]
fn a_def_of_either_name_is_refused_at_its_definition() {
    for name in ["__setattr__", "__delattr__"] {
        let source = format!(
            "class C:\n    def __init__(self, n: int) -> None:\n        self.n = n\n\n    \
             def {name}(self, name: str) -> None:\n        print(name)\n"
        );
        let message = refused_at(&source, &format!("def {name}"), METHOD_FRAGMENT);
        assert!(
            message.starts_with(&format!("a `def {name}` in a class body")),
            "{message}"
        );
    }
}

/// Every other method binding form reaches the same refusal: CPython calls
/// whatever the name binds, so the binding form changes nothing.
#[test]
fn every_method_binding_form_is_refused() {
    for (decorated, located) in [
        (
            "@staticmethod\n    def __setattr__(name: str, value: int) -> None:\n        pass\n",
            "def __setattr__",
        ),
        (
            "@classmethod\n    def __delattr__(cls, name: str) -> None:\n        pass\n",
            "def __delattr__",
        ),
        (
            "@override\n    def __setattr__(self, name: str, value: int) -> None:\n        pass\n",
            "def __setattr__",
        ),
    ] {
        let source = format!(
            "from typing import override\n\n\nclass C:\n    def __init__(self) -> None:\n        \
             self.n = 1\n\n    {decorated}"
        );
        refused_at(&source, located, METHOD_FRAGMENT);
    }
}

/// The `@property` getter spelling is refused before the getter's own
/// reserved-name dispatch, with the store-protocol message: CPython calls
/// the property's *value* on the store (`'int' object is not callable`).
#[test]
fn a_property_getter_named_setattr_is_refused() {
    refused_at(
        "class C:\n    @property\n    def __setattr__(self) -> int:\n        return 1\n",
        "def __setattr__",
        METHOD_FRAGMENT,
    );
}

/// A `@__setattr__.setter` with no getter keeps its own, truthful "requires
/// a preceding getter" rejection: the guard short-circuits a setter.
#[test]
fn a_setattr_property_setter_without_a_getter_keeps_its_own_message() {
    let source = "class C:\n    @__setattr__.setter\n    def __setattr__(self, v: int) -> None:\n        pass\n";
    let module = pycc_parser_test_helper::parse(source);
    let diagnostic = lower_checked(&module).unwrap_err();
    assert_eq!(diagnostic.code, "C0001");
    assert!(
        diagnostic
            .message
            .contains("a `@__setattr__.setter` decorator requires a preceding"),
        "{}",
        diagnostic.message
    );
    assert!(
        !diagnostic.message.contains("#1459"),
        "{}",
        diagnostic.message
    );
}

/// A `@dataclass` body shares the method loop.
#[test]
fn a_dataclass_body_is_refused() {
    refused_at(
        "from dataclasses import dataclass\n\n\n@dataclass\nclass P:\n    x: int\n\n    \
         def __setattr__(self, name: str, value: int) -> None:\n        pass\n",
        "def __setattr__",
        METHOD_FRAGMENT,
    );
}

/// A user exception class: the body loop runs before the exception-dunder
/// check, so the store-protocol message wins there, and it makes no claim
/// that would be false for an exception value.
#[test]
fn an_exception_class_gets_the_store_protocol_message() {
    let message = refused_at(
        "class E(Exception):\n    def __setattr__(self, name: str, value: int) -> None:\n        \
         pass\n",
        "def __setattr__",
        METHOD_FRAGMENT,
    );
    assert!(!message.contains("slot"), "{message}");
}

/// Every class-attribute route: the bare and annotated literal bindings
/// (CPython raises `TypeError` on the store or `del`) and the value-less
/// declaration, D-236's conservative over-rejection.
#[test]
fn a_class_attribute_of_either_name_is_refused() {
    for (binding, name) in [
        ("__setattr__ = 1", "__setattr__"),
        ("__delattr__ = 1", "__delattr__"),
        ("__setattr__: int = 1", "__setattr__"),
        ("__delattr__: int", "__delattr__"),
    ] {
        let source = format!(
            "class C:\n    {binding}\n\n    def __init__(self) -> None:\n        self.n = 1\n"
        );
        let message = refused_at(&source, binding, ATTR_FRAGMENT);
        assert!(
            message.starts_with(&format!("a class attribute named `{name}`")),
            "{message}"
        );
    }
}

/// An `Enum` body keeps #979's dunder-shape message: that arm runs first.
#[test]
fn an_enum_body_keeps_the_dunder_shape_message() {
    let source = "from enum import Enum\n\n\nclass C(Enum):\n    __setattr__ = 1\n    B = 2\n";
    let module = pycc_parser_test_helper::parse(source);
    let diagnostic = lower_checked(&module).unwrap_err();
    assert_eq!(diagnostic.code, "C0001");
    assert!(
        diagnostic
            .message
            .contains("a dunder-named assignment in an `Enum` body"),
        "{}",
        diagnostic.message
    );
}

/// A `Protocol` body is not checked: its members are never lowered into a
/// class's method table, and a class listing it as a base is itself a
/// protocol that cannot be instantiated.
#[test]
fn a_protocol_member_is_accepted() {
    accepted(
        "from typing import Protocol\n\n\nclass P(Protocol):\n    \
         def __setattr__(self, name: str, value: int) -> None:\n        ...\n",
    );
}

/// Neighbouring names are not reserved, and a subclass of an accepted class
/// is unaffected.
#[test]
fn neighbouring_names_are_accepted() {
    accepted(
        "class C:\n    def __init__(self, n: int) -> None:\n        self.n = n\n        \
         self.setattr_count = 0\n\n    def setattr(self, v: int) -> None:\n        \
         self.n = v\n\n    def __setattr_x__(self) -> int:\n        return 1\n\n\n\
         class D(C):\n    def get(self) -> int:\n        return self.n\n",
    );
}
