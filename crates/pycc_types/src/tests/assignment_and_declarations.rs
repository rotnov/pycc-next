//! Assignments, annotated and value-less declarations, representation changes.

use super::*;

#[test]
fn an_assignment_binds_the_inferred_type_in_the_environment() {
    let mut env = Environment::new();
    check_stmt(
        &mut env,
        &HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::IntLiteral(1),
        },
    )
    .unwrap();
    assert_eq!(env.lookup("x"), Some(Ty::Int));
}

#[test]
fn an_assignment_whose_value_is_undefined_propagates_the_error() {
    let mut env = Environment::new();
    let err = check_stmt(
        &mut env,
        &HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::Name("undefined".to_string()),
        },
    )
    .unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(env.lookup("x"), None);
}

#[test]
fn an_incompatible_reassignment_is_t0023_and_preserves_the_inferred_type() {
    let mut env = Environment::new();
    check_stmt(
        &mut env,
        &HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::IntLiteral(1),
        },
    )
    .unwrap();
    let err = check_stmt(
        &mut env,
        &HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::StringLiteral("changed".to_string()),
        },
    )
    .unwrap_err();
    assert_eq!(err.code, "T0023");
    assert_eq!(env.lookup("x"), Some(Ty::Int));
}

#[test]
fn an_annotated_assignment_with_a_matching_value_binds_the_annotation_type() {
    // x: int = True -- bool is assignable to int (matches is_assignable's
    // existing widening rule), and the environment should record Ty::Int
    // (the annotation), not Ty::Bool (the initializer's own inferred type).
    let mut env = Environment::new();
    check_stmt(
        &mut env,
        &HirStmt::AnnAssign {
            is_final: false,
            target: "x".to_string(),
            annotation: Ty::Int,
            value: Some(HirExpr::BoolLiteral(true)),
        },
    )
    .unwrap();
    assert_eq!(env.lookup("x"), Some(Ty::Int));
}

#[test]
fn an_annotated_assignment_with_a_mismatched_value_is_t0025() {
    let mut env = Environment::new();
    let err = check_stmt(
        &mut env,
        &HirStmt::AnnAssign {
            is_final: false,
            target: "x".to_string(),
            annotation: Ty::Int,
            value: Some(HirExpr::StringLiteral("nope".to_string())),
        },
    )
    .unwrap_err();
    assert_eq!(err.code, "T0025");
    assert_eq!(env.lookup("x"), None);
}

#[test]
fn an_annotated_assignment_propagates_an_error_from_the_initializer_expression() {
    // The initializer itself can fail to type-check (here, referencing an
    // undefined name) before `is_assignable` is ever consulted -- the `?`
    // on `infer_expr` inside the new arm must propagate that error rather
    // than being reachable only via the is_assignable mismatch path.
    let mut env = Environment::new();
    let err = check_stmt(
        &mut env,
        &HirStmt::AnnAssign {
            is_final: false,
            target: "x".to_string(),
            annotation: Ty::Int,
            value: Some(HirExpr::Name("undefined".to_string())),
        },
    )
    .unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(env.lookup("x"), None);
}

#[test]
fn an_annotation_only_declaration_does_not_bind_a_value_at_module_scope() {
    // x: int alone must only declare x, not bind it -- collect_local_names
    // registers `x` as local independently of this arm, so a premature
    // read still falls through to the existing T0021 mechanism (see the
    // function-scope regression test below for the end-to-end proof).
    let mut env = Environment::new();
    check_stmt(
        &mut env,
        &HirStmt::AnnAssign {
            is_final: false,
            target: "x".to_string(),
            annotation: Ty::Int,
            value: None,
        },
    )
    .unwrap();
    assert_eq!(env.lookup("x"), None);
}

#[test]
fn value_less_declaration_then_matching_assignment_binds_the_declared_type() {
    // x: int; x = 1 -- issue #245: the value-less declaration must be
    // retained and honored by the later plain assignment.
    let mut env = Environment::new();
    check_stmt(
        &mut env,
        &HirStmt::AnnAssign {
            is_final: false,
            target: "x".to_string(),
            annotation: Ty::Int,
            value: None,
        },
    )
    .unwrap();
    check_stmt(
        &mut env,
        &HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::IntLiteral(1),
        },
    )
    .unwrap();
    assert_eq!(env.lookup("x"), Some(Ty::Int));
}

