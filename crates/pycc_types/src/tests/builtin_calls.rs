//! Builtin calls (`len`, `float`, `math`, conversions of foreign objects), enum markers and C0001 builtins.

use super::*;

#[test]
fn len_of_a_list_of_int_infers_int() {
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::List(Box::new(Ty::Int)));
    let expr = HirExpr::Call {
        callee: "len".to_string(),
        args: vec![HirExpr::Name("x".to_string())],
    };
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::Int));
}

#[test]
fn len_of_a_list_of_str_infers_int_proving_len_is_not_int_specific() {
    // Proves `len`'s own check isn't hardcoded to `Ty::Int` -- generic
    // over any scalar element type, same discipline already applied to
    // `Subscript`/`ListAppend`/`ForList` (D-105's own genericity claim).
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::List(Box::new(Ty::Str)));
    let expr = HirExpr::Call {
        callee: "len".to_string(),
        args: vec![HirExpr::Name("x".to_string())],
    };
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::Int));
}

#[test]
fn len_with_no_arguments_is_rejected_as_t0033() {
    let env = Environment::new();
    let expr = HirExpr::Call {
        callee: "len".to_string(),
        args: vec![],
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0033");
    assert_eq!(err.message, "`len` expects exactly 1 argument, got 0");
}

#[test]
fn len_with_two_arguments_is_rejected_as_t0033() {
    let env = Environment::new();
    let expr = HirExpr::Call {
        callee: "len".to_string(),
        args: vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)],
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0033");
    assert_eq!(err.message, "`len` expects exactly 1 argument, got 2");
}

#[test]
fn len_of_a_non_list_value_is_rejected_as_t0033() {
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::Int);
    let expr = HirExpr::Call {
        callee: "len".to_string(),
        args: vec![HirExpr::Name("x".to_string())],
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0033");
    assert_eq!(
        err.message,
        "`len` expects a `list[T]`, `dict[K, V]`, `set[T]`, or `frozenset[T]` argument, got `int`"
    );
}

#[test]
fn len_propagates_an_ill_typed_argument_s_error() {
    let env = Environment::new();
    let expr = HirExpr::Call {
        callee: "len".to_string(),
        args: vec![HirExpr::Name("undefined".to_string())],
    };
    assert_eq!(infer_expr(&env, &expr).unwrap_err().code, "T0021");
}

#[test]
fn float_of_an_int_infers_float() {
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::Int);
    let expr = HirExpr::Call {
        callee: "float".to_string(),
        args: vec![HirExpr::Name("x".to_string())],
    };
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::Float));
}

#[test]
fn float_of_a_bool_infers_float() {
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::Bool);
    let expr = HirExpr::Call {
        callee: "float".to_string(),
        args: vec![HirExpr::Name("x".to_string())],
    };
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::Float));
}

#[test]
fn float_of_a_float_infers_float() {
    // Proves `float`'s own check isn't a narrowing-only conversion --
    // an already-`float` argument is accepted too (identity), same
    // discipline as `len_of_a_list_of_str_infers_int_proving_len_is_
    // not_int_specific`'s own genericity claim above.
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::Float);
    let expr = HirExpr::Call {
        callee: "float".to_string(),
        args: vec![HirExpr::Name("x".to_string())],
    };
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::Float));
}

#[test]
fn float_with_no_arguments_is_rejected_as_t0021() {
    let env = Environment::new();
    let expr = HirExpr::Call {
        callee: "float".to_string(),
        args: vec![],
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(err.message, "`float` expects exactly 1 argument, got 0");
}

#[test]
fn float_with_two_arguments_is_rejected_as_t0021() {
    let env = Environment::new();
    let expr = HirExpr::Call {
        callee: "float".to_string(),
        args: vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)],
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(err.message, "`float` expects exactly 1 argument, got 2");
}

