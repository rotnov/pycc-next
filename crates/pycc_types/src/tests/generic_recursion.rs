//! The generic self-recursion gate and generic-to-generic calls.

use super::*;

// ---------------------------------------------------------------
// PR-13 final review fixes: self/mutual generic recursion (Critical),
// consistent `type_aliases` on both `check_and_resolve` paths (I1),
// and a generic body seeing its non-generic siblings (I3).
// ---------------------------------------------------------------

/// `def rec[T](x: T, n: int) -> T` whose body calls `rec` again --
/// the exact shape that used to be accepted by `check` and then ICE
/// in `pycc_mir` during `build`.
fn self_recursive_generic_module() -> HirModule {
    let param = Ty::Param(Box::new("T".to_string()));
    HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "rec".to_string(),
                params: vec![("x".to_string(), param.clone()), ("n".to_string(), Ty::Int)],
                return_ty: param,
                body: vec![
                    HirStmt::If {
                        test: HirExpr::Compare {
                            op: CmpOpKind::Gt,
                            left: Box::new(HirExpr::Name("n".to_string())),
                            right: Box::new(HirExpr::IntLiteral(0)),
                        },
                        body: vec![HirStmt::Return(Some(HirExpr::Call {
                            callee: "rec".to_string(),
                            args: vec![
                                HirExpr::Name("x".to_string()),
                                HirExpr::BinOp {
                                    op: BinOpKind::Sub,
                                    left: Box::new(HirExpr::Name("n".to_string())),
                                    right: Box::new(HirExpr::IntLiteral(1)),
                                },
                            ],
                        }))],
                        orelse: vec![],
                    },
                    HirStmt::Return(Some(HirExpr::Name("x".to_string()))),
                ],
            },
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
                callee: "print".to_string(),
                args: vec![HirExpr::Call {
                    callee: "rec".to_string(),
                    args: vec![HirExpr::IntLiteral(5), HirExpr::IntLiteral(3)],
                }],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    }
}

/// A call to the enclosing generic function itself -- the shape
/// `reject_generic_calls_in_block` must find wherever it is nested.
fn self_call() -> HirExpr {
    HirExpr::Call {
        callee: "f".to_string(),
        args: vec![],
    }
}

fn benign() -> HirExpr {
    HirExpr::IntLiteral(0)
}

fn assert_self_call_found(stmt: HirStmt) {
    let err = reject_generic_calls_in_block(&Environment::new(), "f", &[stmt]).unwrap_err();
    assert_eq!(err.code, "T0042");
    assert!(err.message.contains("calls itself"));
}

