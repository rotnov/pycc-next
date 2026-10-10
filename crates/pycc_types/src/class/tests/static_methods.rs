//! Static and class methods called through the class name or an instance.

use super::*;

#[test]
fn static_method_call_through_class_name_type_checks() {
    let hir = static_class_module(vec![top_level(HirStmt::ExprStmt(HirExpr::Call {
        callee: "print".to_string(),
        args: vec![HirExpr::MethodCall {
            base: Box::new(HirExpr::Name("C".to_string())),
            method: "create".to_string(),
            args: vec![HirExpr::IntLiteral(42)],
        }],
    }))]);
    check(&hir).expect("a static method call through a class name should type-check");
}

#[test]
fn class_method_call_through_class_name_type_checks() {
    let hir = static_class_module(vec![top_level(HirStmt::ExprStmt(HirExpr::Call {
        callee: "print".to_string(),
        args: vec![HirExpr::MethodCall {
            base: Box::new(HirExpr::Name("C".to_string())),
            method: "greet".to_string(),
            args: vec![HirExpr::IntLiteral(42)],
        }],
    }))]);
    check(&hir).expect("a class method call through a class name should type-check");
}

#[test]
fn static_method_call_through_instance_type_checks() {
    let hir = static_class_module(vec![
        top_level(HirStmt::Assign {
            target: "c".to_string(),
            value: HirExpr::Call {
                callee: "C".to_string(),
                args: vec![],
            },
        }),
        top_level(HirStmt::ExprStmt(HirExpr::Call {
            callee: "print".to_string(),
            args: vec![HirExpr::MethodCall {
                base: Box::new(HirExpr::Name("c".to_string())),
                method: "create".to_string(),
                args: vec![HirExpr::IntLiteral(42)],
            }],
        })),
    ]);
    check(&hir).expect("a static method call through an instance should type-check");
}

#[test]
fn class_method_call_through_instance_type_checks() {
    let hir = static_class_module(vec![
        top_level(HirStmt::Assign {
            target: "c".to_string(),
            value: HirExpr::Call {
                callee: "C".to_string(),
                args: vec![],
            },
        }),
        top_level(HirStmt::ExprStmt(HirExpr::Call {
            callee: "print".to_string(),
            args: vec![HirExpr::MethodCall {
                base: Box::new(HirExpr::Name("c".to_string())),
                method: "greet".to_string(),
                args: vec![HirExpr::IntLiteral(42)],
            }],
        })),
    ]);
    check(&hir).expect("a class method call through an instance should type-check");
}

#[test]
fn static_method_wrong_argument_type_is_rejected() {
    let hir = static_class_module(vec![top_level(HirStmt::ExprStmt(HirExpr::Call {
        callee: "print".to_string(),
        args: vec![HirExpr::MethodCall {
            base: Box::new(HirExpr::Name("C".to_string())),
            method: "create".to_string(),
            args: vec![HirExpr::StringLiteral("hi".to_string())],
        }],
    }))]);
    let diagnostic = check(&hir).unwrap_err();
    assert_eq!(diagnostic.code, "T0021");
}

#[test]
fn class_method_wrong_argument_type_is_rejected() {
    let hir = static_class_module(vec![top_level(HirStmt::ExprStmt(HirExpr::Call {
        callee: "print".to_string(),
        args: vec![HirExpr::MethodCall {
            base: Box::new(HirExpr::Name("C".to_string())),
            method: "greet".to_string(),
            args: vec![HirExpr::StringLiteral("hi".to_string())],
        }],
    }))]);
    let diagnostic = check(&hir).unwrap_err();
    assert_eq!(diagnostic.code, "T0021");
}

#[test]
fn static_method_wrong_argument_count_is_rejected() {
    let hir = static_class_module(vec![top_level(HirStmt::ExprStmt(HirExpr::Call {
        callee: "print".to_string(),
        args: vec![HirExpr::MethodCall {
            base: Box::new(HirExpr::Name("C".to_string())),
            method: "create".to_string(),
            args: vec![],
        }],
    }))]);
    let diagnostic = check(&hir).unwrap_err();
    assert_eq!(diagnostic.code, "T0021");
}

#[test]
fn class_method_excludes_cls_from_argument_count() {
    // `greet` takes `(cls, x: int)` — the caller passes only `x`. The
    // type checker excludes `cls` from the argument count check, so
    // passing one argument is correct.
    let hir = static_class_module(vec![top_level(HirStmt::ExprStmt(HirExpr::Call {
        callee: "print".to_string(),
        args: vec![HirExpr::MethodCall {
            base: Box::new(HirExpr::Name("C".to_string())),
            method: "greet".to_string(),
            args: vec![HirExpr::IntLiteral(42)],
        }],
    }))]);
    check(&hir).expect("cls is excluded from the argument count check");
}

#[test]
fn unknown_static_or_class_method_on_class_name_falls_through() {
    // A method call on a class name that is neither a static method nor
    // a class method falls through to the regular resolution, which
    // tries to infer the base expression (`HirExpr::Name("C")`). A bare
    // class name is not a binding, so `infer_expr_in` rejects it with
    // T0021 ("name `C` is not defined").
    let hir = static_class_module(vec![top_level(HirStmt::ExprStmt(HirExpr::Call {
        callee: "print".to_string(),
        args: vec![HirExpr::MethodCall {
            base: Box::new(HirExpr::Name("C".to_string())),
            method: "nonexistent".to_string(),
            args: vec![],
        }],
    }))]);
    let diagnostic = check(&hir).unwrap_err();
    assert_eq!(diagnostic.code, "T0021");
}

#[test]
fn attribute_access_on_a_class_name_is_rejected() {
    // `C.x` — accessing an attribute on a class name (not an instance)
    // is not supported. A static or class method must be called, not
    // accessed as a bare attribute value.
    let hir = static_class_module(vec![top_level(HirStmt::ExprStmt(HirExpr::Call {
        callee: "print".to_string(),
        args: vec![HirExpr::AttrGet {
            base: Box::new(HirExpr::Name("C".to_string())),
            attr: "create".to_string(),
        }],
    }))]);
    let diagnostic = check(&hir).unwrap_err();
    assert_eq!(diagnostic.code, "T0044");
}

#[test]
fn class_name_static_method_call_propagates_arg_inference_error() {
    // Exercises the `?` on argument inference at lib.rs:3289 inside the
    // class-name static/class method call branch of `infer_expr_in`.
    // `C.create(undefined_name)` — the class `C` is registered and has a
    // static method `create`, so the class-name branch is entered, but
    // the argument `undefined_name` fails inference (T0021 "name not
    // defined"), which propagates through the `collect::<Result<..>>?`.
    let hir = static_class_module(vec![top_level(HirStmt::ExprStmt(HirExpr::Call {
        callee: "print".to_string(),
        args: vec![HirExpr::MethodCall {
            base: Box::new(HirExpr::Name("C".to_string())),
            method: "create".to_string(),
            args: vec![HirExpr::Name("undefined_name".to_string())],
        }],
    }))]);
    let diagnostic = check(&hir).unwrap_err();
    assert_eq!(diagnostic.code, "T0021");
}
