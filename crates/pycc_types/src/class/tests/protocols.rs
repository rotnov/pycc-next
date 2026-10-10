//! Protocol conformance, instantiation refusal and protocol-typed calls.

use super::*;

#[test]
fn protocol_conformance_with_wrong_parameter_count_is_t0046() {
    let err = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self, x: int) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\nc: P = C()\n",
    )
    .unwrap_err();
    assert_eq!(err.code, "T0046");
    assert!(
        err.message.contains("parameter"),
        "unexpected message: {}",
        err.message
    );
}

#[test]
fn protocol_conformance_with_wrong_parameter_type_is_t0046() {
    let err = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self, x: int) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self, x: str) -> int:\n        return 1\nc: P = C()\n",
    )
    .unwrap_err();
    assert_eq!(err.code, "T0046");
    assert!(
        err.message.contains("parameter 1 has type"),
        "unexpected message: {}",
        err.message
    );
}

#[test]
fn protocol_conformance_with_wrong_return_type_is_t0046() {
    let err = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> str:\n        return \"hi\"\nc: P = C()\n",
    )
    .unwrap_err();
    assert_eq!(err.code, "T0046");
    assert!(
        err.message.contains("return type is"),
        "unexpected message: {}",
        err.message
    );
}

#[test]
fn protocol_conformance_with_missing_attribute_is_t0046() {
    let err = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    x: int\nclass C:\n    def __init__(self) -> None:\n        self.y = 0\nc: P = C()\n",
    )
    .unwrap_err();
    assert_eq!(err.code, "T0046");
    assert!(
        err.message.contains("missing attribute"),
        "unexpected message: {}",
        err.message
    );
}

#[test]
fn protocol_conformance_with_wrong_attribute_type_is_t0046() {
    let err = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    x: int\nclass C:\n    def __init__(self) -> None:\n        self.x = \"hi\"\nc: P = C()\n",
    )
    .unwrap_err();
    assert_eq!(err.code, "T0046");
    assert!(
        err.message.contains("attribute `x` has type"),
        "unexpected message: {}",
        err.message
    );
}

// -- #914: a class attribute satisfies a protocol attribute member ----

/// A `#911` class-level attribute satisfies a protocol's attribute
/// member, exactly as an instance slot or a `@property` does: the
/// `ProtocolMember::Attribute` arm falls back to
/// `lookup_class_attr_through_mro` when the instance/property walk
/// misses.
#[test]
fn protocol_conformance_with_class_attribute_succeeds() {
    check_source(
        "from typing import Protocol\nclass P(Protocol):\n    x: int\nclass C:\n    x: int = 1\n    def __init__(self) -> None:\n        self.y = 0\nc: P = C()\n",
    )
    .expect("a class attribute satisfies the protocol's attribute member");
}

/// The fallback walks the full MRO, so a class attribute declared on a
/// *base* satisfies the member for a derived class that declares none
/// of its own.
#[test]
fn protocol_conformance_with_inherited_class_attribute_succeeds() {
    check_source(
        "from typing import Protocol\nclass P(Protocol):\n    x: int\nclass B:\n    x: int = 1\nclass C(B):\n    def __init__(self) -> None:\n        self.y = 0\nc: P = C()\n",
    )
    .expect("an inherited class attribute satisfies the protocol's attribute member");
}

/// A class attribute of the wrong type still fails, with the same
/// "has type" wording an instance attribute produces -- the fallback
/// feeds the existing `is_assignable` branch rather than short-circuiting
/// past it.
#[test]
fn protocol_conformance_with_wrong_typed_class_attribute_is_t0046() {
    let err = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    x: int\nclass C:\n    x: str = \"hi\"\n    def __init__(self) -> None:\n        self.y = 0\nc: P = C()\n",
    )
    .unwrap_err();
    assert_eq!(err.code, "T0046");
    assert!(
        err.message.contains("attribute `x` has type `str`"),
        "unexpected message: {}",
        err.message
    );
}

