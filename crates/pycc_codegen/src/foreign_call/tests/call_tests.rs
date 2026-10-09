//! #1313: a direct call of a CPython object, `MirExpr::ObjCall`.
//!
//! Kept out of the parent file, which is already past the ~1,000-line
//! threshold; `use super::*` reaches its private `entry_ir` helper.

use super::*;

/// `from itertools import product` followed by one discarded
/// `product(args)` call.
fn direct_call(args: Vec<MirExpr>) -> Vec<MirItem> {
    vec![
        MirItem::ForeignImport {
            local_name: "product".to_string(),
            module_path: "itertools".to_string(),
            from: Some(pycc_mir::FromImport {
                name: "product".to_string(),
                fromlist: vec!["product".to_string()],
                index: 0,
                level: 0,
            }),
        },
        MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::ObjCall {
            callee: Box::new(MirExpr::Name {
                name: "product".to_string(),
                ty: Ty::Object,
            }),
            args,
        })),
    ]
}

/// The callee is the loaded module global itself, handed to the borrowing
/// entry point: no method lookup is emitted, and the consuming
/// `pycc_ext_obj_call` (whose name is a prefix of the borrowing one, hence
/// the `(` in the pattern) is never called, since it would steal the
/// global's only reference.
#[test]
fn a_direct_call_reaches_the_borrowing_shim_with_the_loaded_global() {
    let ir = entry_ir("foreign_direct_call_zero_arg", direct_call(Vec::new()));
    assert!(
        ir.contains(&format!("@{EXT_OBJ_CALL_BORROWED_SYMBOL}(")),
        "{ir}"
    );
    assert!(!ir.contains(&format!("@{EXT_OBJ_CALL_SYMBOL}(")), "{ir}");
    assert!(!ir.contains(EXT_OBJ_GETATTR_SYMBOL), "{ir}");
    assert!(!ir.contains(EXT_OBJ_PACK_INT_SYMBOL), "{ir}");
    assert!(
        ir.contains("i64 0"),
        "the arity is passed as a constant: {ir}"
    );
    assert!(
        ir.contains("%load = load ptr, ptr @pyglobal_product"),
        "the callee is the module global: {ir}"
    );
    assert!(
        ir.contains(&format!("@{EXT_OBJ_CALL_BORROWED_SYMBOL}(ptr %load,")),
        "the loaded global is the call's first operand: {ir}"
    );
}

/// Each admitted scalar argument is packed into the argument array before
/// the call, in order.
#[test]
fn a_direct_call_packs_each_argument() {
    let ir = entry_ir(
        "foreign_direct_call_args",
        direct_call(vec![
            MirExpr::IntLiteral(7),
            MirExpr::StringLiteral("ab".to_string()),
        ]),
    );
    assert!(ir.contains(EXT_OBJ_PACK_INT_SYMBOL), "{ir}");
    assert!(ir.contains(EXT_OBJ_PACK_STR_SYMBOL), "{ir}");
    assert!(ir.contains("foreign_call_arg_slot"), "{ir}");
    assert!(ir.contains("i64 2"), "the arity is two: {ir}");
    let pack_at = ir.find(EXT_OBJ_PACK_STR_SYMBOL).expect("asserted above");
    let call_at = ir
        .find(&format!("call ptr @{EXT_OBJ_CALL_BORROWED_SYMBOL}("))
        .unwrap_or_else(|| panic!("no borrowing call was emitted: {ir}"));
    assert!(pack_at < call_at, "{ir}");
}

/// A `NULL` result stops the module body on the module-exec failure edge,
/// as every other foreign producer does.
#[test]
fn a_failed_direct_call_returns_on_the_module_exec_failure_edge() {
    let ir = entry_ir(
        "foreign_direct_call_fail_edge",
        direct_call(vec![MirExpr::IntLiteral(1)]),
    );
    assert!(ir.contains("foreign_call_failed"), "{ir}");
    assert!(ir.contains("foreign_call_fail:"), "{ir}");
    assert!(ir.contains("foreign_call_cont:"), "{ir}");
    assert!(
        ir.contains(&format!("ret i64 {EXT_MODULE_EXEC_FAILED}")),
        "{ir}"
    );
}

/// A direct call runs arbitrary CPython code, so it can leave an exception
/// set.
#[test]
fn a_direct_call_can_set_an_exception() {
    let node = MirExpr::ObjCall {
        callee: Box::new(MirExpr::Name {
            name: "product".to_string(),
            ty: Ty::Object,
        }),
        args: Vec::new(),
    };
    assert!(crate::exception::expression_can_set_exception(&node));
}

/// The defensive arm in `foreign_attr::expect_object_pointer`, reached
/// through a direct call: `pycc_mir` builds `ObjCall` only over a
/// `Ty::Object` callee. The panic text is shared by every foreign-object
/// operation and deliberately names no node.
#[test]
#[should_panic(expected = "did not evaluate to a CPython object")]
fn a_non_object_callee_is_an_internal_error() {
    entry_ir(
        "foreign_direct_call_bad_callee",
        vec![MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::ObjCall {
            callee: Box::new(MirExpr::IntLiteral(1)),
            args: Vec::new(),
        }))],
    );
}

/// `product` as a loaded module global.
fn product() -> MirExpr {
    MirExpr::Name {
        name: "product".to_string(),
        ty: Ty::Object,
    }
}

