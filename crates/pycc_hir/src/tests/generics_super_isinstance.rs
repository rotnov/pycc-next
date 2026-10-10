//! Generic class instantiation, `super()` and `isinstance` evaluation.

use super::*;

// -- PEP 695 generic class instantiation (#387) -----------------------

/// Helper: lowers source that defines a generic class `C[T]` and then
/// uses `C[<type_arg>](<args>)` at module scope, returning the lowered
/// HIR so the test can inspect the `GenericClassInstantiate` expression.
fn lower_generic_class_instantiation(source: &str) -> crate::HirModule {
    let module = pycc_parser_test_helper::parse(source);
    lower_checked(&module).expect("test fixture should lower successfully")
}

#[test]
fn generic_class_instantiation_lowers_with_int_type_arg() {
    let hir = lower_generic_class_instantiation(
        "class C[T]:\n    def __init__(self, x: T) -> None:\n        self.x = x\nC[int](1)\n",
    );
    // The last item should be a top-level ExprStmt wrapping a
    // GenericClassInstantiate with class "C", type_arg Int, and one arg.
    let last = hir.items.last().expect("should have items");
    assert!(matches!(
        last,
        HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::GenericClassInstantiate {
            class,
            type_arg,
            args,
        })) if class == "C" && *type_arg == Ty::Int && args.len() == 1 && args[0] == HirExpr::IntLiteral(1)
    ));
}

#[test]
fn generic_class_instantiation_lowers_with_float_bool_and_str_type_args() {
    for (source, expected_ty) in [
        (
            "class C[T]:\n    def __init__(self, x: T) -> None:\n        self.x = x\nC[float](1)\n",
            Ty::Float,
        ),
        (
            "class C[T]:\n    def __init__(self, x: T) -> None:\n        self.x = x\nC[bool](1)\n",
            Ty::Bool,
        ),
        (
            "class C[T]:\n    def __init__(self, x: T) -> None:\n        self.x = x\nC[str](1)\n",
            Ty::Str,
        ),
    ] {
        let hir = lower_generic_class_instantiation(source);
        let last = hir.items.last().expect("should have items");
        assert!(matches!(
            last,
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::GenericClassInstantiate {
                type_arg, ..
            })) if *type_arg == expected_ty
        ));
    }
}

#[test]
fn generic_class_instantiation_rejects_a_non_name_type_arg() {
    // `C[1](args)` — the slice is a number literal, not a bare name.
    assert_capability_error_message(
        "class C[T]:\n    def __init__(self, x: T) -> None:\n        self.x = x\nC[1](1)\n",
        "a generic class type argument must be a bare type name",
    );
}

#[test]
fn generic_class_instantiation_rejects_an_unrecognized_type_arg_name() {
    // `C[unknown](args)` — the name is not one of int/float/bool/str.
    assert_capability_error_message(
        "class C[T]:\n    def __init__(self, x: T) -> None:\n        self.x = x\nC[unknown](1)\n",
        "a generic class type argument `unknown` is not supported yet",
    );
}

// `(1 + 2)[int](args)` -- a non-name subscript base -- lowers to a call of a
// subscript result since Part 2a of #1371; `tests/subscript_call.rs` pins it.

#[test]
fn generic_class_instantiation_propagates_an_arg_lowering_error() {
    // `C[int](lambda: 1)` — the arg `lambda: 1` is an unsupported
    // expression that `lower_expr` rejects. This exercises the `?` on
    // the `.collect::<Result<Vec<_>, _>>()?` at expr.rs line 352,
    // propagating the error from the arg's `lower_expr` call.
    assert_capability_error_message(
        "class C[T]:\n    def __init__(self, x: T) -> None:\n        self.x = x\nC[int](lambda: 1)\n",
        "expression kind not supported yet",
    );
}

