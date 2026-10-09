//! Unit tests for the `isinstance` narrowing recognizer and gate (#1476).

use super::*;
use crate::pycc_parser_test_helper::parse;
use crate::{HirModule, lower_checked};

fn name(n: &str) -> HirExpr {
    HirExpr::Name(n.to_string())
}

fn call(callee: &str, args: Vec<HirExpr>) -> HirExpr {
    HirExpr::Call {
        callee: callee.to_string(),
        args,
    }
}

fn not(operand: HirExpr) -> HirExpr {
    HirExpr::UnaryOp {
        op: UnaryOpKind::Not,
        operand: Box::new(operand),
    }
}

#[test]
fn a_positive_guard_names_the_operand_and_class() {
    let test = call("isinstance", vec![name("o"), name("int")]);
    assert_eq!(
        isinstance_test(&test),
        Some(("o", "int", IsInstancePolarity::Positive))
    );
}

#[test]
fn a_negated_guard_is_recognized_with_its_polarity() {
    let test = not(call("isinstance", vec![name("o"), name("C")]));
    assert_eq!(
        isinstance_test(&test),
        Some(("o", "C", IsInstancePolarity::Negated))
    );
}

#[test]
fn every_other_shape_is_not_a_guard() {
    let tuple = HirExpr::TupleLiteral(vec![name("int"), name("str")]);
    let rejected = [
        // A tuple of classes.
        call("isinstance", vec![name("o"), tuple]),
        // A non-name operand.
        call("isinstance", vec![HirExpr::IntLiteral(1), name("int")]),
        // A wrong arity.
        call("isinstance", vec![name("o")]),
        // Another callee.
        call("issubclass", vec![name("o"), name("int")]),
        // `not` over something that is not a guard.
        not(name("o")),
        // Another unary operator.
        HirExpr::UnaryOp {
            op: UnaryOpKind::USub,
            operand: Box::new(call("isinstance", vec![name("o"), name("int")])),
        },
        // Not a call at all.
        name("o"),
    ];
    for test in &rejected {
        assert_eq!(isinstance_test(test), None, "{test:?}");
    }
}

fn lower_ok(source: &str) -> HirModule {
    lower_checked(&parse(source)).expect("fixture should lower")
}

fn target(hir: &HirModule, class: &str) -> Option<Ty> {
    isinstance_narrow_target(
        class,
        |wanted| {
            hir.class_defs
                .iter()
                .find(|(name, _)| name == wanted)
                .map(|(_, def)| def)
        },
        hir.class_defs.iter().map(|(_, def)| def),
    )
}

#[test]
fn the_scalar_builtins_narrow_to_their_own_type() {
    let hir = lower_ok("x = 1\n");
    assert_eq!(target(&hir, "int"), Some(Ty::Int));
    assert_eq!(target(&hir, "float"), Some(Ty::Float));
    assert_eq!(target(&hir, "bool"), Some(Ty::Bool));
    assert_eq!(target(&hir, "str"), Some(Ty::Str));
}

#[test]
fn containers_and_unknown_names_do_not_narrow() {
    let hir = lower_ok("x = 1\n");
    for class in ["list", "dict", "tuple", "Missing"] {
        assert_eq!(target(&hir, class), None, "{class}");
    }
}

#[test]
fn a_regular_class_and_an_erased_generic_base_narrow_to_an_instance() {
    let hir = lower_ok(
        "from typing import Generic, TypeVar\nT = TypeVar(\"T\")\n\
         class C:\n    pass\n\
         class S(Generic[T]):\n    pass\n",
    );
    assert_eq!(target(&hir, "C"), Some(Ty::Instance(Box::new("C".into()))));
    assert_eq!(target(&hir, "S"), Some(Ty::Instance(Box::new("S".into()))));
}

/// A module-level class spelled like a builtin shadows it: the guard tests
/// the compiled class, so the narrowing names it too, or nothing when the
/// class is not admissible.
#[test]
fn a_compiled_class_spelled_like_a_builtin_shadows_it() {
    let hir = lower_ok(
        "from enum import Enum\n\
         class int:\n    pass\n\
         class str(Enum):\n    A = 1\n",
    );
    assert_eq!(
        target(&hir, "int"),
        Some(Ty::Instance(Box::new("int".into())))
    );
    assert_eq!(target(&hir, "str"), None);
    assert_eq!(target(&hir, "float"), Some(Ty::Float));
}

/// A guard on a class that has a compiled subclass also admits the
/// subclass's instances, which static dispatch would run as the base
/// (D-254), so only a leaf class narrows; a `Generic[T]` subclass counts.
#[test]
fn a_class_with_a_compiled_subclass_does_not_narrow() {
    let hir = lower_ok(
        "from typing import Generic, TypeVar\nT = TypeVar(\"T\")\n\
         class Base:\n    pass\n\
         class Mid(Base):\n    pass\n\
         class Leaf(Mid):\n    pass\n\
         class Root:\n    pass\n\
         class G(Root, Generic[T]):\n    pass\n",
    );
    for class in ["Base", "Mid", "Root"] {
        assert_eq!(target(&hir, class), None, "{class}");
    }
    assert_eq!(
        target(&hir, "Leaf"),
        Some(Ty::Instance(Box::new("Leaf".into())))
    );
    assert_eq!(target(&hir, "G"), Some(Ty::Instance(Box::new("G".into()))));
}

#[test]
fn enums_exceptions_protocols_and_pep695_generics_do_not_narrow() {
    let hir = lower_ok(
        "from enum import Enum\nfrom typing import Protocol\n\
         class Color(Enum):\n    RED = 1\n\
         class Boom(Exception):\n    pass\n\
         class Shape(Protocol):\n    def area(self) -> int: ...\n\
         class Box[T]:\n    def __init__(self, v: T) -> None:\n        self.v = v\n",
    );
    for class in ["Color", "Boom", "Shape", "Box"] {
        assert_eq!(target(&hir, class), None, "{class}");
    }
}

#[test]
fn a_tagged_exception_class_does_not_narrow_even_without_a_builtin_base() {
    let hir = lower_ok("class C:\n    pass\n");
    let mut def = hir
        .class_defs
        .iter()
        .find(|(name, _)| name == "C")
        .map(|(_, def)| def.clone())
        .expect("class is defined");
    assert!(def.admits_isinstance_narrowing());
    def.exception_type_tag = Some(1);
    assert!(!def.admits_isinstance_narrowing());
}
