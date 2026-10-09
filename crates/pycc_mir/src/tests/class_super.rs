//! `super()` lowering.
//!
//! Covers direct calls to the base `__init__`, base methods, and base
//! property getters, plus the internal-error panics raised when a bare
//! `super()` or a `super()` base outside a method body reaches MIR.

use crate::*;
use pycc_hir::{HirClassDef, HirExpr, HirItem, HirModule, HirStmt, Ty};

// #433: a bare `HirExpr::Super` should never reach MIR lowering —
// HIR lowering rejects a standalone `super()` with C0001, and
// `super().method()`/`super().attr` are handled by the special-case
// blocks before recursing into `lower_expr` for the base. This test
// bypasses the type checker with a hand-built HIR to exercise the
// panic arm, matching this file's own established internal-error-test
// convention.
#[test]
#[should_panic(expected = "pycc_mir: internal error: a bare `HirExpr::Super` reached MIR lowering")]
fn bare_super_reaching_mir_panics_with_an_internal_error() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Super))],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let _ = build(&hir);
}

// #449: `current_class.expect(...)` panic paths in the Super-base blocks
// of `HirExpr::AttrGet` and `HirExpr::MethodCall`. A `HirExpr::Super` as
// the base of an `AttrGet` or `MethodCall` at top level (outside a method
// body, where `current_class` is `None`) should never reach MIR lowering
// — HIR lowering rejects it with C0001. These tests bypass the type
// checker with a hand-built HIR to exercise the panic arms, matching this
// file's own established internal-error-test convention.

#[test]
#[should_panic(
    expected = "pycc_mir: internal error: `HirExpr::Super` reached lower_expr outside a method body"
)]
fn super_attr_get_outside_method_panics_with_an_internal_error() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::AttrGet {
            base: Box::new(HirExpr::Super),
            attr: "x".to_string(),
        }))],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let _ = build(&hir);
}

#[test]
#[should_panic(
    expected = "pycc_mir: internal error: `HirExpr::Super` reached lower_expr outside a method body"
)]
fn super_method_call_outside_method_panics_with_an_internal_error() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::TopLevelStmt(HirStmt::ExprStmt(
            HirExpr::MethodCall {
                base: Box::new(HirExpr::Super),
                method: "f".to_string(),
                args: vec![],
            },
        ))],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let _ = build(&hir);
}

// #433: super() MIR lowering tests.

