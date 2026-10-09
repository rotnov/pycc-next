//! Unit tests for `receiver_dispatch.rs` (issue #1188): the container form a
//! receiver-dispatched call derives, the accessors its readers use, and the
//! lowering that keeps both readings.

use crate::{ContainerFallback, HirExpr, HirItem, HirStmt, lower_checked};

/// A class defining all four names, so the gate is on for each of them.
const ALL_FOUR: &str = "\
class K:
    def get(self) -> int:
        return 1
    def pop(self) -> int:
        return 2
    def append(self, v: int) -> None:
        return
    def add(self, v: int) -> None:
        return
";

/// The value of the last top-level expression statement `source` lowers to.
fn last_expr(source: &str) -> HirExpr {
    let module = pycc_parser::parse(source).expect("test fixture must parse");
    let hir = lower_checked(&module).expect("test fixture must lower");
    hir.items
        .into_iter()
        .rev()
        .find_map(|item| match item {
            HirItem::TopLevelStmt(HirStmt::ExprStmt(expr)) => Some(expr),
            _ => None,
        })
        .expect("the fixture ends in an expression statement")
}

fn lower_message(source: &str) -> String {
    let module = pycc_parser::parse(source).expect("test fixture must parse");
    lower_checked(&module)
        .expect_err("fixture must be rejected")
        .message
}

/// Splits a receiver-dispatched call into its two parts.
fn parts(expr: HirExpr) -> (HirExpr, ContainerFallback) {
    match expr {
        HirExpr::ReceiverDispatchedCall { call, container } => (*call, container),
        other => panic!("expected a receiver-dispatched call, got {other:?}"),
    }
}

#[test]
fn the_derived_container_form_is_the_gate_off_node() {
    // Parity: with the gate on, `container_form` rebuilds exactly the node
    // the gate-off lowering of the same call produces.
    let calls = [
        "xs = [1]\nxs.append(1 + 2)\n",
        "xs = [1]\nxs.pop()\n",
        "d = {'a': 1}\nd.get('a', 3 * 2)\n",
        "s = {1}\ns.add(-4)\n",
        // #1263: an attribute receiver.
        "class B:\n    def __init__(self, xs: list[int], d: dict[str, int]) -> None:\n        self.xs = xs\n        self.d = d\nb = B([1], {'a': 1})\nb.xs.append(5)\n",
        "class B:\n    def __init__(self, xs: list[int]) -> None:\n        self.xs = xs\nb = B([1])\nb.xs.pop()\n",
        "class B:\n    def __init__(self, d: dict[str, int]) -> None:\n        self.d = d\nb = B({'a': 1})\nb.d.get('a', 0)\n",
    ];
    for call in calls {
        let gate_off = last_expr(call);
        let (method_call, container) = parts(last_expr(&format!("{ALL_FOUR}{call}")));
        assert_eq!(container, ContainerFallback::Admitted, "call: {call}");
        assert_eq!(method_call.container_form(), Some(gate_off), "call: {call}");
    }
}

#[test]
fn a_boundary_literal_refuses_the_container_reading() {
    // 2**62 fits in i64, so the method reading lowers it, but it is outside
    // the tagged smallint range the container fast path's boundary check
    // (T0051) admits: the container reading is kept as refused.
    let (_, container) = parts(last_expr(&format!(
        "{ALL_FOUR}xs = [1]\nxs.append(4611686018427387904)\n"
    )));
    let ContainerFallback::Refused(diagnostic) = container else {
        panic!("expected a refused container reading, got {container:?}");
    };
    assert_eq!(diagnostic.code, "T0051");
}

#[test]
fn a_literal_neither_reading_lowers_is_rejected_as_before() {
    // An integer literal outside i64 fails both readings; the gate-on module
    // reports exactly what the gate-off module reports.
    let call = "xs = [1]\nxs.append(99999999999999999999999)\n";
    assert_eq!(
        lower_message(&format!("{ALL_FOUR}{call}")),
        lower_message(call)
    );
}

#[test]
fn a_failed_method_reading_returns_the_container_result_verbatim() {
    // The method reading lowers the lambda argument and fails; the container
    // reading of `pop` never looks at its arguments and reports its arity,
    // which is exactly what the gate-off module reports.
    let call = "xs = [1]\nxs.pop(lambda: 1)\n";
    assert_eq!(
        lower_message(&format!("{ALL_FOUR}{call}")),
        lower_message(call)
    );
}

#[test]
fn the_accessors_answer_none_off_their_shape() {
    let literal = HirExpr::IntLiteral(1);
    assert_eq!(literal.container_form(), None);
    assert_eq!(literal.method_receiver(), None);
    assert!(literal.clone().method_call_parts_mut().is_none());

    // A receiver that is not a bare name, and an arity no container form has.
    let on_a_call = HirExpr::MethodCall {
        base: Box::new(HirExpr::Call {
            callee: "f".to_string(),
            args: Vec::new(),
        }),
        method: "get".to_string(),
        args: Vec::new(),
    };
    assert_eq!(on_a_call.container_form(), None);
    // #1263: `.add()` on an attribute has no container form (no `set`
    // instance slot exists, so the fast path refuses it).
    let add_on_an_attribute = HirExpr::MethodCall {
        base: Box::new(HirExpr::AttrGet {
            base: Box::new(HirExpr::Name("b".to_string())),
            attr: "s".to_string(),
        }),
        method: "add".to_string(),
        args: vec![HirExpr::IntLiteral(1)],
    };
    assert_eq!(add_on_an_attribute.container_form(), None);
    let wrong_arity = HirExpr::MethodCall {
        base: Box::new(HirExpr::Name("d".to_string())),
        method: "get".to_string(),
        args: vec![HirExpr::IntLiteral(1)],
    };
    assert_eq!(wrong_arity.container_form(), None);
    assert!(wrong_arity.clone().method_call_parts_mut().is_some());
}