/// Pins the `.or_else` *ordering*. `reject_class_attr_collisions` only
/// compares a class's own `class_attrs` against its own MRO, so two
/// independent sibling bases can contribute an instance slot and a
/// same-named class attribute to one derived class without ever being
/// compared. The instance walk runs first, so the member resolves to
/// `A.x`'s `int` -- CPython's own answer, since an instance `__dict__`
/// entry shadows a non-data-descriptor class attribute. Swapping the
/// two walks would resolve it to `B.x`'s `str` and reject this program.
/// The same shape used to expose a MIR-side value divergence for ordinary
/// attribute reads (#960): `pycc_mir` folded the class attribute before
/// looking for the slot, so `c.x` printed `B`'s value and, for the `str`
/// spelling above, aborted codegen outright. `pycc_mir`'s instance
/// `AttrGet` arm now mirrors this order, so the two layers agree.
#[test]
fn protocol_conformance_prefers_a_sibling_base_instance_attribute() {
    check_source(
        "from typing import Protocol\nclass P(Protocol):\n    x: int\nclass A:\n    def __init__(self) -> None:\n        self.x = 1\nclass B:\n    x: str = \"hi\"\nclass C(A, B):\n    pass\nc: P = C()\n",
    )
    .expect("the sibling base's instance attribute wins over the class attribute");
}

#[test]
fn protocol_conformance_with_contravariant_param_narrowing_is_t0046() {
    let err = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self, x: int) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self, x: bool) -> int:\n        return 1\nc: P = C()\n",
    )
    .unwrap_err();
    assert_eq!(err.code, "T0046");
    assert!(
        err.message.contains("parameter 1 has type"),
        "unexpected message: {}",
        err.message
    );
}

#[test]
fn protocol_conformance_with_matching_method_succeeds() {
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self, x: int) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self, x: int) -> int:\n        return x\nc: P = C()\n",
    );
    assert!(result.is_ok(), "conforming class should type-check");
}

#[test]
fn protocol_conformance_with_matching_attribute_succeeds() {
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    x: int\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\nc: P = C()\n",
    );
    assert!(result.is_ok(), "conforming class should type-check");
}

#[test]
fn protocol_to_protocol_assignability_with_different_protocols_is_t0046() {
    let err = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass Q(Protocol):\n    def bar(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\nc: P = C()\nq: Q = c\n",
    )
    .unwrap_err();
    assert_eq!(err.code, "T0046");
}

#[test]
fn protocol_cannot_be_instantiated() {
    let err = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\np = P()\n",
    )
    .unwrap_err();
    assert_eq!(err.code, "C0001");
    assert!(
        err.message.contains("cannot instantiate protocol"),
        "unexpected message: {}",
        err.message
    );
}

#[test]
fn abstract_class_cannot_be_instantiated() {
    // #380 (PR-20): exercises `resolve_instantiation`'s
    // `is_abstract` error path. Covered here as a unit test to
    // avoid cargo-llvm-cov issue #276.
    let err = check_source(
        "from abc import ABC, abstractmethod\nclass A(ABC):\n    def __init__(self) -> None:\n        self.x = 0\n    @abstractmethod\n    def foo(self) -> int: ...\na = A()\n",
    )
    .unwrap_err();
    assert_eq!(err.code, "C0001");
    assert!(
        err.message.contains("cannot instantiate abstract class"),
        "unexpected message: {}",
        err.message
    );
}

#[test]
fn protocol_typed_variable_unknown_attribute_is_t0044() {
    let err = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\nc: P = C()\nprint(c.bar)\n",
    )
    .unwrap_err();
    assert_eq!(err.code, "T0044");
}

#[test]
fn protocol_typed_variable_unknown_method_is_t0044() {
    let err = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\nc: P = C()\nc.bar()\n",
    )
    .unwrap_err();
    assert_eq!(err.code, "T0044");
}

