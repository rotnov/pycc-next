//! `isinstance`/`issubclass` checks and same-class instance comparisons.

use super::*;

// -----------------------------------------------------------------------
// #435: isinstance/issubclass type checker unit tests
// -----------------------------------------------------------------------

#[test]
fn isinstance_with_float_target_type_checks_as_bool() {
    // `isinstance(1.5, float)` — covers the `Ty::Float` arm of
    // `eval_isinstance_single` at the type-checker level (the type
    // checker only validates and returns `Ty::Bool`; the MIR computes
    // the constant). This also exercises `check_isinstance` with a
    // builtin type target.
    let hir = static_class_module(vec![top_level(HirStmt::ExprStmt(HirExpr::Call {
        callee: "print".to_string(),
        args: vec![HirExpr::Call {
            callee: "isinstance".to_string(),
            args: vec![
                HirExpr::FloatLiteral(1.5),
                HirExpr::Name("float".to_string()),
            ],
        }],
    }))]);
    let result = check(&hir);
    assert!(result.is_ok(), "isinstance(1.5, float) should type-check");
}

#[test]
fn isinstance_with_wrong_arg_count_is_t0021() {
    // `isinstance(1)` — only 1 argument. Covers the `args.len() != 2`
    // error branch in `check_isinstance`.
    let hir = static_class_module(vec![top_level(HirStmt::ExprStmt(HirExpr::Call {
        callee: "print".to_string(),
        args: vec![HirExpr::Call {
            callee: "isinstance".to_string(),
            args: vec![HirExpr::IntLiteral(1)],
        }],
    }))]);
    let diagnostic = check(&hir).unwrap_err();
    assert_eq!(diagnostic.code, "T0021");
}

#[test]
fn isinstance_with_non_class_second_arg_is_t0021() {
    // `isinstance(1, 5)` — the second argument is not a class name or
    // tuple of class names. Covers the `extract_class_names` error
    // branch in `check_isinstance`.
    let hir = static_class_module(vec![top_level(HirStmt::ExprStmt(HirExpr::Call {
        callee: "print".to_string(),
        args: vec![HirExpr::Call {
            callee: "isinstance".to_string(),
            args: vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(5)],
        }],
    }))]);
    let diagnostic = check(&hir).unwrap_err();
    assert_eq!(diagnostic.code, "T0021");
}

#[test]
fn issubclass_with_wrong_arg_count_is_t0021() {
    // `issubclass(int)` — only 1 argument. Covers the `args.len() != 2`
    // error branch in `check_issubclass`.
    let hir = static_class_module(vec![top_level(HirStmt::ExprStmt(HirExpr::Call {
        callee: "print".to_string(),
        args: vec![HirExpr::Call {
            callee: "issubclass".to_string(),
            args: vec![HirExpr::Name("int".to_string())],
        }],
    }))]);
    let diagnostic = check(&hir).unwrap_err();
    assert_eq!(diagnostic.code, "T0021");
}

#[test]
fn isinstance_with_undefined_name_in_first_arg_is_t0021() {
    // `isinstance(undefined_name, int)` — the first argument references
    // an undefined name, so `infer_expr_in` fails with T0021 before
    // the class argument is even examined. This covers the `?` error
    // branch on the `infer_expr_in` call in `check_isinstance`.
    let hir = static_class_module(vec![top_level(HirStmt::ExprStmt(HirExpr::Call {
        callee: "print".to_string(),
        args: vec![HirExpr::Call {
            callee: "isinstance".to_string(),
            args: vec![
                HirExpr::Name("undefined_name".to_string()),
                HirExpr::Name("int".to_string()),
            ],
        }],
    }))]);
    let diagnostic = check(&hir).unwrap_err();
    assert_eq!(diagnostic.code, "T0021");
}

