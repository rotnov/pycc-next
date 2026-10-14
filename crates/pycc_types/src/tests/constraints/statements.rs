//! Statement-level constraint collection: loops, dict/set/tuple literals, `try*`, `match`.

use super::*;

#[test]
fn collect_block_constraints_gives_a_for_list_loop_variable_a_fresh_term_when_unbound() {
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment::empty(&["i"]);
    let body = vec![HirStmt::ForList {
        var: "i".to_string(),
        list: "lst".to_string(),
        body: vec![],
    }];

    collect_block_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut constraints,
        &mut env,
        &body,
        None,
    )
    .unwrap();

    assert!(env.bindings.contains_key("i"));
}
#[test]
fn collect_block_constraints_keeps_a_for_list_loop_variable_s_existing_term() {
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment {
        bindings: HashMap::from([("i".to_string(), Ok(Ty::Int))]),
        ..ConstraintEnvironment::empty(&["i"])
    };
    let body = vec![HirStmt::ForList {
        var: "i".to_string(),
        list: "lst".to_string(),
        body: vec![],
    }];

    collect_block_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut constraints,
        &mut env,
        &body,
        None,
    )
    .unwrap();

    assert_eq!(env.bindings.get("i"), Some(&Ok(Ty::Int)));
}
#[test]
fn collect_block_constraints_propagates_an_error_from_a_for_list_loop_body() {
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment::empty(&["i", "z"]);
    let body = vec![HirStmt::ForList {
        var: "i".to_string(),
        list: "lst".to_string(),
        body: vec![HirStmt::ExprStmt(HirExpr::Name("z".to_string()))],
    }];

    let err = collect_block_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut constraints,
        &mut env,
        &body,
        None,
    )
    .unwrap_err();

    assert_eq!(err.code, "T0021");
}
#[test]
fn constraint_collection_leaves_top_level_return_to_validation() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Return(Some(HirExpr::IntLiteral(1)))),
            HirItem::Function {
                name: "_constant".to_string(),
                params: vec![],
                return_ty: Ty::Infer,
                body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(1)))],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    assert_eq!(check(&hir).unwrap_err().code, "T0024");
}
#[test]
fn constraint_collection_treats_a_dict_literal_as_unconstrained_but_recurses_into_pairs() {
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut binops = DeferredConstraints::default();
    let env = ConstraintEnvironment::empty(&[]);
    let expr = HirExpr::DictLiteral(vec![(
        HirExpr::StringLiteral("a".to_string()),
        HirExpr::IntLiteral(1),
    )]);

    let term = collect_expr_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut binops,
        &env,
        &expr,
    )
    .unwrap();

    assert!(term.is_none());
}
#[test]
fn constraint_collection_propagates_an_error_from_a_dict_literal_key() {
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut binops = DeferredConstraints::default();
    let env = ConstraintEnvironment::empty(&["missing"]);
    let expr = HirExpr::DictLiteral(vec![(
        HirExpr::Name("missing".to_string()),
        HirExpr::IntLiteral(1),
    )]);

    let err = collect_expr_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut binops,
        &env,
        &expr,
    )
    .unwrap_err();

    assert_eq!(err.code, "T0021");
}
#[test]
fn constraint_collection_propagates_an_error_from_a_dict_literal_value() {
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut binops = DeferredConstraints::default();
    let env = ConstraintEnvironment::empty(&["missing"]);
    let expr = HirExpr::DictLiteral(vec![(
        HirExpr::StringLiteral("a".to_string()),
        HirExpr::Name("missing".to_string()),
    )]);

    let err = collect_expr_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut binops,
        &env,
        &expr,
    )
    .unwrap_err();

    assert_eq!(err.code, "T0021");
}
#[test]
fn collect_block_constraints_recurses_into_a_dict_set_s_key_and_value() {
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment::empty(&[]);
    let body = vec![HirStmt::DictSet {
        dict: "x".to_string(),
        key: HirExpr::StringLiteral("a".to_string()),
        value: HirExpr::IntLiteral(1),
    }];

    collect_block_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut constraints,
        &mut env,
        &body,
        None,
    )
    .unwrap();
}
#[test]
fn collect_block_constraints_propagates_an_error_from_a_dict_set_key() {
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment::empty(&["missing"]);
    let body = vec![HirStmt::DictSet {
        dict: "x".to_string(),
        key: HirExpr::Name("missing".to_string()),
        value: HirExpr::IntLiteral(1),
    }];

    let err = collect_block_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut constraints,
        &mut env,
        &body,
        None,
    )
    .unwrap_err();

    assert_eq!(err.code, "T0021");
}
#[test]
fn collect_block_constraints_propagates_an_error_from_a_dict_set_value() {
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment::empty(&["missing"]);
    let body = vec![HirStmt::DictSet {
        dict: "x".to_string(),
        key: HirExpr::StringLiteral("a".to_string()),
        value: HirExpr::Name("missing".to_string()),
    }];

    let err = collect_block_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut constraints,
        &mut env,
        &body,
        None,
    )
    .unwrap_err();

    assert_eq!(err.code, "T0021");
}
/// Part 3 of #382 (#542, PEP 654): `collect_block_constraints`'s `TryStar`
/// arm collects constraints from all four of its blocks (body, handler
/// body, `else`, `finally`) and, when a handler names its binding, seeds it
/// as `ExceptionGroup` -- exactly like `check_try_star_stmt` does at
/// type-checking time. Nothing else in this file calls
/// `collect_block_constraints` (directly or via `infer_function_signatures_
/// with_solver`) with a `TryStar` body, so without this test the entire arm
/// -- including the named-handler-binding branch -- goes unexercised.
#[test]
fn collect_block_constraints_recurses_into_every_try_star_block_and_binds_a_named_handler() {
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment::empty(&[]);
    let body = vec![HirStmt::TryStar {
        body: vec![HirStmt::ExprStmt(HirExpr::IntLiteral(1))],
        handlers: vec![pycc_hir::HirExceptHandler {
            exc_type: Some(vec!["ValueError".to_string()]),
            name: Some("e".to_string()),
            body: vec![HirStmt::ExprStmt(HirExpr::IntLiteral(2))],
        }],
        orelse: vec![HirStmt::ExprStmt(HirExpr::IntLiteral(3))],
        finalbody: vec![HirStmt::ExprStmt(HirExpr::IntLiteral(4))],
    }];

    collect_block_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut constraints,
        &mut env,
        &body,
        None,
    )
    .unwrap();
}
/// A `DictSet` whose target name is declared local but never bound
/// (`local_names` claims it but `bindings` never seeds it) is the same
/// unresolved-local-reference shape the two `DictSet`-focused tests above
/// use to reach `T0021`. Placing it in `TryStar`'s own `body` block reaches
/// the first of that arm's four `collect_block_constraints(...)?` call
/// sites and proves the `?` actually propagates the body block's error
/// instead of only ever seeing `Ok` there.
#[test]
fn collect_block_constraints_propagates_an_error_from_a_try_star_body_block() {
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment::empty(&["missing"]);
    let body = vec![HirStmt::TryStar {
        body: vec![HirStmt::DictSet {
            dict: "x".to_string(),
            key: HirExpr::Name("missing".to_string()),
            value: HirExpr::IntLiteral(1),
        }],
        handlers: vec![],
        orelse: vec![],
        finalbody: vec![],
    }];

    let err = collect_block_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut constraints,
        &mut env,
        &body,
        None,
    )
    .unwrap_err();

    assert_eq!(err.code, "T0021");
}
/// Same unresolved-local-reference shape as above, but placed in a
/// `TryStar` handler's body instead -- reaches the arm's second
/// `collect_block_constraints(...)?` call site (the per-handler loop).
#[test]
fn collect_block_constraints_propagates_an_error_from_a_try_star_handler_block() {
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment::empty(&["missing"]);
    let body = vec![HirStmt::TryStar {
        body: vec![],
        handlers: vec![pycc_hir::HirExceptHandler {
            exc_type: Some(vec!["ValueError".to_string()]),
            name: None,
            body: vec![HirStmt::DictSet {
                dict: "x".to_string(),
                key: HirExpr::Name("missing".to_string()),
                value: HirExpr::IntLiteral(1),
            }],
        }],
        orelse: vec![],
        finalbody: vec![],
    }];

    let err = collect_block_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut constraints,
        &mut env,
        &body,
        None,
    )
    .unwrap_err();

    assert_eq!(err.code, "T0021");
}
/// Same shape again, placed in `TryStar`'s `else` block -- reaches the
/// arm's third `collect_block_constraints(...)?` call site.
#[test]
fn collect_block_constraints_propagates_an_error_from_a_try_star_else_block() {
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment::empty(&["missing"]);
    let body = vec![HirStmt::TryStar {
        body: vec![],
        handlers: vec![],
        orelse: vec![HirStmt::DictSet {
            dict: "x".to_string(),
            key: HirExpr::Name("missing".to_string()),
            value: HirExpr::IntLiteral(1),
        }],
        finalbody: vec![],
    }];

    let err = collect_block_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut constraints,
        &mut env,
        &body,
        None,
    )
    .unwrap_err();

    assert_eq!(err.code, "T0021");
}
/// Issue #771 join-site follow-up, extended to `TryStar`'s own
/// `pre_existing` snapshot in the same commit that adds this test: `y` is
/// already opaquely bound *before* the `try*` (e.g. from `y = d.get("a",
/// 0)` two lines above), then reassigned to a real, solver-representable
/// term inside the `try*` body only. Mirrors
/// `solver_if_reassigns_pre_existing_opaque_binding_in_one_branch_only`'s
/// scenario but through the `TryStar` arm's `join_loop_body_solver` call
/// instead of `If`'s `join_if_branches_solver` -- before this fix,
/// `TryStar`'s `pre_existing` was computed as `env.bindings.keys()` alone
/// (unlike every sibling join site, including the `Try` arm immediately
/// above it, which all `.chain(env.opaque_bindings.iter())`), so `y` looked
/// "newly introduced" to the join helper and was wrongly folded into
/// `env.maybe_bindings` even though it was already definitely (if opaquely)
/// bound beforehand.
#[test]
fn collect_block_constraints_try_star_body_reassigns_pre_existing_opaque_binding() {
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment {
        opaque_bindings: HashSet::from(["y".to_string()]),
        ..ConstraintEnvironment::empty(&["y"])
    };
    let body = vec![HirStmt::TryStar {
        body: vec![HirStmt::Assign {
            target: "y".to_string(),
            value: HirExpr::IntLiteral(1),
        }],
        handlers: vec![],
        orelse: vec![],
        finalbody: vec![],
    }];

    collect_block_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut constraints,
        &mut env,
        &body,
        None,
    )
    .unwrap();

    // `y` must never become newly *maybe* bound: it was already definitely
    // (if opaquely) bound before the `try*`, and the body reassigning it
    // does not retroactively make a pre-existing name conditional.
    assert!(!env.maybe_bindings.contains("y"));
}
/// Same shape again, placed in `TryStar`'s `finally` block -- reaches the
/// arm's fourth and last `collect_block_constraints(...)?` call site.
#[test]
fn collect_block_constraints_propagates_an_error_from_a_try_star_finally_block() {
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment::empty(&["missing"]);
    let body = vec![HirStmt::TryStar {
        body: vec![],
        handlers: vec![],
        orelse: vec![],
        finalbody: vec![HirStmt::DictSet {
            dict: "x".to_string(),
            key: HirExpr::Name("missing".to_string()),
            value: HirExpr::IntLiteral(1),
        }],
    }];

    let err = collect_block_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut constraints,
        &mut env,
        &body,
        None,
    )
    .unwrap_err();

    assert_eq!(err.code, "T0021");
}
#[test]
fn constraint_collection_treats_a_set_literal_as_unconstrained_but_recurses_into_elements() {
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut binops = DeferredConstraints::default();
    let env = ConstraintEnvironment::empty(&[]);
    let expr = HirExpr::SetLiteral(vec![HirExpr::IntLiteral(1)]);

    let term = collect_expr_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut binops,
        &env,
        &expr,
    )
    .unwrap();

    assert!(term.is_none());
}
#[test]
fn constraint_collection_propagates_an_error_from_a_set_literal_element() {
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut binops = DeferredConstraints::default();
    let env = ConstraintEnvironment::empty(&["missing"]);
    let expr = HirExpr::SetLiteral(vec![HirExpr::Name("missing".to_string())]);

    let err = collect_expr_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut binops,
        &env,
        &expr,
    )
    .unwrap_err();

    assert_eq!(err.code, "T0021");
}
#[test]
fn constraint_collection_types_a_tuple_literal_of_concrete_scalars_and_recurses_into_elements() {
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut binops = DeferredConstraints::default();
    let env = ConstraintEnvironment::empty(&[]);
    let expr = HirExpr::TupleLiteral(vec![HirExpr::IntLiteral(1)]);

    let term = collect_expr_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut binops,
        &env,
        &expr,
    )
    .unwrap();

    assert_eq!(term, Some(Ok(Ty::Tuple(Box::new(vec![Ty::Int])))));
    // A non-scalar element leaves the tuple without a term, as before.
    let expr = HirExpr::TupleLiteral(vec![HirExpr::ListLiteral(vec![HirExpr::IntLiteral(1)])]);
    let term = collect_expr_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut binops,
        &env,
        &expr,
    )
    .unwrap();
    assert!(term.is_none());
}
#[test]
fn constraint_collection_propagates_an_error_from_a_tuple_literal_element() {
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut binops = DeferredConstraints::default();
    let env = ConstraintEnvironment::empty(&["missing"]);
    let expr = HirExpr::TupleLiteral(vec![HirExpr::Name("missing".to_string())]);

    let err = collect_expr_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut binops,
        &env,
        &expr,
    )
    .unwrap_err();

    assert_eq!(err.code, "T0021");
}

