//! `frozenset(...)` lowering to `MirExpr::FrozenSetFrom` (Part 1 of #1319).

use crate::*;
use pycc_hir::{HirExpr, HirItem, HirModule, HirStmt, Ty};

/// `def f(): a = frozenset(<args>)`, preceded by `extra` items.
fn module_with_frozenset_call(args: Vec<HirExpr>, extra: Vec<HirItem>) -> HirModule {
    let mut items = extra;
    items.push(HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![HirStmt::Assign {
            target: "a".to_string(),
            value: HirExpr::Call {
                callee: "frozenset".to_string(),
                args,
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
fn a_frozenset_call_lowers_to_a_frozenset_from_typed_frozenset_int() {
    let mir = build(&module_with_frozenset_call(vec![], vec![]));
    let value = assigned_value(&mir);
    assert_eq!(value.ty(), Ty::FrozenSet(Box::new(Ty::Int)));
    assert_eq!(value, &MirExpr::FrozenSetFrom { source: None });

    let source = HirExpr::SetLiteral(vec![HirExpr::IntLiteral(1)]);
    let mir = build(&module_with_frozenset_call(vec![source], vec![]));
    let MirExpr::FrozenSetFrom {
        source: Some(source),
    } = assigned_value(&mir)
    else {
        panic!("expected a `FrozenSetFrom` with a source");
    };
    assert_eq!(source.ty(), Ty::Set(Box::new(Ty::Int)));
}

/// A user `def frozenset` keeps the ordinary call it has always lowered to.
#[test]
fn a_user_frozenset_function_shadows_the_builtin() {
    let user = HirItem::Function {
        name: "frozenset".to_string(),
        params: vec![("x".to_string(), Ty::Int)],
        return_ty: Ty::Int,
        body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(0)))],
    };
    let mir = build(&module_with_frozenset_call(
        vec![HirExpr::IntLiteral(2)],
        vec![user],
    ));
    let value = assigned_value(&mir);
    assert!(
        matches!(value, MirExpr::Call { callee, .. } if callee == "frozenset"),
        "{value:?}"
    );
    assert_eq!(value.ty(), Ty::Int);
}

/// A walrus inside the argument (`if frozenset([(n := 3)]):`) is a binding the
/// collector reports; a bare `frozenset()` binds nothing. The MIR is
/// hand-built so the test isolates the collector's `FrozenSetFrom` arm.
#[test]
fn a_walrus_in_the_frozenset_argument_is_a_collected_binding() {
    let set_ty = Ty::Set(Box::new(Ty::Int));
    let named = MirExpr::NamedExpr {
        name: "s".to_string(),
        value: Box::new(MirExpr::SetLiteral {
            elements: vec![MirExpr::IntLiteral(1)],
            ops: None,
        }),
        ty: set_ty.clone(),
    };
    let mut out = Vec::new();
    MirExpr::FrozenSetFrom {
        source: Some(Box::new(named)),
    }
    .collect_named_expr_bindings(&mut out);
    assert_eq!(out, vec![("s".to_string(), set_ty)]);

    let mut none = Vec::new();
    MirExpr::FrozenSetFrom { source: None }.collect_named_expr_bindings(&mut none);
    assert!(none.is_empty());
}
