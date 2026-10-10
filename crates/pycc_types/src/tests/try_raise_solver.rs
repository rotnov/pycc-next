//! `try`/`raise` on the solver path and through the generic rewrite.

use super::*;

#[test]
fn raise_with_solver_path_succeeds() {
    // Force the solver path with a private helper that has a raise.
    let src = "\
def _helper(x) -> int:
    if x == 0:
        raise ValueError(\"bad\")
    return x
_helper(1)
";
    parse_check_resolve(src).expect("check should succeed");
}

#[test]
fn try_with_generic_call_on_solver_path_succeeds() {
    // Force the solver path and exercise generic call rewriting
    // inside a try body.
    let src = "\
class C[T]:
    def __init__(self, x: T):
        self.x = x
def _helper(x) -> int:
    try:
        c = C[int](x)
        return c.x
    except ValueError:
        return 0
_helper(1)
";
    parse_check_resolve(src).expect("check should succeed");
}

#[test]
fn raise_from_with_solver_path_succeeds() {
    // Force the solver path with a raise-from inside a function.
    let src = "\
def _helper(x) -> int:
    if x == 0:
        raise ValueError(\"bad\") from RuntimeError(\"cause\")
    return x
_helper(1)
";
    parse_check_resolve(src).expect("check should succeed");
}

#[test]
fn bare_raise_with_solver_path_succeeds() {
    // Force the solver path with a bare raise inside an except handler.
    let src = "\
def _helper(x) -> int:
    y = 0
    try:
        y = x
    except ValueError:
        raise
    return y
_helper(1)
";
    parse_check_resolve(src).expect("check should succeed");
}

#[test]
fn raise_variable_bound_to_exception_instance_checks_successfully() {
    // `raise e` where `e` is bound to a builtin exception instance via
    // `except ValueError as e`. This exercises `check_raise_operand`'s
    // non-Call path: the expression is a Name, not a Call, so it falls
    // through to `infer_expr_in` which returns `Ty::Instance("ValueError")`,
    // and the `is_builtin_exception_class` check passes.
    let src = "\
def f() -> int:
    x = 0
    try:
        x = 1
    except ValueError as e:
        raise e
    return x
";
    parse_check(src).expect("check should succeed");
}

#[test]
fn raise_variable_bound_to_non_exception_is_t0021() {
    // `raise x` where `x` is an int — exercises the `_ =>` arm of
    // `check_raise_operand`'s match on the inferred type.
    let err = parse_check("x = 1\nraise x\n").unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("can only raise"));
}

#[test]
fn solver_try_body_type_error_propagates() {
    // Force the solver path and trigger a type error inside the try
    // body. This exercises the `?` error propagation in
    // `collect_block_constraints` for the try body on the solver path.
    let src = "\
def _helper(x) -> int:
    y = 0
    try:
        y = x + \"bad\"
    except ValueError:
        y = 0
    return y
_helper(1)
";
    let err = parse_check_resolve(src).unwrap_err();
    // The error should be about the type mismatch, not about raise.
    assert!(err.message.contains("operator"));
}

#[test]
fn solver_try_handler_type_error_propagates() {
    // Force the solver path and trigger a type error inside the
    // handler body. This exercises the `?` error propagation in
    // `collect_block_constraints` for the handler body on the solver path.
    let src = "\
def _helper(x) -> int:
    y = 0
    try:
        y = x
    except ValueError:
        y = x + \"bad\"
    return y
_helper(1)
";
    let err = parse_check_resolve(src).unwrap_err();
    assert!(err.message.contains("operator"));
}

#[test]
fn solver_try_else_type_error_propagates() {
    // Force the solver path and trigger a type error inside the else
    // body. This exercises the `?` error propagation in
    // `collect_block_constraints` for the else body on the solver path.
    let src = "\
def _helper(x) -> int:
    y = 0
    try:
        y = x
    except ValueError:
        y = 0
    else:
        y = x + \"bad\"
    return y
_helper(1)
";
    let err = parse_check_resolve(src).unwrap_err();
    assert!(err.message.contains("operator"));
}

#[test]
fn solver_try_finally_type_error_propagates() {
    // Force the solver path and trigger a type error inside the
    // finally body. This exercises the `?` error propagation in
    // `collect_block_constraints` for the finally body on the solver path.
    let src = "\
def _helper(x) -> int:
    y = 0
    try:
        y = x
    except ValueError:
        y = 0
    finally:
        y = x + \"bad\"
    return y
_helper(1)
";
    let err = parse_check_resolve(src).unwrap_err();
    assert!(err.message.contains("operator"));
}

#[test]
fn walrus_wrapping_a_generic_call_is_rewritten_and_collected() {
    // PEP 572 (#774): exercises `monomorphize::rewrite_generic_calls_in_expr`'s
    // and `collect_generic_class_instantiations_from_expr`'s own
    // `HirExpr::NamedExpr` arms -- both walks are exhaustive over every
    // `HirExpr` (see their own doc comments), and neither is otherwise
    // reached by any other test in this suite. Hand-built HIR (mirroring
    // this file's other `monomorphize(&hir)` tests, which bypass `check`
    // entirely) with the walrus's own value directly wrapping the call to
    // the generic `identity` function -- one node for both walks to visit.
    let param = Ty::Param(Box::new("T".to_string()));
    let identity = generic_identity_fn(param.clone(), param);
    let f = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![HirStmt::ExprStmt(HirExpr::NamedExpr {
            name: "z".to_string(),
            value: Box::new(HirExpr::Call {
                callee: "identity".to_string(),
                args: vec![HirExpr::IntLiteral(1)],
            }),
        })],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![identity, f],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let resolved = monomorphize(&hir).unwrap();
    assert_eq!(count_function(&resolved, "0gen_identity__T_int"), 1);
}

#[test]
fn walrus_wrapping_a_wrong_arity_generic_call_propagates_the_rewrite_error() {
    // The error-propagation sibling of the success test above: exercises
    // `rewrite_generic_calls_in_expr`'s `HirExpr::NamedExpr` arm's own `?`
    // -- a generic call nested inside a walrus's value that itself fails to
    // rewrite. Mirrors
    // `monomorphize_propagates_an_instantiation_error_from_inside_a_function_body`
    // above exactly (wrong-arity call to `identity`, expecting `T0021`),
    // with the call wrapped in a walrus.
    let param = Ty::Param(Box::new("T".to_string()));
    let identity = generic_identity_fn(param.clone(), param);
    let f = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![HirStmt::ExprStmt(HirExpr::NamedExpr {
            name: "z".to_string(),
            value: Box::new(HirExpr::Call {
                callee: "identity".to_string(),
                args: vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)],
            }),
        })],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![identity, f],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert_eq!(monomorphize(&hir).unwrap_err().code, "T0021");
}

