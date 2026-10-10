//! Inherited attributes, methods and `__init__` through the MRO.

use super::*;

// -- #432: MRO-aware type checking --------------------------------------

/// Builds an `Animal` base class with a `name` attribute and a `speak`
/// method, plus a `Dog` derived class with its own `speak` override,
/// plus `extra_items` for each test's own exercise.
fn inheritance_module(extra_items: Vec<HirItem>) -> HirModule {
    let animal_ty = Ty::Instance(Box::new("Animal".to_string()));
    let dog_ty = Ty::Instance(Box::new("Dog".to_string()));
    let animal_init = HirItem::Function {
        name: "Animal.__init__".to_string(),
        params: vec![
            ("self".to_string(), animal_ty.clone()),
            ("name".to_string(), Ty::Str),
        ],
        return_ty: Ty::None,
        body: vec![
            HirStmt::AttrSet {
                base: HirExpr::Name("self".to_string()),
                attr: "name".to_string(),
                value: HirExpr::Name("name".to_string()),
            },
            HirStmt::Return(None),
        ],
    };
    let animal_speak = HirItem::Function {
        name: "Animal.speak".to_string(),
        params: vec![("self".to_string(), animal_ty.clone())],
        return_ty: Ty::Str,
        body: vec![HirStmt::Return(Some(HirExpr::StringLiteral(
            "...".to_string(),
        )))],
    };
    let dog_init = HirItem::Function {
        name: "Dog.__init__".to_string(),
        params: vec![
            ("self".to_string(), dog_ty.clone()),
            ("name".to_string(), Ty::Str),
        ],
        return_ty: Ty::None,
        body: vec![
            HirStmt::AttrSet {
                base: HirExpr::Name("self".to_string()),
                attr: "name".to_string(),
                value: HirExpr::Name("name".to_string()),
            },
            HirStmt::Return(None),
        ],
    };
    let dog_speak = HirItem::Function {
        name: "Dog.speak".to_string(),
        params: vec![("self".to_string(), dog_ty.clone())],
        return_ty: Ty::Str,
        body: vec![HirStmt::Return(Some(HirExpr::StringLiteral(
            "Woof".to_string(),
        )))],
    };
    let mut items = vec![animal_init, animal_speak, dog_init, dog_speak];
    items.extend(extra_items);
    HirModule {
        seeded_builtin_exception_classes: false,
        items,
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: vec![
            (
                "Animal".to_string(),
                HirClassDef {
                    class_attrs: Vec::new(),
                    exception_type_tag: None,
                    name: "Animal".to_string(),
                    bases: Vec::new(),
                    mro: vec!["Animal".to_string()],
                    attrs: vec![("name".to_string(), Ty::Str)],
                    methods: vec![
                        ("__init__".to_string(), "Animal.__init__".to_string()),
                        ("speak".to_string(), "Animal.speak".to_string()),
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
                "Dog".to_string(),
                HirClassDef {
                    class_attrs: Vec::new(),
                    exception_type_tag: None,
                    name: "Dog".to_string(),
                    bases: vec!["Animal".to_string()],
                    mro: vec!["Dog".to_string(), "Animal".to_string()],
                    attrs: vec![("name".to_string(), Ty::Str)],
                    methods: vec![
                        ("__init__".to_string(), "Dog.__init__".to_string()),
                        ("speak".to_string(), "Dog.speak".to_string()),
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
fn derived_class_inherits_base_attribute_through_mro() {
    // `d.name` on a `Dog` instance resolves `name` through the MRO --
    // `Dog` declares it, so it's found on the first MRO entry.
    let hir = inheritance_module(vec![
        top_level(HirStmt::Assign {
            target: "d".to_string(),
            value: HirExpr::Call {
                callee: "Dog".to_string(),
                args: vec![HirExpr::StringLiteral("Rex".to_string())],
            },
        }),
        top_level(HirStmt::ExprStmt(HirExpr::Call {
            callee: "print".to_string(),
            args: vec![HirExpr::AttrGet {
                base: Box::new(HirExpr::Name("d".to_string())),
                attr: "name".to_string(),
            }],
        })),
    ]);
    check(&hir).expect("inherited attribute read should type-check");
}

#[test]
fn derived_class_method_call_resolves_through_mro() {
    // `d.speak()` on a `Dog` instance resolves to `Dog.speak` (the
    // override), not `Animal.speak`.
    let hir = inheritance_module(vec![
        top_level(HirStmt::Assign {
            target: "d".to_string(),
            value: HirExpr::Call {
                callee: "Dog".to_string(),
                args: vec![HirExpr::StringLiteral("Rex".to_string())],
            },
        }),
        top_level(HirStmt::ExprStmt(HirExpr::MethodCall {
            base: Box::new(HirExpr::Name("d".to_string())),
            method: "speak".to_string(),
            args: vec![],
        })),
    ]);
    check(&hir).expect("method call on derived class should type-check");
}

#[test]
fn derived_class_instantiation_with_inherited_init_type_checks() {
    // A `Dog` instance is created with `(name: str)` -- the `__init__`
    // is `Dog.__init__`, which takes `(self, name: str)`.
    let hir = inheritance_module(vec![top_level(HirStmt::Assign {
        target: "d".to_string(),
        value: HirExpr::Call {
            callee: "Dog".to_string(),
            args: vec![HirExpr::StringLiteral("Rex".to_string())],
        },
    })]);
    check(&hir).expect("derived class instantiation should type-check");
}

#[test]
fn derived_class_instantiation_with_wrong_arg_type_is_rejected() {
    let hir = inheritance_module(vec![top_level(HirStmt::Assign {
        target: "d".to_string(),
        value: HirExpr::Call {
            callee: "Dog".to_string(),
            args: vec![HirExpr::IntLiteral(42)],
        },
    })]);
    assert_eq!(check(&hir).unwrap_err().code, "T0021");
}

/// Builds a `Base` class with `__init__` and a `Derived` class with no
/// `__init__` of its own (inheriting `Base.__init__`).
fn inherited_init_module(extra_items: Vec<HirItem>) -> HirModule {
    let base_ty = Ty::Instance(Box::new("Base".to_string()));
    let base_init = HirItem::Function {
        name: "Base.__init__".to_string(),
        params: vec![
            ("self".to_string(), base_ty.clone()),
            ("x".to_string(), Ty::Int),
        ],
        return_ty: Ty::None,
        body: vec![
            HirStmt::AttrSet {
                base: HirExpr::Name("self".to_string()),
                attr: "x".to_string(),
                value: HirExpr::Name("x".to_string()),
            },
            HirStmt::Return(None),
        ],
    };
    let mut items = vec![base_init];
    items.extend(extra_items);
    HirModule {
        seeded_builtin_exception_classes: false,
        items,
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: vec![
            (
                "Base".to_string(),
                HirClassDef {
                    class_attrs: Vec::new(),
                    exception_type_tag: None,
                    name: "Base".to_string(),
                    bases: Vec::new(),
                    mro: vec!["Base".to_string()],
                    attrs: vec![("x".to_string(), Ty::Int)],
                    methods: vec![("__init__".to_string(), "Base.__init__".to_string())],
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
                "Derived".to_string(),
                HirClassDef {
                    class_attrs: Vec::new(),
                    exception_type_tag: None,
                    name: "Derived".to_string(),
                    bases: vec!["Base".to_string()],
                    mro: vec!["Derived".to_string(), "Base".to_string()],
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
fn derived_class_without_init_inherits_base_init_for_instantiation() {
    // `Derived(42)` should resolve to `Base.__init__` via the MRO and
    // type-check against its `(self, x: int)` parameter list.
    let hir = inherited_init_module(vec![
        top_level(HirStmt::Assign {
            target: "d".to_string(),
            value: HirExpr::Call {
                callee: "Derived".to_string(),
                args: vec![HirExpr::IntLiteral(42)],
            },
        }),
        top_level(HirStmt::ExprStmt(HirExpr::Call {
            callee: "print".to_string(),
            args: vec![HirExpr::AttrGet {
                base: Box::new(HirExpr::Name("d".to_string())),
                attr: "x".to_string(),
            }],
        })),
    ]);
    check(&hir).expect("inherited __init__ instantiation should type-check");
}

#[test]
fn derived_class_without_init_instantiation_wrong_arg_type_is_rejected() {
    let hir = inherited_init_module(vec![top_level(HirStmt::Assign {
        target: "d".to_string(),
        value: HirExpr::Call {
            callee: "Derived".to_string(),
            args: vec![HirExpr::StringLiteral("wrong".to_string())],
        },
    })]);
    assert_eq!(check(&hir).unwrap_err().code, "T0021");
}

#[test]
fn inherited_attribute_read_resolves_through_check_and_resolve() {
    // Full pipeline: `check_and_resolve` → MIR build, exercising the
    // MIR lowering's MRO-aware attribute resolution.
    let hir = inherited_init_module(vec![
        top_level(HirStmt::Assign {
            target: "d".to_string(),
            value: HirExpr::Call {
                callee: "Derived".to_string(),
                args: vec![HirExpr::IntLiteral(42)],
            },
        }),
        top_level(HirStmt::ExprStmt(HirExpr::Call {
            callee: "print".to_string(),
            args: vec![HirExpr::AttrGet {
                base: Box::new(HirExpr::Name("d".to_string())),
                attr: "x".to_string(),
            }],
        })),
    ]);
    let resolved = check_and_resolve(&hir).expect("check_and_resolve should succeed");
    let _mir = pycc_mir::build(&resolved);
}

#[test]
fn inherited_method_call_resolves_through_check_and_resolve() {
    // Full pipeline for a method call that resolves to a base class
    // method via the MRO.
    let base_ty = Ty::Instance(Box::new("Base".to_string()));
    let base_init = HirItem::Function {
        name: "Base.__init__".to_string(),
        params: vec![("self".to_string(), base_ty.clone())],
        return_ty: Ty::None,
        body: vec![HirStmt::Return(None)],
    };
    let base_greet = HirItem::Function {
        name: "Base.greet".to_string(),
        params: vec![("self".to_string(), base_ty.clone())],
        return_ty: Ty::Str,
        body: vec![HirStmt::Return(Some(HirExpr::StringLiteral(
            "hi".to_string(),
        )))],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            base_init,
            base_greet,
            top_level(HirStmt::Assign {
                target: "d".to_string(),
                value: HirExpr::Call {
                    callee: "Derived".to_string(),
                    args: vec![],
                },
            }),
            top_level(HirStmt::ExprStmt(HirExpr::MethodCall {
                base: Box::new(HirExpr::Name("d".to_string())),
                method: "greet".to_string(),
                args: vec![],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: vec![
            (
                "Base".to_string(),
                HirClassDef {
                    class_attrs: Vec::new(),
                    exception_type_tag: None,
                    name: "Base".to_string(),
                    bases: Vec::new(),
                    mro: vec!["Base".to_string()],
                    attrs: Vec::new(),
                    methods: vec![
                        ("__init__".to_string(), "Base.__init__".to_string()),
                        ("greet".to_string(), "Base.greet".to_string()),
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
                "Derived".to_string(),
                HirClassDef {
                    class_attrs: Vec::new(),
                    exception_type_tag: None,
                    name: "Derived".to_string(),
                    bases: vec!["Base".to_string()],
                    mro: vec!["Derived".to_string(), "Base".to_string()],
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
                    is_protocol: false,
                    runtime_checkable: false,
                    protocol_members: Vec::new(),
                    abstract_methods: Vec::new(),
                    is_abstract: false,
                },
            ),
        ],
    };
    let resolved = check_and_resolve(&hir).expect("check_and_resolve should succeed");
    let _mir = pycc_mir::build(&resolved);
}

// PEP 572 (#774): `pycc_mir::expr::pre_bind_named_expr_targets` is a
// function private to `pycc_mir` (`pub(super)`), reachable from outside
// that crate only indirectly through `pycc_mir::build`. Its `BinOp`/
// `Compare`/`UnaryOp`/`FString` arms (a walrus nested inside one of
// those, at any depth within an `ExprStmt`) are already exercised
// through `pycc_mir`'s own hand-built-HIR test suite (see that crate's
// `walrus_in_an_if_test_binds_the_name_for_the_body` and neighbors) and
// through `pycc_types::tests::check_source`'s
// `a_walrus_nested_inside_every_remaining_container_shape_binds_every_name_at_module_scope`,
// but every one of those calls a *different* crate's own compiled copy
// of `pycc_mir`: this crate's own `[dev-dependencies]` link on
// `pycc_mir` is measured as its own separate coverage instantiation,
// independent of `pycc_mir`'s own `cfg(test)` binary. Only this
// module's handful of `pycc_mir::build(&resolved)` calls (see the
// property/inheritance tests above) exercise *this* crate's copy of the
// HIR→MIR pipeline at all, and none of them contain a walrus, so this
// test closes that gap directly.
#[test]
fn a_walrus_nested_inside_binop_compare_unaryop_and_fstring_binds_every_name_via_mir_build() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            top_level(HirStmt::ExprStmt(HirExpr::BinOp {
                op: BinOpKind::Add,
                left: Box::new(HirExpr::NamedExpr {
                    name: "bo_l".to_string(),
                    value: Box::new(HirExpr::IntLiteral(1)),
                }),
                right: Box::new(HirExpr::NamedExpr {
                    name: "bo_r".to_string(),
                    value: Box::new(HirExpr::IntLiteral(2)),
                }),
            })),
            top_level(HirStmt::ExprStmt(HirExpr::Compare {
                op: pycc_hir::CmpOpKind::Lt,
                left: Box::new(HirExpr::NamedExpr {
                    name: "cmp_l".to_string(),
                    value: Box::new(HirExpr::IntLiteral(1)),
                }),
                right: Box::new(HirExpr::NamedExpr {
                    name: "cmp_r".to_string(),
                    value: Box::new(HirExpr::IntLiteral(2)),
                }),
            })),
            top_level(HirStmt::ExprStmt(HirExpr::UnaryOp {
                op: pycc_hir::UnaryOpKind::USub,
                operand: Box::new(HirExpr::NamedExpr {
                    name: "uop".to_string(),
                    value: Box::new(HirExpr::IntLiteral(3)),
                }),
            })),
            top_level(HirStmt::ExprStmt(HirExpr::FString(vec![
                pycc_hir::FStringPart::Literal("x=".to_string()),
                pycc_hir::FStringPart::Interpolation(Box::new(HirExpr::NamedExpr {
                    name: "fsp".to_string(),
                    value: Box::new(HirExpr::IntLiteral(4)),
                })),
            ]))),
            top_level(HirStmt::Assign {
                target: "readback".to_string(),
                value: HirExpr::Name("fsp".to_string()),
            }),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: vec![],
    };
    let resolved = check_and_resolve(&hir).expect("check_and_resolve should succeed");
    let mir = pycc_mir::build(&resolved);
    // Proof every name was actually bound (not just that lowering
    // didn't panic): the trailing `Assign` above reads `fsp` back by
    // name, which would panic looking it up in `scopes` if
    // `pre_bind_named_expr_targets` had skipped the `FString`
    // interpolation arm.
    assert_eq!(
        mir.items[4],
        pycc_mir::MirItem::TopLevelStmt(pycc_mir::MirStmt::Assign {
            target: "readback".to_string(),
            value: pycc_mir::MirExpr::Name {
                name: "fsp".to_string(),
                ty: Ty::Int,
            },
        })
    );
}