#[test]
fn rename_name_in_expr_handles_generic_class_instantiate_in_comprehension() {
    // A list comprehension whose `elt` is a `GenericClassInstantiate`
    // expression exercises `rename_name_in_expr`'s
    // `GenericClassInstantiate` arm: the loop variable `x` inside the
    // instantiation's args is renamed to the comprehension's synthesized
    // variable. The expression must lower successfully and produce a
    // `ListCompAssign` whose `elt` is a `GenericClassInstantiate`.
    let hir = lower_generic_class_instantiation(
        "class C[T]:\n    def __init__(self, x: T) -> None:\n        self.x = x\nxs = [C[int](x) for x in range(3)]\n",
    );
    // Find the ListCompAssign statement.
    let comp = hir.items.iter().find_map(|item| match item {
        HirItem::TopLevelStmt(HirStmt::ListCompAssign { elt, .. }) => Some(elt.clone()),
        _ => None,
    });
    let elt = comp.expect("should find a ListCompAssign");
    // The elt should be a GenericClassInstantiate — proving
    // rename_name_in_expr's GenericClassInstantiate arm was traversed.
    assert!(
        matches!(elt.as_ref(), HirExpr::GenericClassInstantiate { class, type_arg, .. }
                if class == "C" && *type_arg == Ty::Int),
        "expected GenericClassInstantiate elt, got {elt:?}",
    );
}

// #433: super() lowering tests.

#[test]
fn super_init_lowers_to_method_call_with_super_base() {
    // `super().__init__()` inside a method body lowers to
    // `HirExpr::MethodCall { base: Super, method: "__init__", args: [] }`.
    let module = pycc_parser_test_helper::parse(
        "class A:\n    def __init__(self) -> None:\n        return\nclass B(A):\n    def __init__(self) -> None:\n        super().__init__()\n",
    );
    let hir = lower_checked(&module).unwrap();
    // Find B.__init__'s body.
    let init = hir.items.iter().find_map(|item| match item {
        HirItem::Function { name, body, .. } if name == "B.__init__" => Some(body.first().cloned()),
        _ => None,
    });
    let stmt = init
        .flatten()
        .expect("should find B.__init__ with a non-empty body");
    assert_eq!(
        stmt,
        HirStmt::ExprStmt(HirExpr::MethodCall {
            base: Box::new(HirExpr::Super),
            method: "__init__".to_string(),
            args: vec![],
        })
    );
}

#[test]
fn super_method_lowers_to_method_call_with_super_base() {
    // `super().greet()` inside a method body lowers to
    // `HirExpr::MethodCall { base: Super, method: "greet", args: [] }`.
    let module = pycc_parser_test_helper::parse(
        "class A:\n    def __init__(self) -> None:\n        return\n    def greet(self) -> int:\n        return 1\nclass B(A):\n    def __init__(self) -> None:\n        return\n    def greet(self) -> int:\n        return super().greet()\n",
    );
    let hir = lower_checked(&module).unwrap();
    let greet = hir.items.iter().find_map(|item| match item {
        HirItem::Function { name, body, .. } if name == "B.greet" => body.first().cloned(),
        _ => None,
    });
    let stmt = greet.expect("should find B.greet with a non-empty body");
    assert_eq!(
        stmt,
        HirStmt::Return(Some(HirExpr::MethodCall {
            base: Box::new(HirExpr::Super),
            method: "greet".to_string(),
            args: vec![],
        }))
    );
}

#[test]
fn super_attr_lowers_to_attr_get_with_super_base() {
    // `super().x` inside a method body lowers to
    // `HirExpr::AttrGet { base: Super, attr: "x" }`.
    let module = pycc_parser_test_helper::parse(
        "class A:\n    def __init__(self) -> None:\n        self.x = 1\nclass B(A):\n    def __init__(self) -> None:\n        super().__init__()\n    def get_x(self) -> int:\n        return super().x\n",
    );
    let hir = lower_checked(&module).unwrap();
    let get_x = hir.items.iter().find_map(|item| match item {
        HirItem::Function { name, body, .. } if name == "B.get_x" => body.first().cloned(),
        _ => None,
    });
    let stmt = get_x.expect("should find B.get_x with a non-empty body");
    assert_eq!(
        stmt,
        HirStmt::Return(Some(HirExpr::AttrGet {
            base: Box::new(HirExpr::Super),
            attr: "x".to_string(),
        }))
    );
}