#[test]
fn issubclass_with_unknown_class_in_first_arg_is_t0001() {
    // `issubclass(UnknownClass, int)` — the first argument is a class
    // name that is neither a user-defined class nor a builtin type.
    // This covers the `validate_class_name` error path for the first
    // argument in `check_issubclass`.
    let hir = static_class_module(vec![top_level(HirStmt::ExprStmt(HirExpr::Call {
        callee: "print".to_string(),
        args: vec![HirExpr::Call {
            callee: "issubclass".to_string(),
            args: vec![
                HirExpr::Name("UnknownClass".to_string()),
                HirExpr::Name("int".to_string()),
            ],
        }],
    }))]);
    let diagnostic = check(&hir).unwrap_err();
    assert_eq!(diagnostic.code, "T0001");
}

#[test]
fn issubclass_with_non_class_second_arg_is_t0021() {
    // `issubclass(C, 5)` — the second argument is not a class name or
    // tuple of class names. This exercises the `extract_class_names`
    // error path in `check_issubclass`.
    let hir = static_class_module(vec![top_level(HirStmt::ExprStmt(HirExpr::Call {
        callee: "print".to_string(),
        args: vec![HirExpr::Call {
            callee: "issubclass".to_string(),
            args: vec![HirExpr::Name("C".to_string()), HirExpr::IntLiteral(5)],
        }],
    }))]);
    let diagnostic = check(&hir).unwrap_err();
    assert_eq!(diagnostic.code, "T0021");
}

#[test]
fn issubclass_with_int_str_same_type_returns_bool() {
    // `issubclass(int, int)` and `issubclass(str, str)` — covers the
    // `return cls == target_class` line in `eval_issubclass_single`
    // at the type-checker level.
    let hir = static_class_module(vec![top_level(HirStmt::ExprStmt(HirExpr::Call {
        callee: "print".to_string(),
        args: vec![HirExpr::Call {
            callee: "issubclass".to_string(),
            args: vec![
                HirExpr::Name("int".to_string()),
                HirExpr::Name("int".to_string()),
            ],
        }],
    }))]);
    let result = check(&hir);
    assert!(result.is_ok(), "issubclass(int, int) should type-check");
}

#[test]
fn isinstance_with_unknown_class_in_second_arg_is_t0001() {
    // `isinstance(1, UnknownClass)` — the second argument is a valid
    // name expression but not a registered class. Covers the
    // `validate_class_name` error path in `check_isinstance`.
    let hir = static_class_module(vec![top_level(HirStmt::ExprStmt(HirExpr::Call {
        callee: "print".to_string(),
        args: vec![HirExpr::Call {
            callee: "isinstance".to_string(),
            args: vec![
                HirExpr::IntLiteral(1),
                HirExpr::Name("UnknownClass".to_string()),
            ],
        }],
    }))]);
    let diagnostic = check(&hir).unwrap_err();
    assert_eq!(diagnostic.code, "T0001");
}

#[test]
fn issubclass_with_unknown_class_in_second_arg_is_t0001() {
    // `issubclass(int, UnknownClass)` — the second argument is a valid
    // name expression but not a registered class. Covers the
    // `validate_class_name` error path for target classes in
    // `check_issubclass`.
    let hir = static_class_module(vec![top_level(HirStmt::ExprStmt(HirExpr::Call {
        callee: "print".to_string(),
        args: vec![HirExpr::Call {
            callee: "issubclass".to_string(),
            args: vec![
                HirExpr::Name("int".to_string()),
                HirExpr::Name("UnknownClass".to_string()),
            ],
        }],
    }))]);
    let diagnostic = check(&hir).unwrap_err();
    assert_eq!(diagnostic.code, "T0001");
}

#[test]
fn issubclass_with_non_name_first_arg_is_t0021() {
    // `issubclass(5, int)` — the first argument is not a bare class
    // name. Covers the `_ =>` error branch in `check_issubclass`'s
    // first-argument match.
    let hir = static_class_module(vec![top_level(HirStmt::ExprStmt(HirExpr::Call {
        callee: "print".to_string(),
        args: vec![HirExpr::Call {
            callee: "issubclass".to_string(),
            args: vec![HirExpr::IntLiteral(5), HirExpr::Name("int".to_string())],
        }],
    }))]);
    let diagnostic = check(&hir).unwrap_err();
    assert_eq!(diagnostic.code, "T0021");
}

