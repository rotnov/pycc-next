//! Literal, operator and comparison inference basics; top-level `return`.

use super::*;

#[test]
fn v0_1_slice_always_type_checks() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert!(check(&hir).is_ok());
}

// #433: a bare `HirExpr::Super` should never reach `infer_expr_in` —
// HIR lowering rejects a standalone `super()` with C0001. This test
// bypasses HIR lowering with a hand-built HIR to exercise the
// defense-in-depth guard.
#[test]
fn bare_super_in_check_returns_c0001() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "f".to_string(),
            params: vec![],
            return_ty: Ty::None,
            body: vec![HirStmt::ExprStmt(HirExpr::Super)],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "C0001");
}

#[test]
fn infers_an_int_literal_as_int() {
    let env = Environment::new();
    assert_eq!(infer_expr(&env, &HirExpr::IntLiteral(1)), Ok(Ty::Int));
}

#[test]
fn infers_a_float_literal_as_float() {
    let env = Environment::new();
    assert_eq!(infer_expr(&env, &HirExpr::FloatLiteral(1.5)), Ok(Ty::Float));
}

#[test]
fn infers_a_bool_literal_as_bool() {
    let env = Environment::new();
    assert_eq!(infer_expr(&env, &HirExpr::BoolLiteral(true)), Ok(Ty::Bool));
}

#[test]
fn infers_a_string_literal_as_str() {
    let env = Environment::new();
    assert_eq!(
        infer_expr(&env, &HirExpr::StringLiteral("hi".to_string())),
        Ok(Ty::Str)
    );
}

#[test]
fn adding_an_int_and_a_str_is_a_clean_type_error() {
    let env = Environment::new();
    let expr = HirExpr::BinOp {
        op: BinOpKind::Add,
        left: Box::new(HirExpr::IntLiteral(1)),
        right: Box::new(HirExpr::StringLiteral("x".to_string())),
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
}

#[test]
fn adding_two_strings_infers_str() {
    let env = Environment::new();
    let expr = HirExpr::BinOp {
        op: BinOpKind::Add,
        left: Box::new(HirExpr::StringLiteral("a".to_string())),
        right: Box::new(HirExpr::StringLiteral("b".to_string())),
    };
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::Str));
}

#[test]
fn subtracting_two_strings_is_a_clean_type_error() {
    // Python allows `"a" + "b"` but no other arithmetic operator between
    // two strings -- `"a" - "b"` is a `TypeError` at runtime in CPython.
    let env = Environment::new();
    let expr = HirExpr::BinOp {
        op: BinOpKind::Sub,
        left: Box::new(HirExpr::StringLiteral("a".to_string())),
        right: Box::new(HirExpr::StringLiteral("b".to_string())),
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
}

#[test]
fn comparing_two_strings_infers_bool() {
    // `"a" == "b"`, `"a" < "b"`, etc. are ordinary, valid Python
    // (lexicographic ordering) -- not covered by numeric_or_bool_compatible
    // before `Ty::Str` became constructible via literals.
    let env = Environment::new();
    for op in [
        CmpOpKind::Eq,
        CmpOpKind::NotEq,
        CmpOpKind::Lt,
        CmpOpKind::LtE,
        CmpOpKind::Gt,
        CmpOpKind::GtE,
    ] {
        let expr = HirExpr::Compare {
            op,
            left: Box::new(HirExpr::StringLiteral("a".to_string())),
            right: Box::new(HirExpr::StringLiteral("b".to_string())),
        };
        assert_eq!(
            infer_expr(&env, &expr),
            Ok(Ty::Bool),
            "comparison {op:?} should type-check"
        );
    }
}

#[test]
fn an_f_string_always_infers_str_regardless_of_interpolated_types() {
    let env = Environment::new();
    let expr = HirExpr::FString(vec![
        FStringPart::Literal("n=".to_string()),
        FStringPart::Interpolation(Box::new(HirExpr::IntLiteral(1))),
    ]);
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::Str));
}

#[test]
fn an_f_string_still_type_checks_its_interpolated_expressions() {
    let env = Environment::new();
    let expr = HirExpr::FString(vec![FStringPart::Interpolation(Box::new(HirExpr::Name(
        "undefined".to_string(),
    )))]);
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
}

