//! Dataclass `__eq__` and `__repr__` MIR rewrites.
//!
//! Covers `==`/`!=` between instances rewriting to an `__eq__` call, the
//! fall-through to `MirExpr::Compare` when no `__eq__` exists or the
//! operator is neither, and `print` of an instance with `__repr__`.

use crate::*;
use pycc_hir::{CmpOpKind, HirClassDef, HirExpr, HirItem, HirModule, HirStmt, Ty};

// -- #378 (PR-18): dataclass __eq__ / __repr__ MIR rewrites -----------

/// A minimal dataclass-like `Point` module with `__init__`, `__eq__`,
/// and `__repr__` methods registered in the class definition. Used by
/// the dataclass MIR-rewrite tests below.
fn dataclass_point_module(extra_items: Vec<HirItem>) -> HirModule {
    let self_ty = Ty::Instance(Box::new("Point".to_string()));
    let init = HirItem::Function {
        name: "Point.__init__".to_string(),
        params: vec![
            ("self".to_string(), self_ty.clone()),
            ("x".to_string(), Ty::Int),
            ("y".to_string(), Ty::Int),
        ],
        return_ty: Ty::None,
        body: vec![
            HirStmt::AttrSet {
                base: HirExpr::Name("self".to_string()),
                attr: "x".to_string(),
                value: HirExpr::Name("x".to_string()),
            },
            HirStmt::AttrSet {
                base: HirExpr::Name("self".to_string()),
                attr: "y".to_string(),
                value: HirExpr::Name("y".to_string()),
            },
            HirStmt::Return(None),
        ],
    };
    let eq = HirItem::Function {
        name: "Point.__eq__".to_string(),
        params: vec![
            ("self".to_string(), self_ty.clone()),
            ("other".to_string(), self_ty.clone()),
        ],
        return_ty: Ty::Bool,
        body: vec![HirStmt::Return(Some(HirExpr::BoolLiteral(true)))],
    };
    let repr = HirItem::Function {
        name: "Point.__repr__".to_string(),
        params: vec![("self".to_string(), self_ty.clone())],
        return_ty: Ty::Str,
        body: vec![HirStmt::Return(Some(HirExpr::StringLiteral(
            "Point()".to_string(),
        )))],
    };
    let mut items = vec![init, eq, repr];
    items.extend(extra_items);
    HirModule {
        seeded_builtin_exception_classes: false,
        items,
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: vec![(
            "Point".to_string(),
            HirClassDef {
                class_attrs: Vec::new(),
                exception_type_tag: None,
                name: "Point".to_string(),
                bases: Vec::new(),
                mro: vec!["Point".to_string()],
                attrs: vec![("x".to_string(), Ty::Int), ("y".to_string(), Ty::Int)],
                methods: vec![
                    ("__init__".to_string(), "Point.__init__".to_string()),
                    ("__eq__".to_string(), "Point.__eq__".to_string()),
                    ("__repr__".to_string(), "Point.__repr__".to_string()),
                ],
                type_param: None,
                properties: Vec::new(),
                static_methods: Vec::new(),
                class_methods: Vec::new(),
                is_enum: false,
                implicit_object_init: false,
                method_defaults: Vec::new(),
                enum_members: Vec::new(),
                is_dataclass: true,
                dataclass_fields: vec![("x".to_string(), Ty::Int), ("y".to_string(), Ty::Int)],
                is_protocol: false,
                runtime_checkable: false,
                protocol_members: Vec::new(),
                abstract_methods: Vec::new(),
                is_abstract: false,
            },
        )],
    }
}

