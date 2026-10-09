//! `type(self)(...)` (#1411): the internal-error panics of the
//! `HirExpr::ReceiverClassCall` arm. `pycc_types` refuses both programs
//! before MIR, so they are reachable only from a hand-built HIR module.

use crate::*;
use pycc_hir::{HirExpr, HirItem, HirModule, HirStmt, Ty};

/// A module holding one function whose `self` parameter has type
/// `self_ty` and whose body returns `type(self)()`. No class is defined.
fn module_with_receiver(self_ty: Ty) -> HirModule {
    HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "f".to_string(),
            params: vec![("self".to_string(), self_ty)],
            return_ty: Ty::None,
            body: vec![HirStmt::ExprStmt(HirExpr::ReceiverClassCall {
                args: Vec::new(),
            })],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    }
}

#[test]
#[should_panic(expected = "`type(self)(...)` outside an instance method")]
fn a_receiver_class_call_whose_self_is_not_an_instance_panics() {
    let _ = build(&module_with_receiver(Ty::Int));
}

#[test]
#[should_panic(expected = "`self`'s class `Ghost` is not a known class")]
fn a_receiver_class_call_whose_self_class_is_unknown_panics() {
    let _ = build(&module_with_receiver(Ty::Instance(Box::new(
        "Ghost".to_string(),
    ))));
}
