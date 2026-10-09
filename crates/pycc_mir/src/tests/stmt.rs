//! Statement lowering (`stmt::lower_stmt`).
//!
//! Covers `if`, `while`, `for ... in range(...)`, `return`, and annotated
//! assignment, including the bool-to-int widening an annotation forces and
//! the value-less annotation that binds nothing.

use crate::*;
use pycc_hir::{CmpOpKind, HirExpr, HirItem, HirModule, HirStmt, Ty};

#[test]
fn builds_an_if_statement_lowering_both_branches() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::TopLevelStmt(HirStmt::If {
            test: HirExpr::BoolLiteral(true),
            body: vec![HirStmt::ExprStmt(HirExpr::Call {
                callee: "print".to_string(),
                args: vec![HirExpr::IntLiteral(1)],
            })],
            orelse: vec![HirStmt::ExprStmt(HirExpr::Call {
                callee: "print".to_string(),
                args: vec![HirExpr::IntLiteral(2)],
            })],
        })],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let mir = build(&hir);
    assert_eq!(
        mir.items,
        vec![MirItem::TopLevelStmt(MirStmt::If {
            test: MirExpr::BoolLiteral(true),
            body: vec![MirStmt::ExprStmt(MirExpr::Call {
                callee: "print".to_string(),
                args: vec![MirExpr::IntLiteral(1)],
                ty: Ty::None,
            })],
            orelse: vec![MirStmt::ExprStmt(MirExpr::Call {
                callee: "print".to_string(),
                args: vec![MirExpr::IntLiteral(2)],
                ty: Ty::None,
            })],
        })]
    );
}

#[test]
fn builds_a_while_loop() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::TopLevelStmt(HirStmt::While {
            test: HirExpr::BoolLiteral(true),
            body: vec![HirStmt::ExprStmt(HirExpr::Call {
                callee: "print".to_string(),
                args: vec![HirExpr::IntLiteral(1)],
            })],
        })],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let mir = build(&hir);
    assert_eq!(
        mir.items,
        vec![MirItem::TopLevelStmt(MirStmt::While {
            test: MirExpr::BoolLiteral(true),
            body: vec![MirStmt::ExprStmt(MirExpr::Call {
                callee: "print".to_string(),
                args: vec![MirExpr::IntLiteral(1)],
                ty: Ty::None,
            })],
        })]
    );
}

#[test]
fn builds_a_for_range_loop_binding_its_variable_as_int() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::TopLevelStmt(HirStmt::ForRange {
            var: "i".to_string(),
            start: HirExpr::IntLiteral(0),
            stop: HirExpr::IntLiteral(3),
            step: HirExpr::IntLiteral(1),
            body: vec![HirStmt::ExprStmt(HirExpr::Call {
                callee: "print".to_string(),
                args: vec![HirExpr::Name("i".to_string())],
            })],
        })],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let mir = build(&hir);
    assert_eq!(
        mir.items,
        vec![MirItem::TopLevelStmt(MirStmt::ForRange {
            var: "i".to_string(),
            start: MirExpr::IntLiteral(0),
            stop: MirExpr::IntLiteral(3),
            step: MirExpr::IntLiteral(1),
            body: vec![MirStmt::ExprStmt(MirExpr::Call {
                callee: "print".to_string(),
                args: vec![MirExpr::Name {
                    name: "i".to_string(),
                    ty: Ty::Int
                }],
                ty: Ty::None,
            })],
        })]
    );
}

#[test]
fn builds_a_return_with_no_value() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "f".to_string(),
            params: vec![],
            return_ty: Ty::None,
            body: vec![HirStmt::Return(None)],
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
            params: vec![],
            return_ty: Ty::None,
            body: vec![MirStmt::Return(None)],
        }]
    );
}

#[test]
fn an_annotated_assignment_whose_value_type_already_matches_the_annotation_lowers_unchanged() {
    // `x: int = 1` -- the initializer's own inferred type (`Ty::Int`)
    // already matches the annotation, so this is `lower_stmt`'s
    // "no widening needed" branch and `value` passes through
    // unchanged. This case cannot by itself distinguish binding the
    // annotation's type from binding the value's type (they're equal
    // here) -- the sibling test below, where they differ, is what
    // actually proves `lower_stmt` binds the annotation.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::AnnAssign {
                is_final: false,
                target: "x".to_string(),
                annotation: Ty::Int,
                value: Some(HirExpr::IntLiteral(1)),
            }),
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Name("x".to_string()))),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let mir = build(&hir);
    assert_eq!(
        mir.items,
        vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "x".to_string(),
                value: MirExpr::IntLiteral(1),
            }),
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Name {
                name: "x".to_string(),
                ty: Ty::Int,
            })),
        ]
    );
}

