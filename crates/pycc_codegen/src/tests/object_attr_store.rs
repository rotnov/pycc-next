//! #1457 (Part 2 of #1443): `MirStmt::ObjAttrSet` and `MirStmt::ObjDelAttr`
//! emission (`foreign_store.rs`), pinned on the LLVM IR of an `ext` object.

use super::*;

fn numpy_e() -> MirExpr {
    MirExpr::ObjAttrGet {
        base: Box::new(MirExpr::Name {
            name: "numpy".to_string(),
            ty: Ty::Object,
        }),
        attr: "e".to_string(),
        ty: Ty::Object,
    }
}

fn store(attr: &str, value: MirExpr) -> MirStmt {
    MirStmt::ObjAttrSet {
        base: numpy_pi(),
        attr: attr.to_string(),
        value,
    }
}

fn sum() -> MirExpr {
    MirExpr::BinOp {
        op: pycc_mir::BinOpKind::Add,
        left: Box::new(MirExpr::IntLiteral(1)),
        right: Box::new(MirExpr::IntLiteral(2)),
        ty: Ty::Int,
    }
}

/// Every admitted value is boxed by its own packer (a `None` value packs
/// CPython's own `None` as an object), the attribute name is a global C
/// string, and a negative status takes the failure edge.
#[test]
fn a_store_is_one_setattr_call_with_a_packed_value() {
    compile_ext_items_checking_ir(
        "obj_set_attr",
        with_foreign_numpy(
            [
                store("i", MirExpr::IntLiteral(1)),
                store("f", MirExpr::FloatLiteral(1.5)),
                store("b", MirExpr::BoolLiteral(true)),
                store("s", MirExpr::StringLiteral("k".to_string())),
                store("o", numpy_e()),
                store("z", MirExpr::NoneLiteral),
            ]
            .into_iter()
            .map(MirItem::TopLevelStmt)
            .collect(),
        ),
        |ir| {
            let calls = ir
                .lines()
                .filter(|line| line.contains("call i32 @pycc_ext_obj_setattr("))
                .count();
            assert_eq!(calls, 6, "{ir}");
            for packer in [
                "@pycc_ext_obj_pack_int(",
                "@pycc_ext_obj_pack_float(",
                "@pycc_ext_obj_pack_bool(",
                "@pycc_ext_obj_pack_str(",
                "@pycc_ext_obj_pack_object(",
            ] {
                assert!(ir.contains(packer), "{packer}: {ir}");
            }
            for name in ["i", "f", "b", "s", "o", "z"] {
                let global = format!("c\"{name}\\00\"");
                assert!(ir.contains(&global), "{global}: {ir}");
            }
            assert!(ir.contains("foreign_set_attr_failed"), "{ir}");
            assert!(!ir.contains("@pycc_ext_obj_delattr("), "{ir}");
            assert!(!ir.contains("DecRef"), "{ir}");
        },
    );
}

/// The value is evaluated before the base: CPython evaluates `v` before
/// `o` in `o.x = v`, so the value's attribute load is emitted first.
#[test]
fn a_store_evaluates_the_value_before_the_base() {
    compile_ext_items_checking_ir(
        "obj_set_attr_order",
        with_foreign_numpy(vec![MirItem::TopLevelStmt(store("x", numpy_e()))]),
        |ir| {
            let first_use = |global: &str| {
                ir.lines()
                    .position(|line| line.contains("call ") && line.contains(global))
                    .unwrap_or_else(|| panic!("{global}: {ir}"))
            };
            let value = first_use("@pycc_foreign_attr_e)");
            let base = first_use("@pycc_foreign_attr_pi)");
            assert!(
                value < base,
                "value line {value} after base line {base}: {ir}"
            );
        },
    );
}

/// A deletion is one `i32`-returning `pycc_ext_obj_delattr` call taking
/// the borrowed base and the name, and a negative status takes the
/// failure edge.
#[test]
fn a_delete_is_one_delattr_call() {
    compile_ext_items_checking_ir(
        "obj_del_attr",
        with_foreign_numpy(vec![MirItem::TopLevelStmt(MirStmt::ObjDelAttr {
            base: numpy_pi(),
            attr: "gone".to_string(),
        })]),
        |ir| {
            assert!(
                ir.lines()
                    .any(|line| line.contains("call i32 @pycc_ext_obj_delattr(")),
                "{ir}"
            );
            assert!(ir.contains("c\"gone\\00\""), "{ir}");
            assert!(ir.contains("foreign_del_attr_failed"), "{ir}");
            assert!(!ir.contains("@pycc_ext_obj_setattr("), "{ir}");
            assert!(!ir.contains("DecRef"), "{ir}");
        },
    );
}

/// In a function body both statements call their shim entry and bridge
/// their failure; the store's value here is an `int` expression, so the
/// temporary-protection path in `foreign_store.rs` runs, though this test
/// asserts only the calls and the bridge, not the release itself.
#[test]
fn a_function_body_store_or_delete_bridges_its_failure() {
    compile_ext_items_checking_ir(
        "obj_set_del_attr_function",
        with_foreign_numpy(vec![MirItem::Function {
            name: "f".to_string(),
            params: vec![],
            return_ty: Ty::None,
            body: vec![
                store("n", sum()),
                MirStmt::ObjDelAttr {
                    base: numpy_pi(),
                    attr: "n".to_string(),
                },
                MirStmt::Return(None),
            ],
        }]),
        |ir| {
            assert!(ir.contains("@pycc_ext_obj_setattr("), "{ir}");
            assert!(ir.contains("@pycc_ext_obj_delattr("), "{ir}");
            assert!(ir.contains("@pycc_ext_obj_error_bridge("), "{ir}");
        },
    );
}