/// Also the pin for PR 4a of #1083's deliberate omission: `float`'s gate now
/// admits `Ty::Object` too, but the message still enumerates only the three
/// spellable types, because in a native build `object` is unspellable in an
/// annotation (`docs/TYPE_SYSTEM.md`'s `object` row; D-258 admits it only in
/// an `--ext` module) and "pass an `object`" is advice nobody can act on.
#[test]
fn float_of_a_list_is_rejected_as_t0021() {
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::List(Box::new(Ty::Int)));
    let expr = HirExpr::Call {
        callee: "float".to_string(),
        args: vec![HirExpr::Name("x".to_string())],
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(
        err.message,
        "`float` expects an `int`, `float`, or `bool` argument, got `list[int]`"
    );
}

#[test]
fn float_propagates_an_ill_typed_argument_s_error() {
    let env = Environment::new();
    let expr = HirExpr::Call {
        callee: "float".to_string(),
        args: vec![HirExpr::Name("undefined".to_string())],
    };
    assert_eq!(infer_expr(&env, &expr).unwrap_err().code, "T0021");
}

#[test]
fn math_sqrt_and_pi_type_check_through_the_constraint_solver_path() {
    // `math.sqrt`/`math.pi`'s `infer_expr_in` coverage above only
    // exercises pass 3 (the final, concrete-environment check). A
    // private helper with an unannotated (`Ty::Infer`) return forces
    // `collect_expr_constraints` (the solver pass) to resolve the same
    // call/name -- this is `inferred_signatures_keep_the_constraint_
    // solver_path`'s own precedent, applied to the new stdlib branch.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_root_plus_pi".to_string(),
            // `x` is annotated (not `Ty::Infer`) deliberately: like
            // `float`/`len`, `math.sqrt`'s solver-pass branch only
            // validates an already-concretely-resolved argument term
            // (see that branch's own doc comment on the "lenient-
            // until-known" precedent) -- it does not itself unify an
            // unresolved parameter's inference variable against the
            // stdlib signature's expected argument type. Only the
            // function's own `Ty::Infer` *return* is left for the
            // solver to determine here.
            params: vec![("x".to_string(), Ty::Float)],
            return_ty: Ty::Infer,
            body: vec![HirStmt::Return(Some(HirExpr::BinOp {
                op: BinOpKind::Add,
                left: Box::new(HirExpr::Call {
                    callee: "math.sqrt".to_string(),
                    args: vec![HirExpr::Name("x".to_string())],
                }),
                right: Box::new(HirExpr::Name("math.pi".to_string())),
            }))],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let local_names = module_function_local_names(&hir);
    let signatures = infer_function_signatures_with_solver(&hir, &local_names).unwrap();
    assert_eq!(signatures["_root_plus_pi"], (vec![Ty::Float], Ty::Float));
    assert!(check(&hir).is_ok());
}

#[test]
fn math_sqrt_wrong_arity_is_rejected_by_the_constraint_solver_path() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_bad_call".to_string(),
            params: vec![],
            return_ty: Ty::Infer,
            body: vec![HirStmt::Return(Some(HirExpr::Call {
                callee: "math.sqrt".to_string(),
                args: vec![],
            }))],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let local_names = module_function_local_names(&hir);
    let err = infer_function_signatures_with_solver(&hir, &local_names).unwrap_err();
    assert_eq!(err.code, "T0021");
}

#[test]
fn math_sqrt_wrong_argument_type_is_rejected_by_the_constraint_solver_path() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_bad_call".to_string(),
            params: vec![],
            return_ty: Ty::Infer,
            body: vec![HirStmt::Return(Some(HirExpr::Call {
                callee: "math.sqrt".to_string(),
                args: vec![HirExpr::StringLiteral("x".to_string())],
            }))],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let local_names = module_function_local_names(&hir);
    let err = infer_function_signatures_with_solver(&hir, &local_names).unwrap_err();
    assert_eq!(err.code, "T0021");
}

#[test]
fn math_pi_called_like_a_function_is_rejected_by_the_constraint_solver_path() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_bad_call".to_string(),
            params: vec![],
            return_ty: Ty::Infer,
            body: vec![HirStmt::Return(Some(HirExpr::Call {
                callee: "math.pi".to_string(),
                args: vec![],
            }))],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let local_names = module_function_local_names(&hir);
    let err = infer_function_signatures_with_solver(&hir, &local_names).unwrap_err();
    assert_eq!(err.code, "T0021");
}