#[test]
fn value_less_declaration_then_mismatched_assignment_is_t0026() {
    // x: int; x = "hello" -- issue #245's primary reproduction: this must
    // be rejected, and rejected with T0026 (not T0023 -- nothing was
    // "previously inferred", it was declared, never assigned).
    let mut env = Environment::new();
    check_stmt(
        &mut env,
        &HirStmt::AnnAssign {
            is_final: false,
            target: "x".to_string(),
            annotation: Ty::Int,
            value: None,
        },
    )
    .unwrap();
    let err = check_stmt(
        &mut env,
        &HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::StringLiteral("hello".to_string()),
        },
    )
    .unwrap_err();
    assert_eq!(err.code, "T0026");
    assert_eq!(env.lookup("x"), None);
}

#[test]
fn value_less_declaration_still_raises_t0021_on_a_premature_read_at_function_scope() {
    // x: int; return x (no assignment in between) -- the declared type
    // must never satisfy `lookup`, so this stays T0021, exactly as an
    // ordinary local without any annotation would.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "f".to_string(),
                params: vec![],
                return_ty: Ty::Int,
                body: vec![
                    HirStmt::AnnAssign {
                        is_final: false,
                        target: "x".to_string(),
                        annotation: Ty::Int,
                        value: None,
                    },
                    HirStmt::Return(Some(HirExpr::Name("x".to_string()))),
                ],
            },
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
                callee: "print".to_string(),
                args: vec![HirExpr::Call {
                    callee: "f".to_string(),
                    args: vec![],
                }],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(err.message, "local name `x` is not bound before this use");
}

#[test]
fn repeated_compatible_value_less_declaration_then_assignment_is_accepted() {
    // x: int; x: int; x = 1 -- a second declaration compatible with the
    // first is accepted (first-declaration-wins), and the later plain
    // assignment still binds the declared type.
    let mut env = Environment::new();
    check_stmt(
        &mut env,
        &HirStmt::AnnAssign {
            is_final: false,
            target: "x".to_string(),
            annotation: Ty::Int,
            value: None,
        },
    )
    .unwrap();
    check_stmt(
        &mut env,
        &HirStmt::AnnAssign {
            is_final: false,
            target: "x".to_string(),
            annotation: Ty::Int,
            value: None,
        },
    )
    .unwrap();
    check_stmt(
        &mut env,
        &HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::IntLiteral(1),
        },
    )
    .unwrap();
    assert_eq!(env.lookup("x"), Some(Ty::Int));
}

#[test]
fn repeated_incompatible_value_less_declaration_is_t0026() {
    // x: int; x: str -- flatly incompatible re-declarations are rejected
    // at the second declaration, before any assignment is even seen.
    let mut env = Environment::new();
    check_stmt(
        &mut env,
        &HirStmt::AnnAssign {
            is_final: false,
            target: "x".to_string(),
            annotation: Ty::Int,
            value: None,
        },
    )
    .unwrap();
    let err = check_stmt(
        &mut env,
        &HirStmt::AnnAssign {
            is_final: false,
            target: "x".to_string(),
            annotation: Ty::Str,
            value: None,
        },
    )
    .unwrap_err();
    assert_eq!(err.code, "T0026");
    assert_eq!(env.lookup("x"), None);
}

#[test]
fn valued_redeclaration_matching_an_earlier_value_less_declaration_keeps_the_declared_type() {
    // x: int; x: bool = True -- the *earlier* declaration wins as the
    // sticky representation (Int), since bool is merely assignable to
    // int, not the later annotation (matches check_assignment's own
    // worked bool/int example in its declared-consult branch).
    let mut env = Environment::new();
    check_stmt(
        &mut env,
        &HirStmt::AnnAssign {
            is_final: false,
            target: "x".to_string(),
            annotation: Ty::Int,
            value: None,
        },
    )
    .unwrap();
    check_stmt(
        &mut env,
        &HirStmt::AnnAssign {
            is_final: false,
            target: "x".to_string(),
            annotation: Ty::Bool,
            value: Some(HirExpr::BoolLiteral(true)),
        },
    )
    .unwrap();
    assert_eq!(env.lookup("x"), Some(Ty::Int));
}

