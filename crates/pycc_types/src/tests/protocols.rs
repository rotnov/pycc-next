//! Protocol-typed parameters, protocol assignability and qualified markers.

use super::*;

#[test]
fn protocol_typed_parameter_function_call_triggers_monomorphization() {
    // This exercises the rewrite_protocol_calls_in_expr and
    // substitute_body_protocol paths in check_and_resolve.
    let result = check_source(
        "from typing import Protocol\nclass Drawable(Protocol):\n    def draw(self) -> None: ...\nclass Circle:\n    def __init__(self) -> None:\n        self.x = 0\n    def draw(self) -> None:\n        return\ndef render(d: Drawable) -> None:\n    d.draw()\nc = Circle()\nrender(c)\n",
    );
    assert!(
        result.is_ok(),
        "protocol-typed parameter call should type-check"
    );
    // Verify that the monomorphized HIR contains a specialization
    // with the concrete type substituted.
    let resolved = result.unwrap();
    let has_specialization = resolved.items.iter().any(|item| {
        if let HirItem::Function { name, .. } = item {
            name.contains("0gen_")
        } else {
            false
        }
    });
    assert!(
        has_specialization,
        "monomorphized HIR should contain a protocol specialization"
    );
}

#[test]
fn protocol_marker_used_as_a_value_is_t0021() {
    // This directly exercises the marker_is_not_a_value function.
    let diag = marker_is_not_a_value("Protocol");
    assert_eq!(diag.code, "T0021");
    assert!(
        diag.message.contains("marker symbol"),
        "unexpected message: {}",
        diag.message
    );
}

#[test]
fn abc_marker_used_as_a_value_is_t0021() {
    let diag = marker_is_not_a_value("ABC");
    assert_eq!(diag.code, "T0021");
}

#[test]
fn runtime_checkable_decorator_used_as_a_value_is_t0021() {
    let diag = marker_is_not_a_value("runtime_checkable");
    assert_eq!(diag.code, "T0021");
}

#[test]
fn protocol_typed_parameter_with_if_statement_triggers_monomorphization() {
    // This exercises rewrite_protocol_calls_in_stmt's If arm — the
    // call to the protocol-parameter function must be inside a
    // *non-protocol* function's if body.
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\ndef proto_fn(p: P) -> int:\n    return p.foo()\ndef caller() -> None:\n    if True:\n        proto_fn(c)\nc = C()\ncaller()\n",
    );
    assert!(result.is_ok());
}

#[test]
fn protocol_typed_parameter_with_while_loop_triggers_monomorphization() {
    // This exercises rewrite_protocol_calls_in_stmt's While arm.
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\ndef proto_fn(p: P) -> int:\n    return p.foo()\ndef caller() -> None:\n    while False:\n        proto_fn(c)\nc = C()\ncaller()\n",
    );
    assert!(result.is_ok());
}

#[test]
fn protocol_typed_parameter_with_for_range_triggers_monomorphization() {
    // This exercises rewrite_protocol_calls_in_stmt's ForRange arm.
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\ndef proto_fn(p: P) -> int:\n    return p.foo()\ndef caller() -> None:\n    for i in range(1):\n        proto_fn(c)\nc = C()\ncaller()\n",
    );
    assert!(result.is_ok());
}

#[test]
fn protocol_typed_parameter_with_return_triggers_monomorphization() {
    // This exercises rewrite_protocol_calls_in_stmt's Return arm.
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\ndef proto_fn(p: P) -> int:\n    return p.foo()\ndef caller() -> int:\n    return proto_fn(c)\nc = C()\nprint(caller())\n",
    );
    assert!(result.is_ok());
}

#[test]
fn protocol_typed_parameter_with_assignment_triggers_monomorphization() {
    // This exercises rewrite_protocol_calls_in_stmt's Assign arm.
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\ndef proto_fn(p: P) -> int:\n    return p.foo()\ndef caller() -> None:\n    x = proto_fn(c)\nc = C()\ncaller()\n",
    );
    assert!(result.is_ok());
}

