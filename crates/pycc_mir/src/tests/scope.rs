//! Name binding and scope resolution in the crate root.
//!
//! Covers the two-pass item registration `lower_item` performs, plus the
//! `bind`/`bind_variable`/`lookup` scope chain: module globals, function
//! locals shadowing them, parameters, and loop variables.

use crate::*;
use pycc_hir::{BinOpKind, HirExpr, HirItem, HirModule, HirStmt, Ty};

#[test]
fn builds_an_assignment_and_a_later_name_reference() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::IntLiteral(1),
            }),
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
                callee: "print".to_string(),
                args: vec![HirExpr::Name("x".to_string())],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let mir = build(&hir);
    assert_eq!(
        mir.items,
        vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "x".to_string(),
                value: MirExpr::IntLiteral(1),
            }),
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
                callee: "print".to_string(),
                args: vec![MirExpr::Name {
                    name: "x".to_string(),
                    ty: Ty::Int
                }],
                ty: Ty::None,
            })),
        ]
    );
}

#[test]
fn builds_a_function_with_typed_params_and_return() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "add".to_string(),
            params: vec![("a".to_string(), Ty::Int), ("b".to_string(), Ty::Int)],
            return_ty: Ty::Int,
            body: vec![HirStmt::Return(Some(HirExpr::BinOp {
                op: BinOpKind::Add,
                left: Box::new(HirExpr::Name("a".to_string())),
                right: Box::new(HirExpr::Name("b".to_string())),
            }))],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let mir = build(&hir);
    assert_eq!(
        mir.items,
        vec![MirItem::Function {
            name: "add".to_string(),
            params: vec![("a".to_string(), Ty::Int), ("b".to_string(), Ty::Int)],
            return_ty: Ty::Int,
            body: vec![MirStmt::Return(Some(MirExpr::BinOp {
                op: BinOpKind::Add,
                left: Box::new(MirExpr::Name {
                    name: "a".to_string(),
                    ty: Ty::Int
                }),
                right: Box::new(MirExpr::Name {
                    name: "b".to_string(),
                    ty: Ty::Int
                }),
                ty: Ty::Int,
            }))],
        }]
    );
}

#[test]
fn a_top_level_call_to_a_function_defined_later_resolves_via_two_pass_registration() {
    // Exercises `build`'s first pass directly: `helper`'s signature must
    // be registered before the top-level call to it is lowered, even
    // though `helper`'s `HirItem::Function` comes *after* the call in
    // `hir.items` -- exactly the forward-reference case D-038/D-039
    // already fixed on the `pycc_types::check` side.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
                callee: "helper".to_string(),
                args: vec![],
            })),
            HirItem::Function {
                name: "helper".to_string(),
                params: vec![],
                return_ty: Ty::Int,
                body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(1)))],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let mir = build(&hir);
    assert_eq!(
        mir.items[0],
        MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
            callee: "helper".to_string(),
            args: vec![],
            ty: Ty::Int,
        }))
    );
}

#[test]
fn a_function_can_call_itself_recursively_and_resolves_its_own_return_type() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "fact".to_string(),
            params: vec![("n".to_string(), Ty::Int)],
            return_ty: Ty::Int,
            body: vec![HirStmt::Return(Some(HirExpr::Call {
                callee: "fact".to_string(),
                args: vec![HirExpr::Name("n".to_string())],
            }))],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let mir = build(&hir);
    assert_eq!(
        mir.items,
        vec![MirItem::Function {
            name: "fact".to_string(),
            params: vec![("n".to_string(), Ty::Int)],
            return_ty: Ty::Int,
            body: vec![MirStmt::Return(Some(MirExpr::Call {
                callee: "fact".to_string(),
                args: vec![MirExpr::Name {
                    name: "n".to_string(),
                    ty: Ty::Int
                }],
                ty: Ty::Int,
            }))],
        }]
    );
}

#[test]
fn a_function_resolves_a_module_global_assigned_after_its_definition() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "read_x".to_string(),
                params: vec![],
                return_ty: Ty::Int,
                body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
            },
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::IntLiteral(5),
            }),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let mir = build(&hir);
    assert_eq!(
        mir.items[0],
        MirItem::Function {
            name: "read_x".to_string(),
            params: vec![],
            return_ty: Ty::Int,
            body: vec![MirStmt::Return(Some(MirExpr::Name {
                name: "x".to_string(),
                ty: Ty::Int,
            }))],
        }
    );
}

#[test]
fn assigning_bool_to_an_existing_int_binding_preserves_its_mir_type() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::IntLiteral(1),
            }),
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::BoolLiteral(true),
            }),
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Name("x".to_string()))),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let mir = build(&hir);
    assert_eq!(
        mir.items[2],
        MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Name {
            name: "x".to_string(),
            ty: Ty::Int,
        }))
    );
}

#[test]
#[should_panic(expected = "has no recorded type")]
fn a_top_level_read_still_cannot_resolve_a_later_assignment() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Name("x".to_string()))),
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::IntLiteral(1),
            }),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    build(&hir);
}

