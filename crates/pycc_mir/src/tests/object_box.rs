//! #1475 (Part 2 of #1387): a native value moved into an `object` binding
//! is wrapped in `MirExpr::ObjectBox` (`object_box.rs`), at the annotated
//! binding and at a later plain rebinding of the same name; an `object`
//! value is never wrapped.

use crate::*;
use pycc_hir::{HirExpr, HirModule, HirStmt};

fn module_with_stmts(stmts: Vec<HirStmt>) -> HirModule {
    HirModule {
        seeded_builtin_exception_classes: false,
        items: stmts.into_iter().map(HirItem::TopLevelStmt).collect(),
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    }
}

/// Every top-level statement `hir` lowers to, in order.
fn top_level_stmts(hir: &HirModule) -> Vec<MirStmt> {
    build(hir)
        .items
        .into_iter()
        .filter_map(|item| match item {
            MirItem::TopLevelStmt(stmt) => Some(stmt),
            _ => None,
        })
        .collect()
}

#[test]
fn an_object_binding_and_its_rebinding_box_their_native_values() {
    let stmts = top_level_stmts(&module_with_stmts(vec![
        HirStmt::AnnAssign {
            is_final: false,
            target: "y".to_string(),
            annotation: Ty::Object,
            value: Some(HirExpr::IntLiteral(4)),
        },
        HirStmt::Assign {
            target: "y".to_string(),
            value: HirExpr::StringLiteral("s".to_string()),
        },
        HirStmt::ExprStmt(HirExpr::Name("y".to_string())),
    ]));
    assert_eq!(
        stmts,
        vec![
            MirStmt::Assign {
                target: "y".to_string(),
                value: MirExpr::ObjectBox(Box::new(MirExpr::IntLiteral(4))),
            },
            MirStmt::Assign {
                target: "y".to_string(),
                value: MirExpr::ObjectBox(Box::new(MirExpr::StringLiteral("s".to_string()))),
            },
            MirStmt::ExprStmt(MirExpr::Name {
                name: "y".to_string(),
                ty: Ty::Object,
            }),
        ]
    );
    assert_eq!(
        MirExpr::ObjectBox(Box::new(MirExpr::IntLiteral(4))).ty(),
        Ty::Object
    );
}

/// A function-local first assignment that shadows a module-level `object`
/// name is a fresh native local, as `pycc_types` checks it: it must not
/// box against the module frame.
#[test]
fn a_function_local_shadowing_a_module_object_name_is_not_boxed() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::AnnAssign {
                is_final: false,
                target: "x".to_string(),
                annotation: Ty::Object,
                value: Some(HirExpr::StringLiteral("m".to_string())),
            }),
            HirItem::Function {
                name: "g".to_string(),
                params: Vec::new(),
                return_ty: Ty::Int,
                body: vec![
                    HirStmt::Assign {
                        target: "x".to_string(),
                        value: HirExpr::IntLiteral(1),
                    },
                    HirStmt::Return(Some(HirExpr::Name("x".to_string()))),
                ],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let body = build(&hir)
        .items
        .into_iter()
        .find_map(|item| match item {
            MirItem::Function { body, .. } => Some(body),
            _ => None,
        })
        .expect("`g` lowers to a function");
    assert_eq!(
        body[0],
        MirStmt::Assign {
            target: "x".to_string(),
            value: MirExpr::IntLiteral(1),
        }
    );
}

#[test]
fn a_none_value_boxes_and_an_object_value_does_not() {
    let stmts = top_level_stmts(&module_with_stmts(vec![
        HirStmt::AnnAssign {
            is_final: false,
            target: "y".to_string(),
            annotation: Ty::Object,
            value: Some(HirExpr::NoneLiteral),
        },
        HirStmt::AnnAssign {
            is_final: false,
            target: "z".to_string(),
            annotation: Ty::Object,
            value: Some(HirExpr::Name("y".to_string())),
        },
    ]));
    assert_eq!(
        stmts,
        vec![
            MirStmt::Assign {
                target: "y".to_string(),
                value: MirExpr::ObjectBox(Box::new(MirExpr::NoneLiteral)),
            },
            MirStmt::Assign {
                target: "z".to_string(),
                value: MirExpr::Name {
                    name: "y".to_string(),
                    ty: Ty::Object,
                },
            },
        ]
    );
}