#[test]
fn protocol_typed_parameter_with_annassign_triggers_monomorphization() {
    // This exercises rewrite_protocol_calls_in_stmt's AnnAssign arm.
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\ndef proto_fn(p: P) -> int:\n    return p.foo()\ndef caller() -> None:\n    x: int = proto_fn(c)\nc = C()\ncaller()\n",
    );
    assert!(result.is_ok());
}

#[test]
fn isinstance_with_protocol_in_generic_context_triggers_rewrite_special_case() {
    // This exercises the isinstance/issubclass special case in
    // rewrite_generic_calls_in_expr.
    let result = check_source(
        "from typing import Protocol, runtime_checkable\n@runtime_checkable\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\nc = C()\nprint(isinstance(c, P))\n",
    );
    assert!(result.is_ok());
}

#[test]
fn protocol_typed_parameter_with_for_list_triggers_monomorphization() {
    // This exercises rewrite_protocol_calls_in_stmt's ForList arm.
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\ndef proto_fn(p: P) -> int:\n    return p.foo()\ndef caller() -> None:\n    xs = [1, 2]\n    for x in xs:\n        proto_fn(c)\nc = C()\ncaller()\n",
    );
    assert!(result.is_ok());
}

#[test]
fn protocol_typed_parameter_with_expr_stmt_triggers_monomorphization() {
    // This exercises rewrite_protocol_calls_in_stmt's ExprStmt arm.
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\ndef proto_fn(p: P) -> int:\n    return p.foo()\ndef caller() -> None:\n    proto_fn(c)\nc = C()\ncaller()\n",
    );
    assert!(result.is_ok());
}

#[test]
fn protocol_typed_parameter_wrapped_in_a_walrus_triggers_monomorphization() {
    // PEP 572 (#774), deep-review follow-up: `rewrite_protocol_calls_in_expr`
    // is a non-exhaustive recursion (a trailing `_ => {}` catch-all) over
    // only the shapes a protocol-typed call can appear inside. Without a
    // dedicated `HirExpr::NamedExpr` arm, `(x := proto_fn(c))` fell into
    // that catch-all: the call site was never rewritten to its
    // specialization, while `monomorphize_protocol_params` unconditionally
    // drops the original `proto_fn` item regardless, leaving a dangling
    // call to a function no longer in the module. Also reads `x` on the
    // next line, exercising `bind_local_types_in_stmt`'s `ExprStmt` arm
    // pre-binding the walrus target for this same pass.
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\ndef proto_fn(p: P) -> int:\n    return p.foo()\ndef caller() -> None:\n    (x := proto_fn(c))\n    print(x)\nc = C()\ncaller()\n",
    );
    assert!(result.is_ok());
}

#[test]
fn protocol_typed_parameter_with_method_call_triggers_monomorphization() {
    // This exercises rewrite_protocol_calls_in_expr's MethodCall arm.
    // `print(proto_fn(c))` is a Call whose argument is a Call to the
    // protocol-parameter function — the recursion into args hits the
    // Call arm, and the outer print is also a Call.
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\ndef proto_fn(p: P) -> int:\n    return p.foo()\ndef caller() -> None:\n    print(proto_fn(c))\nc = C()\ncaller()\n",
    );
    assert!(result.is_ok());
}

#[test]
fn protocol_typed_parameter_with_attr_get_triggers_monomorphization() {
    // This exercises rewrite_protocol_calls_in_expr's AttrGet arm.
    // We need the protocol function to return an instance so we can
    // access an attribute on the result.
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 42\n    def foo(self) -> int:\n        return 1\nclass Q(Protocol):\n    def get_c(self) -> C: ...\nclass D:\n    def __init__(self) -> None:\n        self.x = 0\n    def get_c(self) -> C:\n        return C()\ndef proto_fn(q: Q) -> C:\n    return q.get_c()\ndef caller() -> None:\n    print(proto_fn(d).x)\nd = D()\ncaller()\n",
    );
    assert!(result.is_ok());
}

