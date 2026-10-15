//! Function signatures, parameters, type parameters and f-strings.

use super::*;

#[test]
fn lowers_a_fully_annotated_public_function_with_params_and_return() {
    let module =
        pycc_parser_test_helper::parse("def add(a: int, b: int) -> int:\n    return a + b\n");
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items,
        vec![HirItem::Function {
            name: "add".to_string(),
            params: vec![("a".to_string(), Ty::Int), ("b".to_string(), Ty::Int)],
            return_ty: Ty::Int,
            body: vec![HirStmt::Return(Some(HirExpr::BinOp {
                op: BinOpKind::Add,
                left: Box::new(HirExpr::Name("a".to_string())),
                right: Box::new(HirExpr::Name("b".to_string())),
            }))],
        }]
    );
}

#[test]
fn lowers_a_return_with_no_value() {
    let module = pycc_parser_test_helper::parse("def f() -> None:\n    return\n");
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items,
        vec![HirItem::Function {
            name: "f".to_string(),
            params: vec![],
            return_ty: Ty::None,
            body: vec![HirStmt::Return(None)],
        }]
    );
}

#[test]
fn an_unannotated_public_function_lowers_its_missing_param_annotations_as_infer() {
    // A native module infers them like a private helper's; `pycc_types`
    // reports `T0001` when inference cannot pin one.
    let module = pycc_parser_test_helper::parse("def add(a, b) -> int:\n    return a + b\n");
    let hir = lower_checked(&module).unwrap();
    let HirItem::Function { params, .. } = &hir.items[0] else {
        panic!("a function")
    };
    assert_eq!(params[0].1, Ty::Infer);
}

#[test]
fn an_unannotated_public_function_lowers_its_missing_return_annotation_as_infer() {
    let module = pycc_parser_test_helper::parse("def add(a: int, b: int):\n    return a + b\n");
    let hir = lower_checked(&module).unwrap();
    let HirItem::Function { return_ty, .. } = &hir.items[0] else {
        panic!("a function")
    };
    assert_eq!(*return_ty, Ty::Infer);
}

#[test]
fn an_unannotated_private_function_is_allowed() {
    let module = pycc_parser_test_helper::parse("def _add(a, b):\n    return a + b\n");
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items,
        vec![HirItem::Function {
            name: "_add".to_string(),
            params: vec![("a".to_string(), Ty::Infer), ("b".to_string(), Ty::Infer)],
            return_ty: Ty::Infer,
            body: vec![HirStmt::Return(Some(HirExpr::BinOp {
                op: BinOpKind::Add,
                left: Box::new(HirExpr::Name("a".to_string())),
                right: Box::new(HirExpr::Name("b".to_string())),
            }))],
        }]
    );
}

#[test]
fn every_supported_annotation_type_lowers_correctly() {
    let cases = [
        ("def f(x: int) -> int:\n    return x\n", Ty::Int),
        ("def f(x: float) -> float:\n    return x\n", Ty::Float),
        ("def f(x: bool) -> bool:\n    return x\n", Ty::Bool),
        ("def f(x: str) -> str:\n    return x\n", Ty::Str),
        ("def f(x: None) -> None:\n    return x\n", Ty::None),
    ];
    for (source, expected_ty) in cases {
        let module = pycc_parser_test_helper::parse(source);
        let hir = lower_checked(&module).unwrap();
        assert_eq!(
            hir.items,
            vec![HirItem::Function {
                name: "f".to_string(),
                params: vec![("x".to_string(), expected_ty.clone())],
                return_ty: expected_ty,
                body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
            }],
            "wrong lowering for {source:?}"
        );
    }
}

#[test]
fn a_non_bare_name_annotation_returns_a_capability_error() {
    assert_capability_error_message(
        "def f(x: a.b) -> None:\n    return\n",
        "only a bare name type annotation is supported so far",
    );
}

#[test]
fn a_default_parameter_value_lowers_the_parameter_with_its_annotated_type() {
    // Part 2 of #884 (#1189): a module-level `def` may now carry a literal
    // default. The lowered parameter list is exactly the one the same `def`
    // without the default produces -- a default changes no `Ty`, which is
    // what keeps `check_incompatible_redefinitions` comparing two such
    // `def`s equal. (This test replaces the pre-#1189
    // `a_default_parameter_value_returns_a_capability_error`, whose own
    // regression subject -- `lower_params` reading `.parameter` and silently
    // ignoring `.default` -- is now covered by the call-site fill tests in
    // `expr::keyword_bind`, since a silently ignored default would leave the
    // signature table with no default to splice.)
    let module =
        pycc_parser_test_helper::parse("def f(a: int, b: int = 2) -> int:\n    return a + b\n");
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items,
        vec![HirItem::Function {
            name: "f".to_string(),
            params: vec![("a".to_string(), Ty::Int), ("b".to_string(), Ty::Int)],
            return_ty: Ty::Int,
            body: vec![HirStmt::Return(Some(HirExpr::BinOp {
                op: BinOpKind::Add,
                left: Box::new(HirExpr::Name("a".to_string())),
                right: Box::new(HirExpr::Name("b".to_string())),
            }))],
        }]
    );
}

