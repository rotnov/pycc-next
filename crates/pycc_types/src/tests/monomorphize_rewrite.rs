//! Monomorphization rewriting generic calls in every expression and statement position.

use super::*;

#[test]
fn check_and_resolve_monomorphizes_a_nested_generic_call_and_drops_the_original() {
    let param = Ty::Param(Box::new("T".to_string()));
    let identity = generic_identity_fn(param.clone(), param);
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            identity,
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
                callee: "print".to_string(),
                args: vec![HirExpr::Call {
                    callee: "identity".to_string(),
                    args: vec![HirExpr::IntLiteral(1)],
                }],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let resolved = check_and_resolve(&hir).unwrap();
    assert!(find_function(&resolved, "identity").is_none());
    assert!(find_function(&resolved, "0gen_identity__T_int").is_some());
    assert_eq!(count_function(&resolved, "0gen_identity__T_int"), 1);
    // Compared against the whole expected statement (not a
    // `let-else { panic!() }` destructure) so the never-taken failure
    // arm was not its own uncovered branch while these tests lived in
    // `lib.rs` -- mirrors the convention described in Task 2's own note
    // on `unreachable!()` in test bodies.
    assert_eq!(
        resolved.items[0],
        HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
            callee: "print".to_string(),
            args: vec![HirExpr::Call {
                callee: "0gen_identity__T_int".to_string(),
                args: vec![HirExpr::IntLiteral(1)],
            }],
        }))
    );
}

#[test]
fn monomorphize_lets_a_non_generic_function_read_a_module_global_defined_later_in_the_file_alongside_a_generic_function()
 {
    // Fix-round regression test (finding #1): `check` (via
    // `check_with_environment_all`'s three-pass D-041 discipline) and
    // `check_and_resolve`/`monomorphize` must agree on validity for a
    // module that mixes a generic function with a non-generic function
    // reading a module-level global assigned *after* that function's
    // own `def` -- Python only evaluates a function body when called,
    // typically after the whole module has already run top to bottom.
    // Before this fix, `monomorphize` walked `hir.items` in a single
    // source-order pass, so `uses_global`'s body was rewritten before
    // the later `g: int = 5` top-level assignment had grown `env`,
    // wrongly reporting `g` as undefined even though `check` (using the
    // correct two-phase order) already accepted the exact same program.
    let param = Ty::Param(Box::new("T".to_string()));
    let identity = generic_identity_fn(param.clone(), param);
    let uses_global = HirItem::Function {
        name: "uses_global".to_string(),
        params: vec![],
        return_ty: Ty::Int,
        body: vec![HirStmt::Return(Some(HirExpr::Name("g".to_string())))],
    };
    let global_assign = HirItem::TopLevelStmt(HirStmt::AnnAssign {
        is_final: false,
        target: "g".to_string(),
        annotation: Ty::Int,
        value: Some(HirExpr::IntLiteral(5)),
    });
    let call_uses_global = HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
        callee: "print".to_string(),
        args: vec![HirExpr::Call {
            callee: "uses_global".to_string(),
            args: vec![],
        }],
    }));
    let call_identity = HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
        callee: "print".to_string(),
        args: vec![HirExpr::Call {
            callee: "identity".to_string(),
            args: vec![HirExpr::IntLiteral(1)],
        }],
    }));
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            identity,
            uses_global.clone(),
            global_assign,
            call_uses_global,
            call_identity,
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    // `check` accepts this module (three-pass discipline lets
    // `uses_global` see `g` regardless of source position).
    assert!(check(&hir).is_ok());
    // `check_and_resolve`/`monomorphize` must accept it too, and must
    // not have dropped or mis-rewritten the non-generic function.
    let resolved = check_and_resolve(&hir).unwrap();
    assert_eq!(find_function(&resolved, "uses_global"), Some(&uses_global));
    assert_eq!(count_function(&resolved, "0gen_identity__T_int"), 1);
}

