//! Generic-class type-substitution and instantiation-collection unit tests
//! for the type-checking crate root.
//!
//! Extracted verbatim from `tests.rs` under AGENTS.md's decomposability rule
//! (part of #695, which tracks decomposing that oversized file). These are the
//! direct unit tests for `substitute_ty_with_class`, `substitute_stmt_with_class`,
//! and the `collect_generic_class_instantiations_from_expr` /
//! `..._from_stmt` walkers. Their `gci_expr` fixture helper stays in the
//! parent, because `tests/generic_monomorphization_arms.rs` uses it too and
//! sibling child modules cannot see each other's private items. As a child
//! module this still sees the parent's private items directly through
//! `use super::*`, so nothing needed widened visibility; only the tests'
//! location changed.

use super::*;

// -- substitute_ty_with_class unit tests -------------------------------

#[test]
fn substitute_ty_with_class_substitutes_param() {
    let ty = Ty::Param(Box::new("T".to_string()));
    let result = substitute_ty_with_class(&ty, "T", &Ty::Int, "C", "0gen_C__T_int");
    assert_eq!(result, Ty::Int);
}

#[test]
fn substitute_ty_with_class_rewrites_instance() {
    let ty = Ty::Instance(Box::new("C".to_string()));
    let result = substitute_ty_with_class(&ty, "T", &Ty::Int, "C", "0gen_C__T_int");
    assert_eq!(result, Ty::Instance(Box::new("0gen_C__T_int".to_string())));
}

#[test]
fn substitute_ty_with_class_clones_other_types() {
    // A type that is neither Param("T") nor Instance("C") falls through
    // to the `other => other.clone()` arm.
    let ty = Ty::Int;
    let result = substitute_ty_with_class(&ty, "T", &Ty::Str, "C", "0gen_C__T_int");
    assert_eq!(result, Ty::Int);
}

#[test]
fn substitute_ty_with_class_does_not_substitute_a_different_param_name() {
    // `Ty::Param("U")` with param_name="T" falls through to `other`.
    let ty = Ty::Param(Box::new("U".to_string()));
    let result = substitute_ty_with_class(&ty, "T", &Ty::Int, "C", "0gen_C__T_int");
    assert_eq!(result, Ty::Param(Box::new("U".to_string())));
}

#[test]
fn substitute_ty_with_class_does_not_rewrite_a_different_class_instance() {
    // `Ty::Instance("Other")` with class_name="C" falls through to `other`.
    let ty = Ty::Instance(Box::new("Other".to_string()));
    let result = substitute_ty_with_class(&ty, "T", &Ty::Int, "C", "0gen_C__T_int");
    assert_eq!(result, Ty::Instance(Box::new("Other".to_string())));
}

// -- substitute_stmt_with_class unit tests -----------------------------

#[test]
fn substitute_stmt_with_class_substitutes_ann_assign_annotation() {
    let stmt = HirStmt::AnnAssign {
        is_final: false,
        target: "y".to_string(),
        annotation: Ty::Param(Box::new("T".to_string())),
        value: None,
    };
    let result = substitute_stmt_with_class(&stmt, "T", &Ty::Int, "C", "0gen_C__T_int");
    assert_eq!(
        result,
        HirStmt::AnnAssign {
            is_final: false,
            target: "y".to_string(),
            annotation: Ty::Int,
            value: None,
        }
    );
}

#[test]
fn substitute_stmt_with_class_recurses_into_if_body_and_orelse() {
    let stmt = HirStmt::If {
        test: HirExpr::BoolLiteral(true),
        body: vec![HirStmt::AnnAssign {
            is_final: false,
            target: "a".to_string(),
            annotation: Ty::Param(Box::new("T".to_string())),
            value: None,
        }],
        orelse: vec![HirStmt::AnnAssign {
            is_final: false,
            target: "b".to_string(),
            annotation: Ty::Param(Box::new("T".to_string())),
            value: None,
        }],
    };
    let result = substitute_stmt_with_class(&stmt, "T", &Ty::Int, "C", "0gen_C__T_int");
    assert_eq!(
        result,
        HirStmt::If {
            test: HirExpr::BoolLiteral(true),
            body: vec![HirStmt::AnnAssign {
                is_final: false,
                target: "a".to_string(),
                annotation: Ty::Int,
                value: None,
            }],
            orelse: vec![HirStmt::AnnAssign {
                is_final: false,
                target: "b".to_string(),
                annotation: Ty::Int,
                value: None,
            }],
        }
    );
}

