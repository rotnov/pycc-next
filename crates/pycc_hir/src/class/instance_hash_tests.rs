//! Unit tests for `class/instance_hash.rs` (#1335, Part 1 of #1332): one
//! test per verdict arm, over class tables lowered from source.

use super::{
    EqRefusal, HashRefusal, InstanceEq, InstanceHash, InstanceHashLowering, resolve_instance_eq,
    resolve_instance_hash,
};
use crate::HirClassDef;
use crate::class::tests::lower_ok;
use std::collections::HashMap;

fn classes(source: &str) -> HashMap<String, HirClassDef> {
    lower_ok(source).class_defs.into_iter().collect()
}

fn verdict(source: &str, class: &str) -> InstanceHash {
    resolve_instance_hash(class, &classes(source))
}

fn unsupported(source: &str, class: &str) -> HashRefusal {
    match verdict(source, class) {
        InstanceHash::Unsupported(refusal) => refusal,
        other => panic!("expected a refusal for {source:?}, got {other:?}"),
    }
}

fn s(text: &str) -> String {
    text.to_string()
}

const INIT: &str = "    def __init__(self) -> None:\n        self.x = 1\n";

#[test]
fn a_class_binding_neither_name_has_the_identity_hash() {
    let source = format!("class R:\n{INIT}");
    let verdict = verdict(&source, "R");
    assert_eq!(verdict, InstanceHash::Identity);
    assert_eq!(verdict.lowerable(), Some(InstanceHashLowering::Identity));
}

#[test]
fn a_plain_hash_method_is_called() {
    let source = format!("class R:\n{INIT}\n    def __hash__(self) -> int:\n        return 3\n");
    let verdict = verdict(&source, "R");
    assert_eq!(verdict, InstanceHash::Method(s("R.__hash__")));
    assert_eq!(
        verdict.lowerable(),
        Some(InstanceHashLowering::Method(s("R.__hash__")))
    );
}

#[test]
fn a_hash_bound_any_other_way_is_not_a_method() {
    for binding in [
        "    @property\n    def __hash__(self) -> int:\n        return 1\n",
        "    @staticmethod\n    def __hash__() -> int:\n        return 1\n",
        "    @classmethod\n    def __hash__(cls) -> int:\n        return 1\n",
        "    __hash__ = 1\n",
    ] {
        let source = format!("class R:\n{INIT}\n{binding}");
        assert_eq!(
            unsupported(&source, "R"),
            HashRefusal::NotAMethod { class: s("R") },
            "{binding}"
        );
    }
}

#[test]
fn an_eq_bound_in_any_of_the_five_ways_makes_the_class_unhashable() {
    for binding in [
        "    def __eq__(self, other: int) -> bool:\n        return True\n",
        "    @property\n    def __eq__(self) -> int:\n        return 1\n",
        "    @staticmethod\n    def __eq__() -> int:\n        return 1\n",
        "    @classmethod\n    def __eq__(cls) -> int:\n        return 1\n",
        "    __eq__ = 2\n",
    ] {
        let source = format!("class A:\n{INIT}\n{binding}");
        let verdict = verdict(&source, "A");
        assert_eq!(
            verdict,
            InstanceHash::Unhashable { class: s("A") },
            "{binding}"
        );
        assert_eq!(verdict.lowerable(), None);
    }
}

#[test]
fn a_dataclass_needs_its_own_hash() {
    let bare = "from dataclasses import dataclass\n\n\n@dataclass\nclass D:\n    x: int\n";
    assert_eq!(
        unsupported(bare, "D"),
        HashRefusal::Dataclass { class: s("D") }
    );
    let transform = "from typing import dataclass_transform\n\n\n@dataclass_transform()\n\
                     class D:\n    x: int\n";
    assert_eq!(
        unsupported(transform, "D"),
        HashRefusal::Dataclass { class: s("D") }
    );
    let own = format!("{bare}\n    def __hash__(self) -> int:\n        return 11\n");
    assert_eq!(verdict(&own, "D"), InstanceHash::Method(s("D.__hash__")));
}

