//! Expression lowering (`expr::lower_expr`) for operators.
//!
//! Covers comparison, f-string interpolation, the `binop_result_ty` result
//! table (string concatenation and repetition, true division, int/float
//! promotion), and the unary-operator rewrites.

use crate::*;
use pycc_hir::{
    BinOpKind, CmpOpKind, FStringPart, HirExpr, HirItem, HirModule, HirStmt, Ty, UnaryOpKind,
};

#[test]
fn lowers_a_bare_none_literal_unchanged() {
    // `HirExpr::NoneLiteral -> MirExpr::NoneLiteral` (D-197, #763, Part 1
    // of #747): a bare `Assign` (not `AnnAssign`), so `stmt::lower_stmt`'s
    // own `OptionalWrap`-introducing branch below is not involved here.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::TopLevelStmt(HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::NoneLiteral,
        })],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let mir = build(&hir);
    assert_eq!(
        mir.items,
        vec![MirItem::TopLevelStmt(MirStmt::Assign {
            target: "x".to_string(),
            value: MirExpr::NoneLiteral,
        })]
    );
}

#[test]
fn lowers_not_implemented_unchanged() {
    // `HirExpr::NotImplemented -> MirExpr::NotImplemented` (#1418): the HIR
    // admits it only as a comparison method's `return NotImplemented` in an
    // `--ext` module, but the lowering arm itself is context-free.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::TopLevelStmt(HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::NotImplemented,
        })],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let mir = build(&hir);
    assert_eq!(
        mir.items,
        vec![MirItem::TopLevelStmt(MirStmt::Assign {
            target: "x".to_string(),
            value: MirExpr::NotImplemented,
        })]
    );
}

#[test]
fn builds_a_compare_expression_with_bool_type() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::TopLevelStmt(HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::Compare {
                op: CmpOpKind::Lt,
                left: Box::new(HirExpr::IntLiteral(1)),
                right: Box::new(HirExpr::IntLiteral(2)),
            },
        })],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let mir = build(&hir);
    assert_eq!(
        mir.items,
        vec![MirItem::TopLevelStmt(MirStmt::Assign {
            target: "x".to_string(),
            value: MirExpr::Compare {
                op: CmpOpKind::Lt,
                left: Box::new(MirExpr::IntLiteral(1)),
                right: Box::new(MirExpr::IntLiteral(2)),
                ty: Ty::Bool,
            },
        })]
    );
}

#[test]
fn builds_an_f_string_with_a_literal_and_an_interpolation() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::IntLiteral(1),
            }),
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "y".to_string(),
                value: HirExpr::FString(vec![
                    FStringPart::Literal("n=".to_string()),
                    FStringPart::Interpolation(Box::new(HirExpr::Name("x".to_string()))),
                ]),
            }),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let mir = build(&hir);
    assert_eq!(
        mir.items[1],
        MirItem::TopLevelStmt(MirStmt::Assign {
            target: "y".to_string(),
            value: MirExpr::FString(vec![
                MirFStringPart::Literal("n=".to_string()),
                MirFStringPart::Interpolation(Box::new(MirExpr::Name {
                    name: "x".to_string(),
                    ty: Ty::Int
                })),
            ]),
        })
    );
}

#[test]
fn string_concatenation_infers_str() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::TopLevelStmt(HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::BinOp {
                op: BinOpKind::Add,
                left: Box::new(HirExpr::StringLiteral("a".to_string())),
                right: Box::new(HirExpr::StringLiteral("b".to_string())),
            },
        })],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let mir = build(&hir);
    assert_eq!(
        mir.items,
        vec![MirItem::TopLevelStmt(MirStmt::Assign {
            target: "x".to_string(),
            value: MirExpr::BinOp {
                op: BinOpKind::Add,
                left: Box::new(MirExpr::StringLiteral("a".to_string())),
                right: Box::new(MirExpr::StringLiteral("b".to_string())),
                ty: Ty::Str,
            },
        })]
    );
}

// #574 (Part 1 of #123): `binop_result_ty`'s repetition clause. One
// test per operand order and per accepted count type, because each
// side of the clause's `||` is its own region under the D-014
// coverage gate, as is each arm of the two `matches!` expressions --
// including their `_ => false` fallbacks, which the negative controls
// at the end of this group reach.

#[test]
fn string_repetition_by_an_int_infers_str() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::TopLevelStmt(HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::BinOp {
                op: BinOpKind::Mul,
                left: Box::new(HirExpr::StringLiteral("a".to_string())),
                right: Box::new(HirExpr::IntLiteral(3)),
            },
        })],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let mir = build(&hir);
    assert_eq!(
        mir.items,
        vec![MirItem::TopLevelStmt(MirStmt::Assign {
            target: "x".to_string(),
            value: MirExpr::BinOp {
                op: BinOpKind::Mul,
                left: Box::new(MirExpr::StringLiteral("a".to_string())),
                right: Box::new(MirExpr::IntLiteral(3)),
                ty: Ty::Str,
            },
        })]
    );
}