#[test]
fn protocol_typed_parameter_with_binop_triggers_monomorphization() {
    // This exercises rewrite_protocol_calls_in_expr's BinOp arm.
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\ndef proto_fn(p: P) -> int:\n    return p.foo()\ndef caller() -> None:\n    x = proto_fn(c) + 1\nc = C()\ncaller()\n",
    );
    assert!(result.is_ok());
}

#[test]
fn protocol_typed_parameter_with_compare_triggers_monomorphization() {
    // This exercises rewrite_protocol_calls_in_expr's Compare arm.
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\ndef proto_fn(p: P) -> int:\n    return p.foo()\ndef caller() -> None:\n    x = proto_fn(c) == 1\nc = C()\ncaller()\n",
    );
    assert!(result.is_ok());
}

#[test]
fn protocol_typed_parameter_with_list_literal_triggers_monomorphization() {
    // This exercises rewrite_protocol_calls_in_expr's ListLiteral arm.
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\ndef proto_fn(p: P) -> int:\n    return p.foo()\ndef caller() -> None:\n    x = [proto_fn(c)]\nc = C()\ncaller()\n",
    );
    assert!(result.is_ok());
}

#[test]
fn protocol_typed_parameter_with_non_protocol_param_covers_protocol_param_name_none() {
    // This exercises the None branch in protocol_param_name (when
    // a non-protocol parameter precedes the protocol parameter).
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\ndef proto_fn(n: int, p: P) -> int:\n    return p.foo()\ndef caller() -> None:\n    proto_fn(0, c)\nc = C()\ncaller()\n",
    );
    assert!(result.is_ok());
}

#[test]
fn protocol_attribute_access_on_unknown_member_is_t0044() {
    // This exercises the t0044_unknown_member error path in
    // resolve_attr_get for protocol-typed variables.
    let err = check_source(
            "from typing import Protocol\nclass P(Protocol):\n    x: int\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\ndef read_unknown(p: P) -> None:\n    print(p.y)\nc = C()\nread_unknown(c)\n",
        )
        .unwrap_err();
    assert_eq!(err.code, "T0044");
}

#[test]
fn protocol_method_call_on_unknown_member_is_t0044() {
    // This exercises the t0044_unknown_member error path in
    // resolve_method_call for protocol-typed variables.
    let err = check_source(
            "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\ndef call_unknown(p: P) -> None:\n    p.bar()\nc = C()\ncall_unknown(c)\n",
        )
        .unwrap_err();
    assert_eq!(err.code, "T0044");
}

#[test]
fn protocol_to_protocol_assignability_with_different_protocols_is_t0046() {
    // This exercises the protocol-to-protocol mismatch path in
    // is_assignable_env and assignable_error.
    let err = check_source(
            "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass Q(Protocol):\n    def bar(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\ndef assign_proto(p: P) -> None:\n    q: Q = p\nc = C()\nassign_proto(c)\n",
        )
        .unwrap_err();
    assert_eq!(err.code, "T0046");
}

#[test]
fn protocol_instantiation_is_c0001() {
    // This exercises the protocol instantiation rejection path.
    let err = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\np = P()\n",
    )
    .unwrap_err();
    assert_eq!(err.code, "C0001");
}

#[test]
fn protocol_typed_annassign_mismatch_is_t0046() {
    // This exercises the protocol annotation mismatch path in
    // AnnAssign checking.
    let err = check_source(
            "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\nc = C()\nx: P = 1\n",
        )
        .unwrap_err();
    assert!(err.code == "T0046" || err.code == "T0021");
}

#[test]
fn protocol_to_same_protocol_assignability_succeeds() {
    // This exercises is_assignable_env's same-protocol arm
    // (class.rs line 242: `return true`).
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\ndef fn(p: P) -> None:\n    q: P = p\nc = C()\nfn(c)\n",
    );
    assert!(result.is_ok());
}

