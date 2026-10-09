//! Collection literal, subscript, iteration, and mutation lowering.
//!
//! Covers list, dict, set, and tuple literals, `MirExpr::Subscript`,
//! `DictGet`/`DictSet`, `append`/`pop`/`add`, the `for` loops over each
//! container, and every internal-error panic their type derivation raises.

use crate::*;
use pycc_hir::{HirExpr, HirItem, HirModule, HirStmt, Ty};

#[test]
fn lowers_list_literal_to_mir() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::TopLevelStmt(HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::ListLiteral(vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)]),
        })],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let mir = build(&hir);
    // Not a `let PATTERN = ... else { panic!(...) }` destructure -- this
    // file's own coverage-gate convention (see `pycc_hir`'s equivalent
    // `ListLiteral` test, commit 48f13e6) is that a hand-written panic
    // arm is never taken on the happy path and shows up as a
    // permanently uncovered region under D-014's 100%-regions gate. A
    // direct `assert_eq!` against the whole expected `MirItem` avoids
    // that without weakening the assertion.
    assert_eq!(
        mir.items[0],
        MirItem::TopLevelStmt(MirStmt::Assign {
            target: "x".to_string(),
            value: MirExpr::ListLiteral(vec![MirExpr::IntLiteral(1), MirExpr::IntLiteral(2)]),
        })
    );
}

#[test]
fn lowers_for_list_to_mir_for_list() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::ListLiteral(vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)]),
            }),
            HirItem::TopLevelStmt(HirStmt::ForList {
                var: "v".to_string(),
                list: "x".to_string(),
                body: vec![HirStmt::ExprStmt(HirExpr::Call {
                    callee: "print".to_string(),
                    args: vec![HirExpr::Name("v".to_string())],
                })],
            }),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let mir = build(&hir);
    assert_eq!(
        mir.items[1],
        MirItem::TopLevelStmt(MirStmt::ForList {
            var: "v".to_string(),
            list: "x".to_string(),
            body: vec![MirStmt::ExprStmt(MirExpr::Call {
                callee: "print".to_string(),
                args: vec![MirExpr::Name {
                    name: "v".to_string(),
                    ty: Ty::Int,
                }],
                ty: Ty::None,
            })],
        })
    );
}

#[test]
fn lowers_subscript_to_mir_recursively() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::ListLiteral(vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)]),
            }),
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "y".to_string(),
                value: HirExpr::Subscript {
                    base: Box::new(HirExpr::Name("x".to_string())),
                    index: Box::new(HirExpr::IntLiteral(0)),
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
            value: MirExpr::Subscript {
                base: Box::new(MirExpr::Name {
                    name: "x".to_string(),
                    ty: Ty::List(Box::new(Ty::Int)),
                }),
                index: Box::new(MirExpr::IntLiteral(0)),
            },
        })
    );
}

#[test]
fn lowers_list_append_to_mir_recursively() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::ListLiteral(vec![HirExpr::IntLiteral(1)]),
            }),
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::ListAppend {
                list: pycc_hir::ContainerReceiver::Name("x".to_string()),
                value: Box::new(HirExpr::IntLiteral(2)),
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let mir = build(&hir);
    assert_eq!(
        mir.items[1],
        MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::ListAppend {
            list: crate::MirContainerReceiver::Name("x".to_string()),
            value: Box::new(MirExpr::IntLiteral(2)),
        }))
    );
}

#[test]
fn lowers_list_pop_to_mir_deriving_its_element_type_from_the_list_binding() {
    // PR-12 Task 11 (D-119): `xs.pop()`'s `ty` is derived from `xs`'s
    // own `Ty::List` binding via `lookup`, mirroring
    // `HirExpr::Subscript`'s own base-type lookup.
    let hir = HirModule {
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
                value: HirExpr::ListPop {
                    list: pycc_hir::ContainerReceiver::Name("xs".to_string()),
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
            value: MirExpr::ListPop {
                list: crate::MirContainerReceiver::Name("xs".to_string()),
                ty: Ty::Int,
            },
        })
    );
}