#[test]
fn an_eq_comparison_between_same_class_instances_lowers_to_eq_call() {
    let hir = dataclass_point_module(vec![
        HirItem::TopLevelStmt(HirStmt::Assign {
            target: "p".to_string(),
            value: HirExpr::Call {
                callee: "Point".to_string(),
                args: vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)],
            },
        }),
        HirItem::TopLevelStmt(HirStmt::Assign {
            target: "q".to_string(),
            value: HirExpr::Call {
                callee: "Point".to_string(),
                args: vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)],
            },
        }),
        HirItem::TopLevelStmt(HirStmt::Assign {
            target: "r".to_string(),
            value: HirExpr::Compare {
                op: CmpOpKind::Eq,
                left: Box::new(HirExpr::Name("p".to_string())),
                right: Box::new(HirExpr::Name("q".to_string())),
            },
        }),
    ]);
    let mir = build(&hir);
    // Items: [__init__, __eq__, __repr__, Assign p, Assign q, Assign r].
    // The comparison is at index 5. Using `matches!` with a guard
    // avoids a hand-written `other => panic!(...)` arm that would be
    // permanently uncovered under D-014's 100%-line-coverage gate
    // (the happy path always matches the expected pattern).
    assert!(matches!(
        &mir.items[5],
        MirItem::TopLevelStmt(MirStmt::Assign {
            value: MirExpr::Call { callee, .. },
            ..
        }) if callee == "Point.__eq__"
    ));
}

#[test]
fn a_neq_comparison_between_same_class_instances_lowers_to_neq_of_eq_call() {
    let hir = dataclass_point_module(vec![
        HirItem::TopLevelStmt(HirStmt::Assign {
            target: "p".to_string(),
            value: HirExpr::Call {
                callee: "Point".to_string(),
                args: vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)],
            },
        }),
        HirItem::TopLevelStmt(HirStmt::Assign {
            target: "q".to_string(),
            value: HirExpr::Call {
                callee: "Point".to_string(),
                args: vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)],
            },
        }),
        HirItem::TopLevelStmt(HirStmt::Assign {
            target: "r".to_string(),
            value: HirExpr::Compare {
                op: CmpOpKind::NotEq,
                left: Box::new(HirExpr::Name("p".to_string())),
                right: Box::new(HirExpr::Name("q".to_string())),
            },
        }),
    ]);
    let mir = build(&hir);
    // The NotEq arm wraps the __eq__ call in a Compare with True.
    assert!(matches!(
        &mir.items[5],
        MirItem::TopLevelStmt(MirStmt::Assign {
            value: MirExpr::Compare {
                op: CmpOpKind::NotEq,
                left,
                right,
                ..
            },
            ..
        }) if matches!(left.as_ref(), MirExpr::Call { callee, .. } if callee == "Point.__eq__")
            && matches!(right.as_ref(), MirExpr::BoolLiteral(true))
    ));
}

#[test]
fn a_non_eq_neq_comparison_between_instances_with_eq_falls_through_to_mir_compare() {
    // A non-Eq/NotEq comparison (`<`, `<=`, `>`, `>=`) between same-class
    // instances with `__eq__` falls through to the default
    // `MirExpr::Compare` rather than being rewritten to an `__eq__` call.
    // The `matches!(op, Eq | NotEq)` guard at the top of the rewrite
    // block prevents entry for other operators. The type checker would
    // reject `<` between class instances (T0021), but the MIR lowering
    // is tested directly here without going through the type checker.
    let hir = dataclass_point_module(vec![
        HirItem::TopLevelStmt(HirStmt::Assign {
            target: "p".to_string(),
            value: HirExpr::Call {
                callee: "Point".to_string(),
                args: vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)],
            },
        }),
        HirItem::TopLevelStmt(HirStmt::Assign {
            target: "q".to_string(),
            value: HirExpr::Call {
                callee: "Point".to_string(),
                args: vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)],
            },
        }),
        HirItem::TopLevelStmt(HirStmt::Assign {
            target: "r".to_string(),
            value: HirExpr::Compare {
                op: CmpOpKind::Lt,
                left: Box::new(HirExpr::Name("p".to_string())),
                right: Box::new(HirExpr::Name("q".to_string())),
            },
        }),
    ]);
    let mir = build(&hir);
    // The `Lt` comparison should NOT be rewritten to an `__eq__` call;
    // it should remain a `MirExpr::Compare` with the original operator.
    assert!(matches!(
        &mir.items[5],
        MirItem::TopLevelStmt(MirStmt::Assign {
            value: MirExpr::Compare { op, .. },
            ..
        }) if *op == CmpOpKind::Lt
    ));
}

