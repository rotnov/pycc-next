//! Statement helpers: local-name collection, `contains_return`, comprehension assignments.

use super::*;

/// `check_try_star_stmt`'s own comment on its `if let Some(exc_types) =
/// &handler.exc_type` check explains that a bare `except*:` is rejected by
/// ruff's own parser as a syntax error (PEP 654 requires every `except*`
/// clause to name a type), so `handler.exc_type` is always `Some` by the
/// time an `HirStmt::TryStar` reaches type-checking through the public
/// `check`/`build` CLI -- there is no valid Python source that reaches this
/// function with `exc_type: None`. `HirExceptHandler.exc_type` is still
/// `Option<Vec<String>>` at the type level, though, so the `None` arm is a
/// real branch the compiler must still exhaustively handle; a fixture that
/// builds a `HirStmt::TryStar` directly (bypassing the parser) and passes a
/// handler with `exc_type: None` is the only way to prove it once
/// unconditionally skips the whole type-validation block and falls through
/// to bind the handler's name (if any) and check its body, exactly like a
/// handler whose single named type happens to validate cleanly.
#[test]
fn check_try_star_stmt_skips_type_validation_for_a_handler_with_no_named_type() {
    let mut env = Environment::default();
    let body: Vec<HirStmt> = vec![];
    let handlers = vec![pycc_hir::HirExceptHandler {
        exc_type: None,
        name: Some("e".to_string()),
        body: vec![],
    }];
    let orelse: Vec<HirStmt> = vec![];
    let finalbody: Vec<HirStmt> = vec![];

    check_try_star_stmt(&mut env, &[], &body, &handlers, &orelse, &finalbody, None)
        .expect("a handler with no named type should skip validation, not error");
}

#[test]
fn local_name_collection_ignores_a_dict_set_target() {
    // `d[k] = v` mutates an existing binding's contents, not a name --
    // unlike `Assign`/`AnnAssign`/`ForList`, it must not be collected as
    // a new local.
    let params: Vec<(String, Ty)> = vec![];
    let body = vec![HirStmt::DictSet {
        dict: "x".to_string(),
        key: HirExpr::StringLiteral("a".to_string()),
        value: HirExpr::IntLiteral(1),
    }];
    assert_eq!(function_local_names(&params, &body), Vec::<&str>::new());
}

#[test]
fn contains_return_treats_a_dict_set_as_not_a_return() {
    let body = vec![HirStmt::DictSet {
        dict: "x".to_string(),
        key: HirExpr::StringLiteral("a".to_string()),
        value: HirExpr::IntLiteral(1),
    }];
    assert!(!contains_return(&body));
}

#[test]
fn block_always_returns_treats_a_dict_set_as_not_always_returning() {
    let body = vec![HirStmt::DictSet {
        dict: "x".to_string(),
        key: HirExpr::StringLiteral("a".to_string()),
        value: HirExpr::IntLiteral(1),
    }];
    assert!(!block_always_returns(&body));
}

fn a_list_comp_assign_stmt() -> HirStmt {
    HirStmt::ListCompAssign {
        target: "xs".to_string(),
        var: "0comp_11_i".to_string(),
        iter: CompIter::Range {
            start: HirExpr::IntLiteral(0),
            stop: HirExpr::IntLiteral(3),
            step: HirExpr::IntLiteral(1),
        },
        cond: None,
        elt: Box::new(HirExpr::Name("0comp_11_i".to_string())),
    }
}

fn a_set_comp_assign_stmt() -> HirStmt {
    HirStmt::SetCompAssign {
        target: "xs".to_string(),
        var: "0comp_11_i".to_string(),
        iter: CompIter::Range {
            start: HirExpr::IntLiteral(0),
            stop: HirExpr::IntLiteral(3),
            step: HirExpr::IntLiteral(1),
        },
        cond: None,
        elt: Box::new(HirExpr::Name("0comp_11_i".to_string())),
    }
}

fn a_dict_comp_assign_stmt() -> HirStmt {
    HirStmt::DictCompAssign {
        target: "xs".to_string(),
        var: "0comp_11_i".to_string(),
        iter: CompIter::Range {
            start: HirExpr::IntLiteral(0),
            stop: HirExpr::IntLiteral(3),
            step: HirExpr::IntLiteral(1),
        },
        cond: None,
        key: Box::new(HirExpr::Name("0comp_11_i".to_string())),
        value: Box::new(HirExpr::IntLiteral(1)),
    }
}

#[test]
fn local_name_collection_includes_a_list_comp_assign_s_target_and_synthesized_var() {
    // PR-12 Task 3 (D-117): unlike `DictSet` above, a comprehension
    // introduces two brand-new local names (`target` and the
    // synthesized `var`), not zero.
    let params: Vec<(String, Ty)> = vec![];
    let body = vec![a_list_comp_assign_stmt()];
    let mut names = function_local_names(&params, &body);
    names.sort_unstable();
    assert_eq!(names, vec!["0comp_11_i", "xs"]);
}

#[test]
fn local_name_collection_skips_a_list_comp_assign_s_target_and_var_when_already_local() {
    // Exercises the `!is_local(...)` guard's "already local" branch: a
    // parameter sharing the comprehension's `target`/`var` names must
    // not be pushed a second time.
    let params: Vec<(String, Ty)> = vec![
        ("xs".to_string(), Ty::Infer),
        ("0comp_11_i".to_string(), Ty::Infer),
    ];
    let body = vec![a_list_comp_assign_stmt()];
    let names = function_local_names(&params, &body);
    assert_eq!(names, vec!["xs", "0comp_11_i"]);
}

