//! Value-position `C[x]` lowering through `__class_getitem__` (PEP 560).
//!
//! Covers the static-method and class-method spellings of the hook, and a
//! value binding that shadows a class name subscripting as an ordinary
//! value instead.

use crate::*;
use pycc_hir::{HirExpr, HirItem, HirModule, HirStmt, Ty};

// -- PEP 560 (#610): value-position `C[x]` lowering ---------------------

/// Builds a module with class `C` whose `__class_getitem__` is spelled
/// as `hook_kind` ("static" or "classmethod"), plus `extra_items`.
fn class_getitem_hir(hook_kind: &str, extra_items: Vec<HirItem>) -> HirModule {
    let self_ty = Ty::Instance(Box::new("C".to_string()));
    let init = HirItem::Function {
        name: "C.__init__".to_string(),
        params: vec![("self".to_string(), self_ty.clone())],
        return_ty: Ty::None,
        body: vec![HirStmt::Return(None)],
    };
    let is_static = hook_kind == "static";
    let hook_symbol = if is_static {
        "C.__class_getitem__.static".to_string()
    } else {
        "C.__class_getitem__.classmethod".to_string()
    };
    let mut params = Vec::new();
    if !is_static {
        params.push(("cls".to_string(), self_ty.clone()));
    }
    params.push(("key".to_string(), Ty::Int));
    let hook = HirItem::Function {
        name: hook_symbol.clone(),
        params,
        return_ty: Ty::Int,
        body: vec![HirStmt::Return(Some(HirExpr::Name("key".to_string())))],
    };
    let mut items = vec![init, hook];
    items.extend(extra_items);
    let (static_methods, class_methods) = if is_static {
        (
            vec![("__class_getitem__".to_string(), hook_symbol)],
            Vec::new(),
        )
    } else {
        (
            Vec::new(),
            vec![("__class_getitem__".to_string(), hook_symbol)],
        )
    };
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
                static_methods,
                class_methods,
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
fn class_getitem_on_a_bare_class_name_lowers_to_the_static_hook() {
    let hir = class_getitem_hir(
        "static",
        vec![HirItem::TopLevelStmt(HirStmt::ExprStmt(
            HirExpr::Subscript {
                base: Box::new(HirExpr::Name("C".to_string())),
                index: Box::new(HirExpr::IntLiteral(3)),
            },
        ))],
    );
    let mir = build(&hir);
    assert_eq!(
        mir.items.last(),
        Some(&MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
            callee: "C.__class_getitem__.static".to_string(),
            args: vec![MirExpr::IntLiteral(3)],
            ty: Ty::Int,
        })))
    );
}

#[test]
fn class_getitem_on_a_bare_class_name_lowers_to_the_classmethod_hook() {
    // The `@classmethod` spelling needs the `NullInstance` receiver the
    // `MethodCall` arm prepends; delegating there is what supplies it.
    let hir = class_getitem_hir(
        "classmethod",
        vec![HirItem::TopLevelStmt(HirStmt::ExprStmt(
            HirExpr::Subscript {
                base: Box::new(HirExpr::Name("C".to_string())),
                index: Box::new(HirExpr::IntLiteral(3)),
            },
        ))],
    );
    let mir = build(&hir);
    assert_eq!(
        mir.items.last(),
        Some(&MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
            callee: "C.__class_getitem__.classmethod".to_string(),
            args: vec![
                MirExpr::NullInstance {
                    ty: Ty::Instance(Box::new("C".to_string())),
                },
                MirExpr::IntLiteral(3),
            ],
            ty: Ty::Int,
        })))
    );
}

#[test]
fn a_value_shadowing_a_class_name_subscripts_as_a_value() {
    // `pycc_types` applies the identical guard, so both crates must agree
    // that a name bound as a value indexes that value rather than
    // dispatching the class hook.
    let hir = class_getitem_hir(
        "static",
        vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "C".to_string(),
                value: HirExpr::ListLiteral(vec![HirExpr::IntLiteral(1)]),
            }),
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Subscript {
                base: Box::new(HirExpr::Name("C".to_string())),
                index: Box::new(HirExpr::IntLiteral(0)),
            })),
        ],
    );
    let mir = build(&hir);
    // Both outcomes of the pattern are exercised so the `matches!`
    // fallback arm is a covered region under D-014: the trailing item is
    // the plain subscript, and the leading item (the class's own
    // `__init__`) is not.
    let is_value_subscript = |item: &MirItem| {
        matches!(
            item,
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Subscript { .. }))
        )
    };
    assert!(is_value_subscript(mir.items.last().unwrap()));
    assert!(!is_value_subscript(&mir.items[0]));
}
