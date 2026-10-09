//! The element ops of a set of user-class instances (#1343, Part 1 of
//! #1336): `SetLiteral`/`SetAdd` carry them, `ForSet` carries the element
//! type, and `FrozenSetFrom` keeps its source's element type.

use crate::*;
use pycc_hir::{HirClassDef, HirExpr, HirItem, HirModule, HirStmt, Ty};

fn s(text: &str) -> String {
    text.to_string()
}

fn r_ty() -> Ty {
    Ty::Instance(Box::new(s("R")))
}

fn new_r() -> HirExpr {
    HirExpr::Call {
        callee: s("R"),
        args: vec![],
    }
}

/// `class R` with a no-argument `__init__`, optionally `__hash__` returning
/// `hash_ret` and `def __eq__(self, other: R) -> bool`; then
/// `def f(): s = {R(), R()}; s.add(R()); for v in s: v; t = frozenset(s)`.
fn module(hash_ret: Option<Ty>, with_eq: bool) -> HirModule {
    let method =
        |name: &str, params: Vec<(String, Ty)>, return_ty: Ty, value: HirExpr| HirItem::Function {
            name: s(name),
            params,
            return_ty,
            body: vec![HirStmt::Return(Some(value))],
        };
    let mut items = vec![HirItem::Function {
        name: s("R.__init__"),
        params: vec![(s("self"), r_ty())],
        return_ty: Ty::None,
        body: vec![HirStmt::Return(None)],
    }];
    let mut methods = vec![(s("__init__"), s("R.__init__"))];
    if let Some(ret) = hash_ret {
        items.push(method(
            "R.__hash__",
            vec![(s("self"), r_ty())],
            ret,
            HirExpr::BoolLiteral(true),
        ));
        methods.push((s("__hash__"), s("R.__hash__")));
    }
    if with_eq {
        items.push(method(
            "R.__eq__",
            vec![(s("self"), r_ty()), (s("other"), r_ty())],
            Ty::Bool,
            HirExpr::BoolLiteral(true),
        ));
        methods.push((s("__eq__"), s("R.__eq__")));
    }
    items.push(HirItem::Function {
        name: s("f"),
        params: vec![],
        return_ty: Ty::None,
        body: vec![
            HirStmt::Assign {
                target: s("s"),
                value: HirExpr::SetLiteral(vec![new_r(), new_r()]),
            },
            HirStmt::ExprStmt(HirExpr::SetAdd {
                set: s("s"),
                value: Box::new(new_r()),
            }),
            HirStmt::ForList {
                var: s("v"),
                list: s("s"),
                body: vec![HirStmt::ExprStmt(HirExpr::Name(s("v")))],
            },
            HirStmt::Assign {
                target: s("t"),
                value: HirExpr::Call {
                    callee: s("frozenset"),
                    args: vec![HirExpr::Name(s("s"))],
                },
            },
        ],
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

/// `f`'s four statements, lowered.
fn f_body(module: &MirModule) -> &[MirStmt] {
    let Some(MirItem::Function { body, .. }) = module.items.last() else {
        panic!("expected the module's last item to be a function");
    };
    body
}

/// The ops on the literal and on `.add`, which must agree.
fn ops_of(body: &[MirStmt]) -> Option<SetElementOps> {
    let MirStmt::Assign {
        value: MirExpr::SetLiteral { ops, .. },
        ..
    } = &body[0]
    else {
        panic!("expected the set literal, got {:?}", body[0]);
    };
    let MirStmt::ExprStmt(MirExpr::SetAdd { ops: add_ops, .. }) = &body[1] else {
        panic!("expected `.add`, got {:?}", body[1]);
    };
    assert_eq!(ops, add_ops);
    ops.clone().map(|ops| *ops)
}

#[test]
fn a_class_without_hash_or_eq_inserts_by_identity() {
    let mir = build(&module(None, false));
    assert_eq!(
        ops_of(f_body(&mir)),
        Some(SetElementOps {
            hash: SetHashOp::Identity,
            eq: SetEqOp::Identity,
        })
    );
}

#[test]
fn user_hash_and_eq_methods_are_carried_with_the_hash_return_type() {
    for ret in [Ty::Int, Ty::Bool] {
        let mir = build(&module(Some(ret.clone()), true));
        assert_eq!(
            ops_of(f_body(&mir)),
            Some(SetElementOps {
                hash: SetHashOp::Method {
                    callee: s("R.__hash__"),
                    ret,
                },
                eq: SetEqOp::Method {
                    callee: s("R.__eq__"),
                },
            })
        );
    }
}

#[test]
fn iteration_binds_the_instance_and_frozenset_keeps_the_element_type() {
    let mir = build(&module(None, false));
    let body = f_body(&mir);
    let MirStmt::ForSet { var_ty, .. } = &body[2] else {
        panic!("expected `ForSet`, got {:?}", body[2]);
    };
    assert_eq!(*var_ty, r_ty());
    let MirStmt::Assign { value, .. } = &body[3] else {
        panic!("expected the frozenset assignment, got {:?}", body[3]);
    };
    assert_eq!(value.ty(), Ty::FrozenSet(Box::new(r_ty())));
}

#[test]
fn an_int_set_carries_no_ops() {
    let int_literal = HirExpr::SetLiteral(vec![HirExpr::IntLiteral(1)]);
    let module = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: s("f"),
            params: vec![],
            return_ty: Ty::None,
            body: vec![
                HirStmt::Assign {
                    target: s("s"),
                    value: int_literal,
                },
                HirStmt::ExprStmt(HirExpr::SetAdd {
                    set: s("s"),
                    value: Box::new(HirExpr::IntLiteral(2)),
                }),
            ],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let mir = build(&module);
    assert_eq!(ops_of(f_body(&mir)), None);
}

#[test]
#[should_panic(expected = "pycc_types admits only a compiled set element `__eq__`")]
fn an_uncompiled_eq_verdict_is_an_internal_error() {
    // A subclass of a class with a user `__eq__` is C0001 in `pycc_types`.
    let mut module = module(Some(Ty::Int), true);
    let mut subclass = module.class_defs[0].1.clone();
    subclass.name = s("S");
    subclass.bases = vec![s("R")];
    subclass.mro = vec![s("S"), s("R")];
    subclass.methods = Vec::new();
    module.class_defs.push((s("S"), subclass));
    build(&module);
}

#[test]
#[should_panic(expected = "pycc_types admits `.add()` only on a set")]
fn add_on_a_non_set_is_an_internal_error() {
    let module = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: s("f"),
            params: vec![(s("s"), Ty::Int)],
            return_ty: Ty::None,
            body: vec![HirStmt::ExprStmt(HirExpr::SetAdd {
                set: s("s"),
                value: Box::new(HirExpr::IntLiteral(2)),
            })],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    build(&module);
}

/// Both set-comprehension forms over a `set[R]` carry the same element ops
/// as a literal of `R` (#1344), so codegen inserts through D-255's
/// hash-once, identity-then-`__eq__` probe.
#[test]
fn a_set_comprehension_of_instances_carries_the_element_ops() {
    let mut module = module(Some(Ty::Int), true);
    let Some(HirItem::Function { body, .. }) = module.items.last_mut() else {
        panic!("expected `f` last");
    };
    body.truncate(1);
    let v = || HirExpr::Name(s("v"));
    body.push(HirStmt::SetCompAssign {
        target: s("t"),
        var: s("v"),
        iter: pycc_hir::CompIter::Name(s("s")),
        cond: None,
        elt: Box::new(v()),
    });
    body.push(HirStmt::ExprStmt(HirExpr::Comprehension(Box::new(
        pycc_hir::HirComprehension {
            var: s("v"),
            iter: pycc_hir::CompIter::Name(s("s")),
            cond: None,
            elt: pycc_hir::CompElt::Set(v()),
        },
    ))));
    let mir = build(&module);
    let body = f_body(&mir);
    let MirStmt::Assign {
        value: MirExpr::SetLiteral { ops: literal, .. },
        ..
    } = &body[0]
    else {
        panic!("expected the set literal, got {:?}", body[0]);
    };
    assert!(literal.is_some());
    let MirStmt::SetCompAssign { ops, var_ty, .. } = &body[1] else {
        panic!(
            "expected the set comprehension statement, got {:?}",
            body[1]
        );
    };
    assert_eq!(ops, literal);
    assert_eq!(*var_ty, r_ty());
    let MirStmt::ExprStmt(expr @ MirExpr::Comprehension(comp)) = &body[2] else {
        panic!(
            "expected the set comprehension expression, got {:?}",
            body[2]
        );
    };
    let MirCompElt::Set(elt, expr_ops) = &comp.elt else {
        panic!("expected a set element, got {:?}", comp.elt);
    };
    assert_eq!(expr_ops, literal);
    assert_eq!(elt.ty(), r_ty());
    assert_eq!(expr.ty(), Ty::Set(Box::new(r_ty())));
}