#[test]
#[should_panic(expected = "`.pop()` receiver Name(\"xs\") is not list-typed")]
fn list_pop_over_a_non_list_binding_panics_with_an_internal_error() {
    // `pycc_types` already rejects `.pop()` on a non-list base (T0033)
    // before HIR reaches `pycc_mir`, but the defensive panic path in
    // `lower_expr`'s own `HirExpr::ListPop` arm still needs direct
    // coverage, mirroring `a_for_list_loop_over_a_non_list_non_dict_non_set_binding_panics_with_an_internal_error`
    // above.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "xs".to_string(),
                value: HirExpr::IntLiteral(5),
            }),
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::ListPop {
                list: pycc_hir::ContainerReceiver::Name("xs".to_string()),
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    build(&hir);
}

#[test]
fn lowers_dict_get_or_default_to_mir_recursively_deriving_its_value_type() {
    // PR-12 Task 11 (D-119): `d.get(key, default)`'s `ty` is derived
    // from `d`'s own `Ty::Dict` binding's value type, and both `key`
    // and `default` are recursively lowered.
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
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "y".to_string(),
                value: HirExpr::DictGetOrDefault {
                    dict: pycc_hir::ContainerReceiver::Name("d".to_string()),
                    key: Box::new(HirExpr::StringLiteral("z".to_string())),
                    default: Box::new(HirExpr::IntLiteral(-1)),
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
            value: MirExpr::DictGetOrDefault {
                dict: crate::MirContainerReceiver::Name("d".to_string()),
                key: Box::new(MirExpr::StringLiteral("z".to_string())),
                default: Box::new(MirExpr::IntLiteral(-1)),
                ty: Ty::Int,
            },
        })
    );
}

#[test]
#[should_panic(expected = "`.get()` receiver Name(\"d\") is not dict-typed")]
fn dict_get_or_default_over_a_non_dict_binding_panics_with_an_internal_error() {
    // Same reasoning as `list_pop_over_a_non_list_binding_panics_with_an_internal_error`
    // above, for `HirExpr::DictGetOrDefault`'s own defensive panic path.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "d".to_string(),
                value: HirExpr::IntLiteral(5),
            }),
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::DictGetOrDefault {
                dict: pycc_hir::ContainerReceiver::Name("d".to_string()),
                key: Box::new(HirExpr::StringLiteral("a".to_string())),
                default: Box::new(HirExpr::IntLiteral(0)),
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    build(&hir);
}

#[test]
fn lowers_set_add_to_mir_recursively() {
    // PR-12 Task 11 (D-119): `s.add(value)` mirrors `ListAppend`'s own
    // shape exactly -- `set` is carried as a plain name, `value` is
    // recursively lowered.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "s".to_string(),
                value: HirExpr::SetLiteral(vec![HirExpr::IntLiteral(1)]),
            }),
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::SetAdd {
                set: "s".to_string(),
                value: Box::new(HirExpr::IntLiteral(2)),
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let mir = build(&hir);
    assert_eq!(
        mir.items[1],
        MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::SetAdd {
            set: "s".to_string(),
            value: Box::new(MirExpr::IntLiteral(2)),
            ops: None,
        }))
    );
}

