//! Part 11 of #1371: `type(o)` on the bound object.
//!
//! The same rationale as the `len` tests in the parent module: this is the
//! only place the `HirExpr::Call { callee: "type" }` -> `MirExpr::ObjType`
//! split runs on real HIR. `pycc_codegen`'s `tests/object_type.rs`
//! hand-builds the node because it is about what LLVM receives.

use super::*;

#[test]
fn type_of_a_foreign_object_lowers_to_obj_type() {
    // `import numpy` / `type(numpy)`, and `type(type(numpy))`: the inner
    // node's own `Ty::Object` is what admits the outer one.
    let hir = module_with_discarded(call(
        "type",
        vec![call(
            "type",
            vec![pycc_hir::HirExpr::Name("numpy".to_string())],
        )],
    ));
    let expr = only_discarded_expr(&hir);
    assert_eq!(expr.ty(), Ty::Object);
    let MirExpr::ObjType { base } = expr else {
        panic!("expected an `ObjType`");
    };
    let MirExpr::ObjType { base } = *base else {
        panic!("expected the operand to be an `ObjType` too");
    };
    assert!(matches!(*base, MirExpr::Name { ref name, ty: Ty::Object } if name == "numpy"));
}

#[test]
fn a_module_level_type_definition_keeps_its_call() {
    // A program's own `def type` puts `$fn:type` in scope; `pycc_types`
    // resolves the call to it, so the lowering must not divert it to the
    // builtin's node.
    let hir = HirModule {
        items: vec![
            HirItem::Function {
                name: "type".to_string(),
                params: vec![("x".to_string(), Ty::Int)],
                return_ty: Ty::Int,
                body: vec![pycc_hir::HirStmt::Return(Some(pycc_hir::HirExpr::Name(
                    "x".to_string(),
                )))],
            },
            HirItem::TopLevelStmt(pycc_hir::HirStmt::ExprStmt(call(
                "type",
                vec![pycc_hir::HirExpr::Name("numpy".to_string())],
            ))),
        ],
        ..module_with_imports(vec![foreign("numpy", 0)])
    };
    assert!(matches!(
        only_discarded_expr(&hir),
        MirExpr::Call { ref callee, .. } if callee == "type"
    ));
}

#[test]
fn a_walrus_in_the_operand_of_type_is_collected() {
    let mut found = Vec::new();
    MirExpr::ObjType {
        base: Box::new(MirExpr::NamedExpr {
            name: "o".to_string(),
            value: Box::new(MirExpr::Name {
                name: "numpy".to_string(),
                ty: Ty::Object,
            }),
            ty: Ty::Object,
        }),
    }
    .collect_named_expr_bindings(&mut found);
    assert_eq!(found, vec![("o".to_string(), Ty::Object)]);
}
