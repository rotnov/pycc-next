//! `@staticmethod` and `@classmethod` dispatch.
//!
//! Covers calls through the class name and through an instance, the null
//! instance receiver a class method takes, the fall-through to the instance
//! path, and the ghost-MRO internal-error panics on each of those routes.

use crate::*;
use pycc_hir::{HirExpr, HirItem, HirModule, HirStmt, Ty};

// -- #436: @staticmethod / @classmethod MIR lowering --------------------

/// Builds a minimal module with a class `C` that has `__init__`, a
/// static method `create(x: int) -> int`, and a class method
/// `greet(cls, x: int) -> int`. Used by the #436 MIR tests below.
fn static_class_hir(extra_items: Vec<HirItem>) -> HirModule {
    let self_ty = Ty::Instance(Box::new("C".to_string()));
    let init = HirItem::Function {
        name: "C.__init__".to_string(),
        params: vec![("self".to_string(), self_ty.clone())],
        return_ty: Ty::None,
        body: vec![HirStmt::Return(None)],
    };
    let static_fn = HirItem::Function {
        name: "C.create.static".to_string(),
        params: vec![("x".to_string(), Ty::Int)],
        return_ty: Ty::Int,
        body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
    };
    let class_fn = HirItem::Function {
        name: "C.greet.classmethod".to_string(),
        params: vec![
            ("cls".to_string(), self_ty.clone()),
            ("x".to_string(), Ty::Int),
        ],
        return_ty: Ty::Int,
        body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
    };
    let mut items = vec![init, static_fn, class_fn];
    items.extend(extra_items);
    HirModule {
        seeded_builtin_exception_classes: false,
        items,
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: vec![(
            "C".to_string(),
            pycc_hir::HirClassDef {
                class_attrs: Vec::new(),
                exception_type_tag: None,
                name: "C".to_string(),
                bases: Vec::new(),
                mro: vec!["C".to_string()],
                attrs: Vec::new(),
                methods: vec![("__init__".to_string(), "C.__init__".to_string())],
                type_param: None,
                properties: Vec::new(),
                static_methods: vec![("create".to_string(), "C.create.static".to_string())],
                class_methods: vec![("greet".to_string(), "C.greet.classmethod".to_string())],
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
fn a_static_method_call_through_class_lowers_without_a_receiver() {
    let hir = static_class_hir(vec![HirItem::TopLevelStmt(HirStmt::ExprStmt(
        HirExpr::MethodCall {
            base: Box::new(HirExpr::Name("C".to_string())),
            method: "create".to_string(),
            args: vec![HirExpr::IntLiteral(42)],
        },
    ))]);
    let mir = build(&hir);
    assert_eq!(
        mir.items.last(),
        Some(&MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
            callee: "C.create.static".to_string(),
            args: vec![MirExpr::IntLiteral(42)],
            ty: Ty::Int,
        })))
    );
}

#[test]
fn a_class_method_call_through_class_lowers_with_null_instance_receiver() {
    let hir = static_class_hir(vec![HirItem::TopLevelStmt(HirStmt::ExprStmt(
        HirExpr::MethodCall {
            base: Box::new(HirExpr::Name("C".to_string())),
            method: "greet".to_string(),
            args: vec![HirExpr::IntLiteral(42)],
        },
    ))]);
    let mir = build(&hir);
    assert_eq!(
        mir.items.last(),
        Some(&MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
            callee: "C.greet.classmethod".to_string(),
            args: vec![
                MirExpr::NullInstance {
                    ty: Ty::Instance(Box::new("C".to_string())),
                },
                MirExpr::IntLiteral(42),
            ],
            ty: Ty::Int,
        })))
    );
}

#[test]
fn a_static_method_call_through_instance_lowers_without_a_receiver() {
    let hir = static_class_hir(vec![
        HirItem::TopLevelStmt(HirStmt::Assign {
            target: "c".to_string(),
            value: HirExpr::Call {
                callee: "C".to_string(),
                args: vec![],
            },
        }),
        HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::MethodCall {
            base: Box::new(HirExpr::Name("c".to_string())),
            method: "create".to_string(),
            args: vec![HirExpr::IntLiteral(42)],
        })),
    ]);
    let mir = build(&hir);
    assert_eq!(
        mir.items.last(),
        Some(&MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
            callee: "C.create.static".to_string(),
            args: vec![MirExpr::IntLiteral(42)],
            ty: Ty::Int,
        })))
    );
}

