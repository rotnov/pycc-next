//! Value bindings shadowing functions and builtins, and later rebinding by `def`.

use super::*;

/// #133: a module-level value binding must shadow a same-named user
/// function at every later call site (CPython raises `TypeError: 'int'
/// object is not callable`; the pre-fix checker resolved the call
/// through the function registry and accepted it).
#[test]
fn a_module_value_binding_shadows_a_same_named_function_call() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "helper".to_string(),
                params: vec![],
                return_ty: Ty::None,
                body: vec![HirStmt::ExprStmt(HirExpr::Call {
                    callee: "print".to_string(),
                    args: vec![HirExpr::IntLiteral(1)],
                })],
            },
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "helper".to_string(),
                value: HirExpr::IntLiteral(1),
            }),
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
                callee: "helper".to_string(),
                args: vec![],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(
        err.message,
        "name `helper` is bound to a non-callable value"
    );
}

/// #133: the same shadowing rule covers builtins -- `print = 1` makes a
/// later `print()` a call on an `int`, which CPython rejects at runtime.
#[test]
fn a_module_value_binding_shadows_a_builtin_call() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "print".to_string(),
                value: HirExpr::IntLiteral(1),
            }),
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
                callee: "print".to_string(),
                args: vec![],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(err.message, "name `print` is bound to a non-callable value");
}

/// #133: pass 3 checks annotated function bodies against the final
/// module environment (D-041), so a body call whose target is
/// module-bound to a value is rejected through `infer_expr_in`'s gate.
#[test]
fn an_annotated_body_call_sees_the_module_value_binding() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "helper".to_string(),
                params: vec![],
                return_ty: Ty::Int,
                body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(1)))],
            },
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "helper".to_string(),
                value: HirExpr::IntLiteral(2),
            }),
            HirItem::Function {
                name: "caller".to_string(),
                params: vec![],
                return_ty: Ty::Int,
                body: vec![HirStmt::Return(Some(HirExpr::Call {
                    callee: "helper".to_string(),
                    args: vec![],
                }))],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(
        err.message,
        "name `helper` is bound to a non-callable value"
    );
}

/// #133: the solver path (an underscore private helper with an inferred
/// signature) seeds its body environment from the module globals, so
/// `collect_expr_constraints`'s mirrored gate rejects the same shape.
/// Mirrors the issue's own third reproduction end to end through
/// `check_and_resolve`.
#[test]
fn a_private_helper_call_sees_the_module_value_binding() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "helper".to_string(),
                params: vec![],
                return_ty: Ty::Int,
                body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(1)))],
            },
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "helper".to_string(),
                value: HirExpr::IntLiteral(2),
            }),
            HirItem::Function {
                name: "_call_helper".to_string(),
                params: vec![],
                return_ty: Ty::Infer,
                body: vec![HirStmt::Return(Some(HirExpr::Call {
                    callee: "helper".to_string(),
                    args: vec![],
                }))],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let err = check_and_resolve(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(
        err.message,
        "name `helper` is bound to a non-callable value"
    );
}

/// D-110's first recorded ordering consequence, pinned as deliberate: a
/// body call is rejected whenever the callee is value-bound anywhere at
/// module top level, even when the program's only call executes *before*
/// the rebinding in CPython's dynamic order (this exact module runs fine
/// under CPython: `caller()` returns 1, then `helper = 2` rebinds). Pass
/// 3 checks bodies against the final module environment (D-041), and
/// D-110 accepts the stricter-than-one-dynamic-trace rejection rather
/// than importing #22's execution-order model.
#[test]
fn a_rebinding_after_the_only_call_still_rejects_the_body_call() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "helper".to_string(),
                params: vec![],
                return_ty: Ty::Int,
                body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(1)))],
            },
            HirItem::Function {
                name: "caller".to_string(),
                params: vec![],
                return_ty: Ty::Int,
                body: vec![HirStmt::Return(Some(HirExpr::Call {
                    callee: "helper".to_string(),
                    args: vec![],
                }))],
            },
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::Call {
                    callee: "caller".to_string(),
                    args: vec![],
                },
            }),
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "helper".to_string(),
                value: HirExpr::IntLiteral(2),
            }),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(
        err.message,
        "name `helper` is bound to a non-callable value"
    );
}

/// D-110's second recorded ordering consequence: the binding gate is
/// callee-first, so a shadowed callee with an additionally-invalid
/// argument reports the non-callable callee, not the argument's own
/// error -- uniform with the pre-existing local gate, though CPython's
/// evaluation order would surface the argument's `NameError` first.
#[test]
fn a_shadowed_callee_reports_before_its_argument_errors() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "helper".to_string(),
                value: HirExpr::IntLiteral(1),
            }),
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
                callee: "helper".to_string(),
                args: vec![HirExpr::Name("undefined_name".to_string())],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(
        err.message,
        "name `helper` is bound to a non-callable value"
    );
}