#[test]
fn inheritance_follows_the_mro() {
    let base = format!("class A:\n{INIT}\n    def __hash__(self) -> int:\n        return 1\n");
    // An inherited `__hash__`.
    let inherited = format!("{base}\n\nclass B(A):\n    pass\n");
    assert_eq!(
        verdict(&inherited, "B"),
        InstanceHash::Method(s("A.__hash__"))
    );
    assert_eq!(
        verdict(&inherited, "A"),
        InstanceHash::Method(s("A.__hash__"))
    );
    // A subclass adding only `__eq__` is unhashable.
    let eq_only = format!(
        "{base}\n\nclass B(A):\n    def __eq__(self, other: int) -> bool:\n        return True\n"
    );
    assert_eq!(
        verdict(&eq_only, "B"),
        InstanceHash::Unhashable { class: s("B") }
    );
    // A subclass redefining `__hash__`.
    let redefined =
        format!("{base}\n\nclass B(A):\n    def __hash__(self) -> int:\n        return 2\n");
    assert_eq!(
        verdict(&redefined, "B"),
        InstanceHash::Method(s("B.__hash__"))
    );
    // A diamond: `D(B, C)` reaches `C.__hash__` before `A`'s.
    let diamond = format!(
        "{base}\n\nclass B(A):\n    pass\n\n\nclass C(A):\n    def __hash__(self) -> int:\n        \
         return 3\n\n\nclass D(B, C):\n    pass\n"
    );
    assert_eq!(
        verdict(&diamond, "D"),
        InstanceHash::Method(s("C.__hash__"))
    );
}

#[test]
fn a_subclass_that_hashes_differently_refuses_the_base() {
    let base = format!("class A:\n{INIT}");
    let redefines =
        format!("{base}\n\nclass B(A):\n    def __hash__(self) -> int:\n        return 2\n");
    assert_eq!(
        unsupported(&redefines, "A"),
        HashRefusal::SubclassDiffers { subclass: s("B") }
    );
    // A subclass that only inherits agrees, including an identity base.
    let agrees = format!("{base}\n\nclass B(A):\n    pass\n");
    assert_eq!(verdict(&agrees, "A"), InstanceHash::Identity);
    // An unhashable base with an agreeing subclass stays unhashable.
    let unhashable = "class A:\n    def __eq__(self, other: int) -> bool:\n        return True\n\
                      \n\nclass B(A):\n    pass\n";
    assert_eq!(
        verdict(unhashable, "A"),
        InstanceHash::Unhashable { class: s("A") }
    );
    // Two unhashable verdicts agree whichever class binds `__eq__`.
    let eq_twice = "class A:\n    def __eq__(self, other: int) -> bool:\n        return True\n\
                    \n\nclass B(A):\n    def __eq__(self, other: int) -> bool:\n        \
                    return False\n";
    assert_eq!(
        verdict(eq_twice, "A"),
        InstanceHash::Unhashable { class: s("A") }
    );
    assert_eq!(
        verdict(eq_twice, "B"),
        InstanceHash::Unhashable { class: s("B") }
    );
    // A subclass failing a precheck disagrees with an identity base.
    let exception = format!("{base}\n\nclass B(A, ValueError):\n    pass\n");
    assert_eq!(
        unsupported(&exception, "A"),
        HashRefusal::SubclassDiffers { subclass: s("B") }
    );
    // A base that is itself refused keeps its own reason.
    let refused = "class A:\n    __hash__ = 1\n\n\nclass B(A):\n    def __hash__(self) -> int:\n        \
                   return 2\n";
    assert_eq!(
        unsupported(refused, "A"),
        HashRefusal::NotAMethod { class: s("A") }
    );
}

#[test]
fn enum_and_generic_classes_are_refused() {
    let enum_source = "from enum import Enum\n\n\nclass Color(Enum):\n    RED = 1\n";
    assert_eq!(
        unsupported(enum_source, "Color"),
        HashRefusal::Enum { class: s("Color") }
    );
    let generic = "class Box[T]:\n    def __init__(self, v: T) -> None:\n        self.v = v\n";
    assert_eq!(
        unsupported(generic, "Box"),
        HashRefusal::Generic { class: s("Box") }
    );
}

#[test]
fn a_protocol_class_is_refused() {
    // Hand-built: a class deriving from a protocol is itself a protocol, so
    // no source program reaches this arm through an ordinary class.
    let mut table = classes(&format!("class P:\n{INIT}"));
    table.get_mut("P").expect("lowered").is_protocol = true;
    assert_eq!(
        resolve_instance_hash("P", &table),
        InstanceHash::Unsupported(HashRefusal::Protocol { class: s("P") })
    );
}

