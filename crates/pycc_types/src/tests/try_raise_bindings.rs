//! `try`/`raise` return and binding analysis on the direct checker.

use super::*;

// -- #382 coverage tests --

#[test]
fn contains_return_finds_a_return_inside_a_try_body() {
    let body = vec![HirStmt::Try {
        body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(1)))],
        handlers: vec![],
        orelse: vec![],
        finalbody: vec![],
    }];
    assert!(contains_return(&body));
}

#[test]
fn contains_return_finds_a_return_inside_a_try_handler() {
    let body = vec![HirStmt::Try {
        body: vec![],
        handlers: vec![pycc_hir::HirExceptHandler {
            exc_type: Some(vec!["ValueError".to_string()]),
            name: None,
            body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(2)))],
        }],
        orelse: vec![],
        finalbody: vec![],
    }];
    assert!(contains_return(&body));
}

#[test]
fn contains_return_finds_a_return_inside_a_try_else() {
    let body = vec![HirStmt::Try {
        body: vec![],
        handlers: vec![],
        orelse: vec![HirStmt::Return(Some(HirExpr::IntLiteral(3)))],
        finalbody: vec![],
    }];
    assert!(contains_return(&body));
}

#[test]
fn contains_return_finds_a_return_inside_a_try_finally() {
    let body = vec![HirStmt::Try {
        body: vec![],
        handlers: vec![],
        orelse: vec![],
        finalbody: vec![HirStmt::Return(Some(HirExpr::IntLiteral(4)))],
    }];
    assert!(contains_return(&body));
}

#[test]
fn contains_return_does_not_find_a_return_in_a_try_without_one() {
    let body = vec![HirStmt::Try {
        body: vec![HirStmt::ExprStmt(HirExpr::IntLiteral(1))],
        handlers: vec![pycc_hir::HirExceptHandler {
            exc_type: Some(vec!["ValueError".to_string()]),
            name: None,
            body: vec![HirStmt::ExprStmt(HirExpr::IntLiteral(2))],
        }],
        orelse: vec![HirStmt::ExprStmt(HirExpr::IntLiteral(3))],
        finalbody: vec![HirStmt::ExprStmt(HirExpr::IntLiteral(4))],
    }];
    assert!(!contains_return(&body));
}

// Part 3 of #382 (#542, PEP 654): `contains_return`/`introduces_bindings`
// share their `HirStmt::Try` match arm with `HirStmt::TryStar` via an `|`
// pattern, but the `Try`-only tests above never actually construct a
// `TryStar` value, so that shared arm's binding pattern is only ever matched
// through `Try`. Mirror each `Try` case once for `TryStar` so the shared arm
// is exercised through both variants.

#[test]
fn contains_return_finds_a_return_inside_a_try_star_body() {
    let body = vec![HirStmt::TryStar {
        body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(1)))],
        handlers: vec![],
        orelse: vec![],
        finalbody: vec![],
    }];
    assert!(contains_return(&body));
}

#[test]
fn contains_return_finds_a_return_inside_a_try_star_handler() {
    let body = vec![HirStmt::TryStar {
        body: vec![],
        handlers: vec![pycc_hir::HirExceptHandler {
            exc_type: Some(vec!["ValueError".to_string()]),
            name: None,
            body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(2)))],
        }],
        orelse: vec![],
        finalbody: vec![],
    }];
    assert!(contains_return(&body));
}

#[test]
fn contains_return_finds_a_return_inside_a_try_star_else() {
    let body = vec![HirStmt::TryStar {
        body: vec![],
        handlers: vec![],
        orelse: vec![HirStmt::Return(Some(HirExpr::IntLiteral(3)))],
        finalbody: vec![],
    }];
    assert!(contains_return(&body));
}

#[test]
fn contains_return_finds_a_return_inside_a_try_star_finally() {
    let body = vec![HirStmt::TryStar {
        body: vec![],
        handlers: vec![],
        orelse: vec![],
        finalbody: vec![HirStmt::Return(Some(HirExpr::IntLiteral(4)))],
    }];
    assert!(contains_return(&body));
}