#[test]
fn math_sqrt_used_as_a_bare_value_is_rejected_by_the_constraint_solver_path() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_bad_ref".to_string(),
            params: vec![],
            return_ty: Ty::Infer,
            body: vec![HirStmt::Return(Some(HirExpr::Name(
                "math.sqrt".to_string(),
            )))],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let local_names = module_function_local_names(&hir);
    let err = infer_function_signatures_with_solver(&hir, &local_names).unwrap_err();
    assert_eq!(err.code, "T0021");
}

#[test]
fn a_dotted_name_whose_module_part_is_not_a_registered_stdlib_module_is_an_ordinary_undefined_name()
{
    // `std_qualified_symbol`'s `resolve_module` step is defense-in-depth
    // against a hand-constructed `HirModule` (this crate's own public
    // API accepts any `&HirModule`, not only one `pycc_hir::lower_checked`
    // produced) -- `pycc_hir`'s real lowering never emits a dotted name
    // whose module part fails to resolve (see `lower_expr`'s own
    // `Expr::Attribute` arm), but this crate does not get to assume
    // that invariant holds for every caller.
    let env = Environment::new();
    let expr = HirExpr::Name("os.path".to_string());
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(err.message, "name `os.path` is not defined");
}

#[test]
fn math_sqrt_of_a_float_infers_float() {
    let env = Environment::new();
    let expr = HirExpr::Call {
        callee: "math.sqrt".to_string(),
        args: vec![HirExpr::FloatLiteral(2.0)],
    };
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::Float));
}

#[test]
fn math_pi_infers_float() {
    let env = Environment::new();
    let expr = HirExpr::Name("math.pi".to_string());
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::Float));
}

#[test]
fn math_sqrt_with_no_arguments_is_rejected_as_t0021() {
    let env = Environment::new();
    let expr = HirExpr::Call {
        callee: "math.sqrt".to_string(),
        args: vec![],
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(err.message, "`math.sqrt` expects 1 argument(s), got 0");
}

#[test]
fn math_sqrt_of_a_str_is_rejected_as_t0021() {
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::Str);
    let expr = HirExpr::Call {
        callee: "math.sqrt".to_string(),
        args: vec![HirExpr::Name("x".to_string())],
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(err.message, "`math.sqrt` expects `float`, got `str`");
}

#[test]
fn math_sqrt_propagates_an_ill_typed_argument_s_error() {
    let env = Environment::new();
    let expr = HirExpr::Call {
        callee: "math.sqrt".to_string(),
        args: vec![HirExpr::Name("undefined".to_string())],
    };
    assert_eq!(infer_expr(&env, &expr).unwrap_err().code, "T0021");
}

#[test]
fn math_pi_called_like_a_function_is_rejected_as_t0021() {
    let env = Environment::new();
    let expr = HirExpr::Call {
        callee: "math.pi".to_string(),
        args: vec![],
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(
        err.message,
        "`math.pi` is a stdlib constant, not a function, and cannot be called"
    );
}

#[test]
fn math_sqrt_used_as_a_bare_value_is_rejected_as_t0021() {
    let env = Environment::new();
    let expr = HirExpr::Name("math.sqrt".to_string());
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(
        err.message,
        "`math.sqrt` is a stdlib function and must be called, e.g. `math.sqrt(...)`"
    );
}

#[test]
fn enum_marker_used_as_a_bare_value_is_rejected_as_t0021() {
    let env = Environment::new();
    let expr = HirExpr::Name("enum.Enum".to_string());
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("class marker"));
}

#[test]
fn enum_marker_called_like_a_function_is_rejected_as_t0021() {
    let env = Environment::new();
    let expr = HirExpr::Call {
        callee: "enum.Enum".to_string(),
        args: vec![],
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("class marker"));
}

#[test]
fn enum_marker_used_as_a_bare_value_is_rejected_by_the_constraint_solver_path() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_bad_ref".to_string(),
            params: vec![],
            return_ty: Ty::Infer,
            body: vec![HirStmt::Return(Some(HirExpr::Name(
                "enum.Enum".to_string(),
            )))],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let local_names = module_function_local_names(&hir);
    let err = infer_function_signatures_with_solver(&hir, &local_names).unwrap_err();
    assert_eq!(err.code, "T0021");
}

