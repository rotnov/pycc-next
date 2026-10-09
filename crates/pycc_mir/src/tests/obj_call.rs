//! #1313: a direct call of a foreign CPython binding (`product("ab")` after
//! `from itertools import product`) lowers to `MirExpr::ObjCall`.
//!
//! The fixtures are copied from `tests/import.rs` rather than shared, which
//! keeps this change out of that file; they are small and private there.

use crate::*;
use pycc_diag::Span;
use pycc_hir::{HirExpr, HirModule, HirStmt, ImportBinding};

fn foreign(local_name: &str, item_index: usize) -> ImportBinding {
    ImportBinding::Foreign {
        local_name: local_name.to_string(),
        module_path: local_name.to_string(),
        from: None,
        site: pycc_hir::ForeignImportSite::Item(item_index),
        span: Span::new(0, 0),
    }
}

/// A module whose body is `items`, below one foreign import of `product`.
fn module_with_items(items: Vec<HirItem>) -> HirModule {
    HirModule {
        seeded_builtin_exception_classes: false,
        items,
        type_aliases: Vec::new(),
        imports: vec![foreign("product", 0)],
        class_defs: Vec::new(),
    }
}

fn call(callee: &str, args: Vec<HirExpr>) -> HirExpr {
    HirExpr::Call {
        callee: callee.to_string(),
        args,
    }
}

/// The lowered form of every top-level expression statement in `hir`.
fn discarded_exprs(hir: &HirModule) -> Vec<MirExpr> {
    build(hir)
        .items
        .into_iter()
        .filter_map(|item| match item {
            MirItem::TopLevelStmt(MirStmt::ExprStmt(expr)) => Some(expr),
            _ => None,
        })
        .collect()
}

#[test]
fn a_direct_call_of_a_foreign_binding_lowers_to_obj_call() {
    let hir = module_with_items(vec![HirItem::TopLevelStmt(HirStmt::ExprStmt(call(
        "product",
        vec![
            HirExpr::StringLiteral("ab".to_string()),
            HirExpr::IntLiteral(2),
        ],
    )))]);
    let exprs = discarded_exprs(&hir);
    let [expr] = exprs.as_slice() else {
        panic!("expected one statement, got {exprs:?}");
    };
    assert_eq!(expr.ty(), Ty::Object);
    let MirExpr::ObjCall { callee, args } = expr else {
        panic!("expected an `ObjCall`, got {expr:?}");
    };
    assert!(
        matches!(**callee, MirExpr::Name { ref name, ty: Ty::Object } if name == "product"),
        "{callee:?}"
    );
    assert_eq!(args.len(), 2);
    assert!(matches!(&args[0], MirExpr::StringLiteral(s) if s == "ab"));
    assert!(matches!(args[1], MirExpr::IntLiteral(2)));
}

/// The variant carries no `ty` field; `ty()` answering `Ty::Object`
/// unconditionally is the contract that replaces it.
#[test]
fn an_obj_call_is_typed_object() {
    let node = MirExpr::ObjCall {
        callee: Box::new(MirExpr::Name {
            name: "product".to_string(),
            ty: Ty::Object,
        }),
        args: Vec::new(),
    };
    assert_eq!(node.ty(), Ty::Object);
}

/// A function-local that spells the foreign name is an ordinary local with
/// its own type: the reverse scope scan finds the parameter before the
/// module global, which is the same scan the `ObjCall` guard uses. (A call
/// of an `int` parameter never reaches lowering -- the checker refuses it --
/// so the read is the observable shape.)
#[test]
fn a_function_local_shadow_of_a_foreign_name_keeps_its_own_type() {
    let hir = module_with_items(vec![HirItem::Function {
        name: "f".to_string(),
        params: vec![("product".to_string(), Ty::Int)],
        return_ty: Ty::Int,
        body: vec![HirStmt::Return(Some(HirExpr::Name("product".to_string())))],
    }]);
    let mir = build(&hir);
    let body = mir
        .items
        .iter()
        .find_map(|item| match item {
            MirItem::Function { body, .. } => Some(body),
            _ => None,
        })
        .expect("the function item lowers");
    let rendered = format!("{body:?}");
    assert!(!rendered.contains("ObjCall"), "{rendered}");
    assert!(!rendered.contains("Object"), "{rendered}");
    assert!(rendered.contains("Int"), "{rendered}");
}

