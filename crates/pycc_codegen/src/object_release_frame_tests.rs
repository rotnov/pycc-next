//! `object` frame slots own their references (`heap_rc`): a parameter is
//! released when the frame exits, a borrowed value stored into a local is
//! retained first, a rebind releases the replaced value after the store,
//! and a returned borrowed value is retained for the caller.

use super::*;

const RETAIN: &str = "call void @pycc_ext_obj_retain(";

fn retains(text: &str) -> usize {
    text.matches(RETAIN).count()
}

fn object_name(name: &str) -> MirExpr {
    MirExpr::Name {
        name: name.to_string(),
        ty: Ty::Object,
    }
}

/// The IR of `def f(o: object) -> <return_ty>: <body>`.
fn param_ir(label: &str, return_ty: Ty, body: Vec<MirStmt>) -> String {
    let item = MirItem::Function {
        name: "f".to_string(),
        params: vec![("o".to_string(), Ty::Object)],
        return_ty,
        body,
    };
    functions_ir(label, vec![item], &["pyfn_f"]).remove(0)
}

/// `x = o; x = o.a`: the first store retains the borrowed parameter, the
/// second moves the produced value in, and each store releases the slot's
/// previous value only after the new one is in place. The frame releases
/// `o` and `x` on its way out.
#[test]
fn a_local_retains_a_borrowed_value_and_releases_the_replaced_one_after_the_store() {
    let ir = param_ir(
        "frame_local_rebind",
        Ty::None,
        vec![
            MirStmt::Assign {
                target: "x".to_string(),
                value: object_name("o"),
            },
            MirStmt::Assign {
                target: "x".to_string(),
                value: attr_of(object_name("o"), "a"),
            },
            MirStmt::Return(None),
        ],
    );
    assert_eq!(retains(&ir), 1, "{ir}");
    let stored = ir
        .find("store ptr %foreign_attr")
        .unwrap_or_else(|| panic!("{ir}"));
    let replaced = ir
        .find(&format!("{RELEASE}ptr %old_heap2)"))
        .unwrap_or_else(|| panic!("{ir}"));
    assert!(stored < replaced, "released before the store: {ir}");
    // The first store's (null) predecessor, the second's, and `o` and `x`
    // in the epilogue.
    assert!(releases(&ir) >= 4, "{ir}");
}

/// `return o` hands the caller a reference of its own: the parameter is
/// retained before the epilogue releases its slot.
#[test]
fn a_returned_parameter_is_retained_for_the_caller() {
    let ir = param_ir(
        "frame_return_param",
        Ty::Object,
        vec![MirStmt::Return(Some(object_name("o")))],
    );
    assert_eq!(retains(&ir), 1, "{ir}");
    let retained = ir.find(RETAIN).unwrap_or_else(|| panic!("{ir}"));
    let released = ir.rfind(RELEASE).unwrap_or_else(|| panic!("{ir}"));
    assert!(retained < released, "{ir}");
}

/// A produced value returned directly is moved to the caller: no retain.
#[test]
fn a_returned_produced_value_is_moved() {
    let ir = param_ir(
        "frame_return_produced",
        Ty::Object,
        vec![MirStmt::Return(Some(attr_of(object_name("o"), "a")))],
    );
    assert_eq!(retains(&ir), 0, "{ir}");
}

/// `len(<name>)` discarded in `f(o)`.
fn len_ir(label: &str, mut body: Vec<MirStmt>, name: &str) -> String {
    body.push(MirStmt::ExprStmt(MirExpr::ObjLen {
        base: Box::new(object_name(name)),
    }));
    body.push(MirStmt::Return(None));
    param_ir(label, Ty::None, body)
}

/// A module global consumed by an operation that runs CPython code is
/// pinned for the operation (`operand_pin`): `__len__` may re-exec the
/// module body and rebind `copy`. It is retained before `len` and released
/// after it, once.
#[test]
fn a_module_global_consumed_by_code_running_operation_is_pinned() {
    let ir = len_ir("frame_pin_global", Vec::new(), "copy");
    assert_eq!(retains(&ir), 1, "{ir}");
    let retained = ir.find(RETAIN).unwrap_or_else(|| panic!("{ir}"));
    let len = ir
        .find("call i32 @pycc_ext_obj_len(")
        .unwrap_or_else(|| panic!("{ir}"));
    let released = ir[len..].find(RELEASE).unwrap_or_else(|| panic!("{ir}"));
    assert!(retained < len, "{ir}");
    assert!(released > 0, "{ir}");
}

/// A frame slot -- a parameter, or a local that shadows a module global --
/// cannot be rebound by the code `len` runs, so it stays borrowed.
#[test]
fn a_frame_slot_consumed_by_code_running_operation_stays_borrowed() {
    let param = len_ir("frame_pin_param", Vec::new(), "o");
    assert_eq!(retains(&param), 0, "{param}");
    let shadow = len_ir(
        "frame_pin_shadow",
        vec![MirStmt::Assign {
            target: "copy".to_string(),
            value: object_name("o"),
        }],
        "copy",
    );
    assert_eq!(retains(&shadow), 1, "only the store retains: {shadow}");
}
