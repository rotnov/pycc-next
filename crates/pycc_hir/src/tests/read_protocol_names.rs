//! A class body that binds `__getattribute__` or `__getattr__` (#1465).
//!
//! Python calls `type(obj).__getattribute__` on every attribute read on an
//! instance and `type(obj).__getattr__` whenever that normal lookup raises
//! `AttributeError`. This compiler reads a compiled field or property
//! without consulting either name, so a `def` of either compiled with the
//! method silently bypassed. Both routes are refused by
//! `class/reserved_names.rs`, the fifth set under D-236's generating rule.
//!
//! Every spelling is a body of its own: the class-body walk returns on the
//! first error, so a combined body would only prove the first one fires.

use super::*;

/// The distinctive fragment of the `def` route's message.
const METHOD_FRAGMENT: &str = "while this compiler never calls it on a read, so the method would \
                               be silently bypassed rather than honored (#1465)";

/// The distinctive fragment of the class-attribute route's message.
const ATTR_FRAGMENT: &str = "never consults a class attribute of that name, so the binding would \
                             be silently ignored rather than honored (#1465)";

/// The `__getattribute__` trigger clause: every read.
const EVERY_READ: &str = "on every attribute read on an instance";

/// The `__getattr__` trigger clause: only a lookup that raises.
const ON_MISS: &str = "whenever normal lookup of an attribute on an instance raises \
                       `AttributeError`";

/// The trigger clause `name`'s message must carry, and the one it must not.
fn triggers(name: &str) -> (&'static str, &'static str) {
    if name == "__getattribute__" {
        (EVERY_READ, ON_MISS)
    } else {
        (ON_MISS, EVERY_READ)
    }
}

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

/// The issue's own module, and its `__getattr__` sibling: a plain `def` of
/// each name, refused at its own definition, naming itself, and stating
/// that name's own trigger -- never the other's.
#[test]
fn a_def_of_either_name_is_refused_at_its_definition() {
    for name in ["__getattribute__", "__getattr__"] {
        let source = format!(
            "class C:\n    def __init__(self, n: int) -> None:\n        self.n = n\n\n    \
             def {name}(self, name: str) -> int:\n        return 42\n"
        );
        let message = refused_at(&source, &format!("def {name}"), METHOD_FRAGMENT);
        let (own, other) = triggers(name);
        assert!(
            message.starts_with(&format!(
                "a `def {name}` in a class body is not supported yet -- Python calls `{name}` \
                 implicitly {own}"
            )),
            "{message}"
        );
        assert!(!message.contains(other), "{message}");
    }
}

/// Every other method binding form reaches the same refusal: CPython calls
/// whatever the name binds, so the binding form changes nothing.
#[test]
fn every_method_binding_form_is_refused() {
    for (decorated, located) in [
        (
            "@staticmethod\n    def __getattr__(name: str) -> int:\n        return 1\n",
            "def __getattr__",
        ),
        (
            "@classmethod\n    def __getattribute__(cls, name: str) -> int:\n        return 1\n",
            "def __getattribute__",
        ),
        (
            "@override\n    def __getattr__(self, name: str) -> int:\n        return 1\n",
            "def __getattr__",
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
/// reserved-name dispatch, with the read-protocol message.
#[test]
fn a_property_getter_named_getattribute_is_refused() {
    refused_at(
        "class C:\n    @property\n    def __getattribute__(self) -> int:\n        return 1\n",
        "def __getattribute__",
        METHOD_FRAGMENT,
    );
}

/// A `@__getattr__.setter` with no getter keeps its own, truthful "requires
/// a preceding getter" rejection: the guard short-circuits a setter.
#[test]
fn a_getattr_property_setter_without_a_getter_keeps_its_own_message() {
    let source = "class C:\n    @__getattr__.setter\n    def __getattr__(self, v: int) -> None:\n        pass\n";
    let module = pycc_parser_test_helper::parse(source);
    let diagnostic = lower_checked(&module).unwrap_err();
    assert_eq!(diagnostic.code, "C0001");
    assert!(
        diagnostic
            .message
            .contains("a `@__getattr__.setter` decorator requires a preceding"),
        "{}",
        diagnostic.message
    );
    assert!(
        !diagnostic.message.contains("#1465"),
        "{}",
        diagnostic.message
    );
}

/// A `@dataclass` body shares the method loop.
#[test]
fn a_dataclass_body_is_refused() {
    refused_at(
        "from dataclasses import dataclass\n\n\n@dataclass\nclass P:\n    x: int\n\n    \
         def __getattr__(self, name: str) -> int:\n        return 0\n",
        "def __getattr__",
        METHOD_FRAGMENT,
    );
}

/// A user exception class: the body loop runs before the exception-dunder
/// check, so the read-protocol message wins there, and it makes no claim
/// that would be false for an exception value.
#[test]
fn an_exception_class_gets_the_read_protocol_message() {
    let message = refused_at(
        "class E(Exception):\n    def __getattribute__(self, name: str) -> int:\n        \
         return 0\n",
        "def __getattribute__",
        METHOD_FRAGMENT,
    );
    assert!(!message.contains("slot"), "{message}");
}

/// Every class-attribute route: the bare and annotated literal bindings
/// (CPython raises `TypeError` on the read that calls them) and the
/// value-less declaration, D-236's conservative over-rejection. Each
/// message states its own name's trigger.
#[test]
fn a_class_attribute_of_either_name_is_refused() {
    for (binding, name) in [
        ("__getattribute__ = 1", "__getattribute__"),
        ("__getattr__ = 1", "__getattr__"),
        ("__getattr__: int = 1", "__getattr__"),
        ("__getattribute__: int", "__getattribute__"),
    ] {
        let source = format!(
            "class C:\n    {binding}\n\n    def __init__(self) -> None:\n        self.n = 1\n"
        );
        let message = refused_at(&source, binding, ATTR_FRAGMENT);
        let (own, other) = triggers(name);
        assert!(
            message.starts_with(&format!(
                "a class attribute named `{name}` is not supported yet -- Python calls whatever \
                 `{name}` is bound to implicitly {own}"
            )),
            "{message}"
        );
        assert!(!message.contains(other), "{message}");
    }
}

/// An `Enum` body keeps #979's dunder-shape message: that arm runs first.
#[test]
fn an_enum_body_keeps_the_dunder_shape_message() {
    let source = "from enum import Enum\n\n\nclass C(Enum):\n    __getattr__ = 1\n    B = 2\n";
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
         def __getattr__(self, name: str) -> int:\n        ...\n",
    );
}

/// Neighbouring names are not reserved, a subclass of an accepted class is
/// unaffected, and a module-level `def __getattr__` (PEP 562, #1467) is not
/// a class body and never reaches the guard.
#[test]
fn neighbouring_names_and_a_module_getattr_are_accepted() {
    accepted(
        "def __getattr__(name: str) -> int:\n    return 42\n\n\n\
         class C:\n    def __init__(self, n: int) -> None:\n        self.n = n\n\n    \
         def getattr(self) -> int:\n        return self.n\n\n    \
         def __getattr_x__(self) -> int:\n        return 1\n\n\n\
         class D(C):\n    def get(self) -> int:\n        return self.n\n",
    );
}
