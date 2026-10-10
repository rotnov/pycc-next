//! `@property` getters and setters.

use super::*;

// -- @property type checking (#377) -------------------------------------

/// Builds a `Box` class with a read-write `@property` `val` backed by
/// the `_val` slot, plus `extra_items`/`extra_stmts` for each test's
/// own exercise. The getter returns `self._val` (int); the setter
/// accepts an `int` and stores it.
fn property_module(extra_items: Vec<HirItem>) -> HirModule {
    use pycc_hir::PropertyDef;
    let self_ty = Ty::Instance(Box::new("Box".to_string()));
    let init = HirItem::Function {
        name: "Box.__init__".to_string(),
        params: vec![("self".to_string(), self_ty.clone())],
        return_ty: Ty::None,
        body: vec![
            HirStmt::AttrSet {
                base: HirExpr::Name("self".to_string()),
                attr: "_val".to_string(),
                value: HirExpr::IntLiteral(0),
            },
            HirStmt::Return(None),
        ],
    };
    let getter = HirItem::Function {
        name: "Box.val".to_string(),
        params: vec![("self".to_string(), self_ty.clone())],
        return_ty: Ty::Int,
        body: vec![HirStmt::Return(Some(HirExpr::AttrGet {
            base: Box::new(HirExpr::Name("self".to_string())),
            attr: "_val".to_string(),
        }))],
    };
    let setter = HirItem::Function {
        name: "Box.val.setter".to_string(),
        params: vec![
            ("self".to_string(), self_ty.clone()),
            ("v".to_string(), Ty::Int),
        ],
        return_ty: Ty::None,
        body: vec![
            HirStmt::AttrSet {
                base: HirExpr::Name("self".to_string()),
                attr: "_val".to_string(),
                value: HirExpr::Name("v".to_string()),
            },
            HirStmt::Return(None),
        ],
    };
    let mut items = vec![init, getter, setter];
    items.extend(extra_items);
    HirModule {
        seeded_builtin_exception_classes: false,
        items,
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: vec![(
            "Box".to_string(),
            HirClassDef {
                class_attrs: Vec::new(),
                exception_type_tag: None,
                name: "Box".to_string(),
                bases: Vec::new(),
                mro: vec!["Box".to_string()],
                attrs: vec![("_val".to_string(), Ty::Int)],
                methods: vec![("__init__".to_string(), "Box.__init__".to_string())],
                properties: vec![PropertyDef {
                    name: "val".to_string(),
                    getter: "Box.val".to_string(),
                    setter: Some("Box.val.setter".to_string()),
                }],
                type_param: None,
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

/// Like `property_module` but the property has no setter (read-only).
fn read_only_property_module(extra_items: Vec<HirItem>) -> HirModule {
    use pycc_hir::PropertyDef;
    let self_ty = Ty::Instance(Box::new("Box".to_string()));
    let init = HirItem::Function {
        name: "Box.__init__".to_string(),
        params: vec![("self".to_string(), self_ty.clone())],
        return_ty: Ty::None,
        body: vec![
            HirStmt::AttrSet {
                base: HirExpr::Name("self".to_string()),
                attr: "_val".to_string(),
                value: HirExpr::IntLiteral(0),
            },
            HirStmt::Return(None),
        ],
    };
    let getter = HirItem::Function {
        name: "Box.val".to_string(),
        params: vec![("self".to_string(), self_ty)],
        return_ty: Ty::Int,
        body: vec![HirStmt::Return(Some(HirExpr::AttrGet {
            base: Box::new(HirExpr::Name("self".to_string())),
            attr: "_val".to_string(),
        }))],
    };
    let mut items = vec![init, getter];
    items.extend(extra_items);
    HirModule {
        seeded_builtin_exception_classes: false,
        items,
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: vec![(
            "Box".to_string(),
            HirClassDef {
                class_attrs: Vec::new(),
                exception_type_tag: None,
                name: "Box".to_string(),
                bases: Vec::new(),
                mro: vec!["Box".to_string()],
                attrs: vec![("_val".to_string(), Ty::Int)],
                methods: vec![("__init__".to_string(), "Box.__init__".to_string())],
                properties: vec![PropertyDef {
                    name: "val".to_string(),
                    getter: "Box.val".to_string(),
                    setter: None,
                }],
                type_param: None,
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

#[test]
fn a_property_getter_read_type_checks() {
    let hir = property_module(vec![
        top_level(HirStmt::Assign {
            target: "b".to_string(),
            value: HirExpr::Call {
                callee: "Box".to_string(),
                args: vec![],
            },
        }),
        top_level(HirStmt::ExprStmt(HirExpr::Call {
            callee: "print".to_string(),
            args: vec![HirExpr::AttrGet {
                base: Box::new(HirExpr::Name("b".to_string())),
                attr: "val".to_string(),
            }],
        })),
    ]);
    check(&hir).expect("a property getter read should type-check");
}

#[test]
fn a_property_setter_assignment_type_checks() {
    let hir = property_module(vec![
        top_level(HirStmt::Assign {
            target: "b".to_string(),
            value: HirExpr::Call {
                callee: "Box".to_string(),
                args: vec![],
            },
        }),
        top_level(HirStmt::AttrSet {
            base: HirExpr::Name("b".to_string()),
            attr: "val".to_string(),
            value: HirExpr::IntLiteral(42),
        }),
    ]);
    check(&hir).expect("a property setter assignment should type-check");
}

#[test]
fn a_read_only_property_assignment_is_rejected() {
    let hir = read_only_property_module(vec![
        top_level(HirStmt::Assign {
            target: "b".to_string(),
            value: HirExpr::Call {
                callee: "Box".to_string(),
                args: vec![],
            },
        }),
        top_level(HirStmt::AttrSet {
            base: HirExpr::Name("b".to_string()),
            attr: "val".to_string(),
            value: HirExpr::IntLiteral(42),
        }),
    ]);
    let diagnostic = check(&hir).unwrap_err();
    assert_eq!(diagnostic.code, "T0044");
    assert!(
        diagnostic.message.contains("read-only"),
        "unexpected message: {}",
        diagnostic.message
    );
}

#[test]
fn a_property_setter_type_mismatch_is_rejected() {
    let hir = property_module(vec![
        top_level(HirStmt::Assign {
            target: "b".to_string(),
            value: HirExpr::Call {
                callee: "Box".to_string(),
                args: vec![],
            },
        }),
        top_level(HirStmt::AttrSet {
            base: HirExpr::Name("b".to_string()),
            attr: "val".to_string(),
            value: HirExpr::StringLiteral("nope".to_string()),
        }),
    ]);
    let diagnostic = check(&hir).unwrap_err();
    assert_eq!(diagnostic.code, "T0021");
}

#[test]
fn a_property_setter_assignment_with_an_ill_typed_value_propagates_the_value_error() {
    // Exercises `check_attr_set`'s `?` on `infer_expr_in` for the
    // *value* expression (line 228) -- distinct from
    // `a_property_setter_type_mismatch_is_rejected` above, where the
    // value infers successfully (`Ty::Str`) and the rejection happens
    // later at the `is_assignable` check (line 246). Here the value is
    // an undefined name, so `infer_expr_in` itself returns `Err`
    // before the setter's parameter type is ever consulted.
    let hir = property_module(vec![
        top_level(HirStmt::Assign {
            target: "b".to_string(),
            value: HirExpr::Call {
                callee: "Box".to_string(),
                args: vec![],
            },
        }),
        top_level(HirStmt::AttrSet {
            base: HirExpr::Name("b".to_string()),
            attr: "val".to_string(),
            value: HirExpr::Name("undefined_name".to_string()),
        }),
    ]);
    let diagnostic = check(&hir).unwrap_err();
    assert_eq!(diagnostic.code, "T0021");
    assert!(
        diagnostic.message.contains("undefined_name"),
        "unexpected message: {}",
        diagnostic.message
    );
}

#[test]
fn a_property_getter_read_inside_a_method_body_type_checks() {
    // A method that reads the property via `self.val` -- exercises
    // `resolve_attr_get`'s property check from within a function body
    // (pass 3), not just top-level (pass 2).
    let self_ty = Ty::Instance(Box::new("Box".to_string()));
    let reader = HirItem::Function {
        name: "Box.read_val".to_string(),
        params: vec![("self".to_string(), self_ty)],
        return_ty: Ty::Int,
        body: vec![HirStmt::Return(Some(HirExpr::AttrGet {
            base: Box::new(HirExpr::Name("self".to_string())),
            attr: "val".to_string(),
        }))],
    };
    let mut hir = property_module(vec![]);
    // Add the reader method to items and to the class's method table.
    // `.expect(...)`, not `if let Some(...)` -- the latter's implicit
    // else (the no-match arm) is its own hand-written region, never
    // executed because `property_module` always defines `Box` -- this
    // crate's own established coverage-gate convention (see
    // `lower_ok`'s own doc comment in `pycc_hir::class::tests`) is
    // `.expect()`, whose panic path lives in libcore, outside this
    // crate's instrumented regions.
    hir.items.push(reader);
    let (_, cd) = hir
        .class_defs
        .iter_mut()
        .find(|(n, _)| n == "Box")
        .expect("property_module always defines Box");
    cd.methods
        .push(("read_val".to_string(), "Box.read_val".to_string()));
    hir.items.push(top_level(HirStmt::Assign {
        target: "b".to_string(),
        value: HirExpr::Call {
            callee: "Box".to_string(),
            args: vec![],
        },
    }));
    hir.items.push(top_level(HirStmt::ExprStmt(HirExpr::Call {
        callee: "print".to_string(),
        args: vec![HirExpr::MethodCall {
            base: Box::new(HirExpr::Name("b".to_string())),
            method: "read_val".to_string(),
            args: vec![],
        }],
    })));
    check(&hir).expect("a property read inside a method body should type-check");
}

#[test]
fn a_property_setter_assignment_inside_a_method_body_type_checks() {
    // A method that writes the property via `self.val = v` -- exercises
    // `check_attr_set`'s property check from within a function body.
    let self_ty = Ty::Instance(Box::new("Box".to_string()));
    let writer = HirItem::Function {
        name: "Box.write_val".to_string(),
        params: vec![("self".to_string(), self_ty), ("v".to_string(), Ty::Int)],
        return_ty: Ty::None,
        body: vec![
            HirStmt::AttrSet {
                base: HirExpr::Name("self".to_string()),
                attr: "val".to_string(),
                value: HirExpr::Name("v".to_string()),
            },
            HirStmt::Return(None),
        ],
    };
    let mut hir = property_module(vec![]);
    hir.items.push(writer);
    // `.expect(...)`, not `if let Some(...)` -- see the sibling test
    // `a_property_getter_read_inside_a_method_body_type_checks` above
    // for the coverage-gate rationale.
    let (_, cd) = hir
        .class_defs
        .iter_mut()
        .find(|(n, _)| n == "Box")
        .expect("property_module always defines Box");
    cd.methods
        .push(("write_val".to_string(), "Box.write_val".to_string()));
    hir.items.push(top_level(HirStmt::Assign {
        target: "b".to_string(),
        value: HirExpr::Call {
            callee: "Box".to_string(),
            args: vec![],
        },
    }));
    hir.items
        .push(top_level(HirStmt::ExprStmt(HirExpr::MethodCall {
            base: Box::new(HirExpr::Name("b".to_string())),
            method: "write_val".to_string(),
            args: vec![HirExpr::IntLiteral(99)],
        })));
    check(&hir).expect("a property write inside a method body should type-check");
}

#[test]
fn a_property_getter_read_resolves_through_check_and_resolve() {
    // Exercises the full `check_and_resolve` → MIR pipeline for a
    // property getter read, ensuring the MIR lowering's property
    // rewrite (AttrGet → Call) produces valid MIR.
    let hir = property_module(vec![
        top_level(HirStmt::Assign {
            target: "b".to_string(),
            value: HirExpr::Call {
                callee: "Box".to_string(),
                args: vec![],
            },
        }),
        top_level(HirStmt::ExprStmt(HirExpr::Call {
            callee: "print".to_string(),
            args: vec![HirExpr::AttrGet {
                base: Box::new(HirExpr::Name("b".to_string())),
                attr: "val".to_string(),
            }],
        })),
    ]);
    let resolved = check_and_resolve(&hir).expect("check_and_resolve should succeed");
    // Build MIR from the resolved HIR -- this exercises the MIR
    // lowering's property rewrite (AttrGet → MirExpr::Call).
    let _mir = pycc_mir::build(&resolved);
}

#[test]
fn a_property_setter_assignment_resolves_through_check_and_resolve() {
    // Exercises the full `check_and_resolve` → MIR pipeline for a
    // property setter assignment, ensuring the MIR lowering's property
    // rewrite (AttrSet → ExprStmt(Call)) produces valid MIR.
    let hir = property_module(vec![
        top_level(HirStmt::Assign {
            target: "b".to_string(),
            value: HirExpr::Call {
                callee: "Box".to_string(),
                args: vec![],
            },
        }),
        top_level(HirStmt::AttrSet {
            base: HirExpr::Name("b".to_string()),
            attr: "val".to_string(),
            value: HirExpr::IntLiteral(42),
        }),
    ]);
    let resolved = check_and_resolve(&hir).expect("check_and_resolve should succeed");
    let _mir = pycc_mir::build(&resolved);
}
