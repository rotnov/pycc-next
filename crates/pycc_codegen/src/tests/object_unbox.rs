//! #1476 (Part 3 of #1387): the narrowed read of an `object` slot
//! (`MirExpr::ObjectUnbox`, `object_unbox.rs`), pinned on the LLVM IR of an
//! `ext` object.
//!
//! A native use calls exactly one `pycc_ext_obj_unbox_*` helper for the
//! narrowed type and routes its `-1` to the foreign failure edge. A use that
//! packs the value back into a `PyObject *` calls none: the identity
//! peephole hands the packer the object itself.

use super::*;
use pycc_mir::{BoolOpKind, CmpOpKind};

fn object(name: &str) -> MirExpr {
    MirExpr::Name {
        name: name.to_string(),
        ty: Ty::Object,
    }
}

fn unboxed(name: &str, ty: Ty) -> MirExpr {
    MirExpr::ObjectUnbox(Box::new(object(name)), Box::new(ty))
}

/// `def <name>(o: object)` evaluating each of `exprs` for its effects.
fn function(name: &str, exprs: Vec<MirExpr>) -> MirItem {
    let mut body: Vec<MirStmt> = exprs.into_iter().map(MirStmt::ExprStmt).collect();
    body.push(MirStmt::Return(None));
    MirItem::Function {
        name: name.to_string(),
        params: vec![("o".to_string(), Ty::Object)],
        return_ty: Ty::None,
        body,
    }
}

#[test]
fn each_narrowed_type_calls_its_own_unbox_helper_and_routes_its_failure() {
    compile_ext_items_checking_ir(
        "object_unbox_kinds",
        vec![function(
            "kinds",
            vec![
                unboxed("o", Ty::Int),
                unboxed("o", Ty::Float),
                unboxed("o", Ty::Bool),
                unboxed("o", Ty::Str),
                unboxed("o", Ty::Instance(Box::new("Token".to_string()))),
            ],
        )],
        |ir| {
            for (symbol, args) in [
                ("pycc_ext_obj_unbox_int", "(ptr %"),
                ("pycc_ext_obj_unbox_float", "(ptr %"),
                ("pycc_ext_obj_unbox_bool", "(ptr %"),
                ("pycc_ext_obj_unbox_str", "(ptr %"),
                ("pycc_ext_obj_unbox_instance", "(ptr %"),
            ] {
                let call = format!("call i32 @{symbol}{args}");
                assert_eq!(
                    ir.lines().filter(|line| line.contains(&call)).count(),
                    1,
                    "{symbol}: {ir}"
                );
            }
            // The instance helper is told the class by name.
            assert!(ir.contains("@pycc_unbox_class_Token"), "{ir}");
            assert!(ir.contains("c\"Token\\00\""), "{ir}");
            // The out slots live in the entry block, and a failed unbox
            // takes the function's bridge-and-branch failure edge.
            assert!(ir.contains("object_unbox_out"), "{ir}");
            assert!(ir.contains("object_unbox_failed"), "{ir}");
            assert!(ir.contains("@pycc_ext_obj_error_bridge("), "{ir}");
        },
    );
}

#[test]
fn a_module_scope_narrowed_read_takes_the_module_exec_failure_edge() {
    compile_ext_items_checking_ir(
        "object_unbox_module",
        with_foreign_numpy(vec![MirItem::TopLevelStmt(MirStmt::ExprStmt(
            MirExpr::ObjectUnbox(Box::new(numpy_pi()), Box::new(Ty::Float)),
        ))]),
        |ir| {
            assert!(ir.contains("call i32 @pycc_ext_obj_unbox_float("), "{ir}");
            assert!(ir.contains("object_unbox_failed"), "{ir}");
        },
    );
}

#[test]
fn packing_a_narrowed_read_back_into_an_object_unboxes_nothing() {
    let read = || Box::new(unboxed("o", Ty::Int));
    compile_ext_items_checking_ir(
        "object_unbox_identity",
        with_foreign_numpy(vec![
            function(
                "packs",
                vec![
                    // `numpy.pi(o)`: a foreign call argument.
                    MirExpr::ObjCall {
                        callee: Box::new(numpy_pi()),
                        args: vec![unboxed("o", Ty::Int)],
                    },
                    // `o == numpy.pi`: a rich-comparison operand.
                    MirExpr::ObjCompare {
                        op: CmpOpKind::Eq,
                        left: read(),
                        right: Box::new(numpy_pi()),
                    },
                    // `o in numpy.pi`: a membership item.
                    MirExpr::ObjContains {
                        negate: false,
                        item: read(),
                        container: Box::new(numpy_pi()),
                    },
                    // `numpy.pi[o]`: a subscript key.
                    MirExpr::ObjSubscript {
                        base: Box::new(numpy_pi()),
                        index: read(),
                    },
                    // `numpy.pi[o:]`: a slice bound.
                    MirExpr::ObjSlice {
                        base: Box::new(numpy_pi()),
                        start: Some(read()),
                        stop: None,
                        step: None,
                    },
                    // `[o]` bound to an object slot.
                    MirExpr::ObjList {
                        elements: vec![unboxed("o", Ty::Int)],
                    },
                    // A boxing into an `object` slot.
                    MirExpr::ObjectBox(read()),
                    // `o or numpy.pi` with an `object` result.
                    MirExpr::BoolOp {
                        op: BoolOpKind::Or,
                        left: read(),
                        right: Box::new(numpy_pi()),
                        ty: Ty::Object,
                        truth_only: false,
                    },
                ],
            ),
            MirItem::Function {
                name: "stores".to_string(),
                params: vec![("o".to_string(), Ty::Object)],
                return_ty: Ty::None,
                body: vec![
                    // `numpy.pi.x = o`: an attribute store.
                    MirStmt::ObjAttrSet {
                        base: numpy_pi(),
                        attr: "x".to_string(),
                        value: unboxed("o", Ty::Str),
                    },
                    MirStmt::Return(None),
                ],
            },
        ]),
        |ir| {
            assert!(!ir.contains("pycc_ext_obj_unbox_"), "{ir}");
            assert!(ir.contains("@pycc_ext_obj_pack_object("), "{ir}");
            assert!(!ir.contains("@pycc_ext_obj_pack_int("), "{ir}");
            assert!(!ir.contains("@pycc_ext_obj_pack_str("), "{ir}");
        },
    );
}

#[test]
#[should_panic(expected = "ObjectUnbox narrows to `list[int]`")]
fn a_narrowed_type_no_guard_produces_is_an_internal_error() {
    compile_ext_items_checking_ir(
        "object_unbox_list",
        vec![function(
            "list",
            vec![unboxed("o", Ty::List(Box::new(Ty::Int)))],
        )],
        |_| {},
    );
}
