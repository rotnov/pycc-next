//! Builtin call lowering.
//!
//! Covers `math.sqrt`, `math.pi`, `len`, `float`, and `cast`, plus the check
//! that a user-defined function shadowing a builtin name lowers as a real call.

use crate::*;
use pycc_hir::{BinOpKind, HirExpr, HirItem, HirModule, HirStmt, Ty};

#[test]
fn lowers_math_sqrt_call_to_mir_with_float_type_without_panicking() {
    // D-136: without the dedicated `callee == "math.sqrt"` branch, this
    // would panic via `lookup`'s own "has no recorded type" message,
    // exactly like `len` above -- there is no `$fn:math.sqrt` signature
    // to find, even though `pycc_types` already accepts `math.sqrt(x)`
    // as valid, `Ty::Float`-typed.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::TopLevelStmt(HirStmt::Assign {
            target: "n".to_string(),
            value: HirExpr::Call {
                callee: "math.sqrt".to_string(),
                args: vec![HirExpr::FloatLiteral(2.0)],
            },
        })],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let mir = build(&hir);
    assert_eq!(
        mir.items[0],
        MirItem::TopLevelStmt(MirStmt::Assign {
            target: "n".to_string(),
            value: MirExpr::Call {
                callee: "math.sqrt".to_string(),
                args: vec![MirExpr::FloatLiteral(2.0)],
                ty: Ty::Float,
            },
        })
    );
}

#[test]
fn lowers_math_pi_name_to_mir_with_float_type_without_panicking() {
    // D-136: without the dedicated `name == "math.pi"` arm, this would
    // panic via `lookup`'s own "has no recorded type" message -- `pi`
    // is never bound in `scopes` the way an ordinary assigned variable
    // is.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::TopLevelStmt(HirStmt::Assign {
            target: "n".to_string(),
            value: HirExpr::Name("math.pi".to_string()),
        })],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let mir = build(&hir);
    assert_eq!(
        mir.items[0],
        MirItem::TopLevelStmt(MirStmt::Assign {
            target: "n".to_string(),
            value: MirExpr::Name {
                name: "math.pi".to_string(),
                ty: Ty::Float,
            },
        })
    );
}

#[test]
fn lowers_len_call_to_mir_with_int_type_without_panicking() {
    // Required fix (beyond the brief): without a parallel `"len"` branch
    // in the `HirExpr::Call` lowering arm, this would panic via
    // `lookup`'s own "has no recorded type" message, since no `$fn:len`
    // signature is ever registered -- even though `pycc_types` already
    // accepts `len(lst)` as valid, `Ty::Int`-typed (D-105 point 3).
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::ListLiteral(vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)]),
            }),
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "n".to_string(),
                value: HirExpr::Call {
                    callee: "len".to_string(),
                    args: vec![HirExpr::Name("x".to_string())],
                },
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
            target: "n".to_string(),
            value: MirExpr::Call {
                callee: "len".to_string(),
                args: vec![MirExpr::Name {
                    name: "x".to_string(),
                    ty: Ty::List(Box::new(Ty::Int)),
                }],
                ty: Ty::Int,
            },
        })
    );
}

#[test]
fn lowers_float_call_to_mir_with_float_type_without_panicking() {
    // Mirrors `lowers_len_call_to_mir_with_int_type_without_panicking`
    // immediately above, for the same reason (#181): without a parallel
    // `"float"` branch in the `HirExpr::Call` lowering arm, this would
    // panic via `lookup`'s own "has no recorded type" message, since no
    // `$fn:float` signature is ever registered -- even though
    // `pycc_types` already accepts `float(x)` as valid, `Ty::Float`-typed.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::IntLiteral(3),
            }),
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "y".to_string(),
                value: HirExpr::Call {
                    callee: "float".to_string(),
                    args: vec![HirExpr::Name("x".to_string())],
                },
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
            value: MirExpr::Call {
                callee: "float".to_string(),
                args: vec![MirExpr::Name {
                    name: "x".to_string(),
                    ty: Ty::Int,
                }],
                ty: Ty::Float,
            },
        })
    );
}

#[test]
fn a_user_defined_float_function_is_lowered_as_a_real_call_not_the_builtin() {
    // Post-merge review finding: unlike `len`/`print`, `float` was
    // undefined until #181, so a program defining its own `float` was
    // valid on `main` immediately before this builtin landed --
    // reproduced directly, printing `6` on a pristine checkout. Without
    // this priority check, the builtin's hardcoded `Ty::Float` would
    // silently override the user function's own registered `Ty::Int`
    // return type.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "float".to_string(),
                params: vec![("x".to_string(), Ty::Int)],
                return_ty: Ty::Int,
                body: vec![HirStmt::Return(Some(HirExpr::BinOp {
                    op: BinOpKind::Add,
                    left: Box::new(HirExpr::Name("x".to_string())),
                    right: Box::new(HirExpr::IntLiteral(1)),
                }))],
            },
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "y".to_string(),
                value: HirExpr::Call {
                    callee: "float".to_string(),
                    args: vec![HirExpr::IntLiteral(5)],
                },
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
            value: MirExpr::Call {
                callee: "float".to_string(),
                args: vec![MirExpr::IntLiteral(5)],
                ty: Ty::Int,
            },
        })
    );
}

#[test]
fn lowers_cast_to_its_value_argument_eliding_the_call() {
    // #767: `typing.cast(T, value)` is a runtime no-op, so the whole call
    // expression lowers to `value` alone -- no `MirExpr::Call` for `cast`
    // ever reaches codegen, and the target type argument is never lowered
    // (it is a type name, not a value expression, and would panic in
    // `lookup` if it were).
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::TopLevelStmt(HirStmt::Assign {
            target: "n".to_string(),
            value: HirExpr::Call {
                callee: "cast".to_string(),
                args: vec![HirExpr::Name("int".to_string()), HirExpr::IntLiteral(5)],
            },
        })],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let mir = build(&hir);
    assert_eq!(
        mir.items[0],
        MirItem::TopLevelStmt(MirStmt::Assign {
            target: "n".to_string(),
            value: MirExpr::IntLiteral(5),
        })
    );
}

#[test]
fn a_user_defined_cast_function_is_lowered_as_a_real_call_not_elided() {
    // #767: a program defining its own `def cast(...)` keeps the real call
    // -- the same user-definition-takes-priority rule `float`,
    // `isinstance`, and `issubclass` follow. `pycc_types`' identical guard
    // means this MIR-side check is defense-in-depth, but without it the
    // user's function would be silently replaced by its second argument.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "cast".to_string(),
                params: vec![("a".to_string(), Ty::Int), ("b".to_string(), Ty::Int)],
                return_ty: Ty::Int,
                body: vec![HirStmt::Return(Some(HirExpr::BinOp {
                    op: BinOpKind::Add,
                    left: Box::new(HirExpr::Name("a".to_string())),
                    right: Box::new(HirExpr::Name("b".to_string())),
                }))],
            },
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "y".to_string(),
                value: HirExpr::Call {
                    callee: "cast".to_string(),
                    args: vec![HirExpr::IntLiteral(2), HirExpr::IntLiteral(3)],
                },
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
            value: MirExpr::Call {
                callee: "cast".to_string(),
                args: vec![MirExpr::IntLiteral(2), MirExpr::IntLiteral(3)],
                ty: Ty::Int,
            },
        })
    );
}