#[test]
fn protocol_to_inherited_protocol_assignability_succeeds() {
    // This exercises is_assignable_env's inherited-protocol arm
    // (class.rs line 248: `return true` when MRO includes to_name).
    // Q inherits P, so Q's MRO includes P. Assigning Q to P should
    // succeed because Q has everything P requires.
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass Q(P):\n    def bar(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\n    def bar(self) -> int:\n        return 2\ndef fn(q: Q) -> None:\n    p: P = q\nc = C()\nfn(c)\n",
    );
    assert!(result.is_ok());
}

#[test]
fn protocol_attribute_access_on_conforming_class_succeeds() {
    // This exercises resolve_attr_get's protocol attribute lookup
    // success path (class.rs line 475: `return Ok(ty.clone())`).
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    x: int\nclass C:\n    def __init__(self) -> None:\n        self.x = 42\ndef fn(p: P) -> None:\n    print(p.x)\nc = C()\nfn(c)\n",
    );
    assert!(result.is_ok());
}

#[test]
fn protocol_function_body_with_if_for_substitute_stmt_protocol() {
    // This exercises substitute_stmt_protocol's If arm (lines 5188-5192)
    // by having an if statement inside a protocol-parameter function.
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\ndef proto_fn(p: P) -> None:\n    if True:\n        p.foo()\nc = C()\nproto_fn(c)\n",
    );
    assert!(result.is_ok());
}

#[test]
fn protocol_function_body_with_while_for_substitute_stmt_protocol() {
    // This exercises substitute_stmt_protocol's While arm.
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\ndef proto_fn(p: P) -> None:\n    while False:\n        p.foo()\nc = C()\nproto_fn(c)\n",
    );
    assert!(result.is_ok());
}

#[test]
fn protocol_function_body_with_for_range_for_substitute_stmt_protocol() {
    // This exercises substitute_stmt_protocol's ForRange arm.
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\ndef proto_fn(p: P) -> None:\n    for i in range(1):\n        p.foo()\nc = C()\nproto_fn(c)\n",
    );
    assert!(result.is_ok());
}

#[test]
fn protocol_function_body_with_for_list_for_substitute_stmt_protocol() {
    // This exercises substitute_stmt_protocol's ForList arm.
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\ndef proto_fn(p: P) -> None:\n    xs = [1]\n    for x in xs:\n        p.foo()\nc = C()\nproto_fn(c)\n",
    );
    assert!(result.is_ok());
}

#[test]
fn protocol_function_body_with_annassign_for_substitute_stmt_protocol() {
    // This exercises substitute_stmt_protocol's AnnAssign arm.
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    x: int\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\ndef proto_fn(p: P) -> None:\n    y: int = p.x\nc = C()\nproto_fn(c)\n",
    );
    assert!(result.is_ok());
}

#[test]
fn protocol_call_in_if_orelse_triggers_monomorphization() {
    // This exercises rewrite_protocol_calls_in_stmt's If orelse
    // iteration (lines 6782-6783).
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\ndef proto_fn(p: P) -> int:\n    return p.foo()\ndef caller() -> None:\n    if False:\n        proto_fn(c)\n    else:\n        proto_fn(c)\nc = C()\ncaller()\n",
    );
    assert!(result.is_ok());
}

#[test]
fn protocol_call_in_method_call_context_triggers_monomorphization() {
    // This exercises rewrite_protocol_calls_in_expr's MethodCall arm
    // (lines 6898-6902). We need a protocol function that returns an
    // instance, then call a method on the result.
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\nclass Q(Protocol):\n    def get_c(self) -> C: ...\nclass D:\n    def __init__(self) -> None:\n        self.x = 0\n    def get_c(self) -> C:\n        return C()\ndef proto_fn(q: Q) -> C:\n    return q.get_c()\ndef caller() -> None:\n    proto_fn(d).foo()\nd = D()\ncaller()\n",
    );
    assert!(result.is_ok());
}

