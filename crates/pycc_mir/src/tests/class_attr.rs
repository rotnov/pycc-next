//! Class instantiation, attribute access, and method-call lowering.
//!
//! Covers `Instantiate`, attribute slot indices for reads and writes,
//! method calls with `self` prepended, and the internal-error panics for
//! undeclared members, non-instance bases, read-only properties, and a
//! generic-class instantiation that should never reach MIR. The enum
//! member test belongs here because it is an `AttrGet` lowering path.

use crate::*;
use pycc_hir::{BinOpKind, ClassAttrValue, HirClassDef, HirExpr, HirItem, HirModule, HirStmt, Ty};

// -- D-154 (Part 1 of #375): class instantiation, attribute access,
// method calls --------------------------------------------------------

/// A minimal `Point` class module: `__init__(self, x: int, y: int)`
/// sets both attributes from its own parameters; `bump(self) -> None`
/// reads and mutates `self.x`. Mirrors `pycc_types::class::tests`'s own
/// `point_module` fixture (same shape, this crate's own `HirModule`
/// literal convention).
fn point_module(extra_items: Vec<HirItem>) -> HirModule {
    let self_ty = Ty::Instance(Box::new("Point".to_string()));
    let init = HirItem::Function {
        name: "Point.__init__".to_string(),
        params: vec![
            ("self".to_string(), self_ty.clone()),
            ("x".to_string(), Ty::Int),
            ("y".to_string(), Ty::Int),
        ],
        return_ty: Ty::None,
        body: vec![
            HirStmt::AttrSet {
                base: HirExpr::Name("self".to_string()),
                attr: "x".to_string(),
                value: HirExpr::Name("x".to_string()),
            },
            HirStmt::AttrSet {
                base: HirExpr::Name("self".to_string()),
                attr: "y".to_string(),
                value: HirExpr::Name("y".to_string()),
            },
            HirStmt::Return(None),
        ],
    };
    let bump = HirItem::Function {
        name: "Point.bump".to_string(),
        params: vec![("self".to_string(), self_ty)],
        return_ty: Ty::None,
        body: vec![
            HirStmt::AttrSet {
                base: HirExpr::Name("self".to_string()),
                attr: "x".to_string(),
                value: HirExpr::BinOp {
                    op: BinOpKind::Add,
                    left: Box::new(HirExpr::AttrGet {
                        base: Box::new(HirExpr::Name("self".to_string())),
                        attr: "x".to_string(),
                    }),
                    right: Box::new(HirExpr::IntLiteral(1)),
                },
            },
            HirStmt::Return(None),
        ],
    };
    let mut items = vec![init, bump];
    items.extend(extra_items);
    HirModule {
        seeded_builtin_exception_classes: false,
        items,
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: vec![(
            "Point".to_string(),
            HirClassDef {
                class_attrs: Vec::new(),
                exception_type_tag: None,
                name: "Point".to_string(),
                bases: Vec::new(),
                mro: vec!["Point".to_string()],
                attrs: vec![("x".to_string(), Ty::Int), ("y".to_string(), Ty::Int)],
                methods: vec![
                    ("__init__".to_string(), "Point.__init__".to_string()),
                    ("bump".to_string(), "Point.bump".to_string()),
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
        )],
    }
}

#[test]
fn instantiation_lowers_to_a_dedicated_mir_node() {
    let hir = point_module(vec![HirItem::TopLevelStmt(HirStmt::Assign {
        target: "p".to_string(),
        value: HirExpr::Call {
            callee: "Point".to_string(),
            args: vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)],
        },
    })]);
    let mir = build(&hir);
    assert_eq!(
        mir.items.last(),
        Some(&MirItem::TopLevelStmt(MirStmt::Assign {
            target: "p".to_string(),
            value: MirExpr::Instantiate(Box::new(InstantiateExpr {
                ctor: "Point.__init__".to_string(),
                class_name: "Point".to_string(),
                slot_names: vec!["x".to_string(), "y".to_string()],
                args: vec![MirExpr::IntLiteral(1), MirExpr::IntLiteral(2)],
                ty: Ty::Instance(Box::new("Point".to_string())),
            })),
        }))
    );
}