#[test]
fn isinstance_with_call_first_arg_is_c0001() {
    // #435 review fix (P1): `isinstance(D(), D)` — a call expression as
    // the first argument is rejected with C0001 because pycc's
    // compile-time `isinstance` would silently discard the call's side
    // effects. Covers the `if let HirExpr::Call { .. }` branch in
    // `check_isinstance`.
    let hir = static_class_module(vec![top_level(HirStmt::ExprStmt(HirExpr::Call {
        callee: "print".to_string(),
        args: vec![HirExpr::Call {
            callee: "isinstance".to_string(),
            args: vec![
                HirExpr::Call {
                    callee: "D".to_string(),
                    args: vec![],
                },
                HirExpr::Name("D".to_string()),
            ],
        }],
    }))]);
    let diagnostic = check(&hir).unwrap_err();
    assert_eq!(diagnostic.code, "C0001");
    assert!(diagnostic.message.contains("isinstance"));
}

// -- #378 (PR-18): dataclass __eq__ comparison type checking ---------

#[test]
fn a_same_class_instance_comparison_without_eq_is_t0021() {
    // `Point` has `__init__` and `bump` but no `__eq__` method.
    // Comparing two `Point` instances with `==` should fall through
    // the `has_eq` check and produce T0021 (cannot compare instances).
    // This covers the `}` (merge point) of the `if has_eq` block in
    // `infer_expr_in`'s `Compare` arm.
    let hir = point_module(vec![
        top_level(HirStmt::Assign {
            target: "p".to_string(),
            value: HirExpr::Call {
                callee: "Point".to_string(),
                args: vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)],
            },
        }),
        top_level(HirStmt::Assign {
            target: "q".to_string(),
            value: HirExpr::Call {
                callee: "Point".to_string(),
                args: vec![HirExpr::IntLiteral(3), HirExpr::IntLiteral(4)],
            },
        }),
        top_level(HirStmt::ExprStmt(HirExpr::Call {
            callee: "print".to_string(),
            args: vec![HirExpr::Compare {
                op: pycc_hir::CmpOpKind::Eq,
                left: Box::new(HirExpr::Name("p".to_string())),
                right: Box::new(HirExpr::Name("q".to_string())),
            }],
        })),
    ]);
    let diagnostic = check(&hir).unwrap_err();
    assert_eq!(diagnostic.code, "T0021");
}