#[test]
fn isinstance_inside_generic_function_triggers_rewrite_special_case() {
    // This exercises the isinstance/issubclass special case in
    // rewrite_generic_calls_in_expr (lines 5560-5569). We need a
    // protocol-typed parameter function to prevent the monomorphization
    // early return, and a generic (unannotated-parameter) function
    // whose body contains isinstance.
    let result = check_source(
        "from typing import Protocol, runtime_checkable\n@runtime_checkable\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\ndef proto_fn(p: P) -> int:\n    return p.foo()\ndef _generic_fn(x) -> int:\n    isinstance(x, P)\n    return x\nc = C()\nproto_fn(c)\nprint(_generic_fn(42))\n",
    );
    assert!(result.is_ok());
}

#[test]
fn protocol_call_with_protocol_typed_arg_covers_non_instance_break() {
    // This exercises the break when arg_ty is Ok(Protocol) instead
    // of Ok(Instance) in rewrite_protocol_calls_in_expr.
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\ndef proto_fn(p: P) -> int:\n    return p.foo()\ndef caller() -> None:\n    x: P = c\n    print(proto_fn(x))\nc = C()\ncaller()\n",
    );
    // This may or may not succeed depending on whether the
    // monomorphization handles protocol-typed args gracefully.
    // We just need to exercise the code path.
    let _ = result;
}

#[test]
fn protocol_call_as_method_argument_covers_methodcall_args() {
    // This exercises the MethodCall arm's args iteration in
    // rewrite_protocol_calls_in_expr (lines 6910-6911).
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\n    def bar(self, y: int) -> int:\n        return y\ndef proto_fn(p: P) -> int:\n    return p.foo()\ndef caller() -> None:\n    c = C()\n    print(c.bar(proto_fn(c)))\ncaller()\n",
    );
    assert!(result.is_ok());
}

#[test]
fn protocol_typed_local_variable_method_call_reaches_mir() {
    // This exercises the `Ty::Protocol(name) => name.as_str()` arm
    // in MIR's `class_def_of` (line 1855) by creating a protocol-
    // typed local variable inside a non-protocol function.
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\ndef caller() -> None:\n    x: P = C()\n    print(x.foo())\ncaller()\n",
    );
    assert!(result.is_ok());
}

#[test]
fn qualified_decorator_marker_called_directly_is_t0021() {
    // This exercises the marker callee path (line 2743) in
    // infer_expr_in's Call arm by using the qualified form
    // `abc.abstractmethod()` which resolves via std_qualified_symbol.
    let err = check_source(
        "class C:\n    def __init__(self) -> None:\n        return\nabc.abstractmethod()\n",
    )
    .unwrap_err();
    assert_eq!(err.code, "T0021");
}

#[test]
fn qualified_decorator_marker_used_as_value_is_t0021() {
    // This exercises the DecoratorMarker arm (line 2557) in
    // infer_expr_in's Name arm by using the qualified form
    // `abc.abstractmethod` as a value (not a call).
    let err = check_source(
        "class C:\n    def __init__(self) -> None:\n        return\nx = abc.abstractmethod\n",
    )
    .unwrap_err();
    assert_eq!(err.code, "T0021");
}

#[test]
fn qualified_decorator_marker_called_in_private_helper_is_t0021() {
    // This exercises the marker callee path (line 1195) in
    // collect_expr_constraints' Call arm (the solver path) by
    // using the qualified form in a private helper.
    let err = check_source(
            "class C:\n    def __init__(self) -> None:\n        return\ndef _helper() -> int:\n    abc.abstractmethod()\n    return 1\n_helper()\n",
        )
        .unwrap_err();
    assert_eq!(err.code, "T0021");
}

#[test]
fn qualified_decorator_marker_as_value_in_private_helper_is_t0021() {
    // This exercises the DecoratorMarker arm (line 1045) in
    // collect_expr_constraints' Name arm (the solver path) by
    // using the qualified form as a value in a private helper.
    let err = check_source(
            "class C:\n    def __init__(self) -> None:\n        return\ndef _helper() -> int:\n    x = abc.abstractmethod\n    return 1\n_helper()\n",
        )
        .unwrap_err();
    assert_eq!(err.code, "T0021");
}