#[test]
fn enum_marker_called_like_a_function_is_rejected_by_the_constraint_solver_path() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_bad_call".to_string(),
            params: vec![],
            return_ty: Ty::Infer,
            body: vec![HirStmt::Return(Some(HirExpr::Call {
                callee: "enum.Enum".to_string(),
                args: vec![],
            }))],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let local_names = module_function_local_names(&hir);
    let err = infer_function_signatures_with_solver(&hir, &local_names).unwrap_err();
    assert_eq!(err.code, "T0021");
}

#[test]
fn math_sqrt_call_is_shadowed_by_a_bound_local_named_math() {
    // Post-review finding: a real local/parameter legally named `math`
    // must shadow the stdlib module -- CPython would raise
    // `AttributeError` calling `.sqrt` on whatever `math` is actually
    // bound to here, not silently call libm's `sqrt`.
    let err = infer_expr_in(
        &Environment::new(),
        &["math"],
        &HirExpr::Call {
            callee: "math.sqrt".to_string(),
            args: vec![HirExpr::FloatLiteral(2.0)],
        },
    )
    .unwrap_err();
    assert_eq!(err.code, "C0001");
    assert_eq!(
        err.message,
        "`math` is a local name here, not the stdlib `math` module -- attribute access on a non-module value is not supported yet"
    );
}

#[test]
fn math_pi_reference_is_shadowed_by_a_bound_local_named_math() {
    let err = infer_expr_in(
        &Environment::new(),
        &["math"],
        &HirExpr::Name("math.pi".to_string()),
    )
    .unwrap_err();
    assert_eq!(err.code, "C0001");
    assert_eq!(
        err.message,
        "`math` is a local name here, not the stdlib `math` module -- attribute access on a non-module value is not supported yet"
    );
}

#[test]
fn math_sqrt_call_is_shadowed_by_a_module_level_binding_named_math() {
    let mut env = Environment::new();
    env.bind("math".to_string(), Ty::Float);
    let err = infer_expr(
        &env,
        &HirExpr::Call {
            callee: "math.sqrt".to_string(),
            args: vec![HirExpr::FloatLiteral(2.0)],
        },
    )
    .unwrap_err();
    assert_eq!(err.code, "C0001");
}

#[test]
fn math_sqrt_and_pi_are_shadowed_by_a_bound_local_through_the_constraint_solver_path() {
    // The other half of the fix -- the private-helper solver pass
    // (`collect_expr_constraints`) needs the same shadowing guard as
    // `infer_expr_in` above.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_shadowed".to_string(),
            params: vec![("math".to_string(), Ty::Float)],
            return_ty: Ty::Infer,
            body: vec![HirStmt::Return(Some(HirExpr::Call {
                callee: "math.sqrt".to_string(),
                args: vec![HirExpr::Name("math".to_string())],
            }))],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let local_names = module_function_local_names(&hir);
    let err = infer_function_signatures_with_solver(&hir, &local_names).unwrap_err();
    assert_eq!(err.code, "C0001");
}

#[test]
fn math_pi_bare_reference_is_shadowed_by_a_bound_local_through_the_constraint_solver_path() {
    // The `HirExpr::Name` (bare-reference) half of the solver-path
    // shadowing guard, as opposed to the `HirExpr::Call` half already
    // covered by `math_sqrt_and_pi_are_shadowed_by_a_bound_local_
    // through_the_constraint_solver_path` above.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_shadowed_pi".to_string(),
            params: vec![("math".to_string(), Ty::Float)],
            return_ty: Ty::Infer,
            body: vec![HirStmt::Return(Some(HirExpr::Name("math.pi".to_string())))],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let local_names = module_function_local_names(&hir);
    let err = infer_function_signatures_with_solver(&hir, &local_names).unwrap_err();
    assert_eq!(err.code, "C0001");
}