#[test]
fn an_eq_comparison_between_instances_without_eq_falls_through_to_mir_compare() {
    // When the class has no `__eq__` method in its MRO, the `if let
    // Some(eq_mangled)` block is not entered, and the comparison falls
    // through to the default `MirExpr::Compare`. This covers the `}`
    // (merge point) of the `if let Some(eq_mangled)` block.
    let self_ty = Ty::Instance(Box::new("Plain".to_string()));
    let init = HirItem::Function {
        name: "Plain.__init__".to_string(),
        params: vec![("self".to_string(), self_ty.clone())],
        return_ty: Ty::None,
        body: vec![HirStmt::Return(None)],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            init,
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "p".to_string(),
                value: HirExpr::Call {
                    callee: "Plain".to_string(),
                    args: vec![],
                },
            }),
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "q".to_string(),
                value: HirExpr::Call {
                    callee: "Plain".to_string(),
                    args: vec![],
                },
            }),
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "r".to_string(),
                value: HirExpr::Compare {
                    op: CmpOpKind::Eq,
                    left: Box::new(HirExpr::Name("p".to_string())),
                    right: Box::new(HirExpr::Name("q".to_string())),
                },
            }),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: vec![(
            "Plain".to_string(),
            HirClassDef {
                class_attrs: Vec::new(),
                exception_type_tag: None,
                name: "Plain".to_string(),
                bases: Vec::new(),
                mro: vec!["Plain".to_string()],
                attrs: Vec::new(),
                methods: vec![("__init__".to_string(), "Plain.__init__".to_string())],
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
    };
    let mir = build(&hir);
    // The last item should be a MirExpr::Compare (not a __eq__ call).
    assert!(matches!(
        &mir.items[3],
        MirItem::TopLevelStmt(MirStmt::Assign {
            value: MirExpr::Compare { op, .. },
            ..
        }) if *op == CmpOpKind::Eq
    ));
}

#[test]
fn print_of_an_instance_with_an_unregistered_class_passes_through() {
    // `rewrite_instance_to_repr` returns `expr.clone()` when the
    // instance's class is not in the `classes` map. This test creates
    // a function with a parameter typed as `Ghost` (an unregistered
    // class), then prints it inside the function body. The MIR should
    // pass the instance through without rewriting (the codegen would
    // panic later, but we only test the MIR lowering here).
    let ghost_ty = Ty::Instance(Box::new("Ghost".to_string()));
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "test".to_string(),
            params: vec![("g".to_string(), ghost_ty.clone())],
            return_ty: Ty::None,
            body: vec![HirStmt::ExprStmt(HirExpr::Call {
                callee: "print".to_string(),
                args: vec![HirExpr::Name("g".to_string())],
            })],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let mir = build(&hir);
    // The function body's print argument should be a MirExpr::Name
    // with the Ghost instance type (not rewritten to a __repr__ call).
    // Using nested `matches!` avoids hand-written `other => panic!(...)`
    // arms that would be permanently uncovered under D-014's coverage gate.
    assert!(matches!(
        &mir.items[0],
        MirItem::Function { body, .. }
            if body.len() == 1
                && matches!(
                    &body[0],
                    MirStmt::ExprStmt(MirExpr::Call { args, .. })
                        if args.len() == 1 && args[0].ty() == ghost_ty
                )
    ));
}

#[test]
fn print_of_an_instance_with_repr_lowers_to_repr_call() {
    let hir = dataclass_point_module(vec![
        HirItem::TopLevelStmt(HirStmt::Assign {
            target: "p".to_string(),
            value: HirExpr::Call {
                callee: "Point".to_string(),
                args: vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)],
            },
        }),
        HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
            callee: "print".to_string(),
            args: vec![HirExpr::Name("p".to_string())],
        })),
    ]);
    let mir = build(&hir);
    // Items: [__init__, __eq__, __repr__, Assign p, ExprStmt(print)].
    // The print is at index 4. The argument should be a MirExpr::Call
    // to Point.__repr__.
    assert!(matches!(
        &mir.items[4],
        MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
            callee,
            args,
            ..
        })) if callee == "print"
              && args.len() == 1
              && matches!(
                  &args[0],
                  MirExpr::Call { callee: repr_callee, .. }
                      if repr_callee == "Point.__repr__"
              )
    ));
}