#[test]
fn list_literal_subscript_and_for_list_derive_their_type_from_actual_elements_not_hardcoded_int() {
    // Mirrors `pycc_types`'s own genericity tests for `ListLiteral`,
    // `Subscript`, and `ForList` (see e.g. its
    // `a_for_list_loop_binds_its_variable_as_str_for_a_list_of_str`):
    // this lowering must derive `ty()`/the loop variable's bound type
    // from the list's *actual* element type, not assume `Ty::Int`.
    // `pycc_types`'s T0034 gate means only `list[int]` ever reaches
    // this crate from a real compiled program, but this crate's own
    // lowering must not bake in that assumption independently of the
    // type it actually observes -- exactly the class of bug the
    // `AnnAssign` widening fix in `stmt.rs`'s `lower_stmt` already
    // guards against (MIR's `ty` silently diverging from what codegen
    // must produce). Uses `str` elements specifically because they are
    // trivially distinguishable from the `Ty::Int` a hardcoded bug
    // would wrongly report.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "xs".to_string(),
                value: HirExpr::ListLiteral(vec![HirExpr::StringLiteral("a".to_string())]),
            }),
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "y".to_string(),
                value: HirExpr::Subscript {
                    base: Box::new(HirExpr::Name("xs".to_string())),
                    index: Box::new(HirExpr::IntLiteral(0)),
                },
            }),
            HirItem::TopLevelStmt(HirStmt::ForList {
                var: "v".to_string(),
                list: "xs".to_string(),
                body: vec![HirStmt::ExprStmt(HirExpr::Name("v".to_string()))],
            }),
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Name("y".to_string()))),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let mir = build(&hir);
    // `y = xs[0]` binds `y` as `Ty::Str`, derived from `xs`'s own
    // `Ty::List(Box::new(Ty::Str))` binding (itself derived from the
    // `StringLiteral` element), not `Ty::Int`.
    assert_eq!(
        mir.items[3],
        MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Name {
            name: "y".to_string(),
            ty: Ty::Str,
        }))
    );
    // `for v in xs:` binds `v` as `Ty::Str` too, derived from the same
    // list, not `Ty::Int`.
    assert_eq!(
        mir.items[2],
        MirItem::TopLevelStmt(MirStmt::ForList {
            var: "v".to_string(),
            list: "xs".to_string(),
            body: vec![MirStmt::ExprStmt(MirExpr::Name {
                name: "v".to_string(),
                ty: Ty::Str,
            })],
        })
    );
}

#[test]
#[should_panic(expected = "an empty list literal has no element type")]
fn an_empty_list_literals_ty_panics_with_an_internal_error() {
    // By construction (see this module's `lookup` panic doc comment /
    // D-057 discussion), `pycc_types::check` already rejects an empty
    // list literal (T0021) before any HIR reaches `pycc_mir` -- this
    // MIR node could never come from a real `check_and_resolve`
    // success, but the panic path itself still needs direct coverage.
    MirExpr::ListLiteral(vec![]).ty();
}

#[test]
#[should_panic(expected = "subscript base has non-list/tuple type")]
fn a_subscript_over_a_non_list_bases_ty_panics_with_an_internal_error() {
    // A non-list, non-dict, non-tuple subscript base (e.g. `Ty::Int`) is
    // rejected by `pycc_types` (T0033) before HIR reaches `pycc_mir` --
    // unlike a dict base (which `pycc_types` accepts, but `lower_expr`'s
    // own `HirExpr::Subscript` arm routes into `MirExpr::DictGet` instead
    // of ever constructing this node), so this defensive panic path in
    // `MirExpr::Subscript`'s own `ty()` arm still needs direct coverage
    // via a hand-built node that bypasses both guarantees.
    MirExpr::Subscript {
        base: Box::new(MirExpr::IntLiteral(1)),
        index: Box::new(MirExpr::IntLiteral(0)),
    }
    .ty();
}

#[test]
#[should_panic(expected = "dict subscript base has non-dict type")]
fn a_dict_get_over_a_non_dict_bases_ty_panics_with_an_internal_error() {
    // Same reasoning as the subscript panic above, for `MirExpr::DictGet`'s
    // own defensive `ty()` arm: no real lowering ever constructs this
    // node with a non-dict base (`lower_expr`'s own `HirExpr::Subscript`
    // arm only builds `MirExpr::DictGet` when the base's derived type is
    // `Ty::Dict`), but the panic path still needs direct coverage.
    MirExpr::DictGet {
        dict: Box::new(MirExpr::IntLiteral(1)),
        key: Box::new(MirExpr::StringLiteral("a".to_string())),
    }
    .ty();
}

#[test]
#[should_panic(expected = "neither a list, dict, set, nor frozenset")]
fn a_for_list_loop_over_a_non_list_non_dict_non_set_binding_panics_with_an_internal_error() {
    // Same reasoning again: `pycc_types` already rejects `for v in x:`
    // when `x` is neither a list, dict, set, nor frozenset (T0033), but the
    // defensive panic path in `lower_stmt`'s `ForList` arm still needs
    // direct coverage.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::IntLiteral(5),
            }),
            HirItem::TopLevelStmt(HirStmt::ForList {
                var: "v".to_string(),
                list: "x".to_string(),
                body: vec![],
            }),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    build(&hir);
}

