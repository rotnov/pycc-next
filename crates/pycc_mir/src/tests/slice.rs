//! `HirExpr::Slice` to `MirExpr::Slice` lowering.
//!
//! Covers every present/absent combination of the start, stop, and step
//! bounds, recursive lowering of the base and bounds, and the slice type.

use crate::*;
use pycc_hir::{HirExpr, HirItem, HirModule, HirStmt, Ty};

// -- Task 8 (D-118): `HirExpr::Slice` -> `MirExpr::Slice` lowering ----

/// Builds `xs = [1, 2, 3]` followed by `y = <slice>` for some
/// `HirExpr::Slice` reading `xs`, mirroring the fixture every Task 6
/// (`pycc_hir`) slicing test starts from, so this lowering is exercised
/// against the same shapes those frontend tests already pin.
fn xs_list_then_slice(slice: HirExpr) -> HirModule {
    HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "xs".to_string(),
                value: HirExpr::ListLiteral(vec![
                    HirExpr::IntLiteral(1),
                    HirExpr::IntLiteral(2),
                    HirExpr::IntLiteral(3),
                ]),
            }),
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "y".to_string(),
                value: slice,
            }),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    }
}

#[test]
fn a_slice_expression_with_both_bounds_present_lowers_with_both_bounds_some() {
    // `xs[1:3]` (mirrors `pycc_hir`'s
    // `lowers_a_slice_expression_with_both_bounds_present`).
    let hir = xs_list_then_slice(HirExpr::Slice {
        base: Box::new(HirExpr::Name("xs".to_string())),
        start: Some(Box::new(HirExpr::IntLiteral(1))),
        stop: Some(Box::new(HirExpr::IntLiteral(3))),
        step: None,
    });
    let mir = build(&hir);
    assert_eq!(
        mir.items[1],
        MirItem::TopLevelStmt(MirStmt::Assign {
            target: "y".to_string(),
            value: MirExpr::Slice {
                base: Box::new(MirExpr::Name {
                    name: "xs".to_string(),
                    ty: Ty::List(Box::new(Ty::Int)),
                }),
                start: Some(Box::new(MirExpr::IntLiteral(1))),
                stop: Some(Box::new(MirExpr::IntLiteral(3))),
                step: None,
            },
        })
    );
}

#[test]
fn a_slice_expression_with_only_the_stop_bound_lowers_with_start_and_step_none() {
    // `xs[:3]` (mirrors `pycc_hir`'s
    // `lowers_a_slice_expression_with_only_the_stop_bound`).
    let hir = xs_list_then_slice(HirExpr::Slice {
        base: Box::new(HirExpr::Name("xs".to_string())),
        start: None,
        stop: Some(Box::new(HirExpr::IntLiteral(3))),
        step: None,
    });
    let mir = build(&hir);
    assert_eq!(
        mir.items[1],
        MirItem::TopLevelStmt(MirStmt::Assign {
            target: "y".to_string(),
            value: MirExpr::Slice {
                base: Box::new(MirExpr::Name {
                    name: "xs".to_string(),
                    ty: Ty::List(Box::new(Ty::Int)),
                }),
                start: None,
                stop: Some(Box::new(MirExpr::IntLiteral(3))),
                step: None,
            },
        })
    );
}

#[test]
fn a_slice_expression_with_only_the_start_bound_lowers_with_stop_and_step_none() {
    // `xs[2:]` (mirrors `pycc_hir`'s
    // `lowers_a_slice_expression_with_only_the_start_bound`).
    let hir = xs_list_then_slice(HirExpr::Slice {
        base: Box::new(HirExpr::Name("xs".to_string())),
        start: Some(Box::new(HirExpr::IntLiteral(2))),
        stop: None,
        step: None,
    });
    let mir = build(&hir);
    assert_eq!(
        mir.items[1],
        MirItem::TopLevelStmt(MirStmt::Assign {
            target: "y".to_string(),
            value: MirExpr::Slice {
                base: Box::new(MirExpr::Name {
                    name: "xs".to_string(),
                    ty: Ty::List(Box::new(Ty::Int)),
                }),
                start: Some(Box::new(MirExpr::IntLiteral(2))),
                stop: None,
                step: None,
            },
        })
    );
}

#[test]
fn a_slice_expression_with_all_bounds_omitted_lowers_with_every_bound_none() {
    // `xs[:]` (mirrors `pycc_hir`'s
    // `lowers_a_slice_expression_with_all_bounds_omitted`).
    let hir = xs_list_then_slice(HirExpr::Slice {
        base: Box::new(HirExpr::Name("xs".to_string())),
        start: None,
        stop: None,
        step: None,
    });
    let mir = build(&hir);
    assert_eq!(
        mir.items[1],
        MirItem::TopLevelStmt(MirStmt::Assign {
            target: "y".to_string(),
            value: MirExpr::Slice {
                base: Box::new(MirExpr::Name {
                    name: "xs".to_string(),
                    ty: Ty::List(Box::new(Ty::Int)),
                }),
                start: None,
                stop: None,
                step: None,
            },
        })
    );
}