#[test]
fn string_repetition_with_the_count_on_the_left_infers_str() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::TopLevelStmt(HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::BinOp {
                op: BinOpKind::Mul,
                left: Box::new(HirExpr::IntLiteral(3)),
                right: Box::new(HirExpr::StringLiteral("a".to_string())),
            },
        })],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let mir = build(&hir);
    assert_eq!(
        mir.items,
        vec![MirItem::TopLevelStmt(MirStmt::Assign {
            target: "x".to_string(),
            value: MirExpr::BinOp {
                op: BinOpKind::Mul,
                left: Box::new(MirExpr::IntLiteral(3)),
                right: Box::new(MirExpr::StringLiteral("a".to_string())),
                ty: Ty::Str,
            },
        })]
    );
}

#[test]
fn string_repetition_by_a_bool_infers_str() {
    // `bool <: int`, so a `bool` count is accepted in either order --
    // matching `pycc_types::numeric_result_type` exactly.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::BinOp {
                    op: BinOpKind::Mul,
                    left: Box::new(HirExpr::StringLiteral("a".to_string())),
                    right: Box::new(HirExpr::BoolLiteral(true)),
                },
            }),
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "y".to_string(),
                value: HirExpr::BinOp {
                    op: BinOpKind::Mul,
                    left: Box::new(HirExpr::BoolLiteral(true)),
                    right: Box::new(HirExpr::StringLiteral("a".to_string())),
                },
            }),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let mir = build(&hir);
    // Asserted against the full expected items rather than destructuring
    // each one with a `let ... else { unreachable!() }`: that `else` arm
    // would be an unexercised line and region under the D-014 gate.
    assert_eq!(
        mir.items,
        vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "x".to_string(),
                value: MirExpr::BinOp {
                    op: BinOpKind::Mul,
                    left: Box::new(MirExpr::StringLiteral("a".to_string())),
                    right: Box::new(MirExpr::BoolLiteral(true)),
                    ty: Ty::Str,
                },
            }),
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "y".to_string(),
                value: MirExpr::BinOp {
                    op: BinOpKind::Mul,
                    left: Box::new(MirExpr::BoolLiteral(true)),
                    right: Box::new(MirExpr::StringLiteral("a".to_string())),
                    ty: Ty::Str,
                },
            }),
        ]
    );
}

#[test]
fn multiplying_two_ints_still_infers_int_after_the_repetition_clause() {
    // Negative control for the repetition clause: `Mul` alone must not
    // route a purely numeric pair into `Ty::Str`.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::TopLevelStmt(HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::BinOp {
                op: BinOpKind::Mul,
                left: Box::new(HirExpr::IntLiteral(2)),
                right: Box::new(HirExpr::IntLiteral(3)),
            },
        })],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let mir = build(&hir);
    assert_eq!(
        mir.items,
        vec![MirItem::TopLevelStmt(MirStmt::Assign {
            target: "x".to_string(),
            value: MirExpr::BinOp {
                op: BinOpKind::Mul,
                left: Box::new(MirExpr::IntLiteral(2)),
                right: Box::new(MirExpr::IntLiteral(3)),
                ty: Ty::Int,
            },
        })]
    );
}

#[test]
fn multiplying_a_str_by_a_float_does_not_take_the_repetition_clause() {
    // Defensive shape: `pycc_types` rejects `str * float` with T0021,
    // so this HIR can never arrive through `pycc check` -- it is built
    // directly here (`build` does not type-check) purely to exercise
    // the `_ => false` fallback of the repetition clause's
    // `matches!(right, Ty::Int | Ty::Bool)`, which no reachable
    // program covers. The clause must decline, leaving the ordinary
    // numeric rule to answer `Ty::Float`.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::TopLevelStmt(HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::BinOp {
                op: BinOpKind::Mul,
                left: Box::new(HirExpr::StringLiteral("a".to_string())),
                right: Box::new(HirExpr::FloatLiteral(2.0)),
            },
        })],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let mir = build(&hir);
    assert_eq!(
        mir.items,
        vec![MirItem::TopLevelStmt(MirStmt::Assign {
            target: "x".to_string(),
            value: MirExpr::BinOp {
                op: BinOpKind::Mul,
                left: Box::new(MirExpr::StringLiteral("a".to_string())),
                right: Box::new(MirExpr::FloatLiteral(2.0)),
                ty: Ty::Float,
            },
        })]
    );
}