#[test]
fn a_method_call_lowers_to_an_ordinary_call_with_self_prepended() {
    let hir = point_module(vec![
        HirItem::TopLevelStmt(HirStmt::Assign {
            target: "p".to_string(),
            value: HirExpr::Call {
                callee: "Point".to_string(),
                args: vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)],
            },
        }),
        HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::MethodCall {
            base: Box::new(HirExpr::Name("p".to_string())),
            method: "bump".to_string(),
            args: vec![],
        })),
    ]);
    let mir = build(&hir);
    assert_eq!(
        mir.items.last(),
        Some(&MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
            callee: "Point.bump".to_string(),
            args: vec![MirExpr::Name {
                name: "p".to_string(),
                ty: Ty::Instance(Box::new("Point".to_string())),
            }],
            ty: Ty::None,
        })))
    );
}

#[test]
fn a_method_call_with_arguments_lowers_each_argument_after_self() {
    // `point_module`'s own `bump` method takes no extra parameters, so
    // `a_method_call_lowers_to_an_ordinary_call_with_self_prepended`
    // above never exercises `MethodCall`'s own per-argument lowering
    // loop -- a standalone minimal `Counter.add(self, n: int) -> None`
    // fixture here does.
    let self_ty = Ty::Instance(Box::new("Counter".to_string()));
    let init = HirItem::Function {
        name: "Counter.__init__".to_string(),
        params: vec![("self".to_string(), self_ty.clone())],
        return_ty: Ty::None,
        body: vec![
            HirStmt::AttrSet {
                base: HirExpr::Name("self".to_string()),
                attr: "n".to_string(),
                value: HirExpr::IntLiteral(0),
            },
            HirStmt::Return(None),
        ],
    };
    let add = HirItem::Function {
        name: "Counter.add".to_string(),
        params: vec![
            ("self".to_string(), self_ty.clone()),
            ("n".to_string(), Ty::Int),
        ],
        return_ty: Ty::None,
        body: vec![HirStmt::Return(None)],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            init,
            add,
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "c".to_string(),
                value: HirExpr::Call {
                    callee: "Counter".to_string(),
                    args: vec![],
                },
            }),
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::MethodCall {
                base: Box::new(HirExpr::Name("c".to_string())),
                method: "add".to_string(),
                args: vec![HirExpr::IntLiteral(5)],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: vec![(
            "Counter".to_string(),
            HirClassDef {
                class_attrs: Vec::new(),
                exception_type_tag: None,
                name: "Counter".to_string(),
                bases: Vec::new(),
                mro: vec!["Counter".to_string()],
                attrs: vec![("n".to_string(), Ty::Int)],
                methods: vec![
                    ("__init__".to_string(), "Counter.__init__".to_string()),
                    ("add".to_string(), "Counter.add".to_string()),
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
        )],
    };
    let mir = build(&hir);
    assert_eq!(
        mir.items.last(),
        Some(&MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
            callee: "Counter.add".to_string(),
            args: vec![
                MirExpr::Name {
                    name: "c".to_string(),
                    ty: self_ty,
                },
                MirExpr::IntLiteral(5),
            ],
            ty: Ty::None,
        })))
    );
}

#[test]
fn an_attribute_read_lowers_to_a_slot_index() {
    let hir = point_module(vec![
        HirItem::TopLevelStmt(HirStmt::Assign {
            target: "p".to_string(),
            value: HirExpr::Call {
                callee: "Point".to_string(),
                args: vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)],
            },
        }),
        HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
            callee: "print".to_string(),
            args: vec![HirExpr::AttrGet {
                base: Box::new(HirExpr::Name("p".to_string())),
                attr: "y".to_string(),
            }],
        })),
    ]);
    let mir = build(&hir);
    assert_eq!(
        mir.items.last(),
        Some(&MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
            callee: "print".to_string(),
            args: vec![MirExpr::AttrGet {
                base: Box::new(MirExpr::Name {
                    name: "p".to_string(),
                    ty: Ty::Instance(Box::new("Point".to_string())),
                }),
                slot: 1,
                ty: Ty::Int,
            }],
            ty: Ty::None,
        })))
    );
}

