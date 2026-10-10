//! Comprehensions inside function bodies.

use super::*;

// -- PR-12 Task 3 (D-117): comprehension type-checking, function scope --

#[test]
fn a_list_comprehension_over_range_type_checks_in_a_function_body() {
    // Also pins that `var` is bound before `elt` is checked at function
    // scope (mirrors the module-scope sibling above): if the ordering
    // were wrong, `elt`'s reference to the loop variable would fail as
    // an unbound local.
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![HirStmt::ListCompAssign {
            target: "y".to_string(),
            var: "0comp_11_i".to_string(),
            iter: CompIter::Range {
                start: HirExpr::IntLiteral(0),
                stop: HirExpr::IntLiteral(3),
                step: HirExpr::IntLiteral(1),
            },
            cond: None,
            elt: Box::new(HirExpr::Name("0comp_11_i".to_string())),
        }],
    };
    check_function(&function).unwrap();
}

#[test]
fn a_list_comprehension_s_if_filter_of_a_non_bool_type_still_type_checks_in_a_function_body() {
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![HirStmt::ListCompAssign {
            target: "y".to_string(),
            var: "0comp_11_i".to_string(),
            iter: CompIter::Range {
                start: HirExpr::IntLiteral(0),
                stop: HirExpr::IntLiteral(3),
                step: HirExpr::IntLiteral(1),
            },
            cond: Some(Box::new(HirExpr::StringLiteral("truthy".to_string()))),
            elt: Box::new(HirExpr::IntLiteral(1)),
        }],
    };
    check_function(&function).unwrap();
}

#[test]
fn a_list_comprehension_producing_a_str_element_type_checks_in_a_function_body() {
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![HirStmt::ListCompAssign {
            target: "y".to_string(),
            var: "0comp_11_i".to_string(),
            iter: CompIter::Range {
                start: HirExpr::IntLiteral(0),
                stop: HirExpr::IntLiteral(3),
                step: HirExpr::IntLiteral(1),
            },
            cond: None,
            elt: Box::new(HirExpr::StringLiteral("x".to_string())),
        }],
    };
    check_function(&function).unwrap();
}

#[test]
fn a_set_comprehension_over_a_bare_set_parameter_type_checks_in_a_function_body() {
    // Exercises `resolve_comp_iter`'s `CompIter::Name` branch resolving
    // to `Ty::Set` at function scope.
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![("s".to_string(), Ty::Set(Box::new(Ty::Int)))],
        return_ty: Ty::None,
        body: vec![HirStmt::SetCompAssign {
            target: "y".to_string(),
            var: "0comp_20_x".to_string(),
            iter: CompIter::Name("s".to_string()),
            cond: None,
            elt: Box::new(HirExpr::Name("0comp_20_x".to_string())),
        }],
    };
    check_function(&function).unwrap();
}

#[test]
fn a_set_comprehension_producing_an_unhashable_element_is_rejected_as_t0038_in_a_function_body() {
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![HirStmt::SetCompAssign {
            target: "y".to_string(),
            var: "0comp_11_i".to_string(),
            iter: CompIter::Range {
                start: HirExpr::IntLiteral(0),
                stop: HirExpr::IntLiteral(3),
                step: HirExpr::IntLiteral(1),
            },
            cond: None,
            elt: Box::new(HirExpr::NoneLiteral),
        }],
    };
    let err = check_function(&function).unwrap_err();
    assert_eq!(err.code, "T0038");
    assert!(err.message.contains("set[None]"));
}

#[test]
fn a_set_comprehension_s_if_filter_of_a_non_bool_type_still_type_checks_in_a_function_body() {
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![HirStmt::SetCompAssign {
            target: "y".to_string(),
            var: "0comp_11_i".to_string(),
            iter: CompIter::Range {
                start: HirExpr::IntLiteral(0),
                stop: HirExpr::IntLiteral(3),
                step: HirExpr::IntLiteral(1),
            },
            cond: Some(Box::new(HirExpr::StringLiteral("truthy".to_string()))),
            elt: Box::new(HirExpr::IntLiteral(1)),
        }],
    };
    check_function(&function).unwrap();
}