#[test]
fn multiplying_a_float_by_a_str_does_not_take_the_repetition_clause() {
    // The mirror of the test above, and equally unreachable through
    // `pycc check` (T0021): it exists to exercise the `_ => false`
    // fallback of the repetition clause's
    // `matches!(left, Ty::Int | Ty::Bool)`.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::TopLevelStmt(HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::BinOp {
                op: BinOpKind::Mul,
                left: Box::new(HirExpr::FloatLiteral(2.0)),
                right: Box::new(HirExpr::StringLiteral("a".to_string())),
            },
        })],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let mir = build(&hir);
    assert_eq!(
        mir.items,
        vec![MirItem::TopLevelStmt(MirStmt::Assign {
            target: "x".to_string(),
            value: MirExpr::BinOp {
                op: BinOpKind::Mul,
                left: Box::new(MirExpr::FloatLiteral(2.0)),
                right: Box::new(MirExpr::StringLiteral("a".to_string())),
                ty: Ty::Float,
            },
        })]
    );
}

#[test]
fn true_division_of_two_ints_infers_float() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::TopLevelStmt(HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::BinOp {
                op: BinOpKind::Div,
                left: Box::new(HirExpr::IntLiteral(5)),
                right: Box::new(HirExpr::IntLiteral(2)),
            },
        })],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let mir = build(&hir);
    assert_eq!(
        mir.items,
        vec![MirItem::TopLevelStmt(MirStmt::Assign {
            target: "x".to_string(),
            value: MirExpr::BinOp {
                op: BinOpKind::Div,
                left: Box::new(MirExpr::IntLiteral(5)),
                right: Box::new(MirExpr::IntLiteral(2)),
                ty: Ty::Float,
            },
        })]
    );
}

#[test]
fn adding_a_float_left_operand_and_an_int_infers_float() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::TopLevelStmt(HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::BinOp {
                op: BinOpKind::Add,
                left: Box::new(HirExpr::FloatLiteral(1.5)),
                right: Box::new(HirExpr::IntLiteral(2)),
            },
        })],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let mir = build(&hir);
    assert_eq!(
        mir.items,
        vec![MirItem::TopLevelStmt(MirStmt::Assign {
            target: "x".to_string(),
            value: MirExpr::BinOp {
                op: BinOpKind::Add,
                left: Box::new(MirExpr::FloatLiteral(1.5)),
                right: Box::new(MirExpr::IntLiteral(2)),
                ty: Ty::Float,
            },
        })]
    );
}

#[test]
fn adding_an_int_and_a_float_right_operand_infers_float() {
    // Distinct region from the left-operand case above: exercises
    // `right == Ty::Float` specifically (`left == Ty::Float` is false
    // here), not just `binop_result_ty`'s overall `Float` outcome.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::TopLevelStmt(HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::BinOp {
                op: BinOpKind::Add,
                left: Box::new(HirExpr::IntLiteral(2)),
                right: Box::new(HirExpr::FloatLiteral(1.5)),
            },
        })],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let mir = build(&hir);
    assert_eq!(
        mir.items,
        vec![MirItem::TopLevelStmt(MirStmt::Assign {
            target: "x".to_string(),
            value: MirExpr::BinOp {
                op: BinOpKind::Add,
                left: Box::new(MirExpr::IntLiteral(2)),
                right: Box::new(MirExpr::FloatLiteral(1.5)),
                ty: Ty::Float,
            },
        })]
    );
}

/// Lowers `-p` / `+p` over a single parameter of type `ty` and asserts the
/// whole lowered function equals `expected` in its return position, so each
/// unary case below states only the arithmetic rewrite it expects.
fn assert_unary_over_param_lowers_to(op: UnaryOpKind, ty: Ty, expected: MirExpr) {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "f".to_string(),
            params: vec![("p".to_string(), ty.clone())],
            return_ty: ty.clone(),
            body: vec![HirStmt::Return(Some(HirExpr::UnaryOp {
                op,
                operand: Box::new(HirExpr::Name("p".to_string())),
            }))],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let mir = build(&hir);
    assert_eq!(
        mir.items,
        vec![MirItem::Function {
            name: "f".to_string(),
            params: vec![("p".to_string(), ty.clone())],
            return_ty: ty,
            body: vec![MirStmt::Return(Some(expected))],
        }]
    );
}

#[test]
fn unary_minus_on_an_int_operand_lowers_to_zero_minus_operand() {
    assert_unary_over_param_lowers_to(
        UnaryOpKind::USub,
        Ty::Int,
        MirExpr::BinOp {
            op: BinOpKind::Sub,
            left: Box::new(MirExpr::IntLiteral(0)),
            right: Box::new(MirExpr::Name {
                name: "p".to_string(),
                ty: Ty::Int,
            }),
            ty: Ty::Int,
        },
    );
}

