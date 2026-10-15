//! Definite/maybe binding and loops inside function bodies.

use super::*;

#[test]
fn a_function_local_if_with_no_else_then_read_is_checked_at_run_time() {
    // `def f(): if cond: x = 1; return x` -- T0041 (possibly-unbound
    // read in function body).
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "f".to_string(),
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
    check(&hir).expect("checked at run time (`NameError` / `UnboundLocalError`)");
}

#[test]
fn a_function_local_while_then_read_is_checked_at_run_time() {
    // `def f(): while cond: x = 1; return x` -- T0041.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "f".to_string(),
            params: vec![("cond".to_string(), Ty::Bool)],
            return_ty: Ty::Int,
            body: vec![
                HirStmt::While {
                    test: HirExpr::Name("cond".to_string()),
                    body: vec![HirStmt::Assign {
                        target: "x".to_string(),
                        value: HirExpr::IntLiteral(1),
                    }],
                },
                HirStmt::Return(Some(HirExpr::Name("x".to_string()))),
            ],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    check(&hir).expect("checked at run time (`NameError` / `UnboundLocalError`)");
}

#[test]
fn a_function_local_both_branches_if_then_read_succeeds() {
    // `def f(): if cond: x = 1 else: x = 2; return x` -- succeeds.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "f".to_string(),
            params: vec![("cond".to_string(), Ty::Bool)],
            return_ty: Ty::Int,
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
fn a_function_local_for_range_then_read_is_checked_at_run_time() {
    // `def f(): for i in range(3): x = i; return x` -- T0041 (the loop
    // body may execute zero times, so `x` is maybe-bound).
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "f".to_string(),
            params: vec![],
            return_ty: Ty::Int,
            body: vec![
                HirStmt::ForRange {
                    var: "i".to_string(),
                    start: HirExpr::IntLiteral(0),
                    stop: HirExpr::IntLiteral(3),
                    step: HirExpr::IntLiteral(1),
                    body: vec![HirStmt::Assign {
                        target: "x".to_string(),
                        value: HirExpr::Name("i".to_string()),
                    }],
                },
                HirStmt::Return(Some(HirExpr::Name("x".to_string()))),
            ],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    check(&hir).expect("checked at run time (`NameError` / `UnboundLocalError`)");
}

#[test]
fn a_function_local_for_range_var_then_read_is_checked_at_run_time() {
    // `def f(): for i in range(3): pass; return i` -- T0041 (the loop
    // variable is maybe-bound after the loop).
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
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
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    check(&hir).expect("checked at run time (`NameError` / `UnboundLocalError`)");
}

#[test]
fn a_function_local_pre_bound_then_for_range_var_read_succeeds() {
    // `def f(): i = 0; for i in range(3): pass; return i` -- succeeds
    // because `i` was definitely bound before the loop.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "f".to_string(),
            params: vec![],
            return_ty: Ty::Int,
            body: vec![
                HirStmt::Assign {
                    target: "i".to_string(),
                    value: HirExpr::IntLiteral(0),
                },
                HirStmt::ForRange {
                    var: "i".to_string(),
                    start: HirExpr::IntLiteral(0),
                    stop: HirExpr::IntLiteral(3),
                    step: HirExpr::IntLiteral(1),
                    body: vec![],
                },
                HirStmt::Return(Some(HirExpr::Name("i".to_string()))),
            ],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert!(check(&hir).is_ok());
}

#[test]
fn a_function_local_for_list_then_read_is_checked_at_run_time() {
    // `def f(xs: list[int]): for i in xs: x = i; return x` -- T0041.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "f".to_string(),
            params: vec![("xs".to_string(), Ty::List(Box::new(Ty::Int)))],
            return_ty: Ty::Int,
            body: vec![
                HirStmt::ForList {
                    var: "i".to_string(),
                    list: "xs".to_string(),
                    body: vec![HirStmt::Assign {
                        target: "x".to_string(),
                        value: HirExpr::Name("i".to_string()),
                    }],
                },
                HirStmt::Return(Some(HirExpr::Name("x".to_string()))),
            ],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    check(&hir).expect("checked at run time (`NameError` / `UnboundLocalError`)");
}

#[test]
fn a_function_local_pre_bound_for_list_var_survives_as_definite() {
    // `def f(xs: list[int]): i = 0; for i in xs: pass; return i` --
    // succeeds because `i` was definitely bound before the loop
    // (issue #118 Part 1).
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "f".to_string(),
            params: vec![("xs".to_string(), Ty::List(Box::new(Ty::Int)))],
            return_ty: Ty::Int,
            body: vec![
                HirStmt::Assign {
                    target: "i".to_string(),
                    value: HirExpr::IntLiteral(0),
                },
                HirStmt::ForList {
                    var: "i".to_string(),
                    list: "xs".to_string(),
                    body: vec![],
                },
                HirStmt::Return(Some(HirExpr::Name("i".to_string()))),
            ],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert!(check(&hir).is_ok());
}

#[test]
fn a_function_local_maybe_then_definite_upgrade_succeeds() {
    // `def f(c: bool): if c: x = 1; x = 2; return x` -- the maybe-bound
    // `x` from the if-body is upgraded to definite by the unconditional
    // `x = 2`, so the return read succeeds.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "f".to_string(),
            params: vec![("c".to_string(), Ty::Bool)],
            return_ty: Ty::Int,
            body: vec![
                HirStmt::If {
                    test: HirExpr::Name("c".to_string()),
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

#[test]
fn a_for_list_s_list_not_defined_in_a_function_body_propagates_the_error() {
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![HirStmt::ForList {
            var: "i".to_string(),
            list: "undefined".to_string(),
            body: vec![],
        }],
    };
    let err = check_function(&function).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("not defined"));
}

#[test]
fn a_for_list_s_list_read_before_a_later_local_assignment_is_unbound_local() {
    // `xs` is a declared local of this function (it's assigned further
    // down), so reading it in the `for` clause before that assignment
    // must be `unbound_local`, not "not defined" -- the same
    // is_local-aware distinction `HirExpr::Name` itself makes.
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![
            HirStmt::ForList {
                var: "i".to_string(),
                list: "xs".to_string(),
                body: vec![],
            },
            HirStmt::Assign {
                target: "xs".to_string(),
                value: HirExpr::ListLiteral(vec![HirExpr::IntLiteral(1)]),
            },
        ],
    };
    let err = check_function(&function).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("not bound before this use"));
}