#[test]
fn valued_redeclaration_disagreeing_with_an_earlier_value_less_declaration_is_t0026() {
    // x: int; x: str = "hello" -- T0025 passes (the initializer matches
    // its own `str` annotation), so this must be rejected by the
    // declared-consult step with T0026, not silently accepted.
    let mut env = Environment::new();
    check_stmt(
        &mut env,
        &HirStmt::AnnAssign {
            is_final: false,
            target: "x".to_string(),
            annotation: Ty::Int,
            value: None,
        },
    )
    .unwrap();
    let err = check_stmt(
        &mut env,
        &HirStmt::AnnAssign {
            is_final: false,
            target: "x".to_string(),
            annotation: Ty::Str,
            value: Some(HirExpr::StringLiteral("hello".to_string())),
        },
    )
    .unwrap_err();
    assert_eq!(err.code, "T0026");
    assert_eq!(env.lookup("x"), None);
}

#[test]
fn value_less_declaration_after_an_existing_binding_is_a_no_op() {
    // x = 1; x: int -- re-declaring an already-bound name is unchanged,
    // pre-existing, out-of-scope behavior: `declare` must not shadow the
    // real binding with an inert `declared` entry.
    let mut env = Environment::new();
    check_stmt(
        &mut env,
        &HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::IntLiteral(1),
        },
    )
    .unwrap();
    check_stmt(
        &mut env,
        &HirStmt::AnnAssign {
            is_final: false,
            target: "x".to_string(),
            annotation: Ty::Int,
            value: None,
        },
    )
    .unwrap();
    assert_eq!(env.lookup("x"), Some(Ty::Int));
}

#[test]
fn function_scope_value_less_declaration_then_matching_assignment_binds_the_declared_type() {
    let mut env = Environment::new();
    check_stmt_in_function(
        &mut env,
        &["x"],
        &HirStmt::AnnAssign {
            is_final: false,
            target: "x".to_string(),
            annotation: Ty::Int,
            value: None,
        },
        Ty::None,
    )
    .unwrap();
    check_stmt_in_function(
        &mut env,
        &["x"],
        &HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::IntLiteral(1),
        },
        Ty::None,
    )
    .unwrap();
    assert_eq!(env.lookup("x"), Some(Ty::Int));
}

#[test]
fn function_scope_value_less_declaration_then_mismatched_assignment_is_t0026() {
    let mut env = Environment::new();
    check_stmt_in_function(
        &mut env,
        &["x"],
        &HirStmt::AnnAssign {
            is_final: false,
            target: "x".to_string(),
            annotation: Ty::Int,
            value: None,
        },
        Ty::None,
    )
    .unwrap();
    let err = check_stmt_in_function(
        &mut env,
        &["x"],
        &HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::StringLiteral("hello".to_string()),
        },
        Ty::None,
    )
    .unwrap_err();
    assert_eq!(err.code, "T0026");
    assert_eq!(env.lookup("x"), None);
}

#[test]
fn function_scope_repeated_incompatible_value_less_declaration_is_t0026() {
    let mut env = Environment::new();
    check_stmt_in_function(
        &mut env,
        &["x"],
        &HirStmt::AnnAssign {
            is_final: false,
            target: "x".to_string(),
            annotation: Ty::Int,
            value: None,
        },
        Ty::None,
    )
    .unwrap();
    let err = check_stmt_in_function(
        &mut env,
        &["x"],
        &HirStmt::AnnAssign {
            is_final: false,
            target: "x".to_string(),
            annotation: Ty::Str,
            value: None,
        },
        Ty::None,
    )
    .unwrap_err();
    assert_eq!(err.code, "T0026");
    assert_eq!(env.lookup("x"), None);
}

#[test]
fn child_for_function_clears_a_declared_entry_for_a_shadowed_local_name() {
    // A module-level value-less declaration for `x` must not leak into
    // an unrelated function's own local `x` -- mirrors the existing
    // `bindings`-clearing behavior `child_for_function` already provides.
    let mut module_env = Environment::new();
    check_stmt(
        &mut module_env,
        &HirStmt::AnnAssign {
            is_final: false,
            target: "x".to_string(),
            annotation: Ty::Int,
            value: None,
        },
    )
    .unwrap();
    let mut fn_env = module_env.child_for_function(&["x"]);
    // The function's own body assigns a `str` to its local `x` -- if the
    // module-level `declared` entry leaked through, this would wrongly
    // fail with T0026 against the stale `Int` declaration.
    check_stmt_in_function(
        &mut fn_env,
        &["x"],
        &HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::StringLiteral("hello".to_string()),
        },
        Ty::None,
    )
    .unwrap();
    assert_eq!(fn_env.lookup("x"), Some(Ty::Str));
}