#[test]
#[should_panic(expected = "has no recorded type")]
fn referencing_an_unbound_name_panics_with_an_internal_error() {
    // By construction (see this module's doc comment / D-057 discussion
    // in the task brief), every `Ty` reaching `pycc_mir` is already
    // concrete and every name already resolved by `pycc_types::check`
    // -- this HIR could never come from a real `check_and_resolve`
    // success, but the panic path itself still needs direct coverage.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Name(
            "undefined".to_string(),
        )))],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    build(&hir);
}

#[test]
fn a_function_local_shadowing_a_module_global_of_a_different_type_resolves_its_own_type() {
    // D-055: a function that assigns a name anywhere in its body
    // classifies that name as local for the *entire* body, independent
    // of any same-named module global -- Python scoping, not a
    // control-flow-sensitive fact. `x` is a module-level `str` here;
    // `f`'s own `x = 5; return x` must resolve to `f`'s own fresh
    // `Ty::Int`, never falling through to the module global's `Ty::Str`.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::StringLiteral("hello".to_string()),
            }),
            HirItem::Function {
                name: "f".to_string(),
                params: vec![],
                return_ty: Ty::Int,
                body: vec![
                    HirStmt::Assign {
                        target: "x".to_string(),
                        value: HirExpr::IntLiteral(5),
                    },
                    HirStmt::Return(Some(HirExpr::Name("x".to_string()))),
                ],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let mir = build(&hir);
    assert_eq!(
        mir.items[1],
        MirItem::Function {
            name: "f".to_string(),
            params: vec![],
            return_ty: Ty::Int,
            body: vec![
                MirStmt::Assign {
                    target: "x".to_string(),
                    value: MirExpr::IntLiteral(5)
                },
                MirStmt::Return(Some(MirExpr::Name {
                    name: "x".to_string(),
                    ty: Ty::Int
                })),
            ],
        }
    );
}

#[test]
fn a_sibling_function_after_a_shadowing_function_still_reads_the_unshadowed_global() {
    // `lower_item` pushes and later pops an isolated function scope;
    // lowering one function's shadowing assignment must not mutate the
    // module scope seen by a later sibling function.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::StringLiteral("hello".to_string()),
            }),
            HirItem::Function {
                name: "shadows".to_string(),
                params: vec![],
                return_ty: Ty::Int,
                body: vec![
                    HirStmt::Assign {
                        target: "x".to_string(),
                        value: HirExpr::IntLiteral(5),
                    },
                    HirStmt::Return(Some(HirExpr::Name("x".to_string()))),
                ],
            },
            HirItem::Function {
                name: "reads_global".to_string(),
                params: vec![],
                return_ty: Ty::Str,
                body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let mir = build(&hir);
    assert_eq!(
        mir.items[2],
        MirItem::Function {
            name: "reads_global".to_string(),
            params: vec![],
            return_ty: Ty::Str,
            body: vec![MirStmt::Return(Some(MirExpr::Name {
                name: "x".to_string(),
                ty: Ty::Str
            }))],
        }
    );
}

#[test]
fn a_function_parameter_shadowing_a_module_global_resolves_its_own_type() {
    // Parameters are part of D-055's lexical-local list too: a
    // parameter named the same as a module global must resolve to the
    // parameter's own type, never fall through to the global's.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::StringLiteral("hello".to_string()),
            }),
            HirItem::Function {
                name: "f".to_string(),
                params: vec![("x".to_string(), Ty::Int)],
                return_ty: Ty::Int,
                body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let mir = build(&hir);
    assert_eq!(
        mir.items[1],
        MirItem::Function {
            name: "f".to_string(),
            params: vec![("x".to_string(), Ty::Int)],
            return_ty: Ty::Int,
            body: vec![MirStmt::Return(Some(MirExpr::Name {
                name: "x".to_string(),
                ty: Ty::Int
            }))],
        }
    );
}

#[test]
fn a_for_range_variable_shadowing_a_module_global_resolves_its_own_type() {
    // `ForRange`'s loop variable is also part of D-055's lexical-local
    // list (it's a binding form, matching Python's own `for`-target
    // classification), so it must shadow a same-named module global too.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "i".to_string(),
                value: HirExpr::StringLiteral("hello".to_string()),
            }),
            HirItem::Function {
                name: "f".to_string(),
                params: vec![],
                return_ty: Ty::Int,
                body: vec![
                    HirStmt::ForRange {
                        var: "i".to_string(),
                        start: HirExpr::IntLiteral(0),
                        stop: HirExpr::IntLiteral(3),
                        step: HirExpr::IntLiteral(1),
                        body: vec![],
                    },
                    HirStmt::Return(Some(HirExpr::Name("i".to_string()))),
                ],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let mir = build(&hir);
    assert_eq!(
        mir.items[1],
        MirItem::Function {
            name: "f".to_string(),
            params: vec![],
            return_ty: Ty::Int,
            body: vec![
                MirStmt::ForRange {
                    var: "i".to_string(),
                    start: MirExpr::IntLiteral(0),
                    stop: MirExpr::IntLiteral(3),
                    step: MirExpr::IntLiteral(1),
                    body: vec![],
                },
                MirStmt::Return(Some(MirExpr::Name {
                    name: "i".to_string(),
                    ty: Ty::Int
                })),
            ],
        }
    );
}