#[test]
fn bare_super_outside_method_is_c0001() {
    // A bare `super()` at top level is rejected with C0001.
    let module = pycc_parser_test_helper::parse("x = super()\n");
    let err = lower_checked(&module).unwrap_err();
    assert_eq!(err.code, "C0001");
    assert!(
        err.message.contains("bare `super()`"),
        "should mention bare super(), got: {}",
        err.message
    );
}

#[test]
fn super_with_arguments_is_not_zero_arg_super() {
    // `super(A, self)` (two-arg super) is not a zero-arg super() call,
    // so `is_zero_arg_super_call` returns false — it falls through to
    // the ordinary `Expr::Call` path, which lowers `super(A, self)` as
    // a regular call to the `super` builtin. The type checker then
    // rejects it with C0001 ("call to builtin `super` is valid Python
    // but not implemented yet"). This test verifies the HIR lowering
    // succeeds (the rejection happens later, at type-check time).
    let module = pycc_parser_test_helper::parse(
        "class A:\n    def __init__(self) -> None:\n        super(A, self).__init__()\n",
    );
    let hir = lower_checked(&module).unwrap();
    // The base of the MethodCall should be a Call to "super", not a Super.
    assert_eq!(
        hir.items,
        vec![HirItem::Function {
            name: "A.__init__".to_string(),
            params: vec![("self".to_string(), Ty::Instance(Box::new("A".to_string())))],
            return_ty: Ty::None,
            body: vec![HirStmt::ExprStmt(HirExpr::MethodCall {
                base: Box::new(HirExpr::Call {
                    callee: "super".to_string(),
                    args: vec![
                        HirExpr::Name("A".to_string()),
                        HirExpr::Name("self".to_string()),
                    ],
                }),
                method: "__init__".to_string(),
                args: vec![],
            })],
        }]
    );
}

#[test]
fn super_method_outside_method_is_c0001() {
    // `super().foo()` at top level (outside a method body) is rejected
    // with C0001 at HIR-lowering time.
    let module = pycc_parser_test_helper::parse("x = super().foo()\n");
    let err = lower_checked(&module).unwrap_err();
    assert_eq!(err.code, "C0001");
}

#[test]
fn super_attr_outside_method_is_c0001() {
    // `super().foo` at top level (outside a method body) is rejected
    // with C0001 at HIR-lowering time.
    let module = pycc_parser_test_helper::parse("x = super().foo\n");
    let err = lower_checked(&module).unwrap_err();
    assert_eq!(err.code, "C0001");
}

#[test]
fn super_method_with_unsupported_arg_is_c0001() {
    // `super().foo(lambda: 1)` — the lambda argument is an unsupported
    // expression kind, so `lower_expr` on the argument returns Err, which
    // the `?` in the super().method() lowering path propagates as C0001.
    // (The conditional expression this test used before #1395 now lowers.)
    let module = pycc_parser_test_helper::parse(
        "class A:\n    def __init__(self) -> None:\n        super().foo(lambda: 1)\n",
    );
    let err = lower_checked(&module).unwrap_err();
    assert_eq!(err.code, "C0001");
}

#[test]
fn super_attr_assignment_is_c0001() {
    // #448: `super().attr = value` is rejected with a dedicated C0001
    // diagnostic that names super() attribute assignment, not the
    // confusing bare-super() message.
    let module = pycc_parser_test_helper::parse(
        "class A:\n    def __init__(self) -> None:\n        super().x = 1\n",
    );
    let err = lower_checked(&module).unwrap_err();
    assert_eq!(err.code, "C0001");
    assert!(
        err.message.contains("super().attr = value"),
        "should mention super().attr = value, got: {}",
        err.message
    );
    assert!(
        !err.message.contains("bare `super()`"),
        "should not use the bare-super() message, got: {}",
        err.message
    );
}

// -----------------------------------------------------------------------
// #435: compile-time isinstance/issubclass helper unit tests
// -----------------------------------------------------------------------