#[test]
fn an_attribute_write_lowers_to_a_slot_index() {
    let hir = point_module(vec![
        HirItem::TopLevelStmt(HirStmt::Assign {
            target: "p".to_string(),
            value: HirExpr::Call {
                callee: "Point".to_string(),
                args: vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)],
            },
        }),
        HirItem::TopLevelStmt(HirStmt::AttrSet {
            base: HirExpr::Name("p".to_string()),
            attr: "x".to_string(),
            value: HirExpr::IntLiteral(9),
        }),
    ]);
    let mir = build(&hir);
    assert_eq!(
        mir.items.last(),
        Some(&MirItem::TopLevelStmt(MirStmt::AttrSet {
            base: MirExpr::Name {
                name: "p".to_string(),
                ty: Ty::Instance(Box::new("Point".to_string())),
            },
            slot: 0,
            value: MirExpr::IntLiteral(9),
        }))
    );
}

#[test]
fn the_init_and_bump_methods_themselves_lower_with_self_typed_as_the_instance() {
    // Exercises `__init__`'s own body -- two `HirStmt::AttrSet`s whose
    // RHS is a bare parameter reference -- and `bump`'s own body -- an
    // `HirExpr::AttrGet` nested inside a `BinOp` -- end to end through
    // `build`, not just the module-scope exercises above. Direct value
    // comparisons against `mir.items[0]`/`[1]`, not a
    // `let PATTERN = .. else { panic!(..) }` destructure -- this file's
    // own established coverage-gate convention (see e.g.
    // `class_instantiation_...` tests elsewhere): a hand-written panic
    // arm never taken on the happy path is a permanently uncovered
    // region under D-014's 100%-region gate.
    let hir = point_module(vec![]);
    let mir = build(&hir);
    let self_ty = Ty::Instance(Box::new("Point".to_string()));
    assert_eq!(
        mir.items[0],
        MirItem::Function {
            name: "Point.__init__".to_string(),
            params: vec![
                ("self".to_string(), self_ty.clone()),
                ("x".to_string(), Ty::Int),
                ("y".to_string(), Ty::Int),
            ],
            return_ty: Ty::None,
            body: vec![
                MirStmt::AttrSet {
                    base: MirExpr::Name {
                        name: "self".to_string(),
                        ty: self_ty.clone(),
                    },
                    slot: 0,
                    value: MirExpr::Name {
                        name: "x".to_string(),
                        ty: Ty::Int,
                    },
                },
                MirStmt::AttrSet {
                    base: MirExpr::Name {
                        name: "self".to_string(),
                        ty: self_ty.clone(),
                    },
                    slot: 1,
                    value: MirExpr::Name {
                        name: "y".to_string(),
                        ty: Ty::Int,
                    },
                },
                MirStmt::Return(None),
            ],
        }
    );
    assert_eq!(
        mir.items[1],
        MirItem::Function {
            name: "Point.bump".to_string(),
            params: vec![("self".to_string(), self_ty.clone())],
            return_ty: Ty::None,
            body: vec![
                MirStmt::AttrSet {
                    base: MirExpr::Name {
                        name: "self".to_string(),
                        ty: self_ty.clone(),
                    },
                    slot: 0,
                    value: MirExpr::BinOp {
                        op: BinOpKind::Add,
                        left: Box::new(MirExpr::AttrGet {
                            base: Box::new(MirExpr::Name {
                                name: "self".to_string(),
                                ty: self_ty.clone(),
                            }),
                            slot: 0,
                            ty: Ty::Int,
                        }),
                        right: Box::new(MirExpr::IntLiteral(1)),
                        ty: Ty::Int,
                    },
                },
                MirStmt::Return(None),
            ],
        }
    );
}

