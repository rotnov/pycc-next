//! Part 9 of #1371: `MirStmt::ObjRaise` emission (`foreign_raise.rs`),
//! pinned on the LLVM IR of an `ext` object.

use super::*;

fn obj_raise() -> MirStmt {
    MirStmt::ObjRaise { value: numpy_pi() }
}

/// In the module body the raise is one borrowed `void` helper call and
/// nothing is released: the helper itself always bridges, so there is no
/// result to test and no failure edge of its own.
#[test]
fn a_module_body_object_raise_is_one_helper_call_routed_to_the_exception_exit() {
    compile_ext_items_checking_ir(
        "obj_raise_module",
        with_foreign_numpy(vec![MirItem::TopLevelStmt(obj_raise())]),
        |ir| {
            let calls = ir
                .lines()
                .filter(|line| line.contains("call void @pycc_ext_obj_raise(ptr"))
                .count();
            assert_eq!(calls, 1, "{ir}");
            assert!(ir.contains("declare void @pycc_ext_obj_raise(ptr)"), "{ir}");
            assert!(!ir.contains("DecRef"), "{ir}");
        },
    );
}

/// A function body raise, inside a `try` whose handler catches it, and a
/// function declared `-> int` whose body only raises: `raise o` terminates
/// the block exactly like a native `raise`, so no fallthrough return is
/// needed and the handler is reachable.
#[test]
fn a_function_body_object_raise_terminates_its_block_like_a_native_raise() {
    compile_ext_items_checking_ir(
        "obj_raise_function",
        with_foreign_numpy(vec![
            MirItem::Function {
                name: "f".to_string(),
                params: vec![],
                return_ty: Ty::Int,
                body: vec![obj_raise()],
            },
            MirItem::Function {
                name: "g".to_string(),
                params: vec![],
                return_ty: Ty::None,
                body: vec![
                    MirStmt::Try {
                        body: vec![obj_raise()],
                        handlers: vec![MirExceptHandler {
                            exc_type_tag: None,
                            binding_name: None,
                            binding_ty: None,
                            body: vec![],
                        }],
                        orelse: vec![],
                        finalbody: vec![],
                    },
                    MirStmt::Return(None),
                ],
            },
        ]),
        |ir| {
            let calls = ir
                .lines()
                .filter(|line| line.contains("call void @pycc_ext_obj_raise(ptr"))
                .count();
            assert_eq!(calls, 2, "{ir}");
        },
    );
}