/// PEP 572: a walrus in a direct call's argument binds for the next
/// statement, so `collect_named_expr_bindings` has to walk the arguments.
#[test]
fn a_walrus_in_an_obj_call_argument_is_collected() {
    let node = MirExpr::ObjCall {
        callee: Box::new(MirExpr::Name {
            name: "product".to_string(),
            ty: Ty::Object,
        }),
        args: vec![MirExpr::NamedExpr {
            name: "n".to_string(),
            value: Box::new(MirExpr::IntLiteral(1)),
            ty: Ty::Int,
        }],
    };
    let mut out = Vec::new();
    node.collect_named_expr_bindings(&mut out);
    assert_eq!(out, vec![("n".to_string(), Ty::Int)]);
}

/// The same walk reached through lowering: the statement after the call
/// reads `n`, which would panic as an unbound name if the walk missed it.
#[test]
fn a_walrus_in_an_obj_call_argument_binds_for_the_next_statement() {
    let hir = module_with_items(vec![
        HirItem::TopLevelStmt(HirStmt::ExprStmt(call(
            "product",
            vec![HirExpr::NamedExpr {
                name: "n".to_string(),
                value: Box::new(HirExpr::IntLiteral(1)),
            }],
        ))),
        HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Name("n".to_string()))),
    ]);
    let exprs = discarded_exprs(&hir);
    assert!(matches!(exprs[0], MirExpr::ObjCall { .. }), "{exprs:?}");
    assert!(
        matches!(&exprs[1], MirExpr::Name { name, ty: Ty::Int } if name == "n"),
        "{exprs:?}"
    );
}

/// `product[k]` as a HIR subscript of the foreign binding.
fn subscript_of_product(index: HirExpr) -> HirExpr {
    HirExpr::Subscript {
        base: Box::new(HirExpr::Name("product".to_string())),
        index: Box::new(index),
    }
}

/// Part 2a of #1371: a call of an object subscript result lowers to the
/// existing `ObjCall` with the subscript (`ObjSubscript`) as its callee,
/// the arguments in source order.
#[test]
fn a_call_of_an_object_subscript_lowers_to_obj_call() {
    let hir = module_with_items(vec![HirItem::TopLevelStmt(HirStmt::ExprStmt(
        HirExpr::ExprCall {
            callee: Box::new(subscript_of_product(HirExpr::StringLiteral(
                "k".to_string(),
            ))),
            args: vec![HirExpr::IntLiteral(1), HirExpr::Name("product".to_string())],
        },
    ))]);
    let exprs = discarded_exprs(&hir);
    let [MirExpr::ObjCall { callee, args }] = exprs.as_slice() else {
        panic!("expected one `ObjCall`, got {exprs:?}");
    };
    assert!(
        matches!(&**callee, MirExpr::ObjSubscript { base, index }
            if matches!(&**base, MirExpr::Name { name, ty: Ty::Object } if name == "product")
                && matches!(&**index, MirExpr::StringLiteral(k) if k == "k")),
        "{callee:?}"
    );
    assert!(matches!(args[0], MirExpr::IntLiteral(1)), "{args:?}");
    assert!(
        matches!(&args[1], MirExpr::Name { ty: Ty::Object, .. }),
        "{args:?}"
    );
}

/// A walrus in the callee's key or in an argument binds for the next
/// statement, through both the statement's pre-bind walk and
/// `collect_named_expr_bindings`.
#[test]
fn a_walrus_in_an_obj_call_callee_binds_for_the_next_statement() {
    let walrus = |name: &str| HirExpr::NamedExpr {
        name: name.to_string(),
        value: Box::new(HirExpr::IntLiteral(1)),
    };
    let hir = module_with_items(vec![
        HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::ExprCall {
            callee: Box::new(subscript_of_product(walrus("k"))),
            args: vec![walrus("y")],
        })),
        HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Name("k".to_string()))),
        HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Name("y".to_string()))),
    ]);
    let exprs = discarded_exprs(&hir);
    let mut out = Vec::new();
    exprs[0].collect_named_expr_bindings(&mut out);
    assert_eq!(
        out,
        vec![("k".to_string(), Ty::Int), ("y".to_string(), Ty::Int)]
    );
    assert!(matches!(&exprs[1], MirExpr::Name { name, ty: Ty::Int } if name == "k"));
    assert!(matches!(&exprs[2], MirExpr::Name { name, ty: Ty::Int } if name == "y"));
}