/// D-110's third recorded consequence: a bound value that shadows
/// nothing at all now reports the accurate non-callable diagnostic
/// instead of final validation's misleading "call to undefined function".
#[test]
fn calling_a_bound_value_that_shadows_nothing_reports_non_callable() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::IntLiteral(1),
            }),
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
                callee: "x".to_string(),
                args: vec![],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(err.message, "name `x` is bound to a non-callable value");
}

/// The discriminating case for `collect_expr_constraints`' mirror gate
/// (D-110): the bound callee is neither `print` nor any `def`, inside an
/// inference-signature helper. Without the mirror, `signatures.get`
/// misses, the call stays unresolved, and the solver dead-ends in
/// "cannot infer return type of private helper `_h`; add an annotation"
/// *before* pass 3 ever runs -- a misleading message, since no
/// annotation fixes calling an `int`. This is the one shape where the
/// mirror's presence changes the observable diagnostic; a shadowed
/// `print` would instead be resolved by the special case and caught by
/// pass 3's own gate later (see the mirror-gate comment).
#[test]
fn a_private_helper_calling_a_shadowed_non_function_needs_the_mirror_gate() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::IntLiteral(1),
            }),
            HirItem::Function {
                name: "_h".to_string(),
                params: vec![],
                return_ty: Ty::Infer,
                body: vec![HirStmt::Return(Some(HirExpr::Call {
                    callee: "x".to_string(),
                    args: vec![],
                }))],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let err = check_and_resolve(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(err.message, "name `x` is bound to a non-callable value");
}

/// Behavior pin for the shadowed-builtin shape inside an
/// inference-signature helper: the mirror gate fires before the `print`
/// special case, so the accurate non-callable diagnostic wins. (Pass 3
/// would independently reject this shape even without the mirror; the
/// discriminating case for the mirror itself is the test above.)
#[test]
fn a_private_helper_calling_a_shadowed_builtin_is_rejected() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "print".to_string(),
                value: HirExpr::IntLiteral(1),
            }),
            HirItem::Function {
                name: "_h".to_string(),
                params: vec![],
                return_ty: Ty::Infer,
                body: vec![HirStmt::Return(Some(HirExpr::Call {
                    callee: "print".to_string(),
                    args: vec![],
                }))],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let err = check_and_resolve(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(err.message, "name `print` is bound to a non-callable value");
}

/// The round-5 blocker's pin (PR #252's review): a *compatible-type*
/// value reassignment after the `def` must re-shadow it, even though
/// `check_assignment`'s already-bound branch never calls `bind()` --
/// clearing the def-rebound flag only inside `bind()` left it
/// permanently stuck for any name with a pre-`def` binding, so this
/// exact module resolved the function where CPython raises `TypeError`.
#[test]
fn a_compatible_reassignment_after_the_def_shadows_it_again() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "helper".to_string(),
                value: HirExpr::IntLiteral(1),
            }),
            HirItem::Function {
                name: "helper".to_string(),
                params: vec![],
                return_ty: Ty::Int,
                body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(2)))],
            },
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "helper".to_string(),
                value: HirExpr::IntLiteral(2),
            }),
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
                callee: "helper".to_string(),
                args: vec![],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(
        err.message,
        "name `helper` is bound to a non-callable value"
    );
}