#[test]
#[should_panic(expected = "expected an instance- or protocol-typed expression")]
fn attr_get_on_a_non_instance_base_panics_with_an_internal_error() {
    // Bypasses `pycc_types::check` (which would reject this) with a
    // hand-built `HirExpr::AttrGet` over an `int`-typed base, matching
    // this file's own established internal-error-test convention (see
    // e.g. `referencing_an_unbound_name_panics_with_an_internal_error`
    // above).
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::IntLiteral(1),
            }),
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::AttrGet {
                base: Box::new(HirExpr::Name("x".to_string())),
                attr: "y".to_string(),
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let _ = build(&hir);
}

#[test]
#[should_panic(expected = "class `Ghost` has no registered HirClassDef")]
fn attr_get_over_an_instance_typed_parameter_from_an_unregistered_class_panics_with_an_internal_error()
 {
    // `class_def_of`'s *second* panic (as opposed to
    // `attr_get_on_a_non_instance_base_panics_with_an_internal_error`
    // above, which exercises its first): `x`'s own declared parameter
    // type names `Ghost`, but `hir.class_defs` never registers it --
    // parameter binding itself never consults `classes` at all, so
    // this reaches `class_def_of`'s own `classes.get(..)` lookup with a
    // genuinely instance-typed expression pointing at a class this
    // module's own side table has no entry for.
    let ghost_ty = Ty::Instance(Box::new("Ghost".to_string()));
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "f".to_string(),
            params: vec![("x".to_string(), ghost_ty)],
            return_ty: Ty::None,
            body: vec![
                HirStmt::ExprStmt(HirExpr::AttrGet {
                    base: Box::new(HirExpr::Name("x".to_string())),
                    attr: "y".to_string(),
                }),
                HirStmt::Return(None),
            ],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let _ = build(&hir);
}

#[test]
#[should_panic(expected = "attribute `z` not declared on class `Point`")]
fn attr_get_of_an_undeclared_attribute_panics_with_an_internal_error() {
    let hir = point_module(vec![
        HirItem::TopLevelStmt(HirStmt::Assign {
            target: "p".to_string(),
            value: HirExpr::Call {
                callee: "Point".to_string(),
                args: vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)],
            },
        }),
        HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::AttrGet {
            base: Box::new(HirExpr::Name("p".to_string())),
            attr: "z".to_string(),
        })),
    ]);
    let _ = build(&hir);
}

#[test]
#[should_panic(expected = "method `fly` not declared on class `Point`")]
fn method_call_of_an_undeclared_method_panics_with_an_internal_error() {
    let hir = point_module(vec![
        HirItem::TopLevelStmt(HirStmt::Assign {
            target: "p".to_string(),
            value: HirExpr::Call {
                callee: "Point".to_string(),
                args: vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)],
            },
        }),
        HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::MethodCall {
            base: Box::new(HirExpr::Name("p".to_string())),
            method: "fly".to_string(),
            args: vec![],
        })),
    ]);
    let _ = build(&hir);
}

#[test]
#[should_panic(expected = "attribute `z` not declared on class `Point`")]
fn attr_set_of_an_undeclared_attribute_panics_with_an_internal_error() {
    let hir = point_module(vec![
        HirItem::TopLevelStmt(HirStmt::Assign {
            target: "p".to_string(),
            value: HirExpr::Call {
                callee: "Point".to_string(),
                args: vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)],
            },
        }),
        HirItem::TopLevelStmt(HirStmt::AttrSet {
            base: HirExpr::Name("p".to_string()),
            attr: "z".to_string(),
            value: HirExpr::IntLiteral(1),
        }),
    ]);
    let _ = build(&hir);
}