#[test]
fn a_slice_expression_with_only_a_step_lowers_with_start_and_stop_none() {
    // `xs[::2]` (mirrors `pycc_hir`'s
    // `lowers_a_slice_expression_with_a_step`).
    let hir = xs_list_then_slice(HirExpr::Slice {
        base: Box::new(HirExpr::Name("xs".to_string())),
        start: None,
        stop: None,
        step: Some(Box::new(HirExpr::IntLiteral(2))),
    });
    let mir = build(&hir);
    assert_eq!(
        mir.items[1],
        MirItem::TopLevelStmt(MirStmt::Assign {
            target: "y".to_string(),
            value: MirExpr::Slice {
                base: Box::new(MirExpr::Name {
                    name: "xs".to_string(),
                    ty: Ty::List(Box::new(Ty::Int)),
                }),
                start: None,
                stop: None,
                step: Some(Box::new(MirExpr::IntLiteral(2))),
            },
        })
    );
}

#[test]
fn a_slice_expressions_base_and_every_present_bound_are_recursively_lowered() {
    // `f()`/`g()`/`h()` stand in for "some already-lowerable non-literal
    // shape" -- confirms `base`/`start`/`stop`/`step` are each passed
    // through the real `lower_expr` recursively (mirroring
    // `pycc_hir`'s own `a_slice_expressions_base_and_bounds_are_recursively_lowered`),
    // not merely accepted as raw literals or the base's bare `Name`.
    // Registers `f`/`g`/`h` as zero-arg functions returning `int` so
    // `lower_expr`'s `HirExpr::Call` arm resolves their `ty` via the
    // real `$fn:` lookup instead of panicking.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "f".to_string(),
                params: vec![],
                return_ty: Ty::Int,
                body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(0)))],
            },
            HirItem::Function {
                name: "g".to_string(),
                params: vec![],
                return_ty: Ty::Int,
                body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(0)))],
            },
            HirItem::Function {
                name: "h".to_string(),
                params: vec![],
                return_ty: Ty::Int,
                body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(0)))],
            },
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "xs".to_string(),
                value: HirExpr::ListLiteral(vec![
                    HirExpr::IntLiteral(1),
                    HirExpr::IntLiteral(2),
                    HirExpr::IntLiteral(3),
                ]),
            }),
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "y".to_string(),
                value: HirExpr::Slice {
                    base: Box::new(HirExpr::Name("xs".to_string())),
                    start: Some(Box::new(HirExpr::Call {
                        callee: "f".to_string(),
                        args: vec![],
                    })),
                    stop: Some(Box::new(HirExpr::Call {
                        callee: "g".to_string(),
                        args: vec![],
                    })),
                    step: Some(Box::new(HirExpr::Call {
                        callee: "h".to_string(),
                        args: vec![],
                    })),
                },
            }),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let mir = build(&hir);
    assert_eq!(
        mir.items[4],
        MirItem::TopLevelStmt(MirStmt::Assign {
            target: "y".to_string(),
            value: MirExpr::Slice {
                base: Box::new(MirExpr::Name {
                    name: "xs".to_string(),
                    ty: Ty::List(Box::new(Ty::Int)),
                }),
                start: Some(Box::new(MirExpr::Call {
                    callee: "f".to_string(),
                    args: vec![],
                    ty: Ty::Int,
                })),
                stop: Some(Box::new(MirExpr::Call {
                    callee: "g".to_string(),
                    args: vec![],
                    ty: Ty::Int,
                })),
                step: Some(Box::new(MirExpr::Call {
                    callee: "h".to_string(),
                    args: vec![],
                    ty: Ty::Int,
                })),
            },
        })
    );
}

#[test]
fn a_slices_ty_derives_from_the_actual_base_type_not_hardcoded_list_of_int() {
    // Mirrors this file's own genericity test for `Subscript`/`ForList`
    // (`list_literal_subscript_and_for_list_derive_their_type_from_actual_elements_not_hardcoded_int`):
    // `MirExpr::Slice`'s `ty()` must derive its result from the actual
    // `base.ty()`, not assume `Ty::List(Box::new(Ty::Int))`.
    // `pycc_types`' T0034 gate means only `list[int]` ever reaches this
    // crate from a real compiled program (a `list[str]` slice is
    // rejected before `pycc_mir` ever sees it), but this crate's own
    // `ty()` must not bake in that assumption independently of the type
    // it actually observes -- so this test bypasses that gate with a
    // hand-built `MirExpr::Slice` over a `list[str]` base, exactly like
    // the `Subscript`/`DictGet` panic tests above bypass gates that
    // can't be reached from a real, type-checked program.
    let slice = MirExpr::Slice {
        base: Box::new(MirExpr::ListLiteral(vec![MirExpr::StringLiteral(
            "a".to_string(),
        )])),
        start: Some(Box::new(MirExpr::IntLiteral(0))),
        stop: None,
        step: None,
    };
    assert_eq!(slice.ty(), Ty::List(Box::new(Ty::Str)));

    // Presence/absence of bounds must not affect `ty()` either --
    // Task 8's brief requirement (c). Compare the all-bounds-omitted
    // shape against the same base.
    let slice_no_bounds = MirExpr::Slice {
        base: Box::new(MirExpr::ListLiteral(vec![MirExpr::StringLiteral(
            "a".to_string(),
        )])),
        start: None,
        stop: None,
        step: None,
    };
    assert_eq!(slice.ty(), slice_no_bounds.ty());
}