#[test]
fn check_and_resolve_monomorphizes_two_call_sites_at_different_concrete_types_into_distinct_specializations()
 {
    // Fix-round regression test (finding #3): the actual monomorphization
    // crux is that two call sites at *different* concrete types produce
    // two distinct, correctly-routed specializations -- not the same
    // specialization reused, and not swapped between call sites.
    let param = Ty::Param(Box::new("T".to_string()));
    let identity = generic_identity_fn(param.clone(), param);
    let call_int = HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
        callee: "print".to_string(),
        args: vec![HirExpr::Call {
            callee: "identity".to_string(),
            args: vec![HirExpr::IntLiteral(1)],
        }],
    }));
    let call_str = HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
        callee: "print".to_string(),
        args: vec![HirExpr::Call {
            callee: "identity".to_string(),
            args: vec![HirExpr::StringLiteral("s".to_string())],
        }],
    }));
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![identity, call_int, call_str],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let resolved = check_and_resolve(&hir).unwrap();
    assert!(find_function(&resolved, "identity").is_none());

    let int_specialization = find_function(&resolved, "0gen_identity__T_int")
        .expect("an `int` specialization must exist");
    let str_specialization = find_function(&resolved, "0gen_identity__T_str")
        .expect("a `str` specialization must exist");
    assert_ne!(int_specialization, str_specialization);
    assert_eq!(count_function(&resolved, "0gen_identity__T_int"), 1);
    assert_eq!(count_function(&resolved, "0gen_identity__T_str"), 1);

    // Each call site's rewritten callee must point to its own
    // specialization -- not the same one, not swapped. The original
    // generic `identity` def (index 0) is dropped entirely, so the two
    // top-level call statements shift down to indices 0 and 1.
    assert_eq!(
        resolved.items[0],
        HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
            callee: "print".to_string(),
            args: vec![HirExpr::Call {
                callee: "0gen_identity__T_int".to_string(),
                args: vec![HirExpr::IntLiteral(1)],
            }],
        }))
    );
    assert_eq!(
        resolved.items[1],
        HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
            callee: "print".to_string(),
            args: vec![HirExpr::Call {
                callee: "0gen_identity__T_str".to_string(),
                args: vec![HirExpr::StringLiteral("s".to_string())],
            }],
        }))
    );
}

#[test]
fn check_and_resolve_dedupes_two_call_sites_with_the_same_concrete_type() {
    let param = Ty::Param(Box::new("T".to_string()));
    let identity = generic_identity_fn(param.clone(), param);
    let use_twice = HirItem::Function {
        name: "use_twice".to_string(),
        params: vec![],
        return_ty: Ty::Int,
        body: vec![
            HirStmt::Assign {
                target: "a".to_string(),
                value: HirExpr::Call {
                    callee: "identity".to_string(),
                    args: vec![HirExpr::IntLiteral(1)],
                },
            },
            HirStmt::Assign {
                target: "b".to_string(),
                value: HirExpr::Call {
                    callee: "identity".to_string(),
                    args: vec![HirExpr::IntLiteral(2)],
                },
            },
            HirStmt::Return(Some(HirExpr::BinOp {
                op: BinOpKind::Add,
                left: Box::new(HirExpr::Name("a".to_string())),
                right: Box::new(HirExpr::Name("b".to_string())),
            })),
        ],
    };
    let top = HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
        callee: "print".to_string(),
        args: vec![HirExpr::Call {
            callee: "identity".to_string(),
            args: vec![HirExpr::IntLiteral(3)],
        }],
    }));
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![identity, use_twice, top],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let resolved = check_and_resolve(&hir).unwrap();
    assert_eq!(count_function(&resolved, "0gen_identity__T_int"), 1);
}