#[test]
fn contains_return_does_not_find_a_return_in_a_try_star_without_one() {
    let body = vec![HirStmt::TryStar {
        body: vec![HirStmt::ExprStmt(HirExpr::IntLiteral(1))],
        handlers: vec![pycc_hir::HirExceptHandler {
            exc_type: Some(vec!["ValueError".to_string()]),
            name: None,
            body: vec![HirStmt::ExprStmt(HirExpr::IntLiteral(2))],
        }],
        orelse: vec![HirStmt::ExprStmt(HirExpr::IntLiteral(3))],
        finalbody: vec![HirStmt::ExprStmt(HirExpr::IntLiteral(4))],
    }];
    assert!(!contains_return(&body));
}

#[test]
fn introduces_bindings_finds_bindings_inside_a_try_star_body() {
    let body = vec![HirStmt::TryStar {
        body: vec![HirStmt::Assign {
            target: "y".to_string(),
            value: HirExpr::IntLiteral(1),
        }],
        handlers: vec![],
        orelse: vec![],
        finalbody: vec![],
    }];
    assert!(introduces_bindings(&body));
}

#[test]
fn introduces_bindings_finds_bindings_inside_a_try_star_handler() {
    let body = vec![HirStmt::TryStar {
        body: vec![],
        handlers: vec![pycc_hir::HirExceptHandler {
            exc_type: Some(vec!["ValueError".to_string()]),
            name: None,
            body: vec![HirStmt::Assign {
                target: "z".to_string(),
                value: HirExpr::IntLiteral(2),
            }],
        }],
        orelse: vec![],
        finalbody: vec![],
    }];
    assert!(introduces_bindings(&body));
}

#[test]
fn introduces_bindings_finds_bindings_inside_a_try_star_else() {
    let body = vec![HirStmt::TryStar {
        body: vec![],
        handlers: vec![],
        orelse: vec![HirStmt::Assign {
            target: "w".to_string(),
            value: HirExpr::IntLiteral(3),
        }],
        finalbody: vec![],
    }];
    assert!(introduces_bindings(&body));
}

#[test]
fn introduces_bindings_finds_bindings_inside_a_try_star_finally() {
    let body = vec![HirStmt::TryStar {
        body: vec![],
        handlers: vec![],
        orelse: vec![],
        finalbody: vec![HirStmt::Assign {
            target: "v".to_string(),
            value: HirExpr::IntLiteral(4),
        }],
    }];
    assert!(introduces_bindings(&body));
}

#[test]
fn introduces_bindings_does_not_find_bindings_in_a_try_star_without_any() {
    let body = vec![HirStmt::TryStar {
        body: vec![HirStmt::ExprStmt(HirExpr::IntLiteral(1))],
        handlers: vec![pycc_hir::HirExceptHandler {
            exc_type: Some(vec!["ValueError".to_string()]),
            name: None,
            body: vec![HirStmt::ExprStmt(HirExpr::IntLiteral(2))],
        }],
        orelse: vec![HirStmt::ExprStmt(HirExpr::IntLiteral(3))],
        finalbody: vec![HirStmt::ExprStmt(HirExpr::IntLiteral(4))],
    }];
    assert!(!introduces_bindings(&body));
}

#[test]
fn introduces_bindings_finds_bindings_inside_a_try_body() {
    let body = vec![HirStmt::Try {
        body: vec![HirStmt::Assign {
            target: "y".to_string(),
            value: HirExpr::IntLiteral(1),
        }],
        handlers: vec![],
        orelse: vec![],
        finalbody: vec![],
    }];
    assert!(introduces_bindings(&body));
}