#[test]
fn a_dict_comprehension_over_a_bare_dict_parameter_type_checks_in_a_function_body() {
    // Exercises `resolve_comp_iter`'s `CompIter::Name` branch resolving
    // to `Ty::Dict` at function scope, binding `var` as the dict's key
    // type.
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![("d".to_string(), Ty::Dict(Box::new((Ty::Str, Ty::Int))))],
        return_ty: Ty::None,
        body: vec![HirStmt::DictCompAssign {
            target: "y".to_string(),
            var: "0comp_20_k".to_string(),
            iter: CompIter::Name("d".to_string()),
            cond: None,
            key: Box::new(HirExpr::Name("0comp_20_k".to_string())),
            value: Box::new(HirExpr::IntLiteral(1)),
        }],
    };
    check_function(&function).unwrap();
}

#[test]
fn a_dict_comprehension_producing_an_unstorable_value_is_rejected_as_t0036_in_a_function_body() {
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![HirStmt::DictCompAssign {
            target: "y".to_string(),
            var: "0comp_11_i".to_string(),
            iter: CompIter::Range {
                start: HirExpr::IntLiteral(0),
                stop: HirExpr::IntLiteral(3),
                step: HirExpr::IntLiteral(1),
            },
            cond: None,
            key: Box::new(HirExpr::IntLiteral(1)),
            value: Box::new(HirExpr::NoneLiteral),
        }],
    };
    let err = check_function(&function).unwrap_err();
    assert_eq!(err.code, "T0036");
    assert!(err.message.contains("dict[int, None]"));
}

#[test]
fn a_dict_comprehension_s_if_filter_of_a_non_bool_type_still_type_checks_in_a_function_body() {
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![HirStmt::DictCompAssign {
            target: "y".to_string(),
            var: "0comp_11_i".to_string(),
            iter: CompIter::Range {
                start: HirExpr::IntLiteral(0),
                stop: HirExpr::IntLiteral(3),
                step: HirExpr::IntLiteral(1),
            },
            cond: Some(Box::new(HirExpr::StringLiteral("truthy".to_string()))),
            key: Box::new(HirExpr::StringLiteral("k".to_string())),
            value: Box::new(HirExpr::IntLiteral(1)),
        }],
    };
    check_function(&function).unwrap();
}

#[test]
fn a_comprehension_over_a_non_list_dict_set_iterable_is_rejected_as_t0033_in_a_function_body() {
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![("x".to_string(), Ty::Int)],
        return_ty: Ty::None,
        body: vec![HirStmt::ListCompAssign {
            target: "y".to_string(),
            var: "0comp_11_i".to_string(),
            iter: CompIter::Name("x".to_string()),
            cond: None,
            elt: Box::new(HirExpr::IntLiteral(1)),
        }],
    };
    let err = check_function(&function).unwrap_err();
    assert_eq!(err.code, "T0033");
    assert!(err.message.contains("cannot be iterated"));
}