/// `from itertools import product` followed by one discarded
/// `callee(args)` call with an arbitrary callee (Part 2a of #1371).
fn computed_call(callee: MirExpr, args: Vec<MirExpr>) -> Vec<MirItem> {
    let mut items = direct_call(Vec::new());
    items[1] = MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::ObjCall {
        callee: Box::new(callee),
        args,
    }));
    items
}

/// Part 2a of #1371: a subscript result is a new reference the shim
/// produced, so the call goes to the *consuming* entry point, which
/// releases it -- the borrowing one would leak it on every call.
#[test]
fn a_subscript_callee_reaches_the_consuming_shim() {
    let callee = MirExpr::ObjSubscript {
        base: Box::new(product()),
        index: Box::new(MirExpr::StringLiteral("k".to_string())),
    };
    let ir = entry_ir(
        "foreign_subscript_callee",
        computed_call(callee, vec![MirExpr::IntLiteral(1)]),
    );
    assert!(ir.contains(EXT_OBJ_GETITEM_SYMBOL), "{ir}");
    assert!(ir.contains(&format!("@{EXT_OBJ_CALL_SYMBOL}(")), "{ir}");
    assert!(
        !ir.contains(&format!("@{EXT_OBJ_CALL_BORROWED_SYMBOL}(")),
        "{ir}"
    );
    let getitem_at = ir.find(EXT_OBJ_GETITEM_SYMBOL).expect("asserted above");
    let pack_at = ir
        .find(EXT_OBJ_PACK_INT_SYMBOL)
        .expect("the argument is packed");
    assert!(getitem_at < pack_at, "the callee is evaluated first: {ir}");
}

/// A second `object` argument is packed with `pycc_ext_obj_pack_object`,
/// which takes the new reference the call array consumes.
#[test]
fn an_object_argument_is_packed_with_the_object_packer() {
    let ir = entry_ir("foreign_object_argument", direct_call(vec![product()]));
    assert!(ir.contains(EXT_OBJ_PACK_OBJECT_SYMBOL), "{ir}");
}

/// #1435: a class-instance argument is packed with
/// `pycc_ext_obj_pack_instance`, which takes the instance pointer alone --
/// the run-time class comes from its layout descriptor, not from the
/// argument's static type -- and returns the carrier reference the call
/// array consumes.
#[test]
fn an_instance_argument_is_packed_with_the_instance_packer() {
    let ir = entry_ir(
        "foreign_instance_argument",
        direct_call(vec![MirExpr::NullInstance {
            ty: Ty::Instance(Box::new("Q".to_string())),
        }]),
    );
    assert!(
        ir.contains(&format!(
            "call ptr @{EXT_OBJ_PACK_INSTANCE_SYMBOL}(ptr null)"
        )),
        "{ir}"
    );
}

/// The routing allowlist: exactly the shim's own new-reference producers
/// are consumed; a name read, a scalar and every other node are borrowed.
///
/// The pycc-side calls matter most among the borrowed ones: a pycc
/// `__class_getitem__` (`Reg["x"]`), and any pycc function or method
/// returning `object`, lowers to `MirExpr::Call` and hands back a borrowed
/// pointer (`docs/RUNTIME.md`), so consuming it would underflow the
/// refcount. A pycc instance's `object` slot read (`MirExpr::AttrGet`) is
/// borrowed the same way.
#[test]
fn only_shim_producers_are_consumed_callees() {
    let produced = [
        MirExpr::ObjSubscript {
            base: Box::new(product()),
            index: Box::new(MirExpr::IntLiteral(0)),
        },
        MirExpr::ObjAttrGet {
            base: Box::new(product()),
            attr: "a".to_string(),
            ty: Ty::Object,
        },
        MirExpr::ObjMethodCall {
            base: Box::new(product()),
            method: "m".to_string(),
            args: Vec::new(),
        },
        MirExpr::ObjCall {
            callee: Box::new(product()),
            args: Vec::new(),
        },
    ];
    for callee in &produced {
        assert!(callee_is_produced(callee), "{callee:?}");
    }
    let borrowed = [
        product(),
        MirExpr::IntLiteral(1),
        MirExpr::Call {
            callee: "Reg.__class_getitem__.static".to_string(),
            args: vec![MirExpr::StringLiteral("x".to_string())],
            ty: Ty::Object,
        },
        MirExpr::AttrGet {
            base: Box::new(MirExpr::Name {
                name: "self".to_string(),
                ty: Ty::Instance(Box::new("C".to_string())),
            }),
            slot: 0,
            ty: Ty::Object,
        },
    ];
    for callee in borrowed {
        assert!(!callee_is_produced(&callee), "{callee:?}");
    }
}

/// A call of a call result (`f()(x)`) consumes the inner call's result,
/// while the inner call still borrows the global.
#[test]
fn a_call_result_callee_reaches_the_consuming_shim() {
    let inner = MirExpr::ObjCall {
        callee: Box::new(product()),
        args: Vec::new(),
    };
    let ir = entry_ir(
        "foreign_call_result_callee",
        computed_call(inner, Vec::new()),
    );
    assert!(
        ir.contains(&format!("@{EXT_OBJ_CALL_BORROWED_SYMBOL}(ptr %load")),
        "{ir}"
    );
    assert!(ir.contains(&format!("@{EXT_OBJ_CALL_SYMBOL}(")), "{ir}");
}
