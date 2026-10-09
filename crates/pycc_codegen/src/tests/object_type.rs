//! Part 11 of #1371: `type(o)` on a CPython object (`MirExpr::ObjType`,
//! `foreign_type.rs`), pinned on the LLVM IR of an `ext` object.

use super::*;

/// `type(o)` is one `pycc_ext_obj_type` call on the borrowed operand whose
/// `NULL` takes the foreign failure edge; a nested `type(type(o))` passes
/// the inner class on as the outer operand. No reference count is touched.
#[test]
fn type_of_an_object_is_one_helper_call_per_node_with_a_null_check() {
    let inner = MirExpr::ObjType {
        base: Box::new(numpy_pi()),
    };
    let outer = MirExpr::ObjType {
        base: Box::new(inner.clone()),
    };
    assert_eq!(outer.ty(), Ty::Object);
    compile_ext_items_checking_ir(
        "obj_type",
        with_foreign_numpy(vec![
            MirItem::TopLevelStmt(MirStmt::ExprStmt(inner)),
            MirItem::TopLevelStmt(MirStmt::ExprStmt(outer)),
        ]),
        |ir| {
            let calls = ir
                .lines()
                .filter(|line| line.contains("call ptr @pycc_ext_obj_type("))
                .count();
            assert_eq!(calls, 3, "{ir}");
            assert!(ir.contains("declare ptr @pycc_ext_obj_type(ptr)"), "{ir}");
            assert!(ir.contains("foreign_type_failed"), "{ir}");
            assert!(!ir.contains("DecRef"), "{ir}");
        },
    );
}
