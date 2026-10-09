//! Part 1 of #891: `MirExpr::ObjUnpack` emission (`foreign_unpack.rs`),
//! pinned on the LLVM IR of an `ext` object.

use super::*;

fn obj_unpack(arity: usize) -> MirExpr {
    MirExpr::ObjUnpack {
        value: Box::new(numpy_pi()),
        arity,
    }
}

/// In a module body each unpack is one shim call carrying its arity as an
/// `i64`, and a `NULL` result takes the module-exec failure edge.
#[test]
fn a_module_body_unpack_is_one_shim_call_with_its_arity() {
    compile_ext_items_checking_ir(
        "obj_unpack",
        with_foreign_numpy(vec![
            MirItem::TopLevelStmt(MirStmt::ExprStmt(obj_unpack(2))),
            MirItem::TopLevelStmt(MirStmt::ExprStmt(obj_unpack(3))),
        ]),
        |ir| {
            for arity in [2, 3] {
                let needle = format!("i64 {arity})");
                assert!(
                    ir.lines()
                        .any(|line| line.contains("call ptr @pycc_ext_obj_unpack(")
                            && line.contains(&needle)),
                    "{needle}: {ir}"
                );
            }
            assert!(ir.contains("foreign_unpack_failed"), "{ir}");
            assert!(!ir.contains("DecRef"), "{ir}");
        },
    );
}

/// In a function body the unpack bridges its failure to the innermost
/// exception target, so a compiled `try`/`except` can catch it.
#[test]
fn a_function_body_unpack_bridges_its_failure() {
    compile_ext_items_checking_ir(
        "obj_unpack_function",
        with_foreign_numpy(vec![MirItem::Function {
            name: "f".to_string(),
            params: vec![],
            return_ty: Ty::None,
            body: vec![MirStmt::ExprStmt(obj_unpack(2)), MirStmt::Return(None)],
        }]),
        |ir| {
            assert!(ir.contains("@pycc_ext_obj_unpack("), "{ir}");
            assert!(ir.contains("@pycc_ext_obj_error_bridge("), "{ir}");
        },
    );
}