#[test]
fn an_annotated_assignment_with_a_bool_value_under_an_int_annotation_widens_and_binds_int() {
    // `x: int = True` -- `pycc_types::is_assignable` accepts a `bool`
    // initializer under an `int` annotation as its one widening case,
    // and `pycc_types` itself binds its checker `env` to `Ty::Int`
    // (the annotation), not `Ty::Bool` (the initializer's own type) --
    // see its own comment citing this exact invariant. `lower_stmt`
    // must agree (D-074's "first assignment fixes a binding's
    // representation" rule): it wraps the lowered `BoolLiteral` in an
    // `IntBoundary` reporting `Ty::Int`, preserving D-141 runtime
    // identity without manufacturing arithmetic, and binds `x` to
    // `Ty::Int`, so a later
    // `Name` reference -- and any later plain reassignment -- agrees.
    // Before this fix, `lower_stmt` bound `Ty::Bool` here instead, and
    // the divergence from `pycc_types`' `Ty::Int` silently mis-sized
    // `x`'s eventual codegen slot (confirmed end to end:
    // `x: int = True; x = 5; return x` printed `11`, not `5`).
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::AnnAssign {
                is_final: false,
                target: "x".to_string(),
                annotation: Ty::Int,
                value: Some(HirExpr::BoolLiteral(true)),
            }),
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Name("x".to_string()))),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let mir = build(&hir);
    assert_eq!(
        mir.items,
        vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "x".to_string(),
                value: MirExpr::IntBoundary(Box::new(MirExpr::BoolLiteral(true))),
            }),
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Name {
                name: "x".to_string(),
                ty: Ty::Int,
            })),
        ]
    );
}

#[test]
fn an_annotated_assignment_with_a_bool_typed_compare_value_also_widens() {
    // The widening branch above is reachable for *any* `Ty::Bool`-typed
    // initializer under an `int` annotation, not merely a literal
    // `True`/`False` -- `pycc_types::is_assignable(Bool, Int)` accepts
    // a `Compare` result, a bool-typed name, or a bool-returning call
    // identically. This proves the same `IntBoundary` wrapping triggers
    // for a `Compare`-sourced `bool`, not only the literal
    // case the previous test exercises.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::TopLevelStmt(HirStmt::AnnAssign {
            is_final: false,
            target: "x".to_string(),
            annotation: Ty::Int,
            value: Some(HirExpr::Compare {
                op: CmpOpKind::Lt,
                left: Box::new(HirExpr::IntLiteral(1)),
                right: Box::new(HirExpr::IntLiteral(2)),
            }),
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
            value: MirExpr::IntBoundary(Box::new(MirExpr::Compare {
                op: CmpOpKind::Lt,
                left: Box::new(MirExpr::IntLiteral(1)),
                right: Box::new(MirExpr::IntLiteral(2)),
                ty: Ty::Bool,
            })),
        })]
    );
}

#[test]
fn an_annotated_assignment_with_a_bare_int_value_under_an_optional_int_annotation_wraps() {
    // `x: int | None = 5` (D-197, #763, Part 1 of #747): mirrors the
    // `IntBoundary`-widening tests directly above exactly, for the
    // `Ty::Optional(inner)` branch instead -- `lower_stmt` wraps the
    // lowered initializer in `MirExpr::OptionalWrap` so its own `.ty()`
    // reports `Ty::Optional(Int)`, and binds `x` to that same
    // `Ty::Optional(Int)` (the annotation), not the initializer's own
    // bare `Ty::Int`, matching D-074's "first assignment fixes a
    // binding's representation" rule.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::AnnAssign {
                is_final: false,
                target: "x".to_string(),
                annotation: Ty::Optional(Box::new(Ty::Int)),
                value: Some(HirExpr::IntLiteral(5)),
            }),
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Name("x".to_string()))),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let mir = build(&hir);
    assert_eq!(
        mir.items,
        vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "x".to_string(),
                value: MirExpr::OptionalWrap(Box::new(MirExpr::IntLiteral(5)), Box::new(Ty::Int)),
            }),
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Name {
                name: "x".to_string(),
                ty: Ty::Optional(Box::new(Ty::Int)),
            })),
        ]
    );
}

#[test]
fn an_annotated_assignment_with_a_bare_none_value_under_an_optional_int_annotation_also_wraps() {
    // `x: int | None = None` (D-197, #763, Part 1 of #747): the other
    // real initializer shape `OptionalWrap` must handle -- a bare
    // `HirExpr::NoneLiteral` (statically `Ty::None`) rather than a bare
    // `inner`-typed value, both accepted by `pycc_types::is_assignable`'s
    // own `Ty::Optional` widening arm.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::TopLevelStmt(HirStmt::AnnAssign {
            is_final: false,
            target: "x".to_string(),
            annotation: Ty::Optional(Box::new(Ty::Int)),
            value: Some(HirExpr::NoneLiteral),
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
            value: MirExpr::OptionalWrap(Box::new(MirExpr::NoneLiteral), Box::new(Ty::Int)),
        })]
    );
}

