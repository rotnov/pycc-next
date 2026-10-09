//! An own `@abstractmethod` in a class with no `ABC` base is a `C0001`
//! ([#1145](https://github.com/rotnov/pycc/issues/1145)).
//!
//! Its own child module rather than more of the 7,000-line `tests.rs`, under
//! `AGENTS.md`'s decomposability rule.
//!
//! **What this pins, and why it is load-bearing elsewhere.** `--ext`'s
//! instance-method export set excludes an `@abstractmethod` with no filter of
//! its own: the exclusion is carried entirely by `HirClassDef::is_abstract`
//! inside the driver's constructibility predicate
//! (`src/ext_build.rs::class_constructible`). That is only *total* because
//! `is_abstract` is set by `bases.iter().any(is_abc_base_name)` alone -- so
//! the argument needs a class that carries an abstract method while
//! `is_abstract` is false to be unreachable. `class.rs`'s
//! unoverridden-abstract check is what makes it unreachable: a class's own
//! abstract methods are in `all_abstract_methods` and are never in the
//! `overridden` set, so a `!is_abstract` class carrying one is rejected
//! before codegen. Nothing pinned that before this test, and deleting the
//! check would silently admit an abstract stub -- a real miscompile, since
//! `class.rs` lowers such a stub with a `return None` body whose `return_ty`
//! says otherwise.

use super::*;

#[test]
fn an_own_abstract_method_without_an_abc_base_is_a_capability_error() {
    assert_capability_error_message(
        "\
from abc import abstractmethod


class Shape:
    @abstractmethod
    def area(self) -> int: ...
",
        "does not override abstract method `area`",
    );
}

#[test]
fn the_same_class_with_an_abc_base_lowers() {
    // The discriminator: the rejection above is about `is_abstract` being
    // false, not about the decorator. With `ABC` in the bases the identical
    // body lowers, which is exactly the class `--ext` then excludes by
    // `is_abstract`.
    let module = pycc_parser_test_helper::parse(
        "\
from abc import ABC, abstractmethod


class Shape(ABC):
    @abstractmethod
    def area(self) -> int: ...


class Square(Shape):
    def __init__(self, side: int) -> None:
        self.side = side

    def area(self) -> int:
        return self.side * self.side


def main() -> None:
    print(Square(2).area())
",
    );
    let hir = lower_checked(&module).expect("an `ABC` base makes the class abstract, not invalid");
    let shape = hir
        .class_defs
        .iter()
        .find(|(name, _)| name == "Shape")
        .expect("Shape is lowered");
    assert!(shape.1.is_abstract, "{:?}", shape.1);
    assert_eq!(shape.1.abstract_methods, vec!["area".to_string()]);
}