#[test]
fn the_generic_recursion_gate_finds_a_self_call_in_every_statement_position() {
    // `reject_generic_calls_in_stmt` mirrors
    // `rewrite_generic_calls_in_stmt`'s own exhaustive statement walk;
    // every arm must actually reach the calls it holds, so each one
    // gets a case here (D-014's region gate would otherwise be
    // satisfied by an arm that silently visits nothing).
    let cases = vec![
        HirStmt::ExprStmt(self_call()),
        HirStmt::Assign {
            target: "t".to_string(),
            value: self_call(),
        },
        HirStmt::AnnAssign {
            is_final: false,
            target: "t".to_string(),
            annotation: Ty::Int,
            value: Some(self_call()),
        },
        HirStmt::Return(Some(self_call())),
        HirStmt::If {
            test: self_call(),
            body: vec![],
            orelse: vec![],
        },
        HirStmt::If {
            test: benign(),
            body: vec![HirStmt::ExprStmt(self_call())],
            orelse: vec![],
        },
        HirStmt::If {
            test: benign(),
            body: vec![],
            orelse: vec![HirStmt::ExprStmt(self_call())],
        },
        HirStmt::While {
            test: self_call(),
            body: vec![],
        },
        HirStmt::While {
            test: benign(),
            body: vec![HirStmt::ExprStmt(self_call())],
        },
        HirStmt::ForRange {
            var: "i".to_string(),
            start: self_call(),
            stop: benign(),
            step: benign(),
            body: vec![],
        },
        HirStmt::ForRange {
            var: "i".to_string(),
            start: benign(),
            stop: benign(),
            step: benign(),
            body: vec![HirStmt::ExprStmt(self_call())],
        },
        HirStmt::ForList {
            var: "i".to_string(),
            list: "xs".to_string(),
            body: vec![HirStmt::ExprStmt(self_call())],
        },
        HirStmt::DictSet {
            dict: "d".to_string(),
            key: self_call(),
            value: benign(),
        },
        HirStmt::DictSet {
            dict: "d".to_string(),
            key: benign(),
            value: self_call(),
        },
        HirStmt::ListCompAssign {
            target: "ys".to_string(),
            var: "v".to_string(),
            iter: CompIter::Range {
                start: self_call(),
                stop: benign(),
                step: benign(),
            },
            cond: None,
            elt: Box::new(benign()),
        },
        HirStmt::ListCompAssign {
            target: "ys".to_string(),
            var: "v".to_string(),
            iter: CompIter::Name("xs".to_string()),
            cond: Some(Box::new(self_call())),
            elt: Box::new(benign()),
        },
        HirStmt::SetCompAssign {
            target: "ys".to_string(),
            var: "v".to_string(),
            iter: CompIter::Name("xs".to_string()),
            cond: None,
            elt: Box::new(self_call()),
        },
        HirStmt::DictCompAssign {
            target: "ys".to_string(),
            var: "v".to_string(),
            iter: CompIter::Range {
                start: benign(),
                stop: benign(),
                step: benign(),
            },
            cond: Some(Box::new(benign())),
            key: Box::new(self_call()),
            value: Box::new(benign()),
        },
        HirStmt::DictCompAssign {
            target: "ys".to_string(),
            var: "v".to_string(),
            iter: CompIter::Name("xs".to_string()),
            cond: None,
            key: Box::new(benign()),
            value: Box::new(self_call()),
        },
    ];
    for case in cases {
        assert_self_call_found(case);
    }
}

#[test]
fn the_generic_recursion_gate_finds_a_self_call_in_every_expression_position() {
    // Same exhaustiveness requirement one level down, for
    // `reject_generic_calls_in_expr`. Every recursive position gets a
    // case whose call is exactly there, so no arm can regress into
    // silently skipping a sub-expression.
    let cases = vec![
        HirExpr::Call {
            callee: "other".to_string(),
            args: vec![self_call()],
        },
        HirExpr::BinOp {
            op: BinOpKind::Add,
            left: Box::new(self_call()),
            right: Box::new(benign()),
        },
        HirExpr::BinOp {
            op: BinOpKind::Add,
            left: Box::new(benign()),
            right: Box::new(self_call()),
        },
        HirExpr::Compare {
            op: CmpOpKind::Eq,
            left: Box::new(benign()),
            right: Box::new(self_call()),
        },
        HirExpr::FString(vec![
            FStringPart::Literal("x".to_string()),
            FStringPart::Interpolation(Box::new(self_call())),
        ]),
        HirExpr::ListLiteral(vec![self_call()]),
        HirExpr::SetLiteral(vec![self_call()]),
        HirExpr::TupleLiteral(vec![self_call()]),
        HirExpr::ObjectList(vec![self_call()]),
        HirExpr::DictLiteral(vec![(self_call(), benign())]),
        HirExpr::DictLiteral(vec![(benign(), self_call())]),
        HirExpr::Subscript {
            base: Box::new(self_call()),
            index: Box::new(benign()),
        },
        HirExpr::Subscript {
            base: Box::new(benign()),
            index: Box::new(self_call()),
        },
        HirExpr::Slice {
            base: Box::new(self_call()),
            start: None,
            stop: None,
            step: None,
        },
        HirExpr::Slice {
            base: Box::new(benign()),
            start: Some(Box::new(benign())),
            stop: Some(Box::new(benign())),
            step: Some(Box::new(self_call())),
        },
        HirExpr::ListAppend {
            list: pycc_hir::ContainerReceiver::Name("xs".to_string()),
            value: Box::new(self_call()),
        },
        HirExpr::SetAdd {
            set: "s".to_string(),
            value: Box::new(self_call()),
        },
        HirExpr::DictGetOrDefault {
            dict: pycc_hir::ContainerReceiver::Name("d".to_string()),
            key: Box::new(self_call()),
            default: Box::new(benign()),
        },
        HirExpr::DictGetOrDefault {
            dict: pycc_hir::ContainerReceiver::Name("d".to_string()),
            key: Box::new(benign()),
            default: Box::new(self_call()),
        },
    ];
    for case in cases {
        assert_self_call_found(HirStmt::ExprStmt(case));
    }
}