#[test]
fn a_value_less_annotated_assignment_lowers_to_a_no_op_and_binds_nothing() {
    // `y: int` alone has no runtime action -- CPython itself does
    // nothing observable for it either. `lower_stmt` must produce
    // `MirStmt::NoOp` and must NOT bind `y` in scope (matching
    // `pycc_types`' own Task 4 choice not to bind a value-less
    // declaration): a later read of `y` with no intervening assignment
    // still panics via `lookup`, proving no phantom binding leaked
    // through.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::TopLevelStmt(HirStmt::AnnAssign {
            is_final: false,
            target: "y".to_string(),
            annotation: Ty::Int,
            value: None,
        })],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let mir = build(&hir);
    assert_eq!(mir.items, vec![MirItem::TopLevelStmt(MirStmt::NoOp)]);
}

#[test]
#[should_panic(expected = "has no recorded type")]
fn a_value_less_annotated_assignment_does_not_bind_the_name() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::AnnAssign {
                is_final: false,
                target: "y".to_string(),
                annotation: Ty::Int,
                value: None,
            }),
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Name("y".to_string()))),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    build(&hir);
}

#[test]
fn a_value_less_optional_annotation_binds_the_declared_type() {
    // `x: int | None` alone (#763/#770 review): unlike a plain-typed
    // value-less `AnnAssign` (asserted immediately above to bind
    // nothing), an `Optional[inner]` declaration must bind its target to
    // the declared `Ty::Optional(inner)` even with no initializer, so a
    // later plain reassignment's own `HirStmt::Assign` arm can see it.
    // This alone should still lower to `MirStmt::NoOp` -- only the scope
    // entry, not any codegen action, changes here.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::TopLevelStmt(HirStmt::AnnAssign {
            is_final: false,
            target: "x".to_string(),
            annotation: Ty::Optional(Box::new(Ty::Int)),
            value: None,
        })],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let mir = build(&hir);
    assert_eq!(mir.items, vec![MirItem::TopLevelStmt(MirStmt::NoOp)]);
}

#[test]
fn a_plain_reassignment_of_a_value_less_optional_declaration_rewraps_into_optional() {
    // The exact #770 review repro: `x: int | None` (no initializer)
    // followed by a plain `x = 5` must still lower `x`'s slot type as
    // `Ty::Optional(Int)`, not the bare `Ty::Int` `5` would otherwise
    // report. Before this fix, `collect_stmt_bindings` (`pycc_codegen`)
    // derived `x`'s storage-slot type from this very `MirStmt::Assign`'s
    // own `value.ty()` -- had it stayed bare `Ty::Int`, a later `x is
    // None` panicked in codegen because its non-`Optional` operand could
    // never satisfy the `is`/`is not` lowering's own `Optional` shape
    // assumption.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::AnnAssign {
                is_final: false,
                target: "x".to_string(),
                annotation: Ty::Optional(Box::new(Ty::Int)),
                value: None,
            }),
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::IntLiteral(5),
            }),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let mir = build(&hir);
    assert_eq!(
        mir.items,
        vec![
            MirItem::TopLevelStmt(MirStmt::NoOp),
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "x".to_string(),
                value: MirExpr::OptionalWrap(Box::new(MirExpr::IntLiteral(5)), Box::new(Ty::Int)),
            }),
        ]
    );
}

#[test]
fn a_plain_reassignment_with_a_bare_none_value_after_a_value_less_optional_declaration_rewraps() {
    // `x: int | None` then `x = None`: the plain-`Assign` rewrap path
    // must also fire for a bare `None` value, not only a bare `inner`
    // value -- `None`'s own lowered `.ty()` is not `Ty::Optional(Int)`
    // either, so it needs the same `OptionalWrap` treatment as `5` above.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::AnnAssign {
                is_final: false,
                target: "x".to_string(),
                annotation: Ty::Optional(Box::new(Ty::Int)),
                value: None,
            }),
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::NoneLiteral,
            }),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let mir = build(&hir);
    assert_eq!(
        mir.items,
        vec![
            MirItem::TopLevelStmt(MirStmt::NoOp),
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "x".to_string(),
                value: MirExpr::OptionalWrap(Box::new(MirExpr::NoneLiteral), Box::new(Ty::Int)),
            }),
        ]
    );
}

// -- #627: `obj.attr = <bool>` into an `int`-declared slot ------------------