#[test]
fn introduces_bindings_finds_bindings_inside_a_try_handler() {
    let body = vec![HirStmt::Try {
        body: vec![],
        handlers: vec![pycc_hir::HirExceptHandler {
            exc_type: Some(vec!["ValueError".to_string()]),
            name: None,
            body: vec![HirStmt::Assign {
                target: "z".to_string(),
                value: HirExpr::IntLiteral(2),
            }],
        }],
        orelse: vec![],
        finalbody: vec![],
    }];
    assert!(introduces_bindings(&body));
}

#[test]
fn introduces_bindings_finds_bindings_inside_a_try_else() {
    let body = vec![HirStmt::Try {
        body: vec![],
        handlers: vec![],
        orelse: vec![HirStmt::Assign {
            target: "w".to_string(),
            value: HirExpr::IntLiteral(3),
        }],
        finalbody: vec![],
    }];
    assert!(introduces_bindings(&body));
}

#[test]
fn introduces_bindings_finds_bindings_inside_a_try_finally() {
    let body = vec![HirStmt::Try {
        body: vec![],
        handlers: vec![],
        orelse: vec![],
        finalbody: vec![HirStmt::Assign {
            target: "v".to_string(),
            value: HirExpr::IntLiteral(4),
        }],
    }];
    assert!(introduces_bindings(&body));
}

#[test]
fn introduces_bindings_does_not_find_bindings_in_a_try_without_any() {
    let body = vec![HirStmt::Try {
        body: vec![HirStmt::ExprStmt(HirExpr::IntLiteral(1))],
        handlers: vec![pycc_hir::HirExceptHandler {
            exc_type: Some(vec!["ValueError".to_string()]),
            name: None,
            body: vec![HirStmt::ExprStmt(HirExpr::IntLiteral(2))],
        }],
        orelse: vec![],
        finalbody: vec![],
    }];
    assert!(!introduces_bindings(&body));
}

#[test]
fn introduces_bindings_treats_raise_as_not_introducing_bindings() {
    let body = vec![HirStmt::Raise {
        exc: Some(HirExpr::Call {
            callee: "ValueError".to_string(),
            args: vec![HirExpr::StringLiteral("bad".to_string())],
        }),
        cause: None,
    }];
    assert!(!introduces_bindings(&body));
}

#[test]
fn raise_with_wrong_arg_count_is_t0021() {
    let err = parse_check("raise ValueError(\"a\", \"b\")\n").unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("exactly 1 argument"));
}

#[test]
fn raise_with_wrong_arg_type_is_t0021() {
    let err = parse_check("raise ValueError(42)\n").unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("str"));
}

#[test]
fn raise_with_no_args_is_t0021() {
    let err = parse_check("raise ValueError()\n").unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("exactly 1 argument"));
}

#[test]
fn raise_user_defined_class_instance_is_t0021() {
    // A user-defined class is not a builtin exception class.
    let src = "\
class C:
    def __init__(self):
        pass
raise C()
";
    let err = parse_check(src).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("can only raise"));
}

#[test]
fn try_except_as_binding_in_function_collects_local_name() {
    // This exercises `collect_local_names` for the `Try` handler name
    // binding path inside a function body.
    let src = "\
def f() -> int:
    y = 0
    try:
        x = 1
    except ValueError as e:
        y = 2
    return y
";
    parse_check(src).expect("check should succeed");
}

#[test]
fn try_except_as_binding_with_typed_handler_in_function() {
    // This exercises `check_try_stmt`'s handler binding path with a
    // typed exception handler inside a function.
    let src = "\
def f() -> int:
    x = 0
    try:
        x = 1
    except ValueError as e:
        return 0
    return x
";
    parse_check(src).expect("check should succeed");
}

#[test]
fn try_with_solver_path_succeeds() {
    // Force the solver path by using a private helper with an
    // unannotated parameter. This exercises `collect_block_constraints`
    // for `Try` and `Raise` on the solver path.
    let src = "\
def _helper(x) -> int:
    y = 0
    try:
        y = x
    except ValueError:
        y = 0
    return y
_helper(1)
";
    parse_check_resolve(src).expect("check should succeed");
}