#[test]
fn the_generic_recursion_gate_accepts_a_body_with_no_generic_call_anywhere() {
    // The success tail of every arm above: the same shapes, none of
    // which contains a self-call or a call to a registered generic.
    let leaves = vec![
        HirExpr::IntLiteral(1),
        HirExpr::FloatLiteral(1.0),
        HirExpr::BoolLiteral(true),
        HirExpr::StringLiteral("s".to_string()),
        HirExpr::Name("x".to_string()),
        HirExpr::ListPop {
            list: pycc_hir::ContainerReceiver::Name("xs".to_string()),
        },
        HirExpr::Call {
            callee: "other".to_string(),
            args: vec![benign()],
        },
        HirExpr::BinOp {
            op: BinOpKind::Add,
            left: Box::new(benign()),
            right: Box::new(benign()),
        },
        HirExpr::Compare {
            op: CmpOpKind::Eq,
            left: Box::new(benign()),
            right: Box::new(benign()),
        },
        HirExpr::FString(vec![
            FStringPart::Literal("x".to_string()),
            FStringPart::Interpolation(Box::new(benign())),
        ]),
        HirExpr::ListLiteral(vec![benign()]),
        HirExpr::SetLiteral(vec![benign()]),
        HirExpr::TupleLiteral(vec![benign()]),
        HirExpr::DictLiteral(vec![(benign(), benign())]),
        HirExpr::Subscript {
            base: Box::new(benign()),
            index: Box::new(benign()),
        },
        HirExpr::Slice {
            base: Box::new(benign()),
            start: Some(Box::new(benign())),
            stop: None,
            step: None,
        },
        HirExpr::ListAppend {
            list: pycc_hir::ContainerReceiver::Name("xs".to_string()),
            value: Box::new(benign()),
        },
        HirExpr::SetAdd {
            set: "s".to_string(),
            value: Box::new(benign()),
        },
        HirExpr::DictGetOrDefault {
            dict: pycc_hir::ContainerReceiver::Name("d".to_string()),
            key: Box::new(benign()),
            default: Box::new(benign()),
        },
    ];
    let mut body: Vec<HirStmt> = leaves.into_iter().map(HirStmt::ExprStmt).collect();
    body.push(HirStmt::AnnAssign {
        is_final: false,
        target: "t".to_string(),
        annotation: Ty::Int,
        value: None,
    });
    body.push(HirStmt::Return(None));
    assert!(reject_generic_calls_in_block(&Environment::new(), "f", &body).is_ok());
}

#[test]
fn check_rejects_a_self_recursive_generic_function() {
    // The validation-only entry point must reject this, not just
    // `check_and_resolve`: the whole defect was `check` (i.e. `pycc
    // check`) accepting a program `pycc build` then panicked on.
    let err = check(&self_recursive_generic_module()).unwrap_err();
    assert_eq!(err.code, "T0042");
    assert!(
        err.message.contains("calls itself"),
        "unexpected message: {}",
        err.message
    );
}

#[test]
fn check_and_resolve_rejects_a_self_recursive_generic_function() {
    // The `build` half of the same gap: `check_and_resolve` is what
    // produces the HIR `pycc_mir` consumes, so the old panic path
    // ("`$fn:rec` has no recorded type") is provably unreachable only
    // if this errors before `monomorphize` runs.
    let err = check_and_resolve(&self_recursive_generic_module()).unwrap_err();
    assert_eq!(err.code, "T0042");
    assert!(err.message.contains("calls itself"));
}