#[test]
fn exception_classes_are_refused() {
    // A seeded builtin exception carries tag `None` and is refused by name.
    let builtin = "try:\n    pass\nexcept ValueError as e:\n    pass\n";
    let table = classes(builtin);
    assert_eq!(table["ValueError"].exception_type_tag, None);
    assert_eq!(
        resolve_instance_hash("ValueError", &table),
        InstanceHash::Unsupported(HashRefusal::Exception {
            class: s("ValueError")
        })
    );
    // A user class deriving from a builtin exception carries a tag.
    let user = "class E(ValueError):\n    pass\n";
    let table = classes(user);
    assert!(table["E"].exception_type_tag.is_some());
    assert_eq!(
        resolve_instance_hash("E", &table),
        InstanceHash::Unsupported(HashRefusal::Exception { class: s("E") })
    );
    // A user class shadowing a builtin exception name: a safe over-refusal.
    let shadow = format!("class ValueError:\n{INIT}");
    assert_eq!(
        unsupported(&shadow, "ValueError"),
        HashRefusal::Exception {
            class: s("ValueError")
        }
    );
}

#[test]
fn every_refusal_has_a_help_line_naming_its_class() {
    for (refusal, needle) in [
        (
            HashRefusal::Dataclass { class: s("K") },
            "`@dataclass_transform()`",
        ),
        (
            HashRefusal::NotAMethod { class: s("K") },
            "plain `def __hash__(self)`",
        ),
        (HashRefusal::Enum { class: s("K") }, "enum member"),
        (
            HashRefusal::Exception { class: s("K") },
            "exception instance",
        ),
        (HashRefusal::Protocol { class: s("K") }, "protocol class"),
        (HashRefusal::Generic { class: s("K") }, "generic class"),
        (HashRefusal::SubclassDiffers { subclass: s("K") }, "#1337"),
    ] {
        let help = refusal.help();
        assert!(help.contains("`K`"), "{help}");
        assert!(help.contains(needle), "{help}");
    }
}

// #1343 (Part 1 of #1336): the eq verdict a set element compares by.

fn eq_verdict(source: &str, class: &str) -> InstanceEq {
    resolve_instance_eq(class, &classes(source))
}

const EQ_METHOD: &str = "    def __eq__(self, other: A) -> bool:\n        return True\n";

#[test]
fn a_class_binding_no_eq_compares_by_identity() {
    let source = format!("class R:\n{INIT}");
    assert_eq!(eq_verdict(&source, "R"), InstanceEq::Identity);
}

#[test]
fn an_own_eq_method_is_called() {
    let source = format!("class A:\n{INIT}\n{EQ_METHOD}");
    assert_eq!(eq_verdict(&source, "A"), InstanceEq::Method(s("A.__eq__")));
}

#[test]
fn an_inherited_eq_method_is_the_base_method() {
    let source = format!("class A:\n{INIT}\n{EQ_METHOD}\nclass B(A):\n    pass\n");
    assert_eq!(eq_verdict(&source, "B"), InstanceEq::Method(s("A.__eq__")));
}

#[test]
fn an_eq_bound_any_other_way_is_not_a_method() {
    for binding in [
        "    @property\n    def __eq__(self) -> int:\n        return 1\n",
        "    @staticmethod\n    def __eq__() -> int:\n        return 1\n",
        // `__eq__ = None` is a class-attribute C0001 before this verdict
        // runs, so an `int` literal stands in for the class-attribute form.
        "    __eq__ = 2\n",
    ] {
        let source = format!("class A:\n{INIT}\n{binding}");
        let refusal = match eq_verdict(&source, "A") {
            InstanceEq::Unsupported(refusal) => refusal,
            other => panic!("expected a refusal for {binding:?}, got {other:?}"),
        };
        assert_eq!(refusal, EqRefusal::NotAMethod { class: s("A") });
        assert!(
            refusal.help().contains("`A` binds `__eq__`"),
            "{}",
            refusal.help()
        );
    }
}

#[test]
fn a_user_eq_with_any_subclass_names_the_first_sorted_subclass() {
    let source =
        format!("class A:\n{INIT}\n{EQ_METHOD}\nclass Z(A):\n    pass\n\nclass M(A):\n    pass\n");
    let refusal = match eq_verdict(&source, "A") {
        InstanceEq::Unsupported(refusal) => refusal,
        other => panic!("expected a refusal, got {other:?}"),
    };
    assert_eq!(refusal, EqRefusal::Subclassed { subclass: s("M") });
    assert!(
        refusal.help().contains("subclass `M`"),
        "{}",
        refusal.help()
    );
}

#[test]
fn an_identity_eq_with_a_subclass_is_not_refused() {
    let source = format!("class A:\n{INIT}\nclass B(A):\n    pass\n");
    assert_eq!(eq_verdict(&source, "A"), InstanceEq::Identity);
}