#[test]
fn a_comprehension_over_a_forward_referenced_iterable_is_rejected_as_unbound_local() {
    // Mirrors `a_for_list_loop_over_a_forward_referenced_list...`-style
    // behavior: `xs` is a genuine function-local name (assigned later in
    // the same body), so referencing it before that assignment is
    // `unbound_local`, not `not defined`.
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![
            HirStmt::ListCompAssign {
                target: "y".to_string(),
                var: "0comp_11_i".to_string(),
                iter: CompIter::Name("xs".to_string()),
                cond: None,
                elt: Box::new(HirExpr::IntLiteral(1)),
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

// Per-arm error-propagation pins (function scope): the same set as the
// module-scope block above, `check_stmt_in_function`'s own three arms.

#[test]
fn a_set_comprehension_over_a_non_list_dict_set_iterable_is_rejected_as_t0033_in_a_function_body() {
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![("x".to_string(), Ty::Int)],
        return_ty: Ty::None,
        body: vec![HirStmt::SetCompAssign {
            target: "y".to_string(),
            var: "0comp_11_i".to_string(),
            iter: CompIter::Name("x".to_string()),
            cond: None,
            elt: Box::new(HirExpr::IntLiteral(1)),
        }],
    };
    let err = check_function(&function).unwrap_err();
    assert_eq!(err.code, "T0033");
    assert!(err.message.contains("cannot be iterated"));
}

#[test]
fn a_dict_comprehension_over_a_non_list_dict_set_iterable_is_rejected_as_t0033_in_a_function_body()
{
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![("x".to_string(), Ty::Int)],
        return_ty: Ty::None,
        body: vec![HirStmt::DictCompAssign {
            target: "y".to_string(),
            var: "0comp_11_i".to_string(),
            iter: CompIter::Name("x".to_string()),
            cond: None,
            key: Box::new(HirExpr::StringLiteral("k".to_string())),
            value: Box::new(HirExpr::IntLiteral(1)),
        }],
    };
    let err = check_function(&function).unwrap_err();
    assert_eq!(err.code, "T0033");
    assert!(err.message.contains("cannot be iterated"));
}

#[test]
fn a_list_comprehension_rejects_a_conflicting_reassignment_of_its_loop_variable_in_a_function_body()
{
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![("0comp_11_i".to_string(), Ty::Str)],
        return_ty: Ty::None,
        body: vec![HirStmt::ListCompAssign {
            target: "y".to_string(),
            var: "0comp_11_i".to_string(),
            iter: CompIter::Range {
                start: HirExpr::IntLiteral(0),
                stop: HirExpr::IntLiteral(3),
                step: HirExpr::IntLiteral(1),
            },
            cond: None,
            elt: Box::new(HirExpr::IntLiteral(1)),
        }],
    };
    assert_eq!(check_function(&function).unwrap_err().code, "T0023");
}

#[test]
fn a_set_comprehension_rejects_a_conflicting_reassignment_of_its_loop_variable_in_a_function_body()
{
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![("0comp_11_i".to_string(), Ty::Str)],
        return_ty: Ty::None,
        body: vec![HirStmt::SetCompAssign {
            target: "y".to_string(),
            var: "0comp_11_i".to_string(),
            iter: CompIter::Range {
                start: HirExpr::IntLiteral(0),
                stop: HirExpr::IntLiteral(3),
                step: HirExpr::IntLiteral(1),
            },
            cond: None,
            elt: Box::new(HirExpr::IntLiteral(1)),
        }],
    };
    assert_eq!(check_function(&function).unwrap_err().code, "T0023");
}

#[test]
fn a_dict_comprehension_rejects_a_conflicting_reassignment_of_its_loop_variable_in_a_function_body()
{
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![("0comp_11_i".to_string(), Ty::Str)],
        return_ty: Ty::None,
        body: vec![HirStmt::DictCompAssign {
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
        }],
    };
    assert_eq!(check_function(&function).unwrap_err().code, "T0023");
}

#[test]
fn a_list_comprehension_s_if_filter_propagates_an_ill_typed_error_in_a_function_body() {
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![HirStmt::ListCompAssign {
            target: "y".to_string(),
            var: "0comp_11_i".to_string(),
            iter: CompIter::Range {
                start: HirExpr::IntLiteral(0),
                stop: HirExpr::IntLiteral(3),
                step: HirExpr::IntLiteral(1),
            },
            cond: Some(Box::new(HirExpr::Name("undefined".to_string()))),
            elt: Box::new(HirExpr::IntLiteral(1)),
        }],
    };
    assert_eq!(check_function(&function).unwrap_err().code, "T0021");
}