#[test]
fn function_scope_annotated_assignment_with_a_matching_value_binds_the_annotation_type() {
    let mut env = Environment::new();
    check_stmt_in_function(
        &mut env,
        &["x"],
        &HirStmt::AnnAssign {
            is_final: false,
            target: "x".to_string(),
            annotation: Ty::Int,
            value: Some(HirExpr::BoolLiteral(true)),
        },
        Ty::None,
    )
    .unwrap();
    assert_eq!(env.lookup("x"), Some(Ty::Int));
}

#[test]
fn function_scope_annotated_assignment_with_a_mismatched_value_is_t0025() {
    let mut env = Environment::new();
    let err = check_stmt_in_function(
        &mut env,
        &["x"],
        &HirStmt::AnnAssign {
            is_final: false,
            target: "x".to_string(),
            annotation: Ty::Int,
            value: Some(HirExpr::StringLiteral("nope".to_string())),
        },
        Ty::None,
    )
    .unwrap_err();
    assert_eq!(err.code, "T0025");
    assert_eq!(env.lookup("x"), None);
}

#[test]
fn function_scope_annotated_assignment_propagates_an_error_from_the_initializer_expression() {
    // Same as the module-scope sibling above, but through
    // `infer_expr_in`'s local-vs-global unbound distinction: `x` is a
    // declared local that was never assigned, so referencing it in `y`'s
    // initializer must propagate T0021 via the new arm's `?` rather than
    // silently falling through to the is_assignable check.
    let mut env = Environment::new();
    let err = check_stmt_in_function(
        &mut env,
        &["x"],
        &HirStmt::AnnAssign {
            is_final: false,
            target: "y".to_string(),
            annotation: Ty::Int,
            value: Some(HirExpr::Name("x".to_string())),
        },
        Ty::None,
    )
    .unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(env.lookup("y"), None);
}

#[test]
fn function_scope_annotation_only_declaration_does_not_bind_a_value() {
    let mut env = Environment::new();
    check_stmt_in_function(
        &mut env,
        &["x"],
        &HirStmt::AnnAssign {
            is_final: false,
            target: "x".to_string(),
            annotation: Ty::Int,
            value: None,
        },
        Ty::None,
    )
    .unwrap();
    assert_eq!(env.lookup("x"), None);
}

#[test]
fn an_annotation_only_declaration_does_not_bind_a_value() {
    // x: int alone must not make a later use of x succeed -- it only
    // declares x local, it does not bind it (matching CPython's own
    // UnboundLocalError for this exact shape, verified during planning;
    // this end-to-end module mirrors
    // tests/diagnostics/d0026_annotation_only_unbound.py exactly).
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "f".to_string(),
                params: vec![],
                return_ty: Ty::None,
                body: vec![
                    HirStmt::AnnAssign {
                        is_final: false,
                        target: "x".to_string(),
                        annotation: Ty::Int,
                        value: None,
                    },
                    HirStmt::ExprStmt(HirExpr::Call {
                        callee: "print".to_string(),
                        args: vec![HirExpr::Name("x".to_string())],
                    }),
                ],
            },
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
                callee: "f".to_string(),
                args: vec![],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    // Pin the exact message, not just the shared "T0021" code: this test's
    // entire purpose is proving the value-less form reaches the
    // *local-unbound* T0021 path (`unbound_local`), not the differently
    // worded *undefined-name* T0021 path -- both share the same code, so
    // only the message distinguishes them.
    assert_eq!(err.message, "local name `x` is not bound before this use");
}

#[test]
fn an_annotated_re_declaration_that_conflicts_with_an_existing_binding_is_t0023() {
    // `x`'s representation is fixed by its first assignment (Int) --
    // `pycc_mir`'s own `bind_variable` keeps that first type regardless of
    // a later compatible reassignment, so the checker must reject a later
    // annotated re-declaration that would otherwise silently repoint `x`
    // at an incompatible representation (here, `str`) via `is_assignable`
    // trivially comparing the initializer to its own annotation alone.
    let mut env = Environment::new();
    check_stmt(
        &mut env,
        &HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::IntLiteral(1),
        },
    )
    .unwrap();
    let err = check_stmt(
        &mut env,
        &HirStmt::AnnAssign {
            is_final: false,
            target: "x".to_string(),
            annotation: Ty::Str,
            value: Some(HirExpr::StringLiteral("s".to_string())),
        },
    )
    .unwrap_err();
    assert_eq!(err.code, "T0023");
    assert_eq!(env.lookup("x"), Some(Ty::Int));
}