#[test]
fn substitute_stmt_with_class_recurses_into_while_body() {
    let stmt = HirStmt::While {
        test: HirExpr::BoolLiteral(true),
        body: vec![HirStmt::AnnAssign {
            is_final: false,
            target: "w".to_string(),
            annotation: Ty::Param(Box::new("T".to_string())),
            value: None,
        }],
    };
    let result = substitute_stmt_with_class(&stmt, "T", &Ty::Int, "C", "0gen_C__T_int");
    assert_eq!(
        result,
        HirStmt::While {
            test: HirExpr::BoolLiteral(true),
            body: vec![HirStmt::AnnAssign {
                is_final: false,
                target: "w".to_string(),
                annotation: Ty::Int,
                value: None,
            }],
        }
    );
}

#[test]
fn substitute_stmt_with_class_recurses_into_for_range_body() {
    let stmt = HirStmt::ForRange {
        var: "i".to_string(),
        start: HirExpr::IntLiteral(0),
        stop: HirExpr::IntLiteral(1),
        step: HirExpr::IntLiteral(1),
        body: vec![HirStmt::AnnAssign {
            is_final: false,
            target: "r".to_string(),
            annotation: Ty::Param(Box::new("T".to_string())),
            value: None,
        }],
    };
    let result = substitute_stmt_with_class(&stmt, "T", &Ty::Int, "C", "0gen_C__T_int");
    assert_eq!(
        result,
        HirStmt::ForRange {
            var: "i".to_string(),
            start: HirExpr::IntLiteral(0),
            stop: HirExpr::IntLiteral(1),
            step: HirExpr::IntLiteral(1),
            body: vec![HirStmt::AnnAssign {
                is_final: false,
                target: "r".to_string(),
                annotation: Ty::Int,
                value: None,
            }],
        }
    );
}

#[test]
fn substitute_stmt_with_class_recurses_into_for_list_body() {
    let stmt = HirStmt::ForList {
        var: "e".to_string(),
        list: "xs".to_string(),
        body: vec![HirStmt::AnnAssign {
            is_final: false,
            target: "l".to_string(),
            annotation: Ty::Param(Box::new("T".to_string())),
            value: None,
        }],
    };
    let result = substitute_stmt_with_class(&stmt, "T", &Ty::Int, "C", "0gen_C__T_int");
    assert_eq!(
        result,
        HirStmt::ForList {
            var: "e".to_string(),
            list: "xs".to_string(),
            body: vec![HirStmt::AnnAssign {
                is_final: false,
                target: "l".to_string(),
                annotation: Ty::Int,
                value: None,
            }],
        }
    );
}

#[test]
fn substitute_stmt_with_class_clones_other_statement_variants() {
    // A statement that is not AnnAssign/If/While/ForRange/ForList falls
    // through to the `other => other.clone()` arm.
    let stmt = HirStmt::ExprStmt(HirExpr::IntLiteral(42));
    let result = substitute_stmt_with_class(&stmt, "T", &Ty::Int, "C", "0gen_C__T_int");
    assert_eq!(result, stmt);
}

// -- collect_generic_class_instantiations_from_expr unit tests ---------

#[test]
fn collect_generic_class_instantiations_finds_a_bare_generic_class_instantiate() {
    let mut out = Vec::new();
    collect_generic_class_instantiations_from_expr(&gci_expr(), &mut out);
    assert_eq!(out, vec![("C".to_string(), Ty::Int)]);
}

#[test]
fn collect_generic_class_instantiations_dedupes_identical_pairs() {
    let mut out = Vec::new();
    // Two identical GCI expressions in a Call's args.
    let expr = HirExpr::Call {
        callee: "print".to_string(),
        args: vec![gci_expr(), gci_expr()],
    };
    collect_generic_class_instantiations_from_expr(&expr, &mut out);
    assert_eq!(out, vec![("C".to_string(), Ty::Int)]);
}