#[test]
fn check_and_resolve_monomorphizes_a_generic_function_alongside_an_inferred_private_helper() {
    // `_helper`'s own `Ty::Infer` signature defeats `concrete_function_environment`/
    // `concrete_function_signatures`'s fast path, forcing the solver-inferred
    // path -- this exercises `check_with_signatures_all`'s own `bind_generic`
    // registration (the fast path's registration lives in
    // `concrete_function_environment` instead, already covered by every
    // other test in this group).
    let param = Ty::Param(Box::new("T".to_string()));
    let identity = generic_identity_fn(param.clone(), param);
    // `_helper`'s call site deliberately does not itself involve a
    // generic call -- the solver path's own constraint collection
    // (`collect_expr_constraints`, a separate implementation from
    // `infer_expr_in`) has no generic-call dispatch of its own, so
    // mixing the two would exercise a distinct, unrelated gap rather
    // than the `bind_generic` registration this test targets.
    let helper = HirItem::Function {
        name: "_helper".to_string(),
        params: vec![("v".to_string(), Ty::Infer)],
        return_ty: Ty::Infer,
        body: vec![HirStmt::Return(Some(HirExpr::Name("v".to_string())))],
    };
    let use_helper = HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
        callee: "_helper".to_string(),
        args: vec![HirExpr::IntLiteral(5)],
    }));
    let top = HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
        callee: "print".to_string(),
        args: vec![HirExpr::Call {
            callee: "identity".to_string(),
            args: vec![HirExpr::IntLiteral(1)],
        }],
    }));
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![identity, helper, use_helper, top],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let resolved = check_and_resolve(&hir).unwrap();
    assert!(find_function(&resolved, "identity").is_none());
    assert_eq!(count_function(&resolved, "0gen_identity__T_int"), 1);
}

#[test]
fn an_unannotated_private_helper_fed_a_generic_call_s_result_reports_a_clean_diagnostic_instead_of_leaking_ty_param()
 {
    // Fix-round regression test (finding #2): `_helper(v)` has no
    // annotation, so its parameter type must be inferred from its call
    // site's argument -- `_helper(identity(1))`, where `identity` is a
    // still-generic function whose call this solver-based inference
    // path cannot instantiate (it has no notion of
    // `instantiate_generic_call`). Before this fix, the solver
    // unified `_helper`'s fresh inference variable directly against
    // `identity`'s raw, uninstantiated `Ty::Param("T")` signature,
    // producing a confusing diagnostic that named the internal `T`
    // representation directly (e.g. "operator Add is not defined for
    // `T` and `int`"). This must instead fail with a clean, dedicated
    // diagnostic that never mentions `Ty::Param`/`T` as if it were a
    // real type.
    let param = Ty::Param(Box::new("T".to_string()));
    let identity = generic_identity_fn(param.clone(), param);
    let helper = HirItem::Function {
        name: "_helper".to_string(),
        params: vec![("v".to_string(), Ty::Infer)],
        return_ty: Ty::Infer,
        body: vec![HirStmt::Return(Some(HirExpr::BinOp {
            op: BinOpKind::Add,
            left: Box::new(HirExpr::Name("v".to_string())),
            right: Box::new(HirExpr::IntLiteral(1)),
        }))],
    };
    let use_helper = HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
        callee: "_helper".to_string(),
        args: vec![HirExpr::Call {
            callee: "identity".to_string(),
            args: vec![HirExpr::IntLiteral(1)],
        }],
    }));
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![identity, helper, use_helper],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let err = check_and_resolve(&hir).unwrap_err();
    assert_eq!(err.code, "T0042");
    assert!(
        !err.message.contains("Ty::Param") && !err.message.contains("\"T\""),
        "diagnostic must never leak the raw `Ty::Param` internal representation, got: {}",
        err.message
    );
    // `check` (the validation-only entry point) must fail the same way.
    let check_err = check(&hir).unwrap_err();
    assert_eq!(check_err.code, "T0042");
}