#[test]
#[should_panic(expected = "an empty dict literal has no key/value type to derive")]
fn an_empty_dict_literals_ty_panics_with_an_internal_error() {
    // By construction, `pycc_types::check` already rejects an empty
    // dict literal (T0021, mirroring the empty-list-literal case above)
    // before any HIR reaches `pycc_mir` -- this MIR node could never
    // come from a real `check_and_resolve` success, but the panic path
    // itself still needs direct coverage.
    MirExpr::DictLiteral(vec![]).ty();
}

#[test]
fn dict_literal_lowers_to_mir_dict_literal_with_correct_ty() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::TopLevelStmt(HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::DictLiteral(vec![(
                HirExpr::StringLiteral("a".to_string()),
                HirExpr::IntLiteral(1),
            )]),
        })],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let mir = build(&hir);
    let expected_value = MirExpr::DictLiteral(vec![(
        MirExpr::StringLiteral("a".to_string()),
        MirExpr::IntLiteral(1),
    )]);
    assert_eq!(expected_value.ty(), Ty::Dict(Box::new((Ty::Str, Ty::Int))));
    assert_eq!(
        mir.items[0],
        MirItem::TopLevelStmt(MirStmt::Assign {
            target: "x".to_string(),
            value: expected_value,
        })
    );
}

#[test]
fn dict_get_ty_unwraps_the_value_type() {
    // `x["a"]` where `x: dict[str, int]` lowers `HirExpr::Subscript`
    // into `MirExpr::DictGet` (not `MirExpr::Subscript`), whose `ty()`
    // unwraps the dict's value type, mirroring `dict_get_ty_unwraps_the_value_type`
    // in the task brief.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::DictLiteral(vec![(
                    HirExpr::StringLiteral("a".to_string()),
                    HirExpr::IntLiteral(1),
                )]),
            }),
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "y".to_string(),
                value: HirExpr::Subscript {
                    base: Box::new(HirExpr::Name("x".to_string())),
                    index: Box::new(HirExpr::StringLiteral("a".to_string())),
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
            value: MirExpr::DictGet {
                dict: Box::new(MirExpr::Name {
                    name: "x".to_string(),
                    ty: Ty::Dict(Box::new((Ty::Str, Ty::Int))),
                }),
                key: Box::new(MirExpr::StringLiteral("a".to_string())),
            },
        })
    );
    assert_eq!(
        MirExpr::DictGet {
            dict: Box::new(MirExpr::Name {
                name: "x".to_string(),
                ty: Ty::Dict(Box::new((Ty::Str, Ty::Int))),
            }),
            key: Box::new(MirExpr::StringLiteral("a".to_string())),
        }
        .ty(),
        Ty::Int
    );
}

#[test]
fn a_list_subscript_still_lowers_to_mir_subscript_not_dict_get() {
    // Genericity check mirroring `list_literal_subscript_and_for_list_derive_their_type_from_actual_elements_not_hardcoded_int`
    // above: `lower_expr`'s `HirExpr::Subscript` arm must route based on
    // the base's *actual* derived type, not assume every subscript is a
    // dict read now that `MirExpr::DictGet` exists.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "xs".to_string(),
                value: HirExpr::ListLiteral(vec![HirExpr::IntLiteral(1)]),
            }),
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "y".to_string(),
                value: HirExpr::Subscript {
                    base: Box::new(HirExpr::Name("xs".to_string())),
                    index: Box::new(HirExpr::IntLiteral(0)),
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
            value: MirExpr::Subscript {
                base: Box::new(MirExpr::Name {
                    name: "xs".to_string(),
                    ty: Ty::List(Box::new(Ty::Int)),
                }),
                index: Box::new(MirExpr::IntLiteral(0)),
            },
        })
    );
}