#[test]
fn qualified_annotation_marker_used_as_value_is_t0021() {
    // #762: exercises the dedicated `AnnotationMarker` arm (routed to
    // `annotation_marker_is_not_a_value`, not the generic
    // `marker_is_not_a_value`, per PR #766 review) folded into the marker
    // arm of infer_expr_in's Name handler (expr.rs) by using the
    // qualified form `typing.Final` as a value (not an annotation
    // subscript).
    let err = check_source("import typing\nx = typing.Final\n").unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(
        err.message.contains("annotation marker") && err.message.contains("typing.Final[int]"),
        "expected an annotation-specific message, got: {}",
        err.message
    );
}

#[test]
fn qualified_annotation_marker_as_value_in_private_helper_is_t0021() {
    // #762: exercises the dedicated `AnnotationMarker` arm folded into the
    // marker arm of collect_expr_constraints' Name handler (constraints.rs,
    // the solver path) by using the qualified form `typing.Annotated`
    // as a value in a private helper.
    let err = check_source(
        "import typing\ndef _helper() -> int:\n    x = typing.Annotated\n    return 1\n_helper()\n",
    )
    .unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(
        err.message.contains("annotation marker"),
        "expected an annotation-specific message, got: {}",
        err.message
    );
}

#[test]
fn qualified_annotation_marker_called_is_t0021() {
    // #762 / PR #766 review: exercises the AnnotationMarker branch of the
    // call-site marker guard in expr.rs's infer_expr_in (the `Function`
    // let-else fallthrough), by calling `typing.Final` directly instead of
    // using it as a bare-name value or an annotation subscript.
    let err = check_source("import typing\nx = typing.Final()\n").unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(
        err.message.contains("annotation marker"),
        "expected an annotation-specific message, got: {}",
        err.message
    );
}

#[test]
fn qualified_annotation_marker_called_in_private_helper_is_t0021() {
    // #762 / PR #766 review: exercises the same call-site AnnotationMarker
    // branch in constraints.rs's collect_expr_constraints (the solver
    // path), by calling `typing.Annotated` directly inside a private
    // helper.
    let err = check_source(
        "import typing\ndef _helper() -> int:\n    x = typing.Annotated()\n    return 1\n_helper()\n",
    )
    .unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(
        err.message.contains("annotation marker"),
        "expected an annotation-specific message, got: {}",
        err.message
    );
}

// -- #380 W1: protocol monomorphization through DictSet/AttrSet/comp arms -

#[test]
fn protocol_call_in_dict_set_value_triggers_monomorphization() {
    // This exercises rewrite_protocol_calls_in_stmt's DictSet arm
    // (lines 6851-6854) — the protocol-parameter function call is the
    // *value* of a `d[k] = ...` item assignment.
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\ndef proto_fn(p: P) -> int:\n    return p.foo()\ndef caller() -> None:\n    d = {\"k\": 0}\n    d[\"k\"] = proto_fn(c)\nc = C()\ncaller()\n",
    );
    assert!(result.is_ok());
}

#[test]
fn protocol_call_in_attr_set_value_triggers_monomorphization() {
    // This exercises rewrite_protocol_calls_in_stmt's AttrSet arm
    // (lines 6855-6858) — the protocol-parameter function call is the
    // *value* of an `obj.attr = ...` attribute assignment.
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\ndef proto_fn(p: P) -> int:\n    return p.foo()\ndef caller() -> None:\n    obj = C()\n    obj.x = proto_fn(c)\nc = C()\ncaller()\n",
    );
    assert!(result.is_ok());
}

#[test]
fn protocol_call_in_list_comp_elt_triggers_monomorphization() {
    // This exercises rewrite_protocol_calls_in_stmt's
    // ListCompAssign arm (lines 6859-6865) with no `if` filter —
    // the protocol-parameter function call is the *elt* of a list
    // comprehension, covering the `cond == None` branch.
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\ndef proto_fn(p: P) -> int:\n    return p.foo()\ndef caller() -> None:\n    xs = [proto_fn(c) for i in range(3)]\nc = C()\ncaller()\n",
    );
    assert!(result.is_ok());
}