#[test]
fn a_positional_only_parameter_lowers_successfully() {
    // PEP 570 (#383): positional-only parameters (`/` marker) are now
    // lowered via the same path as ordinary args, prepended before `args`.
    let module =
        pycc_parser_test_helper::parse("def f(a: int, /, b: int) -> int:\n    return a + b\n");
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items,
        vec![HirItem::Function {
            name: "f".to_string(),
            params: vec![("a".to_string(), Ty::Int), ("b".to_string(), Ty::Int),],
            return_ty: Ty::Int,
            body: vec![HirStmt::Return(Some(HirExpr::BinOp {
                op: BinOpKind::Add,
                left: Box::new(HirExpr::Name("a".to_string())),
                right: Box::new(HirExpr::Name("b".to_string())),
            }))],
        }]
    );
}

#[test]
fn a_positional_only_parameter_with_a_default_value_lowers() {
    // PEP 570 (#383) + Part 2 of #884 (#1189): a default *before* the `/`
    // marker goes through the posonlyargs `lower_arg_list` call, which must
    // validate and keep it rather than have the concatenation bypass it.
    // `crates/pycc_hir/src/expr/keyword_bind.rs` owns the two call-site
    // halves of this shape: such a parameter fills positionally, and naming
    // it by keyword stays the PEP 570 `T0021`.
    let module = pycc_parser_test_helper::parse("def f(a: int = 0, /) -> int:\n    return a\n");
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items,
        vec![HirItem::Function {
            name: "f".to_string(),
            params: vec![("a".to_string(), Ty::Int)],
            return_ty: Ty::Int,
            body: vec![HirStmt::Return(Some(HirExpr::Name("a".to_string())))],
        }]
    );
}

#[test]
fn a_keyword_only_parameter_returns_a_capability_error() {
    assert_capability_error_message(
        "def f(a: int, *, b: int) -> int:\n    return a + b\n",
        "keyword-only parameters",
    );
}

#[test]
fn a_vararg_parameter_returns_a_capability_error() {
    assert_capability_error_message(
        "def f(*args: int) -> None:\n    return\n",
        "*args` is not supported yet",
    );
}

#[test]
fn a_kwarg_parameter_returns_a_capability_error() {
    assert_capability_error_message(
        "def f(**kwargs: int) -> None:\n    return\n",
        "**kwargs` is not supported yet",
    );
}

#[test]
fn an_async_function_is_rejected_without_losing_async_semantics() {
    assert_capability_error_message(
        "async def f() -> None:\n    return\n",
        "async functions are not supported yet",
    );
}

#[test]
fn a_decorated_function_is_rejected_without_losing_the_decorator() {
    assert_capability_error_message(
        "@decorator\ndef f() -> None:\n    return\n",
        "function decorators are not supported yet",
    );
}

#[test]
fn a_generic_function_with_two_type_parameters_is_rejected() {
    // D-133: exactly one type parameter is accepted (see
    // `a_single_type_parameter_is_lowered_to_ty_param` below); two or
    // more still hit the frontend arity gate, since the underlying
    // representation and call-site substitution (D-134) are scoped to
    // the single-type-parameter case only.
    assert_capability_error_message(
        "def f[T, U](x: T) -> U:\n    return x\n",
        "generic functions with more than one type parameter are not supported yet",
    );
}

#[test]
fn a_single_type_parameter_is_lowered_to_ty_param() {
    // D-133: `Ty::Param` is resolved by call-site substitution (D-134),
    // not by unification -- this test only asserts the frontend lowers
    // `T` consistently to `Ty::Param("T")` everywhere it appears in the
    // signature, not that substitution happens yet.
    let module = pycc_parser_test_helper::parse("def f[T](x: T) -> T:\n    return x\n");
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items,
        vec![HirItem::Function {
            name: "f".to_string(),
            params: vec![("x".to_string(), Ty::Param(Box::new("T".to_string())))],
            return_ty: Ty::Param(Box::new("T".to_string())),
            body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
        }]
    );
}

