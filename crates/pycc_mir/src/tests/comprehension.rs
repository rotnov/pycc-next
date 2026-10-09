//! Comprehension lowering and `resolve_comp_source`.
//!
//! Covers list, set, and dict comprehensions over `range`, list, set, and
//! dict sources, their filters, and the unsupported-source panic.

use crate::*;
use pycc_hir::{HirExpr, HirItem, HirModule, HirStmt, Ty};

// -- PR-12 Task 4 (D-117): comprehension lowering --

#[test]
fn a_range_sourced_list_comprehension_lowers_to_comp_source_range_with_var_ty_int_and_evaluates_its_filter()
 {
    // Exercises `resolve_comp_source`'s `CompIter::Range` branch and the
    // `ListCompAssign` arm's `cond: Some(..)` path (both closures on
    // that arm need at least one executing test for D-014's coverage
    // gate).
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::ListCompAssign {
                target: "y".to_string(),
                var: "i".to_string(),
                iter: CompIter::Range {
                    start: HirExpr::IntLiteral(0),
                    stop: HirExpr::IntLiteral(3),
                    step: HirExpr::IntLiteral(1),
                },
                cond: Some(Box::new(HirExpr::BoolLiteral(true))),
                elt: Box::new(HirExpr::Name("i".to_string())),
            }),
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
                callee: "len".to_string(),
                args: vec![HirExpr::Name("y".to_string())],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let mir = build(&hir);
    assert_eq!(
        mir.items[0],
        MirItem::TopLevelStmt(MirStmt::ListCompAssign {
            target: "y".to_string(),
            var: "i".to_string(),
            var_ty: Ty::Int,
            source: CompSource::Range {
                start: MirExpr::IntLiteral(0),
                stop: MirExpr::IntLiteral(3),
                step: MirExpr::IntLiteral(1),
            },
            cond: Some(Box::new(MirExpr::BoolLiteral(true))),
            elt: Box::new(MirExpr::Name {
                name: "i".to_string(),
                ty: Ty::Int,
            }),
        })
    );
    // `target` is bound as `Ty::List(Ty::Int)`, derived from `elt`'s
    // type -- confirmed via the following statement's own lowered type.
    assert_eq!(
        mir.items[1],
        MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
            callee: "len".to_string(),
            args: vec![MirExpr::Name {
                name: "y".to_string(),
                ty: Ty::List(Box::new(Ty::Int)),
            }],
            ty: Ty::Int,
        }))
    );
}

#[test]
fn a_bare_name_list_sourced_list_comprehension_resolves_comp_source_list_with_the_lists_element_type()
 {
    // Exercises `resolve_comp_source`'s `CompIter::Name` branch resolving
    // to `Ty::List` -- uses `str` elements specifically (mirroring this
    // file's own `list_literal_subscript_and_for_list_derive_their_type_from_actual_elements_not_hardcoded_int`)
    // so `var_ty` is trivially distinguishable from the `Ty::Int` a
    // hardcoded bug would wrongly report.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "xs".to_string(),
                value: HirExpr::ListLiteral(vec![HirExpr::StringLiteral("a".to_string())]),
            }),
            HirItem::TopLevelStmt(HirStmt::ListCompAssign {
                target: "y".to_string(),
                var: "v".to_string(),
                iter: CompIter::Name("xs".to_string()),
                cond: None,
                elt: Box::new(HirExpr::Name("v".to_string())),
            }),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let mir = build(&hir);
    assert_eq!(
        mir.items[1],
        MirItem::TopLevelStmt(MirStmt::ListCompAssign {
            target: "y".to_string(),
            var: "v".to_string(),
            var_ty: Ty::Str,
            source: CompSource::List("xs".to_string()),
            cond: None,
            elt: Box::new(MirExpr::Name {
                name: "v".to_string(),
                ty: Ty::Str,
            }),
        })
    );
}

#[test]
fn a_range_sourced_set_comprehension_lowers_to_comp_source_range_with_var_ty_int_and_evaluates_its_filter()
 {
    // Exercises the `SetCompAssign` arm's own `cond: Some(..)` path
    // (distinct closures from `ListCompAssign`'s own, needing their own
    // executing test for D-014's coverage gate).
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::TopLevelStmt(HirStmt::SetCompAssign {
            target: "y".to_string(),
            var: "i".to_string(),
            iter: CompIter::Range {
                start: HirExpr::IntLiteral(0),
                stop: HirExpr::IntLiteral(3),
                step: HirExpr::IntLiteral(1),
            },
            cond: Some(Box::new(HirExpr::BoolLiteral(true))),
            elt: Box::new(HirExpr::Name("i".to_string())),
        })],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let mir = build(&hir);
    assert_eq!(
        mir.items[0],
        MirItem::TopLevelStmt(MirStmt::SetCompAssign {
            ops: None,
            target: "y".to_string(),
            var: "i".to_string(),
            var_ty: Ty::Int,
            source: CompSource::Range {
                start: MirExpr::IntLiteral(0),
                stop: MirExpr::IntLiteral(3),
                step: MirExpr::IntLiteral(1),
            },
            cond: Some(Box::new(MirExpr::BoolLiteral(true))),
            elt: Box::new(MirExpr::Name {
                name: "i".to_string(),
                ty: Ty::Int,
            }),
        })
    );
}