#[test]
fn a_user_defined_float_function_takes_priority_over_the_builtin() {
    // Post-merge review finding: unlike `len`/`print`, which have been
    // hand-recognized since before this compiler could compile
    // user-declared functions at all, `float` was undefined until #181,
    // so `def float(x: int) -> int: return x + 1` was a valid, working
    // program on `main` immediately before this builtin landed --
    // reproduced directly against a pristine `main` checkout, printing
    // `6`. Without this priority check, the builtin would silently
    // intercept the call and infer `Ty::Float` instead of the user's own
    // declared `Ty::Int` return type.
    let mut env = Environment::new();
    env.bind_function("float".to_string(), vec![Ty::Int], Ty::Int);
    let expr = HirExpr::Call {
        callee: "float".to_string(),
        args: vec![HirExpr::IntLiteral(5)],
    };
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::Int));
}

#[test]
fn a_user_defined_float_function_accepting_a_non_numeric_argument_type_checks() {
    // The other half of the same finding: the builtin's own argument-type
    // gate (`int`/`float`/`bool` only) must not apply when the user's own
    // `float` accepts something else entirely, e.g. `str`.
    let mut env = Environment::new();
    env.bind_function("float".to_string(), vec![Ty::Str], Ty::Str);
    let expr = HirExpr::Call {
        callee: "float".to_string(),
        args: vec![HirExpr::StringLiteral("hello".to_string())],
    };
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::Str));
}

// -- Issue #142: callable builtin classification as C0001 ------------

#[test]
fn known_callable_builtins_table_is_sorted() {
    // The `is_known_callable_builtin` binary search relies on
    // `KNOWN_CALLABLE_BUILTINS` being sorted by Rust's `str` ordering.
    // If this invariant is ever violated, binary search silently returns
    // false negatives -- a known builtin would be misclassified as
    // T0021 instead of C0001. The explicit `let` bindings ensure both
    // values are always evaluated, and the `assert!` carries no format
    // args -- the idiom these tests adopted while they lived in `lib.rs`
    // and counted toward the D-014 denominator.
    for w in KNOWN_CALLABLE_BUILTINS.windows(2) {
        let prev = w[0];
        let next = w[1];
        assert!(prev <= next, "KNOWN_CALLABLE_BUILTINS is not sorted");
        // Strict ordering (no duplicates): `prev < next` is the real
        // invariant, but `prev <= next` above already covers both
        // values. This redundant strict check catches duplicates without
        // introducing an uncovered format-arg branch.
        assert!(prev != next, "KNOWN_CALLABLE_BUILTINS has a duplicate");
    }
}

#[test]
fn known_callable_builtins_table_has_135_entries() {
    assert_eq!(
        KNOWN_CALLABLE_BUILTINS.len(),
        135,
        "KNOWN_CALLABLE_BUILTINS should have 135 entries (139 builtins minus print/len/float/isinstance/issubclass, plus __import__)"
    );
}

#[test]
fn known_callable_builtins_excludes_already_implemented() {
    // `print`, `len`, `float`, `isinstance`, and `issubclass` are already
    // hand-recognized and must not appear in the table -- they would never
    // reach this fallback.
    assert!(!is_known_callable_builtin("print"));
    assert!(!is_known_callable_builtin("len"));
    assert!(!is_known_callable_builtin("float"));
    assert!(!is_known_callable_builtin("isinstance"));
    assert!(!is_known_callable_builtin("issubclass"));
}

#[test]
fn is_known_callable_builtin_finds_table_entries() {
    // Representative samples from both halves of the table (exception
    // classes and ordinary builtins).
    assert!(is_known_callable_builtin("ValueError"));
    assert!(is_known_callable_builtin("Exception"));
    assert!(is_known_callable_builtin("int"));
    assert!(is_known_callable_builtin("range"));
    assert!(is_known_callable_builtin("zip"));
    assert!(is_known_callable_builtin("ArithmeticError"));
    assert!(is_known_callable_builtin("ZeroDivisionError"));
}

#[test]
fn is_known_callable_builtin_rejects_unknown_names() {
    assert!(!is_known_callable_builtin("totally_undefined"));
    assert!(!is_known_callable_builtin("print"));
    assert!(!is_known_callable_builtin(""));
}

