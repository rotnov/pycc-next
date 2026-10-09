//! Part 10 of #1371: `not o` on a CPython object, pinned on the LLVM IR of
//! an `ext` object.

use super::*;

/// `not o` is the object's own truth test (`pycc_ext_obj_truthy`, whose
/// `-1` takes the foreign failure edge immediately) inverted into a native
/// `bool`. Nothing is boxed and no reference count is touched.
#[test]
fn not_on_an_object_inverts_its_truth_test() {
    compile_ext_items_checking_ir(
        "obj_not",
        with_foreign_numpy(vec![MirItem::TopLevelStmt(MirStmt::ExprStmt(
            MirExpr::Not(Box::new(numpy_pi())),
        ))]),
        |ir| {
            let truthy = ir
                .lines()
                .filter(|line| line.contains("call i32 @pycc_ext_obj_truthy("))
                .count();
            assert_eq!(truthy, 1, "{ir}");
            assert!(ir.contains("not_truthy"), "{ir}");
            assert!(ir.contains("bool_from_not"), "{ir}");
            assert!(!ir.contains("@pycc_ext_obj_pack_"), "{ir}");
            assert!(!ir.contains("DecRef"), "{ir}");
        },
    );
}