/// Helper: builds a minimal two-class HIR module where `B.__init__`
/// calls `super().__init__()` and `B.greet` calls `super().greet()`.
fn super_module() -> HirModule {
    let self_a = Ty::Instance(Box::new("A".to_string()));
    let self_b = Ty::Instance(Box::new("B".to_string()));
    HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            // A.__init__
            HirItem::Function {
                name: "A.__init__".to_string(),
                params: vec![("self".to_string(), self_a.clone())],
                return_ty: Ty::None,
                body: vec![HirStmt::AttrSet {
                    base: HirExpr::Name("self".to_string()),
                    attr: "x".to_string(),
                    value: HirExpr::IntLiteral(1),
                }],
            },
            // A.greet
            HirItem::Function {
                name: "A.greet".to_string(),
                params: vec![("self".to_string(), self_a.clone())],
                return_ty: Ty::Int,
                body: vec![HirStmt::Return(Some(HirExpr::AttrGet {
                    base: Box::new(HirExpr::Name("self".to_string())),
                    attr: "x".to_string(),
                }))],
            },
            // B.__init__ — calls super().__init__()
            HirItem::Function {
                name: "B.__init__".to_string(),
                params: vec![("self".to_string(), self_b.clone())],
                return_ty: Ty::None,
                body: vec![HirStmt::ExprStmt(HirExpr::MethodCall {
                    base: Box::new(HirExpr::Super),
                    method: "__init__".to_string(),
                    args: vec![],
                })],
            },
            // B.greet — calls super().greet()
            HirItem::Function {
                name: "B.greet".to_string(),
                params: vec![("self".to_string(), self_b)],
                return_ty: Ty::Int,
                body: vec![HirStmt::Return(Some(HirExpr::MethodCall {
                    base: Box::new(HirExpr::Super),
                    method: "greet".to_string(),
                    args: vec![],
                }))],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: vec![
            (
                "A".to_string(),
                HirClassDef {
                    class_attrs: Vec::new(),
                    exception_type_tag: None,
                    name: "A".to_string(),
                    bases: vec![],
                    mro: vec!["A".to_string()],
                    attrs: vec![("x".to_string(), Ty::Int)],
                    methods: vec![
                        ("__init__".to_string(), "A.__init__".to_string()),
                        ("greet".to_string(), "A.greet".to_string()),
                    ],
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
            ),
            (
                "B".to_string(),
                HirClassDef {
                    class_attrs: Vec::new(),
                    exception_type_tag: None,
                    name: "B".to_string(),
                    bases: vec!["A".to_string()],
                    mro: vec!["B".to_string(), "A".to_string()],
                    attrs: Vec::new(),
                    methods: vec![
                        ("__init__".to_string(), "B.__init__".to_string()),
                        ("greet".to_string(), "B.greet".to_string()),
                    ],
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
            ),
        ],
    }
}

#[test]
fn super_init_lowers_to_direct_call_to_base_init() {
    let hir = super_module();
    let mir = build(&hir);
    let init = mir.items.iter().find_map(|item| match item {
        MirItem::Function { name, body, .. } if name == "B.__init__" => body.first(),
        _ => None,
    });
    assert_eq!(
        init,
        Some(&MirStmt::ExprStmt(MirExpr::Call {
            callee: "A.__init__".to_string(),
            args: vec![MirExpr::Name {
                name: "self".to_string(),
                ty: Ty::Instance(Box::new("B".to_string())),
            }],
            ty: Ty::None,
        }))
    );
}

#[test]
fn super_method_lowers_to_direct_call_to_base_method() {
    let hir = super_module();
    let mir = build(&hir);
    let greet = mir.items.iter().find_map(|item| match item {
        MirItem::Function { name, body, .. } if name == "B.greet" => body.first(),
        _ => None,
    });
    assert_eq!(
        greet,
        Some(&MirStmt::Return(Some(MirExpr::Call {
            callee: "A.greet".to_string(),
            args: vec![MirExpr::Name {
                name: "self".to_string(),
                ty: Ty::Instance(Box::new("B".to_string())),
            }],
            ty: Ty::Int,
        })))
    );
}

#[test]
#[should_panic(expected = "is not a property or class attribute on any class after `B` in its MRO")]
fn super_attr_get_naming_an_instance_attr_panics_with_an_internal_error() {
    // #587: `super().x` where `x` is an instance attribute is rejected
    // by `pycc_types::class::resolve_super_attr_get` with `T0047`, so
    // this HIR shape cannot reach `pycc_mir` through the real pipeline.
    // Constructing it directly exercises the arm's panic-on-inconsistency
    // guard, which replaced the slot-read lowering this test previously
    // asserted (`super_attr_lowers_to_attr_get_with_self_base`).
    let self_a = Ty::Instance(Box::new("A".to_string()));
    let self_b = Ty::Instance(Box::new("B".to_string()));
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "A.__init__".to_string(),
                params: vec![("self".to_string(), self_a)],
                return_ty: Ty::None,
                body: vec![HirStmt::AttrSet {
                    base: HirExpr::Name("self".to_string()),
                    attr: "x".to_string(),
                    value: HirExpr::IntLiteral(42),
                }],
            },
            HirItem::Function {
                name: "B.get_x".to_string(),
                params: vec![("self".to_string(), self_b)],
                return_ty: Ty::Int,
                body: vec![HirStmt::Return(Some(HirExpr::AttrGet {
                    base: Box::new(HirExpr::Super),
                    attr: "x".to_string(),
                }))],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: vec![
            (
                "A".to_string(),
                HirClassDef {
                    class_attrs: Vec::new(),
                    exception_type_tag: None,
                    name: "A".to_string(),
                    bases: vec![],
                    mro: vec!["A".to_string()],
                    attrs: vec![("x".to_string(), Ty::Int)],
                    methods: vec![("__init__".to_string(), "A.__init__".to_string())],
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
            ),
            (
                "B".to_string(),
                HirClassDef {
                    class_attrs: Vec::new(),
                    exception_type_tag: None,
                    name: "B".to_string(),
                    bases: vec!["A".to_string()],
                    mro: vec!["B".to_string(), "A".to_string()],
                    attrs: Vec::new(),
                    methods: vec![("get_x".to_string(), "B.get_x".to_string())],
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
            ),
        ],
    };
    build(&hir);
}

#[test]
fn super_property_lowers_to_call_to_base_getter() {
    use pycc_hir::PropertyDef;
    let self_a = Ty::Instance(Box::new("A".to_string()));
    let self_b = Ty::Instance(Box::new("B".to_string()));
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "A.__init__".to_string(),
                params: vec![("self".to_string(), self_a.clone())],
                return_ty: Ty::None,
                body: vec![
                    HirStmt::AttrSet {
                        base: HirExpr::Name("self".to_string()),
                        attr: "_val".to_string(),
                        value: HirExpr::IntLiteral(0),
                    },
                    HirStmt::Return(None),
                ],
            },
            HirItem::Function {
                name: "A.val".to_string(),
                params: vec![("self".to_string(), self_a)],
                return_ty: Ty::Int,
                body: vec![HirStmt::Return(Some(HirExpr::AttrGet {
                    base: Box::new(HirExpr::Name("self".to_string())),
                    attr: "_val".to_string(),
                }))],
            },
            HirItem::Function {
                name: "B.get_val".to_string(),
                params: vec![("self".to_string(), self_b)],
                return_ty: Ty::Int,
                body: vec![HirStmt::Return(Some(HirExpr::AttrGet {
                    base: Box::new(HirExpr::Super),
                    attr: "val".to_string(),
                }))],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: vec![
            (
                "A".to_string(),
                HirClassDef {
                    class_attrs: Vec::new(),
                    exception_type_tag: None,
                    name: "A".to_string(),
                    bases: vec![],
                    mro: vec!["A".to_string()],
                    attrs: vec![("_val".to_string(), Ty::Int)],
                    methods: vec![("__init__".to_string(), "A.__init__".to_string())],
                    type_param: None,
                    properties: vec![PropertyDef {
                        name: "val".to_string(),
                        getter: "A.val".to_string(),
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
            ),
            (
                "B".to_string(),
                HirClassDef {
                    class_attrs: Vec::new(),
                    exception_type_tag: None,
                    name: "B".to_string(),
                    bases: vec!["A".to_string()],
                    mro: vec!["B".to_string(), "A".to_string()],
                    attrs: Vec::new(),
                    methods: vec![("get_val".to_string(), "B.get_val".to_string())],
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
            ),
        ],
    };
    let mir = build(&hir);
    let get_val = mir.items.iter().find_map(|item| match item {
        MirItem::Function { name, body, .. } if name == "B.get_val" => body.first(),
        _ => None,
    });
    assert_eq!(
        get_val,
        Some(&MirStmt::Return(Some(MirExpr::Call {
            callee: "A.val".to_string(),
            args: vec![MirExpr::Name {
                name: "self".to_string(),
                ty: Ty::Instance(Box::new("B".to_string())),
            }],
            ty: Ty::Int,
        })))
    );
}

// -- #915: `super().CLASS_CONST` folds a base class's class attribute ------

/// Builds a class def with no members other than the given name, MRO,
/// bases and class attributes -- enough for the `super()` `AttrGet` arm.
fn class_attr_class(
    name: &str,
    bases: Vec<String>,
    mro: Vec<String>,
    class_attrs: Vec<(String, Ty, pycc_hir::ClassAttrValue)>,
    methods: Vec<(String, String)>,
) -> HirClassDef {
    HirClassDef {
        class_attrs,
        exception_type_tag: None,
        name: name.to_string(),
        bases,
        mro,
        attrs: Vec::new(),
        methods,
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
    }
}

/// `class B(A)` (or a diamond, when `extra` adds classes) whose `B.read`
/// returns `super().X`.
fn super_class_attr_module(
    class_defs: Vec<(String, HirClassDef)>,
    reader_class: &str,
    attr: &str,
) -> HirModule {
    HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: format!("{reader_class}.read"),
            params: vec![(
                "self".to_string(),
                Ty::Instance(Box::new(reader_class.to_string())),
            )],
            return_ty: Ty::Int,
            body: vec![HirStmt::Return(Some(HirExpr::AttrGet {
                base: Box::new(HirExpr::Super),
                attr: attr.to_string(),
            }))],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs,
    }
}

fn read_body(mir: &MirModule, name: &str) -> Option<MirStmt> {
    mir.items.iter().find_map(|item| match item {
        MirItem::Function { name: n, body, .. } if n == name => body.first().cloned(),
        _ => None,
    })
}

#[test]
fn super_class_attr_folds_to_the_base_literal() {
    // #915: `super().X` where `X` is a base class's class attribute folds
    // to the literal, exactly as `A.X` does -- no receiver is emitted.
    use pycc_hir::ClassAttrValue;
    let mir = build(&super_class_attr_module(
        vec![
            (
                "A".to_string(),
                class_attr_class(
                    "A",
                    vec![],
                    vec!["A".to_string()],
                    vec![("X".to_string(), Ty::Int, ClassAttrValue::Int(1))],
                    Vec::new(),
                ),
            ),
            (
                "B".to_string(),
                class_attr_class(
                    "B",
                    vec!["A".to_string()],
                    vec!["B".to_string(), "A".to_string()],
                    Vec::new(),
                    vec![("read".to_string(), "B.read".to_string())],
                ),
            ),
        ],
        "B",
        "X",
    ));
    assert_eq!(
        read_body(&mir, "B.read"),
        Some(MirStmt::Return(Some(MirExpr::IntLiteral(1))))
    );
}

#[test]
fn super_class_attr_reaches_the_second_base_of_a_diamond() {
    // #915: the fold walks the *current class's* post-current MRO slice.
    // `C` is reachable only through `D`'s linearization `[D, B, C, object]`;
    // walking `B`'s own MRO instead would miss it.
    use pycc_hir::ClassAttrValue;
    let mir = build(&super_class_attr_module(
        vec![
            (
                "B".to_string(),
                class_attr_class("B", vec![], vec!["B".to_string()], Vec::new(), Vec::new()),
            ),
            (
                "C".to_string(),
                class_attr_class(
                    "C",
                    vec![],
                    vec!["C".to_string()],
                    vec![("Y".to_string(), Ty::Int, ClassAttrValue::Int(42))],
                    Vec::new(),
                ),
            ),
            (
                "D".to_string(),
                class_attr_class(
                    "D",
                    vec!["B".to_string(), "C".to_string()],
                    vec!["D".to_string(), "B".to_string(), "C".to_string()],
                    Vec::new(),
                    vec![("read".to_string(), "D.read".to_string())],
                ),
            ),
        ],
        "D",
        "Y",
    ));
    assert_eq!(
        read_body(&mir, "D.read"),
        Some(MirStmt::Return(Some(MirExpr::IntLiteral(42))))
    );
}

#[test]
fn super_class_attr_on_an_earlier_mro_entry_outranks_a_later_property() {
    // #915: the fold walks the slice one class at a time, checking every
    // class-level member kind per class. `B` (earlier in `D`'s MRO) carries
    // the class attribute and `C` (later) a `@property` of the same name, so
    // the literal wins -- scanning all properties first would emit a call to
    // `C.X` instead.
    use pycc_hir::{ClassAttrValue, PropertyDef};
    let mut c = class_attr_class("C", vec![], vec!["C".to_string()], Vec::new(), Vec::new());
    c.properties = vec![PropertyDef {
        name: "X".to_string(),
        getter: "C.X".to_string(),
        setter: None,
    }];
    let mir = build(&super_class_attr_module(
        vec![
            (
                "B".to_string(),
                class_attr_class(
                    "B",
                    vec![],
                    vec!["B".to_string()],
                    vec![("X".to_string(), Ty::Int, ClassAttrValue::Int(1))],
                    Vec::new(),
                ),
            ),
            ("C".to_string(), c),
            (
                "D".to_string(),
                class_attr_class(
                    "D",
                    vec!["B".to_string(), "C".to_string()],
                    vec!["D".to_string(), "B".to_string(), "C".to_string()],
                    Vec::new(),
                    vec![("read".to_string(), "D.read".to_string())],
                ),
            ),
        ],
        "D",
        "X",
    ));
    assert_eq!(
        read_body(&mir, "D.read"),
        Some(MirStmt::Return(Some(MirExpr::IntLiteral(1))))
    );
}
