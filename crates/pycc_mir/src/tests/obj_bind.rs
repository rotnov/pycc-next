//! #1325: a module-level name bound to a CPython object value, and a
//! bare-name `for` over it.
//!
//! The fixtures mirror `tests/obj_call.rs`'s; they are small and private
//! there.

use crate::*;
use pycc_diag::Span;
use pycc_hir::{HirExpr, HirModule, HirStmt, ImportBinding};

/// A module whose body is `stmts`, below one foreign import of `product`.
fn module_with_stmts(stmts: Vec<HirStmt>) -> HirModule {
    HirModule {
        seeded_builtin_exception_classes: false,
        items: stmts.into_iter().map(HirItem::TopLevelStmt).collect(),
        type_aliases: Vec::new(),
        imports: vec![ImportBinding::Foreign {
            local_name: "product".to_string(),
            module_path: "itertools".to_string(),
            from: None,
            site: pycc_hir::ForeignImportSite::Item(0),
            span: Span::new(0, 0),
        }],
        class_defs: Vec::new(),
    }
}

fn bind_product_to_x() -> HirStmt {
    HirStmt::Assign {
        target: "x".to_string(),
        value: HirExpr::Call {
            callee: "product".to_string(),
            args: vec![HirExpr::StringLiteral("ab".to_string())],
        },
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
fn binding_a_foreign_call_result_lowers_to_an_object_typed_assign() {
    let stmts = top_level_stmts(&module_with_stmts(vec![bind_product_to_x()]));
    let [MirStmt::Assign { target, value }] = stmts.as_slice() else {
        panic!("expected one assignment, got {stmts:?}");
    };
    assert_eq!(target, "x");
    assert!(matches!(value, MirExpr::ObjCall { .. }), "{value:?}");
    assert_eq!(value.ty(), Ty::Object);
}

#[test]
fn a_bare_name_for_over_an_object_binding_lowers_to_for_object() {
    let stmts = top_level_stmts(&module_with_stmts(vec![
        bind_product_to_x(),
        HirStmt::ForList {
            var: "t".to_string(),
            list: "x".to_string(),
            body: vec![],
        },
    ]));
    let [_, MirStmt::ForObject { var, iter, .. }] = stmts.as_slice() else {
        panic!("expected an assignment then a `ForObject`, got {stmts:?}");
    };
    assert_eq!(var, "t");
    assert_eq!(
        iter,
        &MirExpr::Name {
            name: "x".to_string(),
            ty: Ty::Object,
        }
    );
}