/// #1346: a static method called through a non-name receiver evaluates
/// the receiver first -- `C().create(42)` lowers to a `Sequence` whose
/// `discard` is `C()`'s own lowering and whose `value` is the receiver-less
/// call the plain-name form above produces.
#[test]
fn a_static_method_call_through_a_call_receiver_sequences_the_receiver() {
    let receiver = || HirExpr::Call {
        callee: "C".to_string(),
        args: vec![],
    };
    let hir = static_class_hir(vec![
        HirItem::TopLevelStmt(HirStmt::ExprStmt(receiver())),
        HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::MethodCall {
            base: Box::new(receiver()),
            method: "create".to_string(),
            args: vec![HirExpr::IntLiteral(42)],
        })),
    ]);
    let mir = build(&hir);
    let [
        ..,
        MirItem::TopLevelStmt(MirStmt::ExprStmt(lowered_receiver)),
        last,
    ] = mir.items.as_slice()
    else {
        panic!("two trailing expression statements: {:?}", mir.items);
    };
    assert_eq!(
        last,
        &MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Sequence {
            discard: Box::new(lowered_receiver.clone()),
            value: Box::new(MirExpr::Call {
                callee: "C.create.static".to_string(),
                args: vec![MirExpr::IntLiteral(42)],
                ty: Ty::Int,
            }),
        }))
    );
}

#[test]
fn a_class_method_call_through_instance_lowers_with_instance_as_cls() {
    let hir = static_class_hir(vec![
        HirItem::TopLevelStmt(HirStmt::Assign {
            target: "c".to_string(),
            value: HirExpr::Call {
                callee: "C".to_string(),
                args: vec![],
            },
        }),
        HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::MethodCall {
            base: Box::new(HirExpr::Name("c".to_string())),
            method: "greet".to_string(),
            args: vec![HirExpr::IntLiteral(42)],
        })),
    ]);
    let mir = build(&hir);
    assert_eq!(
        mir.items.last(),
        Some(&MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
            callee: "C.greet.classmethod".to_string(),
            args: vec![
                MirExpr::Name {
                    name: "c".to_string(),
                    ty: Ty::Instance(Box::new("C".to_string())),
                },
                MirExpr::IntLiteral(42),
            ],
            ty: Ty::Int,
        })))
    );
}

#[test]
fn null_instance_ty_returns_the_stored_type() {
    let expr = MirExpr::NullInstance {
        ty: Ty::Instance(Box::new("C".to_string())),
    };
    assert_eq!(expr.ty(), Ty::Instance(Box::new("C".to_string())));
}

#[test]
#[should_panic(expected = "pycc_mir: internal error: `C` has no recorded type")]
fn class_name_method_call_with_no_static_or_class_method_falls_through_to_instance_path() {
    // When a `MethodCall` on a class name does not find the method in
    // `static_methods` or `class_methods`, the code falls through to the
    // instance-receiver path, which calls `lower_expr` on the bare class
    // name. Since class names are not in the scope, `lookup` panics.
    // This covers the `None` branch of the class-name `class_mangled`
    // `if let` (the fallthrough past the class-name interception block).
    let hir = static_class_hir(vec![HirItem::TopLevelStmt(HirStmt::ExprStmt(
        HirExpr::MethodCall {
            base: Box::new(HirExpr::Name("C".to_string())),
            method: "nonexistent".to_string(),
            args: vec![],
        },
    ))]);
    let _ = build(&hir);
}