/// The round-6 pin (PR #252's review): a private helper's *parameter*
/// that collides with a module-level def-rebound name re-binds it for
/// that body, so calling the parameter reports the accurate
/// non-callable diagnostic -- before the fix the per-function env setup
/// stripped the name from `bindings` but left the stale def-rebound
/// flag, skipping the mirror gate and mislabeling a bound parameter as
/// "not bound before this use".
#[test]
fn a_parameter_colliding_with_a_def_rebound_name_reports_non_callable() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "helper".to_string(),
                value: HirExpr::IntLiteral(1),
            }),
            HirItem::Function {
                name: "helper".to_string(),
                params: vec![],
                return_ty: Ty::Int,
                body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(2)))],
            },
            HirItem::Function {
                name: "_h".to_string(),
                params: vec![("helper".to_string(), Ty::Infer)],
                return_ty: Ty::Infer,
                body: vec![HirStmt::Return(Some(HirExpr::Call {
                    callee: "helper".to_string(),
                    args: vec![],
                }))],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let err = check_and_resolve(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(
        err.message,
        "name `helper` is bound to a non-callable value"
    );
}

/// The solver-path twin of the test above, proving the two environment
/// walks agree: the solver's Assign arm already cleared its def-rebound
/// flag unconditionally, and after the round-5 fix the `Environment`
/// walk does too.
#[test]
fn a_compatible_reassignment_after_the_def_shadows_it_in_the_solver_too() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "helper".to_string(),
                value: HirExpr::IntLiteral(1),
            }),
            HirItem::Function {
                name: "helper".to_string(),
                params: vec![],
                return_ty: Ty::Int,
                body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(2)))],
            },
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "helper".to_string(),
                value: HirExpr::IntLiteral(2),
            }),
            HirItem::Function {
                name: "_h".to_string(),
                params: vec![],
                return_ty: Ty::Infer,
                body: vec![HirStmt::Return(Some(HirExpr::Call {
                    callee: "helper".to_string(),
                    args: vec![],
                }))],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let err = check_and_resolve(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(
        err.message,
        "name `helper` is bound to a non-callable value"
    );
}

/// The round-4 blocker's pin (PR #252's review): the `def`-rebinding
/// fact must NOT erase the representation record, so D-040's sticky
/// rule still rejects `helper = 1; def helper(): ...; helper = "leaked"`
/// with T0023 -- a version that cleared `bindings` at the `def` let this
/// program reach codegen with an `int`-allocated slot stored as `str`.
#[test]
fn a_value_reassignment_after_the_def_still_enforces_representation() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "helper".to_string(),
                value: HirExpr::IntLiteral(1),
            }),
            HirItem::Function {
                name: "helper".to_string(),
                params: vec![],
                return_ty: Ty::None,
                body: vec![HirStmt::ExprStmt(HirExpr::Call {
                    callee: "print".to_string(),
                    args: vec![HirExpr::IntLiteral(1)],
                })],
            },
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "helper".to_string(),
                value: HirExpr::StringLiteral("leaked".to_string()),
            }),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0023");
    assert_eq!(
        err.message,
        "cannot assign `str` to `helper`, previously inferred as `int`"
    );
}

/// D-110's source-order `def` rebinding (PR #252's review caught the
/// initial gate rejecting this): a later `def` rebinds the name over an
/// earlier value binding, so the call resolves the function exactly
/// as CPython does (`helper = 1; def helper(): ...; helper()` prints
/// the function's result there).
#[test]
fn a_later_def_rebinds_over_an_earlier_value_binding() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "helper".to_string(),
                value: HirExpr::IntLiteral(1),
            }),
            HirItem::Function {
                name: "helper".to_string(),
                params: vec![],
                return_ty: Ty::Int,
                body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(2)))],
            },
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
                callee: "print".to_string(),
                args: vec![HirExpr::Call {
                    callee: "helper".to_string(),
                    args: vec![],
                }],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    assert!(check(&hir).is_ok());
}

/// The same source-order `def` rebinding seen from a function body:
/// pass 3's final module environment reflects the net binding, so a
/// body call to a name whose value binding was cleared by a later
/// `def` resolves the function.
#[test]
fn a_body_call_resolves_when_a_later_def_rebinds_the_value() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "helper".to_string(),
                value: HirExpr::IntLiteral(1),
            }),
            HirItem::Function {
                name: "helper".to_string(),
                params: vec![],
                return_ty: Ty::Int,
                body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(2)))],
            },
            HirItem::Function {
                name: "caller".to_string(),
                params: vec![],
                return_ty: Ty::Int,
                body: vec![HirStmt::Return(Some(HirExpr::Call {
                    callee: "helper".to_string(),
                    args: vec![],
                }))],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    assert!(check(&hir).is_ok());
}

/// And from the solver path: the accumulated globals clear a value
/// binding when the same-named `def` follows it, so an
/// inference-signature helper's body resolves the function.
#[test]
fn a_private_helper_resolves_when_a_later_def_rebinds_the_value() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "helper".to_string(),
                value: HirExpr::IntLiteral(1),
            }),
            HirItem::Function {
                name: "helper".to_string(),
                params: vec![],
                return_ty: Ty::Int,
                body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(2)))],
            },
            HirItem::Function {
                name: "_h".to_string(),
                params: vec![],
                return_ty: Ty::Infer,
                body: vec![HirStmt::Return(Some(HirExpr::Call {
                    callee: "helper".to_string(),
                    args: vec![],
                }))],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    assert!(check_and_resolve(&hir).is_ok());
}

/// #133's deliberate boundary with #22: pass 2 checks top-level code in
/// source order, so a call *before* the shadowing assignment still
/// resolves the function -- the binding simply does not exist yet at
/// that point. Only the already-active binding shadows.
#[test]
fn a_top_level_call_before_the_shadowing_assignment_still_resolves() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "helper".to_string(),
                params: vec![],
                return_ty: Ty::None,
                body: vec![HirStmt::ExprStmt(HirExpr::Call {
                    callee: "print".to_string(),
                    args: vec![HirExpr::IntLiteral(1)],
                })],
            },
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
                callee: "helper".to_string(),
                args: vec![],
            })),
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "helper".to_string(),
                value: HirExpr::IntLiteral(1),
            }),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    assert!(check(&hir).is_ok());
}