#[test]
fn protocol_call_in_set_comp_elt_with_cond_triggers_monomorphization() {
    // This exercises rewrite_protocol_calls_in_stmt's
    // SetCompAssign arm (lines 6859-6865, shared with
    // ListCompAssign) with an `if` filter — covering the
    // `cond == Some(c)` branch.
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\ndef proto_fn(p: P) -> int:\n    return p.foo()\ndef caller() -> None:\n    xs = {proto_fn(c) for i in range(3) if i > 0}\nc = C()\ncaller()\n",
    );
    assert!(result.is_ok());
}

#[test]
fn protocol_call_in_dict_comp_value_triggers_monomorphization() {
    // This exercises rewrite_protocol_calls_in_stmt's
    // DictCompAssign arm (lines 6866-6872) with no `if` filter —
    // the protocol-parameter function call is the *value* of a
    // dict comprehension, covering the `cond == None` branch.
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\ndef proto_fn(p: P) -> int:\n    return p.foo()\ndef caller() -> None:\n    xs = {\"k\": proto_fn(c) for i in range(3)}\nc = C()\ncaller()\n",
    );
    assert!(result.is_ok());
}

#[test]
fn protocol_call_in_dict_comp_value_with_cond_triggers_monomorphization() {
    // This exercises rewrite_protocol_calls_in_stmt's
    // DictCompAssign arm (lines 6866-6872) with an `if` filter —
    // covering the `cond == Some(c)` branch for the dict-comp arm.
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\ndef proto_fn(p: P) -> int:\n    return p.foo()\ndef caller() -> None:\n    xs = {\"k\": proto_fn(c) for i in range(3) if i > 0}\nc = C()\ncaller()\n",
    );
    assert!(result.is_ok());
}

// -- Bug 2: function-local variable as protocol call argument ----------

#[test]
fn protocol_call_with_function_local_variable_argument_compiles() {
    // A function-local variable (not a module-level global) passed
    // to a protocol-parameter function must be resolved through the
    // monomorphization pass's child environment, not the module env.
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\ndef proto_fn(p: P) -> int:\n    return p.foo()\ndef caller() -> None:\n    c = C()\n    print(proto_fn(c))\ncaller()\n",
    );
    assert!(result.is_ok());
    let resolved = result.unwrap();
    let has_specialization = resolved.items.iter().any(|item| {
        if let HirItem::Function { name, .. } = item {
            name.contains("0gen_")
        } else {
            false
        }
    });
    assert!(
        has_specialization,
        "monomorphized HIR should contain a protocol specialization for a local-variable argument"
    );
}

#[test]
fn protocol_call_with_local_from_if_block_compiles() {
    // A local variable assigned inside an if block (after being
    // pre-bound) and then passed to a protocol-parameter function —
    // exercises bind_local_types_in_stmt's If arm.
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\ndef proto_fn(p: P) -> int:\n    return p.foo()\ndef caller() -> None:\n    c = C()\n    if True:\n        c = C()\n    print(proto_fn(c))\ncaller()\n",
    );
    assert!(result.is_ok());
}

#[test]
fn protocol_call_with_local_from_for_range_compiles() {
    // A local variable reassigned inside a for-range loop body and
    // then passed to a protocol-parameter function — exercises
    // bind_local_types_in_stmt's ForRange arm.
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\ndef proto_fn(p: P) -> int:\n    return p.foo()\ndef caller() -> None:\n    c = C()\n    for i in range(1):\n        c = C()\n    print(proto_fn(c))\ncaller()\n",
    );
    assert!(result.is_ok());
}

#[test]
fn protocol_call_with_local_from_for_list_compiles() {
    // A for-list loop over a list[int] alongside a protocol call —
    // exercises bind_local_types_in_stmt's ForList arm (binding the
    // loop variable's type from the list's element type).
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\ndef proto_fn(p: P) -> int:\n    return p.foo()\ndef caller() -> None:\n    xs = [1, 2, 3]\n    for x in xs:\n        print(x)\n    c = C()\n    print(proto_fn(c))\ncaller()\n",
    );
    assert!(result.is_ok());
}