#[test]
fn a_bare_name_set_sourced_set_comprehension_resolves_comp_source_set() {
    // Exercises `resolve_comp_source`'s `CompIter::Name` branch resolving
    // to `Ty::Set`.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "s".to_string(),
                value: HirExpr::SetLiteral(vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)]),
            }),
            HirItem::TopLevelStmt(HirStmt::SetCompAssign {
                target: "y".to_string(),
                var: "v".to_string(),
                iter: CompIter::Name("s".to_string()),
                cond: None,
                elt: Box::new(HirExpr::Name("v".to_string())),
            }),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let mir = build(&hir);
    assert_eq!(
        mir.items[1],
        MirItem::TopLevelStmt(MirStmt::SetCompAssign {
            ops: None,
            target: "y".to_string(),
            var: "v".to_string(),
            var_ty: Ty::Int,
            source: CompSource::Set("s".to_string()),
            cond: None,
            elt: Box::new(MirExpr::Name {
                name: "v".to_string(),
                ty: Ty::Int,
            }),
        })
    );
}

#[test]
fn a_bare_name_dict_sourced_dict_comprehension_resolves_comp_source_dict_with_var_ty_as_the_key_type_not_the_value_type()
 {
    // Exercises `resolve_comp_source`'s `CompIter::Name` branch resolving
    // to `Ty::Dict`, and the `DictCompAssign` arm's own `cond: Some(..)`
    // path (distinct closures from `ListCompAssign`/`SetCompAssign`'s
    // own). Pins that `var_ty` is the dict's *key* type (`kv.0`), not its
    // value type (`kv.1`) -- mirrors `ForList`'s own identical
    // `Ty::Dict(kv) => kv.0` choice (`for_k_in_dict_lowers_to_mir_for_dict`
    // above binds a `dict[str, int]`'s loop variable as `Ty::Str`, the
    // key type, for the same reason).
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "d".to_string(),
                value: HirExpr::DictLiteral(vec![(
                    HirExpr::StringLiteral("a".to_string()),
                    HirExpr::IntLiteral(1),
                )]),
            }),
            HirItem::TopLevelStmt(HirStmt::DictCompAssign {
                target: "y".to_string(),
                var: "k".to_string(),
                iter: CompIter::Name("d".to_string()),
                cond: Some(Box::new(HirExpr::BoolLiteral(true))),
                key: Box::new(HirExpr::Name("k".to_string())),
                value: Box::new(HirExpr::IntLiteral(2)),
            }),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let mir = build(&hir);
    assert_eq!(
        mir.items[1],
        MirItem::TopLevelStmt(MirStmt::DictCompAssign {
            target: "y".to_string(),
            var: "k".to_string(),
            var_ty: Ty::Str,
            source: CompSource::Dict("d".to_string()),
            cond: Some(Box::new(MirExpr::BoolLiteral(true))),
            key: Box::new(MirExpr::Name {
                name: "k".to_string(),
                ty: Ty::Str,
            }),
            value: Box::new(MirExpr::IntLiteral(2)),
        })
    );
}

#[test]
#[should_panic(expected = "neither a list, dict, set, nor frozenset")]
fn a_comprehension_over_a_non_list_non_dict_non_set_binding_panics_with_an_internal_error() {
    // Same reasoning as `a_for_list_loop_over_a_non_list_non_dict_non_set_binding_panics_with_an_internal_error`
    // above: `pycc_types` already rejects a comprehension whose bare-name
    // iterable is neither a list, dict, set, nor frozenset (T0033), but
    // `resolve_comp_source`'s own defensive panic path still needs
    // direct coverage via a hand-built HIR that bypasses that guarantee.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::IntLiteral(5),
            }),
            HirItem::TopLevelStmt(HirStmt::ListCompAssign {
                target: "y".to_string(),
                var: "v".to_string(),
                iter: CompIter::Name("x".to_string()),
                cond: None,
                elt: Box::new(HirExpr::Name("v".to_string())),
            }),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    build(&hir);
}

