//! Definite/maybe binding on the constraint solver path.

use super::*;

// ---- Issue #359 (Part 2 of #118): solver definite-assignment tracking ----

#[test]
fn solver_if_no_else_marks_body_only_binding_as_maybe() {
    // `def _f(cond: bool):` with `if cond: x = 1` (no else) — x is
    // maybe-bound. The solver should track x in `maybe_bindings` so
    // `return x` does not unify the return type with x's type.
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment {
        bindings: HashMap::from([("cond".to_string(), Ok(Ty::Bool))]),
        ..ConstraintEnvironment::empty(&["cond", "x"])
    };
    let body = vec![HirStmt::If {
        test: HirExpr::Name("cond".to_string()),
        body: vec![HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::IntLiteral(1),
        }],
        orelse: vec![],
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

    assert!(env.maybe_bindings.contains("x"));
    assert!(env.bindings.contains_key("x"));
}

#[test]
fn solver_if_with_else_marks_both_branch_binding_as_definite() {
    // `if cond: x = 1` + `else: x = 2` — both branches bind x, so x
    // is definitely bound (not in `maybe_bindings`).
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment {
        bindings: HashMap::from([("cond".to_string(), Ok(Ty::Bool))]),
        ..ConstraintEnvironment::empty(&["cond", "x"])
    };
    let body = vec![HirStmt::If {
        test: HirExpr::Name("cond".to_string()),
        body: vec![HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::IntLiteral(1),
        }],
        orelse: vec![HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::IntLiteral(2),
        }],
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

    assert!(!env.maybe_bindings.contains("x"));
    assert!(env.bindings.contains_key("x"));
}

#[test]
fn solver_if_no_else_does_not_leak_binding_into_orelse() {
    // Before #359, both branches shared the same env, so a binding
    // from the body leaked into the orelse. With cloning, the orelse
    // branch should NOT see x from the body.
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment {
        bindings: HashMap::from([("cond".to_string(), Ok(Ty::Bool))]),
        ..ConstraintEnvironment::empty(&["cond", "x", "y"])
    };
    // Body assigns x; orelse assigns y. After the if, both x and y
    // should be maybe-bound (each was introduced by only one branch).
    let body = vec![HirStmt::If {
        test: HirExpr::Name("cond".to_string()),
        body: vec![HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::IntLiteral(1),
        }],
        orelse: vec![HirStmt::Assign {
            target: "y".to_string(),
            value: HirExpr::IntLiteral(2),
        }],
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

    assert!(env.maybe_bindings.contains("x"));
    assert!(env.maybe_bindings.contains("y"));
    assert!(env.bindings.contains_key("x"));
    assert!(env.bindings.contains_key("y"));
}

#[test]
fn solver_if_no_else_marks_opaque_body_only_binding_as_maybe() {
    // Issue #771 join-site follow-up: `if cond: y = d.get("a", 0)` (no
    // else), where `DictGetOrDefault` is one of the constructs whose
    // `collect_expr_constraints` arm returns `Ok(None)` (no solver term).
    // Before `join_if_branches_solver` learned to merge `opaque_bindings`,
    // `y` was dropped from every tracked set by the join (it never enters
    // `body_env.bindings`, only `body_env.opaque_bindings`), so a later
    // read of `y` misfired as an unbound local even though the assignment
    // is textually right above it. It must now be maybe-bound: present in
    // `env.opaque_bindings` and `env.maybe_bindings`, absent from
    // `env.bindings` (the solver still has no term for it).
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment {
        bindings: HashMap::from([
            ("cond".to_string(), Ok(Ty::Bool)),
            ("d".to_string(), Ok(Ty::Dict(Box::new((Ty::Str, Ty::Int))))),
        ]),
        ..ConstraintEnvironment::empty(&["cond", "d", "y"])
    };
    let body = vec![HirStmt::If {
        test: HirExpr::Name("cond".to_string()),
        body: vec![HirStmt::Assign {
            target: "y".to_string(),
            value: HirExpr::DictGetOrDefault {
                dict: pycc_hir::ContainerReceiver::Name("d".to_string()),
                key: Box::new(HirExpr::StringLiteral("a".to_string())),
                default: Box::new(HirExpr::IntLiteral(0)),
            },
        }],
        orelse: vec![],
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

    assert!(env.maybe_bindings.contains("y"));
    assert!(env.opaque_bindings.contains("y"));
    assert!(!env.bindings.contains_key("y"));
}

#[test]
fn solver_if_with_else_marks_both_branch_opaque_binding_as_definite() {
    // Issue #771 join-site follow-up: `if cond: y = d.get(..) else: y =
    // d.get(..)` -- both branches opaquely assign `y`, so it is
    // definitely bound afterward, exactly like the real-term case in
    // `solver_if_with_else_marks_both_branch_binding_as_definite` above.
    // Before the join-site fix this was dropped from every tracked set
    // (not just left "maybe"), which is the case the deep-review pass
    // flagged as not actually branch-conditional in the definite-
    // assignment sense: it reproduced the exact #771 misdiagnosis even
    // though every path through the `if` assigns `y`.
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment {
        bindings: HashMap::from([
            ("cond".to_string(), Ok(Ty::Bool)),
            ("d".to_string(), Ok(Ty::Dict(Box::new((Ty::Str, Ty::Int))))),
        ]),
        ..ConstraintEnvironment::empty(&["cond", "d", "y"])
    };
    let opaque_get = || HirStmt::Assign {
        target: "y".to_string(),
        value: HirExpr::DictGetOrDefault {
            dict: pycc_hir::ContainerReceiver::Name("d".to_string()),
            key: Box::new(HirExpr::StringLiteral("a".to_string())),
            default: Box::new(HirExpr::IntLiteral(0)),
        },
    };
    let body = vec![HirStmt::If {
        test: HirExpr::Name("cond".to_string()),
        body: vec![opaque_get()],
        orelse: vec![opaque_get()],
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

    assert!(!env.maybe_bindings.contains("y"));
    assert!(env.opaque_bindings.contains("y"));
    assert!(!env.bindings.contains_key("y"));
}

#[test]
fn solver_if_reassigns_pre_existing_opaque_binding_in_one_branch_only() {
    // Issue #771 join-site follow-up, second reviewer pass: `y` is already
    // opaquely bound *before* the `if` (e.g. from `y = d.get("a", 0)` two
    // lines above), then reassigned to a real, solver-representable term in
    // only ONE branch (`if cond: y = 1` / no `else`). Before the
    // `pre_existing` fix (computed as `env.bindings.keys()` alone, blind to
    // `env.opaque_bindings`), this name looked "newly introduced" to the
    // join helper on the body side (its `bindings` entry) and also on the
    // orelse side (its untouched `opaque_bindings` entry), so both sides
    // appeared to "introduce" it and it was wrongly folded into
    // `env.maybe_bindings.remove` (i.e. marked *definitely* bound) --
    // exposing the body-only term everywhere, including on the path where
    // `y` was never reassigned. With `pre_existing` now covering
    // `opaque_bindings`, `y` is correctly recognized as already present
    // before the `if` on both sides, so the join's introduced/not-introduced
    // classification leaves its `maybe_bindings` status untouched (it was
    // not maybe before, and reassignment in only one branch does not by
    // itself make a previously-definite name newly maybe under this
    // solver's existing merge semantics for pre-existing names -- the same
    // conservative "leave alone" treatment a pre-existing real-term entry
    // already gets, since `join_if_branches_solver`'s unconditional
    // `entry().or_insert()` merge never overwrites a `bindings` entry that
    // was already populated before the `if`; this test's assertions confirm
    // the analogous protection now holds when the pre-existing entry lives
    // in `opaque_bindings` instead of `bindings`).
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment {
        bindings: HashMap::from([("cond".to_string(), Ok(Ty::Bool))]),
        opaque_bindings: HashSet::from(["y".to_string()]),
        ..ConstraintEnvironment::empty(&["cond", "y"])
    };
    let body = vec![HirStmt::If {
        test: HirExpr::Name("cond".to_string()),
        body: vec![HirStmt::Assign {
            target: "y".to_string(),
            value: HirExpr::IntLiteral(1),
        }],
        orelse: vec![],
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
    // (if opaquely) bound before the `if`, and a one-branch reassignment
    // does not retroactively make a pre-existing name conditional.
    assert!(!env.maybe_bindings.contains("y"));
    // `y` must still be resolvable as opaque afterward -- the whole point
    // of this test is that the orelse (no-op) path's opaque value for `y`
    // is not silently discarded in favor of the body-only real term.
    assert!(env.opaque_bindings.contains("y"));
}

#[test]
fn solver_if_reassigns_pre_existing_opaque_binding_in_orelse_branch_only() {
    // Mirror image of `solver_if_reassigns_pre_existing_opaque_binding_in_one_branch_only`
    // above, exercising the `orelse_env.bindings` merge loop's `pre_existing`
    // guard instead of the `body_env.bindings` one -- `join_if_branches_solver`
    // has two separate, textually near-identical merge loops (one per
    // branch), and only writing a body-side test left the orelse-side
    // `continue` unexercised. Here `y` is again pre-existing-opaque before
    // the `if`, but this time only the `orelse` branch reassigns it to a
    // real term; `body` is empty.
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment {
        bindings: HashMap::from([("cond".to_string(), Ok(Ty::Bool))]),
        opaque_bindings: HashSet::from(["y".to_string()]),
        ..ConstraintEnvironment::empty(&["cond", "y"])
    };
    let body = vec![HirStmt::If {
        test: HirExpr::Name("cond".to_string()),
        body: vec![],
        orelse: vec![HirStmt::Assign {
            target: "y".to_string(),
            value: HirExpr::IntLiteral(1),
        }],
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

    assert!(!env.maybe_bindings.contains("y"));
    assert!(env.opaque_bindings.contains("y"));
}

#[test]
fn solver_if_reassigns_pre_existing_opaque_binding_in_both_branches_to_real_terms() {
    // Issue #771 join-site follow-up, THIRD reviewer pass: `y` is
    // pre-existing-opaque before the `if`, and this time BOTH branches
    // reassign it to a real, solver-representable term (`if cond: y = 1
    // else: y = 2`). Unlike the single-branch siblings above, this is not
    // branch-conditional at all -- every path assigns `y` a concrete type --
    // so it must end up definitely bound with a real term, not left opaque.
    // The naive form of the `pre_existing` guard added for the single-branch
    // case (skip whenever the name is pre-existing-opaque, regardless of
    // what the other branch did) incorrectly fired in BOTH merge loops here,
    // since neither loop had inserted into `env.bindings` yet when the other
    // ran. Because `env.opaque_bindings` was never cleared by the join (the
    // name was already present there before the `if`, and the opaque-merge
    // loop only ever inserts), the skipped real terms did not reproduce
    // `unbound_local` -- `HirExpr::Name` still resolved `y` via the
    // surviving stale opaque marker, silently masking both branches' actual
    // concrete terms back down to "opaque, no term available" instead. The
    // fix consults the *other* branch's own `bindings` map instead of
    // `env.bindings`'s own mutation state, so this case correctly falls
    // through to the ordinary first-wins merge and `y` ends up with a real,
    // solver-representable term as it should.
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment {
        bindings: HashMap::from([("cond".to_string(), Ok(Ty::Bool))]),
        opaque_bindings: HashSet::from(["y".to_string()]),
        ..ConstraintEnvironment::empty(&["cond", "y"])
    };
    let body = vec![HirStmt::If {
        test: HirExpr::Name("cond".to_string()),
        body: vec![HirStmt::Assign {
            target: "y".to_string(),
            value: HirExpr::IntLiteral(1),
        }],
        orelse: vec![HirStmt::Assign {
            target: "y".to_string(),
            value: HirExpr::IntLiteral(2),
        }],
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

    // `y` must not become newly maybe-bound: every path through the `if`
    // assigns it, so it stays definitely bound.
    assert!(!env.maybe_bindings.contains("y"));
    // `y` must be resolvable as a real term afterward -- it must not be
    // dropped from every tracked set, which would silently mask both
    // branches' concrete terms back down to "opaque, no term available"
    // (see the comment above).
    assert!(env.bindings.contains_key("y"));
    // `env.opaque_bindings` still carries a stale "y" marker here: the join
    // never removes from that set, only inserts into it, and this name was
    // already present there before the `if`. This is harmless only because
    // `HirExpr::Name`'s lookup checks `env.bindings` first and a present
    // real term always wins over a stale opaque marker for the same name
    // (see the lookup-order comment on `HirExpr::Name` in constraints.rs and
    // the merge-loop comment on `join_if_branches_solver` in solver.rs).
    // Pinned explicitly so a future change to that lookup order would be
    // caught here rather than silently reintroducing a masking regression.
    assert!(env.opaque_bindings.contains("y"));
}

#[test]
fn solver_while_loop_marks_opaque_body_only_binding_as_maybe() {
    // Issue #771 join-site follow-up: a `while` body may execute zero
    // times, so `y` assigned only in the body via an opaque initializer
    // is maybe-bound after the loop, exactly like a real-term binding
    // introduced there (`solver_while_loop_marks_body_only_binding_as_maybe`
    // above). Exercises `join_loop_body_solver`'s `opaque_bindings` merge.
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment {
        bindings: HashMap::from([
            ("cond".to_string(), Ok(Ty::Bool)),
            ("d".to_string(), Ok(Ty::Dict(Box::new((Ty::Str, Ty::Int))))),
        ]),
        ..ConstraintEnvironment::empty(&["cond", "d", "y"])
    };
    let body = vec![HirStmt::While {
        test: HirExpr::Name("cond".to_string()),
        body: vec![HirStmt::Assign {
            target: "y".to_string(),
            value: HirExpr::DictGetOrDefault {
                dict: pycc_hir::ContainerReceiver::Name("d".to_string()),
                key: Box::new(HirExpr::StringLiteral("a".to_string())),
                default: Box::new(HirExpr::IntLiteral(0)),
            },
        }],
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

    assert!(env.maybe_bindings.contains("y"));
    assert!(env.opaque_bindings.contains("y"));
    assert!(!env.bindings.contains_key("y"));
}

#[test]
fn solver_while_loop_marks_body_only_binding_as_maybe() {
    // A `while` body may execute zero times, so x assigned only in
    // the body is maybe-bound after the loop.
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment {
        bindings: HashMap::from([("cond".to_string(), Ok(Ty::Bool))]),
        ..ConstraintEnvironment::empty(&["cond", "x"])
    };
    let body = vec![HirStmt::While {
        test: HirExpr::Name("cond".to_string()),
        body: vec![HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::IntLiteral(1),
        }],
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

    assert!(env.maybe_bindings.contains("x"));
    assert!(env.bindings.contains_key("x"));
}

#[test]
fn solver_for_range_marks_loop_variable_as_maybe() {
    // A `for` loop may execute zero times, so the loop variable is
    // maybe-bound after the loop (if newly introduced).
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment::empty(&["i"]);
    let body = vec![HirStmt::ForRange {
        var: "i".to_string(),
        start: HirExpr::IntLiteral(0),
        stop: HirExpr::IntLiteral(3),
        step: HirExpr::IntLiteral(1),
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

    assert!(env.maybe_bindings.contains("i"));
    assert_eq!(env.bindings.get("i"), Some(&Ok(Ty::Int)));
}

#[test]
fn solver_for_range_body_only_binding_is_maybe() {
    // A binding introduced only in the for-loop body is maybe-bound
    // after the loop (the loop may not execute).
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment::empty(&["i", "x"]);
    let body = vec![HirStmt::ForRange {
        var: "i".to_string(),
        start: HirExpr::IntLiteral(0),
        stop: HirExpr::IntLiteral(3),
        step: HirExpr::IntLiteral(1),
        body: vec![HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::IntLiteral(1),
        }],
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

    assert!(env.maybe_bindings.contains("i"));
    assert!(env.maybe_bindings.contains("x"));
}

#[test]
fn solver_for_range_pre_existing_binding_stays_definite() {
    // A binding that existed before the loop stays definite (not in
    // maybe_bindings) after the loop.
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment {
        bindings: HashMap::from([("x".to_string(), Ok(Ty::Int))]),
        ..ConstraintEnvironment::empty(&["i", "x"])
    };
    let body = vec![HirStmt::ForRange {
        var: "i".to_string(),
        start: HirExpr::IntLiteral(0),
        stop: HirExpr::IntLiteral(3),
        step: HirExpr::IntLiteral(1),
        body: vec![HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::IntLiteral(1),
        }],
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

    // x was pre-existing → stays definite (not maybe)
    assert!(!env.maybe_bindings.contains("x"));
    // i was newly introduced → maybe
    assert!(env.maybe_bindings.contains("i"));
}

#[test]
fn solver_maybe_bound_name_skips_unification_in_name_arm() {
    // A maybe-bound name returns Ok(None) from the Name arm, so
    // `return x` does not unify the return type with x's type.
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut binops = DeferredConstraints::default();
    let env = ConstraintEnvironment {
        bindings: HashMap::from([("x".to_string(), Ok(Ty::Int))]),
        maybe_bindings: HashSet::from(["x".to_string()]),
        ..ConstraintEnvironment::empty(&["x"])
    };

    let term = collect_expr_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut binops,
        &env,
        &HirExpr::Name("x".to_string()),
    )
    .unwrap();

    // Ok(None) — the type term is not available for unification
    assert_eq!(term, None);
}

#[test]
fn solver_unconditional_assignment_upgrades_maybe_to_definite() {
    // Issue #359: an unconditional assignment after a maybe-binding
    // join upgrades the name back to definitely bound, mirroring the
    // validation pass's contract: `if c: x = 1; x = 2; return x` is
    // valid because `x = 2` makes `x` definitely bound.
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment {
        bindings: HashMap::from([("cond".to_string(), Ok(Ty::Bool))]),
        ..ConstraintEnvironment::empty(&["cond", "x"])
    };
    let body = vec![
        HirStmt::If {
            test: HirExpr::Name("cond".to_string()),
            body: vec![HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::IntLiteral(1),
            }],
            orelse: vec![],
        },
        // Unconditional assignment upgrades x to definitely bound
        HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::IntLiteral(2),
        },
    ];

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

    // x should NOT be in maybe_bindings after the unconditional assignment
    assert!(!env.maybe_bindings.contains("x"));
    assert!(env.bindings.contains_key("x"));
}

#[test]
fn solver_definitely_bound_name_returns_its_term() {
    // A definitely-bound name (not in maybe_bindings) returns its
    // type term as before.
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut binops = DeferredConstraints::default();
    let env = ConstraintEnvironment {
        bindings: HashMap::from([("x".to_string(), Ok(Ty::Int))]),
        ..ConstraintEnvironment::empty(&["x"])
    };

    let term = collect_expr_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut binops,
        &env,
        &HirExpr::Name("x".to_string()),
    )
    .unwrap();

    assert_eq!(term, Some(Ok(Ty::Int)));
}

#[test]
fn solver_private_helper_with_maybe_bound_return_does_not_infer_wrong_type() {
    // End-to-end: a private helper with `if cond: x = 1; return x`
    // should not infer a wrong return type from the maybe-bound x.
    // The solver conservatively refuses to infer the return type
    // (T0021 "cannot infer return type") because `return x` skips
    // unification when x is maybe-bound. With an explicit annotation,
    // the validation pass catches T0041 separately (see the next test).
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_helper".to_string(),
            params: vec![("cond".to_string(), Ty::Bool)],
            return_ty: Ty::Infer,
            body: vec![
                HirStmt::If {
                    test: HirExpr::Name("cond".to_string()),
                    body: vec![HirStmt::Assign {
                        target: "x".to_string(),
                        value: HirExpr::IntLiteral(1),
                    }],
                    orelse: vec![],
                },
                HirStmt::Return(Some(HirExpr::Name("x".to_string()))),
            ],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    // The solver cannot infer the return type because x is maybe-bound
    // and `return x` skips unification. This is the conservative
    // behavior the issue requires.
    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("cannot infer return type"));
}

#[test]
fn solver_private_helper_with_annotation_catches_t0041_for_maybe_bound() {
    // End-to-end: with an explicit return annotation, the solver
    // doesn't need to infer the return type, so the validation pass
    // runs and catches T0041 for the maybe-bound read of x.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_helper".to_string(),
            params: vec![("cond".to_string(), Ty::Bool)],
            return_ty: Ty::Int,
            body: vec![
                HirStmt::If {
                    test: HirExpr::Name("cond".to_string()),
                    body: vec![HirStmt::Assign {
                        target: "x".to_string(),
                        value: HirExpr::IntLiteral(1),
                    }],
                    orelse: vec![],
                },
                HirStmt::Return(Some(HirExpr::Name("x".to_string()))),
            ],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0041");
}

#[test]
fn solver_private_helper_with_both_branches_binding_infers_correctly() {
    // End-to-end: a private helper with both if-branches binding x
    // should infer the return type correctly (x is definitely bound).
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_helper".to_string(),
            params: vec![("cond".to_string(), Ty::Bool)],
            return_ty: Ty::Infer,
            body: vec![
                HirStmt::If {
                    test: HirExpr::Name("cond".to_string()),
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
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert!(check(&hir).is_ok());
}

#[test]
fn solver_private_helper_with_unconditional_upgrade_infers_correctly() {
    // End-to-end: `if c: x = 1; x = 2; return x` — the unconditional
    // `x = 2` after the maybe-binding `if` upgrades x to definitely
    // bound, so the solver can infer the return type from `return x`.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_helper".to_string(),
            params: vec![("cond".to_string(), Ty::Bool)],
            return_ty: Ty::Infer,
            body: vec![
                HirStmt::If {
                    test: HirExpr::Name("cond".to_string()),
                    body: vec![HirStmt::Assign {
                        target: "x".to_string(),
                        value: HirExpr::IntLiteral(1),
                    }],
                    orelse: vec![],
                },
                HirStmt::Assign {
                    target: "x".to_string(),
                    value: HirExpr::IntLiteral(2),
                },
                HirStmt::Return(Some(HirExpr::Name("x".to_string()))),
            ],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert!(check(&hir).is_ok());
}