#[test]
fn dict_set_lowers_to_mir_dict_set_stmt() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::DictLiteral(vec![(
                    HirExpr::StringLiteral("a".to_string()),
                    HirExpr::IntLiteral(1),
                )]),
            }),
            HirItem::TopLevelStmt(HirStmt::DictSet {
                dict: "x".to_string(),
                key: HirExpr::StringLiteral("b".to_string()),
                value: HirExpr::IntLiteral(2),
            }),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let mir = build(&hir);
    assert_eq!(
        mir.items[1],
        MirItem::TopLevelStmt(MirStmt::DictSet {
            dict: "x".to_string(),
            key: MirExpr::StringLiteral("b".to_string()),
            value: MirExpr::IntLiteral(2),
        })
    );
}

#[test]
fn for_k_in_dict_lowers_to_mir_for_dict() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::DictLiteral(vec![(
                    HirExpr::StringLiteral("a".to_string()),
                    HirExpr::IntLiteral(1),
                )]),
            }),
            HirItem::TopLevelStmt(HirStmt::ForList {
                var: "k".to_string(),
                list: "x".to_string(),
                body: vec![HirStmt::ExprStmt(HirExpr::Call {
                    callee: "print".to_string(),
                    args: vec![HirExpr::Name("k".to_string())],
                })],
            }),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let mir = build(&hir);
    assert_eq!(
        mir.items[1],
        MirItem::TopLevelStmt(MirStmt::ForDict {
            var: "k".to_string(),
            dict: "x".to_string(),
            body: vec![MirStmt::ExprStmt(MirExpr::Call {
                callee: "print".to_string(),
                args: vec![MirExpr::Name {
                    name: "k".to_string(),
                    ty: Ty::Str,
                }],
                ty: Ty::None,
            })],
        })
    );
}

#[test]
#[should_panic(expected = "an empty set literal has no element type to derive")]
fn an_empty_set_literals_ty_panics_with_an_internal_error() {
    // By construction, `pycc_types::check` already rejects an empty set
    // literal (mirroring the empty-list/empty-dict-literal cases above)
    // before any HIR reaches `pycc_mir` -- and, unlike those two, an
    // empty `SetLiteral` cannot even be *written* in real Python source
    // (`{}` always parses as an empty `dict`, never an empty `set`) --
    // but the panic path itself still needs direct coverage.
    MirExpr::SetLiteral {
        elements: vec![],
        ops: None,
    }
    .ty();
}

#[test]
fn tuple_literal_ty_derives_positionally_from_every_element() {
    let expr = MirExpr::TupleLiteral(vec![
        MirExpr::IntLiteral(1),
        MirExpr::BoolLiteral(true),
        MirExpr::FloatLiteral(2.5),
    ]);
    assert_eq!(
        expr.ty(),
        Ty::Tuple(Box::new(vec![Ty::Int, Ty::Bool, Ty::Float]))
    );
}

#[test]
#[should_panic(expected = "an empty tuple literal has no element types to derive")]
fn an_empty_tuple_literal_ty_panics_with_an_internal_error() {
    // By construction, `pycc_types::check` already rejects an empty
    // tuple literal (T0039, mirroring the empty-list/empty-dict-literal
    // cases above) before any HIR reaches `pycc_mir` -- but the panic
    // path itself still needs direct coverage.
    MirExpr::TupleLiteral(vec![]).ty();
}

#[test]
fn tuple_subscript_ty_derives_the_positional_element_type() {
    let expr = MirExpr::Subscript {
        base: Box::new(MirExpr::TupleLiteral(vec![
            MirExpr::IntLiteral(1),
            MirExpr::BoolLiteral(true),
        ])),
        index: Box::new(MirExpr::IntLiteral(1)),
    };
    assert_eq!(expr.ty(), Ty::Bool);
}

