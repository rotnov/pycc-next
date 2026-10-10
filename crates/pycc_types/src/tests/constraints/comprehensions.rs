//! Constraint collection over comprehension assignments.

use super::*;

#[test]
fn collect_block_constraints_treats_a_list_comp_assign_target_as_a_no_op() {
    // PR-12 Task 3 (D-117), Step 5: mirrors the dict/set/tuple-literal
    // solver-gap precedent directly -- `collect_block_constraints`'s
    // `ListCompAssign`/`SetCompAssign`/`DictCompAssign` arm registers no
    // term for `target` at all (though it does, unlike a bare no-op,
    // still recurse into `elt`/`cond` -- see
    // `private_helper_parameter_is_inferred_through_a_comprehension_s_elt`
    // below for that half). An unrelated private helper with an
    // unresolved (`Ty::Infer`) signature elsewhere in the same module
    // leaves a `list[str]` comprehension to the real, comprehension-aware
    // check pass, which admits it.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::ListCompAssign {
                target: "xs".to_string(),
                var: "0comp_11_i".to_string(),
                iter: CompIter::Range {
                    start: HirExpr::IntLiteral(0),
                    stop: HirExpr::IntLiteral(3),
                    step: HirExpr::IntLiteral(1),
                },
                cond: None,
                elt: Box::new(HirExpr::StringLiteral("x".to_string())),
            }),
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
    assert!(check(&hir).is_ok());
}
#[test]
fn collect_block_constraints_unifies_a_range_comprehension_s_stop_forwarded_from_an_unannotated_parameter()
 {
    // Exercises `bind_comp_loop_var`'s `CompIter::Range` operand-check
    // branch (mirrors `ForRange`'s own analogous `start`/`stop`/`step`
    // checks): the comprehension's own `stop` operand is itself an
    // unresolved solver term (an unannotated parameter forwarded
    // directly as the range bound), which this arm must still visit and
    // unify against `Ty::Int` -- exactly like a plain `for i in
    // range(n):` loop's own `ForRange` arm already does.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_h".to_string(),
            params: vec![("n".to_string(), Ty::Infer)],
            return_ty: Ty::None,
            body: vec![
                HirStmt::ListCompAssign {
                    target: "y".to_string(),
                    var: "0comp_11_i".to_string(),
                    iter: CompIter::Range {
                        start: HirExpr::IntLiteral(0),
                        stop: HirExpr::Name("n".to_string()),
                        step: HirExpr::IntLiteral(1),
                    },
                    cond: None,
                    elt: Box::new(HirExpr::IntLiteral(1)),
                },
                HirStmt::Return(None),
            ],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    check(&hir).unwrap();
}
#[test]
fn collect_block_constraints_rejects_a_range_comprehension_whose_stop_was_already_resolved_to_an_incompatible_type()
 {
    // The failure half of the operand-check branch above: `n` is first
    // forwarded into a `str`-typed parameter (resolving its solver term
    // to `Ty::Str`), then reused as the comprehension's own `stop`
    // operand -- `bind_comp_loop_var`'s `unify_terms(term, Ok(Ty::Int),
    // ...)` call must propagate the resulting conflict rather than
    // silently accepting it.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "_sink_str".to_string(),
                params: vec![("value".to_string(), Ty::Str)],
                return_ty: Ty::None,
                body: vec![HirStmt::Return(None)],
            },
            HirItem::Function {
                name: "_h".to_string(),
                params: vec![("n".to_string(), Ty::Infer)],
                return_ty: Ty::None,
                body: vec![
                    HirStmt::ExprStmt(HirExpr::Call {
                        callee: "_sink_str".to_string(),
                        args: vec![HirExpr::Name("n".to_string())],
                    }),
                    HirStmt::ListCompAssign {
                        target: "y".to_string(),
                        var: "0comp_11_i".to_string(),
                        iter: CompIter::Range {
                            start: HirExpr::IntLiteral(0),
                            stop: HirExpr::Name("n".to_string()),
                            step: HirExpr::IntLiteral(1),
                        },
                        cond: None,
                        elt: Box::new(HirExpr::IntLiteral(1)),
                    },
                    HirStmt::Return(None),
                ],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert_eq!(check(&hir).unwrap_err().code, "T0021");
}
#[test]
fn collect_block_constraints_rejects_a_range_comprehension_whose_loop_variable_conflicts_with_an_existing_binding()
 {
    // Exercises `bind_comp_loop_var`'s `CompIter::Range` "existing
    // binding" branch's own *failure* path (mirrors `ForRange`'s own
    // `T0023` branch): the comprehension's loop variable happens, in
    // this hand-built test only, to share a name with the enclosing
    // function's own `str`-typed parameter (real lowering's
    // `synthesize_comp_var_name` never produces such a collision), so
    // `Range`'s `Ty::Int` fact genuinely conflicts with the existing
    // binding.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "_h".to_string(),
                params: vec![("0comp_11_i".to_string(), Ty::Str)],
                return_ty: Ty::None,
                body: vec![
                    HirStmt::ListCompAssign {
                        target: "y".to_string(),
                        var: "0comp_11_i".to_string(),
                        iter: CompIter::Range {
                            start: HirExpr::IntLiteral(0),
                            stop: HirExpr::IntLiteral(3),
                            step: HirExpr::IntLiteral(1),
                        },
                        cond: None,
                        elt: Box::new(HirExpr::IntLiteral(1)),
                    },
                    HirStmt::Return(None),
                ],
            },
            HirItem::Function {
                name: "_trigger".to_string(),
                params: vec![],
                return_ty: Ty::Infer,
                body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(1)))],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert_eq!(check(&hir).unwrap_err().code, "T0023");
}
#[test]
fn collect_block_constraints_gives_a_name_iterable_comprehension_s_loop_variable_a_fresh_term() {
    // Exercises `bind_comp_loop_var`'s `CompIter::Name` branch (mirrors
    // `ForList`'s own analogous branch): this solver doesn't track a
    // list-typed name's element type, so the loop variable just gets an
    // unconstrained fresh term here -- real element-type checking is
    // the subsequent `check_with_signatures_all` pass's job. Every other
    // solver-path comprehension test above uses `CompIter::Range`.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "_h".to_string(),
                params: vec![("xs".to_string(), Ty::List(Box::new(Ty::Int)))],
                return_ty: Ty::None,
                body: vec![
                    HirStmt::ListCompAssign {
                        target: "y".to_string(),
                        var: "0comp_11_i".to_string(),
                        iter: CompIter::Name("xs".to_string()),
                        cond: None,
                        elt: Box::new(HirExpr::Name("0comp_11_i".to_string())),
                    },
                    HirStmt::Return(None),
                ],
            },
            HirItem::Function {
                name: "_trigger".to_string(),
                params: vec![],
                return_ty: Ty::Infer,
                body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(1)))],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    check(&hir).unwrap();
}
#[test]
fn collect_block_constraints_visits_a_set_comp_assign_s_cond_and_elt() {
    // The combined `HirStmt::ListCompAssign | HirStmt::SetCompAssign`
    // solver arm is only exercised with `ListCompAssign` and a `None`
    // `cond` by the tests above -- this pins the `SetCompAssign` half of
    // that pattern and the `cond.is_some()` branch, both distinct
    // coverage regions from the `ListCompAssign`/`cond: None` case.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "_h".to_string(),
                params: vec![],
                return_ty: Ty::None,
                body: vec![
                    HirStmt::SetCompAssign {
                        target: "y".to_string(),
                        var: "0comp_11_i".to_string(),
                        iter: CompIter::Range {
                            start: HirExpr::IntLiteral(0),
                            stop: HirExpr::IntLiteral(3),
                            step: HirExpr::IntLiteral(1),
                        },
                        cond: Some(Box::new(HirExpr::IntLiteral(1))),
                        elt: Box::new(HirExpr::Name("0comp_11_i".to_string())),
                    },
                    HirStmt::Return(None),
                ],
            },
            HirItem::Function {
                name: "_trigger".to_string(),
                params: vec![],
                return_ty: Ty::Infer,
                body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(1)))],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    check(&hir).unwrap();
}
#[test]
fn collect_block_constraints_visits_a_dict_comp_assign_s_cond_key_and_value() {
    // Pins the `HirStmt::DictCompAssign` solver arm (its own separate
    // key/value split, not shared with the list/set arm in `lib.rs`), with a
    // `cond` present so its own `cond.is_some()` branch is covered too.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "_h".to_string(),
                params: vec![],
                return_ty: Ty::None,
                body: vec![
                    HirStmt::DictCompAssign {
                        target: "y".to_string(),
                        var: "0comp_11_i".to_string(),
                        iter: CompIter::Range {
                            start: HirExpr::IntLiteral(0),
                            stop: HirExpr::IntLiteral(3),
                            step: HirExpr::IntLiteral(1),
                        },
                        cond: Some(Box::new(HirExpr::IntLiteral(1))),
                        key: Box::new(HirExpr::StringLiteral("k".to_string())),
                        value: Box::new(HirExpr::Name("0comp_11_i".to_string())),
                    },
                    HirStmt::Return(None),
                ],
            },
            HirItem::Function {
                name: "_trigger".to_string(),
                params: vec![],
                return_ty: Ty::Infer,
                body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(1)))],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    check(&hir).unwrap();
}
#[test]
fn collect_block_constraints_visits_a_dict_comp_assign_s_key_and_value_with_no_cond() {
    // Companion to the test above: pins the `DictCompAssign` solver
    // arm's `cond.is_none()` branch, which that test's `cond: Some(...)`
    // shape never reaches.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "_h".to_string(),
                params: vec![],
                return_ty: Ty::None,
                body: vec![
                    HirStmt::DictCompAssign {
                        target: "y".to_string(),
                        var: "0comp_11_i".to_string(),
                        iter: CompIter::Range {
                            start: HirExpr::IntLiteral(0),
                            stop: HirExpr::IntLiteral(3),
                            step: HirExpr::IntLiteral(1),
                        },
                        cond: None,
                        key: Box::new(HirExpr::StringLiteral("k".to_string())),
                        value: Box::new(HirExpr::Name("0comp_11_i".to_string())),
                    },
                    HirStmt::Return(None),
                ],
            },
            HirItem::Function {
                name: "_trigger".to_string(),
                params: vec![],
                return_ty: Ty::Infer,
                body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(1)))],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    check(&hir).unwrap();
}
#[test]
fn collect_block_constraints_propagates_an_error_from_a_range_comprehension_s_stop_operand() {
    // Pins `bind_comp_loop_var`'s own `collect_expr_constraints(...)?`
    // call for a `CompIter::Range` operand failing outright, distinct
    // from the operand-resolves-but-conflicts tests above. Uses a
    // forward reference to a name assigned *later* in the same body
    // (mirrors `private_helper_inference_rejects_a_read_before_local_assignment`)
    // rather than a plain undefined name: this solver's own
    // `collect_expr_constraints` is deliberately lenient about a
    // non-local undefined name (`None => Ok(None)`, no error -- real
    // "not defined" checking is `infer_expr_in`'s job in the second
    // pass), so only a genuine *local-but-not-yet-bound* read actually
    // triggers this solver's own `unbound_local` error.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_h".to_string(),
            params: vec![],
            return_ty: Ty::None,
            body: vec![
                HirStmt::ListCompAssign {
                    target: "y".to_string(),
                    var: "0comp_11_i".to_string(),
                    iter: CompIter::Range {
                        start: HirExpr::IntLiteral(0),
                        stop: HirExpr::Name("later".to_string()),
                        step: HirExpr::IntLiteral(1),
                    },
                    cond: None,
                    elt: Box::new(HirExpr::IntLiteral(1)),
                },
                HirStmt::Assign {
                    target: "later".to_string(),
                    value: HirExpr::IntLiteral(3),
                },
                HirStmt::Return(None),
            ],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert_eq!(check(&hir).unwrap_err().code, "T0021");
}
#[test]
fn collect_block_constraints_keeps_a_name_iterable_comprehension_s_loop_variable_s_existing_term() {
    // Companion to the "fresh term" test above: pins the
    // `CompIter::Name` branch's `contains_key(var)` guard's *true*
    // branch (variable already bound), mirroring
    // `collect_block_constraints_keeps_a_for_list_loop_variable_s_existing_term`'s
    // own analogous `ForList` coverage. The loop variable is given the
    // same name as a real parameter (`0comp_11_i`), whose own term is
    // seeded into the solver's environment before any statement in the
    // body runs -- unlike a plain `ExprStmt` reference to an
    // undeclared name (which this solver's own local-name tracking
    // would instead reject as unbound, never reaching the
    // comprehension at all).
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_h".to_string(),
            params: vec![
                ("0comp_11_i".to_string(), Ty::List(Box::new(Ty::Int))),
                ("ys".to_string(), Ty::List(Box::new(Ty::Int))),
            ],
            return_ty: Ty::None,
            body: vec![
                HirStmt::ListCompAssign {
                    target: "y".to_string(),
                    var: "0comp_11_i".to_string(),
                    iter: CompIter::Name("ys".to_string()),
                    cond: None,
                    elt: Box::new(HirExpr::Name("0comp_11_i".to_string())),
                },
                HirStmt::Return(None),
            ],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    // The real check pass still rejects this program overall (`0comp_11_i`
    // is a `list[int]`-typed parameter, and the comprehension tries to
    // rebind it to `int`) -- that is not what this test pins. What
    // matters is that the *solver's* own `bind_comp_loop_var` reaches
    // its `CompIter::Name` branch with `var` already a key in
    // `env.bindings` (from the parameter seeding) and takes the
    // "already bound, do nothing" path without itself erroring, which
    // is what lets the module-wide solver pass complete and fall
    // through to the real, comprehension-aware check pass that then
    // reports the actual `T0023` conflict.
    assert_eq!(check(&hir).unwrap_err().code, "T0023");
}
#[test]
fn collect_block_constraints_propagates_an_error_from_a_list_comp_assign_s_cond() {
    // See the range-stop-operand test above for why a forward reference
    // to a same-body local, not a plain undefined name, is required to
    // exercise this solver's own error path.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_h".to_string(),
            params: vec![],
            return_ty: Ty::None,
            body: vec![
                HirStmt::ListCompAssign {
                    target: "y".to_string(),
                    var: "0comp_11_i".to_string(),
                    iter: CompIter::Range {
                        start: HirExpr::IntLiteral(0),
                        stop: HirExpr::IntLiteral(3),
                        step: HirExpr::IntLiteral(1),
                    },
                    cond: Some(Box::new(HirExpr::Name("later".to_string()))),
                    elt: Box::new(HirExpr::IntLiteral(1)),
                },
                HirStmt::Assign {
                    target: "later".to_string(),
                    value: HirExpr::IntLiteral(1),
                },
                HirStmt::Return(None),
            ],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert_eq!(check(&hir).unwrap_err().code, "T0021");
}
#[test]
fn collect_block_constraints_propagates_an_error_from_a_list_comp_assign_s_elt() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_h".to_string(),
            params: vec![],
            return_ty: Ty::None,
            body: vec![
                HirStmt::ListCompAssign {
                    target: "y".to_string(),
                    var: "0comp_11_i".to_string(),
                    iter: CompIter::Range {
                        start: HirExpr::IntLiteral(0),
                        stop: HirExpr::IntLiteral(3),
                        step: HirExpr::IntLiteral(1),
                    },
                    cond: None,
                    elt: Box::new(HirExpr::Name("later".to_string())),
                },
                HirStmt::Assign {
                    target: "later".to_string(),
                    value: HirExpr::IntLiteral(1),
                },
                HirStmt::Return(None),
            ],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert_eq!(check(&hir).unwrap_err().code, "T0021");
}
#[test]
fn collect_block_constraints_propagates_an_error_from_a_dict_comp_assign_s_loop_variable_binding() {
    // Pins `DictCompAssign`'s own `bind_comp_loop_var(...)?` call site
    // (textually distinct from the list/set arm's own call site above)
    // failing outright, using the same loop-variable-collision shape as
    // the list-comprehension version above.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "_h".to_string(),
                params: vec![("0comp_11_i".to_string(), Ty::Str)],
                return_ty: Ty::None,
                body: vec![
                    HirStmt::DictCompAssign {
                        target: "y".to_string(),
                        var: "0comp_11_i".to_string(),
                        iter: CompIter::Range {
                            start: HirExpr::IntLiteral(0),
                            stop: HirExpr::IntLiteral(3),
                            step: HirExpr::IntLiteral(1),
                        },
                        cond: None,
                        key: Box::new(HirExpr::StringLiteral("k".to_string())),
                        value: Box::new(HirExpr::IntLiteral(1)),
                    },
                    HirStmt::Return(None),
                ],
            },
            HirItem::Function {
                name: "_trigger".to_string(),
                params: vec![],
                return_ty: Ty::Infer,
                body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(1)))],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert_eq!(check(&hir).unwrap_err().code, "T0023");
}
#[test]
fn collect_block_constraints_propagates_an_error_from_a_dict_comp_assign_s_cond() {
    // See `collect_block_constraints_propagates_an_error_from_a_range_comprehension_s_stop_operand`
    // above for why a forward reference to a same-body local, not a
    // plain undefined name, is required to exercise this solver's own
    // error path (rather than being leniently ignored as `Ok(None)`).
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_h".to_string(),
            params: vec![],
            return_ty: Ty::None,
            body: vec![
                HirStmt::DictCompAssign {
                    target: "y".to_string(),
                    var: "0comp_11_i".to_string(),
                    iter: CompIter::Range {
                        start: HirExpr::IntLiteral(0),
                        stop: HirExpr::IntLiteral(3),
                        step: HirExpr::IntLiteral(1),
                    },
                    cond: Some(Box::new(HirExpr::Name("later".to_string()))),
                    key: Box::new(HirExpr::StringLiteral("k".to_string())),
                    value: Box::new(HirExpr::IntLiteral(1)),
                },
                HirStmt::Assign {
                    target: "later".to_string(),
                    value: HirExpr::IntLiteral(1),
                },
                HirStmt::Return(None),
            ],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert_eq!(check(&hir).unwrap_err().code, "T0021");
}
#[test]
fn collect_block_constraints_propagates_an_error_from_a_dict_comp_assign_s_key() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_h".to_string(),
            params: vec![],
            return_ty: Ty::None,
            body: vec![
                HirStmt::DictCompAssign {
                    target: "y".to_string(),
                    var: "0comp_11_i".to_string(),
                    iter: CompIter::Range {
                        start: HirExpr::IntLiteral(0),
                        stop: HirExpr::IntLiteral(3),
                        step: HirExpr::IntLiteral(1),
                    },
                    cond: None,
                    key: Box::new(HirExpr::Name("later".to_string())),
                    value: Box::new(HirExpr::IntLiteral(1)),
                },
                HirStmt::Assign {
                    target: "later".to_string(),
                    value: HirExpr::StringLiteral("k".to_string()),
                },
                HirStmt::Return(None),
            ],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert_eq!(check(&hir).unwrap_err().code, "T0021");
}
#[test]
fn collect_block_constraints_propagates_an_error_from_a_dict_comp_assign_s_value() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_h".to_string(),
            params: vec![],
            return_ty: Ty::None,
            body: vec![
                HirStmt::DictCompAssign {
                    target: "y".to_string(),
                    var: "0comp_11_i".to_string(),
                    iter: CompIter::Range {
                        start: HirExpr::IntLiteral(0),
                        stop: HirExpr::IntLiteral(3),
                        step: HirExpr::IntLiteral(1),
                    },
                    cond: None,
                    key: Box::new(HirExpr::StringLiteral("k".to_string())),
                    value: Box::new(HirExpr::Name("later".to_string())),
                },
                HirStmt::Assign {
                    target: "later".to_string(),
                    value: HirExpr::IntLiteral(1),
                },
                HirStmt::Return(None),
            ],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert_eq!(check(&hir).unwrap_err().code, "T0021");
}