/// Builds a module whose `Holder` class declares `n` with `slot_ty` and
/// whose top level assigns `value` into `holder.n`, returning the lowered
/// `MirStmt::AttrSet`'s own `value` expression.
///
/// The class is spelled out here rather than reusing `class_attr.rs`'s
/// `point_module` because these cases turn on the *declared slot type*,
/// which that fixture fixes at `Ty::Int`.
fn lowered_attr_set_value(slot_ty: Ty, value: HirExpr) -> MirExpr {
    use pycc_hir::HirClassDef;
    let self_ty = Ty::Instance(Box::new("Holder".to_string()));
    let init = HirItem::Function {
        name: "Holder.__init__".to_string(),
        params: vec![
            ("self".to_string(), self_ty),
            ("n".to_string(), slot_ty.clone()),
        ],
        return_ty: Ty::None,
        body: vec![
            HirStmt::AttrSet {
                base: HirExpr::Name("self".to_string()),
                attr: "n".to_string(),
                value: HirExpr::Name("n".to_string()),
            },
            HirStmt::Return(None),
        ],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            init,
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "holder".to_string(),
                value: HirExpr::Call {
                    callee: "Holder".to_string(),
                    args: vec![HirExpr::IntLiteral(0)],
                },
            }),
            HirItem::TopLevelStmt(HirStmt::AttrSet {
                base: HirExpr::Name("holder".to_string()),
                attr: "n".to_string(),
                value,
            }),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: vec![(
            "Holder".to_string(),
            HirClassDef {
                class_attrs: Vec::new(),
                exception_type_tag: None,
                name: "Holder".to_string(),
                bases: Vec::new(),
                mro: vec!["Holder".to_string()],
                attrs: vec![("n".to_string(), slot_ty)],
                methods: vec![("__init__".to_string(), "Holder.__init__".to_string())],
                type_param: None,
                properties: Vec::new(),
                static_methods: Vec::new(),
                class_methods: Vec::new(),
                is_enum: false,
                implicit_object_init: false,
                method_defaults: Vec::new(),
                enum_members: Vec::new(),
                is_dataclass: false,
                dataclass_fields: Vec::new(),
                is_protocol: false,
                runtime_checkable: false,
                protocol_members: Vec::new(),
                abstract_methods: Vec::new(),
                is_abstract: false,
            },
        )],
    };
    let mir = build(&hir);
    mir.items
        .iter()
        .find_map(|item| match item {
            MirItem::TopLevelStmt(MirStmt::AttrSet { value, .. }) => Some(value.clone()),
            _ => None,
        })
        .expect("the top-level attribute store must lower to a `MirStmt::AttrSet`")
}

#[test]
fn an_attr_set_of_a_bool_into_an_int_declared_slot_widens_through_int_boundary() {
    // #627: `c.n = True` where `n` is declared `int`. `pycc_types`
    // accepts this (`docs/TYPE_SYSTEM.md`: `int` admits `bool` at a
    // checked boundary), so the store is legal -- but the slot holds
    // D-141-encoded `int` words, and an unencoded `bool` word lands
    // there as a raw `1`/`0`. `0` is not a valid encoded int word at
    // all, so the next read of the slot aborted the program
    // (`pycc_rt: invalid encoded int word 0x0`). Wrap the value in
    // `MirExpr::IntBoundary` exactly as the `AnnAssign` arm above
    // does, which is the mechanism D-141 mandates for this widening,
    // and which additionally makes `value.ty()` report `Ty::Int` so
    // codegen's slot-release gate fires (D-180 Consequences item 6).
    assert_eq!(
        lowered_attr_set_value(Ty::Int, HirExpr::BoolLiteral(true)),
        MirExpr::IntBoundary(Box::new(MirExpr::BoolLiteral(true))),
    );
}

#[test]
fn an_attr_set_of_a_non_bool_into_an_int_declared_slot_is_left_alone() {
    // The second operand of the `&&`: an `int`-declared slot taking an
    // already-`int` value must not gain a redundant boundary.
    assert_eq!(
        lowered_attr_set_value(Ty::Int, HirExpr::IntLiteral(7)),
        MirExpr::IntLiteral(7),
    );
}

#[test]
fn an_attr_set_of_a_bool_into_a_bool_declared_slot_is_left_alone() {
    // The first operand of the `&&`: a `bool`-declared slot stores its
    // `bool` word verbatim. Wrapping here would be the regression the
    // issue's own literal completion criterion would have introduced --
    // `slot_word_to_scalar` truncates a `Ty::Bool` slot to `i8`, so the
    // encoded `False` marker `0b0010` would read back as truthy and
    // `print` would render it `True`.
    assert_eq!(
        lowered_attr_set_value(Ty::Bool, HirExpr::BoolLiteral(false)),
        MirExpr::BoolLiteral(false),
    );
}