#[test]
fn unary_plus_on_an_int_operand_lowers_to_zero_plus_operand() {
    assert_unary_over_param_lowers_to(
        UnaryOpKind::UAdd,
        Ty::Int,
        MirExpr::BinOp {
            op: BinOpKind::Add,
            left: Box::new(MirExpr::IntLiteral(0)),
            right: Box::new(MirExpr::Name {
                name: "p".to_string(),
                ty: Ty::Int,
            }),
            ty: Ty::Int,
        },
    );
}

#[test]
fn unary_minus_on_a_float_operand_lowers_to_a_multiply_by_minus_one() {
    assert_unary_over_param_lowers_to(
        UnaryOpKind::USub,
        Ty::Float,
        MirExpr::BinOp {
            op: BinOpKind::Mul,
            left: Box::new(MirExpr::Name {
                name: "p".to_string(),
                ty: Ty::Float,
            }),
            right: Box::new(MirExpr::FloatLiteral(-1.0)),
            ty: Ty::Float,
        },
    );
}

#[test]
fn unary_plus_on_a_float_operand_lowers_to_a_multiply_by_one() {
    assert_unary_over_param_lowers_to(
        UnaryOpKind::UAdd,
        Ty::Float,
        MirExpr::BinOp {
            op: BinOpKind::Mul,
            left: Box::new(MirExpr::Name {
                name: "p".to_string(),
                ty: Ty::Float,
            }),
            right: Box::new(MirExpr::FloatLiteral(1.0)),
            ty: Ty::Float,
        },
    );
}

// #604 (Part 3 of #573): `~x` rewrites into the same `0 - x` shape `USub`
// uses, chained twice (`~x == -x - 1`), inheriting `int_sub`'s bigint
// handling. This is a dedicated inline unit test (not just the integration
// coverage in `tests/issue_604_unary_not_invert.rs`) because #603's own
// build-test-coverage failure (see `docs/TESTING.md`'s "Practical notes")
// showed that a crate's plain-vs-`--cfg test` compilations of the same
// function are scored by the *maximum* covered-region count per
// instantiation, not their union -- an arm reached only by an integration
// test through the `pycc` binary does not count toward the unit-test
// binary's own instantiation of `lower_expr`.
#[test]
fn bitwise_invert_on_an_int_operand_lowers_to_double_subtraction() {
    assert_unary_over_param_lowers_to(
        UnaryOpKind::Invert,
        Ty::Int,
        MirExpr::BinOp {
            op: BinOpKind::Sub,
            left: Box::new(MirExpr::BinOp {
                op: BinOpKind::Sub,
                left: Box::new(MirExpr::IntLiteral(0)),
                right: Box::new(MirExpr::Name {
                    name: "p".to_string(),
                    ty: Ty::Int,
                }),
                ty: Ty::Int,
            }),
            right: Box::new(MirExpr::IntLiteral(1)),
            ty: Ty::Int,
        },
    );
}

// #604: `not x` lowers to a dedicated `MirExpr::Not` node rather than a
// `BinOp` rewrite, since truthiness has no binary-expression equivalent.
#[test]
fn logical_not_on_an_int_operand_lowers_to_a_dedicated_not_node() {
    assert_unary_over_param_lowers_to(
        UnaryOpKind::Not,
        Ty::Int,
        MirExpr::Not(Box::new(MirExpr::Name {
            name: "p".to_string(),
            ty: Ty::Int,
        })),
    );
}

// PEP 572 (#774): a walrus inside an `if` test binds the name into `scopes`
// *before* the body is lowered (`pycc_mir::stmt::lower_stmt`'s `If` arm),
// exercising both `expr::lower_expr`'s own `HirExpr::NamedExpr` arm and
// `expr::pre_bind_named_expr_targets`'s `NamedExpr` arm (through the
// `Compare`'s `left` operand).
#[test]
fn walrus_in_an_if_test_binds_the_name_for_the_body() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::TopLevelStmt(HirStmt::If {
            test: HirExpr::Compare {
                op: CmpOpKind::Gt,
                left: Box::new(HirExpr::NamedExpr {
                    name: "n".to_string(),
                    value: Box::new(HirExpr::IntLiteral(7)),
                }),
                right: Box::new(HirExpr::IntLiteral(5)),
            },
            body: vec![HirStmt::Assign {
                target: "y".to_string(),
                value: HirExpr::Name("n".to_string()),
            }],
            orelse: vec![],
        })],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let mir = build(&hir);
    assert_eq!(
        mir.items,
        vec![MirItem::TopLevelStmt(MirStmt::If {
            test: MirExpr::Compare {
                op: CmpOpKind::Gt,
                left: Box::new(MirExpr::NamedExpr {
                    name: "n".to_string(),
                    value: Box::new(MirExpr::IntLiteral(7)),
                    ty: Ty::Int,
                }),
                right: Box::new(MirExpr::IntLiteral(5)),
                ty: Ty::Bool,
            },
            body: vec![MirStmt::Assign {
                target: "y".to_string(),
                value: MirExpr::Name {
                    name: "n".to_string(),
                    ty: Ty::Int,
                },
            }],
            orelse: vec![],
        })]
    );
}