#[test]
fn an_unannotated_private_helper_s_argument_passed_directly_into_a_generic_parameter_reports_a_clean_diagnostic()
 {
    // Complements the test above by exercising the *other* operand of
    // `collect_expr_constraints`'s own `Ty::Param`-leak guard: here it
    // is the callee's own uninstantiated parameter type
    // (`identity`'s `x: T`) that is `Ok(Ty::Param(_))`, not the
    // caller-supplied argument -- `_helper(x)`'s own unannotated `x` is
    // the unresolved (`Err`) side instead. Both operands of the `||`
    // need their own case to reach `true` (the other test's argument
    // is already resolved when this one's is not, and vice versa),
    // since Rust's `||` short-circuits and a covering test for only one
    // operand leaves the other's `true` branch unreached.
    let param = Ty::Param(Box::new("T".to_string()));
    let identity = generic_identity_fn(param.clone(), param);
    let helper = HirItem::Function {
        name: "_helper".to_string(),
        params: vec![("x".to_string(), Ty::Infer)],
        return_ty: Ty::Infer,
        body: vec![HirStmt::Return(Some(HirExpr::Call {
            callee: "identity".to_string(),
            args: vec![HirExpr::Name("x".to_string())],
        }))],
    };
    let use_helper = HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
        callee: "_helper".to_string(),
        args: vec![HirExpr::IntLiteral(1)],
    }));
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![identity, helper, use_helper],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert_eq!(check_and_resolve(&hir).unwrap_err().code, "T0042");
    assert_eq!(check(&hir).unwrap_err().code, "T0042");
}

#[test]
fn an_unannotated_private_helper_s_return_type_fed_directly_from_a_generic_call_reports_a_clean_diagnostic()
 {
    // Exercises `unify_terms`'s own dedicated `Ty::Param`-leak guard
    // directly (as opposed to `collect_expr_constraints`'s own Call-arm
    // guard, covered by the two tests above): a private helper's
    // *return* type is unified against its body's final expression
    // unconditionally (`collect_block_constraints`'s `Return` handling
    // has no analogous pre-guard of its own), so `_helper`'s
    // unannotated return type unifying directly against `identity(1)`'s
    // own uninstantiated `Ty::Param` return term must still produce a
    // clean `T0042`, not a leaked-`Ty::Param` diagnostic.
    let param = Ty::Param(Box::new("T".to_string()));
    let identity = generic_identity_fn(param.clone(), param);
    let helper = HirItem::Function {
        name: "_helper".to_string(),
        params: vec![],
        return_ty: Ty::Infer,
        body: vec![HirStmt::Return(Some(HirExpr::Call {
            callee: "identity".to_string(),
            args: vec![HirExpr::IntLiteral(1)],
        }))],
    };
    let use_helper = HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
        callee: "_helper".to_string(),
        args: vec![],
    }));
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![identity, helper, use_helper],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert_eq!(check_and_resolve(&hir).unwrap_err().code, "T0042");
    assert_eq!(check(&hir).unwrap_err().code, "T0042");
}

#[test]
fn check_and_resolve_rewrites_generic_calls_inside_binop_and_compare() {
    let param = Ty::Param(Box::new("T".to_string()));
    let identity = generic_identity_fn(param.clone(), param);
    let f = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::Int,
        body: vec![HirStmt::Return(Some(HirExpr::BinOp {
            op: BinOpKind::Add,
            left: Box::new(HirExpr::Call {
                callee: "identity".to_string(),
                args: vec![HirExpr::IntLiteral(1)],
            }),
            right: Box::new(HirExpr::IntLiteral(2)),
        }))],
    };
    let g = HirItem::Function {
        name: "g".to_string(),
        params: vec![],
        return_ty: Ty::Bool,
        body: vec![HirStmt::Return(Some(HirExpr::Compare {
            op: CmpOpKind::Eq,
            left: Box::new(HirExpr::Call {
                callee: "identity".to_string(),
                args: vec![HirExpr::IntLiteral(1)],
            }),
            right: Box::new(HirExpr::IntLiteral(2)),
        }))],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![identity, f, g],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let resolved = check_and_resolve(&hir).unwrap();
    assert_eq!(count_function(&resolved, "0gen_identity__T_int"), 1);
}

#[test]
fn check_and_resolve_rewrites_a_generic_call_inside_an_fstring_interpolation() {
    let param = Ty::Param(Box::new("T".to_string()));
    let identity = generic_identity_fn(param.clone(), param);
    let f = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::Str,
        body: vec![HirStmt::Return(Some(HirExpr::FString(vec![
            FStringPart::Literal("x=".to_string()),
            FStringPart::Interpolation(Box::new(HirExpr::Call {
                callee: "identity".to_string(),
                args: vec![HirExpr::IntLiteral(1)],
            })),
        ])))],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![identity, f],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let resolved = check_and_resolve(&hir).unwrap();
    assert_eq!(count_function(&resolved, "0gen_identity__T_int"), 1);
}

