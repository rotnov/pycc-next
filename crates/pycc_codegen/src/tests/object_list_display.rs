//! Part 2d of #1371: `MirExpr::ObjList` emission (`foreign_call.rs`'s
//! `emit_list`), pinned on the LLVM IR of an `ext` object.

use super::*;

fn obj_list(elements: Vec<MirExpr>) -> MirExpr {
    MirExpr::ObjList { elements }
}

/// Compiles `exprs` as discarded module-body statements and hands the IR
/// to `check`.
fn module_ir(label: &str, exprs: Vec<MirExpr>, check: impl Fn(&str)) {
    compile_ext_items_checking_ir(
        label,
        with_foreign_numpy(
            exprs
                .into_iter()
                .map(|expr| MirItem::TopLevelStmt(MirStmt::ExprStmt(expr)))
                .collect(),
        ),
        check,
    );
}

/// Every packable element is boxed by its own packer (an object element by
/// `pack_object`), the packed array and its count go to one
/// `pycc_ext_obj_build_list` call per display, an empty display passes a
/// zero count, and a `NULL` result takes the failure edge. The boundary
/// never releases the list it built (#1092).
#[test]
fn a_display_is_one_build_list_call_with_every_element_packed() {
    module_ir(
        "obj_list",
        vec![
            obj_list(vec![
                MirExpr::IntLiteral(1),
                MirExpr::FloatLiteral(1.5),
                MirExpr::BoolLiteral(true),
                MirExpr::StringLiteral("k".to_string()),
                numpy_pi(),
            ]),
            obj_list(Vec::new()),
        ],
        |ir| {
            let calls: Vec<&str> = ir
                .lines()
                .filter(|line| line.contains("call ptr @pycc_ext_obj_build_list("))
                .collect();
            assert_eq!(calls.len(), 2, "{ir}");
            assert!(calls[0].contains("i64 5)"), "{ir}");
            assert!(calls[1].contains("i64 0)"), "{ir}");
            for packer in [
                "@pycc_ext_obj_pack_int(",
                "@pycc_ext_obj_pack_float(",
                "@pycc_ext_obj_pack_bool(",
                "@pycc_ext_obj_pack_str(",
                "@pycc_ext_obj_pack_object(",
            ] {
                assert!(ir.contains(packer), "{packer}: {ir}");
            }
            assert!(ir.contains("foreign_list_failed"), "{ir}");
            assert!(!ir.contains("DecRef"), "{ir}");
        },
    );
}

/// In a function body the display bridges its failure, and an `int`
/// temporary element is protected across the later elements and released
/// after the call.
#[test]
fn a_function_body_display_bridges_its_failure_with_int_temporaries() {
    let sum = || MirExpr::BinOp {
        op: pycc_mir::BinOpKind::Add,
        left: Box::new(MirExpr::IntLiteral(1)),
        right: Box::new(MirExpr::IntLiteral(2)),
        ty: Ty::Int,
    };
    compile_ext_items_checking_ir(
        "obj_list_function",
        with_foreign_numpy(vec![MirItem::Function {
            name: "f".to_string(),
            params: vec![],
            return_ty: Ty::None,
            body: vec![
                MirStmt::ExprStmt(obj_list(vec![sum(), numpy_pi(), sum()])),
                MirStmt::Return(None),
            ],
        }]),
        |ir| {
            assert!(ir.contains("@pycc_ext_obj_build_list("), "{ir}");
            assert!(ir.contains("@pycc_ext_obj_error_bridge("), "{ir}");
            assert!(ir.contains("@pycc_rt_bigint_release("), "{ir}");
        },
    );
}