#[test]
#[should_panic(
    expected = "pycc_mir: internal error: class `Ghost` in MRO has no registered HirClassDef"
)]
fn static_method_call_through_class_name_with_ghost_mro_panics() {
    // A `MethodCall` on a class name whose MRO contains a ghost class
    // triggers the defensive panic in the static_methods MRO walk. The
    // method name `missing` is not in `C`'s own static_methods, so the
    // `find_map` continues to `Ghost` and panics.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::TopLevelStmt(HirStmt::ExprStmt(
            HirExpr::MethodCall {
                base: Box::new(HirExpr::Name("C".to_string())),
                method: "missing".to_string(),
                args: vec![],
            },
        ))],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: vec![(
            "C".to_string(),
            pycc_hir::HirClassDef {
                class_attrs: Vec::new(),
                exception_type_tag: None,
                name: "C".to_string(),
                bases: vec!["Ghost".to_string()],
                mro: vec!["C".to_string(), "Ghost".to_string()],
                attrs: Vec::new(),
                methods: vec![("__init__".to_string(), "C.__init__".to_string())],
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
    let _ = build(&hir);
}

#[test]
#[should_panic(
    expected = "pycc_mir: internal error: class `Ghost` in MRO has no registered HirClassDef"
)]
fn class_method_call_through_class_name_with_ghost_mro_panics() {
    // Same as above but exercises the class_methods MRO walk. The
    // method name `missing` is not in `C`'s own static_methods or
    // class_methods, so both walks reach `Ghost` and the second one
    // (class_methods) panics.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::TopLevelStmt(HirStmt::ExprStmt(
            HirExpr::MethodCall {
                base: Box::new(HirExpr::Name("C".to_string())),
                method: "missing".to_string(),
                args: vec![],
            },
        ))],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: vec![(
            "C".to_string(),
            pycc_hir::HirClassDef {
                class_attrs: Vec::new(),
                exception_type_tag: None,
                name: "C".to_string(),
                bases: vec!["Ghost".to_string()],
                mro: vec!["C".to_string(), "Ghost".to_string()],
                attrs: Vec::new(),
                methods: vec![("__init__".to_string(), "C.__init__".to_string())],
                type_param: None,
                properties: Vec::new(),
                static_methods: vec![("found".to_string(), "C.found.static".to_string())],
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

#[test]
#[should_panic(
    expected = "pycc_mir: internal error: class `Ghost` in MRO has no registered HirClassDef"
)]
fn static_method_call_through_instance_with_ghost_mro_panics() {
    // A `MethodCall` on an instance whose class MRO contains a ghost
    // class triggers the defensive panic in the instance-receiver
    // static_methods MRO walk.
    let self_ty = Ty::Instance(Box::new("Derived".to_string()));
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "Derived.__init__".to_string(),
                params: vec![("self".to_string(), self_ty.clone())],
                return_ty: Ty::None,
                body: vec![HirStmt::Return(None)],
            },
            HirItem::Function {
                name: "use_derived".to_string(),
                params: vec![("d".to_string(), self_ty.clone())],
                return_ty: Ty::None,
                body: vec![HirStmt::ExprStmt(HirExpr::MethodCall {
                    base: Box::new(HirExpr::Name("d".to_string())),
                    method: "missing".to_string(),
                    args: vec![],
                })],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: vec![(
            "Derived".to_string(),
            pycc_hir::HirClassDef {
                class_attrs: Vec::new(),
                exception_type_tag: None,
                name: "Derived".to_string(),
                bases: vec!["Ghost".to_string()],
                mro: vec!["Derived".to_string(), "Ghost".to_string()],
                attrs: Vec::new(),
                methods: vec![("__init__".to_string(), "Derived.__init__".to_string())],
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
    let _ = build(&hir);
}

#[test]
#[should_panic(
    expected = "pycc_mir: internal error: class `Ghost` in MRO has no registered HirClassDef"
)]
fn class_method_call_through_instance_with_ghost_mro_panics() {
    // Same as above but exercises the instance-receiver class_methods
    // MRO walk. `Derived` has a static method `found` so the
    // static_methods walk does not reach `Ghost`, but the class_methods
    // walk does.
    let self_ty = Ty::Instance(Box::new("Derived".to_string()));
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "Derived.__init__".to_string(),
                params: vec![("self".to_string(), self_ty.clone())],
                return_ty: Ty::None,
                body: vec![HirStmt::Return(None)],
            },
            HirItem::Function {
                name: "use_derived".to_string(),
                params: vec![("d".to_string(), self_ty.clone())],
                return_ty: Ty::None,
                body: vec![HirStmt::ExprStmt(HirExpr::MethodCall {
                    base: Box::new(HirExpr::Name("d".to_string())),
                    method: "missing".to_string(),
                    args: vec![],
                })],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: vec![(
            "Derived".to_string(),
            pycc_hir::HirClassDef {
                class_attrs: Vec::new(),
                exception_type_tag: None,
                name: "Derived".to_string(),
                bases: vec!["Ghost".to_string()],
                mro: vec!["Derived".to_string(), "Ghost".to_string()],
                attrs: Vec::new(),
                methods: vec![("__init__".to_string(), "Derived.__init__".to_string())],
                type_param: None,
                properties: Vec::new(),
                static_methods: vec![("found".to_string(), "Derived.found.static".to_string())],
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