#[test]
fn eval_isinstance_single_covers_all_builtin_types() {
    // `float` target against `float` object — covers the `Ty::Float` arm.
    assert!(eval_isinstance_single(&Ty::Float, "float", &[]));
    assert!(!eval_isinstance_single(&Ty::Float, "int", &[]));
    // `None` object against any target — covers the `_ => false` arm.
    assert!(!eval_isinstance_single(&Ty::None, "int", &[]));
    // `Ty::Int` arm — true for "int", false for others.
    assert!(eval_isinstance_single(&Ty::Int, "int", &[]));
    assert!(!eval_isinstance_single(&Ty::Int, "str", &[]));
    // `Ty::Bool` arm — true for "bool" and "int" (subtype).
    assert!(eval_isinstance_single(&Ty::Bool, "bool", &[]));
    assert!(eval_isinstance_single(&Ty::Bool, "int", &[]));
    assert!(!eval_isinstance_single(&Ty::Bool, "str", &[]));
    // `Ty::Str` arm — true for "str", false for others.
    assert!(eval_isinstance_single(&Ty::Str, "str", &[]));
    assert!(!eval_isinstance_single(&Ty::Str, "int", &[]));
    // `Ty::Instance` arm — checks MRO membership.
    let mro = vec!["D".to_string(), "B".to_string(), "A".to_string()];
    assert!(eval_isinstance_single(
        &Ty::Instance(Box::new("D".to_string())),
        "D",
        &mro
    ));
    assert!(eval_isinstance_single(
        &Ty::Instance(Box::new("D".to_string())),
        "B",
        &mro
    ));
    assert!(eval_isinstance_single(
        &Ty::Instance(Box::new("D".to_string())),
        "A",
        &mro
    ));
    assert!(!eval_isinstance_single(
        &Ty::Instance(Box::new("D".to_string())),
        "C",
        &mro
    ));
}

#[test]
fn eval_issubclass_single_covers_builtin_same_type_and_user_vs_builtin() {
    // Same builtin type — covers the `return cls == target_class` line.
    assert!(eval_issubclass_single("int", "int", &[]));
    assert!(eval_issubclass_single("str", "str", &[]));
    assert!(!eval_issubclass_single("int", "str", &[]));
    // User class vs builtin target — covers the `return false` line.
    assert!(!eval_issubclass_single("D", "int", &["D".to_string()]));
    // `bool` is a subtype of `int` — covers the `bool`/`int` special case.
    assert!(eval_issubclass_single("bool", "int", &[]));
    assert!(eval_issubclass_single("bool", "bool", &[]));
    assert!(!eval_issubclass_single("bool", "str", &[]));
    // User class MRO check — covers the `cls_mro.iter().any` line.
    let mro = vec!["D".to_string(), "B".to_string(), "A".to_string()];
    assert!(eval_issubclass_single("D", "D", &mro));
    assert!(eval_issubclass_single("D", "B", &mro));
    assert!(eval_issubclass_single("D", "A", &mro));
    assert!(!eval_issubclass_single("D", "C", &mro));
}

#[test]
fn extract_class_names_rejects_empty_tuple_and_non_name_elements() {
    // Empty tuple — covers the `elements.is_empty()` error path.
    let result = extract_class_names(&HirExpr::TupleLiteral(vec![]));
    assert!(result.is_err());

    // Tuple with a non-name element — covers the `_ => return Err` path.
    let result = extract_class_names(&HirExpr::TupleLiteral(vec![HirExpr::IntLiteral(42)]));
    assert!(result.is_err());

    // Non-name, non-tuple expression — covers the top-level `_ => Err` path.
    let result = extract_class_names(&HirExpr::IntLiteral(99));
    assert!(result.is_err());

    // Valid single name.
    let result = extract_class_names(&HirExpr::Name("D".to_string()));
    assert_eq!(result.unwrap(), vec!["D".to_string()]);

    // Valid tuple of names.
    let result = extract_class_names(&HirExpr::TupleLiteral(vec![
        HirExpr::Name("B".to_string()),
        HirExpr::Name("C".to_string()),
    ]));
    assert_eq!(result.unwrap(), vec!["B".to_string(), "C".to_string()]);
}

#[test]
fn is_builtin_type_name_recognizes_four_scalar_types() {
    assert!(is_builtin_type_name("int"));
    assert!(is_builtin_type_name("str"));
    assert!(is_builtin_type_name("float"));
    assert!(is_builtin_type_name("bool"));
    assert!(!is_builtin_type_name("D"));
    assert!(!is_builtin_type_name("object"));
}