// -- collect_expr_constraints GCI ? path (line 1384) ------------------

#[test]
fn collect_expr_constraints_propagates_error_from_generic_class_instantiate_arg() {
    // Exercises `collect_expr_constraints`'s GCI arm's `?` on the
    // recursive call (line 1384). A private function `_f` with inferred
    // types whose body contains a GCI whose argument is a call to a
    // non-callable binding (`y = 1` then `y()`). The solver path calls
    // `collect_expr_constraints`, which recurses into the GCI's args;
    // the Call arm sees `y` in `env.bindings` and returns
    // `non_callable_binding`, which propagates through the GCI arm's `?`.
    let self_ty = Ty::Instance(Box::new("C".to_string()));
    let init = HirItem::Function {
        name: "C.__init__".to_string(),
        params: vec![
            ("self".to_string(), self_ty),
            ("x".to_string(), Ty::Param(Box::new("T".to_string()))),
        ],
        return_ty: Ty::None,
        body: vec![HirStmt::AttrSet {
            base: HirExpr::Name("self".to_string()),
            attr: "x".to_string(),
            value: HirExpr::Name("x".to_string()),
        }],
    };
    let class_def = HirClassDef {
        class_attrs: Vec::new(),
        exception_type_tag: None,
        name: "C".to_string(),
        bases: Vec::new(),
        mro: vec!["C".to_string()],
        attrs: vec![("x".to_string(), Ty::Param(Box::new("T".to_string())))],
        methods: vec![("__init__".to_string(), "C.__init__".to_string())],
        type_param: Some("T".to_string()),
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
    };
    // Private function with inferred types — forces the solver path.
    let f = HirItem::Function {
        name: "_f".to_string(),
        params: vec![],
        return_ty: Ty::Infer,
        body: vec![
            HirStmt::ExprStmt(HirExpr::GenericClassInstantiate {
                class: "C".to_string(),
                type_arg: Ty::Int,
                args: vec![HirExpr::Call {
                    callee: "y".to_string(),
                    args: vec![],
                }],
            }),
            HirStmt::Return(None),
        ],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            init,
            // Top-level assignment creates a non-callable binding `y`
            // that the solver's first pass adds to `globals.bindings`.
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "y".to_string(),
                value: HirExpr::IntLiteral(1),
            }),
            f,
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: vec![("C".to_string(), class_def)],
    };
    // `check` should fail — the solver's `collect_expr_constraints`
    // sees `y` in `env.bindings` (a non-callable int binding) and
    // returns `non_callable_binding`, which propagates through the
    // GCI arm's `?` at line 1384.
    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("non-callable"));
}
#[test]
fn collect_block_constraints_propagates_error_from_match_subject() {
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment::empty(&["z"]);
    let body = vec![HirStmt::Match {
        subject: HirExpr::Name("z".to_string()),
        cases: vec![HirMatchCase {
            pattern: HirPattern::Wildcard,
            guard: None,
            body: vec![],
        }],
    }];
    let err = collect_block_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut constraints,
        &mut env,
        &body,
        None,
    )
    .unwrap_err();
    assert_eq!(err.code, "T0021");
}
#[test]
fn collect_block_constraints_propagates_error_from_match_case_body() {
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment::empty(&["z"]);
    let body = vec![HirStmt::Match {
        subject: HirExpr::IntLiteral(0),
        cases: vec![HirMatchCase {
            pattern: HirPattern::Wildcard,
            guard: None,
            body: vec![HirStmt::ExprStmt(HirExpr::Name("z".to_string()))],
        }],
    }];
    let err = collect_block_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut constraints,
        &mut env,
        &body,
        None,
    )
    .unwrap_err();
    assert_eq!(err.code, "T0021");
}
#[test]
fn collect_block_constraints_propagates_an_error_from_a_logical_not_operand() {
    // #604 (Part 3 of #573): `collect_expr_constraints`'s dedicated `Not`
    // arm (`HirExpr::UnaryOp { op: UnaryOpKind::Not, operand }`) walks its
    // operand with its own `collect_expr_constraints(..., operand)?` call,
    // separate from the generic `UnaryOp` arm exercised elsewhere in this
    // file (`Not` is peeled off before that arm ever runs, since its
    // result is always `Ty::Bool` regardless of the operand's type). A
    // parsed-source forward-reference walrus doesn't reach this specific
    // `?`: `bind_named_expr_targets`'s own pre-pass (same file) rejects an
    // unbound forward reference before `collect_expr_constraints` ever
    // walks the `if` test, exactly like
    // `collect_block_constraints_propagates_an_error_from_the_initializer_
    // expression` above needed a direct `collect_block_constraints` call
    // rather than parseable source for the same reason. `z` is declared
    // local but never bound, so wrapping it in `not z` fails with
    // `unbound_local` (T0021) from the recursive call, and the `?` here
    // must propagate that unchanged.
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment::empty(&["z"]);
    let body = vec![HirStmt::AnnAssign {
        is_final: false,
        target: "y".to_string(),
        annotation: Ty::Bool,
        value: Some(HirExpr::UnaryOp {
            op: UnaryOpKind::Not,
            operand: Box::new(HirExpr::Name("z".to_string())),
        }),
    }];

    let err = collect_block_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut constraints,
        &mut env,
        &body,
        None,
    )
    .unwrap_err();

    assert_eq!(err.code, "T0021");
}
