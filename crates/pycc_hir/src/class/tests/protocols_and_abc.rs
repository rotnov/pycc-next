//! Protocol classes and abstract base classes.

use super::*;

// -- PEP 544 (#380): Protocol class lowering ---------------------------

#[test]
fn a_protocol_class_with_a_pass_body_lowers_successfully() {
    let hir = lower_ok("from typing import Protocol\nclass P(Protocol):\n    pass\n");
    assert_eq!(hir.class_defs.len(), 1);
    assert!(hir.class_defs[0].1.is_protocol);
    assert!(hir.class_defs[0].1.protocol_members.is_empty());
}

#[test]
fn a_protocol_class_with_a_docstring_lowers_successfully() {
    // #744: a class docstring (a bare string-literal expression
    // statement) is a no-op in a protocol body.
    let hir = lower_ok(
        "from typing import Protocol\nclass P(Protocol):\n    \"A protocol.\"\n    def foo(self) -> int: ...\n",
    );
    let def = &hir.class_defs[0].1;
    assert!(def.is_protocol);
    assert_eq!(def.protocol_members.len(), 1);
}

#[test]
fn a_protocol_class_with_a_non_leading_docstring_lowers_successfully() {
    // #744's guard has no position check: place the docstring after a
    // method to exercise the non-leading case directly.
    let hir = lower_ok(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\n    \"A protocol.\"\n",
    );
    let def = &hir.class_defs[0].1;
    assert!(def.is_protocol);
    assert_eq!(def.protocol_members.len(), 1);
}

#[test]
fn a_non_string_expression_statement_in_a_protocol_body_is_still_rejected() {
    // #744's docstring exemption covers only a bare string-literal
    // expression statement: a bare non-string expression statement in a
    // protocol body remains C0001, exercising the guard's false branch
    // distinctly from a non-`Stmt::Expr` statement
    // (`a_protocol_class_with_an_unsupported_statement_is_rejected`
    // above uses `Stmt::Assign`, which never reaches this guard at all).
    let module = crate::pycc_parser_test_helper::parse(
        "from typing import Protocol\nclass P(Protocol):\n    42\n    def foo(self) -> int: ...\n",
    );
    let diagnostic = lower_checked(&module).unwrap_err();
    assert_eq!(diagnostic.code, "C0001");
}

#[test]
fn a_protocol_class_with_a_method_and_attribute_lowers_successfully() {
    let hir = lower_ok(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\n    x: str\n",
    );
    let def = &hir.class_defs[0].1;
    assert!(def.is_protocol);
    assert_eq!(def.protocol_members.len(), 2);
}

#[test]
fn a_protocol_method_with_a_parameter_lowers_successfully() {
    // Exercises the `params.iter().map(|(_, ty)| ty.clone())` closure
    // on line 715 — when a protocol method has parameters besides
    // `self`, the closure body is entered to collect parameter types
    // into `ProtocolMember::Method::param_tys`.
    let hir = lower_ok(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self, x: int) -> int: ...\n",
    );
    let def = &hir.class_defs[0].1;
    assert!(def.is_protocol);
    assert_eq!(def.protocol_members.len(), 1);
}

#[test]
fn a_protocol_class_with_init_is_rejected() {
    let module = crate::pycc_parser_test_helper::parse(
        "from typing import Protocol\nclass P(Protocol):\n    def __init__(self) -> None: ...\n",
    );
    let diagnostic = lower_checked(&module).unwrap_err();
    assert_eq!(diagnostic.code, "C0001");
    assert!(
        diagnostic.message.contains("cannot define `__init__`"),
        "unexpected message: {}",
        diagnostic.message
    );
}

#[test]
fn a_protocol_method_with_a_decorator_is_rejected() {
    let module = crate::pycc_parser_test_helper::parse(
        "from typing import Protocol\nclass P(Protocol):\n    @staticmethod\n    def foo(self) -> int: ...\n",
    );
    let diagnostic = lower_checked(&module).unwrap_err();
    assert_eq!(diagnostic.code, "C0001");
    assert!(
        diagnostic.message.contains("decorators on protocol method"),
        "unexpected message: {}",
        diagnostic.message
    );
}