#[test]
fn a_local_first_assigned_inside_nested_if_and_else_bodies_shadows_a_module_global() {
    // Exercises `lower_stmt` recursing into both `body` and `orelse` --
    // D-055 classifies a name as
    // function-local even when its only assignment is nested inside a
    // conditional, not just when it appears directly in the body.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::StringLiteral("hello".to_string()),
            }),
            HirItem::Function {
                name: "f".to_string(),
                params: vec![],
                return_ty: Ty::Int,
                body: vec![
                    HirStmt::If {
                        test: HirExpr::BoolLiteral(true),
                        body: vec![HirStmt::Assign {
                            target: "x".to_string(),
                            value: HirExpr::IntLiteral(1),
                        }],
                        orelse: vec![HirStmt::Assign {
                            target: "x".to_string(),
                            value: HirExpr::IntLiteral(2),
                        }],
                    },
                    HirStmt::Return(Some(HirExpr::Name("x".to_string()))),
                ],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let mir = build(&hir);
    assert_eq!(
        mir.items[1],
        MirItem::Function {
            name: "f".to_string(),
            params: vec![],
            return_ty: Ty::Int,
            body: vec![
                MirStmt::If {
                    test: MirExpr::BoolLiteral(true),
                    body: vec![MirStmt::Assign {
                        target: "x".to_string(),
                        value: MirExpr::IntLiteral(1)
                    }],
                    orelse: vec![MirStmt::Assign {
                        target: "x".to_string(),
                        value: MirExpr::IntLiteral(2)
                    }],
                },
                MirStmt::Return(Some(MirExpr::Name {
                    name: "x".to_string(),
                    ty: Ty::Int
                })),
            ],
        }
    );
}

#[test]
fn a_local_first_assigned_inside_a_while_body_shadows_a_module_global() {
    // Exercises `lower_stmt` recursing into a `While` body.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::StringLiteral("hello".to_string()),
            }),
            HirItem::Function {
                name: "f".to_string(),
                params: vec![],
                return_ty: Ty::Int,
                body: vec![
                    HirStmt::While {
                        test: HirExpr::BoolLiteral(false),
                        body: vec![HirStmt::Assign {
                            target: "x".to_string(),
                            value: HirExpr::IntLiteral(1),
                        }],
                    },
                    HirStmt::Return(Some(HirExpr::Name("x".to_string()))),
                ],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let mir = build(&hir);
    assert_eq!(
        mir.items[1],
        MirItem::Function {
            name: "f".to_string(),
            params: vec![],
            return_ty: Ty::Int,
            body: vec![
                MirStmt::While {
                    test: MirExpr::BoolLiteral(false),
                    body: vec![MirStmt::Assign {
                        target: "x".to_string(),
                        value: MirExpr::IntLiteral(1)
                    }],
                },
                MirStmt::Return(Some(MirExpr::Name {
                    name: "x".to_string(),
                    ty: Ty::Int
                })),
            ],
        }
    );
}

#[test]
fn a_local_first_assigned_inside_a_for_range_body_shadows_a_module_global() {
    // Exercises `lower_stmt` recursing into a `ForRange` body (distinct from the loop variable
    // itself, already covered by
    // `a_for_range_variable_shadowing_a_module_global_resolves_its_own_type`).
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::StringLiteral("hello".to_string()),
            }),
            HirItem::Function {
                name: "f".to_string(),
                params: vec![],
                return_ty: Ty::Int,
                body: vec![
                    HirStmt::ForRange {
                        var: "loop_i".to_string(),
                        start: HirExpr::IntLiteral(0),
                        stop: HirExpr::IntLiteral(3),
                        step: HirExpr::IntLiteral(1),
                        body: vec![HirStmt::Assign {
                            target: "x".to_string(),
                            value: HirExpr::IntLiteral(1),
                        }],
                    },
                    HirStmt::Return(Some(HirExpr::Name("x".to_string()))),
                ],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let mir = build(&hir);
    assert_eq!(
        mir.items[1],
        MirItem::Function {
            name: "f".to_string(),
            params: vec![],
            return_ty: Ty::Int,
            body: vec![
                MirStmt::ForRange {
                    var: "loop_i".to_string(),
                    start: MirExpr::IntLiteral(0),
                    stop: MirExpr::IntLiteral(3),
                    step: MirExpr::IntLiteral(1),
                    body: vec![MirStmt::Assign {
                        target: "x".to_string(),
                        value: MirExpr::IntLiteral(1)
                    }],
                },
                MirStmt::Return(Some(MirExpr::Name {
                    name: "x".to_string(),
                    ty: Ty::Int
                })),
            ],
        }
    );
}
