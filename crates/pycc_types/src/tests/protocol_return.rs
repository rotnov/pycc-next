//! #934: the type-checker regions that only a protocol-*returning* function
//! used to reach, re-covered after `pycc_hir` started rejecting `-> P` with
//! `C0001`.
//!
//! Four tests in the parent module used a `-> P` fixture through
//! `check_source`, which lowers with `pycc_hir::lower_checked(..).expect(..)`
//! and therefore panics -- not errs -- under the new gate. Two of them were
//! behaviour pins with no surviving premise and were deleted outright; the
//! other two owned regions D-014's 100% gate still needs executed:
//!
//! - `monomorphize::rewrite_protocol_calls_in_expr`'s `if let
//!   Ok(Ty::Instance(..))` false branch (a protocol-typed argument to a
//!   protocol-typed parameter) and the `!substitutions.is_empty()` false
//!   branch. No source program can reach them any more (after the gate the
//!   only way an argument infers to `Ty::Protocol` is a protocol-typed
//!   *parameter*, and a protocol-parameter function's original body is
//!   never walked for call rewriting), so the HIR is lowered from a
//!   `-> C` program and its return type is then rewritten to
//!   `Ty::Protocol("P")` by hand -- the shape `pycc_hir` used to produce.
//! - `check`'s `Assign` and `Return` mismatch arms with a `Ty::Protocol`
//!   side (`class::assignable_error`'s fall-through). These are reachable
//!   from source inside a protocol-parameter function body, which the
//!   validation pass still walks.
//!
//! Separate file rather than more lines in `tests.rs` (AGENTS.md's
//! decomposability rule; that module is tracked by #695).

use super::check_source;
use crate::check_and_resolve;
use pycc_hir::{HirItem, Ty};

/// A protocol, a conforming class, a protocol-parameter function, and a
/// `-> C` factory whose return type each test below may rewrite.
const PRELUDE: &str = "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\ndef proto_fn(p: P) -> int:\n    return p.foo()\ndef make() -> C:\n    return C()\n";

/// Lowers `source` and rewrites `make`'s declared return type to
/// `Ty::Protocol("P")`, reproducing the HIR a `def make() -> P` produced
/// before #934's gate.
fn lower_with_protocol_returning_make(source: &str) -> pycc_hir::HirModule {
    let module = pycc_parser::parse(source).expect("test fixture must parse");
    let mut hir = pycc_hir::lower_checked(&module).expect("test fixture must lower");
    let rewritten = hir
        .items
        .iter_mut()
        .filter(|item| matches!(item, HirItem::Function { name, .. } if name == "make"))
        .map(|item| {
            if let HirItem::Function { return_ty, .. } = item {
                *return_ty = Ty::Protocol(Box::new("P".to_string()));
            }
        })
        .count();
    assert_eq!(rewritten, 1, "the fixture declares exactly one `make`");
    hir
}

#[test]
fn a_protocol_typed_argument_to_a_protocol_parameter_produces_no_specialization() {
    // Module-level `proto_fn(make())`: the argument's inferred type is
    // `Ty::Protocol("P")`, so `rewrite_protocol_calls_in_expr` records no
    // substitution for it (the `Ok(Ty::Instance(..))` test is false) and,
    // with the substitution list empty, leaves the call unrewritten instead
    // of minting a `0gen_` specialization. The checker itself accepts the
    // program -- `P` is assignable to `P` -- which is exactly why the gate
    // had to move to HIR lowering.
    let hir = lower_with_protocol_returning_make(&format!("{PRELUDE}print(proto_fn(make()))\n"));
    let resolved = check_and_resolve(&hir).expect("a protocol-to-protocol argument type-checks");
    let function_names: Vec<&str> = resolved
        .items
        .iter()
        .filter_map(|item| match item {
            HirItem::Function { name, .. } => Some(name.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        !function_names
            .iter()
            .any(|name| name.starts_with("0gen_proto_fn")),
        "no specialization can be minted without a concrete class: {function_names:?}"
    );
    assert!(
        !function_names.contains(&"proto_fn"),
        "the un-specializable original is still dropped: {function_names:?}"
    );
    assert!(function_names.contains(&"make"));
}

#[test]
fn a_protocol_typed_local_argument_inside_a_function_body_produces_no_specialization() {
    // Same two false branches, reached through a function body's local:
    // `x = make()` binds `x` to `Ty::Protocol("P")` and the argument
    // inference either yields that protocol type or fails to see the local
    // at all -- both fall into the same non-`Instance` branch, which is why
    // it cannot be simplified into an `Ok(Ty::Instance)`-only match.
    let hir = lower_with_protocol_returning_make(&format!(
        "{PRELUDE}def caller() -> None:\n    x = make()\n    print(proto_fn(x))\ncaller()\n"
    ));
    let resolved = check_and_resolve(&hir).expect("a protocol-typed local type-checks");
    assert!(!resolved.items.iter().any(|item| matches!(
        item,
        HirItem::Function { name, .. } if name.starts_with("0gen_proto_fn")
    )));
}

#[test]
fn reassigning_an_int_local_with_a_protocol_typed_parameter_is_t0021() {
    // `check`'s `Assign` arm: `previous` is `int`, `ty` is `Protocol("P")`,
    // so the mismatch goes through `class::assignable_error`, whose
    // fall-through (neither side is a concrete class to run conformance
    // on) is the plain `T0021` mismatch.
    let err = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\ndef f(p: P) -> int:\n    x = 1\n    x = p\n    return x\n",
    )
    .unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(err.message, "type mismatch: `P` is not assignable to `int`");
}

#[test]
fn returning_a_protocol_typed_local_from_an_int_function_is_t0021() {
    // `check`'s `Return` arm with a `Ty::Protocol` side. The direct
    // `return p` spelling does not reach it -- D-146's return-type solver
    // reports `T0022` first -- so the value goes through an annotated
    // local: the parameter and the `AnnAssign` are both assignable, and
    // the `Return` arm is the only `assignable_error` caller left.
    let err = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\ndef f(p: P) -> int:\n    x: P = p\n    return x\n",
    )
    .unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(err.message, "type mismatch: `P` is not assignable to `int`");
}
