//! `super()` resolution and class attribute reads/writes.

use super::*;

#[test]
fn infer_expr_in_rejects_call_before_def_in_top_level() {
    // Directly exercises the call-before-def check in infer_expr_in's
    // HirExpr::Call arm for top-level code (not inside a function body).
    let mut env = Environment::new();
    // Manually insert into functions without adding to defined_functions,
    // simulating a function that exists but hasn't been "executed" yet
    // in source order.
    Arc::make_mut(&mut env.functions).insert("foo".to_string(), (vec![], Ty::None));
    // Do NOT insert "foo" into defined_functions -- simulates a call
    // before the def's source-order position.
    let expr = HirExpr::Call {
        callee: "foo".to_string(),
        args: vec![],
    };
    let err = infer_expr_in(&env, &[], &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(
        err.message
            .contains("cannot call function `foo` before its definition")
    );
}

#[test]
fn infer_expr_in_resolves_super_attr_get() {
    // Directly exercises the HirExpr::Super arm of infer_expr_in's
    // HirExpr::AttrGet branch (#433). The resolved member is a base
    // class *property* rather than an instance attribute: since #587
    // only class-level members resolve through `super()`, a property
    // is what proves this arm still routes to a successful
    // `resolve_super_attr_get` rather than only to its rejection.
    use pycc_hir::PropertyDef;
    let mut env = Environment::new();
    env.current_class = Some("B".to_string());
    // #915: a receiver-less `super()` is now `C0001`, so this fixture binds
    // the `self` a real method body's environment carries.
    env.bind("self".to_string(), Ty::Instance(Box::new("B".to_string())));
    env.bind_class(
        "A".to_string(),
        HirClassDef {
            class_attrs: Vec::new(),
            exception_type_tag: None,
            name: "A".to_string(),
            bases: vec![],
            mro: vec!["A".to_string()],
            attrs: vec![("_x".to_string(), Ty::Int)],
            methods: vec![("__init__".to_string(), "A.__init__".to_string())],
            type_param: None,
            properties: vec![PropertyDef {
                name: "x".to_string(),
                getter: "A.x".to_string(),
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
    env.bind_class(
        "B".to_string(),
        HirClassDef {
            class_attrs: Vec::new(),
            exception_type_tag: None,
            name: "B".to_string(),
            bases: vec!["A".to_string()],
            mro: vec!["B".to_string(), "A".to_string()],
            attrs: vec![],
            methods: vec![("__init__".to_string(), "B.__init__".to_string())],
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
            is_protocol: false,
            runtime_checkable: false,
            protocol_members: Vec::new(),
            abstract_methods: Vec::new(),
            is_abstract: false,
        },
    );
    env.bind_function(
        "A.x".to_string(),
        vec![Ty::Instance(Box::new("A".to_string()))],
        Ty::Int,
    );
    let expr = HirExpr::AttrGet {
        base: Box::new(HirExpr::Super),
        attr: "x".to_string(),
    };
    assert_eq!(infer_expr_in(&env, &[], &expr), Ok(Ty::Int));
}

#[test]
fn infer_expr_in_resolves_super_method_call() {
    // Directly exercises the HirExpr::Super arm of infer_expr_in's
    // HirExpr::MethodCall branch (#433), including the argument-
    // inference closure (`.map(|arg| infer_expr_in(...))`).
    let mut env = Environment::new();
    env.current_class = Some("B".to_string());
    // #915: a receiver-less `super()` is now `C0001`, so this fixture binds
    // the `self` a real method body's environment carries.
    env.bind("self".to_string(), Ty::Instance(Box::new("B".to_string())));
    env.bind_class(
        "A".to_string(),
        HirClassDef {
            class_attrs: Vec::new(),
            exception_type_tag: None,
            name: "A".to_string(),
            bases: vec![],
            mro: vec!["A".to_string()],
            attrs: vec![],
            methods: vec![("add".to_string(), "A.add".to_string())],
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
            is_protocol: false,
            runtime_checkable: false,
            protocol_members: Vec::new(),
            abstract_methods: Vec::new(),
            is_abstract: false,
        },
    );
    env.bind_class(
        "B".to_string(),
        HirClassDef {
            class_attrs: Vec::new(),
            exception_type_tag: None,
            name: "B".to_string(),
            bases: vec!["A".to_string()],
            mro: vec!["B".to_string(), "A".to_string()],
            attrs: vec![],
            methods: vec![("__init__".to_string(), "B.__init__".to_string())],
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
            is_protocol: false,
            runtime_checkable: false,
            protocol_members: Vec::new(),
            abstract_methods: Vec::new(),
            is_abstract: false,
        },
    );
    env.bind_function(
        "A.add".to_string(),
        vec![Ty::Instance(Box::new("A".to_string())), Ty::Int],
        Ty::Int,
    );
    let expr = HirExpr::MethodCall {
        base: Box::new(HirExpr::Super),
        method: "add".to_string(),
        args: vec![HirExpr::IntLiteral(1)],
    };
    assert_eq!(infer_expr_in(&env, &[], &expr), Ok(Ty::Int));
}

#[test]
fn infer_expr_in_super_method_call_propagates_arg_error() {
    // Exercises the `?` error-propagation path in the super().method()
    // argument-inference closure (`.collect::<Result<Vec<_>, _>>()?`).
    // An undefined name in the argument list causes `infer_expr_in` to
    // return `Err`, which the `?` operator propagates.
    let mut env = Environment::new();
    env.current_class = Some("B".to_string());
    env.bind_class(
        "A".to_string(),
        HirClassDef {
            class_attrs: Vec::new(),
            exception_type_tag: None,
            name: "A".to_string(),
            bases: vec![],
            mro: vec!["A".to_string()],
            attrs: vec![],
            methods: vec![("add".to_string(), "A.add".to_string())],
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
            is_protocol: false,
            runtime_checkable: false,
            protocol_members: Vec::new(),
            abstract_methods: Vec::new(),
            is_abstract: false,
        },
    );
    env.bind_class(
        "B".to_string(),
        HirClassDef {
            class_attrs: Vec::new(),
            exception_type_tag: None,
            name: "B".to_string(),
            bases: vec!["A".to_string()],
            mro: vec!["B".to_string(), "A".to_string()],
            attrs: vec![],
            methods: vec![("__init__".to_string(), "B.__init__".to_string())],
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
            is_protocol: false,
            runtime_checkable: false,
            protocol_members: Vec::new(),
            abstract_methods: Vec::new(),
            is_abstract: false,
        },
    );
    env.bind_function(
        "A.add".to_string(),
        vec![Ty::Instance(Box::new("A".to_string())), Ty::Int],
        Ty::Int,
    );
    let expr = HirExpr::MethodCall {
        base: Box::new(HirExpr::Super),
        method: "add".to_string(),
        args: vec![HirExpr::Name("undefined_var".to_string())],
    };
    let err = infer_expr_in(&env, &[], &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
}

// -- #911 (Part 1 of #885): class-level attributes at the checking seam ----
//
// `tests/issue_911_class_attrs.rs` drives these same programs through the
// compiled `pycc` binary; the tests below pin the same contract directly at
// this crate's own seams -- `class::resolve_attr_get`'s class-attribute
// fallback, `class::check_attr_set`'s write rejection, `expr`'s class-name
// read and its plain-name base restriction, and `check_class_pattern`'s
// keyword rejection.

/// A class with one class attribute of each supported scalar type, plus a
/// single instance slot. `body` is appended at module level.
fn class_attr_source(body: &str) -> String {
    format!(
        "class C:\n    X: int = 1\n    K: str = \"a\"\n\n    def __init__(self) -> None:\n        self.n = 0\n\n\n{body}"
    )
}

#[test]
fn a_class_attribute_read_through_an_instance_type_checks() {
    // `resolve_attr_get`'s class-attribute fallback, reached only after the
    // property walk and the instance-slot walk both miss.
    check_source(&class_attr_source("c = C()\nprint(c.X)\nprint(c.K)\n"))
        .expect("an instance read of a class attribute must type-check");
}

#[test]
fn a_class_attribute_read_through_the_class_name_type_checks() {
    // `infer_expr_in`'s class-name attribute arm -- the only class-name
    // attribute read pycc supports.
    check_source(&class_attr_source("print(C.X)\n"))
        .expect("a class-name read of a class attribute must type-check");
}

#[test]
fn an_unknown_class_name_attribute_on_a_class_with_class_attributes_is_t0044() {
    let err = check_source(&class_attr_source("print(C.nope)\n"))
        .expect_err("an unknown class-name attribute must be rejected");
    assert_eq!(err.code, "T0044");
}

#[test]
fn a_class_attribute_read_off_a_non_name_base_is_rejected() {
    // The fold discards the base expression, so the base is restricted to a
    // bare name; a call-shaped base would otherwise be silently dropped.
    let err = check_source(
        "class C:\n    X: int = 1\n\n    def __init__(self) -> None:\n        self.n = 0\n\n\ndef make() -> C:\n    return C()\n\n\nprint(make().X)\n",
    )
    .expect_err("a call-shaped base must be rejected");
    assert_eq!(err.code, "T0044");
    assert!(
        err.message
            .contains("can only be read through a plain name"),
        "unexpected diagnostic: {}",
        err.message
    );
}

#[test]
fn writing_a_class_attribute_is_rejected_by_check_attr_set() {
    // Both writes that actually reach `check_attr_set`: through an instance
    // binding, and through `self` in a method other than `__init__` (a write
    // inside `__init__` is rejected earlier, by `pycc_hir`).
    for source in [
        class_attr_source("c = C()\nc.X = 5\n"),
        "class C:\n    X: int = 1\n\n    def __init__(self) -> None:\n        self.n = 0\n\n    def bump(self) -> None:\n        self.X = 5\n"
            .to_string(),
    ] {
        let err = check_source(&source).expect_err("a class-attribute write must be rejected");
        assert_eq!(err.code, "T0044");
        assert!(
            err.message
                .contains("it is a class-level attribute of class `C`"),
            "unexpected diagnostic: {}",
            err.message
        );
    }
}

#[test]
fn a_subclass_init_writing_an_inherited_class_attribute_name_is_rejected() {
    // The one shape `pycc_hir::class::body::reject_class_attr_collisions`
    // does *not* cover: the class attribute lives on an ancestor and the
    // subclass's `__init__` declares an instance slot of the same name, so
    // HIR sees no collision within either class's own tables. Only
    // `check_attr_set`'s full-MRO `lookup_class_attr_through_mro` walk stops
    // it -- without which `pycc_mir`'s instance-read fold would walk the same
    // MRO and fold `d.LIMIT` to the ancestor's `1` while the write targeted a
    // real slot. Narrowing that walk to the class's own `class_attrs` must
    // fail this test. The message names the receiver's own class (`Derived`),
    // since the lookup starts there and walks up to `Base`'s declaration.
    let err = check_source(
        "class Base:\n    LIMIT: int = 1\n\n    def __init__(self) -> None:\n        self.n = 0\n\n\nclass Derived(Base):\n    def __init__(self) -> None:\n        self.LIMIT = 5\n",
    )
    .expect_err("a subclass write to an inherited class-attribute name must be rejected");
    assert_eq!(err.code, "T0044");
    assert!(
        err.message
            .contains("it is a class-level attribute of class `Derived`"),
        "unexpected diagnostic: {}",
        err.message
    );
}

#[test]
fn a_class_attribute_class_pattern_keyword_is_rejected() {
    // PEP 634: a class attribute has no per-instance value to match against.
    let err = check_source(&class_attr_source(
        "c = C()\nmatch c:\n    case C(X=v):\n        print(v)\n    case _:\n        print(0)\n",
    ))
    .expect_err("a class-attribute class-pattern keyword must be rejected");
    assert_eq!(err.code, "T0044");
    assert!(
        err.message.contains("not an instance attribute"),
        "unexpected diagnostic: {}",
        err.message
    );
}

// -- #915: `super().CLASS_CONST` reaches a base class's class attribute ----
//
// `tests/issue_915_super_class_attr.rs` drives these programs through the
// compiled `pycc` binary and asserts their runtime output; the tests below
// pin the same contract at this crate's own seam, `class::resolve_super_attr_get`.

#[test]
fn a_class_attribute_read_through_super_type_checks() {
    // The new class-attribute loop, reached after the property walk misses.
    check_source(
        "class Base:\n    X: int = 1\n\n    def __init__(self) -> None:\n        self.n = 0\n\n\nclass Derived(Base):\n    def __init__(self) -> None:\n        self.n = 0\n\n    def read(self) -> int:\n        return super().X\n\n\nprint(Derived().read())\n",
    )
    .expect("`super().X` must reach a base class's class attribute");
}

#[test]
fn a_class_attribute_on_the_second_base_of_a_diamond_is_reached_through_super() {
    // The loop must walk the *current class's* post-current MRO slice, not
    // the first base's own MRO -- `C` is reachable only through `D`'s
    // linearization `[D, B, C, A, object]`.
    check_source(
        "class A:\n    def __init__(self) -> None:\n        self.n = 0\n\n\nclass B(A):\n    def __init__(self) -> None:\n        self.n = 0\n\n\nclass C(A):\n    Y: int = 42\n\n    def __init__(self) -> None:\n        self.n = 0\n\n\nclass D(B, C):\n    def __init__(self) -> None:\n        self.n = 0\n\n    def read(self) -> int:\n        return super().Y\n\n\nprint(D().read())\n",
    )
    .expect("`super().Y` must reach a class attribute on the second base of a diamond");
}

#[test]
fn an_instance_attribute_on_a_sibling_base_does_not_mask_a_class_attribute() {
    // The class-attribute loop runs before the `T0047` instance-attribute
    // rejection: a CPython `super` object never sees the instance
    // `__dict__`, so `C`'s `self.X` must not mask `B`'s class attribute.
    // CPython prints `1` for this program; pycc rejected it with `T0047`
    // before #915.
    //
    // #969 narrowed the original program used here: it also gave `B` its own
    // `self.n`, which makes `B`'s flat instance layout `[n]` while `D`'s is
    // `[X, n]` -- not a prefix -- so `pycc_hir`'s layout gate now rejects that
    // class shape with `C0001` before the checker ever runs (D-234). Dropping
    // `B`'s instance attribute leaves `B`'s layout empty, which is a prefix of
    // everything, and exercises exactly the same `super()` resolution path.
    // The rejected shape is pinned in `tests/issue_969_mi_slot_layout.rs`.
    check_source(
        "class B:\n    X: int = 1\n\n\nclass C:\n    def __init__(self) -> None:\n        self.X = 5\n\n\nclass D(B, C):\n    def read(self) -> int:\n        return super().X\n\n\nprint(D().read())\n",
    )
    .expect("a sibling base's instance attribute must not mask a class attribute");
}

#[test]
fn a_class_attribute_declared_only_on_the_current_class_is_not_reachable_through_super() {
    // `mro[current_pos + 1..]`, not the full MRO: CPython raises
    // `AttributeError: 'super' object has no attribute 'X'` here.
    let err = check_source(
        "class Base:\n    def __init__(self) -> None:\n        self.n = 0\n\n\nclass Derived(Base):\n    X: int = 7\n\n    def __init__(self) -> None:\n        self.n = 0\n\n    def read(self) -> int:\n        return super().X\n\n\nprint(Derived().read())\n",
    )
    .expect_err("a class attribute declared only on the current class is not reachable");
    assert_eq!(err.code, "T0044");
}

#[test]
fn super_inside_a_classmethod_is_rejected_as_a_capability_gap() {
    // #915: a `@classmethod` has no `self` receiver for `pycc_mir`'s
    // `Super` arm to bind, so the form is a `C0001` capability gap rather
    // than a compiler abort.
    let err = check_source(
        "class Base:\n    X: int = 1\n\n    def __init__(self) -> None:\n        self.n = 0\n\n\nclass Derived(Base):\n    def __init__(self) -> None:\n        self.n = 0\n\n    @classmethod\n    def read(cls) -> int:\n        return super().X\n\n\nprint(Derived.read())\n",
    )
    .expect_err("`super()` inside a `@classmethod` must be rejected");
    assert_eq!(err.code, "C0001");
}

#[test]
fn super_method_call_inside_a_staticmethod_is_rejected_as_a_capability_gap() {
    let err = check_source(
        "class Base:\n    def __init__(self) -> None:\n        self.n = 0\n\n    def m(self) -> int:\n        return 1\n\n\nclass Derived(Base):\n    def __init__(self) -> None:\n        self.n = 0\n\n    @staticmethod\n    def read() -> int:\n        return super().m()\n\n\nprint(Derived.read())\n",
    )
    .expect_err("`super().m()` inside a `@staticmethod` must be rejected");
    assert_eq!(err.code, "C0001");
}

#[test]
fn a_class_attribute_earlier_in_the_slice_outranks_a_later_property() {
    // One pass over the slice, all member kinds per class: `B` contributes
    // the class attribute and the later `C` a `@property` of the same name.
    // CPython yields the class attribute (`int`), not the property.
    check_source(
        "class A:\n    def __init__(self) -> None:\n        self.n = 0\n\n\nclass B(A):\n    X: int = 1\n\n    def __init__(self) -> None:\n        self.n = 0\n\n\nclass C(A):\n    def __init__(self) -> None:\n        self.n = 0\n\n    @property\n    def X(self) -> int:\n        return 99\n\n\nclass D(B, C):\n    def __init__(self) -> None:\n        self.n = 0\n\n    def read(self) -> int:\n        return super().X\n\n\nprint(D().read())\n",
    )
    .expect("an earlier class attribute must outrank a later property");
}