// Mirrors the `if` test above but for `while` (`pycc_mir::stmt::lower_stmt`'s
// own separate `While` arm binds independently of `If`'s).
#[test]
fn walrus_in_a_while_test_binds_the_name_for_the_body() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::TopLevelStmt(HirStmt::While {
            test: HirExpr::Compare {
                op: CmpOpKind::Gt,
                left: Box::new(HirExpr::NamedExpr {
                    name: "v".to_string(),
                    value: Box::new(HirExpr::IntLiteral(3)),
                }),
                right: Box::new(HirExpr::IntLiteral(0)),
            },
            body: vec![HirStmt::Assign {
                target: "y".to_string(),
                value: HirExpr::Name("v".to_string()),
            }],
        })],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let mir = build(&hir);
    assert_eq!(
        mir.items,
        vec![MirItem::TopLevelStmt(MirStmt::While {
            test: MirExpr::Compare {
                op: CmpOpKind::Gt,
                left: Box::new(MirExpr::NamedExpr {
                    name: "v".to_string(),
                    value: Box::new(MirExpr::IntLiteral(3)),
                    ty: Ty::Int,
                }),
                right: Box::new(MirExpr::IntLiteral(0)),
                ty: Ty::Bool,
            },
            body: vec![MirStmt::Assign {
                target: "y".to_string(),
                value: MirExpr::Name {
                    name: "v".to_string(),
                    ty: Ty::Int,
                },
            }],
        })]
    );
}

// A bare expression statement (the third permitted walrus placement) with a
// walrus nested inside a tuple/list literal, a slice bound, and a dict-key
// position -- exercises `expr::pre_bind_named_expr_targets`'s
// `ListLiteral`/`SetLiteral`/`TupleLiteral`, `Slice`, and `DictLiteral` arms
// in one pass.
#[test]
fn walrus_nested_inside_a_tuple_slice_and_dict_literal_binds_every_name() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "lst".to_string(),
                value: HirExpr::ListLiteral(vec![HirExpr::IntLiteral(1)]),
            }),
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::TupleLiteral(vec![
                HirExpr::ListLiteral(vec![HirExpr::NamedExpr {
                    name: "a".to_string(),
                    value: Box::new(HirExpr::IntLiteral(1)),
                }]),
                HirExpr::Slice {
                    base: Box::new(HirExpr::Name("lst".to_string())),
                    start: Some(Box::new(HirExpr::NamedExpr {
                        name: "b".to_string(),
                        value: Box::new(HirExpr::IntLiteral(0)),
                    })),
                    stop: None,
                    step: None,
                },
                HirExpr::DictLiteral(vec![(
                    HirExpr::NamedExpr {
                        name: "c".to_string(),
                        value: Box::new(HirExpr::IntLiteral(2)),
                    },
                    HirExpr::IntLiteral(3),
                )]),
            ]))),
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "sum".to_string(),
                value: HirExpr::BinOp {
                    op: BinOpKind::Add,
                    left: Box::new(HirExpr::Name("a".to_string())),
                    right: Box::new(HirExpr::BinOp {
                        op: BinOpKind::Add,
                        left: Box::new(HirExpr::Name("b".to_string())),
                        right: Box::new(HirExpr::Name("c".to_string())),
                    }),
                },
            }),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    // The point of this test is that `build` does not panic looking up `a`,
    // `b`, or `c` in the trailing `sum` assignment -- proof every nested
    // walrus in the tuple/list/slice/dict literal above was actually bound.
    let mir = build(&hir);
    assert_eq!(
        mir.items[2],
        MirItem::TopLevelStmt(MirStmt::Assign {
            target: "sum".to_string(),
            value: MirExpr::BinOp {
                op: BinOpKind::Add,
                left: Box::new(MirExpr::Name {
                    name: "a".to_string(),
                    ty: Ty::Int,
                }),
                right: Box::new(MirExpr::BinOp {
                    op: BinOpKind::Add,
                    left: Box::new(MirExpr::Name {
                        name: "b".to_string(),
                        ty: Ty::Int,
                    }),
                    right: Box::new(MirExpr::Name {
                        name: "c".to_string(),
                        ty: Ty::Int,
                    }),
                    ty: Ty::Int,
                }),
                ty: Ty::Int,
            },
        })
    );
}