#[test]
fn protocol_typed_variable_method_call_wrong_arity_is_t0023() {
    // #380 (PR-20): exercises `resolve_method_call`'s `check_call_args`
    // error path for a protocol-typed variable — calling a protocol
    // method with the wrong number of arguments. Covered here as a
    // unit test to avoid cargo-llvm-cov issue #276.
    //
    // NOTE: A top-level `c: P = C()` binds `c` as the concrete
    // `Ty::Instance("C")` (D-040 sticky representation), so the call
    // resolves through the instance-method path, not the protocol
    // path.  To exercise the *protocol* branch of
    // `class::method_call::resolve_method_call` (moved there by
    // #815, Part 1 of #737), the receiver must be a function
    // parameter annotated `x: P`, which is bound as
    // `Ty::Protocol("P")` in the function body.
    let err = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\ndef f(x: P) -> int:\n    return x.foo(99)\n",
    )
    .unwrap_err();
    assert_eq!(err.code, "T0021");
}

#[test]
fn protocol_typed_param_method_call_wrong_arg_type_is_t0021() {
    // #380 (PR-20): exercises the `check_call_args` *type-mismatch*
    // error path (not just arity) inside
    // `class::method_call::resolve_method_call`'s protocol branch
    // (moved there by #815, Part 1 of #737).  The receiver is a
    // function parameter typed `x: P`, so it is bound as
    // `Ty::Protocol("P")` and the protocol branch is entered.  The
    // argument count matches
    // (1 == 1) but the type (`str` vs `int`) does not, so
    // `check_call_args` returns a type-mismatch T0021.
    let err = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self, x: int) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self, x: int) -> int:\n        return x\ndef f(x: P) -> int:\n    return x.foo(\"hello\")\n",
    )
    .unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(
        err.message.contains("argument 1"),
        "unexpected message: {}",
        err.message
    );
}

#[test]
fn isinstance_against_non_runtime_checkable_protocol_is_c0001() {
    let err = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\nc = C()\nprint(isinstance(c, P))\n",
    )
    .unwrap_err();
    assert_eq!(err.code, "C0001");
    assert!(
        err.message.contains("not `@runtime_checkable`"),
        "unexpected message: {}",
        err.message
    );
}

#[test]
fn issubclass_with_protocol_first_arg_is_c0001() {
    let err = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nprint(issubclass(P, int))\n",
    )
    .unwrap_err();
    assert_eq!(err.code, "C0001");
    assert!(
        err.message.contains("`issubclass` with protocol"),
        "unexpected message: {}",
        err.message
    );
}

#[test]
fn issubclass_against_protocol_target_is_c0001() {
    let err = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\nprint(issubclass(C, P))\n",
    )
    .unwrap_err();
    assert_eq!(err.code, "C0001");
    assert!(
        err.message.contains("`issubclass` against protocol"),
        "unexpected message: {}",
        err.message
    );
}

// -- #380 W2: @property satisfies a protocol attribute requirement -----

