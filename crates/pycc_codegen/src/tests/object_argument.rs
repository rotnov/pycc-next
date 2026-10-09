//! Part 1 of #1333: passing a CPython object to a pycc-compiled function.
//!
//! `build_call_to_with_leading_args`' `Scalar::Object` argument arm hands
//! the `PyObject *` to the callee as-is: the callee borrows the caller's
//! reference, so the call emits no increment and no release (#1092's
//! leak-only rule). These tests pin the call through real MIR, for a
//! module-level caller and for a function-body caller.

use super::*;

/// `def _keep(o): return None`, with `o` solver-inferred as `object`.
fn keep() -> MirItem {
    MirItem::Function {
        name: "_keep".to_string(),
        params: vec![("o".to_string(), Ty::Object)],
        return_ty: Ty::None,
        body: vec![MirStmt::Return(None)],
    }
}

/// `_keep(numpy.pi)` as a discarded call.
fn call_keep() -> MirStmt {
    MirStmt::ExprStmt(MirExpr::Call {
        callee: "_keep".to_string(),
        args: vec![numpy_pi()],
        ty: Ty::None,
    })
}

#[test]
fn a_cpython_object_argument_is_passed_through_without_refcount_traffic() {
    compile_ext_items_checking_ir(
        "object_argument_module",
        with_foreign_numpy(vec![keep(), MirItem::TopLevelStmt(call_keep())]),
        |ir| {
            // The callee is called through its function-pointer slot, and its one
            // argument is the attribute load's `PyObject *` itself.
            assert!(
                ir.contains("call void %load_fnptr(ptr %foreign_attr)"),
                "{ir}"
            );
            assert!(
                !ir.contains("DecRef"),
                "a passed object is not released: {ir}"
            );
            assert!(!ir.contains("IncRef"), "a passed object is borrowed: {ir}");
        },
    );
}

#[test]
fn a_function_body_may_pass_a_cpython_object_on() {
    compile_ext_items_checking_ir(
        "object_argument_function",
        with_foreign_numpy(vec![
            keep(),
            MirItem::Function {
                name: "f".to_string(),
                params: vec![],
                return_ty: Ty::None,
                body: vec![call_keep(), MirStmt::Return(None)],
            },
        ]),
        |ir| {
            // The callee is called through its function-pointer slot, and its one
            // argument is the attribute load's `PyObject *` itself.
            assert!(
                ir.contains("call void %load_fnptr(ptr %foreign_attr)"),
                "{ir}"
            );
            assert!(
                !ir.contains("DecRef"),
                "a passed object is not released: {ir}"
            );
            assert!(!ir.contains("IncRef"), "a passed object is borrowed: {ir}");
        },
    );
}