#[test]
fn protocol_call_with_local_from_while_block_compiles() {
    // A local variable reassigned inside a while loop body and then
    // passed to a protocol-parameter function — exercises
    // bind_local_types_in_stmt's While arm.
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\ndef proto_fn(p: P) -> int:\n    return p.foo()\ndef caller() -> None:\n    c = C()\n    while False:\n        c = C()\n    print(proto_fn(c))\ncaller()\n",
    );
    assert!(result.is_ok());
}

#[test]
fn protocol_call_with_annassign_local_compiles() {
    // A local variable bound via AnnAssign (with a value) passed to
    // a protocol-parameter function — exercises
    // bind_local_types_in_stmt's AnnAssign arm with a value.
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\ndef proto_fn(p: P) -> int:\n    return p.foo()\ndef caller() -> None:\n    c: C = C()\n    print(proto_fn(c))\ncaller()\n",
    );
    assert!(result.is_ok());
}

#[test]
fn protocol_call_with_valueless_annassign_then_assign_compiles() {
    // A value-less AnnAssign (`c: C`) followed by an assignment —
    // exercises bind_local_types_in_stmt's AnnAssign arm without a
    // value (binding from the annotation type).
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\ndef proto_fn(p: P) -> int:\n    return p.foo()\ndef caller() -> None:\n    c: C\n    c = C()\n    print(proto_fn(c))\ncaller()\n",
    );
    assert!(result.is_ok());
}

// -- Bug 4: nested protocol-parameter function calls -------------------

#[test]
fn nested_protocol_parameter_functions_both_get_specialized() {
    // One protocol-parameter function calling another — the inner
    // call in the specialized body must also be rewritten.
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\ndef inner(p: P) -> int:\n    return p.foo()\ndef outer(p: P) -> int:\n    return inner(p)\nc = C()\nprint(outer(c))\n",
    );
    assert!(result.is_ok());
    let resolved = result.unwrap();
    let specialization_count = resolved
        .items
        .iter()
        .filter(|item| {
            if let HirItem::Function { name, .. } = item {
                name.contains("0gen_")
            } else {
                false
            }
        })
        .count();
    assert!(
        specialization_count >= 2,
        "both inner and outer should be specialized, found {specialization_count}"
    );
}

// -- Bug 5: multiple protocol-typed parameters ------------------------

#[test]
fn multiple_protocol_parameters_all_get_specialized() {
    // A function with two different protocol-typed parameters — both
    // must be substituted with their respective concrete types.
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass Q(Protocol):\n    def bar(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\nclass D:\n    def __init__(self) -> None:\n        self.x = 0\n    def bar(self) -> int:\n        return 2\ndef combine(a: P, b: Q) -> int:\n    return a.foo() + b.bar()\nc = C()\nd = D()\nprint(combine(c, d))\n",
    );
    assert!(result.is_ok());
    let resolved = result.unwrap();
    let has_specialization = resolved.items.iter().any(|item| {
        if let HirItem::Function { name, params, .. } = item {
            name.contains("0gen_combine")
                && params.iter().all(|(_, ty)| !matches!(ty, Ty::Protocol(_)))
        } else {
            false
        }
    });
    assert!(
        has_specialization,
        "combine should be specialized with both protocol types replaced"
    );
}

#[test]
fn multiple_protocol_parameters_same_protocol_compiles() {
    // A function with two parameters of the same protocol type —
    // both should be substituted with the same concrete type.
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\ndef combine(a: P, b: P) -> int:\n    return a.foo() + b.foo()\nc = C()\nprint(combine(c, c))\n",
    );
    assert!(result.is_ok());
}

#[test]
fn protocol_argument_mismatch_emits_t0046() {
    // This exercises the assignable_error call (line 2894) when
    // a non-conforming class is passed to a protocol-typed parameter.
    let err = check_source(
            "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\ndef bar(p: P) -> int:\n    return p.foo()\nbar(C())\n",
        )
        .unwrap_err();
    assert_eq!(err.code, "T0046");
}