#[test]
fn collect_generic_class_instantiations_from_expr_covers_every_arm() {
    // A single expression tree that nests a `GenericClassInstantiate`
    // inside every expression variant, exercising every arm of
    // `collect_generic_class_instantiations_from_expr`.
    let gci = gci_expr();
    let mut out = Vec::new();

    // BinOp
    collect_generic_class_instantiations_from_expr(
        &HirExpr::BinOp {
            op: BinOpKind::Add,
            left: Box::new(gci.clone()),
            right: Box::new(HirExpr::IntLiteral(1)),
        },
        &mut out,
    );
    // Compare
    collect_generic_class_instantiations_from_expr(
        &HirExpr::Compare {
            op: CmpOpKind::Eq,
            left: Box::new(gci.clone()),
            right: Box::new(HirExpr::IntLiteral(1)),
        },
        &mut out,
    );
    // FString
    collect_generic_class_instantiations_from_expr(
        &HirExpr::FString(vec![FStringPart::Interpolation(Box::new(gci.clone()))]),
        &mut out,
    );
    // ListLiteral
    collect_generic_class_instantiations_from_expr(
        &HirExpr::ListLiteral(vec![gci.clone()]),
        &mut out,
    );
    // SetLiteral
    collect_generic_class_instantiations_from_expr(
        &HirExpr::SetLiteral(vec![gci.clone()]),
        &mut out,
    );
    // TupleLiteral
    collect_generic_class_instantiations_from_expr(
        &HirExpr::TupleLiteral(vec![gci.clone()]),
        &mut out,
    );
    // ObjectList (Part 2d of #1371)
    collect_generic_class_instantiations_from_expr(
        &HirExpr::ObjectList(vec![gci.clone()]),
        &mut out,
    );
    // DictLiteral
    collect_generic_class_instantiations_from_expr(
        &HirExpr::DictLiteral(vec![(gci.clone(), gci.clone())]),
        &mut out,
    );
    // Subscript
    collect_generic_class_instantiations_from_expr(
        &HirExpr::Subscript {
            base: Box::new(gci.clone()),
            index: Box::new(gci.clone()),
        },
        &mut out,
    );
    // Slice
    collect_generic_class_instantiations_from_expr(
        &HirExpr::Slice {
            base: Box::new(gci.clone()),
            start: Some(Box::new(gci.clone())),
            stop: Some(Box::new(gci.clone())),
            step: Some(Box::new(gci.clone())),
        },
        &mut out,
    );
    // ListAppend
    collect_generic_class_instantiations_from_expr(
        &HirExpr::ListAppend {
            list: pycc_hir::ContainerReceiver::Name("xs".to_string()),
            value: Box::new(gci.clone()),
        },
        &mut out,
    );
    // SetAdd
    collect_generic_class_instantiations_from_expr(
        &HirExpr::SetAdd {
            set: "s".to_string(),
            value: Box::new(gci.clone()),
        },
        &mut out,
    );
    // DictGetOrDefault
    collect_generic_class_instantiations_from_expr(
        &HirExpr::DictGetOrDefault {
            dict: pycc_hir::ContainerReceiver::Name("d".to_string()),
            key: Box::new(gci.clone()),
            default: Box::new(gci.clone()),
        },
        &mut out,
    );
    // AttrGet
    collect_generic_class_instantiations_from_expr(
        &HirExpr::AttrGet {
            base: Box::new(gci.clone()),
            attr: "x".to_string(),
        },
        &mut out,
    );
    // MethodCall
    collect_generic_class_instantiations_from_expr(
        &HirExpr::MethodCall {
            base: Box::new(gci.clone()),
            method: "m".to_string(),
            args: vec![gci.clone()],
        },
        &mut out,
    );
    // Leaf arms — should produce no entries.
    collect_generic_class_instantiations_from_expr(&HirExpr::IntLiteral(1), &mut out);
    collect_generic_class_instantiations_from_expr(&HirExpr::FloatLiteral(1.0), &mut out);
    collect_generic_class_instantiations_from_expr(&HirExpr::BoolLiteral(true), &mut out);
    collect_generic_class_instantiations_from_expr(
        &HirExpr::StringLiteral("s".to_string()),
        &mut out,
    );
    collect_generic_class_instantiations_from_expr(&HirExpr::Name("x".to_string()), &mut out);
    collect_generic_class_instantiations_from_expr(
        &HirExpr::ListPop {
            list: pycc_hir::ContainerReceiver::Name("xs".to_string()),
        },
        &mut out,
    );

    // All the GCI expressions above reference the same (C, Int) pair,
    // so the deduped output should have exactly one entry.
    assert_eq!(out, vec![("C".to_string(), Ty::Int)]);
}

// -- collect_generic_class_instantiations_from_stmt unit tests ---------