#[test]
fn a_for_list_loop_over_a_non_list_value_in_a_function_body_is_rejected() {
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![("x".to_string(), Ty::Int)],
        return_ty: Ty::None,
        body: vec![HirStmt::ForList {
            var: "i".to_string(),
            list: "x".to_string(),
            body: vec![],
        }],
    };
    let err = check_function(&function).unwrap_err();
    assert_eq!(err.code, "T0033");
    assert!(err.message.contains("cannot be iterated"));
}

#[test]
fn a_for_list_loop_whose_body_statement_is_ill_typed_in_a_function_body_propagates_the_error() {
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![("xs".to_string(), Ty::List(Box::new(Ty::Int)))],
        return_ty: Ty::None,
        body: vec![HirStmt::ForList {
            var: "i".to_string(),
            list: "xs".to_string(),
            body: vec![HirStmt::ExprStmt(HirExpr::Name("undefined".to_string()))],
        }],
    };
    assert_eq!(check_function(&function).unwrap_err().code, "T0021");
}

#[test]
fn a_for_list_loop_rejects_a_conflicting_reassignment_of_its_variable_in_a_function_body() {
    // Same as the module-scope sibling above: `check_assignment` itself
    // is what rejects rebinding an already-`str` parameter to the list's
    // `int` element type.
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![
            ("i".to_string(), Ty::Str),
            ("xs".to_string(), Ty::List(Box::new(Ty::Int))),
        ],
        return_ty: Ty::None,
        body: vec![HirStmt::ForList {
            var: "i".to_string(),
            list: "xs".to_string(),
            body: vec![],
        }],
    };
    assert_eq!(check_function(&function).unwrap_err().code, "T0023");
}