#[test]
fn check_and_resolve_rewrites_generic_calls_inside_list_set_and_tuple_literals() {
    let param = Ty::Param(Box::new("T".to_string()));
    let identity = generic_identity_fn(param.clone(), param);
    let f = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![
            HirStmt::Assign {
                target: "xs".to_string(),
                value: HirExpr::ListLiteral(vec![HirExpr::Call {
                    callee: "identity".to_string(),
                    args: vec![HirExpr::IntLiteral(1)],
                }]),
            },
            HirStmt::Assign {
                target: "ys".to_string(),
                value: HirExpr::SetLiteral(vec![HirExpr::Call {
                    callee: "identity".to_string(),
                    args: vec![HirExpr::IntLiteral(2)],
                }]),
            },
            HirStmt::Assign {
                target: "zs".to_string(),
                value: HirExpr::TupleLiteral(vec![
                    HirExpr::Call {
                        callee: "identity".to_string(),
                        args: vec![HirExpr::IntLiteral(3)],
                    },
                    HirExpr::IntLiteral(4),
                ]),
            },
            HirStmt::Return(None),
        ],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![identity, f],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let resolved = check_and_resolve(&hir).unwrap();
    assert_eq!(count_function(&resolved, "0gen_identity__T_int"), 1);
}

#[test]
fn check_and_resolve_rewrites_a_generic_call_inside_a_dict_literal() {
    let param = Ty::Param(Box::new("T".to_string()));
    let identity = generic_identity_fn(param.clone(), param);
    let f = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![HirStmt::Assign {
            target: "d".to_string(),
            value: HirExpr::DictLiteral(vec![(
                HirExpr::StringLiteral("a".to_string()),
                HirExpr::Call {
                    callee: "identity".to_string(),
                    args: vec![HirExpr::IntLiteral(1)],
                },
            )]),
        }],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![identity, f],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let resolved = check_and_resolve(&hir).unwrap();
    assert_eq!(count_function(&resolved, "0gen_identity__T_int"), 1);
}

#[test]
fn check_and_resolve_rewrites_generic_calls_inside_subscript_and_slice() {
    let param = Ty::Param(Box::new("T".to_string()));
    let identity = generic_identity_fn(param.clone(), param);
    let f = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![
            HirStmt::Assign {
                target: "xs".to_string(),
                value: HirExpr::ListLiteral(vec![
                    HirExpr::IntLiteral(1),
                    HirExpr::IntLiteral(2),
                    HirExpr::IntLiteral(3),
                ]),
            },
            HirStmt::Assign {
                target: "y".to_string(),
                value: HirExpr::Subscript {
                    base: Box::new(HirExpr::Name("xs".to_string())),
                    index: Box::new(HirExpr::Call {
                        callee: "identity".to_string(),
                        args: vec![HirExpr::IntLiteral(0)],
                    }),
                },
            },
            HirStmt::Assign {
                target: "z".to_string(),
                value: HirExpr::Slice {
                    base: Box::new(HirExpr::Name("xs".to_string())),
                    start: Some(Box::new(HirExpr::Call {
                        callee: "identity".to_string(),
                        args: vec![HirExpr::IntLiteral(0)],
                    })),
                    stop: Some(Box::new(HirExpr::IntLiteral(2))),
                    step: None,
                },
            },
        ],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![identity, f],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let resolved = check_and_resolve(&hir).unwrap();
    assert_eq!(count_function(&resolved, "0gen_identity__T_int"), 1);
}