#[test]
fn value_error_call_produces_c0001_not_t0021() {
    // `ValueError("x")` is valid Python -- the builtin genuinely exists
    // in Python 3.14 -- so it must be classified as a capability gap
    // (C0001), not a name-resolution failure (T0021).
    let env = Environment::new();
    let expr = HirExpr::Call {
        callee: "ValueError".to_string(),
        args: vec![HirExpr::StringLiteral("x".to_string())],
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "C0001");
    assert!(err.message.contains("ValueError"));
}

#[test]
fn exception_call_produces_c0001() {
    let env = Environment::new();
    let expr = HirExpr::Call {
        callee: "Exception".to_string(),
        args: vec![HirExpr::StringLiteral("msg".to_string())],
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "C0001");
    assert!(err.message.contains("Exception"));
}

#[test]
fn other_callable_builtins_produce_c0001() {
    // A representative sample of other callable builtins from the table.
    for name in ["bool", "range", "zip", "dict", "list", "sum"] {
        let env = Environment::new();
        let expr = HirExpr::Call {
            callee: name.to_string(),
            args: vec![HirExpr::IntLiteral(1)],
        };
        let err = infer_expr(&env, &expr).unwrap_err();
        assert_eq!(err.code, "C0001", "builtin `{name}` should be C0001");
        assert!(err.message.contains(name));
    }
}

#[test]
fn a_genuinely_undefined_function_still_produces_t0021() {
    // A typo or genuinely undefined name must retain the T0021
    // name-resolution behavior, not be reclassified as C0001.
    let env = Environment::new();
    let expr = HirExpr::Call {
        callee: "totally_undefined".to_string(),
        args: vec![HirExpr::IntLiteral(1)],
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("undefined function"));
}

#[test]
fn a_user_defined_value_error_takes_priority_over_c0001() {
    // A user `def ValueError(...)` is called correctly, not classified as
    // C0001 -- user definitions always take priority over the builtin
    // classification, matching `float`'s own user-definition-takes-
    // priority precedent.
    let mut env = Environment::new();
    env.bind_function("ValueError".to_string(), vec![Ty::Str], Ty::Int);
    let expr = HirExpr::Call {
        callee: "ValueError".to_string(),
        args: vec![HirExpr::StringLiteral("x".to_string())],
    };
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::Int));
}

#[test]
fn a_user_defined_exception_takes_priority_over_c0001() {
    let mut env = Environment::new();
    env.bind_function("Exception".to_string(), vec![Ty::Str], Ty::Bool);
    let expr = HirExpr::Call {
        callee: "Exception".to_string(),
        args: vec![HirExpr::StringLiteral("msg".to_string())],
    };
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::Bool));
}

#[test]
fn range_as_a_standalone_call_produces_c0001() {
    // `range` works in `for` loops (via `HirStmt::ForRange`) but not as
    // a standalone call -- a standalone `range(10)` is C0001.
    let env = Environment::new();
    let expr = HirExpr::Call {
        callee: "range".to_string(),
        args: vec![HirExpr::IntLiteral(10)],
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "C0001");
    assert!(err.message.contains("range"));
}

#[test]
fn import_dunder_call_produces_c0001_not_t0021() {
    // `__import__("math")` is valid Python -- the callable dunder
    // `__import__` is the one dunder users legitimately call, so it
    // is included in the builtin table. It must be classified as C0001,
    // not T0021 (Codex review P2).
    let env = Environment::new();
    let expr = HirExpr::Call {
        callee: "__import__".to_string(),
        args: vec![HirExpr::StringLiteral("math".to_string())],
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "C0001");
    assert!(err.message.contains("__import__"));
}

// ---------------------------------------------------------------------
// PR 4a of #1083 (Part 4 of #1026): `float(o)` and `bool(o)`.
//
// `tests/issue_1083_foreign_conversions.rs` drives the same claims through
// the public CLI; these pin the two arms directly, including the shapes the
// CLI cannot reach because `pycc_hir` or an earlier pass refuses them first.
// ---------------------------------------------------------------------