// #377: a read-only `@property` (no setter) should never reach MIR
// lowering -- `pycc_types::check` rejects the assignment with `T0044`
// before MIR runs. This test bypasses the type checker with a hand-built
// HIR to exercise the panic arm, matching this file's own established
// internal-error-test convention (see e.g.
// `attr_set_of_an_undeclared_attribute_panics_with_an_internal_error`
// above).
#[test]
#[should_panic(expected = "pycc_mir: internal error: property `val` on class `Box` has no setter")]
fn attr_set_on_a_read_only_property_panics_with_an_internal_error() {
    use pycc_hir::{HirClassDef, PropertyDef};
    let self_ty = Ty::Instance(Box::new("Box".to_string()));
    let init = HirItem::Function {
        name: "Box.__init__".to_string(),
        params: vec![("self".to_string(), self_ty.clone())],
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
    let getter = HirItem::Function {
        name: "Box.val".to_string(),
        params: vec![("self".to_string(), self_ty.clone())],
        return_ty: Ty::Int,
        body: vec![HirStmt::Return(Some(HirExpr::AttrGet {
            base: Box::new(HirExpr::Name("self".to_string())),
            attr: "_val".to_string(),
        }))],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            init,
            getter,
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "b".to_string(),
                value: HirExpr::Call {
                    callee: "Box".to_string(),
                    args: vec![],
                },
            }),
            HirItem::TopLevelStmt(HirStmt::AttrSet {
                base: HirExpr::Name("b".to_string()),
                attr: "val".to_string(),
                value: HirExpr::IntLiteral(42),
            }),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: vec![(
            "Box".to_string(),
            HirClassDef {
                class_attrs: Vec::new(),
                exception_type_tag: None,
                name: "Box".to_string(),
                bases: Vec::new(),
                mro: vec!["Box".to_string()],
                attrs: vec![("_val".to_string(), Ty::Int)],
                methods: vec![("__init__".to_string(), "Box.__init__".to_string())],
                properties: vec![PropertyDef {
                    name: "val".to_string(),
                    getter: "Box.val".to_string(),
                    setter: None,
                }],
                type_param: None,
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
        )],
    };
    let _ = build(&hir);
}

// PEP 695 (#387): `GenericClassInstantiate` should never reach MIR —
// `pycc_types::monomorphize` rewrites every such expression to an
// ordinary `HirExpr::Call` before MIR lowering runs. This test bypasses
// the type checker with a hand-built HIR to exercise the panic arm,
// matching this file's own established internal-error-test convention.
#[test]
#[should_panic(
    expected = "pycc_mir: internal error: `GenericClassInstantiate` for class `C` reached MIR lowering"
)]
fn generic_class_instantiate_reaching_mir_panics_with_an_internal_error() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::TopLevelStmt(HirStmt::ExprStmt(
            HirExpr::GenericClassInstantiate {
                class: "C".to_string(),
                type_arg: Ty::Int,
                args: vec![HirExpr::IntLiteral(1)],
            },
        ))],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let _ = build(&hir);
}

#[test]
fn enum_member_attr_get_lowers_to_synthetic_global() {
    // #379: `Color.RED` lowers to `MirExpr::Name` reading the
    // synthetic `<Class>.<Member>.enum_member` global.
    let class_def = pycc_hir::HirClassDef {
        class_attrs: Vec::new(),
        exception_type_tag: None,
        name: "Color".to_string(),
        bases: vec![],
        mro: vec!["Color".to_string()],
        attrs: vec![
            ("value".to_string(), Ty::Int),
            ("name".to_string(), Ty::Str),
        ],
        methods: vec![],
        properties: vec![],
        static_methods: vec![],
        class_methods: vec![],
        type_param: None,
        is_enum: false,
        implicit_object_init: false,
        method_defaults: Vec::new(),
        enum_members: vec![
            ("RED".to_string(), pycc_hir::EnumMemberValue::Int(1)),
            ("GREEN".to_string(), pycc_hir::EnumMemberValue::Int(2)),
        ],
        is_dataclass: false,
        dataclass_fields: Vec::new(),
        is_protocol: false,
        runtime_checkable: false,
        protocol_members: Vec::new(),
        abstract_methods: Vec::new(),
        is_abstract: false,
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
            callee: "print".to_string(),
            args: vec![HirExpr::AttrGet {
                base: Box::new(HirExpr::Name("Color".to_string())),
                attr: "RED".to_string(),
            }],
        }))],
        type_aliases: vec![],
        imports: vec![],
        class_defs: vec![("Color".to_string(), class_def)],
    };
    let mir = build(&hir);
    assert_eq!(
        mir.items[0],
        MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
            callee: "print".to_string(),
            args: vec![MirExpr::Name {
                name: "Color.RED.enum_member".to_string(),
                ty: Ty::Instance(Box::new("Color".to_string())),
            }],
            ty: Ty::None,
        }))
    );
}