#[test]
fn check_and_resolve_rewrites_generic_calls_inside_list_append_and_set_add() {
    let param = Ty::Param(Box::new("T".to_string()));
    let identity = generic_identity_fn(param.clone(), param);
    let f = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![
            HirStmt::Assign {
                target: "xs".to_string(),
                value: HirExpr::ListLiteral(vec![HirExpr::IntLiteral(1)]),
            },
            HirStmt::Assign {
                target: "ys".to_string(),
                value: HirExpr::SetLiteral(vec![HirExpr::IntLiteral(1)]),
            },
            HirStmt::ExprStmt(HirExpr::ListAppend {
                list: pycc_hir::ContainerReceiver::Name("xs".to_string()),
                value: Box::new(HirExpr::Call {
                    callee: "identity".to_string(),
                    args: vec![HirExpr::IntLiteral(2)],
                }),
            }),
            HirStmt::ExprStmt(HirExpr::SetAdd {
                set: "ys".to_string(),
                value: Box::new(HirExpr::Call {
                    callee: "identity".to_string(),
                    args: vec![HirExpr::IntLiteral(3)],
                }),
            }),
        ],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![identity, f],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let resolved = check_and_resolve(&hir).unwrap();
    assert_eq!(count_function(&resolved, "0gen_identity__T_int"), 1);
}

#[test]
fn check_and_resolve_rewrites_generic_calls_inside_dict_get_or_default() {
    let param = Ty::Param(Box::new("T".to_string()));
    let identity = generic_identity_fn(param.clone(), param);
    let f = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![
            HirStmt::Assign {
                target: "d".to_string(),
                value: HirExpr::DictLiteral(vec![(
                    HirExpr::StringLiteral("a".to_string()),
                    HirExpr::IntLiteral(1),
                )]),
            },
            HirStmt::Assign {
                target: "v".to_string(),
                value: HirExpr::DictGetOrDefault {
                    dict: pycc_hir::ContainerReceiver::Name("d".to_string()),
                    key: Box::new(HirExpr::Call {
                        callee: "identity".to_string(),
                        args: vec![HirExpr::StringLiteral("a".to_string())],
                    }),
                    default: Box::new(HirExpr::Call {
                        callee: "identity".to_string(),
                        args: vec![HirExpr::IntLiteral(0)],
                    }),
                },
            },
        ],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![identity, f],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let resolved = check_and_resolve(&hir).unwrap();
    assert_eq!(count_function(&resolved, "0gen_identity__T_str"), 1);
    assert_eq!(count_function(&resolved, "0gen_identity__T_int"), 1);
}

#[test]
fn check_and_resolve_rewrites_generic_calls_inside_if_while_and_for_range() {
    let param = Ty::Param(Box::new("T".to_string()));
    let identity = generic_identity_fn(param.clone(), param);
    let f = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![
            HirStmt::If {
                test: HirExpr::Call {
                    callee: "identity".to_string(),
                    args: vec![HirExpr::BoolLiteral(true)],
                },
                body: vec![HirStmt::ExprStmt(HirExpr::Call {
                    callee: "identity".to_string(),
                    args: vec![HirExpr::IntLiteral(1)],
                })],
                orelse: vec![HirStmt::ExprStmt(HirExpr::Call {
                    callee: "identity".to_string(),
                    args: vec![HirExpr::IntLiteral(2)],
                })],
            },
            HirStmt::While {
                test: HirExpr::Call {
                    callee: "identity".to_string(),
                    args: vec![HirExpr::BoolLiteral(false)],
                },
                body: vec![HirStmt::ExprStmt(HirExpr::Call {
                    callee: "identity".to_string(),
                    args: vec![HirExpr::IntLiteral(3)],
                })],
            },
            HirStmt::ForRange {
                var: "i".to_string(),
                start: HirExpr::IntLiteral(0),
                stop: HirExpr::Call {
                    callee: "identity".to_string(),
                    args: vec![HirExpr::IntLiteral(3)],
                },
                step: HirExpr::IntLiteral(1),
                body: vec![HirStmt::ExprStmt(HirExpr::Name("i".to_string()))],
            },
        ],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![identity, f],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let resolved = check_and_resolve(&hir).unwrap();
    assert_eq!(count_function(&resolved, "0gen_identity__T_bool"), 1);
    assert_eq!(count_function(&resolved, "0gen_identity__T_int"), 1);
}