#[test]
fn a_generic_protocol_method_is_rejected() {
    let module = crate::pycc_parser_test_helper::parse(
        "from typing import Protocol\nclass P(Protocol):\n    def foo[T](self) -> int: ...\n",
    );
    let diagnostic = lower_checked(&module).unwrap_err();
    assert_eq!(diagnostic.code, "C0001");
    assert!(
        diagnostic.message.contains("generic protocol method"),
        "unexpected message: {}",
        diagnostic.message
    );
}

#[test]
fn a_protocol_method_with_an_implementation_body_is_rejected() {
    let module = crate::pycc_parser_test_helper::parse(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int:\n        return 1\n",
    );
    let diagnostic = lower_checked(&module).unwrap_err();
    assert_eq!(diagnostic.code, "C0001");
    assert!(
        diagnostic.message.contains("declaration-style body"),
        "unexpected message: {}",
        diagnostic.message
    );
}

#[test]
fn a_protocol_class_with_an_unsupported_statement_is_rejected() {
    let module = crate::pycc_parser_test_helper::parse(
        "from typing import Protocol\nclass P(Protocol):\n    x = 1\n",
    );
    let diagnostic = lower_checked(&module).unwrap_err();
    assert_eq!(diagnostic.code, "C0001");
}

#[test]
fn a_protocol_class_with_an_attribute_with_a_default_is_rejected() {
    let module = crate::pycc_parser_test_helper::parse(
        "from typing import Protocol\nclass P(Protocol):\n    x: int = 0\n",
    );
    let diagnostic = lower_checked(&module).unwrap_err();
    assert_eq!(diagnostic.code, "C0001");
    assert!(
        diagnostic.message.contains("cannot have a default value"),
        "unexpected message: {}",
        diagnostic.message
    );
}

#[test]
fn a_generic_protocol_class_is_rejected() {
    let module = crate::pycc_parser_test_helper::parse(
        "from typing import Protocol\nclass P[T](Protocol):\n    def foo(self) -> int: ...\n",
    );
    let diagnostic = lower_checked(&module).unwrap_err();
    assert_eq!(diagnostic.code, "C0001");
    assert!(
        diagnostic.message.contains("generic protocol class"),
        "unexpected message: {}",
        diagnostic.message
    );
}

#[test]
fn a_dataclass_protocol_class_is_rejected() {
    let module = crate::pycc_parser_test_helper::parse(
        "from typing import Protocol\n@dataclass\nclass P(Protocol):\n    def foo(self) -> int: ...\n",
    );
    let diagnostic = lower_checked(&module).unwrap_err();
    assert_eq!(diagnostic.code, "C0001");
    assert!(
        diagnostic.message.contains("cannot be a `@dataclass`"),
        "unexpected message: {}",
        diagnostic.message
    );
}

#[test]
fn a_protocol_class_inheriting_and_redeclaring_a_method_lowers() {
    let hir = lower_ok(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass Q(P):\n    def foo(self) -> str: ...\n",
    );
    let q_def = hir.class_defs.iter().find(|(n, _)| n == "Q").unwrap();
    assert!(q_def.1.is_protocol);
    // Q inherits P's `foo` but redeclares it with a different return type.
    // The redeclared version should replace the inherited one.
    assert_eq!(q_def.1.protocol_members.len(), 1);
}

#[test]
fn a_protocol_class_inheriting_an_attribute_lowers() {
    let hir = lower_ok(
        "from typing import Protocol\nclass P(Protocol):\n    x: int\nclass Q(P):\n    def foo(self) -> int: ...\n",
    );
    let q_def = hir.class_defs.iter().find(|(n, _)| n == "Q").unwrap();
    assert!(q_def.1.is_protocol);
    // Q inherits P's `x` attribute and adds its own `foo` method.
    assert_eq!(q_def.1.protocol_members.len(), 2);
}

// -- PEP 3119 (#380): ABC and @abstractmethod lowering ----------------