// -- #911 (Part 1 of #885): class-level attribute folding ------------------
//
// A class attribute is a compile-time constant: every read folds to its
// literal and the base expression is discarded (`pycc_types` restricts the
// base to a bare name, so evaluating it has no observable effect). Both
// interception points are exercised here -- the class-name read, which runs
// *before* `lower_expr` because the base is a class name rather than a value
// binding, and the instance read, which runs after the property walk and
// before the `mro_attrs` slot lookup whose miss panics.

/// A `Limit` class carrying one class attribute of each `ClassAttrValue`
/// variant plus a single instance slot, so the fold and the slot layout can
/// be asserted against each other.
fn limit_module(extra_items: Vec<HirItem>) -> HirModule {
    use pycc_hir::ClassAttrValue;

    let self_ty = Ty::Instance(Box::new("Limit".to_string()));
    let init = HirItem::Function {
        name: "Limit.__init__".to_string(),
        params: vec![("self".to_string(), self_ty)],
        return_ty: Ty::None,
        body: vec![
            HirStmt::AttrSet {
                base: HirExpr::Name("self".to_string()),
                attr: "n".to_string(),
                value: HirExpr::IntLiteral(0),
            },
            HirStmt::Return(None),
        ],
    };
    let mut items = vec![init];
    items.extend(extra_items);
    HirModule {
        seeded_builtin_exception_classes: false,
        items,
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: vec![(
            "Limit".to_string(),
            HirClassDef {
                class_attrs: vec![
                    ("LIMIT".to_string(), Ty::Int, ClassAttrValue::Int(8)),
                    ("SCALE".to_string(), Ty::Float, ClassAttrValue::Float(1.5)),
                    ("DEBUG".to_string(), Ty::Bool, ClassAttrValue::Bool(true)),
                    (
                        "KIND".to_string(),
                        Ty::Str,
                        ClassAttrValue::Str("k".to_string()),
                    ),
                ],
                exception_type_tag: None,
                name: "Limit".to_string(),
                bases: Vec::new(),
                mro: vec!["Limit".to_string()],
                attrs: vec![("n".to_string(), Ty::Int)],
                methods: vec![("__init__".to_string(), "Limit.__init__".to_string())],
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
        )],
    }
}

/// The four class-attribute reads, as top-level assignments, in declaration
/// order -- so one fixture pins every `ClassAttrValue` variant's fold.
fn class_attr_reads(base: HirExpr) -> Vec<HirStmt> {
    ["LIMIT", "SCALE", "DEBUG", "KIND"]
        .into_iter()
        .map(|attr| HirStmt::Assign {
            target: format!("v_{attr}"),
            value: HirExpr::AttrGet {
                base: Box::new(base.clone()),
                attr: attr.to_string(),
            },
        })
        .collect()
}

fn folded_values(items: &[MirItem]) -> Vec<MirExpr> {
    items
        .iter()
        .filter_map(|item| match item {
            MirItem::TopLevelStmt(MirStmt::Assign { target, value })
                if target.starts_with("v_") =>
            {
                Some(value.clone())
            }
            _ => None,
        })
        .collect()
}

fn expected_folds() -> Vec<MirExpr> {
    vec![
        MirExpr::IntLiteral(8),
        MirExpr::FloatLiteral(1.5),
        MirExpr::BoolLiteral(true),
        MirExpr::StringLiteral("k".to_string()),
    ]
}

#[test]
fn a_class_name_class_attribute_read_folds_to_its_constant() {
    let hir = limit_module(
        class_attr_reads(HirExpr::Name("Limit".to_string()))
            .into_iter()
            .map(HirItem::TopLevelStmt)
            .collect(),
    );
    let mir = build(&hir);
    assert_eq!(folded_values(&mir.items), expected_folds());
}

#[test]
fn an_instance_class_attribute_read_folds_to_its_constant() {
    let mut items = vec![HirItem::TopLevelStmt(HirStmt::Assign {
        target: "p".to_string(),
        value: HirExpr::Call {
            callee: "Limit".to_string(),
            args: vec![],
        },
    })];
    items.extend(
        class_attr_reads(HirExpr::Name("p".to_string()))
            .into_iter()
            .map(HirItem::TopLevelStmt),
    );
    let hir = limit_module(items);
    let mir = build(&hir);
    assert_eq!(folded_values(&mir.items), expected_folds());

    // D-154: the fold happens instead of a slot read, so the instance still
    // allocates exactly the one word its single `__init__` slot needs.
    let attr_count = mir.items.iter().find_map(|item| match item {
        MirItem::TopLevelStmt(MirStmt::Assign {
            value: MirExpr::Instantiate(instantiate),
            ..
        }) => Some(instantiate.slot_names.len()),
        _ => None,
    });
    assert_eq!(attr_count, Some(1));
}

/// #960: when a sibling MRO base contributes an instance slot of the same
/// name, the instance read resolves to that slot instead of folding the class
/// attribute. `reject_class_attr_collisions` never compares two independent
/// sibling bases with each other, so this shape is legal, and
/// `pycc_types::class::resolve_attr_get` already answers it CPython's way --
/// this pins `pycc_mir` to the same order.
#[test]
fn a_sibling_bases_instance_slot_wins_over_a_class_attribute() {
    use pycc_hir::ClassAttrValue;

    let slot_ty = Ty::Instance(Box::new("Slotted".to_string()));
    let init = HirItem::Function {
        name: "Slotted.__init__".to_string(),
        params: vec![("self".to_string(), slot_ty)],
        return_ty: Ty::None,
        body: vec![
            HirStmt::AttrSet {
                base: HirExpr::Name("self".to_string()),
                attr: "x".to_string(),
                value: HirExpr::IntLiteral(1),
            },
            HirStmt::Return(None),
        ],
    };
    let base_def = |name: &str,
                    attrs: Vec<(String, Ty)>,
                    class_attrs: Vec<(String, Ty, ClassAttrValue)>,
                    methods: Vec<(String, String)>,
                    mro: Vec<String>| {
        (
            name.to_string(),
            HirClassDef {
                class_attrs,
                exception_type_tag: None,
                name: name.to_string(),
                bases: Vec::new(),
                mro,
                attrs,
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
            },
        )
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            init,
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "d".to_string(),
                value: HirExpr::Call {
                    callee: "Derived".to_string(),
                    args: vec![],
                },
            }),
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "v".to_string(),
                value: HirExpr::AttrGet {
                    base: Box::new(HirExpr::Name("d".to_string())),
                    attr: "x".to_string(),
                },
            }),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: vec![
            base_def(
                "Slotted",
                vec![("x".to_string(), Ty::Int)],
                Vec::new(),
                vec![("__init__".to_string(), "Slotted.__init__".to_string())],
                vec!["Slotted".to_string()],
            ),
            base_def(
                "Constant",
                Vec::new(),
                vec![("x".to_string(), Ty::Int, ClassAttrValue::Int(2))],
                Vec::new(),
                vec!["Constant".to_string()],
            ),
            base_def(
                "Derived",
                Vec::new(),
                Vec::new(),
                vec![("__init__".to_string(), "Slotted.__init__".to_string())],
                vec![
                    "Derived".to_string(),
                    "Slotted".to_string(),
                    "Constant".to_string(),
                ],
            ),
        ],
    };
    let mir = build(&hir);
    let read = mir.items.iter().find_map(|item| match item {
        MirItem::TopLevelStmt(MirStmt::Assign { target, value }) if target == "v" => {
            Some(value.clone())
        }
        _ => None,
    });
    match read {
        Some(MirExpr::AttrGet { slot, ty, .. }) => {
            assert_eq!(slot, 0, "`Slotted.x` is the derived class's only slot");
            assert_eq!(ty, Ty::Int);
        }
        other => panic!(
            "the sibling base's instance slot must win over `Constant.x`'s folded \
             constant, but the read lowered to {other:?}"
        ),
    }
}