#[test]
fn check_and_resolve_rewrites_a_generic_call_inside_for_list_over_a_list() {
    let param = Ty::Param(Box::new("T".to_string()));
    let identity = generic_identity_fn(param.clone(), param);
    let f = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![
            HirStmt::Assign {
                target: "xs".to_string(),
                value: HirExpr::ListLiteral(vec![HirExpr::IntLiteral(1)]),
            },
            HirStmt::ForList {
                var: "x".to_string(),
                list: "xs".to_string(),
                body: vec![HirStmt::ExprStmt(HirExpr::Call {
                    callee: "identity".to_string(),
                    args: vec![HirExpr::Name("x".to_string())],
                })],
            },
        ],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![identity, f],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let resolved = check_and_resolve(&hir).unwrap();
    assert_eq!(count_function(&resolved, "0gen_identity__T_int"), 1);
}

#[test]
fn check_and_resolve_rewrites_a_generic_call_inside_for_list_over_a_dict() {
    let param = Ty::Param(Box::new("T".to_string()));
    let identity = generic_identity_fn(param.clone(), param);
    let f = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![
            HirStmt::Assign {
                target: "d".to_string(),
                value: HirExpr::DictLiteral(vec![(
                    HirExpr::StringLiteral("a".to_string()),
                    HirExpr::IntLiteral(1),
                )]),
            },
            HirStmt::ForList {
                var: "k".to_string(),
                list: "d".to_string(),
                body: vec![HirStmt::ExprStmt(HirExpr::Call {
                    callee: "identity".to_string(),
                    args: vec![HirExpr::Name("k".to_string())],
                })],
            },
        ],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![identity, f],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let resolved = check_and_resolve(&hir).unwrap();
    assert_eq!(count_function(&resolved, "0gen_identity__T_str"), 1);
}

#[test]
fn check_and_resolve_rewrites_a_generic_call_inside_for_list_over_a_set() {
    let param = Ty::Param(Box::new("T".to_string()));
    let identity = generic_identity_fn(param.clone(), param);
    let f = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![
            HirStmt::Assign {
                target: "s".to_string(),
                value: HirExpr::SetLiteral(vec![HirExpr::IntLiteral(1)]),
            },
            HirStmt::ForList {
                var: "x".to_string(),
                list: "s".to_string(),
                body: vec![HirStmt::ExprStmt(HirExpr::Call {
                    callee: "identity".to_string(),
                    args: vec![HirExpr::Name("x".to_string())],
                })],
            },
        ],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![identity, f],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let resolved = check_and_resolve(&hir).unwrap();
    assert_eq!(count_function(&resolved, "0gen_identity__T_int"), 1);
}

#[test]
fn check_and_resolve_rewrites_generic_calls_inside_ann_assign_with_and_without_a_value() {
    let param = Ty::Param(Box::new("T".to_string()));
    let identity = generic_identity_fn(param.clone(), param);
    let f = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![
            HirStmt::AnnAssign {
                is_final: false,
                target: "y".to_string(),
                annotation: Ty::Int,
                value: None,
            },
            HirStmt::AnnAssign {
                is_final: false,
                target: "z".to_string(),
                annotation: Ty::Int,
                value: Some(HirExpr::Call {
                    callee: "identity".to_string(),
                    args: vec![HirExpr::IntLiteral(1)],
                }),
            },
            HirStmt::Assign {
                target: "y".to_string(),
                value: HirExpr::IntLiteral(2),
            },
        ],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![identity, f],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let resolved = check_and_resolve(&hir).unwrap();
    assert_eq!(count_function(&resolved, "0gen_identity__T_int"), 1);
}

