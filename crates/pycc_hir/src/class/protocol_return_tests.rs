//! Unit tests for #934's protocol-return gate as reached through a *protocol
//! member declaration* (`protocol::lower_protocol_class`'s two
//! `lower_return_annotation` call sites, with and without `self`).
//!
//! A member declared `-> P` (another protocol) is rejected deliberately, not
//! by accident of the shared function: `pycc_types::class::
//! check_protocol_conformance` compares member return types with plain
//! `is_assignable`, so a concrete `def clone(self) -> C` would not satisfy a
//! member `-> P` today, and the only satisfying spelling (`-> P` on the
//! concrete class) is itself gated. Accepting the member would declare an
//! unsatisfiable protocol. The message carries no "annotate the concrete
//! class" advice for the same reason -- that advice is wrong for an
//! interface declaration.
//!
//! Since #948 that reasoning covers the *self-referential* member too. A
//! protocol member returning its own protocol -- spelled either as the
//! enclosing protocol's own name (`-> P`, PEP 649/749) or as `-> Self`
//! (PEP 673) -- resolves through `func::enclosing_class_ty` to
//! `Ty::Protocol("P")` and reaches the same gate with the same message. It
//! used to resolve to `Ty::Instance("P")` because `annotation_to_ty`'s
//! self-reference arms ran before the `class_defs` protocol lookup, which
//! made every conforming concrete class fail with a spurious `T0046`. A
//! self-referential *parameter* or *attribute* is not gated: `Ty::Protocol`
//! is supported in those positions (D-166), so those members stay accepted
//! and are pinned below.
//!
//! Sibling of `class::tests`, in its own file per AGENTS.md's
//! decomposability rule (`class.rs` is well past the threshold).

use super::tests::lower_ok;
use crate::lower_checked;

const TWO_PROTOCOLS: &str =
    "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\n";

const MESSAGE_P: &str = "a protocol class (`P`) as a return type annotation is not supported yet -- a protocol type is currently supported in parameter and variable positions only";

fn assert_protocol_return_c0001(source: &str) {
    assert_protocol_return_c0001_spelled(source, "P");
}

/// The same assertion for a source whose return annotation is spelled
/// `annotation` rather than `P` -- `Self` (PEP 673) reaches the identical
/// gate with the identical message, but is four characters wide and cannot
/// be located by searching for `-> P:` (#948).
fn assert_protocol_return_c0001_spelled(source: &str, annotation: &str) {
    let module = crate::pycc_parser_test_helper::parse(source);
    let diagnostic = lower_checked(&module).unwrap_err();
    assert_eq!(diagnostic.code, "C0001", "source: {source:?}");
    // The message, not just the code: the `Frobnicate` tests in
    // `class::tests` reach the same two `?` sites with a *different*
    // `C0001`, so the code alone would not prove the new gate fired.
    assert_eq!(diagnostic.message, MESSAGE_P, "source: {source:?}");
    let needle = format!("-> {annotation}:");
    let start = source
        .rfind(&needle)
        .expect("source carries the annotation")
        + 3;
    let start = u32::try_from(start).expect("test source fits a span");
    let width = u32::try_from(annotation.len()).expect("test source fits a span");
    assert_eq!(
        diagnostic.span,
        Some(pycc_diag::Span::new(start, start + width)),
        "source: {source:?}"
    );
}

/// The `Ty` a protocol member of `P` named `member` lowers to: a method's
/// return type or an attribute's own type.
fn protocol_member_ty(source: &str, member: &str) -> Option<crate::Ty> {
    let hir = lower_ok(source);
    let (_, class_def) = hir
        .class_defs
        .iter()
        .find(|(name, _)| name == "P")
        .expect("the protocol is registered");
    class_def
        .protocol_members
        .iter()
        .find_map(|candidate| match candidate {
            crate::ProtocolMember::Method {
                name, return_ty, ..
            } if name == member => Some(return_ty.clone()),
            crate::ProtocolMember::Attribute { name, ty } if name == member => Some(ty.clone()),
            _ => None,
        })
}

/// The parameter types of `P`'s method `member`, excluding `self`.
fn protocol_method_param_tys(source: &str, member: &str) -> Vec<crate::Ty> {
    let hir = lower_ok(source);
    let (_, class_def) = hir
        .class_defs
        .iter()
        .find(|(name, _)| name == "P")
        .expect("the protocol is registered");
    class_def
        .protocol_members
        .iter()
        .find_map(|candidate| match candidate {
            crate::ProtocolMember::Method {
                name, param_tys, ..
            } if name == member => Some(param_tys.clone()),
            _ => None,
        })
        .expect("the method is a protocol member")
}