/// #974: the class-name-qualified read's own internal-error panic, and the
/// shadow `break` that reaches it.
///
/// No compiling program can get here -- `pycc_types`' class-name arm runs the
/// identical MRO walk over the identical shared
/// `declares_name_outside_class_attrs` predicate and reports `T0044` for this
/// exact shape (see
/// `tests/issue_974_inherited_class_attr_by_class_name.rs`'s four shadow
/// tests), so `pycc_mir` only ever sees the read when the checker accepted
/// it. The panic is the stated invariant that the two walks agree; this
/// hand-built module bypasses the checker to prove it fires rather than
/// silently folding `A`'s `2`, which is precisely the mis-compile the shared
/// predicate exists to prevent.
#[test]
#[should_panic(expected = "attribute `x` is not a class attribute reachable through class `B`")]
fn a_shadowed_class_name_read_panics_with_an_internal_error() {
    let base = HirClassDef {
        class_attrs: vec![("x".to_string(), Ty::Int, ClassAttrValue::Int(2))],
        exception_type_tag: None,
        name: "A".to_string(),
        bases: Vec::new(),
        mro: vec!["A".to_string()],
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
    };
    let derived = HirClassDef {
        class_attrs: Vec::new(),
        exception_type_tag: None,
        name: "B".to_string(),
        bases: vec!["A".to_string()],
        mro: vec!["B".to_string(), "A".to_string()],
        attrs: Vec::new(),
        methods: vec![("x".to_string(), "B.x".to_string())],
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
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::AttrGet {
            base: Box::new(HirExpr::Name("B".to_string())),
            attr: "x".to_string(),
        }))],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: vec![("A".to_string(), base), ("B".to_string(), derived)],
    };
    let _ = build(&hir);
}