#[test]
fn check_rejects_a_generic_function_calling_another_generic_function() {
    // Indirect/mutual recursion between two generic functions is
    // rejected by the same gate: `f` cannot be proven non-recursive
    // without a whole-module call-graph analysis D-134's thin slice
    // does not have.
    let param = Ty::Param(Box::new("T".to_string()));
    let g = HirItem::Function {
        name: "g".to_string(),
        params: vec![("y".to_string(), param.clone())],
        return_ty: param.clone(),
        body: vec![HirStmt::Return(Some(HirExpr::Name("y".to_string())))],
    };
    let f = HirItem::Function {
        name: "f".to_string(),
        params: vec![("x".to_string(), param.clone())],
        return_ty: param,
        body: vec![HirStmt::Return(Some(HirExpr::Call {
            callee: "g".to_string(),
            args: vec![HirExpr::Name("x".to_string())],
        }))],
    };
    // `g` is declared *after* `f`, proving the gate does not depend on
    // source order (pass 1 binds every signature before any body runs).
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![f, g],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0042");
    assert!(
        err.message.contains("calls generic function `g`"),
        "unexpected message: {}",
        err.message
    );
}

#[test]
fn a_generic_function_body_can_call_a_non_generic_sibling() {
    // I3: this used to report a factually false `T0021` "call to
    // undefined function `helper`" because the generic body was
    // checked without the module's function environment.
    let param = Ty::Param(Box::new("T".to_string()));
    let helper = HirItem::Function {
        name: "helper".to_string(),
        params: vec![("n".to_string(), Ty::Int)],
        return_ty: Ty::Int,
        body: vec![HirStmt::Return(Some(HirExpr::BinOp {
            op: BinOpKind::Add,
            left: Box::new(HirExpr::Name("n".to_string())),
            right: Box::new(HirExpr::IntLiteral(1)),
        }))],
    };
    let g = HirItem::Function {
        name: "g".to_string(),
        params: vec![("x".to_string(), param.clone())],
        return_ty: param,
        body: vec![
            HirStmt::ExprStmt(HirExpr::Call {
                callee: "print".to_string(),
                args: vec![HirExpr::Call {
                    callee: "helper".to_string(),
                    args: vec![HirExpr::IntLiteral(1)],
                }],
            }),
            HirStmt::Return(Some(HirExpr::Name("x".to_string()))),
        ],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            helper,
            g,
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
                callee: "print".to_string(),
                args: vec![HirExpr::Call {
                    callee: "g".to_string(),
                    args: vec![HirExpr::IntLiteral(7)],
                }],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert!(check(&hir).is_ok());
    let resolved = check_and_resolve(&hir).unwrap();
    assert_eq!(count_function(&resolved, "0gen_g__T_int"), 1);
}

#[test]
fn check_and_resolve_returns_empty_type_aliases_on_both_paths() {
    // I1: a resolved module's `type_aliases` value must not depend on
    // whether the module happened to contain a generic function.
    // D-135 aliases are fully discharged during HIR lowering, so the
    // resolved HIR's own field is empty by design on both paths.
    let param = Ty::Param(Box::new("T".to_string()));
    let aliases = vec![("MyInt".to_string(), Ty::Int)];
    let non_generic = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "f".to_string(),
            params: vec![("x".to_string(), Ty::Int)],
            return_ty: Ty::Int,
            body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
        }],
        type_aliases: aliases.clone(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let generic = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            generic_identity_fn(param.clone(), param),
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
                callee: "identity".to_string(),
                args: vec![HirExpr::IntLiteral(1)],
            })),
        ],
        type_aliases: aliases,
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert!(
        check_and_resolve(&non_generic)
            .unwrap()
            .type_aliases
            .is_empty(),
        "no-generics path must discharge `type_aliases`"
    );
    assert!(
        check_and_resolve(&generic).unwrap().type_aliases.is_empty(),
        "monomorphization path must discharge `type_aliases`"
    );
}