#[test]
fn check_and_resolve_rewrites_generic_calls_inside_a_dict_set_statement() {
    let param = Ty::Param(Box::new("T".to_string()));
    let identity = generic_identity_fn(param.clone(), param);
    let f = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![
            HirStmt::Assign {
                target: "d".to_string(),
                value: HirExpr::DictLiteral(vec![(
                    HirExpr::StringLiteral("a".to_string()),
                    HirExpr::IntLiteral(1),
                )]),
            },
            HirStmt::DictSet {
                dict: "d".to_string(),
                key: HirExpr::Call {
                    callee: "identity".to_string(),
                    args: vec![HirExpr::StringLiteral("b".to_string())],
                },
                value: HirExpr::Call {
                    callee: "identity".to_string(),
                    args: vec![HirExpr::IntLiteral(2)],
                },
            },
        ],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![identity, f],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let resolved = check_and_resolve(&hir).unwrap();
    assert_eq!(count_function(&resolved, "0gen_identity__T_str"), 1);
    assert_eq!(count_function(&resolved, "0gen_identity__T_int"), 1);
}

#[test]
fn check_and_resolve_rewrites_generic_calls_inside_comprehension_assignments() {
    let param = Ty::Param(Box::new("T".to_string()));
    let identity = generic_identity_fn(param.clone(), param);
    let f = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![
            HirStmt::ListCompAssign {
                target: "xs".to_string(),
                var: "0comp_v".to_string(),
                iter: CompIter::Range {
                    start: HirExpr::IntLiteral(0),
                    stop: HirExpr::Call {
                        callee: "identity".to_string(),
                        args: vec![HirExpr::IntLiteral(3)],
                    },
                    step: HirExpr::IntLiteral(1),
                },
                cond: Some(Box::new(HirExpr::Call {
                    callee: "identity".to_string(),
                    args: vec![HirExpr::BoolLiteral(true)],
                })),
                elt: Box::new(HirExpr::Call {
                    callee: "identity".to_string(),
                    args: vec![HirExpr::Name("0comp_v".to_string())],
                }),
            },
            HirStmt::SetCompAssign {
                target: "ys".to_string(),
                var: "0comp_w".to_string(),
                iter: CompIter::Name("xs".to_string()),
                cond: Some(Box::new(HirExpr::Call {
                    callee: "identity".to_string(),
                    args: vec![HirExpr::BoolLiteral(true)],
                })),
                elt: Box::new(HirExpr::Name("0comp_w".to_string())),
            },
            HirStmt::DictCompAssign {
                target: "zs".to_string(),
                var: "0comp_u".to_string(),
                iter: CompIter::Name("ys".to_string()),
                cond: Some(Box::new(HirExpr::Call {
                    callee: "identity".to_string(),
                    args: vec![HirExpr::BoolLiteral(false)],
                })),
                key: Box::new(HirExpr::StringLiteral("k".to_string())),
                value: Box::new(HirExpr::Name("0comp_u".to_string())),
            },
            HirStmt::SetCompAssign {
                target: "ks".to_string(),
                var: "0comp_p".to_string(),
                iter: CompIter::Name("zs".to_string()),
                cond: None,
                elt: Box::new(HirExpr::IntLiteral(1)),
            },
        ],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![identity, f],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let resolved = check_and_resolve(&hir).unwrap();
    assert_eq!(count_function(&resolved, "0gen_identity__T_int"), 1);
    assert_eq!(count_function(&resolved, "0gen_identity__T_bool"), 1);
}

#[test]
fn check_and_resolve_leaves_a_list_pop_expression_untouched_as_a_structural_leaf() {
    let param = Ty::Param(Box::new("T".to_string()));
    let identity = generic_identity_fn(param.clone(), param);
    let f = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![
            HirStmt::Assign {
                target: "xs".to_string(),
                value: HirExpr::ListLiteral(vec![HirExpr::IntLiteral(1)]),
            },
            HirStmt::ExprStmt(HirExpr::ListPop {
                list: pycc_hir::ContainerReceiver::Name("xs".to_string()),
            }),
            HirStmt::ExprStmt(HirExpr::Call {
                callee: "identity".to_string(),
                args: vec![HirExpr::IntLiteral(2)],
            }),
        ],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![identity, f],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let resolved = check_and_resolve(&hir).unwrap();
    assert_eq!(count_function(&resolved, "0gen_identity__T_int"), 1);
}
