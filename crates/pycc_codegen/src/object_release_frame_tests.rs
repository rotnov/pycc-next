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

/// `x = o; x = copy.a`: the first store retains the borrowed parameter, the
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
                value: attr("a"),
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
        vec![MirStmt::Return(Some(attr("a")))],
    );
    assert_eq!(retains(&ir), 0, "{ir}");
}