#[test]
fn float_of_a_foreign_object_is_admitted_as_float() {
    let mut env = Environment::new();
    env.bind("o".to_string(), Ty::Object);
    let expr = HirExpr::Call {
        callee: "float".to_string(),
        args: vec![HirExpr::Name("o".to_string())],
    };
    assert_eq!(infer_expr(&env, &expr).unwrap(), Ty::Float);
}

#[test]
fn bool_of_a_foreign_object_is_admitted_as_bool() {
    let mut env = Environment::new();
    env.bind("o".to_string(), Ty::Object);
    let expr = HirExpr::Call {
        callee: "bool".to_string(),
        args: vec![HirExpr::Name("o".to_string())],
    };
    assert_eq!(infer_expr(&env, &expr).unwrap(), Ty::Bool);
}

#[test]
fn bool_of_a_non_object_keeps_its_c0001_refusal() {
    // Fork 1 of the plan, stated openly: Part 4 relaxes exactly the object
    // case. `bool(1)` is still the capability-gap refusal it was, and
    // #1017/#1018 own the general builtin-conversion story.
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::Int);
    let expr = HirExpr::Call {
        callee: "bool".to_string(),
        args: vec![HirExpr::Name("x".to_string())],
    };
    assert_eq!(infer_expr(&env, &expr).unwrap_err().code, "C0001");
}

#[test]
fn bool_of_a_foreign_object_with_a_second_argument_keeps_its_c0001_refusal() {
    // The arm admits exactly one argument, so a mis-arity call falls through
    // to the same C0001 every other unsupported `bool` shape gets rather
    // than inventing a T0021 of its own.
    let mut env = Environment::new();
    env.bind("o".to_string(), Ty::Object);
    let expr = HirExpr::Call {
        callee: "bool".to_string(),
        args: vec![
            HirExpr::Name("o".to_string()),
            HirExpr::Name("o".to_string()),
        ],
    };
    assert_eq!(infer_expr(&env, &expr).unwrap_err().code, "C0001");
}

#[test]
fn int_and_str_of_a_foreign_object_are_admitted_as_int_and_str() {
    // Part 4 of #1026 (PR 4b of #1083), `bool_of_a_foreign_object_is_
    // admitted_as_bool`'s claim for the other two conversions.
    for (callee, expected) in [("int", Ty::Int), ("str", Ty::Str)] {
        let mut env = Environment::new();
        env.bind("o".to_string(), Ty::Object);
        let expr = HirExpr::Call {
            callee: callee.to_string(),
            args: vec![HirExpr::Name("o".to_string())],
        };
        assert_eq!(infer_expr(&env, &expr).unwrap(), expected, "{callee}");
    }
}

#[test]
fn str_of_a_non_object_keeps_its_c0001_refusal() {
    // `str(x)` of a scalar is a runtime builtin (`pycc_hir::builtins`); a
    // set argument is still a capability gap.
    for callee in ["str"] {
        let mut env = Environment::new();
        env.bind("x".to_string(), Ty::Set(Box::new(Ty::Int)));
        let expr = HirExpr::Call {
            callee: callee.to_string(),
            args: vec![HirExpr::Name("x".to_string())],
        };
        assert_eq!(
            infer_expr(&env, &expr).unwrap_err().code,
            "C0001",
            "{callee}"
        );
    }
}

#[test]
fn int_and_str_of_a_foreign_object_with_a_second_argument_keep_their_c0001_refusal() {
    // Each arm admits exactly one argument, so a mis-arity call falls through
    // to the same C0001 every other unsupported shape of that name gets
    // rather than inventing a T0021 of its own.
    for callee in ["int", "str"] {
        let mut env = Environment::new();
        env.bind("o".to_string(), Ty::Object);
        let expr = HirExpr::Call {
            callee: callee.to_string(),
            args: vec![
                HirExpr::Name("o".to_string()),
                HirExpr::Name("o".to_string()),
            ],
        };
        assert_eq!(
            infer_expr(&env, &expr).unwrap_err().code,
            "C0001",
            "{callee}"
        );
    }
}