#[test]
fn an_abc_class_with_an_abstract_method_lowers_successfully() {
    let hir = lower_ok(
        "from abc import ABC, abstractmethod\nclass A(ABC):\n    @abstractmethod\n    def foo(self) -> int: ...\n    def __init__(self) -> None:\n        return\n",
    );
    let def = &hir.class_defs[0].1;
    assert!(def.is_abstract);
    assert!(def.abstract_methods.contains(&"foo".to_string()));
}

#[test]
fn an_abstract_method_with_an_implementation_body_is_rejected() {
    let module = crate::pycc_parser_test_helper::parse(
        "from abc import ABC, abstractmethod\nclass A(ABC):\n    @abstractmethod\n    def foo(self) -> int:\n        return 1\n    def __init__(self) -> None:\n        return\n",
    );
    let diagnostic = lower_checked(&module).unwrap_err();
    assert_eq!(diagnostic.code, "C0001");
    assert!(
        diagnostic.message.contains("declaration-style body"),
        "unexpected message: {}",
        diagnostic.message
    );
}

#[test]
fn a_concrete_subclass_inheriting_an_unoverridden_abstract_method_is_rejected() {
    let module = crate::pycc_parser_test_helper::parse(
        "from abc import ABC, abstractmethod\nclass A(ABC):\n    @abstractmethod\n    def foo(self) -> int: ...\n    def __init__(self) -> None:\n        return\nclass B(A):\n    def __init__(self) -> None:\n        return\n",
    );
    let diagnostic = lower_checked(&module).unwrap_err();
    assert_eq!(diagnostic.code, "C0001");
    assert!(
        diagnostic.message.contains("abstract"),
        "unexpected message: {}",
        diagnostic.message
    );
}

#[test]
fn a_class_with_more_than_two_decorators_is_rejected() {
    let module = crate::pycc_parser_test_helper::parse(
        "from typing import Protocol, runtime_checkable\n@runtime_checkable\n@dataclass\n@runtime_checkable\nclass P(Protocol):\n    def foo(self) -> int: ...\n",
    );
    let diagnostic = lower_checked(&module).unwrap_err();
    assert_eq!(diagnostic.code, "C0001");
    assert!(
        diagnostic
            .message
            .contains("more than two class decorators"),
        "unexpected message: {}",
        diagnostic.message
    );
}

#[test]
fn a_protocol_method_with_a_pass_body_lowers_successfully() {
    let hir = lower_ok(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int:\n        pass\n",
    );
    let def = &hir.class_defs[0].1;
    assert!(def.is_protocol);
    assert_eq!(def.protocol_members.len(), 1);
}

#[test]
fn a_protocol_method_without_self_lowers_successfully() {
    let hir = lower_ok(
        "from typing import Protocol\nclass P(Protocol):\n    def foo() -> int: ...\n",
    );
    let def = &hir.class_defs[0].1;
    assert!(def.is_protocol);
    assert_eq!(def.protocol_members.len(), 1);
}

#[test]
fn a_protocol_attribute_with_a_non_name_target_is_rejected() {
    let module = crate::pycc_parser_test_helper::parse(
        "from typing import Protocol\nclass P(Protocol):\n    self.x: int\n",
    );
    let diagnostic = lower_checked(&module).unwrap_err();
    assert_eq!(diagnostic.code, "C0001");
    assert!(
        diagnostic.message.contains("must target a bare name"),
        "unexpected message: {}",
        diagnostic.message
    );
}

#[test]
fn a_protocol_class_redeclaring_an_inherited_attribute_lowers() {
    let hir = lower_ok(
        "from typing import Protocol\nclass P(Protocol):\n    x: int\nclass Q(P):\n    x: str\n",
    );
    let q_def = hir.class_defs.iter().find(|(n, _)| n == "Q").unwrap();
    assert!(q_def.1.is_protocol);
    // Q redeclares P's `x` with a different type. The redeclared
    // version should replace the inherited one.
    assert_eq!(q_def.1.protocol_members.len(), 1);
}