#[test]
fn a_same_class_dataclass_instance_comparison_with_eq_type_checks() {
    // A dataclass class with a compiler-synthesized `__eq__` method
    // accepts `==` between same-class instances. This covers the
    // `return Ok(Ty::Bool)` path in the dataclass comparison check.
    // Non-dataclass classes with a user-defined `__eq__` are rejected
    // (the MIR rewrite assumes the synthesized signature).
    let self_ty = Ty::Instance(Box::new("EqPoint".to_string()));
    let init = HirItem::Function {
        name: "EqPoint.__init__".to_string(),
        params: vec![
            ("self".to_string(), self_ty.clone()),
            ("x".to_string(), Ty::Int),
        ],
        return_ty: Ty::None,
        body: vec![
            HirStmt::AttrSet {
                base: HirExpr::Name("self".to_string()),
                attr: "x".to_string(),
                value: HirExpr::Name("x".to_string()),
            },
            HirStmt::Return(None),
        ],
    };
    let eq = HirItem::Function {
        name: "EqPoint.__eq__".to_string(),
        params: vec![
            ("self".to_string(), self_ty.clone()),
            ("other".to_string(), self_ty.clone()),
        ],
        return_ty: Ty::Bool,
        body: vec![HirStmt::Return(Some(HirExpr::BoolLiteral(true)))],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            init,
            eq,
            top_level(HirStmt::Assign {
                target: "p".to_string(),
                value: HirExpr::Call {
                    callee: "EqPoint".to_string(),
                    args: vec![HirExpr::IntLiteral(1)],
                },
            }),
            top_level(HirStmt::Assign {
                target: "q".to_string(),
                value: HirExpr::Call {
                    callee: "EqPoint".to_string(),
                    args: vec![HirExpr::IntLiteral(2)],
                },
            }),
            top_level(HirStmt::ExprStmt(HirExpr::Call {
                callee: "print".to_string(),
                args: vec![HirExpr::Compare {
                    op: pycc_hir::CmpOpKind::Eq,
                    left: Box::new(HirExpr::Name("p".to_string())),
                    right: Box::new(HirExpr::Name("q".to_string())),
                }],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: vec![(
            "EqPoint".to_string(),
            pycc_hir::HirClassDef {
                class_attrs: Vec::new(),
                exception_type_tag: None,
                name: "EqPoint".to_string(),
                bases: Vec::new(),
                mro: vec!["EqPoint".to_string()],
                attrs: vec![("x".to_string(), Ty::Int)],
                methods: vec![
                    ("__init__".to_string(), "EqPoint.__init__".to_string()),
                    ("__eq__".to_string(), "EqPoint.__eq__".to_string()),
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
                dataclass_fields: vec![("x".to_string(), Ty::Int)],
                is_protocol: false,
                runtime_checkable: false,
                protocol_members: Vec::new(),
                abstract_methods: Vec::new(),
                is_abstract: false,
            },
        )],
    };
    check(&hir).expect("a same-class dataclass comparison with __eq__ should type-check");
}

#[test]
fn a_non_dataclass_instance_comparison_with_user_eq_is_t0021() {
    // A non-dataclass class with a user-defined `__eq__` method does
    // NOT accept `==` between same-class instances -- the MIR rewrite
    // for `__eq__` is restricted to dataclass classes (whose
    // synthesized `__eq__` has a known-correct signature), so the type
    // checker must reject this to prevent a codegen panic.
    let self_ty = Ty::Instance(Box::new("PlainEq".to_string()));
    let init = HirItem::Function {
        name: "PlainEq.__init__".to_string(),
        params: vec![
            ("self".to_string(), self_ty.clone()),
            ("x".to_string(), Ty::Int),
        ],
        return_ty: Ty::None,
        body: vec![
            HirStmt::AttrSet {
                base: HirExpr::Name("self".to_string()),
                attr: "x".to_string(),
                value: HirExpr::Name("x".to_string()),
            },
            HirStmt::Return(None),
        ],
    };
    let eq = HirItem::Function {
        name: "PlainEq.__eq__".to_string(),
        params: vec![
            ("self".to_string(), self_ty.clone()),
            ("other".to_string(), self_ty.clone()),
        ],
        return_ty: Ty::Bool,
        body: vec![HirStmt::Return(Some(HirExpr::BoolLiteral(true)))],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            init,
            eq,
            top_level(HirStmt::Assign {
                target: "p".to_string(),
                value: HirExpr::Call {
                    callee: "PlainEq".to_string(),
                    args: vec![HirExpr::IntLiteral(1)],
                },
            }),
            top_level(HirStmt::Assign {
                target: "q".to_string(),
                value: HirExpr::Call {
                    callee: "PlainEq".to_string(),
                    args: vec![HirExpr::IntLiteral(2)],
                },
            }),
            top_level(HirStmt::ExprStmt(HirExpr::Call {
                callee: "print".to_string(),
                args: vec![HirExpr::Compare {
                    op: pycc_hir::CmpOpKind::Eq,
                    left: Box::new(HirExpr::Name("p".to_string())),
                    right: Box::new(HirExpr::Name("q".to_string())),
                }],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: vec![(
            "PlainEq".to_string(),
            pycc_hir::HirClassDef {
                class_attrs: Vec::new(),
                exception_type_tag: None,
                name: "PlainEq".to_string(),
                bases: Vec::new(),
                mro: vec!["PlainEq".to_string()],
                attrs: vec![("x".to_string(), Ty::Int)],
                methods: vec![
                    ("__init__".to_string(), "PlainEq.__init__".to_string()),
                    ("__eq__".to_string(), "PlainEq.__eq__".to_string()),
                ],
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
    let diagnostic = check(&hir).unwrap_err();
    assert_eq!(diagnostic.code, "T0021");
}