// -- #1254 (D-250): comprehensions in expression position --

fn module_of(items: Vec<HirStmt>) -> HirModule {
    HirModule {
        seeded_builtin_exception_classes: false,
        items: items.into_iter().map(HirItem::TopLevelStmt).collect(),
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    }
}

fn comp(var: &str, iter: CompIter, cond: Option<HirExpr>, elt: pycc_hir::CompElt) -> HirExpr {
    HirExpr::Comprehension(Box::new(pycc_hir::HirComprehension {
        var: var.to_string(),
        iter,
        cond,
        elt,
    }))
}

fn int_name(name: &str) -> MirExpr {
    MirExpr::Name {
        name: name.to_string(),
        ty: Ty::Int,
    }
}

#[test]
fn an_expression_list_comprehension_lowers_its_filter_and_element_against_the_loop_variable() {
    let hir = module_of(vec![HirStmt::ExprStmt(HirExpr::Call {
        callee: "len".to_string(),
        args: vec![comp(
            "0comp_i",
            CompIter::Range {
                start: HirExpr::IntLiteral(0),
                stop: HirExpr::IntLiteral(3),
                step: HirExpr::IntLiteral(1),
            },
            Some(HirExpr::Name("0comp_i".to_string())),
            pycc_hir::CompElt::List(HirExpr::Name("0comp_i".to_string())),
        )],
    })]);
    let mir = build(&hir);
    let MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call { args, .. })) = &mir.items[0] else {
        panic!("expected a `len` call statement, got {:?}", mir.items[0]);
    };
    let expected = MirExpr::Comprehension(Box::new(MirComprehension {
        var: "0comp_i".to_string(),
        var_ty: Ty::Int,
        source: CompSource::Range {
            start: MirExpr::IntLiteral(0),
            stop: MirExpr::IntLiteral(3),
            step: MirExpr::IntLiteral(1),
        },
        cond: Some(int_name("0comp_i")),
        elt: MirCompElt::List(int_name("0comp_i")),
    }));
    assert_eq!(args[0], expected);
    assert_eq!(expected.ty(), Ty::List(Box::new(Ty::Int)));
    // The loop variable is node-scoped: there is nothing to predeclare.
    let mut found = Vec::new();
    expected.collect_named_expr_bindings(&mut found);
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn expression_set_and_dict_comprehensions_over_a_dict_bind_the_key_type() {
    let dict = HirStmt::Assign {
        target: "d".to_string(),
        value: HirExpr::DictLiteral(vec![(
            HirExpr::StringLiteral("a".to_string()),
            HirExpr::IntLiteral(1),
        )]),
    };
    let set_comp = comp(
        "0comp_k",
        CompIter::Name("d".to_string()),
        None,
        pycc_hir::CompElt::Set(HirExpr::IntLiteral(7)),
    );
    let dict_comp = comp(
        "0comp_k",
        CompIter::Name("d".to_string()),
        None,
        pycc_hir::CompElt::Dict {
            key: HirExpr::Name("0comp_k".to_string()),
            value: HirExpr::IntLiteral(2),
        },
    );
    let mir = build(&module_of(vec![
        dict,
        HirStmt::ExprStmt(set_comp),
        HirStmt::ExprStmt(dict_comp),
    ]));
    let MirItem::TopLevelStmt(MirStmt::ExprStmt(set_mir)) = &mir.items[1] else {
        panic!("expected an expression statement, got {:?}", mir.items[1]);
    };
    let MirItem::TopLevelStmt(MirStmt::ExprStmt(dict_mir)) = &mir.items[2] else {
        panic!("expected an expression statement, got {:?}", mir.items[2]);
    };
    assert_eq!(set_mir.ty(), Ty::Set(Box::new(Ty::Int)));
    // An `int` element needs no user element ops (#1344).
    let MirExpr::Comprehension(set_comp) = set_mir else {
        panic!("expected a comprehension, got {set_mir:?}");
    };
    assert!(
        matches!(set_comp.elt, MirCompElt::Set(_, None)),
        "{set_comp:?}"
    );
    assert_eq!(dict_mir.ty(), Ty::Dict(Box::new((Ty::Str, Ty::Int))));
    let MirExpr::Comprehension(dict_comp) = dict_mir else {
        panic!("expected a comprehension, got {dict_mir:?}");
    };
    assert_eq!(dict_comp.var_ty, Ty::Str);
    assert_eq!(dict_comp.source, CompSource::Dict("d".to_string()));
    assert_eq!(dict_comp.cond, None);
}