// The test above only ever nests a walrus inside `ListLiteral`
// (`HirExpr::ListLiteral`, as the outer `TupleLiteral`'s first element)
// and `TupleLiteral` itself -- never `SetLiteral`. Even though all three
// share one combined match arm in both `pre_bind_named_expr_targets` and
// `MirExpr::collect_named_expr_bindings`, each `|`-alternative gets its
// own source-coverage region, so the `SetLiteral` alternative specifically
// stays unexercised without a fixture that nests a walrus inside a real
// set literal.
#[test]
fn walrus_nested_inside_a_set_literal_binds_the_name() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::SetLiteral(vec![
                HirExpr::NamedExpr {
                    name: "a".to_string(),
                    value: Box::new(HirExpr::IntLiteral(1)),
                },
            ]))),
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "sum".to_string(),
                value: HirExpr::Name("a".to_string()),
            }),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    // As with the tuple/slice/dict test above, the point is that `build`
    // does not panic looking up `a` in the trailing assignment -- proof
    // the walrus nested inside the set literal was actually bound.
    let mir = build(&hir);
    assert_eq!(
        mir.items[1],
        MirItem::TopLevelStmt(MirStmt::Assign {
            target: "sum".to_string(),
            value: MirExpr::Name {
                name: "a".to_string(),
                ty: Ty::Int,
            },
        })
    );
}

// PEP 572 (#774): `MirExpr::collect_named_expr_bindings` is exhaustive over
// every `MirExpr` variant (see its own doc comment), including several --
// `IntBoundary`, `OptionalWrap`, `Slice` -- that a `NamedExpr` can never
// actually nest inside via the real HIR-to-MIR pipeline (an `IntBoundary`/
// `OptionalWrap` wrapper is only ever introduced by `AnnAssign`/`Assign`
// lowering around a whole initializer, never around a walrus's own `value`
// specifically, and a `Slice` cannot appear inside another `Slice`). These
// arms exist purely so a future new `MirExpr` variant is a compile error
// here rather than a silently-skipped binding, so they are exercised
// directly against the public `collect_named_expr_bindings` method instead
// of through `build`.
#[test]
fn collect_named_expr_bindings_walks_into_int_boundary_and_optional_wrap() {
    let named = MirExpr::NamedExpr {
        name: "z".to_string(),
        value: Box::new(MirExpr::IntLiteral(1)),
        ty: Ty::Int,
    };

    let mut out = Vec::new();
    MirExpr::IntBoundary(Box::new(named.clone())).collect_named_expr_bindings(&mut out);
    assert_eq!(out, vec![("z".to_string(), Ty::Int)]);

    let mut out = Vec::new();
    MirExpr::OptionalWrap(Box::new(named), Box::new(Ty::Int)).collect_named_expr_bindings(&mut out);
    assert_eq!(out, vec![("z".to_string(), Ty::Int)]);
}

// #604 (Part 3 of #573): `collect_named_expr_bindings`'s `Not` arm shares its
// match pattern with `ExceptionMessage` (both are simple single-field
// wrappers), exercised directly for the same reason as the `IntBoundary`/
// `OptionalWrap` pair above -- a walrus cannot actually nest inside a real
// `not` expression's operand via the HIR-to-MIR pipeline used by `build`
// (only a bare `if`/`while` test or expression statement admits one), so
// this arm needs its own direct call against the public method.
#[test]
fn collect_named_expr_bindings_walks_into_a_logical_not_operand() {
    let mir = MirExpr::Not(Box::new(MirExpr::NamedExpr {
        name: "w".to_string(),
        value: Box::new(MirExpr::IntLiteral(0)),
        ty: Ty::Int,
    }));
    let mut out = Vec::new();
    mir.collect_named_expr_bindings(&mut out);
    assert_eq!(out, vec![("w".to_string(), Ty::Int)]);
}

#[test]
fn collect_named_expr_bindings_walks_into_a_slice_bound() {
    let mir = MirExpr::Slice {
        base: Box::new(MirExpr::Name {
            name: "lst".to_string(),
            ty: Ty::List(Box::new(Ty::Int)),
        }),
        start: Some(Box::new(MirExpr::NamedExpr {
            name: "s".to_string(),
            value: Box::new(MirExpr::IntLiteral(0)),
            ty: Ty::Int,
        })),
        stop: None,
        step: None,
    };
    let mut out = Vec::new();
    mir.collect_named_expr_bindings(&mut out);
    assert_eq!(out, vec![("s".to_string(), Ty::Int)]);
}