#[test]
fn comparing_a_string_and_an_int_is_a_clean_type_error() {
    let env = Environment::new();
    let expr = HirExpr::Compare {
        op: CmpOpKind::Eq,
        left: Box::new(HirExpr::StringLiteral("a".to_string())),
        right: Box::new(HirExpr::IntLiteral(1)),
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
}

#[test]
fn comparing_two_ints_infers_bool() {
    let env = Environment::new();
    let expr = HirExpr::Compare {
        op: CmpOpKind::Lt,
        left: Box::new(HirExpr::IntLiteral(1)),
        right: Box::new(HirExpr::IntLiteral(2)),
    };
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::Bool));
}

#[test]
fn comparing_a_bool_and_an_int_succeeds_since_bool_is_a_subtype_of_int() {
    let env = Environment::new();
    let expr = HirExpr::Compare {
        op: CmpOpKind::Eq,
        left: Box::new(HirExpr::IntLiteral(1)),
        right: Box::new(HirExpr::BoolLiteral(true)),
    };
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::Bool));
}

#[test]
fn comparing_an_undefined_left_operand_propagates_the_error() {
    let env = Environment::new();
    let expr = HirExpr::Compare {
        op: CmpOpKind::Eq,
        left: Box::new(HirExpr::Name("undefined".to_string())),
        right: Box::new(HirExpr::IntLiteral(1)),
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
}

#[test]
fn comparing_an_undefined_right_operand_propagates_the_error() {
    let env = Environment::new();
    let expr = HirExpr::Compare {
        op: CmpOpKind::Eq,
        left: Box::new(HirExpr::IntLiteral(1)),
        right: Box::new(HirExpr::Name("undefined".to_string())),
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
}

#[test]
fn comparing_incompatible_types_is_a_clean_type_error() {
    let mut env = Environment::new();
    // A call to a properly declared, zero-arg, `None`-returning function
    // legitimately infers `Ty::None`, which isn't numeric-like --
    // comparing an int against it is a genuine, both-sides-defined
    // incompatibility.
    env.bind_function("f".to_string(), vec![], Ty::None);
    let expr = HirExpr::Compare {
        op: CmpOpKind::Eq,
        left: Box::new(HirExpr::IntLiteral(1)),
        right: Box::new(HirExpr::Call {
            callee: "f".to_string(),
            args: vec![],
        }),
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("int") && err.message.contains("None"));
}

#[test]
fn a_binop_treats_bool_as_int() {
    let env = Environment::new();
    let expr = HirExpr::BinOp {
        op: BinOpKind::Add,
        left: Box::new(HirExpr::BoolLiteral(true)),
        right: Box::new(HirExpr::IntLiteral(1)),
    };
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::Int));
}

#[test]
fn a_binop_treats_bool_and_float_as_float() {
    let env = Environment::new();
    let expr = HirExpr::BinOp {
        op: BinOpKind::Add,
        left: Box::new(HirExpr::BoolLiteral(true)),
        right: Box::new(HirExpr::FloatLiteral(1.5)),
    };
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::Float));
}

#[test]
fn a_top_level_return_is_a_clean_diagnostic_not_a_panic() {
    // Regression test (self-review finding, pre-merge): this used to be
    // `panic!(...)`, so a bare `return` at module scope crashed the
    // compiler (exit code 101) instead of producing a diagnostic through
    // the documented exit-1 contract every other error path uses.
    // `ruff_python_parser` does not reject `return` outside a function at
    // the grammar level (CPython itself only rejects it in a later
    // compile pass), so this is reachable from ordinary CLI input.
    let mut env = Environment::new();
    let err = check_stmt(&mut env, &HirStmt::Return(None)).unwrap_err();
    assert_eq!(err.code, "T0024");
}

#[test]
fn check_and_resolve_also_rejects_a_top_level_return_with_t0024() {
    // Regression test: `collect_block_constraints` (the private-helper
    // solver) is invoked over top-level statements with `return_term:
    // None` (no enclosing function), so a top-level `Return` hits its
    // own defensive `let Some(return_term) = return_term else {
    // continue }` arm and is silently skipped by the solver -- it's
    // `check_and_resolve`'s later, ordinary `check_stmt` pass (Pass 2)
    // that actually rejects it with T0024, exactly like the
    // `pycc_types::check` entry point already does.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::TopLevelStmt(HirStmt::Return(None))],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let err = check_and_resolve(&hir).unwrap_err();
    assert_eq!(err.code, "T0024");
}

#[test]
fn a_return_nested_in_a_top_level_if_is_also_a_clean_diagnostic() {
    let mut env = Environment::new();
    let stmt = HirStmt::If {
        test: HirExpr::BoolLiteral(true),
        body: vec![HirStmt::Return(None)],
        orelse: vec![],
    };
    assert_eq!(check_stmt(&mut env, &stmt).unwrap_err().code, "T0024");
}
