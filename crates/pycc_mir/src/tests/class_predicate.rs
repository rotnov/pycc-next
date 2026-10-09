//! Compile-time `isinstance` and `issubclass` folding.
//!
//! Covers folding to a boolean literal for user classes and builtin types,
//! the false results, and the non-name-argument panic.

use crate::*;
use pycc_hir::{HirClassDef, HirExpr, HirItem, HirModule, HirStmt, Ty};

// -----------------------------------------------------------------------
// #435: isinstance/issubclass MIR lowering unit tests
// -----------------------------------------------------------------------

#[test]
fn isinstance_lowers_to_bool_literal_for_user_class() {
    // `isinstance(D(), D)` — D is in D's MRO, so the result is `true`.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
            callee: "print".to_string(),
            args: vec![HirExpr::Call {
                callee: "isinstance".to_string(),
                args: vec![
                    HirExpr::Call {
                        callee: "D".to_string(),
                        args: vec![],
                    },
                    HirExpr::Name("D".to_string()),
                ],
            }],
        }))],
        type_aliases: vec![],
        imports: vec![],
        class_defs: vec![(
            "D".to_string(),
            HirClassDef {
                class_attrs: Vec::new(),
                exception_type_tag: None,
                name: "D".to_string(),
                bases: vec![],
                mro: vec!["D".to_string()],
                methods: vec![("__init__".to_string(), "D.__init__".to_string())],
                attrs: vec![],
                static_methods: vec![],
                class_methods: vec![],
                properties: vec![],
                type_param: None,
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
        mir.items[0],
        MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
            callee: "print".to_string(),
            args: vec![MirExpr::BoolLiteral(true)],
            ty: Ty::None,
        }))
    );
}

#[test]
fn issubclass_lowers_to_bool_literal_for_same_class() {
    // `issubclass(D, D)` — D is in D's MRO, so the result is `true`.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
            callee: "print".to_string(),
            args: vec![HirExpr::Call {
                callee: "issubclass".to_string(),
                args: vec![
                    HirExpr::Name("D".to_string()),
                    HirExpr::Name("D".to_string()),
                ],
            }],
        }))],
        type_aliases: vec![],
        imports: vec![],
        class_defs: vec![(
            "D".to_string(),
            HirClassDef {
                class_attrs: Vec::new(),
                exception_type_tag: None,
                name: "D".to_string(),
                bases: vec![],
                mro: vec!["D".to_string()],
                methods: vec![("__init__".to_string(), "D.__init__".to_string())],
                attrs: vec![],
                static_methods: vec![],
                class_methods: vec![],
                properties: vec![],
                type_param: None,
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
        mir.items[0],
        MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
            callee: "print".to_string(),
            args: vec![MirExpr::BoolLiteral(true)],
            ty: Ty::None,
        }))
    );
}

#[test]
fn isinstance_with_float_lowers_to_bool_literal_true() {
    // `isinstance(1.5, float)` — covers the `Ty::Float` arm in
    // `eval_isinstance_single` at the MIR level.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
            callee: "print".to_string(),
            args: vec![HirExpr::Call {
                callee: "isinstance".to_string(),
                args: vec![
                    HirExpr::FloatLiteral(1.5),
                    HirExpr::Name("float".to_string()),
                ],
            }],
        }))],
        type_aliases: vec![],
        imports: vec![],
        class_defs: vec![],
    };
    let mir = build(&hir);
    assert_eq!(
        mir.items[0],
        MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
            callee: "print".to_string(),
            args: vec![MirExpr::BoolLiteral(true)],
            ty: Ty::None,
        }))
    );
}

#[test]
fn issubclass_with_int_int_lowers_to_bool_literal_true() {
    // `issubclass(int, int)` — covers the `return cls == target_class`
    // line in `eval_issubclass_single` at the MIR level.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
            callee: "print".to_string(),
            args: vec![HirExpr::Call {
                callee: "issubclass".to_string(),
                args: vec![
                    HirExpr::Name("int".to_string()),
                    HirExpr::Name("int".to_string()),
                ],
            }],
        }))],
        type_aliases: vec![],
        imports: vec![],
        class_defs: vec![],
    };
    let mir = build(&hir);
    assert_eq!(
        mir.items[0],
        MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
            callee: "print".to_string(),
            args: vec![MirExpr::BoolLiteral(true)],
            ty: Ty::None,
        }))
    );
}

#[test]
fn issubclass_with_user_class_vs_builtin_lowers_to_false() {
    // `issubclass(D, int)` — user class vs builtin target, covers the
    // `return false` line in `eval_issubclass_single`.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
            callee: "print".to_string(),
            args: vec![HirExpr::Call {
                callee: "issubclass".to_string(),
                args: vec![
                    HirExpr::Name("D".to_string()),
                    HirExpr::Name("int".to_string()),
                ],
            }],
        }))],
        type_aliases: vec![],
        imports: vec![],
        class_defs: vec![(
            "D".to_string(),
            HirClassDef {
                class_attrs: Vec::new(),
                exception_type_tag: None,
                name: "D".to_string(),
                bases: vec![],
                mro: vec!["D".to_string()],
                methods: vec![("__init__".to_string(), "D.__init__".to_string())],
                attrs: vec![],
                static_methods: vec![],
                class_methods: vec![],
                properties: vec![],
                type_param: None,
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
        mir.items[0],
        MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
            callee: "print".to_string(),
            args: vec![MirExpr::BoolLiteral(false)],
            ty: Ty::None,
        }))
    );
}

#[test]
#[should_panic(expected = "internal error: issubclass's first argument")]
fn issubclass_with_non_name_first_arg_panics() {
    // This covers the `unreachable!` branch in `lower_issubclass`.
    // In practice, the type checker rejects this before MIR lowering,
    // but the MIR function must handle the case defensively.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
            callee: "print".to_string(),
            args: vec![HirExpr::Call {
                callee: "issubclass".to_string(),
                args: vec![HirExpr::IntLiteral(42), HirExpr::Name("int".to_string())],
            }],
        }))],
        type_aliases: vec![],
        imports: vec![],
        class_defs: vec![],
    };
    let _ = build(&hir);
}
