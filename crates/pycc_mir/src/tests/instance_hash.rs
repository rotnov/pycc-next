//! `hash(instance)` lowering to `MirExpr::InstanceHash` (#1335, Part 1 of
//! #1332).

use crate::*;
use pycc_hir::{HirClassDef, HirExpr, HirItem, HirModule, HirStmt, Ty};

fn s(text: &str) -> String {
    text.to_string()
}

/// `class R` with a no-argument `__init__` and, when `with_hash`, a
/// `def __hash__(self) -> int`; then `def f(): a = hash(R())`, preceded by
/// `extra` items.
fn module(with_hash: bool, extra: Vec<HirItem>) -> HirModule {
    let self_ty = Ty::Instance(Box::new(s("R")));
    let mut items = vec![HirItem::Function {
        name: s("R.__init__"),
        params: vec![(s("self"), self_ty.clone())],
        return_ty: Ty::None,
        body: vec![HirStmt::Return(None)],
    }];
    let mut methods = vec![(s("__init__"), s("R.__init__"))];
    if with_hash {
        items.push(HirItem::Function {
            name: s("R.__hash__"),
            params: vec![(s("self"), self_ty)],
            return_ty: Ty::Int,
            body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(7)))],
        });
        methods.push((s("__hash__"), s("R.__hash__")));
    }
    items.extend(extra);
    items.push(HirItem::Function {
        name: s("f"),
        params: vec![],
        return_ty: Ty::None,
        body: vec![HirStmt::Assign {
            target: s("a"),
            value: HirExpr::Call {
                callee: s("hash"),
                args: vec![HirExpr::Call {
                    callee: s("R"),
                    args: vec![],
                }],
            },
        }],
    });
    HirModule {
        seeded_builtin_exception_classes: false,
        items,
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: vec![(
            s("R"),
            HirClassDef {
                class_attrs: Vec::new(),
                exception_type_tag: None,
                name: s("R"),
                bases: Vec::new(),
                mro: vec![s("R")],
                attrs: Vec::new(),
                methods,
                type_param: None,
                properties: Vec::new(),
                static_methods: Vec::new(),
                class_methods: Vec::new(),
                is_enum: false,
                implicit_object_init: false,
                method_defaults: Vec::new(),
                enum_members: Vec::new(),
                is_dataclass: false,
                dataclass_fields: Vec::new(),
                is_protocol: false,
                runtime_checkable: false,
                protocol_members: Vec::new(),
                abstract_methods: Vec::new(),
                is_abstract: false,
            },
        )],
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
fn a_class_without_hash_or_eq_hashes_the_instance_by_identity() {
    let mir = build(&module(false, vec![]));
    let value = assigned_value(&mir);
    assert_eq!(value.ty(), Ty::Int);
    let MirExpr::InstanceHash { operand, via } = value else {
        panic!("expected an `InstanceHash`, got {value:?}");
    };
    assert_eq!(*via, InstanceHashVia::Identity);
    assert_eq!(operand.ty(), Ty::Instance(Box::new(s("R"))));
}

#[test]
fn a_user_hash_method_lowers_to_a_call_with_the_instance_as_self() {
    let mir = build(&module(true, vec![]));
    let value = assigned_value(&mir);
    assert_eq!(value.ty(), Ty::Int);
    let MirExpr::InstanceHash { operand, via } = value else {
        panic!("expected an `InstanceHash`, got {value:?}");
    };
    assert_eq!(*via, InstanceHashVia::Method);
    let MirExpr::Call { callee, args, ty } = operand.as_ref() else {
        panic!("expected the method call, got {operand:?}");
    };
    assert_eq!(callee, "R.__hash__");
    assert_eq!(*ty, Ty::Int);
    let [instance] = args.as_slice() else {
        panic!("expected `self` only");
    };
    assert_eq!(instance.ty(), Ty::Instance(Box::new(s("R"))));
}

/// A user `def hash` keeps the ordinary call it has always lowered to.
#[test]
fn a_user_function_named_hash_is_not_intercepted() {
    let user_hash = HirItem::Function {
        name: s("hash"),
        params: vec![(s("x"), Ty::Instance(Box::new(s("R"))))],
        return_ty: Ty::Int,
        body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(0)))],
    };
    let mir = build(&module(false, vec![user_hash]));
    assert!(matches!(
        assigned_value(&mir),
        MirExpr::Call { callee, .. } if callee == "hash"
    ));
}

#[test]
fn a_named_expression_operand_surfaces_its_binding() {
    let r_ty = Ty::Instance(Box::new("R".to_string()));
    let named = MirExpr::NamedExpr {
        name: "r".to_string(),
        value: Box::new(MirExpr::Name {
            name: "q".to_string(),
            ty: r_ty.clone(),
        }),
        ty: r_ty.clone(),
    };
    let mut out = Vec::new();
    MirExpr::InstanceHash {
        operand: Box::new(named),
        via: InstanceHashVia::Identity,
    }
    .collect_named_expr_bindings(&mut out);
    assert_eq!(out, vec![("r".to_string(), r_ty)]);
}