#[test]
fn local_name_collection_includes_a_set_comp_assign_s_target_and_var() {
    let params: Vec<(String, Ty)> = vec![];
    let body = vec![a_set_comp_assign_stmt()];
    let mut names = function_local_names(&params, &body);
    names.sort_unstable();
    assert_eq!(names, vec!["0comp_11_i", "xs"]);
}

#[test]
fn local_name_collection_includes_a_dict_comp_assign_s_target_and_var() {
    let params: Vec<(String, Ty)> = vec![];
    let body = vec![a_dict_comp_assign_stmt()];
    let mut names = function_local_names(&params, &body);
    names.sort_unstable();
    assert_eq!(names, vec!["0comp_11_i", "xs"]);
}

#[test]
fn contains_return_treats_a_list_comp_assign_as_not_a_return() {
    assert!(!contains_return(&[a_list_comp_assign_stmt()]));
}

#[test]
fn contains_return_treats_a_set_comp_assign_as_not_a_return() {
    assert!(!contains_return(&[a_set_comp_assign_stmt()]));
}

#[test]
fn contains_return_treats_a_dict_comp_assign_as_not_a_return() {
    assert!(!contains_return(&[a_dict_comp_assign_stmt()]));
}

#[test]
fn block_always_returns_treats_a_list_comp_assign_as_not_always_returning() {
    assert!(!block_always_returns(&[a_list_comp_assign_stmt()]));
}

#[test]
fn block_always_returns_treats_a_set_comp_assign_as_not_always_returning() {
    assert!(!block_always_returns(&[a_set_comp_assign_stmt()]));
}

#[test]
fn block_always_returns_treats_a_dict_comp_assign_as_not_always_returning() {
    assert!(!block_always_returns(&[a_dict_comp_assign_stmt()]));
}

#[test]
fn private_helper_parameter_is_inferred_through_a_comprehension_s_elt() {
    // Regression test (pinned-reviewer finding, pre-merge, P1): an
    // earlier version of `collect_block_constraints`'s comprehension arm
    // was a bare no-op -- it never recursed into `iter`/`cond`/`elt`
    // at all, unlike every sibling arm in this function (`ForRange`,
    // `ForList`, `DictSet` above all still recurse into their own
    // sub-expressions even though they don't register a term for their
    // own container). That meant a call expression appearing only
    // inside a comprehension's `elt` never participated in this
    // solver's argument<->parameter unification (see
    // `private_parameter_is_inferred_by_forwarding_into_an_annotated_callee`
    // for the plain, non-comprehension version of this exact mechanism):
    // `_forward`'s unannotated `x` parameter, used only as an argument
    // to the fully-annotated `_sink`, used to spuriously fail with
    // "cannot infer type of parameter `x`; add an annotation" even
    // though `x: int` is the only type consistent with this program.
    // `bind_comp_loop_var` + recursing into `elt` (Step 5's actual fix)
    // makes this resolve correctly and reach the real check pass, which
    // accepts the program (`_sink` returns `int`, satisfying the list
    // comprehension's own `T0034` gate).
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "_sink".to_string(),
                params: vec![("value".to_string(), Ty::Int)],
                return_ty: Ty::Int,
                body: vec![HirStmt::Return(Some(HirExpr::Name("value".to_string())))],
            },
            HirItem::Function {
                name: "_forward".to_string(),
                params: vec![("x".to_string(), Ty::Infer)],
                return_ty: Ty::None,
                body: vec![
                    HirStmt::ListCompAssign {
                        target: "y".to_string(),
                        var: "0comp_11_i".to_string(),
                        iter: CompIter::Range {
                            start: HirExpr::IntLiteral(0),
                            stop: HirExpr::IntLiteral(1),
                            step: HirExpr::IntLiteral(1),
                        },
                        cond: None,
                        elt: Box::new(HirExpr::Call {
                            callee: "_sink".to_string(),
                            args: vec![HirExpr::Name("x".to_string())],
                        }),
                    },
                    HirStmt::Return(None),
                ],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    check(&hir).unwrap();
}

#[test]
fn bind_comp_loop_var_unifies_a_range_comprehension_s_loop_variable_with_an_existing_term() {
    // Exercises `bind_comp_loop_var`'s `unify_terms(existing, Ok(Ty::Int), ...)`
    // branch (mirrors `ForRange`'s own analogous branch, exercised by
    // `collect_block_constraints_unifies_a_for_range_loop_variable_s_existing_term`-
    // style coverage elsewhere in this file): the loop variable is
    // already bound to an inferred term (via forwarding into another
    // annotated helper) before the comprehension re-binds it to `Ty::Int`.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "_sink".to_string(),
                params: vec![("value".to_string(), Ty::Int)],
                return_ty: Ty::Int,
                body: vec![HirStmt::Return(Some(HirExpr::Name("value".to_string())))],
            },
            HirItem::Function {
                name: "_forward".to_string(),
                params: vec![("x".to_string(), Ty::Infer)],
                return_ty: Ty::None,
                body: vec![
                    HirStmt::ExprStmt(HirExpr::Call {
                        callee: "_sink".to_string(),
                        args: vec![HirExpr::Name("x".to_string())],
                    }),
                    HirStmt::ListCompAssign {
                        target: "y".to_string(),
                        var: "x".to_string(),
                        iter: CompIter::Range {
                            start: HirExpr::IntLiteral(0),
                            stop: HirExpr::IntLiteral(1),
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
    check(&hir).unwrap();
}