// PEP 572 (#774): same reasoning as
// `collect_named_expr_bindings_walks_into_int_boundary_and_optional_wrap`
// above -- `BinOp`/`Compare` cannot nest a `NamedExpr` in either operand via
// the real HIR-to-MIR pipeline (a walrus is restricted to an `if`/`while`
// test or a bare expression statement, so a walrus can be the *whole* test
// but not an operand of a further binary/comparison expression the way
// these hand-built fixtures place it), so this arm is exercised directly
// against the public method instead of through `build`.
#[test]
fn collect_named_expr_bindings_walks_into_both_binop_operands() {
    let mir = MirExpr::BinOp {
        op: BinOpKind::Add,
        left: Box::new(MirExpr::NamedExpr {
            name: "l".to_string(),
            value: Box::new(MirExpr::IntLiteral(1)),
            ty: Ty::Int,
        }),
        right: Box::new(MirExpr::NamedExpr {
            name: "r".to_string(),
            value: Box::new(MirExpr::IntLiteral(2)),
            ty: Ty::Int,
        }),
        ty: Ty::Int,
    };
    let mut out = Vec::new();
    mir.collect_named_expr_bindings(&mut out);
    assert_eq!(
        out,
        vec![("l".to_string(), Ty::Int), ("r".to_string(), Ty::Int)]
    );
}

#[test]
fn collect_named_expr_bindings_walks_into_both_compare_operands() {
    let mir = MirExpr::Compare {
        op: CmpOpKind::Lt,
        left: Box::new(MirExpr::NamedExpr {
            name: "l".to_string(),
            value: Box::new(MirExpr::IntLiteral(1)),
            ty: Ty::Int,
        }),
        right: Box::new(MirExpr::NamedExpr {
            name: "r".to_string(),
            value: Box::new(MirExpr::IntLiteral(2)),
            ty: Ty::Int,
        }),
        ty: Ty::Bool,
    };
    let mut out = Vec::new();
    mir.collect_named_expr_bindings(&mut out);
    assert_eq!(
        out,
        vec![("l".to_string(), Ty::Int), ("r".to_string(), Ty::Int)]
    );
}

#[test]
fn collect_named_expr_bindings_walks_into_an_fstring_interpolation() {
    let mir = MirExpr::FString(vec![
        MirFStringPart::Literal("x=".to_string()),
        MirFStringPart::Interpolation(Box::new(MirExpr::NamedExpr {
            name: "n".to_string(),
            value: Box::new(MirExpr::IntLiteral(3)),
            ty: Ty::Int,
        })),
    ]);
    let mut out = Vec::new();
    mir.collect_named_expr_bindings(&mut out);
    assert_eq!(out, vec![("n".to_string(), Ty::Int)]);
}

#[test]
fn collect_named_expr_bindings_walks_into_dict_get_dict_and_key() {
    let mir = MirExpr::DictGet {
        dict: Box::new(MirExpr::NamedExpr {
            name: "d".to_string(),
            value: Box::new(MirExpr::DictLiteral(vec![(
                MirExpr::StringLiteral("k".to_string()),
                MirExpr::IntLiteral(1),
            )])),
            ty: Ty::Dict(Box::new((Ty::Str, Ty::Int))),
        }),
        key: Box::new(MirExpr::NamedExpr {
            name: "k".to_string(),
            value: Box::new(MirExpr::StringLiteral("k".to_string())),
            ty: Ty::Str,
        }),
    };
    let mut out = Vec::new();
    mir.collect_named_expr_bindings(&mut out);
    assert_eq!(
        out,
        vec![
            ("d".to_string(), Ty::Dict(Box::new((Ty::Str, Ty::Int)))),
            ("k".to_string(), Ty::Str),
        ]
    );
}

#[test]
fn collect_named_expr_bindings_walks_into_dict_get_or_default_key_and_default() {
    let mir = MirExpr::DictGetOrDefault {
        dict: crate::MirContainerReceiver::Name("d".to_string()),
        key: Box::new(MirExpr::NamedExpr {
            name: "k".to_string(),
            value: Box::new(MirExpr::StringLiteral("k".to_string())),
            ty: Ty::Str,
        }),
        default: Box::new(MirExpr::NamedExpr {
            name: "def".to_string(),
            value: Box::new(MirExpr::IntLiteral(0)),
            ty: Ty::Int,
        }),
        ty: Ty::Int,
    };
    let mut out = Vec::new();
    mir.collect_named_expr_bindings(&mut out);
    assert_eq!(
        out,
        vec![("k".to_string(), Ty::Str), ("def".to_string(), Ty::Int)]
    );
}