#[test]
fn function_scope_annotated_re_declaration_that_conflicts_with_an_existing_binding_is_t0023() {
    let mut env = Environment::new();
    check_stmt_in_function(
        &mut env,
        &["x"],
        &HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::IntLiteral(1),
        },
        Ty::None,
    )
    .unwrap();
    let err = check_stmt_in_function(
        &mut env,
        &["x"],
        &HirStmt::AnnAssign {
            is_final: false,
            target: "x".to_string(),
            annotation: Ty::Str,
            value: Some(HirExpr::StringLiteral("s".to_string())),
        },
        Ty::None,
    )
    .unwrap_err();
    assert_eq!(err.code, "T0023");
    assert_eq!(env.lookup("x"), Some(Ty::Int));
}

#[test]
fn assigning_bool_to_an_int_binding_keeps_the_declared_representation() {
    let mut env = Environment::new();
    check_stmt(
        &mut env,
        &HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::IntLiteral(1),
        },
    )
    .unwrap();
    check_stmt(
        &mut env,
        &HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::BoolLiteral(true),
        },
    )
    .unwrap();
    assert_eq!(env.lookup("x"), Some(Ty::Int));
}

#[test]
fn a_for_target_cannot_change_an_existing_binding_representation() {
    let mut env = Environment::new();
    env.bind("value".to_string(), Ty::Str);
    let err = check_stmt(
        &mut env,
        &HirStmt::ForRange {
            var: "value".to_string(),
            start: HirExpr::IntLiteral(0),
            stop: HirExpr::IntLiteral(3),
            step: HirExpr::IntLiteral(1),
            body: vec![],
        },
    )
    .unwrap_err();
    assert_eq!(err.code, "T0023");
    assert_eq!(env.lookup("value"), Some(Ty::Str));
}

#[test]
fn a_for_target_cannot_change_a_parameter_representation() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "loop_over".to_string(),
            params: vec![("value".to_string(), Ty::Str)],
            return_ty: Ty::None,
            body: vec![HirStmt::ForRange {
                var: "value".to_string(),
                start: HirExpr::IntLiteral(0),
                stop: HirExpr::IntLiteral(3),
                step: HirExpr::IntLiteral(1),
                body: vec![],
            }],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert_eq!(check(&hir).unwrap_err().code, "T0023");
}

#[test]
fn direct_function_check_rejects_a_for_target_representation_change() {
    let function = HirItem::Function {
        name: "loop_over".to_string(),
        params: vec![("value".to_string(), Ty::Str)],
        return_ty: Ty::None,
        body: vec![HirStmt::ForRange {
            var: "value".to_string(),
            start: HirExpr::IntLiteral(0),
            stop: HirExpr::IntLiteral(3),
            step: HirExpr::IntLiteral(1),
            body: vec![],
        }],
    };
    assert_eq!(check_function(&function).unwrap_err().code, "T0023");
}

#[test]
fn a_private_for_target_infers_an_unannotated_parameter_as_int() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_loop".to_string(),
            params: vec![("value".to_string(), Ty::Infer)],
            return_ty: Ty::None,
            body: vec![HirStmt::ForRange {
                var: "value".to_string(),
                start: HirExpr::IntLiteral(0),
                stop: HirExpr::IntLiteral(3),
                step: HirExpr::IntLiteral(1),
                body: vec![],
            }],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let resolved = check_and_resolve(&hir).unwrap();
    assert_eq!(
        resolved.items[0],
        HirItem::Function {
            name: "_loop".to_string(),
            params: vec![("value".to_string(), Ty::Int)],
            return_ty: Ty::None,
            body: vec![HirStmt::ForRange {
                var: "value".to_string(),
                start: HirExpr::IntLiteral(0),
                stop: HirExpr::IntLiteral(3),
                step: HirExpr::IntLiteral(1),
                body: vec![],
            }],
        }
    );
}