#[test]
#[should_panic(expected = "tuple subscript index is not a literal int")]
fn a_tuple_subscript_with_a_non_literal_index_ty_panics_with_an_internal_error() {
    // By construction, `pycc_types::check` already rejects a
    // non-literal tuple subscript index (T0040) before any HIR reaches
    // `pycc_mir` -- but the panic path itself still needs direct
    // coverage via a hand-built node that bypasses that guarantee.
    let expr = MirExpr::Subscript {
        base: Box::new(MirExpr::TupleLiteral(vec![MirExpr::IntLiteral(1)])),
        index: Box::new(MirExpr::Name {
            name: "i".to_string(),
            ty: Ty::Int,
        }),
    };
    expr.ty();
}

#[test]
#[should_panic(expected = "tuple subscript index is negative")]
fn a_tuple_subscript_with_a_negative_index_ty_panics_with_an_internal_error() {
    // By construction, `pycc_types::check` already rejects a negative
    // literal tuple subscript index (T0040) before any HIR reaches
    // `pycc_mir` -- but the panic path itself still needs direct
    // coverage via a hand-built node that bypasses that guarantee.
    let expr = MirExpr::Subscript {
        base: Box::new(MirExpr::TupleLiteral(vec![MirExpr::IntLiteral(1)])),
        index: Box::new(MirExpr::IntLiteral(-1)),
    };
    expr.ty();
}

#[test]
#[should_panic(expected = "tuple subscript index out of range")]
fn a_tuple_subscript_out_of_range_ty_panics_with_an_internal_error() {
    // By construction, `pycc_types::check` already rejects an
    // out-of-range literal tuple subscript index (T0040) before any HIR
    // reaches `pycc_mir` -- but the panic path itself still needs
    // direct coverage via a hand-built node that bypasses that
    // guarantee.
    let expr = MirExpr::Subscript {
        base: Box::new(MirExpr::TupleLiteral(vec![MirExpr::IntLiteral(1)])),
        index: Box::new(MirExpr::IntLiteral(5)),
    };
    expr.ty();
}

#[test]
fn tuple_literal_lowers_to_mir_tuple_literal_with_correct_ty() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::TopLevelStmt(HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::TupleLiteral(vec![HirExpr::IntLiteral(1), HirExpr::BoolLiteral(true)]),
        })],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let mir = build(&hir);
    let expected_value =
        MirExpr::TupleLiteral(vec![MirExpr::IntLiteral(1), MirExpr::BoolLiteral(true)]);
    assert_eq!(
        expected_value.ty(),
        Ty::Tuple(Box::new(vec![Ty::Int, Ty::Bool]))
    );
    assert_eq!(
        mir.items[0],
        MirItem::TopLevelStmt(MirStmt::Assign {
            target: "x".to_string(),
            value: expected_value,
        })
    );
}

#[test]
fn set_literal_lowers_to_mir_set_literal_with_correct_ty() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::TopLevelStmt(HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::SetLiteral(vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)]),
        })],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let mir = build(&hir);
    let expected_value = MirExpr::SetLiteral {
        elements: vec![MirExpr::IntLiteral(1), MirExpr::IntLiteral(2)],
        ops: None,
    };
    assert_eq!(expected_value.ty(), Ty::Set(Box::new(Ty::Int)));
    assert_eq!(
        mir.items[0],
        MirItem::TopLevelStmt(MirStmt::Assign {
            target: "x".to_string(),
            value: expected_value,
        })
    );
}

#[test]
fn for_x_in_set_lowers_to_mir_for_set() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::SetLiteral(vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)]),
            }),
            HirItem::TopLevelStmt(HirStmt::ForList {
                var: "v".to_string(),
                list: "x".to_string(),
                body: vec![HirStmt::ExprStmt(HirExpr::Call {
                    callee: "print".to_string(),
                    args: vec![HirExpr::Name("v".to_string())],
                })],
            }),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let mir = build(&hir);
    assert_eq!(
        mir.items[1],
        MirItem::TopLevelStmt(MirStmt::ForSet {
            var: "v".to_string(),
            var_ty: Ty::Int,
            set: "x".to_string(),
            body: vec![MirStmt::ExprStmt(MirExpr::Call {
                callee: "print".to_string(),
                args: vec![MirExpr::Name {
                    name: "v".to_string(),
                    ty: Ty::Int,
                }],
                ty: Ty::None,
            })],
        })
    );
}
