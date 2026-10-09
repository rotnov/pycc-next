//! `hash(...)` typing in MIR (#1331, Part 1 of #1327).

use crate::*;
use pycc_hir::{HirExpr, HirItem, HirModule, HirStmt, Ty};

/// `def f(): a = hash(5)`, preceded by `extra` items.
fn module_with_hash_call(extra: Vec<HirItem>) -> HirModule {
    let mut items = extra;
    items.push(HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![HirStmt::Assign {
            target: "a".to_string(),
            value: HirExpr::Call {
                callee: "hash".to_string(),
                args: vec![HirExpr::IntLiteral(5)],
            },
        }],
    });
    HirModule {
        seeded_builtin_exception_classes: false,
        items,
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    }
}

/// The value assigned in the last item, which is `f`.
fn assigned_value(module: &MirModule) -> &MirExpr {
    let Some(MirItem::Function { body, .. }) = module.items.last() else {
        panic!("expected the module's last item to be a function");
    };
    let [MirStmt::Assign { value, .. }] = body.as_slice() else {
        panic!("expected a single assignment");
    };
    value
}

#[test]
fn the_hash_builtin_stays_a_call_typed_int() {
    let mir = build(&module_with_hash_call(vec![]));
    let value = assigned_value(&mir);
    assert!(
        matches!(value, MirExpr::Call { callee, .. } if callee == "hash"),
        "{value:?}"
    );
    assert_eq!(value.ty(), Ty::Int);
}

/// A user `def hash` keeps its own declared return type.
#[test]
fn a_user_hash_function_shadows_the_builtin() {
    let user = HirItem::Function {
        name: "hash".to_string(),
        params: vec![("x".to_string(), Ty::Int)],
        return_ty: Ty::Str,
        body: vec![HirStmt::Return(Some(HirExpr::StringLiteral(
            "h".to_string(),
        )))],
    };
    let mir = build(&module_with_hash_call(vec![user]));
    assert_eq!(assigned_value(&mir).ty(), Ty::Str);
}