#[test]
fn a_protocol_member_with_self_returning_another_protocol_is_c0001() {
    assert_protocol_return_c0001(&format!(
        "{TWO_PROTOCOLS}class Q(Protocol):\n    def clone(self) -> P: ...\n"
    ));
}

#[test]
fn a_protocol_member_without_self_returning_another_protocol_is_c0001() {
    // The no-`self` branch of `lower_protocol_class` lowers every
    // parameter and the return annotation through the same two helpers.
    assert_protocol_return_c0001(&format!(
        "{TWO_PROTOCOLS}class Q(Protocol):\n    def clone() -> P: ...\n"
    ));
}

#[test]
fn a_self_referential_protocol_member_return_is_c0001() {
    // #948: inside `class P(Protocol)`, `annotation_to_ty`'s
    // enclosing-class-name arm used to resolve `P` to `Ty::Instance("P")`
    // before the `class_defs` protocol lookup would have produced
    // `Ty::Protocol("P")`, so the member lowered as returning an *instance*
    // and every conforming concrete class was rejected with a spurious
    // `T0046`. It now resolves through `func::enclosing_class_ty` and
    // reaches #934's gate with #934's own message.
    assert_protocol_return_c0001(
        "from typing import Protocol\nclass P(Protocol):\n    def clone(self) -> P: ...\n",
    );
}

#[test]
fn a_self_referential_protocol_member_return_spelled_self_is_c0001() {
    // The PEP 673 spelling goes through the same helper, so it must reach
    // the same gate. Before #948 it lowered to `Ty::Instance("P")` and the
    // program compiled silently.
    assert_protocol_return_c0001_spelled(
        "from typing import Protocol\nclass P(Protocol):\n    def clone(self) -> Self: ...\n",
        "Self",
    );
}

#[test]
fn a_self_referential_protocol_member_parameter_is_the_protocol_type() {
    // The parameter position is *not* gated -- `Ty::Protocol` is supported
    // there (D-166) -- so both spellings stay accepted and now carry the
    // protocol type rather than an instance type.
    for source in [
        "from typing import Protocol\nclass P(Protocol):\n    def same(self, other: P) -> bool: ...\n",
        "from typing import Protocol\nclass P(Protocol):\n    def same(self, other: Self) -> bool: ...\n",
    ] {
        assert_eq!(
            protocol_method_param_tys(source, "same"),
            vec![crate::Ty::Protocol(Box::new("P".to_string()))],
            "source: {source:?}"
        );
    }
}

#[test]
fn a_self_referential_protocol_attribute_is_the_protocol_type() {
    // The third `annotation_to_ty` call site inside `lower_protocol_class`
    // is the attribute `AnnAssign` arm. It is not gated either, so it just
    // carries the protocol type now.
    for (source, spelling) in [
        (
            "from typing import Protocol\nclass P(Protocol):\n    nxt: P\n",
            "P",
        ),
        (
            "from typing import Protocol\nclass P(Protocol):\n    nxt: Self\n",
            "Self",
        ),
    ] {
        assert_eq!(
            protocol_member_ty(source, "nxt"),
            Some(crate::Ty::Protocol(Box::new("P".to_string()))),
            "spelling: {spelling}"
        );
    }
}

#[test]
fn a_self_referential_member_of_an_ordinary_class_still_lowers_to_an_instance() {
    // The non-protocol branch of `func::enclosing_class_ty`: PEP 649/749's
    // original `class Node: def next(self) -> Node` behavior (#387) is
    // unchanged, and so is `-> Self`.
    for source in [
        "class Node:\n    def __init__(self) -> None:\n        self.x = 0\n\n    def next(self) -> Node:\n        return self\n",
        "class Node:\n    def __init__(self) -> None:\n        self.x = 0\n\n    def next(self) -> Self:\n        return self\n",
    ] {
        let hir = lower_ok(source);
        let return_ty = hir
            .items
            .iter()
            .find_map(|item| match item {
                crate::HirItem::Function {
                    name, return_ty, ..
                } if name == "Node.next" => Some(return_ty.clone()),
                _ => None,
            })
            .expect("the method is lowered");
        assert_eq!(
            return_ty,
            crate::Ty::Instance(Box::new("Node".to_string())),
            "source: {source:?}"
        );
    }
}
