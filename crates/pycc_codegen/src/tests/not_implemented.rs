//! #1418: `MirExpr::NotImplemented` emission, pinned on the LLVM IR of an
//! `ext` object.

use super::*;

#[test]
fn not_implemented_is_one_infallible_accessor_call_returned_as_the_object() {
    compile_ext_items_checking_ir(
        "not_implemented_return",
        vec![MirItem::Function {
            name: "_ni".to_string(),
            params: Vec::new(),
            return_ty: Ty::Object,
            body: vec![MirStmt::Return(Some(MirExpr::NotImplemented))],
        }],
        |ir| {
            assert!(
                ir.contains("declare ptr @pycc_ext_obj_not_implemented()"),
                "{ir}"
            );
            assert!(
                ir.contains("call ptr @pycc_ext_obj_not_implemented()"),
                "{ir}"
            );
            assert!(ir.contains("ret ptr %not_implemented"), "{ir}");
        },
    );
}