/// Verifies that a class with a `@property` getter (but no direct
/// attribute slot) satisfies a protocol's attribute requirement.
/// This exercises the `properties` check added to
/// `lookup_attr_through_mro` (lines 233-242): the protocol requires
/// attribute `x: int`, the class has no `x` in its `attrs` table, but
/// it does have a `@property` getter for `x` whose return type is
/// `int`. `check_protocol_conformance` should return `Ok(())`.
#[test]
fn protocol_conformance_with_property_satisfying_attribute_succeeds() {
    use pycc_hir::PropertyDef;

    // --- Protocol "P": requires attribute `x: int` ---
    let proto_def = HirClassDef {
        class_attrs: Vec::new(),
        exception_type_tag: None,
        name: "P".to_string(),
        bases: Vec::new(),
        mro: vec!["P".to_string()],
        attrs: Vec::new(),
        methods: Vec::new(),
        type_param: None,
        properties: Vec::new(),
        static_methods: Vec::new(),
        class_methods: Vec::new(),
        is_enum: false,
        implicit_object_init: false,
        method_defaults: Vec::new(),
        enum_members: Vec::new(),
        is_dataclass: false,
        dataclass_fields: Vec::new(),
        is_protocol: true,
        runtime_checkable: false,
        protocol_members: vec![pycc_hir::ProtocolMember::Attribute {
            name: "x".to_string(),
            ty: Ty::Int,
        }],
        abstract_methods: Vec::new(),
        is_abstract: false,
    };

    // --- Concrete class "C": `@property` getter for `x`, no `x` attr ---
    let c_self_ty = Ty::Instance(Box::new("C".to_string()));
    let c_init = HirItem::Function {
        name: "C.__init__".to_string(),
        params: vec![("self".to_string(), c_self_ty.clone())],
        return_ty: Ty::None,
        body: vec![
            HirStmt::AttrSet {
                base: HirExpr::Name("self".to_string()),
                attr: "_val".to_string(),
                value: HirExpr::IntLiteral(0),
            },
            HirStmt::Return(None),
        ],
    };
    let c_getter = HirItem::Function {
        name: "C.x".to_string(),
        params: vec![("self".to_string(), c_self_ty.clone())],
        return_ty: Ty::Int,
        body: vec![HirStmt::Return(Some(HirExpr::AttrGet {
            base: Box::new(HirExpr::Name("self".to_string())),
            attr: "_val".to_string(),
        }))],
    };
    let c_def = HirClassDef {
        class_attrs: Vec::new(),
        exception_type_tag: None,
        name: "C".to_string(),
        bases: Vec::new(),
        mro: vec!["C".to_string()],
        // Deliberately NO `x` in `attrs` — the property must satisfy
        // the protocol requirement, not a direct attribute slot.
        attrs: vec![("_val".to_string(), Ty::Int)],
        methods: vec![("__init__".to_string(), "C.__init__".to_string())],
        type_param: None,
        properties: vec![PropertyDef {
            name: "x".to_string(),
            getter: "C.x".to_string(),
            setter: None,
        }],
        static_methods: Vec::new(),
        class_methods: Vec::new(),
        is_enum: false,
        implicit_object_init: false,
        method_defaults: Vec::new(),
        enum_members: Vec::new(),
        is_dataclass: false,
        dataclass_fields: Vec::new(),
        is_protocol: false,
        runtime_checkable: false,
        protocol_members: Vec::new(),
        abstract_methods: Vec::new(),
        is_abstract: false,
    };

    // --- Trigger `check_protocol_conformance` via `c: P = C()` ---
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            c_init,
            c_getter,
            top_level(HirStmt::AnnAssign {
                target: "c".to_string(),
                annotation: Ty::Protocol(Box::new("P".to_string())),
                value: Some(HirExpr::Call {
                    callee: "C".to_string(),
                    args: vec![],
                }),
                is_final: false,
            }),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: vec![("P".to_string(), proto_def), ("C".to_string(), c_def)],
    };

    check(&hir).expect(
        "a class with a @property getter should satisfy a protocol \
         attribute requirement",
    );
}

/// #380 W2: covers the defensive panic in `lookup_attr_through_mro`
/// (lines 234-239) — a property's getter is in the class's own
/// property table but was never registered in
/// `Environment::functions`. This "declared shape and Environment
/// disagree" scenario is unreachable from any real `check`-validated
/// program, mirroring `resolve_attr_get`'s own analogous panic test.
#[test]
#[should_panic(expected = "was not registered as an ordinary function")]
fn lookup_attr_through_mro_panics_when_a_property_getter_is_not_registered() {
    use pycc_hir::PropertyDef;
    let mut env = crate::Environment::new();
    env.bind_class(
        "Ghost".to_string(),
        HirClassDef {
            class_attrs: Vec::new(),
            exception_type_tag: None,
            name: "Ghost".to_string(),
            bases: Vec::new(),
            mro: vec!["Ghost".to_string()],
            attrs: vec![],
            methods: vec![("__init__".to_string(), "Ghost.__init__".to_string())],
            type_param: None,
            properties: vec![PropertyDef {
                name: "x".to_string(),
                getter: "Ghost.x".to_string(),
                setter: None,
            }],
            static_methods: Vec::new(),
            class_methods: Vec::new(),
            is_enum: false,
            implicit_object_init: false,
            method_defaults: Vec::new(),
            enum_members: Vec::new(),
            is_dataclass: false,
            dataclass_fields: Vec::new(),
            is_protocol: false,
            runtime_checkable: false,
            protocol_members: Vec::new(),
            abstract_methods: Vec::new(),
            is_abstract: false,
        },
    );
    let _ = super::super::lookup_attr_through_mro(&env, &["Ghost".to_string()], "x");
}