/// #974: the class-name-qualified read resolves an attribute declared on a
/// *base* class, folding it to the base's constant.
///
/// This is the MIR half of the issue's fix, and the half that the sibling
/// `#[should_panic]` test above cannot reach: here the MRO walk must *carry
/// on past* the derived class -- which declares the name in no namespace at
/// all -- and find `A`'s declaration on the next entry. The class-name arm
/// deliberately runs its own walk rather than the instance arm's, so this
/// fixture gives `B` no instance slot and no `__init__` to prove the fold
/// needs neither.
#[test]
fn an_inherited_class_attribute_read_through_the_derived_class_name_folds() {
    let base = HirClassDef {
        class_attrs: vec![("x".to_string(), Ty::Int, ClassAttrValue::Int(2))],
        exception_type_tag: None,
        name: "A".to_string(),
        bases: Vec::new(),
        mro: vec!["A".to_string()],
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
    };
    let derived = HirClassDef {
        class_attrs: Vec::new(),
        exception_type_tag: None,
        name: "B".to_string(),
        bases: vec!["A".to_string()],
        mro: vec!["B".to_string(), "A".to_string()],
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
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::TopLevelStmt(HirStmt::Assign {
            target: "v".to_string(),
            value: HirExpr::AttrGet {
                base: Box::new(HirExpr::Name("B".to_string())),
                attr: "x".to_string(),
            },
        })],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: vec![("A".to_string(), base), ("B".to_string(), derived)],
    };
    let mir = build(&hir);
    let read = mir.items.iter().find_map(|item| match item {
        MirItem::TopLevelStmt(MirStmt::Assign { target, value }) if target == "v" => {
            Some(value.clone())
        }
        _ => None,
    });
    assert_eq!(
        read,
        Some(MirExpr::IntLiteral(2)),
        "`B.x` must fold to `A`'s constant even though `B` declares nothing"
    );
}