#[test]
fn a_set_comprehension_s_if_filter_propagates_an_ill_typed_error_in_a_function_body() {
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![HirStmt::SetCompAssign {
            target: "y".to_string(),
            var: "0comp_11_i".to_string(),
            iter: CompIter::Range {
                start: HirExpr::IntLiteral(0),
                stop: HirExpr::IntLiteral(3),
                step: HirExpr::IntLiteral(1),
            },
            cond: Some(Box::new(HirExpr::Name("undefined".to_string()))),
            elt: Box::new(HirExpr::IntLiteral(1)),
        }],
    };
    assert_eq!(check_function(&function).unwrap_err().code, "T0021");
}

#[test]
fn a_dict_comprehension_s_if_filter_propagates_an_ill_typed_error_in_a_function_body() {
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![HirStmt::DictCompAssign {
            target: "y".to_string(),
            var: "0comp_11_i".to_string(),
            iter: CompIter::Range {
                start: HirExpr::IntLiteral(0),
                stop: HirExpr::IntLiteral(3),
                step: HirExpr::IntLiteral(1),
            },
            cond: Some(Box::new(HirExpr::Name("undefined".to_string()))),
            key: Box::new(HirExpr::StringLiteral("k".to_string())),
            value: Box::new(HirExpr::IntLiteral(1)),
        }],
    };
    assert_eq!(check_function(&function).unwrap_err().code, "T0021");
}

#[test]
fn a_list_comprehension_s_elt_propagates_an_ill_typed_error_in_a_function_body() {
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![HirStmt::ListCompAssign {
            target: "y".to_string(),
            var: "0comp_11_i".to_string(),
            iter: CompIter::Range {
                start: HirExpr::IntLiteral(0),
                stop: HirExpr::IntLiteral(3),
                step: HirExpr::IntLiteral(1),
            },
            cond: None,
            elt: Box::new(HirExpr::Name("undefined".to_string())),
        }],
    };
    assert_eq!(check_function(&function).unwrap_err().code, "T0021");
}

#[test]
fn a_set_comprehension_s_elt_propagates_an_ill_typed_error_in_a_function_body() {
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![HirStmt::SetCompAssign {
            target: "y".to_string(),
            var: "0comp_11_i".to_string(),
            iter: CompIter::Range {
                start: HirExpr::IntLiteral(0),
                stop: HirExpr::IntLiteral(3),
                step: HirExpr::IntLiteral(1),
            },
            cond: None,
            elt: Box::new(HirExpr::Name("undefined".to_string())),
        }],
    };
    assert_eq!(check_function(&function).unwrap_err().code, "T0021");
}

#[test]
fn a_dict_comprehension_s_key_propagates_an_ill_typed_error_in_a_function_body() {
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![HirStmt::DictCompAssign {
            target: "y".to_string(),
            var: "0comp_11_i".to_string(),
            iter: CompIter::Range {
                start: HirExpr::IntLiteral(0),
                stop: HirExpr::IntLiteral(3),
                step: HirExpr::IntLiteral(1),
            },
            cond: None,
            key: Box::new(HirExpr::Name("undefined".to_string())),
            value: Box::new(HirExpr::IntLiteral(1)),
        }],
    };
    assert_eq!(check_function(&function).unwrap_err().code, "T0021");
}

#[test]
fn a_dict_comprehension_s_value_propagates_an_ill_typed_error_in_a_function_body() {
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![HirStmt::DictCompAssign {
            target: "y".to_string(),
            var: "0comp_11_i".to_string(),
            iter: CompIter::Range {
                start: HirExpr::IntLiteral(0),
                stop: HirExpr::IntLiteral(3),
                step: HirExpr::IntLiteral(1),
            },
            cond: None,
            key: Box::new(HirExpr::StringLiteral("k".to_string())),
            value: Box::new(HirExpr::Name("undefined".to_string())),
        }],
    };
    assert_eq!(check_function(&function).unwrap_err().code, "T0021");
}