#[test]
fn a_protocol_class_inheriting_a_method_with_own_attribute_lowers() {
    // This covers the Attribute arm in the inherited-member dedup
    // check (line 606 in lower_protocol_body).
    let hir = lower_ok(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass Q(P):\n    x: str\n",
    );
    let q_def = hir.class_defs.iter().find(|(n, _)| n == "Q").unwrap();
    assert!(q_def.1.is_protocol);
    // Q inherits P's `foo` method and adds its own `x` attribute.
    assert_eq!(q_def.1.protocol_members.len(), 2);
}

#[test]
fn a_dataclass_inheriting_from_a_protocol_is_rejected() {
    let module = crate::pycc_parser_test_helper::parse(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\n@dataclass\nclass Q(P):\n    x: int\n",
    );
    let diagnostic = lower_checked(&module).unwrap_err();
    assert_eq!(diagnostic.code, "C0001");
    assert!(
        diagnostic.message.contains("cannot be a `@dataclass`"),
        "unexpected message: {}",
        diagnostic.message
    );
}

#[test]
fn an_abstract_method_redefined_in_the_same_class_is_accepted() {
    // Redefining an abstract method in the same class should update
    // the method binding (matching regular method redefinition
    // semantics) and keep it in abstract_methods.
    let hir = lower_ok(
        "from abc import ABC, abstractmethod\nclass A(ABC):\n    @abstractmethod\n    def foo(self) -> int: ...\n    @abstractmethod\n    def foo(self) -> str: ...\n    def __init__(self) -> None:\n        return\n",
    );
    let def = &hir.class_defs[0].1;
    assert!(def.is_abstract);
    assert!(def.abstract_methods.contains(&"foo".to_string()));
}

#[test]
fn a_concrete_subclass_overriding_an_inherited_abstract_method_lowers() {
    let hir = lower_ok(
        "from abc import ABC, abstractmethod\nclass A(ABC):\n    @abstractmethod\n    def foo(self) -> int: ...\n    def __init__(self) -> None:\n        return\nclass B(A):\n    def foo(self) -> int:\n        return 1\n    def __init__(self) -> None:\n        return\n",
    );
    let b_def = hir.class_defs.iter().find(|(n, _)| n == "B").unwrap();
    // B is concrete (not abstract) because it overrides foo.
    assert!(!b_def.1.is_abstract);
    // B's abstract_methods includes inherited ones from A, but B
    // overrides foo with a concrete method.
    assert!(b_def.1.abstract_methods.contains(&"foo".to_string()));
}

#[test]
fn a_protocol_inheriting_from_two_protocols_with_overlapping_methods_lowers() {
    // This exercises the dedup check in the inherited-member loop
    // (class.rs lines 604-606): when two base protocols declare the
    // same method, the second inheritance skips the duplicate.
    let hir = lower_ok(
        "from typing import Protocol\nclass P1(Protocol):\n    def foo(self) -> int: ...\nclass P2(Protocol):\n    def foo(self) -> int: ...\nclass Q(P1, P2):\n    def bar(self) -> int: ...\n",
    );
    let q_def = hir.class_defs.iter().find(|(n, _)| n == "Q").unwrap();
    assert!(q_def.1.is_protocol);
    // Q should have `foo` (from P1/P2, deduplicated) and `bar` (own).
    assert_eq!(q_def.1.protocol_members.len(), 2);
}

#[test]
fn a_protocol_inheriting_from_two_protocols_with_overlapping_attributes_lowers() {
    // This exercises the Attribute arm of the dedup check in the
    // inherited-member loop (class.rs line 606): when two base
    // protocols declare the same attribute, the second inheritance
    // skips the duplicate.
    let hir = lower_ok(
        "from typing import Protocol\nclass P1(Protocol):\n    x: int\nclass P2(Protocol):\n    x: int\nclass Q(P1, P2):\n    y: str\n",
    );
    let q_def = hir.class_defs.iter().find(|(n, _)| n == "Q").unwrap();
    assert!(q_def.1.is_protocol);
    // Q should have `x` (from P1/P2, deduplicated) and `y` (own).
    assert_eq!(q_def.1.protocol_members.len(), 2);
}