#[test]
fn collect_generic_class_instantiations_from_stmt_covers_every_arm() {
    let gci = gci_expr();
    let mut out = Vec::new();

    // ExprStmt
    collect_generic_class_instantiations_from_stmt(&HirStmt::ExprStmt(gci.clone()), &mut out);
    // Assign
    collect_generic_class_instantiations_from_stmt(
        &HirStmt::Assign {
            target: "x".to_string(),
            value: gci.clone(),
        },
        &mut out,
    );
    // AnnAssign with value
    collect_generic_class_instantiations_from_stmt(
        &HirStmt::AnnAssign {
            is_final: false,
            target: "y".to_string(),
            annotation: Ty::Int,
            value: Some(gci.clone()),
        },
        &mut out,
    );
    // AnnAssign without value (None arm)
    collect_generic_class_instantiations_from_stmt(
        &HirStmt::AnnAssign {
            is_final: false,
            target: "z".to_string(),
            annotation: Ty::Int,
            value: None,
        },
        &mut out,
    );
    // If
    collect_generic_class_instantiations_from_stmt(
        &HirStmt::If {
            test: gci.clone(),
            body: vec![HirStmt::ExprStmt(gci.clone())],
            orelse: vec![HirStmt::ExprStmt(gci.clone())],
        },
        &mut out,
    );
    // While
    collect_generic_class_instantiations_from_stmt(
        &HirStmt::While {
            test: gci.clone(),
            body: vec![HirStmt::ExprStmt(gci.clone())],
        },
        &mut out,
    );
    // ForRange
    collect_generic_class_instantiations_from_stmt(
        &HirStmt::ForRange {
            var: "i".to_string(),
            start: gci.clone(),
            stop: gci.clone(),
            step: gci.clone(),
            body: vec![HirStmt::ExprStmt(gci.clone())],
        },
        &mut out,
    );
    // ForList
    collect_generic_class_instantiations_from_stmt(
        &HirStmt::ForList {
            var: "e".to_string(),
            list: "xs".to_string(),
            body: vec![HirStmt::ExprStmt(gci.clone())],
        },
        &mut out,
    );
    // DictSet
    collect_generic_class_instantiations_from_stmt(
        &HirStmt::DictSet {
            dict: "d".to_string(),
            key: gci.clone(),
            value: gci.clone(),
        },
        &mut out,
    );
    // ListCompAssign
    collect_generic_class_instantiations_from_stmt(
        &HirStmt::ListCompAssign {
            target: "xs".to_string(),
            var: "_v0".to_string(),
            iter: CompIter::Range {
                start: HirExpr::IntLiteral(0),
                stop: HirExpr::IntLiteral(1),
                step: HirExpr::IntLiteral(1),
            },
            cond: Some(Box::new(gci.clone())),
            elt: Box::new(gci.clone()),
        },
        &mut out,
    );
    // SetCompAssign
    collect_generic_class_instantiations_from_stmt(
        &HirStmt::SetCompAssign {
            target: "s".to_string(),
            var: "_v1".to_string(),
            iter: CompIter::Range {
                start: HirExpr::IntLiteral(0),
                stop: HirExpr::IntLiteral(1),
                step: HirExpr::IntLiteral(1),
            },
            cond: None,
            elt: Box::new(gci.clone()),
        },
        &mut out,
    );
    // DictCompAssign
    collect_generic_class_instantiations_from_stmt(
        &HirStmt::DictCompAssign {
            target: "d".to_string(),
            var: "_v2".to_string(),
            iter: CompIter::Range {
                start: HirExpr::IntLiteral(0),
                stop: HirExpr::IntLiteral(1),
                step: HirExpr::IntLiteral(1),
            },
            cond: None,
            key: Box::new(gci.clone()),
            value: Box::new(gci.clone()),
        },
        &mut out,
    );
    // Return(None)
    collect_generic_class_instantiations_from_stmt(&HirStmt::Return(None), &mut out);
    // Return(Some)
    collect_generic_class_instantiations_from_stmt(&HirStmt::Return(Some(gci.clone())), &mut out);
    // AttrSet
    collect_generic_class_instantiations_from_stmt(
        &HirStmt::AttrSet {
            base: gci.clone(),
            attr: "x".to_string(),
            value: gci.clone(),
        },
        &mut out,
    );

    // All GCI expressions reference the same (C, Int) pair.
    assert_eq!(out, vec![("C".to_string(), Ty::Int)]);
}