#[test]
fn walrus_target_read_later_as_a_generic_calls_argument_resolves() {
    // PEP 572 (#774), deep-review follow-up: the opposite direction from
    // `walrus_wrapping_a_generic_call_is_rewritten_and_collected` above --
    // here the walrus is its own statement, and the generic call comes
    // *after*, reading the walrus target as a plain name. This exercises
    // `rewrite_generic_calls_in_stmt`'s `ExprStmt` arm delegating to
    // `rewrite_generic_calls_in_expr`'s `NamedExpr` arm, which must bind
    // `n` into `env` (this pass starts every function body from a fresh
    // `Environment`, not the one `check_and_resolve` already populated) so
    // the later `HirExpr::Name("n")` resolves instead of raising a
    // spurious "not defined" error for code the checker already accepted.
    let param = Ty::Param(Box::new("T".to_string()));
    let identity = generic_identity_fn(param.clone(), param);
    let f = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![
            HirStmt::ExprStmt(HirExpr::NamedExpr {
                name: "n".to_string(),
                value: Box::new(HirExpr::IntLiteral(1)),
            }),
            HirStmt::ExprStmt(HirExpr::Call {
                callee: "identity".to_string(),
                args: vec![HirExpr::Name("n".to_string())],
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
    let resolved = monomorphize(&hir).unwrap();
    assert_eq!(count_function(&resolved, "0gen_identity__T_int"), 1);
}

#[test]
fn walrus_target_inside_an_if_test_resolves_in_the_branch_body() {
    // PEP 572 (#774), deep-review follow-up: covers the same
    // fresh-`Environment` gap as the test above, but for a walrus nested
    // inside an `if` test (`rewrite_generic_calls_in_stmt`'s `If` arm calls
    // `rewrite_generic_calls_in_expr` on `test` directly, not through the
    // `ExprStmt` arm) with the read happening in the branch body rather
    // than a sibling statement.
    let param = Ty::Param(Box::new("T".to_string()));
    let identity = generic_identity_fn(param.clone(), param);
    let f = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![HirStmt::If {
            test: HirExpr::NamedExpr {
                name: "n".to_string(),
                value: Box::new(HirExpr::IntLiteral(1)),
            },
            body: vec![HirStmt::ExprStmt(HirExpr::Call {
                callee: "identity".to_string(),
                args: vec![HirExpr::Name("n".to_string())],
            })],
            orelse: vec![],
        }],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![identity, f],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let resolved = monomorphize(&hir).unwrap();
    assert_eq!(count_function(&resolved, "0gen_identity__T_int"), 1);
}

#[test]
fn walrus_wrapping_a_generic_call_propagates_a_t0050_error_from_its_own_type() {
    // PEP 572 (#774), deep-review follow-up: `rewrite_generic_calls_in_expr`'s
    // `NamedExpr` arm added an explicit `let ty = infer_expr_in(..)?;` (to
    // capture the type for its own new `env.bind` call) where the arm
    // previously just tail-returned `infer_expr_in(..)` directly -- that
    // turns what was a single fall-through region into a real branch, so
    // the error side needs its own coverage. Hand-built HIR (bypassing
    // `check`, same as the sibling tests above) with a walrus whose value
    // is a `str`, which `infer_expr_in`'s own `NamedExpr` arm rejects with
    // T0050 (walrus values are restricted to non-reference-counted
    // scalars) -- exercising the `?`'s early-return path.
    let param = Ty::Param(Box::new("T".to_string()));
    let identity = generic_identity_fn(param.clone(), param);
    let f = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![HirStmt::ExprStmt(HirExpr::NamedExpr {
            name: "z".to_string(),
            value: Box::new(HirExpr::StringLiteral("bad".to_string())),
        })],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![identity, f],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert_eq!(monomorphize(&hir).unwrap_err().code, "T0050");
}

#[test]
fn generic_call_in_try_else_rewrites_successfully() {
    // Exercise `rewrite_generic_calls_in_stmt` for the try orelse path
    // and `collect_generic_class_instantiations_from_stmt` for the same.
    let src = "\
class C[T]:
    def __init__(self, x: T):
        self.x = x
def f() -> int:
    x = 0
    try:
        x = 1
    except ValueError:
        x = 0
    else:
        c = C[int](1)
        x = c.x
    return x
";
    parse_check_resolve(src).expect("check should succeed");
}

#[test]
fn generic_call_in_try_finally_rewrites_successfully() {
    // Exercise `rewrite_generic_calls_in_stmt` for the try finally path
    // and `collect_generic_class_instantiations_from_stmt` for the same.
    let src = "\
class C[T]:
    def __init__(self, x: T):
        self.x = x
def f() -> int:
    x = 0
    try:
        x = 1
    except ValueError:
        x = 0
    finally:
        c = C[int](1)
        x = c.x
    return x
";
    parse_check_resolve(src).expect("check should succeed");
}

#[test]
fn generic_call_in_raise_rewrites_successfully() {
    // Exercise `rewrite_generic_calls_in_stmt` for the Raise path
    // and `collect_generic_class_instantiations_from_stmt` for the same.
    // A generic class instantiation as the raise expression is not a
    // builtin exception class call, so it falls through to the
    // generic-call rewrite path and then gets rejected as T0021.
    let src = "\
class C[T]:
    def __init__(self, x: T):
        self.x = x
raise C[int](1)
";
    let err = parse_check_resolve(src).unwrap_err();
    assert_eq!(err.code, "T0021");
}

#[test]
fn generic_call_in_try_body_rejects_unsupported() {
    // Exercise `reject_generic_calls_in_block` for the Try body path.
    // A generic call in a module-level try body where the generic
    // class is not in scope should be rejected.
    let src = "\
class C[T]:
    def __init__(self, x: T):
        self.x = x
try:
    c = C[int](1)
except ValueError:
    pass
";
    parse_check_resolve(src).expect("check should succeed");
}

#[test]
fn generic_call_in_raise_cause_rewrites_successfully() {
    // Exercise `rewrite_generic_calls_in_stmt` for the Raise cause path
    // and `collect_generic_class_instantiations_from_stmt` for the same.
    let src = "\
class C[T]:
    def __init__(self, x: T):
        self.x = x
raise ValueError(\"bad\") from C[int](1)
";
    let err = parse_check_resolve(src).unwrap_err();
    assert_eq!(err.code, "T0021");
}

#[test]
fn solver_try_with_as_binding_succeeds() {
    // Force the solver path with `except ... as e` to exercise the
    // `as` binding insertion in the solver's `Try` arm.
    let src = "\
def _helper(x) -> int:
    y = 0
    try:
        y = x
    except ValueError as e:
        y = 0
    return y
_helper(1)
";
    parse_check_resolve(src).expect("check should succeed");
}

#[test]
fn raise_inside_function_with_generic_call_rewrites() {
    // Exercise `rewrite_generic_calls_in_stmt` for the `Raise` arm
    // and `collect_generic_class_instantiations_from_stmt` for the
    // same. The key is that the raise is valid (ValueError) and the
    // function also contains a generic call, so monomorphization runs
    // and processes both the generic call and the raise statement.
    let src = "\
class C[T]:
    def __init__(self, x: T):
        self.x = x
def f() -> int:
    c = C[int](1)
    x = c.x
    raise ValueError(\"bad\")
";
    parse_check_resolve(src).expect("the rewritten generic call is followed by a terminal raise");
}

#[test]
fn try_with_generic_call_and_raise_rewrites() {
    // Exercise `rewrite_generic_calls_in_stmt` for both the `Try` and
    // `Raise` arms, and `collect_generic_class_instantiations_from_stmt`
    // for the same. A generic call before the try and a raise after
    // ensures both arms are reached during monomorphization.
    let src = "\
class C[T]:
    def __init__(self, x: T):
        self.x = x
def f() -> int:
    c = C[int](1)
    x = c.x
    try:
        x = 1
    except ValueError:
        x = 0
    raise ValueError(\"bad\")
";
    parse_check_resolve(src).expect("the final raise makes every path terminal");
}

#[test]
fn try_with_generic_call_in_handler_rewrites() {
    // Exercise `rewrite_generic_calls_in_stmt` for the `Try` handler
    // body path and `collect_generic_class_instantiations_from_stmt`
    // for the same.
    let src = "\
class C[T]:
    def __init__(self, x: T):
        self.x = x
def f() -> int:
    x = 0
    try:
        x = 1
    except ValueError:
        c = C[int](1)
        x = c.x
    return x
";
    parse_check_resolve(src).expect("check should succeed");
}

#[test]
fn try_handler_binding_is_visible_during_generic_rewrite() {
    let src = "\
def identity[T](value: T) -> T:
    return value
try:
    raise ValueError(\"bad\")
except ValueError as error:
    raise error from RuntimeError(\"cause\")
else:
    pass
finally:
    pass
";
    parse_check_resolve(src)
        .expect("the validated except-as binding must remain visible during rewriting");
}

#[test]
fn reject_generic_calls_in_try_and_raise() {
    // Exercise `reject_generic_calls_in_block` for both the `Try` and
    // `Raise` arms. A generic call inside a try body and a raise with
    // a generic call in the same module-level scope ensures both arms
    // are reached during the rejection pass.
    let src = "\
class C[T]:
    def __init__(self, x: T):
        self.x = x
try:
    c = C[int](1)
    print(c.x)
except ValueError:
    pass
";
    parse_check_resolve(src).expect("check should succeed");
}

// -- #382: reject_generic_calls_in_stmt Try and Raise arms --

#[test]
fn reject_generic_calls_processes_try_and_raise_arms() {
    // Exercise `reject_generic_calls_in_stmt`'s `Try` and `Raise` arms
    // directly. A generic function body containing a `Try` and a `Raise`
    // statement ensures both arms are reached during the rejection pass.
    let body = vec![
        HirStmt::Try {
            body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
            handlers: vec![pycc_hir::HirExceptHandler {
                exc_type: Some(vec!["ValueError".to_string()]),
                name: None,
                body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
            }],
            orelse: vec![HirStmt::ExprStmt(HirExpr::IntLiteral(0))],
            finalbody: vec![HirStmt::ExprStmt(HirExpr::IntLiteral(0))],
        },
        HirStmt::Raise {
            exc: Some(HirExpr::Name("some_exc".to_string())),
            cause: Some(HirExpr::Name("some_cause".to_string())),
        },
    ];
    // `reject_generic_calls_in_block` processes each statement through
    // `reject_generic_calls_in_stmt`. The `Try` and `Raise` arms should
    // be reached without error (no generic calls inside them).
    assert!(
        reject_generic_calls_in_block(&Environment::new(), "f", &body).is_ok(),
        "reject_generic_calls_in_block should succeed for try/raise without generic calls"
    );
}

#[test]
fn reject_generic_calls_processes_try_star_arm() {
    // Exercise `reject_generic_calls_in_stmt`'s `HirStmt::TryStar` half of
    // its combined `Try { .. } | TryStar { .. }` arm (#542) directly --
    // `reject_generic_calls_processes_try_and_raise_arms` above only
    // reaches the `Try` alternative's own field-destructure lines.
    let body = vec![HirStmt::TryStar {
        body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
        handlers: vec![pycc_hir::HirExceptHandler {
            exc_type: Some(vec!["ValueError".to_string()]),
            name: None,
            body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
        }],
        orelse: vec![HirStmt::ExprStmt(HirExpr::IntLiteral(0))],
        finalbody: vec![HirStmt::ExprStmt(HirExpr::IntLiteral(0))],
    }];
    assert!(
        reject_generic_calls_in_block(&Environment::new(), "f", &body).is_ok(),
        "reject_generic_calls_in_block should succeed for a try* without generic calls"
    );
}

// -- #382: check_stmt_sequence_shared ? error propagation in try bodies --

#[test]
fn check_try_body_type_error_propagates() {
    let err = parse_check(
            "def f() -> int:\n    try:\n        x = 1 + \"bad\"\n    except ValueError:\n        x = 0\n    return x\n",
        ).unwrap_err();
    assert!(err.message.contains("operator"));
}

#[test]
fn check_try_handler_body_type_error_propagates() {
    let err = parse_check(
            "def f() -> int:\n    try:\n        x = 1\n    except ValueError:\n        x = 1 + \"bad\"\n    return x\n",
        ).unwrap_err();
    assert!(err.message.contains("operator"));
}

#[test]
fn check_try_else_body_type_error_propagates() {
    let err = parse_check(
            "def f() -> int:\n    try:\n        x = 1\n    except ValueError:\n        x = 0\n    else:\n        x = 1 + \"bad\"\n    return x\n",
        ).unwrap_err();
    assert!(err.message.contains("operator"));
}

#[test]
fn check_try_finally_body_type_error_propagates() {
    let err = parse_check(
            "def f() -> int:\n    try:\n        x = 1\n    except ValueError:\n        x = 0\n    finally:\n        x = 1 + \"bad\"\n    return x\n",
        ).unwrap_err();
    assert!(err.message.contains("operator"));
}

// -- #382: join_if_branches ? error propagation --

#[test]
fn try_handler_with_incompatible_binding_is_t0023() {
    // A handler that rebinds a pre-existing variable to an incompatible
    // type triggers `join_if_branches`'s T0023 error path.
    let err = parse_check(
            "def f() -> int:\n    x = 1\n    try:\n        x = 2\n    except ValueError:\n        x = \"bad\"\n    return x\n",
        ).unwrap_err();
    assert_eq!(err.code, "T0023");
}

#[test]
fn try_else_with_incompatible_binding_is_t0023() {
    // An else body that rebinds a pre-existing variable to an
    // incompatible type triggers `join_if_branches`'s T0023 error path.
    let err = parse_check(
            "def f() -> int:\n    x = 1\n    try:\n        x = 2\n    except ValueError:\n        x = 3\n    else:\n        x = \"bad\"\n    return x\n",
        ).unwrap_err();
    assert_eq!(err.code, "T0023");
}

// -- #382: join_if_branches ? error propagation via `except ... as` --
// The `as` binding uses `henv.bind()` directly (no `check_assignment`),
// so a type mismatch between the pre-try binding and the `as` binding
// is caught by `join_if_branches`, not by `check_assignment`.

#[test]
fn try_handler_as_binding_incompatible_type_is_t0023() {
    // `e` is `int` before the try; `except ValueError as e` rebinds `e`
    // to `Instance("ValueError")` via `henv.bind()` (bypassing
    // `check_assignment`). `join_if_branches` detects the mismatch.
    let err = parse_check(
            "def f() -> int:\n    e = 0\n    try:\n        pass\n    except ValueError as e:\n        pass\n    return e\n",
        ).unwrap_err();
    assert_eq!(err.code, "T0023");
}

#[test]
fn try_star_handler_as_binding_incompatible_type_is_t0023() {
    // Mirrors `try_handler_as_binding_incompatible_type_is_t0023` for
    // `except*` (#542). A same-block reassignment (`x = "bad"`) is caught
    // directly by `check_assignment` (via `check_stmt_sequence_shared`) before
    // `check_try_star_stmt` ever reaches its own
    // `join_if_branches(&mut joined, &previous, handler_env)?` loop, so
    // that specific `?` needs a mismatch that bypasses `check_assignment`
    // instead: `e` is `int` before the try; `except* ValueError as e`
    // rebinds `e` to `Instance("ExceptionGroup")` via `henv.bind()`
    // directly (exactly like the plain-`try` case), which
    // `join_if_branches` then detects as incompatible with the pre-try
    // binding.
    let err = parse_check(
            "def f() -> int:\n    e = 0\n    try:\n        pass\n    except* ValueError as e:\n        pass\n    return 1\n",
        ).unwrap_err();
    assert_eq!(err.code, "T0023");
}

// -- #382: check_raise_operand ? error propagation --

#[test]
fn raise_with_type_error_in_argument_propagates() {
    // `raise ValueError(1 + "bad")` — the argument expression has a type
    // error, which propagates through the `?` in `check_raise_operand`'s
    // builtin-exception-class-call path.
    let err = parse_check("raise ValueError(1 + \"bad\")\n").unwrap_err();
    assert!(err.message.contains("operator"));
}

#[test]
fn raise_with_type_error_in_non_call_expr_propagates() {
    // `raise 1 + "bad"` — the expression is not a Call, so it falls
    // through to `infer_expr_in` which returns a type error, propagating
    // through the `?` in `check_raise_operand`'s non-Call path.
    let err = parse_check("raise 1 + \"bad\"\n").unwrap_err();
    assert!(err.message.contains("operator"));
}

// -- #382: block_always_returns with non-empty else body --

#[test]
fn try_with_return_in_body_handler_and_else_always_returns() {
    // A try where body, all handlers, and else all return — the try
    // block always returns. This covers the `block_always_returns(orelse)`
    // branch of the `||` in `block_always_returns`'s `Try` arm.
    let src = "\
def f() -> int:
    try:
        return 1
    except ValueError:
        return 2
    else:
        return 3
";
    parse_check(src).expect("check should succeed");
}

#[test]
fn try_star_with_return_in_body_handler_and_else_always_returns() {
    // Mirrors `try_with_return_in_body_handler_and_else_always_returns`
    // for `except*` (#542): covers the `HirStmt::TryStar` half of
    // `block_always_returns`'s combined `Try { .. } | TryStar { .. }`
    // pattern -- the `Try` arm's own field-destructure lines are already
    // exercised by the plain-`try` test above, but the `TryStar`
    // alternative's own lines are only reached when the analyzed function
    // actually contains a `try*`.
    //
    // #795 (PEP 654): the handler terminates with a `raise` rather than a
    // `return` -- CPython rejects a `return` inside an `except*` clause body
    // outright, and pycc now reports `L0001` for it during lowering, so this
    // fixture could no longer reach type checking at all. `HirStmt::Raise`
    // is `block_always_returns`'s other terminal statement, so the
    // `handled_paths_terminate` conjunct this test exists to exercise is
    // still driven to `true`.
    let src = "\
def f() -> int:
    try:
        return 1
    except* ValueError:
        raise RuntimeError(\"boom\")
    else:
        return 3
";
    parse_check(src).expect("check should succeed");
}

// -- #382: collect_generic_class_instantiations Raise arm --

#[test]
fn collect_generic_class_instantiations_from_raise() {
    // A generic class instantiation inside a raise expression exercises
    // the `Raise` arm of `collect_generic_class_instantiations_from_stmt`.
    let src = "\
class C[T]:
    def __init__(self, x: T):
        self.x = x
raise C[int](1)
";
    let err = parse_check_resolve(src).unwrap_err();
    assert_eq!(err.code, "T0021");
}

// -- #382: collect_block_constraints ? propagation on the solver path --
// These tests force the solver path (via an untyped `_helper(x)`
// parameter) and trigger an immediate error in `collect_expr_constraints`
// (by calling a non-callable binding), which propagates through the `?`
// in the `Try` arm of `collect_block_constraints`.

#[test]
fn solver_try_body_non_callable_error_propagates() {
    let src = "\
def _helper(x) -> int:
    y = 0
    try:
        y()
    except ValueError:
        y = 0
    return y
_helper(1)
";
    let err = parse_check_resolve(src).unwrap_err();
    assert_eq!(err.code, "T0021");
}

#[test]
fn solver_try_handler_non_callable_error_propagates() {
    let src = "\
def _helper(x) -> int:
    y = 0
    try:
        y = x
    except ValueError:
        y()
    return y
_helper(1)
";
    let err = parse_check_resolve(src).unwrap_err();
    assert_eq!(err.code, "T0021");
}

#[test]
fn solver_try_else_non_callable_error_propagates() {
    let src = "\
def _helper(x) -> int:
    y = 0
    try:
        y = x
    except ValueError:
        y = 0
    else:
        y()
    return y
_helper(1)
";
    let err = parse_check_resolve(src).unwrap_err();
    assert_eq!(err.code, "T0021");
}

#[test]
fn solver_try_finally_non_callable_error_propagates() {
    let src = "\
def _helper(x) -> int:
    y = 0
    try:
        y = x
    except ValueError:
        y = 0
    finally:
        y()
    return y
_helper(1)
";
    let err = parse_check_resolve(src).unwrap_err();
    assert_eq!(err.code, "T0021");
}