#[test]
fn collect_named_expr_bindings_walks_into_every_instantiate_arg() {
    let mir = MirExpr::Instantiate(Box::new(InstantiateExpr {
        ctor: "C.__init__".to_string(),
        class_name: "C".to_string(),
        slot_names: vec!["s0".to_string()],
        args: vec![MirExpr::NamedExpr {
            name: "a".to_string(),
            value: Box::new(MirExpr::IntLiteral(1)),
            ty: Ty::Int,
        }],
        ty: Ty::Instance(Box::new("C".to_string())),
    }));
    let mut out = Vec::new();
    mir.collect_named_expr_bindings(&mut out);
    assert_eq!(out, vec![("a".to_string(), Ty::Int)]);
}

// PEP 572 (#774): `MirExpr::ty()`'s own `NamedExpr` arm -- a walrus's type
// is its value's type, mirroring `pycc_types::expr::infer_expr_in`'s own
// `NamedExpr` arm.
#[test]
fn a_named_exprs_ty_is_its_values_ty() {
    let mir = MirExpr::NamedExpr {
        name: "z".to_string(),
        value: Box::new(MirExpr::IntLiteral(1)),
        ty: Ty::Int,
    };
    assert_eq!(mir.ty(), Ty::Int);
}

// PEP 572 (#774): `pre_bind_named_expr_targets`'s `HirExpr::BinOp { .. } |
// HirExpr::Compare { .. }` arm is one combined match arm over two
// alternatives, and each `|`-alternative earns its own source-coverage
// region (see `walrus_nested_inside_a_set_literal_binds_the_name`'s own doc
// comment for the same phenomenon on a different arm). Every other walrus
// test above that reaches this arm does so through `Compare` (an `if`/
// `while` test's own top-level shape), never through a bare top-level
// `BinOp` -- a walrus can only appear as the *whole* `if`/`while` test or a
// bare expression statement, so this is the one construction that puts a
// `BinOp` (not a `Compare`) directly at one of those three placements.
#[test]
fn walrus_as_the_left_operand_of_a_top_level_binop_expr_stmt_binds_the_name() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::BinOp {
                op: BinOpKind::Add,
                left: Box::new(HirExpr::NamedExpr {
                    name: "n".to_string(),
                    value: Box::new(HirExpr::IntLiteral(4)),
                }),
                right: Box::new(HirExpr::IntLiteral(1)),
            })),
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "y".to_string(),
                value: HirExpr::Name("n".to_string()),
            }),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    // As with the tuple/slice/dict test above, the point is that `build`
    // does not panic looking up `n` in the trailing assignment -- proof
    // the walrus was bound even though it sits under a `BinOp`, not a
    // `Compare`.
    let mir = build(&hir);
    assert_eq!(
        mir.items[1],
        MirItem::TopLevelStmt(MirStmt::Assign {
            target: "y".to_string(),
            value: MirExpr::Name {
                name: "n".to_string(),
                ty: Ty::Int,
            },
        })
    );
}

// PEP 572 (#774): `pre_bind_named_expr_targets`'s `HirExpr::UnaryOp { .. }`
// arm -- no other walrus test above puts a `UnaryOp` directly at one of the
// three permitted placements (`if`/`while` test, bare expression
// statement), so this arm stays unexercised without a dedicated fixture.
#[test]
fn walrus_under_a_top_level_unary_op_expr_stmt_binds_the_name() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::UnaryOp {
                op: UnaryOpKind::USub,
                operand: Box::new(HirExpr::NamedExpr {
                    name: "n".to_string(),
                    value: Box::new(HirExpr::IntLiteral(4)),
                }),
            })),
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "y".to_string(),
                value: HirExpr::Name("n".to_string()),
            }),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let mir = build(&hir);
    assert_eq!(
        mir.items[1],
        MirItem::TopLevelStmt(MirStmt::Assign {
            target: "y".to_string(),
            value: MirExpr::Name {
                name: "n".to_string(),
                ty: Ty::Int,
            },
        })
    );
}

// PEP 572 (#774): `pre_bind_named_expr_targets`'s `HirExpr::FString(parts)`
// arm -- no other walrus test above puts an f-string directly at one of the
// three permitted placements, so its `Interpolation` branch stays
// unexercised without a dedicated fixture.
#[test]
fn walrus_inside_a_top_level_fstring_interpolation_expr_stmt_binds_the_name() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::FString(vec![
                FStringPart::Literal("n=".to_string()),
                FStringPart::Interpolation(Box::new(HirExpr::NamedExpr {
                    name: "n".to_string(),
                    value: Box::new(HirExpr::IntLiteral(4)),
                })),
            ]))),
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "y".to_string(),
                value: HirExpr::Name("n".to_string()),
            }),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let mir = build(&hir);
    assert_eq!(
        mir.items[1],
        MirItem::TopLevelStmt(MirStmt::Assign {
            target: "y".to_string(),
            value: MirExpr::Name {
                name: "n".to_string(),
                ty: Ty::Int,
            },
        })
    );
}