#[test]
fn a_protocol_method_with_self_and_unsupported_param_annotation_is_rejected() {
    // This exercises the `?` error path on lower_arg_list in the
    // `self` branch of protocol method lowering (line 677).
    assert_c0001(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self, x: Frobnicate) -> int: ...\n",
    );
}

#[test]
fn a_protocol_method_with_self_and_unsupported_return_annotation_is_rejected() {
    // This exercises the `?` error path on lower_return_annotation
    // in the `self` branch of protocol method lowering (line 686).
    assert_c0001(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> Frobnicate: ...\n",
    );
}

#[test]
fn a_protocol_method_with_an_unsupported_param_annotation_is_rejected() {
    // Exercises the `?` error path on `lower_arg_list` in protocol
    // method lowering. #1181 made the receiver strip *positional*, so
    // the offending annotation has to sit on a parameter after the
    // receiver -- with `def foo(x: Frobnicate)` the bad annotation is
    // now the receiver's own and is stripped before it is ever lowered.
    assert_c0001(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(this, x: Frobnicate) -> int: ...\n",
    );
}

#[test]
fn a_protocol_method_with_an_unsupported_posonly_param_annotation_is_rejected() {
    // #1181 split the receiver strip across `posonlyargs` and `args`, so
    // the two `lower_arg_list` calls have two separate `?` error paths.
    // This one is the `posonlyargs` call's: `x` stays in `posonlyargs`
    // after the receiver is stripped from that same list.
    assert_c0001(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(this, x: Frobnicate, /) -> int: ...\n",
    );
}

#[test]
fn a_protocol_method_without_self_and_unsupported_return_annotation_is_rejected() {
    // This exercises the `?` error path on lower_return_annotation
    // in the no-`self` branch of protocol method lowering (line 710).
    assert_c0001(
        "from typing import Protocol\nclass P(Protocol):\n    def foo() -> Frobnicate: ...\n",
    );
}

#[test]
fn a_protocol_attribute_with_unsupported_annotation_is_rejected() {
    // This exercises the `?` error path on annotation_to_ty in
    // protocol attribute lowering (line 745).
    assert_c0001("from typing import Protocol\nclass P(Protocol):\n    x: Frobnicate\n");
}

#[test]
fn a_protocol_method_with_a_docstring_body_is_rejected() {
    // A docstring is a `Stmt::Expr` whose value is a string literal,
    // not an `EllipsisLiteral`.  This exercises the `false` branch of
    // the `matches!` inside `is_declaration_body` (line 560), which
    // causes the protocol-method body check to reject it.
    assert_c0001(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int:\n        \"docstring\"\n",
    );
}

#[test]
fn an_abstract_method_with_a_docstring_body_is_rejected() {
    // Same `is_declaration_body` false branch, reached via the
    // `@abstractmethod` lowering path (line 1693).
    assert_c0001(
        "from abc import ABC, abstractmethod\nclass A(ABC):\n    @abstractmethod\n    def foo(self) -> int:\n        \"docstring\"\n",
    );
}

#[test]
fn a_concrete_subclass_inheriting_abstract_method_from_grandparent_lowers() {
    // This exercises the abstract method inheritance loop (line 1786)
    // by having a three-level inheritance chain: A (abstract) -> B
    // (abstract, explicitly inherits ABC) -> C (concrete, overrides).
    // The loop iterates over MRO entries beyond the first, finding
    // inherited abstract methods from both A and B.
    let hir = lower_ok(
        "from abc import ABC, abstractmethod\nclass A(ABC):\n    @abstractmethod\n    def foo(self) -> int: ...\n    def __init__(self) -> None:\n        return\nclass B(A, ABC):\n    @abstractmethod\n    def bar(self) -> int: ...\n    def __init__(self) -> None:\n        return\nclass C(B):\n    def foo(self) -> int:\n        return 1\n    def bar(self) -> int:\n        return 2\n    def __init__(self) -> None:\n        return\n",
    );
    let c_def = hir.class_defs.iter().find(|(n, _)| n == "C").unwrap();
    assert!(!c_def.1.is_abstract);
    assert!(c_def.1.abstract_methods.contains(&"foo".to_string()));
    assert!(c_def.1.abstract_methods.contains(&"bar".to_string()));
}