#[test]
fn a_type_var_tuple_type_parameter_is_rejected() {
    // D-133: `Ty::Param` models one `TypeVar` resolved to one concrete
    // scalar via call-site substitution (D-134) -- `*Ts` stands for a
    // variable-length sequence of types instead, which `Ty::Param`
    // cannot represent, so this must be an explicit capability
    // rejection rather than silently treated like a plain `TypeVar`.
    assert_capability_error_message(
        "def f[*Ts](x: int) -> None:\n    return\n",
        "a `TypeVarTuple` type parameter (`*Ts`) is not supported yet",
    );
}

#[test]
fn a_param_spec_type_parameter_is_rejected() {
    // D-133: same reasoning as the `TypeVarTuple` case above -- `**P`
    // stands for a parameter-list shape, not a single scalar type.
    assert_capability_error_message(
        "def f[**P](x: int) -> None:\n    return\n",
        "a `ParamSpec` type parameter (`**P`) is not supported yet",
    );
}

#[test]
fn a_keyword_call_argument_is_kept_instead_of_being_erased() {
    // Part 1 of #884 (#1125) made a keyword call to a module-level `def`
    // bind by name, so this regression needs a callee the signature table
    // cannot answer for. An undefined name is the simplest such callee, and
    // the point of the test is unchanged: the keyword is rejected, never
    // silently dropped on the way to `HirExpr::Call`. Since Part 8 of #1371
    // a bare-name callee could be a CPython object, so lowering keeps the
    // keyword in a `HirExpr::KeywordCall` and `pycc_types` refuses it.
    let hir = lower_checked(&pycc_parser_test_helper::parse("f(extra=undefined)\n"))
        .expect("a bare-name keyword call is deferred to pycc_types");
    let Some(HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::KeywordCall {
        call, keywords, ..
    }))) = hir.items.last()
    else {
        panic!("{:?}", hir.items);
    };
    assert!(
        matches!(call.as_ref(), HirExpr::Call { callee, args } if callee == "f" && args.is_empty()),
        "{call:?}"
    );
    assert_eq!(
        keywords,
        &vec![("extra".to_string(), HirExpr::Name("undefined".to_string()))]
    );
}

#[test]
fn a_keyword_range_argument_is_rejected_instead_of_being_erased() {
    assert_capability_error_message(
        "for i in range(stop=3):\n    i\n",
        "keyword arguments to range() are not supported yet",
    );
}

#[test]
fn lowers_a_plain_string_literal() {
    let module = pycc_parser_test_helper::parse("x = \"hi\"\n");
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items,
        vec![HirItem::TopLevelStmt(HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::StringLiteral("hi".to_string()),
        })]
    );
}

#[test]
fn an_any_annotation_produces_t0002() {
    let module = pycc_parser_test_helper::parse("def f(x: Any) -> None:\n    pass\n");
    let diag = lower_checked(&module).unwrap_err();
    assert_eq!(diag.code, "T0002");
}

#[test]
fn an_any_return_annotation_produces_t0002() {
    let module = pycc_parser_test_helper::parse("def f() -> Any:\n    pass\n");
    let diag = lower_checked(&module).unwrap_err();
    assert_eq!(diag.code, "T0002");
}

#[test]
fn lowers_a_basic_f_string_with_one_interpolation() {
    let module = pycc_parser_test_helper::parse("x = 1\ny = f\"value: {x}\"\n");
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items[1],
        HirItem::TopLevelStmt(HirStmt::Assign {
            target: "y".to_string(),
            value: HirExpr::FString(vec![
                FStringPart::Literal("value: ".to_string()),
                FStringPart::Interpolation(Box::new(HirExpr::Name("x".to_string()))),
            ]),
        })
    );
}

#[test]
fn lowers_an_f_string_with_only_literal_parts() {
    let module = pycc_parser_test_helper::parse("y = f\"no interpolation\"\n");
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items,
        vec![HirItem::TopLevelStmt(HirStmt::Assign {
            target: "y".to_string(),
            value: HirExpr::FString(vec![FStringPart::Literal("no interpolation".to_string())]),
        })]
    );
}

#[test]
fn an_f_string_with_a_nested_format_spec_field_is_not_supported_yet() {
    assert_capability_error_message("x = 1.5\nw = 4\ny = f\"{x:{w}}\"\n", "format spec");
}

#[test]
fn an_f_string_with_the_ascii_conversion_flag_is_not_supported_yet() {
    assert_capability_error_message("x = 1\ny = f\"{x!a}\"\n", "conversion flag");
}
